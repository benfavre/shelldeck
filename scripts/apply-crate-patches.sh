#!/usr/bin/env bash
# Apply lightweight crate patches from patches/diffs/ into the cargo registry cache.
# Idempotent — safe to run before every build/CI step.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
REGISTRY="${CARGO_HOME:-$HOME/.cargo}/registry/src"

# `cargo fetch` has no package selector. Fetch once so every exact-version
# lookup below operates on the dependency graph pinned by Cargo.lock.
(cd "$ROOT" && cargo fetch >/dev/null)

apply_zed_xim() {
    local patch="$ROOT/patches/diffs/zed-xim-SDPATCH-001.patch"
    local dir marker

    if [[ ! -f "$patch" ]]; then
        echo "apply-crate-patches: missing $patch" >&2
        exit 1
    fi

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

portable_pty_patch_is_complete() {
    local source="$1"
    local marker_count

    marker_count="$(grep -F -c 'ShellDeck patch: SDPATCH-117' "$source" || true)"
    [[ "$marker_count" == "4" ]] \
        && grep -F -q 'let dir: &OsStr = self.cwd.as_deref().unwrap_or(home.as_ref());' "$source" \
        && grep -F -q 'let dir: Option<&OsStr> = self.cwd.as_deref().or(home);' "$source" \
        && ! grep -F -q 'let cwd: Option<&OsStr> = self.cwd.as_deref().filter' "$source" \
        && grep -F -q '// SDTEST-1745' "$source" \
        && grep -F -q '// SDTEST-1744' "$source"
}

portable_pty_patch_is_legacy_complete() {
    local source="$1"
    local marker_count

    marker_count="$(grep -F -c 'ShellDeck patch: SDPATCH-117' "$source" || true)"
    [[ "$marker_count" == "2" ]] \
        && grep -F -q 'let dir: Option<&OsStr> = self.cwd.as_deref().or(home);' "$source" \
        && grep -F -q '.filter(|dir| std::path::Path::new(dir).is_dir())' "$source" \
        && grep -F -q '// SDTEST-1744' "$source" \
        && ! grep -F -q '// SDTEST-1745' "$source"
}

apply_portable_pty() {
    local patch="$ROOT/patches/diffs/portable-pty-SDPATCH-117.patch"
    local cache_upgrade="$ROOT/patches/diffs/portable-pty-SDPATCH-117-cache-upgrade.patch"
    local dir source

    if [[ ! -f "$patch" || ! -f "$cache_upgrade" ]]; then
        echo "apply-crate-patches: missing portable-pty patch input" >&2
        exit 1
    fi

    dir="$(find "$REGISTRY" -maxdepth 2 -type d -name 'portable-pty-0.8.1' 2>/dev/null | head -1)"
    if [[ -z "$dir" ]]; then
        echo "apply-crate-patches: portable-pty-0.8.1 not in registry after cargo fetch" >&2
        exit 1
    fi

    source="$dir/src/cmdbuilder.rs"
    if grep -F -q 'ShellDeck patch: SDPATCH-117' "$source" 2>/dev/null; then
        if portable_pty_patch_is_complete "$source"; then
            echo "apply-crate-patches: portable-pty SDPATCH-117 already applied"
            return 0
        fi
        if portable_pty_patch_is_legacy_complete "$source"; then
            # BSD patch (the macOS runner) handles zero-context hunks
            # differently from GNU patch. Git's applicator is available on
            # every supported runner and gives this replay one exact behavior.
            (cd "$dir" && git apply -p0 --unidiff-zero --ignore-space-change --check "$cache_upgrade")
            (cd "$dir" && git apply -p0 --unidiff-zero --ignore-space-change "$cache_upgrade")
            if ! portable_pty_patch_is_complete "$source"; then
                echo "apply-crate-patches: portable-pty SDPATCH-117 cache upgrade failed" >&2
                exit 1
            fi
            echo "apply-crate-patches: upgraded cached portable-pty SDPATCH-117"
            return 0
        fi
        echo "apply-crate-patches: portable-pty SDPATCH-117 is only partially applied" >&2
        exit 1
    fi

    # Windows checkout may convert the repository-owned patch file to CRLF
    # while Cargo extracts the registry source with LF. Ignore that sole
    # whitespace difference; every semantic hunk and the marker audit below
    # must still match exactly.
    (cd "$dir" && git apply -p0 --unidiff-zero --ignore-space-change --check "$patch")
    (cd "$dir" && git apply -p0 --unidiff-zero --ignore-space-change "$patch")
    if ! portable_pty_patch_is_complete "$source"; then
        echo "apply-crate-patches: portable-pty SDPATCH-117 failed its post-apply audit" >&2
        exit 1
    fi
    echo "apply-crate-patches: applied portable-pty SDPATCH-117"
}

apply_zed_xim
apply_portable_pty
