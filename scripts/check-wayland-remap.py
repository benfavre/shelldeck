#!/usr/bin/env python3
"""SDTEST-2943: check a provider-free SHELLDECK_PREVIEW_REMAP Wayland trace.

Run the isolated agent-console-preview with SHELLDECK_PREVIEW_REMAP=1,
SHELLDECK_PREVIEW_SECONDS=8, WAYLAND_DEBUG=1 and three disposable XDG roots.
Pass its combined stdout/stderr log as the sole argument. No desktop input.
"""
import re
import sys
from pathlib import Path


def check(log):
    hidden = False
    cycle = 0
    configure = False
    acknowledged = False
    presented = set()
    for line in log.splitlines():
        marker = re.search(r"REMAP (\d+) (HIDE|SHOW)", line)
        if marker:
            cycle = int(marker[1])
            hidden = marker[2] == "HIDE"
            configure = acknowledged = False
        if cycle and "xdg_surface@" in line:
            if ".configure(" in line:
                configure = True
            if ".ack_configure(" in line:
                acknowledged = configure
        if cycle and re.search(r"wl_surface@\d+\.attach\(wl_buffer@", line):
            assert not hidden, f"cycle {cycle}: GPU buffer attached while hidden"
            if cycle not in presented:
                assert acknowledged, f"cycle {cycle}: remap buffer preceded fresh configure/ack"
                presented.add(cycle)
        assert "wl_display@1.error(" not in line, f"Wayland protocol error: {line}"
    assert presented == {1, 2}, f"expected two remapped presentations, got {presented}"


if __name__ == "__main__":
    try:
        check(Path(sys.argv[1]).read_text(errors="replace"))
    except (AssertionError, IndexError) as error:
        sys.exit(f"FAIL: {error}")
    print("PASS: two native remaps configured before presenting; no hidden buffer attachments")
