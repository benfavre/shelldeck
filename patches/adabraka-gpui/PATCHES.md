# Patches — adabraka-gpui

**Vendored from**: `adabraka-gpui` v0.5.1
**Upstream**: `https://github.com/Augani/adabraka-gpui` *(the repo is
currently 404 on GitHub even though crates.io lists it; sync workflow
falls back to the `https://static.crates.io/crates/adabraka-gpui/…`
tarball. If GitHub ever comes back, prefer that per `.agents/patches.md`
step 3.)*
**Last synced**: 2026-07-07 (v0.3.0 → v0.5.1)

Total marker occurrences in code: **128**
(`rg "ShellDeck patch:" src/`; SDPATCH-103 is Cargo.toml-only and outside
the src-scoped marker convention.)

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

### SDPATCH-106 — Dispatch Linux/X11 global hotkeys from the root window

- **Files / symbols**:
  - `src/platform/linux/global_hotkey.rs` — X11 lock-state grabs and ID matching
  - `src/platform/linux/x11/client.rs` — `X11Client::dispatch_global_hotkey`
- **Markers**:
  - `src/platform/linux/global_hotkey.rs:54` — `// ShellDeck patch: global grabs must survive Caps Lock and Num Lock state.`
  - `src/platform/linux/global_hotkey.rs:206` — `// ShellDeck patch: grab every lock-state variant and roll back partial grabs.`
  - `src/platform/linux/global_hotkey.rs:232` — `// ShellDeck patch: map root-window KeyPress events back to registered IDs.`
  - `src/platform/linux/global_hotkey.rs:248` — `// ShellDeck patch: release every lock-state grab registered above.`
  - `src/platform/linux/global_hotkey.rs:257` — `// ShellDeck patch: protect lock-state matching against regressions.`
  - `src/platform/linux/x11/client.rs:624` — `// ShellDeck patch: root-window hotkeys must bypass window/XIM routing.`
  - `src/platform/linux/x11/client.rs:680` — `// ShellDeck patch: invoke the Linux platform callback for matched root KeyPress events.`
- **Why**: X11 delivers a successful `GrabKey` as a `KeyPress` on the root
  window, but the upstream event path immediately looked that ID up as a GPUI
  application window (or forwarded it through XIM), returned `None`, and never
  invoked `on_global_hotkey`. Dispatching matched root events before normal
  window/XIM routing makes the callback functional. Grabbing and matching all
  Caps Lock / Num Lock variants keeps the shortcut reliable regardless of lock
  state.
- **Upstream status**: not filed yet — Linux/X11 framework bug suitable for an
  upstream PR; Wayland still needs a compositor shortcuts portal.

### SDPATCH-107 — Keep interactive overlays focusable on Linux/X11

- **Files / symbols**:
  - `src/platform/linux/x11/window.rs` — `XcbAtoms`, `X11WindowState::new`
  - `src/platform/linux/x11/client.rs` — `X11Client::handle_event`
- **Markers**:
  - `src/platform/linux/x11/window.rs:78` — `// ShellDeck patch: interactive overlays need a focusable EWMH type.`
  - `src/platform/linux/x11/window.rs:586` — `// ShellDeck patch: overlays host real inputs, so expose them as`
  - `src/platform/linux/x11/client.rs:936` — `// ShellDeck patch: passive keyboard grabs emit synthetic focus`
- **Why**: ShellDeck's standalone command palette and Assistant Dock are
  keyboard-interactive always-on-top surfaces. Advertising `WindowKind::Overlay`
  as `_NET_WM_WINDOW_TYPE_DOCK` made the window manager treat them like system
  panels, while unfiltered `FocusOut` events generated by passive keyboard grabs
  looked like real application switches and triggered auto-close. A focusable
  `UTILITY` type preserves the overlay position and above-state, and accepting
  only `NotifyMode::NORMAL` focus transitions separates genuine focus loss from
  grab/ungrab bookkeeping.
- **Upstream status**: not filed yet — the upstream API would ideally expose a
  distinct interactive-overlay kind rather than changing generic overlay
  semantics per platform.

### SDPATCH-108 — Discard stale XIM callbacks after window teardown

- **Files / symbols**:
  - `src/platform/linux/x11/client.rs` — `X11ClientStatePtr::drop_window`,
    `X11Client::xim_handle_commit`, `X11Client::xim_handle_preedit`
- **Markers**:
  - `src/platform/linux/x11/client.rs:250` — `// ShellDeck patch: forget XIM work targeting a transient window before`
  - `src/platform/linux/x11/client.rs:1366` — `// ShellDeck patch: a transient window may close before XIM delivers`
  - `src/platform/linux/x11/client.rs:1381` — `// ShellDeck patch: preedit callbacks can race transient-window`
- **Why**: XIM delivery is asynchronous, so closing the standalone palette or
  Assistant Dock can destroy its X11 window while a commit or preedit callback
  still targets that window. The upstream path kept the destroyed window as the
  active XIM target and logged the expected race as an internal bug. Clearing
  queued composition state during teardown and dropping only callbacks whose
  target no longer exists prevents stale text from reaching another surface and
  removes the misleading error.
- **Upstream status**: not filed yet — lifecycle hardening suitable for an
  upstream PR.

### SDPATCH-109 — Register Wayland global shortcuts through the XDG portal

- **Files / symbols**:
  - `src/platform.rs` — `GlobalHotkeyRegistrationEvent`, `Platform`
  - `src/app.rs` — `App::on_global_hotkey_registration`
  - `src/platform/linux/platform.rs` — `PlatformHandlers`, `Platform for LinuxPlatform`
  - `src/platform/linux/global_hotkey.rs` — `wayland::WaylandGlobalHotkey`
  - `src/platform/linux/wayland/client.rs` — `WaylandClientState`,
    `WaylandClient::new`, `LinuxClient for WaylandClient`
- **Markers**:
  - `src/platform.rs` — `// ShellDeck patch: surface asynchronous Wayland portal registration results.`
  - `src/platform.rs` — `// ShellDeck patch: let clients observe portal acceptance or refusal.`
  - `src/app.rs` — `// ShellDeck patch: expose asynchronous portal registration outcomes.`
  - `src/platform/linux/platform.rs` — `// ShellDeck patch: retain the Wayland portal registration-result callback.`
  - `src/platform/linux/platform.rs` — `// ShellDeck patch: route asynchronous Wayland registration outcomes.`
  - `src/platform/linux/global_hotkey.rs` — `// ShellDeck patch: bridge GPUI global-hotkey registrations through the`
  - `src/platform/linux/wayland/client.rs` — `// ShellDeck patch: retain the portal session manager with the Wayland client.`
  - `src/platform/linux/wayland/client.rs` — `// ShellDeck patch: marshal portal activations back onto calloop before`
  - `src/platform/linux/wayland/client.rs` — `// ShellDeck patch: expose the Wayland portal manager through LinuxClient.`
- **Why**: Wayland deliberately forbids the root-window grabs used by the X11
  backend. The XDG Global Shortcuts portal is the compositor-supported path:
  GPUI batches synchronous startup registrations into one portal session and
  one `BindShortcuts` request, emits XDG-spec preferred triggers, listens for
  `Activated`, then marshals IDs through calloop before invoking the existing
  foreground callback. Portal acceptance/refusal is also returned
  asynchronously to the application so Settings can leave its pending state
  and display the real outcome. Portal absence, denial, or an empty accepted
  set stays non-fatal and leaves ShellDeck's tray fallback available.
- **Upstream status**: not filed yet — generic GPUI capability suitable for an
  upstream PR after live validation on GNOME and KDE portal backends.

### SDPATCH-110 — Cross-platform `Window::set_window_origin`

- **Sticky**: required by `docs/clippy.md`; keep this API across GPUI fork syncs.
- **Files / symbols**:
  - `src/window.rs` — `Window::set_window_origin` public API
  - `src/platform.rs` — `PlatformWindow::set_window_origin` routing hook
  - `src/platform/windows/window.rs` — `WindowsWindow::set_window_origin`
  - `src/platform/mac/window.rs` — `MacWindow::set_window_origin`
  - `src/platform/linux/x11/window.rs` — `X11Window::set_window_origin`
  - `src/platform/linux/wayland/window.rs` — explicit unsupported Wayland backend
  - `src/platform/test/window.rs` — explicit unsupported test backend
- **Markers**:
  - `src/platform.rs` — `// ShellDeck patch: route public window-origin changes through each platform backend.`
  - `src/window.rs` — `// ShellDeck patch: expose cross-platform window positioning for ShellDeck's clippy tooling.`
  - `src/platform/windows/window.rs` — `// ShellDeck patch: move windows without resizing, changing Z-order, or activating them.`
  - `src/platform/mac/window.rs` — `// ShellDeck patch: map global GPUI top-left coordinates onto AppKit's global bottom-left space.`
  - `src/platform/linux/x11/window.rs` — `// ShellDeck patch: move X11 windows through ConfigureWindow without changing their size.`
  - `src/platform/linux/wayland/window.rs` — `// ShellDeck patch: Wayland does not allow clients to set top-level window coordinates.`
  - `src/platform/test/window.rs` — `// ShellDeck patch: test windows have no real platform surface to reposition.`
- **Why**: ShellDeck's clippy helper needs to place auxiliary GPUI windows without
  resizing or activating them. The public `Window` method delegates through
  `PlatformWindow`; Windows uses native global desktop coordinates with
  `SetWindowPos`, `SWP_NOACTIVATE`, `SWP_NOSIZE`, and `SWP_NOZORDER`, macOS
  converting GPUI's top-left logical coordinates to AppKit's bottom-left frame
  coordinates, and X11 issues `ConfigureWindow` with only `x`/`y`. Wayland,
  headless/default, and test windows return explicit unsupported errors because
  those environments have no client-controlled top-level origin to set. X11
  relies on the resulting `ConfigureNotify` to dispatch the moved callback
  rather than re-borrowing platform callbacks synchronously during frame work.
- **Upstream status**: not filed yet — generally useful platform API, but the
  Wayland semantics need to be clearly documented before upstreaming.

### SDPATCH-111 — Read-only external top-level window snapshots

- **Files / symbols**:
  - `src/app.rs` — `App::{global_display_bounds,global_display_metrics,visible_external_window_bounds,visible_external_windows,external_window}`
  - `src/platform.rs` — `ExternalWindowId`, `ExternalWindow`, `Platform::{global_display_bounds,visible_external_windows,external_window,visible_external_window_bounds}` and `PlatformDisplay::scale_factor`
  - `src/platform/linux/platform.rs` — `LinuxClient` and `Platform for P`
  - `src/platform/linux/x11/client.rs` — X11 property/geometry helpers and
    `LinuxClient::{visible_external_windows,external_window,visible_external_window_bounds}`
  - `src/platform/linux/x11/window.rs` — `_NET_WM_WINDOW_TYPE_DESKTOP` atom
  - `src/platform/windows/display.rs` — per-monitor DPI scale exposure
  - `src/platform/windows/platform.rs` — Win32 enumeration/filtering helpers and
    `Platform::{visible_external_windows,external_window}`
  - `src/platform/mac.rs` — `external_windows` module wiring
  - `src/platform/mac/display.rs` — `MacDisplay::global_bounds`
  - `src/platform/mac/platform.rs` — `Platform::{visible_external_windows,external_window}`
  - `src/platform/mac/external_windows.rs` — CoreGraphics window-list and targeted lookup helpers
- **Markers**:
  - `src/app.rs` — `// ShellDeck patch: expose global display geometry for cross-monitor desktop companions.`
  - `src/app.rs` — `// ShellDeck patch: import public desktop-companion snapshot and metrics types for App APIs.`
  - `src/app.rs` — `// ShellDeck patch: expose scale-aware global display metrics for desktop companions.`
  - `src/app.rs` — `// ShellDeck patch: expose read-only external window geometry for desktop companions.`
  - `src/app.rs` — `// ShellDeck patch: expose stable read-only external window snapshots for desktop companions.`
  - `src/app.rs` — `// ShellDeck patch: expose targeted external-window lookup for attached companions.`
  - `src/platform.rs` — `// ShellDeck patch: document the optional diagnostic raw external-window ID accessor.`
  - `src/platform.rs` — `// ShellDeck patch: document the public external-window bounds field.`
  - `src/platform.rs` — `// ShellDeck patch: document the public native external-window ID field.`
  - `src/platform.rs` — `// ShellDeck patch: expose documented external-window snapshots with native IDs and bounds.`
  - `src/platform.rs` — `// ShellDeck patch: expose typed external-window IDs without making raw IDs structural API.`
  - `src/platform.rs` — `// ShellDeck patch: let backends and tests construct typed IDs from native lifetime values.`
  - `src/platform.rs` — `// ShellDeck patch: expose per-display scale for coherent mixed-DPI desktop routing.`
  - `src/platform.rs` — `// ShellDeck patch: expose global display geometry for cross-monitor desktop companions.`
  - `src/platform.rs` — `// ShellDeck patch: platform backends may expose safe read-only external window snapshots.`
  - `src/platform.rs` — `// ShellDeck patch: allow targeted native-ID lookup without forcing callers to rescan all windows.`
  - `src/platform.rs` — `// ShellDeck patch: preserve the legacy bounds-only external window API.`
  - `src/platform/linux/platform.rs` — `// ShellDeck patch: import external-window snapshot types for the Linux platform trait.`
  - `src/platform/linux/platform.rs` — `// ShellDeck patch: X11 overrides this while Wayland retains the safe empty snapshot fallback.`
  - `src/platform/linux/platform.rs` — `// ShellDeck patch: X11 overrides targeted lookup while Wayland keeps the safe None fallback.`
  - `src/platform/linux/platform.rs` — `// ShellDeck patch: preserve the legacy bounds-only external window API.`
  - `src/platform/linux/platform.rs` — `// ShellDeck patch: route external desktop snapshots through the active Linux backend.`
  - `src/platform/linux/platform.rs` — `// ShellDeck patch: route targeted external desktop lookup through the active Linux backend.`
  - `src/platform/linux/platform.rs` — `// ShellDeck patch: route legacy external desktop geometry through the active Linux backend.`
  - `src/platform/linux/x11/client.rs` — `// ShellDeck patch: convert visible external X11 top-level windows to global logical bounds.`
  - `src/platform/linux/x11/client.rs` — `// ShellDeck patch: expand client geometry to the EWMH outer frame used as companion collision chrome.`
  - `src/platform/linux/x11/client.rs` — `// ShellDeck patch: import external-window snapshot types for X11 native IDs.`
  - `src/platform/linux/x11/client.rs` — `// ShellDeck patch: enumerate eligible visible X11 windows with native XID snapshots for companion climbing.`
  - `src/platform/linux/x11/client.rs` — `// ShellDeck patch: target one X11 XID directly for attached companion following.`
  - `src/platform/linux/x11/window.rs` — `// ShellDeck patch: external geometry excludes desktop-background windows.`
  - `src/platform/linux/x11/window.rs` — `// ShellDeck patch: include window-manager chrome in external companion geometry.`
  - `src/platform/windows/display.rs` — `// ShellDeck patch: report the monitor DPI scale alongside global display geometry.`
  - `src/platform/windows/platform.rs` — `// ShellDeck patch: enumerate visible external top-level Win32 windows with native HWND snapshots.`
  - `src/platform/windows/platform.rs` — `// ShellDeck patch: expose visible external top-level Win32 window snapshots.`
  - `src/platform/windows/platform.rs` — `// ShellDeck patch: target one Win32 HWND directly for attached companion following.`
  - `src/platform/mac.rs` — `// ShellDeck patch: wire the read-only external window geometry helper.`
  - `src/platform/mac/display.rs` — `// ShellDeck patch: retain CoreGraphics global origins for desktop companion routing.`
  - `src/platform/mac/external_windows.rs` — `// ShellDeck patch: enumerate visible external top-level macOS windows with CoreGraphics IDs.`
  - `src/platform/mac/external_windows.rs` — `// ShellDeck patch: import external-window snapshot types for CoreGraphics window IDs.`
  - `src/platform/mac/external_windows.rs` — `// ShellDeck patch: include kCGWindowNumber in each external-window snapshot.`
  - `src/platform/mac/external_windows.rs` — `// ShellDeck patch: preserve the legacy bounds-only macOS helper for App compatibility.`
  - `src/platform/mac/external_windows.rs` — `// ShellDeck patch: target one CoreGraphics window ID directly for attached companion following.`
  - `src/platform/mac/external_windows.rs` — `// ShellDeck patch: read CoreGraphics' native per-window lifetime ID.`
  - `src/platform/mac/platform.rs` — `// ShellDeck patch: return CoreGraphics global display origins instead of local NSScreen coordinates.`
  - `src/platform/mac/platform.rs` — `// ShellDeck patch: expose visible external top-level macOS window snapshots.`
  - `src/platform/mac/platform.rs` — `// ShellDeck patch: expose targeted CoreGraphics external-window lookup.`
- **Why**: The desktop companion can climb only geometry that is current and
  safe to observe. The App-facing APIs return global bounds plus per-display DPI
  scale and stable external-window snapshots without mutating platform state.
  `visible_external_window_bounds` remains as a compatibility wrapper over
  `visible_external_windows`, while `external_window` lets attached followers
  refresh one known native ID without rescanning every top-level window.
  `ExternalWindowId` stores the raw native lifetime
  ID privately, exposes typed equality/hash/copy, and provides `from_raw` for
  backends/tests plus a documented diagnostic `raw()` accessor. ShellDeck uses
  those metrics to build a coherent native Windows desktop coordinate space for
  mixed-DPI monitor routing.
  X11 uses the EWMH stacking list and returns
  mapped external input/output windows with their native XIDs after excluding
  ShellDeck-owned, hidden, fullscreen, desktop-background, and zero-size
  surfaces. When `_NET_FRAME_EXTENTS` is available with an exact CARDINAL/32
  four-value payload inside the defensive extent bound, X11 expands client
  geometry to the outer window-manager frame so collision and perching use the
  visible top chrome, and can re-check one XID directly with the same filters.
  Windows uses Win32 `EnumWindows` plus
  visibility/iconic/owner/toolwindow/DWM cloaking/fullscreen filters, excluding
  ShellDeck-owned HWNDs and returning the raw HWND pointer value; its companion
  geometry uses native global desktop coordinates so per-window DPI scaling
  cannot overlap monitors, and can re-check one HWND directly. The targeted
  lookup reconstructs the pointer-backed `HWND` and passes it as `Some(hwnd)`
  to the nullable Windows 0.61 API. macOS uses CoreGraphics'
  on-screen window list with desktop elements excluded, keeps only normal layer-0
  windows, returns `kCGWindowNumber`, and drops fullscreen-like display-covering
  windows; targeted lookup uses `kCGWindowListOptionIncludingWindow` with the
  same filtering. Wayland, test, headless/default backends intentionally return
  an empty vector or `None` because they do not expose other clients' top-level
  geometry safely.
- **Upstream status**: not filed yet — useful as an opt-in desktop integration
  API, but platform privacy and capability semantics need upstream discussion.

### SDPATCH-112 — Platform companion hardening, work areas, and reduced motion

- **Files / symbols**:
  - `src/app.rs` — `App::{desktop_display_metrics,prefers_reduced_motion}`
  - `src/platform.rs` — `DesktopDisplayMetrics`, `Platform::desktop_display_metrics`,
    `Platform::prefers_reduced_motion`, and `PlatformDisplay::work_area`
  - `src/platform/windows/window.rs` — non-activating `WindowKind::Overlay` style,
    first show, and explicit activation path
  - `src/platform/windows/display.rs` — `WindowsDisplay::{all,desktop_metrics}`
    and `rcWork` work-area conversion
  - `src/platform/windows/platform.rs` — physical desktop display metrics and
    `SPI_GETCLIENTAREAANIMATION`
  - `src/platform/mac/display.rs` — `MacDisplay::{global_work_area,scale_factor}`
    using `NSScreen.visibleFrame`
  - `src/platform/mac/platform.rs` — desktop display metrics and
    accessibility reduce-motion preference
  - `src/platform/linux/x11/display.rs` — `_NET_WORKAREA` work-area fallback and
    scale reporting
  - `src/platform/linux/platform.rs` — cheap GTK animation preference fallback
- **Markers** (23 net-new markers; the App import marker is renamed in SDPATCH-111):
  - `src/app.rs` — `// ShellDeck patch: expose coherent desktop metrics with per-display work area.`
  - `src/app.rs` — `// ShellDeck patch: expose reduced-motion platform preference for animation policy.`
  - `src/platform.rs` — `// ShellDeck patch: expose desktop display metrics in the same coordinate space as window routing.`
  - `src/platform.rs` — `// ShellDeck patch: cheap platform preference hook for reducing non-essential animation.`
  - `src/platform.rs` — `// ShellDeck patch: add a coherent desktop metrics API for companion placement.`
  - `src/platform.rs` — `// ShellDeck patch: expose per-display usable work area with a safe full-bounds fallback.`
  - `src/platform/linux/platform.rs` — `// ShellDeck patch: cheap Linux fallback for GTK's gtk-enable-animations setting.`
  - `src/platform/linux/platform.rs` — `// ShellDeck patch: honor cheap Linux animation settings fallbacks.`
  - `src/platform/linux/x11/display.rs` — `// ShellDeck patch: use EWMH _NET_WORKAREA when the window manager exposes it.`
  - `src/platform/linux/x11/display.rs` — `// ShellDeck patch: report X11 scale alongside display metrics.`
  - `src/platform/linux/x11/display.rs` — `// ShellDeck patch: read the first EWMH work area entry and keep it root-bounds-relative.`
  - `src/platform/mac/display.rs` — `// ShellDeck patch: expose AppKit's true visible work area in GPUI's global top-left coordinates.`
  - `src/platform/mac/display.rs` — `// ShellDeck patch: expose NSScreen.visibleFrame as the per-display usable work area.`
  - `src/platform/mac/display.rs` — `// ShellDeck patch: report the backing scale for display metrics.`
  - `src/platform/mac/platform.rs` — `// ShellDeck patch: expose CoreGraphics global display metrics with AppKit visible work areas.`
  - `src/platform/mac/platform.rs` — `// ShellDeck patch: query macOS Accessibility reduce-motion preference cheaply.`
  - `src/platform/windows/display.rs` — `// ShellDeck patch: keep concrete monitor metrics so Windows can expose physical desktop coordinates.`
  - `src/platform/windows/display.rs` — `// ShellDeck patch: Windows rcWork gives the true taskbar-aware per-monitor work area.`
  - `src/platform/windows/platform.rs` — `// ShellDeck patch: Windows desktop companion placement uses native physical pixels.`
  - `src/platform/windows/platform.rs` — `// ShellDeck patch: map Windows client-area animation preference to reduced motion.`
  - `src/platform/windows/window.rs` — `// ShellDeck patch: overlays are interactive but must never activate or steal foreground focus.`
  - `src/platform/windows/window.rs` — `// ShellDeck patch: opening an overlay must be non-activating even when first shown.`
  - `src/platform/windows/window.rs` — `// ShellDeck patch: explicit activation requests keep overlays visible without focus theft.`
- **Why**: ShellDeck's desktop companions need a single explicit placement
  contract that separates native desktop routing coordinates from normal GPUI
  window-creation coordinates. `DesktopDisplayMetrics::global_bounds` and
  `global_work_area` are in the coordinate space accepted by
  `Window::set_window_origin`; on Windows this is native physical desktop pixels
  to avoid mixed-DPI virtual monitor overlap. `logical_work_area` remains in
  normal GPUI logical coordinates for `WindowOptions::window_bounds` and other
  creation-time APIs. Per-display work areas use Win32 `rcWork`, macOS
  `NSScreen.visibleFrame` converted to GPUI's global top-left space (with
  fully-qualified `NSArray::objectAtIndex` access so the Cocoa trait resolves
  without an ambient import), and X11
  EWMH `_NET_WORKAREA` intersected with the root/display bounds; Wayland,
  headless, and test backends inherit the safe full-bounds fallback.
  Windows overlay windows also gain `WS_EX_NOACTIVATE` and non-activating show
  paths so interactive mouse input remains available without stealing foreground
  focus. Reduced-motion is a cheap best-effort platform query: Windows uses
  `SPI_GETCLIENTAREAANIMATION`, macOS uses the accessibility reduce-motion flag,
  Linux uses GTK animation settings from environment/config files, and other
  platforms default to `false`.
- **Upstream status**: not filed yet — should be split into smaller upstreamable
  platform capability PRs after native Windows/macOS validation.

### SDPATCH-113 — Companion external-window filter parity on X11

- **Files / symbols**:
  - `src/platform/linux/x11/client.rs` — `X11ClientState` (new
    `companion_excluded_window_types` field, interned in `X11Client::new`),
    `companion_excluded_window_types()`, `x11_window_has_transient_for()`,
    `is_visible_external_x11_window`
- **Markers** (7 markers, one per site):
  - `src/platform/linux/x11/client.rs:193` — `// ShellDeck patch: EWMH window types the companion external-window filter excludes, interned once at startup.`
  - `src/platform/linux/x11/client.rs:368` — `// ShellDeck patch: intern the extra EWMH window types the companion filter excludes beyond the XcbAtoms bundle.`
  - `src/platform/linux/x11/client.rs:513` — `// ShellDeck patch: companion external-window filter exclusion list interned above.`
  - `src/platform/linux/x11/client.rs:1596` — `// ShellDeck patch: EWMH window types the desktop companion must never treat as`
  - `src/platform/linux/x11/client.rs:1634` — `// ShellDeck patch: WM_TRANSIENT_FOR marks a window as owned by another window`
  - `src/platform/linux/x11/client.rs:1671` — `// ShellDeck patch: parity with the Windows/macOS companion filters —`
  - `src/platform/linux/x11/client.rs:1677` — `// ShellDeck patch: transient windows are owned popups/dialogs — parity`
- **Why**: SDPATCH-111's X11 filter only excluded ShellDeck-owned, hidden,
  fullscreen, desktop-background, and zero-size windows, so the desktop
  companion treated docks, panels, menus, tooltips and other window-manager
  chrome as climbable platforms — unlike the Windows backend (which filters
  `WS_EX_TOOLWINDOW` and owned windows) and the macOS backend (which keeps
  only layer-0 windows). The filter now also excludes the EWMH types
  `_NET_WM_WINDOW_TYPE_{DOCK,MENU,TOOLBAR,TOOLTIP,POPUP_MENU,DROPDOWN_MENU,
  SPLASH,NOTIFICATION,UTILITY}` (DESKTOP stays excluded as before) and any
  window with `WM_TRANSIENT_FOR` set (owned dialogs/popups; the property is a
  predefined core atom, read via `AtomEnum::WM_TRANSIENT_FOR`). The six types
  `XcbAtoms` does not already intern are interned once in `X11Client::new` —
  batched cookies, same failure handling as `XcbAtoms::new` — and cached on
  `X11ClientState`, keeping the companion-only exclusion list next to its
  single consumer instead of widening the shared `XcbAtoms` bundle in
  `window.rs`. Folding them into `XcbAtoms` later is a fine simplification;
  the markers above are the sites to touch.
- **Upstream status**: not filed yet — extends SDPATCH-111; batch with it if
  that API is ever upstreamed.

### SDPATCH-114 — Rounded `ObjectFit::Cover` images use layout bounds

- **Files / symbols**:
  - `src/elements/img.rs` — `Img::paint`
  - `src/window.rs` — `Window::paint_image`
  - `src/scene.rs` — `PolychromeSprite`
  - `src/platform/blade/shaders.wgsl` — `PolychromeSprite`, `fs_poly_sprite`
  - `src/platform/windows/shaders.hlsl` — `PolychromeSprite`,
    `polychrome_sprite_fragment`
  - `src/platform/mac/shaders.metal` — `polychrome_sprite_fragment`
- **Markers** (9 markers):
  - `src/elements/img.rs` — `// ShellDeck patch: ObjectFit::Cover expands`
  - `src/window.rs` — `/// ShellDeck patch: image sampling bounds and rounded mask bounds differ`
  - `src/window.rs` — `// ShellDeck patch: emoji sprites use their sampling bounds as`
  - `src/scene.rs` — `// ShellDeck patch: rounded image clipping follows the element bounds,`
  - `src/platform/blade/shaders.wgsl` — `// ShellDeck patch: separate Cover sampling geometry from rounded mask geometry.`
  - `src/platform/blade/shaders.wgsl` — `// ShellDeck patch: clip against the laid-out image box, not the oversized Cover texture bounds.`
  - `src/platform/windows/shaders.hlsl` — `// ShellDeck patch: separate Cover sampling geometry from rounded mask geometry.`
  - `src/platform/windows/shaders.hlsl` — `// ShellDeck patch: clip against the laid-out image box, not the oversized Cover texture bounds.`
  - `src/platform/mac/shaders.metal` — `// ShellDeck patch: clip against the laid-out image box, not the oversized`
- **Why**: `Img::paint` computes expanded sampling bounds for
  `ObjectFit::Cover`, but the polychrome sprite shader also used those expanded
  bounds as its rounded-rectangle SDF. The actual element corners then sat on a
  straight section of the oversized image and remained square. Carrying the
  original layout bounds separately lets the texture keep its correct cover
  crop while the alpha mask follows the visible card exactly — no inset,
  border, distortion or preprocessed bitmap required.
- **Upstream status**: not filed yet; suitable for a focused GPUI image-rendering
  bug report and PR.

### SDPATCH-115 — Element animations honor reduced motion

- **Files / symbols**:
  - `src/elements/animation.rs` — `AnimationElement::request_layout`
- **Markers** (1 marker):
  - `src/elements/animation.rs` — `// ShellDeck patch: reduced-motion platforms snap element animations and schedule no follow-up frames.`
- **Why**: SDPATCH-112 exposed the operating system's reduced-motion
  preference, but GPUI's element animation wrapper still requested a frame on
  every display refresh. One-shot animations now snap to their final state;
  repeating animations render their stable initial state, report themselves
  complete, and schedule no follow-up frame. Chained one-shot animations use
  the final animation in the chain so reduced motion does not leave transitional
  chrome mounted in its first pose.
- **Upstream status**: not filed yet — an upstream policy may also want a
  per-animation opt-out for essential motion.

## Preserved files (do not overwrite on sync)

- `PATCHES.md` (this file)
- `src/elements/div.rs` — hosts an in-progress smooth-scroll animation
  patch. **NOT** part of our replayable SDPATCH set (no marker convention
  applies inside it) and not tracked here beyond this note; the
  `/sync-patches` workflow must leave it alone (see the "Non-negotiables"
  section of `.agents/patches.md`). If a sync introduces upstream changes
  to `div.rs`, stop and report — do not merge them silently.

### SDPATCH-041 — a run background is a chip, not a rectangle glued to the glyphs

- **Files / symbols**:
  - `src/text_system/line.rs` — `paint_run_background`, the three background
    quad sites in `WrappedLineLayout::paint_background`
- **Markers** (4):
  - `src/text_system/line.rs` — `// ShellDeck patch: SDPATCH-041 — a run background is a chip, not a rectangle`
  - `src/text_system/line.rs` — `// ShellDeck patch: SDPATCH-041 — chip geometry.` (×3, one per quad site)
- **Why**: `TextRun::background_color` painted a bare rect of exactly the run's
  advance and the full line height. That reads as *selected text*, not as a
  token: the first and last glyphs touch the fill and the square corners
  collide with the surrounding prose. ShellDeck paints resolved `@mentions`
  with a run background, and a mention has to look like one object, so every
  run background now gains a small horizontal padding, a vertical inset and
  rounded corners.

  Applied unconditionally rather than behind a new `TextRun` field: adding one
  would ripple through every struct literal in gpui, adabraka-ui and ShellDeck
  for a purely cosmetic option. The blast radius is small — the only other
  producer of run backgrounds in the tree is adabraka's inline-HTML
  `style="background-color:…"` path, which ShellDeck does not use.

  A run that wraps is painted once per visual line, so each fragment is rounded
  on its own. That is what browsers do with an inline background, and it keeps
  the fragments legible instead of producing one shape with a hole in it.
- **Upstream status**: not filed yet.

## Sync log

- **2026-08-23** — Added SDPATCH-115. The header's accumulated count had
  drifted from the source (114 documented versus 127 actual before this patch),
  so it was reconciled to the auditable source count; the new marker brings the
  total to 128.
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
