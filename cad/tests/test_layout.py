"""Layout: derived sizes, tile cuts and keycap positions, without CAD geometry."""

import math
from dataclasses import replace

import pytest

from opendeck_case.layout import LayoutError, layout
from opendeck_case.params import P1S, Bed, block_4x8, strip_1x16


def test_block_4x8_outline_and_tiles():
    lay = layout(block_4x8())
    assert lay.outer_width == pytest.approx(295.2)
    assert lay.outer_depth == pytest.approx(136.0)
    assert [t.kind for t in lay.tiles] == ["keys", "side"]


def test_strip_1x16_outline_and_cut():  # Covers AE1.
    lay = layout(strip_1x16())
    assert lay.outer_width == pytest.approx(495.2)
    assert lay.outer_depth == pytest.approx(52.0)
    keys = [t for t in lay.tiles if t.kind == "keys"]
    assert [(t.first_column, t.last_column) for t in keys] == [(0, 7), (8, 15)]


@pytest.mark.parametrize(("bed", "key_tiles"), [(350, 2), (500, 1)])
def test_bed_size_changes_only_the_tiling(bed, key_tiles):  # Covers AE2.
    base = layout(strip_1x16())
    lay = layout(replace(strip_1x16(), bed=Bed(bed, bed)))
    assert sum(t.kind == "keys" for t in lay.tiles) == key_tiles
    assert (lay.outer_width, lay.outer_depth) == (base.outer_width, base.outer_depth)
    assert lay.keycap_centres == base.keycap_centres


@pytest.mark.parametrize("params", [block_4x8(), strip_1x16()])
def test_keycaps_sit_inside_their_tile_and_no_cut_crosses_one(params):
    lay = layout(params)
    kw, kd = params.keycap
    assert len(lay.keycap_centres) == params.columns * params.rows
    for x, y in lay.keycap_centres:
        tiles = [t for t in lay.tiles if t.x0 <= x - kw / 2 and x + kw / 2 <= t.x1]
        assert len(tiles) == 1 and tiles[0].kind == "keys"
        assert 0 < y - kd / 2 and y + kd / 2 < lay.outer_depth
        for t in lay.tiles:
            assert not (x - kw / 2 < t.x0 < x + kw / 2)


@pytest.mark.parametrize("params", [block_4x8(), strip_1x16()])
def test_every_tile_fits_the_bed_including_its_tabs(params):
    lay = layout(params)
    usable = params.bed.usable
    for t in lay.tiles:
        assert t.print_width <= usable[0] and lay.outer_depth <= usable[1]


def test_wedge_rise_uses_the_slope_length():
    lay = layout(block_4x8())
    assert lay.wedge_rise(15) == pytest.approx(136 * math.sin(math.radians(15)))
    assert lay.wedge_rise(15) == pytest.approx(35.2, abs=0.1)


def test_default_bed_is_the_p1s():
    assert block_4x8().bed == P1S == Bed(256, 256)


def test_bed_too_narrow_for_one_column_is_an_error():
    with pytest.raises(LayoutError, match="strip_1x16"):
        layout(replace(strip_1x16(), bed=Bed(40, 256)))


def test_bed_shallower_than_the_plate_is_an_error():
    with pytest.raises(LayoutError, match="136"):
        layout(replace(block_4x8(), bed=Bed(100, 100)))
