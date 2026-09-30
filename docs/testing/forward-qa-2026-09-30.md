# Forwarding QA — 2026-09-30

Tested the native Linux app on a private Xvfb display with an isolated XDG
profile, a local SSH server and fictional Manage HTTP data. The baseline binary
contains main at `be4d24b` (v0.9.10); the corrected binary was rebuilt from this
PR. This is app QA on an owned test display, not control of the user's desktop.
`HOME` and the user's SSH configuration were unchanged. The fixture's exact
localhost host key was trusted only for this run and removed afterward.

## Reproductions and corrections

| Workflow | Baseline | Corrected app |
|---|---|---|
| Saved local bind `127.0.0.2:8899` | UI showed the saved address, but only `127.0.0.1:8899` accepted connections. | Only `127.0.0.2:8899` accepted connections; an HTTP request through SSH returned the fixture's issues with status 200. |
| Stop during five-second SSH authentication delay | Start, then Stop before authentication completed; the listener later reopened and the row became active. | The listener stayed closed beyond six seconds and the row stayed inactive. |
| Stop, then immediate retry | Older pending completions had no attempt ownership. | A canceled delayed attempt could not replace a newer successful retry; the newer listener remained active after the older delay elapsed. |
| Reverse-forward cleanup | The worker previously destroyed its runtime after a fixed 100 ms, even when explicit remote cancellation was still waiting. | A server deliberately delaying cancellation by two seconds closed its listener after 2.03 seconds. The runtime stayed alive for cleanup, then disconnected SSH. |
| SOCKS forward | Previously covered in the preceding QA batch. | An HTTP request through the SOCKS proxy returned the issues payload, then Stop closed its listener. |
| Logout during ten-second authentication delay | Pending setup was outside authenticated-runtime cleanup. | Logout returned to welcome and the listener remained closed after the delay elapsed. |

Reproduce the address bug with a local-forward profile using a non-default
loopback address: start it, then attempt TCP connections to both the saved and
default addresses. Reproduce the pending-start bug by delaying public-key
authentication on a test SSH server, clicking Start then Stop before the delay
ends, and checking both the row and socket after authentication would finish.
Repeat with a new Start and with account logout. No production hosts or writes
were used. The reverse-forward fixture delayed its cancellation handler before
closing the remote listener, making the runtime-cleanup check observable.

The correction passes the saved local address into `TunnelManager`, owns setup
cancellation before starting the worker, and tags each attempt so stopped,
replaced or logged-out callbacks cannot publish success, failure or timeout.
Worker shutdown waits up to six seconds for tunnel cleanup (remote cancellation
has an internal five-second bound), then bounds SSH disconnect to two seconds.
The 30-second UI setup timeout also cancels the worker. The full timeout itself
was not exercised through the native UI.

## Evidence

All screenshots contain fictional fixture data on the forwarding/welcome
surfaces. Socket observations are in
[observations.json](screenshots/2026-09-30/forwards/observations.json).

- Before: [saved address](screenshots/2026-09-30/forwards/03-baseline-bind.png),
  [wrong actual bind](screenshots/2026-09-30/forwards/04-baseline-wrong-bind.png),
  [stopped pending setup](screenshots/2026-09-30/forwards/05-baseline-stopped-pending.png),
  [unwanted reactivation](screenshots/2026-09-30/forwards/06-baseline-reactivated.png).
- After: [configured bind and traffic](screenshots/2026-09-30/forwards/08-fixed-bind-traffic.png),
  [stop during setup](screenshots/2026-09-30/forwards/09-fixed-stop-pending.png),
  [still stopped after delay](screenshots/2026-09-30/forwards/10-fixed-stays-stopped.png),
  [new retry retained](screenshots/2026-09-30/forwards/11-fixed-retry-retained.png).
- Cleanup: [reverse traffic](screenshots/2026-09-30/forwards/12-fixed-reverse-traffic.png),
  [SOCKS traffic](screenshots/2026-09-30/forwards/13-fixed-socks-traffic.png),
  [all stopped](screenshots/2026-09-30/forwards/14-fixed-all-stopped.png),
  [logout after pending setup](screenshots/2026-09-30/forwards/17-logout-remains-closed.png).

## Validation and limits

`cargo test --workspace`: 859 unique tests passed (app 100, core 404, SSH 47,
terminal 88, UI 207, update 13); seven gated live tests and two documentation
examples remain ignored. The configured-bind regression additionally ran with
IPv6 loopback (`::1`) to verify address handling without requiring a macOS
IPv4 loopback alias. Workspace Clippy with all targets and warnings denied,
formatting, Linux build and UI compilation passed. SDTEST-1942 exercises actual
SSH echo, occupied configured bind and listener shutdown; SDTEST-1943 exercises
pending cancellation and stale-completion ownership. SDTEST-1944 records the
remaining automated native click-coverage gap.

Native GUI checks were Linux only. CI runs Linux workspace tests, native macOS
compilation and macOS/Windows core checks; it does not run these new SSH/UI tests
on macOS or Windows. Signing was unchanged. Subscription execution was verified
in the preceding [feature follow-up](feature-followup-qa-2026-09-30.md), and was
not rerun for this forwarding-only change.
