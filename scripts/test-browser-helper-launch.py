#!/usr/bin/env python3
"""SDTEST-2598 / SDUC-557: launch the packaged helper's actual stdio entry point.

Usage: python3 scripts/test-browser-helper-launch.py target/debug/browser
This explicitly launches only the browser helper, with no profile or provider.
The missing capability must fail in Rust's MCP proxy after the native launcher
loads the client DLL/framework; an OS loader error is never accepted as a pass.
"""
from pathlib import Path
import json
import os
import re
import subprocess
import sys


def sdtest_2598_packaged_helper_entry_point(bundle: Path) -> None:
    root = Path(__file__).resolve().parent.parent
    package = (root / "Cargo.toml").read_text().split("[workspace.package]", 1)[1].split("\n[", 1)[0]
    version = re.search(r'^version\s*=\s*"([^"]+)"', package, re.MULTILINE).group(1)
    metadata = bundle / "runtime.json"
    if not metadata.is_file():
        metadata = bundle / version / "runtime.json"
    details = json.loads(metadata.read_text())
    if details["shelldeck_version"] != version:
        raise RuntimeError("The staged helper does not match the application version")
    directory = metadata.parent.resolve()
    executable = (directory / details["executable"]).resolve()
    if not executable.is_relative_to(directory) or not executable.is_file():
        raise RuntimeError("The staged helper executable escaped its bundle or is missing")
    environment = dict(os.environ)
    environment.pop("SHELLDECK_BROWSER_ENDPOINT", None)
    environment.pop("SHELLDECK_BROWSER_CAPABILITY", None)
    if sys.platform == "linux":
        environment["LD_LIBRARY_PATH"] = str(executable.parent) + os.pathsep + environment.get("LD_LIBRARY_PATH", "")
    elif sys.platform == "win32":
        environment["PATH"] = str(executable.parent) + os.pathsep + environment.get("PATH", "")
    result = subprocess.run([str(executable), "--mcp"], env=environment,
                            capture_output=True, text=True, timeout=15)
    if result.returncode != 1 or "ShellDeck browser bridge:" not in result.stderr:
        raise RuntimeError(f"Packaged helper failed before its MCP entry point: exit={result.returncode}, stderr={result.stderr[:2000]}")
    print("SDTEST-2598 passed: packaged native helper reached its scoped MCP entry point")


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit("Usage: test-browser-helper-launch.py target/PROFILE/browser")
    sdtest_2598_packaged_helper_entry_point(Path(sys.argv[1]).resolve())
