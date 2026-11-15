# Deck wiring and key calibration (P2)

How one Waveshare 0.85" ScreenKey Module B is wired to the Adafruit Feather ESP32 V2, and the panel values the firmware uses. This closes the three bench checks P1 left open in [the board decision](board-and-framework-decision.md): KEY polarity, the pull-up, and the panel offset.

> **Status: calibrated on the bench on 2026-11-15** (U8 in [the PoC plan](../plans/2026-11-13-1722-feat-companion-network-deck-poc-plan.md)), with one Feather ESP32 V2 (ESP32-PICO-V3-02, rev v3.0) and one ScreenKey Module B.

## Wiring

![Wiring diagram: Feather ESP32 V2 to one ScreenKey](deck-wiring.svg)

[`deck-wiring.svg`](deck-wiring.svg) is hand-maintained. Whenever a pin changes, update the table below, the pin setup in `firmware/src/main.rs` and the diagram in the same commit.

| ScreenKey pin | Feather ESP32 V2 | Notes |
|---|---|---|
| VCC | 3V | 3.3 V only |
| GND | GND | |
| SCLK | GPIO5 (SCK) | SPI2. Strapping pin with a boot pull-up. |
| DIN | GPIO19 (MOSI) | MISO unused |
| CS | GPIO33 | One per key |
| DC | GPIO27 | Shared by later keys |
| RST | GPIO15 | Shared. Strapping pin with a boot pull-up. |
| PWM | GPIO14 | Driven high; P2 has no brightness control |
| KEY | GPIO32 | Internal pull-up; one per key |

The pins are set in `firmware/src/main.rs`.

## Calibration

| Value | Firmware setting | Bench result |
|---|---|---|
| Panel offset (x, y) in the 132×162 RAM | `(2, 1)` in `firmware/src/display.rs` | Correct. The test border sits exactly on the glass edge. Its corners are hidden by the panel's rounded corners, which is expected. |
| Colour order | `Bgr` | Correct |
| Colour inversion | `Inverted` | Required. With `Normal`, every colour showed as its complement (red as cyan, white as black). |
| SPI clock | 20 MHz, mode 0 | Works. A full 128×128 frame draws in 40–45 ms. |
| KEY when pressed | low (`ACTIVE_LOW` in `firmware/src/key.rs`) | Confirmed |
| KEY needs the internal pull-up | yes | Works with only the internal pull-up |
| Debounce interval | 20 ms | 20 presses logged exactly 20 `key pressed` and 20 `key released` |
| Board boots with the key attached | expected | Yes. GPIO5 and GPIO15 (strapping pins) do not disturb boot. |

## How to calibrate

1. Flash the firmware (`firmware/README.md`). For 3 s after boot the key shows a test pattern: a 1 px white border, a red block top-left, green top-right, blue bottom-left and white bottom-right, with "TEST" in the middle.
2. **Offset.** Every border edge must be visible, and none may be doubled or show noise beyond it. Adjust `calibration::OFFSET`; common values for this panel size are `(2, 1)`, `(2, 3)` and `(0, 0)`.
3. **Colours.** If red and blue swap places, switch `COLOR_ORDER`. If every colour shows as its complement (white as black, red as cyan, green as magenta), switch `INVERSION`.
4. **KEY.** Press and release 20 times while watching the serial log. Each press must log exactly one `key pressed`, and each release one `key released`. If presses log as releases, flip `ACTIVE_LOW`. If edges are missing or doubled, check the pull-up and tune the debounce interval.
