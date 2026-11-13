# Linux Pro workspace visual check — 2026-11-13

Both runs use the real Linux GPUI renderer in private Xvfb with a compositor,
disposable XDG directories and a private D-Bus session. HOME stays unchanged.
No production account or real browser profile was used. The runs cover different
parts of the integration and must not be presented as one full packaged run.

## Full Workspace shell and lifecycle

The full application used synthetic Manage data on loopback port 8097. A test-only
bwrap mount hid real SSH, Codex and Claude profiles.

- [Existing tools](01-tools.png): signed-in synthetic client and Pro entry.
- [Dedicated Pro shell](02-pro-shell.png): independent browser chrome, return to tools and change address; shared Chrome and agent-only controls are absent.
- [Compact address form](03-address-compact.png): complete form and fixed window controls at 600×400.
- [Enter-key validation](04-address-validation.png): invalid origin produces a readable error and keeps the submit button visible.
- [Native logout](06-logout.png): clicking the account menu's sign-out returns to the welcome screen and removes the Pro entry.

The shell image shows a loading viewport, **not a rendered Pro page**. Chromium
could not initialize inside the extra bwrap sandbox: the host's bwrap child
AppArmor policy denies capabilities and its SUID fallback fails. No sandbox was
disabled, no host policy changed and no approved executable was replaced.

## Dedicated native browser component with real CEF

The opt-in `native_browser_preview` mode constructs the same production
`NativeBrowserView::new_workspace` as Pro. It loads no Workspace accounts, SSH
config or agent profiles, and therefore needs no bwrap. Its browser profile and
private keyring are disposable. The web app and synthetic issuer run only on
loopback ports 8096 and 8095.

- [Sign-in](07-pro-native-login.png) and [signed-in inbox](08-pro-native-workspace.png): real page paint and native mouse input complete synthetic authorization.
- [CRM](09-pro-native-clients.png) and [keyboard, resize and scrolling](10-pro-native-keyboard-compact.png): text entered through X11/GPUI filters the real web list at 600×600. Ctrl+A and Backspace restore the client result.
- [Client details](11-pro-native-client-detail.png): native mouse input navigates into the real client route; browser Back returns to CRM.
- [Page confirmation](13-pro-native-signout-confirm.png) and [web logout](14-pro-native-web-logout.png): the JavaScript confirmation is rendered by GPUI; accepting it returns to sign-in. Browser Back stays signed out.
- [Minimum-height inbox](15-pro-native-minimum-inbox.png) and [CRM](16-pro-native-minimum-crm.png): after web fix `927a5735d7`, 600×400 leaves scrollable content plus navigation and Manage controls.
- [Short client page](17-pro-native-short-client.png) and [return to CRM](18-pro-native-minimum-return.png): details and the explicit return action work even at 600×320, accounting for extra full-shell chrome.
- [Manage action](19-pro-native-minimum-manage.png): the compact action successfully reaches `/webview/manage`. Its one-time real Manage handoff is intentionally absent from the fixture; the error is **not evidence that the real handoff works**.

[Process evidence](sandbox.json) records the existing approved helper's profile,
child PID namespaces, `NoNewPrivs=1`, `Seccomp=2` and absence of sandbox-bypass
flags. The 0.13.0 development helper uses the same stdio protocol v1 and CEF
bindings as this source; the runtime diff since that tag is the default canvas
background and product-name strings. The ordinary native fixture also passed its
live DOM/paint audit at 1100×656 with the updated preview example.

Reproduce the dedicated component using the existing preview setup, private
Xvfb/compositor/D-Bus and all three disposable XDG homes, with
`SHELLDECK_PREVIEW_PRO_ORIGIN=http://127.0.0.1:8096` and
`SHELLDECK_PREVIEW_SECONDS=1800`. Stage a compatible, sandbox-permitted helper
beside the preview executable using the documented browser discovery layout.
The preview refuses non-loopback origins and rejects combining this mode with
the generic fixture's DOM audit. A fresh private keyring may ask for its initial
password; that is the disposable session's keyring, not the user's login keyring.

## Remaining release checks

A matching 0.25.0 helper built successfully (25.82s) and was staged under
`/tmp/inklura-pro-native-helper-0.25.0`; its required resources, version metadata
and non-setuid file modes were checked. Five offline packaging contract tests
passed. It was not launched because this host's existing user-namespace approval
names only the old development helper path. Running the matching payload needs
a host where that path is already permitted or separately authorized host setup.
Neither the installed app nor the approved helper was replaced.

SDTEST-3173 remains partial for **the full Workspace with a live matching
packaged helper**. macOS and Windows runtime checks are also unverified. The
component evidence above closes the Linux native viewport/input gap, while the
full-shell lifecycle checks remain separate.

Earlier automated evidence is unchanged: the full UI suite passed 454 tests,
with one existing ignored (158.96s); the final dedicated-mode targeted suite
passed SDTEST-3172/3174/3175 (4.79s). This follow-up changes only the preview
example and documentation. Its build, loopback-only guards, generic live audit,
formatting and diff checks pass.
