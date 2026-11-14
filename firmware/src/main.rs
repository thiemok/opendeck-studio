#![no_std]
#![no_main]
#![deny(
    clippy::mem_forget,
    reason = "mem::forget is generally not safe to do with esp_hal types, especially those \
    holding buffers for the duration of a data transfer."
)]
#![deny(clippy::large_stack_frames)]

use embassy_executor::Spawner;
use embassy_time::{Duration, Timer};
use esp_backtrace as _;
use esp_bootloader_esp_idf::partitions;
use esp_hal::clock::CpuClock;
use esp_hal::timer::timg::TimerGroup;
use esp_storage::FlashStorage;
use log::{error, info};

extern crate alloc;

esp_bootloader_esp_idf::esp_app_desc!();

/// Label of the config partition declared in `partitions.csv` (KTD14).
const CONFIG_PARTITION: &str = "deckcfg";

#[esp_rtos::main]
async fn main(_spawner: Spawner) -> ! {
    esp_println::logger::init_logger_from_env();

    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(config);

    esp_alloc::heap_allocator!(#[esp_hal::ram(reclaimed)] size: 98768);

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0, peripherals.FROM_CPU_INTR0);

    let rev = esp_hal::efuse::chip_revision();
    info!(
        "opendeck booting: chip {:?} rev v{}.{}, free heap {} B",
        esp_hal::efuse::chip_type(),
        rev.major,
        rev.minor,
        esp_alloc::HEAP.free()
    );

    let mut flash = FlashStorage::new(peripherals.FLASH);
    log_config_partition(&mut flash);

    loop {
        Timer::after(Duration::from_secs(10)).await;
    }
}

/// Logs where the `deckcfg` partition is, or that it is missing.
fn log_config_partition(flash: &mut FlashStorage<'_>) {
    // On the heap: the template denies large stack frames, and this runs once at boot.
    let mut table_buf = alloc::vec![0u8; partitions::PARTITION_TABLE_MAX_LEN];
    let table = match partitions::read_partition_table(flash, &mut table_buf) {
        Ok(table) => table,
        Err(e) => {
            error!("partition table unreadable: {e:?}");
            return;
        }
    };
    match table.iter().find(|p| p.label_as_str() == CONFIG_PARTITION) {
        Some(p) => info!(
            "{CONFIG_PARTITION} found at {:#x}, {} KB",
            p.offset(),
            p.len() / 1024
        ),
        None => error!("{CONFIG_PARTITION} partition missing; flash with partitions.csv"),
    }
}
