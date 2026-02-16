#!/usr/bin/env bash
#
# Build a ShellDeck AppImage for Linux x86_64.
#
# Usage:
#   bash packaging/linux/build-appimage.sh [path/to/shelldeck-binary]
#
# If no binary path is given, uses target/release/shelldeck.
#
# Output: dist/ShellDeck-x86_64.AppImage
#
# Requirements: builds on Ubuntu, needs FUSE for final AppImage (or --appimage-extract-and-run)

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
cd "$PROJECT_ROOT"

BINARY="${1:-target/release/shelldeck}"
if [ ! -f "$BINARY" ]; then
    echo "ERROR: Binary not found at $BINARY" >&2
    echo "Build with: cargo build --release -p shelldeck" >&2
    exit 1
fi

VERSION="$(grep '^version' Cargo.toml | head -1 | sed 's/.*"\(.*\)".*/\1/')"
echo "==> Building AppImage for ShellDeck $VERSION"

# ---------------------------------------------------------------------------
# Download linuxdeploy if not present
# ---------------------------------------------------------------------------
TOOLS_DIR="$PROJECT_ROOT/.build-tools"
mkdir -p "$TOOLS_DIR"

LINUXDEPLOY="$TOOLS_DIR/linuxdeploy-x86_64.AppImage"
if [ ! -f "$LINUXDEPLOY" ]; then
    echo "==> Downloading linuxdeploy..."
    curl -fsSL -o "$LINUXDEPLOY" \
        "https://github.com/linuxdeploy/linuxdeploy/releases/download/continuous/linuxdeploy-x86_64.AppImage"
    chmod +x "$LINUXDEPLOY"
fi

# ---------------------------------------------------------------------------
# Create AppDir structure
# ---------------------------------------------------------------------------
APPDIR="$PROJECT_ROOT/dist/ShellDeck.AppDir"
rm -rf "$APPDIR"
mkdir -p "$APPDIR/usr/bin"
mkdir -p "$APPDIR/usr/share/applications"
mkdir -p "$APPDIR/usr/share/icons/hicolor/256x256/apps"

# Copy binary
cp "$BINARY" "$APPDIR/usr/bin/shelldeck"
chmod +x "$APPDIR/usr/bin/shelldeck"

# Copy .desktop file
cp "$SCRIPT_DIR/shelldeck.desktop" "$APPDIR/usr/share/applications/shelldeck.desktop"

# Copy icon (use project icon or generate a placeholder)
ICON_SRC="$PROJECT_ROOT/packaging/icons/shelldeck-256.png"
if [ -f "$ICON_SRC" ]; then
    cp "$ICON_SRC" "$APPDIR/usr/share/icons/hicolor/256x256/apps/shelldeck.png"
else
    echo "WARNING: No icon at $ICON_SRC, generating placeholder..."
    # Create a simple 256x256 placeholder with ImageMagick if available
    if command -v convert &>/dev/null; then
        convert -size 256x256 xc:'#1a1b26' \
            -fill '#7aa2f7' -font Helvetica-Bold -pointsize 72 \
            -gravity center -annotate 0 'SD' \
            "$APPDIR/usr/share/icons/hicolor/256x256/apps/shelldeck.png"
    elif command -v python3 &>/dev/null; then
        # Generate a minimal 256x256 PNG with Python (no dependencies)
        python3 -c "
import struct, zlib
w, h = 256, 256
# RGBA: dark blue #1a1b26
row = b'\\x00' + (b'\\x1a\\x1b\\x26\\xff' * w)
raw = row * h
compressed = zlib.compress(raw)
def chunk(t, d):
    c = t + d
    return struct.pack('>I', len(d)) + c + struct.pack('>I', zlib.crc32(c) & 0xffffffff)
with open('$APPDIR/usr/share/icons/hicolor/256x256/apps/shelldeck.png', 'wb') as f:
    f.write(b'\\x89PNG\\r\\n\\x1a\\n')
    f.write(chunk(b'IHDR', struct.pack('>IIBBBBB', w, h, 8, 6, 0, 0, 0)))
    f.write(chunk(b'IDAT', compressed))
    f.write(chunk(b'IEND', b''))
"
        echo "  Generated placeholder PNG with Python"
    else
        echo "WARNING: No icon, no ImageMagick, no Python. AppImage will have no icon."
    fi
fi

# ---------------------------------------------------------------------------
# Build AppImage
# ---------------------------------------------------------------------------
mkdir -p "$PROJECT_ROOT/dist"

export ARCH=x86_64
export VERSION="$VERSION"

# linuxdeploy bundles shared libraries and creates the AppImage
"$LINUXDEPLOY" \
    --appdir "$APPDIR" \
    --desktop-file "$APPDIR/usr/share/applications/shelldeck.desktop" \
    --output appimage 2>&1 || {
    # Fallback: manual AppImage creation if linuxdeploy fails (e.g., no FUSE)
    echo "==> linuxdeploy failed, trying manual AppImage creation..."

    # Create AppRun
    cat > "$APPDIR/AppRun" << 'APPRUN'
#!/bin/bash
SELF=$(readlink -f "$0")
HERE=${SELF%/*}
export PATH="${HERE}/usr/bin:${PATH}"
export LD_LIBRARY_PATH="${HERE}/usr/lib:${LD_LIBRARY_PATH:-}"
exec "${HERE}/usr/bin/shelldeck" "$@"
APPRUN
    chmod +x "$APPDIR/AppRun"

    # Symlink icon and desktop to AppDir root
    ln -sf usr/share/icons/hicolor/256x256/apps/shelldeck.png "$APPDIR/shelldeck.png" 2>/dev/null || true
    ln -sf usr/share/applications/shelldeck.desktop "$APPDIR/shelldeck.desktop"

    # Use appimagetool if available
    APPIMAGETOOL="$TOOLS_DIR/appimagetool-x86_64.AppImage"
    if [ ! -f "$APPIMAGETOOL" ]; then
        curl -fsSL -o "$APPIMAGETOOL" \
            "https://github.com/AppImage/AppImageKit/releases/download/continuous/appimagetool-x86_64.AppImage"
        chmod +x "$APPIMAGETOOL"
    fi

    "$APPIMAGETOOL" --no-appstream "$APPDIR" "$PROJECT_ROOT/dist/ShellDeck-x86_64.AppImage" 2>&1 || {
        # Last resort: try with --appimage-extract-and-run (no FUSE needed)
        "$APPIMAGETOOL" --appimage-extract-and-run --no-appstream "$APPDIR" \
            "$PROJECT_ROOT/dist/ShellDeck-x86_64.AppImage"
    }
}

# Move output if linuxdeploy created it in current directory
if [ -f "$PROJECT_ROOT/ShellDeck-${VERSION}-x86_64.AppImage" ]; then
    mv "$PROJECT_ROOT/ShellDeck-${VERSION}-x86_64.AppImage" "$PROJECT_ROOT/dist/ShellDeck-x86_64.AppImage"
elif [ -f "ShellDeck-${VERSION}-x86_64.AppImage" ]; then
    mv "ShellDeck-${VERSION}-x86_64.AppImage" "$PROJECT_ROOT/dist/ShellDeck-x86_64.AppImage"
fi

echo ""
if [ -f "$PROJECT_ROOT/dist/ShellDeck-x86_64.AppImage" ]; then
    SIZE="$(wc -c < "$PROJECT_ROOT/dist/ShellDeck-x86_64.AppImage" | tr -d ' ')"
    echo "=== AppImage Built ==="
    echo "  Output: dist/ShellDeck-x86_64.AppImage"
    echo "  Size:   $SIZE bytes"
    echo "======================"
else
    echo "ERROR: AppImage not found at expected location" >&2
    ls -la "$PROJECT_ROOT/dist/"
    exit 1
fi
