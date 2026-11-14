//! The `deckcfg` partition: load, save and clear the Companion host (KTD14).
//!
//! The record format and erase-then-write logic live in `deck_core::config::store`;
//! this only finds the partition and hands it over as a NOR flash.

use deck_core::config::{HostConfig, store};
use esp_bootloader_esp_idf::partitions;
use esp_storage::FlashStorage;

/// Label of the config partition declared in `partitions.csv`.
pub const PARTITION: &str = "deckcfg";

#[derive(Debug)]
#[allow(
    dead_code,
    reason = "the flash error is reported through Debug in console replies"
)]
pub enum StoreError {
    /// The partition table can't be read, or has no `deckcfg`.
    NoPartition,
    Flash(partitions::Error),
}

pub struct ConfigStore {
    flash: FlashStorage<'static>,
}

impl ConfigStore {
    pub fn new(flash: FlashStorage<'static>) -> Self {
        Self { flash }
    }

    /// Where `deckcfg` is, as `(offset, length)`.
    pub fn locate(&mut self) -> Option<(u32, u32)> {
        let mut table_buf = alloc::vec![0u8; partitions::PARTITION_TABLE_MAX_LEN];
        let table = partitions::read_partition_table(&mut self.flash, &mut table_buf).ok()?;
        let entry = table.iter().find(|p| p.label_as_str() == PARTITION)?;
        Some((entry.offset(), entry.len()))
    }

    pub fn load(&mut self) -> Option<HostConfig> {
        self.with_region(|nor| Ok(store::load(nor))).ok().flatten()
    }

    pub fn save(&mut self, config: &HostConfig) -> Result<(), StoreError> {
        self.with_region(|nor| store::save(nor, config))
    }

    pub fn clear(&mut self) -> Result<(), StoreError> {
        self.with_region(|nor| store::clear(nor))
    }

    /// Runs `f` on `deckcfg` as a NOR flash. The table is re-read each time; this
    /// only happens at boot and when the operator types a command.
    fn with_region<R>(
        &mut self,
        f: impl FnOnce(&mut partitions::NorFlashRegion<'_, '_, '_>) -> Result<R, partitions::Error>,
    ) -> Result<R, StoreError> {
        let mut table_buf = alloc::vec![0u8; partitions::PARTITION_TABLE_MAX_LEN];
        let table = partitions::read_partition_table(&mut self.flash, &mut table_buf)
            .map_err(|_| StoreError::NoPartition)?;
        let entry = table
            .iter()
            .find(|p| p.label_as_str() == PARTITION)
            .ok_or(StoreError::NoPartition)?;
        let mut region = entry.as_flash_region(&mut self.flash);
        let mut nor = region.as_nor_flash().map_err(StoreError::Flash)?;
        f(&mut nor).map_err(StoreError::Flash)
    }
}
