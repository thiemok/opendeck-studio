---
title: Companion Network Deck PoC - Plan
type: feat
date: 2026-11-13
topic: companion-network-deck-poc
artifact_contract: ce-unified-plan/v1
product_contract_source: ce-brainstorm
execution: code
---

# Companion Network Deck PoC - Plan

## Goal Capsule

- **Objective:** A self-built deck works as one Bitfocus Companion surface over Wi-Fi. Every piece of hardware on hand has been proven in it: two LCD keys, a rotary encoder with menu buttons, and an OLED. Enough has been learned to choose the final device shape and to start the Ethernet/PoE stages.
- **Product authority:** This plan covers the proof-of-concept stages (P0–P4 below). The final device shape, Ethernet/PoE and scaling toward ~32 keys are context only. Each gets its own plan once the PoC is done.
- **Open blockers:** None. Hardware identification and the firmware framework choice are part of the stages themselves.

---

## Product Contract

### Summary

Build a microcontroller-based deck that connects to Bitfocus Companion as a network surface over Companion's Satellite protocol. Its keys show images pushed by Companion and report presses. Its encoder works as a Companion control, and its OLED doubles as a local device menu. The work runs in five proof-of-concept stages, from a desktop protocol spike to an OTA-updatable deck with every part working. After the PoC, the final shape is decided.

### Problem Frame

The Elgato Stream Deck Studio is the reference: a networked, PoE-powered control surface with LCD keys and push-dials. Elgato's software stack doesn't fit this setup. Every desktop host runs Linux, and Elgato's Stream Deck app only exists for Windows and macOS. The Studio's network protocol is undocumented, and posing as an Elgato device is fragile and depends on Elgato's goodwill.

The parts on hand are a Feather ESP32, a Pico 2 W, a Pi Zero 2 W, two Waveshare ST7735 LCD key modules, and a 1.3" I²C OLED module with a rotary encoder and menu buttons. Nothing has been built yet, and the key and OLED modules aren't fully identified. The intended use is mostly dev work, but nothing is fixed. The deck must be general-purpose rather than tuned to one workflow.

### Key Decisions

- **Bitfocus Companion is the only host software; the Satellite protocol is the device contract.** Companion runs natively on Linux, its protocol is open and versioned, and missing integrations can be written as custom Companion modules. (session-settled: user-directed — chosen over Elgato's Stream Deck app, alone or alongside Companion: no Linux build, undocumented network protocol, depends on impersonating a device.) Governs R1, R2, R3.
- **The deck is a pure network surface; Companion runs on a Linux desktop or server.** Companion can only run shell actions on the machine it runs on, so desktop-side dev actions work directly. (session-settled: user-directed — chosen over running Companion inside the deck on the Pi Zero 2 W: desktop actions would need extra plumbing.) Governs R1, R4.
- **The OLED module is mode-switched.** It is normally a Companion control and becomes a local device menu on a menu-button action. (session-settled: user-directed — chosen over Companion-only or local-menu-only.) Governs R8, R9, R10.
- **The deck runs on a microcontroller, not a Linux board.** It matches the appliance-style end goal: instant boot, low power and a proven PoE path. A desktop "fake deck" spike provides the protocol learning first. (session-settled: user-directed — chosen over a Pi Zero 2 W as the deck's brain, and over a Linux reference implementation later ported to a microcontroller.) Governs R14, R15.
- **Compiled firmware with OTA updates; no MicroPython.** OTA keeps flashing out of the iteration loop. The framework is chosen in planning (candidates: ESP-IDF, Arduino/PlatformIO, Zephyr, Rust embassy/esp-hal). Governs R12, R13.
- **The board comparison comes before deck-feature work.** (session-settled: user-directed — chosen over building a single key first and comparing boards afterwards: the basic setup would be done twice.) Governs R15, R16.
- **The final shape is decided after the PoC, not now.** Rack mounting is dropped. Key count (~32 eventually), display technology and enclosure stay open until the PoC has shown what works. (session-settled: user-directed — chosen over fixing the final form now.) Governs R18.

### Actors

- A1. Operator: the person setting up and using the deck (mainly for dev work).
- A2. Companion: Bitfocus Companion on a Linux desktop or server. It owns button layouts, images, actions and integrations.
- A3. Deck: the microcontroller device. It shows what Companion sends and reports input.

### Requirements

**Host integration**

- R1. The deck connects to a Companion instance over the LAN and registers as a single Satellite surface; nothing besides Companion runs on the desktop.
- R2. The surface exposes both LCD keys and the encoder control to Companion, so Companion's normal button editor configures them.
- R3. Companion's own and custom modules drive all actions; the deck holds no action logic.
- R4. The operator sets which Companion host the deck connects to without reflashing firmware.

**Keys and display output**

- R5. Each LCD key shows the image Companion assigns it and updates when Companion changes it.
- R6. Pressing and releasing a key reports press and release to Companion.
- R7. When Companion is unreachable, the deck shows a clear disconnected/searching state and reconnects automatically when Companion returns.

**Encoder and OLED**

- R8. In control mode, encoder rotation, encoder push and the OLED appear in Companion as one control: rotation and push are reported to Companion, and the OLED shows that control's image or text from Companion.
- R9. A menu-button action switches the OLED module into a local device menu and back; while the menu is open, encoder input goes to the menu and not to Companion.
- R10. The local menu shows connection status and the Companion host, and lets the operator adjust brightness and Wi-Fi settings.

**Connectivity and updates**

- R11. The PoC deck connects over Wi-Fi.
- R12. Firmware updates install over the network (OTA), without a USB cable.
- R13. The deck accepts OTA updates only from the operator, enforced in the PoC by a password or shared secret on the OTA endpoint.

**PoC stages and outcomes**

- R14. P0, protocol spike: a desktop "fake deck" script registers with Companion, shows key images and sends presses. Done when the Satellite protocol's key/encoder/image behaviour is understood well enough to implement in firmware.
- R15. P1, board comparison: the Pico 2 W and the Feather ESP32 are each tested for toolchain, Wi-Fi plus a Satellite connection, an OTA proof of life, and driving one ST7735 key, after the key module's controller and pinout are identified. One board is chosen with written reasons, which also record each board's path to wired Ethernet/PoE (native MAC vs SPI module, available PoE boards) and its headroom for more keys (free RAM after the Wi-Fi/OTA stack, spare SPI chip-select/GPIO pins).
- R16. P2, first key on the chosen board: one LCD key works end-to-end with Companion (R5, R6). The key module's model and pinout are documented.
- R17. P3, full PoC: both keys, the encoder and the OLED with its local menu work together as one surface (R2–R10). The OLED/encoder module is identified and documented.
- R18. P4, OTA and wrap-up: OTA works on the full PoC (R12, R13), and a written findings note feeds the final-shape decision (key count, display technology, enclosure, Ethernet/PoE hardware). The note records measured image memory per key, the time to refresh all keys after a Companion page change, and SPI redraw time per display, extrapolated to ~32 keys for both the per-key and shared-panel options.

### PoC Stages

```mermaid
flowchart TB
  P0[P0 Protocol spike on desktop] --> P1[P1 Pico 2 W vs ESP32 comparison]
  P1 --> P2[P2 First key on chosen board]
  P2 --> P3[P3 Full PoC: 2 keys + encoder + OLED menu]
  P3 --> P4[P4 OTA + findings]
  P4 --> D{Final shape decision}
  D --> E[Later: Ethernet]
  E --> F[Later: PoE]
  D --> S[Later: scale toward ~32 keys]
```

Each stage is defined by its requirement (R14–R18). The stages after the decision point are context, not scope.

### Key Flows

- F1. Power-on and connect
  - **Trigger:** The deck powers on.
  - **Actors:** A3, A2
  - **Steps:** The deck joins Wi-Fi and connects to the configured Companion host. It registers as a surface and Companion pushes the current images. Until then the deck shows the disconnected/searching state.
  - **Covered by:** R1, R4, R5, R7, R11
- F2. Press a key
  - **Trigger:** The operator presses an LCD key.
  - **Actors:** A1, A3, A2
  - **Steps:** The deck reports press and release. Companion runs the action and may push a new image, which the deck shows.
  - **Covered by:** R5, R6
- F3. Use the encoder, then open the local menu
  - **Trigger:** The operator turns or pushes the encoder, then uses the menu-button action.
  - **Actors:** A1, A3, A2
  - **Steps:** In control mode, input goes to Companion and the OLED shows Companion's content. The menu action switches to the local menu, where encoder input navigates the menu. Leaving the menu returns to control mode with Companion's content restored.
  - **Covered by:** R8, R9, R10
- F4. Update firmware
  - **Trigger:** The operator pushes a new firmware build.
  - **Actors:** A1, A3
  - **Steps:** The deck accepts the operator's update, installs it, restarts, and reconnects via F1.
  - **Covered by:** R12, R13

### Acceptance Examples

- AE1. **Covers R7.** **Given** the deck is connected, **when** Companion is stopped, **then** the deck shows the disconnected state within a few seconds, **and** when Companion restarts it reconnects and restores the key images without operator action.
- AE2. **Covers R9.** **Given** the local menu is open, **when** the operator turns the encoder, **then** the menu moves and Companion receives no rotation events.
- AE3. **Covers R9, R8.** **Given** Companion changed the encoder control's image while the local menu was open, **when** the operator leaves the menu, **then** the OLED shows Companion's latest content.
- AE4. **Covers R13.** **Given** a firmware image pushed without the OTA password, **when** it reaches the deck, **then** the deck rejects it and keeps running its current firmware.

### Success Criteria

- The full PoC (P3) feels responsive in daily use: a key press triggers its Companion action, and the key's new image appears, without noticeable delay over Wi-Fi.
- The P4 findings note is enough to decide the final shape without repeating PoC experiments.

### Scope Boundaries

**Deferred for later**

- Ethernet and PoE: separate stages after the final-shape decision, needing hardware not yet owned (e.g. a W5500 module or an ESP32 PoE board).
- Scaling toward ~32 keys, choosing display technology (one display per key vs a shared panel) and enclosure design.
- Custom Companion modules for specific dev workflows.
- Signed firmware for OTA; the PoC uses a password gate (R13).
- Custom PCB: a likely step once the final shape is set; the PoC uses off-the-shelf boards and modules.

**Outside this product's identity**

- Compatibility with Elgato's Stream Deck app or impersonating Elgato hardware.
- Running Companion on the deck itself.
- Rack-mount (1U) form factor.

### Dependencies / Assumptions

- Companion 4.2 or newer (Satellite API 1.9+) is available on a Linux host. Earlier versions have no layout manifests, so the LCD keys and the encoder/OLED control could not be declared as separate controls with their own bitmap sizes. Non-square bitmaps need Companion 5.0+ (API 1.11). P0 confirms the exact minimum.
- The Waveshare key modules are SPI ST7735 displays with a key switch, and the OLED module is an I²C display plus encoder and buttons. Both are assumptions until identified: the key module in P1, the OLED module in P3.
- The Pi Zero 2 W is not part of the deck. It may serve as a spare headless Companion host or test bench.
- The LAN is trusted for Satellite traffic, which is unauthenticated. Only OTA needs protection (R13).

### Outstanding Questions

**Resolve Before Planning**

- None.

**Deferred to Planning**

- Which firmware framework best meets the OTA requirement on each candidate board, and whether that alone decides P1. OTA is more mature on the ESP32.
- How the operator sets the Companion host and Wi-Fi credentials the first time (R4, R10): captive portal, serial, mDNS discovery, or a mix.
- Which Satellite API version and image sizes/formats to target, given memory limits on the chosen board.
- Which menu-button action opens the local menu (e.g. long press vs a dedicated button), once the module's buttons are known.
- What measurement backs the responsiveness criterion, if one is needed.

### Sources / Research

- Companion Satellite API: https://companion.free/for-developers/Satellite-API and https://companion.free/user-guide/remote_control/satellite
- Companion 3.4 release (Stream Deck Studio support, surfaces over 32 keys; background only, below the version baseline): https://newreleases.io/project/github/bitfocus/companion/release/v3.4.0
- Companion module development: https://companion.free/for-developers/setting-up-developer-environment
- Elgato HID protocol docs, for reference only: https://docs.elgato.com/streamdeck/hid/
- Prior DIY Elgato emulation, background on the rejected path: https://github.com/DDRBoxman/picodeck
