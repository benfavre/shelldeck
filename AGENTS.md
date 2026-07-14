# ShellDeck - AGENTS.md

> This is the canonical agent instruction file for ShellDeck. `CLAUDE.md` at the
> repo root is a thin pointer that imports this file, so Claude Code, Cursor,
> Codex, and any other AGENTS.md-aware tool all read the same source of truth.
>
> Modular topic-specific rules live in [`.agents/`](.agents/) — see the imports
> section at the bottom of this file.

## Project Overview

ShellDeck is a GPU-accelerated native desktop SSH & Terminal companion app built with Rust. It provides a unified control plane for managing terminal sessions, SSH connections, remote script execution, and port forwarding from a polished sidebar/dashboard UI.

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
- On Linux: `libxkbcommon-dev`, `libwayland-dev` (for GPUI)

## Crate Dependencies

### shelldeck-core
Models (Connection, PortForward, Script, ExecutionRecord), SSH config parser, app config (TOML), connection store (JSON), keychain wrapper, config file watcher.

### shelldeck-ssh
SSH client (russh), session management, port forwarding tunnels (local/remote/SOCKS), remote command execution with streaming output, connection pool.

### shelldeck-terminal
Terminal grid (with scrollback, alt screen buffer, scroll regions), VTE escape sequence parser (full SGR, CSI, OSC support), local PTY spawning, terminal session management.

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

`shelldeck-core::config::cloud_sync` pulls SSH connection profiles from the Inklura Manage portal (`manage.inklura.fr`) into the connection store. `sync_now()` does a device check-in (`POST /api/manage/shelldeck/sync`, falling back to `GET` on 404/405), then `merge_profiles()` upserts by UUID as `ConnectionSource::CloudSync` and prunes cloud entries that vanished remotely — **never** touching `Manual`/`SshConfig` connections. Config lives in a `[cloud_sync]` section of `shelldeck.toml` (`enabled`/`base_url`/`token`/`sync_on_startup`); `AppConfig.cloud_sync` is `#[serde(default)]` so older configs still parse. Startup sync runs in `main.rs` (best-effort, bounded by 4s/10s timeouts); the manual path is the `CloudSyncNow` action → `Workspace::cloud_sync_now` (runs the blocking fetch on `background_executor`, never the UI thread). Token is shown masked in Settings → General.

### Account login

`shelldeck-core::config::cloud_account` signs in to Inklura Manage and mints an account-bound sync token. `login_password()` (`POST …/auth {action:"login"}`), `whoami()` (`GET …/auth?action=whoami`, Bearer), and `logout()` (`{action:"logout"}`, best-effort revoke) mirror the sync module's reqwest-blocking + 4s/10s style. The browser/OIDC device flow is std-only: `browser_connect_url()` builds `…/manage/shelldeck/connect?port&state&device[&provider]`, `open_in_browser()` shells out to `xdg-open`/`open`/`start`, and `browser_connect_listen()` runs a loopback `TcpListener` that verifies the `state` echo and returns the redirected token (ignores favicon / mismatches, 180s timeout). `provider=sso|google|github|linkedin` → CM on-host OIDC; **omitting `provider` → the Manage password login page** (round-trips back via `?next=`), surfaced as the modal's "Via le navigateur (mot de passe)" button (`StartOidc(None)`). The signed-in identity persists in `AppConfig.account: Option<AccountInfo>` (`[account]`, `skip_serializing_if` so it's absent when logged out). UI: a titlebar account chip (`Workspace::render_account_menu`, mirrors the theme dropdown) with a status dot; the `LoginForm` modal (`login_form.rs`, mirrors `connection_form`) captures email/password + OIDC buttons; `Workspace::{show_login_form,start_password_login,start_oidc_login,apply_login,logout_account,check_account_on_startup}` drive the flows (all network on `background_executor`). On login, `apply_login` enables cloud_sync + saves the token, then syncs and toasts the profile count. **Config sync:** `Workspace::app_config` is authoritative for session fields; refresh `SettingsView`'s snapshot via `sync_settings_config` after login/logout/401 — never `app_config = config.clone()` from `ConfigChanged` ([`.agents/session-state.md`](.agents/session-state.md)).

### Site switcher

`shelldeck-core::config::manage_sites` — `fetch_sites()` (`GET …/sites`, Bearer) returns `SitesPayload { manage_origin, sites: Vec<ManagedSiteInfo>, areas: Vec<ManageArea> }`; `manage_area_url(origin, site, area_path)` builds the `…/api/manage/switch?tenantId&siteId&host&label&next` browser deep link (opened via `open_in_browser`). Cloud-synced `Connection`/`RemoteProfile` gained `site_id: Option<Uuid>` + `site_label` (merged like other managed fields); the active site persists in `CloudSyncConfig.active_site_id/active_site_label` (both `#[serde(default)]`). UI: a titlebar site chip + `Workspace::render_site_menu` dropdown (active pinned, connection-bearing next, capped at 20 — no in-dropdown text filter, GPUI limitation) lists sites and, for the active site, the manage-area links; `SidebarView::set_site_filter` scopes the sidebar (active site + unbound connections) and each row shows a site badge. `refresh_sites` (background, after login/whoami) caches the directory; `select_site`/`open_manage_area`/`open_site_switcher` + the `SwitchSite`/`OpenManageArea` actions drive switching and area links (also surfaced in the command palette, rebuilt by `refresh_command_palette`).

### App modes (User / Support / Dev)

`AppMode` (cloud_account) persists in `CloudSyncConfig.mode` (default `Dev`); `AccountInfo.is_superadmin` (from whoami/login top-level, `#[serde(default)]`) is the only role signal. `Workspace::effective_mode()`: logged-out → Dev (classic); super-admin → persisted mode; non-super-admin → forced User. Only `can_switch_mode()` (signed-in super-admin) shows the titlebar three-segment switcher (`SetAppMode` action + palette "Mode : …" entries). `render()` swaps the whole surface by mode — **Dev = today's sidebar+ActiveView (hidden, never destroyed, so terminals survive); User = `render_user_home` (account header + Mes sites with Activer + per-site `open_area_for_site` deep links); Support = the `SupportView` entity**. Support data: `shelldeck-core::config::manage_support` (`support_list`/`support_ticket`/`support_agents` + `support_{reply,note,status,priority,assign,resolve,read}`, all reqwest-blocking); the view emits `SupportViewEvent`, `Workspace::handle_support_event`/`support_action` run it on `background_executor` + `refresh_support` polls every 30s while Support is active (`sync_support_poll`). ⚠️ the support JSON is loose: `message.from` and most strings can be **null** (→ `de_nullable_string`, unknown `from` renders agent-side) and `lastAt`/`at` are **int OR ISO-8601 string** (→ `de_flex_millis`, chrono-parsed to epoch ms). Composer is single-line-ish (Enter sends, Shift+Enter newline).

### JeanClaude

`shelldeck-core::config::jeanclaude` is a native client for Ben's `#jean` Slack ticket bot (`slack-claude-bot`), replacing its web dashboard. Shapes are derived from the bot source (`src/dashboard.ts` routes, `getState()` in `index.ts`, `registry.ts` Ticket/Ignored, `memory.ts`, targets `{suffixes,mappings}` of `{sshHost,note}`). `JeanConfig{url,user,pass}` + Basic-auth client fns: `get_{state,history,ticket,targets,memory,slack_history}` + `{confirm,reject,cancel,force_ticket,set_paused,set_concurrency,say,add_target,remove_target,add_memory,remove_memory}`. Config precedence: a local `[jeanclaude]` in `shelldeck.toml` wins, else `SitesPayload.jeanclaude` (server delivers it ONLY to super-admin tokens); `Workspace::effective_jean_config()`/`has_jean()`. UI: Dev `JeanView` (`jean_view.rs`, tabs Aperçu/Historique/Cibles/Mémoire, `ActiveView::JeanConsole` + conditional sidebar nav) fed by the Workspace (`handle_jean_event`/`jean_action` on `background_executor`, `refresh_jean_state` + 10s `sync_jean_poll` while a Jean surface is visible); Support strip + "Envoyer à Jean" (`SupportView::set_jean_brief` + `JeanConfirm`/`JeanReject`/`SendToJean` events); User `render_jean_ask_card` (`/api/say` with `[via ShellDeck — <name>]` prefix + recent activity). Actions `OpenJeanConsole`/`JeanTogglePause` (+ palette). ⚠️ No live instance on this box — the verification is `jeanclaude.rs`'s std-`TcpListener` mock (validates the Basic header, canned fixtures per route, 401 path). ⚠️ Jean timestamps are epoch-ms numbers (unlike the support API's ISO strings).

### Jean fleet runtime

`shelldeck-core::config::jean_fleet` makes ShellDeck a runtime for the tenant/site-aware Jean fleet (`GET|POST {base}/api/manage/shelldeck/fleet`, Bearer). Types `JeanInstance`/`JeanJob`/`FleetSnapshot` (⚠️ **snake_case** unlike the camelCase jeanclaude/support APIs; timestamps `created_at`/`updated_at`/`last_seen_at` come back as **ISO-8601 strings** → `de_flex_millis`). Client fns `get_fleet`/`register`/`heartbeat`/`claim`/`update_job`/`dispatch`. Execution goes through the `JobExecutor` trait — `ClaudeExecutor` mirrors the bot's `claude.ts` exactly (`claude -p --output-format stream-json --verbose --permission-mode acceptEdits [--model]`, prompt on stdin, drops `ANTHROPIC_API_KEY`, keeps `CLAUDE_CODE_OAUTH_TOKEN`, cwd=workdir, 30-min SIGKILL, parses the final `result` event) — so `runtime_tick`/`execute_job` are unit-tested with a **fake** executor (`claude` never runs in tests). `[jean_runtime]` config (`enabled` default **false**, `instance_id` persisted after first register, `workdir`/`name`). **SAFETY**: `Workspace::sync_runtime_loop` (register→heartbeat→claim→execute, 15s, concurrency 1 via `runtime_busy`) only auto-executes when `enabled && instance.autonomy=="auto"`; `"confirm"` (the register default) leaves the claimed job in `runtime_awaiting` for an explicit Exécuter/Rejeter in `FleetView` (`fleet_view.rs`, `ActiveView::Fleet`). Actions `OpenFleet`/`ToggleJeanRuntime`. ⚠️ verification = `jean_fleet.rs` mock TcpListener + fake executor (register/claim/job flow, auto vs confirm gating, 401) + the live shapes confirmed by curling GET fleet + a throwaway register/heartbeat/claim (no `claude` executed). ⚠️ a throwaway instance `4365eee9-391f-4eae-a11f-494374e1eda6` (tenant fghfg) was left parked offline — safe to wipe.

### Hosted issue management (requests)

`shelldeck-core::config::issues` — per tenant/site request tracker, GitHub-synced + fleet-dispatchable (`GET|POST {base}/api/manage/shelldeck/issues`, Bearer, **snake_case + ISO timestamps** like the fleet API → `de_flex_millis`). `Issue`/`IssueComment`/`IssueList` (+ `IssueInstance` for dispatch). Client fns: `list_issues`/`get_issue`/`create_issue`/`comment_issue` (anyone) + staff-only `set_status`/`assign`/`set_priority`/`dispatch_issue`/`github_push`/`github_refresh` (403 for non-super-admin → surfaced); `delete_issue` is owner-or-staff (the original filer may also delete). `delete_issue` is a **soft delete**: the server stamps `deleted_at` (retained in KV for audit) and hides the row from every read path — the ShellDeck client posts `{action:"delete", id}` and both the Support Requests kebab (staff) and the User-mode detail sheet (own requests) open a destructive confirm modal (`render_delete_issue_modal`, `SupportViewEvent::IssueDelete`/`Workspace::confirm_issue_delete` → `Workspace::delete_issue_now`, which closes the detail pane + refreshes). ⚠️ the server side (`deleted_at` field + `action="delete"` in `lib/issues.ts` / `routes/{shelldeck-issues,issues}.ts`) lives in the `bext` repo (`inklura-manage-prism`) and ships via a **separate PR** — the ShellDeck client will 400 (`unknown action`) until that lands. The Workspace owns the issues cache (`refresh_issues` + 15s `sync_issues_poll` while User/Support visible; `select_issue`/`create_issue_now`/`comment_issue_now`/`issue_staff_action`) and pushes it to `SupportView` via `set_issues`/`set_issue_detail`. UI: User `render_user_requests` ("Mes demandes": create composer + list + expand-to-comment, `IssueField` key-capture); Support `SupportSection::{Tickets,Requests}` tab (`render_requests` list/detail/composer + `render_issue_staff_bar` when `issues_staff`) + a "Convertir en demande" button on tickets (`ConvertToIssue`→`create_issue(source="support")`). Palette `NewRequest`/`OpenSupportRequests`. ⚠️ verification = `issues.rs` mock TcpListener (list/detail parse, create/comment bodies, staff-403, 401) + live list/create/comment/get (no github-push / dispatch — those hit real GitHub / create fleet jobs). ⚠️ throwaway issue `7313c010-816a-41bf-96ea-cb3a4f29ac29` (source=user) left for wipe.

### bext Cloud + single instance

Core clients (committed by the lead, `3d961cf`): `shelldeck-core::config::bext_cloud` (Bearer `bext_…`; `whoami`/`list_sites`/`create_site`/`site_action`/`dashboard`/`list_instances`; `cli_login_url` + `browser_connect_listen`→`CloudConnect` — the cloud CLI flow has **NO state param**, server-side port cookie; `BextCloudConfig` in `AppConfig.bext_cloud`) and `bext_instance` (`BextInstance{base_url,app_id}` + `list_sites`/`get_site`/`create_site`/`go_live`/`config_site`/`destroy_site` over `/__bext/sdk/site/*` with `X-Bext-App-Id`). UI (this round, `bext_cloud_view.rs`, `ActiveView::BextCloud` + Dev sidebar nav): two tabs — **Cloud** (`render_connection_card` connect/whoami/disconnect via `Workspace::connect_bext`/`disconnect_bext`; dashboard strip; sites list + create + go_live/config/destroy-with-confirm via `bext_cloud_action`; super-admin instances) and **Instance** (target base+app-id → `refresh_bext_instance`/`bext_instance_action` list/create/go_live/destroy). `Workspace::refresh_bext_cloud` (whoami+sites+dashboard+instances in one bg spawn) + 15s `sync_bext_poll` while visible. Per-connection **bext** hover button → `SidebarEvent::ConnectionManageBext` → `manage_bext_for_connection` (⚠️ v1 targets `http://127.0.0.1` local loopback; remote-over-SSH-tunnel is the follow-up). Palette `OpenBextCloud`/`ConnectBextCloud`. ⚠️ verification: the clients' mock tests (lead) + live whoami(super_admin)/sites(empty,max 8)/dashboard/admin-instances(1: activ-2) via `--resolve cloud.bext.dev:443:127.0.0.1` + the on-box instance SDK `/__bext/sdk/site/list` (site `testcool`) — no real WordPress site created.

## Releasing & Auto-Update

- **Slash command (Claude Code / Cursor):** `/release [patch|minor|major|status|check|monitor]` — drives `scripts/release.sh` (bump, tag, push, optional CI monitor). Spec: `.claude/commands/release.md`.
- **Cut a release:** bump `[workspace.package] version` in the root `Cargo.toml`, commit, then push a matching `vX.Y.Z` git tag. The tag triggers `.github/workflows/release.yml`: it builds linux/macos/windows, creates the GitHub Release with assets, and publishes the update manifest to Cloudflare KV (key `latest-release`). If any build fails, the release + manifest jobs are SKIPPED — fix and re-tag.
- **Site + update server:** a single Cloudflare Worker (`cloudflare/update-worker/`, bound to `shelldeck.1clic.pro`) serves the landing/marketing page (`/`), the update API (`/api/releases/latest?platform=…`), and install scripts (`/install.sh`, `/install.ps1`).
- **Update client:** `crates/shelldeck-update/` polls that API hourly. Platform keys are `{os}-{arch}` and use **`macos-*`, never `darwin-*`** — manifest, workflow, client, and worker must all agree.
- **Worker code deploy:** `release.yml` only updates KV *data*. Worker *code* deploys via `.github/workflows/deploy-worker.yml` on changes to `cloudflare/update-worker/**` — but that needs the `CLOUDFLARE_API_TOKEN` secret to have **Workers Scripts: Edit** permission. Otherwise deploy manually: `cd cloudflare/update-worker && wrangler login && npm run deploy`.

## Critical Rules

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
@.agents/ui-components.md
@.agents/theming.md
@.agents/icons.md
@.agents/overflow.md
@.agents/spacing.md
@.agents/session-state.md
@.agents/roles.md
@.agents/patches.md
@.agents/testing.md
@.agents/i18n.md

<!-- Add one @-import per rule file above this comment. Example:
@.agents/gpui-patterns.md
@.agents/ssh-safety.md
-->

