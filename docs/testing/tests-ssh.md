# SDTEST inventory — `shelldeck-ssh`

> Rules for this file live in [`.agents/testing.md`](../../.agents/testing.md).
> Use case IDs (`SDUC-…`) resolve in [`USE_CASES.md`](./USE_CASES.md).

**Big picture.** SSH is the weakest-tested crate today: only the
`parse_jump_spec` helper has coverage. The rest (`session`, `pool`,
`tunnel`, `known_hosts`, `handler`) is exercised only through the UI —
one broken change lands as a runtime error, not a red test.

Strategy: for anything that spans a real network, we introduce two
kinds of fakes rather than reaching for a real SSH server:

- **`FakeTransport`** for `session.rs` — a `russh::client::Handler`
  double that lets us assert what the session pushed on the wire
  (open-shell request, exec request, resize, EOF) without a network.
- **`FakePool`** — mostly not needed; `ConnectionPool` is a
  boundary around `SshSession` and can be tested with the same
  transport fake.
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

---

## 2. `session.rs` — `SshSession`

Existing: **0 tests.**

| ID | Location | SDUC | Status | Notes |
|---|---|---|---|---|
| SDTEST-520 | *to write* — open_shell requests the initial window size and returns a channel | SDUC-044 | **Red / P0** | Use `FakeTransport` that asserts the pty-request and window dimensions. |
| SDTEST-521 | *to write* — exec captures stdout, stderr, and exit code | SDUC-045 | **Red / P0** | Fake transport feeds canned `Data` + `ExtendedData(1)` + `ExitStatus`. |
| SDTEST-522 | *to write* — exec success() bit matches exit code | SDUC-045 | **Red / P1** | |
| SDTEST-523 | *to write* — exec_streaming yields chunks without buffering the whole output | SDUC-046 | **Red / P1** | Assert the receiver observes chunks *before* the exit signal. |
| SDTEST-524 | *to write* — exec_cancellable interrupts the future when the token fires | SDUC-047 | **Red / P0** | Regression class: leaked long-running remote work. |
| SDTEST-525 | *to write* — resize propagates to the channel's window request | SDUC-044 | **Red / P1** | |
| SDTEST-526 | *to write* — EOF triggers `SshChannel::read` → None | SDUC-044 | **Red / P2** | |
| SDTEST-527 | *to write* — disconnect() drains the event channel cleanly | SDUC-044, SDUC-054 | **Red / P1** | No stray events after `disconnect`. |
| SDTEST-528 | *to write* — new_with_jump wires the jump session as ProxyJump transport | SDUC-053 | **Red / P0** | Fake outer transport that observes the "direct-tcpip" request opened against the inner host. |
| SDTEST-529 | *to write* — ExecResult::stdout_string / stderr_string handle non-utf8 without panic | SDUC-045 | **Red / P1** | Lossy conversion; assert it doesn't panic on invalid utf-8 bytes. |

---

## 3. `pool.rs` — `ConnectionPool`

Existing: **0 tests.**

| ID | Location | SDUC | Status | Notes |
|---|---|---|---|---|
| SDTEST-540 | *to write* — connect returns a UUID and marks connected | SDUC-048 | **Red / P0** | Use a fake connector trait to avoid a real handshake. |
| SDTEST-541 | *to write* — repeated connect for same Connection reuses session | SDUC-048 | **Red / P0** | |
| SDTEST-542 | *to write* — disconnect closes the session and clears connected_ids | SDUC-048 | **Red / P0** | |
| SDTEST-543 | *to write* — disconnect_all is idempotent | SDUC-048 | **Red / P1** | |
| SDTEST-544 | *to write* — with_session / with_session_mut do not deadlock under contention | SDUC-048 | **Red / P0** | Two threads holding closures over the same session ID. |
| SDTEST-545 | *to write* — take_session / return_session round-trip preserves the session | SDUC-048 | **Red / P1** | |
| SDTEST-546 | *to write* — is_connected(uuid) returns false after remote disconnect | SDUC-048, SDUC-054 | **Red / P1** | Requires the event stream to bubble a Disconnected event. |

---

## 4. `tunnel.rs` — port forwards

Existing: **0 tests.**

| ID | Location | SDUC | Status | Notes |
|---|---|---|---|---|
| SDTEST-560 | *to write* — validate_port(0), validate_port(1..=65535), overflow | SDUC-049 | **Red / P0** | Cross-referenced with SDTEST-030. |
| SDTEST-561 | *to write* — check_port_available true for a free port, false for a taken one | SDUC-049 | **Red / P1** | Bind a `TcpListener` in the test to reserve the port. |
| SDTEST-562 | *to write* — start_local_forward binds and forwards bytes both ways | SDUC-049 | **Red / P0** | Fake session opens a "loopback echo" channel; assert total_bytes counters. |
| SDTEST-563 | *to write* — start_local_forward Errs when local bind fails | SDUC-049 | **Red / P1** | Pre-bind the port in the test. |
| SDTEST-564 | *to write* — stop() drains connections and cleans up | SDUC-052 | **Red / P0** | Regression sensor: leaked tasks on tunnel drop. |
| SDTEST-565 | *to write* — start_remote_forward routes ForwardedTcpIp events to local target | SDUC-050 | **Red / P1** | Fake session emits synthetic `ForwardedTcpIpEvent`. |
| SDTEST-566 | *to write* — start_socks_forward accepts CONNECT/BIND/UDP-associate handshake and rejects invalid | SDUC-051 | **Red / P0** | Feed raw SOCKS5 bytes into the listener from the test. |
| SDTEST-567 | *to write* — stop_all closes every active tunnel | SDUC-052 | **Red / P1** | |
| SDTEST-568 | *to write* — cleanup() removes stopped tunnels from the list | SDUC-052 | **Red / P2** | |
| SDTEST-569 | *to write* — TunnelHandle::total_bytes accumulates monotonically | SDUC-049 | **Red / P2** | |

---

## 5. `known_hosts.rs`

Existing: **0 tests.**

| ID | Location | SDUC | Status | Notes |
|---|---|---|---|---|
| SDTEST-580 | *to write* — check_known_host returns Match for a plain hostname entry | SDUC-043 | **Red / P0** | Use a TempDir'd known_hosts file. |
| SDTEST-581 | *to write* — check_known_host returns Mismatch when key differs | SDUC-043 | **Red / P0** | Security-critical. |
| SDTEST-582 | *to write* — check_known_host returns NotFound for a fresh host | SDUC-043 | **Red / P0** | |
| SDTEST-583 | *to write* — check_known_host returns Match for a hashed hostname (HMAC-SHA1) | SDUC-043 | **Red / P0** | Real-world OpenSSH clients hash by default. |
| SDTEST-584 | *to write* — check_known_host returns ReadError on file-permissions failure | SDUC-043 | **Red / P1** | |
| SDTEST-585 | *to write* — add_known_host appends to file, never overwrites existing entries | SDUC-043 | **Red / P0** | Regression class: silent trust loss. |
| SDTEST-586 | *to write* — add_known_host writes atomically | SDUC-043, SDUC-091 | **Red / P0** | Same rationale as SDTEST-070. |

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

*(none yet)*
