#!/usr/bin/env bash
#
# Build a release archive for the current platform.
#
# Usage:
#   bash scripts/build-release.sh
#
# Output:
#   dist/shelldeck-<platform>.<ext>
#
# Platforms:
#   - Linux:   shelldeck-linux-x86_64.tar.gz
#   - macOS:   shelldeck-macos-aarch64.zip
#   - Windows: shelldeck-windows-x86_64.zip

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$PROJECT_ROOT"

# ---------------------------------------------------------------------------
# Detect platform
# ---------------------------------------------------------------------------
detect_platform() {
    local os arch
    os="$(uname -s)"
    arch="$(uname -m)"

    case "$os" in
        Linux)
            case "$arch" in
                x86_64)  echo "linux-x86_64" ;;
                *)       echo "Unsupported Linux architecture: $arch" >&2; exit 1 ;;
            esac
            ;;
        Darwin)
            case "$arch" in
                arm64|aarch64) echo "macos-aarch64" ;;
                x86_64)        echo "macos-aarch64" ;; # cross-compile target
                *)             echo "Unsupported macOS architecture: $arch" >&2; exit 1 ;;
            esac
            ;;
        MINGW*|MSYS*|CYGWIN*|Windows_NT)
            echo "windows-x86_64"
            ;;
        *)
            echo "Unsupported OS: $os" >&2
            exit 1
            ;;
    esac
}

PLATFORM="$(detect_platform)"
echo "==> Platform: $PLATFORM"

# ---------------------------------------------------------------------------
# Read version from workspace Cargo.toml
# ---------------------------------------------------------------------------
VERSION="$(grep '^version' Cargo.toml | head -1 | sed 's/.*"\(.*\)".*/\1/')"
if [ -z "$VERSION" ]; then
    echo "ERROR: Could not read version from Cargo.toml" >&2
    exit 1
fi
echo "==> Version: $VERSION"

# ---------------------------------------------------------------------------
# Build release binary
# ---------------------------------------------------------------------------
echo "==> Building release binary..."
cargo build --release -p shelldeck

# ---------------------------------------------------------------------------
# Determine binary path
# ---------------------------------------------------------------------------
EXE_EXT=""
if [ "$PLATFORM" = "windows-x86_64" ]; then
    EXE_EXT=".exe"
fi

BINARY="target/release/shelldeck${EXE_EXT}"

if [ ! -f "$BINARY" ]; then
    echo "ERROR: Expected binary not found: $BINARY" >&2
    exit 1
fi

echo "==> Binary: $BINARY ($(wc -c < "$BINARY" | tr -d ' ') bytes)"

# ---------------------------------------------------------------------------
# Package into archive
# ---------------------------------------------------------------------------
mkdir -p dist

case "$PLATFORM" in
    linux-x86_64)
        ARCHIVE_NAME="shelldeck-linux-x86_64.tar.gz"
        ARCHIVE_PATH="dist/$ARCHIVE_NAME"
        echo "==> Packaging $ARCHIVE_PATH..."
        tar -czf "$ARCHIVE_PATH" -C target/release shelldeck
        ;;
    macos-aarch64)
        ARCHIVE_NAME="shelldeck-macos-aarch64.zip"
        ARCHIVE_PATH="dist/$ARCHIVE_NAME"
        echo "==> Packaging $ARCHIVE_PATH..."
        (cd target/release && zip -j "../../$ARCHIVE_PATH" shelldeck)
        ;;
    windows-x86_64)
        ARCHIVE_NAME="shelldeck-windows-x86_64.zip"
        ARCHIVE_PATH="dist/$ARCHIVE_NAME"
        echo "==> Packaging $ARCHIVE_PATH..."
        if command -v zip &>/dev/null; then
            (cd target/release && zip -j "../../$ARCHIVE_PATH" shelldeck.exe)
        else
            powershell -Command "Compress-Archive -Path 'target/release/shelldeck.exe' -DestinationPath '$ARCHIVE_PATH' -Force"
        fi
        ;;
esac

# ---------------------------------------------------------------------------
# Compute SHA256
# ---------------------------------------------------------------------------
if command -v sha256sum &>/dev/null; then
    SHA256="$(sha256sum "$ARCHIVE_PATH" | awk '{print $1}')"
elif command -v shasum &>/dev/null; then
    SHA256="$(shasum -a 256 "$ARCHIVE_PATH" | awk '{print $1}')"
else
    SHA256="(sha256sum not available)"
fi

SIZE="$(wc -c < "$ARCHIVE_PATH" | tr -d ' ')"

# ---------------------------------------------------------------------------
# Summary
# ---------------------------------------------------------------------------
echo ""
echo "=== Build Summary ==="
echo "  Platform:  $PLATFORM"
echo "  Version:   $VERSION"
echo "  Archive:   $ARCHIVE_PATH"
echo "  Size:      $SIZE bytes"
echo "  SHA256:    $SHA256"
echo "====================="
