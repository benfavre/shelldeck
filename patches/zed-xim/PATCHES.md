# Patches — zed-xim

**Vendored from**: `zed-xim` v0.4.0-zed (crates.io)
**Upstream**: https://github.com/Riey/xim-rs
**Last synced**: 2026-07-08

## Patches

### SDPATCH-001 — Latin-1 fallback for XIM compound text decode

- **Files / symbols**:
  - `src/client.rs` — `compound_text_to_utf8_or_latin1`, `handle_request`
- **Markers**:
  - `src/client.rs:52` — `// ShellDeck patch: IBus/XIM sometimes sends raw Latin-1 bytes`
- **Why**: On Linux + IBus, accent commits (e.g. `ç` as byte `0xE7`) can arrive as
  unescaped non-UTF-8 compound text. Upstream called `.expect("Encoding Error")` on
  three decode sites (commit, preedit draw, reset IC), crashing GPUI. We log a warning
  and decode bytes as ISO-8859-1 instead of panicking.
- **Upstream status**: not filed yet

## Sync log

- **2026-07-08** — initial vendor from crates.io v0.4.0-zed + SDPATCH-001.
