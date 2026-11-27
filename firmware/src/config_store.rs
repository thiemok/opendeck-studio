//! The `deckcfg` partition: load, save and clear the Companion host (KTD14). Also reads and
//! switches the A/B app slot for the bench's boot check (THI-84 KTD12).
//!
//! The record format and erase-then-write logic live in `deck_core::config::store`;
//! this only finds the partition and hands it over as a NOR flash.

use alloc::string::String;
use deck_core::config::{HostConfig, store};
use esp_bootloader_esp_idf::ota::OtaImageState;
use esp_bootloader_esp_idf::ota_updater::OtaUpdater;
use esp_bootloader_esp_idf::partitions::{self, AppPartitionSubType, PartitionType};
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

    /// The booted app partition's label, as the flash cache maps it.
    pub fn booted_slot(&mut self) -> Result<Option<String>, partitions::Error> {
        let mut table_buf = table_buffer();
        let table = partitions::read_partition_table(&mut self.flash, &mut *table_buf)?;
        Ok(table.booted_partition()?.map(|p| p.label_as_str().into()))
    }

    /// The app slot the OTA data selects for the next boot, and its image state.
    pub fn selected_slot(
        &mut self,
    ) -> Result<(AppPartitionSubType, OtaImageState), partitions::Error> {
        let mut table_buf = table_buffer();
        let mut ota = OtaUpdater::new(&mut self.flash, &mut table_buf)?;
        Ok((ota.selected_partition()?, ota.current_ota_state()?))
    }

    /// Selects the other app slot in the OTA data; the next boot runs it. Writes no image.
    ///
    /// The target is the slot that didn't boot. `OtaUpdater::activate_next_partition` picks it
    /// from the OTA data instead, and on blank OTA data (a board flashed by espflash, which
    /// never writes it) it selects the booted `ota_0` again.
    pub fn select_next_slot(&mut self) -> Result<AppPartitionSubType, partitions::Error> {
        let mut table_buf = table_buffer();
        let booted = partitions::read_partition_table(&mut self.flash, &mut *table_buf)?
            .booted_partition()?
            .map(|p| p.partition_type());
        let target = match booted {
            Some(PartitionType::App(AppPartitionSubType::Ota0)) => AppPartitionSubType::Ota1,
            Some(PartitionType::App(AppPartitionSubType::Ota1)) => AppPartitionSubType::Ota0,
            _ => return Err(partitions::Error::Invalid),
        };
        let mut table_buf = table_buffer();
        let mut ota = OtaUpdater::new(&mut self.flash, &mut table_buf)?;
        let mut data = ota.ota_data()?;
        data.set_current_app_partition(target)?;
        data.set_current_ota_state(OtaImageState::Valid)?;
        Ok(target)
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

/// A partition-table buffer on the heap; 3 KB is too much for the console task's stack.
fn table_buffer() -> alloc::boxed::Box<[u8; partitions::PARTITION_TABLE_MAX_LEN]> {
    alloc::vec![0u8; partitions::PARTITION_TABLE_MAX_LEN]
        .into_boxed_slice()
        .try_into()
        .unwrap_or_else(|_| unreachable!("the vec has the table's length"))
}
