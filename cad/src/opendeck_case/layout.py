"""Derived dimensions, keycap positions and tile cuts, without CAD geometry.

Coordinates are the assembled deck seen from above, dial on the right:
x runs left to right from the key area's left edge, y from the front edge.
"""

import math
from dataclasses import dataclass
from typing import Literal

from .params import DeckParams


TileKind = Literal["keys", "side"]


class LayoutError(ValueError):
    """The deck cannot be tiled onto the bed without splitting a keycap."""


@dataclass(frozen=True)
class Tile:
    name: str
    kind: TileKind
    x0: float
    x1: float
    first_column: int | None = None
    last_column: int | None = None
    tab_right: bool = False
    print_width: float = 0.0

    @property
    def width(self) -> float:
        return self.x1 - self.x0


@dataclass(frozen=True)
class Layout:
    params: DeckParams
    tiles: tuple[Tile, ...]
    seams: tuple[float, ...]  # x of every joint, including the side tile's
    keycap_centres: tuple[tuple[float, float], ...]
    key_area_width: float
    outer_width: float
    outer_depth: float

    @property
    def key_tiles(self) -> tuple[Tile, ...]:
        return self.tiles[:-1]

    @property
    def side_tile(self) -> Tile:
        return self.tiles[-1]

    @property
    def joint_centres(self) -> tuple[float, float]:
        """Joints sit in the top and bottom border bands (KTD4)."""
        b = self.params.border
        return (b / 2, self.outer_depth - b / 2)

    @property
    def module_centre(self) -> tuple[float, float]:
        side = self.side_tile
        return ((side.x0 + side.x1) / 2, self.outer_depth / 2)

    def wedge_rise(self, angle: float) -> float:
        """Height of the plate's back edge when its front edge rests on the desk."""
        return self.outer_depth * math.sin(math.radians(angle))

    def wedge_count(self) -> int:
        """One wedge per tile per angle: the joints don't keep tiles level (KTD5)."""
        return len(self.tiles)


def layout(p: DeckParams) -> Layout:
    usable_w, usable_d = p.bed.usable
    pitch_x, pitch_y = p.pitch
    depth = p.rows * pitch_y + 2 * p.border
    key_w = p.columns * pitch_x + 2 * p.border

    if depth > usable_d:
        raise LayoutError(
            f"{p.name}: plate depth {depth:g} mm does not fit the bed's "
            f"usable {usable_d:g} mm, and no column cut can shorten it"
        )

    def column_x(c: int) -> float:
        return p.border + c * pitch_x

    # Fewest key-area tiles that fit, cut only in the gap between columns.
    for n in range(1, p.columns + 1):
        bounds = [0] + [round(p.columns * i / n) for i in range(1, n)] + [p.columns]
        edges = [0.0] + [column_x(c) for c in bounds[1:-1]] + [key_w]
        widths = [
            edges[i + 1] - edges[i] + (p.tab_depth if i < n - 1 else 0) for i in range(n)
        ]
        if max(widths) <= usable_w:
            break
    else:
        raise LayoutError(
            f"{p.name}: one key column ({pitch_x:g} mm plus borders) is wider "
            f"than the bed's usable {usable_w:g} mm"
        )

    side_print = p.side_width + p.tab_depth
    if side_print > usable_w:
        raise LayoutError(
            f"{p.name}: the side tile ({side_print:g} mm with its tab) is wider "
            f"than the bed's usable {usable_w:g} mm"
        )

    tiles = []
    for i in range(n):
        tiles.append(
            Tile(
                name=f"keys-{i + 1}" if n > 1 else "keys",
                kind="keys",
                x0=edges[i],
                x1=edges[i + 1],
                first_column=bounds[i],
                last_column=bounds[i + 1] - 1,
                tab_right=i < n - 1,
                print_width=widths[i],
            )
        )
    tiles.append(
        Tile(
            name="side",
            kind="side",
            x0=key_w,
            x1=key_w + p.side_width,
            print_width=side_print,
        )
    )

    centres = tuple(
        (column_x(c) + pitch_x / 2, p.border + r * pitch_y + pitch_y / 2)
        for r in range(p.rows)
        for c in range(p.columns)
    )
    return Layout(
        params=p,
        tiles=tuple(tiles),
        seams=tuple(edges[1:]),
        keycap_centres=centres,
        key_area_width=key_w,
        outer_width=key_w + p.side_width,
        outer_depth=depth,
    )
