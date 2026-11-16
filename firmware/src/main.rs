#![no_std]
#![no_main]
#![deny(
    clippy::mem_forget,
    reason = "mem::forget is generally not safe to do with esp_hal types, especially those \
    holding buffers for the duration of a data transfer."
)]
#![deny(clippy::large_stack_frames)]

mod config_store;
mod console;
mod display;
mod encoder;
mod inputs;
mod link;
mod oled;
mod wifi;

use core::cell::{Cell, RefCell};
use deck_core::config::HostConfig;
use deck_core::mono::MonoFrame;
use deck_core::session::{DeviceIds, Session};
use display::{KeyDisplay, SharedPin};
use embassy_executor::Spawner;
use embassy_sync::blocking_mutex::Mutex;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::signal::Signal;
use embassy_time::{Duration, Timer};
use esp_backtrace as _;
use esp_hal::clock::CpuClock;
use esp_hal::efuse::{self, InterfaceMacAddress};
use esp_hal::gpio::{Input, InputConfig, Level, Output, OutputConfig, Pull};
use esp_hal::i2c::master::{Config as I2cConfig, I2c};
use esp_hal::pcnt::Pcnt;
use esp_hal::spi::Mode;
use esp_hal::spi::master::{Config as SpiConfig, Spi};
use esp_hal::time::Rate;
use esp_hal::timer::timg::TimerGroup;
use esp_hal::uart::{Config as UartConfig, UartRx};
use esp_storage::FlashStorage;
use inputs::Button;
use log::{error, info, warn};
use oled::Oled;
use static_cell::{ConstStaticCell, StaticCell};

extern crate alloc;

esp_bootloader_esp_idf::esp_app_desc!();

/// The saved Companion host, shared by the console (writer) and the link (reader).
pub static HOST: Mutex<CriticalSectionRawMutex, Cell<Option<HostConfig>>> =
    Mutex::new(Cell::new(None));
/// Raised by the console when the host is set or cleared, so the link reconnects at once.
pub static HOST_CHANGED: Signal<CriticalSectionRawMutex, ()> = Signal::new();

/// Replaces the saved host and wakes the link so it reconnects at once.
pub fn set_host(host: Option<HostConfig>) {
    HOST.lock(|h| h.set(host));
    HOST_CHANGED.signal(());
}

/// Holds the longest `KEY-STATE` line: a 128×128 `rgb` bitmap is 65,536 B of base64 (KTD15).
static LINE_BUF: ConstStaticCell<[u8; 72 * 1024]> = ConstStaticCell::new([0; 72 * 1024]);
/// The socket's receive buffer, which is also its TCP window. 32 KB did not change image
/// receive time on the P3 bench (about 190 ms per key image either way) and cost 24 KB of
/// heap, so it stays at 8 KB.
static RX_BUF: ConstStaticCell<[u8; 8 * 1024]> = ConstStaticCell::new([0; 8 * 1024]);
static TX_BUF: ConstStaticCell<[u8; 2 * 1024]> = ConstStaticCell::new([0; 2 * 1024]);
static CHUNK_BUF: ConstStaticCell<[u8; 2 * 1024]> = ConstStaticCell::new([0; 2 * 1024]);
static OUT_BUF: ConstStaticCell<[u8; 1024]> = ConstStaticCell::new([0; 1024]);
static SPI_BUF_A: ConstStaticCell<[u8; 1024]> = ConstStaticCell::new([0; 1024]);
static SPI_BUF_B: ConstStaticCell<[u8; 1024]> = ConstStaticCell::new([0; 1024]);
static OLED_FRAME: ConstStaticCell<MonoFrame> = ConstStaticCell::new(MonoFrame::new());
/// The keys share SPI2 and the DC line (KTD21).
static SPI_BUS: StaticCell<RefCell<display::SpiBus>> = StaticCell::new();
static KEY_DC: StaticCell<RefCell<Output<'static>>> = StaticCell::new();

/// How long the calibration patterns stay up at boot.
const TEST_PATTERN_TIME: Duration = Duration::from_secs(3);

#[allow(
    clippy::large_stack_frames,
    reason = "it's not unusual to allocate larger buffers etc. in main"
)]
#[esp_rtos::main]
async fn main(spawner: Spawner) -> ! {
    esp_println::logger::init_logger_from_env();

    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(config);

    esp_alloc::heap_allocator!(#[esp_hal::ram(reclaimed)] size: 98768);
    esp_alloc::heap_allocator!(size: 36 * 1024);

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0, peripherals.FROM_CPU_INTR0);

    let rev = efuse::chip_revision();
    info!(
        "opendeck booting: chip {:?} rev v{}.{}, free heap {} B",
        efuse::chip_type(),
        rev.major,
        rev.minor,
        esp_alloc::HEAP.free()
    );

    let mut store = config_store::ConfigStore::new(FlashStorage::new(peripherals.FLASH));
    match store.locate() {
        Some((offset, len)) => info!(
            "{} found at {offset:#x}, {} KB",
            config_store::PARTITION,
            len / 1024
        ),
        None => error!(
            "{} partition missing; flash with partitions.csv",
            config_store::PARTITION
        ),
    }
    let saved = store.load();
    match &saved {
        Some(host) => info!("saved host {}:{}", host.host(), host.port),
        None => info!("no host saved; set one with `host <ip-or-name>[:port]`"),
    }
    HOST.lock(|h| h.set(saved));

    // Pins: docs/notes/deck-wiring.md.
    let _backlight = Output::new(peripherals.GPIO14, Level::High, OutputConfig::default());
    let mut key_rst = Output::new(peripherals.GPIO15, Level::High, OutputConfig::default());
    display::reset_panels(&mut key_rst);
    let spi = Spi::new(
        peripherals.SPI2,
        SpiConfig::default()
            .with_frequency(Rate::from_mhz(20))
            .with_mode(Mode::_0),
    )
    .expect("SPI config")
    .with_sck(peripherals.GPIO5)
    .with_mosi(peripherals.GPIO19);
    let bus = SPI_BUS.init(RefCell::new(spi));
    let dc = KEY_DC.init(RefCell::new(Output::new(
        peripherals.GPIO27,
        Level::Low,
        OutputConfig::default(),
    )));
    let mut key_a = KeyDisplay::new(
        bus,
        Output::new(peripherals.GPIO33, Level::High, OutputConfig::default()),
        SharedPin::new(dc),
        SPI_BUF_A.take(),
    );
    let mut key_b = KeyDisplay::new(
        bus,
        Output::new(peripherals.GPIO26, Level::High, OutputConfig::default()),
        SharedPin::new(dc),
        SPI_BUF_B.take(),
    );
    key_a.test_pattern("KEY A");
    key_b.test_pattern("KEY B");

    let i2c = I2c::new(
        peripherals.I2C0,
        I2cConfig::default().with_frequency(Rate::from_khz(oled::calibration::I2C_KHZ)),
    )
    .expect("I2C0 config")
    .with_sda(peripherals.GPIO22)
    .with_scl(peripherals.GPIO20)
    .into_async();
    let mut oled = Oled::new(i2c, OLED_FRAME.take());
    oled.init().await;
    oled.test_pattern();
    if let Err(e) = oled.flush().await {
        warn!("OLED test pattern not shown: {e:?}");
    }

    // The keys use the internal pull-up; the module's inputs have their own (KTD24, KTD25).
    let key_input = InputConfig::default().with_pull(Pull::Up);
    let module_input = InputConfig::default();
    spawner.spawn(
        inputs::inputs_task([
            (Button::KeyA, Input::new(peripherals.GPIO32, key_input)),
            (Button::KeyB, Input::new(peripherals.GPIO25, key_input)),
            (
                Button::EncoderPush,
                Input::new(peripherals.GPIO36, module_input),
            ),
            (Button::Back, Input::new(peripherals.GPIO39, module_input)),
            (
                Button::Confirm,
                Input::new(peripherals.GPIO37, module_input),
            ),
        ])
        .expect("spawn inputs task"),
    );

    let pcnt = Pcnt::new(peripherals.PCNT);
    let phase_a = Input::new(peripherals.GPIO34, module_input);
    let phase_b = Input::new(peripherals.GPIO4, module_input);
    encoder::configure(&pcnt.unit0, &phase_a, &phase_b);
    spawner
        .spawn(encoder::encoder_task(pcnt.unit0, [phase_a, phase_b]).expect("spawn encoder task"));

    let console_rx = UartRx::new(
        peripherals.UART0,
        UartConfig::default().with_baudrate(115_200),
    )
    .expect("UART0 config")
    .with_rx(peripherals.GPIO3)
    .into_async();
    spawner.spawn(console::console_task(console_rx, store).expect("spawn console task"));

    let stack = wifi::start(spawner, peripherals.WIFI);

    let mac: [u8; 6] = efuse::interface_mac_address(InterfaceMacAddress::Station)
        .as_bytes()
        .try_into()
        .expect("6-byte MAC");
    let session = Session::new(DeviceIds::from_mac(mac));

    Timer::after(TEST_PATTERN_TIME).await;
    link::run(
        stack,
        link::Displays { key_a, key_b, oled },
        session,
        link::Buffers {
            line: LINE_BUF.take(),
            rx: RX_BUF.take(),
            tx: TX_BUF.take(),
            chunk: CHUNK_BUF.take(),
            out: OUT_BUF.take(),
        },
    )
    .await
}
