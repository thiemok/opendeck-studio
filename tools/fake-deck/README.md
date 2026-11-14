# fake-deck

A desktop stand-in for the opendeck hardware. It registers with Bitfocus Companion as a Satellite surface with two LCD keys and one encoder/OLED control. It saves every image Companion pushes as a PNG, and sends key presses and encoder rotate/push from a small REPL. It is the P0 protocol spike (THI-71). The findings it produced are in [`docs/notes/satellite-protocol-firmware-notes.md`](../../docs/notes/satellite-protocol-firmware-notes.md).

Requires Node 24+ (runs the TypeScript directly) and Docker.

## 1. Start Companion

```bash
docker compose up -d          # Companion v5.1.3, pinned in compose.yaml
```

| What | Where |
|---|---|
| Web UI | http://localhost:18000 |
| Satellite TCP | `localhost:26622` (container port 16622) |
| Config/data | `./companion-data/` (gitignored) |

The ports are remapped so this never clashes with a Companion already running on the desktop. Satellite TCP is on by default.

Verified on 2026-11-13: `BEGIN CompanionVersion="5.1.3+10036-stable-93ea3fa61c" ApiVersion="1.14.0"`, then `CAPS SUBSCRIPTIONS=0 NONSQUARE=1 BITMAP_FORMATS="rgb,png,webp" ROTARY_AMOUNT=1`.

### Load the test buttons

1. Finish the first-run wizard in the web UI.
2. **Variables → Custom Variables**: create a variable named `count`.
3. **Import / Export → Import configuration**: pick `companion-config/fake-deck-page.companionconfig`, then **Replace page 1 with imported page**.

Page 1, row 0 then holds:

| Control | Shows | Press | Rotate |
|---|---|---|---|
| `0/0` key A | red, `A <count>` | `count + 1` | – |
| `0/1` key B | blue, `B <count>` | `count - 1` | – |
| `0/2` encoder | black, `Vol <count>` | `count = 0` | right `+1`, left `-1` |

All three controls show `count`, so any input visibly changes every image. `companion-config/make-page.ts` generates the import file: `node companion-config/make-page.ts > companion-config/fake-deck-page.companionconfig`.

## 2. Run the fake deck

```bash
npm install
npm start -- --port 26622     # same as: node src/main.ts --port 26622
```

```text
deck> press 0        # press + release key A
deck> press 1        # key B
deck> rotate +1      # encoder, n detents (+/-), one KEY-ROTATE DIRECTION=±1 each
deck> push           # press + release encoder
deck> quit           # sends QUIT
```

Commands run one at a time, and any typed or piped before the deck has registered wait until it has. Scripted runs work too: `printf 'press 0\nrotate +3\n' | npm start -- --port 26622`. At end of input the deck sends QUIT, so Companion's final image updates may arrive after it has gone.

Options (`--help`): `--host`, `--port`, `--key-size <px>` (square key bitmaps, default 72), `--format rgb|png`, `--device-id`, `--out <dir>`, `--no-ping`.

`--no-ping` sends no keepalive and disables the 5 s silence watchdog, so only the server ends the session. Use it to measure Companion's idle timeout.

### Output (`out/`, gitignored)

- `0-0.png`, `0-1.png`, `0-2.png`: the latest image per control.
- `0-2-mono.png`: the encoder image thresholded to 1-bit, as a mono OLED would show it.
- `protocol.log`: every line in (`<-`) and out (`->`), with timestamps. Bitmaps appear as their byte size, and session events start with `##`.

## 3. Tests

```bash
npm test             # node --test: codec, session against a scripted fake Companion, image sink
npm run typecheck    # tsc --noEmit
```
