//! The USB serial console: set, clear and show the Companion host (KTD13).
//!
//! Only UART0's receiver is taken here; esp-println keeps transmitting through the
//! ROM routine at the same 115200 baud, so log output and replies share the port.

use crate::config_store::ConfigStore;
use crate::{HOST, HOST_CHANGED};
use deck_core::config::HostConfig;
use deck_core::console::{self, Command, ConsoleError};
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

fn run(text: &str, store: &mut ConfigStore) {
    match console::parse(text) {
        Ok(Command::SetHost { host, port }) => {
            // `parse` already validated the host, so `new` succeeds.
            let Some(config) = HostConfig::new(host, port) else {
                return;
            };
            match store.save(&config) {
                Ok(()) => {
                    HOST.lock(|h| h.set(Some(config)));
                    HOST_CHANGED.signal(());
                    println!("host set to {host}:{port}; reconnecting");
                }
                Err(e) => println!("could not save the host: {e:?}"),
            }
        }
        Ok(Command::ClearHost) => match store.clear() {
            Ok(()) => {
                HOST.lock(|h| h.set(None));
                HOST_CHANGED.signal(());
                println!("host cleared");
            }
            Err(e) => println!("could not clear the host: {e:?}"),
        },
        Ok(Command::Show) => match HOST.lock(|h| h.get()) {
            Some(config) => println!("host {}:{}", config.host(), config.port),
            None => println!("no host set"),
        },
        Ok(Command::Help) => println!("{}", console::HELP),
        Err(ConsoleError::Empty) => {}
        Err(e) => println!("{}", e.reason()),
    }
}
