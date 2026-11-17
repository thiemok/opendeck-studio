"""Deck parameters: the one source every case part is derived from (KTD2, KTD3).

All lengths are millimetres. A deck variant is a `DeckParams`; change a value
here (or with `dataclasses.replace`) and every part follows.
"""

from dataclasses import dataclass


@dataclass(frozen=True)
class Bed:
    """Printer build plate, and the margin kept free around every part."""

    width: float
    depth: float
    margin: float = 5.0

    @property
    def usable(self) -> tuple[float, float]:
        return (self.width - self.margin, self.depth - self.margin)


# Bambu Lab P1S, the printer selected in the operator's Bambu Studio.
P1S = Bed(256.0, 256.0)


@dataclass(frozen=True)
class DeckParams:
    name: str
    columns: int
    rows: int

    # Waveshare 0.85" ScreenKey keycap (THI-82) and the estimated key pitch.
    keycap: tuple[float, float] = (22.00, 25.30)
    pitch: tuple[float, float] = (25.0, 28.0)
    border: float = 12.0

    # 1.3" OLED + EC11 module board (docs/notes/oled-encoder-module.md).
    module: tuple[float, float] = (65.2, 35.1)
    side_margin: float = 3.0
    # Rough marks on the module footprint. Estimates, not measured: the OLED
    # window sits towards one end of the board and the dial towards the other.
    oled_window: tuple[float, float] = (30.0, 16.0)
    oled_offset: float = -14.0  # window centre, from the board centre along x
    dial_diameter: float = 14.0
    dial_offset: float = 20.0  # dial centre, from the board centre along x

    # Plate, marks and label (KTD3).
    thickness: float = 4.0
    corner_radius: float = 4.0  # the side tile's two outer corners
    engrave_depth: float = 0.6
    groove_width: float = 0.8
    emboss_height: float = 0.6
    label_size: float = 5.0

    # Dovetail joints (KTD4): neck at the seam, head at the tab's tip.
    tab_neck: float = 5.0
    tab_head: float = 8.0
    tab_depth: float = 6.0
    joint_clearance: float = 0.2

    # Tilt wedges (KTD5).
    wedge_width: float = 20.0
    wedge_grip: float = 8.0
    wedge_wall: float = 3.0
    wedge_foot: float = 0.6  # footprint depth as a share of the wedge height
    wedge_angles: tuple[float, ...] = (8.0, 15.0)

    bed: Bed = P1S

    @property
    def side_width(self) -> float:
        return self.module[0] + 2 * self.side_margin


def block_4x8() -> DeckParams:
    """The 4×8 block: 32 keys."""
    return DeckParams(name="block_4x8", columns=8, rows=4)


def strip_1x16() -> DeckParams:
    """The 1×16 strip: 16 keys."""
    return DeckParams(name="strip_1x16", columns=16, rows=1)


VARIANTS = {"block_4x8": block_4x8, "strip_1x16": strip_1x16}
