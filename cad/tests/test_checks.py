"""The individual build checks, on parts built to pass or fail them."""

from build123d import Box, Pos

from opendeck_case.checks import fits_bed, joints_clear, outline_matches, watertight
from opendeck_case.export import export_part
from opendeck_case.layout import layout
from opendeck_case.params import P1S, block_4x8
from opendeck_case.parts import deck_tiles


def test_fits_bed_allows_turning_the_part():
    assert fits_bed(Box(250, 100, 4), P1S)
    assert fits_bed(Box(100, 250, 4), P1S)
    assert not fits_bed(Box(252, 100, 4), P1S)  # 256 less the 5 mm margin


def test_joints_clear_catches_overlap():
    a = Box(10, 10, 4, align=None)
    assert joints_clear([a, Pos(10, 0, 0) * a])
    assert not joints_clear([a, Pos(9, 0, 0) * a])


def test_watertight_reads_the_exported_mesh(tmp_path):
    stl = export_part(Box(10, 10, 10), tmp_path, "cube")["stl"]
    assert watertight(stl)
    stl.write_text("solid broken\nendsolid broken\n")
    assert not watertight(stl)


def test_outline_matches_catches_a_missing_or_shifted_tile():
    lay = layout(block_4x8())
    tiles = deck_tiles(lay)
    assert outline_matches(tiles, lay)
    assert not outline_matches(tiles[:-1], lay)
    assert not outline_matches([tiles[0], Pos(5, 0, 0) * tiles[1]], lay)
