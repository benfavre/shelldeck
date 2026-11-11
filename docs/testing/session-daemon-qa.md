# Session daemon: QA record for the local opt-in (2026-11-11)

Scope: build step "opt-in client integration" of the session daemon
prototype (SDUC-574, design in
[`session-daemon.md`](../design/session-daemon.md), section 15, "Local
client integration"). Branch `feat/session-daemon`, Linux x86_64.

**The real application was not launched for this step.** No visual check
was done: not with the setting on, not with it off. Everything under
"Verified automatically" is tests and compile checks. Everything under
"Manual checks still needed" is unverified.

## What changed for a user

Nothing unless `session_daemon = true` is added to the `[general]` table of
`config.toml` (default false, no Settings toggle). With it on, on Unix:

- a new local tab or split of the main terminal runs in a pane owned by the
  per-user `shelldeck-sessiond` daemon (started on demand);
- closing the tab or the pane ends its shell; quitting the app does not;
- at the next start, panes still running come back, one tab each.

Workspace-scoped terminals, SSH tabs and the tmux mode are not touched.

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
| The real daemon under a 12 descriptor limit: extra connections are refused at once, 40 failed attempts cost no connection slot | SDTEST-3001 (real binary, foreground, private directory) |
| A daemon started under an earlier host name is found; a start that lands while the old daemon is exiting is repeated | SDTEST-3002, SDTEST-3003 |
| The real daemon binary is started on demand; looking for panes never starts one; a relative binary path is refused; a pane survives the loss of every session and of the connection, and is found again with its screen | SDTEST-2996 (`shelldeck-sessiond`, real binary, detached) |
| The setting defaults to false and an older config file without the key still loads | existing `shelldeck-core` config tests (default values test extended; the legacy file fixture has no such key) |
| `shelldeck-ui` and `shelldeck` compile with the wiring; the `shelldeck-ui` test targets compile | `cargo check --locked -p shelldeck-ui -p shelldeck`, `cargo check --locked -p shelldeck-ui --tests` |

Not covered by any automated test: every line of the GPUI wiring
(`terminal_view/daemon.rs`, the branch in `spawn_configured_local`,
`close_tab`, `close_split`, `restore_daemon_panes`, the call in
`Workspace::restore_session`). It was compile-checked and reviewed by
reading only. The `shelldeck-ui` test suite was not run. This includes the
review fix in `daemon.rs` (panes offered back are attached with
`keep_pane_size`): the library behaviour behind it is SDTEST-2998, the
call site itself is unverified.

## Manual checks still needed

Setup for a development build: build both binaries
(`cargo build -p shelldeck -p shelldeck-sessiond`) so that
`shelldeck-sessiond` sits next to the app executable, or point
`SHELLDECK_SESSIOND_BIN` at an absolute path. Use a throwaway profile, not
the installed app. The daemon then uses the real run directory,
`~/.shelldeck/run/<host>/` (`sessiond-v1.sock`, `sessiond-v1.lock`).
`shelldeck-sessiond list` shows the panes; `kill <pid from the lock file>`
(SIGTERM) stops the daemon and every pane.

Setting off (regression):

- [ ] New tab, split, close tab, close pane, quit: unchanged; no
      `shelldeck-sessiond` process appears.

Setting on, basics:

- [ ] First new tab: the daemon starts (one `shelldeck-sessiond serve`
      process), the prompt appears without visible delay; note the delay of
      the very first tab (daemon start blocks the UI thread, up to 3 s).
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
- [ ] Split right and split down: each pane is its own daemon pane
      (`shelldeck-sessiond list` shows one per leaf); focus switching; zoom.
- [ ] Close a pane, close a tab with several panes, "close tabs to the
      right/left": the corresponding shells are gone within about 1 s
      (`list`), the others stay.
- [ ] `exit` in a shell: the tab behaves like an in-process tab after exit
      and keeps its final screen; the pane stays in `list` as `exited(n)`
      until the tab is closed, then it is gone. If the app is quit with
      such a tab open, the dead pane is not offered back and expires after
      an hour (the daemon does not idle out before that): decide whether
      that is acceptable.
- [ ] Ctrl-C during `yes` on a slow or busy machine (and later over SSH):
      the flood stops even if the tab was being resynced.
- [ ] Heavy output (`yes`, `cat` of a large file, `find /`): UI stays
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

- [ ] Quit the app with tabs open (menu quit, window close, Ctrl-Q): the
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
- [ ] Layout after restart: splits come back as separate tabs (known
      limit); confirm nothing worse happens.
- [ ] Two app instances at once (if the app allows it): both list the same
      panes; the second start attaches panes the first one already shows.
      Record the behaviour (shared typing, size ownership). `exit` in one
      instance: the other shows the final output too; closing the tab in
      one leaves the other on its last screen.
- [ ] Change the host name while panes run (`hostnamectl set-hostname`,
      or on macOS a network change), then restart the app: the panes are
      found again and no second daemon is started.
- [ ] Daemon idle exit: close every tab, wait more than 60 s, the daemon is
      gone; a new tab starts it again.

Failure modes:

- [ ] `shelldeck-sessiond` missing next to the app: new tabs are in-process
      terminals, a warning is logged, nothing is shown to the user (decide
      whether that is acceptable).
- [ ] Kill the daemon while tabs are open: each tab shows the "connection
      lost" line and freezes; a new tab works (new daemon); the frozen tabs
      can be closed.
- [ ] Run directory with wrong permissions (`chmod 755`): refused, fallback
      to in-process, never repaired.
- [ ] Home on a path so long that the socket path does not fit: the
      `/tmp/shelldeck-<uid>` fallback is used.
- [ ] Turn the setting off again with panes still running: they are not
      offered back and keep running until closed with
      `shelldeck-sessiond kill` or a daemon stop. Decide whether the app
      should warn.

Platforms and packaging:

- [ ] macOS: nothing was built or run (peer check, 104-byte socket path,
      app bundle location of the daemon binary).
- [ ] Windows: the backend is a stub; confirm the app builds
      (`cargo check --target x86_64-pc-windows-msvc`) and the setting is
      inert.
- [ ] Packaging: no bundle, installer or updater ships `shelldeck-sessiond`
      yet.

## Known gaps (not bugs to report again)

Listed with their reasons in the design document, section 15 (status
summary and the subsections after it): no Settings toggle, no layout
restore, workspace-scoped terminals stay in-process, no reconnect after a
daemon loss, blocking calls on the UI thread, no remote (SSH) use of the
daemon in the app, no version check against a daemon left by another app
version, the daemon does not raise its descriptor limit (about 80 panes
under a 256 limit, about 330 under 1024, before new connections are
refused with `LimitExceeded`), no daemon log file.

## Review fixes (2026-11-11)

Nine review findings were fixed after the step above; the table is in the
design document, section 15, "Review fixes". Automated coverage is in the
table "Verified automatically" (SDTEST-2997..3003, SDTEST-2978 extended,
SDTEST-2995 changed). None of the fixes was checked in the running app.
