# Session daemon (shelldeck-sessiond)

Status: design for branch `feat/session-daemon`. Section 14 is the planned
rollout. Section 15 is the record of what exists: it starts with a status
summary (implemented now, deferred) and lists every difference from the
sections above.

## 1. Goals, non-goals, relation to tmux mode

Today persistence and splits on SSH hosts rely on tmux drawing panes as text
inside one PTY (`crates/shelldeck-ssh/src/tmux.rs`). The app cannot scroll,
select or lay out those panes itself, and local terminals die with the app.

Goals:

- A per-user session server owns the PTYs, one PTY per pane, and is the source
  of truth. It parses output to keep authoritative screen state and forwards
  the raw byte stream to clients.
- Each client parses the stream with its own emulator: scrolling, selection
  and rendering are local, and nothing is re-encoded on the way.
- Attach is fast and exact: binary snapshot, READY marker, live bytes, with
  scrollback backfilled in the background, newest first.
- Tabs and splits are drawn by the client (the native `TabLayout` tree). Many
  clients may attach to one pane.
- One binary serves local sessions (they survive app quit and updates) and
  remote sessions (tunnelled over the existing SSH connection, no new port).

Non-goals:

- No TCP listener, no cross-user sharing, no authentication beyond "same uid".
- No server-drawn layout, status line or key bindings.
- No rendering of sixel or kitty graphics in this work. The daemon passes the
  bytes through untouched, but the client emulator still discards DCS
  (`parser.rs` hook/put/unhook are empty). Rendering them is a separate task.
- No Windows daemon. All daemon code is `#[cfg(unix)]`; the binary has a
  non-unix stub `main` that returns failure, like `shelldeck-workspace-helper`.
- No change to the tmux code path.

Relation to tmux mode: `open_tmux_shell`, `TmuxClient`, `set_tmux_control` and
the `is_tmux_managed()` branches stay as they are. The backend for an SSH tab
is chosen in `connect_ssh_at` where `attach_tmux` is computed today:
daemon if enabled and available on the host, else tmux if
`general.auto_attach_tmux` is on, else a plain shell. A daemon failure never
blocks the connection: it degrades to the next choice.

## 2. Crate layout

New workspace member `crates/shelldeck-sessiond`:

| Item | Content |
|---|---|
| `src/protocol.rs` | Frame codec, limits, typed messages, bounded decoder. Portable. |
| `src/client.rs` | Async client over any `AsyncRead + AsyncWrite` (Unix socket, SSH exec stream). Portable. |
| `src/server/` | Daemon: listener, pane registry, PTY threads. `#[cfg(unix)]`. |
| `src/proxy.rs` | stdio to Unix socket bridge, with auto-start. `#[cfg(unix)]`. |
| `src/bin/shelldeck-sessiond.rs` | Thin `main`: `serve`, `proxy`, `--version`. |

It depends on `shelldeck-terminal` for the parser and grid, used headless:
`TerminalGrid::with_scrollback` + `TerminalProcessor::process_bytes` with a
`vte::Parser` per pane, and `LocalPty::spawn_at` + `into_parts()` for the PTY.

`shelldeck-terminal` currently depends on `shelldeck-core` (reqwest, keyring,
notify and more) for three small items: `util::home_dir`,
`util::executable_on_path` and `tmux::TmuxAction`. The prototype accepts that
link. Before any remote binary is shipped, `shelldeck-terminal` gets a default
feature `app` that owns the `shelldeck-core` dependency and `TerminalSession`;
the daemon builds with `default-features = false` and `pty.rs` uses local
helpers. This is a hard prerequisite for phase 3 (small static binary).

New code inside `shelldeck-terminal`:

- `grid/snapshot.rs` (child module of `grid`, so it can read private fields):
  `TerminalGrid::snapshot()`, `TerminalGrid::restore()`,
  `TerminalGrid::history_rows()` and `TerminalGrid::prepend_history()`.
- `boundary.rs`: `StreamBoundary`, a small byte classifier that tells whether
  the parser is at ground state (section 5).
- `pty.rs`: `LocalPty::spawn_command(argv, env, cwd, rows, cols)`. Today
  `spawn_at` only takes a shell path and hardcodes `TERM`, so it cannot carry
  the `Spawn` frame. `PtyMaster` gains `process_id()` and `try_wait()`:
  today the child is private and "never read", so the daemon could not
  report an exit status.
- `lib.rs`: `pub const EMULATOR_REV: u16`, bumped by any change to the parser
  or grid that alters grid state for the same bytes (section 4, version
  skew).
- `session.rs`: a third constructor `TerminalSession::spawn_attached`
  (section 10).

## 3. Process model

### Daemon lifecycle

- One daemon per user per host. Started on demand by the first client (the
  app locally, `shelldeck-sessiond proxy` remotely): on `ENOENT` or
  `ECONNREFUSED` the client spawns `shelldeck-sessiond serve` and retries the
  connect for up to 3 s. The binary is always an absolute path (the proxy
  uses its own `current_exe`, the app its bundled binary), never a `PATH`
  lookup.
- Detach, done single-threaded before anything else: double fork (the
  starter never waits on it), `setsid`, `chdir` to home, stdio to
  `/dev/null`, close every fd above 2 (an inherited SSH exec pipe would keep
  the channel open forever), log to `<run dir>/sessiond.log` (size capped,
  rotated once). The process umask is not changed (panes inherit it).
- Environment scrub at start, so panes do not inherit the state of whichever
  client happened to start the daemon: `SSH_AUTH_SOCK`, `SSH_CONNECTION`,
  `SSH_CLIENT`, `SSH_TTY`, `DISPLAY`, `WAYLAND_DISPLAY`, `XAUTHORITY`,
  `TMUX`, `TMUX_PANE`, `TERM`, `COLORTERM` and `SHELLDECK_*` are removed.
- Single instance: the daemon takes an exclusive `flock` on
  `<run dir>/sessiond-v1.lock` and keeps it for its lifetime. Only the lock
  holder may unlink a stale socket and bind. A second `serve` exits 0.
- Idle exit: with zero panes (dead panes count until they expire) and zero
  clients for 60 s the daemon leaves. It first renames its socket to
  `sessiond-v1.sock.leaving`, so no new connection can reach the listener,
  waits 30 ms and reads the listener once more: a client whose `connect`
  had already found the socket is in the backlog by then. If there is one,
  the socket gets its name back and the daemon keeps serving. Otherwise the
  socket is removed and the daemon exits. A connection is therefore either
  served or finds no socket; it is not accepted into a backlog that is then
  closed.
- The client side keeps its own guard for daemons of earlier releases: a
  connection that ends before `HelloAck` (EOF or reset) is treated as "not
  running" and the daemon is started once more, and the start is repeated
  every 250 ms while the socket stays missing.
- Lock handoff: the instance lock is released (an explicit unlock) as soon
  as the socket is unlinked, before the grace period below, because from
  that moment the leaving daemon never touches the run directory again. A
  `serve` that finds the lock taken and no socket waits for the lock (up to
  5 s, polling every 10 ms) instead of exiting: the holder is either
  leaving or has not bound yet (then its socket appears and the waiting
  `serve` exits 0). A client that starts a daemon while the old one is
  still ending its panes is served at once. Side effect: clients repeat
  the start while the socket is missing, so several `serve` processes can
  wait at once; all but one exit when the socket appears. If the daemon
  that won is stopped within the 10 ms poll of a waiting one, that one
  takes over and, with nothing to serve, leaves by idle exit a minute
  later.
- Signals: `SIGTERM` unlinks the socket, releases the lock, closes every
  PTY master (the kernel hangs up each pane's session), waits up to 3 s,
  sends `SIGKILL` to the process group of every pane child still alive,
  then exits. The existing `ChildReaper` is not enough here: it reaps on a
  detached thread that dies with the process and it only kills the direct
  child.
- Descriptor limit: the daemon raises its own soft `RLIMIT_NOFILE` to 4096
  (or the hard limit if lower); 256 panes at four descriptors each (the
  master, one for the reader, one for the writer and the PTY library's
  writer) do not fit 1024. A limit is inherited, and programs behave differently under a
  high one, so every pane child starts with the limit the daemon was
  started with: the PTY library has no hook between `fork` and `exec`, so
  the daemon starts each pane through its own executable,
  `shelldeck-sessiond exec-pane --nofile N [--program PATH -- ARGV]`, which
  sets the limit and replaces itself with the program (same pid, so the
  process group and the exit status are the program's). On Linux the
  executable is `/proc/self/exe`, which stays valid when an update replaces
  the file. The program is looked up before the spawn, so a missing one is
  still `SpawnFailed`. If the limit could not be raised, or the executable
  is gone (other systems), panes are started directly.
- Log: `<run dir>/sessiond.log`, mode 0600, opened by the daemon once it
  holds the lock and before it listens. 1 MiB, then renamed to
  `sessiond.log.1` (one old generation). It holds lifecycle events only
  (start, stop, pane spawned and ended, limits hit, history trimmed, panic
  locations), never pane output, input, environment or command lines. A
  daemon that has given its directory up (lock released) writes nothing
  more to it.
- `SIGHUP` has a no-op handler. It is never set to `SIG_IGN` and no signal is
  blocked on the spawning thread: ignored dispositions and masks survive
  `exec`. portable-pty also resets both in `pre_exec`; a test pins it
  (`SigIgn` and `SigBlk` of a pane shell in `/proc/<pid>/status`).
- Detach never closes a PTY master, so a pane child never sees `SIGHUP`
  because a client went away. The master is closed only by `Kill`, pane
  expiry or daemon exit.
- Panic isolation: the workspace keeps `panic = unwind`. Parsing runs inside
  `catch_unwind` per pane; a panic marks that pane dead (`Exited`, signal 0,
  exit code -1), hangs up its child and leaves the other panes alone.
- Daemon crash (`SIGKILL`, OOM, abort): the masters close, the kernel sends
  `SIGHUP` to each pane's foreground group. Processes that ignore it are
  orphaned exactly as after a tmux server crash; the next daemon does not
  try to adopt them.

### Socket path and permissions

- Run directory: `~/.shelldeck/run/<machine>/`, created with mode 0700.
  `<machine>` is `m-` and 16 hex digits: the first 8 bytes of a keyed
  SHA-256 of the machine identity (`/etc/machine-id`, else
  `/var/lib/dbus/machine-id`, else `kern.uuid` on macOS). It does not
  change with the host name, and two machines that share an NFS home get
  two directories, so they do not unlink each other's live socket. The
  identity itself is not stored. Without a usable identity the name is
  `gethostname` reduced to `[A-Za-z0-9.-]`, at most 64 bytes, as in the
  first prototype.
- A daemon in another directory is still found: when no socket exists in
  the directory for this machine, the other directories under
  `~/.shelldeck/run/` are tried (at most 32, same checks) and one where a
  daemon answers is used instead (`paths::live_run_dir`). This covers a
  daemon started by a release that named the directory after the host
  name. A socket left by another machine refuses the connection and is
  never picked.
- Socket: `<run dir>/sessiond-v1.sock`. The 0700 directory is what protects
  it during `bind`; the daemon then sets and verifies mode 0600. No `umask`
  call (it is process-wide and would leak into panes).
- `XDG_RUNTIME_DIR` is deliberately not used: logind removes `/run/user/<uid>`
  when the last login session ends, which would orphan a running daemon on
  exactly the SSH disconnect it is meant to survive.
- If the path exceeds `sun_path` (108 bytes on Linux, 104 on macOS) the
  fallback is `/tmp/shelldeck-<uid>/sessiond-v1.sock`.
- Path checks, done by the daemon before bind AND by every client and the
  proxy before connect: `lstat` the run directory, require a real directory
  (no symlink), owner equal to the caller's euid, no group or other bit;
  then require the socket to be owned by that uid. Any failure is fatal for
  the daemon path (fallback, section 12), never "fix and continue".
- Peer check, both directions: the daemon verifies every accepted connection
  with `SO_PEERCRED` (Linux) or `getpeereid` (macOS) and closes a peer whose
  uid differs from its euid before reading a byte. The client and the proxy
  do the same on the connected socket before sending `Hello`. Without the
  client-side checks another local user could pre-create the `/tmp` fallback
  and collect keystrokes; with them the worst case is a denied daemon.
- Tests pass `serve --dir <path>` and `proxy --dir <path>`. The same checks
  apply to that directory. The app never puts `--dir` in a remote command.

### Panes, groups, what survives

- Pane = one PTY + one child process + one headless grid + one parser + one
  output byte counter. Identified by a daemon-generated UUID.
- Grouping metadata per pane: `group` (UUID chosen by the client, one per
  tab), `label` (client text, at most 255 bytes), `created_at`. The daemon
  stores it and returns it in the pane list; it never interprets it. The
  split tree, ratios and focus stay client-side (section 10).
- Client quit, crash, SSH drop, app update: the connection closes, its
  attachments are removed, panes keep running. Nothing else changes.
- Pane exit is defined by the child's `wait`, not by PTY EOF (a background
  job can hold the slave open). After `wait` returns the reader drains until
  EOF, `EIO` or 250 ms, then `Exited` is queued, so it always follows the
  final output. The master is then closed.
- The dead pane stays listed with its final screen and exit status until a
  client sends `Kill` (which then acts as "close") or one hour passes, so a
  client that reattaches can still show the result.
- Memory: history is the only large state (about 80 bytes per cell). Default
  `scrollback_max` is 10 000 rows as in the app, the cap is 100 000. Once a
  second the daemon adds up `history rows x cols x size_of::<Cell>()` over
  all panes; above 2 GiB (`serve --history-budget BYTES`, with a K, M or G
  suffix) it halves the history of the largest pane, panes without
  attachments first (`set_max_scrollback` keeps the newest rows), until it
  is under budget again or every history is down to one row. A trimmed
  pane keeps the lower limit. Screens are never touched.
- Lost on daemon exit, host reboot, or `KillUserProcesses=yes` at logout:
  everything (same limits as tmux).

### Upgrade story

- The protocol major version is in the socket name (`-v1`), and `Hello`
  negotiates the version and feature bits. A v2 protocol gets a new socket
  and a new daemon; v1 is never widened in place. Additive v1 frames are
  allowed only behind a feature bit that both sides announced.
- A running daemon is never replaced while it holds panes. A newer app talks
  v1 to an older daemon and uses only the features it announced. The
  version string in `HelloAck` is informative: only the protocol version
  decides.
- A daemon that answers `Hello` with `UnsupportedVersion` (or acknowledges
  another protocol version) was left by a version of the app this one
  cannot speak to. The app sends it nothing more, starts no daemon next to
  it and does not stop it. Sessions for its panes report "managed by
  another version" and keep their pane id in the saved layout; new
  terminals are ordinary in-process ones until that daemon has ended by
  itself.
- Emulator skew: client and daemon parse the same bytes with possibly
  different builds of `shelldeck-terminal`. `HelloAck` carries the daemon's
  `EMULATOR_REV`. When it differs from the client's, the daemon's grid is
  authoritative and the client refreshes from it: it re-attaches after every
  applied `Resize`, and after 2 s without output if any output arrived since
  the last snapshot (at most once per 10 s per pane). A refresh that missed
  no bytes keeps the client's history (section 5).
- When a v2 protocol exists, the app keeps the v1 client for at least one
  release so panes held by a v1 daemon stay reachable.
- The old daemon ends by idle exit, after which the next start uses the new
  binary. The UI may offer "restart session server" (explicit, kills panes).
- Later (phase 4): seamless upgrade by re-exec. The daemon serializes each
  pane with the snapshot codec, keeps the PTY master fds open across `exec`,
  and the new binary adopts them.

## 4. Wire protocol v1

### Framing

```
u32 big-endian length | u8 version (=1) | u8 type | body
```

`length` counts version, type and body: minimum 2, hard cap
`MAX_FRAME_BYTES = 1 MiB`, both checked on encode and on decode before any
payload is buffered. Each type has its own smaller cap below; where a field
list could exceed the frame cap, the frame cap wins. `version` is the
negotiated protocol (1, also for `Hello`).

Partial frames: the decoder is incremental and holds at most one frame. Once
the first byte of a frame has arrived the rest must arrive within 30 s, and
a connection with pending outbound data whose socket accepts no byte for
30 s is closed. Both only detach. The proxy is a byte pipe and never looks
at frame boundaries; a transport that dies mid-frame is a closed connection.

Integers are big-endian. Strings are `u16 length +
UTF-8`. Byte blobs are `u32 length + bytes`. UUIDs are 16 raw bytes. Decoding
uses a bounded typed decoder that rejects trailing bytes (same style as
`workspace_helper.rs`).

Fail closed: wrong version byte, unknown type, a type sent in the wrong
direction or before the handshake, an oversized frame or field, invalid
UTF-8, or trailing bytes make the receiver send `Error(BadFrame)` when it
still can and close the connection. A closed connection only detaches; panes
are not affected.

### Handshake

The first frame on a connection must be `Hello`, answered by `HelloAck`
within 10 s. The daemon picks the highest protocol version in the client's
range (only 1 exists). Anything else before `HelloAck` closes the connection.

### Frames, client to daemon

| Type | Name | Fields | Limits |
|---|---|---|---|
| 0x01 | Hello | magic `SDSD` (4 bytes), `proto_min` u16, `proto_max` u16, `features` u32, `client_version` string | string 64; frame 128 |
| 0x02 | ListPanes | none | frame 2 |
| 0x03 | Spawn | `request_id` u32, `group` uuid, `label` string, `rows` u16, `cols` u16, `scrollback_max` u32, `cwd` string (empty = home), `argv` (u8 count + strings, empty = login shell), `env` (u8 count + name/value strings) | label 255; cwd 4096; argv 64 x 4096; env 32 entries, name 64, value 1024; rows and cols 1..=1000; scrollback at most 100000; frame 64 KiB |
| 0x04 | Attach | `pane` uuid, `attach_id` u32 (unique per connection, increasing), `rows` u16, `cols` u16, `scrollback_rows` u32, `flags` u8 (bit 0 = read only) | frame 64; on a pane this connection already has attached it replaces that attachment (refresh, section 5); `rows` = `cols` = 0 means "keep the pane's current size" (section 7), any other value outside 1..1000 is refused |
| 0x05 | Input | `pane` uuid, `bytes` blob | blob 64 KiB |
| 0x06 | Resize | `pane` uuid, `rows` u16, `cols` u16 | 1..=1000 each |
| 0x07 | Detach | `pane` uuid | frame 32 |
| 0x08 | Kill | `pane` uuid, `signal` u8 | signal in {1 HUP, 15 TERM, 9 KILL}, sent to the pane child's process group; on a dead pane it removes the pane |

### Frames, daemon to client

| Type | Name | Fields | Limits |
|---|---|---|---|
| 0x81 | HelloAck | `proto` u16, `features` u32, `daemon_version` string, `instance` uuid, `max_frame` u32, `emulator_rev` u16 | string 64 |
| 0x82 | PaneList | u16 count, then per pane: `pane`, `group`, `label`, `title` (OSC 0/2), `cwd` (OSC 7), `rows`, `cols`, `created_at` u64 (unix seconds), `alive` u8, `exit_code` i32, `clients` u16, `offset` u64 | at most 256 panes; title 255; cwd truncated to 1024 (so 256 panes always fit); frame 1 MiB |
| 0x83 | Spawned | `request_id` u32, `pane` uuid, `pid` u32, `cwd` string (canonical path the daemon opened) | cwd 4096 |
| 0x84 | Snapshot | `pane`, `attach_id` u32, `seq` u32, `flags` u8 (bit 0 = last), `bytes` blob | blob 256 KiB; whole snapshot 32 MiB |
| 0x85 | Ready | `pane`, `attach_id`, `offset` u64, `rows` u16, `cols` u16 | |
| 0x86 | Output | `pane`, `attach_id`, `offset` u64 (stream offset of the first byte), `bytes` blob | blob 64 KiB |
| 0x87 | Resize (applied) | `pane`, `attach_id`, `offset` u64, `rows` u16, `cols` u16, `pulled` u16 (rows the daemon's resize moved from history back onto the screen) | |
| 0x88 | ScrollbackChunk | `pane`, `attach_id`, `start` u32 (rows already sent for this attachment; the client checks it against its own count), `count` u16, `flags` u8 (bit 0 = last), `bytes` blob (rows, newest first) | blob 256 KiB |
| 0x89 | Exited | `pane`, `offset` u64 (final), `exit_code` i32, `signal` u8 | |
| 0xff | Error | `request_id` u32 (0 if none), `pane` uuid (nil if none), `code` u16, `retryable` u8 | no free text |

`Resize` exists in both directions: 0x06 is a client request, 0x87 is the
daemon announcing the size it applied, positioned in the output stream.

Error codes: 1 BadFrame, 2 UnsupportedVersion, 3 NoSuchPane, 4 SpawnFailed,
5 Resync, 6 Busy, 7 LimitExceeded, 8 NotAttached, 9 ReadOnly, 10 Internal.
An unknown code received by a client is treated as fatal for the connection.

Daemon limits: 256 panes, 32 client connections, 16 attachments per pane.
Exceeding one returns `Error(LimitExceeded)`. Memory per connection: 4 MiB
of queued `Output` per attachment (section 5) and 64 MiB of queued snapshot
and scrollback bytes in total; an `Attach` that would exceed the latter gets
`Error(Busy, retryable)`.

### Spawn: no generic command field

`Spawn` is the only frame that starts a process and it is fully typed:

- `argv`: explicit vector, executed with `execvp` semantics, never through a
  shell. Empty means the user's login shell (passwd entry, then `$SHELL`),
  started as a login shell (`argv[0]` prefixed with `-`) so its profile
  rebuilds `PATH` instead of inheriting the starter's.
- `cwd`: the daemon opens it, requires a directory, and returns the canonical
  path in `Spawned`. Failure is `Error(SpawnFailed)`; there is no fallback
  to another directory.
- `env`: allowlist of names. Accepted: `TERM`, `COLORTERM`, `LANG`,
  `LC_*`, `TERM_PROGRAM`, `TERM_PROGRAM_VERSION`, `SSH_AUTH_SOCK`,
  `DISPLAY`, `WAYLAND_DISPLAY`, `XAUTHORITY` and `SHELLDECK_*`. Any other
  name fails the frame. Everything else comes from the daemon's scrubbed
  environment (section 3). `TERM` defaults to `xterm-256color`. The local
  app passes its own display and agent variables; a remote client has none
  to pass, so agent forwarding into daemon panes is not supported in v1.

The peer is the same uid, so this is not a privilege boundary. The point is
that no frame carries text that is interpreted as shell source, and that the
client cannot poison variables such as `LD_PRELOAD` or `PATH` by accident or
through a confused deputy.

## 5. Attach handoff

### Mechanism

Each pane has a monotonically increasing byte counter `offset`: the number of
PTY output bytes the daemon has read and parsed since spawn.

Per pane, one mutex guards `{grid, parser, boundary, offset, subscribers}`.

- PTY reader thread: `read` up to 64 KiB, take the pane lock, feed the bytes
  to the parser, advance `offset`, push `Output{offset_before, bytes}` to
  every subscriber queue, release the lock.
- Attach: take the pane lock; if this attachment becomes the size owner
  (section 7) apply its size first, so the snapshot is taken at the final
  size; serialize the snapshot (a memory walk of the grid, no I/O), record
  `snapshot_offset = offset`, and enqueue the `Snapshot` chunks and
  `Ready{offset = snapshot_offset}` on the subscriber queue before
  registering it; release the lock.
- One FIFO per attachment carries everything for it: `Snapshot`, `Ready`,
  `Output`, applied `Resize`, `ScrollbackChunk`, `Exited`. All of them are
  enqueued under the pane lock, which is what makes their order equal to
  stream order. Queues never block: the reader thread waits only on the PTY
  and the pane lock, never on a client, so a slow or stopped client cannot
  stall a pane or another client.
- One writer task per connection drains its attachment queues round-robin,
  at most 64 KiB per turn, so a flooding pane cannot starve the other panes
  that share the connection (remote: all panes of a host).
- The daemon always reads the PTY at full speed. The child is never
  throttled by clients; overload is handled per client by resync (below).

Ordering guarantee: for one `(pane, attach_id)` the client receives every
`Snapshot` chunk, then `Ready`, then `Output` and applied `Resize` frames in
stream order. The snapshot reflects exactly bytes `[0, snapshot_offset)`.
Client rule:

- `Output.offset + len <= expected`: drop (already reflected).
- `Output.offset < expected < offset + len`: apply only the suffix.
- `Output.offset == expected`: apply, `expected += len`.
- `Output.offset > expected`: gap, the client is out of sync: restart attach.
- Applied `Resize` and `Exited` must carry `offset == expected`, else
  restart attach.

With the subscriber registered under the same lock as the snapshot, the first
`Output` normally starts exactly at `snapshot_offset`. The drop and trim rules
make the client correct regardless of that implementation detail (for example
a later broadcast-channel implementation that subscribes first).

Why this avoids a long PTY pause: the PTY read is blocked only while the pane
lock is held, which is the in-memory serialization of the visible screen and
modes (a few hundred microseconds to a few milliseconds). Sending the
snapshot, which can be slow over SSH, happens after the lock is released,
while live output accumulates in that client's queue with offsets. Scrollback
is not part of the locked section at all.

Frames carry `attach_id` (chosen by the client, new for each attach), so
anything still in flight from a previous attach of the same pane is dropped.

Refresh: an `Attach` for a pane the connection already has attached replaces
the attachment in place. Frames already queued for the old `attach_id` stay
ahead in the same FIFO, followed by the new snapshot. A client that asked for
a refresh while in sync keeps applying old frames until the first frame of
the new `attach_id`. If its `expected` then equals `Ready.offset` it missed
nothing: it replaces the screen and keeps its history, and the daemon sends
no backfill (the client says so with `scrollback_rows = 0`). In every other
case (gap, overflow, reconnect) the client drops its history on restore and
backfills again, because rows scrolled off during the missed bytes would
leave a hole.

### Mid-sequence boundary

A fresh client parser starts in ground state, but the daemon's parser may be
inside an escape sequence, an OSC string or a multi-byte UTF-8 character at
`snapshot_offset`. vte 0.13 does not expose its state, so `StreamBoundary`
tracks it next to the parser: ground, escape, CSI, string (OSC, DCS, SOS, PM,
APC) and UTF-8 continuation. It keeps `tail`: the bytes received since the
parser last was at ground, excluding C0 controls that were already executed
inside an escape or CSI sequence (those took effect on the grid and are not
part of the sequence).

The snapshot carries `tail`. After `restore`, the client feeds `tail` to its
new parser, then the live stream.

The tail never delays or refuses an attach. Waiting for a ground boundary
would make a pane unattachable after a stray `ESC P` or `ESC ]` (for example
`cat` of a binary file followed by silence). `StreamBoundary` keeps at most
4 KiB. Past that it replaces the tail with a canonical prefix that puts a
fresh parser in the same kind of state:

| State at the cut | Prefix sent as tail |
|---|---|
| DCS | `ESC P` |
| SOS, PM, APC | `ESC X` |
| OSC | `ESC ] 7999 ;` (a number the parser ignores) |
| CSI | `ESC [ 0 <` (CSI ignore) |
| Escape with intermediates | `ESC SP` |

The string then ends on both sides at the same byte. The only loss is the
effect of that one oversize sequence on that client (DCS is discarded by the
parser anyway and vte caps an OSC payload at 1024 bytes), healed by the next
snapshot.

### Resync

Loss of sync restarts the attach: the client sends `Attach` again with a new
`attach_id` and replaces grid content from the new snapshot. It keeps its
scroll offset (clamped while history refills) and clears its selection.
Triggers:

- an offset gap or a frame that fails validation for that pane;
- `Error(Resync)` from the daemon. The daemon sends it when the queued
  `Output` of one attachment exceeds 4 MiB (slow client): it frees that
  queue, stops sending output for the attachment, and leaves the PTY and
  the other clients alone. The connection stays registered as a writer of
  the pane until its next `Attach` or its disconnect: `Input` and `Resize`
  are still accepted, so Ctrl-C reaches a pane that is flooding even while
  the client waits to reattach. It is not counted in `clients`;
- the client's own feed for one pane exceeding 8 MiB (parser slower than the
  link): it drops that pane's pending bytes and reattaches. The connection
  reader itself never stops reading, it only demultiplexes into per-pane
  feeds;
- an applied `Resize` the client cannot reproduce (section 7);
- transport reconnect (SSH `ConnectionLost`, daemon restart).

The client waits before a reattach caused by overload: 250 ms, doubling to
2 s, reset after 10 s in sync. It keeps showing the last screen meanwhile.

A flood such as `yes` over a slow link therefore turns into periodic fresh
snapshots for that one client, never into unbounded memory or a stalled
connection. A client that stops reading entirely (stopped process, dead
link) costs at most its queue caps until the 30 s write timeout closes it.

## 6. Snapshot format

Decision: structured grid state, binary, produced by
`TerminalGrid::snapshot()` and applied by `TerminalGrid::restore()`.

Why not synthesized escape sequences:

- They are lossy for this grid: CUP clears `pending_wrap`, DECSTBM homes the
  cursor, soft-wrap flags survive only if rows are replayed through autowrap,
  DEC special graphics cells are stored already translated, and OSC 133 state
  cannot be replayed without side effects (133;A pushes `prompt_lines`, 133;D
  bumps `prompt_mark_sequence`).
- They are not cheaper to build: both approaches need new code inside
  `grid.rs` because most of the fields are private without getters.
- They cost a full parse on the client, and the alt screen needs the saved
  primary screen as well, which a replay can only approximate.
- A structured snapshot can be compared byte for byte, which gives the tests
  an exact equality check between server and client grids.

Encoding: hand-rolled, no serde (the crate has none and `Cell` has no
derives). Layout: `u8 format (=1)`, then tagged sections `u8 tag | u32 length
| body`. Unknown tags fail the restore (fail closed); a new section needs a
feature bit. Sections:

| Tag | Content |
|---|---|
| 1 Size | rows, cols |
| 2 Cursor | row, col, visible, shape, blink, `pending_wrap`, `saved_cursor` (row, col) |
| 3 Pen | `current_attrs`, `current_fg`, `current_bg`, active hyperlink, `last_char` |
| 4 Modes | origin, auto wrap, mouse mode, mouse encoding, bracketed paste, application cursor keys, application keypad, focus reporting, synchronized output, alt screen active |
| 5 Region | `scroll_top`, `scroll_bottom`, tab stops (bitset) |
| 6 Charsets | G0, G1, active |
| 7 Screen | visible rows: per row line flags + run-length encoded cells |
| 8 SavedPrimary | present only while alt is active: its own row count and column count, then `alt_cells`, `alt_cursor`, `alt_line_flags` (the saved primary; `cells` is then the alt screen) |
| 9 Meta | title, working directory (OSC 7), palette overrides (OSC 4) |
| 10 Prompt | `prompt_mark`, `prompt_mark_sequence`, `command_output_start`, `prompt_lines` (OSC 133) |
| 11 Tail | unconsumed parser bytes (section 5) |
| 12 Links | string table of hyperlink URLs (u16 count, each at most 2048 bytes), referenced by cells and by the pen |

Cell encoding: a run is `u16 count` + one cell (`char` as u32, combining
count + chars, fg, bg, attribute bits, underline style, underline colour,
hyperlink as a u16 index into Links with 0xffff = none, width: normal, wide
or spacer). Run equality is a field-by-field compare written in
`snapshot.rs` (`Cell` has no `PartialEq`). Blank default cells compress to
one run per row.

Fidelity rules found by reading `grid.rs`:

- `resize` does not touch `alt_cells`, so after a resize inside the alt
  screen the saved primary has the old dimensions. SavedPrimary therefore
  carries its own size and is restored as is. Fixing that grid bug is not
  part of this work; reproducing it keeps both sides identical.
- `command_output_start` is an index into history + screen and
  `prompt_lines` are screen rows. The client's history length differs from
  the daemon's, so `command_output_start` is encoded as a signed row
  relative to the first screen row and converted with the client's own
  history length on restore.
- Per-cell `attrs.hyperlink` and `attrs.underline_color` exist and are
  encoded (above). The pen's hyperlink uses the same table.
- A snapshot taken while synchronized output (mode 2026) is on carries the
  flag; the client still paints the restored screen once, so a fresh attach
  is never blank while an application holds the mode.

Bounds are validated on restore: rows and cols within 1..=1000, every Screen
row exactly `cols` wide and every SavedPrimary row as wide as its own
header, cursor and region inside the screen, link indexes inside the table,
`char::from_u32` checked. A snapshot that fails validation is treated as a
protocol error.

Decoded size: cells are run-length encoded and each cell owns a copy of its
hyperlink, so the wire size does not bound the memory a snapshot expands
to (66 KiB can describe more than 8 GiB). The decoder charges every run
(cells times cell size plus hyperlink length) against a budget before it
allocates: 256 MiB for a snapshot (screen and saved primary together),
64 MiB for a history chunk. Over budget is `Invalid("decoded size")` and
nothing is applied.

Not in the snapshot and kept by `restore` on the client: `scroll_offset`
(clamped), dirty flags (all set dirty), `response_tx` (none), `max_scrollback`.
`restore` clears the selection and `clipboard_request`. History is kept or
dropped by the refresh rule of section 5.

### Scrollback backfill, newest first

The snapshot contains no history. After `Ready`, if the client asked for
`scrollback_rows > 0`, the daemon streams `ScrollbackChunk` frames for that
attachment, interleaved with live output, starting at the newest history row
as of `snapshot_offset` and walking toward older rows. Rows use the same row
encoding as the Screen section (tag 7) of the snapshot.

- The daemon keeps one cursor per attachment: the absolute index
  (`selection_history_base` + position, the numbering the grid already uses
  for selections) of the next older row to send. Pushes at the new end and
  evictions at the old end do not renumber rows, so later chunks stay
  addressable. The backfill ends early (`last` flag) when the cursor falls
  below the base (evicted) or when the history epoch changes.
- History epoch: a counter in the grid, bumped when history is cleared
  (ED 3, RIS). The client clears its own history at the same stream
  position by parsing the same bytes; because chunks travel in the same
  FIFO as `Output` (section 5), no chunk built before the clear can arrive
  after it.
- Each chunk (at most 256 rows) is built and enqueued under the pane lock,
  one chunk per writer turn of that attachment, and only while its queue is
  under 1 MiB, so live output keeps priority.
- The client applies a chunk with `TerminalGrid::prepend_history(rows)`,
  which inserts at the old end and adds the row count to
  `command_output_start` and to the selection anchors (the base is not
  changed). Rows produced by the live stream keep appending at the new end,
  so the two never collide. The client stops accepting rows at its own
  `max_scrollback`: rows beyond the room it has are validated but not
  decoded. A chunk that would exceed the `scrollback_rows` of its attach
  is a protocol error and closes the connection.
- Resize does not restart a backfill. `resize` only truncates or pads
  history rows and moves whole rows between the newest end of history and
  the screen, which the client reproduces (section 7). The daemon clamps
  the cursor to the new newest row.

## 7. Multiple clients

Input ordering: one writer per pane. `Input` frames are written to the PTY in
the order the daemon receives them, each frame as one uninterrupted write, so
bytes of two clients never interleave inside a frame. A client must send a
bracketed paste or a multi-byte key sequence in a single frame when it fits
(64 KiB). Pastes larger than that may interleave with another client's
typing; an input lock is a later feature bit. Read-only attachments get
`Error(ReadOnly)` for `Input`, `Resize` and `Kill`.

Input backpressure: each pane has one writer thread fed by a queue capped at
1 MiB (a PTY accepts only a few KiB when the application is not reading).
While a pane's queue is full the daemon stops reading from the connection
whose `Input` hit the cap, so a large paste is flow-controlled end to end
instead of dropped; that client's other panes wait with it. If the PTY then
accepts no byte for 10 s, the daemon discards that pane's queued input,
sends `Error(Busy)` for the pane and resumes reading. The client library
queues outbound frames off the UI thread, capped at 16 MiB; beyond that the
paste fails visibly.

Query replies: only the daemon's grid has `response_tx` set, wired to the
pane's input queue (so replies are ordered with client input). Client grids
of attached sessions never answer DA1, DA2, DSR, CPR or DECRPM, otherwise
the application would receive one reply per attached emulator. OSC 52 writes
are applied by every attached client (each sets its own clipboard); the
daemon's copy is discarded. Focus reports (mode 1004) are ordinary `Input`
from whichever client gains or loses focus; with several clients the
application sees the latest one (accepted limit).

Resize policy: last active client wins. The pane size is the size requested
by the owner attachment. An attachment becomes owner when it is the only
writable one, when the owner detaches (the most recently active remaining
one takes over), or when it sends `Input` while the current owner has sent
none for 1 s. That hysteresis stops two people typing alternately from
resizing the pane on every keystroke. Requests from other attachments are
remembered and applied when they become the owner. Read-only attachments
never own the size.

An `Attach` with `rows` = `cols` = 0 asks for no size: it never resizes the
pane and never takes ownership. It is for a client that does not know its
view size yet (panes offered back at app start, tabs not laid out). Its
first `Resize` records the size it wants and, when the pane has no owner
present, makes it the owner. A placeholder size would send `SIGWINCH` to
the program and, because `grid.resize` truncates history rows, cut the
daemon's history to the placeholder width for good.

Resize storms: the client sends at most one `Resize` per pane per 50 ms
(latest size). The daemon applies at most one size change per pane per
50 ms (trailing edge, latest request wins) and ignores a request equal to
the current size, so the application sees a bounded number of `SIGWINCH`.

Justification: the real multi-client cases here are one person with a second
window, or a session left attached on another machine. With "smallest client
wins" a forgotten laptop or a small split shrinks the pane everywhere, the
well-known tmux annoyance, and every attach or detach of a smaller client
causes a resize and a reflow for all. With "last active" the pane always fits
the screen someone is typing in. The cost is that an inactive client sees a
grid that does not match its rect: it renders the server-sized grid anchored
top-left, letterboxed when smaller and clipped when larger, and takes the
size back on its next keystroke.

A resize is applied in one step under the pane lock: `TIOCSWINSZ`,
`grid.resize`, then an applied `Resize{offset, pulled}` frame queued to
every subscriber. Clients resize their grid when that frame arrives, so both
sides resize at the same stream position (`TerminalGrid::resize` resets the
scroll region and tab stops and reflows, so the position matters).

`grid.resize` is not a pure function of the screen: when rows grow it pulls
up to that many rows back from history. A client with less history than the
daemon (backfill not finished, `scrollback_rows = 0`, smaller
`max_scrollback`) would pad with blank rows instead and end up with a
different screen and cursor. Rule: the client applies the resize locally
only if its history holds at least `pulled` rows; otherwise it restarts the
attach. With a different `emulator_rev` it always restarts the attach.

Per-client scroll: `scroll_offset` and selection live in each client's own
grid and are never sent. Two clients on one pane scroll independently.

## 8. Remote transport

No new port. The client opens one extra exec channel, without a PTY, on the
existing SSH connection and runs a fixed command:

```
exec "$HOME/.shelldeck/bin/shelldeck-sessiond-<version>" proxy --hello <nonce>
```

`<version>` is the app's compiled-in version, checked against
`[0-9a-z.-]{1,32}`, and `<nonce>` is 32 hex characters generated by the
client and checked against `[0-9a-f]{32}` before use. Nothing else from the
client, the host or the user is interpolated.

The proxy is not a command runner or a forwarder: it takes no command, host,
port or socket path from the remote command line, connects only to the
per-user socket after the path and peer checks of section 3, starts only its
own executable with `serve`, and never opens a TCP socket. What a client can
run through it is exactly what `Spawn` allows, as the user who already has
a shell on that host. On accounts with a forced command or a restricted
shell the exec fails and the connection falls back.

- `proxy` connects to the socket (starting the daemon if needed), writes
  `\x1eSHELLDECK_SESSIOND_<nonce>_READY\x1f` to stdout, then copies stdin to
  the socket and the socket to stdout until either side closes. EOF on
  stdin closes the socket and the proxy exits, so nothing lingers after the
  SSH channel is gone.
- If it cannot connect it prints one line `SDSD-ERR <code>` (fixed codes:
  1 path check, 2 peer check, 3 start failed) to stderr and exits non-zero;
  the client maps the code to the fallback reason.
- The client discards stdout up to the marker (login banners, rc noise) with
  a 64 KiB cap and a 10 s timeout, the same pattern as `TmuxHandshake` and
  `folder_shell.rs`. After the marker the stream is protocol v1, starting
  with `Hello`.
- Any stderr data (`ExtendedData` ext 1) after the marker is a protocol
  error: close and fall back.
- One exec channel carries all panes of that host (frames carry the pane id),
  so a split no longer opens a new SSH connection.

`shelldeck-ssh` gets a new public method next to `exec_with_input_bounded`:
`SshSession::open_exec_stream(command)`, returning read and write halves with
a close guard (bounded EOF + close, 2 s). Rules taken from the existing code:

- no `request_pty` (8-bit clean only without a PTY);
- read loops end on `Close` or stream end, never on `Eof`;
- `CleanEnd` versus `ConnectionLost` decides between "proxy exited" and
  "reconnect then reattach";
- the client drains the channel continuously into a bounded buffer (russh
  0.61 blocks the whole connection when one channel's 100-message queue is
  full) and relies on section 5 for overload;
- writes run concurrently with reads (peer window 2 MiB, packets 32768).

Latency: the production client config leaves Nagle on. Setting `nodelay` for
connections that carry a daemon tunnel is evaluated in phase 3 with
measurements, not assumed.

### Binary delivery and version matching

- Install path: `~/.shelldeck/bin/shelldeck-sessiond-<version>`, mode 0700.
  Versioned names let several app versions use one host and make the proxy
  command version-exact.
- The proxy and daemon may differ in build version: they share the socket and
  negotiate through `Hello`. A newer proxy with an older running daemon is
  the normal state after an app update (section 3, upgrade story).
- Source: static musl builds for `x86_64` and `aarch64` Linux, produced by
  the release workflow from the same revision as the app and bundled with it.
  No download happens on the remote host.
- Install flow (phase 3): probe with one exec (`uname -sm` and a version
  check of the versioned path); if missing and the user agreed for this
  connection, stream the binary over an exec channel's stdin into a temp
  file in `~/.shelldeck/bin`, verify SHA-256 remotely, `chmod 700`, atomic
  rename. Fixed scripts only, quoted with `tmux::shell_quote`. After an
  install the three newest `shelldeck-sessiond-*` files are kept and older
  ones removed (a running daemon keeps its unlinked binary).
- Until phase 3 the remote binary is installed by hand (documented command),
  as for `shelldeck-workspace-helper`.

Fallback to tmux mode: unsupported OS or architecture, no consent, `noexec`
home, failed checksum, missing marker, handshake timeout or
`UnsupportedVersion` all end the daemon attempt for that connection. The
connection then continues with tmux (`open_tmux_shell`) when
`general.auto_attach_tmux` is on, otherwise a plain shell, and the reason is
reported through `SshLifecycleEvent` like `TmuxFailed` today.

## 9. Local transport

The app connects to `~/.shelldeck/run/<machine>/sessiond-v1.sock` directly with
the same client library, after the path and peer checks of section 3. The daemon binary ships next to the app binary; the app
starts it on demand. Because the daemon is a separate process in its own
session, quitting or updating the app leaves panes running. On the next start
the app lists panes and reattaches those belonging to its saved layout.

## 10. Client integration

`TerminalSession` stays a concrete struct. A third constructor is added next
to `spawn_local_at_authorized` and `spawn_ssh`:

```rust
pub fn spawn_attached(title: String, rows: u16, cols: u16)
    -> crate::Result<(TerminalSession, AttachedFeed, UnboundedReceiver<Vec<u8>>)>
```

- Shaped like `spawn_ssh`: bytes in through a feed, input out through a
  receiver. It does not call `set_response_tx`.
- `AttachedFeed` accepts three events, applied in order on the reader thread:
  `Restore(snapshot)`, `Bytes(data)`, `Resize(rows, cols)`, plus
  `History(rows)` for backfill. Each one pings `output_notifier`, so
  `ensure_refresh_running` repaints without change.
- `TerminalSession::resize` for an attached session does not touch the local
  grid: `resize_fn` sends a `Resize` request, and the grid changes only when
  the applied `Resize` arrives.

The `shelldeck-sessiond` client library owns one connection per host,
demultiplexes frames by pane into those feeds, enforces the offset rules and
restarts attaches.

UI plug points:

- Local: `TerminalView::spawn_configured_local` and
  `prepare_authorized_local_terminal`. With the flag on, they send `Spawn`
  to the local daemon instead of creating a PTY in-process. The fail-closed
  cwd rule is kept: the `AuthorizedLocalRoot` is validated before `Spawn`,
  the canonical cwd goes in the frame, and the path returned in `Spawned` is
  revalidated before the pane is shown; on mismatch the pane is killed.
  This rule is not implemented and workspace-scoped terminals stay
  in-process: with protocol v1 it cannot be met safely (section 15,
  "Workspace-scoped terminals").
- Remote: `connect_ssh_at` and `connect_ssh_split` in `workspace/ssh.rs`,
  beside the `attach_tmux` branch. `TerminalEvent::SplitRequested` spawns a
  new pane over the existing tunnel and calls `set_split_session`.
- Native splits map one to one to panes: each `PaneNode::Leaf` holds a
  session attached to one daemon pane; a tab is a `group`. `is_tmux_managed()`
  is false for these sessions, so `do_split`, focus and the toolbar use the
  native tree.
- Layout restore: `TerminalWorkspaceSnapshot` already describes tabs, the
  split tree, ratios, focus and per-pane viewports. It gains a daemon pane id
  per leaf. On start the client lists panes, reattaches leaves whose pane
  still exists, drops leaves whose pane is gone, and offers unknown panes as
  new tabs grouped by `group`.
- Closing a tab or pane sends `Kill`; quitting the app only disconnects.

Setting: `general.session_daemon: bool`, `#[serde(default)]`, default false,
next to `auto_attach_tmux` in `GeneralConfig` (default, sample TOML and
config test updated). Toggle row in `settings.rs` after the tmux row, gated
by a `GeneralSettingsVisibility` entry (dev-capable roles, Unix only), with
i18n keys `settings.general.session_daemon.*`. Being a `general.*` field it
follows the existing slice merge in `handle_settings_event`. The field, the
local wiring and the toggle exist (section 15, "Local client integration").

## 11. Security and threat model

Assets: the user's shells and everything typed into or printed by them.

Trust boundary: the uid. Anyone who can connect to the socket can run
commands as the user, exactly like a tmux socket. Controls:

- 0700 directory, 0600 socket, owner and mode verified by the daemon at
  start and by every client and proxy before connect, peer uid verified in
  both directions, no TCP, no abstract-namespace socket. A hostile local
  user can at most make the daemon path unavailable (pre-created `/tmp`
  fallback), never receive input or impersonate the daemon.
- Remote access only through the user's own authenticated SSH session
  running `proxy` as that user.
- Parser hardening on both ends: hard frame cap checked before buffering,
  per-field maxima, typed frames, no free-text errors, fail closed on
  anything unknown, 10 s handshake timeout, size and time caps on the proxy
  prelude, snapshot validation before applying.
- No shell interpolation: fixed proxy command, validated nonce, `Spawn` uses
  an argv vector and an environment allowlist.
- Resource limits: panes, connections, attachments, queue sizes, snapshot
  size, scrollback size.
- Binary integrity: bundled artifact, SHA-256 verified on the remote before
  it becomes executable, install only after per-connection consent.

Out of scope: root or same-uid malware on the host (it can already ptrace the
shells), and a compromised remote host attacking the client beyond what the
bounded decoder and snapshot validation prevent. A malicious daemon can feed
arbitrary terminal output, which is the same exposure as any SSH session.

Notes: pane content stays in daemon memory after the app quits (documented
behaviour, "restart session server" clears it). The log never contains pane
bytes or input.

## 12. Failure modes

| Failure | Behaviour |
|---|---|
| Daemon not running | Client starts it, retries 3 s, then reports and uses the in-process PTY (local) or tmux/plain shell (remote). |
| Daemon crash | All its panes are lost; the kernel hangs up their sessions, processes ignoring `SIGHUP` are orphaned. Clients see the socket close, mark panes disconnected, and offer a new shell. |
| Parser panic in one pane | That pane is marked dead and hung up; the daemon and other panes continue. |
| Stale socket file | Removed only by the process holding the lock. |
| Idle exit racing a connect | The daemon takes its socket out of the way and reads the listener once more before leaving: the connection is served, or finds no socket and the client starts a daemon. A start during an exit waits for the lock. For daemons of earlier releases the client still treats a connection that ends before `HelloAck` as "not running". |
| Daemon out of file descriptors | A pending connection is accepted on a spare descriptor, answered `Error(LimitExceeded)` and closed; other accept errors back off 50 ms. No connection slot is lost. Existing connections and panes are not affected. |
| Host name changes while the daemon runs | Nothing happens: the run directory is named after the machine identity. A daemon of an earlier release, in a directory named after the host, is found by the scan of section 3. |
| Snapshot or history chunk that expands past the decode budget | Refused before allocation; the pane stays on its last screen (snapshot) or loses that history chunk. |
| Client crash or SSH drop | Attachments dropped, panes keep running, no signal reaches pane children. Reconnect then reattach. |
| Slow client | Queue cap, `Error(Resync)`, backoff, fresh snapshot (section 5). |
| Client that stops reading, or a half-sent frame | Bounded by the queue caps; closed after 30 s. |
| Application not reading its input | Connection backpressure, then that pane's queued input is discarded after 10 s with `Error(Busy)`. |
| Offset gap or bad frame for a pane | Client restarts the attach; a malformed frame closes the connection. |
| Version mismatch | `Error(UnsupportedVersion)`: the daemon and its panes are left running, its sessions say "managed by another version", new local terminals are in-process (a remote host falls back to tmux or a plain shell). Never auto-kill a daemon. |
| Emulator revision mismatch | Works; the client refreshes from the daemon after resizes and when output goes quiet. |
| Pane child exits | Final output, then `Exited`; pane kept as dead until closed (`Kill` on the dead pane, or expiry). |
| Pane removed while other clients are attached | Each attachment still receives what was queued for it (final output, `Exited`), then `Error(NoSuchPane)` with its `attach_id`. |
| Peer that trickles a frame byte by byte | The 10 s handshake and 30 s frame deadlines are checked on every read, and a first frame declaring more than 128 bytes is refused on its length. |
| Long string in flight at attach | Never blocks: tail up to 4 KiB, else a canonical state prefix (section 5). |
| Resize the client cannot reproduce | Client restarts the attach (`pulled` rule, section 7). |
| History over budget | Oldest history of the largest panes is trimmed; screens are never touched. |
| Run dir or socket with wrong owner or mode, or wrong peer uid | Daemon refuses to start; client and proxy refuse to connect and fall back. |
| Logout with `KillUserProcesses=yes` | Daemon and panes are killed by systemd (same as tmux); documented. |
| Remote install impossible | tmux mode or plain shell. |

## 13. Test plan

Rules from `.agents/testing.md` apply: every test gets an SDTEST id mapped to
an SDUC use case, registered in `docs/testing/USE_CASES.md` and a new
`docs/testing/tests-sessiond.md` (ids are allocated when the tests land).
Daemon tests use a private temp directory and socket and kill the daemon at
the end. Runs stay targeted: `cargo test --locked -p shelldeck-sessiond` and
`-p shelldeck-terminal --lib`.

- Protocol codec: round trip of every frame type (the wire format is a
  contract); rejection of oversize length before buffering, unknown type,
  wrong version, trailing bytes, over-limit fields, invalid UTF-8, a
  disallowed env name, frames before `Hello`.
- Snapshot: feed a corpus (SGR, wide and combining characters, alt screen,
  scroll region, tab stops, origin mode, OSC 8, OSC 133, DEC graphics,
  pending wrap at the right margin) to grid A, snapshot, restore into B,
  then feed the same continuation to both and require identical snapshots.
  Restore must keep B's scroll offset and selection.
- Boundary: for a tricky stream, split at every byte index; at each split
  take snapshot + tail, restore into a fresh grid and parser, feed the rest;
  the result must equal the unsplit reference. Covers UTF-8, CSI with an
  embedded C0 control, OSC and DCS strings. Separately: an unterminated
  `ESC P` or `ESC ]` followed by silence still attaches at once, and a
  string longer than 4 KiB attaches with the canonical prefix and ends at
  the same byte on both sides.
- Snapshot corpus additions: per-cell hyperlinks and underline colour, a
  resize while the alt screen is active (saved primary with old size), and
  `command_output_start` restored into a grid with a different history
  length.
- Backfill: history prepended newest first equals the server history;
  selection and `command_output_start` stay on the same text; early end when
  rows are evicted or history is cleared (ED 3, RIS) mid-backfill; a row
  grow during backfill either reproduces the daemon's screen or resyncs
  (`pulled` rule), never diverges.
- Backpressure: a client that never reads is dropped at the queue cap and
  the pane and a second client keep full speed; a pane whose child does not
  read input blocks only the pasting connection and recovers after the
  discard; half a frame followed by silence is closed after the timeout
  (timeouts are injectable so the test stays short).
- Hijack: a client refuses a run directory or socket with the wrong owner or
  mode and refuses a symlinked directory.
- Signals: after client disconnect the pane child is alive with no pending
  `SIGHUP`, and its `SigIgn` and `SigBlk` exclude `SIGHUP`; `SIGTERM` to the
  daemon leaves no pane process behind, including one that ignores
  `SIGHUP`.
- Heavy-output attach race (integration, real daemon and PTY): a pane runs a
  deterministic generator that floods several MiB of seeded output mixing
  SGR, cursor movement, wide characters, scrolling and alt-screen switches,
  then stops. Client A is attached from the start; several other clients
  attach, detach and reattach at random offsets during the flood, and one
  client reads slowly enough to be dropped and resynced. Assertions: no
  client ever observes an offset gap without recovering; after the final
  offset is reached, every client grid snapshot equals the daemon's
  (visible screen, cursor, modes); total bytes accounted for equal the final
  offset.
- Multi-client: input frames from two clients arrive whole and in daemon
  order; exactly one DA1 reply reaches the application with three clients
  attached; resize ownership follows the last active client with the 1 s
  hysteresis; a burst of 100 resize requests from two clients produces a
  bounded number of applied `Resize` frames and both grids end equal to the
  daemon's; a read-only attachment cannot write.
- Lifecycle: second `serve` exits and the first keeps serving; a pane
  survives client disconnect and reattach shows the same screen; idle exit;
  stale socket recovery; refusal on a 0755 run directory; `Exited` carries
  the status.
- Proxy: prelude noise before the marker is skipped, oversize or late prelude
  fails, and a full spawn, attach, input, output cycle works through
  `proxy` over a pipe pair.
- SSH (inside `shelldeck-ssh`, where the in-memory russh fixtures live): a
  new `ExecBehavior` variant bridges exec channel data to an in-process
  proxy; the `window_size: 0` fixture covers write backpressure; `Eof` before
  exit status does not end the stream.
- Live, `#[ignore]` behind `SHELLDECK_LIVE_SSH=1`: daemon on a real host
  through sshd, including reconnect and reattach.
- Manual QA note in `docs/testing/` mirroring the tmux pane QA document:
  splits, scroll and selection per client, app quit and relaunch, app update
  with live panes.

## 14. Phased rollout

Phase 1, the prototype on this branch (implemented now):

- crate `shelldeck-sessiond` in the workspace: protocol codec with all v1
  frames and limits, daemon (`serve`) with lock, path and peer uid checks on
  both sides, spawn, list, attach, input, resize, detach, kill, exited;
- `TerminalGrid::snapshot` / `restore`, `StreamBoundary`, offset based
  handoff, per-attachment FIFO with the 4 MiB cap and slow-client resync,
  input queue with backpressure, frame and write timeouts;
- `LocalPty::spawn_command`, `PtyMaster::try_wait`, `EMULATOR_REV`;
- `proxy` subcommand with the nonce marker;
- client library over generic async streams;
- codec, snapshot, boundary, lifecycle, proxy and heavy-output race tests.
- Scrollback backfill is implemented if time allows; otherwise the daemon
  sends one empty `ScrollbackChunk` with the `last` flag so the wire contract
  is already final.
- Accepted shortcuts: links `shelldeck-core` through `shelldeck-terminal`,
  no auto-start from the app, no UI wiring, Linux tested only, no history
  budget trimming, no quiescent refresh on emulator mismatch (the client
  only resyncs after a resize).

Phase 2, local opt-in: `spawn_attached`, `general.session_daemon` setting,
local tabs and splits through the daemon, layout restore on app start, daemon
bundled with the app, macOS peer check, history budget, quiescent refresh on
emulator mismatch, manual QA document (including: the daemon survives app
quit when the app was launched from the desktop, from a terminal and by the
updater).

Phase 3, remote: `SshSession::open_exec_stream`, tunnel in `connect_ssh_at` /
`connect_ssh_split`, one connection for all panes of a host, `app` feature
split of `shelldeck-terminal`, static musl artifacts in the release workflow
and `deny.toml` targets, consented upload and checksum, fallback reporting,
`nodelay` evaluation, live SSH test.

Phase 4, hardening and extras: seamless daemon upgrade by re-exec with fd
handoff, input lock for large pastes, read-only sharing UI, daemon-stored
layout metadata per group, default on for dev-capable roles, then for
everyone, with tmux mode kept as the fallback for hosts without a binary.

## 15. Prototype notes (as implemented in phase 1)

What exists on the branch, and where it differs from the sections above.
The sections above stay the target; each difference is either a refinement
to fold back into them or a gap listed under "not implemented".

### Status summary (2026-11-11)

Implemented now, with automated tests on Linux:

- Daemon, protocol v1, snapshot and restore, attach handoff by byte
  offset, slow-client resync, history backfill, multi-client rules, stdio
  proxy, debug commands (SDTEST-2955..2980, 2986).
- SSH transport in `shelldeck-ssh` (SDTEST-2985, 2987..2991).
- Opt-in local use in the app (`general.session_daemon`, default off,
  toggle in Settings, General): new unscoped local tabs and splits are
  daemon panes (SDTEST-2992..2996). No call waits for the daemon on the
  UI thread; a session reattaches by itself after the daemon connection
  is lost and reports a pane that did not survive; the native layout is
  saved by pane id and rebuilt at startup (SDTEST-3004..3013). The pure
  parts and the layout capture are tested; the app itself was never
  launched with the setting, so nothing was checked on screen.
- Opt-in remote use in the app, with the same setting (subsection "Remote
  app integration" below): an SSH tab opened without a directory is a
  pane of the host's daemon when the host has the binary, one SSH
  connection per host carries all its panes, panes survive the app and
  are offered back at the next connect. Probe of the remote binary,
  upload with SHA-256 verification under a per-host consent recorded in
  the configuration, and a typed fallback to the tmux attachment or a
  plain shell (SDTEST-3014..3024). The transport, the install and the
  host object are tested on the in-memory SSH fixture, once with the
  real binary (SDTEST-3024, opt-in); the workspace wiring is compiled
  and read only, and no real SSH server was used: nothing was checked on
  screen or against `sshd`.
- Review fixes (subsection "Review fixes" below, SDTEST-2997..3003 and an
  extended SDTEST-2978).
- Daemon hardening and Linux packaging (subsection "Daemon hardening,
  version skew and packaging" below): input backpressure with a
  non-blocking PTY writer, size ownership hysteresis, shutdown with lock
  handoff, parser panic isolation (SDTEST-2981..2984); the daemon's own
  descriptor limit with the original restored in pane children, the log
  file, the daemon side of the idle-exit race, a run directory named after
  the machine identity, the history budget (SDTEST-3025..3030, SDTEST-3001
  extended); a daemon left by another app version is used when it speaks
  the protocol and left alone otherwise (SDTEST-3031, SDTEST-3032,
  SDTEST-3012 and SDTEST-3013 extended); the daemon binary is built by the
  release script and carried by the Linux tar.gz, the install script and
  the updater (SDTEST-3033..3036). The release pipeline itself was not
  run, nothing was published, and the app was still not launched.
- First run of the real application (subsection "First run in the
  application" below): the local path was driven on a private headless
  desktop with a throwaway profile, with the setting off and on. Tabs,
  splits, resize, full-screen programs, heavy output, scrollback, quit
  and relaunch with the layout and the running commands, daemon kill,
  app kill, sign-out, idle exit and the Settings row in both languages
  passed. Three defects were found and fixed (SDTEST-3037, SDTEST-3038,
  the Settings text). The remote path was not run (no sshd on the
  machine). Results, one row per check, in
  [`session-daemon-qa.md`](../testing/session-daemon-qa.md), section
  "Checks in the running application".

Deferred (details in the subsections below):

- Packaging: the two static remote builds are wired
  (`scripts/build-sessiond-remote.sh`, opt-in) but were never built: the
  musl targets are not installed on the machine this was written on, and
  the repository has no established musl build. Until they are built and
  shipped, a release build falls back on every SSH host with "no program
  for this kind of server" unless the binary was installed by hand. The
  AppImage, the macOS bundle and the Windows archive do not carry the
  daemon. Saving the split tree of remote tabs.
  Workspace-scoped terminals are not deferred but excluded: they stay
  in-process until the protocol can carry a directory identity (see
  "Workspace-scoped terminals" below).
- Daemon: quiescent refresh on emulator revision mismatch.
- Remote: pruning old versioned binaries after an install, an upgrade
  path for a running daemon of an incompatible release (today: fallback),
  latency work (`nodelay`), SSH tabs opened in a directory (they keep
  the plain path), a remote daemon from a Windows client.
- Platforms: macOS is coded but never built or run (this now includes the
  `kern.uuid` lookup for the run directory name); Windows is a stub.

Layout:

- `crates/shelldeck-sessiond/src/`: `protocol.rs`, `client.rs` (client and
  `PaneMirror`, a headless client emulator that applies pane events),
  `paths.rs` (run directory, socket and peer checks), `proxy.rs`,
  `server/{mod,conn,pane}.rs`, `bin/shelldeck-sessiond.rs`.
- The binary also has debug commands for manual testing: `list`, `spawn`,
  `attach PANE` (raw mode on the current tty, Ctrl-\ detaches, the restored
  screen is painted as plain text) and `kill`, plus a hidden `debug-flood`
  output generator used by the attach race test.
- `shelldeck-terminal`: `boundary.rs`, `grid/snapshot.rs`, a history epoch
  in the grid, `LocalPty::spawn_command`, `PtyMaster::{process_id, try_wait,
  as_raw_fd}`, `EMULATOR_REV = 1` (2 since the first run in the
  application, see below).
- The daemon uses OS threads (one reader and one writer per connection, one
  reader and one writer per pane), no async runtime. The client is async
  (Tokio) over any `AsyncRead + AsyncWrite`.

Refinements made while implementing:

- Tail past 4 KiB (section 5): the canonical prefix depends on the exact
  parser state, because vte ends a DCS on `0x9c` only from the passthrough
  and ignore states. Prefixes: CSI `ESC [ 0 <`; DCS entry `ESC P`, DCS
  parameters `ESC P 0`, DCS intermediates `ESC P SP`, DCS passthrough
  `ESC P q`, DCS ignore `ESC P 0 <`; SOS, PM, APC `ESC X`; OSC
  `ESC ] 7999 ;`. An escape with intermediates never needs one: vte keeps
  two intermediates and flags the third, so ESC plus three intermediates is
  an exact tail. Bytes that do not change the parser state (executed C0
  controls, ignored bytes) are never recorded.
- `Error.request_id` carries the `attach_id` when the error concerns one
  attachment (`Resync`, `Busy` or `NoSuchPane` for an `Attach`), so a client
  can ignore an error about an attach it already replaced.
- `ScrollbackChunk.bytes` is self-contained: `u8 format`, `u16 cols`, a link
  table (same encoding as section Links), `u16 row count`, then the rows in
  the Screen row encoding. The Links section of a snapshot is not reachable
  from a later chunk.
- Screen section (tag 7) starts with a `u16` row count: after a reflow the
  grid can hold more rows than `rows` (existing behaviour). Rows are
  normalized to `cols` on encode; restore requires `rows <= count <= 4000`.
- `Exited.exit_code` is `128 + signal` for a signal death.
- The lock file holds the daemon's pid (diagnostics, tests).
- `Spawn.scrollback_max = 0` means the default (10 000 rows).

Not implemented yet (beyond the phase 1 exclusions of section 14):

- The quiescent refresh on emulator revision mismatch (the client only
  resyncs after a resize it cannot reproduce).
- macOS: `getpeereid`, the 104-byte `sun_path` and the `kern.uuid` lookup
  are coded but untested. The non-unix stub `main` is not compiled or
  tested here.
- Input held back by a full PTY is only detected as a stall once the
  pane's queue is full as well (1 MiB by default): a smaller paste into a
  program that never reads simply stays queued.

Known cost: `shelldeck-sessiond` links `shelldeck-core` through
`shelldeck-terminal` (accepted for the prototype, section 2).

### Remote transport (as implemented in phase 3, step 1)

`crates/shelldeck-ssh/src/sessiond.rs`, tests in `sessiond_tests.rs`
(SDTEST-2985, 2987..2991). Wired into the app by the next subsection.

- Entry point: `shelldeck_ssh::sessiond::connect(&SshSession, options)`
  returning `SessiondConnection { client, ended }`, instead of the
  `SshSession::open_exec_stream` of section 8. The channel is opened through
  `shared_handle()`, so `session.rs` is unchanged. The stream handed to
  `Client::connect` is one end of a 256 KiB in-process duplex; a task relays
  between it and the channel halves in both directions concurrently.
- Remote command: `RemoteBinary::Versioned(version)` gives the command of
  section 8. `RemoteBinary::Absolute(path)` gives
  `exec '<path>' proxy --hello <nonce>` for an administrator install
  (`protocol::proxy_command_at`): absolute, at most 512 bytes, only
  `A-Z a-z 0-9 . _ + - /`, no `.` or `..` component. A rejected setting is
  `InvalidBinary` and nothing is sent.
- The marker is consumed in `shelldeck-ssh` (not by
  `Client::connect_via_proxy`), because the exit status and stderr are only
  visible at the channel level. Before the marker: exit status 127 is
  `BinaryMissing`, 126 is `BinaryNotExecutable`, a channel `Failure` is
  `ExecRefused` (these three are `is_unavailable()`, the tmux fallback);
  `SDSD-ERR 1/2/3` is `ProxyRefused`; any other end is `ProxyExited`; more
  than 64 KiB is `Prelude`; no marker within `startup_timeout` (10 s) is
  `Timeout`; the connection going away is `Ssh(SessionClosed)`. The wait
  does not end on EOF, so a status that follows the EOF is still read.
- After the marker: `Error(UnsupportedVersion)` to the `Hello` is
  `VersionMismatch`. Stderr data ends the tunnel (`UnexpectedStderr`).
  `ended` reports `LocalClose`, `RemoteExit { exit_status }` (EOF, exit
  status or Close seen; after an EOF the status is awaited for at most 2 s)
  or `ConnectionLost` (stream end without any of them).
- Release: a guard sends EOF then Close, 2 s each, when the client is
  dropped, when a connect fails, times out or is cancelled.

Open points:

- Inbound backpressure relies on the client reader never stalling (it only
  demultiplexes into bounded per-pane queues). If it did stall, the relay
  would stop draining the channel and russh would block the whole
  connection, as described in section 8.
- `nodelay` is unchanged (Nagle on); no latency measurement was made.
- Install and probe exist since step 2 (next subsection); there is still
  no upgrade flow.
- On Windows the module compiles against the client library only; that
  build was not checked here.

### Remote app integration (as implemented in phase 3, step 2)

Same opt-in (`general.session_daemon`), Unix clients only. With the
setting off, `workspace/ssh.rs` behaves as before: the shell body was moved
into a function (`run_ssh_shell`) that both paths call, nothing else.

Pieces:

- `shelldeck-sessiond`: `supervisor::Connector`, the way a `Supervisor`
  reaches its daemon. `Supervisor::new` wraps the local resolver in one;
  `Supervisor::with_connector` takes any other. `LocalSessions::from_client`
  wraps a client that is already connected. `Supervisor::session_count`,
  `set_on_idle` (called when the last session thread ends) and
  `close_connection`.
- `shelldeck-ssh/src/sessiond_install.rs`: `probe` (one exec:
  `present | absent | noexec`, `uname -s`, `uname -m`, found by a nonce
  among rc noise), `install` (binary on stdin of a fixed script: private
  umask, `mkdir -p "$HOME/.shelldeck/bin"`, temporary file there, SHA-256
  with `sha256sum`, `shasum` or `openssl`, compare, `chmod 700`, rename;
  the temporary file is removed on every failure; the digest printed by
  the host is compared again on the client), `target_triple`,
  `find_artifact` / `load_artifact` (the file must be a 64-bit
  little-endian ELF for the right machine), and the decision table
  `decide(probe, consent, has_artifact)`. Only a checked version, a nonce
  and a hex digest are placed in a script; both scripts run through
  `sh -c`. No `sudo`, nothing outside `$HOME/.shelldeck`.
- `shelldeck-ssh/src/sessiond_host.rs`: `RemoteHost`, one per SSH
  connection entry. It owns the shared SSH connection (dialed on a runtime
  given by the app) and a `Supervisor` whose connector opens the exec
  channel on it. `prepare(consent)` is the first contact: dial, probe,
  decide, upload if allowed, connect, list. It returns `Ready { panes }`,
  `Fallback { reason, ask, session }` (the SSH connection is handed back
  for the fallback backend) or `ConnectFailed`. When the channel ends with
  `ConnectionLost` the connection is forgotten and the next attempt of a
  session dials again (`HostEvent::ConnectionLost`, `Reconnected`). When
  the last session is over the channel and the connection are released
  (`HostEvent::Idle`).
- `shelldeck-ui/src/workspace/remote_daemon.rs`: the process-wide registry
  of hosts (keyed by connection id, replaced when the entry points
  elsewhere), one two-thread runtime for all shared SSH connections, the
  artifact directories, and `ready_plan` (which pane a connecting tab
  takes, which come back as tabs).
- `workspace/ssh.rs`: with the setting on and no directory asked,
  `connect_ssh_at` still opens the tab at once with an SSH session, and
  its thread calls `prepare` first. `DaemonReady`: the workspace replaces
  the tab's session in place with a supervisor session
  (`TerminalView::replace_session`). `DaemonFallback`: the thread goes on
  with `run_ssh_shell` on the connection it was handed, tmux attachment
  or plain shell as with the setting off. A host whose daemon is already
  connected skips all of it: a new tab or split is `Supervisor::open` on
  the UI thread, without a second SSH connection.

Decision table (SDTEST-3015), in this order:

| Probe | Consent | Artifact for the host | Outcome |
|---|---|---|---|
| no usable answer | any | any | fallback `ProbeFailed` |
| binary present | any | any | connect |
| missing or not executable, platform without a build | any | any | fallback `UnsupportedPlatform` |
| missing or not executable | any | not bundled | fallback `NoArtifact` |
| missing or not executable | allow | bundled | upload, verify, connect |
| missing or not executable | never | bundled | fallback `ConsentDeclined` |
| missing or not executable | not asked | bundled | fallback `ConsentPending`, ask |

After a connect attempt: `is_unavailable()` gives `Unavailable`,
`VersionMismatch` gives `VersionMismatch` (the old daemon is never stopped:
its panes belong to the user), a failed upload gives `InstallFailed`,
anything else `ConnectFailed`. Every fallback continues with
`open_tmux_shell` when `general.auto_attach_tmux` is on, else a plain
shell.

Consent: `[general.session_daemon_hosts]` in the configuration, connection
id to `allow` or `never` (SDTEST-3014). A host without an entry is asked
once per run, by a toast with three buttons (install, later, never for
this server) that blocks nothing: the tab is already running on the
fallback backend. "Install" records `allow` and uploads in the background;
the next tab of that host uses the daemon. "Later" records nothing.

Notices: one per host and per run. The install question is the notice when
it is shown; otherwise one toast names the reason and the backend in use.

Artifacts: `shelldeck-sessiond-x86_64-unknown-linux-musl` and
`shelldeck-sessiond-aarch64-unknown-linux-musl`, looked up in
`SHELLDECK_SESSIOND_ARTIFACT_DIR` (development), next to the app binary,
then in `sessiond/` of the app data directory. They are produced by the
opt-in `scripts/build-sessiond-remote.sh` and, when present, carried by
the Linux archive, the install script and the updater; that script has
not been executed yet (see "Daemon hardening, version skew and packaging").

Panes offered back: the app labels its remote panes `shelldeck-remote`.
On `DaemonReady` the connecting tab takes the first live pane with that
label that no session shows, else spawns one; the other such panes become
tabs, unless other tabs of the same host are still connecting (a restored
session), in which case each of them takes one (SDTEST-3021). Closing a
tab or a pane ends the remote pane; quitting the app does not.

Differences from sections 8 and 10, and limits:

- The split tree of a remote tab is not saved: after a restart its panes
  come back as tabs.
- An SSH tab opened in a directory (`workdir`) keeps the plain path.
- Old `shelldeck-sessiond-*` files are not pruned after an install.
- The consent is per host and permanent until the entry is removed from
  the configuration file (there is no Settings control for it), not "for
  this connection" as section 8 says.
- A tab closed while the host is unreachable does not end its pane: the
  pane is offered back at the next connect.
- A new tab opened while the host is reconnecting goes through `prepare`
  again and may fall back.
- The supervisor and the host object are Unix only, so a Windows client
  never uses a remote daemon.
- Only one workspace applies host events (idle, lost, reconnected) to the
  connection status.

### Local client integration (as implemented in phase 2, steps 1 and 2)

Opt-in, off by default, Unix only. What exists:

- Setting `general.session_daemon` (`GeneralConfig`, `#[serde(default)]`,
  default false), with a toggle in Settings, General, after the tmux row
  ("Persistent terminal sessions", French and English), shown to
  dev-capable roles on Unix. The value reaches every `TerminalView`
  through `WorkspaceTerminalConfig.persistent_sessions` and
  `TerminalView::set_persistent_sessions`, the same path as
  `default_shell`. It is read when a terminal is created: changing it
  applies to new terminals only, running ones keep their backend.
- `shelldeck-terminal`: `TerminalSession::spawn_attached(title, rows, cols,
  max_scrollback, daemon_emulator_rev)` returns the session, an
  `attached::AttachedFeed` and the input receiver. The session has no
  `response_tx`; `resize` only calls the resize callback, once per size
  change (the view asks on every render), and never touches the grid;
  `terminate` runs an optional callback (end the pane) and is a no-op for
  PTY and SSH sessions; dropping the session closes the input channel.
  The session and its feed share an `attached::AttachedStatus`: the phase
  (`Connecting`, `Live`, `Reconnecting`, `Ended { exit_code }`,
  `Unavailable`) and the daemon pane id once known
  (`TerminalSession::attached_status`, `daemon_pane`). A restore sets
  `Live`; the feed's emulator revision is set on every attach, because a
  session can outlive a daemon.
- `shelldeck-sessiond::attached::LocalSessions`: a synchronous facade over
  one connection, with a private two-worker Tokio runtime. `connect(dir,
  explicit_dir, daemon_exe, options)` starts the daemon when absent (and
  once more when the connection ends before `HelloAck`);
  `connect_existing` never starts one. `spawn` and `attach` return a
  `TerminalSession` and block for the round trip: they are used by tests
  and by the supervisor below, not by the UI thread. `terminate` sends
  `Kill(HUP)`, `Kill(KILL)` after 1 s if the pane is still alive, then
  `Kill` on the dead pane, which removes it. A pane whose process exits
  on its own is not removed by the session that sees the exit (another
  client may not have read its end yet): it stays listed as dead, with
  its final screen, until its tab is closed (`terminate`) or the daemon
  expires it after an hour. Dead panes are never offered back.
  `SessionOptions.keep_pane_size` attaches with size 0 x 0 (section 7).
- `shelldeck-sessiond::supervisor::Supervisor`: what the app uses.
  `open(PaneTarget::Spawn(..) | PaneTarget::Existing(pane), options)`
  returns a `TerminalSession` at once, in the `Connecting` phase, and
  never takes the connection lock. One OS thread per session
  (`sessiond-pane`) then connects (one shared connection, opened under a
  lock so several tabs opening together start one daemon), spawns the
  pane if it is a new one, attaches and applies the pane's events to the
  feed (parsing stays off the runtime; the client's per-pane feed cap
  still bounds the backlog). A task on the supervisor's own one-worker
  runtime forwards input and resize requests to whatever connection the
  pane is attached through. Input typed before the first attach is kept
  (up to 64 KiB) and delivered once attached, so a command sent right
  after opening a tab is not lost; input typed while a session is
  disconnected is dropped. The target (run directory, daemon binary) is
  resolved on the connecting thread, on every connect; without a daemon
  binary a daemon that already runs is still used.
- Reconnect: when the connection ends, the session thread shows a notice,
  sets `Reconnecting` and tries again after a delay that doubles from
  250 ms to 5 s (`supervisor::Backoff`), connecting with "start the
  daemon when absent". The rules are a pure state machine
  (`supervisor::Reconnect`): a reattach that shows the pane resets the
  delays; `NoSuchPane` from the daemon (a new daemon never has the old
  panes; the pane was closed elsewhere or expired) ends the session as
  `Ended` with a notice; a daemon that can neither be reached nor
  started, with none running, ends it the same way (its panes died with
  it); 12 attempts in a row without the pane's screen end it as
  `Unavailable`. A new pane whose daemon cannot be started or that the
  daemon refuses is `Unavailable` at once, with the reason on screen. A
  process that exits is `Ended` with its exit code and keeps its final
  screen, without a notice. The view maps the phase of a tab's primary
  session to the tab state in its refresh loop (exit code, or an error
  for a lost or unreachable pane) and shows a pending colour on the tab
  dot while `Connecting` or `Reconnecting`.
- `shelldeck-ui`, `terminal_view/daemon.rs`: one process-wide `Supervisor`
  on `paths::default_run_dir()`. The daemon binary is `shelldeck-sessiond`
  next to the app executable, or the absolute path in
  `SHELLDECK_SESSIOND_BIN` (development). `TerminalView::
  spawn_configured_local` uses it for new tabs and splits of an unscoped
  terminal; each native pane (`PaneNode::Leaf`) is one daemon pane. The
  only synchronous check is one `stat` of the daemon binary: without it
  the view logs a warning and opens an in-process PTY, as with the
  setting off. Any later failure (run directory refused, daemon not
  starting) is shown inside the tab, which stays `Unavailable`; there is
  no automatic fallback to an in-process terminal at that point.
- Spawn request: `argv` is the configured `terminal.default_shell` alone, or
  empty (login shell); `cwd` is empty (home); `env` is `TERM` plus, when
  set in the app, `LANG`, `LC_ALL`, `LC_CTYPE`, `LC_MESSAGES`, `COLORTERM`,
  `DISPLAY`, `WAYLAND_DISPLAY`, `XAUTHORITY`, `SSH_AUTH_SOCK`; label
  `shelldeck-local`. The pane is spawned at the size the view last asked
  for.
- Closing a tab (`close_tab`) or a pane (`close_split`) calls `terminate`
  on the sessions it drops; a session that is not attached at that moment
  (still connecting, or reconnecting) ends its pane as soon as it can
  reach it, without starting a daemon for that. `close_all_sessions` (app
  shutdown) does not: the sessions are dropped, which only detaches. A
  sign-out (`invalidate_cloud_session`) ends every daemon pane first
  (`TerminalView::end_daemon_panes`), so the shells of one account are
  not offered back to the next.
- Layout save: `Workspace::save_workspace_state` (app shutdown and
  sign-out, the existing `workspace.json`) stores, for each tab of the
  main terminal whose primary pane is a live daemon pane, a
  `DaemonTabLayout` in the tab's `TabState.daemon`: the split tree with
  directions and ratios, leaves named by daemon pane id, the focused pane
  and whether the tab title was chosen deliberately. Leaves that are not
  live daemon panes (in-process, ended, not spawned yet) are left out and
  their split collapses. Files written before the field existed load
  unchanged; a tab without daemon panes writes no `daemon` key.
- Layout restore: `Workspace::restore_session` (called once at startup)
  lists the panes on the background executor (`restorable_daemon_panes`:
  connect only if a daemon already runs, never start one; live panes
  labelled `shelldeck-local` that this process does not show), then
  continues on the UI thread with `WorkspaceState::daemon_restore_plan`:
  a saved daemon tab keeps its place, tree and ratios for the panes still
  alive; a slot whose pane is gone is dropped (its split collapses, a tab
  with none left disappears); a live pane without a slot comes back as a
  tab of its own, after the saved ones; a pane saved twice is used once
  and ratios are clamped to the range a divider drag allows.
  `TerminalView::restore_daemon_tab` opens one session per pane with
  `keep_pane_size`: the view has no size at that point, and a tab in the
  background is not laid out until it is shown, so each pane keeps its
  size until its session sends the first real one. The saved active tab
  is selected again and its focused pane refocused. This happens whatever
  `auto_connect_on_startup` says (the panes are live processes, not saved
  state); with that preference on, saved SSH tabs are restored at their
  saved place between the daemon tabs. When at least one pane came back,
  saved local tabs without daemon panes are not recreated (they would
  duplicate the panes). With the setting off the restore runs
  synchronously, as before.

#### Workspace-scoped terminals

Decision: `LocalCwdScope::Required` terminals (workspace terminals,
including `prepare_authorized_local_terminal`) keep their in-process PTY,
with the setting on or off. The rule of section 10 (validate the
`AuthorizedLocalRoot`, send the canonical path, revalidate the path
returned in `Spawned`, kill on mismatch) is not implemented because it
cannot be met safely with protocol v1:

- The authority of a workspace terminal is a held directory descriptor
  with a recorded device and inode; the PTY child is started through
  `/proc/<pid>/fd/<n>` (Linux) so a rename or a symlink swap between the
  check and the `chdir` cannot redirect it.
- The daemon receives a path string, canonicalizes it, then the child
  does `chdir` by name after `fork` (`conn.rs`, `LocalPty::spawn_command`).
  The `cwd` returned in `Spawned` is the string computed before that
  `chdir`. A directory swapped in between leaves the returned path equal
  to the authorized one while the shell runs somewhere else, so
  revalidating the returned path proves nothing.
- Even a mismatch that is detected is detected after the login shell
  started and ran its startup files in the wrong directory. The
  in-process path fails closed before any process exists.
- A pane offered back after an app restart would be shown without any
  held descriptor: the directory may have been replaced while the app
  was not running.

What would make it possible: a `Spawn` field carrying the expected device
and inode, checked by the daemon on a descriptor it opened with
`O_DIRECTORY | O_NOFOLLOW` and used for the child's `fchdir`, with the
identity reported in `PaneInfo` so a reattach can verify it. That is a
protocol change, not part of this step.

Differences from sections 9 and 10, and limits of this slice:

- `AttachedFeed` is applied by the caller, on the caller's thread, through
  plain methods (`restore`, `bytes`, `resize`, `history`); it is not a
  channel drained by a thread inside `shelldeck-terminal`. This keeps the
  client's feed cap effective and reports `NeedsReattach` directly.
- A pane's environment is the login shell's own (the daemon scrubs its
  environment and starts a login shell) plus the forwarded names above. An
  in-process terminal inherits the whole environment of the app. Variables
  exported only in the app's environment are therefore missing in a daemon
  pane.
- The layout is saved when the app shuts down or signs out, not on every
  change. After a crash or `kill -9` the previous file is used: panes
  created since then come back as tabs of their own and splits made
  since then are lost.
- Not restored: scroll positions, per-tab zoom, the focused pane of tabs
  other than the active one (selecting a tab focuses its primary pane, as
  before), and titles set by programs before the restart until the
  program sets them again. `group` is still not used.
- Panes are offered back only at startup. Turning the setting on while the
  app runs affects new tabs only.
- The notices shown inside a terminal are translated once, when the first
  daemon session is opened: a language change applies to them after a
  restart.
- A reconnect starts a daemon when none runs, only to learn from it that
  the panes are gone; that daemon idles out after 60 s if nothing else
  uses it.
- The tab state follows the pane: after `exit` the tab shows the exit
  state (an in-process local terminal stays "running" today).
- Reading `workspace.json` at restore still happens on the UI thread (a
  small local file, as before); it now also happens with
  `auto_connect_on_startup` off when panes came back.
- The daemon binary ships with the Linux tar.gz install only (see "Daemon
  hardening, version skew and packaging"). From an AppImage, on macOS and
  on Windows the setting falls back to in-process terminals (with a
  warning in the log).
- A daemon left over from another app version keeps serving until it idles
  out (60 s without pane or client). It is used when it speaks the
  protocol; otherwise its panes are shown as managed by another version
  (same subsection).
- Not built or run on macOS or Windows. On non Unix targets the backend is
  a stub that reports "not available", so the setting has no effect and
  the toggle is hidden.
- The real app was not launched with the setting on: see
  `docs/testing/session-daemon-qa.md` for what was verified automatically
  and the manual checks still to do.

Follow-up done: the remote SSH use of the daemon in the app is described
in "Remote app integration" above. It was written without a real host: see
the status summary for what was and was not verified.

### Review fixes (final step of the prototype)

Two independent reviews of the uncommitted work reported nine findings. All
were confirmed against the code. Eight are fixed with a test; one is fixed
on the client side only.

| Finding | Outcome |
|---|---|
| Restore at app start resized live panes to 24x80 and cut the daemon's history to 80 columns | Fixed: `Attach` with size 0 x 0 keeps the pane size; the restore path uses it (`keep_pane_size`). SDTEST-2998. |
| Input refused with `NotAttached` between `Error(Resync)` and the delayed reattach, so Ctrl-C could not stop a flood | Fixed: the dropped subscriber stays registered without output; `Input` and `Resize` are accepted. SDTEST-2999. |
| Each client sent `Kill` on `Exited`, removing the dead pane and discarding the other clients' final frames | Fixed twice: the session no longer removes a pane on exit, and a removal lets every attachment drain its queue and ends it with `Error(NoSuchPane)`. SDTEST-3000, SDTEST-2995. |
| Snapshot and history decoders expanded run-length cells without a budget (66 KiB to more than 8 GiB) | Fixed: decode budgets, rows beyond the grid's room are not decoded, a chunk beyond the requested history closes the connection. SDTEST-2997. |
| A connection slot leaked when the writer's descriptor could not be cloned | Fixed: the slot is an RAII guard that follows the connection; a reader panic no longer holds it either. SDTEST-3001. |
| Accept loop spinning at 100% CPU on `EMFILE` | Fixed: spare descriptor to accept and refuse with `LimitExceeded`, 50 ms backoff on other errors. SDTEST-3001. Later: the daemon raises its own `RLIMIT_NOFILE` (SDTEST-3028), and a connection accepted with no descriptor left for its writer is told `LimitExceeded` too instead of being closed without a word. |
| Handshake and frame deadlines only checked on a read timeout | Fixed: checked on every turn; an oversize first frame is refused on its length bytes. SDTEST-2978. The frame deadline now restarts whenever a frame completes, so a long paste is not cut. |
| Run directory keyed on a host name that can change | Fixed: a live daemon in a sibling directory is preferred (`paths::live_run_dir`). SDTEST-3002. Later: the directory is named after the machine identity (SDTEST-3025). |
| Idle-exit race: a reset instead of EOF was not retried; a start during the old daemon's exit did nothing | Fixed on the client: reset, broken pipe and unexpected EOF before `HelloAck` count as "not running", and the start is repeated every 250 ms while the socket is missing. SDTEST-3003. Later, in the daemon: the idle exit reads the listener once more after taking the socket out of the way, the lock is released before the grace period and a start waits for it (SDTEST-3029, SDTEST-2983). |

Side effects to know:

- `PaneInfo.clients` no longer counts an attachment dropped for `Resync`
  until it attaches again.
- `Error(NoSuchPane)` is now also sent to the connection that removed the
  pane, if it had it attached.
- A pane whose shell exited stays listed as dead until its tab is closed or
  for an hour, which also holds off the daemon's idle exit for that time.
- `default_run_dir()` may open and close one connection to a daemon in a
  sibling directory when the directory for the current host name has no
  socket.



### Daemon hardening, version skew and packaging

Third step after the prototype. Tests on Linux only; the app was not
launched, the release pipeline was not run and nothing was published.

Tests that were still to write, and what they changed:

| Test | What it pins | Changed in the code |
|---|---|---|
| SDTEST-2981 | A program that does not read its terminal holds back only the connection pasting into it; after the stall timeout the queue is discarded with `Error(Busy)` for that pane; the pane can still be killed | The PTY master is non-blocking and the writer thread waits with `poll`: it sees a discarded queue (it drops the frame it holds) and a closed pane, where it used to stay inside `write` until the program exited. Progress is now bytes accepted by the PTY, not frames taken from the queue. Other connections waiting for room are woken when a queue is discarded. |
| SDTEST-2982 | The size follows the client that typed last; an active owner keeps it for 1 s; a resize request of a non-owner is only remembered | Nothing: `Pane::note_input` behaved as designed. |
| SDTEST-2983 | `SIGTERM`: the socket goes at once, a pane that ignores the hangup lives through the grace period and is then killed with its group, and a new daemon serves the directory meanwhile | Lock handoff (section 3). `serve --term-grace-ms` and `--idle-exit-ms` exist for tests and are not in the usage text. |
| SDTEST-2984 | A parser panic ends only its pane (`Exited`, code -1, signal 0), the chunk that panicked is not forwarded, the program is hung up, everything else goes on | Nothing in the logic. `ServerConfig::fault_parser_panic` is the injection point (hidden, never set by the binary). |

Finished from the review list:

- Descriptor limit (SDTEST-3027, SDTEST-3028): as in section 3. Seen in
  the test: daemon soft limit 100 raised to 4096, a command pane and a
  login shell pane both at 100, the login shell with `argv[0]` `-sh`, not
  ignoring or blocking `SIGHUP`, a missing program still refused.
- Writer descriptor (SDTEST-3001 extended): the test now runs under 13
  and under 12 descriptors. With 12 a connection is accepted and has no
  descriptor left for its writer: it used to be closed without a word and
  is now told `LimitExceeded`. This was found because the log file takes
  one descriptor more than before.
- Idle exit and lock (SDTEST-3029): as in section 3. The race is made
  certain in the test by a hook (`ServerConfig::before_leaving`, hidden,
  never set by the binary) that connects after the daemon's last ordinary
  `accept`: the connection is served and the daemon stays. With the last
  read of the listener disabled that test fails with a reset connection
  (checked by hand, once). The test also sweeps a connect across the
  moment of the exit 40 times; that part passes with or without the fix
  and only guards the two allowed outcomes. A daemon killed during the
  30 ms of the exit leaves `sessiond-v1.sock.leaving` behind; the next
  daemon removes it.
- Run directory (SDTEST-3025): as in section 3. `default_run_dir()` is
  used by the app, the proxy and `serve` alike, so a remote host follows
  the same rule.
- Log (SDTEST-3026, SDTEST-3028): as in section 3. `tracing-subscriber`
  is a new dependency of the crate (already in the workspace).
- History budget (SDTEST-3030): as in section 3. Accounting uses the
  current column count for every history row.

Version skew (SDTEST-3031, SDTEST-3032; SDTEST-3012 and SDTEST-3013
extended):

- Compatible: nothing special. `LocalSessions::daemon_version()` exposes
  the announced version and the supervisor logs once per connection when
  it differs from its own.
- Incompatible: the handshake error becomes `AttachError::OtherVersion`;
  the supervisor remembers it (`daemon_is_other_version`), a session ends
  at once in `AttachedPhase::OtherVersion` with the notice
  `terminal.session_daemon.other_version`, and keeps its pane id. The tab
  shows an error state with "managed by another version of ShellDeck".
- At startup `restorable_daemon_panes()` returns
  `RestorablePanes::OtherVersion`; the saved daemon tabs are rebuilt from
  the saved layout (`WorkspaceState::saved_daemon_panes`) and each pane
  reports the state. Which of them still run is unknown to this app. The
  layout saved at the next quit keeps them.
- A new local terminal is an in-process one while the flag is set, and
  each such open rechecks in the background (`Supervisor::recheck`) so
  daemon panes come back once the other daemon has ended.
- What cannot be done for such a daemon: list, attach, end its panes
  (closing the tab or signing out leaves them running), or tell the user
  which version it is (the refusal carries no version).
- The test daemon for the incompatible case is a stand-in that answers
  every `Hello` with `UnsupportedVersion` and records what else it
  receives (nothing). No real daemon of another protocol version exists
  to test against. The UI part (`restore_session`, the fallback in
  `daemon::spawn`, the tab text) is compiled and read only.
- A remote host with an incompatible daemon still falls back to tmux or a
  plain shell (unchanged); its panes are not shown as tabs.

Packaging:

- `scripts/build-release.sh` builds `shelldeck-sessiond` (release
  profile) on Linux. `scripts/package-release.sh` requires it and puts it
  in `shelldeck-linux-x86_64.tar.gz` beside `shelldeck`, with the static
  remote builds when they exist (only the two shipped names, each checked
  to be an ELF file).
- The install script (`cloudflare/update-worker/src/index.ts`) copies
  those files into `~/.shelldeck/bin` by rename. It takes effect when the
  worker is deployed, which was not done.
- The updater (`shelldeck-update::installer::SessiondPayload`) stages the
  files beside the installed ones before the application binary is
  swapped and renames them into place after; a failed update removes the
  staged copies. An archive without them is accepted.
- A running daemon keeps the file it was started from in all three cases
  (replacement is always a rename), and starts its panes through
  `/proc/self/exe`.
- The AppImage does not carry the daemon: the image is a mount that goes
  away with the application, and a daemon outlives it. Shipping it there
  needs a copy to a stable path first (not done). The macOS bundle and
  the Windows archive are unchanged.
- Static remote builds: `scripts/build-sessiond-remote.sh` checks the
  targets, builds with `crt-static`, checks the ELF machine and that the
  result has no interpreter, and writes
  `target/release/shelldeck-sessiond-<triple>`. It is opt-in
  (`SHELLDECK_SESSIOND_REMOTE=1`) and was never executed. An open risk:
  the crate links `shelldeck-core` through `shelldeck-terminal`, and with
  it crates that expect system libraries at link time (D-Bus through the
  keyring); the glibc build drops them as unused, a static musl link may
  not. If so, the terminal crate has to lose that dependency first.
- Contract tests: `scripts/test-sessiond-packaging.py` (archive contents,
  the rendered installer run under a temporary HOME, the remote build
  script's checks) and SDTEST-3033 for the updater.

### First run in the application

Until this step every statement about the app came from tests and from
reading the code. The app was then built (debug) and driven on a private
headless desktop, with a throwaway profile and its own run directory. The
record, check by check, is in `docs/testing/session-daemon-qa.md`.

What the run changed:

| Found | Change |
|---|---|
| Once the last tab had been closed, no refresh loop ran any more: the loop owned the only receiver of the output channel and was dropped with it. The next terminals repainted on input events only, which made daemon panes look frozen (their output arrives after the key press that caused it). The defect predates the daemon and also affects in-process terminals. | `TerminalView::ensure_refresh_running` creates a new channel and rewires the live sessions when the receiver is gone. SDTEST-3037. |
| `TerminalGrid::resize` left the screen with fewer rows than its row count after merging soft-wrapped lines on a widening. The next whole-screen erase or scroll indexed past the end and panicked. In the daemon that is a parser panic, which closes the pane (section 12): a tab opened from a narrow split and then `clear` lost its shell. The defect predates the daemon; an in-process terminal loses its reader thread the same way. | The screen is padded back to its row count. Because the grid state for the same input changed, `EMULATOR_REV` is 2. SDTEST-3038. |
| The description of the setting did not fit the two lines a Settings row shows. | Shorter text, English and French. |

What the run confirmed about the design: the daemon and its shells keep
their process ids across an app quit and across `kill -9` of the app; the
layout comes back by pane id with ratios, active tab and focused pane; a
killed daemon gives the two notices and an error dot, and the next tab
starts a new daemon; sign-out ends every pane; the daemon leaves 60 s
after the last connection is gone, not 60 s after the last pane (the app
keeps its connection while it runs); an app with emulator revision 2
works against a daemon still running revision 1.

Open risk noted during the run, not verified: after a narrowing reflow
the grid holds more rows than its row count until it is widened again
(existing behaviour of `reflow_on_shrink`). If a snapshot does not carry
those rows, a client that attaches in that state and the daemon would
reflow differently at the next widening, and the client would show a
different wrap until its next snapshot. The applied `Resize` frame only
covers rows pulled back from the history (`pulled`). Either the snapshot
has to carry the extra rows or `resize` has to normalise the screen to
its row count in both directions; the second also changes what an
in-process terminal shows after a split, so it was left alone here.

Not exercised in the app: the remote path, a daemon of another protocol
version, the connecting and reconnecting states as such, packaging.
