#!/usr/bin/env python3
"""Compose the "away mode" menu-bar icon from the shipped idle icon.

`src-tauri/icons/tray/tray-idle-away.png` is the idle robot head with a round badge in the
corner the "?" badge uses, and a crescent moon knocked out of the badge (docs/specs/special-replies.md
D24). Like the other tray icons it is a 48x36 RGBA template image: ink is pure black, only the
alpha channel matters (macOS tints it; Windows / Linux invert the ink on a dark panel).

The badge is cut out of the head outline first (a clearance ring), then painted solid, then the
crescent is cut out of it (outer disc minus an offset disc). All numbers are in pixels of the
48x36 canvas, measured from the top-left corner.

Requires ImageMagick (`magick`): `brew install imagemagick` / `apt install imagemagick`.
"""

import os
import shutil
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
TRAY = os.path.join(HERE, "..", "src-tauri", "icons", "tray")
SRC = os.path.join(TRAY, "tray-idle.png")
DST = os.path.join(TRAY, "tray-idle-away.png")

W, H = 48, 36
# The badge slot of the "?" icon.
BADGE = (39.4, 7.7)
RING_R = 8.0  # clearance ring cut out of the head outline
DISC_R = 7.1  # solid badge
# Crescent: the outer disc is knocked out of the badge, then the offset disc is painted back.
MOON_OUTER = (BADGE[0] - 0.7, BADGE[1] + 0.6, 4.0)
MOON_INNER = (MOON_OUTER[0] + 2.1, MOON_OUTER[1] - 1.8, 3.35)
# ImageMagick addresses pixel centres, so a coordinate measured on the pixel grid shifts by half a pixel.
SHIFT = -0.5


def disc(cx: float, cy: float, r: float) -> str:
    x, y = cx + SHIFT, cy + SHIFT
    return f"circle {x:.2f},{y:.2f} {x + r:.2f},{y:.2f}"


def layer(fill: str, shape: str) -> list[str]:
    return ["(", "-size", f"{W}x{H}", "xc:none", "-fill", fill, "-draw", shape, ")"]


def main() -> int:
    if not shutil.which("magick"):
        print("error: ImageMagick `magick` not found", file=sys.stderr)
        return 1
    cmd = ["magick", SRC]
    cmd += layer("white", disc(*BADGE, RING_R)) + ["-compose", "DstOut", "-composite"]
    cmd += layer("black", disc(*BADGE, DISC_R)) + ["-compose", "Over", "-composite"]
    cmd += layer("white", disc(*MOON_OUTER)) + ["-compose", "DstOut", "-composite"]
    cmd += layer("black", disc(*MOON_INNER)) + ["-compose", "Over", "-composite"]
    cmd += [f"PNG32:{DST}"]
    subprocess.run(cmd, check=True)
    print(f"wrote {os.path.normpath(DST)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
