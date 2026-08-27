# Patch diff — portable-pty

**Crate**: `portable-pty` v0.8.1 (crates.io)
**Upstream**: https://github.com/wez/wezterm
**Mechanism**: unified diff in `patches/diffs/` + `scripts/apply-crate-patches.sh`
**Not vendored** — no full copy under `patches/portable-pty/`.

## SDPATCH-117 — Preserve an explicit Windows cwd through process creation

- **Diff**: `portable-pty-SDPATCH-117.patch`
- **Files / symbols**: `src/cmdbuilder.rs` — `CommandBuilder::current_directory`,
  `tests::explicit_missing_cwd_is_forwarded_instead_of_falling_back_home`
- **Markers**:
  - `src/cmdbuilder.rs:566` — `// ShellDeck patch: SDPATCH-117 — preserve an explicit cwd even after it`
  - `src/cmdbuilder.rs:761` — `// ShellDeck patch: SDPATCH-117 — compile-check the Windows cwd authority`
- **Why**: Upstream 0.8.1 filters an explicit cwd through `Path::is_dir` immediately
  before calling `CreateProcessW`. If an authorized workspace disappears in the
  validation-to-spawn window, that filter silently substitutes `USERPROFILE` and
  launches outside the authorized workspace. Forwarding the explicit path makes
  Windows reject child creation atomically instead.
- **Upstream status**: not filed yet
- **Regression coverage**: SDTEST-1744 compile-checks the patched Windows-only
  branch in CI; the status remains Yellow until a live Windows spawn/race test
  exercises `CreateProcessW` itself.

## Apply

After `cargo fetch` (or any build that pulls dependencies):

```bash
./scripts/apply-crate-patches.sh
```

The applicator verifies both SDPATCH markers and the security postconditions.
It skips a complete prior application and rejects a partial application.
