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

# The session daemon ships beside the application binary; a bundle without
# it is not built.
SESSIOND="$(dirname "$BINARY")/shelldeck-sessiond"
if [ ! -x "$SESSIOND" ]; then
    echo "ERROR: Session daemon not found at $SESSIOND" >&2
    echo "Build with: cargo build --release -p shelldeck-sessiond --no-default-features" >&2
    exit 1
fi

# A missing icon is a release error; never ship an unbranded .app.
ICON_SRC="$PROJECT_ROOT/packaging/icons/shelldeck.icns"
if [ ! -s "$ICON_SRC" ]; then
    echo "ERROR: Required macOS icon is missing: $ICON_SRC" >&2
    echo "Regenerate it with python3 scripts/export-inklura-brand.py" >&2
    exit 1
fi

VERSION="$(grep '^version' Cargo.toml | head -1 | sed 's/.*"\(.*\)".*/\1/')"
ARCH="$(uname -m)"
case "$ARCH" in
    arm64|aarch64) PLATFORM="macos-aarch64" ;;
    x86_64)        PLATFORM="macos-x86_64" ;;
    *)             PLATFORM="macos-$ARCH" ;;
esac

echo "==> Building DMG for Inklura Desktop $VERSION ($PLATFORM)"

# ---------------------------------------------------------------------------
# Create .app bundle
# ---------------------------------------------------------------------------
APP_DIR="$PROJECT_ROOT/dist/Inklura Desktop.app"
rm -rf "$APP_DIR"
mkdir -p "$APP_DIR/Contents/MacOS"
mkdir -p "$APP_DIR/Contents/Resources"

# Copy binary
cp "$BINARY" "$APP_DIR/Contents/MacOS/shelldeck"
chmod +x "$APP_DIR/Contents/MacOS/shelldeck"

# The session daemon sits beside the application binary. It outlives the
# application, so the application copies it to its per-user directory and
# starts it from there, never from the bundle
# (crates/shelldeck-ui/src/sessiond_files.rs).
cp "$SESSIOND" "$APP_DIR/Contents/MacOS/shelldeck-sessiond"
chmod 755 "$APP_DIR/Contents/MacOS/shelldeck-sessiond"

# The CEF runtime is a separate app with its own subprocess helpers and
# framework. Preserve framework symlinks and sign it before the outer GPUI app.
BROWSER_APP="$(dirname "$BINARY")/browser/ShellDeck Browser.app"
if [ ! -x "$BROWSER_APP/Contents/MacOS/shelldeck-browser" ] || \
   [ ! -d "$BROWSER_APP/Contents/Frameworks/Chromium Embedded Framework.framework" ]; then
    echo "ERROR: Missing native browser runtime; run scripts/build-release.sh first" >&2
    exit 1
fi
mkdir -p "$APP_DIR/Contents/Frameworks/browser"
ditto "$BROWSER_APP" "$APP_DIR/Contents/Frameworks/browser/ShellDeck Browser.app"

# Create Info.plist
cat > "$APP_DIR/Contents/Info.plist" << PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key>
    <string>Inklura Desktop</string>
    <key>CFBundleDisplayName</key>
    <string>Inklura Desktop</string>
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
    <key>NSHumanReadableCopyright</key>
    <string>Copyright © 2026 Benjamin Favre — Webdesign29</string>
    <key>NSHighResolutionCapable</key>
    <true/>
    <key>LSMinimumSystemVersion</key>
    <string>11.0</string>
    <key>NSSupportsAutomaticGraphicsSwitching</key>
    <true/>
    <key>CFBundleDocumentTypes</key>
    <array>
        <dict>
            <key>CFBundleTypeName</key>
            <string>Inklura Share file</string>
            <key>CFBundleTypeRole</key>
            <string>Viewer</string>
            <key>LSHandlerRank</key>
            <string>Alternate</string>
            <key>LSItemContentTypes</key>
            <array>
                <string>public.image</string>
                <string>public.movie</string>
                <string>public.audio</string>
                <string>com.adobe.pdf</string>
                <string>public.plain-text</string>
                <string>public.comma-separated-values-text</string>
                <string>public.json</string>
                <string>net.daringfireball.markdown</string>
                <string>public.zip-archive</string>
                <string>org.gnu.gnu-zip-archive</string>
                <string>org.7-zip.7-zip-archive</string>
            </array>
        </dict>
    </array>
    <key>CFBundleURLTypes</key>
    <array>
        <dict>
            <key>CFBundleURLName</key>
            <string>com.shelldeck.app</string>
            <key>CFBundleURLSchemes</key>
            <array>
                <string>shelldeck</string>
            </array>
        </dict>
    </array>
</dict>
</plist>
PLIST

# The canonical exporter commits this native icon alongside the other formats.
cp "$ICON_SRC" "$APP_DIR/Contents/Resources/shelldeck.icns"


SIGNING_IDENTITY="${SHELLDECK_MACOS_SIGNING_IDENTITY:--}"
if [ "${SHELLDECK_REQUIRE_SIGNING:-0}" = "1" ] && [ "$SIGNING_IDENTITY" = "-" ]; then
    echo "ERROR: A Developer ID Application identity is required" >&2
    exit 1
fi

SIGN_ARGS=(--force --sign "$SIGNING_IDENTITY")
if [ "$SIGNING_IDENTITY" != "-" ]; then
    SIGN_ARGS+=(--timestamp)
fi
BUNDLED_BROWSER="$APP_DIR/Contents/Frameworks/browser/ShellDeck Browser.app"
CEF_FRAMEWORK="$BUNDLED_BROWSER/Contents/Frameworks/Chromium Embedded Framework.framework"
# Sign actual library files once (not their version symlinks). Process
# entitlements belong to the helper apps, rather than their loaded libraries.
while IFS= read -r -d '' library; do
    if /usr/bin/file -b "$library" | /usr/bin/grep -q 'Mach-O'; then
        codesign "${SIGN_ARGS[@]}" "$library"
        codesign --verify --strict "$library"
    fi
done < <(find "$CEF_FRAMEWORK" -type f -print0)
codesign "${SIGN_ARGS[@]}" "$CEF_FRAMEWORK"
for role in "" " (Alerts)" " (GPU)" " (Plugin)" " (Renderer)"; do
    HELPER="$BUNDLED_BROWSER/Contents/Frameworks/shelldeck-browser Helper${role}.app"
    if [ ! -d "$HELPER" ]; then
        echo "ERROR: Missing CEF subprocess bundle: $HELPER" >&2
        exit 1
    fi
    if [ "$role" = " (Renderer)" ] || [ "$role" = " (GPU)" ]; then
        codesign "${SIGN_ARGS[@]}" --options runtime \
            --entitlements "$SCRIPT_DIR/browser-jit.entitlements.plist" "$HELPER"
    else
        codesign "${SIGN_ARGS[@]}" --options runtime "$HELPER"
    fi
    codesign --verify --strict "$HELPER"
done
codesign "${SIGN_ARGS[@]}" --options runtime "$BUNDLED_BROWSER"
# Signed on its own, with its own identifier: the copy the application makes
# of it runs outside the bundle and must stay valid there.
codesign "${SIGN_ARGS[@]}" --options runtime \
    --identifier com.shelldeck.app.sessiond "$APP_DIR/Contents/MacOS/shelldeck-sessiond"
codesign --verify --strict "$APP_DIR/Contents/MacOS/shelldeck-sessiond"
codesign "${SIGN_ARGS[@]}" --options runtime "$APP_DIR"
codesign --verify --deep --strict --verbose=2 "$APP_DIR"

if [ "${SHELLDECK_MACOS_APP_ONLY:-0}" = "1" ]; then
    echo "==> App bundle ready at dist/Inklura Desktop.app"
    exit 0
fi

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
        --volname "Inklura Desktop" \
        --window-pos 200 120 \
        --window-size 600 400 \
        --icon-size 100 \
        --icon "Inklura Desktop.app" 150 190 \
        --icon "Applications" 450 190 \
        --hide-extension "Inklura Desktop.app" \
        --app-drop-link 450 190 \
        --no-internet-enable \
        "$DMG_PATH" \
        "$DMG_TEMP" || {
        echo "==> create-dmg failed, falling back to hdiutil..."
        hdiutil create -volname "Inklura Desktop" -srcfolder "$DMG_TEMP" \
            -ov -format UDZO "$DMG_PATH"
    }
else
    echo "==> Using hdiutil..."
    hdiutil create -volname "Inklura Desktop" -srcfolder "$DMG_TEMP" \
        -ov -format UDZO "$DMG_PATH"
fi

# Cleanup staging. Keep the app bundle for the signed auto-update ZIP.
rm -rf "$DMG_TEMP"

if [ "$SIGNING_IDENTITY" != "-" ]; then
    codesign --force --timestamp --sign "$SIGNING_IDENTITY" "$DMG_PATH"
    codesign --verify --verbose=2 "$DMG_PATH"
fi

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
