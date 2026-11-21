//! The MCP23017 that carries the buttons, the key chip selects and the key reset, on the
//! I²C bus it shares with the OLED (THI-89: KTD1, KTD2, KTD6 in
//! `docs/plans/2026-11-21-1733-feat-mcp23017-key-inputs-bench-plan.md`).
//!
//! The bus is async and shared through a mutex that each transfer holds only for itself, so
//! key draws switch CS around the draw rather than from inside it. The register bytes come
//! from [`deck_core::expander`].
//!
//! Transfers are never cancelled by a software timeout. While esp-radio starts Wi-Fi it holds
//! the CPU long enough for a transfer to take well over 50 ms, and an esp-hal transfer
//! dropped mid-flight leaves SDA and SCL held low for every later one. The peripheral's own
//! bus timeout still ends a transfer on a dead bus, with an error.

use core::cell::Cell;
use deck_core::expander::{self, BUTTON_COUNT, INIT, Key, PortA, reg};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::mutex::Mutex;
use embassy_time::Timer;
use esp_hal::Async;
use esp_hal::i2c::master::{Error as I2cError, I2c};
use log::{info, warn};

pub type I2cBus = I2c<'static, Async>;

/// I²C0, shared by the expander and the OLED.
pub type SharedI2c = &'static Mutex<CriticalSectionRawMutex, I2cBus>;

/// Writes `bytes` to `address` on the shared bus.
pub async fn write(i2c: SharedI2c, address: u8, bytes: &[u8]) -> Result<(), I2cError> {
    i2c.lock().await.write_async(address, bytes).await
}

async fn write_read(
    i2c: SharedI2c,
    address: u8,
    bytes: &[u8],
    buffer: &mut [u8],
) -> Result<(), I2cError> {
    i2c.lock()
        .await
        .write_read_async(address, bytes, buffer)
        .await
}

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
    pub async fn init(&self) -> bool {
        let mut result = Ok(());
        for &(register, value) in &INIT {
            result = self.write(register, value).await;
            if result.is_err() {
                break;
            }
        }
        if result.is_ok() {
            result = self.read(reg::GPIOB).await.map(drop);
        }
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
                true
            }
            Err(e) => {
                warn!("MCP23017 port A write failed: {e:?}");
                false
            }
        }
    }

    async fn write(&self, register: u8, value: u8) -> Result<(), I2cError> {
        write(self.i2c, expander::ADDRESS, &[register, value]).await
    }

    async fn read(&self, register: u8) -> Result<u8, I2cError> {
        let mut value = [0u8];
        write_read(self.i2c, expander::ADDRESS, &[register], &mut value).await?;
        Ok(value[0])
    }
}
