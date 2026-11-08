#!/usr/bin/env python3
"""Renders the iOS app icon: a Literata R in the Paper theme's colors. Needs Pillow."""
from pathlib import Path

from PIL import Image, ImageDraw, ImageFont

ROOT = Path(__file__).resolve().parent.parent
OUT = ROOT / "ios/Reader/Assets.xcassets/AppIcon.appiconset/icon-1024.png"
SIZE = 1024
PAPER = "#FBFAF7"
INK = "#1F1D1A"

font = ImageFont.truetype(str(ROOT / "assets/fonts/Literata.ttf"), 680)
font.set_variation_by_axes([36, 500])

icon = Image.new("RGB", (SIZE, SIZE), PAPER)
draw = ImageDraw.Draw(icon)
left, top, right, bottom = draw.textbbox((0, 0), "R", font=font)
draw.text(((SIZE - left - right) / 2, (SIZE - top - bottom) / 2), "R", font=font, fill=INK)

OUT.parent.mkdir(parents=True, exist_ok=True)
icon.save(OUT)
(OUT.parent / "Contents.json").write_text(
    '{\n  "images": [\n    {\n      "filename": "icon-1024.png",\n      "idiom": "universal",\n'
    '      "platform": "ios",\n      "size": "1024x1024"\n    }\n  ],\n'
    '  "info": { "author": "xcode", "version": 1 }\n}\n'
)
(OUT.parent.parent / "Contents.json").write_text('{\n  "info": { "author": "xcode", "version": 1 }\n}\n')
print(OUT)
