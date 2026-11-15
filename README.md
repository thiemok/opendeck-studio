# opendeck-studio

A self-built network control deck in the spirit of the Stream Deck Studio. It appears in [Bitfocus Companion](https://bitfocus.io/companion) as a surface over the Satellite protocol. Companion runs on a Linux desktop or server, and the deck only shows images and reports input.

Current phase: proof of concept on Wi-Fi with off-the-shelf parts (two ST7735 LCD keys, a 1.3" OLED with rotary encoder, on a Feather ESP32 V2 running Rust no_std firmware). Ethernet/PoE, scaling toward ~32 keys and a custom PCB come after the final-shape decision.

The requirements and stage plan are in [`docs/plans/2026-11-13-1722-feat-companion-network-deck-poc-plan.md`](docs/plans/2026-11-13-1722-feat-companion-network-deck-poc-plan.md).

| Stage | Goal |
|---|---|
| P0 | Desktop "fake deck" script learns the Satellite protocol |
| P1 | Board and framework decision; identify the key module (done: [`docs/notes/board-and-framework-decision.md`](docs/notes/board-and-framework-decision.md)) |
| P2 | First LCD key end-to-end on the chosen board |
| P3 | Full PoC: two keys, encoder, OLED with local menu |
| P4 | Password-gated OTA and measured findings for the final-shape decision |

## Checks

Every PR to `main`, and every push to it, runs three jobs in [`.github/workflows/ci.yml`](.github/workflows/ci.yml):

| Job | Runs | Local commands |
|---|---|---|
| `deck-core` | `cargo fmt --check`, `cargo clippy --locked --all-targets -- -D warnings`, `cargo test --locked` at the repo root | `mise exec -- cargo …` at the repo root |
| `firmware` | `cargo fmt --check`, `cargo build --release --locked`, `cargo clippy --release --locked -- -D warnings` in `firmware/` | [`firmware/README.md`](firmware/README.md#checks) |
| `fake-deck` | `npm test`, `npm run typecheck` in `tools/fake-deck/` | [`tools/fake-deck/README.md`](tools/fake-deck/README.md) |

`main` requires all three to pass, takes changes only through PRs, and refuses force-pushes and deletion, with no bypass. The ruleset lives in [`.github/rulesets/main.json`](.github/rulesets/main.json). After editing it, apply it with `gh api repos/thiemok/opendeck-studio/rulesets/<id> --method PUT --input .github/rulesets/main.json`; `gh api repos/thiemok/opendeck-studio/rulesets` lists the id. Renaming a CI job means updating the ruleset in the same change.

CI installs its tools through mise like a local setup does, and it uses no repository secrets. Hardware tests aren't part of CI; bench checks stay manual.
