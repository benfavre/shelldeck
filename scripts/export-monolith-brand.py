#!/usr/bin/env python3
"""Regenerate ShellDeck Monolith brand assets (SVG + PNG + packaging icons).

Requires: ImageMagick `convert` (or `magick`) on PATH.

Usage:
  python3 scripts/export-monolith-brand.py
"""
from __future__ import annotations

import json
import shutil
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
BRAND = ROOT / "crates/shelldeck/assets/images/brand"
IMAGES = ROOT / "crates/shelldeck/assets/images"
PACK = ROOT / "packaging/icons"

# Original enlarged Monolith geometry, square-centered without thinning:
# legacy bbox 716×774 → uniform horizontal scale to 774×774 + vertical center (+32px).
FRAME_OUTER = (
    "M 207 125 L 817 125 L 899 195 L 899 829 L 817 899 L 207 899 L 125 829 L 125 195 Z"
)
FRAME_INNER = (
    "M 246 189 L 778 189 L 818 225 L 818 799 L 778 827 L 246 827 L 207 799 L 207 225 Z"
)

FACE_DEFAULT = """  <path fill="{face}" d="M 261 432 L 382 512 L 261 592 L 304 592 L 426 512 L 304 432 Z"/>
  <path fill="{face}" d="M 763 432 L 642 512 L 763 592 L 720 592 L 598 512 L 720 432 Z"/>
  <rect x="417" y="624" width="190" height="40" rx="6" fill="{face}"/>"""

FACE_NEUTRAL = """  <path fill="{face}" d="M 304 416 L 477 544 L 304 672 L 374 672 L 547 544 L 374 416 Z"/>
  <rect x="547" y="640" width="173" height="64" rx="6" fill="{face}"/>"""

FACE_WINK = """  <path fill="{face}" d="M 304 416 L 477 544 L 304 672 L 374 672 L 547 544 L 374 416 Z"/>
  <rect x="547" y="656" width="173" height="20" rx="4" fill="{face}"/>"""

EXPRESSIONS = {
    "default": FACE_DEFAULT,
    "neutral": FACE_NEUTRAL,
    "wink": FACE_WINK,
}

THEMES = [
    ("dark", "#1a1a1a", "#2ec4a8", "#1a1a1a", "#e6e6e6", "Dark"),
    ("light", "#f5f5f5", "#15c097", "#ededed", "#1f1f1f", "Light"),
    ("dracula", "#282a36", "#bd93f9", "#282a36", "#f8f8f2", "Dracula"),
    ("nord", "#2e3440", "#88c0d0", "#2e3440", "#eceff4", "Nord"),
    ("tokyo-night", "#1a1b26", "#7aa2f7", "#1a1b26", "#c0caf5", "Tokyo Night"),
    ("gruvbox-dark", "#282828", "#fe8019", "#282828", "#ebdbb2", "Gruvbox Dark"),
    ("solarized-dark", "#002b36", "#268bd2", "#002b36", "#93a1a1", "Solarized Dark"),
    ("solarized-light", "#fdf6e3", "#268bd2", "#eee8d5", "#586e75", "Solarized Light"),
    ("catppuccin-mocha", "#1e1e2e", "#cba6f7", "#1e1e2e", "#cdd6f4", "Catppuccin Mocha"),
    ("one-dark", "#282c34", "#61afef", "#282c34", "#abb2bf", "One Dark"),
    ("monokai", "#272822", "#f92672", "#272822", "#f8f8f2", "Monokai"),
    ("rose-pine", "#191724", "#c4a7e7", "#191724", "#e0def4", "Rosé Pine"),
]

PNG_SIZES = (16, 32, 48, 64, 128, 256, 512, 1024)
ICONSET_MAP = {
    16: "icon_16x16.png",
    32: ("icon_16x16@2x.png", "icon_32x32.png"),
    64: "icon_32x32@2x.png",
    128: "icon_128x128.png",
    256: ("icon_128x128@2x.png", "icon_256x256.png"),
    512: ("icon_256x256@2x.png", "icon_512x512.png"),
    1024: "icon_512x512@2x.png",
}


def convert_cmd() -> list[str]:
    for cmd in (["magick"], ["convert"]):
        try:
            subprocess.run(cmd + ["-version"], capture_output=True, check=True)
            return cmd
        except (FileNotFoundError, subprocess.CalledProcessError):
            continue
    sys.exit("ImageMagick (convert/magick) required")


def svg_body(canvas: str, primary: str, inner: str, face_tpl: str, face: str, rounded: bool) -> str:
    rx = ' rx="224"' if rounded else ""
    face_block = face_tpl.format(face=face)
    return f"""  <rect width="1024" height="1024"{rx} fill="{canvas}"/>
  <path fill="{primary}" d="{FRAME_OUTER}"/>
  <path fill="{inner}" d="{FRAME_INNER}"/>
{face_block}"""


def write_svg(path: Path, body: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(
        '<?xml version="1.0" encoding="UTF-8"?>\n'
        f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1024 1024" width="1024" height="1024">\n'
        f"{body}\n</svg>\n",
        encoding="utf-8",
    )


def rasterize(cmd: list[str], src: Path, dst: Path, size: int) -> None:
    dst.parent.mkdir(parents=True, exist_ok=True)
    subprocess.run(
        [*cmd, "-background", "none", "-density", "300", str(src), "-resize", f"{size}x{size}", str(dst)],
        check=True,
        capture_output=True,
    )


def main() -> None:
    cmd = convert_cmd()
    themes_dir = BRAND / "svg/themes"
    expr_dir = BRAND / "svg/expressions"
    themes_dir.mkdir(parents=True, exist_ok=True)
    expr_dir.mkdir(parents=True, exist_ok=True)

    all_themes = list(THEMES)
    all_themes.insert(2, ("system", "#1a1a1a", "#2ec4a8", "#1a1a1a", "#e6e6e6", "System"))

    for slug, canvas, primary, inner, face, _label in all_themes:
        if slug == "system":
            continue
        for variant, rounded in (("logo", False), ("app-icon", True)):
            suffix = "app-icon" if variant == "app-icon" else "logo"
            body = svg_body(canvas, primary, inner, FACE_DEFAULT, face, rounded)
            write_svg(themes_dir / f"{slug}-{suffix}.svg", body)

    shutil.copy(themes_dir / "dark-logo.svg", themes_dir / "system-logo.svg")
    shutil.copy(themes_dir / "dark-app-icon.svg", themes_dir / "system-app-icon.svg")

    for expr_name, face_tpl in EXPRESSIONS.items():
        if expr_name == "default":
            for slug, *_ in all_themes:
                if slug == "system":
                    continue
                shutil.copy(
                    themes_dir / f"{slug}-logo.svg",
                    expr_dir / f"{slug}-default-logo.svg",
                )
                shutil.copy(
                    themes_dir / f"{slug}-app-icon.svg",
                    expr_dir / f"{slug}-default-app-icon.svg",
                )
            shutil.copy(expr_dir / "dark-default-logo.svg", expr_dir / "system-default-logo.svg")
            shutil.copy(
                expr_dir / "dark-default-app-icon.svg",
                expr_dir / "system-default-app-icon.svg",
            )
            continue
        for variant, rounded in (("logo", False), ("app-icon", True)):
            suffix = "app-icon" if variant == "app-icon" else "logo"
            for slug, canvas, primary, inner, face, _label in all_themes:
                if slug == "system":
                    continue
                body = svg_body(canvas, primary, inner, face_tpl, face, rounded)
                write_svg(expr_dir / f"{slug}-{expr_name}-{suffix}.svg", body)
            shutil.copy(
                expr_dir / f"dark-{expr_name}-{suffix}.svg",
                expr_dir / f"system-{expr_name}-{suffix}.svg",
            )

    app_icon_svg = themes_dir / "dark-app-icon.svg"
    app_png_dir = BRAND / "png/app-icon"
    pack_dir = PACK

    for size in PNG_SIZES:
        rasterize(cmd, app_icon_svg, app_png_dir / f"monolith-{size}.png", size)
        if size in (16, 32, 48, 64, 128, 256, 512, 1024):
            rasterize(cmd, app_icon_svg, pack_dir / f"shelldeck-{size}.png", size)

    # Windows ICO
    ico_inputs = [pack_dir / f"shelldeck-{s}.png" for s in (16, 32, 48, 64, 128, 256)]
    subprocess.run([*cmd, *map(str, ico_inputs), str(pack_dir / "shelldeck.ico")], check=True)

    # macOS iconset (icns via iconutil on macOS only)
    iconset = pack_dir / "iconset"
    iconset.mkdir(exist_ok=True)
    png_by_size = {s: pack_dir / f"shelldeck-{s}.png" for s in PNG_SIZES}
    for size, names in ICONSET_MAP.items():
        targets = (names,) if isinstance(names, str) else names
        for name in targets:
            shutil.copy(png_by_size[size], iconset / name)

    # In-app embeds — square logo (no dock rounding) for titlebar/sidebar UI.
    logo_svg = themes_dir / "dark-logo.svg"
    icon_body = svg_body("#1a1a1a", "#2ec4a8", "#1a1a1a", FACE_DEFAULT, "#e6e6e6", False)
    write_svg(
        IMAGES / "shelldeck-icon.svg",
        icon_body.replace('width="1024" height="1024"', 'width="1024" height="1024"', 1),
    )
    # shelldeck-icon: add width/height 32 for GPUI
    icon_text = (IMAGES / "shelldeck-icon.svg").read_text(encoding="utf-8")
    icon_text = icon_text.replace(
        'viewBox="0 0 1024 1024" width="1024" height="1024"',
        'viewBox="0 0 1024 1024" width="32" height="32"',
    )
    (IMAGES / "shelldeck-icon.svg").write_text(icon_text, encoding="utf-8")

    mark_path = IMAGES / "shelldeck-mark.svg"
    mark_path.write_text(
        '<?xml version="1.0" encoding="UTF-8"?>\n'
        '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1024 1024" fill="currentColor">\n'
        f'  <path fill-rule="evenodd" d="{FRAME_OUTER} {FRAME_INNER}"/>\n'
        + FACE_DEFAULT.format(face="currentColor").replace('fill="currentColor"', "").replace("<path ", '<path fill="currentColor" ')
        + "\n</svg>\n",
        encoding="utf-8",
    )
    # Fix mark — build manually
    mark_path.write_text(
        '<?xml version="1.0" encoding="UTF-8"?>\n'
        '<!-- Monolith mark — monochrome, currentColor -->\n'
        '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1024 1024" fill="currentColor">\n'
        f'  <path fill-rule="evenodd" d="{FRAME_OUTER} {FRAME_INNER}"/>\n'
        '  <path d="M 261 432 L 382 512 L 261 592 L 304 592 L 426 512 L 304 432 Z"/>\n'
        '  <path d="M 763 432 L 642 512 L 763 592 L 720 592 L 598 512 L 720 432 Z"/>\n'
        '  <rect x="417" y="624" width="190" height="40" rx="6"/>\n'
        "</svg>\n",
        encoding="utf-8",
    )

    rasterize(cmd, logo_svg, IMAGES / "shelldeck-icon.png", 128)

    manifest = {
        "name": "ShellDeck Monolith",
        "version": 1,
        "expression_default": ">_<",
        "expressions": list(EXPRESSIONS.keys()),
        "themes": [t[0] for t in all_themes],
        "paths": {
            "brand_root": "crates/shelldeck/assets/images/brand",
            "in_app": {
                "icon_png": "crates/shelldeck/assets/images/shelldeck-icon.png",
                "icon_svg": "crates/shelldeck/assets/images/shelldeck-icon.svg",
                "mark": "crates/shelldeck/assets/images/shelldeck-mark.svg",
                "expressions": "crates/shelldeck/assets/images/brand/svg/expressions/dark-{expression}-logo.svg",
            },
            "themes": "crates/shelldeck/assets/images/brand/svg/themes/{theme}-logo.svg",
            "packaging": {
                "png": "packaging/icons/shelldeck-{size}.png",
                "ico": "packaging/icons/shelldeck.ico",
                "iconset": "packaging/icons/iconset/",
                "icns": "packaging/icons/shelldeck.icns (macOS: iconutil -c icns iconset -o shelldeck.icns)",
            },
        },
        "regenerate": "python3 scripts/export-monolith-brand.py",
    }
    (BRAND / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
    print(f"OK — {len(all_themes)} themes, {len(all_themes) * len(EXPRESSIONS) * 2} expression SVGs")


if __name__ == "__main__":
    main()
