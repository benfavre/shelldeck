# Patches — adabraka-gpui

**Vendored from**: `adabraka-gpui` v0.3.0 *(the fork itself pre-dates our
patch tracking; v0.3.0 confirmed from `Cargo.toml` at bootstrap time)*.
**Upstream**: `https://github.com/Augani/adabraka-gpui` *(the repo is
currently 404 on GitHub even though crates.io lists it; sync workflow
falls back to the `https://static.crates.io/crates/adabraka-gpui/…`
tarball. If GitHub ever comes back, prefer that per `.agents/patches.md`
step 3.)*
**Last synced**: 2026-07-07 (inventory bootstrap — the fork itself
predates this file).

Total markers in code: **8**
(sum of the per-entry `Markers` lists below; SDPATCH-103 is Cargo.toml
only, out of the src/-scoped marker convention.)

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

### SDPATCH-103 — macOS `core-graphics` version bump

- **Files / symbols**:
  - `Cargo.toml` — `[target.'cfg(target_os = "macos")'.dependencies.core-graphics]`
    entry (bumps `version = "0.24"` to `"0.25"`)
- **Markers**: none — `Cargo.toml` is outside the `patches/<crate>/src/`
  marker scope. The entry exists so the sync knows to re-apply the bump
  after each overlay.
- **Why**: `core-text 21` pulls in `core-graphics 0.25`, and pinning
  gpui's `core-graphics` at `0.24` caused type mismatches inside the
  `core_text` font loader on macOS release builds (regression surfaced by
  `6881329` before this inventory existed). Bumping to `0.25` unifies the
  dependency tree with no effect on Linux/Windows.
- **Upstream status**: not filed yet — genuine bug, worth an upstream PR
  once we confirm the same tree still reproduces against a current
  upstream. If upstream ever pins `0.25` on its own, retire this entry.

### SDPATCH-104 — WGSL alignment padding for `Quad` and `Shadow`

- **Files / symbols**:
  - `src/scene.rs` — `pub(crate) struct Quad` (adds trailing `_pad: u32`)
  - `src/scene.rs` — `pub(crate) struct Shadow` (adds trailing `_pad: u32`)
  - `src/window.rs` — `Window::paint_shadows` (initialises `_pad: 0` on
    the `Shadow` primitive)
  - `src/window.rs` — `Window::paint_quad` (initialises `_pad: 0` on the
    `Quad` primitive)
- **Markers** (4 markers total, one per site):
  - `src/scene.rs:463` — `/// ShellDeck patch: WGSL alignment fix — `Bounds` contains `vec2<f32>``
  - `src/scene.rs:506` — `/// ShellDeck patch: WGSL alignment fix — same reasoning as `Quad::_pad``
  - `src/window.rs:2837` — `// ShellDeck patch: initialise the WGSL alignment padding`
  - `src/window.rs:2869` — `// ShellDeck patch: initialise the WGSL alignment padding`
- **Why**: WGSL treats a struct containing `vec2<f32>` (via `Bounds`) as
  8-byte aligned, so the *element stride* of `array<Quad>` / `array<Shadow>`
  in a storage buffer is rounded up to a multiple of 8. Rust `#[repr(C)]`
  with a trailing `u32` leaves the struct's own size at only a 4-byte
  multiple. Uploading a `&[Quad]` slice as an `array<Quad>` therefore
  misindexes every element after the first. Explicit `_pad: u32` keeps
  the Rust-side stride in lockstep with WGSL. This spans four non-adjacent
  sites — two struct definitions and their two initialisers — hence four
  markers grouped under one SDPATCH per `.agents/patches.md`.
- **Upstream status**: not filed yet — real bug worth reproducing +
  upstreaming.

### SDPATCH-105 — HLSL `squircle_sdf` parameter rename

- **Files / symbols**:
  - `src/platform/windows/shaders.hlsl` — `squircle_sdf` (parameter
    `point` → `pt`, and the two internal references)
- **Markers**:
  - `src/platform/windows/shaders.hlsl:305` — `// ShellDeck patch: HLSL keyword collision — the parameter was originally`
- **Why**: `point` is a reserved token in HLSL; `fxc.exe` (Windows shader
  compiler) fails with `unexpected token 'point'` on the vanilla
  signature. Renaming to `pt` is the smallest possible fix.
- **Upstream status**: **shipped natively in `adabraka-gpui` v0.5.1** —
  upstream applied the exact same rename. This patch retires on the
  next sync (the overlay will bring in upstream's version and the marker
  disappears with it); move the entry to `## Retired patches` at that
  point.

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
- **2026-07-07** — retro-inventory pass. Diffing the fork against vanilla
  `v0.3.0` surfaced three undocumented tweaks the bootstrap missed:
  the macOS `core-graphics` bump (`6881329`, now SDPATCH-103), the WGSL
  alignment padding on `Quad`/`Shadow` (present since `280f2ab`, now
  SDPATCH-104 with 4 markers), and the Windows HLSL `squircle_sdf`
  rename (`b0890e6`, now SDPATCH-105 — already superseded by upstream
  v0.5.1, tagged for retirement on the next sync). Marker count is now
  8 = 1 + 2 + 4 + 1 (SDPATCH-103 has none by design — `Cargo.toml` is
  outside the src/-scoped marker convention).

## Retired patches

*(empty for now — SDPATCH-105 will land here after the next sync brings
in upstream v0.5.1's native rename.)*
