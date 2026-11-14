//! Pure deck logic shared by the firmware and tested on the host.
//!
//! Nothing here depends on esp or embassy crates. Time arrives as milliseconds
//! and flash through the `embedded-storage` traits, so `cargo test` runs it all
//! on a desktop toolchain.
#![cfg_attr(not(test), no_std)]

pub mod base64;
pub mod bitmap;
pub mod config;
pub mod console;
pub mod debounce;
pub mod line;
pub mod session;
