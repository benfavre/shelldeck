# SDTEST inventory: `shelldeck-sessiond`

> Rules for this file live in [`.agents/testing.md`](../../.agents/testing.md).
> Use case IDs (`SDUC-…`) resolve in [`USE_CASES.md`](./USE_CASES.md).

**Big picture.** The session daemon (SDUC-574, design in
[`session-daemon.md`](../design/session-daemon.md)). The codec tests are
plain unit tests. The integration tests in `tests/daemon.rs` run a real
daemon, real PTYs and the real client library: the daemon runs in-process
on a private temporary run directory, except for the tests marked "real
binary", which start the binary on its own private directory (and, for a
login shell, under a private `HOME`) and stop it with `SIGTERM` at the
end. Every private directory also stops, when its test ends, any daemon
process still running on it (found by its `--dir` argument; Linux). No
test touches `~/.shelldeck`, an existing socket, tmux or any user
process. The whole crate runs in about fifteen seconds:
`cargo test --locked -p shelldeck-sessiond`.

The snapshot, history and stream boundary tests live with the code they
cover, in `shelldeck-terminal` (SDTEST-2955..2965, see
[`tests-terminal.md`](./tests-terminal.md)).

Unix only (`#![cfg(unix)]`); verified on Linux. macOS is not verified.

## 1. `protocol.rs`: wire protocol v1

| ID | Location | SDUC | Status | Notes |
|---|---|---|---|---|
| SDTEST-2966 | `protocol.rs::sdtest_2966_every_frame_type_round_trips` | SDUC-574 | Green | Every frame of both directions; one frame pinned byte for byte (the wire layout is a contract); the hot-path `Output` encoder matches the typed one. |
| SDTEST-2967 | `protocol.rs::sdtest_2967_frames_fail_closed_on_direction_version_type_and_trailing_bytes` | SDUC-574 | Green | Wrong direction, wrong version byte, unknown type, trailing bytes, every truncation of every frame. |
| SDTEST-2968 | `protocol.rs::sdtest_2968_oversize_length_is_rejected_before_the_payload_is_buffered` | SDUC-574 | Green | Length above 1 MiB or below 2 fails on the four length bytes alone; frames split at arbitrary points come out whole and in order; encoding refuses what decoding would refuse. |
| SDTEST-2969 | `protocol.rs::sdtest_2969_spawn_rejects_disallowed_env_and_out_of_range_fields` | SDUC-574 | Green | Hand-built `Spawn` bodies: environment allowlist (`PATH`, `LD_PRELOAD` and others refused), rows, cols, scrollback, label, argv and env limits, the 64 KiB frame cap, invalid UTF-8, bad magic, bad signal, unknown error code. `SSH_CONNECTION` and `SSH_CLIENT` are accepted (given per pane by the client), `SSH_TTY` is not. |
| SDTEST-2970 | `protocol.rs::sdtest_2970_proxy_command_interpolates_only_a_checked_version_and_nonce` | SDUC-574 | Green | The fixed remote command and the nonce marker; shell metacharacters in version or nonce are refused. |
| SDTEST-2986 | `protocol.rs::sdtest_2986_proxy_command_at_accepts_only_a_plain_absolute_path` | SDUC-574 | Green | The fixed remote command for a configured absolute binary path: relative paths, `.` and `..` components, quotes, spaces, `$`, `;`, newlines, non-ASCII and over-long paths are refused. |

## 2. `tests/daemon.rs`: daemon, client and proxy

| ID | Location | SDUC | Status | Notes |
|---|---|---|---|---|
| SDTEST-2971 | `sdtest_2971_spawn_attach_and_input_echo` | SDUC-574 | Green | Spawn (canonical cwd returned, missing cwd and a shell-style command line refused), attach, input, echo; pane list fields; the pane child dies with the daemon. |
| SDTEST-2972 | `sdtest_2972_pane_survives_detach_and_reattach_reproduces_the_screen` | SDUC-574 | Green | Client disconnect leaves the child alive; a new client's snapshot equals the previous client's grid byte for byte; explicit detach behaves the same. |
| SDTEST-2973 | `sdtest_2973_heavy_output_attach_race_loses_and_duplicates_nothing` | SDUC-574 | Green | A pane floods 1.5 MB of seeded output (SGR, cursor movement, wide characters, scrolling, scroll regions, alt screen) written in slices that cut sequences. One client records the whole stream, another attaches and detaches six times mid-flood, a third attaches once. Each late attach is compared with a replay of the recorded stream at its own offset and a little later; at the end every grid equals the daemon's and every byte is accounted for exactly once. A mutation that drops the parser tail from the snapshot was caught in one run out of three (the deterministic check for the tail is SDTEST-2955). |
| SDTEST-2974 | `sdtest_2974_slow_clients_are_dropped_and_resynced_without_stalling_the_pane` | SDUC-574 | Green | A raw client that never reads is dropped at the (lowered) queue cap with `Error(Resync)` for its attach; a library client with a tiny feed cap resyncs after backoff and ends on the daemon's grid; a healthy client is never disturbed. |
| SDTEST-2975 | `sdtest_2975_two_clients_share_a_pane_and_get_exactly_one_query_reply` | SDUC-574 | Green | Three attached emulators, one DA1 reply reaches the application; input from two writers; a read-only attachment gets `ReadOnly` for input, resize and kill. |
| SDTEST-2976 | `sdtest_2976_pane_exit_is_reported_after_the_final_output` | SDUC-574 | Green | `Exited` carries the status and follows the final output; the dead pane stays listed and attachable; `Kill` on it removes it; a signal death carries the signal; a missing pane is an error for that pane only. |
| SDTEST-2977 | `sdtest_2977_single_instance_idle_exit_stale_socket_and_private_directory` | SDUC-574 | Green | Second instance does nothing, socket 0600 in a 0700 directory, idle exit (held off by a connected client), stale socket replaced, a 0755 directory and a symlinked directory refused by daemon and client and never repaired. |
| SDTEST-2978 | `sdtest_2978_frames_before_hello_bad_frames_and_stalled_frames_only_detach` | SDUC-574 | Green | Frame before `Hello`, unsupported version range, unknown type, daemon-direction type, oversize length, disallowed env name, silence instead of `Hello`, half a frame then silence (timeouts injected): each closes only that connection. A first frame declaring 1 MiB is refused on its length bytes; a peer trickling one byte every 50 ms, before and after `Hello`, is closed at the deadline. |
| SDTEST-2979 | `sdtest_2979_backfill_and_applied_resize_keep_every_client_on_the_daemon_grid` | SDUC-574 | Green | Backfilled history equals the live history; a row grow is reproduced by clients that hold the pulled rows and forces a resync for one that does not; a refresh that missed no byte keeps history; a burst of 100 resize requests leaves all grids equal to the daemon's. |
| SDTEST-2999 | `sdtest_2999_a_client_dropped_for_resync_can_still_interrupt_the_flood` | SDUC-574 | Green | A raw client starts `yes`, reads nothing and is dropped at the queue cap (no longer counted in `clients`); its `Input` Ctrl-C, sent before any new `Attach`, kills the flood (pane exits with signal 2); the only error it ever received is `Resync` for its attach; a new `Attach` gets the final screen and `Exited`. |
| SDTEST-3000 | `sdtest_3000_removing_a_dead_pane_still_delivers_its_end_to_other_viewers` | SDUC-574 | Green | Two attachments, the pane exits, one client removes it while the other has read nothing: the other still receives the final output, `Exited`, then `Error(NoSuchPane)` for its attach as the last frame; the remover gets `NoSuchPane` too. |
| SDTEST-3001 | `sdtest_3001_descriptor_exhaustion_refuses_at_once_and_leaks_no_connection_slot` | SDUC-574 | Green | Real binary in the foreground under `ulimit -n 13`, then 12 (hard and soft, so the daemon cannot raise it): three, then two connections are served; 40 further attempts are each answered `LimitExceeded` within 2 s, whether `accept` itself fails (13) or the accepted connection has no descriptor left for its writer (12); after the held connections close the same number is served again, so no slot leaked. Linux only as written (descriptor count of the daemon: stdio, lock, listener, spare, log). On macOS the kernel closes a connection that `accept` cannot give a descriptor to, so that case may end as a close instead of the words there; the answer at once and the slot count are checked on both. The count of clients that fit is repeated until the daemon has released the connection the test probed it with. |
| SDTEST-3002 | `sdtest_3002_a_daemon_under_another_host_name_is_found` | SDUC-574 | Green | `paths::live_run_dir`: a daemon in a sibling directory is chosen when the preferred one has no socket; a sibling with an unserved socket is skipped; a daemon in the preferred directory wins; no daemon gives the preferred directory. The same when the preferred directory is outside the root (the `/tmp` last resort): the running daemon is not abandoned. |
| SDTEST-3003 | `sdtest_3003_a_start_during_the_old_daemons_exit_is_repeated` | SDUC-574 | Green | Socket unlinked while the old (in-process) daemon still holds the lock for 400 ms: `proxy::connect_or_start` reaches a new real daemon within the 3 s limit. Since the lock wait of SDTEST-3029 the first start already succeeds; the client's repeat stays for daemons that do not wait. |
| SDTEST-2980 | `sdtest_2980_proxy_starts_the_daemon_and_carries_a_full_session_over_stdio` | SDUC-574 | Green | Real binary: `proxy` starts a detached daemon, banner noise and a marker with another nonce are skipped, full spawn, attach, input, output cycle over a pipe pair, stdin EOF ends the proxy and leaves daemon and pane running, the pane child neither ignores nor blocks `SIGHUP`, oversize, late and missing preludes fail, `SIGTERM` leaves no pane process and no socket. |

## 3. `tests/daemon.rs`: hardening

| ID | Location | SDUC | Status | Notes |
|---|---|---|---|---|
| SDTEST-2981 | `sdtest_2981_stalled_input_blocks_only_the_pasting_connection_and_is_discarded` | SDUC-574 | Green | A pane in raw mode whose program never reads (in line mode the kernel drops input instead of holding the writer back), queue cap 128 KiB, stall 800 ms. 400 KiB are pasted: a request behind the paste on the same connection waits, while another connection lists panes and its own pane echoes; then `Error(Busy)` for that pane only, and the pasting connection answers again. The pane is alive, is killed with `SIGKILL` (exit with signal 9) and removed: its writer was not stuck in a write to the full terminal. |
| SDTEST-2982 | `sdtest_2982_size_follows_the_last_active_client_with_hysteresis` | SDUC-574 | Green | Two clients, 24x80 and 30x100. Attaching second does not take the size; the first key of the second client does. While it types every 100 ms for 1.5 s, ten keys of the first client change nothing. 1.2 s after its last key the first client's key takes the size back, and is then itself protected. A resize request of the non-owner is only remembered; the owner's applies. Both clients end on the daemon's grid. About 3.5 s. |
| SDTEST-2983 | `sdtest_2983_sigterm_kills_a_pane_that_ignores_the_hangup_after_the_grace_period` | SDUC-574 | Green | Real binary, `--term-grace-ms 2500`. After `SIGTERM` the socket is gone and an ordinary pane dies of the hangup, while a pane that ignores `SIGHUP` and the daemon are still there. A second daemon started at that moment serves the directory and holds the lock (the lock file names it) before the first has exited. The first exits 0 after the grace period and the stubborn process group is gone; the second is unaffected. Timing assertions allow for a slow machine by failing with "too slow to observe" rather than passing. |
| SDTEST-2984 | `sdtest_2984_a_parser_panic_ends_only_its_pane` | SDUC-574 | Green | `ServerConfig::fault_parser_panic` makes the parser panic on a marker one pane prints. Both connections attached to it get `Exited` with code -1 and signal 0 and never see the chunk; its program is hung up; the pane is listed dead with code -1. The other pane echoes, a new connection spawns and attaches a new pane, the dead pane is removed with `Kill`. The panic message appears in the test output. |
| SDTEST-3028 | `sdtest_3028_daemon_raises_its_descriptor_limit_and_panes_keep_the_original` | SDUC-574 | Green | Linux, real binary started with a soft limit of 100 under a private `HOME` and `SHELL=/bin/sh`. The daemon's own soft limit is 4096 (or the hard limit); a command pane reports soft 100 and the unchanged hard limit, and the pane process is the program itself; a login shell pane has `argv[0]` `-sh`, limit 100, `SIGHUP` neither ignored nor blocked; a missing program and a shell-style command line are `SpawnFailed`. The log exists with mode 0600, names the daemon pid and the limit it started with, has one line per spawn, and contains neither typed text, pane output nor command lines. A second `serve` writes nothing to it. Skipped (with a message) if the hard limit is 100 or less. |
| SDTEST-3029 | `sdtest_3029_idle_exit_and_lock_handoff_lose_no_start_and_no_connection` | SDUC-574 | Green | Lock taken and socket gone: a start with `lock_wait` zero gives up, a normal start waits about 300 ms and takes over; lock taken with a socket: gives up at once. A stale file under the leaving name is removed by the next daemon that gets the lock, not by one that does not. The race itself, through `ServerConfig::before_leaving`: a client connects after the daemon's last ordinary `accept` and before the socket is out of the way; it gets `HelloAck`, the daemon stays, the socket has its name back, another client is served, and the daemon leaves once alone. With the last read of the listener disabled this part fails (the connection is reset), checked once by hand. Then 40 rounds with an idle exit of 60 ms and a connect delayed by 40 to 196 ms: an accepted connection always gets `HelloAck` and holds the daemon up; otherwise there is no socket; both outcomes must occur. That sweep is only a guard on the outcomes: it passes with the fix disabled too. About 10 s. The timing sweep of its last part starts at no delay (it began at 40 ms and once missed the daemon every time on a loaded macOS runner). |
| SDTEST-3030 | `sdtest_3030_history_over_budget_is_trimmed_unattached_panes_first` | SDUC-574 | Green | Budget for 4000 history rows of 80 columns, checked every 20 ms. One pane with 3000 rows is within budget. A second, attached the whole time, fills up too: the unattached one is halved twice, the attached one keeps everything, both screens are intact, and what is left of the trimmed history is its newest part without a gap. With a budget of zero a history ends at one row, the screen stays and the daemon keeps serving. |

Unit tests of the same step, next to their code:

| ID | Location | SDUC | Status | Notes |
|---|---|---|---|---|
| SDTEST-3025 | `paths.rs::sdtest_3025_run_directory_name_tells_machines_apart` | SDUC-574 | Green | `run_dir_name`: `m-` and 16 hex digits from the machine identity and the host name, showing neither; same name whatever the trailing newline or case; another identity or another host name (two clones of one image on a shared home) gives another name; a hardware UUID is accepted; `run_dir_names` adds the 8 digit form and the host name; no identity, a blank one, `uninitialized`, a short one, one with a slash or one over 128 bytes fall back to the host name. |
| SDTEST-3045 | `paths.rs::sdtest_3045_a_long_home_path_gets_a_shorter_run_directory_name` | SDUC-574 | Green | `run_dir_under`: the full name up to the longest home path it fits (socket path of exactly 107 bytes), then the 8 digit name, then the host name, and `/tmp/shelldeck-<uid>` only when none fits; every result holds a socket path that fits, for home paths of 2 to 119 bytes. |
| SDTEST-3066 | `paths.rs::sdtest_3066_a_daemon_keeps_nothing_of_the_appimage_mount` | SDUC-574 | Green | `appimage_env_changes`: with `$APPDIR` set, every entry of a `:` separated value that lies inside the image is dropped (`PATH`, `LD_LIBRARY_PATH`, any other name), a value made only of such entries is removed, the runtime's own variables (`APPDIR`, `APPIMAGE`, `OWD`, `ARGV0`) are removed, a directory whose name merely starts like the image's is kept. Without `$APPDIR`, or with a relative one, nothing changes. Used by `proxy::start_daemon` and by the daemon on itself. |
| SDTEST-3067 | `tests/attached.rs::sdtest_3067_refused_input_is_reported_on_the_session_status` | SDUC-574 | Green | In-process daemon, input queue 64 KiB, stall 300 ms. A pane in raw mode whose program never reads gets 400 KiB pasted through `TerminalSession::write_input`: the daemon's `Error(Busy)` is counted on the session's `AttachedStatus` (`take_dropped_input` > 0, read once per batch), the session stays `Live` with its screen. The terminal view turns the count into a warning toast (`toast.terminal.input_dropped`) instead of a log line only. |
| SDTEST-3068 | `workspace/remote_daemon.rs::sdtest_3068_a_saved_split_tree_takes_its_panes_back` | SDUC-574 | Green | `ready_plan_with`: an SSH tab reconnecting with a saved split tree takes the panes of the tree that are alive, ours and shown by no other client, keeping the tree's shape and focus (dead panes and panes shown elsewhere collapse their split); its leftmost pane is the tab's own. Panes kept for other reconnecting tabs (`reserved_panes`) are neither taken nor offered as tabs. One pane left: no splits. Nothing left: the plain plan (first free pane). `expect_layout` / `take_layout` / `end_connect(conn, Some(tab))` keep and release the reservation. |
| SDTEST-3069 | `terminal_view.rs::sdtest_3069_ssh_daemon_tabs_save_and_get_back_their_splits` | SDUC-574 | Green | `remote_tab_layouts` saves the split tree of SSH tabs made of daemon panes by pane id (plain SSH sessions and local tabs give `None`; `daemon_tab_layouts` never lists host panes). `install_daemon_splits` rebuilds a handed-over tab's splits in its stored layout when the tab is not active, keeps the saved title flag, and refuses an unknown tab. |
| SDTEST-3026 | `server/log.rs::sdtest_3026_log_is_private_capped_and_rotated_once` | SDUC-574 | Green | Mode 0600; never above the cap in either file over 200 records; exactly two names; the first generation is gone after the second rotation; an oversize record is cut; a reopened log continues; a symlink at the log name is not written through. |
| SDTEST-3027 | `server/launch.rs::sdtest_3027_launcher_command_and_program_lookup` | SDUC-574 | Green | The `exec-pane` command line for a login shell and for a command (own `argv[0]` kept, program resolved); a shell-style command line and a missing program give none; lookup rules (PATH, slash relative to the directory, directories and non-executable files refused); a limit that is high enough is left alone. |

## 4. `tests/attached.rs`: app facade (`attached::LocalSessions`, `supervisor::Supervisor`)

Starts the real daemon binary, detached, on a private temporary run
directory and stops it with `SIGTERM` (also when the test fails).
`cargo test --locked -p shelldeck-sessiond --test attached`. The backend
itself (attach, resize, terminate, attach without a size) is covered against
an in-process daemon by SDTEST-2992..2995 and SDTEST-2998 in [`tests-terminal.md`](./tests-terminal.md).

| ID | Location | SDUC | Status | Notes |
|---|---|---|---|---|
| SDTEST-2996 | `sdtest_2996_connect_starts_the_daemon_and_panes_survive_the_client` | SDUC-574 | Green | `connect_existing` on a directory without daemon returns none and creates nothing; a relative daemon path is refused before anything starts; `connect` starts the binary; a pane echoes; every session and the connection are dropped (app quit) and the daemon keeps running; a new `connect_existing` lists the pane, the attached grid shows the earlier screen and `$$` is the same pid; `terminate` removes the pane. |
| SDTEST-3006 | `sdtest_3006_supervised_session_connects_in_the_background_and_reconnects` | SDUC-574 | Green | `supervisor::Supervisor`: `open` returns in under 400 ms while the connect is delayed by 800 ms and no daemon runs; phase `Connecting` with the notice, then `Live` with the pane id; input typed before the pane existed is delivered; the same pane cannot be opened twice; `drop_connection` (daemon alive) is followed by a reattach to the same shell pid, same daemon; a second supervisor that has no daemon binary lists the pane and opens it by id with `keep_pane_size` and the grid ends 24x80 although created 10x40; an unknown pane id ends with the notice; `SIGTERM` to the daemon gives the "lost" and "ended" notices, phase `Ended { None }`, a new daemon pid and no claimed pane; a new session works on the new daemon; `terminate` removes the pane; `exit 3` gives `Ended { Some(3) }` without notice. |
| SDTEST-3032 | `sdtest_3032_a_daemon_of_another_version_is_used_if_compatible_and_left_alone_if_not` | SDUC-574 | Green | In-process daemon announcing version `0.1.0-earlier`: a supervisor that has its own binary lists the pane an earlier client left, attaches it, types into it and spawns a new pane there; no second daemon. Then a stand-in daemon that refuses every `Hello` with `UnsupportedVersion`: listing is `OtherVersion` with or without permission to start; a saved pane and a new one both end at once in `AttachedPhase::OtherVersion` with the notice, the saved one keeping its pane id; typing and closing reach nobody; the stand-in received only `Hello` frames, no lock or log file appeared and the socket inode is unchanged. `Reconnect` finishes on `OtherVersion` from any state. After the stand-in is gone, `recheck` clears the flag and a new session starts this version's daemon. The stand-in is not a real daemon of another protocol version (none exists). |
| SDTEST-3039 | `sdtest_3039_unlimited_attempts_reattach_after_a_long_outage` | SDUC-574 | Green | Real binary behind a `Connector` that can be away (stands for another host). With `UNLIMITED_ATTEMPTS` a session survives 40 failed connects in a row, stays "reconnecting", and reattaches to the same pane, which answers; with a budget of 3 the same outage ends the session as unavailable. A pane closed after its session gave up is ended at the supervisor's next contact. |
| SDTEST-3040 | `sdtest_3040_a_pane_being_ended_keeps_the_connection_until_it_is_gone` | SDUC-574 | Green | Real binary. A pane whose program ignores `SIGHUP` is closed as the last session; the idle callback closes the connection, as a remote host does. The supervisor is not idle while the pane is being ended, the callback runs only after the pane was killed and removed (about 1 s later), and never while the pane is still listed. Not run against the code before the fix. |
| SDTEST-3041 | `sdtest_3041_a_pane_closed_during_an_outage_is_ended_at_the_next_contact` | SDUC-574 | Green | Real binary, connector away. Two panes are closed while unreachable (one whose session waits, one whose session had given up), a third is only detached. Nothing reaches the daemon then; once it is back, `list_panes` leaves the closed panes out even before they are gone, they are ended, and the detached pane is still offered. |
| SDTEST-3053 | `sdtest_3053_panes_to_end_are_carried_to_the_next_run` | SDUC-574 | Green | Real binary, two supervisors standing for two runs of the app. A tab is closed while the daemon is out of reach: `doomed_panes` names its pane and the pane still runs. A new supervisor given that list (`doom`) before it reaches the daemon does not offer the pane, ends it at its first contact and then has nothing left to end; a pane that was only detached is still offered. |
| SDTEST-3054 | `sdtest_3054_a_pane_ended_on_request_is_removed_when_it_exits` | SDUC-574 | Green | A client sends `Kill` (hangup) for two panes and disconnects at once. The pane that dies is gone from the list within moments, with no second `Kill`; the one that ignores the hangup keeps running and stays listed, and goes the same way after `SIGKILL`. A pane that exited by itself is still listed, dead, with its exit code. |
| SDTEST-3007 | `sdtest_3007_supervised_session_reports_a_daemon_that_cannot_start` | SDUC-574 | Green | A relative daemon path: phase `Unavailable`, notice on screen, no pane id, no run directory created; input and `terminate` on that session are harmless; `list_panes(false)` reports no daemon. Without any binary the reason given by the app is shown and nothing is created either. |

## 5. `supervisor.rs`: reconnect rules (unit tests)

| ID | Location | SDUC | Status | Notes |
|---|---|---|---|---|
| SDTEST-3004 | `supervisor.rs::sdtest_3004_backoff_doubles_up_to_the_cap` | SDUC-574 | Green | Delays 100, 200, 400, 800 ms then the 1 s cap; no overflow for huge attempt numbers or durations. |
| SDTEST-3005 | `supervisor.rs::sdtest_3005_reconnect_state_machine` | SDUC-574 | Green | `Reconnect`, no I/O: a new pane that cannot be created is unavailable at once; a lost connection retries with growing delays and a reattach that showed the pane resets them; attempts without the pane's screen are bounded, except with `UNLIMITED_ATTEMPTS` (200 failed connects still retry, at the capped delay; a pane that was never created still fails at once); no daemon at all, or `NoSuchPane`, ends the session; the exit code survives a later loss or removal; "connecting" until first shown, "reconnecting" after. |

## SSH transport

The daemon over SSH is tested inside `shelldeck-ssh`, where the in-memory
russh fixtures live: SDTEST-2985 and SDTEST-2987..2991 in
[`tests-ssh.md`](./tests-ssh.md), section "Session daemon transport", and
the remote binary (probe, upload, checksum), the fallback rules and the
host object of the app in SDTEST-3015..3020, SDTEST-3024, SDTEST-3044,
SDTEST-3046 and SDTEST-3048, section
"Session daemon remote binary and host object". Those also exercise
`supervisor::Connector`, `Supervisor::with_connector`, `set_on_idle` and
`LocalSessions::from_client` of this crate. The per-host consent is
SDTEST-3014 and SDTEST-3043 ([`tests-core.md`](./tests-core.md)); the UI
parts are SDTEST-3021..3023, SDTEST-3042 and SDTEST-3043
([`tests-ui-and-app.md`](./tests-ui-and-app.md)).

## Build marker

| ID | Location | SDUC | Status | Notes |
|---|---|---|---|---|
| SDTEST-3047 | `tests/daemon.rs::sdtest_3047_the_binary_carries_the_build_marker_of_its_release` | SDUC-574 | Green | Real binary: the file contains `BUILD_MARKER` of its release and no marker of another version, and `--version` prints `shelldeck-sessiond <version>`. The marker is what `shelldeck-ssh` checks before uploading a bundled build (SDTEST-3044). |

## Packaging

The updater's handling of the daemon files is SDTEST-3033 and the
packaging scripts are SDTEST-3034..3036, both in
[`tests-ui-and-app.md`](./tests-ui-and-app.md). The saved layout under a
daemon of another version is SDTEST-3031 ([`tests-core.md`](./tests-core.md)).

## Retired tests

*(none yet)*
