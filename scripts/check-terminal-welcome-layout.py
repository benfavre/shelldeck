#!/usr/bin/env python3
"""SDTEST-2949: inspect actual GPUI native prepaint metrics, not test font metrics.

Run the provider-free agent-console-preview with disposable XDG roots and
SHELLDECK_PREVIEW_TERMINAL=1 SHELLDECK_PREVIEW_TERMINAL_METRICS=1.
SHELLDECK_PREVIEW_FONT_SIZE=17.5 reproduces the reported 125% text-scale case.
Pass its combined output log here. The three-argument form also checks the
expected native scale: script LOG 1.25.
"""
import json
import sys
from pathlib import Path


def check(log, expected_scale=None):
    records = {}
    for line in log.splitlines():
        if line.startswith("TERMINAL_LAYOUT "):
            record = json.loads(line.removeprefix("TERMINAL_LAYOUT "))
            records[record["id"]] = record
    assert "terminal-empty-scroll" in records, "no native terminal prepaint metrics"
    if expected_scale is not None:
        assert abs(records["terminal-empty-scroll"]["scale"] - expected_scale) < 0.02, ("scale mismatch", records["terminal-empty-scroll"]["scale"], expected_scale)
    assert "empty-launch-terminal" in records, "primary action was not painted"
    cards = 0
    labels = 0
    for name, record in records.items():
        scale = record["scale"]
        children = record["children"]
        if name.startswith("empty-launch-"):
            assert len(children) == 2, (name, children)
            icon, text = children
            assert text[2] / scale >= 70, (name, "text collapsed", text)
            assert text[3] / scale <= 62, (name, "text stretched vertically", text)
            assert abs((icon[1] + icon[3] / 2) - (text[1] + text[3] / 2)) <= 2, (name, "icon/text centers differ", children)
            cards += 1
        if name.startswith("shortcut-row-"):
            assert len(children) == 2, (name, children)
            label, keys = children
            assert label[2] / scale >= 70, (name, "label collapsed", label)
            assert label[3] / scale <= 34, (name, "label wraps one character per line", label)
            assert keys[2] > 10 and keys[3] / scale < 32, (name, "keycap stretched", keys)
            labels += 1
    assert cards >= 1 and labels >= 5, (cards, labels)
    top, content, bottom = records["terminal-empty-scroll"]["children"]
    assert abs(top[3] - bottom[3]) <= 2, ("unequal centering insets", top, bottom)
    return cards, labels


if __name__ == "__main__":
    try:
        cards, labels = check(Path(sys.argv[1]).read_text(errors="replace"), float(sys.argv[2]) if len(sys.argv) > 2 else None)
    except (AssertionError, IndexError, ValueError) as error:
        sys.exit(f"FAIL: {error}")
    print(f"PASS: native geometry for {cards} launch cards and {labels} shortcut rows")
