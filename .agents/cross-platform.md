# Cross-platform support

ShellDeck is a **multi-function companion app that must run on Linux, macOS,
and Windows**. All three platforms are first-class targets — none is a
"best-effort" afterthought.

**Why:** the release pipeline (`.github/workflows/release.yml`) builds and
ships all three. If any of the three builds fails, the release + update
manifest jobs are skipped entirely, so a regression on any one platform
blocks *every* user from getting an update.

**How to apply:**

- **Before writing platform-specific code**, ask whether a portable
  alternative exists in an already-vendored dep (`tokio`, `portable-pty`,
  `keyring`, `notify`, `russh`, GPUI/adabraka-ui). Prefer the portable path.
- **When platform-specific code is unavoidable**, gate it with
  `#[cfg(target_os = "...")]` and provide a working implementation for all
  three targets — not a `todo!()` / `unimplemented!()` for the others.
- **Paths:** never hardcode `/`, `~`, or backslashes. Use `std::path::PathBuf`,
  `dirs::` (config/data/cache dirs), or the existing config-location helpers.
- **Shell-outs:** `xdg-open` (Linux) / `open` (macOS) / `start` (Windows) —
  see the existing `open_in_browser()` helper in
  `shelldeck-core::config::cloud_account` and reuse that pattern.
- **Line endings, env var casing, process spawning:** verify the Windows path
  explicitly. `cargo check` on Linux does **not** catch these.
- **CI is the source of truth.** `cargo check` locally on Linux is not
  sufficient — a change that touches build config, deps, or platform code
  must be validated against the full CI matrix before tagging a release. The
  pinned nightly (`rust-toolchain.toml`) is there specifically because a
  newer nightly broke the macOS release build silently — do not float it.
- **Install scripts:** any change to install/update flow must land in *both*
  `install.sh` (Linux + macOS) and `install.ps1` (Windows) in
  `cloudflare/update-worker/`.
- **Platform keys** in the update manifest use `macos-*`, never `darwin-*` —
  manifest, workflow, `shelldeck-update` client, and worker must all agree.

## Global shortcuts on Wayland — an environment limit, not an app bug

Wayland gives a client **no** way to grab a key it is not focused for. The only
route is the `org.freedesktop.portal.GlobalShortcuts` portal, which needs
xdg-desktop-portal **≥ 1.16** *and* a backend that implements it (GNOME's
arrived in **GNOME 48**). On a session without both, `ashpd` returns *"A portal
frontend implementing `org.freedesktop.portal.GlobalShortcuts` was not found"*
and no ShellDeck change can make the shortcut fire.

Verified on this dev box (2026-07-25, Ubuntu 22.04 / GNOME 42 Wayland,
xdg-desktop-portal 1.14.4, `xdg-desktop-portal-gtk` only): the interface is
absent from the session bus entirely.

**How to apply:**

- **Check the environment before reading code.** One command settles it:
  `gdbus introspect --session --dest org.freedesktop.portal.Desktop
  --object-path /org/freedesktop/portal/desktop | grep GlobalShortcuts`.
  No output = no global shortcuts on that session, full stop.
- **Do not "fix" it by falling back to X11.** Registration *succeeds* under
  XWayland (`XGrabKey` returns fine — verified), but a Wayland compositor only
  routes keys to XWayland while an X client is focused, so the grab never fires
  for the case that matters. A successful registration is not a working
  shortcut.
- **Report it as environmental.** `shortcut_error_is_portal_missing` in
  `settings.rs` maps this one error to a translated explanation; everything
  else reaches the user verbatim.

## Desktop companion on Wayland — top-level positioning is unavailable

Same class of limit as the portal above, different capability: Wayland gives a
client **no** protocol to position its own top-level window, so
`set_window_origin` cannot be implemented there and the roaming desktop
character cannot work on native Wayland. ShellDeck does not fake it: the
companion runtime reports `OverlayCapabilityTier::Unavailable`
(`crates/shelldeck/src/companion_desktop.rs`), keeps the character disabled,
and Appearance explains the limitation instead of showing a dead toggle.
External-window geometry of native Wayland apps is equally unavailable (no
compositor exposes a permitted global window-geometry API), so such windows
are never presented as climbable.

Full write-up: `docs/clippy.md` (§ Cross-platform behavior, and the
external-window filtering rules under § Desktop character runtime) and
`docs/testing/USE_CASES.md` SDUC-449 / SDUC-451.

- **Do not "fix" it** with an XWayland fallback, compositor-specific hacks, or
  guessed geometry — degrading honestly *is* the specified behavior
  (SDTEST-1570 pins the Appearance report; misclassifying X11 as limited is
  the regression to fear).
- **Detection follows GPUI**, not env vars: `companion_desktop::is_x11_session`
  wraps `gpui::guess_compositor()` and is the single source of truth —
  `XDG_SESSION_TYPE` is no longer consulted anywhere in the `shelldeck` crate.

## Optional Linux/X11 runtime tools — `xrandr` and `xprop`

Companion window placement on X11 shells out to `xrandr` (multi-monitor
bounds, `main.rs::x11_monitor_bounds`) and `xprop` (`_NET_WORKAREA`,
`main.rs::x11_workarea`). Both are **optional runtime dependencies**: spawn
failure, non-zero exit, or unparseable output logs a `tracing::warn` and falls
back to GPUI display bounds — never an error surface. Keep any new consumer on
that same warn-and-continue pattern.
