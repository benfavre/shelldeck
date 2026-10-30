# Native browser companion-pane QA — 2026-10-30

The browser now opens beside the conversation and composer through a header
globe, using the shared resizable split component. Narrow layouts stack the
panes. The expanded toolbox no longer duplicates Files, Changes, Terminal or
Browser; Sources has a distinct link icon. Browser runtime errors render once.
The globe retains/hides the browser, while its pane close button releases tabs
and scoped grants without changing the chat draft. Unsupported agent contexts
show a browsing-only status. Web pages without a CSS background use a white
browser canvas instead of inheriting a dark application background.

The development binary had been launched without its optional helper. Built
and staged the matching Chromium runtime; `scripts/run-dev.sh --main` now
provides a complete development launch. Ubuntu then rejected its user-namespace
sandbox. After explicit user approval and administrator authentication, loaded
`/etc/apparmor.d/shelldeck-browser-dev`, restricted to the exact 0.13.0 debug
helper path. No global sysctl, setuid mode or Chromium sandbox flag changed.

Validation:

- All 286 UI tests passed, including SDTEST-2798 for draft/session/context
  retention across conversation focus, browser hide and browser close.
- Workspace Clippy and native application build passed. The focused regression
  and Clippy were repeated after the sizing/unsupported-control adjustment.
- Packaged helper entrypoint test passed (SDTEST-2598).
- The real runtime smoke passed (SDTEST-2552), exercising paints, input,
  popups, dialogs, uploads/downloads, navigation, HiDPI and profile persistence.
- Full native app on private Xvfb with disposable XDG profile and loopback
  fixtures: URL entry, real CEF form typing/submission, second-page navigation,
  resizing, retained form/draft on hide/reopen, stacked 740×900 layout, and
  browser close with retained draft. No provider was called.
- Initial QA used a network/filesystem namespace. Chromium's nested sandbox
  could not start there; the final native CEF check used the same disposable
  XDG profile on private Xvfb, a deliberately unavailable keyring bus, synthetic
  Manage responses and a loopback-only page server.

Evidence: `/tmp/shelldeck-browser-qa/07-input-and-draft.png`, `08-resized.png`,
`10-stacked.png`, `12-navigation.png`, `13-closed-draft-retained.png`.
Runtime smoke evidence: `/tmp/shelldeck-cef-smoke.3566lstz`.

Native checks cover Linux. Remote/unsupported provider sessions can browse
manually; this UI change does not grant SSH agents control of the local browser.

Final build check: the unsupported-control button is hidden, divider drag stops
at the minimum chat width, and an unstyled page renders black text on a white
canvas (`17-white-page.png`).
