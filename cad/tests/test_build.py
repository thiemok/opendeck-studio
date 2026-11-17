"""`case build`: the whole print set, end to end through the CLI."""

import json

import pytest

from opendeck_case.cli import main


@pytest.fixture(scope="module")
def thi82(tmp_path_factory):
    out = tmp_path_factory.mktemp("thi-82")
    code = main(["build", "thi-82", "--out", str(out)])
    return code, out, json.loads((out / "report.json").read_text())


def test_thi82_builds_and_every_check_passes(thi82):
    code, out, report = thi82
    assert code == 0
    assert report["ok"] is True
    names = {v["variant"]: [p["name"] for p in v["parts"]] for v in report["variants"]}
    assert names == {
        "block_4x8": ["keys", "side", "wedge-8", "wedge-15"],
        "strip_1x16": ["keys-1", "keys-2", "side", "wedge-8", "wedge-15"],
    }
    for variant in report["variants"]:
        assert all(variant["checks"].values())
        assert (out / f"{variant['variant']}-assembly.png").is_file()
        for part in variant["parts"]:
            assert all(part["checks"].values()), part
            for ext in ("stl", "3mf", "step", "png"):
                assert (out / f"{variant['variant']}-{part['name']}.{ext}").is_file()


def test_wedges_come_one_per_tile(thi82):
    _, _, report = thi82
    qty = {v["variant"]: {p["name"]: p["quantity"] for p in v["parts"]} for v in report["variants"]}
    assert qty["block_4x8"]["wedge-15"] == 2
    assert qty["strip_1x16"]["wedge-8"] == 3
    assert qty["strip_1x16"]["keys-1"] == 1


def test_strip_parts_fit_the_p1s_and_assemble_to_495_by_52(thi82):  # Covers AE1.
    _, _, report = thi82
    strip = next(v for v in report["variants"] if v["variant"] == "strip_1x16")
    assert strip["outer_mm"] == pytest.approx([495.2, 52.0])
    for part in strip["parts"]:
        x, y, _ = part["size_mm"]
        assert max(x, y) <= 251  # 256 mm bed less the 5 mm margin


def test_wedges_are_exported_lying_on_their_side(thi82):
    _, _, report = thi82
    for variant in report["variants"]:
        for part in variant["parts"]:
            if part["name"].startswith("wedge"):
                assert part["size_mm"][2] == pytest.approx(20.0)  # wedge width on the bed


def test_a_failed_check_exits_non_zero(tmp_path, capsys, monkeypatch):
    monkeypatch.setattr("opendeck_case.checks.fits_bed", lambda part, bed: False)
    code = main(["build", "thi-82", "--out", str(tmp_path)])
    assert code == 1
    assert json.loads((tmp_path / "report.json").read_text())["ok"] is False
    assert "FAILED fits_bed" in capsys.readouterr().out


def test_a_bed_too_small_fails_and_names_the_variant(tmp_path, capsys):
    # 100 mm is shallower than the 4x8's 136 mm plate, which no column cut can split.
    code = main(["build", "thi-82", "--out", str(tmp_path), "--bed", "100"])
    assert code != 0
    assert "block_4x8" in capsys.readouterr().out
    assert json.loads((tmp_path / "report.json").read_text())["ok"] is False


def test_unknown_print_set_lists_the_known_ones(tmp_path, capsys):
    code = main(["build", "nope", "--out", str(tmp_path)])
    assert code != 0
    assert "thi-82" in capsys.readouterr().err
