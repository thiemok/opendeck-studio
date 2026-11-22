//! Pure deck logic shared by the firmware and tested on the host.
//!
//! Nothing here depends on esp or embassy crates. Time arrives as milliseconds
//! and flash through the `embedded-storage` traits, so `cargo test` runs it all
//! on a desktop toolchain.
#![cfg_attr(not(test), no_std)]

pub mod base64;
pub mod bitmap;
pub mod buttons;
pub mod config;
pub mod console;
pub mod crc32;
pub mod debounce;
pub mod encoder;
pub mod expander;
pub mod latency;
pub mod line;
pub mod mono;
pub mod pagetime;
pub mod png;
pub mod session;
