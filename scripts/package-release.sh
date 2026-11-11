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
        # The session daemon sits beside the application binary: that is
        # where the application, the installer and the updater expect it.
        if [ ! -x target/release/shelldeck-sessiond ]; then
            echo "ERROR: Missing session daemon: target/release/shelldeck-sessiond" >&2
            exit 1
        fi
        PAYLOAD=(shelldeck shelldeck-sessiond "browser/$VERSION")
        # The daemon files again, as a small archive of their own: an
        # install whose updater predates the daemon (it replaces the
        # application binary only) fetches them from it
        # (crates/shelldeck-update/src/sessiond.rs).
        SESSIOND_PAYLOAD=(shelldeck-sessiond)
        # Static builds for SSH servers, when they were built
        # (scripts/build-sessiond-remote.sh). Only these two names ship.
        for TRIPLE in x86_64-unknown-linux-musl aarch64-unknown-linux-musl; do
            REMOTE="shelldeck-sessiond-$TRIPLE"
            if [ -e "target/release/$REMOTE" ]; then
                if [ -L "target/release/$REMOTE" ] || [ ! -f "target/release/$REMOTE" ] \
                    || [ "$(head -c 4 "target/release/$REMOTE" | od -An -tx1 | tr -d ' ')" != "7f454c46" ]; then
                    echo "ERROR: target/release/$REMOTE is not an ELF file" >&2
                    exit 1
                fi
                PAYLOAD+=("$REMOTE")
                SESSIOND_PAYLOAD+=("$REMOTE")
            fi
        done
        rm -f dist/shelldeck-sessiond-linux-x86_64.tar.gz
        tar -czf dist/shelldeck-linux-x86_64.tar.gz -C target/release "${PAYLOAD[@]}"
        tar -czf dist/shelldeck-sessiond-linux-x86_64.tar.gz -C target/release \
            "${SESSIOND_PAYLOAD[@]}"
        ARCHIVE="dist/shelldeck-linux-x86_64.tar.gz"
        ;;
    macos-aarch64)
        test -d "dist/Inklura Desktop.app"
        if [ ! -s "dist/Inklura Desktop.app/Contents/Resources/shelldeck.icns" ]; then
            echo "ERROR: macOS bundle icon is missing; refusing to publish the archive" >&2
            exit 1
        fi
        # The session daemon sits beside the application binary
        # (packaging/macos/build-dmg.sh).
        if [ ! -x "dist/Inklura Desktop.app/Contents/MacOS/shelldeck-sessiond" ]; then
            echo "ERROR: macOS bundle has no session daemon; refusing to publish the archive" >&2
            exit 1
        fi
        test -x "dist/Inklura Desktop.app/Contents/Frameworks/browser/ShellDeck Browser.app/Contents/MacOS/shelldeck-browser"
        test -f "dist/Inklura Desktop.app/Contents/Frameworks/browser/ShellDeck Browser.app/Contents/Frameworks/Chromium Embedded Framework.framework/Chromium Embedded Framework"
        rm -f dist/shelldeck-macos-aarch64.zip
        ditto -c -k --sequesterRsrc --keepParent \
            "dist/Inklura Desktop.app" dist/shelldeck-macos-aarch64.zip
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
