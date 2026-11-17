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
    # Module features from the vendor's dimension drawing, transcribed in
    # docs/notes/oled-encoder-module.md (Mechanical drawing). Offsets are from
    # the board centre: +x towards the encoder, +y towards the header.
    # Display window: the drawn 35.05 x 22.85 glass outline, which covers the
    # 1.3" panel; its flex cable folds round the glass's bottom edge.
    oled_window: tuple[float, float] = (35.05, 22.85)
    oled_offset: tuple[float, float] = (-10.92, 0.04)
    encoder_bushing: float = 7.0  # EC11 shaft bushing diameter
    encoder_offset: tuple[float, float] = (20.84, -0.09)
    module_button: tuple[float, float] = (6.10, 3.63)  # tactile switch body
    module_button_offsets: tuple[tuple[float, float], ...] = ((20.84, 11.81), (20.84, -11.79))
    module_mount_hole: float = 3.4  # M3 clearance
    module_mount_offsets: tuple[tuple[float, float], ...] = (
        (-29.58, 14.67),
        (29.45, 14.62),
        (-29.50, -14.55),
        (29.48, -14.37),
    )

    # Plate, marks and label (KTD3).
    thickness: float = 4.0
    corner_radius: float = 4.0  # the side tile's two outer corners
    engrave_depth: float = 0.6
    groove_width: float = 0.8
    # Keys, the OLED window and the dial are cut through the plate so the
    # parts can be tried in place; keycap holes get this much room per side.
    hole_clearance: float = 0.3
    emboss_height: float = 0.6
    label_size: float = 5.0

    # Dovetail joints (KTD4): neck at the seam, head at the tab's tip.
    tab_neck: float = 5.0
    tab_head: float = 8.0
    tab_depth: float = 6.0
    # Per side. 0.2 printed loose on the P1S (THI-82 test print); the joint-fit
    # print set tries 0.00 to 0.15 to settle it.
    joint_clearance: float = 0.1

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


def block_2x8() -> DeckParams:
    """The 2x8 block: 16 keys, as wide as the 4x8 but half as deep."""
    return DeckParams(name="block_2x8", columns=8, rows=2)


def strip_1x16() -> DeckParams:
    """The 1×16 strip: 16 keys."""
    return DeckParams(name="strip_1x16", columns=16, rows=1)


VARIANTS = {"block_4x8": block_4x8, "strip_1x16": strip_1x16, "block_2x8": block_2x8}
