"""Solid parts: key-area tiles, the side tile and the tilt wedges.

Tiles are built in assembled coordinates (see `layout.py`), so an assembly is
just the tiles together and joint fit can be checked by intersecting them.
"""

import math
from dataclasses import replace

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
from .layout import Layout, Tile, layout
from .params import DeckParams

def label_text(lay: Layout) -> str:
    p = lay.params
    shape = "strip" if p.rows == 1 else "block"
    return f"{p.rows}x{p.columns} {shape}  {lay.outer_width:.0f} x {lay.outer_depth:.0f} mm"


def _ring(width: float, depth: float, groove: float) -> Sketch:
    return Rectangle(width, depth) - Rectangle(width - 2 * groove, depth - 2 * groove)


def _engrave(part: Part, marks: Sketch, lay: Layout) -> Part:
    p = lay.params
    return part - Pos(0, 0, p.thickness - p.engrave_depth) * extrude(marks, p.engrave_depth + 1)


def _cut_through(part: Part, holes: Sketch, lay: Layout) -> Part:
    return part - Pos(0, 0, -1) * extrude(holes, lay.params.thickness + 2)


def _plate(tile: Tile, lay: Layout) -> Part:
    p = lay.params
    return Pos(tile.x0, 0, 0) * Box(
        tile.width, lay.outer_depth, p.thickness, align=None
    )


def key_tile(lay: Layout, index: int) -> Part:
    p = lay.params
    tile = lay.key_tiles[index]
    part = _plate(tile, lay)
    if tile.tab_right:
        part += tabs(lay, tile.x1, 1)
    # The left end takes the previous tile's tabs, or the side tile turned round.
    part -= sockets(lay, tile.x0, 1)
    if index == len(lay.key_tiles) - 1:
        part -= sockets(lay, lay.key_area_width, -1)

    kw, kd = (c + 2 * p.hole_clearance for c in p.keycap)
    keycaps = [
        Pos(x, y) * Rectangle(kw, kd) for x, y in lay.keycap_centres if tile.x0 < x < tile.x1
    ]
    part = _cut_through(part, Sketch() + keycaps, lay)

    if index == 0:
        text = Text(label_text(lay), font_size=p.label_size)
        at = Pos((tile.x0 + tile.x1) / 2, p.border / 2, p.thickness)
        part += at * extrude(text, p.emboss_height)
    return part


def side_tile(lay: Layout) -> Part:
    """The OLED/dial tile, in its dial-on-the-right position."""
    p = lay.params
    tile = lay.side_tile
    part = _plate(tile, lay)
    far_corners = part.edges().filter_by(Axis.Z).group_by(Axis.X)[-1]
    part = fillet(far_corners, radius=p.corner_radius)
    part += tabs(lay, tile.x0, -1)

    mx, my = lay.module_centre
    part = _engrave(part, Sketch() + [Pos(mx, my) * _ring(*p.module, p.groove_width)], lay)
    c = p.hole_clearance
    bw, bh = p.module_button

    def at(offset: tuple[float, float]) -> Pos:
        return Pos(mx + offset[0], my + offset[1])

    holes = Sketch() + [
        at(p.oled_offset) * Rectangle(*p.oled_window),
        at(p.encoder_offset) * Circle(p.encoder_bushing / 2 + c),
        *(at(o) * Rectangle(bw + 2 * c, bh + 2 * c) for o in p.module_button_offsets),
        *(at(o) * Circle(p.module_mount_hole / 2) for o in p.module_mount_offsets),
    ]
    return _cut_through(part, holes, lay)


def side_tile_on_left(lay: Layout, side: Part) -> Part:
    """The side tile turned round to sit on the key area's left end (R8)."""
    mx, my = lay.module_centre
    turned = Pos(mx, my) * Rot(Z=180) * Pos(-mx, -my) * side
    return Pos(-lay.outer_width, 0, 0) * turned


def deck_tiles(lay: Layout) -> list[Part]:
    """Every tile in assembled position, left to right, dial on the right."""
    return [key_tile(lay, i) for i in range(len(lay.key_tiles))] + [side_tile(lay)]


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
    back = d * math.cos(a) + p.wedge_wall
    top = d * math.sin(a) + t * math.cos(a) + p.wedge_wall
    # Reach forward under the plate so the wedge stands without tipping.
    front = min((d - p.wedge_grip) * math.cos(a), back - p.wedge_foot * top)
    block = Pos(0, front, 0) * Box(p.wedge_width, back - front, top, align=None)
    block -= plate_slab(lay, angle, grow=p.joint_clearance)
    # Above the plate, keep only the lip over the back border (wedge_grip).
    reach = 500.0
    above = Pos(-50, -reach, t) * Box(p.wedge_width + 100, reach + d - p.wedge_grip, reach, align=None)
    return block - Rot(X=angle) * above


def wedge_for_print(lay: Layout, angle: float) -> Part:
    """The wedge laid on its side: the slot profile builds straight up, no supports."""
    return Rot(Y=90) * wedge(lay, angle)


# A joint-fit coupon: the deck's own dovetail on two small blocks, one per
# side of a seam, sized to one border band pair (both joints of a seam).
COUPON_SEAM = 20.0


def coupon_layout(params: DeckParams) -> Layout:
    """A one-column, keyless layout: its joint centres are the deck's."""
    return layout(replace(params, name="coupon", columns=1, rows=0))


def joint_fit_coupon(params: DeckParams, clearance: float) -> tuple[Part, Part]:
    """A tab block and a socket block cut with `clearance` per side, labelled."""
    lay = coupon_layout(replace(params, joint_clearance=clearance))
    p = lay.params
    w, d = COUPON_SEAM, lay.outer_depth
    tab = Box(w, d, p.thickness, align=None) + tabs(lay, COUPON_SEAM, 1)
    socket = Pos(COUPON_SEAM, 0, 0) * Box(w, d, p.thickness, align=None)
    socket -= sockets(lay, COUPON_SEAM, 1)
    label = Pos(COUPON_SEAM + w / 2 + 2, d / 2) * Text(f"{clearance:.2f}", font_size=4)
    return tab, _engrave(socket, Sketch() + [label], lay)
