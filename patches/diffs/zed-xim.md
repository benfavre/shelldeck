# Patch diff — zed-xim

**Crate**: `zed-xim` v0.4.0-zed (crates.io, transitive via `adabraka-gpui`)
**Upstream**: https://github.com/Riey/xim-rs
**Mechanism**: unified diff in `patches/diffs/` + `scripts/apply-crate-patches.sh`
**Not vendored** — no full copy under `patches/zed-xim/`.

## SDPATCH-001 — Latin-1 fallback for XIM compound text decode

- **Diff**: `zed-xim-SDPATCH-001.patch`
- **Files / symbols**: `src/client.rs` — `compound_text_to_utf8_or_latin1`, `handle_request`
- **Why**: On Linux + IBus, accent commits (e.g. `ç` as byte `0xE7`) can arrive as
  unescaped non-UTF-8 compound text. Upstream `.expect("Encoding Error")` panicked GPUI.
  Fallback to ISO-8859-1 instead of crashing.
- **Upstream status**: not filed yet

## Apply

After `cargo fetch` (or any build that pulls deps):

```bash
./scripts/apply-crate-patches.sh
```

Idempotent — skips if already applied. CI runs this before `cargo check`.
