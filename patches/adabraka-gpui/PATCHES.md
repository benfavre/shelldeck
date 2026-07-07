# Patches — adabraka-gpui

**Vendored from**: `adabraka-gpui` v0.3.x (see `Cargo.toml` for the
exact upstream version — this fork pre-dates our patch tracking, so
the vendor-time version isn't recorded here).
**Upstream**: https://github.com/Augani/adabraka-gpui *(the repo listed
on crates.io was inaccessible when checked — the sync workflow will
need to re-confirm the real upstream location on first use)*
**Last synced**: 2026-07-07 (inventory bootstrap — the fork itself
predates this file)

Total markers in code: **3**

## Patches

### SDPATCH-101 — `PathPromptOptions::starting_directory`

- **Files / symbols**:
  - `src/platform.rs` — `PathPromptOptions` struct
- **Markers**:
  - `src/platform.rs:1341` — `/// ShellDeck patch: initial directory the OS picker should open in`
- **Why**: The upstream `PathPromptOptions` has no way to hint a starting
  folder. ShellDeck's Identity File picker wants to open straight in
  `~/.ssh/`. We added an optional `starting_directory: Option<PathBuf>`
  and `#[derive(Default)]` on the struct so existing call sites can build
  it with `..Default::default()` and omit the new field.
- **Upstream status**: not filed yet — small addition, easy PR.

### SDPATCH-102 — Linux portal wire-up for `starting_directory`

- **Files / symbols**:
  - `src/platform/linux/platform.rs` — `LinuxCommon::prompt_for_paths`
    (the XDG portal branch)
- **Markers**:
  - `src/platform/linux/platform.rs:300` — `// ShellDeck patch: capture two identifier futures so the picker can`
  - `src/platform/linux/platform.rs:318` — `// ShellDeck patch: pre-seed the picker's starting folder`
- **Why**: Threads SDPATCH-101 into
  `ashpd::desktop::file_chooser::OpenFileRequest::current_folder()`.
  `OpenFileRequest` doesn't `Clone` and `current_folder` consumes it on
  error, so we capture a second `window_identifier()` future up front
  (marker at line 300) and use it to rebuild the request without the
  folder hint if `current_folder` rejects the path (marker at line 318).
  Two markers because the fix legitimately spans two non-adjacent
  locations in the same function.
- **Upstream status**: pairs with SDPATCH-101 in the same PR.

## Preserved files (do not overwrite on sync)

- `PATCHES.md` (this file)
- `src/elements/div.rs` — hosts an in-progress smooth-scroll animation
  patch owned by Benjamin Favre. **NOT** part of our replayable patch
  set and not tracked here; the `/sync-patches` workflow must leave it
  alone (see the "Non-negotiables" section of `.agents/patches.md`).
  If a sync introduces upstream changes to `div.rs`, stop and report —
  do not merge them silently.

## Sync log

- **2026-07-07** — patch inventory bootstrapped after the fact. Marker
  count 3 = 1 (SDPATCH-101) + 2 (SDPATCH-102). The fork itself predates
  this file; any earlier tweaks made at genesis time that aren't in
  `SDPATCH-*` form live in `src/elements/div.rs` and are documented in
  the `Preserved files` list above.

## Retired patches

*(empty for now)*
