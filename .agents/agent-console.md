# Agent cockpit presentation

The Dev Agents surface is a worktree-first cockpit: project/worktree
navigation, named concurrent sessions, a compact tab strip, and one ordered
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
- Treat provider, target, permissions, workdir, and model as one execution
  context, collapsed into the compact session header by default.
- Change execution context through the core context setter so an opaque resume
  token never crosses target, permission, workdir, model, or provider changes.
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
