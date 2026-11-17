"""Write each part as STL, 3MF (for the slicer) and STEP (for CAD)."""

from pathlib import Path

from build123d import Mesher, Part, Pos, export_step, export_stl

# Finer than the default so engraved outlines and text come out clean.
TOLERANCE = 0.02
ANGULAR_TOLERANCE = 0.1


def at_origin(part: Part) -> Part:
    return Pos(-part.bounding_box().min) * part


def export_part(part: Part, out_dir: Path, stem: str) -> dict[str, Path]:
    out_dir.mkdir(parents=True, exist_ok=True)
    part = at_origin(part)
    files = {ext: out_dir / f"{stem}.{ext}" for ext in ("stl", "3mf", "step")}
    export_stl(part, files["stl"], tolerance=TOLERANCE, angular_tolerance=ANGULAR_TOLERANCE)
    mesher = Mesher()
    mesher.add_shape(part, linear_deflection=TOLERANCE, angular_deflection=ANGULAR_TOLERANCE)
    mesher.write(str(files["3mf"]))
    export_step(part, files["step"])
    return files
