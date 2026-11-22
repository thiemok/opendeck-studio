//! The MCP23017 that carries the buttons, the key chip selects and the key reset, on the
//! I²C bus it shares with the OLED (THI-89: KTD1, KTD2, KTD6 in
//! `docs/plans/2026-11-21-1733-feat-mcp23017-key-inputs-bench-plan.md`).
//!
//! It shares the bus in [`crate::i2c`] with the OLED. The register bytes come from
//! [`deck_core::expander`].

use crate::i2c::{self, SharedI2c};
use core::cell::Cell;
use deck_core::expander::{self, BUTTON_COUNT, INIT, Key, PortA, reg};
use embassy_time::Timer;
use esp_hal::i2c::master::Error as I2cError;
use log::{info, warn};

/// What [`INIT`] writes to port A's direction register. The chip resets to all inputs, so
/// reading anything else back means it lost power and needs [`Expander::reinit`].
const IODIRA_VALUE: u8 = {
    let mut i = 0;
    while INIT[i].0 != reg::IODIRA {
        i += 1;
    }
    INIT[i].1
};

pub struct Expander {
    i2c: SharedI2c,
    port_a: Cell<PortA>,
    present: Cell<bool>,
    /// A port-A write failed and no write has succeeded since; logs once per run.
    port_a_failing: Cell<bool>,
}

impl Expander {
    pub const fn new(i2c: SharedI2c) -> Self {
        Self {
            i2c,
            port_a: Cell::new(PortA::new()),
            present: Cell::new(false),
            port_a_failing: Cell::new(false),
        }
    }

    /// Configures the ports and clears any pending interrupt. Returns whether the expander
    /// answered; a missing one is logged and stays missing until the next boot (KTD6).
    pub async fn init(&self) -> bool {
        match self.configure().await {
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

    /// Writes the configuration again, keeping the current chip selects and reset, after
    /// the chip may have lost power (it then resets every register).
    pub async fn reinit(&self) {
        match self.configure().await {
            Ok(()) => info!("MCP23017 configured again"),
            Err(e) => warn!("MCP23017 configuration failed: {e:?}"),
        }
    }

    /// Whether the chip still holds the configuration; false after a power loss, or when
    /// the read fails.
    pub async fn configured(&self) -> bool {
        self.read(reg::IODIRA).await == Ok(IODIRA_VALUE)
    }

    /// [`INIT`], then the current port-A latch, then a port-B read to clear any pending
    /// interrupt.
    async fn configure(&self) -> Result<(), I2cError> {
        for &(register, value) in &INIT {
            self.write(register, value).await?;
        }
        self.write(reg::OLATA, self.port_a.get().byte()).await?;
        self.read(reg::GPIOB).await.map(drop)
    }

    /// Selects `key`'s panel alone. Returns false, and logs, if the write failed.
    pub async fn select(&self, key: Key) -> bool {
        self.update_port_a(|port| port.select(key)).await
    }

    pub async fn deselect(&self) {
        self.update_port_a(PortA::deselect).await;
    }

    /// Pulses the keys' shared RST so every panel starts from a hardware reset; it then
    /// stays high. Each panel's init follows with a software reset (PoC KTD21).
    pub async fn reset_panels(&self) {
        self.update_port_a(|port| port.reset(true)).await;
        Timer::after_micros(20).await;
        self.update_port_a(|port| port.reset(false)).await;
        Timer::after_millis(120).await;
    }

    /// The five buttons' pressed states, in [`crate::inputs::Button`] order. Reading also
    /// clears the expander's interrupt.
    pub async fn buttons(&self) -> Result<[bool; BUTTON_COUNT], I2cError> {
        self.read(reg::GPIOB).await.map(expander::pressed)
    }

    /// Applies `change` to the port-A latch and writes it; the latch keeps the change only
    /// if the write succeeded.
    async fn update_port_a(&self, change: impl FnOnce(&mut PortA) -> u8) -> bool {
        let mut port = self.port_a.get();
        let byte = change(&mut port);
        match self.write(reg::OLATA, byte).await {
            Ok(()) => {
                self.port_a.set(port);
                if self.port_a_failing.replace(false) {
                    info!("MCP23017 port A writes again");
                }
                true
            }
            Err(e) => {
                if !self.port_a_failing.replace(true) {
                    warn!("MCP23017 port A write failed: {e:?}; keys stay dark until it works");
                }
                false
            }
        }
    }

    async fn write(&self, register: u8, value: u8) -> Result<(), I2cError> {
        i2c::write(self.i2c, expander::ADDRESS, &[register, value]).await
    }

    async fn read(&self, register: u8) -> Result<u8, I2cError> {
        let mut value = [0u8];
        i2c::write_read(self.i2c, expander::ADDRESS, &[register], &mut value).await?;
        Ok(value[0])
    }
}
