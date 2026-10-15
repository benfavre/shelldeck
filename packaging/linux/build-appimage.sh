#!/usr/bin/env bash
# Build the production binary and its matching browser payload into an AppImage.
# Usage: bash packaging/linux/build-appimage.sh [path/to/shelldeck-binary]
# Output: dist/ShellDeck-x86_64.AppImage
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
cd "$PROJECT_ROOT"

BINARY="${1:-target/release/shelldeck}"
if [ ! -f "$BINARY" ]; then
    echo "ERROR: Binary not found at $BINARY; run scripts/build-release.sh first" >&2
    exit 1
fi
VERSION="$(sed -n '/^\[workspace.package\]/,/^\[/p' Cargo.toml | sed -n 's/^version = "\([^"]*\)"/\1/p' | head -1)"
if [ -z "$VERSION" ]; then
    echo "ERROR: Missing workspace version" >&2
    exit 1
fi
BINARY_DIR="$(cd "$(dirname "$BINARY")" && pwd)"
BROWSER_SOURCE="$BINARY_DIR/browser/$VERSION"
if [ ! -x "$BROWSER_SOURCE/shelldeck-browser" ]; then
    echo "ERROR: Missing native browser helper; run scripts/build-release.sh first" >&2
    exit 1
fi
for RESOURCE in libcef.so icudtl.dat resources.pak chrome_100_percent.pak chrome_200_percent.pak v8_context_snapshot.bin locales/en-US.pak runtime.json; do
    if [ ! -f "$BROWSER_SOURCE/$RESOURCE" ]; then
        echo "ERROR: Missing browser resource: $BROWSER_SOURCE/$RESOURCE" >&2
        exit 1
    fi
done
ICON_SOURCE="$PROJECT_ROOT/packaging/icons/shelldeck-256.png"
if [ ! -f "$ICON_SOURCE" ]; then
    echo "ERROR: Missing committed application icon: $ICON_SOURCE" >&2
    exit 1
fi

echo "==> Building AppImage for ShellDeck $VERSION"
TOOLS_DIR="$PROJECT_ROOT/.build-tools"
mkdir -p "$TOOLS_DIR"
download_tool() {
    local destination="$1" repository="$2" asset="$3"
    if [ ! -x "$destination" ]; then
        if ! gh release download continuous --repo "$repository" --pattern "$asset" \
            --output "$destination.download" --clobber; then
            rm -f "$destination.download"
            return 1
        fi
        chmod +x "$destination.download"
        mv -f "$destination.download" "$destination"
    fi
}
LINUXDEPLOY="$TOOLS_DIR/linuxdeploy-x86_64.AppImage"
download_tool "$LINUXDEPLOY" \
    linuxdeploy/linuxdeploy linuxdeploy-x86_64.AppImage

# Keep the exact versioned helper/resources and symlinks beside the main binary.
# The same shelldeck executable handles normal, --agents and protocol launches.
APPDIR="$PROJECT_ROOT/dist/ShellDeck.AppDir"
rm -rf "$APPDIR"
mkdir -p "$APPDIR/usr/bin/browser" "$APPDIR/usr/share/applications" \
    "$APPDIR/usr/share/icons/hicolor/256x256/apps"
cp "$BINARY" "$APPDIR/usr/bin/shelldeck"
chmod +x "$APPDIR/usr/bin/shelldeck"
cp -a "$BROWSER_SOURCE" "$APPDIR/usr/bin/browser/$VERSION"
# Staging uses a private 0700 temporary directory. SquashFS normalizes ownership
# to root, so published runtime directories/data must remain readable/executable
# by the actual desktop user after mounting the AppImage.
chmod -R a+rX "$APPDIR/usr/bin/browser/$VERSION"
cp "$SCRIPT_DIR/shelldeck.desktop" "$APPDIR/usr/share/applications/shelldeck.desktop"
cp "$ICON_SOURCE" "$APPDIR/usr/share/icons/hicolor/256x256/apps/shelldeck.png"

export ARCH=x86_64
export VERSION
# These files are already in their final relative locations. Deploy dependency
# libraries and fix rpaths without flattening the versioned CEF tree.
# Seed only the bundler's dependency lookup. libcef remains in its canonical
# version directory and is scanned directly for its own dependencies; excluding
# it as a copied dependency prevents a second multi-gigabyte copy in usr/lib.
BROWSER_HELPER="$APPDIR/usr/bin/browser/$VERSION/shelldeck-browser"
# NSS loads these modules with dlopen, so an ELF dependency scan alone misses
# them. Resolve them beside the exact host libnss3 used by the matching CEF;
# Debian places the modules in nss/, while other distributions use the same
# library directory. Deploy them beside the bundled libnss3, with dependencies.
if ! NSS_TRACE="$(LD_LIBRARY_PATH="$BROWSER_SOURCE${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}" \
    LC_ALL=C ldd "$BROWSER_SOURCE/libcef.so" 2>&1)"; then
    echo "ERROR: Could not locate Chromium's matching NSS distribution" >&2
    exit 1
fi
NSS_LIBRARY="$(sed -n 's/^[[:space:]]*libnss3\.so => \(\/.*\) (0x[[:xdigit:]]*)$/\1/p' <<< "$NSS_TRACE")"
if [ ! -f "$NSS_LIBRARY" ]; then
    echo "ERROR: Could not locate Chromium's matching libnss3.so" >&2
    exit 1
fi
NSS_DIRECTORY="$(dirname "$(readlink -f "$NSS_LIBRARY")")"
NSS_ARGUMENTS=()
NSS_MODULES=(libsoftokn3.so libfreebl3.so libfreeblpriv3.so libnssckbi.so libnssdbm3.so)
for MODULE in "${NSS_MODULES[@]}"; do
    MODULE_SOURCE=""
    for CANDIDATE in "$NSS_DIRECTORY/nss/$MODULE" "$NSS_DIRECTORY/$MODULE"; do
        if [ -f "$CANDIDATE" ]; then
            MODULE_SOURCE="$CANDIDATE"
            break
        fi
    done
    if [ -z "$MODULE_SOURCE" ]; then
        echo "ERROR: Missing Chromium NSS runtime module: $MODULE" >&2
        exit 1
    fi
    NSS_ARGUMENTS+=(--library "$MODULE_SOURCE")
done
if ! LD_LIBRARY_PATH="$APPDIR/usr/bin/browser/$VERSION${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}" \
    "$LINUXDEPLOY" --appdir "$APPDIR" --exclude-library libcef.so \
    "${NSS_ARGUMENTS[@]}" \
    --deploy-deps-only "$APPDIR/usr/bin/shelldeck" \
    --deploy-deps-only "$BROWSER_HELPER" \
    --deploy-deps-only "$APPDIR/usr/bin/browser/$VERSION" \
    --desktop-file "$APPDIR/usr/share/applications/shelldeck.desktop"; then
    echo "ERROR: Could not bundle ShellDeck and Chromium's shared-library dependencies" >&2
    exit 1
fi

# Directory scans can warn and exit successfully after skipping an unresolved
# ELF file. Independently require the main/helper loaders to resolve without the
# bundler's LD_LIBRARY_PATH, and require the matching versioned libcef itself.
verify_loader() {
    local executable="$1" expected_cef="${2:-}" trace
    if ! trace="$(env -u LD_LIBRARY_PATH LC_ALL=C ldd "$executable" 2>&1)"; then
        echo "ERROR: Could not inspect packaged loader dependencies: $executable" >&2
        echo "$trace" >&2
        return 1
    fi
    if printf '%s\n' "$trace" | grep -q 'not found'; then
        echo "ERROR: Unresolved packaged loader dependency: $executable" >&2
        echo "$trace" >&2
        return 1
    fi
    if [ -n "$expected_cef" ] && ! printf '%s\n' "$trace" | grep -F -q "libcef.so => $expected_cef ("; then
        echo "ERROR: Browser helper did not resolve its matching versioned libcef.so" >&2
        echo "$trace" >&2
        return 1
    fi
}
verify_loader "$APPDIR/usr/bin/shelldeck"
verify_loader "$BROWSER_HELPER" "$APPDIR/usr/bin/browser/$VERSION/libcef.so"
for MODULE in "${NSS_MODULES[@]}"; do
    if [ ! -f "$APPDIR/usr/lib/$MODULE" ]; then
        echo "ERROR: NSS runtime module was not deployed: $MODULE" >&2
        exit 1
    fi
    verify_loader "$APPDIR/usr/lib/$MODULE"
done
if [ -e "$APPDIR/usr/lib/libcef.so" ]; then
    echo "ERROR: Browser engine was duplicated outside its versioned runtime" >&2
    exit 1
fi

# Successful older linuxdeploy versions may omit the root entry point. Preserve
# arguments exactly, including --agents, while exposing bundled dependencies.
if [ ! -x "$APPDIR/AppRun" ]; then
    cat > "$APPDIR/AppRun" <<'APPRUN'
#!/bin/sh
SELF="$(readlink -f "$0")"
HERE="${SELF%/*}"
export PATH="${HERE}/usr/bin:${PATH}"
export LD_LIBRARY_PATH="${HERE}/usr/lib:${LD_LIBRARY_PATH:-}"
exec "${HERE}/usr/bin/shelldeck" "$@"
APPRUN
    chmod +x "$APPDIR/AppRun"
fi
ln -sf usr/share/applications/shelldeck.desktop "$APPDIR/shelldeck.desktop"
ln -sf usr/share/icons/hicolor/256x256/apps/shelldeck.png "$APPDIR/shelldeck.png"

# Release CI opts into the real packaged CEF workflow before compression. Its
# disposable profile and loopback fixture catch dlopen failures that ldd cannot.
if [ "${SHELLDECK_APPIMAGE_VERIFY_RUNTIME:-0}" = "1" ]; then
    xvfb-run -a python3 "$PROJECT_ROOT/scripts/test-browser-runtime.py" "$BROWSER_HELPER"
fi

APPIMAGETOOL="$TOOLS_DIR/appimagetool-x86_64.AppImage"
download_tool "$APPIMAGETOOL" \
    AppImage/AppImageKit appimagetool-x86_64.AppImage
SIGN_ARGS=()
if [ "${SHELLDECK_APPIMAGE_SIGN:-0}" = "1" ]; then
    if ! command -v gpg2 >/dev/null 2>&1 && ! command -v gpg >/dev/null 2>&1; then
        echo "ERROR: GPG is required to sign the AppImage" >&2
        exit 1
    fi
    SIGN_ARGS+=(--sign)
fi

OUTPUT="$PROJECT_ROOT/dist/ShellDeck-x86_64.AppImage"
rm -f "$OUTPUT"
"$APPIMAGETOOL" --no-appstream "$APPDIR" "${SIGN_ARGS[@]}" "$OUTPUT" 2>&1 || {
    "$APPIMAGETOOL" --appimage-extract-and-run --no-appstream \
        "$APPDIR" "${SIGN_ARGS[@]}" "$OUTPUT"
}
if [ ! -f "$OUTPUT" ]; then
    echo "ERROR: AppImage was not produced at $OUTPUT" >&2
    exit 1
fi
echo "==> AppImage built: $OUTPUT ($(wc -c < "$OUTPUT" | tr -d ' ') bytes)"
