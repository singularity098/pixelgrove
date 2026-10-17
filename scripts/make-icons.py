#!/usr/bin/env python3
"""Renders the PWA icons in web/public from the logo design (see favicon.svg).

Usage: python3 scripts/make-icons.py   (needs Pillow)
"""
from pathlib import Path

from PIL import Image, ImageDraw

OUT = Path(__file__).resolve().parent.parent / "web" / "public"
GREEN = (31, 111, 74)
TILES = [((7, 7), (142, 228, 180)), ((17, 7), (79, 193, 132)), ((7, 17), (79, 193, 132)), ((17, 17), (214, 247, 227))]


def draw(size: int, *, rounded: bool, inset: float) -> Image.Image:
    """The 32-unit logo scaled to `size`, with the tiles shrunk by `inset` (0..1) so
    maskable icons keep them inside the safe zone."""
    image = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    d = ImageDraw.Draw(image)
    unit = size / 32
    d.rounded_rectangle([0, 0, size - 1, size - 1], radius=7 * unit if rounded else 0, fill=GREEN)
    scale = 1 - inset
    offset = size * inset / 2
    for (x, y), color in TILES:
        box = [offset + x * unit * scale, offset + y * unit * scale, offset + (x + 8) * unit * scale, offset + (y + 8) * unit * scale]
        d.rounded_rectangle(box, radius=1.5 * unit * scale, fill=color)
    return image


for size in (192, 512):
    draw(size, rounded=True, inset=0).save(OUT / f"icon-{size}.png", optimize=True)
    # Maskable: full-bleed background, artwork inside the central 80% safe zone.
    draw(size, rounded=False, inset=0.2).save(OUT / f"icon-maskable-{size}.png", optimize=True)
draw(180, rounded=False, inset=0.1).convert("RGB").save(OUT / "apple-touch-icon.png", optimize=True)
