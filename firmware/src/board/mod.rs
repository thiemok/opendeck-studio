//! The board layer: everything that knows which board the firmware runs on (THI-84 KTD3, KTD4).
//!
//! Exactly one of the Cargo features `board-feather` (the default) and `board-p4-eth` picks a
//! module here. Each provides the same items: [`Board`] with the board's configured buses and
//! pins, the network start, the device MAC, the encoder's count source and a few labels.
//! Nothing outside this module names a GPIO.

#[cfg(all(feature = "board-feather", feature = "board-p4-eth"))]
compile_error!("pick one board: `board-feather` or `board-p4-eth`, not both");
#[cfg(not(any(feature = "board-feather", feature = "board-p4-eth")))]
compile_error!("pick a board feature: `board-feather` or `board-p4-eth`");

#[cfg(feature = "board-feather")]
mod feather;
#[cfg(feature = "board-feather")]
pub use feather::*;

#[cfg(feature = "board-p4-eth")]
mod p4_eth;
#[cfg(feature = "board-p4-eth")]
pub use p4_eth::*;

use esp_hal::Async;
use esp_hal::Blocking;
use esp_hal::gpio::{Input, Io, Output};
use esp_hal::i2c::master::I2c;
use esp_hal::peripherals::{FLASH, FROM_CPU_INTR0, TIMG0};
use esp_hal::spi::master::Spi;
use esp_hal::uart::UartRx;

/// The board's peripherals, set up for the deck. Fields shared by every board are here;
/// [`Encoder`] and [`Network`] are the board's own.
pub struct Board {
    pub timg0: TIMG0<'static>,
    pub sw_interrupt: FROM_CPU_INTR0<'static>,
    pub flash: FLASH<'static>,
    pub io: Io<'static>,
    /// Key backlight, driven high (no brightness control yet, THI-81).
    pub backlight: Output<'static>,
    /// Shared by the MCP23017 and the OLED (THI-89 KTD1).
    pub i2c: I2c<'static, Async>,
    /// Shared by the keys, with one DC line (PoC plan KTD21).
    pub spi: Spi<'static, Blocking>,
    pub dc: Output<'static>,
    /// The MCP23017's interrupt line (THI-89 KTD4).
    pub intb: Input<'static>,
    /// The bench-only reference wire from key A's switch (THI-89 KTD5).
    pub reference: Input<'static>,
    pub encoder: Encoder,
    pub console_rx: UartRx<'static, Async>,
    pub network: Network,
}
