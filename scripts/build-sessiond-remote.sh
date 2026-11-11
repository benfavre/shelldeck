#!/usr/bin/env bash
#
# Build the static session daemon binaries that the application uploads to
# SSH servers (docs/design/session-daemon.md, section 8).
#
# Usage:
#   bash scripts/build-sessiond-remote.sh [triple...]
#
# Default triples: x86_64-unknown-linux-musl aarch64-unknown-linux-musl
#
# Output, beside the release application binary, where the application
# looks for them and where scripts/package-release.sh picks them up:
#   target/release/shelldeck-sessiond-x86_64-unknown-linux-musl
#   target/release/shelldeck-sessiond-aarch64-unknown-linux-musl
#
# Requirements, checked before anything is built:
#   - the Rust target of each triple (rustup target add <triple>);
#   - a linker for it. x86_64 on an x86_64 host needs musl-gcc (musl-tools);
#     aarch64 needs a cross linker, given as
#     CARGO_TARGET_AARCH64_UNKNOWN_LINUX_MUSL_LINKER (for example
#     aarch64-linux-musl-gcc).
#
# This script is opt-in (SHELLDECK_SESSIOND_REMOTE=1 in build-release.sh)
# and is not part of the default release: without the two files the
# application simply reports that no program ships for that kind of server
# and uses its tmux or plain SSH path.
#
# Nothing is installed, downloaded or published here.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$PROJECT_ROOT"

if [ "$#" -gt 0 ]; then
    TRIPLES=("$@")
else
    TRIPLES=(x86_64-unknown-linux-musl aarch64-unknown-linux-musl)
fi

INSTALLED="$(rustup target list --installed 2>/dev/null || true)"
for TRIPLE in "${TRIPLES[@]}"; do
    case "$TRIPLE" in
        x86_64-unknown-linux-musl|aarch64-unknown-linux-musl) ;;
        *) echo "ERROR: Unsupported session daemon target: $TRIPLE" >&2; exit 1 ;;
    esac
    if ! printf '%s\n' "$INSTALLED" | grep -qx "$TRIPLE"; then
        echo "ERROR: Rust target $TRIPLE is not installed (rustup target add $TRIPLE)" >&2
        exit 1
    fi
done

elf_machine() {
    # e_machine, little endian, at offset 18 of the ELF header.
    od -An -tu2 -j18 -N2 "$1" | tr -d ' '
}

mkdir -p target/release
for TRIPLE in "${TRIPLES[@]}"; do
    echo "==> Building shelldeck-sessiond for $TRIPLE (static)..."
    RUSTFLAGS="${RUSTFLAGS:-} -C target-feature=+crt-static" \
        cargo build --locked --release -p shelldeck-sessiond --target "$TRIPLE"
    BUILT="target/$TRIPLE/release/shelldeck-sessiond"
    if [ ! -x "$BUILT" ]; then
        echo "ERROR: Expected binary not found: $BUILT" >&2
        exit 1
    fi
    case "$TRIPLE" in
        x86_64-*) EXPECTED_MACHINE=62 ;;
        aarch64-*) EXPECTED_MACHINE=183 ;;
    esac
    if [ "$(head -c 4 "$BUILT" | od -An -tx1 | tr -d ' ')" != "7f454c46" ] \
        || [ "$(elf_machine "$BUILT")" != "$EXPECTED_MACHINE" ]; then
        echo "ERROR: $BUILT is not an ELF executable for $TRIPLE" >&2
        exit 1
    fi
    # The upload refuses a binary that still needs a loader on the server.
    if command -v readelf >/dev/null 2>&1 && readelf -l "$BUILT" | grep -q 'INTERP'; then
        echo "ERROR: $BUILT is dynamically linked; a static build is required" >&2
        exit 1
    fi
    OUTPUT="target/release/shelldeck-sessiond-$TRIPLE"
    cp "$BUILT" "$OUTPUT.new"
    chmod 755 "$OUTPUT.new"
    mv -f "$OUTPUT.new" "$OUTPUT"
    echo "==> $OUTPUT ($(wc -c < "$OUTPUT" | tr -d ' ') bytes)"
done
