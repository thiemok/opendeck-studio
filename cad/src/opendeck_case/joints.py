"""Dovetail joints between tiles (KTD4).

A joint is a trapezoid in the plate's plane: its neck sits on the seam and its
wider head points into the neighbouring tile, so the tiles push together flat
and can't pull apart along x. Every seam gets two, one centred in each of the
top and bottom border bands, which keeps them clear of the keycap marks (the
gap between key columns is only 3 mm).
"""

from build123d import Face, Polygon, Pos, Solid, extrude, offset

from .layout import Layout

# The tab starts this far behind the seam so it fuses with its own tile.
_ROOT = 1.0


def tab_profile(lay: Layout, seam_x: float, y: float, direction: int) -> Face:
    """Tab outline on the seam at `seam_x`, pointing +x (1) or -x (-1)."""
    p = lay.params
    n, h, d = p.tab_neck / 2, p.tab_head / 2, p.tab_depth
    pts = [(-_ROOT, -n), (0, -n), (d, -h), (d, h), (0, n), (-_ROOT, n)]
    pts = [(seam_x + direction * x, y + dy) for x, dy in pts]
    return Polygon(*pts, align=None).face()


def socket_profile(lay: Layout, seam_x: float, y: float, direction: int) -> Face:
    """The matching socket: the tab grown by the joint clearance."""
    return offset(tab_profile(lay, seam_x, y, direction), lay.params.joint_clearance).face()


def tabs(lay: Layout, seam_x: float, direction: int) -> Solid:
    faces = [tab_profile(lay, seam_x, y, direction) for y in lay.joint_centres]
    # Explicit direction: a tab pointing -x has reversed winding (normal -Z).
    return extrude(faces, lay.params.thickness, dir=(0, 0, 1))


def sockets(lay: Layout, seam_x: float, direction: int) -> Solid:
    faces = [socket_profile(lay, seam_x, y, direction) for y in lay.joint_centres]
    # Cut a little above and below the plate so no skin is left.
    return Pos(0, 0, -1) * extrude(faces, lay.params.thickness + 2, dir=(0, 0, 1))


def joint_profiles(lay: Layout, clearance: float = 0.0) -> list[Face]:
    """Every joint's outline in the assembled deck, grown by `clearance`.

    Covers the seams between key-area tiles and the side tile on either end.
    """
    seams = [(s, 1) for s in lay.seams[:-1]]  # key-area seams: tab points +x
    seams += [(lay.key_area_width, -1), (0.0, 1)]  # side tile right / left
    out = []
    for seam_x, direction in seams:
        for y in lay.joint_centres:
            face = tab_profile(lay, seam_x, y, direction)
            out.append(offset(face, clearance).face() if clearance else face)
    return out
