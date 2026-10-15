# Embedded Chromium browser

ShellDeck renders a real Chromium browser in its native GPUI center pane. A
separate `shelldeck-browser` helper owns CEF and its native event loop. GPUI does
not link CEF or share its `NSApplication`; this also prevents Chromium lifecycle
failures from taking down the terminal UI.

The helper uses tauri-apps `cef` 154.5.0+154.0.34 and the matching CEF prebuilt
runtime. CEF is optional behind the browser crate's `runtime` Cargo feature.
Normal UI/core tests build the lightweight client and protocol only.

## Rendering and control

CEF off-screen rendering produces complete premultiplied BGRA view and popup
frames. A dedicated writer coalesces pending paints to the latest frame per tab
and surface. Frames travel over a bounded binary stdin/stdout protocol; they are
not base64 screenshots and they are not captured on a timer. GPUI uploads the
pixels and handles pointer, scrolling, key, focus and UTF-16 IME input.

On Linux the helper selects Chromium's `--ozone-platform=headless` backend with
software rendering (`--disable-gpu`, `--disable-gpu-compositing`). GPUI continues
to use the user's native Wayland connection. This does not embed an X11 child
window or send input to another application. The default CEF context menu is
suppressed and its page metadata is handed to GPUI; HTML select popups retain
CEF's real OSR popup surface.

A helper owns one explicit absolute profile directory. Tabs share the profile's
request context, cookies and storage; clean shutdown releases browsers before
CEF exits and flushes the profile. ShellDeck profiles are independent of the
user's Chrome/Firefox profile. JavaScript dialogs, uploads and downloads return
correlated requests to the visible UI. Related popup tabs keep CEF’s real
`window.opener` and `postMessage` relationship. A normal tab close waits for
the page’s before-unload confirmation, retaining the tab when canceled. Downloads only start after a destination
is chosen. Camera, microphone, screen capture and browser permission prompts
are denied by this initial runtime.

DevTools commands go through `BrowserHost::execute_dev_tools_method` and a
registered observer. No remote debugging port is opened. Provider access is a
separate, revocable, capability-scoped MCP bridge owned by the host client.
The runtime never starts a provider or makes an agent task implicitly.

## Building and staging

On Linux set `PKG_CONFIG_PATH=/usr/lib/x86_64-linux-gnu/pkgconfig` for Cargo as
usual. Ninja and CMake are required for CEF's C++ wrapper. Set `CEF_PATH` to a
cache outside `target/` to reuse the large matching native runtime:

```sh
CEF_PATH="$HOME/.cache/shelldeck/cef" \
PKG_CONFIG_PATH=/usr/lib/x86_64-linux-gnu/pkgconfig \
cargo build --locked -p shelldeck-browser --features runtime
python3 scripts/bundle-browser-runtime.py --profile debug
```

The bundler reads the exact CEF directory from `cef-dll-sys`'s Cargo build
metadata. `--target TRIPLE`, `--target-dir PATH`, `--cef-dir PATH`, and
`--output PATH` support cross-target packaging and controlled staging. No runtime
is downloaded by the bundler. A `runtime.json` records the packaged application
and binding versions. Each bundle includes the matching CEF BSD notice
(`CEF-LICENSE.txt`) and Chromium third-party credits. The checked-in CEF notice
comes from upstream commit `14c5a08`, matching this pinned CEF runtime.

* Linux: `browser/VERSION/shelldeck-browser`, `libcef.so`, Chromium data/pak files,
  shared libraries and `locales/`. The child gets only its own library search
  path. Chromium's renderer sandbox remains enabled; no root/setuid installation
  step and no `--no-sandbox` fallback are added.
* Windows: `browser/VERSION/shelldeck-browser.exe` is CEF's `bootstrap.exe`, with
  `shelldeck-browser.dll` built from the Rust cdylib and the CEF DLL/resources.
  `RunWinMain` forwards the bootstrap's sandbox context to CEF. The normal Rust
  console executable is not the packaged Windows runtime.
* macOS: `browser/ShellDeck Browser.app/Contents/MacOS/shelldeck-browser` owns its
  CEF-compatible `NSApplication`. `Contents/Frameworks/` holds the CEF framework
  and the Helper, Renderer, GPU, Plugin and Alerts helper app bundles. Framework
  symlinks are preserved. Release signing must sign libraries and nested helper
  bundles before the browser app and outer ShellDeck app; the hardened-runtime
  CEF entitlements belong to these helper executables.

Linux and Windows versioned payloads let an existing helper finish against its
original executable, DLL and resource paths while a new app version is installed.

## Verification

The offline browser tests pin transport bounds, scoped provider permissions,
frame coalescing and logical-to-physical viewport limits. The optional live
fixture runs a bundled helper against a loopback server and a disposable profile:

```sh
python3 scripts/test-browser-runtime.py target/debug/browser/0.10.0/shelldeck-browser
```

SDTEST-2552 covers real BGRA paints, mouse/Unicode IME input, an HTML select
popup, dialog acknowledgement, before-unload cancel/accept, upload selection,
downloaded bytes, new-window
routing with an opener/postMessage round trip, native context-menu metadata,
DevTools screenshot capture, HiDPI resizing, persisted cookies/storage
across helper restart, and clean shutdown. It prints an artifact directory with
its fixture screenshot, event trace, stderr and Chromium log. It never launches
the production ShellDeck app or uses a real browser profile.

Linux helper verification passed on 2026-10-15 with CEF's headless Ozone backend
and the renderer sandbox enabled. Native GPUI/Wayland viewport, DOM and rendering integration also passed. macOS and
Windows builds are tracked in the normal native UI/release matrix. A Linux
helper smoke alone is not evidence for the other two platforms.

Primary implementation references: [CEF off-screen rendering](https://github.com/chromiumembedded/cef/blob/master/docs/general_usage.md#off-screen-rendering),
[cef-rs OSR example](https://github.com/tauri-apps/cef-rs/tree/dev/examples/osr),
[cef-rs Windows bootstrap example](https://github.com/tauri-apps/cef-rs/blob/dev/examples/cefsimple/src/win.rs),
[CEF BrowserHost input and DevTools API](https://github.com/chromiumembedded/cef/blob/master/include/cef_browser.h).

The client/MCP end-to-end fixture (SDTEST-2555) goes through a real scoped grant
and the actual `--mcp` stdio subprocess, using the bundled CEF helper:

```sh
cargo run --locked -p shelldeck-browser --example computer_use_smoke -- \
  target/debug/browser/0.10.0/shelldeck-browser
```

It verifies native PNG and accessibility output; observation-only refusal of
mutating tools; click, Unicode typing, keys, and scrolling against real DOM
assertions; a related popup using an exact decimal string ID; exclusion of an
unshared preexisting tab; and immediate control/grant revocation. The fixture
starts no model or provider and saves only local fixture screenshots/profile.


The provider bridge was also exercised through its real stdio subprocess against
CEF (SDTEST-2555), including read-only refusal, actual Unicode input, Enter form
submission, related popup IDs and immediate revocation. Profiles are additionally
namespaced by the authenticated Manage origin/account, so signing in as a different
account cannot reuse a previous account's browser cookies through a saved session.
[Native preview and verification](testing/native-browser-qa-2026-10-15.md).
