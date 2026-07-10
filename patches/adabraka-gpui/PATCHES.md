# Patches — adabraka-gpui

**Vendored from**: `adabraka-gpui` v0.5.1
**Upstream**: `https://github.com/Augani/adabraka-gpui` *(the repo is
currently 404 on GitHub even though crates.io lists it; sync workflow
falls back to the `https://static.crates.io/crates/adabraka-gpui/…`
tarball. If GitHub ever comes back, prefer that per `.agents/patches.md`
step 3.)*
**Last synced**: 2026-07-07 (v0.3.0 → v0.5.1)

Total markers in code: **9**
(sum of the per-entry `Markers` lists below; SDPATCH-103 is Cargo.toml
only, out of the src/-scoped marker convention.)

## Patches

### SDPATCH-101 — `PathPromptOptions::starting_directory`

- **Files / symbols**:
  - `src/platform.rs` — `PathPromptOptions` struct
- **Markers**:
  - `src/platform.rs:1703` — `/// ShellDeck patch: initial directory the OS picker should open in`
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
  - `src/platform/linux/platform.rs:356` — `// ShellDeck patch: capture two identifier futures so the picker can`
  - `src/platform/linux/platform.rs:374` — `// ShellDeck patch: pre-seed the picker's starting folder`
- **Why**: Threads SDPATCH-101 into
  `ashpd::desktop::file_chooser::OpenFileRequest::current_folder()`.
  `OpenFileRequest` doesn't `Clone` and `current_folder` consumes it on
  error, so we capture a second `window_identifier()` future up front
  (first marker) and use it to rebuild the request without the folder
  hint if `current_folder` rejects the path (second marker). Two markers
  because the fix legitimately spans two non-adjacent locations in the
  same function.
- **Upstream status**: pairs with SDPATCH-101 in the same PR.

### SDPATCH-103 — macOS `core-graphics` / `core-text` alignment

- **Files / symbols**:
  - `Cargo.toml` — `[target.'cfg(target_os = "macos")'.dependencies.core-graphics]`
    entry (bumps `version = "0.24"` to `"0.25"`)
  - `Cargo.toml` — `[target.'cfg(target_os = "macos")'.dependencies.core-text]`
    entry (relaxes `version = "=21.0.0"` to `"22"`)
- **Markers**: none — `Cargo.toml` is outside the `patches/<crate>/src/`
  marker scope. The entries exist so the sync knows to re-apply them
  after each overlay.
- **Why**: `core-text 21.0.0` (what upstream's `=21.0.0` pin resolves
  to) pulls in `core-graphics 0.24`, so gpui's own `core-graphics 0.25`
  code cross-calls into `core_text::font::*` signatures typed with the
  wrong `CGFont`, producing 7× E0308 mismatches on macOS release builds.
  `core-text 21.1.0` was upstream's intended fix (uses `core-graphics
  0.25`) but has since been **yanked** from crates.io, so pinning `"21"`
  silently falls back to 21.0.0 and reintroduces the bug. Bumping to
  `core-text = "22"` (uses `core-graphics 0.25`, not yanked) is the
  stable path. `zed-font-kit` fork carries the same bump — both sides
  need it for cargo to unify. No effect on Linux/Windows.
- **Upstream status**: not filed yet — worth an upstream PR once the
  yank/reissue situation on `core-text` settles. If upstream ever pins
  a compatible `core-text` on its own, retire this entry.

### SDPATCH-104 — WGSL alignment padding for `Quad` and `Shadow`

- **Files / symbols**:
  - `src/scene.rs` — `pub(crate) struct Quad` (adds interior
    `_pad_transform: u32` between `continuous_corners` and `transform`,
    and a trailing `_pad: u32` after `blend_mode`)
  - `src/scene.rs` — `pub(crate) struct Shadow` (adds trailing `_pad: u32`
    after `inset`)
  - `src/window.rs` — `Window::paint_shadows` (initialises `_pad: 0` on
    the `Shadow` primitive)
  - `src/window.rs` — `Window::paint_quad` (initialises `_pad_transform: 0`
    and `_pad: 0` on the `Quad` primitive)
- **Markers** (6 markers total, one per site):
  - `src/scene.rs:520` — `/// ShellDeck patch: interior padding — WGSL's `TransformationMatrix``
  - `src/scene.rs:531` — `/// ShellDeck patch: trailing pad — with `_pad_transform` above the tail`
  - `src/scene.rs:574` — `/// ShellDeck patch: WGSL alignment fix — same reasoning as `Quad::_pad``
  - `src/window.rs:2842` — `// ShellDeck patch: initialise the WGSL alignment padding` *(Shadow)*
  - `src/window.rs:2874` — `// ShellDeck patch: initialise the interior WGSL alignment`
  - `src/window.rs:2880` — `// ShellDeck patch: initialise the trailing WGSL alignment`
- **Why**: two intertwined WGSL/Rust alignment mismatches:
  1. **Element stride**: WGSL treats a struct containing `vec2<f32>` (via
     `Bounds`) as 8-byte aligned, so `array<Quad>` / `array<Shadow>` round
     the element stride up to a multiple of 8. Rust `#[repr(C)]` with a
     trailing `u32` doesn't add that padding on its own, so the Rust
     `sizeof` lands 4 bytes short — misindexes every element after the
     first. Fixed by the trailing `_pad: u32`.
  2. **Interior alignment**: `TransformationMatrix` in WGSL contains
     `mat2x2<f32>` (align 8) so the shader implicitly pads 4 bytes before
     `transform`. Rust's `[[f32; 2]; 2]` is align 4 → no implicit pad, so
     every field after `continuous_corners` is 4 bytes early on the Rust
     side. Symptom: `background` / `border_color` were read from the
     wrong bytes shader-side, translating to alpha=0 on every solid fill
     (the whole UI rendered translucent — desktop showed through, cf.
     `img.ascencia.re/C18BPYwyhd5H.png` before the split). Fixed by the
     `_pad_transform: u32` between `continuous_corners` and `transform`.
  Upstream v0.5.1 already applies the trailing variant to `Underline`
  (`pub pad: u32, // align to 8 bytes` between `order` and `bounds`) but
  hasn't propagated any of it to Quad/Shadow.
- **Upstream status**: not filed yet — real bug worth reproducing +
  upstreaming; batch with SDPATCH-101/102 in one Augani/adabraka-gpui PR.

## Preserved files (do not overwrite on sync)

- `PATCHES.md` (this file)
- `src/elements/div.rs` — hosts an in-progress smooth-scroll animation
  patch. **NOT** part of our replayable SDPATCH set (no marker convention
  applies inside it) and not tracked here beyond this note; the
  `/sync-patches` workflow must leave it alone (see the "Non-negotiables"
  section of `.agents/patches.md`). If a sync introduces upstream changes
  to `div.rs`, stop and report — do not merge them silently.

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
- **2026-07-07** — synced v0.3.0 → v0.5.1. SDPATCH-101/102/103/104
  replayed clean (only line-number shifts and the two new `Quad` fields
  `transform`/`blend_mode` to sit above the `_pad`); SDPATCH-105 retired
  (upstream v0.5.1 shipped the same `point → pt` rename in
  `squircle_sdf`). Initial post-sync marker count was 7 = 1 + 2 + 4 + 0
  (SDPATCH-105 moved to `## Retired patches`, SDPATCH-103 remains
  marker-less by design). The workflow's "stop and report on upstream
  `div.rs` changes" rule was consciously overridden for this sync —
  user opted to port the smooth-scroll WIP onto v0.5.1's `div.rs` in the
  same run rather than defer. v0.5.1 also adds `transform`/`blend_mode`
  fields to `PaintQuad` — workspace call sites in `shelldeck-*` that
  construct `PaintQuad` had to be updated in the same sync.
- **2026-07-07** — SDPATCH-104 hardened at runtime. First launch panicked
  on `blade_graphics::shader:105` (`Host struct 'Quad' size doesn't match
  the shader, left: 252 right: 256`) → bumped trailing `_pad` from `u32`
  to `[u32; 2]`. Second launch didn't panic but rendered every solid
  fill translucent (desktop bled through the whole UI) — root cause was
  the WGSL `mat2x2<f32>` alignment inside `TransformationMatrix` forcing
  an implicit 4-byte pad before `transform` shader-side that Rust's
  `[[f32; 2]; 2]` doesn't emit. Split the pad: interior
  `_pad_transform: u32` between `continuous_corners` and `transform`,
  plus trailing `_pad: u32`. Marker count is now 9 = 1 + 2 + 6 + 0.
  Runtime confirmed opaque paints.

## Retired patches

### SDPATCH-105 — HLSL `squircle_sdf` parameter rename *(retired 2026-07-07)*

- **Files / symbols** (historical):
  - `src/platform/windows/shaders.hlsl` — `squircle_sdf` (parameter
    `point` → `pt`, and the two internal references)
- **Why we needed it**: `point` is a reserved token in HLSL; `fxc.exe`
  (Windows shader compiler) failed with `unexpected token 'point'` on
  the vanilla signature. Renaming to `pt` was the smallest possible fix.
- **Why we retired it**: adabraka-gpui v0.5.1 shipped the exact same
  rename natively (`float squircle_sdf(float2 pt, …)` in the upstream
  tree). The overlay brought in upstream's version and no divergence
  remains.
