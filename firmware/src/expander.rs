//! The MCP23017 that carries the buttons, the key chip selects and the key reset, on the
//! I²C bus it shares with the OLED (THI-89: KTD1, KTD2, KTD6 in
//! `docs/plans/2026-11-21-1733-feat-mcp23017-key-inputs-bench-plan.md`).
//!
//! The bus is blocking and every user borrows it only for one synchronous transfer, so
//! nothing ever awaits while holding it: key draws run synchronously inside the link's line
//! callback and must switch CS from there. The register bytes come from
//! [`deck_core::expander`].

use core::cell::{Cell, RefCell};
use deck_core::expander::{self, BUTTON_COUNT, INIT, Key, PortA, reg};
use esp_hal::Blocking;
use esp_hal::delay::Delay;
use esp_hal::i2c::master::{Error as I2cError, I2c};
use log::{info, warn};

pub type I2cBus = I2c<'static, Blocking>;

/// I²C0, shared by the expander and the OLED. A borrow never spans an `.await`.
pub type SharedI2c = &'static RefCell<I2cBus>;

pub struct Expander {
    i2c: SharedI2c,
    port_a: Cell<PortA>,
    present: Cell<bool>,
}

impl Expander {
    pub const fn new(i2c: SharedI2c) -> Self {
        Self {
            i2c,
            port_a: Cell::new(PortA::new()),
            present: Cell::new(false),
        }
    }

    /// Configures the ports and clears any pending interrupt. Returns whether the expander
    /// answered; a missing one is logged and stays missing until the next boot (KTD6).
    pub fn init(&self) -> bool {
        let result = INIT
            .iter()
            .try_for_each(|&(register, value)| self.write(register, value))
            .and_then(|()| self.read(reg::GPIOB).map(drop));
        match result {
            Ok(()) => {
                info!("MCP23017 ready at {:#04x}", expander::ADDRESS);
                self.present.set(true);
            }
            Err(e) => warn!(
                "MCP23017 not found at {:#04x}: {e:?}; keys and buttons are off",
                expander::ADDRESS
            ),
        }
        self.present.get()
    }

    pub fn present(&self) -> bool {
        self.present.get()
    }

    /// Selects `key`'s panel alone. Returns false, and logs, if the write failed.
    pub fn select(&self, key: Key) -> bool {
        let mut port = self.port_a.get();
        let byte = port.select(key);
        self.set_port_a(port, byte)
    }

    pub fn deselect(&self) {
        let mut port = self.port_a.get();
        let byte = port.deselect();
        self.set_port_a(port, byte);
    }

    /// Pulses the keys' shared RST so every panel starts from a hardware reset; it then
    /// stays high. Each panel's init follows with a software reset (PoC KTD21).
    pub fn reset_panels(&self) {
        let delay = Delay::new();
        let mut port = self.port_a.get();
        let byte = port.reset(true);
        self.set_port_a(port, byte);
        delay.delay_micros(20);
        let byte = port.reset(false);
        self.set_port_a(port, byte);
        delay.delay_millis(120);
    }

    /// The five buttons' pressed states, in [`crate::inputs::Button`] order. Reading also
    /// clears the expander's interrupt.
    pub fn buttons(&self) -> Result<[bool; BUTTON_COUNT], I2cError> {
        self.read(reg::GPIOB).map(expander::pressed)
    }

    fn set_port_a(&self, port: PortA, byte: u8) -> bool {
        match self.write(reg::OLATA, byte) {
            Ok(()) => {
                self.port_a.set(port);
                true
            }
            Err(e) => {
                warn!("MCP23017 port A write failed: {e:?}");
                false
            }
        }
    }

    fn write(&self, register: u8, value: u8) -> Result<(), I2cError> {
        self.i2c
            .borrow_mut()
            .write(expander::ADDRESS, &[register, value])
    }

    fn read(&self, register: u8) -> Result<u8, I2cError> {
        let mut value = [0u8];
        self.i2c
            .borrow_mut()
            .write_read(expander::ADDRESS, &[register], &mut value)?;
        Ok(value[0])
    }
}
