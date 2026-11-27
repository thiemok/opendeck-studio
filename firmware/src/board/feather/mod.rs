//! Adafruit Feather ESP32 V2: Wi-Fi, and the encoder counted by the pulse counter.
//!
//! Pins: `docs/notes/deck-wiring.md`, Feather ESP32 V2 column. A pin change updates that note
//! in the same commit.

mod wifi;

use super::Board;
use embassy_executor::Spawner;
use embassy_net::Stack;
use esp_hal::efuse::{self, InterfaceMacAddress};
use esp_hal::gpio::{Input, InputConfig, Io, Level, Output, OutputConfig, Pull};
use esp_hal::i2c::master::{Config as I2cConfig, I2c};
use esp_hal::pcnt::Pcnt;
use esp_hal::pcnt::channel::{CtrlMode, EdgeMode};
use esp_hal::pcnt::unit::Unit;
use esp_hal::peripherals::{Peripherals, WIFI};
use esp_hal::spi::Mode;
use esp_hal::spi::master::{Config as SpiConfig, Spi};
use esp_hal::time::Rate;
use esp_hal::uart::{Config as UartConfig, UartRx};

/// What the displays show until Wi-Fi has an address (THI-84 KTD11).
pub const NO_NETWORK: &str = "No Wi-Fi";
/// Pin names for log text.
pub const INTB_PIN: &str = "GPIO37";
/// `1` if the encoder count rises when turning clockwise, `-1` if it falls.
pub const CLOCKWISE_SIGN: i32 = -1;

/// The chip as the boot log reports it, e.g. `Esp32Picov302`.
pub fn chip() -> impl core::fmt::Debug {
    efuse::chip_type()
}

/// The heap: the bootloader's reclaimed DRAM plus a smaller block of main DRAM.
pub fn init_heap() {
    esp_alloc::heap_allocator!(#[esp_hal::ram(reclaimed)] size: 98768);
    esp_alloc::heap_allocator!(size: 36 * 1024);
}

/// The Wi-Fi station MAC, which Companion remembers the surface by.
pub fn device_mac() -> [u8; 6] {
    efuse::interface_mac_address(InterfaceMacAddress::Station)
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
        .with_sda(p.GPIO22)
        .with_scl(p.GPIO20)
        .into_async();
        let spi = Spi::new(
            p.SPI2,
            SpiConfig::default()
                .with_frequency(Rate::from_mhz(20))
                .with_mode(Mode::_0),
        )
        .expect("SPI config")
        .with_sck(p.GPIO5)
        .with_mosi(p.GPIO19);
        let console_rx = UartRx::new(p.UART0, UartConfig::default().with_baudrate(115_200))
            .expect("UART0 config")
            .with_rx(p.GPIO3)
            .into_async();
        Self {
            timg0: p.TIMG0,
            sw_interrupt: p.FROM_CPU_INTR0,
            flash: p.FLASH,
            io: Io::new(p.IO_MUX),
            backlight: Output::new(p.GPIO14, Level::High, OutputConfig::default()),
            i2c,
            spi,
            dc: Output::new(p.GPIO27, Level::Low, OutputConfig::default()),
            // INTB is push-pull and needs no pull-up (THI-89 KTD4).
            intb: Input::new(p.GPIO37, module_input),
            // With nothing on it, the pull-up keeps it quiet (THI-89 KTD5).
            reference: Input::new(p.GPIO32, InputConfig::default().with_pull(Pull::Up)),
            encoder: Encoder {
                pcnt: Pcnt::new(p.PCNT),
                phase_a: Input::new(p.GPIO34, module_input),
                phase_b: Input::new(p.GPIO4, module_input),
            },
            console_rx,
            network: Network { wifi: p.WIFI },
        }
    }
}

/// The Feather's radio.
pub struct Network {
    wifi: WIFI<'static>,
}

impl Network {
    pub async fn start(self, spawner: Spawner, _mac: [u8; 6]) -> Stack<'static> {
        wifi::start(spawner, self.wifi)
    }
}

/// The encoder phases and the pulse counter that decodes them.
pub struct Encoder {
    pcnt: Pcnt<'static>,
    phase_a: Input<'static>,
    phase_b: Input<'static>,
}

/// 10 µs at the 80 MHz APB clock: ignores contact chatter and the ESP32's short input
/// glitches, yet passes every edge of a fast spin.
const FILTER_CYCLES: u16 = 800;

impl Encoder {
    /// Configures PCNT unit 0 to decode the two phases in quadrature and starts counting.
    pub fn start(self) -> EncoderCount {
        let unit = self.pcnt.unit0;
        let limit = crate::encoder::LIMIT;
        let _ = unit.set_low_limit(Some(-limit));
        let _ = unit.set_high_limit(Some(limit));
        let _ = unit.set_filter(Some(FILTER_CYCLES));
        unit.clear();

        let a = self.phase_a.peripheral_input();
        let b = self.phase_b.peripheral_input();
        let ch0 = &unit.channel0;
        ch0.set_ctrl_signal(a.clone());
        ch0.set_edge_signal(b.clone());
        ch0.set_ctrl_mode(CtrlMode::Reverse, CtrlMode::Keep);
        ch0.set_input_mode(EdgeMode::Increment, EdgeMode::Decrement);
        let ch1 = &unit.channel1;
        ch1.set_ctrl_signal(b);
        ch1.set_edge_signal(a);
        ch1.set_ctrl_mode(CtrlMode::Reverse, CtrlMode::Keep);
        ch1.set_input_mode(EdgeMode::Decrement, EdgeMode::Increment);
        unit.resume();
        EncoderCount {
            unit,
            _phases: [self.phase_a, self.phase_b],
        }
    }
}

/// The running pulse counter. The phase inputs are held so they stay configured.
pub struct EncoderCount {
    unit: Unit<'static, 0>,
    _phases: [Input<'static>; 2],
}

impl EncoderCount {
    pub fn count(&self) -> i16 {
        self.unit.value()
    }
}

/// The pulse counter needs no GPIO interrupt; the shared handler calls this anyway.
pub fn on_gpio_interrupt() {}
