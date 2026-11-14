# opendeck firmware

Rust `no_std` firmware for the Adafruit Feather ESP32 V2. It uses `esp-hal`, embassy (through `esp-rtos`) and `esp-radio`. The decisions behind it are in [the PoC plan](../docs/plans/2026-11-13-1722-feat-companion-network-deck-poc-plan.md), P2 (KTD9–KTD19).

Pure logic lives in [`crates/deck-core`](../crates/deck-core) and is tested on the desktop. This directory is a separate cargo root with its own target, toolchain and lockfile.

## Tools (mise)

Every tool is pinned in the repo's `mise.toml` files. Install them once from the repo root:

```sh
mise trust && mise trust firmware/mise.toml
mise install                # stable Rust, espup, espflash, esp-generate
mise run esp-toolchain      # espup's `esp` Xtensa toolchain + .esp/export-esp.sh
```

On this setup mise runs through shims, not shell activation, so run commands through `mise exec --` (or activate mise in your shell). Inside `firmware/`, `firmware/mise.toml` switches `RUSTUP_TOOLCHAIN` to `esp` and loads espup's environment. Check it with:

```sh
mise exec -- rustup show active-toolchain   # repo root: 1.100.0; firmware/: esp
```

## Build and flash

```sh
cd firmware
mise exec -- cargo build --release
mise exec -- cargo run --release     # flash over USB and open the serial monitor
```

espflash picks up `partitions.csv` and the 8 MB flash size from `espflash.toml`. The layout reserves two 3 MB OTA slots plus a 64 KB `deckcfg` partition for the saved Companion host.

Serial access needs your user in the `dialout` group. On the Feather the USB-UART's DTR/RTS lines drive EN/IO0, so opening the port with some terminal programs resets the board.

## What a healthy boot logs

```text
opendeck booting: chip Esp32Picov302 rev v3.x, free heap … B
deckcfg found at 0x610000, 64 KB
```

A chip revision below v3 isn't supported by esp-hal.
