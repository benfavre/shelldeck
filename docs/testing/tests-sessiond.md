# SDTEST inventory: `shelldeck-sessiond`

> Rules for this file live in [`.agents/testing.md`](../../.agents/testing.md).
> Use case IDs (`SDUC-…`) resolve in [`USE_CASES.md`](./USE_CASES.md).

**Big picture.** The session daemon prototype (SDUC-574, design in
[`session-daemon.md`](../design/session-daemon.md)). The codec tests are
plain unit tests. The integration tests in `tests/daemon.rs` run a real
daemon, real PTYs and the real client library: the daemon runs in-process
on a private temporary run directory, except for the proxy test, which
starts the real binary (detached) on its own private directory and stops it
with `SIGTERM` at the end. No test touches `~/.shelldeck`, an existing
socket, tmux or any user process. The whole crate runs in about eight
seconds: `cargo test --locked -p shelldeck-sessiond`.

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
| SDTEST-2969 | `protocol.rs::sdtest_2969_spawn_rejects_disallowed_env_and_out_of_range_fields` | SDUC-574 | Green | Hand-built `Spawn` bodies: environment allowlist (`PATH`, `LD_PRELOAD` and others refused), rows, cols, scrollback, label, argv and env limits, the 64 KiB frame cap, invalid UTF-8, bad magic, bad signal, unknown error code. |
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
| SDTEST-3001 | `sdtest_3001_descriptor_exhaustion_refuses_at_once_and_leaks_no_connection_slot` | SDUC-574 | Green | Real binary in the foreground under `ulimit -n 12`: connections are served until descriptors run out; 40 further attempts are each answered within 2 s (at least one with `LimitExceeded`, none left in the backlog); after the held connections close the same number is served again, so no slot leaked. Linux only as written (descriptor count of the daemon). |
| SDTEST-3002 | `sdtest_3002_a_daemon_under_another_host_name_is_found` | SDUC-574 | Green | `paths::live_run_dir`: a daemon in a sibling directory is chosen when the preferred one has no socket; a sibling with an unserved socket is skipped; a daemon in the preferred directory wins; no daemon gives the preferred directory. |
| SDTEST-3003 | `sdtest_3003_a_start_during_the_old_daemons_exit_is_repeated` | SDUC-574 | Green | Socket unlinked while the old (in-process) daemon still holds the lock for 400 ms: `proxy::connect_or_start` repeats the start and reaches a new real daemon within the 3 s limit. |
| SDTEST-2980 | `sdtest_2980_proxy_starts_the_daemon_and_carries_a_full_session_over_stdio` | SDUC-574 | Green | Real binary: `proxy` starts a detached daemon, banner noise and a marker with another nonce are skipped, full spawn, attach, input, output cycle over a pipe pair, stdin EOF ends the proxy and leaves daemon and pane running, the pane child neither ignores nor blocks `SIGHUP`, oversize, late and missing preludes fail, `SIGTERM` leaves no pane process and no socket. |

## 3. To write

| ID | Location | SDUC | Status | Notes |
|---|---|---|---|---|
| SDTEST-2981 | *to write*: input backpressure (a pane whose child does not read blocks only the pasting connection, queue discarded after the stall timeout with `Error(Busy)`) | SDUC-574 | **Red / P1** | The mechanism exists (`InputQueue::push_blocking`), untested. |
| SDTEST-2982 | *to write*: resize ownership follows the last active client with the 1 s hysteresis | SDUC-574 | **Red / P1** | Implemented in `Pane::note_input`, untested. |
| SDTEST-2983 | *to write*: `SIGTERM` to the daemon kills a pane child that ignores `SIGHUP` after the grace period | SDUC-574 | **Red / P2** | Needs the 3 s grace to be injectable in the binary to stay fast. |
| SDTEST-2984 | *to write*: parser panic in one pane marks only that pane dead | SDUC-574 | **Red / P2** | Needs a fault injection point. |

## 4. `tests/attached.rs`: app facade (`attached::LocalSessions`)

Starts the real daemon binary, detached, on a private temporary run
directory and stops it with `SIGTERM` (also when the test fails).
`cargo test --locked -p shelldeck-sessiond --test attached`. The backend
itself (attach, resize, terminate, attach without a size) is covered against
an in-process daemon by SDTEST-2992..2995 and SDTEST-2998 in [`tests-terminal.md`](./tests-terminal.md).

| ID | Location | SDUC | Status | Notes |
|---|---|---|---|---|
| SDTEST-2996 | `sdtest_2996_connect_starts_the_daemon_and_panes_survive_the_client` | SDUC-574 | Green | `connect_existing` on a directory without daemon returns none and creates nothing; a relative daemon path is refused before anything starts; `connect` starts the binary; a pane echoes; every session and the connection are dropped (app quit) and the daemon keeps running; a new `connect_existing` lists the pane, the attached grid shows the earlier screen and `$$` is the same pid; `terminate` removes the pane. |

## SSH transport

The daemon over SSH is tested inside `shelldeck-ssh`, where the in-memory
russh fixtures live: SDTEST-2985 and SDTEST-2987..2991 in
[`tests-ssh.md`](./tests-ssh.md), section "Session daemon transport".

## Retired tests

*(none yet)*
