---
title: esp-bootloader-esp-idf's activate_next_partition re-selects the booted slot on blank OTA data
date: 2026-11-27
category: runtime-errors
module: firmware (config_store, OTA slot switching)
problem_type: logic_error
component: tooling
symptoms:
  - "After a normal espflash flash, `slot next` reports `selected Ota0; restarting` and the board boots ota_0 again"
  - "An A/B boot check reads as a failure although ota_1 holds a valid image"
root_cause: wrong_api
resolution_type: code_fix
severity: medium
retire_when: "esp-bootloader-esp-idf's next_ota_part treats blank OTA data as 'ota_0 is current' and picks ota_1 when ota_0 is booted; check by calling activate_next_partition on a freshly flashed board with ota_0 booted"
tags: [esp-bootloader-esp-idf, ota, otadata, espflash, a-b-boot, firmware]
---

# esp-bootloader-esp-idf's activate_next_partition re-selects the booted slot on blank OTA data

## Problem

The firmware's `slot next` console command (THI-84) switches the A/B app slot so the bench can prove both slots boot. It first called `OtaUpdater::activate_next_partition` from esp-bootloader-esp-idf 0.6.0. On a board flashed the usual way, that call selects the slot that is already running, so the reboot comes back on `ota_0` and the check looks like a failed A/B boot.

## Symptoms

- `slot next` prints `selected Ota0; restarting` right after a `cargo run` / `cargo p4` flash, and the board boots `ota_0` again.
- Nothing errors. The OTA data is written and the restart happens, so the result reads as "the bootloader ignored ota_1" rather than as a wrong selection.

## What Didn't Work

- Trusting the library's "next" to mean "the slot that didn't boot". It means "the slot after the one the OTA data selects", and blank OTA data selects nothing.
- Writing the same image into `ota_1` first does not help on its own: the selection still lands on `ota_0`.

The code review of the THI-84 branch caught this before any bench run, by reading the library source. The steps below were traced there, not measured on hardware.

## Solution

Choose the target from the partition that actually booted, then write the OTA data directly. Condensed from `ConfigStore::select_next_slot` in `firmware/src/config_store.rs`:

```rust
let booted = partitions::read_partition_table(&mut self.flash, &mut *table_buf)?
    .booted_partition()?
    .map(|p| p.partition_type());
let target = match booted {
    Some(PartitionType::App(AppPartitionSubType::Ota0)) => AppPartitionSubType::Ota1,
    Some(PartitionType::App(AppPartitionSubType::Ota1)) => AppPartitionSubType::Ota0,
    _ => return Err(partitions::Error::Invalid),
};
let mut ota = OtaUpdater::new(&mut self.flash, &mut table_buf)?;
let mut data = ota.ota_data()?;
data.set_current_app_partition(target)?;
data.set_current_ota_state(OtaImageState::Valid)?;
```

## Why This Works

espflash 4.7 writes the bootloader, partition table and app, but never the `otadata` partition, so a normally flashed board has blank OTA data. In esp-bootloader-esp-idf 0.6.0:

1. `Ota::current_app_partition` returns `Factory` when both OTA sequence numbers are uninitialized (`src/ota.rs:233-234`).
2. `OtaUpdater::next_ota_part` maps `Factory` to `Ota0` (`src/ota_updater.rs:86`).
3. `Ota0` is the booted slot, so it retries with `(current.ota_app_number() + 2) % ota_count` (`src/ota_updater.rs:95-97`).
4. `ota_app_number` is `*self as u8 - OTA_SUBTYPE_OFFSET` (`src/partitions.rs:523-524`). For `Factory` (0) that wraps to 240 in a release build, and 242 % 2 = 0, which is `Ota0` again. A dev build, which has overflow checks on, panics instead.

Our partition table has no factory partition, so the booted slot is always `ota_0` or `ota_1`. Picking the other one from the booted partition never goes near the library's `Factory` path.

## Prevention

- Wherever the firmware switches slots, including the real OTA flow planned in THI-75, derive the target from `booted_partition()` rather than from `activate_next_partition` or `next_partition`. `next_partition` shares `next_ota_part`, so an OTA writer that asks it where to write the new image gets the booted slot on a fresh board.
- When a bench check depends on OTA data, run it on a board in the state espflash leaves it, with blank OTA data, not only after an earlier switch has initialized it.
