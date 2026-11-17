"""Named print sets: which variants and parts a build produces."""

from dataclasses import dataclass, replace
from pathlib import Path

from build123d import Part

from .layout import Layout, layout
from .params import VARIANTS, Bed, DeckParams
from .parts import deck_tiles, side_tile_on_left, wedge_for_print


@dataclass(frozen=True)
class PrintSet:
    name: str
    description: str
    variants: tuple[str, ...]


PRINT_SETS = {
    "thi-82": PrintSet(
        name="thi-82",
        description="Flat desk mock-ups of the 4x8 block and the 1x16 strip (THI-82)",
        variants=("block_4x8", "strip_1x16"),
    ),
}


@dataclass
class BuiltPart:
    name: str
    part: Part
    quantity: int = 1


@dataclass
class BuiltVariant:
    params: DeckParams
    layout: Layout
    tiles: list[Part]
    side_on_left: Part
    parts: list[BuiltPart]


def build_variant(name: str, bed: Bed | None = None) -> BuiltVariant:
    params = VARIANTS[name]()
    if bed is not None:
        params = replace(params, bed=bed)
    lay = layout(params)
    tiles = deck_tiles(lay)
    parts = [BuiltPart(t.name, part) for t, part in zip(lay.tiles, tiles)]
    parts += [
        BuiltPart(f"wedge-{angle:g}", wedge_for_print(lay, angle), quantity=lay.wedge_count())
        for angle in params.wedge_angles
    ]
    return BuiltVariant(params, lay, tiles, side_tile_on_left(lay, tiles[-1]), parts)


def default_out_dir(set_name: str) -> Path:
    return Path(__file__).resolve().parents[2] / "out" / set_name
