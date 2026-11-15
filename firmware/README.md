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

## Wi-Fi credentials

The SSID and password are compiled in from `firmware/wifi.env`, which is gitignored:

```sh
cp firmware/wifi.env.example firmware/wifi.env   # then edit it
```

The access point must offer WPA2-Personal, or mixed WPA2/WPA3. esp-radio can't join a WPA3-only network. A missing file or key stops the build with a message that names it.

## Build and flash

```sh
cd firmware
mise exec -- cargo build --release
mise exec -- cargo run --release     # flash over USB and open the serial monitor
```

espflash picks up `partitions.csv` and the 8 MB flash size from `espflash.toml`. The layout reserves two 3 MB OTA slots plus a 64 KB `deckcfg` partition for the saved Companion host.

Serial access needs your user in the `dialout` group. On the Feather the USB-UART's DTR/RTS lines drive EN/IO0, so opening the port with some terminal programs resets the board.

## Checks

CI runs these three in `firmware/` on every PR (see the repo [README](../README.md#checks)):

```sh
cd firmware
mise exec -- cargo fmt --check
mise exec -- cargo build --release
mise exec -- cargo clippy --release -- -D warnings
```

CI has no real Wi-Fi credentials. It builds with a copy of `wifi.env.example` as `wifi.env`.

## What a healthy boot logs

```text
opendeck booting: chip Esp32Picov302 rev v3.x, free heap … B
deckcfg found at 0x610000, 64 KB
no host saved; set one with `host <ip-or-name>[:port]`
console ready; type `help`
Wi-Fi connected: …
```

A chip revision below v3 isn't supported by esp-hal.

For 3 s after boot the key shows a calibration test pattern. [`docs/notes/deck-wiring.md`](../docs/notes/deck-wiring.md) explains how to read it and how to check the switch.

## Serial console

Type into the serial monitor (`cargo run` opens it, or `espflash monitor`) at 115200 baud:

| Command | Effect |
|---|---|
| `host <ip-or-name>[:port]` | Saves the Companion host to flash and reconnects at once. The port defaults to 16622. |
| `host clear` | Forgets the host. The key shows "No host". |
| `show` | Prints the saved host. |
| `help` | Lists the commands. |

The host survives reboots and reflashing, because it lives in the `deckcfg` partition, not in the app image.

## What the key shows

| Key shows | Meaning |
|---|---|
| "No Wi-Fi" | Not joined, or no DHCP address yet |
| "No host" | Wi-Fi is up, but no Companion host is set |
| "Connecting" plus the host | Trying to reach and register with Companion; retries back off from 1 s to 5 s |
| Companion's image | Registered. Presses go to Companion. |

Presses while the key isn't registered are dropped, not sent later. For a test Companion, use the pinned container in `tools/fake-deck/` (`docker compose up`). The deck appears as one 128×128 key on button `1/0/1` (page 1, row 0, column 1).
