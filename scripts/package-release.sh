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

mkdir -p dist
case "$PLATFORM" in
    linux-x86_64)
        test -x target/release/shelldeck
        tar -czf dist/shelldeck-linux-x86_64.tar.gz -C target/release shelldeck
        ARCHIVE="dist/shelldeck-linux-x86_64.tar.gz"
        ;;
    macos-aarch64)
        test -d dist/ShellDeck.app
        rm -f dist/shelldeck-macos-aarch64.zip
        ditto -c -k --sequesterRsrc --keepParent \
            dist/ShellDeck.app dist/shelldeck-macos-aarch64.zip
        ARCHIVE="dist/shelldeck-macos-aarch64.zip"
        ;;
    windows-x86_64)
        test -f target/release/shelldeck.exe
        rm -f dist/shelldeck-windows-x86_64.zip
        if command -v zip >/dev/null 2>&1; then
            (cd target/release && zip -j -q \
                ../../dist/shelldeck-windows-x86_64.zip shelldeck.exe)
        else
            powershell -NoProfile -Command \
                "Compress-Archive -Path 'target/release/shelldeck.exe' -DestinationPath 'dist/shelldeck-windows-x86_64.zip' -Force"
        fi
        ARCHIVE="dist/shelldeck-windows-x86_64.zip"
        ;;
    *)
        echo "ERROR: Unsupported release platform: $PLATFORM" >&2
        exit 1
        ;;
esac

echo "Packaged $ARCHIVE ($(wc -c < "$ARCHIVE" | tr -d ' ') bytes)"
