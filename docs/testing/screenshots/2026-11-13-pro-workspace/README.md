# Linux Pro workspace visual check — 2026-11-13

These screenshots use the real Linux native renderer in private Xvfb with a
compositor, isolated XDG directories, a private D-Bus session and synthetic
Manage data on loopback port 8097. The test-only mount hides real SSH, Codex and
Claude profiles. No production account, browser profile or data was used.

- [Existing tools](01-tools.png): signed-in synthetic client and Pro entry.
- [Dedicated Pro shell](02-pro-shell.png): independent browser chrome, return to tools and change address; shared Chrome and agent-only controls are absent.
- [Compact address form](03-address-compact.png): complete form and fixed window controls at 600×400.
- [Enter-key validation](04-address-validation.png): invalid origin produces a readable error and keeps the submit button visible.
- [Native logout](06-logout.png): clicking the account menu's sign-out returns to the welcome screen and removes the Pro entry.

The Pro shell image shows the loading viewport, **not a rendered Pro page**.
The existing approved 0.13.0 browser helper subsequently failed Chromium sandbox
initialization under the additional test bwrap mount (SUID fallback owner/mode).
No sandbox was disabled and no host sandbox policy was changed. Independent CEF
web-flow checks are separate evidence; they do not validate the combined native
viewport, focus, page input or browser process lifecycle. SDTEST-3173 therefore
remains partial. macOS, Windows and matching release browser packaging remain
unverified.

Automated evidence: full UI suite 454 passed, one existing ignored (158.96s)
before the final dedicated browser mode addition; final SDTEST-3172/3174/3175
all passed (4.79s) afterward. The final native executable built successfully.
