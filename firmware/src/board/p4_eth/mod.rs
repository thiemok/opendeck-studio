//! Waveshare ESP32-P4-ETH: wired Ethernet, and the encoder decoded in the GPIO interrupt
//! (THI-84).
//!
//! Pins: `docs/notes/deck-wiring.md`, ESP32-P4-ETH column (plan KTD14). The bench parts sit on
//! free header pins, away from the RMII pins, the SD slot, the strapping pins and the console.

mod eth;

use super::Board;
use core::cell::RefCell;
use deck_core::quadrature::Quadrature;
use embassy_executor::Spawner;
use embassy_net::Stack;
use embassy_sync::blocking_mutex::Mutex;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_time::Instant;
use esp_hal::efuse;
use esp_hal::gpio::{Event, Input, InputConfig, Io, Level, Output, OutputConfig, Pull};
use esp_hal::i2c::master::{Config as I2cConfig, I2c};
use esp_hal::peripherals::{ETH, Peripherals};
use esp_hal::spi::Mode;
use esp_hal::spi::master::{Config as SpiConfig, Spi};
use esp_hal::time::Rate;
use esp_hal::uart::{Config as UartConfig, UartRx};

/// What the displays show until Ethernet has an address (THI-84 KTD11).
pub const NO_NETWORK: &str = "No Ethernet";
/// Pin names for log text.
pub const INTB_PIN: &str = "GPIO26";
/// `1` if the encoder count rises when turning clockwise, `-1` if it falls. Checked on the
/// bench (`docs/notes/p4-eth-bench.md`).
pub const CLOCKWISE_SIGN: i32 = -1;

/// Edges closer than this to the last accepted one are contact bounce. A fast spin of five
/// turns a second still leaves about 2.5 ms between edges.
const MIN_EDGE_INTERVAL_US: u64 = 200;

/// The chip as the boot log reports it.
pub fn chip() -> impl core::fmt::Debug {
    "ESP32-P4"
}

/// The heap. The P4's reclaimed bootloader RAM is 68 KB, and the main RAM has room for the
/// rest; the total is above the Feather's 132 KB, so the PNG decoder (44,676 B) fits as there.
pub fn init_heap() {
    esp_alloc::heap_allocator!(#[esp_hal::ram(reclaimed)] size: 64 * 1024);
    esp_alloc::heap_allocator!(size: 96 * 1024);
}

/// The factory base MAC: both the Ethernet MAC and the Companion device ID, so Companion
/// remembers the surface across reflashes (KTD9).
pub fn device_mac() -> [u8; 6] {
    efuse::base_mac_address()
        .as_bytes()
        .try_into()
        .expect("6-byte MAC")
}

impl Board {
    pub fn new(p: Peripherals) -> Self {
        let module_input = InputConfig::default();
        let i2c = I2c::new(
            p.I2C0,
            I2cConfig::default().with_frequency(Rate::from_khz(crate::oled::calibration::I2C_KHZ)),
        )
        .expect("I2C0 config")
        .with_sda(p.GPIO7)
        .with_scl(p.GPIO8)
        .into_async();
        let spi = Spi::new(
            p.SPI2,
            SpiConfig::default()
                .with_frequency(Rate::from_mhz(20))
                .with_mode(Mode::_0),
        )
        .expect("SPI config")
        .with_sck(p.GPIO20)
        .with_mosi(p.GPIO21);
        // UART0 behind the board's CH343P USB-UART bridge (KTD10).
        let console_rx = UartRx::new(p.UART0, UartConfig::default().with_baudrate(115_200))
            .expect("UART0 config")
            .with_rx(p.GPIO38)
            .into_async();
        Self {
            timg0: p.TIMG0,
            sw_interrupt: p.FROM_CPU_INTR0,
            flash: p.FLASH,
            io: Io::new(p.IO_MUX),
            backlight: Output::new(p.GPIO23, Level::High, OutputConfig::default()),
            i2c,
            spi,
            dc: Output::new(p.GPIO22, Level::Low, OutputConfig::default()),
            // INTB is push-pull and needs no pull-up (THI-89 KTD4).
            intb: Input::new(p.GPIO26, module_input),
            // With nothing on it, the pull-up keeps it quiet (THI-89 KTD5).
            reference: Input::new(p.GPIO27, InputConfig::default().with_pull(Pull::Up)),
            encoder: Encoder {
                phase_a: Input::new(p.GPIO32, module_input),
                phase_b: Input::new(p.GPIO33, module_input),
            },
            console_rx,
            network: Network {
                eth: p.ETH,
                pins: eth::Pins {
                    ref_clk: p.GPIO50,
                    rxd0: p.GPIO29,
                    rxd1: p.GPIO30,
                    crs_dv: p.GPIO28,
                    txd0: p.GPIO34,
                    txd1: p.GPIO35,
                    tx_en: p.GPIO49,
                    mdc: p.GPIO31,
                    mdio: p.GPIO52,
                    phy_reset: p.GPIO51,
                },
            },
        }
    }
}

/// The EMAC and the RMII pins to the IP101GRI PHY.
pub struct Network {
    eth: ETH<'static>,
    pins: eth::Pins,
}

impl Network {
    pub async fn start(self, spawner: Spawner, mac: [u8; 6]) -> Stack<'static> {
        eth::start(spawner, self.eth, self.pins, mac).await
    }
}

/// The encoder phases. esp-hal has no PCNT driver for the ESP32-P4, so the GPIO interrupt
/// reads both phases on every edge and steps a [`Quadrature`] decoder (KTD13). The interrupt
/// runs even while a blocking key draw stalls the executor, so no edge is lost.
pub struct Encoder {
    phase_a: Input<'static>,
    phase_b: Input<'static>,
}

struct Decoder {
    phases: [Input<'static>; 2],
    quadrature: Quadrature,
}

/// The phase inputs and their decoder, owned by the GPIO interrupt handler.
static DECODER: Mutex<CriticalSectionRawMutex, RefCell<Option<Decoder>>> =
    Mutex::new(RefCell::new(None));

impl Encoder {
    /// Starts decoding. The GPIO interrupt handler must already be installed
    /// ([`crate::inputs::install_reference`]); it calls [`on_gpio_interrupt`].
    pub fn start(self) -> EncoderCount {
        let Self {
            mut phase_a,
            mut phase_b,
        } = self;
        DECODER.lock(|slot| {
            let quadrature = Quadrature::new(
                phase_a.is_high(),
                phase_b.is_high(),
                crate::encoder::LIMIT,
                MIN_EDGE_INTERVAL_US,
            );
            phase_a.listen(Event::AnyEdge);
            phase_b.listen(Event::AnyEdge);
            slot.replace(Some(Decoder {
                phases: [phase_a, phase_b],
                quadrature,
            }));
        });
        EncoderCount
    }
}

/// Reads the decoder's count.
pub struct EncoderCount;

impl EncoderCount {
    pub fn count(&self) -> i16 {
        DECODER.lock(|slot| slot.borrow().as_ref().map_or(0, |d| d.quadrature.count()))
    }
}

/// Steps the decoder if either phase changed. Clears only the phases' interrupt status, so
/// other pins sharing the handler keep theirs.
pub fn on_gpio_interrupt() {
    DECODER.lock(|slot| {
        let mut slot = slot.borrow_mut();
        let Some(decoder) = slot.as_mut() else {
            return;
        };
        let [a, b] = &mut decoder.phases;
        if !a.is_interrupt_set() && !b.is_interrupt_set() {
            return;
        }
        a.clear_interrupt();
        b.clear_interrupt();
        let now = Instant::now().as_micros();
        decoder.quadrature.update(a.is_high(), b.is_high(), now);
    });
}
