# ShellDeck — Use Cases (SDUC catalogue)

> Every externally-observable behaviour ShellDeck ships has an
> `SDUC-NNN` ID here. IDs are **sticky**: once allocated, never
> re-used. See [`.agents/testing.md`](../../.agents/testing.md) for
> the rules that govern this file and how it maps to `SDTEST-NNN`
> entries in the per-crate inventories.

Legend (in the per-crate SDTEST tables, not here):
- **Green** — covered by an existing passing test.
- **Yellow** — partially covered / weak assertion / needs adaptation.
- **Red** — not covered; test to write. **P0** blocks release,
  **P1** current cycle, **P2** nice to have.
- **Retired** — behaviour removed on purpose (kept for ID stability).

---

## 1. Local terminal

`crates/shelldeck-terminal/`

### SDUC-001 — Grid stores and advances printable text

Writing printable bytes into the parser produces glyphs in the grid at
the expected cell, advances the cursor left-to-right, and wraps at the
right edge when auto-wrap is on. Combining characters attach to the
previous cell; wide characters occupy two cells.

### SDUC-002 — Control chars behave per VT100

`\r` returns the cursor to column 0. `\n` moves down one row (and
scrolls at the bottom, accumulating scrollback). `\b` moves the cursor
back but never wraps past column 0. `\t` advances to the next
eight-column tab stop.

### SDUC-003 — SGR attributes render styled text

The parser recognises the full SGR family: single attribute, multiple
attributes in one CSI, named 8-colour, 256-colour indexed, truecolour
24-bit, curly / colon sub-parameter underlines, and the "reset"
sequence. An empty SGR resets attributes.

### SDUC-004 — Cursor movement CSI (`CUP`, `CUF/CUB/CUU/CUD`, `CHA`)

Absolute positioning is 1-indexed and clamps to bounds. Relative
movement clamps to bounds. `CHA` sets the absolute column only.

### SDUC-005 — Erase display / line variants

`ED` modes 0/1/2/3 clear cursor-to-end, start-to-cursor, whole screen,
and scrollback respectively. `EL` variants mirror the behaviour on the
current line. Erases use the current background colour, not "black".

### SDUC-006 — Scroll region and origin mode

Setting a scroll region homes the cursor and bounds subsequent
scrolling. Origin mode makes the cursor row relative to the region.
`RI` (reverse index) scrolls the region down at the top.

### SDUC-007 — Insert / delete lines and characters

`IL`, `DL`, `ICH`, `DCH`, `ECH` behave per VT220: insertions push
content, deletions pull content, erase-chars replace without shifting.

### SDUC-008 — Save / restore cursor

`ESC 7` / `ESC 8` (and CSI `s`/`u`) save and restore cursor position
and attributes.

### SDUC-009 — Alt screen preserves and restores primary

Entering the alt screen isolates the buffer; leaving it restores the
primary buffer bit-for-bit including cursor position.

### SDUC-010 — Scrollback ring buffer

The scrollback ring evicts oldest on overflow. Popping returns the
newest. `set_max_scrollback` shrinks by dropping oldest, keeping the
newest N lines. `pop`/`clear` semantics are correct for the alternate
scroll direction.

### SDUC-011 — Resize preserves content

Shrinking clamps the cursor; growing reflows soft-wrapped lines back
into the newly-available columns.

### SDUC-012 — Dirty tracking

Cells and lines mark dirty when written; `take_dirty` clears the
signal so the renderer paints only changed regions.

### SDUC-013 — Selection produces textual content

A mouse-drag selection membership check is inclusive at the anchor,
inclusive at the focus, and the extracted text preserves whitespace
and line breaks correctly across wraps.

### SDUC-014 — OSC sequences (title, palette, prompt marker)

`OSC 0/1/2` set the window title (BEL- or ST-terminated). `OSC 4`
overrides a palette entry. `OSC 133` prompt markers are recognised for
shell-integration features (jump-to-prompt).

### SDUC-015 — Charset switching (DEC special graphics)

`ESC ( 0` switches to the DEC special graphics charset and printable
bytes are translated to line-drawing glyphs until switched back.

### SDUC-016 — Bracketed paste mode

CSI `?2004h/l` toggles bracketed paste; pastes are wrapped in the
expected control sequences when the mode is on.

### SDUC-017 — Cursor visibility mode

CSI `?25h/l` toggles cursor visibility, observable via a public
`is_cursor_visible()` accessor.

### SDUC-018 — Full reset (`RIS`) and soft reset

`ESC c` (RIS) clears the grid, resets attributes, clears scrollback,
homes the cursor. Soft reset does the subset per VT220.

### SDUC-019 — Cursor position report

`CSI 6n` responds with the current cursor position via the OS-write
channel when one is wired.

### SDUC-020 — Malformed sequences never panic

Truncated or invalid escape sequences are dropped without panicking
and the parser recovers on the next valid byte.

### SDUC-021 — URL & path detection in scrollback

Selecting a screen region detects `http(s)://` URLs (trimming trailing
punctuation) and file paths with optional `:line[:col]` suffixes,
including paths that contain colons.

### SDUC-022 — Local PTY spawn on all platforms

`LocalPty::spawn` on Linux, macOS, and Windows produces a live process
with a writable stdin, readable stdout, correct initial size, and
`is_alive()` transitions to `false` after the child exits.

### SDUC-455 — Local terminals honor the configured default shell with platform-correct fallbacks

`[terminal] default_shell` in `shelldeck.toml`, when set and non-blank, is the
shell spawned for every new local terminal and local split (previously a dead
config field). Without it, the fallback chain is platform-correct: Unix
resolves `$SHELL` then `/bin/bash`; Windows resolves `powershell.exe` when on
`PATH`, then `%COMSPEC%`, then `cmd.exe` — `$SHELL` is deliberately ignored on
Windows (MSYS/Git-Bash leakage). Blank candidates fall through to the next.
The PTY working directory falls back to the cross-platform home directory,
then `"."` — never a hardcoded `/`. `ShellFlavor` detection derives from the
same resolved shell the PTY actually spawns (one source of truth).

### SDUC-023 — Terminal session ties PTY to grid via async pipe

`TerminalSession::spawn_local` boots the PTY, forwards output into the
grid via the parser, and drives repaints via the output-notifier
channel (event-driven, **not** polled).

### SDUC-410 — Terminal launchers follow locally installed AI CLIs

The empty terminal surface always offers the default shell. It checks
`PATH` when the view starts and only adds Claude Code and Codex launchers
when `claude` and `codex` are installed; the live terminal toolbar follows
the same availability rules.

### SDUC-024 — Terminal session resize propagates

`TerminalSession::resize` reshapes the grid *and* the PTY window size
in the same call so downstream apps (`vim`, `htop`) see `SIGWINCH`.

### SDUC-025 — Terminal theme mapping (indexed → RGBA)

Named colours and 256-index colours map to the correct RGBA tuples per
theme (dark, light, pastel, high contrast); foreground vs background
inheritance is applied for `TermColor::Default`.

---

## 2. Local SSH — session, pool, tunnels, known hosts

`crates/shelldeck-ssh/`

### SDUC-040 — Parse SSH `~/.ssh/config`

Reads user's SSH config, honours `Include` directives, resolves
wildcards, strips comment / keyword prefixes, and populates the derived
`Connection` list. Never writes to `~/.ssh/config`.

### SDUC-041 — Parse jump host spec (`ProxyJump`)

Accepts `host`, `user@host`, `user@host:port`, `host:port`, and the
`ssh://` URI form. Trims whitespace. Rejects empty hostnames. Does not
attach an identity file (delegated to the SSH agent).

### SDUC-042 — Keychain read / write per host+user

`store_password`, `get_password`, `delete_password` round-trip via the
OS keychain (`keyring` crate) on Linux (Secret Service), macOS
(Keychain), Windows (Credential Manager). Same for key passphrases
keyed on `key_path`.

### SDUC-043 — Known hosts check and add

`check_known_host` returns `Match`, `Mismatch`, `NotFound`, or
`ReadError` for `~/.ssh/known_hosts` and hashed hostname entries.
`add_known_host` appends the new entry without truncating the file
and never rewrites existing entries.

The `known_hosts` path is resolved from the cross-platform home directory
(never `$HOME`-else-`/root`). When no home resolves, host-key persistence
degrades *explicitly*: `check_known_host` returns `NotFound` and
`add_known_host` skips the write, with a single once-per-process warning —
a non-persistent TOFU, never a write to a fabricated root-level path.

### SDUC-456 — Default SSH key discovery is home-resolved cross-platform

When a connection has no explicit identity file, the client probes
`~/.ssh/id_ed25519`, `~/.ssh/id_rsa`, then `~/.ssh/id_ecdsa` — built with
`PathBuf` joins off the resolved home directory on every platform, probe
order preserved. When no home resolves, the candidate list is empty (never
fabricated root-level `/.ssh/*` paths from an empty `$HOME`). The jump-spec
username fallback resolves the current user from `USER` → `LOGNAME` →
`USERNAME` (Windows covered), keeping `"root"` only as the explicit last
resort.

### SDUC-044 — Open interactive shell channel

`SshSession::open_shell(rows, cols)` returns a channel with initial
window size honoured, readable via `SshChannel::read`, writable via
`write`, resizable via `resize`, and clean EOF handling on `eof()`.

### SDUC-045 — One-shot command execution (`exec`)

`SshSession::exec` runs a command remotely, captures stdout, stderr,
and exit code, and returns a `success()` bit matching the exit code.

### SDUC-046 — Streaming execution

`SshSession::exec_streaming` yields stdout / stderr chunks as they
arrive without buffering the whole output.

### SDUC-047 — Cancellable execution

`SshSession::exec_cancellable` cooperates with a cancellation token so
a long-running remote command is interrupted client-side and the
remote process is signalled where possible.

### SDUC-048 — Connect pool: single active session per Connection

`ConnectionPool::connect` establishes a session and returns its UUID.
Repeated calls for the same Connection reuse the pooled session.
`disconnect(id)` closes it. `disconnect_all` cleans everything up.
`active_count` and `connected_ids` reflect reality.

### SDUC-049 — Local port forward tunnel

`TunnelManager::start_local_forward` binds a local port and forwards
each accepted connection over the SSH session. `check_port_available`
short-circuits if the local port is taken. Bytes-transferred counters
increment for both directions.

### SDUC-050 — Remote port forward tunnel

`TunnelManager::start_remote_forward` requests remote port binding via
the SSH channel and forwards `ForwardedTcpIpEvent`s back to a local
target.

### SDUC-051 — SOCKS forward tunnel

`TunnelManager::start_socks_forward` runs a SOCKS5 server locally that
proxies TCP through the SSH session.

### SDUC-052 — Tunnel lifecycle

`stop()` on a tunnel drains and closes cleanly. `stop_all` walks every
active tunnel. `cleanup` removes stopped entries so `active_count`
matches `tunnels().len()`.

### SDUC-053 — Jump-host session

`SshSession::new_with_jump` connects through a jump host with its own
credentials and window resize; the caller sees the target session as
if the jump were transparent.

### SDUC-054 — SSH event stream

`event_rx()` yields `SshEvent`s (connected, disconnected, forwarded,
error) for the workspace's status bar and toast layer.

---

## 3. Scripts & remote execution

`crates/shelldeck-core/src/models/{script,script_runner,execution,templates}.rs`

### SDUC-060 — Script variables: extraction

`extract_variables(body)` finds every `{{name}}` (with optional
`{{name:default}}`), de-duplicates, preserves declaration order,
ignores escaped braces and code fences.

### SDUC-061 — Script variables: substitution

`substitute_variables(body, values)` replaces every placeholder with
the caller-provided value; missing values fall back to the inline
default (`{{name:default}}`) when present. **When neither a value nor a
default exists, the placeholder is left unchanged in the output** — not
replaced by empty. Downstream code relies on this to detect
missing-prompt cases and re-prompt or error out. Extra `values`
entries are ignored. Malformed placeholders (unclosed `{{`) never
panic — the stray brace is emitted verbatim.

### SDUC-062 — Runner spec per language

`ScriptLanguage::runner_spec()` returns the correct interpreter,
argument shape, and file extension per language (bash, sh, python,
node, ruby, php, sql). Custom runners round-trip through
`CustomRunner`.

### SDUC-063 — Package manager detection command

`build_package_manager_detect_command()` produces a shell snippet that
prints the first installed package manager on the remote host.

### SDUC-064 — Dependency install commands

`build_dependency_check_command(deps)` emits a probe. `get_install_command(pm, dep)`
returns the correct install line per package manager (apt, yum, dnf,
apk, brew, pacman, zypper).

### SDUC-065 — Built-in scripts round-trip

`Script::builtin_disk_usage`, `builtin_tail_logs`, `builtin_system_info`
serialise/deserialise identically and produce the expected runner spec.

### SDUC-066 — Script templates catalogue

`all_templates()` returns the shipped template list with unique IDs,
non-empty bodies, at least one variable exposed, and matching
categories. `to_script()` produces a valid `Script`.

### SDUC-067 — Execution record lifecycle

`ExecutionRecord::new` starts running; `append_output` accumulates
text; `finish(exit_code)` transitions to done; `succeeded()` matches
the exit code; `duration_secs()` is `None` while running and
monotonic-positive after finish.

---

## 4. Discovery (remote server inventory)

`crates/shelldeck-core/src/models/discovery.rs`

### SDUC-070 — Parse `stat` output → `FileEntry`

Handles GNU and BSD `stat` shapes, mode bits, size, mtime, symlink
target.

### SDUC-071 — Parse `ls -la` output → file entries

Multi-word owners/groups, weird filenames with spaces, symlink target
extraction, dotfiles.

### SDUC-072 — Parse nginx configs → sites

Extracts `server_name`, `listen`, `root`, SSL flag, and log paths from
a typical `/etc/nginx/sites-*` snippet. Multiple `server_name`
directives yield multiple sites.

### SDUC-073 — MySQL discovery

Parses `SHOW DATABASES` + `information_schema.tables` output into
`DiscoveredDatabase` entries with size totals.

### SDUC-074 — PostgreSQL discovery

Same as MySQL but for `psql -l` and `pg_database_size` output.

### SDUC-075 — rsync command shape

`SyncOptions` produces a well-formed `rsync` argv (dry-run, delete,
exclude patterns, checksum flag, remote-user@host prefix).

### SDUC-076 — Sync operation progress

`SyncProgress::percent` returns a value in **`[0, 100]`** (a
percentage, not a ratio — corrected from initial catalogue) when
`total_bytes` is known; `Some(100.0)` as a safety when `total_bytes = 0`
(guards against 0/0 in the progress bar); `None` when
`total_bytes.is_none()`. Value is clamped to `100.0` even if
`bytes_transferred > total` (rsync sometimes over-reports during
verify).

`SyncOperation::overall_percent` is **size-weighted**, not
item-count-weighted: a 1 GB item at 50% dominates ten 1 KB items at
100% (aggregate stays ~50%, not ~95%). Returns `None` for an empty
operation OR when no item knows its total.

### SDUC-457 — Local discovery and file browsing are shell-free and cross-platform

Local service discovery never spawns `sh -c` (absent on Windows; GNU
`timeout` is not stock on macOS — local discovery used to be silently empty
on both): nginx vhosts are read via `std::fs` into the same `---FILE:` wire
format the SSH command emits (symlinks followed, dotfiles skipped, name
order), and `mysql`/`psql` run as direct argv commands (shared SQL consts, a
real tab instead of the `$'\t'` bash-ism, no empty arg on empty credentials,
nulled stdin, bounded 10 s poll-and-kill timeout). Missing binary, spawn
failure, timeout, and non-zero exit are logged so "tool errored" is
distinguishable from "nothing found"; results stay best-effort.

Local file listing joins paths via `std::path` (drive-letter-correct); on
non-Unix the permission string derives honestly from the readonly flag
(`drw`/`dr-`/`-rw`/`-r-`) and owner/group are empty — never a fabricated
`drwxr-xr-x`/`user`. The Server Sync local pane resolves home via the
cross-platform helper and builds breadcrumbs from `std::path` components
(`C:\Users\ben` round-trips, the root crumb shows the actual root, and the
`..` row disappears at filesystem roots); remote panes keep pure Unix string
math because remote servers are Unix. Remote discovery over SSH is unchanged
(shell command strings verified byte-identical). Known limitation: Windows
local nginx discovery only probes `C:\nginx\conf\sites-enabled` (no packaging
convention exists); absence is logged and honestly reported as no sites.

---

## 5. App config (`shelldeck.toml`)

`crates/shelldeck-core/src/config/app_config.rs` + `store.rs` + `workspace_state.rs`

### SDUC-080 — Round-trip `AppConfig` (non-default values)

All fields serialize back into the same TOML on disk, including nested
sections (`[cloud_sync]`, `[account]`, `[jeanclaude]`,
`[jean_runtime]`).

### SDUC-081 — Backward compat: missing sections still parse

A pre-cloud-sync `shelldeck.toml` with no `[cloud_sync]`, no
`[account]`, no `[jeanclaude]`, no `[jean_runtime]` still parses into
sane defaults (`#[serde(default)]` on every new section is the
contract).

### SDUC-082 — `[account]` omitted when logged out

`AppConfig` serialisation omits the `[account]` table when
`account` is `None` (`skip_serializing_if`), so a logout leaves no
trace in the file.

### SDUC-083 — `[jeanclaude]` overrides survive round-trip and stay absent when unset

Local `[jeanclaude]` overrides the server-delivered Jean config; when
`None`, the section is not written back.

### SDUC-084 — `[jean_runtime]` defaults to disabled

Fresh config has `enabled = false`. Once persisted `enabled = true`,
the round-trip preserves it.

### SDUC-085 — Load-from-missing returns defaults

`AppConfig::load` on a missing path yields defaults; no file is
created until an explicit save.

### SDUC-086 — Load-from-corrupt returns Err

Corrupt TOML surfaces an error rather than silently returning
defaults (dataloss prevention).

### SDUC-087 — Connection store round-trip

`ConnectionStore::load` / `save` round-trip an arbitrary
`Vec<Connection>` with sources, tags, port forwards, and script IDs
preserved.

### SDUC-088 — Connection store missing → empty; corrupt → err

Missing store file yields an empty list; corrupt JSON yields Err
(dataloss prevention).

### SDUC-089 — Workspace state (tabs) round-trip

`WorkspaceState` restores terminal tabs and their titles/PIDs across
restart. Missing state → default (no tabs). Corrupt state → Err.
`clear_at` removes the state file for a clean start.

### SDUC-090 — Config watcher notifies on external edit

`ConfigWatcher` fires the callback when `shelldeck.toml` is edited by
another process (editor, Manage sync). Debounced to coalesce burst
writes.

### SDUC-091 — Atomic write

`atomic_write(path, bytes)` never leaves a partial file on disk:
writes to `path.tmp`, fsyncs, renames. Failure at any step leaves the
prior file untouched. No stale `.tmp` files remain after success.

### SDUC-092 — Themes: builtins & lookup

`TerminalTheme::builtins()` returns the four shipped themes.
`by_name(name)` returns the matching theme, or the dark theme as a
safe fallback for unknown names.

### SDUC-093 — App defaults are stable

Fresh `AppConfig::default()` values (window size, theme, font,
sidebar width) match documented defaults so a user with no config
gets the intended first-run experience.

---

## 6. Cloud sync (Inklura Manage → connection store)

`crates/shelldeck-core/src/config/cloud_sync.rs`

### SDUC-100 — Device check-in via POST, falling back to GET on 404/405

`sync_now()` first tries `POST /api/manage/shelldeck/sync`; on 404 or
405 falls back to `GET`. Any other error surfaces to the caller. The
check-in reports the machine's real hostname on every platform (env
`HOSTNAME`/`COMPUTERNAME`, then the platform lookup); the terminal
fallback is `"ShellDeck"` (changed 2026-08-06 from `"unknown"` — Manage
must not key on the literal `"unknown"`).

### SDUC-101 — Merge: adds new profiles

Cloud profiles absent locally are appended as
`ConnectionSource::CloudSync` connections with the matching UUID.

### SDUC-102 — Merge: updates existing while preserving local-only fields

For a UUID that exists locally as `CloudSync`, cloud fields (hostname,
user, port, tags) are refreshed but local-only fields (last-used
timestamp, port-forward customisations, tag additions) are preserved.

### SDUC-103 — Merge: removes vanished cloud profiles

A cloud profile that stops appearing in the payload is deleted from
the local store on the next sync.

### SDUC-104bis — Connection accessors (display_name / connection_string)

`display_name()` returns a borrowed slice: the `alias` when it is
non-empty, otherwise the `hostname`. There is **no UUID fallback** —
callers must ensure at least one of `alias`/`hostname` is set at
construction (both `new_manual` and cloud-sync paths do). Every
sidebar row consumes this to render its label. `connection_string()`
returns `user@host:port` — the port is **always included**, even
when it is the default 22 (opinionated toward unambiguous strings).

### SDUC-104 — Merge: never touches Manual / SshConfig connections

Connections with `ConnectionSource::Manual` or `ConnectionSource::SshConfig`
are never modified or deleted by cloud sync, even if a UUID collides.

### SDUC-105 — Merge: unparseable IDs are skipped, others still processed

A malformed UUID in the payload is skipped without aborting the merge.

### SDUC-106 — Merge: copies site binding

`site_id` and `site_label` are copied onto the local Connection so the
sidebar site filter (SDUC-125) can scope it.

### SDUC-107 — Merge: no-change report when nothing moves

A merge where the cloud payload matches the local store produces a
no-change signal so the UI does not toast a redundant "synced".

### SDUC-108 — `CloudSyncConfig` back-compat

Config without `active_site_id` / `active_site_label` still parses
(older configs).

### SDUC-109 — `is_configured` semantics

`is_configured()` is true only when `enabled && !token.is_empty()`.

### SDUC-110 — `RemoteProfile` tolerates nulls / missing fields

A payload entry with `null` for optional string fields still parses
(defensive against Manage schema drift).

### SDUC-111 — `SitesPayload` example round-trip

The example payload from the Manage API contract parses into all
`sites` + `areas` + `manage_origin` fields.

### SDUC-112 — Live sync smoke (opt-in)

`SHELLDECK_LIVE=1` — hit real Manage with a test token; assert we get
at least one profile back and the merge produces a stable count.

---

## 7. Manage sites, areas, and site switcher

`crates/shelldeck-core/src/config/manage_sites.rs`

### SDUC-120 — Fetch sites returns `SitesPayload`

`fetch_sites()` GETs `…/sites` with the Bearer token and returns
`SitesPayload { manage_origin, sites, areas, jeanclaude? }`.

### SDUC-121 — Sites payload from contract example

The reference JSON example in AGENTS.md parses without loss.

### SDUC-122 — Manage area URL encoding

`manage_area_url(origin, site, area_path)` builds
`…/api/manage/switch?tenantId=…&siteId=…&host=…&label=…&next=…`
with each param URL-encoded. Empty `host` is handled without producing
`host=`.

### SDUC-123 — Display label fallback

`ManagedSiteInfo::display_label()` prefers `label`, falls back to
`host`, then `tenant.name`, then `siteId`.

### SDUC-124 — Active site persistence

Selecting a site persists `active_site_id`/`active_site_label` into
`CloudSyncConfig`, survives restart, and is exposed via
`Workspace::active_site_*`.

### SDUC-125 — Sidebar filter: active site + unbound

`SidebarView::set_site_filter(Some(uuid))` shows connections bound to
that site *and* connections with no site binding (`site_id.is_none()`).
`None` disables the filter.

### SDUC-126 — Refresh sites is non-blocking

`Workspace::refresh_sites` runs on `background_executor`, never on the
UI thread.

---

## 8. Manage account & authentication

`crates/shelldeck-core/src/config/cloud_account.rs`

### SDUC-140 — Password login

`login_password(email, password)` POSTs `{"action":"login", …}`, returns
`AccountInfo` with `token`, `email`, and `is_superadmin` (defaulted to
false if missing).

### SDUC-141 — Whoami

`whoami(token)` GETs `?action=whoami`, returns `AccountInfo`; label
falls back to email when server-side `label` is missing.

### SDUC-142 — Whoami parses `is_superadmin` from top level

The superadmin flag is at the whoami response top level (not nested)
and defaults to false when absent.

### SDUC-143 — Logout revokes token (best effort)

`logout(token)` POSTs `{"action":"logout"}`; errors are logged but
never surface (the local state clears regardless).

### SDUC-144 — Browser connect URL shape

`browser_connect_url(port, state, device, provider?)` produces
`…/manage/shelldeck/connect?port=…&state=…&device=…[&provider=…]`
with every value percent-encoded. `provider=None` targets the Manage
password page.

### SDUC-145 — Browser connect listener validates `state`

`browser_connect_listen(port, expected_state, timeout)` accepts the
first request whose `state` param matches, ignores favicon and
mismatched states, and returns the token from the redirected URL.

### SDUC-146 — Browser connect listener times out

If no matching request arrives within the timeout, `browser_connect_listen`
returns Err (default 180s per AGENTS.md).

### SDUC-147 — Browser connect percent-decodes token

Tokens delivered with percent-escaped characters are decoded before
storage.

### SDUC-148 — 401 / 403 detection

`is_auth_rejected(err)` returns true for 401 and 403 status codes so
the workspace can transparently trigger re-login.

### SDUC-149 — Provider defaults to Manage password page

`start_password_login` / `start_oidc_login(None)` targets the Manage
web password login (round-trips back via `?next=`).

### SDUC-150 — Provider OIDC branches

`provider = sso | google | github | linkedin` targets the CM on-host
OIDC endpoint.

### SDUC-151 — App mode default is Dev

`AppMode::default()` is `Dev`; `CloudSyncConfig.mode` back-compat →
Dev when the field is absent. This persistence default never bypasses
authentication: a logged-out workspace renders the welcome screen.

### SDUC-152 — Mode enforcement per role

Logged-out users are intercepted by the welcome screen. Authenticated
regular users and customer admins are forced to User mode.
`inklura_support` accounts may switch between User and Support;
super-admins may additionally enter Dev. The titlebar, command palette,
keyboard actions, activity links, and deep links must expose and execute
only operations allowed by that same role matrix.

### SDUC-153 — Login persists identity, enables cloud sync, toasts profile count

`apply_login` writes `[account]`, sets `cloud_sync.enabled = true`,
saves the token, runs a sync, and toasts the number of profiles
merged.

### SDUC-154 — Startup account check refreshes silently

`check_account_on_startup` runs whoami in the background; on 401/403
it clears `account` but leaves cloud_sync config alone.

---

## 9. Manage support

`crates/shelldeck-core/src/config/manage_support.rs`

### SDUC-160 — List tickets

`support_list(token)` GETs `…/support`, returns `SupportList` with
tickets ordered by `lastAt` desc; tolerates null `lastAt`.

### SDUC-161 — Ticket detail messages classification

`support_ticket(token, id)` parses the message list, assigning
user/agent/system origin from `from` (with `null` treated as
agent-side per AGENTS.md).

### SDUC-162 — Ticket detail tolerates nulls

`null` for `message.from` and top-level string fields is accepted
(`de_nullable_string`).

### SDUC-163 — Flex timestamp parsing

`lastAt`, `at`, `createdAt` etc. accept both integer epoch-ms *and*
ISO-8601 strings (`de_flex_millis` chrono-parsed to epoch ms).

### SDUC-164 — Channel glyph fallback

`SupportChannel` returns a fallback glyph when the channel is unknown
so the UI never renders empty.

### SDUC-165 — Agent list

`support_agents(token)` returns the assignable agent list (staff
context).

### SDUC-166 — Reply, note, status, priority, assign, resolve, mark-read

Each write endpoint POSTs the correct body shape and Bearer token.
Non-staff callers get 403 surfaced.

### SDUC-167 — Composer semantics

The support view composer treats Enter as send and Shift+Enter as
newline; the empty body cannot be sent.

### SDUC-168 — Poll while visible

The workspace polls support every 30s only while `ActiveView` is a
support surface — no wasted requests when the user is elsewhere.

### SDUC-169 — Convert ticket to request

`ConvertToIssue` action creates an Issue with `source="support"`,
linking back to the originating ticket ID.

### SDUC-170 — `createdAt` / `created_at` alias parses

Message and ticket timestamps deserialize from both the camelCase
`createdAt` field and the snake_case `created_at` alias (Manage may
send either shape depending on route). Epoch seconds are up-scaled
to milliseconds.

### SDUC-171 — `message.lastAt` alias parses as message timestamp

Older Manage builds emit `lastAt` on individual messages instead of
`at`; both forms accepted. Ensures backward compat with legacy tenants.

### SDUC-172 — `channel_lucide(channel)` maps every documented channel

`SupportTicket::channel_lucide()` returns the Lucide icon slug for
each known channel (`email` → `mail`, `livechat` → `reply`, …).
Unknown channel → `inbox` fallback (safe default, per SDUC-164 for the
glyph variant).

---

## 10. JeanClaude (native #jean bot client)

`crates/shelldeck-core/src/config/jeanclaude.rs`

### SDUC-180 — State: aperçu shape

`get_state()` returns `paused`, `concurrency`, `queue_length`, active
targets — parses the reference fixture from the bot source.

### SDUC-181 — History, ticket detail, targets, memory, slack history

Each read endpoint parses the corresponding fixture without loss.

### SDUC-182 — Write endpoints send correct bodies

`confirm`, `reject`, `cancel`, `force_ticket`, `set_paused`,
`set_concurrency`, `say`, `add_target`, `remove_target`, `add_memory`,
`remove_memory` each POST the expected JSON body.

### SDUC-183 — Basic auth headers

Every request carries `Authorization: Basic base64(user:pass)`;
wrong credentials surface 401.

### SDUC-184 — `is_set` semantics

`JeanConfig::is_set()` is true only when URL, user, and password are
all non-empty.

### SDUC-185 — Config precedence

`Workspace::effective_jean_config()` prefers local `[jeanclaude]` in
`shelldeck.toml` over the server-delivered `SitesPayload.jeanclaude`
(which the server only sends to super-admin tokens).

### SDUC-186 — Timestamp shape (epoch ms numbers)

Jean returns timestamps as epoch-ms **numbers** (unlike Support's
ISO strings). Parsing tolerates both.

### SDUC-187 — Send-to-Jean from support

`SupportView::set_jean_brief` produces a prefilled `say` body that the
"Envoyer à Jean" button submits with the `[via ShellDeck — <name>]`
prefix.

### SDUC-188 — Poll while Jean surface visible

10s poll runs only when `ActiveView::JeanConsole` is active.

---

## 11. Jean fleet runtime

`crates/shelldeck-core/src/config/jean_fleet.rs`

### SDUC-200 — Fleet endpoint uses snake_case + ISO timestamps

`get_fleet()` parses `JeanInstance`, `JeanJob`, `FleetSnapshot` with
snake_case fields (unlike jeanclaude / support) and ISO-string
timestamps (`de_flex_millis` → epoch ms).

### SDUC-201 — Register, heartbeat, claim, update_job, dispatch

Each API call sends the correct route, Bearer token, and body shape.

### SDUC-202 — ClaudeExecutor argv matches slack-claude-bot

`ClaudeExecutor::spawn` invokes
`claude -p --output-format stream-json --verbose --permission-mode acceptEdits [--model …]`
with the prompt on stdin, cwd = workdir, `ANTHROPIC_API_KEY` dropped
from the env, `CLAUDE_CODE_OAUTH_TOKEN` preserved, 30-minute SIGKILL
timeout.

### SDUC-203 — Stream JSON parsing finds `result` event

The final `result` event of a `claude -p` stream-json run is extracted
and reported as the job outcome.

### SDUC-204 — Runtime loop tick (auto mode)

`runtime_tick` in `autonomy = "auto"` mode claims a job and executes
via the fake `JobExecutor` (unit tests never spawn `claude`).

### SDUC-205 — Runtime loop tick (confirm mode)

`runtime_tick` in `autonomy = "confirm"` (the default at register
time) claims a job but leaves it in `runtime_awaiting` for explicit
Exécuter/Rejeter — never executes autonomously.

### SDUC-206 — Runtime disabled by default

`[jean_runtime].enabled` defaults to false; `Workspace::sync_runtime_loop`
is a no-op until the user explicitly enables the runtime.

### SDUC-207 — Concurrency = 1

`runtime_busy` guards the loop; only one job executes at a time per
instance.

### SDUC-208 — Auth failures surface 401

Wrong Bearer token surfaces 401 without silently retrying forever.

### SDUC-458 — Jcode is the fleet executor, with Claude as startup-only fallback

Fleet jobs run through `jcode run --ndjson|--json --quiet --no-update
--no-selfdev -C <workdir> [--provider …] [--model …] [--tool-profile …]
<prompt>` (binary from `[jean_runtime]` config, `$JCODE_BIN`, or `jcode`).
Both NDJSON stream events and a final JSON object parse to the job result;
the workdir is validated before spawn and the child is killed on timeout.
The ACP transport is explicitly disabled by contract today and falls back to
the `jcode run` process transport. Legacy Claude remains available as an
explicit mode and as a *startup-only* fallback when Jcode cannot launch —
never after Jcode has started and returned output. Real subprocesses never
run in unit tests (fake executables only, per `.agents/testing.md`).
*(Back-filled 2026-08-06 — the contract shipped in `b864c32`/`148b975`
without a use-case entry.)*

### SDUC-209 — Instance ID persistence

The first successful `register()` persists `instance_id` into
`[jean_runtime]`; subsequent heartbeats reuse it.

---

## 12. Hosted issue management (requests)

`crates/shelldeck-core/src/config/issues.rs`

### SDUC-220 — List issues (list shape)

`list_issues(token)` parses `IssueList` (snake_case, ISO timestamps →
`de_flex_millis`).

### SDUC-221 — Detail parse

`get_issue(token, id)` parses the full `Issue` including comments and
GitHub linkage fields.

### SDUC-222 — Create issue

`create_issue(token, body)` POSTs the correct shape; supports
`source = "user" | "support"` and an optional `site_id` + `site_label`
target. Untargeted requests omit both site fields so Manage stores `null`.

### SDUC-223 — Comment on issue

`comment_issue(token, id, body)` POSTs the comment; body is required.

### SDUC-224 — Anyone can list / create / comment

The regular-user token is accepted for the read + create + comment
endpoints.

### SDUC-225 — Staff-only actions surface 403 for non-staff

`set_status`, `assign`, `set_priority`, `dispatch_issue`,
`github_push`, `github_refresh` return 403 for regular users.

### SDUC-226 — Missing Bearer → 401

Any endpoint without an auth header returns 401.

### SDUC-227 — Poll cadence

Workspace polls issues every 15s while User or Support is visible.

### SDUC-228 — User "Mes demandes" view

`render_user_requests` shows the caller's own issues with expand-to-comment
and create composer. The composer exposes a searchable site picker backed by
the signed-in account's Manage directory, defaults to the active site when
available, and offers an explicit no-specific-site choice. User-mode polling
requests `mine=1`, and both the overview counter and recent-request card apply
the same owner filter defensively. A broader Support cache or an older server
must never surface another requester's title in the User dashboard.

### SDUC-229 — Support "Requests" section

`SupportView` gains a `Requests` tab distinct from Tickets, with a
staff bar exposing status / priority / assign / dispatch / github when
the user is `issues_staff`. In the selected-request header, status, priority,
and assignee are compact semantic selectors (colored state dots and an `@`
marker), not decorative filled badges. The title keeps its own line, two quiet
actions remain at the right edge — an explicit, labeled AI summary action and
a horizontal overflow menu — and tenant plus relative update time are one
non-breaking context phrase before optional site/GitHub chips. Every selector
opens as an anchored overlay; the searchable assignee list is height-bounded
and virtualized, so even hundreds of agents never change the header height.
The shared reply composer stays compact at rest and puts attachments plus the
AI suggestion action in its footer. Requests keep the real AI backend/model
picker in the right-hand option slot; they do not invent a destination picker
because the current Issues API has no internal-note field. Tickets use that
slot as a real reply/internal-note popover because their API supports both.

### SDUC-459 — A request thread preserves every semantic message state

The Support request detail renders one chronological, virtualized timeline for
the opening message, human comments, status/GitHub/fleet notes, day separators,
rich Markdown (including disabled task checkboxes and code), attachment-only
messages, attributed quotes, external links, delivery/read/failure states,
live typing, an AI suggestion, a local draft, and retry affordance. Message
actions stay contextual and never turn an AI suggestion into a sent reply
without explicit review. Periodic issue refreshes must preserve the reader's
virtual-list position instead of snapping the thread back to its newest item.

The wire contract is additive and future-ready: per-comment `channel`, `quote`,
and `delivery`, plus issue-level `thread_state`, are optional and default empty
so payloads from the current Manage API remain valid. The local demo fixture
contains all thirteen states even while production omits unavailable data.

---

## 13. Bext Cloud

`crates/shelldeck-core/src/config/bext_cloud.rs`

### SDUC-240 — Config default and connected semantics

`BextCloudConfig::default()` is unconnected. `is_connected()` requires
a non-empty `bext_…` token.

### SDUC-241 — CLI login URL shape

`cli_login_url(port)` targets `…/cli/login?port=…` — **no state param**
(server uses a port-scoped cookie).

### SDUC-242 — Browser connect returns token

`browser_connect_listen(port, timeout)` returns the token from the
redirect on match. Favicon requests are ignored, then the real request
is accepted.

### SDUC-243 — whoami

`whoami(token)` returns the account (superadmin flag included).

### SDUC-244 — List sites (tolerates nulls)

`list_sites(token)` parses the sites list even when optional fields
are `null`.

### SDUC-245 — Create site body shape

`create_site(token, body)` sends the correct shape (name, plan, region).

### SDUC-246 — Site actions (`go_live`, `config`, `destroy`)

Each POST hits the correct path with the site ID and returns the
updated site.

### SDUC-247 — Destroy is confirmed via `AlertDialog`

The Bext view routes destroy through a confirm dialog before firing
the API call (guard against accidental clicks).

### SDUC-248 — Dashboard + admin instances

`dashboard(token)` and `list_instances(token)` parse. `list_instances`
is only invoked for superadmin tokens.

### SDUC-249 — Bext poll cadence

Workspace refreshes bext every 15s while `ActiveView::BextCloud` is
visible.

---

## 14. Bext Instance (single WordPress instance)

`crates/shelldeck-core/src/config/bext_instance.rs`

### SDUC-260 — List sites carries `X-Bext-App-Id`

`list_sites(instance)` GETs `/__bext/sdk/site/list` with the correct
`X-Bext-App-Id` header.

### SDUC-261 — Create site body shape

`create_site(instance, body)` POSTs the correct shape.

### SDUC-262 — Per-site actions

`get_site`, `go_live`, `config_site`, `destroy_site` hit the right
paths.

### SDUC-263 — Manage-bext connection button targets loopback

`Workspace::manage_bext_for_connection` targets
`http://127.0.0.1` (v1 local loopback). The remote-over-SSH-tunnel
variant is a follow-up (not shipped).

---

## 15. Update client & release pipeline

`crates/shelldeck-update/` + `.github/workflows/` + `cloudflare/update-worker/`

### SDUC-280 — Platform key is `{os}-{arch}` with `macos-*`

`current_platform()` returns `linux-x86_64`, `linux-aarch64`,
`macos-x86_64`, `macos-aarch64`, `windows-x86_64` — **never
`darwin-*`** (contract-critical: manifest, worker, workflow, client
must agree).

### SDUC-281 — Poll cadence hourly

`AutoUpdater::start_polling` fires the first check on start and then
every hour; user-triggered `check_for_update` is separate.

### SDUC-282 — Release info parses

`ReleaseInfo` parses the Cloudflare Worker JSON contract (version,
tag, per-platform URL + SHA-256).

### SDUC-283 — Download and hash verification

`installer::download_and_verify` streams the archive, computes SHA-256,
compares against the expected hash, and Errs on mismatch (never
installs an unverified binary).

### SDUC-284 — Install replaces binary safely per platform

`installer::install` on Linux/macOS moves-with-rename; on Windows uses
the pending-replace pattern (rename-old-then-rename-new post-exit).
No half-installed state on failure. The Windows `Expand-Archive`
PowerShell invocation doubles single quotes in the archive and staging
paths before interpolation, so an install path containing `'` can
neither break nor inject the command.

### SDUC-285 — Auto-update disabled respects setting

`set_enabled(false)` cancels the poll task and future manual
`check_for_update` no-ops until re-enabled. A development build compiled
without `SHELLDECK_UPDATE_PUBLIC_KEY_BASE64` behaves as disabled even when the
persisted setting is on: it performs no request and shows no misleading
verification error. Official tagged builds still require the key at build time
and verify every manifest signature.

### SDUC-286 — Install scripts serve both platform pairs

`install.sh` covers Linux + macOS (arch-detect via `uname -m`);
`install.ps1` covers Windows x86_64. Both live in
`cloudflare/update-worker/` and are served under `/install.sh`
`/install.ps1`.

### SDUC-287 — Release manifest matches workflow outputs

`.github/workflows/release.yml` produces per-platform asset names that
the worker manifest expects (naming drift is the highest-risk
regression class).

---

## 16. UI helpers (pure logic)

`crates/shelldeck-ui/src/{command_palette,sidebar,workspace}.rs`

### SDUC-300 — Fuzzy match: palette

`command_palette::fuzzy_match(haystack, needle)` returns true iff
every char of `needle` appears in the *lowercased* haystack in order.
**The needle is taken as-is** — the caller
(`CommandPalette::update_filter`) pre-lowercases the query. Empty
needle matches every haystack, including empty. Comparison is by
unicode `char`, not byte, so accented characters (`é`, `à`, `ü`) do
not silently match their ASCII counterparts.

### SDUC-301 — Fuzzy match with indices: sidebar

`sidebar::fuzzy_match_indices(haystack, needle)` returns the **char
positions** in the lowercased haystack (not byte offsets — the
consumer walks a `Vec<char>` at the same index). Case-insensitive on
both sides. Empty needle returns `Some(vec![])` (match, no
highlights); no match returns `None`.

### SDUC-302 — Sidebar site filter

`SidebarView::conn_matches_site(conn)` matches when
`site_filter == conn.site_id` **or** when `conn.site_id.is_none()`
(unbound connections always show).

### SDUC-303 — Command palette rebuild is deterministic

`Workspace::refresh_command_palette` produces the same action list for
the same input state (idempotent — no dupes on repeat calls).

### SDUC-304 — Palette selection preview event

Moving up/down in the palette emits the preview event so the workspace
can flash the target surface without confirming.

### SDUC-305 — Palette keyboard flow

Enter confirms, Escape dismisses, arrow keys move selection, typing
filters.

### SDUC-306 — Sidebar search bar filters connections

`conn_matches_search` matches on alias, hostname, user, and tag.

### SDUC-307 — Sidebar resize width bounds

`set_width` clamps within `[MIN_SIDEBAR_WIDTH, MAX_SIDEBAR_WIDTH]`.

### SDUC-308 — Sidebar collapse toggle

`toggle_collapsed` flips the state; when collapsed, the sidebar
renders only nav icons.

### SDUC-309 — Effective app mode

`Workspace::effective_mode()` delegates to the tested role matrix:
logged out → defensive User fallback behind the welcome screen;
regular/customer-admin → forced User; `inklura_support` → persisted
User/Support with Dev clamped to User; super-admin → persisted mode.

### SDUC-310 — Mode and Settings switches preserve active surfaces

Switching between Dev / User / Support hides the Dev surface without
destroying terminal sessions (SDUC-023 must not be interrupted).
Settings is a closable personal surface available in every authenticated
mode. User/Support expose General, AI, Appearance and About; Dev-capable
accounts additionally expose Terminal and Editor. Opening Settings pauses
surface-only polling and closing it returns to the intact current mode.

### SDUC-311 — Toasts respect level

`show_toast` renders Info / Success / Warn / Error variants with the
correct styling and auto-dismiss timer.

### SDUC-312 — Confirm-window-close guards unsaved work

`Workspace::confirm_window_close` returns false (block) when there is
in-flight work (script running, sync in progress) and true otherwise.

### SDUC-313 — Connection form validation

Aliases must be unique; hostname required; port defaults to 22 and
validates via `port_forward::validate_port`.

### SDUC-314 — Port forward form connection picker

Picker shows only connected (or connectable) hosts; disabled when
none.

### SDUC-315 — Login form flows

Email + password submit is disabled while empty; OIDC buttons pass
the provider correctly; browser password button emits
`StartOidc(None)`.

---

## 17. Cross-platform

Applies globally — see [`cross-platform.md`](../../.agents/cross-platform.md).

### SDUC-330 — Path helpers use `dirs::`

No hardcoded `/`, `~`, or backslashes anywhere. Config, data, cache
paths resolve via the platform-appropriate helper.

### SDUC-331 — Browser open helper spawns the right binary

`open_in_browser(url)` shells out to `xdg-open` (Linux), `open`
(macOS), or `start` (Windows). Failure to spawn returns Err (does not
silently swallow).

### SDUC-332 — CI matrix builds all three targets

`.github/workflows/release.yml` builds `linux`, `macos`, `windows`;
one failure skips the release + manifest jobs.

### SDUC-333 — Rust toolchain pin is enforced

`rust-toolchain.toml` pins `nightly-2026-03-06` for the pathfinder_simd
regression. Any PR that changes the toolchain must document the reason.

### SDUC-334 — Keychain works on each platform

SDUC-042 must pass on Linux (Secret Service), macOS (Keychain),
Windows (Credential Manager).

---

## 18. Internationalisation (i18n)

`crates/shelldeck-ui/src/i18n.rs` +
`crates/shelldeck-core/locales/{fr,en}.toml` — governed by
[`.agents/i18n.md`](../../.agents/i18n.md).

### SDUC-400 — `[general].ui_language` persists across restart

`UiLanguage` (`System` / `Fr` / `En`, `snake_case` on disk) round-trips
in `shelldeck.toml`. Absent field parses back to `System` (the
default) — **backward compat with configs written before i18n
landed**.

### SDUC-401 — Locale resolution is French-biased

`resolve_locale(&Fr)` → `"fr"`. `resolve_locale(&En)` → `"en"`.
`resolve_locale(&System)` returns `"fr"` when the OS locale starts
with `fr*` **and also when the OS locale is unknown / not readable**
(product default per AGENTS.md is French, not English).

### SDUC-402 — Locale is applied at startup and on config change

`apply_ui_language` runs once at boot (in `main.rs`) and once per
`SettingsEvent::ConfigChanged` (in the workspace) — `rust_i18n::set_locale`
is process-global; `cx.notify()` follows to repaint every open view.

### SDUC-403 — Missing keys fall back to French, not English

`rust_i18n::i18n!(fallback = "fr")` — a key present only in `fr.toml`
still renders in the UI when the active locale is `en`, and vice versa
never the reverse. Guarantee: no key ever renders as its raw slug.

### SDUC-404 — `rel_time(at_ms)` is fully localized

Relative timestamps ("à l'instant" / "just now" / "il y a 3 min" /
"3 min ago") go through `t!("time.just_now")`,
`t!("time.ago_minutes", count = …)`, `ago_hours`, `ago_days` — no
hardcoded French strings in the view layer.

### SDUC-405 — `t!()` accepts named variable interpolation

`t!("login.device", device = self.device)` interpolates `%{device}`
in the source key. The interpolation contract survives locale
switches; a key without `%{…}` placeholders ignores extra vars
without erroring.

---

## 19. Deep links (`shelldeck://`)

### SDUC-406 — `shelldeck://…` URLs parse to typed actions

`DeepLink::parse` turns an OS-delivered URL into a typed variant
(`Assistant`/`OpenConnection`/`SshConnect`/`TunnelStart`/`OpenSite`/`OpenIssue`/
`OpenTicket`/`JeanConfirm`). The scheme is case-insensitive; embedded
UUIDs are validated (bad UUID → `None`); query strings, fragments and
trailing slashes are ignored; unknown verbs and wrong schemes parse to
`None` so the router can no-op instead of guessing. Server-side IDs
(sites/tickets/issues/Jean jobs) keep their original casing.
`JeanConfirm` retains its job id across an asynchronous Fleet refresh and opens
the matching job detail rather than only navigating to the Fleet surface.

### SDUC-407 — Single instance + deep-link hand-off

ShellDeck runs as one process per user session. A second launch (or a
`shelldeck://` link followed while the app is open) forwards its
payload to the running instance over a loopback socket guarded by a
shared token, then exits — never a duplicate window. A stale discovery
file (crashed primary) is taken over by the next launch instead of
stranding it, and a hand-off carrying the wrong token is dropped so a
rogue local process cannot inject links.
The `shelldeck://assistant` payload follows the same authenticated hand-off,
but lands directly in the lightweight companion runtime instead of creating
the full Workspace or revealing the main window.

---

## 20. Recent activity

### SDUC-408 — Durable local activity log

ShellDeck records user-visible activity to a local JSONL log
(`activity.jsonl`) and reloads the newest entries at startup. The log
captures the activity kind, timestamp, message, optional target/action, and
optional detail (for example script exit code). Reads return newest-first,
respect the requested limit, and skip malformed lines so one bad append does
not prevent the app from opening.

### SDUC-409 — Recent activity surface

Dev mode exposes an "Activité" surface with search, kind filters, relative
timestamps, and contextual open actions. Entries can route back to the
matching surface when enough target data is present: terminal, connection,
script, tunnel, support ticket, hosted request, site, Jean, Fleet, or bext.
When Recent AI is enabled, a row can explicitly open the assistant with only
that activity entry and the bounded host directory as context.

---

## 21. Pinned connections

### SDUC-411 — Pinned connections persist and remain connection-scoped

Only connections can be pinned. Their UUIDs and order round-trip through
`AppConfig.pinned_connections`; configurations written before the feature
default to an empty list. The sidebar shows matching pins in a dedicated
top section and exposes pin/unpin as a localized row hover action. Deleting a
connection also removes its stale pin.

### SDUC-412 — Tray quick access connects the selected pinned host

The native tray submenu mirrors the current persisted pins on every desktop
platform. Each menu id embeds
the connection UUID, so clicks route to that exact host even after the list is
updated. Selecting an entry restores ShellDeck and starts the same SSH flow as
the sidebar connection action. Unknown and malformed menu ids are ignored.

---

## 22. Contextual AI assistant

### SDUC-413 — Provider configuration and real connection test

Settings → IA selects one local CLI (Claude Code, Codex, Aider) or API
provider (OpenAI, Anthropic), an optional model, and per-surface opt-outs. API
keys live only in the OS keychain. The assistant affordance remains hidden
when AI or the current surface is disabled, or when a selected local CLI is
not executable. The explicit connection test sends a real minimal completion
and reports the provider/model result, but does not become a per-launch lock.

### SDUC-414 — Contextual drafts never execute automatically

The shared AI sheet receives bounded structured context from Support,
requests, scripts, terminal, Jean, naming, or recent activity. Every call is
explicit and every result remains a draft: ShellDeck never sends a reply,
executes a terminal command, mutates a request, or overwrites a script from an
AI response. In the durable assistant conversation, both user and assistant
message bodies render Markdown structure—including headings, emphasis, lists,
links, code blocks, and tables—while the assistant copy action preserves the
original source text. User messages form right-aligned surface bubbles: short
one-line turns keep a compact width, while long or structured Markdown is
capped at 88% of the reading column and receives a definite layout width so
every wrapped line contributes to the bubble height instead of being clipped.
Assistant responses remain unframed prose, and both roles use compact
conversation block spacing without a document-style trailing margin.

### SDUC-415 — AI context and API privacy boundaries

Sensitive named fields are recursively redacted and serialized context is
capped before transmission. Provider system guardrails are sent separately
from untrusted context on OpenAI/Anthropic, and OpenAI Responses requests set
`store=false`. Contexts include a bounded host directory (display identity,
hostname, port, user, grouping, tags, site) so host aliases are understood,
but never include identity-file paths or credentials.

### SDUC-416 — Local CLI isolation

Local AI subprocesses run outside the project by default with tools, project
rules, persistence, MCP servers, repository writes, and analytics disabled
where the provider supports those controls. Claude defaults to the `sonnet`
alias instead of inheriting a potentially expensive user-selected model.

### SDUC-417 — Recently used command-palette actions

With an empty search, the command palette shows up to five commands used most
recently in the current session, newest first, followed by the remaining full
command list without duplicates. Executing the same command moves it back to
the top. A non-empty search ignores sections and filters all available
commands normally.

### SDUC-418 — Integrated AI drafts for Support and Scripts

When the configured provider and matching surface are enabled, Support exposes
explicit reply, summary, and triage actions; Scripts exposes generation,
explanation, and review actions. Reply and script-generation drafts remain
editable; analysis results are read-only, internally scrollable, and adjusted
through guidance plus regeneration. Nothing is sent, saved, or executed
automatically: accepting a reply fills the Support composer, accepting
generation opens the selected script in its unsaved inline editor, and
accepting an analysis copies it to the clipboard. The New/Edit Script form also
offers a compact AI instruction field. Its provider response must validate
against the structured name/description/language/category/body contract before
all five unsaved fields are populated; one corrective regeneration is attempted
for invalid JSON. The selected target and host are never changed implicitly. A
free-form read-only analysis (Support/request summary, script explanation, or
script review) renders as Markdown in both its workflow and task preview.
Structured JSON keeps its dedicated typed presentation, while editable replies,
commands, and script bodies remain raw so formatting cannot alter the payload. A
failed latest execution exposes a contextual correction action using its exact
exit code and output log; accepting opens the corrected body in the unsaved
inline editor and never reruns it automatically. A draft put on hold is
persisted under its distinct capability and target, capped to the latest 100
entries, and restored when reopened.

---

## 23. Launch at login

### SDUC-419 — Autostart uses the native per-user mechanism on every platform

The Settings launch-at-login toggle creates a per-user XDG autostart entry on
Linux, a Launch Agent on macOS, and an HKCU Run entry on Windows. Constructing
the backend must compile against each platform-specific `auto-launch` API;
macOS explicitly selects the Launch Agent path and never falls back to an
AppleScript login item.

### SDUC-420 — Requests expose contextual AI drafts and scripts show changes

When the Issues surface is enabled, the selected request exposes explicit AI
actions to draft a reply, summarize the thread, and propose triage. Accepting
a reply only fills the unsent comment composer; summaries and triage remain
read-only analyses copied explicitly. Every capability keeps a distinct
persistent pending draft. Script generation and correction show a bounded,
scrollable line diff against the current saved body before the user accepts
the unsaved replacement.

---

## 24. Virtualized operational lists

### SDUC-421 — Large request and support lists render only visible rows

User-mode requests, Support requests, and Support tickets use uniform virtual
lists. Loading hundreds of records must construct and paint only the visible
range while preserving filters, selection, row actions, contextual menus, and
scrolling. User-mode requests retain the same four-pixel visual separation
between compact rows and cap their nested viewport at 600 pixels, matching the
existing virtualized sites list.

---

## 25. AI-assisted request creation

### SDUC-422 — AI prepares but never submits a new request

When Issues AI is enabled, the New Request sheet accepts explicit instructions
from an AI panel that is collapsed by default and asks the configured provider
for a validated JSON draft containing title, structured description, and
priority. Existing form values and the bounded host directory are context only.
Valid output replaces the local unsent form fields; malformed output receives
one schema-repair attempt. Closing the sheet collapses the panel, invalidates
the pending response, and neither generation nor insertion creates the request.

### SDUC-423 — Staff explicitly applies structured AI triage

From the selected Support request, staff may ask AI for a strict triage proposal
containing an optional supported priority, an optional exact agent email, a
rationale, and bounded next actions. The review shows current and proposed
values. Applying is a separate explicit action and revalidates staff access,
request identity, schema, and agent availability before sequential API writes.
Non-staff users never see the applicable triage action. Tags remain excluded
until the Issues API exposes a dedicated mutation.

### SDUC-424 — Support conversion opens an unsent request draft

Converting a Support ticket switches to the New Request sheet with title and
description prefilled and source set to `support`. The user may edit or use AI,
and only the existing Create action sends the request. Closing the sheet resets
the source so a later ordinary request remains `user`.

### SDUC-425 — Terminal output becomes a bounded unsent request draft

With diagnostic context available, the Terminal AI toolbar can open New
Request with session, working directory, and the current selection or latest
120 visible lines. The draft source is `shelldeck`; no command runs and no
request is created until the existing Create action is confirmed.

### SDUC-426 — AI proposes reviewable entity names

When Naming AI is enabled, the Script form, Terminal toolbar, Tunnel form, and
New Request sheet expose a visually consistent AI naming action. The provider
receives only the current entity context plus the bounded host directory and
must return a strict one-line JSON name of at most 80 characters. The shared
workflow previews the proposal; only Accept updates the still-local field or
session title. Cancelling, closing, or losing the original target changes
nothing and never saves, creates, connects, or executes an entity.

### SDUC-427 — AI actions require a typed plan and separate confirmation

An executable AI result remains a draft after generation and after the normal
Accept/Insert action. Execute or Send creates an in-memory `AiActionPlan` with
the exact target, action kind, risk, provider/model, timeout, and payload, then
opens a second confirmation dialog showing what will affect the real system.
The final click revalidates target and permissions. Terminal commands target
the exact active session; generated/fixed scripts run without saving; Support
replies, Jean sends, and Fleet dispatches reuse their existing service clients.

### SDUC-428 — Confirmed AI actions are stoppable and safely audited

Confirmed script executions reuse the existing Stop control and are forcibly
stopped after 30 minutes. Completion, failure, manual cancellation, and timeout
remove the action-specific tracker so stale timers cannot affect later runs.
Network actions retain their bounded client timeouts. Durable activity entries
record action ID, capability, kind, risk, target, provider/model, timeout, and
status, but never command bodies, replies, prompts, terminal output, or secrets.
Terminal submission is audited as submitted; completion remains observable in
the PTY and is interrupted manually with Ctrl+C.

### SDUC-429 — AI tasks remain visible outside their originating workflow

Integrated AI generations and confirmed actions appear in a shared Tasks tab
inside the assistant sheet. The durable task state distinguishes generation,
ready and pending drafts, confirmation, execution, application, success,
failure, and cancellation. Legacy persisted drafts load as pending tasks.
Actionable tasks contribute to the titlebar assistant badge; a task can reopen
its exact workflow or target, and active Terminal/Script actions reuse their
existing Stop path. A generation completed after its workflow closes produces
one in-app result notification. Restarted active states become cancelled rather
than pretending that a lost process or request is still running. The system
tray exposes a localized running count limited to `Generating` and `Executing`;
confirmation waits and ready drafts do not inflate it. Selecting that indicator
shows the existing single-instance Dock directly on its Tasks tab without
revealing the main window. Every desktop backend updates the count live on the
thread that owns its native menu.

### SDUC-430 — Executable AI capabilities obey persisted autonomy policies

Settings exposes `Prepare`, `Confirm`, and `Automatic` policies for Support
send, Support triage, Terminal execution, Script execution, Jean dispatch, and Fleet dispatch;
surface toggles remain the single disabled state. Older configurations default
every executable capability to confirmation. Preparation hides or blocks the
final executable action. Automatic skips the second dialog only for low or
moderate risk; high-risk Terminal, Script, and Fleet plans always require the
dialog. The exact effective policy is frozen into `AiActionPlan`, target and
permissions are still revalidated, and redacted audit metadata records the
autonomy used without recording executable content. A capability the current
desktop cannot support (today `clippy_replace_selection`, until a production
desktop context provider lands) shows a disabled, localized "not available on
this desktop yet" row instead of autonomy level buttons — an unreachable
capability must never offer an `Automatic` setting — while its config field
stays parsed and persisted for forward-compat.

### SDUC-431 — Terminal diagnosis produces bounded executable steps

Terminal diagnosis returns a strict structured plan containing a concise
summary and one to five distinct steps. Core validation accepts only bounded,
read-only commands from an explicit executable and subcommand allowlist; shell
operators, elevation, mutation, continuous follow modes, and duplicate steps
are rejected. The workflow renders each command and explanation separately.
Running a step revalidates both the command and exact active terminal session,
then stages a high-risk Terminal action, so confirmation remains mandatory
under every autonomy policy. A full plan may advance step by step only after a
deduplicated `OSC 133;D` completion event; missing shell integration is framed
by ShellDeck, non-zero exit stops the sequence, output is bounded, and Ctrl+C
remains the stop path.

### SDUC-432 — Requests and Support tickets accept image evidence through every desktop path

The New Request, request-comment, Support request-comment, ticket-reply, and
internal-note composers accept up to five PNG, JPEG, or WebP images (9 Mo each,
leaving multipart headroom below Bext's 10 MiB request cap) from an image URL, the native file picker,
Ctrl/Cmd+V, drag-and-drop, or the platform's interactive area capture. Local
drafts show a removable preview and are not uploaded until submission.
On Linux, an area capture with no capture tool installed (none of
`gnome-screenshot`, `spectacle`, or ImageMagick `import` on `PATH`) reports a
dedicated localized missing-tool error distinct from user cancellation.
An area capture opens a native annotation editor before it becomes a draft,
with freehand, arrow, rectangle, text, undo, clear, and color controls; saving
exports a PNG while cancelling preserves the unedited capture.
ShellDeck obtains a short-lived, single-use, issue-scoped ticket from Manage,
uploads the bytes directly to Inklura Share, and sends only opaque receipts
back to Manage. Manage validates tenant and issue scope before persisting
structured attachments. Request and comment attachments remain visible to
User and Support surfaces and are mirrored as image links to GitHub/Jean.
Ticket attachments remain structured in the helpdesk thread and their Share
viewer links are routed to the originating email, livechat, Manage, or SMS channel.
Issue uploads never appear in the uploader's personal Share gallery.
After publication, request owners and internal staff can permanently delete an
image from the native gallery after a destructive confirmation. Manage removes
the thread reference and uses a short-lived, single-use scope capability so
Share deletes only the blob belonging to that exact request or Support ticket.

### SDUC-433 — Multi-line inputs behave like native textareas

Every multi-line Input uses its wrapped visual layout for cursor movement and
selection. Up/Down retain the preferred visual column, Shift+Up/Down extends
the selection, and Home/End move to the current visual line edges. Selections
remain visible across hard newlines and soft wraps. When `max_rows` caps the
field, mouse-wheel scrolling moves through the whole value and keyboard
editing scrolls the internal viewport to keep the caret visible instead of
growing the surrounding screen or typing off-screen.

### SDUC-434 — The tray toggles one standalone AI Dock

The system-tray menu can create, hide, show and focus one Assistant Dock without
making the main ShellDeck window visible. The 480 px Dock is anchored to the
right edge for the display height, has no native titlebar, and
cannot be moved, resized or minimized. Its exposed top-left and bottom-left
corners use the same theme radius as the floating main window; both right
corners remain square so the Dock stays visually fused to the screen edge.
Its 56 px activity rail is a full-height sibling of the 424 px conversation
column. The single 44 px conversation header is therefore confined to the
left column and contains only the thread title and hide action; reopening
ShellDeck remains available once, in the rail's bottom toolbox.
Repeated invocations reuse the existing
Dock rather than creating duplicates. Closing the Dock hides it and keeps an
in-flight request alive. Reopening it re-prepares the same Global context
*without* invalidating the pending request gate — the reply still lands with
its loading state intact — while a genuine surface/title switch still
invalidates the gate and drops the stale reply. It inherits ShellDeck's UI font and scale, uses a
bounded global context, shares durable conversations and tasks with the main
assistant, exposes an explicit action to reopen ShellDeck, and disables
submission with an explanation when no usable global AI backend is configured.
The Dock's global chat is owned by an independent `AiCompanionController`:
opening it and completing a conversation do not initialize `Workspace`.
Selecting a task action may initialize `Workspace` when its terminal, ticket,
or script target is required. `CompanionRuntime` retains the Dock and palette
window handles and owns global-shortcut routing, so repeated invocations do
not depend on scanning or constructing the main application surface.
The enabled-by-default global shortcut toggles that same single Dock from any
application: Ctrl+Shift+Space on Windows/Linux and Cmd+Shift+Space on macOS.
The Dock opens on the display containing the pointer, moves to that display on
the next invocation if necessary, and hides on Escape or when its window loses
focus.
On Wayland, both startup shortcuts are submitted together through one XDG
Global Shortcuts portal session; accepted `Activated` signals route through
the same runtime IDs as native backends. Portal absence, user refusal, or an
empty accepted set is non-fatal and leaves the tray path available.
Changing either global-shortcut toggle in Settings registers or unregisters
that shortcut immediately without restarting ShellDeck. The runtime tracks
successful registrations, avoids duplicate work for unchanged settings, and
can retry a failed registration on the next enable/sync transition.
Each shortcut can be captured independently and persists in GPUI keystroke
syntax; old configs receive platform defaults. Capture requires Ctrl, Alt,
Cmd, or Super, rejects a duplicate Dock/palette combination, supports reset
to default, and applies immediately. Settings shows disabled, applying,
active, portal-pending, conflict, and native error states. Wayland portal
acceptance or refusal replaces the pending state asynchronously.
Following `shelldeck://assistant` creates or focuses this same Dock
idempotently: an already visible Dock stays visible instead of being toggled
off, and the main ShellDeck window remains hidden. Every visible tray label,
including zero/one/many counter forms and the empty pinned-connections row,
follows the selected French or English UI locale. A live language change
republishes the tray snapshot so every desktop backend updates the native menu
immediately. Counters and pinned connections follow the same owner-thread
snapshot path. The Dock header and rail toolbox use keyboard-focusable controls
with visible localized names or tooltips; Escape remains an explicit hide
action. On macOS, the tray uses a dedicated
36 px black-and-alpha Monolith mark as an AppKit template image, so the system
controls its light, dark, pressed, and accessibility appearance. Linux and
Windows retain the colored app icon.

### SDUC-435 — Companion startup never strands an invisible process

`companion.start_hidden` keeps the main ShellDeck window hidden on startup only
when the system tray was created successfully. The default remains a visible
start for old and fresh configurations. If the tray backend is unavailable,
ShellDeck ignores the hidden-start preference and opens its main window so the
process is always recoverable. Tray and deep-link show actions explicitly show
the hidden window before activating it. A hidden start initially owns only a
lightweight `CompanionRoot`: it does not construct `Workspace`, its views or
its pollers until a tray, deep-link, palette, or task-target command needs
application state. The standalone AI Dock, including the
`shelldeck://assistant` route, is served directly by the companion controller
and is not such a command. SSH config parsing and the connection
store are deferred to that first Workspace demand; configured startup Cloud
Sync begins afterward on the background executor instead of blocking process
startup or the UI thread. When `tray.close_to_tray` is enabled, both the native window-close
request and ShellDeck's custom titlebar × hide the main window without shutting
down the workspace or tray.

### SDUC-436 — The global shortcut opens only the standalone command palette

With the main ShellDeck window hidden, Ctrl+Alt+Space on Windows/Linux or
Cmd+Alt+Space on macOS opens one borderless command-palette window with the
search input focused. Repeated triggers reuse and toggle that window. Escape
and command confirmation hide it, as does moving focus to another window. The
palette is centered on the display containing the pointer and migrates between
displays on its next invocation. Commands that navigate ShellDeck reveal the
main window after selection; background commands can complete without showing
it. Labels, icons and shortcut hints remain contained at the minimum palette
width. Linux/Wayland registration failure remains non-fatal and the tray entry
opens the same standalone palette. The search field exposes a localized
accessible name and keyboard description. Up/Down and Tab/Shift+Tab wrap
through results, Home/End jump to the edges, Page Up/Page Down move by a bounded
page, Enter activates the selected command, and Escape dismisses the palette.

---

## Retired use cases

*(none yet)*

---

## 26. Workspace Git status

### SDUC-437 — Compact Git status

When ShellDeck runs inside a Git repository, the status bar displays the
current branch and counts staged, modified, and untracked paths. Collecting
that snapshot uses a single porcelain-status invocation and pauses while the
main window is hidden in the tray.

---

## 27. Embedded icon integrity

### SDUC-438 — Reachable icons and Monolith motions render instead of blank slots

Every named Lucide icon selected dynamically by ShellDeck's AI actions and
shared Alert variants is present in the curated asset directory and registered
in the binary asset source. Contextual Monolith WebP motions used during AI
generation and on terminal/site empty states follow the same contract. Missing
assets or non-WebP study exports must fail a unit test instead of silently
rendering an empty fixed-size slot in the interface.

### SDUC-439 — Only unexpected SSH transport loss produces an OS notification

Each primary SSH terminal reports an explicit lifecycle result to the
Workspace. Closing its tab and exiting the remote shell normally update the
connection state without an OS notification. A transport that disappears
without EOF, channel close, or an exit status is treated as unexpected: the
connection leaves its active state, the tray counter refreshes, and—when the
existing SSH notification preference is enabled—the OS notification names the
exact connection. Another live tab for the same connection keeps the shared
sidebar status connected. Notification copy follows the selected French or
English locale.

### SDUC-440 — User and Support modes have a real home

User mode opens on an Accueil tab summarizing available sites and open
requests, with direct actions for Sites, Requests, and a new request. Its three
most recent requests open directly from the dashboard, while a compact status
card exposes the Manage session, active site, synchronized directory, and a
manual sync action. A dashboard-specific network illustration with a contrast
gradient gives the page a clear identity without reducing the readability of
those operational cards. Support mode opens on its own Accueil tab with open,
SLA-risk, unassigned, and hosted request counters plus direct triage actions.
Operational lists remain separate tabs. Onboarding only describes modes and
shortcuts the signed-in role can actually reach.

---

## 28. Application chrome

### SDUC-441 — Every Workspace-drawn surface scales with the App Font Size

The App Font Size setting drives the window rem size, and the surfaces the
Workspace renders itself — User mode's home, the pre-login welcome screen, the
titlebar chrome, and the theme / account / site / mode dropdowns — grow and
shrink with it exactly as the child views (sidebar, Support, Settings,
Dashboard) already did. Genuine device-pixel call sites stay absolute: the
window client inset, the rem size itself, box-shadow geometry, window-edge
resize hit-testing, and the sidebar width the terminal grid is offset by.

### SDUC-442 — An application menu row is available in every mode

A File / Edit / View / Go / Terminal / Help row sits under the titlebar in
User, Support and Dev, and on the pre-login welcome screen. Its contents follow
the account: logged out it offers only sign-in, quit, interface zoom and about;
User mode omits every SSH, terminal and staff-console command; the staff
consoles (JeanClaude, Fleet) appear only when both the capability and the
configuration are present. Commands route through the same handler the command
palette and the keyboard shortcuts use. The row can be hidden from View → Menu
Bar; the preference persists and the terminal grid resizes to match.

### SDUC-443 — The Dev sidebar is an activity rail plus a contextual panel

Dev mode shows a fixed-width icon rail listing the navigation sections that
have a contextual panel behind them, with the active one marked, Settings
pinned to the bottom, and connected-host / open-tab counts carried as badges.
Destinations without a panel — JeanClaude, Fleet, bext Cloud — are reached from
the Aller menu and the command palette rather than taking a rail slot.

The panel follows the selected activity: Connections keeps its grouped host
list with pins and per-row actions, while Terminals lists open tabs, Scripts
the saved scripts, Port Forwards the configured forwards, Sites the available
sites, Recent the activity feed, and Editor the open buffers. Each row reports
whether it is the active one and whether it is live, and selecting it performs
that activity's own open/focus action. An activity with no rows shows a
localized empty state; an activity with no panel at all hides the panel so its
main view takes the full width.

The rail is unconditional: no state renders the Dev sidebar without a
navigation surface, and there is no setting to hide it. The panel collapses
independently via the sidebar toggle, leaving the rail. The terminal grid is
offset by whatever is actually on screen, for both panel collapse and
panel-less activities.

The panel header names the active activity, so the list below it does not
repeat that name as its own section header.

### SDUC-444 — A global shortcut that cannot register says why

A global shortcut whose registration is refused reports it instead of looking
like a shortcut that merely "rarely works": once as a toast on the transition
into failure, and durably as a status badge next to the combination in
Settings. Repeat publications of the same failure stay silent, and the two
shortcuts report independently.

The reason reaches the user in their own language when it is one ShellDeck can
explain. A Wayland session whose portal stack does not implement
`org.freedesktop.portal.GlobalShortcuts` cannot grant a global grab to any
application at all, so that case is named as the environmental limitation it
is rather than forwarded as the ashpd/D-Bus sentence. Platform errors ShellDeck
cannot interpret still reach the user verbatim.

The status the Workspace shows is the current one, not the one that existed
when the process started. A portal answers asynchronously and a tray-mode
launch (`start_hidden`) has no Workspace to receive that answer, so the first
window to open is seeded from the live registration state.

---

## 29. Assistant routing and typed workflows

### SDUC-452 — The assistant can prepare a real request form

When the latest chat message explicitly asks ShellDeck to create or prepare a
new customer request, the provider returns a strict routed request draft.
The main Assistant sheet and the standalone Dock both open the existing New
Request sheet with title, description, and priority prefilled. The conversation
records a clear acknowledgement, while the existing Create button remains the
only operation that submits the request. Questions, explanations, quoted
instructions, malformed routing output, and ordinary chat continue through the
normal Markdown response path.

### SDUC-453 — Assistant shortcuts declare whether they send or prefill

Every contextual Assistant shortcut has an explicit interaction mode. Actions
that are complete from the current context submit immediately into the
conversation. Actions that require the user to provide an objective — Script
Generate, Script Convert, Terminal Command, and Create Request on Issue or
Terminal — insert a localized, editable fill-in template and focus the composer,
without creating a conversation message or starting an AI request.
Context-complete analyses such as Summary submit immediately. Chat Triage uses
a readable analysis prompt; the strict JSON prompt remains reserved for the
typed triage workflow. Since the 2026-08 empty-screen redesign the shortcuts
render as uniform tiles: there is no per-mode tooltip and no button-variant
visual code. The submit-versus-prefill distinction is behavioral only, pinned
per surface by SDTEST-1429.

### SDUC-454 — Natural-language actions reuse typed ShellDeck workflows

An explicit conversational instruction can open the existing unsaved Script
form and generate its draft, prepare a command for the currently active
Terminal workflow, draft a reply for the selected Support ticket, stage a Jean
dispatch behind its existing confirmation, or navigate to one visible hosted
request by exact ID/title or an unambiguous partial match. The assistant never
executes a command, saves a script, sends a reply, or dispatches to Jean by
itself. Missing, stale, unauthorized, or ambiguous targets stop at a localized
warning instead of being guessed.

---

## 30. Clippy assistant and desktop characters

### SDUC-445 — Clippy transforms explicitly supplied clipboard text

The AI Dock (tray → rail), the command palette ("Ouvrir Clippy" via the
`OpenClippy` action, shown only when a usable backend is configured and the
Clippy surface is enabled), and a Clippy header pill in the main assistant
Sheet can all open the dedicated Clippy surface. The Sheet pill mirrors the
tasks pill and toggles Chat↔Clippy; it stays visible while on Clippy even if
the capability drops, so the route back never vanishes. Clipboard text
enters ShellDeck only after the user presses **Use clipboard** or pastes it into
the source field. Rewrite, translate, shorten, summarize, explain, draft reply,
and custom operations use the configured AI backend. The response remains a
reviewable draft with a line diff and explicit Edit, Regenerate, Copy, and
Cancel controls. Clippy is an opt-in AI surface and is disabled by default.

### SDUC-446 — Desktop context is bounded, untrusted, and replacement-safe

Clippy delimits application text from trusted instructions, redacts common
credential forms, blocks password-role selections, and bounds source, result,
instruction, and screenshot metadata. Logs and durable audit details retain
only operation and size metadata, never source or generated text. A native
selection replacement can proceed only when the adapter still reports the same
window, identity, and text that produced the reviewed result. Unsupported,
closed, permission-denied, and stale targets preserve Copy as the fallback.

### SDUC-447 — The selected companion character persists everywhere

Appearance settings offer no character plus Clippy, Shelly, Spark, Byte, Orbit,
and Nox with real embedded previews. Character, motion preference, scale,
desktop enablement, roaming level, window-climbing preference, and multi-screen
preference persist in `[clippy]`. Unknown future character IDs fall back safely
to Clippy. The selected mascot appears as an independent desktop character
without being embedded in the AI Dock, and updates without requiring a restart.

### SDUC-448 — Desktop roaming is transparent, interactive, and event-driven

On X11, Windows, and macOS, an enabled desktop character uses one transparent,
no-focus pointer-enabled overlay and native top-level window movement. A
fixed-step simulation caps catch-up, clamps positions to available platform work areas,
preserves fractional refresh time and landing events across catch-up steps,
stops animation-frame requests at rest, and wakes from one-shot timers for
occasional or playful actions rather than polling continuously. When enabled,
multi-display routing cycles through connected displays and recovers after
monitor changes. Windows and macOS use per-display taskbar/dock-aware work areas;
X11 currently applies the root EWMH `_NET_WORKAREA`. Active GPUI drag
delivery and inside/outside release handling keep interaction alive even when a
magnetic preview moves the native overlay away from the pointer. Tray actions
pause/resume movement and return the character to a safe screen corner.

### SDUC-449 — Unsupported desktop capabilities degrade honestly and safely

Wayland does not permit reliable arbitrary top-level positioning, so ShellDeck
does not fake roaming there: it keeps the desktop character disabled and reports
the platform limitation in Appearance. System motion honors the OS reduced-
motion preference; Reduced/Off motion and Still roaming request no continuous
frames. Overlay creation or native movement failures pause the character
without stealing keyboard focus, and Windows overlays use non-activating native
styles and show paths. External-window climbing is used
only when the platform geometry provider can supply eligible visible window
edges; invalid, minimized, fullscreen, and desktop surfaces are excluded. The
X11 external-window filter is at parity with the Windows
(`WS_EX_TOOLWINDOW`/owned) and macOS (layer-0) filters: EWMH
`_NET_WM_WINDOW_TYPE_{DOCK,MENU,TOOLBAR,TOOLTIP,POPUP_MENU,DROPDOWN_MENU,
SPLASH,NOTIFICATION,UTILITY}` windows and windows with `WM_TRANSIENT_FOR` set
are never climbable (SDPATCH-113). The X11-vs-Wayland decision follows GPUI's
own compositor detection (`companion_desktop::is_x11_session`, backed by
`gpui::guess_compositor()`); `XDG_SESSION_TYPE` is no longer consulted
anywhere in the `shelldeck` crate, so pointer coordinate-space math always
agrees with the backend GPUI actually connected with. An
X11 backend can observe X11 and XWayland clients only. Native Wayland windows,
including browsers or messaging apps using their Wayland backend, are not
presented as climbable because GNOME and other compositors do not expose a
standard permitted global window-geometry API.

### SDUC-450 — Character selection is discoverable and immediately visible

Authenticated users can open the character cards directly from the File menu,
command palette, or native tray instead of finding them below unrelated theme
controls. The targeted route opens Appearance with the six mascot previews at
the top. Choosing any mascot enables its desktop runtime immediately; choosing
None disables it. The separate motion, size, roaming, window-climbing, and
multi-monitor controls remain available directly below the cards.

### SDUC-451 — The desktop character responds directly to the user

The selected mascot is rendered only in its transparent desktop window, never
inside the AI Dock. Pressing and dragging the character preserves the exact grab
offset, pauses autonomous movement, follows the pointer across monitor bounds,
and clamps the dropped position to the selected display. Approaching the outer
top edge of an eligible unmaximized window shows a live magnetic preview with a
larger acquisition band and stable-ID hysteresis, so small pointer movements do
not flicker between targets. Releasing revalidates that exact preview ID against
a fresh native snapshot; it never silently switches to a competing window if the
preview moved, minimized, or disappeared. Once a preview ID is invalidated, that
drag remains unsnapped until release instead of acquiring a different cached
window. A click triggers a short hop;
a double-click triggers a larger bounded dash and no keyboard focus theft. Each
mascot keeps its production PNG artwork while procedural poses provide distinct
walking, flying, dragging, reaction, and landing motion. Character-specific
speeds, bounce, tilt, and target choices make their movement visibly different.
Static idle periods use no continuous runtime frames; an occasional one-shot
flourish adds life without turning the overlay into a permanent render loop.
Motion preference changes apply immediately, and the tray can pause/resume the
character or return it to a safe screen corner. When window climbing is enabled,
the character chooses an eligible external top-level window by its stable native
lifetime ID, climbs or snaps from drag release to its outer top edge, and treats
window tops as one-way floors for falling only from above. Snap and autonomous-
climb candidates exclude maximized/taskbar-inset windows and are ranked
deterministically by vertical gap, horizontal distance, then stable native ID
with target-display DPI-scaled bands, overlap, extent, and size thresholds.
Preview, release commit, and follow all use the same perch-origin calculation,
including windows narrower than the mascot, so mouse-up and the first follow
refresh cannot jump, including when the preview remains locked only through the
wider hysteresis exit band. Autonomous climbing obeys the multi-monitor setting:
when cross-monitor movement is disabled, only windows on the current display are
eligible. The attached character
preserves its horizontal perch offset as the window moves between displays or
resizes; a snap also adopts the supporting window's display so later
screen-floor recovery uses that monitor instead of an old display or virtual
layout gap. If no eligible top is reached, the display work-area floor is the
safe fallback landing surface. The single-body deterministic AABB solver also
clamps and reflects against display side walls and the ceiling, settles tiny
post-impact horizontal velocities, and prevents fast descending motion from
tunnelling through one-way window tops. Cached platform snapshots feed the
runtime instead of native enumeration on every animation frame: one initial
full list seeds the fall, then each 100 ms refresh simulates the next fixed-step
trajectory and revalidates at most the first reachable stable ID. If that target
moves or closes, only its fresh geometry may become the active collision
surface; no unvalidated cached fallback is promoted, and the remaining cached
windows are reconsidered on a later refresh. Closing, minimizing, or otherwise
losing the target window detaches safely, restarts falling only when full motion
is allowed, and otherwise returns to still/reduced-motion behavior. Screen-floor
landings resume the one-shot roaming schedule only when the character is not
attached or being dragged. Dragging, clicking, pausing, returning to a corner,
disabling climbing, or closing the overlay cancels the attachment and its
generation-guarded refresh timer immediately; disabling climbing during a fall
clears cached window platforms so the character continues to the screen floor.
Subthreshold pointer jitter must preserve click delivery and avoid native overlay
moves. Pause, reduced-motion transitions, clicks, and new drags clear stale
dynamic velocity/contact immediately; native movement is gated by rounded
platform origins. When gravity starts without cached geometry, the runtime
performs one full visible-window discovery, then simulates the next fixed-step
trajectory and refreshes at most the first reachable collision candidate's
stable native ID per 100 ms tick. This bounds synchronous native lookup work
independently of the total visible-window count and prevents a closed target
from activating an older unvalidated cache entry. Per-pixel native hit testing
remains a platform-hardening
follow-up, so the transparent overlay is still bounded by its configured mascot
viewport rather than its exact alpha silhouette.

---

## Change log

- **2026-08-11** — Amended SDUC-414 and the pending SDTEST-1425 contract: Dock
  user bubbles now receive a definite compact-or-88%-capped width before
  Markdown layout, preventing both single-line clipping and GPUI's min-content
  collapse to one character per line. The assistant remains unframed prose and
  compact Markdown removes the trailing document margin from both roles. The
  composer's attachment/target actions are also injected once regardless of
  whether its context chip is visible, instead of duplicating both icons.
- **2026-08-11** — Amended SDUC-434: the screen-edge AI Dock now mirrors the
  floating main window's client inset and theme radius on its two exposed left
  corners while keeping its right edge square. Its 56 px activity rail is now
  the full-height sibling shown in the prototype instead of starting below a
  duplicate host toolbar; the 44 px header is confined to the conversation
  column and « Ouvrir ShellDeck » lives only in the bottom toolbox. The existing
  SDTEST-1392 Linux runtime smoke was repeated with composite/window captures to
  verify both the curved pixels and this two-column hierarchy.
- **2026-08-10** — Added SDUC-459 and SDTEST-1598/1599 for the complete semantic
  Support-request timeline. The demo exercises all thirteen prototype cases;
  optional future API fields default empty for backward compatibility, and
  polling preserves the reader's virtual-list position.
- **2026-08-06** — Windows-portability wave: added SDUC-455 (the
  `[terminal] default_shell` field is honored for new local terminals and
  splits — it was dead — with the platform-correct shell fallback chain and a
  home-then-`"."` PTY cwd, never `/`), SDUC-456 (SSH default key discovery is
  home-resolved, empty without a home), and SDUC-457 (local discovery and the
  Server Sync file browser are shell-free with honest Windows
  permissions/owner and `std::path` breadcrumbs). Amended SDUC-043 (explicit
  non-persistent TOFU with one warning when no home resolves, never
  `/root/.ssh`), SDUC-100 (device check-in sends the real hostname; terminal
  fallback is now `"ShellDeck"`, not `"unknown"`), and SDUC-284 (quote-safe
  Windows `Expand-Archive` paths). New tests SDTEST-1579..1591;
  SDTEST-1584 is `#[cfg(windows)]` and Yellow until a Windows CI test target
  exists. Back-filled SDUC-458 for the Jcode fleet executor that shipped
  without a use-case entry.
- **2026-08-06** — Amended SDUC-434 and added SDTEST-1578: reopening the AI
  Dock re-prepares the same Global context without invalidating the in-flight
  request gate (the pending reply used to vanish without spinner or error);
  only a genuine surface/title switch discards a stale reply.
- **2026-08-06** — Amended SDUC-445 (Clippy is reachable from the Sheet host
  via the new `OpenClippy` palette entry and a Sheet header pill, in addition
  to the Dock rail and tray) and SDUC-430 (an unsupported
  `clippy_replace_selection` capability renders a disabled localized row
  instead of autonomy buttons). Amended SDUC-453 and the SDTEST-1429 row: the
  quick-action tile redesign removed the tooltips and the button-variant
  visual code the entry promised; the submit/prefill distinction is
  behavioral only.
- **2026-08-06** — Amended SDUC-449 for X11 external-window filter parity
  (SDPATCH-113: EWMH dock/menu/toolbar/tooltip/popup-menu/dropdown-menu/
  splash/notification/utility types and `WM_TRANSIENT_FOR` windows excluded,
  matching Windows/macOS) and the compositor-detection contract
  (`gpui::guess_compositor()` via `companion_desktop::is_x11_session`;
  `XDG_SESSION_TYPE` retired); registered SDTEST-1594..1597. Amended
  SDUC-432: Linux area capture reports a dedicated tool-missing error
  distinct from cancellation (SDTEST-1593 Red).
- **2026-08-06** — Renumbered the assistant-routing use cases to SDUC-452..454
  (formerly SDUC-445..447 on `feat/composer-partage`): the Clippy / desktop
  companion work merged first and holds SDUC-445..451. IDs are sticky, so the
  collision is resolved by giving the later allocation fresh numbers; the
  affected SDTEST rows (1427..1432) now reference the new IDs.
- **2026-08-06** — Reconciled SDUC-445 with SDUC-454 at merge time: Clippy
  transforms share the assistant Submit path but carry no user message, and
  `complete_assistant_turn` now skips the action router entirely for such
  turns, so untrusted clipboard content can never surface a typed action
  (extends SDTEST-1427). Clippy also became an `AiActivity` reachable from the
  Dock rail and both hosts of the redesigned assistant.
- **2026-07-30** — Extended SDUC-448/449/451 and added SDTEST-1571..1577 so
  gravity discovers windows after a fall starts, applies the actual drag-release
  velocity before prediction, trajectory-ranks the first reachable top by exact
  projected time of impact, and lands on visible chrome instead of falling directly to the
  display floor. Subsequent updates revalidate one captured stable ID and never
  promote an older unvalidated fallback, avoiding repeated synchronous full X11
  scans. X11 snapshots include validated EWMH `_NET_FRAME_EXTENTS` when
  available; native Wayland clients remain outside the X11 geometry provider.
- **2026-07-29** — Hardened SDUC-448/449/451 and added SDTEST-1547..1562 plus
  SDTEST-1570 for impact-time diagonal collision, deterministic platform ties,
  changed work-area floors, preserved catch-up landings, fractional frame time,
  bounded live fall-platform refresh, safe attachment/config cancellation,
  active-drag outside release, stale throw suppression, rounded native moves,
  OS reduced motion, a bounded idle-flourish duty cycle, true platform work
  areas, Windows no-focus overlays, and a localized native-Wayland capability
  warning.
- **2026-07-29** — Extended SDUC-451 and added SDTEST-1529..1546 for live
  magnetic preview, stable-ID hysteresis and release validation, shared
  preview/commit/follow perch geometry, target-display DPI scaling, throttled
  full-list plus targeted locked-window refresh, exact screen-floor bounds,
  autonomous unmaximized-window filtering, and deterministic wall/ceiling
  collision response.
- **2026-07-29** — Extended SDUC-451 and added SDTEST-1505..1528 for the
  standalone deterministic AABB companion physics/runtime: drag-release outer
  top-edge snapping, one-way window-top floors, screen-floor fallback, stable-ID
  attachment/follow after snapping or landing, disappearance-to-fall recovery,
  reduced/off/still suppression, cached event-driven snapshots, snap-display
  adoption before disappearance floor recovery, subthreshold-jitter click
  preservation, and mid-fall climbing-disable platform clearing.
- **2026-07-29** — Extended SDUC-451 and added SDTEST-1500..1504 for stable
  external-window identities, top-edge attachment, movement and resize following,
  redundant-move suppression, disappearance recovery, and generation-cancelled
  low-rate monitoring only while a character is attached.
- **2026-07-29** — Extended SDUC-451 and added SDTEST-1494..1499 for production
  PNG-backed procedural poses, distinct mascot personalities, bounded varied
  roam targets, one-shot idle flourishes, real-time frame pacing, DPI-aware drag
  thresholds, and suppression of redundant native window movement.
- **2026-07-29** — Added SDUC-451 and SDTEST-1491/1492 after clarifying that
  companions are standalone interactive desktop characters, not AI Dock art.
  The overlay now accepts direct dragging across displays, bounded click and
  double-click reactions, and resumes event-driven roaming after interaction.
- **2026-07-29** — Added SDUC-450 and SDTEST-1489/1490 after live launch showed
  the character picker was technically present but hidden below theme controls
  and required a second enable toggle. File, palette, and tray routes now land
  directly on visible cards, and selection applies immediately.
- **2026-07-29** — Added SDUC-445..449 and SDTEST-1460..1488 for the native
  Clippy clipboard assistant, privacy and stale-selection contracts, selectable
  character persistence, deterministic desktop simulation, multi-display
  routing, pointer-interactive overlays, and honest Wayland fallback.
- **2026-07-29** — Hardened SDUC-228 and added SDTEST-1433: User-mode request
  polling now forces owner scope, while the dashboard independently filters
  counters and recent titles to the signed-in requester.
- **2026-07-29** — Added SDUC-454 and SDTEST-1430..1432 for typed
  natural-language workflow routing, exact target revalidation, and the
  existing review/confirmation boundaries.
- **2026-07-29** — Added SDUC-453 and SDTEST-1429 for typed Assistant shortcut
  behavior: immediate contextual submissions versus editable composer prefill.
- **2026-07-29** — Extended SDUC-285 and added SDTEST-1225: development
  builds without the embedded update-verification key now keep the updater
  silently disabled, while signed release builds retain strict verification.
- **2026-07-29** — Added SDUC-452 and SDTEST-1427/1428 for explicit
  conversational request preparation from both Assistant surfaces, with strict
  routing, normal-chat fallback, and the existing unsent review boundary.
- **2026-07-29** — Extended SDUC-414/418 and added SDTEST-1425/1426 for
  Markdown rendering of durable conversations and free-form read-only analyses,
  while structured and executable/editable outputs retain their typed/raw
  presentation.
- **2026-07-29** — Extended SDUC-438 and added SDTEST-1424 for the contextual
  Monolith motions used by AI generation, terminal startup, and site discovery.
- **2026-07-27** — Extended SDUC-432 and added SDTEST-1421..1423 for native
  capture annotation plus confirmed, coordinated deletion of posted request
  and Support images from both the discussion and Share storage.
- **2026-07-25** — Added SDUC-444 and SDTEST-1415..1420 for global-shortcut
  failure reporting, and renumbered the v0.6.4 shortcut-toast tests off
  SDTEST-1220/1221/1222, which were already held by the updater rows.

- **2026-07-25** — Added SDUC-441/442/443 and SDTEST-1200..1205 / 1210..1211
  for application chrome: proportional scaling of Workspace-drawn surfaces, the
  cross-mode application menu row, and the VS Code style sidebar rail.

- **2026-07-24** — Added SDUC-439 and SDTEST-1412/1413 for session-scoped SSH
  lifecycle reporting: protocol terminators, voluntary tab closes and clean
  remote exits stay silent, while unexpected transport loss identifies the
  exact connection.
- **2026-07-24** — Extended SDUC-429/434 and added SDTEST-1411 for live
  counter, pinned-connection and locale snapshots on the native tray owner
  thread across Linux, macOS and Windows.
- **2026-07-24** — Extended SDUC-434 and added SDTEST-1410 for the dedicated
  Retina macOS tray template generated from the canonical Monolith mark.
- **2026-07-24** — Extended SDUC-429 and added SDTEST-1407..1409 for the
  running AI-task tray count and direct single-instance Tasks-tab routing.
- **2026-07-24** — Extended SDUC-434/436 and SDTEST-1300/1302/1406 with
  complete FR/EN tray labels, live Linux relocalization, named Dock controls,
  an accessible palette search field, and bounded full-keyboard navigation.
- **2026-07-24** — Extended SDUC-406/407/434/435 and SDTEST-1320/1405
  for the idempotent `shelldeck://assistant` hand-off into the lightweight
  Dock runtime.
- **2026-07-23** — Extended SDUC-434 with immediate dynamic registration and
  unregistration plus persisted shortcut capture, validation, reset, visible
  native results, and asynchronous Wayland portal outcomes
  (SDTEST-1398..1404).
- **2026-07-22** — Extended SDUC-222/228 with the searchable request-site
  target and added SDTEST-1389/1390 for its wire contract and UI wiring.
- **2026-07-22** — Added SDUC-438 and SDTEST-1388 after auditing reachable
  dynamic icon names against the embedded Lucide subset.
- **2026-07-21** — Added SDUC-434..436 and SDTEST-1380..1386 for the standalone,
  single-instance AI Dock plus recoverable hidden startup.
- **2026-07-21** — Extended SDUC-432 and SDTEST-1373/1375/1377 to cover image
  replies and internal notes on Support tickets plus the Support requests composer.

- **2026-07-20** — Added SDUC-432 and SDTEST-1373..1375 for request image
  attachments, byte-signature validation, Share receipts, and the five desktop
  intake paths.
- **2026-07-20** — Added SDUC-429 and SDTEST-1367/1368 for the durable AI task
  center, legacy-draft migration, titlebar badge, target routing, and stop path.
- **2026-07-20** — Added SDUC-430 and SDTEST-1369/1370 for persisted
  per-capability autonomy and the non-bypassable high-risk confirmation rule.
- **2026-07-20** — Added SDUC-431 and SDTEST-1371/1372 for strict bounded
  Terminal diagnostic plans and separately confirmed read-only steps.
- **2026-07-21** — Added SDUC-433 and SDTEST-1376 for native wrapped-line
  cursor, selection, and caret-follow behavior in shared multi-line Inputs.
- **2026-07-24** — Amended SDUC-433 and SDTEST-1376 to explicitly cover
  mouse-wheel scrolling inside capped multi-line Inputs.
- **2026-07-24** — Corrected SDUC-151/152/309/310 to the deployed three-tier
  role model, capability-filtered actions, mandatory welcome screen, and
  cross-mode personal Settings surface.
- **2026-07-24** — Added SDUC-440 and SDTEST-1414 for role-specific User and
  Support home dashboards and capability-filtered onboarding.
- **2026-07-17** — Added SDUC-423 and SDTEST-1358/1359 for validated,
  explicitly confirmed AI priority and assignment triage.
- **2026-07-17** — Added SDUC-424 for non-submitting Support-to-request drafts.
- **2026-07-17** — Added SDUC-425 for bounded Terminal-to-request drafts.
- **2026-07-17** — Added SDUC-426 and SDTEST-1362/1363 for explicit,
  schema-validated naming of scripts, sessions, tunnels, and requests.
- **2026-07-17** — Added SDUC-427/428 and SDTEST-1364..1366 for typed,
  separately confirmed, bounded, and redacted AI actions.
- **2026-07-17** — Added SDUC-422 and SDTEST-1356/1357 for structured,
  non-submitting AI preparation in the New Request sheet.
- **2026-07-17** — Added SDUC-421 and SDTEST-1355 for virtualized User/Support
  request and ticket lists.
- **2026-07-16** — Added SDUC-419 and SDTEST-1352 after the macOS release
  matrix caught the platform-specific fourth `auto-launch` constructor
  argument.
- **2026-07-16** — Added § 22 contextual AI assistant (SDUC-413..416) and
  SDTEST-1338..1342 for fake-CLI connection tests, provider payload privacy,
  executable validation, stale-response rejection, and credential-free config.
  The connection test is diagnostic rather than a volatile per-process gate.
- **2026-07-16** — Added command-palette recently used ordering (SDUC-417,
  SDTEST-1343), capped at five commands per session.
- **2026-07-16** — Added integrated Support/Script AI draft workflows and
  persistent per-target pending drafts (SDUC-418, SDTEST-1344).
- **2026-07-16** — Completed phase 1 integrated AI analysis workflows:
  Support summary/triage and Script explanation/review (SDUC-418,
  SDTEST-1347).
- **2026-07-16** — Made integrated analyses read-only and internally
  scrollable, added inline AI generation to the Script form, and exposed the
  non-secret host directory to contextual AI (SDUC-415/418,
  SDTEST-1348/1349).
- **2026-07-16** — Structured Script-form generation now validates and fills
  name, description, language, category and body together, with one repair
  attempt for malformed provider output (SDUC-418, SDTEST-1350).
- **2026-07-16** — Added contextual Script correction after a failed latest
  execution; correction remains unsaved and never auto-runs (SDUC-418,
  SDTEST-1351).
- **2026-07-15** — Added § 21 Pinned connections (SDUC-411 persistence/sidebar,
  SDUC-412 dynamic tray routing). Tests SDTEST-1335..1337 cover backward
  compatibility and tray menu-id dispatch.
- **2026-07-15** — Added SDUC-410 dynamic terminal launchers: default
  shell always visible, Claude Code / Codex gated by executable discovery.
- **2026-07-15** — Added § 20 Recent activity (SDUC-408 durable JSONL
  store, SDUC-409 Dev surface with filters/search/open actions). Core
  tests SDTEST-1330..1332 cover the durable file contract.
- **2026-07-15** — Added § 19 deep links (SDUC-406 parse grammar,
  SDUC-407 single-instance + hand-off) for the `shelldeck://` companion
  feature. Tests SDTEST-1320..1323 in `config/{deep_link,single_instance}.rs`.
- **2026-07-07** — Initial catalogue.
- **2026-07-22** — Added SDUC-437 for the event-efficient workspace Git
  status used by the background companion mode.
- **2026-07-09** — Added SDUC-170/171/172 (Support timestamp aliases,
  Lucide channel mapping) and § 18 i18n (SDUC-400..405) following the
  rust-i18n landing (`.agents/i18n.md`, commits `ae99be5` +
  `0837c74` + `c1ef0f3` + `4bd6d21` + `f8c2ac5`).
- **2026-07-09 (later)** — Amended SDUC-060/061/300/301 wording after
  implementing SDTEST-034/036/1000-1024/1302. Contract corrections:
  `fuzzy_match` needle is NOT lowercased (caller's job);
  `fuzzy_match_indices` returns CHAR positions, not byte offsets;
  `substitute_variables` LEAVES missing placeholders unchanged instead
  of emitting empty.
- **2026-07-09 (D)** — Cluster D landed: SDTEST-030/032/033/037/044
  (validate_port, Connection accessors, ScriptLanguage runner_spec
  table, ExecutionRecord lifecycle). Introduced SDUC-104bis for the
  Connection accessor contract; SDUC-104 no longer conflates that
  with the cloud-sync merge rule. `display_name` fallback corrected:
  alias → hostname only, **no UUID fallback**.
- **2026-07-09 (E)** — Cluster E landed: SDTEST-016/017/018/019/020
  (parse_ls edges, nginx include tolerance + multi-name limitation,
  SyncProgress percent, rsync argv coverage). SDUC-076 amended: 
  `percent()` returns a percentage 0..=100, not a ratio 0..=1
  (initial catalogue was wrong).
- **2026-07-09 (F)** — Cluster F long-tail: SDTEST-035 (fence
  behaviour pinned as-is), SDTEST-038/039/040 (PM detect +
  dependency check + install lookup), SDTEST-042/043 (templates
  catalog invariants + `to_script`), SDTEST-069 (AppConfig defaults
  first-run pin). Closes the last "no-infra" pockets in
  `shelldeck-core::models` + `config::app_config`.
- **2026-07-09 (G)** — Cluster G cloud_sync P0: SDTEST-152/153/154
  (404/405 → GET fallback, 401 without retry). First mock-based
  cluster of the session; extends the zero-dep `TcpListener` pattern
  from `jean_fleet` / `issues` / `manage_support` to cover the sync
  entry point. SDTEST-154 is the load-bearing safety test — a bad
  token can never reach `merge_profiles` with an empty payload and
  silently prune every CloudSync connection.
- **2026-07-09 (H)** — Cluster H user/support priority list from
  reviewer: SDTEST-1052/184 (effective_mode truth table — non-super
  forced User), SDTEST-225 (7 support write body shapes + 401),
  SDTEST-295 (create_issue source elision), SDTEST-1053/1057
  (can_switch predicate — palette leak fix drafted), SDTEST-1054/185
  (JeanConfig::resolve_effective precedence), SDTEST-227/228
  (support agents empty + list order preserved), SDTEST-298
  (dispatch_issue instance_id body), SDTEST-246 (format_via_shelldeck
  prefix shape). Ported 4 pure fns to `shelldeck-core` (`AppMode::can_switch`,
  `AppMode::resolve_effective`, `JeanConfig::resolve_effective`,
  `format_via_shelldeck`) so the truth tables are testable outside
  GPUI. Workspace delegate call-sites drafted in the working tree,
  land in a follow-up commit once the concurrent i18n WIP merges.
- **2026-07-09 (I)** — Cluster I `known_hosts` (SDTEST-580..585 +
  bonus). Extracted `check_known_host_in(contents, …)`,
  `build_known_host_line(…)`, `add_known_host_to(path, …)` as pure
  fns testable without `$HOME` mutation (parallel-safe). MITM sensor
  + append-never-overwrites property. Full SSH FakeTransport for
  session/pool/tunnel deferred → [`INFRA_BLOCKED.md`](./INFRA_BLOCKED.md).
- **2026-07-09 (J)** — Cluster J release contract (SDTEST-1200..1203,
  SDTEST-1260/1261). `platform.rs` OS/arch key format + `darwin-*`
  forbidden; `include_str!`-based parity check between
  `release.yml`, worker `index.ts`, and runtime `current_platform()`.
  AutoUpdater cadence + hash-verify need injectable clock/HTTP →
  [`INFRA_BLOCKED.md`](./INFRA_BLOCKED.md).
- **2026-07-09 (K)** — Cluster K PTY Unix smoke (SDTEST-960/962/963/965/966,
  `#[cfg(all(test, unix))]`). Spawn/echo round-trip/resize/exit-code
  on Linux CI. macOS/Windows deferred (CI matrix) →
  [`INFRA_BLOCKED.md`](./INFRA_BLOCKED.md). Zombie-on-drop
  (SDTEST-967) needs impl decision — deferred.
- **2026-07-09 (L)** — Cluster L keychain (SDTEST-120/123/124). Pure
  key builders (`entry_key`, `passphrase_entry_key`) + hostile
  namespace-isolation test (SSH key path spelling out `user@host`
  proves the `passphrase:` prefix is load-bearing). Live smoke gated
  by `SHELLDECK_LIVE_KEYCHAIN=1`. macOS/Windows deferred (CI matrix)
  → [`INFRA_BLOCKED.md`](./INFRA_BLOCKED.md).
- **2026-07-09 (M)** — Cluster M long tail: SDTEST-084 (store mix),
  SDTEST-106/108 (ssh_config Include + never-writes),
  SDTEST-130/131/132 (themes builtins + fallback + fields),
  SDTEST-045/046/047 (ManagedSite constructors + url elision),
  SDTEST-155/156 (cloud_sync tags overwrite policy + no-dup).
  Contract correction SDUC-102: cloud is authoritative on tags,
  local additions ARE overwritten (initial inventory said
  "preserves" — aspirational, reality tested and locked).
