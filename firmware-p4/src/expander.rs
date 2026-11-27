//! The MCP23017 that carries the buttons, the key chip selects and the key reset, on the
//! I²C bus it shares with the OLED (THI-89: KTD1, KTD2, KTD6 in
//! `docs/plans/2026-11-21-1733-feat-mcp23017-key-inputs-bench-plan.md`).
//!
//! It shares the bus in [`crate::i2c`] with the OLED. The register bytes come from
//! [`deck_core::expander`].

use crate::i2c::{self, SharedI2c};
use core::cell::Cell;
use deck_core::expander::{self, BUTTON_COUNT, INIT, IODIRA_INIT, Key, PortA, reg};
use embassy_time::Timer;
use esp_hal::i2c::master::Error as I2cError;
use log::{info, warn};

/// Tries for the port-A writes at boot that nothing repeats later.
const BOOT_WRITE_TRIES: u32 = 3;

pub struct Expander {
    i2c: SharedI2c,
    port_a: Cell<PortA>,
    present: Cell<bool>,
    /// A port-A write failed and no write has succeeded since; logs once per run.
    port_a_failing: Cell<bool>,
    /// A reconfiguration failed and none has succeeded since; logs once per run.
    configure_failing: Cell<bool>,
}

impl Expander {
    pub const fn new(i2c: SharedI2c) -> Self {
        Self {
            i2c,
            port_a: Cell::new(PortA::new()),
            present: Cell::new(false),
            port_a_failing: Cell::new(false),
            configure_failing: Cell::new(false),
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
            Ok(()) => {
                self.configure_failing.set(false);
                info!("MCP23017 configured again");
            }
            Err(e) => {
                if !self.configure_failing.replace(true) {
                    warn!("MCP23017 configuration failed: {e:?}");
                }
            }
        }
    }

    /// Reconfigures a chip that answers but lost its configuration to a power loss. One
    /// that doesn't answer is left to the button reads, which log the failure once.
    pub async fn restore_if_reset(&self) {
        if let Ok(iodira) = self.read(reg::IODIRA).await
            && iodira != IODIRA_INIT
        {
            self.reinit().await;
        }
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

    /// [`Self::select`] for a panel's init at boot: retried, so one transient bus error
    /// doesn't leave the panel dark until the next reboot.
    pub async fn select_with_retries(&self, key: Key) -> bool {
        self.update_port_a_with_retries(|port| port.select(key))
            .await
    }

    pub async fn deselect(&self) {
        self.update_port_a(PortA::deselect).await;
    }

    /// Pulses the keys' shared RST so every panel starts from a hardware reset; it then
    /// stays high. Each panel's init follows with a software reset (PoC KTD21). The release
    /// is retried: nothing releases RST again later in the run.
    pub async fn reset_panels(&self) {
        self.update_port_a(|port| port.reset(true)).await;
        Timer::after_micros(20).await;
        if !self
            .update_port_a_with_retries(|port| port.reset(false))
            .await
        {
            warn!("key RST may still be low; the keys stay dark until the next boot");
        }
        Timer::after_millis(120).await;
    }

    /// The five buttons' pressed states, in [`expander::BUTTONS`] order. Reading also
    /// clears the expander's interrupt.
    pub async fn buttons(&self) -> Result<[bool; BUTTON_COUNT], I2cError> {
        self.read(reg::GPIOB).await.map(expander::pressed)
    }

    async fn update_port_a_with_retries(&self, change: impl Fn(&mut PortA) -> u8) -> bool {
        for _ in 0..BOOT_WRITE_TRIES {
            if self.update_port_a(&change).await {
                return true;
            }
            Timer::after_millis(10).await;
        }
        false
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
