# Satellite protocol: firmware notes (P0)

What the opendeck firmware must implement to appear in Bitfocus Companion as a Satellite surface with two LCD keys and one encoder/OLED control. All results come from the P0 fake deck (`tools/fake-deck/`). It was run on 2026-11-13 against Companion **v5.1.3** (build `5.1.3+10036-stable-93ea3fa61c`, Satellite **API 1.14.0**) in the pinned container from `tools/fake-deck/compose.yaml`. Statements marked *(source)* were read from the Companion v5.1.3 source rather than observed.

Only the latest Companion release was tested. Older versions are out of scope (see the plan's Dependencies / Assumptions).

## Summary for P1

- **Transport:** plain TCP to port 16622, newline-terminated ASCII lines. No TLS and no authentication.
- **Keepalive:** send `PING` every 2 s. **Companion closes any connection that sends nothing for 5 s** (measured 5.0 s, `socket.setTimeout(5000)` *(source)*).
- **Registration:** after `BEGIN` + `CAPS`, send one `ADD-DEVICE` with a `LAYOUT_MANIFEST`. Re-send it after every reconnect, because disconnect removes the device.
- **Input:** `KEY-PRESS` (pressed / released) and `KEY-ROTATE` by `CONTROLID`. Every one is answered with `… OK`.
- **Output:** `KEY-STATE` per control, carrying a raw RGB bitmap at exactly the manifest size. It is pushed on registration and on every change.
- **Images:** default raw `rgb` costs 15,552 B per 72×72 key (20,736 B on the wire as base64), and 24,576 B per 128×64 OLED image (32,768 B base64). Optional `png` is about 10× smaller on the wire but needs a decoder.
- **OLED:** request a 128×64 bitmap and threshold it to 1-bit on the device. Don't build a text renderer (see [OLED](#oled-image-vs-text)).

## Messages

Format: `COMMAND [OK|ERROR] KEY=value KEY="quoted value" …\n`. Companion quotes every string value, writes numbers and booleans bare (`1`/`0`), and ends each line with a trailing space before `\n`. A backslash escapes the next character inside values. Split key from value on the **first** `=` only, because base64 padding contains `=`. Lines over 2 MB close the connection (Satellite API docs).

### Device → Companion

| Message | When | Notes |
|---|---|---|
| `ADD-DEVICE DEVICEID="…" SERIAL="…" PRODUCT_NAME="…" LAYOUT_MANIFEST="<base64 JSON>"` | Once per connection, after `CAPS` | Optional `BITMAP_FORMAT="png"` when `CAPS` lists it. `DEVICEID` must be unique on the server. A duplicate gets `ADD-DEVICE ERROR … MESSAGE="Device exists elsewhere"` *(source)*. `SERIAL` is what Companion remembers the surface by. |
| `KEY-PRESS DEVICEID="…" CONTROLID="0/0" PRESSED=1` / `PRESSED=0` | Key down / key up, and encoder push | Send both edges. Companion runs press actions on down and release actions on up. |
| `KEY-ROTATE DEVICEID="…" CONTROLID="0/2" DIRECTION=1` / `-1` | Per encoder detent | See [rotation amounts](#rotation-amounts). |
| `PING <payload>` | Every 2 s | Companion replies `PONG <payload>`. |
| `PONG <payload>` | In reply to a server `PING` | Not observed from Companion 5.1.3, but required by the API. |
| `QUIT` | Clean shutdown | Companion drops the socket and removes the device. |

### Companion → device

| Message | When | Firmware action |
|---|---|---|
| `BEGIN CompanionVersion="…" ApiVersion="1.14.0"` | Immediately on connect | Log it. |
| `CAPS SUBSCRIPTIONS=0 NONSQUARE=1 BITMAP_FORMATS="rgb,png,webp" ROTARY_AMOUNT=1` | Right after `BEGIN` | Refuse to register if `NONSQUARE` is missing, because the 128×64 OLED preset needs it. |
| `ADD-DEVICE OK DEVICEID="…"` / `ADD-DEVICE ERROR … MESSAGE="…"` | Reply to registration | On error: log, drop the socket, back off. |
| `BRIGHTNESS DEVICEID="…" VALUE=100` | Right after `ADD-DEVICE OK`, and when changed in Companion | Set the backlight to 0–100. |
| `KEY-STATE DEVICEID="…" CONTROLID="0/0" LOCATION="1/0/0" PRESSED=0 TYPE="BUTTON" BITMAP="…"` | All controls on registration, then on every change | Draw it. The encoder control also carries `TEXT="<base64>" FONT_SIZE="auto"` because its preset requests text. |
| `KEY-PRESS OK DEVICEID="…"`, `KEY-ROTATE OK DEVICEID="…"` | After each input message | Ignore, but don't treat them as unknown or as errors. |
| `KEYS-CLEAR DEVICEID="…"` | Companion clears the surface | Blank every control. |
| `PING <payload>` | Optional | Reply `PONG <payload>`. |
| `ERROR MESSAGE="…"` | Unknown command | Log it. |

A real session (`tools/fake-deck/out/run1/protocol.log`, bitmaps shortened to their size):

```text
<- BEGIN CompanionVersion="5.1.3+10036-stable-93ea3fa61c" ApiVersion="1.14.0"
<- CAPS SUBSCRIPTIONS=0 NONSQUARE=1 BITMAP_FORMATS="rgb,png,webp" ROTARY_AMOUNT=1
-> ADD-DEVICE DEVICEID="fake-deck-01" SERIAL="opendeck:fake-deck-01" PRODUCT_NAME="opendeck fake deck" LAYOUT_MANIFEST="eyJzdHlsZVByZXNldHMi…"
<- ADD-DEVICE OK DEVICEID="fake-deck-01"
<- BRIGHTNESS DEVICEID="fake-deck-01" VALUE=100
<- KEY-STATE DEVICEID="fake-deck-01" CONTROLID="0/0" LOCATION="1/0/0" PRESSED=0 TYPE="BUTTON" BITMAP=<20736 bytes>
<- KEY-STATE DEVICEID="fake-deck-01" CONTROLID="0/2" LOCATION="1/0/2" PRESSED=0 TYPE="BUTTON" BITMAP=<32768 bytes> TEXT="Vm9sIDA=" FONT_SIZE="auto"
-> KEY-PRESS DEVICEID="fake-deck-01" CONTROLID="0/0" PRESSED=1
<- KEY-PRESS OK DEVICEID="fake-deck-01"
<- KEY-STATE DEVICEID="fake-deck-01" CONTROLID="0/0" LOCATION="1/0/0" PRESSED=1 TYPE="BUTTON" BITMAP=<20736 bytes>
<- KEY-STATE DEVICEID="fake-deck-01" CONTROLID="0/2" … TEXT="Vm9sIDE=" …        # action ran, the encoder's "Vol 1" image updated
-> KEY-PRESS DEVICEID="fake-deck-01" CONTROLID="0/0" PRESSED=0
-> KEY-ROTATE DEVICEID="fake-deck-01" CONTROLID="0/2" DIRECTION=1
<- KEY-ROTATE OK DEVICEID="fake-deck-01"
<- KEY-STATE DEVICEID="fake-deck-01" CONTROLID="0/2" … TEXT="Vm9sIDI=" …        # "Vol 2"
```

A press triggers several `KEY-STATE` messages: one for the pressed state of the key itself, and one for each control whose content changed. In the test page all three controls show the same variable, so one press redrew all three. The firmware must keep up with bursts of full-control redraws.

## Layout manifest

`LAYOUT_MANIFEST` is base64 of this JSON. The control IDs follow Companion's own `row/column` convention and must match `^[a-zA-Z0-9\-/]+$` *(source)*.

```json
{
  "stylePresets": {
    "default": {},
    "key": { "bitmap": { "w": 72, "h": 72 } },
    "encoder": { "bitmap": { "w": 128, "h": 64 }, "text": true, "textStyle": true }
  },
  "controls": {
    "0/0": { "row": 0, "column": 0, "stylePreset": "key" },
    "0/1": { "row": 0, "column": 1, "stylePreset": "key" },
    "0/2": { "row": 0, "column": 2, "stylePreset": "encoder" }
  }
}
```

- The surface maps onto Companion's page grid by row and column. Page 1 row 0 columns 0–2 drive the deck (`LOCATION="1/0/0"` …).
- A new Companion page has **Page up / Page number / Page down** buttons in column 0. Our key `0/0` lands on Page up until the operator replaces it, and pressing it changes the page.
- The encoder is an ordinary Companion button with **Rotary actions** enabled (button Options). Rotation runs its *rotate left* / *rotate right* actions, and push runs its press/release actions. Nothing encoder-specific is declared in the manifest.
- Once the firmware knows the real ST7735 size (P1), it changes only the `key` preset's `w`/`h`. Companion renders any requested size, including 80×80 and 128×128 (both tested).
- The firmware can drop `"text": true, "textStyle": true` if it follows the OLED recommendation below. They were requested in P0 only for the comparison.

## Images

`rgb` is 8-bit raw RGB, row-major and top-left first, base64-encoded in `BITMAP`. Its size is always exactly `w × h × 3` bytes; the fake deck rejects anything else and never saw a mismatch. With `BITMAP_FORMAT="png"`, `BITMAP` is a `data:image/png;base64,…` URL instead.

Measured per `KEY-STATE` (`tools/fake-deck/out/run*/protocol.log`):

| Control | Size | Raw RGB | `rgb` on the wire (base64) | `png` on the wire |
|---|---|---|---|---|
| LCD key (default) | 72×72 | 15,552 B | 20,736 B | 1,650–2,222 B |
| LCD key (ST7735 candidate) | 80×80 | 19,200 B | 25,600 B | not measured |
| LCD key (ST7735 candidate) | 128×128 | 49,152 B | 65,536 B | not measured |
| Encoder / OLED | 128×64 | 24,576 B | 32,768 B | 3,838–4,894 B |

Firmware memory budget: the line buffer must hold the largest `KEY-STATE` line, about 33 KB for the OLED, or about 66 KB with 128×128 keys. Add the decoded frame (raw size above) or decode base64 in streaming fashion straight to the display. An ST7735 wants RGB565, so convert 3 bytes to 2 per pixel while writing. That halves the frame (72×72 → 10,368 B). PNG cuts the wire size about 10×, but needs an inflate implementation and its working memory (about 32 KB window) on the device. P2 weighs that against the 10× bandwidth saving on the chosen board (see `board-and-framework-decision.md`).

Contact sheet of what Companion sent (keys A/B 72×72, encoder 128×64, then the 1-bit OLED preview):

![Images received from Companion](satellite-spike-images.png)

Companion draws a top bar with the button location (`1/0/2`) on every image by default. It is per-button styling in Companion (**Canvas → decoration**), not a protocol option. On a 64 px OLED it takes about 10 px.

## OLED: image vs TEXT

**Recommendation: use the bitmap.** Request a 128×64 bitmap for the encoder control, threshold it to 1-bit on the device (luma ≥ 128 → on), and push 1,024 bytes to the SSD1306/SH1106. The fake deck's `0-2-mono.png` shows that Companion's anti-aliased white-on-black text stays fully legible after a plain threshold (contact sheet above, right-hand image).

Why not `TEXT`:

- `TEXT` carries only the button's text (`"Vol 0"`, base64). Colours, icons, images, layout and Companion's font scaling are lost, so the device would need its own font renderer and layout rules to approximate what Companion already rendered.
- The bitmap is exactly what the operator sees in Companion's editor, so what you design is what you get.
- Bandwidth isn't a reason either: an OLED update is 32,768 B of base64 RGB, the same order as one key update, and only on change.

The caveat is that coloured backgrounds threshold badly: a mid-luma colour such as the red key, at luma 61, turns black. OLED buttons should be designed white-on-black in Companion, or the device could dither instead of threshold. Text stays an option for later, for example local-menu labels drawn by the firmware itself (R10), which never come from Companion anyway.

## Reconnect behaviour

| Scenario | What happened (`out/run-reconnect`, `out/run-noping`) |
|---|---|
| Client silent (`--no-ping`) | Companion closed the socket **5.0 s** after the client's last line (45.690 → 50.695). |
| Companion stopped (`docker stop`) | The socket closed when the container exited (FIN/RST). The client saw `disconnected` at once, then got `ECONNREFUSED` on each retry. |
| Companion back up | Retries at 1, 2, 4, 8 s backoff; it re-registered 7 s after the container started, because the 8 s backoff step overshot. |
| Half-open link (`docker pause`) | No FIN. The client's 5 s silence watchdog fired 5 s after the last `PONG` and it reconnected. (Docker's port proxy accepted the TCP connect while paused. `BEGIN` arrived only after unpause, so a real dead host would show a connect timeout instead.) |
| After re-registration | Companion re-sent `KEY-STATE` for all three controls within 100 ms of `ADD-DEVICE OK`. The surface stayed on page 1 with the same settings, because it is remembered by `SERIAL`. |

Firmware policy (KTD7, confirmed):

1. `PING` every 2 s. This must stay well under Companion's 5 s idle timeout.
2. Treat 5 s without any line from Companion as a dead link: close and reconnect. A healthy link always has a `PONG` every 2 s.
3. Reconnect with backoff, and **re-send `ADD-DEVICE` on every connection**. Device state never survives a disconnect.
4. Show the disconnected/searching state (R7) from the moment of disconnect until `ADD-DEVICE OK`.
5. Give each TCP connect attempt its own timeout of about 5 s. A Companion host that vanishes without a FIN or RST (powered off, Wi-Fi gone, firewall DROP) otherwise holds every attempt for the stack's SYN timeout, which is about 2 minutes on Linux, and the backoff never bounds recovery. The fake deck does this with `socket.setTimeout` until `connect`.
6. Cap the backoff at about 5 s instead of 10 s. With the 8 s step, recovery after a Companion restart took 7 s when 2–3 s was possible. AE1 asks for automatic recovery and shorter is better. A home LAN does not need the larger cap.

## On hardware (P2)

Measured on 2026-11-15 with the P2 firmware on a Feather ESP32 V2 (ESP32-PICO-V3-02, rev v3.0) and one ScreenKey. It ran against the same Companion v5.1.3 container over WPA2 Wi-Fi, with the deck on a different subnet from the desktop, routed. Times come from host-timestamped serial logs and the deck's own log lines.

| Measurement | Result |
|---|---|
| One 128×128 `rgb` key image (65,646 B line) | received in 193–357 ms, drawn in 40–45 ms |
| Key press to Companion's redrawn image | about 330 ms after release, then about 45 ms to draw |
| Free heap | 135,628 B at boot, about 90,000 B after registration (72 KB line buffer static) |
| Companion stopped (`docker stop`) | link closed the moment the container exited; key back to "Connecting" |
| Companion started again | registered 1.8 s after `docker start`, image 2.2 s after it |
| Half-open link (`docker pause`) | silence watchdog fired 4.8 s after the pause; registered 0.1 s after `docker unpause` |
| Host changed over serial | old link dropped at once (`HostChanged`); registered 90 ms after a correct `host` command |
| Unreachable host | each connect attempt times out after 5 s and is retried with backoff |
| Wi-Fi lost (access point off for about 1 min 45 s) | IPv4 went down at the disconnect, so the key shows "No Wi-Fi". Rejoin attempts ran every 6.6 s. After the access point returned, the deck rejoined (on another mesh node) and registered with Companion 0.55 s after the Wi-Fi join; the key image followed at 0.73 s. |
| Presses while not registered | dropped, not queued. Companion's count did not change, and no `KEY-PRESS` was sent after reconnecting. |

**Differences from the fake deck:**

- **First connection after a deck reboot.** The new connection gets `BEGIN` and `CAPS`, and the deck sends `ADD-DEVICE`. Companion then closes the device's stale connection from before the reboot, but never answers the new `ADD-DEVICE`. After 5 s the deck's silence watchdog closes it, and the retry 1 s later registers normally. So a reboot costs about 6 s before the key shows Companion's image. A fix could re-send `ADD-DEVICE` after a short timeout; it's left out because recovery is already automatic (R7).
- **Startup hang (fixed).** With xtensa-lx-rt 0.23.0, every esp-hal app hung before `main` on this chip. The firmware uses a patched copy; see `firmware/vendor/README.md` and THI-77.

## Rotation amounts

`CAPS` advertises `ROTARY_AMOUNT=1`, and Companion passes the signed step count from `DIRECTION` to the button (clamped to ±100 *(source)*). But `DIRECTION=3` ran the button's *rotate right* action **once** (count 2 → 3), not three times. Actions see one rotate event. Firmware should therefore send one `KEY-ROTATE` with `DIRECTION=±1` per detent, unless Companion-side actions are written to read the step amount. Coalescing several detents into one message would silently drop steps.

## Minimum versions

Tested: Companion 5.1.3 / API 1.14.0 only. The features used, with the API version that introduced each (Satellite API docs):

| Feature used | API | Companion |
|---|---|---|
| `KEY-ROTATE` | 1.3 | early 3.x |
| `BRIGHTNESS` | 1.7 | 3.4 |
| `LAYOUT_MANIFEST`, `CONTROLID` | 1.9 | 4.2 |
| `CAPS`, `SERIAL` | 1.10 | 4.3 |
| `NONSQUARE` (128×64 OLED preset) | 1.11 | 5.0 |
| `BITMAP_FORMAT=png` (optional) | 1.12 | 5.0 |

So the documented floor for this design is **API 1.11 / Companion 5.0**, set by the non-square OLED bitmap. The firmware should check `NONSQUARE` in `CAPS` rather than parse versions.

## Left out on purpose

These are not needed for the PoC. They can be adopted later by version: subscriptions (`ADD-SUB` …, 1.10), variables (`VARIABLES`, `SET-VARIABLE-VALUE`, 1.7), pincode lock (`PINCODE_LOCK`, 1.8), device config fields (`CONFIG_FIELDS`, 1.10), `CHANGE-PAGE` (1.10), `FIRMWARE-UPDATE-INFO` (1.10, possibly useful for R12 OTA), LED rings (`leds`, 1.13), `DEVICE-APPEARANCE` (1.15), the bitmap cache (1.16), `webp` bitmaps (1.12), and the WebSocket transport on port 16623. API 1.15/1.16 need Companion 5.2, which was not tested.
