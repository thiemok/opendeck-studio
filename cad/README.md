# cad

Parametric case models for the deck, in [build123d](https://build123d.readthedocs.io/) (Python CAD on the OpenCascade kernel). Every part is derived from one set of deck parameters, so changing a pitch, border or printer bed regenerates all of it. The first use is the THI-82 desk mock-ups: flat, full-size plates of the 4×8 block and the 1×16 strip, to choose the deck's shape before any PCB work. The plan is [`docs/plans/2026-11-17-0824-feat-case-model-setup-plan.md`](../docs/plans/2026-11-17-0824-feat-case-model-setup-plan.md).

## Setup and build

Tools come from mise at the repo root: uv is pinned in `mise.toml`, and uv installs its own Python 3.14 and the locked build123d from `uv.lock`. The system Python isn't used.

```bash
mise install                       # once: installs the pinned uv
mise run case -- list              # the print sets
mise run case -- build thi-82      # build, export, preview and check a print set
mise run case-test                 # the tests
```

A build writes into `cad/out/<set>/` (gitignored):

| File | What |
|---|---|
| `<variant>-<part>.3mf` | For the slicer |
| `<variant>-<part>.stl` | Same mesh as STL; the watertight check reads it |
| `<variant>-<part>.step` | For CAD |
| `<variant>-<part>.png` | Top and isometric line drawing of the part |
| `<variant>-assembly.png` | The deck assembled, with its outer size |
| `report.json` | Every part's size, quantity and checks |

The build also prints the report as a table. It exits non-zero if any check fails: a part larger than the bed less a 5 mm margin, a mesh that isn't watertight, an assembled deck that isn't the size the parameters give, or tiles that overlap at a joint. `--bed <mm>` builds for a different square bed.

## The THI-82 print set

`mise run case -- build thi-82` gives, with the defaults (P1S bed, 25 × 28 mm key pitch, 12 mm border, side section sized for the 65.2 × 35.1 mm OLED/encoder module):

| Deck | Assembled | Parts (quantity) |
|---|---|---|
| 4×8 block | 295 × 136 mm | `keys` (1), `side` (1), `wedge-8` (2), `wedge-15` (2) |
| 1×16 strip | 495 × 52 mm | `keys-1` (1), `keys-2` (1), `side` (1), `wedge-8` (3), `wedge-15` (3) |
| 2×8 block | 295 × 80 mm | `keys` (1), `side` (1), `wedge-8` (2), `wedge-15` (2) |

`mise run case -- build thi-82-2x8` builds only the 2×8 block plus the joint-fit pieces, for the second test round.

Each plate is 4 mm thick. Every key, the OLED window, the encoder and the module buttons are cut through the plate, so the parts can be tried in place: key holes are the 22.00 × 25.30 mm keycap plus `hole_clearance` (0.3 mm) per side, which leaves about 2.4 mm of plate between columns and 2.1 mm between rows. The module board outline is engraved 0.6 mm deep, and the first key-area tile has the deck name and size embossed on its front border. The side tile's holes follow the module vendor's dimension drawing (transcribed in [`docs/notes/oled-encoder-module.md`](../docs/notes/oled-encoder-module.md#mechanical-drawing)): the display window, the encoder's 7 mm shaft bushing, both module buttons (BACK, CONFIRM) and the four M3 mounting holes, so the module can be screwed under the plate. Take the knob off, fit the plate over the shaft, and push the knob back on. The display window is the drawn 35.05 × 22.85 glass outline. The button tops sit below the plate surface, so the real case will need button caps.

### Print

Import the `.3mf` files into Bambu Studio. Each part is already oriented for printing at the origin: the plates lie flat, labelled side up, and the wedges lie on their side so the slot builds straight up. No supports are needed.

### Assemble

- Tiles join with dovetails in the top and bottom border bands. Slide each tab into its socket from above, on a flat desk.
- The side tile (OLED and dial) fits either end of the key area. As printed it goes on the right. Turn it round 180° to put the dial on the left.
- Wedges clip onto a tile's back edge and hold it at 8° or 15°. Use one per tile, because the joints keep the tiles in line but not level with each other.

If the joints are too tight or too loose, change `joint_clearance` in `src/opendeck_case/params.py` and rebuild. The default is 0.1 mm per side; 0.2 printed loose on the P1S. To find the right value without reprinting plates, print the `joint-fit` pieces from `thi-82-2x8`: four identical tab blocks and four socket blocks engraved with their clearance (0.00, 0.05, 0.10, 0.15 mm). Push each tab into a socket and pick the tightest one that still goes together by hand.

## Layout

| Module | Role |
|---|---|
| `params.py` | Deck parameters and the two variants |
| `layout.py` | Derived sizes, keycap positions, tile cuts (no CAD) |
| `joints.py`, `parts.py` | Dovetails, tiles and wedges as solids |
| `export.py`, `preview.py` | STL/3MF/STEP and the PNG drawings |
| `checks.py`, `printsets.py`, `cli.py` | Checks, named print sets and the `case` command |
