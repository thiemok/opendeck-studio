# MCP23017 expander bench (THI-89)

Does an MCP23017 on the OLED's I²C bus handle the keys' buttons and chip selects well enough for the 16-key PCB (THI-83)? This note holds the runbook, the numbers, and the recommendation. The plan is [2026-11-21-1733-feat-mcp23017-key-inputs-bench-plan.md](../plans/2026-11-21-1733-feat-mcp23017-key-inputs-bench-plan.md), and the wiring is in [deck-wiring.md](deck-wiring.md).

> **Status:** measured on 2026-11-22 on the async build (firmware from commit `45fa4a5`), Companion 5.1.3 in `fake-deck-companion`, deck on the UniFi "Devices" network. The recommendation is below and on THI-83.

## Runbook

1. Wire the bench as in [deck-wiring.md](deck-wiring.md), including the dashed bench-only wire from key A's KEY pin to GPIO32.
2. Flash the firmware (`firmware/README.md`). The boot log must show `MCP23017 ready at 0x20`, and both keys must show their test pattern for 3 s.
3. **Buttons.** Press each of the five buttons 20 times with the deck idle. Each press must log exactly one `… pressed` and one `… released` line. Repeat while spinning the encoder (the OLED redraws on every detent), and again while paging in Companion (both keys receiving images).
4. **Edge to detection.** Each KEY A press logs `KEY A edge to expander read: N µs`, and every 20 presses a `median … µs, max … µs` summary. Copy the summary for an idle bus, for presses while spinning the encoder, and for presses during page changes.
5. **Round trip.** With the deck registered normally, press KEY A and KEY B with buttons that change their own image. Copy the `round trip` summary line (KTD27).
6. **Page change.** Run `bench 16 draw` (the deck only asks for `png` since PR #7), ten or more page changes, as in [the page-change bench](satellite-protocol-firmware-notes.md). Copy the `press to last key median … ms, max … ms` line.
7. **Missing expander.** Disconnect the expander's header wires while running: the log shows one `MCP23017 read failed` line, not a flood, and the link stays up. Reboot with it still disconnected: the log says the MCP23017 was not found, the keys stay dark, and Wi-Fi and the Companion link come up. The OLED/encoder module is chained off the breakout, so it goes dark too.

## Results

The baseline was measured on 2026-11-16 with direct GPIO pins, on the same bench (P3 Feather, Companion 5.1.3, same Wi-Fi).

| Measurement | Direct pins (baseline) | MCP23017 |
|---|---|---|
| 20 presses × 5 buttons, idle | 20/20 (key A, P2 check) | 20/20 on all five |
| 20 KEY A presses, OLED redrawing | not measured | 20/20 |
| Presses during page changes (KEY A pages down, KEY B goes to P1, tight A → B × 20) | not measured | A 20/20; B only 9/20, see below |
| Edge to detection, idle bus | ≤ 2 ms by construction (2 ms sampler) | median 1.27 ms, max 2.93 ms (20 presses) |
| Edge to detection, OLED redrawing | ≤ 2 ms plus any synchronous draw at the edge | median 1.26 ms, max 3.0 ms (20 presses) |
| Edge to detection, during page changes | ≤ 2 ms plus up to one key draw (26–66 ms) | median 1.25 ms, max 3.0 ms |
| Key round trip, median / max | 216 / 516 ms | 91 / 113 ms idle, 93 / 133 ms during page changes |
| `bench 16 draw`, `rgb`, press to last key, median / max | 3,957 / 4,684 ms | not measured (`rgb` removed in PR #7) |
| `bench 16 draw`, `png`, press to last key, median / max | 1,056 / 1,379 ms | 1,063 / 1,171 ms (20 page changes); decode + draw about 51 ms per key |
| Missing expander at boot / at runtime | n/a | Boot: `MCP23017 not found`, keys dark, link and registration up. Runtime: one `read failed` line, link stays up |

### Findings

- **Edge to detection is about 1.3 ms through the expander, against a 20 ms debounce.** The async path (interrupt wakes the task, which takes the shared bus and reads port B) costs about 1 ms more than the blocking first version (0.1–0.2 ms). An OLED page write on the shared bus adds at most one page, about 3 ms in the worst press seen.
- **A short tap during a page change is lost.** Key images are decoded and sent over SPI synchronously, which blocks the executor for about 50 ms per key and 100–150 ms per page change. A tap that starts and ends inside that window is never read. In the tight A → B test, B was registered only when it went down while A was still held (`A+ B+ A- B-`); every B after A's release was lost. This isn't caused by the expander: the direct-pin version sampled on the same executor and would lose the same taps. The MCP23017 latches the port at interrupt time (INTCAP), so the firmware could recover such taps, at the cost of a more complex debounce. A TCA8418 scans and debounces in hardware and queues key events in a FIFO, which makes the executor irrelevant.
- **CS over I²C costs a page change nothing measurable.** Two writes per key draw (select, deselect) add about 1 ms per key; 16 keys moved the median press-to-last-key from 1,056 to 1,063 ms, inside the run-to-run spread.
- **The round trip is better than the 2026-11-16 baseline,** but under different conditions (Companion recreated, Wi-Fi state), so it isn't evidence that the expander speeds anything up.
- **Bench pitfalls found on the way:** an esp-hal I²C transfer cancelled by a software timeout while Wi-Fi starts leaves SDA and SCL low for good; a missing IB → GPIO37 wire leaves GPIO37 floating low, and the inputs task then spun on the bus until the firmware learned to poll instead.

## Recommendation for THI-83

- **Key inputs: a TCA8418 keypad scanner, not two MCP23017s.** Both carry 16 keys plus the module buttons; the bench showed the difference that matters. Inputs read by the firmware are lost when a tap falls inside a synchronous redraw (100–150 ms per page change). The TCA8418 scans, debounces and queues key events in a FIFO in hardware, so a busy executor delays an event but never loses it. Its 18 pins cover a 21-button matrix (for example 5 × 5) and leave spare GPIOs. The MCP23017 route stays possible: two chips (14 inputs each, A7/B7 are output-only) plus INTCAP handling in firmware to recover short taps.
- **Chip selects: one MCP23017 on the same I²C bus, not a 74HC154.** CS over I²C cost about 1 ms per key draw and nothing measurable per 16-key page change, and it needs no ESP32 pins, against five for a 74HC154 (four address lines and enable). One MCP23017 gives exactly 16 outputs (A7/B7 are fine as outputs). The shared panel RST then needs a spare TCA8418 GPIO or one ESP32 pin.
- **ESP32 pins next to Ethernet:** I²C (2), TCA8418 interrupt (1), SPI clock and data for two buses of 8 panels (4), DC per bus (2), backlight (1), encoder phases on PCNT (2), panel RST if no expander pin is free (1): **12–13 pins**, against about 13 free on the ESP32-POE2 once its fixed EMAC pins are taken. That fits, but tightly; one SPI bus would save 3 pins at the cost of page-change time (THI-85). The interrupt and the encoder phases can use input-only pins (GPIO34/35/39), keeping GPIO36/39 off the encoder (errata). THI-84 confirms the real free-pin list.
- **Firmware rules for the PCB:** keep the I²C bus async and never cancel a transfer with a software timeout; treat an interrupt line that stays low after reads as a wiring fault and poll; draw panels in chunks that yield to the executor (follow-up) so inputs and the link keep running during page changes.
