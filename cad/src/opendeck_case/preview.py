"""Hidden-line preview images the agent can read without a display (KTD6)."""

from pathlib import Path

import matplotlib

matplotlib.use("Agg")

import matplotlib.pyplot as plt  # noqa: E402
from build123d import Compound, Part  # noqa: E402

_SAMPLES = 16


def _draw(ax, shape: Part, direction: tuple[float, float, float], up) -> None:
    centre = shape.bounding_box().center()
    span = shape.bounding_box().diagonal * 4
    origin = tuple(c + d * span for c, d in zip(centre, direction))
    visible, _hidden = shape.project_to_viewport(origin, up, look_at=tuple(centre))
    for edge in visible.edges():
        pts = [edge.position_at(i / _SAMPLES) for i in range(_SAMPLES + 1)]
        ax.plot([p.X for p in pts], [p.Y for p in pts], "k-", lw=0.4)
    ax.set_aspect("equal")
    ax.axis("off")


def _views(shape: Part, path: Path, title: str) -> Path:
    fig, (top, iso) = plt.subplots(1, 2, figsize=(14, 6))
    _draw(top, shape, (0, 0, 1), (0, 1, 0))
    top.set_title("top")
    _draw(iso, shape, (1, -1, 1), (0, 0, 1))
    iso.set_title("isometric")
    fig.suptitle(title)
    fig.savefig(path, dpi=150, bbox_inches="tight")
    plt.close(fig)
    return path


def part_png(part: Part, path: Path, title: str) -> Path:
    size = part.bounding_box().size
    return _views(part, path, f"{title}  ({size.X:.1f} x {size.Y:.1f} x {size.Z:.1f} mm)")


def assembly_png(parts: list[Part], path: Path, title: str) -> Path:
    assembly = Compound(children=list(parts))
    size = assembly.bounding_box().size
    return _views(assembly, path, f"{title} assembled  ({size.X:.1f} x {size.Y:.1f} mm)")
