# Board and firmware framework decision (P1)

Decided on 2026-11-14 for R15 of the [PoC plan](../plans/2026-11-13-1722-feat-companion-network-deck-poc-plan.md). This is a spec-sheet decision with no bench tests. Both boards are on hand, so switching later is cheap. P2 (first key) is the first time the stack runs on hardware.

## Decision

- **Board: Adafruit Feather ESP32 V2** (ESP32-PICO-MINI-02, dual-core Xtensa at 240 MHz, 520 KB SRAM, **8 MB flash, 2 MB PSRAM**, USB-C via CH9102F).
- **Firmware: Rust no_std.** It uses `esp-hal` for peripherals, embassy for async, `esp-radio` for Wi-Fi and `esp-bootloader-esp-idf` for the A/B OTA slots. The Xtensa chip needs Espressif's Rust toolchain fork, which `espup` installs.
- **Fallback:** Rust std (`esp-idf-svc`) on the same board. It uses the same ESP-IDF bootloader and partition layout, so a switch keeps the hardware and the OTA scheme.

## Key module: Waveshare 0.85" ScreenKey Module B

- **Panel:** a 0.85" IPS LCD, 128×128 pixels, 65K colours, with an **ST7735** controller on 4-wire SPI. It runs on 3.3 V.
- **Switch:** a mechanical key on the module PCB. "Module B" means black keycap with the switch; the LCD-only versions have no switch.
- **Control header (9 pins):**

| Pin | Function |
|---|---|
| VCC | 3.3 V power |
| GND | Ground |
| DIN | SPI MOSI |
| SCLK | SPI clock |
| CS | Chip select, active low |
| DC | Data/command select: low = command, high = data |
| RST | Reset, active low |
| PWM | Backlight, PWM-dimmable |
| KEY | Key switch output |

- **Sharing pins across keys:** each key needs its own CS and KEY line. SCLK, DIN, DC, RST and PWM can be shared when all keys use the same brightness.
- **Image size:** the native resolution is 128×128. At that size a `KEY-STATE` line is 65,536 B of base64, or 49,152 B as raw RGB (see [the P0 notes](satellite-protocol-firmware-notes.md#images)). Whether to request 128×128 or a smaller preset is decided when P2 is planned.
- **Still to check on the bench in P2:** the KEY output's polarity, whether it needs a pull-up, and the ST7735 panel's column/row offset. The datasheet does not give them.

Sources: [Waveshare ScreenKey wiki](https://docs.waveshare.com/0.85inch-ScreenKey-Module-W), [product page](https://www.waveshare.com/0.85inch-screenkey.htm).

## Board comparison

Both boards were judged on the R15 criteria. Boards that failed a gate (OTA, Satellite over Wi-Fi, driving an ST7735) were ruled out first. The rest were weighed on toolchain, Ethernet/PoE path and headroom for more keys.

| | Feather ESP32 V2 | Pico 2 W |
|---|---|---|
| OTA (gate) | ✅ ESP-IDF bootloader with A/B slots and rollback. Mature, and usable from Rust. | ⚠️ The RP2350 boot ROM supports A/B partitions, but there is no official SDK OTA flow, only third-party bootloaders and arduino-pico's LittleFS-staged OTA. |
| Satellite over Wi-Fi (gate) | ✅ | ✅ The CYW43 firmware takes about 300–400 KB of flash. |
| ST7735 over SPI (gate) | ✅ | ✅ |
| RAM for a 65 KB image line | 520 KB SRAM plus **2 MB PSRAM**, so line buffers fit comfortably. | 520 KB SRAM with no PSRAM. Enough, but tighter at scale. |
| Wired Ethernet/PoE path | **Native EMAC** with an RMII PHY (LAN8720), on the same chip family as ready-made PoE boards such as the Olimex ESP32-POE2 (802.3at, 8 MB PSRAM) and ESP32-POE-ISO. | W5500 over SPI, e.g. W5500-EVB-Pico2 plus the WIZPoE-P1 module. GPIO16–21 go to the W5500, and that board has no Wi-Fi. |
| Pins for more keys | See [headroom](#headroom-for-more-keys). Input-only pins limit the choice. | 26 GPIOs |
| Toolchain feel | The Xtensa Rust fork via `espup`, `espflash` over USB-UART. JTAG needs an external adapter. | UF2 drag-and-drop and a cheap SWD Debug Probe. Upstream Rust targets it (`embassy-rp`). |

**Why the ESP32:** it is the only board that clearly passes the OTA gate. It also has the clearest PoE path: a PoE board with the same chip runs the same firmware, and PSRAM removes the image-buffer worry. The Pico 2 W is more comfortable to debug, but that doesn't outweigh weaker OTA and a W5500-only wired path.

## Framework comparison (on the Feather ESP32 V2)

| | Rust no_std (`esp-hal` + embassy) ✅ | Rust std (`esp-idf-svc`) | Arduino-ESP32 3.x on PlatformIO | ESP-IDF (C) | Zephyr |
|---|---|---|---|---|---|
| OTA | A/B slots via `esp-bootloader-esp-idf` (`ota_updater`). Rollback and upload endpoint are our code. | IDF's OTA with rollback. Upload endpoint is our code. | ArduinoOTA with a password, which matches R13. | IDF's OTA with rollback. Upload endpoint is our code. | MCUboot, which needs extra setup |
| Wi-Fi | `esp-radio`: pre-1.0, the API changes between releases. It wraps the same Wi-Fi blobs ESP-IDF uses. | IDF stack | IDF stack | IDF stack | Works, less proven on classic ESP32 |
| Async | embassy throughout, including async SPI and GPIO | embassy executor/time/sync or tokio; peripherals mostly blocking | FreeRTOS tasks and callbacks | FreeRTOS tasks | Zephyr threads |
| Ethernet later | Community EMAC crates, or W5500 via `embassy-net-wiznet` | Built into IDF | `ETH.h` | Built in | ESP32 Ethernet driver |
| Maintenance | Espressif's funded Rust team | Community ("little to no paid developer time") | The official PlatformIO platform is stuck on Arduino 2.x, so 3.x needs the community pioarduino fork | Espressif | Espressif + Zephyr project |
| Build | Pure Rust, fast | Compiles ESP-IDF via CMake, so the first build is slow and uses GBs of disk | Fast | CMake + menuconfig | west + devicetree |

**Why Rust no_std:** pure Rust with async from the network down to the display, fast builds, and the stack Espressif itself funds.

**Accepted costs:**
- **API churn.** esp-hal 1.0 froze a core API. Everything behind its `unstable` feature, and all of `esp-radio`, can change in a minor release. Pin crate versions and budget migration time for upgrades. "Unstable" here means API churn, not unreliable code.
- **Our own OTA rollback and upload.** The firmware must mark a new image valid only after it reaches Companion, and otherwise roll back. The password-checked upload endpoint for R13 is also our code. Both belong to P4.
- **Ethernet comes from community crates.** See below.
- **Classic ESP32 is not the ecosystem's focus.** Most attention goes to the C3/C6/S3, so fixes for classic ESP32 may arrive later.

**Why the others lost:**
- **Rust std:** mature underneath, but the bindings are community-maintained and trail new IDF releases, the C build is slow, and peripherals are mostly blocking.
- **Arduino on PlatformIO:** the quickest start, but no async model, and Arduino 3.x depends on a community fork of PlatformIO's platform.
- **ESP-IDF in C:** the lowest risk, but the most boilerplate, and C instead of Rust.
- **Zephyr:** devicetree and west are a lot of overhead for a single-board project, and its classic ESP32 Wi-Fi is less proven.

## Wired Ethernet/PoE path (later stages)

> **Update, 2026-11-27 (THI-84):** the wired board is now the Waveshare ESP32-P4-ETH, not an ESP32 PoE board. The Olimex ESP32-POE2 doesn't fit under the side section, and the P4-ETH has more free pins. It uses esp-hal's own Ethernet driver, which supports the ESP32-P4 in the esp-hal version already pinned, so none of the routes below is needed. The firmware builds for both boards ([plan](../plans/2026-11-27-1330-feat-esp32-p4-eth-port-plan.md), [bench note](p4-eth-bench.md)).

Once the final shape is decided, these are the routes in order of preference:

1. **Native EMAC + LAN8720** on an ESP32 PoE board such as the Olimex ESP32-POE2 or ESP32-POE-ISO (WROVER variants have PSRAM). Use the community crates [`esp-emac`](https://github.com/jethub-iot/esp-emac-rs) + [`eth-phy-lan87xx`](https://github.com/jethub-iot/eth-phy-rs), which target esp-hal 1.1 / embassy-net 0.9 and include a DHCP example, or [`ph-esp32-mac`](https://github.com/photon-circus/ph-esp32-mac), which is tested on a WT32-ETH01. esp-hal lists in-tree Ethernet for the ESP32 as work in progress.
2. **W5500 over SPI** with `embassy-net-wiznet`, which is part of embassy and works on any SPI bus. WIZnet PoE modules are available.
3. **Our own EMAC driver**, only if neither route works. It needs DMA descriptor rings, the 50 MHz RMII clock setup, PHY setup over MDIO and the `embassy-net-driver` trait. That is roughly 1–2k lines and a few weeks of work, using ESP-IDF's C driver and the crates above as references. Patching an existing crate is days of work.

## Headroom for more keys

- **RAM:** the 2 MB PSRAM can serve as heap for Satellite line buffers and frame buffers. The ST7735 holds its own frame, so the firmware only needs memory for the image currently arriving, not one buffer per key. RAM is not the limit at ~32 keys.
- **Pins on the Feather V2 headers:**
  - SPI is SCK 5, MOSI 19 and MISO 21. I²C is SDA 22 and SCL 20, which the OLED will use.
  - Output-capable pins free for CS/DC/RST/backlight are 4, 12, 13, 14, 15, 25, 26, 27, 32, 33, 7 and 8. That's 12 pins; 12 is a strapping pin that must be low at boot, and 13 drives the red LED.
  - Input-only pins 34, 36, 37 and 39 suit key switches but have **no internal pull-ups**.
  - GPIO 0 is the NeoPixel, 2 powers NeoPixel/STEMMA, 35 is the battery monitor and 38 is the on-board button.
- **Two keys (PoC):** 2 CS + 2 KEY + shared DC, RST and backlight = 7 pins. That fits easily.
- **~32 keys:** these need chip-select decoding (e.g. 74HC138 or shift registers) and an I/O expander for the key inputs (e.g. MCP23017 on the shared I²C bus). The same is true on the Pico 2 W, so pin count doesn't separate the boards. Whether to use per-key displays or a shared panel is still decided in P4 (R18).

## Sources

- Boards: [Adafruit Feather ESP32 V2 overview](https://learn.adafruit.com/adafruit-esp32-feather-v2/overview) and [pinouts](https://learn.adafruit.com/adafruit-esp32-feather-v2/pinouts), [W5500-EVB-Pico2](https://docs.wiznet.io/Product/Chip/Ethernet/W5500/w5500-evb-pico2), [Olimex ESP32-POE2](https://www.olimex.com/Products/IoT/ESP32/ESP32-POE2/), [ESP32-POE-ISO](https://olimex.com/Products/IoT/ESP32/ESP32-POE-ISO)
- Pico OTA: [arduino-pico OTA](https://arduino-pico.readthedocs.io/en/latest/ota.html), [Sming RP2350 A/B partitions](https://sming.readthedocs.io/en/latest/_inc/Sming/Arch/Rp2040/README.html), [picowota](https://github.com/xythobuz/picowota)
- Rust no_std: [esp-hal 1.0 announcement](https://developer.espressif.com/blog/2025/10/esp-hal-1/), [esp-hal releases](https://github.com/esp-rs/esp-hal/releases), [esp-radio docs milestone](https://github.com/esp-rs/esp-hal/issues/4448), [Rust book: OTA](https://docs.espressif.com/projects/rust/book/application-development/ota.html), [`ota_updater`](https://docs.espressif.com/projects/rust/esp-bootloader-esp-idf/0.6.0/esp32/esp_bootloader_esp_idf/ota_updater/index.html), [ancillary crates](https://docs.espressif.com/projects/rust/book/introduction/ancillary-crates.html)
- Rust std: [esp-idf-svc](https://docs.rs/esp-idf-svc/), [esp-idf-hal](https://github.com/esp-rs/esp-idf-hal)
- PlatformIO and Arduino 3.x: [PlatformIO forum thread](https://community.platformio.org/t/using-espressif-arduino-esp32-v3-2-0-with-platformio/47629), [pioarduino version table](https://github.com/sivar2311/platform-espressif32-versions)
- Zephyr: [Espressif Zephyr support status](https://developer.espressif.com/software/zephyr-support-status/)
