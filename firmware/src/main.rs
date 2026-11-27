#![no_std]
#![no_main]
#![deny(
    clippy::mem_forget,
    reason = "mem::forget is generally not safe to do with esp_hal types, especially those \
    holding buffers for the duration of a data transfer."
)]
#![deny(clippy::large_stack_frames)]

mod board;
mod config_store;
mod console;
mod display;
mod encoder;
mod expander;
mod i2c;
mod inputs;
mod link;
mod oled;

use core::cell::{Cell, RefCell};
use deck_core::config::HostConfig;
use deck_core::expander::Key;
use deck_core::mono::MonoFrame;
use deck_core::session::{DeviceIds, Session, SurfaceConfig};
use display::{KeyDisplay, SharedPin};
use embassy_executor::Spawner;
use embassy_sync::blocking_mutex::Mutex;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::mutex::Mutex as AsyncMutex;
use embassy_sync::signal::Signal;
use embassy_time::{Duration, Timer};
use esp_backtrace as _;
use esp_hal::clock::CpuClock;
use esp_hal::efuse;
use esp_hal::gpio::Output;
use esp_hal::timer::timg::TimerGroup;
use esp_storage::FlashStorage;
use expander::Expander;
use i2c::I2cBus;
use log::{error, info, warn};
use oled::Oled;
use static_cell::{ConstStaticCell, StaticCell};

extern crate alloc;

esp_bootloader_esp_idf::esp_app_desc!();

/// The saved Companion host, shared by the console (writer) and the link (reader).
pub static HOST: Mutex<CriticalSectionRawMutex, Cell<Option<HostConfig>>> =
    Mutex::new(Cell::new(None));
/// Raised by the console when the host or the surface changes, so the link reconnects at
/// once.
pub static HOST_CHANGED: Signal<CriticalSectionRawMutex, ()> = Signal::new();
/// What the next registration registers as (THI-76). RAM only: a reboot returns to the
/// normal surface.
pub static SURFACE: Mutex<CriticalSectionRawMutex, Cell<SurfaceConfig>> =
    Mutex::new(Cell::new(SurfaceConfig::NORMAL));

/// Replaces the saved host and wakes the link so it reconnects at once.
pub fn set_host(host: Option<HostConfig>) {
    HOST.lock(|h| h.set(host));
    HOST_CHANGED.signal(());
}

/// Replaces the surface config and wakes the link so it re-registers at once.
pub fn set_surface(surface: SurfaceConfig) {
    SURFACE.lock(|s| s.set(surface));
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
/// Outgoing lines; a 32-key bench `ADD-DEVICE` is about 2.3 KB (THI-76).
static OUT_BUF: ConstStaticCell<[u8; 4 * 1024]> = ConstStaticCell::new([0; 4 * 1024]);
static SPI_BUF_A: ConstStaticCell<[u8; 1024]> = ConstStaticCell::new([0; 1024]);
static SPI_BUF_B: ConstStaticCell<[u8; 1024]> = ConstStaticCell::new([0; 1024]);
static OLED_FRAME: ConstStaticCell<MonoFrame> = ConstStaticCell::new(MonoFrame::new());
/// The keys share SPI2 and the DC line (KTD21).
static SPI_BUS: StaticCell<RefCell<display::SpiBus>> = StaticCell::new();
static KEY_DC: StaticCell<RefCell<Output<'static>>> = StaticCell::new();
/// The OLED and the MCP23017 share I²C0 (THI-89 KTD1).
static I2C_BUS: StaticCell<AsyncMutex<CriticalSectionRawMutex, I2cBus>> = StaticCell::new();
static EXPANDER: StaticCell<Expander> = StaticCell::new();

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
    let board = board::Board::new(esp_hal::init(config));
    board::init_heap();

    let timg0 = TimerGroup::new(board.timg0);
    esp_rtos::start(timg0.timer0, board.sw_interrupt);

    let rev = efuse::chip_revision();
    info!(
        "opendeck booting: chip {:?} rev v{}.{}, free heap {} B",
        board::chip(),
        rev.major,
        rev.minor,
        esp_alloc::HEAP.free()
    );

    let mut store = config_store::ConfigStore::new(FlashStorage::new(board.flash));
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

    // The keys' CS and RST and the five buttons are on the MCP23017, which must be up before
    // any panel is reset or selected (THI-89).
    let _backlight = board.backlight;
    // Async, like the PoC's OLED bus (KTD22), and shared with the expander; `i2c` explains
    // why no transfer is ever cancelled (THI-89 KTD1).
    let i2c = I2C_BUS.init(AsyncMutex::new(board.i2c));
    let expander = EXPANDER.init(Expander::new(i2c));
    if expander.init().await {
        expander.reset_panels().await;
    }
    let bus = SPI_BUS.init(RefCell::new(board.spi));
    let dc = KEY_DC.init(RefCell::new(board.dc));
    // A panel is set up only while the expander selects it; without the expander its CS
    // would float, so the keys stay dark (KTD6).
    let mut key_a = None;
    let mut key_b = None;
    for (key, slot, buffer, label) in [
        (Key::A, &mut key_a, SPI_BUF_A.take(), "KEY A"),
        (Key::B, &mut key_b, SPI_BUF_B.take(), "KEY B"),
    ] {
        if expander.present() && expander.select_with_retries(key).await {
            let mut panel = KeyDisplay::new(bus, SharedPin::new(dc), buffer);
            panel.test_pattern(label);
            expander.deselect().await;
            *slot = Some(panel);
        }
    }

    let mut oled = Oled::new(i2c, OLED_FRAME.take());
    oled.init().await;
    oled.test_pattern();
    if let Err(e) = oled.flush().await {
        warn!("OLED test pattern not shown: {e:?}");
    }

    // The GPIO handler also serves the P4-ETH's software encoder decoder, so it goes in
    // before the encoder starts.
    let mut io = board.io;
    inputs::install_gpio_handler(&mut io, board.reference);
    spawner.spawn(inputs::inputs_task(expander, board.intb).expect("spawn inputs task"));
    spawner.spawn(encoder::encoder_task(board.encoder.start()).expect("spawn encoder task"));

    spawner.spawn(console::console_task(board.console_rx, store).expect("spawn console task"));

    let mac = board::device_mac();
    let stack = board.network.start(spawner, mac).await;
    let session = Session::new(DeviceIds::from_mac(mac));

    Timer::after(TEST_PATTERN_TIME).await;
    link::run(
        stack,
        link::Displays {
            key_a,
            key_b,
            oled,
            expander,
        },
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
