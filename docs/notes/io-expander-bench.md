# MCP23017 expander bench (THI-89)

Does an MCP23017 on the OLED's I²C bus handle the keys' buttons and chip selects well enough for the 16-key PCB (THI-83)? This note holds the runbook, the numbers, and the recommendation. The plan is [2026-11-21-1733-feat-mcp23017-key-inputs-bench-plan.md](../plans/2026-11-21-1733-feat-mcp23017-key-inputs-bench-plan.md), and the wiring is in [deck-wiring.md](deck-wiring.md).

> **Status:** firmware and wiring ready on 2026-11-21. Bench numbers still to be measured.

## Runbook

1. Wire the bench as in [deck-wiring.md](deck-wiring.md), including the dashed bench-only wire from key A's KEY pin to GPIO32.
2. Flash the firmware (`firmware/README.md`). The boot log must show `MCP23017 ready at 0x20`, and both keys must show their test pattern for 3 s.
3. **Buttons.** Press each of the five buttons 20 times with the deck idle. Each press must log exactly one `… pressed` and one `… released` line. Repeat while spinning the encoder (the OLED redraws on every detent), and again while paging in Companion (both keys receiving images).
4. **Edge to detection.** Each KEY A press logs `KEY A edge to expander read: N µs`, and every 20 presses a `median … µs, max … µs` summary. Copy the summary for an idle bus, for presses while spinning the encoder, and for presses during page changes.
5. **Round trip.** With the deck registered normally, press KEY A and KEY B with buttons that change their own image. Copy the `round trip` summary line (KTD27).
6. **Page change.** Run `bench 16 draw` in `rgb`, then in `png`, ten or more page changes each, as in [the page-change bench](satellite-protocol-firmware-notes.md). Copy the `press to last key median … ms, max … ms` line.
7. **Missing expander.** Unplug the expander's QT cable and reboot. The log must say the MCP23017 was not found, the keys stay dark, and the OLED, the encoder and the Companion link keep working. Plug it back in, and unplug it again while running: the log shows one `MCP23017 read failed` line, not a flood.

## Results

The baseline was measured on 2026-11-16 with direct GPIO pins, on the same bench (P3 Feather, Companion 5.1.3, same Wi-Fi).

| Measurement | Direct pins (baseline) | MCP23017 |
|---|---|---|
| 20 presses × 5 buttons, idle / OLED redrawing / page change | 20/20 (key A, P2 check) | |
| Edge to detection, idle bus | ≤ 2 ms by construction (2 ms sampler) | |
| Edge to detection, OLED redrawing | ≤ 2 ms plus any synchronous draw at the edge | |
| Edge to detection, during page changes | ≤ 2 ms plus up to one key draw (26–66 ms) | |
| Key round trip, median / max | 216 / 516 ms | |
| `bench 16 draw`, `rgb`, press to last key, median / max | 3,957 / 4,684 ms | |
| `bench 16 draw`, `png`, press to last key, median / max | 1,056 / 1,379 ms | |
| Missing expander at boot / at runtime | n/a | |

## Recommendation for THI-83

To be written from the results above. It covers the key-input chip (two MCP23017s or a TCA8418), the CS scheme (I²C expander or a 74HC154 decoder), and the ESP32 pin count for the 16-key deck next to Ethernet.
