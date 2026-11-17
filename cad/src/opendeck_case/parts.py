"""Solid parts: key-area tiles, the side tile and the tilt wedges.

Tiles are built in assembled coordinates (see `layout.py`), so an assembly is
just the tiles together and joint fit can be checked by intersecting them.
"""

import math

from build123d import (
    Axis,
    Box,
    Circle,
    Part,
    Pos,
    Rectangle,
    Rot,
    Sketch,
    Text,
    extrude,
    fillet,
)

from .joints import sockets, tabs
from .layout import Layout, Tile

_NAMES = {"block_4x8": "{rows}x{columns} block", "strip_1x16": "{rows}x{columns} strip"}


def label_text(lay: Layout) -> str:
    p = lay.params
    name = _NAMES.get(p.name, p.name).format(rows=p.rows, columns=p.columns)
    return f"{name}  {lay.outer_width:.0f} x {lay.outer_depth:.0f} mm"


def _ring(width: float, depth: float, groove: float) -> Sketch:
    return Rectangle(width, depth) - Rectangle(width - 2 * groove, depth - 2 * groove)


def _engrave(part: Part, marks: Sketch, lay: Layout) -> Part:
    p = lay.params
    return part - Pos(0, 0, p.thickness - p.engrave_depth) * extrude(marks, p.engrave_depth + 1)


def _plate(tile: Tile, lay: Layout) -> Part:
    p = lay.params
    return Pos(tile.x0, 0, 0) * Box(
        tile.width, lay.outer_depth, p.thickness, align=None
    )


def key_tile(lay: Layout, index: int) -> Part:
    p = lay.params
    key_tiles = [t for t in lay.tiles if t.kind == "keys"]
    tile = key_tiles[index]
    part = _plate(tile, lay)
    if tile.tab_right:
        part += tabs(lay, tile.x1, 1)
    if index > 0:
        part -= sockets(lay, tile.x0, 1)  # the previous tile's tabs
    if index == 0:
        part -= sockets(lay, 0.0, 1)  # side tile turned round to the left
    if index == len(key_tiles) - 1:
        part -= sockets(lay, lay.key_area_width, -1)  # side tile on the right

    kw, kd = p.keycap
    keycaps = [
        Pos(x, y) * _ring(kw, kd, p.groove_width)
        for x, y in lay.keycap_centres
        if tile.x0 < x < tile.x1
    ]
    part = _engrave(part, Sketch() + keycaps, lay)

    if index == 0:
        text = Text(label_text(lay), font_size=p.label_size)
        at = Pos((tile.x0 + tile.x1) / 2, p.border / 2, p.thickness)
        part += at * extrude(text, p.emboss_height)
    return part


def side_tile(lay: Layout) -> Part:
    """The OLED/dial tile, in its dial-on-the-right position."""
    p = lay.params
    tile = lay.tiles[-1]
    part = _plate(tile, lay)
    far_corners = part.edges().filter_by(Axis.Z).group_by(Axis.X)[-1]
    part = fillet(far_corners, radius=4.0)
    part += tabs(lay, tile.x0, -1)

    mx, my = lay.module_centre
    g = p.groove_width
    marks = Sketch() + [
        Pos(mx, my) * _ring(*p.module, g),
        Pos(mx + p.oled_offset, my) * _ring(*p.oled_window, g),
        Pos(mx + p.dial_offset, my)
        * (Circle(p.dial_diameter / 2) - Circle(p.dial_diameter / 2 - g)),
    ]
    return _engrave(part, marks, lay)


def side_tile_on_left(lay: Layout, side: Part) -> Part:
    """The side tile turned round to sit on the key area's left end (R8)."""
    mx, my = lay.module_centre
    turned = Pos(mx, my) * Rot(Z=180) * Pos(-mx, -my) * side
    return Pos(-lay.outer_width, 0, 0) * turned


def deck_tiles(lay: Layout) -> list[Part]:
    """Every tile in assembled position, left to right, dial on the right."""
    n = sum(t.kind == "keys" for t in lay.tiles)
    return [key_tile(lay, i) for i in range(n)] + [side_tile(lay)]


def plate_slab(lay: Layout, angle: float, grow: float = 0.0) -> Part:
    """The deck's plate cross-section tilted at `angle`, front edge on the desk.

    Wedge-local coordinates: x across the wedge, y towards the back.
    """
    p = lay.params
    slab = Pos(-50, -grow, -grow) * Box(
        p.wedge_width + 100, lay.outer_depth + 2 * grow, p.thickness + 2 * grow, align=None
    )
    return Rot(X=angle) * slab


def wedge(lay: Layout, angle: float) -> Part:
    """A clip-on wedge that holds a tile's back edge at `angle` (KTD5)."""
    p = lay.params
    a = math.radians(angle)
    d, t = lay.outer_depth, p.thickness
    front = (d - p.wedge_grip) * math.cos(a)
    back = d * math.cos(a) + p.wedge_wall
    top = d * math.sin(a) + t * math.cos(a) + p.wedge_wall
    block = Pos(0, front, 0) * Box(p.wedge_width, back - front, top, align=None)
    return block - plate_slab(lay, angle, grow=p.joint_clearance)
