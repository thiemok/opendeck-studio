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
from .printsets import PRINT_SETS, build_joint_fit, build_variant, default_out_dir


def _build(set_name: str, out: Path, bed: Bed | None) -> dict:
    report = {"set": set_name, "variants": []}
    print_set = PRINT_SETS[set_name]
    groups = []
    for name in print_set.variants:
        try:
            groups.append(build_variant(name, bed))
        except LayoutError as err:
            report["variants"].append({"variant": name, "error": str(err), "checks": {}})
    if print_set.joint_fit:
        try:
            groups.append(build_joint_fit(bed))
        except LayoutError as err:
            report["variants"].append({"variant": "joint-fit", "error": str(err), "checks": {}})
    for group in groups:
        entry = {
            "variant": group.name,
            "outer_mm": list(group.outer_mm) if group.outer_mm else None,
            "checks": {name: check() for name, check in group.checks.items()},
            "parts": [],
        }
        for bp in group.parts:
            stem = f"{group.name}-{bp.name}"
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
                        "fits_bed": checks.fits_bed(bp.part, group.bed),
                        "watertight": checks.watertight(files["stl"]),
                    },
                }
            )
        assembly_png(group.assembly, out / f"{group.name}-assembly.png", group.name)
        report["variants"].append(entry)
    report["ok"] = all(
        "error" not in v
        and all(v["checks"].values())
        and all(all(p["checks"].values()) for p in v["parts"])
        for v in report["variants"]
    )
    return report


def _status(checks: dict[str, bool]) -> str:
    failed = [name for name, ok in checks.items() if not ok]
    return "FAILED " + ", ".join(failed) if failed else "ok"


def _print_report(report: dict, out: Path) -> None:
    for v in report["variants"]:
        if "error" in v:
            print(f"{v['variant']}: FAILED  {v['error']}")
            continue
        size = "assembled {:g} x {:g} mm".format(*v["outer_mm"]) if v["outer_mm"] else "test pieces"
        print(f"{v['variant']}: {size}  {_status(v['checks'])}")
        for p in v["parts"]:
            x, y, z = p["size_mm"]
            size = f"{x:7.1f} x {y:6.1f} x {z:5.1f} mm"
            print(f"  {p['quantity']}x {p['name']:<10} {size}  {_status(p['checks'])}")
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
