"""Exports and previews for single parts."""

import matplotlib.image as mpimg
import pytest
import trimesh

from opendeck_case.export import export_part
from opendeck_case.layout import layout
from opendeck_case.params import block_4x8
from opendeck_case.parts import deck_tiles, wedge
from opendeck_case.preview import assembly_png, part_png


@pytest.fixture(scope="module")
def lay():
    return layout(block_4x8())


def test_export_writes_mesh_cad_and_print_files(lay, tmp_path):
    part = wedge(lay, 15)
    files = export_part(part, tmp_path, "block_4x8-wedge-15")
    assert sorted(f.name for f in files.values()) == [
        "block_4x8-wedge-15.3mf",
        "block_4x8-wedge-15.step",
        "block_4x8-wedge-15.stl",
    ]
    for f in files.values():
        assert f.stat().st_size > 0


def test_exported_stl_is_watertight_and_sits_at_the_origin(lay, tmp_path):
    part = deck_tiles(lay)[-1]
    stl = export_part(part, tmp_path, "side")["stl"]
    mesh = trimesh.load_mesh(stl)
    assert mesh.is_watertight
    size = part.bounding_box().size
    assert mesh.bounds[0] == pytest.approx([0, 0, 0], abs=0.01)
    assert mesh.extents == pytest.approx([size.X, size.Y, size.Z], abs=0.01)


def test_previews_render_headless(lay, tmp_path, monkeypatch):
    monkeypatch.delenv("DISPLAY", raising=False)
    tiles = deck_tiles(lay)
    one = part_png(tiles[-1], tmp_path / "side.png", "side")
    many = assembly_png(tiles, tmp_path / "assembly.png", "block_4x8")
    for png in (one, many):
        image = mpimg.imread(png)
        assert image.shape[0] > 200 and image.shape[1] > 400
