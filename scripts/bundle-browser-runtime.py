#!/usr/bin/env python3
"""Stage CEF's complete, versioned runtime beside ShellDeck without linking GPUI to CEF.

Build first: cargo build --release --locked -p shelldeck-browser --features runtime
Then: python3 scripts/bundle-browser-runtime.py --profile release [--target TRIPLE]
The script reads cef-dll-sys's build metadata, preserving the exact CEF version
compiled by Cargo. It never downloads, modifies profiles, signs, or installs.
"""
from __future__ import annotations
import argparse
import json
import os
from pathlib import Path
import plistlib
import shutil
import sys
import tempfile
import re
import stat

ROOT = Path(__file__).resolve().parent.parent


def platform_name(target: str | None) -> str:
    if target:
        if "windows" in target: return "windows"
        if "apple-darwin" in target: return "macos"
        if "linux" in target: return "linux"
        raise ValueError(f"Unsupported browser target: {target}")
    return {"darwin": "macos", "win32": "windows"}.get(sys.platform, sys.platform)


def find_cef(build: Path, explicit: Path | None) -> Path:
    if explicit:
        directory = explicit.resolve()
        if not directory.is_dir(): raise FileNotFoundError(f"CEF directory missing: {directory}")
        return directory
    outputs = sorted((build / "build").glob("cef-dll-sys-*/output"), key=lambda path: path.stat().st_mtime, reverse=True)
    for output in outputs:
        for line in output.read_text(errors="replace").splitlines():
            if "metadata=CEF_DIR=" in line:
                directory = Path(line.split("metadata=CEF_DIR=", 1)[1])
                if directory.is_dir(): return directory
    raise FileNotFoundError("No matching cef-dll-sys build metadata. Build shelldeck-browser --features runtime first, or pass --cef-dir.")


def copy_file(source: Path, destination: Path) -> None:
    if not source.is_file(): raise FileNotFoundError(f"Required browser resource missing: {source}")
    destination.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(source, destination)


def flat_runtime(cef: Path, destination: Path) -> None:
    for source in cef.iterdir():
        if source.is_file() and (source.suffix.lower() in {".dll", ".pak", ".dat", ".bin"} or ".so" in source.name or source.name in {"chrome-sandbox", "vk_swiftshader_icd.json", "CREDITS.html", "LICENSE", "LICENSE.txt"}):
            copy_file(source, destination / source.name)
    locales = cef / "locales"
    if not locales.is_dir(): raise FileNotFoundError(f"CEF locales missing: {locales}")
    shutil.copytree(locales, destination / "locales", symlinks=True)
    # The flattened distribution carries credits; the original minimal archive
    # keeps CEF's license one directory up. Include it when available.
    if not (destination / "LICENSE.txt").is_file():
        for candidate in cef.parent.glob("cef_binary_*/LICENSE.txt"):
            copy_file(candidate, destination / "LICENSE.txt")
            break


def plist(destination: Path, executable: str, identifier: str, version: str) -> None:
    destination.parent.mkdir(parents=True, exist_ok=True)
    with destination.open("wb") as stream:
        plistlib.dump({
            "CFBundleExecutable": executable,
            "CFBundleIdentifier": identifier,
            "CFBundleName": executable,
            "CFBundleDisplayName": "ShellDeck Browser",
            "CFBundleVersion": version,
            "CFBundleShortVersionString": version,
            "CFBundleInfoDictionaryVersion": "6.0",
            "CFBundlePackageType": "APPL",
            "LSMinimumSystemVersion": "11.0",
            "LSUIElement": True,
            "LSEnvironment": {"MallocNanoZone": "0"},
            "NSHighResolutionCapable": True,
            "NSPrincipalClass": "NSApplication",
            "NSSupportsAutomaticGraphicsSwitching": True,
        }, stream)


def stage(build: Path, cef: Path, destination: Path, system: str, version: str) -> Path:
    # Exact CEF 154.0.34 notice from its upstream 14c5a08 source commit. The
    # downloader flattens runtime files and omits the original LICENSE.txt.
    copy_file(ROOT / "crates/shelldeck-browser/CEF-LICENSE.txt", destination / "CEF-LICENSE.txt")
    if (cef / "CREDITS.html").is_file(): copy_file(cef / "CREDITS.html", destination / "CREDITS.html")
    if system == "macos":
        app = destination / "ShellDeck Browser.app"
        contents = app / "Contents"
        executable = build / "shelldeck-browser"
        copy_file(executable, contents / "MacOS" / "shelldeck-browser")
        plist(contents / "Info.plist", "shelldeck-browser", "dev.bext.shelldeck.browser", version)
        copy_file(ROOT / "crates/shelldeck-browser/CEF-LICENSE.txt", contents / "Resources/CEF-LICENSE.txt")
        if (cef / "CREDITS.html").is_file(): copy_file(cef / "CREDITS.html", contents / "Resources/CREDITS.html")
        framework = cef / "Chromium Embedded Framework.framework"
        if not framework.is_dir(): raise FileNotFoundError(f"CEF framework missing: {framework}")
        shutil.copytree(framework, contents / "Frameworks" / framework.name, symlinks=True)
        for suffix, identifier in [("Helper", "helper"), ("Helper (Renderer)", "helper.renderer"), ("Helper (GPU)", "helper.gpu"), ("Helper (Plugin)", "helper.plugin"), ("Helper (Alerts)", "helper.alerts")]:
            name = f"shelldeck-browser {suffix}"
            helper = contents / "Frameworks" / f"{name}.app" / "Contents"
            copy_file(executable, helper / "MacOS" / name)
            plist(helper / "Info.plist", name, f"dev.bext.shelldeck.browser.{identifier}", version)
        return contents / "MacOS" / "shelldeck-browser"
    flat_runtime(cef, destination)
    if system == "windows":
        # cef-rs's bootstrap passes sandbox_info into RunWinMain in our DLL.
        copy_file(cef / "bootstrap.exe", destination / "shelldeck-browser.exe")
        copy_file(build / "shelldeck_browser.dll", destination / "shelldeck-browser.dll")
        if not (destination / "libcef.dll").is_file(): raise FileNotFoundError("CEF libcef.dll missing")
        return destination / "shelldeck-browser.exe"
    if system == "linux":
        copy_file(build / "shelldeck-browser", destination / "shelldeck-browser")
        if not (destination / "libcef.so").is_file(): raise FileNotFoundError("CEF libcef.so missing")
        # Rely on Chromium's unprivileged user namespace sandbox. We neither set
        # setuid bits nor weaken the Chromium renderer sandbox at packaging time.
        return destination / "shelldeck-browser"
    raise ValueError(f"Unsupported browser platform: {system}")


def normalize_runtime_permissions(directory: Path) -> None:
    """Publish application resources, not the private staging directory's mode.

    macOS framework links must remain links and must never chmod their targets
    outside this generated payload. Profiles are created by the runtime later;
    this function is called only on the fresh temporary distribution tree.
    """
    for current, directories, files in os.walk(directory, followlinks=False):
        current = Path(current)
        current.chmod(0o755)
        for name in files:
            path = current / name
            if path.is_symlink():
                continue
            mode = stat.S_IMODE(path.stat().st_mode)
            path.chmod(0o755 if mode & 0o111 else 0o644)
        for name in directories:
            path = current / name
            if not path.is_symlink():
                path.chmod(0o755)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--profile", default="release")
    parser.add_argument("--target")
    parser.add_argument("--target-dir", type=Path)
    parser.add_argument("--cef-dir", type=Path)
    parser.add_argument("--output", type=Path, help="Exact payload directory; default target/PROFILE/browser/VERSION (macOS browser/)")
    args = parser.parse_args()
    manifest = (ROOT / "Cargo.toml").read_text()
    package = manifest.split("[workspace.package]", 1)[1].split("\n[", 1)[0]
    match = re.search(r'^version\s*=\s*"(\d+\.\d+\.\d+(?:-[a-zA-Z0-9.-]+)?)"\s*$', package, re.MULTILINE)
    if not match: raise ValueError("Workspace package version is missing")
    version = match.group(1)
    target_dir = args.target_dir or Path(os.environ.get("CARGO_TARGET_DIR", ROOT / "target"))
    if not target_dir.is_absolute(): target_dir = ROOT / target_dir
    build = target_dir / args.target / args.profile if args.target else target_dir / args.profile
    system = platform_name(args.target)
    output = args.output or build / "browser" / (version if system != "macos" else "")
    output = output.resolve()
    if output == build.resolve() or output == ROOT or output == Path(output.anchor): raise ValueError("Refusing an unsafe browser output directory")
    cef = find_cef(build, args.cef_dir)
    output.parent.mkdir(parents=True, exist_ok=True)
    temporary = Path(tempfile.mkdtemp(prefix=".shelldeck-browser-stage-", dir=output.parent))
    try:
        executable = stage(build, cef, temporary, system, version)
        (temporary / "runtime.json").write_text(json.dumps({"shelldeck_version": version, "cef_bindings": "154.5.0+154.0.34", "platform": system, "executable": str(executable.relative_to(temporary)).replace("\\", "/")}, indent=2) + "\n")
        normalize_runtime_permissions(temporary)
        # Only replace generated build output. A published/runtime install is the
        # updater's responsibility; never put production profiles in this folder.
        if output.exists():
            if not (output / "runtime.json").is_file(): raise ValueError(f"Existing output is not a generated browser bundle: {output}")
            shutil.rmtree(output)
        temporary.rename(output)
        print(output)
    finally:
        if temporary.exists(): shutil.rmtree(temporary)

if __name__ == "__main__":
    try: main()
    except (OSError, ValueError) as error: sys.exit(f"Browser bundle failed: {error}")
