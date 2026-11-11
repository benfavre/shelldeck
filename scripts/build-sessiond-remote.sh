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
#   - readelf (binutils), to prove the result is static;
#   - a linker. By default this is rust-lld, the linker that ships inside
#     the Rust toolchain: the daemon is built without the `app` feature, so
#     it links no C code and needs neither musl-gcc nor a cross compiler,
#     for either architecture. Another linker can be named per triple with
#     CARGO_TARGET_<TRIPLE>_LINKER (for example aarch64-linux-musl-gcc).
#
# Each result is checked: ELF for the right machine, no program
# interpreter, no shared library dependency, and the build marker of the
# workspace version (shelldeck_sessiond::BUILD_MARKER), which is what the
# application looks for before it uploads the file. On an x86_64 Linux host
# the x86_64 build is also run once (--version).
#
# scripts/build-release.sh runs this on Linux unless
# SHELLDECK_SESSIOND_REMOTE=0. Without the two files the application
# reports that no program ships for that kind of server and uses its tmux
# or plain SSH path.
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

if ! command -v readelf >/dev/null 2>&1; then
    echo "ERROR: readelf is required to check the static builds (install binutils)" >&2
    exit 1
fi

VERSION="$(grep '^version' Cargo.toml | head -1 | sed 's/.*"\(.*\)".*/\1/')"
if [ -z "$VERSION" ]; then
    echo "ERROR: Could not read version from Cargo.toml" >&2
    exit 1
fi
MARKER="@(#)shelldeck-sessiond $VERSION"

HOST_TRIPLE="$(rustc -vV | sed -n 's/^host: //p')"
RUST_LLD="$(rustc --print sysroot)/lib/rustlib/$HOST_TRIPLE/bin/rust-lld"

# The linker of a triple: the caller's CARGO_TARGET_<TRIPLE>_LINKER when it
# is set, otherwise rust-lld.
linker_variable() {
    printf 'CARGO_TARGET_%s_LINKER' "$(printf '%s' "$1" | tr 'a-z-' 'A-Z_')"
}
for TRIPLE in "${TRIPLES[@]}"; do
    VARIABLE="$(linker_variable "$TRIPLE")"
    if [ -z "${!VARIABLE:-}" ] && [ ! -x "$RUST_LLD" ]; then
        echo "ERROR: rust-lld is not in the Rust toolchain ($RUST_LLD); set $VARIABLE" >&2
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
    VARIABLE="$(linker_variable "$TRIPLE")"
    # Without the `app` feature the daemon does not link shelldeck-core
    # (keyring, D-Bus, TLS), which cannot be linked statically here.
    env "$VARIABLE=${!VARIABLE:-rust-lld}" \
        RUSTFLAGS="${RUSTFLAGS:-} -C target-feature=+crt-static" \
        cargo build --locked --release -p shelldeck-sessiond --no-default-features \
        --target "$TRIPLE"
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
    # A server may have no loader and no libc of ours: nothing may be
    # resolved at run time.
    if readelf -lW "$BUILT" | grep -q 'INTERP' || readelf -dW "$BUILT" | grep -q 'NEEDED'; then
        echo "ERROR: $BUILT is dynamically linked; a static build is required" >&2
        exit 1
    fi
    # The application uploads a file only when it carries this line.
    if ! LC_ALL=C grep -qaF "$MARKER" "$BUILT"; then
        echo "ERROR: $BUILT does not carry the build marker of version $VERSION" >&2
        exit 1
    fi
    if [ "$TRIPLE" = "x86_64-unknown-linux-musl" ] && [ "$HOST_TRIPLE" = "x86_64-unknown-linux-gnu" ]; then
        if [ "$("$BUILT" --version)" != "shelldeck-sessiond $VERSION" ]; then
            echo "ERROR: $BUILT --version did not answer shelldeck-sessiond $VERSION" >&2
            exit 1
        fi
    fi
    OUTPUT="target/release/shelldeck-sessiond-$TRIPLE"
    cp "$BUILT" "$OUTPUT.new"
    chmod 755 "$OUTPUT.new"
    mv -f "$OUTPUT.new" "$OUTPUT"
    echo "==> $OUTPUT ($(wc -c < "$OUTPUT" | tr -d ' ') bytes)"
done
