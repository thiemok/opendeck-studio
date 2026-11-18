"""Build checks (KTD7). Each returns True when the part or deck passes."""

import itertools
from pathlib import Path

import trimesh
from build123d import Compound, Part

from .layout import Layout
from .params import Bed

_OVERLAP_MM3 = 1e-3


def fits_bed(part: Part, bed: Bed) -> bool:
    """The part's footprint fits the bed less its margin, turned if need be."""
    size = part.bounding_box().size
    footprint = sorted((size.X, size.Y))
    return footprint[0] <= min(bed.usable) and footprint[1] <= max(bed.usable)


def outline_matches(tiles: list[Part], lay: Layout) -> bool:
    """The assembled tiles span exactly the size the parameters call for."""
    p = lay.params
    want_w = p.columns * p.pitch[0] + 2 * p.border + p.side_width
    want_d = p.rows * p.pitch[1] + 2 * p.border
    size = Compound(list(tiles)).bounding_box().size
    return abs(size.X - want_w) < 0.01 and abs(size.Y - want_d) < 0.01


def joints_clear(tiles: list[Part]) -> bool:
    """Neighbouring tiles, as assembled, share no volume."""
    for a, b in itertools.pairwise(tiles):
        common = a & b
        if common is not None and common.volume > _OVERLAP_MM3:
            return False
    return True


def watertight(mesh_file: Path) -> bool:
    """The exported mesh is closed, so the slicer needs no repair."""
    try:
        mesh = trimesh.load_mesh(mesh_file)
    except ValueError:
        return False
    return len(mesh.faces) > 0 and bool(mesh.is_watertight)
