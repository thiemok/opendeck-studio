//! The USB serial console: set, clear and show the Companion host (KTD13), and check A/B boot
//! (THI-84 KTD12).
//!
//! Only UART0's receiver is taken here; esp-println keeps transmitting through the
//! ROM routine at the same 115200 baud, so log output and replies share the port.

use crate::config_store::ConfigStore;
use crate::{HOST, SURFACE, set_host, set_surface};
use deck_core::config::HostConfig;
use deck_core::console::{self, Command, ConsoleError};
use deck_core::session::{Layout, SurfaceConfig};
use embassy_time::{Duration, Timer};
use esp_hal::Async;
use esp_hal::uart::UartRx;
use esp_println::{print, println};

const MAX_LINE: usize = 96;

#[embassy_executor::task]
pub async fn console_task(mut rx: UartRx<'static, Async>, mut store: ConfigStore) {
    let mut line = [0u8; MAX_LINE];
    let mut len = 0;
    let mut chunk = [0u8; 16];
    println!("console ready; type `help`");
    loop {
        let Ok(n) = rx.read_async(&mut chunk).await else {
            // Back off so a persistent UART error can't starve the other tasks.
            Timer::after(Duration::from_millis(10)).await;
            continue;
        };
        for &byte in &chunk[..n] {
            match byte {
                b'\r' | b'\n' => {
                    println!();
                    if let Ok(text) = core::str::from_utf8(&line[..len]) {
                        run(text, &mut store);
                    }
                    len = 0;
                }
                0x08 | 0x7f if len > 0 => {
                    len -= 1;
                    print!("\x08 \x08");
                }
                b' '..=b'~' if len < MAX_LINE => {
                    line[len] = byte;
                    len += 1;
                    print!("{}", byte as char);
                }
                _ => {}
            }
        }
    }
}

#[allow(
    clippy::large_stack_frames,
    reason = "about 1 KB, mostly reply formatting; runs on the console task's stack"
)]
fn run(text: &str, store: &mut ConfigStore) {
    match console::parse(text) {
        Ok(Command::SetHost { host, port }) => {
            let Some(config) = HostConfig::new(host, port) else {
                return;
            };
            match store.save(&config) {
                Ok(()) => {
                    set_host(Some(config));
                    println!("host set to {host}:{port}; reconnecting");
                }
                Err(e) => println!("could not save the host: {e:?}"),
            }
        }
        Ok(Command::ClearHost) => match store.clear() {
            Ok(()) => {
                set_host(None);
                println!("host cleared");
            }
            Err(e) => println!("could not clear the host: {e:?}"),
        },
        Ok(Command::Bench { layout, draw }) => {
            set_surface(SurfaceConfig { layout, draw });
            println!(
                "surface: {}; re-registering",
                Describe(SURFACE.lock(|s| s.get()))
            );
        }
        Ok(Command::Show) => {
            match HOST.lock(|h| h.get()) {
                Some(config) => println!("host {}:{}", config.host(), config.port),
                None => println!("no host set"),
            }
            println!("surface: {}", Describe(SURFACE.lock(|s| s.get())));
        }
        Ok(Command::Slot) => {
            match store.booted_slot() {
                Ok(Some(label)) => println!("booted {label}"),
                Ok(None) => println!("booted slot unknown"),
                Err(e) => println!("could not read the booted slot: {e:?}"),
            }
            match store.selected_slot() {
                Ok((slot, state)) => println!("next boot {slot:?} ({state:?})"),
                Err(e) => println!("no OTA selection: {e:?}"),
            }
            let app = &crate::ESP_APP_DESC;
            println!(
                "image {} built {} {}",
                app.version(),
                app.date(),
                app.time()
            );
        }
        Ok(Command::SlotNext) => match store.select_next_slot() {
            Ok(slot) => {
                println!("selected {slot:?}; restarting");
                esp_hal::system::software_reset();
            }
            Err(e) => println!("could not switch slots: {e:?}"),
        },
        Ok(Command::Help) => println!("{}", console::HELP),
        Err(ConsoleError::Empty) => {}
        Err(e) => println!("{}", e.reason()),
    }
}

/// The surface in console words, e.g. `bench 32 draw`.
pub struct Describe(pub SurfaceConfig);

impl core::fmt::Display for Describe {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let SurfaceConfig { layout, draw } = self.0;
        match layout {
            Layout::Normal => f.write_str("normal"),
            _ if draw => write!(f, "bench {} draw", layout.bench_keys()),
            _ => write!(f, "bench {}", layout.bench_keys()),
        }
    }
}
