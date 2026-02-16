#!/usr/bin/env bash
#
# Build a ShellDeck .dmg for macOS.
#
# Usage:
#   bash packaging/macos/build-dmg.sh [path/to/shelldeck-binary]
#
# If no binary path is given, uses target/release/shelldeck.
#
# Output: dist/ShellDeck-macos-aarch64.dmg
#
# Requirements: macOS with hdiutil and codesign (Xcode CLT)

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
ARCH="$(uname -m)"
case "$ARCH" in
    arm64|aarch64) PLATFORM="macos-aarch64" ;;
    x86_64)        PLATFORM="macos-x86_64" ;;
    *)             PLATFORM="macos-$ARCH" ;;
esac

echo "==> Building DMG for ShellDeck $VERSION ($PLATFORM)"

# ---------------------------------------------------------------------------
# Create .app bundle
# ---------------------------------------------------------------------------
APP_DIR="$PROJECT_ROOT/dist/ShellDeck.app"
rm -rf "$APP_DIR"
mkdir -p "$APP_DIR/Contents/MacOS"
mkdir -p "$APP_DIR/Contents/Resources"

# Copy binary
cp "$BINARY" "$APP_DIR/Contents/MacOS/shelldeck"
chmod +x "$APP_DIR/Contents/MacOS/shelldeck"

# Create Info.plist
cat > "$APP_DIR/Contents/Info.plist" << PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key>
    <string>ShellDeck</string>
    <key>CFBundleDisplayName</key>
    <string>ShellDeck</string>
    <key>CFBundleIdentifier</key>
    <string>com.shelldeck.app</string>
    <key>CFBundleVersion</key>
    <string>$VERSION</string>
    <key>CFBundleShortVersionString</key>
    <string>$VERSION</string>
    <key>CFBundleExecutable</key>
    <string>shelldeck</string>
    <key>CFBundleIconFile</key>
    <string>shelldeck</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>NSHighResolutionCapable</key>
    <true/>
    <key>LSMinimumSystemVersion</key>
    <string>11.0</string>
    <key>NSSupportsAutomaticGraphicsSwitching</key>
    <true/>
</dict>
</plist>
PLIST

# Copy icon if available, generate placeholder otherwise
ICON_SRC="$PROJECT_ROOT/packaging/icons/shelldeck.icns"
if [ -f "$ICON_SRC" ]; then
    cp "$ICON_SRC" "$APP_DIR/Contents/Resources/shelldeck.icns"
else
    echo "==> No shelldeck.icns found, generating placeholder..."
    # Create a simple PNG and convert to icns using sips (macOS built-in)
    TEMP_PNG="$(mktemp /tmp/shelldeck-icon-XXXX.png)"
    if command -v sips &>/dev/null; then
        # Create a 256x256 PNG placeholder with Python
        python3 -c "
import struct, zlib
w, h = 256, 256
# RGBA: dark blue #1a1b26
row = b''
for x in range(w):
    row += b'\\x1a\\x1b\\x26\\xff'
raw = b''
for y in range(h):
    raw += b'\\x00' + row
compressed = zlib.compress(raw)
def chunk(t, d):
    c = t + d
    return struct.pack('>I', len(d)) + c + struct.pack('>I', zlib.crc32(c) & 0xffffffff)
with open('$TEMP_PNG', 'wb') as f:
    f.write(b'\\x89PNG\\r\\n\\x1a\\n')
    f.write(chunk(b'IHDR', struct.pack('>IIBBBBB', w, h, 8, 6, 0, 0, 0)))
    f.write(chunk(b'IDAT', compressed))
    f.write(chunk(b'IEND', b''))
" 2>/dev/null
        # Convert PNG to iconset to icns
        ICONSET="$(mktemp -d /tmp/shelldeck-XXXX.iconset)"
        sips -z 256 256 "$TEMP_PNG" --out "$ICONSET/icon_256x256.png" &>/dev/null || true
        sips -z 128 128 "$TEMP_PNG" --out "$ICONSET/icon_128x128.png" &>/dev/null || true
        iconutil -c icns "$ICONSET" -o "$APP_DIR/Contents/Resources/shelldeck.icns" 2>/dev/null || {
            echo "  WARNING: iconutil failed, .app will have no icon"
        }
        rm -rf "$ICONSET" "$TEMP_PNG"
    fi
fi

# Ad-hoc codesign (required for macOS Gatekeeper on ARM)
codesign --force --deep --sign - "$APP_DIR" 2>/dev/null || {
    echo "WARNING: codesign failed (expected on Linux CI). Skipping."
}

# ---------------------------------------------------------------------------
# Create DMG
# ---------------------------------------------------------------------------
mkdir -p "$PROJECT_ROOT/dist"
DMG_NAME="ShellDeck-${PLATFORM}.dmg"
DMG_PATH="$PROJECT_ROOT/dist/$DMG_NAME"
DMG_TEMP="$PROJECT_ROOT/dist/dmg-staging"

rm -rf "$DMG_TEMP" "$DMG_PATH"
mkdir -p "$DMG_TEMP"

# Copy .app to staging
cp -R "$APP_DIR" "$DMG_TEMP/"

# Create symlink to /Applications for drag-and-drop install
ln -s /Applications "$DMG_TEMP/Applications"

# Try create-dmg for pretty DMG, fall back to hdiutil
if command -v create-dmg &>/dev/null; then
    create-dmg \
        --volname "ShellDeck" \
        --window-pos 200 120 \
        --window-size 600 400 \
        --icon-size 100 \
        --icon "ShellDeck.app" 150 190 \
        --icon "Applications" 450 190 \
        --hide-extension "ShellDeck.app" \
        --app-drop-link 450 190 \
        --no-internet-enable \
        "$DMG_PATH" \
        "$DMG_TEMP" || {
        echo "==> create-dmg failed, falling back to hdiutil..."
        hdiutil create -volname "ShellDeck" -srcfolder "$DMG_TEMP" \
            -ov -format UDZO "$DMG_PATH"
    }
else
    echo "==> Using hdiutil..."
    hdiutil create -volname "ShellDeck" -srcfolder "$DMG_TEMP" \
        -ov -format UDZO "$DMG_PATH"
fi

# Cleanup staging
rm -rf "$DMG_TEMP" "$APP_DIR"

echo ""
if [ -f "$DMG_PATH" ]; then
    SIZE="$(wc -c < "$DMG_PATH" | tr -d ' ')"
    echo "=== DMG Built ==="
    echo "  Output: dist/$DMG_NAME"
    echo "  Size:   $SIZE bytes"
    echo "=================="
else
    echo "ERROR: DMG not found" >&2
    exit 1
fi
