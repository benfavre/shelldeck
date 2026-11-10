# Full Google Chrome preview

Open the Agents browser pane and choose **Google Chrome**. Chrome starts inside
the pane; no separate desktop Chrome window needs to be opened. Use Chrome's own
profile button or **Sign in to Chrome** prompt to connect a Google account once.
Google Password Manager, save/update prompts, autofill and account sync belong to
Google Chrome itself.

This is an opt-in **Linux, manual-browsing preview**. Google Chrome, Python 3,
Xvfb, libX11 and libXtst must be installed. Other platforms retain the built-in
Chromium browser. Choose **Built-in Chromium** to return to that browser.

## Profile and lifetime

The app-owned profile lives under
`<ShellDeck config>/browser-profiles/full-chrome/google-chrome`.
It persists across pane closure and application restart. It is separate from the
regular desktop Chrome profile: existing website cookies are not copied, and
Google account sign-in is required to sync passwords to this new profile.
ShellDeck adds no password database and does not copy or decrypt Chrome vaults.

One wrapper owns this profile at a time. A second Chrome pane reports that the
profile is already open; close the first pane and retry. The wrapper uses a
private authenticated X display, no TCP listener and no Chrome debugging port.
It retains Chrome's browser UI and popup windows in the same live surface.
Chrome and its display are stopped when their owning helper exits.

A whole personal Chrome window includes account settings and password-manager
UI. Existing page-scoped agent grants do not cover that surface, so the preview
does not offer agent observation/control or conversation screenshot attachment.
The built-in Chromium backend keeps its existing scoped browser tools.

## Verification

The opt-in fixture uses only a loopback HTTP server, a disposable profile and
dummy credentials. It does not sign into Google or open an existing Chrome
profile. On Chrome 152 with the fixture's 1000 × 760 viewport:

```sh
python3 scripts/test-chrome-wrapper.py --native --save-at 844 375
```

The Save coordinates refer to the native Chrome password bubble and must be
visually checked when its UI changes. Without `--save-at`, the script verifies
input/session/lifetime behavior and reports password-save coverage as false.
The full check covers Unicode fields, exactly one form submission, native Save,
password autofill after restart, retained cookies, native tab/copy shortcuts,
profile locking, resize and damage-driven idle painting. Google account sync
needs the user's actual sign-in and is reported separately.

Use the existing isolated native UI fixture with
`SHELLDECK_PREVIEW_CHROME=1` to inspect the production GPUI surface. Set all three
XDG homes to a disposable directory as required by that fixture.
