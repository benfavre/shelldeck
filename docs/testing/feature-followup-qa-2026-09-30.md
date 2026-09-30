# Feature follow-up QA — 2026-09-30

This continues the [native visual pass](visual-qa-2026-09-30.md) and
[subscription checks](subscription-qa-2026-09-30.md). The published v0.9.10 and
rebuilt debug binaries ran on a private Xvfb X11 display, private D-Bus and
temporary XDG profiles. HOME and provider credential locations stayed unchanged.
Manage data is fictional and served by the repository demo fixture, extended
locally with per-request delay and 401 injection. Established SSH connections
used a disposable, key-authenticated Paramiko loopback server. Signing was not
changed.

## Reproduced defects and fixes

| Journey | Published behavior | Rebuilt behavior / evidence |
|---|---|---|
| Select request A, then B while A takes six seconds | B opens first; the late A response replaces the detail cache, making B's sheet disappear | Only the latest detail read may publish. [B before delayed A](screenshots/2026-09-30/followups/detail-newer-before.png), [disappeared sheet](screenshots/2026-09-30/followups/detail-stale-before.png), [retained B after delay](screenshots/2026-09-30/followups/detail-after.png) |
| Revoke the fixture token during requests/sync | Expiry toast appears but User remains signed in and continues polling | Current 401 invalidates the session, stops its polls and returns to welcome. 403 remains a permission error; old-session responses cannot reject a new login. [Before](screenshots/2026-09-30/followups/expired-before.png), [after](screenshots/2026-09-30/followups/expired-after.png) |
| Start a creation draft, revoke the token, then sign in again | Source inspection found draft buffers survived invalidation | Creation/comment drafts, pending request AI/import state and mention identities clear. Re-login using the same fixture token opens an [empty creation sheet](screenshots/2026-09-30/followups/draft-after-relogin.png) |
| Logout while Manage fetches or writes are pending | Completion callbacks lacked account ownership checks; sync could also write old profiles before the callback | Session generations fence identity, sites, lists, details, people, sync and writes. Fetch remains in the background; profile merge/persistence is accepted only by the current foreground session. Settings receives refreshed whoami identity. |
| Reverse forwarding with a loopback remote bind | The saved remote host was ignored; the SSH request always asked for 0.0.0.0 | Saved remote host reaches tcpip-forward. Stop sends cancel-tcpip-forward for that exact host/port while the owning SSH session can remain connected. SDTEST-565 covers traffic, directional counters, existing-channel shutdown and explicit cancellation. |
| Choose another provider/SSH host before the first agent run | Subscription help still described the session's previously committed provider/local host | Setup uses the selected controls immediately, including [target-host login guidance](screenshots/2026-09-30/followups/ssh-subscription-guide-after.png). Opening help never launches login/inference. |
| Run Claude, switch to Codex, run again | Both earlier and newer replies were labeled Codex | New replies retain their original provider in durable history. Legacy files without this metadata still load. SDTEST-1941 covers streamed deltas, context switch and reload; a repeated native Claude → Codex SSH journey keeps [both author labels](screenshots/2026-09-30/followups/provider-history-after.png). |

## Additional live checks

**Published Linux archive and update:** a fresh extraction of v0.9.10 opened
successfully. A separate v0.9.9 installation, with its own XDG profile and
updates enabled, detected v0.9.10, downloaded the actual GitHub asset, verified
SHA-256 and replaced its binary. The installed binary matches the fresh
v0.9.10 extraction; the v0.9.9 backup remains present. A real restart logged
`Starting ShellDeck v0.9.10` and rendered the User home. This verifies archive
installation and the in-app Linux updater, not the home-writing installer script.

**All three SSH tunnels:** native Start controls opened Local → Remote
(127.0.0.1:8899), Remote → Local (remote 127.0.0.1:8895) and SOCKS5
(127.0.0.1:8896). Curl traversed each tunnel to the fixture API and received the
expected JSON. Native Stop controls closed all three ports; fresh TCP connects
were refused. The SSH server recorded `remote-listen 127.0.0.1 8895` and
`remote-cancel 127.0.0.1 8895`. [All active](screenshots/2026-09-30/followups/tunnels-active.png)
and [stopped with retained byte counts](screenshots/2026-09-30/followups/tunnels-stopped.png).
A deliberately changed disposable server key was also rejected, as expected;
subsequent checks used a new fixture endpoint and stable key.

**Subscription agents over SSH:** provider status on the fixture target reported
Codex ChatGPT login and Claude.ai/Max. Both native Agents runs used the SSH
target, isolated workdir and Read Only access, returned
`SHELLDECK_SSH_SUBSCRIPTION_OK`, and completed successfully. API-key environment
variables were absent in both app and target. The target is a loopback process
under the same OS account, with existing provider-owned OAuth storage: this is
real SSH transport and agent execution, not a fresh independent remote-host
login. No OAuth token was copied by ShellDeck. Codex's installed MCP workers
emitted shutdown diagnostics after the successful reply; ShellDeck retained
those diagnostics and still reported the successful process completion.

**Desktop fallback and X11 global shortcuts:** a separate temporary profile
requested `start_hidden=true`, `close_to_tray=true` and both global shortcuts.
With no tray service, the app logged the expected warning and showed its main
window. After closing the Dock, the main close button exited the process
instead of leaving an invisible app. With an owned xmessage window focused, Ctrl+Alt+Space opened the
[standalone command palette](screenshots/2026-09-30/followups/global-palette.png)
and Ctrl+Shift+Space opened the [AI Dock](screenshots/2026-09-30/followups/global-ai-dock.png).
No prompt was submitted. This exercised the private native X11 backend;
it makes no claim about the user's Wayland portal or a physical desktop tray.

## Validation and limits

The full workspace suite passes 857 unique tests (app 100, core 404, SSH 46,
terminal 88, UI 206, update 13; seven live tests and two documentation examples
remain ignored). Linux debug build, UI compile check, Clippy over all
targets, formatting and whitespace checks pass. SDTEST-1937..1939 exercise
completion ordering, account transitions (including identical token reuse) and
401/403 handling. SDTEST-1941 exercises provider attribution and legacy history.
The workspace orchestrator remains below 2,000 lines. Cross-platform CI is
checked before merge.

SDTEST-1940 and the earlier native render/click inventory entries remain Red for
automated GPUI coverage; screenshots do not substitute for such a harness.
Production cloud writes, fresh remote-host subscription login, successful OS
keychain/OIDC, physical multi-monitor/tray integration and native macOS/Windows
interaction still require their dedicated environments. Xvfb has one display,
no window manager, no tray service and no portal; these limitations are not
reported as product bugs or passing physical-desktop tests.
