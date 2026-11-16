#!/usr/bin/env python3
"""Export the "ludique" street artwork used by the welcome screen, the home
banner and the empty states of Inklura Desktop.

The vector source is the street of the inklura.fr homepage ("Le quartier"),
kept in `scripts/ludique-art/source.json`. This script assembles it into the
few images the application draws:

  town-day.svg / town-night.svg   far skyline, back row and nine buildings
  road.svg                        pavement and road, drawn under the town
  sun-a.svg / sun-b.svg           the two frames of the sun (rays turn)
  moon.svg, stars.svg             the night sky
  cloud-day.svg / cloud-night.svg
  balloon.svg, van.svg, bird.svg, parcel.svg, pin.svg
  badge-*-day.svg / badge-*-night.svg   section marks of "Mes informations"

GPUI draws a colour SVG through `img()` at twice its intrinsic size, so every
file states the size it is shown at (or a little more) in `width`/`height`.
The town and the road are 2400 units wide and repeat without a seam.

    python3 scripts/export-ludique-art.py            # write the assets
    python3 scripts/export-ludique-art.py --check    # fail if they are stale
"""

import json
import random
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SOURCE = ROOT / "scripts" / "ludique-art" / "source.json"
OUT = ROOT / "crates" / "shelldeck" / "assets" / "images" / "ludique"

INK = "#14161c"
STROKE = f'fill="none" stroke="{INK}" stroke-width="2.5" stroke-linejoin="round" stroke-linecap="round"'

# Colours of the back rows, by class, as the site defines them for day and night.
BACK = {
    "day": {
        "far": "#C3D7FB", "f1": "#E9F0FE", "f2": "#D3E1FC", "f3": "#F6EEDC",
        "r1": "#F6BFB0", "r2": "#F8DE98", "tr": "#A9E2C8", "bw": "#ffffff",
        "bd": "#BDD0F6", "line": "#7C89A6", "pave": "#FFF4DE",
    },
    "night": {
        "far": "#1D2858", "f1": "#27346F", "f2": "#2E3D80", "f3": "#3A3768",
        "r1": "#5A4170", "r2": "#44508F", "tr": "#2C6A69", "bw": "#FFD86B",
        "bd": "#1B2552", "line": "#0B1030", "pave": "#D9CBAA",
    },
}
NIGHT_WINDOW = "#FFE08A"
NIGHT_DIM = 0.84

TOWN_W, TOWN_H = 2400, 300
GROUND = TOWN_H  # the baseline every building stands on
ROAD_H, PAVE_H = 56, 20

# Left to right. Buildings are 180 wide, lands 120, lamps 40; gaps are 16.
STREET = [
    ("land", "bench"), ("building", "boulangerie"), ("lamp", None),
    ("building", "agence"), ("building", "cafe"), ("land", "fountain"),
    ("building", "commune"), ("building", "librairie"), ("lamp", None),
    ("building", "fleuriste"), ("building", "restaurant"), ("land", "busstop"),
    ("building", "jouets"), ("lamp", None), ("building", "port"),
]
WIDTHS = {"building": 180, "land": 120, "lamp": 40}
HEIGHTS = {"building": 204, "land": 204, "lamp": 170}
GAP = 16
# The cloud outline reaches past the box the site gives it (it lets it overflow).
CLOUD_BOX = "-11 -20 98 52"


def svg(width, height, view_box, body):
    return (
        f'<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" '
        f'viewBox="{view_box}">{body}</svg>\n'
    )


def dim(hex_colour, factor):
    value = hex_colour.lstrip("#")
    if len(value) == 3:
        value = "".join(ch * 2 for ch in value)
    parts = [int(value[i:i + 2], 16) for i in (0, 2, 4)]
    return "#" + "".join(f"{round(p * factor):02x}" for p in parts)


def plain(body, night):
    """Drop the site's animation hooks; at night light the windows and dim the rest."""

    def element(match):
        tag = match.group(0)
        lit = re.search(r'class="[^"]*\blw\b', tag) is not None
        tag = re.sub(r'\s(class|style)="[^"]*"', "", tag)
        if not night:
            return tag
        if lit:
            return re.sub(r'fill="#[0-9A-Fa-f]{3,6}"', f'fill="{NIGHT_WINDOW}"', tag)
        return re.sub(
            r'fill="(#[0-9A-Fa-f]{3,6})"',
            lambda m: f'fill="{dim(m.group(1), NIGHT_DIM)}"',
            tag,
        )

    return re.sub(r"<[a-z]+\b[^>]*>", element, body)


def back_row(body, colours):
    """Inline the fills the site gives the back row through CSS classes."""

    def element(match):
        tag = match.group(0)
        found = re.search(r'class="([^"]*)"', tag)
        if not found:
            return tag
        classes = found.group(1).split()
        tag = re.sub(r'\sclass="[^"]*"', "", tag)
        key = classes[0].removeprefix("lc-")
        if key == "ln":
            return tag
        extra = f' fill="{colours[key]}"'
        if key in ("bw", "bd"):
            extra += ' stroke="none"'
        return tag[:-2] + extra + "/>" if tag.endswith("/>") else tag[:-1] + extra + ">"

    return re.sub(r"<[a-z]+\b[^>]*>", element, body)


def town(source, mode):
    colours = BACK[mode]
    night = mode == "night"
    parts = []

    # Far silhouettes: a 480 x 340 tile, six of them across the strip.
    scale = (TOWN_W / 6) / 480
    far = "".join(
        f'<rect x="{x}" y="{y}" width="{w}" height="{h}"/>' for x, y, w, h in source["far"]
    )
    for i in range(6):
        parts.append(
            f'<g transform="translate({i * TOWN_W / 6:g} {GROUND - 340 * scale:g}) '
            f'scale({scale:g})" fill="{colours["far"]}">{far}</g>'
        )

    # Back row: two 1200-wide tiles of muted buildings and trees.
    for i, body in enumerate(source["back"][:2]):
        parts.append(
            f'<g transform="translate({i * 1200} {GROUND - 300})" fill="none" '
            f'stroke="{colours["line"]}" stroke-width="2" stroke-linejoin="round" '
            f'stroke-linecap="round">{back_row(body, colours)}</g>'
        )

    total = sum(WIDTHS[kind] for kind, _ in STREET) + GAP * (len(STREET) - 1)
    x = (TOWN_W - total) / 2
    for kind, name in STREET:
        if kind == "building":
            body = plain(source["buildings"][name], night)
        elif kind == "land":
            body = plain(source["lands"][name], night)
        else:
            glow = 0.5 if night else 0
            body = plain(source["lamp"], False).replace(
                'fill="#FFE08A" stroke="none"',
                f'fill="#FFE08A" fill-opacity="{glow}" stroke="none"',
            )
        parts.append(
            f'<g transform="translate({x:g} {GROUND - HEIGHTS[kind]})" {STROKE}>{body}</g>'
        )
        x += WIDTHS[kind] + GAP

    return svg(1560, 195, f"0 0 {TOWN_W} {TOWN_H}", "".join(parts))


def road(mode):
    colours = BACK[mode]
    height = PAVE_H + ROAD_H
    middle = PAVE_H + ROAD_H / 2
    parts = [
        f'<rect width="{TOWN_W}" height="{PAVE_H}" fill="{colours["pave"]}"/>',
        f'<rect y="{PAVE_H}" width="{TOWN_W}" height="{ROAD_H}" fill="#3b404c"/>',
        f'<path d="M0 {middle}H{TOWN_W}" stroke="#ffffff" stroke-opacity="0.5" '
        f'stroke-width="3" stroke-dasharray="16 16" fill="none"/>',
    ]
    # Zebra crossings, placed so the strip still repeats.
    for x in (420, 1620):
        for i in range(6):
            parts.append(
                f'<rect x="{x}" y="{PAVE_H + 3 + i * 9}" width="44" height="5" '
                f'fill="#ffffff" fill-opacity="0.8"/>'
            )
    return svg(1560, round(1560 * height / TOWN_W, 2), f"0 0 {TOWN_W} {height}", "".join(parts))


def sun(source, turned):
    body = plain(source["sun"], False)
    if turned:
        body = body.replace("<g>", '<g transform="rotate(22.5 70 70)">', 1)
    return svg(140, 140, "0 0 140 140", f"<g {STROKE}>{body}</g>")


def stars():
    rng = random.Random(29)
    dots = []
    for _ in range(46):
        x, y = rng.uniform(4, 796), rng.uniform(4, 296)
        radius = rng.choice((1.0, 1.2, 1.2, 1.6))
        colour = "#FFE9A8" if radius > 1.5 else "#ffffff"
        dots.append(
            f'<circle cx="{x:.1f}" cy="{y:.1f}" r="{radius}" fill="{colour}" '
            f'fill-opacity="{rng.uniform(0.55, 0.95):.2f}"/>'
        )
    return svg(800, 300, "0 0 800 300", "".join(dots))


def sprite(body, view_box, width, height, extra=""):
    return svg(width, height, view_box, f"<g {STROKE}{extra}>{plain(body, False)}</g>")


# Section marks: a drawing of the street with three strokes of the marker
# yellow. They are shown on a panel of the theme, not on the sky, so the night
# variant (dark themes) draws them with a light line and darker fills.
BADGE_LINE_NIGHT = "#E6EBFA"
BADGE_SPARKS = (
    '<path d="M92 20l7-9M98 33l11-4M82 12l2-10" stroke="#FFC83D" '
    'stroke-width="4" fill="none" stroke-linecap="round"/>'
)
BADGE_LAPTOP = (
    '<rect x="20" y="26" width="60" height="42" rx="5" fill="#D6E6FF"/>'
    '<path d="M10 78h80l-6 9H16z" fill="#fff"/>'
)
BADGE_SHIELD = (
    '<path d="M50 14l30 10v22c0 21-13 34-30 42-17-8-30-21-30-42V24z" fill="#fff"/>'
    '<path d="M37 51l10 10 17-19" stroke="#146BFF" stroke-width="5"/>'
)


def badge(body, night, scale=1.0, shift=(0, 0)):
    stroke = STROKE
    if night:
        # Paper turns to the night facade colour before the usual dimming.
        body = re.sub(r'fill="#(?:fff|ffffff)"', 'fill="#3A4A99"', body, flags=re.I)
    body = plain(body, night)
    if night:
        stroke = STROKE.replace(INK, BADGE_LINE_NIGHT)
        body = body.replace(f'fill="{INK}"', f'fill="{BADGE_LINE_NIGHT}"')
    drawing = (
        f'<g transform="translate({shift[0]} {shift[1]}) scale({scale:g})" {stroke}>{body}</g>'
    )
    return svg(112, 100, "0 0 112 100", drawing + BADGE_SPARKS)


def badges(source):
    drawings = {
        "account": (source["parcel"], 1.0, (0, 4)),
        # A building of the street, brought down to the size of the others.
        "shop": (source["buildings"]["librairie"], 0.46, (4, 6)),
        "laptop": (BADGE_LAPTOP, 1.0, (0, 6)),
        "shield": (BADGE_SHIELD, 1.0, (0, 6)),
    }
    files = {}
    for name, (body, scale, shift) in drawings.items():
        files[f"badge-{name}-day.svg"] = badge(body, False, scale, shift)
        files[f"badge-{name}-night.svg"] = badge(body, True, scale, shift)
    return files


def build():
    source = json.loads(SOURCE.read_text(encoding="utf-8"))
    return badges(source) | {
        "town-day.svg": town(source, "day"),
        "town-night.svg": town(source, "night"),
        "road-day.svg": road("day"),
        "road-night.svg": road("night"),
        "sun-a.svg": sun(source, False),
        "sun-b.svg": sun(source, True),
        "moon.svg": sprite(source["moon"], "28 24 96 96", 140, 140),
        "stars.svg": stars(),
        "cloud-day.svg": sprite(source["cloud"], CLOUD_BOX, 196, 104),
        "cloud-night.svg": svg(
            196, 104, CLOUD_BOX,
            f'<g fill="#ffffff" fill-opacity="0.16">{plain(source["cloud"], False)}</g>'.replace(
                ' fill="#fff"', ""
            ),
        ),
        "balloon.svg": sprite(source["balloon"], "0 0 90 126", 90, 126),
        "van.svg": sprite(source["van"], "0 -2 150 90", 150, 90),
        "bird.svg": sprite(source["bird"], "0 0 40 20", 40, 20),
        "parcel.svg": sprite(source["parcel"], "0 0 100 100", 100, 100),
        "pin.svg": sprite(source["pin"], "0 0 36 48", 36, 48),
    }


def main():
    files = build()
    if "--check" in sys.argv:
        stale = [
            name for name, text in files.items()
            if not (OUT / name).exists() or (OUT / name).read_text(encoding="utf-8") != text
        ]
        extra = sorted(p.name for p in OUT.glob("*.svg") if p.name not in files)
        if stale or extra:
            print("ludique art is stale:", ", ".join(stale + extra))
            return 1
        print(f"ludique art is up to date ({len(files)} files)")
        return 0
    OUT.mkdir(parents=True, exist_ok=True)
    for name, text in files.items():
        (OUT / name).write_text(text, encoding="utf-8")
    print(f"wrote {len(files)} files to {OUT.relative_to(ROOT)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
