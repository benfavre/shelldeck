# Inklura Desktop - AGENTS.md

> This is the canonical agent instruction file for Inklura Desktop. `CLAUDE.md` at the
> repo root is a thin pointer that imports this file, so Claude Code, Cursor,
> Codex, and any other AGENTS.md-aware tool all read the same source of truth.
>
> Modular topic-specific rules live in [`.agents/`](.agents/) — see the imports
> section at the bottom of this file.

## Project Overview

Inklura Desktop (formerly ShellDeck) is a GPU-accelerated native desktop SSH & Terminal companion app built with Rust. It provides a unified control plane for managing terminal sessions, SSH connections, remote script execution, and port forwarding from a polished sidebar/dashboard UI.

## Product identity

The public product name is **Inklura Desktop** (`shelldeck_core::APP_NAME`).
Use it in windows, menus, notifications, onboarding and installer display names.
The repository/crates/executables, profile directories, keychain services,
autostart registration identity, bundle IDs, `shelldeck://` links, release asset
filenames and signed-manifest domain separator retain their ShellDeck identifiers
for upgrade compatibility. Do not rename those as a display-copy cleanup.
The canonical visual mark is `crates/shelldeck/assets/images/brand/inklura-desktop.svg`.
Regenerate icons, theme badges, tray masks and motion assets with
`python3 scripts/export-inklura-brand.py`; the legacy Monolith export command
forwards to it. Keep the Inklura blue-and-white identity across platforms.
The exporter also writes the committed `packaging/icons/shelldeck.icns`.
macOS packaging must fail if the source or bundled icon is missing; native
Apple icon decoding and ZIP inclusion are checked in CI.
New macOS packages contain `Inklura Desktop.app`; the download installer accepts
both that bundle and legacy `ShellDeck.app` and preserves an existing location.

## Retired Dev tools

The server-sync, detected-site scan and manual port-forward screens are retired
from navigation, menus, palette actions and tunnel deep links. Preserve their
saved records and the SSH/browser transport layers. Do not reintroduce their
entry points when adding navigation. Retained Workspaces belongs under Agents
and does not get a separate rail icon. Manage site context and cloud profile
synchronization remain active features.

## Tech Stack

| Layer | Technology |
|-------|-----------|
| UI Framework | GPUI (via adabraka-gpui fork) |
| Component Library | adabraka-ui (85+ components) |
| SSH | russh (async, tokio-based) |
| Terminal | portable-pty + vte |
| Async Runtime | tokio |
| Config | serde + toml |
| Credentials | keyring (OS keychain) |
| File Watching | notify |

## Workspace Structure

```
shelldeck/
├── Cargo.toml              # Workspace root
├── rust-toolchain.toml     # Nightly required (GPUI dependency)
├── crates/
│   ├── shelldeck/          # Main binary crate (app entry, wiring)
│   ├── shelldeck-core/     # Models, config, SSH config parser, keychain
│   ├── shelldeck-ssh/      # SSH client, sessions, tunnels, remote exec
│   ├── shelldeck-terminal/ # PTY, VTE parser, terminal grid
│   ├── shelldeck-sessiond/ # Session daemon: protocol, client, daemon binary, stdio proxy
│   ├── shelldeck-update/   # Auto-update client and installer
│   ├── shelldeck-browser/  # Native browser helper (CEF), separate process
│   └── shelldeck-ui/       # GPUI views, sidebar, dashboard, forms
```

## Essential Commands

```bash
# Apply lightweight crate patches (zed-xim Latin-1 fix, etc.)
./scripts/apply-crate-patches.sh

# Build (requires nightly + system deps)
cargo build

# Check compilation
PKG_CONFIG_PATH=/usr/lib/x86_64-linux-gnu/pkgconfig cargo check

# Run
cargo run

# Run specific example
cargo run --example hello_world
```

## System Dependencies

- **Rust nightly** (specified in rust-toolchain.toml)
- **OpenSSL dev** (`libssl-dev` on Ubuntu, `openssl-devel` on Fedora)
- **pkg-config**
- On Linux: `libxkbcommon-dev`, `libwayland-dev` (for GPUI), `libasound2-dev` (microphone and speech playback)
- Optional on Linux/X11 (runtime, not build): `xrandr` and `xprop` — the
  desktop companion uses them for multi-monitor bounds and `_NET_WORKAREA`
  window placement; without them it logs a warning and falls back to GPUI
  display bounds (see `.agents/cross-platform.md`)

## Crate Dependencies

### shelldeck-core
Models (Connection, PortForward, Script, ExecutionRecord), SSH config parser, app config (TOML), connection store (JSON), keychain wrapper, config file watcher.

### shelldeck-ssh
SSH client (russh), session management, port forwarding tunnels (local/remote/SOCKS), remote command execution with streaming output, connection pool. russh ≥ 0.63: a channel the other side opens (forwarded-tcpip on the client, every channel in test servers) exists only if the handler calls `reply.accept()` on its `ChannelOpenHandle`; dropping the handle refuses it. `check_server_key` receives a key or a host certificate; a certificate is checked through the key it certifies, against known_hosts. `sessiond.rs` is the remote transport of the session daemon: an exec channel running `shelldeck-sessiond proxy`, handed to the `shelldeck-sessiond` client library (depends on `shelldeck-sessiond`). `sessiond_install.rs` probes for the remote binary, uploads it (fixed scripts, SHA-256 verified, only under `$HOME/.shelldeck`, only with the per-host consent of `[general.session_daemon_hosts]`) and holds the fallback decision table. `sessiond_host.rs` (`RemoteHost`, Unix) owns the one SSH connection per host that carries its daemon panes and a `Supervisor` on top of it.

### shelldeck-terminal
Terminal grid (with scrollback, alt screen buffer, scroll regions), VTE escape sequence parser (full SGR, CSI, OSC support), local PTY spawning, terminal session management. `program_status.rs` is OSC 7501, the Program Status Protocol (Rex, Ghostty, Herdr): a pure parser that follows Ghostty's reference rules, the per-terminal records (`TerminalGrid::program_status`) and the reply to the support query, which goes out through the grid's response channel (so the session daemon answers for its panes). The records are cleared by OSC 133;A and RIS and are not in the snapshot. Never infer a status from screen text.

### shelldeck-sessiond
Session daemon (`docs/design/session-daemon.md`): one PTY per pane owned by a per-user daemon, wire protocol v1, async client library with a headless client emulator, stdio proxy for the SSH transport. The crate is both a library and the `shelldeck-sessiond` binary (`src/bin/shelldeck-sessiond.rs`: `serve`, `proxy`, the internal `exec-pane` and debug commands). Depends on `shelldeck-terminal` (grid, parser, `StreamBoundary`, snapshot), never on GPUI or `shelldeck-ui`. Daemon code is Unix only. `attached.rs` (`LocalSessions`) is the synchronous facade: it starts the local daemon and turns a pane into a `TerminalSession` (`TerminalSession::spawn_attached` in `shelldeck-terminal`). `supervisor.rs` (`Supervisor`) is what the app uses on top of it: sessions returned at once that connect, attach and reattach on their own thread, so nothing waits for the daemon on the GPUI thread. Wired into the app only behind `general.session_daemon` (default off, toggle in Settings, local unscoped terminals, `shelldeck-ui/src/terminal_view/daemon.rs`, and SSH tabs opened without a directory, `shelldeck-ui/src/workspace/remote_daemon.rs` and `workspace/ssh.rs`; the layout is saved by pane id in `workspace.json`); manual QA list in `docs/testing/session-daemon-qa.md`. Tests: `cargo test --locked -p shelldeck-sessiond` (private temp directories only). The supervisor reaches its daemon through a `Connector`: the local resolver by default, or another transport (`Supervisor::with_connector`, used by `shelldeck-ssh` for a daemon on an SSH host).

Pane commands (`docs/pane-cli.md`): the daemon binary also carries `ls`, `run`, `send`, `read`, `wait` (plus `attach` and `kill`) for scripts and agents. They are a client of the daemon like the app (`client.rs`, a `PaneMirror` for the screen), in `mod panes` of the binary; pane selection and the JSON list are in the library (`pane_cli.rs`). `send`, `read` and `wait` attach with a 0 x 0 size so a live pane is never resized, `send` wraps several lines in a bracketed paste when the pane's program enabled it, `wait --until STATE` follows OSC 7501 reports in the mirror's grid, and only `run` starts a daemon. They add no frame to the wire protocol. Since SDUC-581: `split`, `send-key`, `status`, `events` and `skill setup` too. The daemon keeps the status a program reported and serves it through `ListStatus`/`StatusList`, the one addition to the wire protocol (feature bit `FEATURE_PANE_STATUS`; never send `ListStatus` to a daemon without it). Every pane gets `SHELLDECK_PANE`, `SHELLDECK_SESSIOND` and the binary's directory at the end of `PATH` (`ServerConfig::pane_identity_env`). `split` asks the application through a file in `<run dir>/requests` (`requests.rs`); the app reads it on its own thread and places the pane in `TerminalView::adopt_daemon_split` (`terminal_view/daemon.rs`), without moving the focus. Since SDUC-582 `focus`, `rename`, `zoom` and `unzoom` are requests of the same kind (`requests::Request::Act`, done in `TerminalView::act_on_daemon_pane`), and `send`/`send-key` refuse a pane whose status is `blocked` on a permission unless `--force`.

Daemon internals worth knowing before touching `server/`: the run directory is `~/.shelldeck/run/<name>` where the name comes from the machine identity, not the host name (`paths::run_dir_name`; a live daemon in a sibling directory is still found); the instance lock is released as soon as the socket is unlinked, and a start that finds the lock taken without a socket waits for it (`ServerConfig::lock_wait`); an idle daemon renames its socket aside and reads the listener once more before leaving (`Server::leave_if_unwanted`); the daemon raises its own `RLIMIT_NOFILE` and starts every pane through `shelldeck-sessiond exec-pane`, which restores the original limit (`server/launch.rs`); `server/log.rs` writes `<run dir>/sessiond.log` (1 MiB, one rotated generation, lifecycle events only, never terminal content); the history of all panes is kept under `ServerConfig::history_budget` (`serve --history-budget`). A daemon left by another version of the app is used when it speaks the protocol and otherwise left alone (`AttachError::OtherVersion`, `AttachedPhase::OtherVersion`): never stop or replace a running daemon from the app.

Packaging: `scripts/build-release.sh` builds `shelldeck-sessiond` and `scripts/package-release.sh` puts it in the tar.gz beside `shelldeck`, which is where the app (`terminal_view/daemon.rs`), the install script (`cloudflare/update-worker`) and the updater (`shelldeck-update::installer::SessiondPayload`) expect it. The daemon binary is built with `--no-default-features`: the `app` feature of `shelldeck-sessiond` and `shelldeck-terminal` (default on, used by the app crates) is what links `shelldeck-core`, and the binary must stay free of it (no C code, or the static builds stop linking). The static musl builds for SSH servers (`shelldeck-sessiond-<triple>`, same directory) come from `scripts/build-sessiond-remote.sh`, which `build-release.sh` runs on Linux unless `SHELLDECK_SESSIOND_REMOTE=0`; it needs `rustup target add x86_64-unknown-linux-musl aarch64-unknown-linux-musl` and links with the toolchain's `rust-lld` (no musl-gcc, no cross compiler). The AppImage and the macOS bundle carry the daemon beside the binary too, but the app never runs it from there: `crates/shelldeck-ui/src/sessiond_files.rs` copies it to `sessiond/` in the configuration directory first (an AppImage is a mount that goes away, a bundle gets replaced), and `paths::appimage_env_changes` keeps the image out of the daemon's environment. An install without the daemon (updated by 0.20.0 or older) fetches the files from the small signed archive `shelldeck-sessiond-linux-x86_64.tar.gz` that every release publishes under the manifest key `linux-x86_64-sessiond` (`shelldeck-update/src/sessiond.rs`). The Windows archive does not carry the daemon. Contract tests: `python3 scripts/test-sessiond-packaging.py`, plus SDTEST-3064 in `test-agents-packaging.py` and SDTEST-3065 in `test-macos-branding.py`.

### shelldeck-ui
GPUI views: Workspace layout, Sidebar (connection tree), Dashboard (stats cards, activity feed), Terminal view (grid renderer, tabs), Port Forward view (table + visual map), Script Editor, Settings, Status Bar, Connection Form.

### shelldeck (main)
App entry point, GPUI Application setup, theme initialization, keyboard shortcuts, app state management.

## Key Patterns

### GPUI Views
```rust
use gpui::*;
use gpui::prelude::*;

struct MyView { /* state */ }

impl Render for MyView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div().flex().size_full().child("Hello")
    }
}
```

### adabraka-ui Components
```rust
use adabraka_ui::prelude::*;

Button::new("id", "Label").variant(ButtonVariant::Default)
```

### Conditional Elements
Use Rust if/else instead of `.when()` chains:
```rust
let mut el = div().flex();
if condition {
    el = el.child(something);
} else {
    el = el.child(other);
}
```

## Config Locations

- App config: `~/.local/share/ShellDeck/shelldeck.toml`
- Connection store: `~/.local/share/ShellDeck/connections.json`
- SSH config (read-only): `~/.ssh/config`

## Cloud Sync (Inklura Manage)

`shelldeck-core::config::cloud_sync` pulls SSH connection profiles from the Inklura Manage portal (`manage.inklura.fr`) into the connection store. `sync_now()` does a device check-in (`POST /api/manage/shelldeck/sync`, falling back to `GET` on 404/405), then `merge_profiles()` upserts by UUID as `ConnectionSource::CloudSync` and prunes cloud entries that vanished remotely — **never** touching `Manual`/`SshConfig` connections. Config lives in a `[cloud_sync]` section of `shelldeck.toml` (`enabled`/`base_url`/`sync_on_startup`); the bearer is memory-only there and persists in the OS keychain. Older serialized tokens are accepted only for one-time migration and removed on the next save. Startup sync is deferred until `Workspace` is first constructed, then `Workspace::cloud_sync_on_startup` runs the blocking fetch on `background_executor` (best-effort, bounded by 4s/10s timeouts, never the UI thread); the manual path is the `CloudSyncNow` action → `Workspace::cloud_sync_now`. Token is shown masked in Settings → General.

### Account login

`shelldeck-core::config::cloud_account` signs in to Inklura Manage and mints an account-bound sync token. `login_password()` (`POST …/auth {action:"login"}`), `whoami()` (`GET …/auth?action=whoami`, Bearer), and `logout()` (`{action:"logout"}`, best-effort revoke) mirror the sync module's reqwest-blocking + 4s/10s style. The browser/OIDC device flow is std-only: `browser_connect_url()` builds `…/manage/shelldeck/connect?port&state&device[&provider]`, `open_in_browser()` shells out to `xdg-open`/`open`/`start`, and `browser_connect_listen()` runs a loopback `TcpListener` that verifies the `state` echo and returns the redirected token (ignores favicon / mismatches, 180s timeout). `provider=sso|google|github|linkedin` → CM on-host OIDC; **omitting `provider` → the Manage password login page** (round-trips back via `?next=`), surfaced as the modal's "Via le navigateur (mot de passe)" button (`StartOidc(None)`). The signed-in identity persists in `AppConfig.account: Option<AccountInfo>` (`[account]`, `skip_serializing_if` so it's absent when logged out), while the token persists only in the OS keychain. UI: a titlebar account chip (`Workspace::render_account_menu`, mirrors the theme dropdown) with a status dot; the `LoginForm` modal (`login_form.rs`, mirrors `connection_form`) captures email/password + OIDC buttons; `Workspace::{show_login_form,start_password_login,start_oidc_login,apply_login,logout_account,check_account_on_startup}` drive the flows (all network on `background_executor`). On login, `apply_login` enables cloud_sync, stores the token in the keychain, then syncs and toasts the profile count. **Config sync:** `Workspace::app_config` is authoritative for session fields; refresh `SettingsView`'s snapshot via `sync_settings_config` after login/logout/401 — never `app_config = config.clone()` from `ConfigChanged` ([`.agents/session-state.md`](.agents/session-state.md)).

### Site switcher

`shelldeck-core::config::manage_sites` — `fetch_sites()` (`GET …/sites`, Bearer) returns `SitesPayload { manage_origin, sites: Vec<ManagedSiteInfo>, areas: Vec<ManageArea> }`; `manage_area_url(origin, site, area_path)` builds the `…/api/manage/switch?tenantId&siteId&host&label&next` browser deep link (opened via `open_in_browser`). Cloud-synced `Connection`/`RemoteProfile` gained `site_id: Option<Uuid>` + `site_label` (merged like other managed fields); the active site persists in `CloudSyncConfig.active_site_id/active_site_label` (both `#[serde(default)]`). UI: a titlebar site chip + `Workspace::render_site_menu` dropdown (active pinned, connection-bearing next, capped at 20 — no in-dropdown text filter, GPUI limitation) lists sites and, for the active site, the manage-area links; `SidebarView::set_site_filter` scopes the sidebar (active site + unbound connections) and each row shows a site badge. `refresh_sites` (background, after login/whoami) caches the directory; `select_site`/`open_manage_area`/`open_site_switcher` + the `SwitchSite`/`OpenManageArea` actions drive switching and area links (also surfaced in the command palette, rebuilt by `refresh_command_palette`).

### App modes (User / Support / Dev)

`AppMode` (cloud_account) persists in `CloudSyncConfig.mode` (default `Dev`). `AccountInfo.is_superadmin` unlocks Dev; `AccountInfo.is_inklura_support` unlocks Support; generic Manage admin roles never unlock staff surfaces. Logged-out rendering is intercepted by the mandatory welcome/login screen. `Workspace::effective_mode()` forces regular/customer-admin accounts to User, clamps Inklura Support to User/Support, and honors all three persisted modes for super-admins. The titlebar and palette consume `AppMode::allowed_modes`, and Dev actions/deep links re-check the same capability before executing. `render()` swaps the whole surface by mode — **Dev = today's sidebar+ActiveView (hidden, never destroyed, so terminals survive); User = `render_user_home`; Support = the `SupportView` entity**. User and Support both land on a dedicated Accueil dashboard before their operational tabs. Staff accounts also reach the same `SupportView` from Dev: the rail's Support/Requests group (`SidebarSection::{Support,Requests}` → `ActiveView::Support`, shown when `can_access_mode(Support)`), polled like Support mode while visible (`polling::support_console_on_screen`). The new-request composer opens over the console in both places (« Nouvelle demande » in the Requests header, ticket « Convertir ») without switching to User mode. Settings is a closable personal surface shared by all authenticated modes (General/AI/Appearance/About; Terminal/Editor only for Dev-capable accounts) and preserves the hidden mode surface. Support data: `shelldeck-core::config::manage_support` (`support_list`/`support_ticket`/`support_agents` + `support_{reply,note,status,priority,assign,resolve,read}`, all reqwest-blocking); the view emits `SupportViewEvent`, `Workspace::handle_support_event`/`support_action` run it on `background_executor` + `refresh_support` polls every 30s while Support is active (`sync_support_poll`). ⚠️ the support JSON is loose: `message.from` and most strings can be **null** (→ `de_nullable_string`, unknown `from` renders agent-side) and `lastAt`/`at` are **int OR ISO-8601 string** (→ `de_flex_millis`, chrono-parsed to epoch ms). Composer is single-line-ish (Enter sends, Shift+Enter newline).

### Workspace views (full app, Agents, Terminal, Editor)

`WorkspacePresentation` (`workspace/presentation.rs`) is what the window shows around the Dev views: `Main` is the full app (titlebar menus, activity rail, every destination); `Agents`, `Terminal` and `Editor` are focused views that draw one Dev view with no rail and no menu row. It is transient (never saved) and independent of `AppMode`: a focused view resolves to Dev for an account that holds the Dev capability, whatever mode is saved, and the saved mode comes back with the full app. Entry points, all through `Workspace::set_presentation` (user choice, returning to the full app keeps the view on screen) or `Workspace::request_presentation` (launch intent, waits behind the welcome screen): the titlebar segmented switcher (`render_presentation_switcher`, shown to Dev-capable accounts in every presentation, so the full app is never a one-way door), `CyclePresentation` on `secondary-shift-l` (`shortcut_reference::CYCLE_PRESENTATION_BINDING`), the palette rows `SetWorkspacePresentation`, the View menu (`MenuCommand::NextPresentation`, one row: that dropdown is at its height limit), the flags `--agents` / `--terminal` / `--editor` / `--main` and the links `shelldeck://agents|terminal|editor`. A focused window follows its view (`WorkspacePresentation::after_view_change`): Agents, Terminal and Editor hand over to one another focused, any other destination opens the full Dev shell. The focused Terminal view draws no sidebar, so `Workspace::render` sets `TerminalView::set_full_width` from the same value that hides the sidebar, and pushes `SidebarView::total_width()` to the terminal in the same place: change one without the other and every pane is mis-sized. The switcher segments carry a dot from `Workspace::presentation_attention` (`ViewAttention`): terminals through `TerminalView::attention()` and `TerminalEvent::AttentionChanged` (OSC 7501 status per tab, `TerminalTab::status`), agents through `AgentConsoleView::attention()` and a deduplicating observer. To add a focused view: one enum variant with its `view()`, `section()`, label, icon, window title and switcher id, then its flag and link.

### Keyboard shortcuts (`[keybindings]`)

`shortcut_reference::BindableAction` is the one list of application shortcuts: its `id()` is the key of the user's `[keybindings]` table (`AppConfig.keybindings`, a file format: never rename an id), `default_keys_for` holds the built-in keys. `apply_keybindings` (startup, before `actions::register_keybindings`) resolves the table with `resolve_keybindings`: unknown actions and unparsable keys are skipped, a change onto keys in use is refused, and the problems are shown once by `Workspace::report_keybinding_problems`. 24 actions today. **Every shortcut label goes through `shortcut_reference::label(action)`** (menus via `MenuEntry::shortcut(BindableAction)`, palette, tooltips, toolbar, status bar, the shortcut reference rows): never write `"Ctrl+T"` or `"secondary-t"` next to a label, it would lie after a remap. A new application shortcut is a new `BindableAction` variant, its row in `docs/keybindings.md`, and its `bind(...)` line in `actions.rs`; SDTEST-3100 fails if its built-in keys collide. User doc: `docs/keybindings.md`.

### Monique / Automonique

`shelldeck-core::config::monique` is the native client for Monique's authenticated Automonique dashboard API. `MoniqueConfig{url,user,pass}` comes from a complete local `[monique]` override or `SitesPayload.monique` (Manage delivers it only to super-admin tokens). The client reads `/api/status`, `/api/processes` (jobs: the console's « Tâches » list with per-job output, and a Dev status-bar indicator « Monique N » fed by a 30 s jobs-only poll, `PolledSurface::MoniqueActivity`, which also toasts each job seen in progress that finishes or fails), and `/api/chat/history`, sends conversation turns through `/api/chat`, starts a durable conversation through `/api/chat/new`, and resolves returned Manage actions only through an explicit `/api/chat/action` approve/reject decision. Basic credentials never follow redirects. `MoniqueView` is the Dev console under `ActiveView::MoniqueConsole`; `Workspace::{effective_monique_config,refresh_monique,sync_monique_poll}` owns background I/O and polls only while the console is visible. There is no alternate bot client or runtime fallback.

### Voice for agents (ElevenLabs, the user's own key)

Settings → IA → « Voix (ElevenLabs) » takes the user's ElevenLabs key; it lives only in the OS keychain (`keychain::*_ai_api_key("elevenlabs")`), never in `AppConfig`, a view or logs. `[voice]` in `shelldeck.toml` holds the choices (dictation, read_replies, voice, models, base URL). `shelldeck-core::config::elevenlabs` is the client: `GET /v1/user` (key check), `/v1/voices`, `/v1/models`, `POST /v1/speech-to-text` (Scribe, one WAV per push-to-talk utterance) and `POST /v1/text-to-speech/{voice}/stream?output_format=pcm_24000` (raw PCM, no decoder). `shelldeck-ui::voice` owns audio on dedicated threads (`cpal` input `Recorder` → mono 16 kHz, `rodio` `Speaker`), and `agent_console_view/voice.rs` adds the composer microphone and reads the last agent reply aloud when a run completes. `VoiceKeyStatus::has_key` is false for a keychain error (`Unavailable`), so no mic shows when nothing can work. macOS: `NSMicrophoneUsageDescription` in the bundle and `packaging/macos/app.entitlements.plist` (`device.audio-input`, hardened runtime). Linux builds need `libasound2-dev`. Monique's own voice calls use one installation key on the server (no per-user key, no shared API), so the desktop key is independent.

### Shared platform client

`shelldeck-core::config::platform` wraps `automonique-platform-client`; all resource, session, receipt, attachment and control-lease values come from the shared protocol crate. The native cockpit reads the authenticated HTTPS projection and may send typed attach/detach/claim/release requests. ShellDeck has no provider executor, runtime configuration, heartbeat, claim loop, or handwritten platform wire model. AI Operations owns federated authentication and global actions; Automonique owns its sessions and control leases.

### Inklura notifications (Manage bell)

`shelldeck-core::config::manage_notifications` reads the Manage bell through `GET|POST {base}/api/manage/shelldeck/notifications` (Bearer device token; server in the bext repo, `routes/shelldeck-notifications.ts`). The server merges the shared per-space feed of every site in the token's directory (capped at 30 sites, 100 items), masks prospect details unless the token is super-admin or admin in its own tenant, and never returns raw links: `open_url` asks for a two-minute user-bound signed switch URL at click time. `mark_read(site_id,id)` / `mark_all_read(before_ms)` / `open_url(site_id,id)`; item identity is `(site_id,id)` because feed ids are only unique per space. UI lives in `workspace/inklura_notifications.rs`: `sync_inklura_notifications_poll` (started from `activate_current_mode`, cleared in `invalidate_cloud_session`) polls every 60 s for the whole signed-in session, tray-hidden included, unlike the surface polls in `polling.rs`. `fresh_unread` seeds silently on the first load, then `TrayNotification::Inklura` fires one OS notification per batch when `tray.notify_inklura` is on. The titlebar bell (`render_inklura_bell`, passed into `render_titlebar`) is hidden until a feed arrives, so a Manage that answers 404 shows nothing. The web bell's personal staff inbox (`mobile.inbox.*`) needs a browser session and is deliberately absent.

### Hosted issue management (requests)

`shelldeck-core::config::issues` — per tenant/site request tracker, GitHub-synced + fleet-dispatchable (`GET|POST {base}/api/manage/shelldeck/issues`, Bearer, **snake_case + ISO timestamps** like the fleet API → `de_flex_millis`). `Issue`/`IssueComment`/`IssueList` (+ `IssueInstance` for dispatch). Client fns: `list_issues`/`get_issue`/`create_issue`/`comment_issue` (anyone) + staff-only `set_status`/`assign`/`set_priority`/`dispatch_issue`/`github_push`/`github_refresh` (403 for non-super-admin → surfaced); `delete_issue` is owner-or-staff (the original filer may also delete). `delete_issue` is a **soft delete**: the server stamps `deleted_at` (retained in KV for audit) and hides the row from every read path — the ShellDeck client posts `{action:"delete", id}` and both the Support Requests kebab (staff) and the User-mode detail sheet (own requests) open a destructive confirm modal (`render_delete_issue_modal`, `SupportViewEvent::IssueDelete`/`Workspace::confirm_issue_delete` → `Workspace::delete_issue_now`, which closes the detail pane + refreshes). ⚠️ the server side (`deleted_at` field + `action="delete"` in `lib/issues.ts` / `routes/{shelldeck-issues,issues}.ts`) lives in the `bext` repo (`inklura-manage-prism`) and ships via a **separate PR** — the ShellDeck client will 400 (`unknown action`) until that lands. The Workspace owns the issues cache (`refresh_issues` + 15s `sync_issues_poll` while User/Support visible; `select_issue`/`create_issue_now`/`comment_issue_now`/`issue_staff_action`) and pushes it to `SupportView` via `set_issues`/`set_issue_detail`. UI: User `render_user_requests` ("Mes demandes": create composer + list + expand-to-comment, `IssueField` key-capture); Support `SupportSection::{Tickets,Requests}` tab (`render_requests` list/detail/composer + `render_issue_staff_bar` when `issues_staff`) + a "Convertir en demande" button on tickets (`ConvertToIssue`→`create_issue(source="support")`). Palette `NewRequest`/`OpenSupportRequests`. ⚠️ verification = `issues.rs` mock TcpListener (list/detail parse, create/comment bodies, staff-403, 401) + live list/create/comment/get (no github-push / dispatch — those hit real GitHub / create fleet jobs). ⚠️ throwaway issue `7313c010-816a-41bf-96ea-cb3a4f29ac29` (source=user) left for wipe.

### bext Cloud + single instance

Core clients (committed by the lead, `3d961cf`): `shelldeck-core::config::bext_cloud` (Bearer `bext_…`; `whoami`/`list_sites`/`create_site`/`site_action`/`dashboard`/`list_instances`; `cli_login_url` + `browser_connect_listen`→`CloudConnect` — the cloud CLI flow has **NO state param**, server-side port cookie; `BextCloudConfig` in `AppConfig.bext_cloud`) and `bext_instance` (`BextInstance{base_url,app_id}` + `list_sites`/`get_site`/`create_site`/`go_live`/`config_site`/`destroy_site` over `/__bext/sdk/site/*` with `X-Bext-App-Id`). UI (this round, `bext_cloud_view.rs`, `ActiveView::BextCloud` + Dev sidebar nav): two tabs — **Cloud** (`render_connection_card` connect/whoami/disconnect via `Workspace::connect_bext`/`disconnect_bext`; dashboard strip; sites list + create + go_live/config/destroy-with-confirm via `bext_cloud_action`; super-admin instances) and **Instance** (target base+app-id → `refresh_bext_instance`/`bext_instance_action` list/create/go_live/destroy). `Workspace::refresh_bext_cloud` (whoami+sites+dashboard+instances in one bg spawn) + 15s `sync_bext_poll` while visible. Per-connection **bext** hover button → `SidebarEvent::ConnectionManageBext` → `manage_bext_for_connection` (⚠️ v1 targets `http://127.0.0.1` local loopback; remote-over-SSH-tunnel is the follow-up). Palette `OpenBextCloud`/`ConnectBextCloud`. ⚠️ verification: the clients' mock tests (lead) + live whoami(super_admin)/sites(empty,max 8)/dashboard/admin-instances(1: activ-2) via `--resolve cloud.bext.dev:443:127.0.0.1` + the on-box instance SDK `/__bext/sdk/site/list` (site `testcool`) — no real WordPress site created.

### Inklura Chrome extension setup

`shelldeck-core::config::inklura_extension` installs the browser assistant whose source lives in `bext/extensions/inklura-assistant` and that Manage serves at `/extensions/inklura-assistant.zip`. It downloads and unpacks the package (own CRC-checked zip reader on `flate2`, staging folder, no `..` entries) into `<data dir>/inklura-assistant`, computes the unpacked-extension ID from that path (SHA-256 → `a`–`p`, the Chrome algorithm), launches Chrome/Chromium on `chrome://extensions` (`xdg-open` refuses `chrome://` URLs), and runs the package's `native/install.py` for the optional Craft bridge (macOS/Linux). **Keep the install path stable**: the Chrome ID, permissions and pinned icon depend on it. Chrome does not allow a program to load an unpacked extension for the user, so the « Load unpacked » click stays manual and the Settings → Général section spells it out. UI: `settings/inklura_extension.rs` (state + intents) and `workspace/inklura_extension.rs` (background work, toasts); palette `SetupInkluraExtension`.

### Share with Inklura (OS hand-off)

`shelldeck-core::config::share_intent` turns OS file hand-offs into staged Share drafts. Entry points: `shelldeck --share FILE…` (every remaining argument is a file, resolved against the launching directory), the Windows Explorer « Send to → Inklura Share » shortcut (NSIS), the hidden Linux `inklura-desktop-share.desktop` « Open With » entry that release builds self-register at startup (`$APPIMAGE` path for AppImages), and macOS `CFBundleDocumentTypes` whose `file://` URLs arrive through `on_open_urls`. The single-instance payload is `inklura-share:` + a JSON array of absolute paths. **It is deliberately not a `shelldeck://` URL**: web pages can open deep links, only local processes reach the authenticated loopback channel, so a browser can never pre-stage local files. `Workspace::stage_external_share` switches to User → Share and calls `ShareView::stage_external`, which opens Review; nothing uploads until the user clicks Share. Signed-out hand-offs wait in `pending_share_files` until `apply_login` (cleared on logout); a busy Share view refuses with `share.error.busy`.

Share shortcuts: the tray (« Capturer et partager… », « Partager des fichiers… », signed-in only), the palette (`CaptureAndShare`, `ShareFiles`) and User Home quick actions call `Workspace::capture_and_share` / `share_files_now`. The tray capture selects the region **before** showing the main window (`ShareView::capture_external` runs its `CaptureReady` callback afterwards), so the window is not in the way of the selection.
An opt-in global shortcut (`[companion] global_capture_shortcut_enabled`, default off, `ctrl-alt-s` / `cmd-alt-s`, hotkey id 3) runs the same flow through `capture_and_share` in `main.rs`. `GlobalShortcutRegistrationState::sync` resolves collisions in priority order AI Dock → palette → Capture & share; the later shortcut is unregistered and reported as `Conflict`.

## Releasing & Auto-Update

- **Release command:** Claude Code / Cursor use `/release [patch|minor|major|status|check|monitor]`; Codex uses `$release` with the same arguments. Both drive `scripts/release.sh` (bump, tag, push, optional CI monitor). Specs: `.claude/commands/release.md` and `.agents/skills/release/SKILL.md`.
- **Cut a release:** bump `[workspace.package] version` in the root `Cargo.toml`, commit, then push a matching `vX.Y.Z` git tag. The tag triggers `.github/workflows/release.yml`: it builds linux/macos/windows, creates the GitHub Release with assets, and publishes the update manifest to Cloudflare KV (key `latest-release`). If any build fails, the release + manifest jobs are SKIPPED — fix and re-tag.
- **Site:** the product site is <https://shelldeck.bext.dev>, a bext PRISM site that lives in the `bext` repository (`sites/shelldeck`), not here. It reads GitHub Releases for its pages and relays the update API and install scripts below, so public links and install commands use that domain.
- **Update server:** a Cloudflare Worker (`cloudflare/update-worker/`, bound to `shelldeck.1clic.pro`) serves the update API (`/api/releases/latest?platform=…`) and the install scripts (`/install.sh`, `/install.ps1`); its `/` redirects to the site. It holds the signed manifest (KV `latest-release`) and is what installed applications poll (`UPDATE_SERVER` in `crates/shelldeck-update`), so it must stay deployed.
- **Update client:** `crates/shelldeck-update/` polls that API hourly. Platform keys are `{os}-{arch}` and use **`macos-*`, never `darwin-*`** — manifest, workflow, client, and worker must all agree.
- **Worker code deploy:** `release.yml` only updates KV *data*. Worker *code* deploys via `.github/workflows/deploy-worker.yml` on changes to `cloudflare/update-worker/**` — but that needs the `CLOUDFLARE_API_TOKEN` secret to have **Workers Scripts: Edit** permission. Otherwise deploy manually: `cd cloudflare/update-worker && wrangler login && npm run deploy`.

## Critical Rules

- **GitHub : utilise exclusivement le CLI local `gh` pour toutes les lectures et écritures. N'utilise JAMAIS de connecteur, app ou outil MCP GitHub — ne le privilégie jamais par rapport à `gh`. Point.**
- **Branches de correctif : utilise toujours le préfixe `fix/`. N'utilise JAMAIS le préfixe `agent/`.**
- NEVER write to `~/.ssh/config` - ShellDeck-specific data goes in its own config
- Rust nightly is **pinned** in `rust-toolchain.toml` (`nightly-2026-03-06`). Do NOT float it back to `nightly`: a newer nightly drops the `simd_fmin/simd_fmax` intrinsics `pathfinder_simd` needs and breaks the macOS release build (Linux/Windows are unaffected, so `cargo check` won't catch it). Bump only deliberately and verify the macOS release build.
- `pathfinder_simd` is **held at 0.5.5** in `Cargo.lock` to match the pinned nightly. 0.5.6 renamed the intrinsics to `simd_minimum_number_nsz` / `simd_maximum_number_nsz`, which don't exist in our nightly and break the macOS release build. A blind `cargo update` will bump it and re-break macOS — after any `cargo update`, run `cargo update -p pathfinder_simd --precise 0.5.5`. If we ever bump the nightly, retest whether 0.5.6+ works and drop this pin.
- ALWAYS set PKG_CONFIG_PATH on Linux for OpenSSL
- Terminal grid operations must be fast - they're on the rendering hot path
- Use `parking_lot::Mutex` for thread-safe grid access, not `std::sync::Mutex`
- Terminal repaint is event-driven (PTY reader → channel → refresh task), not polled — don't reintroduce a fixed-interval poll. The main grid paint loop must keep using `shape_line`; the `paint_glyph`/`GlyphCache` fast path silently fails to render (breaks bold/colored glyphs).

## Modular rules (`.agents/`)

Topic-scoped instructions live under [`.agents/`](.agents/). Each `.md` file there
covers one focused area (a subsystem, a workflow, a class of bugs). Add a new
file for each new topic — keep them small and self-contained.

Claude Code auto-imports these via the `@`-directives below. Other AGENTS.md
readers (Cursor, Codex, …) that don't support imports will see the list as
prose — they can still open the referenced files manually or via their own
rules loader.

@.agents/cross-platform.md
@.agents/chrome.md
@.agents/window-rounding.md
@.agents/ui-components.md
@.agents/theming.md
@.agents/icons.md
@.agents/overflow.md
@.agents/spacing.md
@.agents/session-state.md
@.agents/workspace-architecture.md
@.agents/roles.md
@.agents/patches.md
@.agents/testing.md
@.agents/i18n.md
@.agents/ai.md
@.agents/ai-mentions.md
@.agents/agent-console.md
@.agents/github.md
@.agents/related-repositories.md

<!-- Add one @-import per rule file above this comment. Example:
@.agents/gpui-patterns.md
@.agents/ssh-safety.md
-->
