#!/usr/bin/env bash
#
# Package an already-built (and, in CI, already-signed) release payload.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$PROJECT_ROOT"

PLATFORM="${SHELLDECK_RELEASE_PLATFORM:-}"
if [ -z "$PLATFORM" ]; then
    case "$(uname -s):$(uname -m)" in
        Linux:x86_64) PLATFORM="linux-x86_64" ;;
        Darwin:arm64|Darwin:aarch64|Darwin:x86_64) PLATFORM="macos-aarch64" ;;
        MINGW*:x86_64|MSYS*:x86_64|CYGWIN*:x86_64|Windows_NT:*) PLATFORM="windows-x86_64" ;;
        *) echo "ERROR: Unsupported packaging platform" >&2; exit 1 ;;
    esac
fi

VERSION="$(sed -n '/^\[workspace.package\]/,/^\[/p' Cargo.toml | sed -n 's/^version = "\([^"]*\)"/\1/p' | head -1)"
if [ -z "$VERSION" ]; then
    echo "ERROR: Missing workspace version" >&2
    exit 1
fi

check_browser_resources() {
    local root="$1" resource
    for resource in icudtl.dat resources.pak chrome_100_percent.pak chrome_200_percent.pak v8_context_snapshot.bin locales/en-US.pak runtime.json; do
        if [ ! -f "$root/$resource" ]; then
            echo "ERROR: Missing browser resource: $root/$resource" >&2
            exit 1
        fi
    done
}

mkdir -p dist
case "$PLATFORM" in
    linux-x86_64)
        test -x target/release/shelldeck
        test -x "target/release/browser/$VERSION/shelldeck-browser"
        test -f "target/release/browser/$VERSION/libcef.so"
        check_browser_resources "target/release/browser/$VERSION"
        tar -czf dist/shelldeck-linux-x86_64.tar.gz -C target/release shelldeck "browser/$VERSION"
        ARCHIVE="dist/shelldeck-linux-x86_64.tar.gz"
        ;;
    macos-aarch64)
        test -d dist/ShellDeck.app
        test -x "dist/ShellDeck.app/Contents/Frameworks/browser/ShellDeck Browser.app/Contents/MacOS/shelldeck-browser"
        test -f "dist/ShellDeck.app/Contents/Frameworks/browser/ShellDeck Browser.app/Contents/Frameworks/Chromium Embedded Framework.framework/Chromium Embedded Framework"
        rm -f dist/shelldeck-macos-aarch64.zip
        ditto -c -k --sequesterRsrc --keepParent \
            dist/ShellDeck.app dist/shelldeck-macos-aarch64.zip
        ARCHIVE="dist/shelldeck-macos-aarch64.zip"
        ;;
    windows-x86_64)
        test -f target/release/shelldeck.exe
        test -f "target/release/browser/$VERSION/shelldeck-browser.exe"
        test -f "target/release/browser/$VERSION/shelldeck-browser.dll"
        test -f "target/release/browser/$VERSION/libcef.dll"
        check_browser_resources "target/release/browser/$VERSION"
        rm -f dist/shelldeck-windows-x86_64.zip
        python3 - "$VERSION" <<'PYZIP'
from pathlib import Path
import sys
import zipfile
root = Path("target/release")
version = sys.argv[1]
with zipfile.ZipFile("dist/shelldeck-windows-x86_64.zip", "w", zipfile.ZIP_DEFLATED) as archive:
    archive.write(root / "shelldeck.exe", "shelldeck.exe")
    for path in sorted((root / "browser" / version).rglob("*")):
        if path.is_file():
            archive.write(path, path.relative_to(root).as_posix())
PYZIP
        ARCHIVE="dist/shelldeck-windows-x86_64.zip"
        ;;
    *)
        echo "ERROR: Unsupported release platform: $PLATFORM" >&2
        exit 1
        ;;
esac

echo "Packaged $ARCHIVE ($(wc -c < "$ARCHIVE" | tr -d ' ') bytes)"
