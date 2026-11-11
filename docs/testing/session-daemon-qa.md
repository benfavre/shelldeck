# Session daemon: QA record for the local opt-in (2026-11-11)

Scope: build steps "opt-in client integration" and "complete the local app
integration" of the session daemon prototype (SDUC-574, design in
[`session-daemon.md`](../design/session-daemon.md), section 15, "Local
client integration"). Branch `feat/session-daemon`, Linux x86_64.

**The real application was launched and driven on 2026-11-11, after these
steps** (private headless desktop, throwaway profile): see the section
"Checks in the running application" at the end of this file, which says
for each check whether it passed and what was not checked. Three defects
were found there and fixed. Before that run nothing had been seen on
screen: everything under "Verified automatically" is tests and compile
checks. In "Manual checks still needed", a ticked box means the main case
of the item was done in that run, as its row in the results table
describes; variants the item names and the table does not (for example
quitting from the menu, or closing the window) were not done. An empty box
is still unverified.

## What changed for a user

Nothing unless "Persistent terminal sessions" is turned on in Settings,
General (dev-capable roles, Unix; `session_daemon = true` in the
`[general]` table of `config.toml`, default false). It applies to new
terminals only. With it on:

- a new local tab or split of the main terminal runs in a pane owned by the
  per-user `shelldeck-sessiond` daemon (started on demand). The tab opens
  at once with a "connecting" line and shows the shell when the pane is
  attached; nothing waits for the daemon on the UI thread;
- closing the tab or the pane ends its shell; quitting the app does not;
  signing out ends every pane;
- if the connection to the daemon is lost, each terminal says so and
  reattaches by itself; a terminal whose pane did not survive (the daemon
  died) says the session ended instead of freezing;
- at the next start, panes still running come back in their saved layout:
  tabs, splits, ratios, active tab and focused pane. A pane the saved
  layout does not know becomes a tab; a saved slot whose pane is gone is
  dropped.

Workspace-scoped terminals stay in-process (decision and reason in the
design document, section 15, "Workspace-scoped terminals"). SSH tabs are
covered by the section "Remote (SSH) use" at the end of this file; with
the setting off they and the tmux mode behave as before.

## Verified automatically

All on private temporary run directories; no test touches `~/.shelldeck`,
an existing socket, tmux or a user process.

| What | How |
|---|---|
| An attached session shows the daemon's snapshot, then live output; ignores output that precedes the snapshot; never answers terminal queries; never resizes its own grid from the view; sends one resize request per size change; goes quiet after a resize it cannot reproduce until the next snapshot; drop closes the input channel | SDTEST-2992 (`shelldeck-terminal`, no daemon) |
| Attach, echo through the PTY, drop, reattach from a new connection: same shell pid, old screen back before any input; double attach refused | SDTEST-2993 (in-process daemon, real PTY) |
| A resize request reaches the child (`stty size`) and comes back to the client grid | SDTEST-2994 |
| `terminate` (closed tab) removes the pane and its process; a shell that exits stays listed as dead with its final screen, also for a second app that attaches later, until its tab is closed; other panes are untouched | SDTEST-2995 |
| A pane attached without a size (app restart) keeps its 40x150 size and its 140 column history line; the first size the session sends is then applied and reaches the child | SDTEST-2998 (in-process daemon, real PTY) |
| A client dropped for being slow can still send Ctrl-C to the flooding pane before it reattaches | SDTEST-2999 |
| When one client closes a dead pane, another attached client still receives the final output, the exit and a "pane is gone" marker | SDTEST-3000 |
| A hostile snapshot or history chunk (66 KiB describing more than 8 GiB) is refused before allocation | SDTEST-2997 (`shelldeck-terminal`) |
| The real daemon under a 13 and a 12 descriptor limit: extra connections are told `LimitExceeded` at once, 40 failed attempts cost no connection slot | SDTEST-3001 (real binary, foreground, private directory) |
| A daemon started under an earlier host name is found; a start that lands while the old daemon is exiting is repeated | SDTEST-3002, SDTEST-3003 |
| The real daemon binary is started on demand; looking for panes never starts one; a relative binary path is refused; a pane survives the loss of every session and of the connection, and is found again with its screen | SDTEST-2996 (`shelldeck-sessiond`, real binary, detached) |
| A supervised session is returned before the daemon is reached (the test delays the connect), says "connecting", goes live by itself, delivers input typed before the pane existed, publishes its pane id; the same pane cannot be opened twice; after the connection is dropped (daemon alive) it reattaches to the same shell without a second daemon; a pane opened by id from a second supervisor keeps its 24x80 size although that session was created 10x40; an unknown pane id is reported as ended; after `SIGTERM` to the daemon the session shows the "lost" then "ended" notices, a daemon is running again and new terminals work; `terminate` removes the pane; `exit 3` ends the session with code 3 and no notice | SDTEST-3006 (`shelldeck-sessiond`, real binary, detached, private directory) |
| A daemon binary that cannot be started makes the session unavailable with the reason on screen and creates nothing | SDTEST-3007 |
| Reconnect delays (doubling, capped) and the reconnect rules: when to retry, when a pane is ended, when it is unavailable, exit code kept | SDTEST-3004, SDTEST-3005 (pure, no I/O) |
| The saved session file carries the daemon layout per tab; files written before still load; restore plan: live panes keep tree and ratios, dead slots dropped and splits collapsed, unknown panes appended as tabs, active tab remapped, damaged files (duplicate pane, bad ratio) tolerated | SDTEST-3008..3010 (`shelldeck-core`) |
| Native split tree to daemon pane ids and back (shape, directions, ratios; leaves that are not live daemon panes left out); tab state for each pane phase | SDTEST-3011, SDTEST-3012 (`shelldeck-ui`, no GPUI) |
| In a real `TerminalView` (GPUI test context, attached sessions without a daemon): the layout captured for saving, tab by tab; a tab whose pane does not exist yet is not saved; an ended pane is left out; the saved pane gets focus back when a restored tab is activated; sign-out ends every pane of every tab and split, an app quit ends none; a workspace-scoped view never takes daemon panes | SDTEST-3013 (`shelldeck-ui`) |
| The Settings toggle is offered to dev-capable roles on Unix only | SDTEST-1718 extended (`settings.rs`) |
| The setting defaults to false and an older config file without the key still loads | existing `shelldeck-core` config tests (default values test extended; the legacy file fixture has no such key) |
| `shelldeck-ui` and `shelldeck` compile with the wiring; the `shelldeck-ui` test targets compile | `cargo check --locked -p shelldeck-ui -p shelldeck`, `cargo check --locked -p shelldeck-ui --tests` |

Suites run for this step, all green: `cargo test --locked -p
shelldeck-sessiond` (26 tests), `-p shelldeck-terminal`, `-p shelldeck-core
--lib config::`, `-p shelldeck-ui --lib -- --test-threads=1` (388 tests, with disposable
XDG roots as `.agents/testing.md` requires), `cargo fmt --all -- --check`,
`cargo check --locked -p shelldeck-ui -p shelldeck`, `cargo clippy --locked
-p shelldeck-ui -p shelldeck-sessiond -p shelldeck-terminal -p
shelldeck-core --all-targets -- -D warnings`. `shelldeck-ssh` was compiled
but its tests were not rerun (the crate is not changed by this step).

One observation outside this work: in one of two parallel runs of the
`shelldeck-ui` suite (default thread count),
`agent_console_view::interaction_tests::rendered_transcript_reveal_copy_and_collapse_keep_the_composer_accessible`
failed on its clipboard read. It passes alone (three runs) and in the
serial run above, and touches no terminal code; it looks like tests
sharing the test clipboard. Not investigated.

Not covered by any automated test, compile-checked and reviewed by reading
only:

- `terminal_view/daemon.rs` (the process-wide supervisor, the resolver, the
  forwarded environment). No UI test opens a daemon session through it: it
  would use the real run directory.
- `TerminalView::restore_daemon_tab` (it opens sessions through that
  backend) and the whole of `Workspace::restore_session` /
  `finish_restore_session`: the background listing, the order of daemon
  and SSH tabs, the active tab choice. The plan they execute is
  SDTEST-3009; the execution is unverified.
- The phase to tab state sync inside the refresh loop and the pending
  colour of the tab dot (the mapping itself is SDTEST-3012).
- The Settings row itself (only its visibility rule is tested).
- The call to `end_daemon_panes` in `invalidate_cloud_session`.
- `close_tab`, `close_split` and the branch in `spawn_configured_local`
  (unchanged since the previous step, still unverified in the app).

## Manual checks still needed

Setup for a development build: build both binaries
(`cargo build -p shelldeck -p shelldeck-sessiond`) so that
`shelldeck-sessiond` sits next to the app executable, or point
`SHELLDECK_SESSIOND_BIN` at an absolute path. Use a throwaway profile, not
the installed app. The daemon then uses the real run directory,
`~/.shelldeck/run/m-<16 hex digits>/` (`sessiond-v1.sock`,
`sessiond-v1.lock`, `sessiond.log`; named after the machine identity, see
the section "Daemon hardening, version skew and packaging").
`shelldeck-sessiond list` shows the panes; `kill <pid from the lock file>`
(SIGTERM) stops the daemon and every pane.

Setting off (regression):

- [x] New tab, split, close tab, close pane, quit: unchanged; no
      `shelldeck-sessiond` process appears.

Setting on, basics:

- [ ] Settings, General: the row "Persistent terminal sessions" (French:
      "Sessions de terminal persistantes") is there for a dev-capable
      account, absent for a user-only account; toggling it saves and does
      not touch open tabs; the next new tab uses the new value.
- [ ] First new tab: the tab opens at once with the "connecting" line and
      a pending (warning colour) dot, the daemon starts (one
      `shelldeck-sessiond serve` process), then the prompt replaces the
      line and the dot turns green. The window must stay responsive during
      the daemon start. Type immediately after opening the tab: the keys
      must arrive once the shell is there, in order.
- [ ] "Launch Claude" / "Launch Codex" from the empty state with the
      setting on: the command is typed into the new pane after it
      attaches (it is sent before the pane exists).
- [x] Open five tabs quickly while no daemon runs: one daemon, five panes.
- [ ] Typing, paste (bracketed), Ctrl-C, Ctrl-D, Ctrl-Z, mouse reporting
      (`htop`, `vim` with mouse), focus reporting.
- [ ] Rendering parity with an in-process tab: colours, wide and combining
      characters, alt screen (`vim`, `less`), cursor shape and blink, OSC 8
      links, title in the tab (OSC 0/2), URL detection.
- [ ] Scrollback: scroll while output flows, selection and copy, search,
      scroll position after a resize.
- [ ] Window resize and divider drag: the grid follows after the daemon
      applies the size (expect up to about 50 ms lag); no garbled wrap; a
      full-screen program redraws once. Check rapid drags.
- [x] Split right and split down: each pane is its own daemon pane
      (`shelldeck-sessiond list` shows one per leaf); focus switching; zoom.
- [x] Close a pane, close a tab with several panes, "close tabs to the
      right/left": the corresponding shells are gone within about 1 s
      (`list`), the others stay.
- [x] `exit` in a shell: the tab keeps its final screen and its dot shows
      the exit state (unlike an in-process tab, which stays green today);
      the pane stays in `list` as `exited(n)`
      until the tab is closed, then it is gone. If the app is quit with
      such a tab open, the dead pane is not offered back and expires after
      an hour (the daemon does not idle out before that): decide whether
      that is acceptable.
- [ ] Ctrl-C during `yes` on a slow or busy machine (and later over SSH):
      the flood stops even if the tab was being resynced.
- [x] Heavy output (`yes`, `cat` of a large file, `find /`): UI stays
      responsive, no lost or duplicated lines, the app catches up; note
      whether a resync flash is visible.
- [ ] Environment: `echo $PATH`, `$DISPLAY`, `$SSH_AUTH_SOCK`, `$LANG`,
      `ssh-add -l`, a GUI program started from the pane. A daemon pane has
      the login shell's environment plus a short forwarded list, not the
      app's environment: record what is missing compared with an in-process
      tab.
- [ ] `terminal.default_shell` set (for example `/bin/zsh`, `fish`) and
      unset.
- [ ] AI and script features that type into the terminal (tracked
      commands, OSC 133 capture, "launch Claude/Codex" buttons).

Persistence:

- [x] Quit the app with tabs open (menu quit, window close, Ctrl-Q): the
      daemon and the shells keep running; start again: one tab per pane
      with its screen and scrollback; typing works; a long-running command
      (`sleep 600`, a build) is still running.
- [ ] Size after restart: before quitting, make the window wide and print
      lines wider than 80 columns. After the restart those history lines
      are not cut at 80 columns; a full-screen program (`htop`) in the
      active tab gets at most one resize, to the real window size; a pane
      in a background tab is not resized until its tab is shown
      (`shelldeck-sessiond list` shows the old size until then).
- [ ] Same when the app is launched from the desktop launcher, from a
      terminal (then close that terminal), and when the app is killed
      (`kill -9`): the daemon must survive all three.
- [ ] Same across an app update done by the updater (old daemon keeps
      serving; note the version it reports).
- [ ] With `auto_connect_on_startup` on and saved local and SSH tabs: local
      tabs are not duplicated, SSH tabs still reconnect, the active tab is
      sensible.
- [x] Layout after restart: build three tabs, one of them split in three
      with uneven ratios, focus a non primary pane, make the second tab
      active, quit, start again: same tabs in the same order, same splits
      and ratios, same active tab, same focused pane, each pane showing
      its own screen. Scroll positions are not restored (known limit).
- [ ] Same, but before the restart end one pane of the split from outside
      (`shelldeck-sessiond kill <pane>`): its slot is gone, the split
      closes around the others. End every pane of one tab: that tab does
      not come back.
- [ ] Same, but `kill -9` the app instead of quitting: the layout of the
      last clean quit is used; panes made since then come back as tabs of
      their own (known limit: the layout is saved at shutdown only).
- [x] A pane created with the debug CLI (`shelldeck-sessiond spawn`) is
      not offered back (other label). A pane of this app that is in no
      saved slot comes back as its own tab, after the saved ones.
- [x] Restored tab in the background: `shelldeck-sessiond list` shows its
      old size until the tab is shown, then the window's size.
- [x] Sign out with daemon tabs open: every pane is ended
      (`shelldeck-sessiond list` is empty within a few seconds); signing
      in again shows no old terminal.
- [ ] Two app instances at once (if the app allows it): both list the same
      panes; the second start attaches panes the first one already shows.
      Record the behaviour (shared typing, size ownership). `exit` in one
      instance: the other shows the final output too; closing the tab in
      one leaves the other on its last screen.
- [ ] Change the host name while panes run (`hostnamectl set-hostname`,
      or on macOS a network change), then restart the app: the panes are
      found again and no second daemon is started.
- [x] Daemon idle exit: close every tab and quit the app, wait more than
      60 s, the daemon is gone; a new tab starts it again. While the app
      runs it keeps its connection, so the daemon stays even with no tab
      (by design: idle means no pane and no connection).

Failure modes:

- [x] `shelldeck-sessiond` missing next to the app: new tabs are in-process
      terminals, a warning is logged, nothing is shown to the user (decide
      whether that is acceptable).
- [x] Kill the daemon while tabs are open (`kill <pid from the lock
      file>`): each tab shows the "connection lost, reconnecting" line,
      then, within about a second, the "session ended" line and an error
      dot; a daemon is running again; a new tab works; the ended tabs can
      be closed. No tab stays frozen without a message.
- [ ] Connection loss with the daemon alive (hard to provoke by hand:
      `gdb -p <app pid>` and `call (int)shutdown(<socket fd>, 2)`, or a
      debug build calling `Supervisor::drop_connection`): each tab shows
      the notice briefly, then its screen again, same shell (`echo $$`),
      scrollback intact; keys typed during the gap are lost (by design).
- [ ] `chmod 000` the daemon binary while the app runs with daemon tabs,
      then kill the daemon: the tabs end ("session ended"), a new tab
      shows "session daemon unavailable" with the reason. Decide whether
      a new tab should fall back to an in-process terminal in that case
      (today it only does so when the binary is missing when the tab is
      opened).
- [ ] Close a tab while it is still "connecting" (slow first start) and
      while it is "reconnecting": no pane is left behind
      (`shelldeck-sessiond list`).
- [ ] Run directory with wrong permissions (`chmod 755`): refused, fallback
      to in-process, never repaired.
- [ ] Home on a path so long that the socket path does not fit: the
      `/tmp/shelldeck-<uid>` fallback is used.
- [x] Turn the setting off again with panes still running: they are not
      offered back and keep running until closed with
      `shelldeck-sessiond kill` or a daemon stop. Decide whether the app
      should warn.

Platforms and packaging:

- [ ] macOS: nothing was built or run (peer check, 104-byte socket path,
      app bundle location of the daemon binary).
- [ ] Windows: the backend is a stub; confirm the app builds
      (`cargo check --target x86_64-pc-windows-msvc`) and the setting is
      inert.
- [ ] Packaging: see the section "Daemon hardening, version skew and
      packaging" at the end of this file.

## Known gaps (not bugs to report again)

Listed with their reasons in the design document, section 15 (status
summary and the subsections after it): workspace-scoped terminals stay
in-process (excluded, not deferred), the layout is saved at shutdown and
sign-out only, scroll positions, zoom and the focus of background tabs are
not restored, no automatic fallback once a tab has been opened as a daemon
tab, input typed while disconnected is dropped, notices keep the language
of the first daemon session. The gaps this list used to end with (no
version check, daemon not packaged, descriptor limit, no log file) were
closed later: see the section "Daemon hardening, version skew and
packaging", which has its own list of what is still open.

## Review fixes (2026-11-11)

Nine review findings were fixed after the step above; the table is in the
design document, section 15, "Review fixes". Automated coverage is in the
table "Verified automatically" (SDTEST-2997..3003, SDTEST-2978 extended,
SDTEST-2995 changed). None of the fixes was checked in the running app.

## Remote (SSH) use (2026-11-11)

Scope: step "remote use of the session daemon in the app, with tmux as
fallback" (design document, section 15, "Remote app integration").

**The real application was not launched and no real SSH server was used.**
There is no `sshd` on this machine (`ssh -o BatchMode=yes localhost true`
is refused), so the planned live test against localhost was not written.
The remote side of every test is the in-memory SSH fixture: a real russh
client and server over an in-process pipe, the real `/bin/sh` under a
private temporary `HOME`, and a real daemon on a private run directory.
The workspace code in `workspace/ssh.rs` (the tab hand-over, the fallback
notice, the install question and its answer handling, the host events)
is compiled and read, never executed.

### What changes for a user

Nothing with the setting off. With "Persistent terminal sessions" on, for
an SSH tab opened without a directory:

- the tab opens at once as before. If the server has the session daemon
  (`~/.shelldeck/bin/shelldeck-sessiond-<app version>`), the tab becomes a
  pane of it: further tabs and splits of that server use the same SSH
  connection, closing a tab ends its shell, quitting the app does not, and
  connecting again brings the running shells back as tabs;
- if the server does not have it and the app ships a build for that
  server, a toast asks once: install (copies the program to
  `~/.shelldeck/bin` on the server, no `sudo`), later, or never for this
  server. The current tab is not held up: it uses tmux (when automatic
  tmux attachment is on) or a plain SSH shell;
- in every other case (no build for that server, unsupported system,
  refused, failed upload, a restricted account, an incompatible daemon
  already running) the tab uses tmux or a plain shell and one toast says
  why, once per server and per run.

Today no package ships the server builds, so outside a development setup
every server falls back.

### Verified automatically

| What | How |
|---|---|
| The per-server answer is stored in `[general.session_daemon_hosts]` as `allow` or `never`, omitted when empty, absent means "not asked" | SDTEST-3014 (`shelldeck-core`) |
| Decision table: an installed binary is used whatever the answer; an upload happens only with a recorded "allow" and a bundled build for the server's platform; each other case names its reason; the question is offered only when an install is possible and nobody answered; connect errors map to their fallback reason; the fallback backend is the choice made with the setting off | SDTEST-3015 (pure) |
| Only a checked version, nonce and digest can reach a remote script; the probe answer is found among rc noise and only with the right nonce; a local file is an artifact only by name and as an ELF of the right machine | SDTEST-3016 (pure, temporary directories) |
| Probe and upload against the real `/bin/sh` under a private `HOME`: nothing written by a probe; upload gives the same bytes, mode 0700 in 0700 directories, the remote digest equals the local one, no temporary file left, nothing else in the home; a file without the execute bit is reported and replaced; six fixed `sh -c` commands, no `sudo` | SDTEST-3017 |
| A byte changed in transit: checksum mismatch, the previous install untouched, temporary file removed; a server with no digest tool: nothing installed | SDTEST-3018 |
| The host object: not asked gives a fallback that offers the install, writes nothing and hands the working SSH connection back; "never", a wrong local file, a missing build, a damaged upload, a refused exec each give their reason and install nothing unverified; an unreachable server is a connect failure, not a fallback; an install after the user's answer works and releases the connection; then the binary is used without any answer | SDTEST-3019 |
| End to end through the tunnel: consented upload, then a tab and a split spawn two panes that echo their own input over one SSH connection and one exec channel; the SSH connection is cut: both sessions reattach over a new one with their screens; sessions dropped (app quit): connection released, panes alive; next connect lists both, one is reattached with its screen; closing it ends only that pane | SDTEST-3020 |
| The same with nothing simulated on the remote side: the real binary (a stripped build) uploaded by the real script, started by the real proxy command under a private `HOME`, a pane echoing, surviving and reattached | SDTEST-3024 (opt-in, run once for this step with `SHELLDECK_SESSIOND_BIN`, green) |
| Which panes a connecting tab takes and which come back as tabs, also when several restored tabs connect at once | SDTEST-3021 (`shelldeck-ui`, pure) |
| In a real `TerminalView`: an SSH tab (or split) is handed over to a daemon pane in place, the old session dropped and not terminated; tabs of an SSH connection are not saved as local daemon layouts | SDTEST-3022 |
| A question toast is not pushed out by later notifications, closing it chooses nothing, an answer runs its handler | SDTEST-3023 (no rendering) |
| `shelldeck-ui` and `shelldeck` compile with the wiring | `cargo check --locked -p shelldeck-ui -p shelldeck` |

Suites run for this step, all green: `cargo fmt --all -- --check`;
`cargo test --locked -p shelldeck-ssh --lib -- --test-threads=1`;
`cargo test --locked -p shelldeck-sessiond`; `cargo test --locked -p
shelldeck-core --lib config::app_config`; `cargo test --locked -p
shelldeck-ui --lib -- --test-threads=1` (disposable XDG roots);
`cargo check --locked -p shelldeck-ui -p shelldeck`; `cargo clippy --locked
-p shelldeck-ssh -p shelldeck-sessiond -p shelldeck-core -p shelldeck-ui
--all-targets -- -D warnings`. Counts are in the commit message.

### Not verified

- Anything against a real `sshd`: authentication, jump hosts, login
  banners and rc files of real accounts, a non-POSIX login shell (the
  scripts run through `sh -c`), a `noexec` home, a forced command.
- The static musl builds: none was built; the uploaded binary of
  SDTEST-3024 is a stripped glibc build for this machine.
- `workspace/ssh.rs`: `run_ssh_daemon_or_shell`, the `DaemonReady` and
  `DaemonFallback` handlers, the fast path for a host that is already
  connected, the split path, the connection status after `Idle`,
  `ConnectionLost` and `Reconnected`, the install answer handler.
- The question toast on screen (buttons, wrapping, light and dark theme).
- That the setting off path is unchanged was checked by reading the diff
  (the shell body moved into `run_ssh_shell`); no SSH tab was opened.
- macOS and Windows builds.

### Manual checks still needed

Setup: a throwaway profile, a test server you own, and a daemon build for
the server placed as `shelldeck-sessiond-x86_64-unknown-linux-musl` (or
`aarch64`) in a directory named by `SHELLDECK_SESSIOND_ARTIFACT_DIR`.

- [ ] Setting off: SSH tab, split, duplicate, tmux attachment on and off,
      disconnect notification: unchanged; nothing appears under
      `~/.shelldeck` on the server.
- [ ] Setting on, server without the binary, never asked: the tab works
      at once (tmux or plain), the question toast appears once, stays
      until answered, and does not block typing. "Later": nothing in the
      configuration file, asked again after an app restart, not again in
      the same run.
- [ ] "Never for this server": `never` in
      `[general.session_daemon_hosts]`, no further question, one notice
      per run.
- [ ] "Install": `allow` recorded, the "installing" then "installed"
      toasts, on the server only
      `~/.shelldeck/bin/shelldeck-sessiond-<version>` (0700) is new. The
      next tab of that server is a daemon tab.
- [ ] Daemon tab: prompt appears, typing, resize, full-screen programs;
      a second tab and a split do not open a second SSH connection
      (`ss -tnp` or the server's `who`); each is its own shell.
- [ ] Close one tab: its shell is gone on the server
      (`shelldeck-sessiond-<version> list` there), the others stay. Close
      the last one: the sidebar shows the server as disconnected.
- [ ] Quit the app with daemon tabs open, start again, connect: the
      shells come back as tabs with their screens. With
      `auto_connect_on_startup` and three saved tabs of one server: three
      tabs, not five.
- [ ] Cut the network for a few seconds: the tabs say "connection lost",
      the sidebar shows "connecting", then the screens are back, same
      shells. Cut it for more than a minute: the tabs end as
      "unavailable"; connecting again offers the shells back.
- [ ] Server where the binary cannot run (`noexec` home, other
      architecture without a build, forced command): plain or tmux tab
      and one notice with the reason.
- [ ] Sign out with remote daemon tabs open: the remote shells are ended.
- [ ] Edit a connection entry to point at another server while a daemon
      tab of the old one is open: the old tab keeps working, a new tab
      talks to the new server.

### Known gaps (remote)

Listed in the design document, section 15, "Remote app integration": the
server builds are not built or packaged, the split tree of remote tabs is
not saved, tabs opened in a directory keep the plain path, old remote
binaries are not pruned, no Settings control to change a recorded answer,
a tab closed while the server is unreachable leaves its shell running, no
remote daemon from a Windows client, no latency measurement.

## Daemon hardening, version skew and packaging (2026-11-11)

Scope: third step (design document, section 15, "Daemon hardening, version
skew and packaging").

**The real application was not launched, no release was built or
published, and the install script was not deployed.** The daemon was run
only by the tests, on private temporary directories. The updater was not
run against a real install. Everything below "Manual checks still needed"
is unverified.

### What changed for a user

- A terminal whose program does not read its input no longer ties up the
  daemon: after 10 s without progress the pasted input still waiting is
  dropped (only once more than 1 MiB is waiting).
- Stopping the daemon (`kill`, logout) and starting a new terminal right
  away works without waiting for the old daemon's terminals to end.
- Programs started in a terminal see the same open-file limit as before
  (`ulimit -n`), while the daemon itself can hold more terminals.
- The daemon's directory no longer depends on the host name. A daemon
  started by an earlier build, in a directory named after the host, is
  still found.
- `~/.shelldeck/run/<machine>/sessiond.log` says when the daemon started,
  which terminals were created and ended, and why something was refused.
  It holds no terminal content.
- A terminal with a very long history may lose its oldest lines when all
  terminals together hold more than 2 GiB of history; terminals nobody
  looks at lose theirs first.
- After an update that changes the daemon's protocol, terminals still held
  by the old daemon are shown as tabs that say they are managed by another
  version. They keep running and are not touched. New terminals are
  ordinary ones until the old daemon has ended.
- Linux installs made with the install script or updated by the updater
  get `shelldeck-sessiond` beside `shelldeck`. The AppImage, macOS and
  Windows do not have it: the setting then uses ordinary terminals.

### Verified automatically

| What | How |
|---|---|
| A program that never reads holds back only the connection pasting into it; the waiting input is discarded with `Busy`; the terminal can still be killed | SDTEST-2981 (in-process daemon, real PTY in raw mode) |
| The size follows the client that typed last, with one second of protection for the active one | SDTEST-2982 |
| `SIGTERM`: socket gone at once, a program ignoring the hangup killed after the grace period with its group, a new daemon serving meanwhile | SDTEST-2983 (real binary, foreground, private directory) |
| A parser panic ends only its pane | SDTEST-2984 (fault injection) |
| The daemon's own descriptor limit is raised, a command and a login shell keep the original one, a missing program is still refused; the log is private and holds no terminal content | SDTEST-3027, SDTEST-3028 (real binary, private `HOME`, Linux) |
| Out of descriptors, both ways, the client is told so | SDTEST-3001 extended |
| A start during an exit waits for the lock; a client that connects after the daemon's last accept and before its socket is out of the way is served and the daemon stays | SDTEST-3029 (the race is forced by a test hook; fails when the fix is disabled) |
| Run directory name: stable across host names, distinct per machine, host name fallback | SDTEST-3025 (pure) |
| Log: mode, cap, one old generation, symlink not followed | SDTEST-3026 |
| History budget: unattached panes first, newest rows kept, screens intact, no loop on an impossible budget | SDTEST-3030 |
| A daemon announcing another version, same protocol, is used and nothing is started; a daemon refusing the protocol is sent nothing but `Hello`, nothing is started next to it, its sessions say so and keep their pane id; this version's daemon starts once the other is gone | SDTEST-3032 (the incompatible daemon is a stand-in) |
| Saved daemon tabs are rebuilt from the saved layout when the daemon cannot list; such a tab is saved again with its pane id; its tab state is an error with its own text | SDTEST-3031, SDTEST-3013 and SDTEST-3012 extended |
| Updater: daemon files staged, published by rename, removed when the update fails; invalid files stop the update | SDTEST-3033 (the helper only, not a real update) |
| Archive contents; the rendered install script run under a temporary HOME, replacing by rename; the remote build script's checks | SDTEST-3034..3036 (`scripts/test-sessiond-packaging.py`) |
| The release profile of the daemon builds: 1.8 MB, links libc, libm and libgcc only | `cargo build --locked --release -p shelldeck-sessiond` |

Suites run for this step, all green at the end: `cargo test --locked -p
shelldeck-sessiond` (37 tests; green in the last four runs in a row),
`-p shelldeck-terminal` (112), `-p shelldeck-core --lib` (545, 7 ignored
as before), `-p shelldeck-update` (19), `-p shelldeck-ssh sessiond --
--test-threads=1` (11) plus the opt-in SDTEST-3024 with the release build
of the daemon, `-p shelldeck-ui --lib -- --test-threads=1` (391, disposable
XDG roots), `python3 scripts/test-sessiond-packaging.py`,
`scripts/test-browser-packaging.py` and `scripts/test-agents-packaging.py`,
`cargo fmt --all -- --check`, `cargo clippy --locked -p shelldeck-sessiond
-p shelldeck-terminal -p shelldeck-ssh -p shelldeck-core -p shelldeck-ui
-p shelldeck-update --all-targets -- -D warnings`, `cargo check --locked
-p shelldeck`.

Two things seen while running them:

- SDTEST-2971 failed once in a full parallel run on its last assertion
  (the daemon's offset against the client's, read before the last bytes
  of the echo had reached the client). The assertion now waits for the
  client to catch up. The race was in the test, not in the daemon.
- After a full run, up to two daemon processes of finished tests were
  still alive for under a minute (they then left by idle exit): repeated
  starts that had been waiting for the lock and took over when the test
  stopped its daemon. The tests now stop every daemon of their private
  directory when they end; none was left after the last four runs.

Not covered by any automated test, compiled and read only:

- The UI side of the other-version case: `Workspace::restore_session` /
  `finish_restore_session` with `RestorablePanes::OtherVersion`, the
  fallback and background recheck in `terminal_view/daemon.rs::spawn`, the
  notice and tab texts on screen.
- `install_linux` as a whole (it replaces the running executable).
- `scripts/build-release.sh` end to end, `release.yml`, the worker
  deployment.
- `scripts/build-sessiond-remote.sh` beyond its checks: no musl target is
  installed here, so no static build was ever produced or uploaded.
- macOS: the `kern.uuid` lookup, `exec-pane` and the descriptor limit
  code are not compiled here (no target installed). Windows: not checked.

### Manual checks still needed

Daemon:

- [x] `cat ~/.shelldeck/run/*/sessiond.log` after a session: start line
      with version and pid, one line per terminal created and ended,
      nothing that was typed or printed. Let it pass 1 MiB (a loop of
      short-lived terminals): `sessiond.log.1` appears once and the pair
      stays under about 2 MiB.
- [ ] `ulimit -n` and `ulimit -Hn` in a daemon terminal equal those of an
      in-process terminal; `cat /proc/<daemon pid>/limits` shows 4096 (or
      the hard limit) for the daemon. `ps -o pid,args` right after opening
      a tab: no `exec-pane` process is left behind; the shell is a login
      shell (`echo $0` starts with a dash) and reads the same profile
      files as before.
- [ ] Open more than 340 terminals (script `shelldeck-sessiond spawn`):
      beyond what a 1024 limit allowed, up to the 256 pane cap, without
      `LimitExceeded`.
- [ ] Paste several MiB into `sleep 1000` (after `stty raw`): other tabs
      stay usable; after about 10 s the tab reports the paste as dropped
      (note what the user actually sees: the `Busy` error has no visible
      notice in the app today).
- [ ] `kill <daemon pid>` with a tab running `trap '' HUP; sleep 1000`,
      then open a new tab at once: it works within a second, on a new
      daemon; the old process is gone after about 3 s.
- [ ] Close every tab, wait for the idle exit (60 s) while opening a new
      tab at the 60 s mark, several times: the tab always opens.
- [ ] Upgrade from a build of the earlier steps with panes running: the
      panes are found (daemon in `run/<host>`), no second daemon in
      `run/m-…`; after that daemon has ended, the new directory is used.
- [ ] Two machines on one NFS home: two directories, each machine sees
      only its own terminals.
- [ ] A machine without `/etc/machine-id` (some containers): the host
      name directory is used.
- [ ] History: three terminals with `scrollback` at 100 000 lines and a
      wide window, `serve --history-budget 64M` (start the daemon by
      hand): the unattended ones are trimmed first; note whether the app's
      own copy of the history (which is not trimmed) hides the effect.

Another version:

- [ ] Cannot be produced with released builds (there is one protocol
      version). With a development build that refuses protocol 1: start
      the old app, open three tabs with a split, quit; start the new app:
      the same tabs come back, each saying it is managed by another
      version, error dot, no shell; `shelldeck-sessiond list` with the old
      binary still shows the panes alive; a new tab is an ordinary
      terminal; quit and start the old app: the panes are back in their
      layout.
- [ ] Decide whether such tabs should offer an action (today the text
      only says to use the other version or close the tab; closing does
      not end the pane, and sign-out does not end those panes either).
- [ ] Same protocol, other version (any two builds with different version
      numbers): panes come back normally; the app log has one line "using
      a session daemon left by another version of the app".

Packaging (needs a release build, which was not made):

- [ ] `bash scripts/build-release.sh` on Linux: the archive lists
      `shelldeck`, `shelldeck-sessiond` and `browser/<version>/…`.
- [ ] Fresh install with the install script (after the worker is
      deployed) into a throwaway HOME: `~/.shelldeck/bin/shelldeck-sessiond`
      exists; with the setting on, a new tab is a daemon pane.
- [ ] Update by the updater from this version to a later one while daemon
      tabs are open: the tabs keep working during and after the update,
      `~/.shelldeck/bin/shelldeck-sessiond` is the new file, the running
      daemon is the old one (its version in the log) until it idles out,
      and it can still open new terminals (they start through
      `/proc/self/exe`).
- [ ] Update from a release without the daemon to one with it, and the
      reverse (a rollback): the app starts in both cases.
- [ ] AppImage: the setting falls back to ordinary terminals and nothing
      is left running after quit.
- [ ] `SHELLDECK_SESSIOND_REMOTE=1 bash scripts/build-release.sh` on a
      machine with the musl targets and linkers: both static builds are
      produced and pass the script's checks, or the link fails (then see
      the open risk in the design document); with them in place, the
      first connect to a Linux server offers the install.

### Known gaps of this step

- The static remote builds do not exist yet (script wired, never run).
- The AppImage, macOS and Windows packages do not carry the daemon.
- A daemon of another protocol version cannot be listed, so its tabs are
  the saved ones, alive or not, and they cannot be ended from this app.
- The `Busy` answer to a dropped paste is not shown to the user.
- Old `shelldeck-sessiond-<version>` files on SSH servers are still not
  pruned.

## Checks in the running application (2026-11-11)

Scope: the first run of the real application with the setting, on Linux
x86_64 (Ubuntu 22.04), debug build of this branch
(`cargo build --locked -p shelldeck -p shelldeck-sessiond`).

How it was run, so that it can be repeated without touching a real
profile or a real desktop: a private headless GNOME Shell
(`gnome-shell --headless --wayland --no-x11 --virtual-monitor 1400x900`
under its own `dbus-run-session`, own `HOME` and `XDG_RUNTIME_DIR`), the
app started on that Wayland display with a throwaway `HOME` and XDG
directories, the demo account of `scripts/demo` (stub Manage server on a
local port, Dev mode), a D-Bus session without service directories.
Input came from `org.gnome.Mutter.RemoteDesktop`, screenshots from
`org.gnome.Shell.Screenshot`. The daemon therefore used
`<throwaway HOME>/.shelldeck/run/m-…`; the real `~/.shelldeck` was never
written and no installed ShellDeck was started, stopped or signalled.
Both the throwaway `HOME` and the runtime directory were reached through
short symbolic links: the real paths were too long for a socket address
(the Wayland socket could not be created at all, and the daemon would
have taken its `/tmp/shelldeck-<uid>` fallback). The login shell was
`bash`, the interface French, then English for the Settings check.

Five screenshots are kept in
[`screenshots/2026-11-11-session-daemon/`](./screenshots/2026-11-11-session-daemon/).
The others (file names in the table) were in a scratch directory and are
not kept.

### Defects found and fixed

| Defect | Cause | Fix | Test |
|---|---|---|---|
| After the last tab was closed, the next terminal repainted only on input events: typed text, command output and everything a daemon pane printed stayed invisible until a key press or a mouse move (`27-refresh-bug.png`, then `28-refresh-fixed.png`). First seen as two split panes that looked frozen. | Not from this branch: closing the last tab drops the refresh task, and the task owned the only receiver of the output channel, so no refresh loop could start again. With in-process terminals a key press usually hid it; with daemon panes it did not. | `TerminalView::ensure_refresh_running` builds a new channel and rewires the live sessions when the receiver is gone. | SDTEST-3037 |
| `clear` in a new tab ended the terminal: the daemon logged `parser panicked, closing pane` (`grid.rs`, index out of range) and the tab was left with an exit dot and no shell. | Not from this branch: `TerminalGrid::resize` merges soft-wrapped lines when a terminal gets wider and left the screen with fewer rows than its row count; the next whole-screen erase indexed past the end. A new tab is started at the size of the pane that had the focus, then widened, so a tab opened from a narrow split hit it at once. An in-process terminal has the same defect (its reader thread dies). | The screen is padded back to its row count after the merge. `EMULATOR_REV` is now 2, because the grid state for the same input changed. | SDTEST-3038 |
| Settings, General: the description of "Persistent terminal sessions" was cut in the middle of a sentence (rows show two lines). | Text too long. | Shorter text in English and French (`settings-fr.png`, `settings-en.png`). | Seen on screen |

Also fixed: SDTEST-3028 read `/proc/<pid>/cmdline` right after the pane
printed its limits and failed once when the `exec` had not happened yet
(busy machine); it now waits for it.

### Results

| # | Check | Result | Screenshot |
|---|---|---|---|
| 1 | Setting off: new tab, split (Ctrl+Shift+D), close split, close tab; shells are children of the app; no `shelldeck-sessiond` process, no `~/.shelldeck` directory. Repeated on the final build, including a new tab after the last one was closed. | Pass | `05-off-split.png`, `07.png`, `09-off-closed.png`, `69-off-final.png` |
| 2 | Settings row in French and in English, for a Dev account; the toggle saves `session_daemon` and does not touch open tabs; the next new tab uses the new value (on then off). | Pass after the text fix | `settings-fr.png`, `settings-en.png` (kept), `10-settings.png` (before) |
| 3 | Setting on, first tab: one `shelldeck-sessiond serve --detach`, the shell is a child of the daemon, keys typed right after the click arrive in order. | Pass. The "connecting" line and the pending dot were never caught: the pane was attached before the first screenshot. | `13-on-first-b.png` |
| 4 | Split right, split again: one daemon pane per leaf (`shelldeck-sessiond list`), each with its own shell pid; closing one pane ends that shell only. | Pass after the refresh fix | `15-on-splitH.png`, `23.png`, `25.png` |
| 5 | Window resize (maximize and restore), divider drag: the grid follows, `top` redraws to the new size. | Pass | `29-top.png`, `30-top-max.png`, `38-before-quit.png` |
| 6 | Full-screen programs: `top`, `less` (alternate screen) and back to the shell with the earlier screen. `vim` is not installed on the machine. | Pass | `29-top.png`, `53-vim.png`, `54-after-vim-yes.png` |
| 7 | Heavy output: `seq 1 200000` complete and in order, last line 200000; `yes abc \| head -200000 \| tail -2`; Ctrl-C stops `yes`; screenshots and scrolling worked while it ran. Debug build: 15.8 s in a daemon pane, about 16 s in an in-process tab (both limited by the unoptimised parser). | Pass | `31-seq.png`, `35-bottom.png`, `55-yes-ctrlc.png`, `61-inproc-seq.png` |
| 8 | Scrollback with the mouse wheel, during output and afterwards. | Pass | `32-scrolled.png`, `34-scrollback.png` |
| 9 | Quit (Ctrl+Q) with two tabs, the first split in three with uneven ratios, a counter loop and `sleep 600` running, focus on the middle pane: daemon and shells alive with the same pids and start times. Relaunch: same tabs, splits, ratios, active tab and focused pane, screens intact, the counter still counting, scrollback there; the background tab kept its size until shown. | Pass | `layout-before-quit.png`, `layout-after-relaunch.png` (kept), `40-restored-tab2.png`, `42-restored-scroll.png` |
| 10 | Close a tab with four panes: the four shells are gone within 2 s, the other tab's stay. | Pass | `26-closed-tab2.png` |
| 11 | `exit 3` in a shell: final screen kept, exit dot, `exited(3)` in `list` until the tab is closed. | Pass | `56-exit.png` |
| 12 | `kill <daemon pid>` with two tabs open: both tabs get the error dot, "connection lost, reconnecting" then "session ended" in the terminal, a new daemon runs, a new tab works on it, the ended tabs close. | Pass. In a pane scrolled up in its history the two lines are below the visible part. | `daemon-killed.png` (kept), `45-newtab-after-kill.png` |
| 13 | `kill -9` of the app: the daemon and the panes survive; relaunch attaches them (as tabs of their own, the layout being the one of the last clean quit). | Pass | `64-after-kill9.png` |
| 14 | Setting turned off with panes running, quit, relaunch: nothing is offered back, the panes keep running; setting on again: they come back as plain tabs. A pane made with `shelldeck-sessiond spawn` is never offered. | Pass | `60-off-with-panes.png`, `62-relaunch-off.png`, `63-relaunch-on.png` |
| 15 | Daemon binary missing (`SHELLDECK_SESSIOND_BIN` pointing nowhere, no daemon running): the new tab is an in-process terminal, one warning in the app log, no daemon. | Pass | `65-missing-binary.png` |
| 16 | Five tabs opened at once with no daemon running: one daemon, five panes. | Pass | `66-five-tabs.png` |
| 17 | Sign out with five daemon tabs: `list` empty within 3 s. | Pass | `68-signed-out.png` |
| 18 | Idle exit: 60 s after the app quit with no pane left, the daemon logged `IdleExit` and removed its socket. With the app running and no tab it stays (the app keeps its connection). | Pass | none (process list and log) |
| 19 | App built from emulator revision 2 against a daemon still running revision 1: the pane came back, resizes and output stayed correct. | Pass | `50-rev-skew-max.png` |
| 20 | Daemon log after the session: start and stop lines, one line per terminal created and ended, nothing typed or printed; run directory `0700`, files `0600`. | Pass | none |
| 21 | Environment of a daemon pane: login shell (`-bash`, `login_shell on`), `LANG` and `WAYLAND_DISPLAY` forwarded, `TERM=xterm-256color`, working directory the home directory, parent process the daemon (no `exec-pane` left). `COLORTERM` is empty in both kinds of terminal. | Pass | `52-env.png` |
| 22 | Remote use over SSH. | Not run: `ssh -o BatchMode=yes localhost true` is refused on this machine (no sshd), and no real server may be used. | none |

### Not checked in this run

- The "connecting" and "reconnecting" states as such (too short to
  capture locally), a tab closed while connecting, a connection lost with
  the daemon still alive.
- Paste, mouse reporting, focus reporting, OSC 8 links, the tab title,
  selection and copy, search, zoom, rapid divider drags, wide and
  combining characters.
- "Launch Claude" and "Launch Codex" (neither is on the `PATH` of the
  throwaway profile), `terminal.default_shell`, AI features that type
  into the terminal.
- The descriptor limit: soft and hard limits are both 1048576 on this
  machine, so the daemon raised nothing.
- A daemon of another protocol version, the log rotation, the history
  budget, more than 256 panes, large pastes, two app instances, a host
  name change, wrong permissions on the run directory.
- Everything remote (SSH), packaging, the updater, the install script,
  macOS and Windows.
- A release build: all timings above are from a debug build.

### Observations, not changed

- With the side panel hidden, the terminal area leaves an empty strip as
  wide as the panel on its right until the panel is toggled once
  (Ctrl+B). Same with the setting off, not from this branch.
- The Terminal menu entries "Split horizontally" and "Split vertically"
  did nothing when clicked, and keyboard focus left the terminal until
  the next click in it (the action is dispatched to the focused element,
  which is the menu). Same with the setting off; the toolbar buttons and
  Ctrl+Shift+D work.
- After a pane has been split, the toolbar offers "Rotate" and "Close
  split" where "Split H" and "Split V" were; a further split needs the
  shortcut.
- A new tab is started at the size of the pane that had the focus and
  resized on its first frame (seen in the daemon log as a spawn at 39 or
  45 columns followed by a resize).
- Narrowing and then widening a pane rewraps old lines differently from
  the original (existing reflow of the emulator, same in both kinds of
  terminal). After a narrowing reflow the grid holds more rows than its
  row count until it is widened again. If a snapshot leaves those rows
  out, a client that attaches in between could differ from the daemon
  after a widening, until its next snapshot. This was read in the code,
  not verified and not seen on screen; it is listed as an open risk in
  the design document, section 15.
- When the window is shorter at the next start, the top rows of each
  screen go to the history (existing behaviour on a row shrink).
- The language select still showed "Anglais" right after switching to
  English.
