"""Named print sets: which parts a build produces, and what to check on them."""

from collections.abc import Callable
from dataclasses import dataclass, field, replace
from pathlib import Path

from build123d import Part

from . import checks
from .layout import layout
from .params import VARIANTS, Bed, DeckParams
from .parts import coupon_socket, coupon_tab, deck_tiles, side_tile_on_left, wedge_for_print

JOINT_FIT_CLEARANCES = (0.0, 0.05, 0.1, 0.15)
JOINT_FIT_VARIANT = "block_2x8"  # whose dovetail the coupons copy


@dataclass(frozen=True)
class PrintSet:
    name: str
    description: str
    variants: tuple[str, ...]
    joint_fit: bool = False


PRINT_SETS = {
    ps.name: ps
    for ps in (
        PrintSet(
            name="thi-82",
            description="Flat desk mock-ups of the 4x8 block, 1x16 strip and 2x8 block (THI-82)",
            variants=("block_4x8", "strip_1x16", "block_2x8"),
        ),
        PrintSet(
            name="thi-82-2x8",
            description="The 2x8 block mock-up plus joint-fit coupons (THI-82, second round)",
            variants=("block_2x8",),
            joint_fit=True,
        ),
    )
}


@dataclass
class BuiltPart:
    name: str
    part: Part
    quantity: int = 1


@dataclass
class BuiltGroup:
    """Parts printed together, their assembly preview and the checks that judge them."""

    name: str
    bed: Bed
    parts: list[BuiltPart]
    assembly: list[Part]
    outer_mm: tuple[float, float] | None = None
    checks: dict[str, Callable[[], bool]] = field(default_factory=dict)


def _params(name: str, bed: Bed | None) -> DeckParams:
    params = VARIANTS[name]()
    return params if bed is None else replace(params, bed=bed)


def build_variant(name: str, bed: Bed | None = None) -> BuiltGroup:
    lay = layout(_params(name, bed))
    tiles = deck_tiles(lay)
    parts = [BuiltPart(t.name, part) for t, part in zip(lay.tiles, tiles)]
    parts += [
        BuiltPart(f"wedge-{angle:g}", wedge_for_print(lay, angle), quantity=lay.wedge_count())
        for angle in lay.params.wedge_angles
    ]
    left = side_tile_on_left(lay, tiles[-1])
    return BuiltGroup(
        name=name,
        bed=lay.params.bed,
        parts=parts,
        assembly=tiles,
        outer_mm=(round(lay.outer_width, 2), round(lay.outer_depth, 2)),
        checks={
            "outline": lambda: checks.outline_matches(tiles, lay),
            "joints_dial_right": lambda: checks.joints_clear(tiles),
            "joints_dial_left": lambda: checks.joints_clear([left, tiles[0]]),
        },
    )


def build_joint_fit(bed: Bed | None = None) -> BuiltGroup:
    """One tab block per clearance and a socket block for each, labelled."""
    params = _params(JOINT_FIT_VARIANT, bed)
    tab = coupon_tab(params)
    sockets = {c: coupon_socket(params, c) for c in JOINT_FIT_CLEARANCES}
    parts = [BuiltPart("tab", tab, quantity=len(sockets))]
    parts += [BuiltPart(f"socket-{c:.2f}", socket) for c, socket in sockets.items()]
    return BuiltGroup(
        name="joint-fit",
        bed=params.bed,
        parts=parts,
        assembly=[tab, sockets[JOINT_FIT_CLEARANCES[0]]],
        checks={
            f"joint_{c:.2f}": (lambda s=s: checks.joints_clear([tab, s]))
            for c, s in sockets.items()
        },
    )


def default_out_dir(set_name: str) -> Path:
    return Path(__file__).resolve().parents[2] / "out" / set_name
