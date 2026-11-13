# opendeck-studio

A self-built network control deck in the spirit of the Stream Deck Studio. It appears in [Bitfocus Companion](https://bitfocus.io/companion) as a surface over the Satellite protocol. Companion runs on a Linux desktop or server, and the deck only shows images and reports input.

Current phase: proof of concept on Wi-Fi with off-the-shelf parts (two ST7735 LCD keys, a 1.3" OLED with rotary encoder, Pico 2 W vs Feather ESP32). Ethernet/PoE, scaling toward ~32 keys and a custom PCB come after the final-shape decision.

The requirements and stage plan are in [`docs/plans/2026-11-13-1722-feat-companion-network-deck-poc-plan.md`](docs/plans/2026-11-13-1722-feat-companion-network-deck-poc-plan.md).

| Stage | Goal |
|---|---|
| P0 | Desktop "fake deck" script learns the Satellite protocol |
| P1 | Pico 2 W vs ESP32 comparison; identify the key module |
| P2 | First LCD key end-to-end on the chosen board |
| P3 | Full PoC: two keys, encoder, OLED with local menu |
| P4 | Password-gated OTA and measured findings for the final-shape decision |
