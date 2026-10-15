# Native browser QA — 2026-10-15

The native CEF helper and GPUI view were exercised without launching a provider,
using a loopback HTTP fixture and disposable XDG config/data/cache directories.
The production app, Chrome profiles, SSH connections and global CLI configuration
were not used for these checks.

- Native Wayland launch inherited `WAYLAND_DISPLAY=wayland-0`. The opt-in fixture
  verified a live 1100×656 Chromium viewport, matching frame/DPI, DOM and geometry;
  its bounded navigation/paint audit completed successfully. Evidence: `/tmp/shelldeck-browser-wayland-delivery.fQiZN2/cache`.
- Native pointer/keyboard checks targeted only the fixture's own X11 test window:
  typing `qa` once, clicking the page's submit button, switching browser tabs,
  Ctrl+T then Ctrl+W while the address field owned focus, returning to retained
  form contents, and selecting the second native dropdown option all passed.
  This X11 input fixture is separate from the production Wayland path.
- [Inspected native window screenshot](native-browser-preview.png) shows the
  retained form value, actual submit result and changed native dropdown.
- A further [native Enter-key check](native-browser-enter.png) typed `qa` and
  submitted the focused single-input form exactly once (`Submitted 1: qa`).
- The packaged helper smoke exercised real OSR frames, Unicode/emoji input,
  page dialogs, upload, downloaded bytes, related popup `window.opener` messaging,
  before-unload cancellation/confirmation, exact HiDPI resizing and profile
  persistence after a clean helper restart.
- The real MCP stdio smoke exercised native screenshots and accessibility,
  read-only refusal, click/type/key/scroll, Enter form submission, popup tab IDs,
  foreign-tab refusal and both control and complete grant revocation.
- The local combined suite passed 477 core, 236 UI, 13 browser and 15 updater tests
  (six opt-in core live tests excluded). Three pre-existing lifecycle failures
  were traced to Git 2.34's lack of `worktree list -z`; strict compatibility
  handling now preserves the same authority checks and passes those cases.

The [native CI matrix](https://github.com/benfavre/shelldeck/actions/runs/38795148349)
passed CEF helper compilation, complete payload staging, packaged launcher
loading and the actual MCP computer-use workflow on Linux, macOS and Windows.
The local UI suite was repeated with all three XDG directories isolated and
passed all 236 tests. Strict workspace Clippy and the full application build
also passed. Linux CI passed all 996 workspace tests. Native GPUI human-input
checks cover Linux; release signing and
notarization remain release-pipeline checks.
