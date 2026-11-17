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
  errors render once in the browser pane; the header globe hides/reopens it. Keep the searchable project chooser and the
  machine chooser above the composer of an empty session (start screen); in a
  conversation the project lives in the header breadcrumb and Détails, and the
  chooser comes back only to repair a missing SSH target. Agent, permissions,
  and model/reasoning controls stay in the composer footer.
  Context intake shares one + menu. The settings control (header « … » row, or
  Détails → Configurer) exposes the full
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
  inside it. The one exception is the start screen of an empty session, which
  has no transcript: it owns its scroll area and holds the composer. Give that
  column a definite width (`render_start_screen(width, …)`): percentage widths
  do not resolve inside the scroll area and the multi-line input then paints
  nothing.
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

## Cockpit layout from the owner's mock-up (SDUC-595)

`agent_console_view/cockpit.rs` holds the pieces; keep new chrome there.

- The sidebar sits on the page colour (`bg_primary`); the conversation and its
  side panel are one card on the lightest surface (`bg_sidebar`, as the User
  home panels), inset 8 px, radius 12. Chips, the composer frame
  (`Composer::roomy`, SDPATCH-134) and cards inside it are transparent so they
  take the card's colour.
- Sidebar head: primary « Nouvelle session », a search face that becomes the
  real input on click (`agent-nav-search-trigger` → `agent-nav-search`, so the
  resting sidebar still has no input), the Tout / Local / SSH filter
  (`NavigatorTargetFilter`, view state only), then « Projets » with the actions
  menu and « + » (project setup). Project rows show their machine badge where
  the hover actions appear, so labels never move. Local discovery, SSH
  conversations and the Platform directory live under one « Conversations
  importées » group that starts closed (`IMPORTED_GROUP_OPEN_KEY`). Foot: the
  running count, Paramètres, Aide (`AgentConsoleEvent::OpenHelp`). No shortcut
  hint on the search face: Ctrl/⌘ K is Quick Connect.
- Focused Agents view title bar (SDUC-596, `workspace/chrome.rs` +
  `TitlebarTabsPane`): « Inklura ▾ » (full app, settings, help), « Agents ▾ »
  (the four views; since 2026-11-17 the only view chooser, in every title bar, with a dot when another view needs you), then the open
  conversations as tabs with × and +. Tabs are view state (`open_tabs`): the
  selected session always has one, × closes the tab and never archives, the
  neighbour takes the selection, closing the last opens a new session. The
  cockpit drops its narrow-window tab row while `tabs_in_titlebar`.
- One title bar kit for every window state (`workspace/presentation.rs`:
  `TITLEBAR_CONTROL` 32, `TITLEBAR_RADIUS` 8, `TITLEBAR_TEXT` 13,
  `TITLEBAR_GAP` 8, `titlebar_brand`, `titlebar_control`): signed out, full
  app, Agents and the other focused views share the brand (28 px mark, 16 px
  « Inklura »), control heights and the action order (Pro, AI, mode, site,
  bell, Aide, light/dark switch, account). Aide and the switch are in every
  title bar, signed in or out. In the full app the brand chevron shows the menu row; in a
  focused view it opens the product menu. Add the hover at the call site:
  GPUI panics on a second hover style.
- Every header inside the conversation card is `PANEL_HEADER_HEIGHT` (54):
  session header, Détails, Fichiers / Modifications, tool panels, so their
  bottom borders make one line. The sidebar head starts 8 px down, level with
  the card's top edge.
- « Récents » heads the sidebar list: the five conversations with messages
  touched last (machine filter applies, hidden while searching), foldable
  (`RECENT_GROUP_KEY`). Rows use `agent-recent-*` selectors so they never
  duplicate the project rows' `agent-nav-session-*`.
- Session header: panel toggle, folder, `project / session`, Files, Changes,
  Terminal and Browser icons, « Détails » (`AgentInspectorTab::Details`,
  persisted as `details`) and « … » for the secondary toolbar, which also holds
  the execution-context settings.
- Replies: the first agent message of a turn carries the robot tile, the agent
  name and « Terminé • 2 min 06 s »; agent text is indented under it; the
  person's messages are a quiet bubble without a label. Completed work is one
  card (« N actions effectuées » / « Afficher le détail »). Under a finished
  reply, `render_turn_summary` lists the files of the last turn's Diff traces
  (+/− counts, « Examiner les changements ») unless the repository review
  already lists changes, and the outcome of its tests (« Voir les résultats »
  opens Détails).
- Détails: environment (machine, connection state, folder, branch), agent
  (state, model, permissions, Configurer = the context editor), context meter
  from `usage.context_used / context_size`, the last turn's checks, then
  folded « Outils et connexions » and « Ressources système ».

## Workflow completion gate

Use `docs/testing/AGENTS_ACCEPTANCE.md` for Agents changes. Review the affected
end-to-end journey against the existing user requirements and pinned reference,
including navigation, saved state and recovery. Verify the native interaction
in addition to reducer/parser tests. Report fixture/live coverage separately;
never substitute a passing test count for an unverified workflow or require the
user to discover adjacent defects that the acceptance journey already covers.

- Desktop native CLI discovery shares the native history/preview/reconnect workflow
  with SSH, using `agent_local_sessions` Rust readers rather than Python or provider
  subprocesses. Local source references serialize `connection_id: null`; old SSH
  UUID references remain unchanged. Missing/malformed source IDs must not become
  local sources. Local discovery is opt-in and persisted; Hide disables refresh
  without removing imported chats. Respect configured CLI roots, bounded paging,
  last-good results, native IDs and read-only revalidation after restart.
