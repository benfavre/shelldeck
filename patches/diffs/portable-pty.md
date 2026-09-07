# Patch diff — portable-pty

**Crate**: `portable-pty` v0.9.0 (crates.io)
**Upstream**: https://github.com/wez/wezterm
**Mechanism**: unified diff in `patches/diffs/` + `scripts/apply-crate-patches.sh`
**Not vendored** — no full copy under `patches/portable-pty/`.

## SDPATCH-117 — Preserve an explicit cwd through process creation

- **Diff**: `portable-pty-SDPATCH-117.patch`
- **Files / symbols**: `src/cmdbuilder.rs` — `CommandBuilder::current_directory`,
  `CommandBuilder::as_command`,
  `tests::{explicit_missing_unix_cwd_is_forwarded_instead_of_falling_back_home,explicit_missing_cwd_is_forwarded_instead_of_falling_back_home}`
- **Markers**:
  - `src/cmdbuilder.rs:453` — `// ShellDeck patch: SDPATCH-117 — an explicit cwd is an authority`
  - `src/cmdbuilder.rs:564` — `// ShellDeck patch: SDPATCH-117 — preserve an explicit cwd even after it`
  - `src/cmdbuilder.rs:759` — `// ShellDeck patch: SDPATCH-117 — prove the Unix command preserves a missing`
  - `src/cmdbuilder.rs:785` — `// ShellDeck patch: SDPATCH-117 — unit-check the Windows cwd authority`
- **Why**: Upstream 0.9.0 filters an explicit cwd through `Path::is_dir`
  immediately before process creation on both Unix and Windows. If an
  authorized workspace disappears in the validation-to-spawn window, that
  filter silently substitutes `HOME` or `USERPROFILE` and launches outside the
  authorized workspace. Forwarding the explicit path makes native process
  creation reject the vanished directory atomically instead.
- **Upstream status**: not filed yet
- **Regression coverage**: SDTEST-1745 runs the patched Unix command-builder
  unit on Linux and macOS; SDTEST-1744 runs its Windows counterpart. Both
  remain Yellow until live disappearance-race tests exercise native process
  creation itself.

## Apply

After `cargo fetch` (or any build that pulls dependencies):

```bash
./scripts/apply-crate-patches.sh
```

The applicator verifies both SDPATCH markers and the security postconditions.
It skips a complete prior application and rejects a partial application.
