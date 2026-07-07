---
description: Sync a vendored fork under `patches/` to a newer upstream release, preserving all ShellDeck patches. Args → `all`, `<crate>`, or `<crate> <version>`.
argument-hint: [all | <crate> [version]]
---

Read `.agents/patches.md` in full — it is the authoritative workflow for
this command (marker convention, `PATCHES.md` schema, per-fork sync
steps, non-negotiables, bootstrap mode).

Then execute that workflow using the arguments below.

## Arguments

**$ARGUMENTS**

Interpret them as documented in `.agents/patches.md`:

- Empty → interactive: list forks with a `PATCHES.md`, print pinned
  versions, ask what to sync.
- `all` → sync every fork sequentially, stop on first red patch.
- `<crate>` → sync that crate to the latest release available.
- `<crate> <version>` → sync that crate to that exact tag.

Do not restate the workflow steps in your response — follow the ones in
`.agents/patches.md`. If any pre-flight check fails, bail out with a
short report; don't try to "help" past the safety guards.
