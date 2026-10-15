# Native browser QA — 2026-10-15

The native CEF helper and GPUI view were exercised without launching a provider,
using a loopback HTTP fixture and disposable XDG config/data/cache directories.
The production app, Chrome profiles, SSH connections and global CLI configuration
were not used for these checks.

- Native Wayland launch inherited `WAYLAND_DISPLAY=wayland-0`. The opt-in fixture
  verified a live 1100×656 Chromium viewport, matching frame/DPI, DOM and geometry;
  its audit completed successfully. Evidence: `/tmp/shelldeck-browser-wayland-final.YLYze7/cache`.
- Native pointer/keyboard checks targeted only the fixture's own X11 test window:
  typing `qa` once, clicking the page's submit button, switching browser tabs,
  Ctrl+T then Ctrl+W while the address field owned focus, returning to retained
  form contents, and selecting the second native dropdown option all passed.
  This X11 input fixture is separate from the production Wayland path.
- [Inspected native window screenshot](native-browser-preview.png) shows the
  retained form value, actual submit result and changed native dropdown.
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

Native macOS and Windows helper compilation, launcher loading and packaging are
covered by the added CI matrix. Remote results must be checked before a release;
a Linux test does not establish native input or signing behavior on those OSes.
