//! The USB serial console: set, clear and show the Companion host (KTD13).
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
            let surface = SURFACE.lock(|s| s.get());
            set_surface(SurfaceConfig {
                layout,
                draw,
                ..surface
            });
            println!(
                "surface: {}; re-registering",
                describe(SURFACE.lock(|s| s.get()))
            );
        }
        Ok(Command::Image(format)) => {
            let surface = SURFACE.lock(|s| s.get());
            set_surface(SurfaceConfig { format, ..surface });
            println!(
                "surface: {}; re-registering",
                describe(SURFACE.lock(|s| s.get()))
            );
        }
        Ok(Command::Show) => {
            match HOST.lock(|h| h.get()) {
                Some(config) => println!("host {}:{}", config.host(), config.port),
                None => println!("no host set"),
            }
            println!("surface: {}", describe(SURFACE.lock(|s| s.get())));
        }
        Ok(Command::Help) => println!("{}", console::HELP),
        Err(ConsoleError::Empty) => {}
        Err(e) => println!("{}", e.reason()),
    }
}

/// The surface in console words, e.g. `bench 32 draw, png images`.
pub fn describe(surface: SurfaceConfig) -> &'static str {
    use deck_core::session::ImageFormat::{Png, Rgb};
    match (surface.layout, surface.draw, surface.format) {
        (Layout::Normal, _, Rgb) => "normal, rgb images",
        (Layout::Normal, _, Png) => "normal, png images",
        (Layout::Bench16, false, Rgb) => "bench 16, rgb images",
        (Layout::Bench16, false, Png) => "bench 16, png images",
        (Layout::Bench16, true, Rgb) => "bench 16 draw, rgb images",
        (Layout::Bench16, true, Png) => "bench 16 draw, png images",
        (Layout::Bench32, false, Rgb) => "bench 32, rgb images",
        (Layout::Bench32, false, Png) => "bench 32, png images",
        (Layout::Bench32, true, Rgb) => "bench 32 draw, rgb images",
        (Layout::Bench32, true, Png) => "bench 32 draw, png images",
    }
}
