//! I²C0, shared by the MCP23017 and the OLED (THI-89 KTD1 in
//! `docs/plans/2026-11-21-1733-feat-mcp23017-key-inputs-bench-plan.md`).
//!
//! The bus is async and shared through a mutex that each transfer holds only for itself, so
//! one device waits at most one transfer for the other (an OLED page is about 3 ms).
//!
//! Transfers are never cancelled by a software timeout. While esp-radio starts Wi-Fi it holds
//! the CPU long enough for a transfer to take well over 50 ms, and an esp-hal transfer
//! dropped mid-flight leaves SDA and SCL held low for every later one. The peripheral's own
//! bus timeout still ends a transfer on a dead bus, with an error.

use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::mutex::Mutex;
use esp_hal::Async;
use esp_hal::i2c::master::{Error as I2cError, I2c};

pub type I2cBus = I2c<'static, Async>;

pub type SharedI2c = &'static Mutex<CriticalSectionRawMutex, I2cBus>;

/// Writes `bytes` to `address`.
pub async fn write(i2c: SharedI2c, address: u8, bytes: &[u8]) -> Result<(), I2cError> {
    i2c.lock().await.write_async(address, bytes).await
}

/// Writes `bytes` to `address`, then reads `buffer.len()` bytes back.
pub async fn write_read(
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
