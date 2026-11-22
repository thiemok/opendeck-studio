"""The 2x8 side tile with two ringed dials (THI-87)."""

import itertools
import json
import math

import pytest
from build123d import Box, Pos

from opendeck_case.cli import main
from opendeck_case.joints import tabs
from opendeck_case.layout import layout
from opendeck_case.params import block_2x8, block_2x8_dials
from opendeck_case.parts import deck_tiles, dummy_knob, key_tile, side_tile_on_left


@pytest.fixture(scope="module")
def deck():
    lay = layout(block_2x8_dials())
    tiles = deck_tiles(lay)
    return lay, tiles, side_tile_on_left(lay, tiles[-1])


def ends(deck):
    """Each end's tile, its left edge, and the sign of the module's offsets."""
    lay, tiles, left = deck
    return [(tiles[-1], lay.key_area_width, 1), (left, -lay.params.side_width, -1)]


def overlap(a, b) -> float:
    common = a & b
    return 0.0 if common is None else sum(s.volume for s in common.solids())


def test_the_outline_is_unchanged(deck):
    lay, _, _ = deck
    assert (lay.outer_width, lay.outer_depth) == pytest.approx((295.2, 80.0))
    assert lay.side_tile.width == pytest.approx(71.2)
    assert not lay.side_reversible
    assert layout(block_2x8()).side_reversible


def test_the_printed_key_tile_can_be_reused(deck):
    lay, tiles, _ = deck
    old = key_tile(layout(block_2x8()), 0)
    assert tiles[0].volume == pytest.approx(old.volume, abs=1e-3)
    assert overlap(tiles[0], old) == pytest.approx(old.volume, abs=1e-2)


def test_dials_stay_at_the_front_on_both_ends(deck):
    lay, _, _ = deck
    p = lay.params
    for side, x0, _ in ends(deck):
        for x, y in p.dials:
            assert y < lay.outer_depth / 2
            assert not side.is_inside((x0 + x, y, p.thickness / 2))
            # The ring is engraved: open at the surface, solid underneath.
            band = x0 + x + sum(p.ring_diameters) / 4
            assert not side.is_inside((band, y, p.thickness - p.engrave_depth / 2))
            assert side.is_inside((band, y, p.thickness / 2))


def test_the_module_turns_round_on_the_left(deck):
    lay, _, _ = deck
    p = lay.params
    my = lay.module_centre[1]
    assert my + p.module[1] / 2 == pytest.approx(lay.outer_depth - 3.0)
    features = [p.oled_offset, p.encoder_offset, *p.module_button_offsets, *p.module_mount_offsets]
    for side, x0, sign in ends(deck):
        mx = x0 + p.side_width / 2
        for dx, dy in features:
            assert not side.is_inside((mx + sign * dx, my + sign * dy, p.thickness / 2))


def test_both_side_tiles_join_the_key_tile(deck):
    lay, tiles, left = deck
    for side, seam, direction in ((tiles[-1], lay.key_area_width, -1), (left, 0.0, 1)):
        joint = tabs(lay, seam, direction)
        assert len(side.solids()) == 1
        assert overlap(side, tiles[0]) < 1e-3
        assert overlap(side, joint) == pytest.approx(joint.volume, abs=0.01)
    assert left.bounding_box().min.X == pytest.approx(-lay.params.side_width, abs=0.01)


def test_rings_clear_the_edges_the_module_and_each_other(deck):
    lay, _, _ = deck
    p = lay.params
    r = p.ring_diameters[1] / 2
    module_front = lay.module_centre[1] - p.module[1] / 2
    for x, y in p.dials:
        assert min(x - r, p.side_width - x - r, y - r) >= p.side_margin
        assert y + r < module_front
    for a, b in itertools.combinations(p.dials, 2):
        assert math.dist(a, b) > 2 * r


def test_the_wedge_lip_has_a_clear_place_on_both_ends(deck):
    lay, _, _ = deck
    p = lay.params
    # Below the engraving, the lip's strip is solid from x = 20 to 50.
    strip = Box(30, p.wedge_grip, 2, align=None)
    for side, x0, _ in ends(deck):
        probe = Pos(x0 + 20, lay.outer_depth - p.wedge_grip, 1) * strip
        assert overlap(side, probe) == pytest.approx(probe.volume, abs=1e-3)


def test_dummy_knobs_plug_in_without_reaching_the_desk(deck):
    lay, _, _ = deck
    p = lay.params
    size = dummy_knob(lay, p.dial_knob).bounding_box().size
    assert (size.X, size.Y) == pytest.approx((16, 16), abs=0.01)
    peg = size.Z - p.knob_height
    assert 0 < peg < p.thickness
    top = dummy_knob(lay, p.dial_knob) & (Pos(-10, -10, p.knob_height + 1) * Box(20, 20, 1, align=None))
    assert top.bounding_box().size.X < p.encoder_bushing + 2 * p.hole_clearance


def test_the_print_set_holds_only_the_new_parts(tmp_path):
    assert main(["build", "thi-87", "--out", str(tmp_path)]) == 0
    [variant] = json.loads((tmp_path / "report.json").read_text())["variants"]
    assert variant["outer_mm"] == pytest.approx([295.2, 80.0])
    quantities = {part["name"]: part["quantity"] for part in variant["parts"]}
    assert quantities == {"side": 1, "side-left": 1, "knob": 2, "knob-module": 1}
    assert not list(tmp_path.glob("*keys*")) and not list(tmp_path.glob("*wedge*"))
