#!/usr/bin/env python3
"""Export Inklura Desktop's existing vector mark to app, tray and package assets.

Requires ImageMagick and Pillow. Legacy output names preserve existing loaders
and packaging contracts; every visible mark comes from inklura-desktop.svg.
"""
from pathlib import Path
import json
import math
import shutil
import subprocess
import tempfile

from PIL import Image

ROOT = Path(__file__).resolve().parents[1]
IMAGES = ROOT / "crates/shelldeck/assets/images"
BRAND = IMAGES / "brand"
PACK = ROOT / "packaging/icons"
SOURCE = BRAND / "inklura-desktop.svg"
THEMES = ("dark", "light", "system", "dracula", "nord", "tokyo-night", "gruvbox-dark",
          "solarized-dark", "solarized-light", "catppuccin-mocha", "one-dark", "monokai", "rose-pine")
SIZES = (16, 32, 48, 64, 128, 256, 512, 1024)
MARK = '''<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 48 48" fill="currentColor">
<circle cx="24" cy="11" r="5"/>
<rect x="19.5" y="19" width="9" height="18" rx="4"/>
<path d="M9 35c6 12 24 12 30 0" fill="none" stroke="currentColor" stroke-width="4" stroke-linecap="round"/>
</svg>
'''


def rasterize(source, target, size):
    target.parent.mkdir(parents=True, exist_ok=True)
    subprocess.run(["convert", "-background", "none", "-density", str(max(192, size * 2)), str(source), "-resize",
                    f"{size}x{size}", "-strip", str(target)], check=True, capture_output=True)


def main():
    source = SOURCE.read_text()
    for folder in (BRAND / "svg/themes", BRAND / "svg/expressions", BRAND / "png/themes",
                   BRAND / "png/app-icon", PACK / "iconset"):
        folder.mkdir(parents=True, exist_ok=True)
    for theme in THEMES:
        for kind in ("logo", "app-icon"):
            (BRAND / f"svg/themes/{theme}-{kind}.svg").write_text(source)
            for expression in ("default", "neutral", "wink"):
                (BRAND / f"svg/expressions/{theme}-{expression}-{kind}.svg").write_text(source)
    (IMAGES / "shelldeck-icon.svg").write_text(source)
    (IMAGES / "shelldeck-mark.svg").write_text(MARK)
    for size in SIZES:
        png = PACK / f"shelldeck-{size}.png"
        rasterize(SOURCE, png, size)
        shutil.copyfile(png, BRAND / f"png/app-icon/monolith-{size}.png")
    shutil.copyfile(PACK / "shelldeck-128.png", IMAGES / "shelldeck-icon.png")
    for theme in THEMES:
        shutil.copyfile(PACK / "shelldeck-128.png", BRAND / f"png/themes/monolith-{theme}-128.png")
    with Image.open(PACK / "shelldeck-256.png") as icon:
        icon.save(PACK / "shelldeck.ico", sizes=[(s, s) for s in SIZES if s <= 256], bitmap_format="bmp")
    # Commit a ready-to-copy ICNS so packaging never depends on iconutil
    # accepting the legacy directory name "iconset" (without .iconset).
    icns_images = []
    for size in (32, 64, 128, 256, 512, 1024):
        with Image.open(PACK / f"shelldeck-{size}.png") as image:
            icns_images.append(image.convert("RGBA"))
    icns_images[-1].save(PACK / "shelldeck.icns", format="ICNS", append_images=icns_images)
    for name, size in {
        "icon_16x16.png": 16, "icon_16x16@2x.png": 32, "icon_32x32.png": 32,
        "icon_32x32@2x.png": 64, "icon_128x128.png": 128, "icon_128x128@2x.png": 256,
        "icon_256x256.png": 256, "icon_256x256@2x.png": 512,
        "icon_512x512.png": 512, "icon_512x512@2x.png": 1024,
    }.items():
        shutil.copyfile(PACK / f"shelldeck-{size}.png", PACK / "iconset" / name)
    with tempfile.TemporaryDirectory(prefix="inklura-brand-") as temporary:
        temporary = Path(temporary)
        mark = temporary / "mark.svg"
        mark.write_text(MARK.replace("currentColor", "#000000"))
        rasterize(mark, PACK / "shelldeck-tray-template-macos.png", 36)
        base_path = temporary / "icon.png"
        rasterize(SOURCE, base_path, 176)
        with Image.open(base_path) as image:
            base = image.convert("RGBA")
        for group, name, amplitude, period in (
            ("modes", "monolith-user", 2, 100), ("modes", "monolith-support", 1, 90),
            ("modes", "monolith-dev", 1, 80), ("studies", "monolith-thinking", 2, 110),
            ("studies", "monolith-scan", 1, 90), ("studies", "monolith-terminal-typing", 1, 80),
        ):
            frames = []
            for frame in range(24):
                canvas = Image.new("RGBA", (192, 192))
                offset = round(amplitude * math.sin(frame * math.tau / 24))
                canvas.alpha_composite(base, (8, 8 + offset))
                frames.append(canvas)
            output = BRAND / f"webp/{group}/{name}.webp"
            output.parent.mkdir(parents=True, exist_ok=True)
            frames[0].save(output, format="WEBP", save_all=True, append_images=frames[1:],
                           duration=period, loop=0, lossless=True, method=6)
    (BRAND / "manifest.json").write_text(json.dumps({
        "name": "Inklura Desktop", "version": 2, "source": str(SOURCE.relative_to(ROOT)),
        "primary": "#146BFF", "themes": list(THEMES),
        "note": "Legacy filenames are retained for loader and package compatibility.",
        "regenerate": "python3 scripts/export-inklura-brand.py",
    }, indent=2) + "\n")
    print("Exported Inklura Desktop app icons, 13 theme badges, tray mask and six motion assets")


if __name__ == "__main__":
    main()
