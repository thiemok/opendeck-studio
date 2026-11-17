"""Smoke test: the pinned build123d builds and exports a solid."""

from build123d import Box, Mesher, export_step, export_stl


def test_cube_bounding_box():
    size = Box(10, 10, 10).bounding_box().size
    assert (round(size.X, 6), round(size.Y, 6), round(size.Z, 6)) == (10, 10, 10)


def test_cube_exports(tmp_path):
    cube = Box(10, 10, 10)
    export_stl(cube, tmp_path / "cube.stl")
    export_step(cube, tmp_path / "cube.step")
    mesher = Mesher()
    mesher.add_shape(cube)
    mesher.write(str(tmp_path / "cube.3mf"))
    for name in ("cube.stl", "cube.step", "cube.3mf"):
        assert (tmp_path / name).stat().st_size > 0
