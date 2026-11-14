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
mod key;
mod link;
mod wifi;

use core::cell::Cell;
use deck_core::config::HostConfig;
use deck_core::session::{DeviceIds, Session};
use embassy_executor::Spawner;
use embassy_sync::blocking_mutex::Mutex;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::signal::Signal;
use embassy_time::{Duration, Timer};
use esp_backtrace as _;
use esp_hal::clock::CpuClock;
use esp_hal::efuse::{self, InterfaceMacAddress};
use esp_hal::gpio::{Input, InputConfig, Level, Output, OutputConfig, Pull};
use esp_hal::spi::Mode;
use esp_hal::spi::master::{Config as SpiConfig, Spi};
use esp_hal::time::Rate;
use esp_hal::timer::timg::TimerGroup;
use esp_hal::uart::{Config as UartConfig, UartRx};
use esp_storage::FlashStorage;
use log::{error, info};
use static_cell::ConstStaticCell;

extern crate alloc;

esp_bootloader_esp_idf::esp_app_desc!();

/// The saved Companion host, shared by the console (writer) and the link (reader).
pub static HOST: Mutex<CriticalSectionRawMutex, Cell<Option<HostConfig>>> =
    Mutex::new(Cell::new(None));
/// Raised by the console when the host is set or cleared, so the link reconnects at once.
pub static HOST_CHANGED: Signal<CriticalSectionRawMutex, ()> = Signal::new();

/// Holds the longest `KEY-STATE` line: a 128×128 `rgb` bitmap is 65,536 B of base64 (KTD15).
static LINE_BUF: ConstStaticCell<[u8; 72 * 1024]> = ConstStaticCell::new([0; 72 * 1024]);
static RX_BUF: ConstStaticCell<[u8; 8 * 1024]> = ConstStaticCell::new([0; 8 * 1024]);
static TX_BUF: ConstStaticCell<[u8; 2 * 1024]> = ConstStaticCell::new([0; 2 * 1024]);
static CHUNK_BUF: ConstStaticCell<[u8; 2 * 1024]> = ConstStaticCell::new([0; 2 * 1024]);
static OUT_BUF: ConstStaticCell<[u8; 1024]> = ConstStaticCell::new([0; 1024]);
static SPI_BUF: ConstStaticCell<[u8; 1024]> = ConstStaticCell::new([0; 1024]);

/// How long the calibration pattern stays up at boot (U8).
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

    // Wiring (P2): SCK 5, MOSI 19, CS 33, DC 27, RST 15, backlight 14, KEY 32.
    let _backlight = Output::new(peripherals.GPIO14, Level::High, OutputConfig::default());
    let spi = Spi::new(
        peripherals.SPI2,
        SpiConfig::default()
            .with_frequency(Rate::from_mhz(20))
            .with_mode(Mode::_0),
    )
    .expect("SPI config")
    .with_sck(peripherals.GPIO5)
    .with_mosi(peripherals.GPIO19);
    let mut display = display::KeyDisplay::new(
        spi,
        Output::new(peripherals.GPIO33, Level::High, OutputConfig::default()),
        Output::new(peripherals.GPIO27, Level::Low, OutputConfig::default()),
        Output::new(peripherals.GPIO15, Level::High, OutputConfig::default()),
        SPI_BUF.take(),
    );
    display.test_pattern();

    let key_pin = Input::new(
        peripherals.GPIO32,
        InputConfig::default().with_pull(Pull::Up),
    );
    spawner.spawn(key::key_task(key_pin).expect("spawn key task"));

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
        display,
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
