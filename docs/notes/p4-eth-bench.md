# ESP32-P4-ETH bench (THI-84)

Does the PoC deck run on the Waveshare ESP32-P4-ETH over wired Ethernet, powered over PoE? And does the board fit under the side section, for THI-83? This note holds the runbook, the numbers and the recommendation. The plan is [2026-11-27-1330-feat-esp32-p4-eth-port-plan.md](../plans/2026-11-27-1330-feat-esp32-p4-eth-port-plan.md), and the wiring is in [deck-wiring.md](deck-wiring.md) (ESP32-P4-ETH column).

> **Status:** waves 1 and 2 run on 2026-11-27 (PR #11). Steps 1, 2 and 3 pass, step 2 after two firmware fixes recorded below; the cable unplug/replug part of step 2 is still to do. The steps that need the PoC bench, PoE or the Feather are still to do.

## What the desk research settled

- esp-hal 1.2.2, the version the Feather already pins, supports the ESP32-P4 (silicon v3.0 and later) and its Ethernet MAC. Both boards stay on the same esp-rs versions, so the Feather needed no upgrade (R3, plan KTD2).
- Ethernet uses esp-hal's own EMAC driver with its generic PHY driver. The community `esp-p4-eth` crate is a fallback only (R4, KTD8).
- esp-hal has no pulse-counter (PCNT) driver for the ESP32-P4, so the P4-ETH decodes the encoder in the GPIO interrupt (KTD13).

## Runbook

Build and flash with `mise exec -- cargo p4` in `firmware/` (see `firmware/README.md`). Wire the bench as in the ESP32-P4-ETH column of [deck-wiring.md](deck-wiring.md).

1. **Silicon.** Run `espflash board-info` with the board on USB-C. esp-hal needs chip revision v3.0 or later. Below that, stop: the firmware can't run, and the THI-84 plan's bench redirect applies.
2. **Ethernet and Companion (R1).** With the board on USB-C power and the Ethernet cable plugged in, boot and check the log for `Ethernet link up` and `DHCP address …`. Set the host with `host <companion-ip>`. The displays must go from "No Ethernet" to "Connecting" to Companion's images. Unplug the cable: the log says `Ethernet link down` and the deck reconnects when it comes back. Check that the host survives a reboot and a reflash (R7).
3. **A/B boot (R2, AE1).** Flash normally, so the image is in `ota_0`. Write the same build into `ota_1` as well, using espflash's target-app-partition option. Type `slot`: it prints the booted slot, the next-boot selection and the image's build time. Type `slot next`; after the restart, `slot` must report `ota_1` with the same build time. A second `slot next` must bring back `ota_0`. If the board falls back to `ota_0` while `ota_1` holds an image, A/B boot fails: record it, keep flashing over USB-C, and file the OTA-on-ESP32-P4 follow-up from THI-75. A fallback while `ota_1` is empty is not a result.
4. **Keys, OLED, encoder, buttons (R5).** During the 3 s test pattern, both keys and the OLED must show their patterns as on the Feather. Then repeat the button and round-trip checks from [io-expander-bench.md](io-expander-bench.md#runbook), steps 3 to 5. Turn the encoder one detent clockwise: the log must say `encoder +1`. If it says `-1`, flip `CLOCKWISE_SIGN` in `firmware/src/board/p4_eth/mod.rs`. Spin it fast while paging, and check that no detents are lost or doubled.
5. **PoE only (R6).** Unplug USB-C and power the board only through the PoE module, from a PoE switch or injector. Repeat steps 2 and 4.
6. **PoE power (R17, AE5).** Measure the board's draw with the full bench at its busiest, while paging with `bench 16 draw`, using a USB power meter on USB-C, or the switch's per-port PoE reading. Read the PoE module's rating off the module or its datasheet, and record both.
7. **Feather retest (R12, AE2).** Flash the Feather from the same commit (`mise exec -- cargo run --release`). It must join Wi-Fi, connect to Companion, and keys, OLED, encoder and buttons must work as before.
8. **Page change (R16).** Run the [page-change bench](satellite-protocol-firmware-notes.md#bench-runbook) with `bench 16 draw` on the P4-ETH. Copy the `press to last key median … ms, max … ms` line, and compare it with the Feather's 1,056 / 1,379 ms (THI-76, `png`, draw).
9. **Fit (R18).** Measure the board with its PoE module fitted, including the height, against the 71.2 × 80 mm side section. Check whether the RJ45 jack can reach the end edge.

## Results

### Wave 1: USB-C only (2026-11-27)

| Step | Result |
|---|---|
| 1. Silicon | **Pass.** `espflash board-info`: `esp32p4 (revision v3.2)`, 32 MB flash, 40 MHz crystal. |
| 2. Ethernet and Companion | **Partly run.** With no cable, the firmware starts the EMAC and the IP101 PHY without the panic a missing PHY would cause, and embassy-net reports `IPv4: DOWN`. The host survives a reboot and a full reflash: `host 192.168.201.99` was still `saved host 192.168.201.99:16622` after both (R7, cleared afterwards). DHCP, the Companion link and cable unplug/replug wait for a cable. |
| 3. A/B boot | **Pass.** After writing the same build into `ota_1` with `--target-app-partition ota_1`, `slot next` restarted into `ota_1` (bootloader: `Loaded app from partition at offset 0x310000`; `slot`: `booted ota_1`, `next boot Ota1 (Valid)`), which survived a power-on reset. A second `slot next` brought back `ota_0` (`0x10000`). The reply line after `slot next` is cut off by the restart. |
| 4–9 | Not run yet: they need the PoC bench, PoE or the Feather. |

### Wave 2: Ethernet to the switch, Companion on the laptop (2026-11-27)

Companion 5.1.3 ran in `tools/fake-deck` on a laptop on the same subnet, over Wi-Fi.

| Step | Result |
|---|---|
| 2. Ethernet and Companion | **Pass, after two fixes.** The link comes up about 5 s after boot and DHCP gives `192.168.201.78/24`. The deck registers as `opendeck-80f1b2dae508` (the factory MAC), switches to `png`, and decodes all three key images. |

Two firmware bugs showed up and are fixed:

- **Lost transmit frames.** The deck connected, received `BEGIN` and `CAPS`, and retransmitted its 540-byte `ADD-DEVICE` until Companion hung up after 5 s with no surface. Frame logging in the driver showed every frame arriving and leaving, so the frames left with bad checksums. esp-hal's driver asks the MAC to insert IP and TCP checksums, but the P4's transmit FIFO is smaller than a full frame and the MAC cannot insert a checksum in a frame larger than its FIFO. Small frames (ACKs) worked; DHCP worked because a zero UDP checksum means none. The P4 board module now has smoltcp compute transmit checksums.
- **Floating encoder phases.** With no encoder module plugged in, the two phase pins floated, and since the P4 interrupts on every edge, the firmware stalled after a second or two of boot. They now have internal pull-ups.

Bench caveat: holding the serial port open with DTR and RTS at different levels disturbs the board, because the CH343's auto-reset circuit also drives GPIO35, which is the PHY's TXD1. `espflash monitor` is fine. A script that opens the port must keep DTR and RTS equal (both asserted works).

The boot log on a bare board, as expected without the PoC parts: `opendeck booting: chip "ESP32-P4" rev v3.2, free heap 163840 B`, `deckcfg found at 0x610000, 64 KB`, then warnings that the MCP23017 and the OLED did not answer on I²C.

Opening the serial port resets the board through the CH343's DTR/RTS lines, as on the Feather.

## Fit (R18)

On paper, the board is 78 × 21 mm (from Waveshare's outline drawing), and the side section is 71.2 × 80 mm. Laid lengthwise along the 80 mm side, the bare board fits with about 2 mm to spare. The PoE module's size, the stack height, and the RJ45 jack's direction still have to be measured (runbook step 9).

## Recommendation for THI-83

Waiting for the results. It will say whether to mount the P4-ETH as a daughter board under the side section or copy its circuit onto the deck board, and it will include the fit (R18) and power (R17) results (R19).
