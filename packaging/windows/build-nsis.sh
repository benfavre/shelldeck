#!/usr/bin/env bash
#
# Build a ShellDeck NSIS installer for Windows.
#
# Usage:
#   bash packaging/windows/build-nsis.sh
#
# Runs on Windows (Git Bash / GitHub Actions).
# Requires NSIS to be installed and makensis in PATH.
#
# Output: dist/ShellDeck-windows-x86_64-setup.exe

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
cd "$PROJECT_ROOT"

BINARY="target/release/shelldeck.exe"
if [ ! -f "$BINARY" ]; then
    echo "ERROR: Binary not found at $BINARY" >&2
    echo "Build with: cargo build --release -p shelldeck" >&2
    exit 1
fi

VERSION="$(grep '^version' Cargo.toml | head -1 | sed 's/.*"\(.*\)".*/\1/')"
echo "==> Building NSIS installer for ShellDeck $VERSION"

mkdir -p dist

# Generate a placeholder .ico if one doesn't exist
if [ ! -f "packaging/icons/shelldeck.ico" ]; then
    echo "==> No shelldeck.ico found, generating placeholder..."
    mkdir -p packaging/icons
    if command -v magick &>/dev/null; then
        magick -size 256x256 xc:'#1a1b26' \
            -fill '#7aa2f7' -font Arial-Bold -pointsize 72 \
            -gravity center -annotate 0 'SD' \
            packaging/icons/shelldeck.ico
    elif command -v convert &>/dev/null; then
        convert -size 256x256 xc:'#1a1b26' \
            -fill '#7aa2f7' -font Arial-Bold -pointsize 72 \
            -gravity center -annotate 0 'SD' \
            packaging/icons/shelldeck.ico
    else
        # Create a minimal valid 16x16 .ico (1-bit, all blue)
        python3 -c "
import struct, sys
# 16x16 1-bit ICO
w, h = 16, 16
# ICO header
hdr = struct.pack('<HHH', 0, 1, 1)
# Entry: 16x16, 0 colors, 0 reserved, 1 plane, 32 bpp
bmp_size = 40 + (w * h * 4)
entry = struct.pack('<BBBBHHII', w, h, 0, 0, 1, 32, bmp_size, 22)
# BITMAPINFOHEADER
bih = struct.pack('<IiiHHIIiiII', 40, w, h*2, 1, 32, 0, w*h*4, 0, 0, 0, 0)
# Pixel data (BGRA, dark blue #1a1b26)
pixels = b'\\x26\\x1b\\x1a\\xff' * (w * h)
sys.stdout.buffer.write(hdr + entry + bih + pixels)
" > packaging/icons/shelldeck.ico
        echo "  Generated minimal .ico placeholder"
    fi
fi

# Find makensis
MAKENSIS=""
if command -v makensis &>/dev/null; then
    MAKENSIS="makensis"
elif [ -f "/c/Program Files (x86)/NSIS/makensis.exe" ]; then
    MAKENSIS="/c/Program Files (x86)/NSIS/makensis.exe"
elif [ -f "/c/Program Files/NSIS/makensis.exe" ]; then
    MAKENSIS="/c/Program Files/NSIS/makensis.exe"
else
    echo "ERROR: makensis not found. Install NSIS from https://nsis.sourceforge.io/" >&2
    exit 1
fi

echo "==> Using makensis: $MAKENSIS"

"$MAKENSIS" /DVERSION="$VERSION" "packaging/windows/shelldeck.nsi"

INSTALLER="dist/ShellDeck-windows-x86_64-setup.exe"
if [ -f "$INSTALLER" ]; then
    SIZE="$(wc -c < "$INSTALLER" | tr -d ' ')"
    echo ""
    echo "=== NSIS Installer Built ==="
    echo "  Output: $INSTALLER"
    echo "  Size:   $SIZE bytes"
    echo "============================="
else
    echo "ERROR: Installer not found at $INSTALLER" >&2
    exit 1
fi
