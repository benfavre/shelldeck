---
name: sync-patches
description: Synchronize one or all vendored Rust forks under patches/ to a newer upstream version while preserving ShellDeck patch markers and PATCHES.md inventories. Use when the user invokes $sync-patches or asks to update adabraka-gpui, adabraka-ui, or another vendored dependency safely.
---

# Synchronize ShellDeck vendored patches

Read `.agents/patches.md` completely before acting. It is the authoritative
workflow for patch selection, safety branches, upstream overlays, replay
commits, marker audits, compilation, regression tests, and reporting.

## Parse the request

Treat words supplied with `$sync-patches` as command arguments:

- No argument: list every fork containing `PATCHES.md`, show its pinned
  version, and ask which fork to synchronize.
- `all`: synchronize every fork sequentially and stop on the first red patch.
- `<crate>`: synchronize that crate to the latest available upstream release.
- `<crate> <version>`: synchronize that crate to the exact requested tag.

## Guardrails

- Run the pre-flight and marker/inventory parity checks before modifying a
  fork.
- Create and preserve the required `sync-patches/<crate>-<from>-to-<to>` safety
  branch.
- Replay one `SDPATCH` per commit in ascending order.
- Never continue past unresolved red patches or invent semantic adaptations.
- Never touch `patches/adabraka-gpui/src/elements/div.rs` unless the user names
  it explicitly.
- Run the Linux `PKG_CONFIG_PATH` compilation check and the regression sweep
  required by `.agents/patches.md`.
- Report the pinned version, per-patch status, retirement candidates, and the
  safety branch.

Follow `.agents/patches.md` rather than restating its workflow to the user.
