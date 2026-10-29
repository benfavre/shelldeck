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
- Keep primary Files/Changes/Terminal actions in the session header; secondary
  tools expand on demand. Conversation/Editor/Browser tabs appear only when
  a document or browser is open. Keep a searchable project/target chooser above
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
