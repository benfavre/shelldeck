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
    curl -fsSL -o if ! "$LINUXDEPLOY" \
    --appdir "$APPDIR" \
    --deploy-deps-only "$APPDIR/usr/bin/browser/$VERSION" \
    --desktop-file "$APPDIR/usr/share/applications/shelldeck.desktop"; then
    echo "ERROR: Could not bundle ShellDeck and Chromium's shared-library dependencies" >&2
    exit 1
fi

# linuxdeploy normally creates AppRun; older successful versions may omit it.
if [ ! -x "$APPDIR/AppRun" ]; then
    ln -sf usr/bin/shelldeck "$APPDIR/AppRun"
fi

APPIMAGETOOL="$TOOLS_DIR/appimagetool-x86_64.AppImage"
if [ ! -f "$APPIMAGETOOL" ]; then
    curl -fsSL -o "$APPIMAGETOOL" \
        "https://github.com/AppImage/AppImageKit/releases/download/continuous/appimagetool-x86_64.AppImage"
    chmod +x "$APPIMAGETOOL"
fi

SIGN_ARGS=()
if [ "${SHELLDECK_APPIMAGE_SIGN:-0}" = "1" ]; then
    if ! command -v gpg2 >/dev/null 2>&1 && ! command -v gpg >/dev/null 2>&1; then
        echo "ERROR: GPG is required to sign the AppImage" >&2
        exit 1
    fi
    SIGN_ARGS+=(--sign)
fi

rm -f "$PROJECT_ROOT/dist/ShellDeck-x86_64.AppImage"
"$APPIMAGETOOL" --no-appstream "$APPDIR" "${SIGN_ARGS[@]}" \
    "$PROJECT_ROOT/dist/ShellDeck-x86_64.AppImage" 2>&1 || {
    "$APPIMAGETOOL" --appimage-extract-and-run --no-appstream \
        "$APPDIR" "${SIGN_ARGS[@]}" "$PROJECT_ROOT/dist/ShellDeck-x86_64.AppImage"
}

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
