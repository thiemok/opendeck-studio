# fake-deck

A desktop stand-in for the opendeck hardware. It registers with Bitfocus Companion as a Satellite surface with two LCD keys and one encoder/OLED control. It saves every image Companion pushes as a PNG, and sends key presses and encoder rotate/push from a small REPL. It is the P0 protocol spike (THI-71). The findings it produced are in [`docs/notes/satellite-protocol-firmware-notes.md`](../../docs/notes/satellite-protocol-firmware-notes.md).

Requires Node 24+ (runs the TypeScript directly) and Docker.

## 1. Start Companion

```bash
npm ci                         # once per worktree
npm run companion              # docker compose up -d, then loads the test layout into a fresh Companion
```

| What | Where |
|---|---|
| Web UI | http://localhost:18000 |
| Satellite TCP | `localhost:26622` (container port 16622) |
| Config/data | Docker volume `opendeck-companion-data`, shared by all worktrees |

The ports are remapped so this never clashes with a Companion already running on the desktop. Satellite TCP is on by default.

Verified on 2026-11-13: `BEGIN CompanionVersion="5.1.3+10036-stable-93ea3fa61c" ApiVersion="1.14.0"`, then `CAPS SUBSCRIPTIONS=0 NONSQUARE=1 BITMAP_FORMATS="rgb,png,webp" ROTARY_AMOUNT=1`.

### Load the test buttons

`npm run companion` waits for Companion to come up. If its data is fresh (the first-run wizard was never done), it seeds it: it marks the wizard done with Companion's defaults, creates the custom variables `count` and `presses`, and imports the hardware deck page. A Companion that is already set up is left alone.

Companion's data lives in the named Docker volume `opendeck-companion-data`, not in the worktree. Every worktree's `docker compose up -d` uses the same container and the same volume, so a new worktree finds Companion already set up, with the pages and variables it had before. The seed runs once per volume. To start over, stop Companion and remove the volume: `docker compose down && docker volume rm opendeck-companion-data`, then `npm run companion`.

Because the data is shared, a page you change in one worktree is changed for all of them.

To load a different layout, or to put a page back after editing it, run the seed directly. It replaces the layout's pages each time:

```bash
npm run seed                      # hardware deck page on page 1 (default)
npm run seed -- --layout fake     # the fake deck's page on page 1
npm run seed -- --layout bench    # the two bench pages on pages 1 and 2
```

The seed script (`src/seed-companion.ts`) uses the same WebSocket API as Companion's web UI, because Companion has no HTTP endpoint for imports. Those procedure names are internal to Companion and match the version pinned in `compose.yaml`. After bumping the image, run `npm run seed` once to check that it still works.

To load a page by hand instead: finish the first-run wizard, create the custom variables `count` and `presses` under **Variables → Custom Variables**, then **Import / Export → Import configuration**, pick a file from `companion-config/` and choose **Replace page 1 with imported page**. The hardware page (`hardware-deck-page.companionconfig`) holds the same buttons as the fake deck's page, one column to the right (`0/1` key A, `0/2` key B, `0/3` encoder). Its keys count their own presses in `presses`, so a turn redraws only the OLED and a press only the keys.

With `--layout fake`, page 1, row 0 holds:

| Control | Shows | Press | Rotate |
|---|---|---|---|
| `0/0` key A | red, `A <count>` | `count + 1` | – |
| `0/1` key B | blue, `B <count>` | `count - 1` | – |
| `0/2` encoder | black, `Vol <count>` | `count = 0` | right `+1`, left `-1` |

All three controls show `count`, so any input visibly changes every image. `companion-config/make-page.ts` generates the import files: `node companion-config/make-page.ts > companion-config/fake-deck-page.companionconfig`, with `--hardware` for `hardware-deck-page.companionconfig`, and with `--bench 1` / `--bench 2` for the two bench pages below.

### Bench pages (THI-76, THI-85)

`bench-page-1.companionconfig` and `bench-page-2.companionconfig` fill all 32 positions of the 8×4 grid for measuring page changes with the firmware's `bench 16|32` surface. `npm run seed -- --layout bench` loads both. By hand, import the first with **Replace page 1 with imported page**, then the second as **[ Insert new page ]** so it becomes page 2. On both pages `0/0` is Companion's page up and `0/1` its page down. Every other button has its own colour and `P<page> <row>/<column>` text, and each one changes between the pages, so a page change redraws every key. A 16-key surface shows rows 0–1 of the same pages. Pressing `0/0` and `0/1` alternately moves between the two pages and never onto an empty page 3.

### Companion MCP (optional)

The repo's `.mcp.json` registers the community [drohi-r/companion-mcp](https://github.com/drohi-r/companion-mcp) server (Apache-2.0) as `companion` for Claude Code. It points at this container (`127.0.0.1:18000`), and agents can use it to inspect pages, press buttons and set variables without the web UI.

- **Pinned:** commit `ef26ca8`, run through `uvx`. It needs `uv`. The server declares `mcp>=1.0` with no upper bound, and mcp 2.x renamed `FastMCP`, so `.mcp.json` adds `--with "mcp>=1,<2"`.
- **Safety:** only localhost is allowed (`COMPANION_ALLOWED_HOSTS`). Snapshots and presets go to the gitignored `out/companion-mcp/`. Set `COMPANION_WRITE_ENABLED=0` for read-only use.
- **Limit with Companion 5:** the imported test buttons are `button-layered`, and the server reads their text and colours as `null`. Page discovery, presses and variables work. Treat its style tools as unverified on layered buttons, and use the import file or the web UI to change button layouts.
- **First use:** Claude Code asks you to approve the project's MCP servers, and new servers load at the next session start.

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
