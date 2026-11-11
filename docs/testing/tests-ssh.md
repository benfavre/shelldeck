# SDTEST inventory — `shelldeck-ssh`

## Managed tmux clients

| ID | Location | SDUC | Status | Notes |
|---|---|---|---|---|
| SDTEST-1951 | `tmux.rs::tmux_handshake_accepts_fragmented_headers_and_preserves_terminal_output` | SDUC-500 | Green | Fragmented private startup headers retain all following TUI bytes and resolve the exact client PTY. |
| SDTEST-1952 | `tmux.rs::tmux_handshake_rejects_invalid_identity_and_unbounded_preludes` | SDUC-500 | Green | Rejects invalid device identities and excessive pre-header output. |
| SDTEST-1953 | `tmux.rs::tmux_controls_use_the_attached_client_and_propagate_failures_without_fallback` | SDUC-500 | Green | Fake executor observes one scoped control command, current-directory inheritance and no keyboard injection; a detached-client error propagates without another attempt. |
| SDTEST-1955 | `session.rs::exec_retains_success_and_failure_status_after_output_eof` | SDUC-045, SDUC-500 | Green | Real in-memory SSH server sends output EOF before exit status and channel close; both zero and nonzero status retain stdout/stderr and report success correctly. |

> Rules for this file live in [`.agents/testing.md`](../../.agents/testing.md).
> Use case IDs (`SDUC-…`) resolve in [`USE_CASES.md`](./USE_CASES.md).

**Big picture.** Parsing, known-hosts, the core session exchanges, the jump
transport and every tunnel direction — local, SOCKS and reverse — now have
direct, protocol-level coverage. No **P0** line is left open here. What remains
uncovered is the dormant `ConnectionPool` (tracked as a technical debt decision,
not a runtime risk) and a set of P1/P2 boundaries listed below.

Strategy: for anything that spans a real network, we introduce controlled
harnesses rather than reaching for a live SSH server:

- **In-memory `russh` server** for `session.rs` — the real client and server
  protocol run over `tokio::io::duplex`, so PTY, exec, resize and EOF are
  asserted without a socket or user `known_hosts` access.
- **`std::net::TcpListener`** + a canned SSH banner for
  `known_hosts.rs` scenarios where we need real socket bytes.

If a test genuinely needs a real SSH server, it is an
`SHELLDECK_LIVE_SSH=1`-gated integration test — never in CI.

---

## 1. `client.rs` — `parse_jump_spec`

| ID | Location | SDUC | Status | Notes |
|---|---|---|---|---|
| SDTEST-500 | `client.rs::test_parse_jump_spec_host_only` | SDUC-041 | Green | |
| SDTEST-501 | `client.rs::test_parse_jump_spec_user_at_host` | SDUC-041 | Green | |
| SDTEST-502 | `client.rs::test_parse_jump_spec_user_at_host_port` | SDUC-041 | Green | |
| SDTEST-503 | `client.rs::test_parse_jump_spec_host_port` | SDUC-041 | Green | |
| SDTEST-504 | `client.rs::test_parse_jump_spec_ssh_uri` | SDUC-041 | Green | |
| SDTEST-505 | `client.rs::test_parse_jump_spec_whitespace_trimmed` | SDUC-041 | Green | |
| SDTEST-506 | `client.rs::test_parse_jump_spec_empty_hostname_fails` | SDUC-041 | Green | |
| SDTEST-507 | `client.rs::test_parse_jump_spec_identity_file_is_none` | SDUC-041 | Green | |
| SDTEST-508 | *to write* — parse_jump_spec rejects invalid ports (e.g. `host:0`, `host:99999`) | SDUC-041 | **Red / P1** | Boundary. |
| SDTEST-509 | *to write* — parse_jump_spec rejects `user@:22` (empty host after user) | SDUC-041 | **Red / P2** | |
| SDTEST-1583 | `client.rs::default_key_candidates_are_under_home_ssh_in_probe_order` + `client.rs::default_key_candidates_empty_without_home_never_root_level` | SDUC-456 | Green | 2 tests, added 2026-08-06. Pure `default_key_candidates(Option<PathBuf>)`: `~/.ssh/{id_ed25519,id_rsa,id_ecdsa}` built with `PathBuf` joins in probe order; no resolvable home ⇒ empty list, never fabricated root-level `/.ssh/*` probes. |

---

## 2. `session.rs` — `SshSession`

Existing: **4 tests** (including the channel-end classification proof).
ProxyJump lives in `client.rs`; SDTEST-528 and SDTEST-530 are listed here because
they pin the transport `session.rs` composes with `new_with_jump`.

| ID | Location | SDUC | Status | Notes |
|---|---|---|---|---|
| SDTEST-520 | `session.rs::shell_requests_pty_dimensions_and_propagates_resize` | SDUC-044 | Green | In-memory SSH handshake; asserts `xterm-256color`, columns, rows and shell request. |
| SDTEST-521 | `session.rs::exec_collects_stdout_stderr_and_exit_status` | SDUC-045 | Green | Real protocol messages over an in-memory duplex stream: `Data`, `ExtendedData(1)`, `ExitStatus`, EOF, channel close. |
| SDTEST-522 | *to write* — exec success() bit matches exit code | SDUC-045 | **Red / P1** | |
| SDTEST-523 | *to write* — exec_streaming yields chunks without buffering the whole output | SDUC-046 | **Red / P1** | Assert the receiver observes chunks *before* the exit signal. |
| SDTEST-524 | `session.rs::cancellable_exec_sends_channel_eof_and_returns_no_exit_status` | SDUC-047 | Green | A held remote command receives channel EOF after cancellation and returns without an exit status. |
| SDTEST-525 | `session.rs::shell_requests_pty_dimensions_and_propagates_resize` | SDUC-044 | Green | Asserts the post-open window-change dimensions on the server side. |
| SDTEST-527 | *to write* — disconnect() drains the event channel cleanly | SDUC-044, SDUC-054 | **Red / P1** | No stray events after `disconnect`. |
| SDTEST-528 | `client.rs::jump_channel_targets_the_inner_host_and_carries_its_session` | SDUC-053 | Green | Two in-memory `russh` servers: the bastion records the `direct-tcpip` request and runs the target server *inside* that channel. Asserts the request names the inner host and port, then execs over the composed `new_with_jump` session and reads a marker only the inner server writes. |
| SDTEST-530 | `client.rs::proxy_jump_none_or_blank_means_direct_and_a_chain_uses_its_first_hop` | SDUC-053 | Green | Pure `first_jump_hop`: `none` (any case) and a blank value fall back to a direct connection; a comma-separated chain uses its **first** hop, never the last. |
| SDTEST-529 | *to write* — ExecResult::stdout_string / stderr_string handle non-utf8 without panic | SDUC-045 | **Red / P1** | Lossy conversion; assert it doesn't panic on invalid utf-8 bytes. |
| SDTEST-1413 | `session.rs::protocol_terminators_are_clean_but_unmarked_channel_loss_is_unexpected` | SDUC-044, SDUC-439 | Green | EOF, channel close, and an exit status followed by stream end classify as clean; disappearance without a protocol terminator classifies as unexpected transport loss. |
| SDTEST-1786 | `session.rs::workspace_helper_uses_only_fixed_subsystem_and_raw_bounded_control_pty` | SDUC-491 | Green | The in-memory SSH server observes a PTY with raw control modes followed by the exact `shelldeck-workspace-v1` subsystem name, and no shell or arbitrary exec request. |

## 2a. `workspace_helper.rs` — retained remote workspace authority

| ID | Location | SDUC | Status | Notes |
|---|---|---|---|---|
| SDTEST-1787 | `workspace_helper.rs::protocol_round_trips_exact_coordinates_and_receipt` + protocol rejection tests | SDUC-491 | Green | The 8-KiB framed codec retains exact operation/workspace coordinates and opaque receipt bytes, and refuses unknown, oversize, trailing, invalid-code, or diagnostic-channel input. |
| SDTEST-1788 | `workspace_helper::remote::retained_descriptor_receipt_revalidates_clean_exact_lineage` | SDUC-491 | Green | Revalidated 2026-09-30 in an isolated child test process because successful helper exchanges change process cwd. A real clean Git repository is opened beneath the configured root, bound to directory identity/HEAD/branch, and a later untracked file makes resume fail closed. |
| SDTEST-1789 | `workspace_helper::remote::root_walk_refuses_symlink_components_and_outside_paths` | SDUC-491 | Green | Component-by-component `openat(O_NOFOLLOW)` refuses a symlink and a path outside every administrator-fixed root. | Native helper exchanges now run in isolated child test processes (2026-09-30) to prevent cwd races with SDTEST-1788.
| SDTEST-1790 | `workspace_helper::remote::exchange_refuses_wrong_receipt_without_starting_shell` | SDUC-491 | Green | A second frame with the wrong opaque token receives only the bounded stale-receipt code; the exchange never reaches shell resume. |

---

## 3. `pool.rs` — dormant `ConnectionPool`

Existing: **0 tests.**

Audit 2026-08-18: this exported type has no production caller. These tests are
deferred until the architecture decides whether to integrate the pool or remove
it; adding a connector abstraction solely to test dormant code would not reduce
current runtime risk. If integration is chosen, restore the appropriate P0
priorities before wiring any caller.

| ID | Location | SDUC | Status | Notes |
|---|---|---|---|---|
| SDTEST-540 | *deferred* — connect returns a UUID and marks connected | SDUC-048 | Deferred | Requires an integration/removal decision first. |
| SDTEST-541 | *deferred* — repeated connect for same Connection follows the chosen sharing policy | SDUC-048 | Deferred | The old reuse claim does not match the implementation or current dedicated-session topology. |
| SDTEST-542 | *deferred* — disconnect closes the session and clears connected_ids | SDUC-048 | Deferred | |
| SDTEST-543 | *deferred* — disconnect_all is idempotent | SDUC-048 | Deferred | |
| SDTEST-544 | *deferred* — with_session / with_session_mut do not deadlock under contention | SDUC-048 | Deferred | Relevant only if the pool becomes a shared runtime boundary. |
| SDTEST-545 | *deferred* — take_session / return_session round-trip preserves the session | SDUC-048 | Deferred | |
| SDTEST-546 | *deferred* — is_connected(uuid) returns false after remote disconnect | SDUC-048, SDUC-054 | Deferred | Requires the event stream and production ownership policy to be defined. |

---

## 4. `tunnel.rs` — port forwards

Existing: **4 protocol/lifecycle tests.**

| ID | Location | SDUC | Status | Notes |
|---|---|---|---|---|
| SDTEST-560 | `shelldeck-core::port_forward::zero_is_rejected` + `all_non_zero_ports_are_accepted` | SDUC-049 | Green | Already covered by SDTEST-030; `u16` makes overflow unrepresentable and 0 is rejected. |
| SDTEST-561 | `tunnel.rs::port_availability_and_prebound_start_failures_are_reported` | SDUC-049 | Green | Reserves a real loopback port and verifies a released ephemeral port. |
| SDTEST-562 | `tunnel.rs::local_forward_echoes_tracks_bytes_and_drains_on_stop` | SDUC-049 | Green | Real `russh` direct-tcpip over an in-memory transport; loopback listener forwards bytes both ways. |
| SDTEST-563 | `tunnel.rs::port_availability_and_prebound_start_failures_are_reported` | SDUC-049 | Green | Local and SOCKS pre-bound ports synchronously return `SshError::PortInUse` without registering a tunnel. |
| SDTEST-564 | `tunnel.rs::local_forward_echoes_tracks_bytes_and_drains_on_stop` | SDUC-052 | Green | Caught detached copy tasks: stop now aborts and joins accepted connections before publishing `Stopped`. |
| SDTEST-565 | `tunnel.rs::remote_forward_requests_the_port_and_routes_connections_to_the_local_target` | SDUC-050, SDUC-052 | Green | Real reverse forward end to end: the in-memory server records the configured `tcpip_forward("127.0.0.1", 8443)`, then plays the remote side by opening a forwarded-tcpip channel back. Bytes reach a loopback echo target and return; counters are directional; stopping the tunnel closes the already-routed connection and sends `cancel_tcpip_forward` for the exact bind while the SSH session remains alive. |
| SDTEST-566 | `tunnel.rs::socks5_connect_echoes_and_rejects_bind_and_udp_associate` | SDUC-051 | Green | Raw SOCKS5 no-auth + domain CONNECT reaches direct-tcpip and echoes; BIND/UDP-associate return command-not-supported without opening a channel. |
| SDTEST-567 | `tunnel.rs::stop_all_closes_every_listener_and_active_connection` | SDUC-052 | Green | Two active listeners and connections are drained; active count reaches zero. |
| SDTEST-568 | local/SOCKS/stop-all tunnel tests | SDUC-052 | Green | `cleanup()` removes stopped handles after task drain. |
| SDTEST-569 | local/SOCKS tunnel tests | SDUC-049 | Green | Counters equal the tunneled payload in both directions; SOCKS negotiation bytes are excluded. |

---

## 5. `known_hosts.rs`

Existing: **0 tests.**

| ID | Location | SDUC | Status | Notes |
|---|---|---|---|---|
| SDTEST-580 | `known_hosts.rs::match_on_plain_hostname_entry` | SDUC-043 | Green | Added 2026-07-09. `check_known_host_in` extracted as pure fn — tests avoid `$HOME` mutation entirely (parallel-safe). |
| SDTEST-581 | `known_hosts.rs::mismatch_when_host_present_but_key_differs` + `mismatch_when_key_type_differs` | SDUC-043 | Green | 2 tests, added 2026-07-09. **Security-critical** MITM sensor — host present but key differs must return Mismatch (never Match, never NotFound). |
| SDTEST-582 | `known_hosts.rs::not_found_for_unknown_host` + `empty_known_hosts_returns_not_found` | SDUC-043 | Green | 2 tests. Empty/missing file ⇒ NotFound (TOFU path). |
| SDTEST-583 | `known_hosts.rs::hashed_entries_are_skipped` | SDUC-043 | Green | Added 2026-07-09. **Contract change vs original inventory** — the parser deliberately does NOT decode hashed hostnames (impl comment: HMAC-SHA1 out of scope). Test pins the current policy: a hashed entry can never accidentally Match against unhashed key material (silent trust break avoided). Full hash-aware parsing would be a future feature. |
| SDTEST-584 | `known_hosts.rs::empty_known_hosts_returns_not_found` (same as SDTEST-582) | SDUC-043 | Green | Subsumed. `ReadError` variant does not exist in the enum today — the impl reads the file with a `?`-like map to NotFound on I/O error, so a permissions failure surfaces the same way as a missing file. |
| SDTEST-585 | `known_hosts.rs::add_known_host_to_appends_never_overwrites` + `add_known_host_to_creates_parent_directory` + `build_line_uses_bare_hostname_for_port_22` + `build_line_brackets_hostname_for_non_default_port` | SDUC-043 | Green | 4 tests. Extracted `add_known_host_to(path, ...)` + `build_known_host_line(...)` as pure fns so append-vs-truncate semantics are testable without `$HOME`. Load-bearing "trust never silently vanishes" property: two consecutive appends preserve both prior + new entries; parent `.ssh` dir auto-created on first-run. |
| SDTEST-586 | *to write* — add_known_host writes atomically | SDUC-043, SDUC-091 | **Red / P1** | Deferred — append semantics + no truncation on partial write is verified by SDTEST-585; full atomic rename-into-place is a nice-to-have for power-loss safety but not blocking today. |
| SDTEST-587bonus | `known_hosts.rs::multi_host_alias_line_matches_each_alias` + `non_default_port_uses_bracketed_pattern` + `comments_and_blank_lines_are_ignored` + `ragged_lines_do_not_panic_or_false_match` | SDUC-043 | Green | 4 bonus tests: comma-alias matching, bracketed non-22 pattern isolation (port 22 lookup on a `[host]:2222` file returns NotFound, not Match), tolerance for comments/blank/ragged lines (never panics, never false Match). |
| SDTEST-1582 | `known_hosts.rs::known_hosts_path_is_built_under_resolved_home` + `known_hosts.rs::known_hosts_path_is_none_without_home_never_fabricated` | SDUC-043 | Green | 2 tests, added 2026-08-06. Pure `known_hosts_path_in(Option<PathBuf>)`: the path is built under the resolved cross-platform home; no home ⇒ `None`, so `check_known_host` degrades to `NotFound` and `add_known_host` skips the write with one warning per process — instead of silently targeting `/root/.ssh/known_hosts`. |

---

## 6. `handler.rs` — event dispatch

Existing: **0 tests.**

| ID | Location | SDUC | Status | Notes |
|---|---|---|---|---|
| SDTEST-600 | *to write* — ClientHandler emits SshEvent::Connected on channel_open_confirmation | SDUC-054 | **Red / P1** | |
| SDTEST-601 | *to write* — ClientHandler emits SshEvent::Disconnected on channel_close | SDUC-054 | **Red / P1** | |
| SDTEST-602 | `tunnel.rs::remote_forward_requests_the_port_and_routes_connections_to_the_local_target` | SDUC-050, SDUC-054 | Green | Covered by SDTEST-565: the event crosses the production `ClientHandler` into `forwarded_tcpip_rx` and out to the tunnel task — a dropped event would leave the echo unread. |

---

## 7. Live smoke (`#[ignore]`)

| ID | Location | SDUC | Status | Notes |
|---|---|---|---|---|
| SDTEST-620 | *to write* — `live_connect_and_exec` against `sshd` in a container | SDUC-045, SDUC-054 | **Red / P2** | Gated by `SHELLDECK_LIVE_SSH=1`. Optional; the mocks + fake transport should catch most regressions before this. |
| SDTEST-1809 | `session.rs::fixed_workspace_subsystem_prepares_releases_and_resumes_exact_remote_repository` | SDUC-491 | Yellow / live | Against an operator-installed `shelldeck-workspace-v1` subsystem, the production client prepares and releases once without a shell, then prepares/resumes a second exact receipt and observes the fixed shell running at the retained clean repository CWD. Passing in the documented opt-in live harness, but excluded from the default suite; gated by `SHELLDECK_LIVE_SSH=1` plus explicit host/key/workspace coordinates. |

| SDTEST-1942 | `tunnel.rs::local_forward_uses_configured_bind_address_and_rejects_occupied_address` | SDUC-049, SDUC-052 | Green | IPv6 loopback bind rejects occupation, accepts real SSH echo only on the configured address, and closes on stop. |

---

## Retired tests

| ID | Previous contract | Status | Reason |
|---|---|---|---|
| SDTEST-526 | EOF makes `SshChannel::read` return `None` | Retired 2026-07-24 | The reader now returns explicit `CleanEnd` versus `ConnectionLost`; SDTEST-1413 covers the stronger observable contract. |


## Agent remote file editor — 2026-10-15

| ID | Location | SDUC | Status | Notes |
|---|---|---|---|---|
| SDTEST-2510 | `agent_files.rs` | SDUC-475, SDUC-551 | Green | Remote root/path/text are JSON stdin data, never shell/argv fragments. |
| SDTEST-2511 | `agent_files.rs` | SDUC-475, SDUC-551 | Green | Invalid paths, revisions and oversized documents refuse before execution. |
| SDTEST-2512 | `agent_files.rs` | SDUC-475, SDUC-551 | Green | Conflicts do not retry; missing Python3 yields explicit unsupported-host error. |
| SDTEST-2513 | `agent_files.rs` | SDUC-475, SDUC-551 | Green | UTF-8/control/CRLF bytes remain exact; malformed/oversized documents refuse. |
| SDTEST-2514 | `agent_files.rs` | SDUC-475, SDUC-551 | Green | Listing canonical root, child paths, count bounds and truncation are validated. |
| SDTEST-2515 | `agent_files.rs` | SDUC-475, SDUC-551 | Green | Invalid protocol, missing exit status and oversized diagnostics fail closed. |
| SDTEST-2516 | `session.rs` | SDUC-475, SDUC-551 | Green | In-memory SSH early exit keeps the unsupported-host status after EOF. |
| SDTEST-2517 | `session.rs` | SDUC-475, SDUC-551 | Green | In-memory transport caps stdin, stdout and stderr. |
| SDTEST-2518 | `session.rs` | SDUC-475, SDUC-551 | Green | Cancelling a bounded operation sends channel EOF/Close. |

## SSH crypto dependency maintenance — 2026-10-28

| ID | Location | SDUC | Status | Notes |
|---|---|---|---|---|
| SDTEST-2710 | `client.rs::sdtest_2710_upstream_key_formats_authenticate_only_after_signature_and_user_match` | SDUC-456 | Green | Generated Ed25519, RSA and P-256 keys round-trip encrypted OpenSSH formats; wrong passphrase/user refuse, then production key authentication succeeds only after server signature verification over in-memory SSH. No real host, known-hosts file or keychain access. |

| SDTEST-2714 | `directory_browser::tests::sdtest_2714_directory_navigation_preserves_exact_paths_and_one_session` | SDUC-488 | Green | HOME and absolute paths remain exact data across one retained authenticated session; invalid paths are refused before listing. Passed locally. |
| SDTEST-2715 | `directory_browser::tests::sdtest_2715_close_cancels_connection_and_bounds_pending_admission` | SDUC-488 | Green | Pending queue is bounded; closing cancels connection and the worker ends without retaining its sender. Passed locally. |
| SDTEST-2716 | `directory_browser::tests::sdtest_2716_close_cancels_listing_disconnects_and_finishes_worker` | SDUC-488 | Green | Closing interrupts an outstanding listing, disconnects and drops the session before completion. Passed locally. |

## Standard SSH existing folders — 2026-10-29

| ID | Location | SDUC | Status | Notes |
|---|---|---|---|---|
| SDTEST-2729 | `session::folder_shell::tests::folder_handshake_preserves_output_and_rejects_missing_or_wrong_nonce` | SDUC-488, SDUC-491 | Green | Exact random-nonce readiness markers tolerate every split boundary, preserve adjacent terminal bytes, and reject wrong-phase, wrong-token and oversized output; invalid paths refuse. |
| SDTEST-2730 | `session::folder_shell::tests::folder_script_holds_exact_directory_until_resume_and_restores_modes` | SDUC-488, SDUC-491 | Green | Unix-only disposable fixture executes the production POSIX script with controlled shell/stty: quotes, newline and trailing spaces remain path data; a renamed folder remains cwd; no shell before resume, on wrong token, EOF or missing folder. Saved-mode restoration is checked through fixture stty; native QA covers a real PTY. No real SSH host or installed helper. |

| SDTEST-2740 | `session::folder_shell::tests::folder_release_attempts_close_after_eof_failure_or_timeout` | SDUC-491 | Green | Bounded EOF failure/timeout still attempts Close, and a blocked Close has its own deadline. Release retains channel Drop custody until cleanup succeeds. Passed in the complete SSH library suite: 67 passed, one intentional live test ignored (2026-10-29). |

## Script transport lifecycle audit — 2026-10-30

| ID | Location | SDUC | Status | Notes |
|---|---|---|---|---|
| SDTEST-2760 | `session::in_memory_ssh_tests::sdtest_2760_cancellable_exec_retains_status_after_eof` | SDUC-046, SDUC-047 | Green | In-memory SSH emits output EOF before success/failure exit status; both streaming entry points retain the actual result. Passed in the complete SSH library suite: 69 passed, one intentional live test ignored. |
| SDTEST-2761 | `session::in_memory_ssh_tests::sdtest_2761_cancelled_exec_never_starts_while_waiting_for_handle` | SDUC-047 | Green | Cancellation while the shared handle is occupied returns without admitting a remote command. Passed in the complete SSH library suite: 69 passed, one intentional live test ignored. |

## Scoped remote browser bridge — 2026-10-31

| ID | SDUC | Scenario | Location | Status |
|---|---|---|---|---|
| SDTEST-2862 | SDUC-557 | Real in-memory SSH reverse forwarding routes only the allocated loopback listener to the scoped browser and revokes it without stopping the provider | `browser_bridge` | Green |
| SDTEST-2863 | SDUC-557 | Wrong run/server, invalid endpoints, denied forwarding and setup cancellation refuse before provider execution; secrets never enter command/debug output | `browser_bridge` | Green |
| SDTEST-2864 | SDUC-557 | Bounded SSH stdin is sent concurrently with streamed output; Stop releases input/exec/forwarding and EOF does not lose a later exit status | `session`, `browser_bridge` | Green |
| SDTEST-2865 | SDUC-557 | Production Python launcher and proxy drive fake Claude/Codex MCP conversations, keep private configuration scoped and remove it on completion/failure/signals | `scripts/test-remote-browser-bridge.py` | Green |
| SDTEST-2867 | SDUC-557 | Opt-in packaged CEF plus actual SSH channels and a harmless fake provider exercise browser observation/control and revocation end to end | `browser_bridge` | Yellow / native gate — passed locally with packaged CEF; explicit runtime/display opt-in, Linux/macOS browser CI |

## Session daemon transport: 2026-11-11

`sessiond.rs` (tests in `sessiond_tests.rs`, Unix only). A real russh client
and server over an in-memory duplex, with a dedicated server handler in the
test file (the `session.rs` fixture answers an exec with a fixed result and
cannot relay channel data). The exec request is answered by a relay to a real
daemon on a private temporary run directory, by the real `/bin/sh`, by a fixed
script, or by a refusal. No ephemeral port is used, so these run in parallel.

| ID | SDUC | Scenario | Location | Status |
|---|---|---|---|---|
| SDTEST-2985 | SDUC-574 | Spawn, attach, input and echoed output end to end through an SSH exec channel to a real daemon and PTY; banner noise and a look-alike marker before the nonce marker are skipped; UTF-8 crosses unchanged; dropping the client sends EOF and Close and reports `LocalClose`; a second channel on the same connection uses a fresh nonce and finds the pane and its screen again; the daemon stopping is reported as `RemoteExit` with status 0 | `sessiond::tests::sdtest_2985_daemon_over_ssh_spawns_attaches_echoes_and_survives_the_channel` | Green |
| SDTEST-2987 | SDUC-574 | The real `/bin/sh` runs the real fixed command: a path that does not exist is `BinaryMissing` (127), a file without the execute bit is `BinaryNotExecutable` (126), both `is_unavailable()` for the tmux fallback; a program that exits 1 without the marker is `ProxyExited`, not a fallback | `sessiond::tests::sdtest_2987_missing_or_unusable_remote_binary_is_a_typed_fallback_error` | Green |
| SDTEST-2988 | SDUC-574 | Startup classification: exit status 127 arriving after the output EOF (OpenSSH order) and before it; `SDSD-ERR 1/2/3` on stderr after rc noise map to the three proxy refusals; an unknown code is a plain early exit; 70 KiB without the marker is refused; the channel is released every time | `sessiond::tests::sdtest_2988_startup_failures_are_classified_and_release_the_channel` | Green |
| SDTEST-2989 | SDUC-574 | A daemon answering `UnsupportedVersion` is `VersionMismatch`; stderr after the marker fails closed; a refused exec request is `ExecRefused` (fallback); a version or path setting with shell syntax is `InvalidBinary` and sends no exec request; the default command names the per-user install of the app version | `sessiond::tests::sdtest_2989_version_mismatch_refused_exec_and_bad_setting_are_distinct` | Green |
| SDTEST-2990 | SDUC-574 | A proxy that never answers times out; a timed-out and a cancelled connect both send EOF and Close; losing the SSH connection during startup is `SessionClosed`, not a missing binary | `sessiond::tests::sdtest_2990_a_proxy_that_never_answers_times_out_and_is_released` | Green |
| SDTEST-2991 | SDUC-574 | The real `shelldeck-sessiond` binary behind the real command under a private `HOME`: the proxy starts the daemon, a pane echoes through SSH, a second channel finds the pane | `sessiond::tests::sdtest_2991_real_proxy_binary_starts_the_daemon_and_serves_a_pane_over_ssh` | Yellow / opt-in: `#[ignore]`, needs `SHELLDECK_SESSIOND_BIN=<built binary>`; passed locally on Linux, not in CI |

## Session daemon remote binary and host object: 2026-11-11

`sessiond_install.rs` (unit tests in the file) and `sessiond_host.rs`
(tests in `sessiond_remote_tests.rs`, a child of the transport tests, Unix
only). The fixture of the section above gained a "host" mode: the proxy
command reaches an in-process daemon once the versioned file is installed
and executable under a private temporary `HOME` (else status 127 or 126,
as a shell would), and every other command runs in the real `/bin/sh` with
that `HOME`. Each dial of a `RemoteHost` starts a new in-memory server in
front of the same home and daemon. No real host, no `sshd` (none answers on
localhost here, so no live localhost test exists).

| ID | SDUC | Scenario | Location | Status |
|---|---|---|---|---|
| SDTEST-3015 | SDUC-574 | Decision table: an installed binary is used whatever the recorded answer; an upload only with "allow" and a bundled build for the host's platform; `ConsentPending` (ask), `ConsentDeclined`, `NoArtifact`, `UnsupportedPlatform`, `ProbeFailed`; connect errors map to `Unavailable`, `VersionMismatch`, `ConnectFailed`; fallback backend tmux or plain shell | `sessiond_install.rs::sdtest_3015_decision_table_uploads_only_with_recorded_consent` | Green |
| SDTEST-3016 | SDUC-574 | Probe and install scripts refuse a version, nonce or digest outside their alphabet; the probe line is parsed among rc noise and only with its nonce; `target_triple`; an artifact is found by name in directory order and must be an ELF of the right machine. SDTEST-3044: it must also carry the build marker of the app's release (no other version, no prefix or suffix of it); a stale build in an earlier directory does not hide the right one in a later directory | `sessiond_install.rs::sdtest_3016_scripts_take_only_checked_values_and_artifacts_are_checked` | Green |
| SDTEST-3017 | SDUC-574 | Real `/bin/sh`, private `HOME`: a probe writes nothing and names the platform; an upload installs the exact bytes, mode 0700 in 0700 directories, remote digest equal to the local one, no temporary file, nothing else in the home; a file without the execute bit is reported and replaced; only fixed `sh -c` scripts are sent, no `sudo`; bad version and empty artifact are refused before any exec | `sessiond_remote_tests.rs::sdtest_3017_probe_and_upload_install_a_verified_private_binary` | Green |
| SDTEST-3018 | SDUC-574 | A byte flipped in transit gives `ChecksumMismatch`, the previous install stays, the temporary file is removed; a host without `sha256sum`, `shasum` or `openssl` gives `NoChecksumTool` and installs nothing | `sessiond_remote_tests.rs::sdtest_3018_an_upload_that_fails_its_checksum_installs_nothing` | Green |
| SDTEST-3019 | SDUC-574 | `RemoteHost::prepare`: not asked (fallback, ask, one probe sent, home untouched, working connection handed over), never, a local file that is not an executable or (SDTEST-3044) is the build of another release (both `NoArtifact`, nothing sent beyond the probe, no question), no bundled build, damaged upload, refused proxy exec, unreachable host (`ConnectFailed`); `install` after the user's answer, then `Ready` without any answer | `sessiond_remote_tests.rs::sdtest_3019_without_consent_or_a_usable_binary_the_host_falls_back_untouched` | Green |
| SDTEST-3020 | SDUC-574 | End to end: consented upload, two supervised sessions spawn and attach two panes that echo through one SSH connection and one exec channel; the connection is cut, `ConnectionLost` then `Reconnected`, both reattach on a new dial with their screens; sessions dropped: `Idle`, connection released; a later `prepare` lists both panes, one is reattached with its screen, `terminate` ends only that pane | `sessiond_remote_tests.rs::sdtest_3020_remote_panes_share_one_connection_survive_and_are_offered_back` | Green |
| SDTEST-3024 | SDUC-574 | Same path with the real binary and nothing simulated remotely: uploaded by the real script, started by the real proxy command under a private `HOME`, a pane echoes, survives the release and is reattached. Opt in with `SHELLDECK_SESSIOND_BIN` (a build of at most 64 MiB, for example a stripped debug build, of the workspace version: the upload checks its build marker) | `sessiond_remote_tests.rs::sdtest_3024_real_binary_is_uploaded_started_and_serves_panes_over_ssh` | Yellow / opt-in (`#[ignore]`), green when run on 2026-11-11 |
| SDTEST-3046 | SDUC-574 | A host that cannot execute what is uploaded (stand-in for a `noexec` home: a `chmod` that changes nothing): the first `prepare` with "allow" sends probe and upload, the script answers `noexec` and leaves nothing, the fallback is `Unavailable`; every later `prepare` of the run sends one probe and no upload, whatever the answer, and asks nothing; `install` uploads nothing. A real `noexec` mount was not used | `sessiond_remote_tests.rs::sdtest_3046_a_host_that_cannot_execute_the_upload_is_uploaded_to_once` | Green |
| SDTEST-3048 | SDUC-574 | Closing the last tab of a host whose program ignores `SIGHUP`: `Idle` comes only after the pane was killed and removed, and the next `prepare` lists nothing to offer back. A host away for 20 failed dials (a local session gives up after 12): the tab stays "reconnecting", its thread keeps running, and it reattaches with its screen when the host is back | `sessiond_remote_tests.rs::sdtest_3048_a_closed_remote_pane_is_ended_and_a_long_outage_is_waited_out` | Green |
