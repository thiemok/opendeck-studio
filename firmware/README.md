# opendeck firmware

Rust `no_std` firmware for two boards, from one codebase:

- **Adafruit Feather ESP32 V2**, on Wi-Fi. This is the default build.
- **Waveshare ESP32-P4-ETH**, on wired Ethernet and PoE. The ESP32-P4 has no radio.

It uses `esp-hal`, embassy (through `esp-rtos`), and `esp-radio` on the Feather. The decisions behind it are in [the PoC plan](../docs/plans/2026-11-13-1722-feat-companion-network-deck-poc-plan.md), P2 (KTD9–KTD19), and for the second board in [the THI-84 plan](../docs/plans/2026-11-27-1330-feat-esp32-p4-eth-port-plan.md).

A Cargo feature picks the board: `board-feather` (the default) or `board-p4-eth`. Everything board-specific lives in `src/board/`: pins, buses, the network, the device MAC and the encoder's count source. The rest of the firmware doesn't know which board it runs on.

Pure logic lives in [`crates/deck-core`](../crates/deck-core) and is tested on the desktop. This directory is a separate cargo root with its own targets, toolchain and lockfile.

## Tools (mise)

Every tool is pinned in the repo's `mise.toml` files. Install them once from the repo root:

```sh
mise trust && mise trust firmware/mise.toml
mise install                # stable Rust, espup, espflash, esp-generate
mise run esp-toolchain      # espup's `esp` toolchain + .esp/export-esp.sh (builds both boards)
```

On this setup mise runs through shims, not shell activation, so run commands through `mise exec --` (or activate mise in your shell). Inside `firmware/`, `firmware/mise.toml` switches `RUSTUP_TOOLCHAIN` to `esp` and loads espup's environment. Check it with:

```sh
mise exec -- rustup show active-toolchain   # repo root: 1.100.0; firmware/: esp
```

## Wi-Fi credentials (Feather)

The SSID and password are compiled in from `firmware/wifi.env`, which is gitignored:

```sh
cp firmware/wifi.env.example firmware/wifi.env   # then edit it
```

The access point must offer WPA2-Personal, or mixed WPA2/WPA3. esp-radio can't join a WPA3-only network. A missing file or key stops the Feather build with a message that names it. The ESP32-P4-ETH build doesn't need the file.

## Build and flash

Feather ESP32 V2:

```sh
cd firmware
mise exec -- cargo build --release
mise exec -- cargo run --release     # flash over USB and open the serial monitor
```

Waveshare ESP32-P4-ETH, through cargo aliases in `.cargo/config.toml` that add the RISC-V target and the `board-p4-eth` feature:

```sh
cd firmware
mise exec -- cargo p4-build
mise exec -- cargo p4                # flash over USB-C and open the serial monitor
```

espflash picks up `partitions.csv` and the 8 MB flash size from `espflash.toml`, for both boards. The layout reserves two 3 MB OTA slots plus a 64 KB `deckcfg` partition for the saved Companion host. The P4-ETH has 32 MB of flash; the build uses the first 8 MB.

Serial access needs your user in the `dialout` group. On both boards the USB-UART's DTR/RTS lines drive EN and the boot pin, so opening the port with some terminal programs resets the board. On the P4-ETH the USB-C port goes to a CH343P USB-UART bridge on UART0.

esp-hal supports chip revision v3 and later on both chips. The boot log prints the revision.

## Checks

CI runs these in `firmware/` on every PR (see the repo [README](../README.md#checks)), in its one `firmware` job:

```sh
cd firmware
mise exec -- cargo fmt --check
mise exec -- cargo build --release --locked
mise exec -- cargo clippy --release --locked -- -D warnings
mise exec -- cargo p4-build
mise exec -- cargo p4-clippy
```

CI has no real Wi-Fi credentials. It builds with a copy of `wifi.env.example` as `wifi.env`.

## What a healthy boot logs

On the Feather:

```text
opendeck booting: chip Esp32Picov302 rev v3.x, free heap … B
deckcfg found at 0x610000, 64 KB
no host saved; set one with `host <ip-or-name>[:port]`
console ready; type `help`
Wi-Fi connected: …
```

On the ESP32-P4-ETH, the first line names `"ESP32-P4"`, and the network lines are `Ethernet link up` and `DHCP address …`.

A chip revision below v3 isn't supported by esp-hal.

For 3 s after boot each key and the OLED show a calibration test pattern. [`docs/notes/deck-wiring.md`](../docs/notes/deck-wiring.md) explains how to read the keys' pattern and check the buttons, and [`docs/notes/oled-encoder-module.md`](../docs/notes/oled-encoder-module.md) covers the OLED and the encoder.

## Serial console

Type into the serial monitor (`cargo run` opens it, or `espflash monitor`) at 115200 baud:

| Command | Effect |
|---|---|
| `host <ip-or-name>[:port]` | Saves the Companion host to flash and reconnects at once. The port defaults to 16622. |
| `host clear` | Forgets the host. The displays show "No host". |
| `bench off` / `bench 16` / `bench 32` | Registers the normal surface, or a bench surface of 16 or 32 keys, and re-registers at once. Add `draw` (`bench 32 draw`) to also draw every key's image on a panel. |
| `show` | Prints the saved host and the surface. |
| `slot` | Prints the booted app slot, the slot the OTA data selects for the next boot, and the running image's version and build time. |
| `slot next` | Selects the other app slot and restarts. It writes no image, so flash the same build into both slots first. This is for checking A/B boot on the bench (THI-84). |
| `help` | Lists the commands. |

The host survives reboots and reflashing, because it lives in the `deckcfg` partition, not in the app image. `bench` is not saved: a reboot returns to the normal surface.

## What the displays show

The keys and the OLED show the same states.

| Displays show | Meaning |
|---|---|
| "No Wi-Fi" (Feather) or "No Ethernet" (ESP32-P4-ETH) | Not joined or no cable, or no DHCP address yet |
| "No host" | The network is up, but no Companion host is set |
| "Connecting" plus the host | Trying to reach and register with Companion; retries back off from 1 s to 5 s |
| Companion's images | Registered. Presses and encoder turns go to Companion. |

The deck asks Companion for PNG images whenever it offers them, and decodes them on the device; otherwise it takes raw `rgb`. PNG cuts the bytes per image about 11×, and page changes about 4× (THI-76). The decoder takes about 45 KB from the heap at the first PNG registration and logs the heap before and after.

Presses and encoder turns while the deck isn't registered are dropped, not sent later. For a test Companion, use the pinned container in `tools/fake-deck/` (`docker compose up`). The deck appears as one surface with three controls in row 0:

| Control | Companion button | Shows | Input |
|---|---|---|---|
| Key A | `1/0/1` | 128×128 image | press and release |
| Key B | `1/0/2` | 128×128 image | press and release |
| Encoder | `1/0/3` | 128×64 image on the OLED, reduced to black and white | push and release; turn the knob for the button's rotate actions (enable **Rotary actions** in the button's options) |

Design the encoder button white-on-black in Companion: mid-brightness colours turn black on the OLED. BACK and CONFIRM on the module are logged on the serial console and never sent to Companion.

The log prints a `round trip` line for each press or turn answered by a new image, and every 20 samples the median and maximum. That is the P3 responsiveness measurement.

## Bench surface (THI-76, THI-85)

`bench 16` or `bench 32` registers 16 or 32 keys in rows of 8 from `0/0`, with no encoder, to measure how long a page change takes. Key A is bench key `0/0` and key B is `0/1`; the bench pages in `tools/fake-deck/companion-config/` make them page up and page down. The other keys' images are decoded and timed but not shown, unless `draw` sends each one to panel A (even keys) or B (odd keys) as well. The OLED shows "BENCH" with the surface.

Each page change logs one `page change` line: keys drawn, format, the time from the press to the last key and from the first image byte to the last key, bytes on the wire, and the summed and largest receive and decode times. Every 10 page changes it logs the median and maximum press-to-last-key time and the free heap. Without pressing anything, `bench 32` again re-registers, and Companion's full redraw gives one `page change … press to last none` line with the receive and decode sums.

The procedure and the THI-76 results are in [the protocol notes](../docs/notes/satellite-protocol-firmware-notes.md#page-change-at-16-and-32-keys-thi-76).
