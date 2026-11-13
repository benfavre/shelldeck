# Dedicated Inklura Pro web workspace

A signed-in account opens **Inklura Pro** from the titlebar and returns to its
retained desktop tools with **Back to tools**. This is an integration preview;
the selected production origin is `https://pro.inklura.fr`. Its deployment and
dedicated web OAuth registration are separate from this desktop change; the
desktop address stays empty until explicitly configured.

The first visit asks for the administrator-provided Pro web origin, saved in
`general.pro_web_url`. HTTPS is required, except loopback development servers.
Only an origin is accepted, without credentials, query parameters or fragments.
The synthetic local demo uses `http://127.0.0.1:8096`.

Pro uses `NativeBrowserView` and the normal packaged CEF helper, with its own
profile derived from Manage origin, desktop account and Pro origin. It is never
granted to an agent. Its dedicated browser mode refuses the shared Chrome
engine and agent observation/control grants, and hides agent-only screenshot
sharing controls. Logout, shutdown and account replacement close Pro; a
same-account identity refresh keeps it open. A missing account email refuses
browser creation instead of sharing an anonymous profile. Switching back to
tools retains its browser. Desktop bearer tokens are never injected into
the page. Pro has independent OIDC authentication and workspace selection.

Native tool, presentation and mode navigation returns to the requested surface.
Agents replies remain unread while Pro covers their conversation. The native
site selector and contextual sidebar/assistant controls are hidden while Pro
is shown, because their scope belongs to the retained tools. On narrow windows,
shortcuts scroll separately from the always-visible window controls. The address
form scrolls at short window heights, supports Enter to submit, and retains edits
when switching away without submitting them.

Source: `workspace/pro_web.rs` (surface/lifecycle), `config/pro_web.rs` (origin
validation and profile identity). The optional address defaults to empty in
`GeneralConfig`. Native User/Support/Dev authorization remains authoritative.

Web app source is prepared in the company-manager worktree
`/home/pc1/dev/inklura-pro-web-integration/apps/app-inklura-pro`; its `README.web.md` documents
building, the same-origin gateway, browser capabilities and the synthetic CEF
test (local review commit `8db7365ced`). The deployed web version owns those capabilities and permission checks;
the desktop host does not change them.

Verified on Linux: the lifecycle/navigation/form revision passed the full UI
suite (454 tests passed, one existing ignored, 158.96 seconds). This covers
central logout, missing account identity, same-account refresh, replacement,
hidden-Agent unread state, and rendered controls at 600×400 through 1280×400,
French/English and 10/14/20px UI fonts. Core origin/account isolation passed in
the initial integration. After the dedicated browser mode change, all three
final targeted tests (SDTEST-3172/3174/3175) passed in 4.79 seconds; the native
Linux executable built successfully. The independent CEF fixture verifies the
web flow, separately from this native host.

The native GPU walkthrough uses private Xvfb with a compositor, disposable XDG
directories, a private D-Bus session and synthetic Manage data. The test-only
bwrap mount hides the real SSH, Codex and Claude profiles; HOME stays unchanged.
Native shell rendering, Pro entry, return to tools, the 600×400 address form,
Enter-key validation and actual account-menu logout passed. Screenshots are in
[the Linux visual evidence directory](testing/screenshots/2026-11-13-pro-workspace/README.md).
Chromium refuses to initialize its sandbox inside that additional test sandbox
(the SUID-helper fallback reports incorrect owner/mode). No sandbox rule was
changed and no sandbox was disabled. Thus this walkthrough does not prove the
combined native viewport plus embedded CEF rendering/input path. The local QA
wrapper invoked the existing approved 0.13.0 helper; a matching 0.25.0 packaged
helper still needs release-package validation.

SDUC-586 and SDTEST-3171/3172/3174/3175 record the contracts. SDTEST-3173 keeps
the combined native/CEF visual gate open. macOS/Windows runtime QA, matching
release browser packaging and production hosting/OAuth verification remain
before release.
