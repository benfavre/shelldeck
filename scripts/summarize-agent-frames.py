#!/usr/bin/env python3
"""Summarize opt-in native GPUI timings; CPU headroom is not displayed FPS."""
import argparse
import json
from pathlib import Path
import re

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("log", type=Path)
parser.add_argument("--samples", type=int, default=179)
args = parser.parse_args()
text = args.log.read_text(errors="replace")
marker = text.find("native large-conversation frame cadence:")
if marker >= 0:
    text = text[:marker]
# Use the same rendered-frame window for sparse input samples as CPU timings;
# otherwise startup pointer/resize events leak into steady-state latency.
draws = list(re.finditer(r"frame cpu draw: [0-9.]+(?:ns|µs|us|ms|s)", text))
if len(draws) >= max(1, args.samples):
    text = text[draws[-max(1, args.samples)].start():]
result = {}
for phase in ("cpu draw", "gpu present", "arena clear", "duration", "input-to-submit"):
    values = [float(value) * {"ns": 1e-6, "µs": 1e-3, "us": 1e-3, "ms": 1, "s": 1000}[unit]
              for value, unit in re.findall(r"frame " + phase + r": ([0-9.]+)(ns|µs|us|ms|s)", text)]
    values = sorted(values[-max(1, args.samples):])
    if values:
        result[phase] = {"samples": len(values), "p50_ms": values[len(values) // 2],
                         "p95_ms": values[min(len(values) - 1, int(len(values) * .95))],
                         "p99_ms": values[min(len(values) - 1, int(len(values) * .99))],
                         "max_ms": values[-1]}
        if phase == "cpu draw":
            result[phase]["over_120hz_budget"] = sum(value >= 1000 / 120 for value in values)
if not result:
    parser.error("No production frame timings found; enable ZED_MEASUREMENTS=1 and use the binary-package fixture")
if "cpu draw" in result:
    result["cpu_budget"] = {"target_120hz_ms": 1000 / 120,
                            "p95_under_120hz_budget": result["cpu draw"]["p95_ms"] < 1000 / 120}
if "duration" in result:
    result["frame_work_p95_under_120hz_budget"] = result["duration"]["p95_ms"] < 1000 / 120
print(json.dumps(result, indent=2))
