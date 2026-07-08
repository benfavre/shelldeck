#!/usr/bin/env bash
# Apply lightweight crate patches from patches/diffs/ into the cargo registry cache.
# Idempotent — safe to run before every build/CI step.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
REGISTRY="${CARGO_HOME:-$HOME/.cargo}/registry/src"

apply_zed_xim() {
    local patch="$ROOT/patches/diffs/zed-xim-SDPATCH-001.patch"
    local dir marker

    if [[ ! -f "$patch" ]]; then
        echo "apply-crate-patches: missing $patch" >&2
        exit 1
    fi

    # Pull zed-xim into the registry if needed (transitive via adabraka-gpui).
    (cd "$ROOT" && cargo fetch -p adabraka-gpui >/dev/null 2>&1) || true

    dir="$(find "$REGISTRY" -maxdepth 2 -type d -name 'zed-xim-0.4.0-zed' 2>/dev/null | head -1)"
    if [[ -z "$dir" ]]; then
        echo "apply-crate-patches: zed-xim-0.4.0-zed not in registry; run cargo fetch first" >&2
        exit 1
    fi

    marker="$dir/src/client.rs"
    if grep -q 'compound_text_to_utf8_or_latin1' "$marker" 2>/dev/null; then
        echo "apply-crate-patches: zed-xim SDPATCH-001 already applied"
        return 0
    fi

    patch -p0 -d "$dir" < "$patch"
    echo "apply-crate-patches: applied zed-xim SDPATCH-001"
}

apply_zed_xim
