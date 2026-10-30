# Agent cockpit presentation

The Dev Agents surface is a worktree-first cockpit: project/worktree
navigation, named concurrent sessions, a compact session header, and one ordered
conversation/execution timeline.

- Use `shelldeck_core::agent_session::AgentSessionCollection` as the canonical
  session, status, attention, message, and trace model. Keep only GPUI draft
  state and run-to-session routing in the view.
- Route provider events by run UUID. The compatibility singleton event method
  is insufficient for parallel agents and background-session attention.
- Render every structured `AgentTraceKind` inline with conversation messages:
  commands, file reads, diffs, tests, tools, and fallback activity. Do not hide
  technical work in a popover.
- Render agent prose with `Markdown::compact()` and technical details with the
  shared monospace style. Bound both through the core session/runtime model.
- Keep plain lifecycle activity compact in the timeline. Model/token/cost details
  expand on demand; repository snapshot diagnostics belong in Changes. SSH runs
  do not create local baseline failures. Preserve actual failure/cancellation
  status and keep structured tool, command and diff activity visible inline.
- Keep provider, status, unread attention, and identity visible in navigator
  rows and tabs. Selecting an actually visible session marks it read and
  restores its context and composer draft; hidden selection is navigation
  identity only.
- Adding or opening a project lands in Agents with its saved checkout selected.
  Project registration alone must not start an SSH terminal or provider; New
  workspace is the explicit retained-workspace launch action.
- The Agents navigator includes every remote session in the authenticated
  Platform directory, including observation-only and stale sessions, without
  requiring a local checkout mapping. Reuse Fleet's typed directory and retained
  cockpit; refresh it while Agents is visible. Remote selection keeps local
  sessions hidden for unread accounting and never attaches or claims control
  implicitly. Account reset clears the shared directory.
- Keep local projects above the remote directory. Project and remote groups
  collapse with persisted preferences; remote starts collapsed. Search reveals
  matching sessions without changing saved collapse. Paginate remote rows, keep
  all directory records reachable, and distinguish attachable from directory-only
  sessions. Lifecycle summaries such as open/closed are status, not titles.
- Remote directory selection starts a read-only connection check automatically; retry remains explicit. Read retained
  history and command state through the shared typed Platform client; no SSH host,
  provider, attachment or control lease may be inferred from a directory ID.
  A fresh exact-session command state permits a reviewed SessionFollowUp to resume
  its original runtime without a live attachment. Keep idempotency reconciliation,
  revision fencing, draft isolation, capability checks and account-reset cleanup.
  A check never starts an agent; control of a live session remains a separate action.
- Keep primary Files/Changes/Terminal actions in the session header; secondary
  tools expand on demand. Browser is a resizable companion to the conversation
  and composer (stacked on narrow windows), never a replacement chat tab.
  Conversation/Editor tabs appear only when a document is open. Browser runtime
  errors render once in the browser pane; the header globe hides/reopens it. Keep a searchable project/target chooser above
  the composer and agent, permissions, and model/reasoning controls in its footer.
  Context intake shares one + menu. The header settings control exposes the full
  execution context, including manual target and directory editing.
- Treat provider, target, permissions, workdir, and model as one execution
  context. Keep its full editor collapsed by default. A provider change clears
  provider-specific model/effort choices; project navigation retains valid unsent
  choices and selecting the current session must not reset them.
- Change execution context through the core context setter so an opaque resume
  token never crosses target, permission, workdir, model, or provider changes.
- Missing saved SSH connections remain SSH in controls and display, and block
  execution. Reconnect SSH requires an explicit saved connection choice; repair
  preserves the exact project folder, transcript and draft, resets provider resume
  state, and refuses active or workspace-bound sessions and checkouts.
  Keep project navigation available beside repair, and disable only Send (not
  draft editing) when the target is missing. Refresh Send readiness on each edit.
- Lock execution-context controls once Workspace binds a session to a checkout;
  worktree navigation creates or selects a separately bound session instead of
  mutating the context underneath an existing workspace tab.
- A catalog-bound local run enters its retained Workspace cockpit. Keep its
  agent tabs in one pane, create the authorized checkout file pane alongside
  it, and let the typed split controls expose the retained terminal without
  duplicating one AgentConsole host for different session bindings.
- Mutating access always requires confirmation. Running sessions cannot be
  removed, and concurrency limits remain enforced by the core model/runtime.
- Keep the timeline as the single scroll owner and the composer as its
  non-shrinking sibling. Do not nest a transcript scroll or put the composer
  inside it.
- Eager repository snapshots belong only to local write-enabled turns. Do not
  put read-only chat behind Git observation; keep Changes available on demand.
- Deliver streams in bounded batches and fence completion behind queued output.
  Schedule persistence once per batch. Only the selected visible session may
  request follow-latest on the shared timeline scroll owner.

- Native SSH conversations are distinct from the Automonique directory. Discover
  only on a user-selected saved server, without launching provider processes.
  Keep bounded output, partial-result warnings and last successful fetch visible.
  Reconnect must revalidate native ID/provider/cwd/model and exact SSH destination;
  prepare only, default to read-only, and preserve the native resume ID through
  Send without catalog path translation. Load native transcript pages read-only into
  the single chat scroll owner; never replay old stream events, approvals or tools.
  Persist only a passive source reference and immutable-prefix cutoff, not resume
  authority. Page callbacks are session/request/source fenced; app restart requires
  explicit validated reconnect before sending. Normal reload preserves the cutoff
  to avoid duplicating new local turns. Account reset clears history caches, discovery
  and destination custody.

## Workflow completion gate

Use `docs/testing/AGENTS_ACCEPTANCE.md` for Agents changes. Review the affected
end-to-end journey against the existing user requirements and pinned reference,
including navigation, saved state and recovery. Verify the native interaction
in addition to reducer/parser tests. Report fixture/live coverage separately;
never substitute a passing test count for an unverified workflow or require the
user to discover adjacent defects that the acceptance journey already covers.
