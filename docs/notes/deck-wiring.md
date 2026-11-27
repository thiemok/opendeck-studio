# Deck wiring and calibration

How the two Waveshare 0.85" ScreenKey Module B keys, the 1.3" OLED/encoder module and the Adafruit MCP23017 I/O expander are wired to the Adafruit Feather ESP32 V2 and to the Waveshare ESP32-P4-ETH, and the panel values the firmware uses. Both boards use the same parts and the same expander pins; only the board's GPIO numbers differ (THI-84). The P2 section closes the three bench checks P1 left open in [the board decision](board-and-framework-decision.md): KEY polarity, the pull-up, and the panel offset. The module itself is described in [oled-encoder-module.md](oled-encoder-module.md).

> **Status:**
> - **Key A** was calibrated on the bench on 2026-11-15 (U8 in [the PoC plan](../plans/2026-11-13-1722-feat-companion-network-deck-poc-plan.md)), with one Feather ESP32 V2 (ESP32-PICO-V3-02, rev v3.0) and one ScreenKey Module B.
> - **Key B and the OLED/encoder module** (P3) were checked on the bench on 2026-11-16 with the direct pins of that time (U13 and U14).
> - **The MCP23017 wiring** (THI-89, [plan](../plans/2026-11-21-1733-feat-mcp23017-key-inputs-bench-plan.md)) replaced those direct pins on 2026-11-21. The keys' KEY, CS and RST and the module's three buttons now go through the expander. The firmware follows the same pins. Checked on the bench on 2026-11-22; results in [io-expander-bench.md](io-expander-bench.md).
> - **The ESP32-P4-ETH column** (THI-84, [plan](../plans/2026-11-27-1330-feat-esp32-p4-eth-port-plan.md), KTD14) was chosen from the board's schematic and is not yet checked on the bench. Bench results go in [p4-eth-bench.md](p4-eth-bench.md).

## Wiring

![Wiring diagram: Feather ESP32 V2 to two ScreenKeys and the OLED/encoder module](deck-wiring.svg)

[`deck-wiring.svg`](deck-wiring.svg) is hand-maintained and shows the Feather. Whenever a pin changes, update the tables below, the board's pin setup in `firmware/src/board/` and, for the Feather, the diagram in the same commit.

On the ESP32-P4-ETH, every part sits on the two 20-pin headers, away from the Ethernet RMII pins (GPIO28–31, 34, 35, 49–52), the SD slot (GPIO39–44), the console (GPIO37, GPIO38) and the strapping pins. Power comes from the header's 3V3 pin.

### MCP23017 expander

The Adafruit MCP23017 breakout shares I²C0 with the OLED at address 0x20 (address pins D0–D2 and the +1/+2/+4 jumpers left open). Power and I²C come from the Feather's header pins: 3V, GND, GPIO22 (SDA) and GPIO20 (SCL), with plain wires or a STEMMA-QT-to-breadboard cable plugged into the breakout. Don't use the Feather's own STEMMA QT port. Its 3.3 V is switched by GPIO2, which the firmware doesn't drive. The OLED/encoder module is chained off the breakout's QT port, so it shares the same 3.3 V, GND, SDA and SCL. The breakout already has 10 kΩ pull-ups on SDA and SCL. The breakout's silkscreen names the expander pins A0–A7 and B0–B7 (GPA0–GPA7 and GPB0–GPB7 in the datasheet); they are not the Feather's A0–A5. A7 and B7 are output-only on current chips, so nothing is wired to them.

| Breakout pin | Feather ESP32 V2 | ESP32-P4-ETH | Wire | Notes |
|---|---|---|---|---|
| VIN | 3V | 3V3 | header wire | 3.3 V only |
| GND | GND | GND | header wire | |
| SDA | GPIO22 (SDA) | GPIO7 (SDA) | header wire | Shared with the OLED |
| SCL | GPIO20 (SCL) | GPIO8 (SCL) | header wire | Shared with the OLED |
| IB (INTB) | GPIO37 | GPIO26 | header wire | Port B changed; push-pull, active low. On the Feather, GPIO37 is input-only and floats low if the wire is missing; the firmware then warns and polls every 2 ms instead. |
| RST | 3V | 3V3 | header wire | Keeps the expander out of reset; Adafruit's guide doesn't say the board pulls it up |
| IA (INTA), A3–A7, B5–B7 | — | — | — | Unused |

| Breakout pin (chip name) | Goes to | Direction | Notes |
|---|---|---|---|
| A0 (GPA0) | Key A CS | output | High = not selected |
| A1 (GPA1) | Key B CS | output | High = not selected |
| A2 (GPA2) | RST of both keys | output | Pulsed low once at boot |
| B0 (GPB0) | Key A KEY | input | Expander's internal pull-up |
| B1 (GPB1) | Key B KEY | input | Expander's internal pull-up |
| B2 (GPB2) | Module PSH | input | Module's own 10 kΩ pull-up, plus the expander's |
| B3 (GPB3) | Module BAK | input | Module's own 10 kΩ pull-up, plus the expander's |
| B4 (GPB4) | Module CON | input | Module's own 10 kΩ pull-up, plus the expander's |

**Bench only:** a second wire from key A's KEY pin to GPIO32 on the Feather, GPIO27 on the ESP32-P4-ETH. The firmware times each press from this pin's edge to the expander read that sees it. Both pull-ups are in parallel, and the switch still pulls the line low. Remove the wire after the THI-89 bench session; the deck works without it.

Freed Feather pins: GPIO15, GPIO25, GPIO26, GPIO33, GPIO36 and GPIO39 (and GPIO32 once the bench wire is gone).

### Keys

Both keys share the SPI bus, DC, RST and the backlight; each has its own CS and KEY.

| ScreenKey pin | Key A | Key B | ESP32-P4-ETH (both keys) | Notes |
|---|---|---|---|---|
| VCC | 3V | 3V | 3V3 | 3.3 V only |
| GND | GND | GND | GND | |
| SCLK | GPIO5 (SCK) | GPIO5 (SCK) | GPIO20 | SPI2. On the Feather, a strapping pin with a boot pull-up. |
| DIN | GPIO19 (MOSI) | GPIO19 (MOSI) | GPIO21 | MISO unused |
| CS | MCP A0 | MCP A1 | MCP A0 / A1 | One per key, through the expander |
| DC | GPIO27 | GPIO27 | GPIO22 | Shared |
| RST | MCP A2 | MCP A2 | MCP A2 | Shared, through the expander; pulsed once at boot |
| PWM | GPIO14 | GPIO14 | GPIO23 | Driven high; no brightness control yet (THI-81) |
| KEY | MCP B0 (bench: also GPIO32) | MCP B1 | MCP B0 (bench: also GPIO27) / B1 | Expander's internal pull-up; one per key |

The Key A and Key B columns are the Feather's pins.

### OLED/encoder module

The module has its own 10 kΩ pull-ups on every signal, so the input-only pins work without internal pull-ups. The encoder phases stay on direct pins, because the PCNT counter needs them, and avoid GPIO36 and GPIO39, which the ESP32 errata say can see short low glitches (KTD25 in the PoC plan). The three buttons go through the expander.

| Module pin | Feather ESP32 V2 | ESP32-P4-ETH | Notes |
|---|---|---|---|
| VCC | QT chain 3.3 V | QT chain 3.3 V | Through the expander's QT port; 3.3 V keeps the module's pull-ups at ESP32 levels |
| GND | QT chain GND | QT chain GND | |
| SDA | QT chain, GPIO22 (SDA) | QT chain, GPIO7 | I²C0, shared with the expander |
| SCL | QT chain, GPIO20 (SCL) | QT chain, GPIO8 | I²C0, shared with the expander |
| TRA | GPIO34 (A2) | GPIO32 | Encoder phase A. Feather: counted by PCNT, input only. P4-ETH: decoded in the GPIO interrupt, because esp-hal has no PCNT driver for the ESP32-P4. |
| TRB | GPIO4 (A5) | GPIO33 | Encoder phase B, as phase A |
| PSH | MCP B2 | MCP B2 | Encoder push |
| BAK | MCP B3 | MCP B3 | BACK; logged, never sent to Companion |
| CON | MCP B4 | MCP B4 | CONFIRM; logged, never sent to Companion |

The pins are set in `firmware/src/board/feather/mod.rs` and `firmware/src/board/p4_eth/mod.rs`.

## Calibration

### Keys

| Value | Firmware setting | Bench result |
|---|---|---|
| Panel offset (x, y) in the 132×162 RAM | `(2, 1)` in `firmware/src/display.rs` | Key A: correct. The test border sits exactly on the glass edge. Its corners are hidden by the panel's rounded corners, which is expected. Key B: correct with the same value. |
| Colour order | `Bgr` | Both keys: correct |
| Colour inversion | `Inverted` | Both keys: required. With `Normal`, every colour showed as its complement (red as cyan, white as black). |
| SPI clock | 20 MHz, mode 0 | Key A: works. A full 128×128 frame decodes and draws in 26 ms. |
| KEY when pressed | low (`pressed` in `crates/deck-core/src/expander.rs`) | Both keys: confirmed |
| KEY needs the internal pull-up | yes | Both keys: work with only the internal pull-up |
| Debounce interval | 20 ms | Key A: 20 presses logged exactly 20 `key pressed` and 20 `key released` (P2 log wording) |
| Board boots with everything attached | expected | Key A alone: yes. GPIO5 and GPIO15 (strapping pins) do not disturb boot. All P3 parts: yes. |

The OLED and encoder values are in [oled-encoder-module.md](oled-encoder-module.md#calibration).

## How to calibrate the keys

1. Flash the firmware (`firmware/README.md`). For 3 s after boot each key shows a test pattern: a 1 px white border, a red block top-left, green top-right, blue bottom-left and white bottom-right, with "KEY A" or "KEY B" in the middle.
2. **Offset.** Every border edge must be visible, and none may be doubled or show noise beyond it. Adjust `calibration::OFFSET`; common values for this panel size are `(2, 1)`, `(2, 3)` and `(0, 0)`.
3. **Colours.** If red and blue swap places, switch `COLOR_ORDER`. If every colour shows as its complement (white as black, red as cyan, green as magenta), switch `INVERSION`.
4. **Buttons.** Press and release each button 20 times while watching the serial log. Each press must log exactly one `KeyA pressed` (or `KeyB`, `EncoderPush`, `Back`, `Confirm`), and each release one `… released`. If presses log as releases, invert `pressed` in `crates/deck-core/src/expander.rs`. If edges are missing or doubled, check the pull-up and tune the debounce interval.
