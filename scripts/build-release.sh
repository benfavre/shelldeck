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
#
# Linux also builds the session daemon (shelldeck-sessiond), which ships
# beside the application binary, and its two static builds for SSH servers
# (scripts/build-sessiond-remote.sh, which needs the musl Rust targets).
# SHELLDECK_SESSIOND_REMOTE=0 leaves the static builds out: the application
# then has no program to install on a server and keeps tmux or plain SSH.

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
echo "==> Applying crate patches..."
bash "$SCRIPT_DIR/apply-crate-patches.sh"

echo "==> Building release binary..."
if [ "$PLATFORM" = "linux-x86_64" ]; then
    export PKG_CONFIG_PATH="/usr/lib/x86_64-linux-gnu/pkgconfig${PKG_CONFIG_PATH:+:$PKG_CONFIG_PATH}"
fi
cargo build --locked --release -p shelldeck
# Keep these separate: the main GPUI process must never link Chromium. Only
# the dedicated helper enables CEF, and provider stdio mode uses that helper.
cargo build --locked --release -p shelldeck-browser --features runtime
python3 scripts/bundle-browser-runtime.py --profile release

# The session daemon is a separate small process that owns the terminals of
# `general.session_daemon`. The application looks for it beside its own
# executable, so it is built with the same profile and packaged with it.
# Unix only: the Windows archive does not carry it (the daemon is Unix
# code, and the application then uses in-process terminals).
if [ "$PLATFORM" != "windows-x86_64" ]; then
    echo "==> Building the session daemon..."
    # The binary needs none of the application side of its crate (the
    # `app` feature, which links shelldeck-core).
    cargo build --locked --release -p shelldeck-sessiond --no-default-features
    if [ ! -x target/release/shelldeck-sessiond ]; then
        echo "ERROR: Expected binary not found: target/release/shelldeck-sessiond" >&2
        exit 1
    fi
fi
# Static builds for SSH servers: Linux only for now, so only the Linux
# packages carry them.
if [ "$PLATFORM" = "linux-x86_64" ]; then
    if [ "${SHELLDECK_SESSIOND_REMOTE:-1}" = "0" ]; then
        echo "==> SHELLDECK_SESSIOND_REMOTE=0: no session daemon builds for SSH servers"
        # A build left by an earlier run would be packaged under this
        # release although it was not built from it.
        rm -f target/release/shelldeck-sessiond-x86_64-unknown-linux-musl \
            target/release/shelldeck-sessiond-aarch64-unknown-linux-musl
    else
        bash "$SCRIPT_DIR/build-sessiond-remote.sh"
    fi
fi

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

if [ "${SHELLDECK_DEFER_ARCHIVE:-0}" = "1" ]; then
    echo "==> Archive deferred until after signing"
    exit 0
fi

if [ "$PLATFORM" = "macos-aarch64" ]; then
    SHELLDECK_MACOS_APP_ONLY=1 bash packaging/macos/build-dmg.sh "$BINARY"
fi
SHELLDECK_RELEASE_PLATFORM="$PLATFORM" bash scripts/package-release.sh

case "$PLATFORM" in
    linux-x86_64) ARCHIVE_PATH="dist/shelldeck-linux-x86_64.tar.gz" ;;
    macos-aarch64) ARCHIVE_PATH="dist/shelldeck-macos-aarch64.zip" ;;
    windows-x86_64) ARCHIVE_PATH="dist/shelldeck-windows-x86_64.zip" ;;
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
