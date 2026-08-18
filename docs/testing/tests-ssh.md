# SDTEST inventory — `shelldeck-ssh`

> Rules for this file live in [`.agents/testing.md`](../../.agents/testing.md).
> Use case IDs (`SDUC-…`) resolve in [`USE_CASES.md`](./USE_CASES.md).

**Big picture.** The parsing, known-hosts and core session exchanges have
direct coverage. Pool, jump transport and tunnels still need controlled
protocol-level proofs — otherwise one broken change lands as a runtime error,
not a red test.

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

| ID | Location | SDUC | Status | Notes |
|---|---|---|---|---|
| SDTEST-520 | `session.rs::shell_requests_pty_dimensions_and_propagates_resize` | SDUC-044 | Green | In-memory SSH handshake; asserts `xterm-256color`, columns, rows and shell request. |
| SDTEST-521 | `session.rs::exec_collects_stdout_stderr_and_exit_status` | SDUC-045 | Green | Real protocol messages over an in-memory duplex stream: `Data`, `ExtendedData(1)`, `ExitStatus`, EOF. |
| SDTEST-522 | *to write* — exec success() bit matches exit code | SDUC-045 | **Red / P1** | |
| SDTEST-523 | *to write* — exec_streaming yields chunks without buffering the whole output | SDUC-046 | **Red / P1** | Assert the receiver observes chunks *before* the exit signal. |
| SDTEST-524 | `session.rs::cancellable_exec_sends_channel_eof_and_returns_no_exit_status` | SDUC-047 | Green | A held remote command receives channel EOF after cancellation and returns without an exit status. |
| SDTEST-525 | `session.rs::shell_requests_pty_dimensions_and_propagates_resize` | SDUC-044 | Green | Asserts the post-open window-change dimensions on the server side. |
| SDTEST-527 | *to write* — disconnect() drains the event channel cleanly | SDUC-044, SDUC-054 | **Red / P1** | No stray events after `disconnect`. |
| SDTEST-528 | *to write* — new_with_jump wires the jump session as ProxyJump transport | SDUC-053 | **Red / P0** | Fake outer transport that observes the "direct-tcpip" request opened against the inner host. |
| SDTEST-529 | *to write* — ExecResult::stdout_string / stderr_string handle non-utf8 without panic | SDUC-045 | **Red / P1** | Lossy conversion; assert it doesn't panic on invalid utf-8 bytes. |
| SDTEST-1413 | `session.rs::protocol_terminators_are_clean_but_unmarked_channel_loss_is_unexpected` | SDUC-044, SDUC-439 | Green | EOF, channel close, and an exit status followed by stream end classify as clean; disappearance without a protocol terminator classifies as unexpected transport loss. |

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
| SDTEST-561 | `tunnel.rs::port_availability_and_prebound_start_failure_are_reported` | SDUC-049 | Green | Reserves a real loopback port and verifies a released ephemeral port. |
| SDTEST-562 | `tunnel.rs::local_forward_echoes_tracks_bytes_and_drains_on_stop` | SDUC-049 | Green | Real `russh` direct-tcpip over an in-memory transport; loopback listener forwards bytes both ways. |
| SDTEST-563 | `tunnel.rs::port_availability_and_prebound_start_failure_are_reported` | SDUC-049 | Green | Pre-bound port returns `SshError::PortInUse` without registering a tunnel. |
| SDTEST-564 | `tunnel.rs::local_forward_echoes_tracks_bytes_and_drains_on_stop` | SDUC-052 | Green | Caught detached copy tasks: stop now aborts and joins accepted connections before publishing `Stopped`. |
| SDTEST-565 | *to write* — start_remote_forward routes ForwardedTcpIp events to local target | SDUC-050 | **Red / P1** | Fake session emits synthetic `ForwardedTcpIpEvent`. |
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
| SDTEST-602 | *to write* — server_channel_open_forwarded_tcpip forwards into forwarded_tcpip_rx | SDUC-050, SDUC-054 | **Red / P1** | |

---

## 7. Live smoke (`#[ignore]`)

| ID | Location | SDUC | Status | Notes |
|---|---|---|---|---|
| SDTEST-620 | *to write* — `live_connect_and_exec` against `sshd` in a container | SDUC-045, SDUC-054 | **Red / P2** | Gated by `SHELLDECK_LIVE_SSH=1`. Optional; the mocks + fake transport should catch most regressions before this. |

---

## Retired tests

| ID | Previous contract | Status | Reason |
|---|---|---|---|
| SDTEST-526 | EOF makes `SshChannel::read` return `None` | Retired 2026-07-24 | The reader now returns explicit `CleanEnd` versus `ConnectionLost`; SDTEST-1413 covers the stronger observable contract. |
