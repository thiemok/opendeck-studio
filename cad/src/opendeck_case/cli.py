"""`case`: build a named print set into printable files, previews and a report.

    case list
    case build thi-82 [--out DIR] [--bed MM]
"""

import argparse
import json
import sys
from pathlib import Path

from . import checks
from .export import export_part
from .layout import LayoutError
from .params import Bed
from .preview import assembly_png, part_png
from .printsets import PRINT_SETS, build_variant, default_out_dir


def _build(set_name: str, out: Path, bed: Bed | None) -> dict:
    report = {"set": set_name, "variants": []}
    for name in PRINT_SETS[set_name].variants:
        try:
            built = build_variant(name, bed)
        except LayoutError as err:
            report["variants"].append({"variant": name, "error": str(err), "checks": {}})
            continue
        lay = built.layout
        entry = {
            "variant": name,
            "outer_mm": [round(lay.outer_width, 2), round(lay.outer_depth, 2)],
            "checks": {
                "outline": checks.outline_matches(built.tiles, lay),
                "joints_dial_right": checks.joints_clear(built.tiles),
                "joints_dial_left": checks.joints_clear([built.side_on_left, built.tiles[0]]),
            },
            "parts": [],
        }
        for bp in built.parts:
            stem = f"{name}-{bp.name}"
            files = export_part(bp.part, out, stem)
            files["png"] = part_png(bp.part, out / f"{stem}.png", stem)
            size = bp.part.bounding_box().size
            entry["parts"].append(
                {
                    "name": bp.name,
                    "quantity": bp.quantity,
                    "size_mm": [round(size.X, 2), round(size.Y, 2), round(size.Z, 2)],
                    "files": {k: v.name for k, v in files.items()},
                    "checks": {
                        "fits_bed": checks.fits_bed(bp.part, lay.params.bed),
                        "watertight": checks.watertight(files["stl"]),
                    },
                }
            )
        assembly_png(built.tiles, out / f"{name}-assembly.png", name)
        report["variants"].append(entry)
    report["ok"] = all(
        "error" not in v
        and all(v["checks"].values())
        and all(all(p["checks"].values()) for p in v["parts"])
        for v in report["variants"]
    )
    return report


def _print_report(report: dict, out: Path) -> None:
    for v in report["variants"]:
        if "error" in v:
            print(f"{v['variant']}: FAILED  {v['error']}")
            continue
        w, d = v["outer_mm"]
        failed = [k for k, ok in v["checks"].items() if not ok]
        print(f"{v['variant']}: assembled {w:g} x {d:g} mm  {'FAILED ' + ', '.join(failed) if failed else 'ok'}")
        for p in v["parts"]:
            x, y, z = p["size_mm"]
            bad = [k for k, ok in p["checks"].items() if not ok]
            status = "FAILED " + ", ".join(bad) if bad else "ok"
            print(f"  {p['quantity']}x {p['name']:<10} {x:7.1f} x {y:6.1f} x {z:5.1f} mm  {status}")
    print(f"{'all checks passed' if report['ok'] else 'CHECKS FAILED'}; output in {out}")


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(prog="case", description=__doc__.splitlines()[0])
    sub = parser.add_subparsers(dest="command", required=True)
    sub.add_parser("list", help="list the print sets")
    build = sub.add_parser("build", help="build a print set")
    build.add_argument("set", help="print set name, e.g. thi-82")
    build.add_argument("--out", type=Path, help="output directory (default cad/out/<set>)")
    build.add_argument("--bed", type=float, help="square bed size in mm (default: the P1S)")
    args = parser.parse_args(argv)

    if args.command == "list":
        for ps in PRINT_SETS.values():
            print(f"{ps.name}: {ps.description}")
        return 0

    if args.set not in PRINT_SETS:
        print(f"unknown print set {args.set!r}; known: {', '.join(PRINT_SETS)}", file=sys.stderr)
        return 2
    out = args.out or default_out_dir(args.set)
    out.mkdir(parents=True, exist_ok=True)
    report = _build(args.set, out, Bed(args.bed, args.bed) if args.bed else None)
    (out / "report.json").write_text(json.dumps(report, indent=2) + "\n")
    _print_report(report, out)
    return 0 if report["ok"] else 1


if __name__ == "__main__":
    sys.exit(main())
