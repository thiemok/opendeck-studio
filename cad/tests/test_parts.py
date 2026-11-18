"""Parts: tiles, joints and wedges as solids, in assembled coordinates."""

import itertools
import math

import pytest

from dataclasses import replace

from build123d import Box, Pos, Rot

from opendeck_case.joints import joint_profiles, sockets, tabs
from opendeck_case.layout import layout
from opendeck_case.params import block_2x8, block_4x8, strip_1x16
from opendeck_case.parts import (
    COUPON_SEAM,
    coupon_layout,
    joint_fit_coupon,
    deck_tiles,
    label_text,
    plate_slab,
    side_tile_on_left,
    wedge,
)

TOL = 1e-3


@pytest.fixture(scope="module")
def strip():
    lay = layout(strip_1x16())
    return lay, deck_tiles(lay)


@pytest.fixture(scope="module")
def block2():
    lay = layout(block_2x8())
    return lay, deck_tiles(lay)


@pytest.fixture(scope="module")
def block():
    lay = layout(block_4x8())
    return lay, deck_tiles(lay)


def overlap(a, b) -> float:
    common = a & b
    return 0.0 if common is None else sum(s.volume for s in common.solids())


@pytest.mark.parametrize("deck", ["strip", "block", "block2"])
def test_each_tile_is_one_solid_of_the_expected_volume(deck, request):
    lay, tiles = request.getfixturevalue(deck)
    p = lay.params
    kw, kd = (c + 2 * p.hole_clearance for c in p.keycap)
    c = p.hole_clearance
    bw, bh = p.module_button
    side_holes = (
        p.oled_window[0] * p.oled_window[1]
        + math.pi * (p.encoder_bushing / 2 + c) ** 2
        + len(p.module_button_offsets) * (bw + 2 * c) * (bh + 2 * c)
        + len(p.module_mount_offsets) * math.pi * (p.module_mount_hole / 2) ** 2
    )
    for tile, part in zip(lay.tiles, tiles):
        assert len(part.solids()) == 1
        bb = part.bounding_box()
        # Only the first key-area tile carries the embossed label.
        top = p.thickness + (p.emboss_height if part is tiles[0] else 0)
        assert (bb.min.Z, bb.max.Z) == pytest.approx((0, top), abs=0.01)
        keys = sum(tile.x0 < x < tile.x1 for x, _ in lay.keycap_centres)
        holes = keys * kw * kd if tile.kind == "keys" else side_holes
        nominal = (tile.width * lay.outer_depth - holes) * p.thickness
        # Grooves, sockets, tabs and the label shift it by a few percent at most.
        assert part.volume == pytest.approx(nominal, rel=0.03)


def test_strip_tiles_mate_without_overlap(strip):
    lay, tiles = strip
    clearance = lay.params.joint_clearance
    for a, b in itertools.pairwise(tiles):
        assert overlap(a, b) < TOL
    # The tabs on the key-area seam sit in their sockets with the clearance all round.
    seam = lay.tiles[0].x1
    assert tabs(lay, seam, 1).distance_to(tiles[1]) == pytest.approx(clearance, abs=0.02)


def test_side_tile_fits_either_end(block):  # Covers AE3.
    lay, tiles = block
    clearance = lay.params.joint_clearance
    keys, side = tiles[0], tiles[-1]
    left = side_tile_on_left(lay, side)
    assert overlap(keys, side) < TOL
    assert overlap(keys, left) < TOL
    # Tab-to-socket gap, side tile on the right and turned round on the left.
    right_tabs = tabs(lay, lay.key_area_width, -1)
    left_tabs = tabs(lay, 0.0, 1)
    assert right_tabs.distance_to(keys) == pytest.approx(clearance, abs=0.02)
    assert left_tabs.distance_to(keys) == pytest.approx(clearance, abs=0.02)
    assert overlap(left, left_tabs) > 0  # the turned tile really carries those tabs
    assert left.bounding_box().max.X == pytest.approx(lay.params.tab_depth, abs=0.01)
    assert left.bounding_box().min.X == pytest.approx(-lay.params.side_width, abs=0.01)


@pytest.mark.parametrize("params", [block_4x8(), strip_1x16(), block_2x8()])
def test_joints_stay_clear_of_keycaps_and_module(params):
    lay = layout(params)
    kw, kd = params.keycap
    mx, my = lay.module_centre
    mw, md = params.module
    marks = [(x - kw / 2, y - kd / 2, x + kw / 2, y + kd / 2) for x, y in lay.keycap_centres]
    marks.append((mx - mw / 2, my - md / 2, mx + mw / 2, my + md / 2))
    for profile in joint_profiles(lay, clearance=params.joint_clearance):
        bb = profile.bounding_box()
        for x0, y0, x1, y1 in marks:
            assert bb.max.X <= x0 or bb.min.X >= x1 or bb.max.Y <= y0 or bb.min.Y >= y1


@pytest.mark.parametrize("angle", [8, 15])
def test_wedge_props_the_plate_at_its_angle(block, angle):
    lay, _ = block
    p = lay.params
    w = wedge(lay, angle)
    slab = plate_slab(lay, angle)
    assert len(w.solids()) == 1
    assert overlap(w, slab) < TOL
    assert w.distance_to(slab) == pytest.approx(p.joint_clearance, abs=0.02)
    size = w.bounding_box().size
    assert w.bounding_box().max.Z >= lay.wedge_rise(angle)
    assert size.Y >= p.wedge_foot * size.Z - 0.01  # a foot long enough to stand on


@pytest.mark.parametrize("deck", ["strip", "block", "block2"])
@pytest.mark.parametrize("angle", [8, 15])
def test_wedge_lip_stays_on_the_back_border(deck, angle, request):
    lay, _ = request.getfixturevalue(deck)
    p = lay.params
    # Everything of the wedge above the plate's top surface, in plate coordinates.
    flat = Rot(X=-angle) * wedge(lay, angle)
    above = flat & (Pos(-100, -100, p.thickness) * Box(400, 400, 200, align=None))
    covered = lay.outer_depth - above.bounding_box().min.Y
    assert covered <= p.wedge_grip + 0.01
    assert covered < p.border  # never over a keycap mark


def test_wedges_one_per_tile(block, strip):
    assert block[0].wedge_count() == 2
    assert strip[0].wedge_count() == 3


def test_label_names_the_assembled_size(block, strip):
    assert label_text(block[0]) == "4x8 block  295 x 136 mm"
    assert label_text(strip[0]) == "1x16 strip  495 x 52 mm"
    assert label_text(layout(block_2x8())) == "2x8 block  295 x 80 mm"


@pytest.mark.parametrize("clearance", [0.0, 0.05, 0.1, 0.15])
def test_joint_fit_coupon_has_the_named_clearance(clearance):
    tab, socket = joint_fit_coupon(block_2x8(), clearance)
    assert overlap(tab, socket) < TOL
    probe = tabs(coupon_layout(block_2x8()), COUPON_SEAM, 1)
    assert probe.distance_to(socket) == pytest.approx(clearance, abs=0.02)
    assert len(tab.solids()) == 1 and len(socket.solids()) == 1


@pytest.mark.parametrize("deck", ["strip", "block", "block2"])
def test_keys_display_and_dial_are_open_through_the_plate(deck, request):
    lay, tiles = request.getfixturevalue(deck)
    p = lay.params
    mx, my = lay.module_centre
    side = tiles[-1]
    module_holes = [
        p.oled_offset,
        p.encoder_offset,
        *p.module_button_offsets,
        *p.module_mount_offsets,
    ]
    for dx, dy in module_holes:
        assert not side.is_inside((mx + dx, my + dy, p.thickness / 2))
    # The holes follow the vendor drawing: the encoder sits between the buttons.
    assert p.module_button_offsets[0][0] == p.encoder_offset[0]
    for x, y in lay.keycap_centres:
        tile = next(t for t, part in zip(lay.tiles, tiles) if t.x0 < x < t.x1)
        part = tiles[lay.tiles.index(tile)]
        assert not part.is_inside((x, y, p.thickness / 2))
        # ...and the plate is still there between neighbouring keys.
        gap = x + p.pitch[0] / 2 if x + p.pitch[0] < tile.x1 else x - p.pitch[0] / 2
        assert part.is_inside((gap, y, p.thickness / 2))


def test_joint_fit_socket_carries_its_engraved_label():
    _, labelled = joint_fit_coupon(block_2x8(), 0.05)
    lay = coupon_layout(replace(block_2x8(), joint_clearance=0.05))
    blank = Pos(COUPON_SEAM, 0, 0) * Box(COUPON_SEAM, lay.outer_depth, lay.params.thickness, align=None)
    blank -= sockets(lay, COUPON_SEAM, 1)
    assert 0 < blank.volume - labelled.volume < 0.05 * blank.volume
    assert labelled.bounding_box().max.Z == pytest.approx(lay.params.thickness)


def test_a_non_zero_clearance_reaches_the_joints_and_the_wedge_slot():
    # The default is 0 mm, which can't tell a dropped clearance from a kept one.
    lay = layout(replace(block_2x8(), joint_clearance=0.2))
    tiles = deck_tiles(lay)
    assert tabs(lay, lay.key_area_width, -1).distance_to(tiles[0]) == pytest.approx(0.2, abs=0.02)
    assert wedge(lay, 15).distance_to(plate_slab(lay, 15)) == pytest.approx(0.2, abs=0.02)


def test_module_offsets_match_the_vendor_drawing():
    # Re-derived from the raw dimensions on the drawing (docs/notes/oled-encoder-module.md),
    # board 64.90 x 35.03, offsets from its centre, +y towards the header.
    w, d = 64.90, 35.03
    p = block_2x8()
    enc = (w / 2 - 11.61, d / 2 - 17.60)
    back = (w / 2 - 14.66 + 6.10 / 2, d / 2 - (3.89 + 3.63 / 2))
    confirm = (w / 2 - 14.66 + 6.10 / 2, -(d / 2 - (7.54 - 3.63 / 2)))
    glass = (4.01 + 35.05 / 2 - w / 2, d / 2 - (6.05 + (d - 6.05 - (d - 9.98 - 18.92)) / 2))
    holes = [
        (2.87 - w / 2, d / 2 - 2.85),
        (w / 2 - 3.00, d / 2 - 2.90),
        (2.95 - w / 2, 2.97 - d / 2),
        (w / 2 - 2.97, 3.15 - d / 2),
    ]
    assert p.encoder_offset == pytest.approx(enc, abs=0.01)
    assert [c for o in p.module_button_offsets for c in o] == pytest.approx([*back, *confirm], abs=0.01)
    assert p.oled_offset == pytest.approx(glass, abs=0.01)
    assert p.oled_window == pytest.approx((35.05, d - 6.05 - (d - 9.98 - 18.92)), abs=0.01)
    assert [c for o in p.module_mount_offsets for c in o] == pytest.approx([c for h in holes for c in h], abs=0.01)
