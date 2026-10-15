# Assistant window placement — 2026-10-14

The first assistant window targets the primary monitor's right usable edge,
with a 240 ms slide. Docked corners mirror for left/right placement. Its
titlebar detaches it into a rounded, resizable floating window with standard
minimize/maximize/hide controls. Left/right buttons target the current monitor;
the monitor menu targets either edge of any connected monitor. Native tiling
updates the frame. Floating windows remain open across focus changes.

Hiding retains the same window and assistant entity. Showing it again retains
the current monitor, bounds and docking state. A generation check prevents a
pending exit timer from hiding a reopened window. Reduced motion skips sliding.
Window controls and resize handling are shared with the main ShellDeck window.

On Wayland, exact top-level coordinates and a primary display are unavailable
through the current backend. The assistant therefore opens as a normal
floating window. Native movement, resizing and compositor snapping remain
available; explicit coordinate-based placement is disabled with a tooltip.
No XWayland or compositor-specific workaround is used.

Checks with the pinned toolchain, locked dependencies, `PKG_CONFIG_PATH`,
debug information disabled and incremental compilation disabled:

- `cargo check --locked -p shelldeck`: passes.
- `cargo build --locked -p shelldeck`: passes; installed locally and restarted.
- `cargo test --locked -p shelldeck-ui --lib ai_dock::layout::tests`: 3 pass.
  Covers primary selection, usable edges, negative origins, small displays,
  detached margins, native tile recognition and the close/reopen race.
- `cargo test --locked -p shelldeck --bin shelldeck -- --test-threads=1`:
  101 pass, including the XRandR inventory regression, tray/deep-link lifecycle
  and global-shortcut routing.
- Full UI library suite: 207 pass, 3 fail in unchanged workspace lifecycle
  tests (`sdtest_1763`, `sdtest_1764`, `sdtest_1770`). Each fails while creating a
  test worktree with “The ShellDeck-owned worktree location changed before
  creation.” No worktree authorization code was changed for this task.
  Repeated under isolated XDG directories: same 207/3 result, with SHA-256
  checks confirming the real config and connection store stayed unchanged.
- Formatting and whitespace checks pass.

The first broad UI run wrote a fixture account/URL and default preferences to
the real profile through existing config persistence. The saved account,
theme and preferences were restored from the pre-setup backup, retaining the
authorized tmux attachment, startup restoration and Dev-mode changes. Legacy
plaintext tokens were omitted; the existing OS-keychain token successfully
loaded the real Manage site's inventory. Connection profiles/scripts were
retained. `.agents/testing.md` now requires an isolated XDG profile for desktop
test runs; `HOME` is not changed.

Native smoke through the normal tray command verifies one assistant creation,
an exit interrupted by reopening (stale hide cancelled), and a completed
hide followed by reopening the same native window. The running Wayland app
reports `Floating` with explicit positioning unavailable and remains alive after
remapping. A real `Infra AI · activ2` SSH connection authenticated after settings
recovery. The later application exit completed through normal graceful shutdown.
No AI request was submitted. The previous binary is retained under
`~/.shelldeck/bin/shelldeck.pre-assistant-window-*`.

Native multi-monitor drag/snap and all four corners in light/dark themes still
need pointer QA (SDTEST-1960). Native UI automation is unavailable in this
session. The macOS/Windows CI matrix must run before publishing a release;
this is a local Linux build.
