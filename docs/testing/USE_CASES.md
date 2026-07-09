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

### SDUC-023 — Terminal session ties PTY to grid via async pipe

`TerminalSession::spawn_local` boots the PTY, forwards output into the
grid via the parser, and drives repaints via the output-notifier
channel (event-driven, **not** polled).

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
405 falls back to `GET`. Any other error surfaces to the caller.

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
Dev when the field is absent.

### SDUC-152 — Mode enforcement per role

Non-superadmin users are forced to User mode regardless of the
persisted value. Only `can_switch_mode()` (signed-in superadmin) may
change modes.

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
`source = "user" | "support"`.

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
and create composer; the composer respects `IssueField` keyboard focus.

### SDUC-229 — Support "Requests" section

`SupportView` gains a `Requests` tab distinct from Tickets, with a
staff bar exposing status / priority / assign / dispatch / github when
the user is `issues_staff`.

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
No half-installed state on failure.

### SDUC-285 — Auto-update disabled respects setting

`set_enabled(false)` cancels the poll task and future manual
`check_for_update` no-ops until re-enabled.

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

`Workspace::effective_mode()` — logged out → Dev; superadmin →
persisted; non-superadmin → forced User (matches SDUC-152).

### SDUC-310 — Active view mode switch preserves terminal tabs

Switching between Dev / User / Support hides the Dev surface without
destroying terminal sessions (SDUC-023 must not be interrupted).

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

## Retired use cases

*(none yet)*

---

## Change log

- **2026-07-07** — Initial catalogue.
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
