# T3 Code behavior and presentation review

Reference: `pingdotgg/t3code` at
[`6575ce3`](https://github.com/pingdotgg/t3code/tree/6575ce32508bd7908444241f3719aa33aaf77f90),
checked against upstream on 2026-10-30. Also exercised the official v0.1.5 Linux
release, checksum verified, with a disposable profile and project. No personal
projects, provider credentials, or live agent execution were used. The release
walkthrough and source review are distinct evidence; this is not a claim to
have exercised every provider or remote route.

## What makes the reference work

T3 is organized around a task thread in an execution environment. Its sidebar
shows work, its center shows the conversation, and its side panel shows the
machine, checkout, and supporting tools. Project, model, permissions and pending
input are parts of the same thread lifecycle. This coherence matters more than
copying the colors, rounded corners, or icon placement.

Observed in the running reference:

- Onboarding proceeds through computers, agents, then project/conversation import.
  Missing agent installations have a status and setup route before sending.
- Add project uses a searchable source/folder picker and lands directly in a draft.
  The project is selectable in the draft heading; machine and checkout belong in
  the details panel. The center stays visually quiet.
- A missing provider disables Send with a nearby provider-settings action.
  Settings retain project and machine scope; returning restores the draft route.
- Typing a draft and opening another task creates an identifiable **Unsent draft**
  row. Discard exposes **Ctrl+Z to undo**; Undo restores the row and its text.
- The layout has one thin breadcrumb header, three panel toggles, a constrained
  conversation/composer column, and a compact context panel. Ordinary navigation
  is not a stack of full-width forms and diagnostic banners.

The missing-provider case also displayed a disabled **Environment disconnected**
button despite the environment being reachable. That label should not be copied:
provider readiness and transport connectivity are separate states.

## Gaps and implementation order

| Priority | T3 behavior | ShellDeck gap / bridge |
| --- | --- | --- |
| P1 | Active thread order stays fixed while activity arrives. | Separate navigation order from stream timestamps. Preserve pin precedence; streams must not move the pointer's target. |
| P1 | Approval, input, working, failed and unread completion have distinct presentations. Collapsed projects summarize hidden work. | Show what needs the user, including in collapsed groups; keep read completions quiet and mark pending drafts. |
| P1 | Started-thread model transitions are checked before selecting an incompatible continuation. | An imported native conversation must not silently become a fresh provider session after changing model, provider, permissions or destination. Offer an explicit new conversation and preserve the original. |
| P1 | Plan/Build is distinct from permission modes; options reflect provider capability. | Current Claude read-only actually invokes Plan; Jcode read-only exposes only its read tool. Name these honestly now. A true independent interaction-mode contract needs provider/runtime work. |
| P1 | Drafts retain context and model choices; pending messages are a durable lifecycle. | ShellDeck still persists text but not the complete attachment/effort/queue bundle. Persist descriptors and visible missing-file recovery; restore queued work paused, never auto-run it on restart. |
| P1 | Imported conversations become ordinary durable threads; older history is prepended with viewport anchoring. | Replace the separate page viewer/fresh-history surface with one reconciled timeline using durable source/message identity. Preserve full native content and require fresh execution revalidation separately. |
| P1 | Provider/model inventories belong to a machine and expose readiness, refresh and setup. | Extend verified discovery beyond local Codex; one coordinated Agent/Model chooser, target-scoped capabilities and last-used model separate from requested Automatic. Never invent available models or reasoning levels. |
| P2 | Row menus, keyboard navigation, F2 rename and immediate Undo keep management near the task. | Reuse existing rename/pin/archive handlers in row actions. Add Undo, accessible tree navigation and search excerpts that open the actual matched message. |
| P2 | Consecutive tool calls become a compact work summary with bounded expandable details. | Group completed activity, expose failures and pending approvals immediately, and avoid a full bordered card for every tool event. Keep all retained output reachable. |
| P2 | Submit, Queue, Steer, Resume and disabled reasons are explicit at the primary action. | Drive button, accessible label and keyboard behavior from one send-intent state. Preserve queued drafts when admission fails. |
| P2 | Project-first import and persistent environment connections make remote work ordinary navigation. | Add deliberate project/folder import to the server/provider inventory and reuse target-scoped SSH connections. Preserve ordinary SSH without mandatory remote installation. |

## Source evidence

All paths below are relative to the pinned T3 checkout.

- Navigation: `docs/user/thread-sidebar.md`,
  `apps/web/src/components/sidebar/SidebarThreadRow.tsx`,
  `SidebarProjectFolderHeader.tsx`, `apps/web/src/components/Sidebar.logic.ts`,
  `apps/web/src/lib/sidebarTree.ts`.
- Complete draft state and failed-send recovery:
  `apps/web/src/composerDraftStore.ts`,
  `apps/web/src/components/chat/useComposerSend.ts`.
- Continuation compatibility: `apps/web/src/components/ChatView.logic.ts`,
  `apps/web/src/components/chat/useComposerSelection.ts`.
- Model/permissions/intent: `apps/web/src/components/chat/ModelPickerContent.tsx`,
  `ComposerFooterControls.tsx`, `runtimeModeConfig.ts`, `ComposerPrimaryActions.tsx`.
- Presentation: `apps/web/src/components/chat/ComposerSurface.tsx`,
  `ComposerControlsBar.tsx`, `TimelineWorkGroupRows.tsx`, `ChatRightPanel.tsx`.
- Native imports: `apps/server/src/project/AgentSessionScanner.ts`,
  `AgentSessionImporter.ts`, `apps/web/src/components/onboarding/WelcomeWizard.tsx`.
- History: `packages/client-runtime/src/state/threadHistoryMerge.ts`,
  `apps/server/src/orchestration-v2/threadHistoryPaging.ts`,
  `apps/web/src/components/chat/MessagesTimeline.tsx`.
- Remote lifetime: `packages/ssh/src/tunnel.ts`,
  `packages/client-runtime/src/connection/supervisor.ts`, `docs/internals/overview.md`.

## What not to copy

T3's desktop SSH mode installs/reuses a remote T3 server. ShellDeck's explicit
requirement is to open existing folders with ordinary SSH and no server helper.
Adopt the connection lifecycle and coherent environment presentation, not that
installation requirement.

T3's native Claude/Codex import is not complete-history import: this revision
filters to 30 days, caps an import at 100 transcripts, and retains at most 200
user/assistant messages per transcript, excluding tool/reasoning history. Keep
ShellDeck's fuller paged reader and Jcode support while improving presentation.

T3 defaults new work to Full access. Preserve ShellDeck's explicit authorization
and conservative native reconnect policy. Its broader scheduling, hosting,
sharing, billing and drag/snooze systems are not prerequisites for a coherent
agent chat and should not displace the gaps above.

## Verification and delivery

Reference review completed: source plus onboarding, project entry, missing-provider
recovery, draft switching and discard/Undo in the isolated release. No live
generation, model switching during execution, SSH runtime bootstrap or remote
history import was performed in T3. Those findings are source-reviewed.

The first implementation slice is stable attention-aware navigation and explicit
native conversation continuity. Remaining rows are real follow-up work, not
claims of completed parity. Use `AGENTS_ACCEPTANCE.md` to verify each affected
journey and record implementation evidence here after validation.

## Test-suite follow-up

Test sources were reviewed at upstream HEAD
[`9b1d351`](https://github.com/pingdotgg/t3code/tree/9b1d351ba0f63f527f884ffb3f2e394f50388764)
(2026-10-30). This is a test-source review, not an execution of T3's suite;
the earlier behavior reference remains pinned to `6575ce3`.

T3 separates pure reducer tests from real Chromium component tests. Its web test
command runs both; render-count scenarios are a separate opt-in project. CI
installs Chromium and runs the web suite. See the
[test projects](https://github.com/pingdotgg/t3code/blob/9b1d351ba0f63f527f884ffb3f2e394f50388764/apps/web/vite.config.ts)
and [CI job](https://github.com/pingdotgg/t3code/blob/9b1d351ba0f63f527f884ffb3f2e394f50388764/.github/workflows/ci.yml).

| Reference coverage | ShellDeck counterpart and disposition |
| --- | --- |
| [Long active tool groups](https://github.com/pingdotgg/t3code/blob/9b1d351ba0f63f527f884ffb3f2e394f50388764/apps/web/src/components/chat/MessagesTimeline.active-tools.test.ts): 1,000 updates, ordering and latest work, folded/expanded. | SDTEST-2846 replays 1,000 updates for Claude, Codex, Jcode and DeepSeek through the production model and timeline grouping. It respects ShellDeck's 512-entry retention bound while requiring every retained payload and unfinished/failed row to remain reachable and ordered. |
| [Timeline reducer scenarios](https://github.com/pingdotgg/t3code/blob/9b1d351ba0f63f527f884ffb3f2e394f50388764/apps/web/src/components/chat/MessagesTimeline.logic.test.ts): failures, late updates, run boundaries and reloads. | Existing SDTEST-2793/2794 and 2840 plus SDTEST-2846 cover stream fences, completed-work disclosure and durable failure ordering. Do not claim parity with T3's attempt/subagent projection. |
| [Rendered copy controls](https://github.com/pingdotgg/t3code/blob/9b1d351ba0f63f527f884ffb3f2e394f50388764/apps/web/src/components/chat/MarkdownCopy.browser.test.tsx): actual control clicks and browser clipboard fallback. | SDTEST-2844/2845 exercise the production reveal/copy handlers with GPUI's private test clipboard across message/command/diff/test/tool/native sources, conversation switches and replaced snapshots. SDTEST-2848 adds real GPUI clicks, exact private clipboard contents, replacement of folded/raw bodies and composer bounds across two themes, three window sizes and local/native transcripts. Native clipboard handling has no browser fallback to port. |
| [Rendered timeline journeys](https://github.com/pingdotgg/t3code/blob/9b1d351ba0f63f527f884ffb3f2e394f50388764/apps/web/src/components/chat/MessagesTimeline.browser.test.tsx): variable-height content, disclosure, thread switching, scrolling, attachments and concurrent approvals. | SDTEST-2848–2854 render the production cockpit and dispatch real mouse, key and wheel events: disclosure/copy, conversation drafts, variable-height streaming, history retry/paging, queued Enter, restart and exact question/approval decisions. SDTEST-1713 measures Send/Stop at 28 px. These offline scenarios do not claim live-provider, split-pane or concurrent-approval support; Codex serializes pending interactions. |
| [Scroll-state transitions](https://github.com/pingdotgg/t3code/blob/9b1d351ba0f63f527f884ffb3f2e394f50388764/apps/web/src/components/chat/timelineScrollAnchoring.test.tsx): hydration, thread changes, queued starts and once-only framing. | SDTEST-2850 verifies follow-at-bottom, reading-position retention during streaming and explicit Jump to latest with a fixed composer. SDTEST-2852 verifies paging resets to the page top and error/retry stays reachable at the footer. ShellDeck replaces bounded pages rather than prepending T3's unified timeline. |

The first increment keeps `.agents/testing.md`'s production-controller/pure-helper
approach instead of adding a browser framework to a native Rust app. SDTEST-2847
adds a wrapped-data/streaming/Unicode corpus; it exposed a short encoded final
line escaping the original folding rule, now covered by regression. Copy/reveal
callbacks now resolve their current retained source and refuse stale page/chat
actions. The `agent-history` CI job runs the real native-reader/discovery fixtures
without SSH, provider binaries or private dependencies. Those Python fixtures
previously lacked a CI step. No new test dependencies or paid provider runs.

The shared GPUI interaction harness now runs by default with the UI suite and
CI's isolated profile. It uses the production renderer/layout and control event
dispatch on GPUI's test platform, with a private clipboard and fixture provider
events. It does not require a GPU or start a provider. GPU painting remains a
separate native walkthrough; a black Xvfb presentation cannot satisfy it.

The scenarios reproduced product defects: Stop was 40 px while Send was 28 px;
new/restored conversations lost composer focus; history errors had no reachable
footer Retry; duplicate approval clicks emitted multiple decisions. Fixes retain
focus, preserve bottom-follow during loading/error layout changes, expose footer
Retry, and reserve approval decisions until an exact acknowledgment or transport
rejection. Stale transport failures cannot reopen another decision. Test metadata
also retained removed bounds across frames; SDPATCH-122 corrects that observability
bug, with production paint behavior unchanged.

Validation of the rendered-suite follow-up: all 1,130 runnable workspace tests
passed (325 UI), with nine existing live/doc tests ignored. Strict workspace
all-target Clippy, formatting and all three Python reader suites passed. Native
Dark/Solarized Light screenshots at 900×700 and 620×560 are recorded in
`AGENTS_ACCEPTANCE.md`. No remote CI run or live provider execution is claimed.

### Implemented and validated in this pass

- Stable pinned/creation ordering replaces activity timestamp sorting. Completed
  read conversations are quiet; approval, question, failed and unread completion
  states remain distinct. Collapsed projects and Other conversations share a
  count/attention summary. Draft text or attachments add a pencil indicator.
- Imported native-context edits remain proposals. A banner beside the composer
  offers restoring the original settings or explicitly creating a new draft.
  The new draft copies text, attachments and reasoning selection; the original
  history and resume binding remain intact. Context edits, stale captured sends
  and project-connection repair all pass the same guard. No agent starts from
  this action. Ordinary unsent conversations retain normal editable contexts.
- Claude's existing read-only mapping is labeled Plan mode; Jcode/DeepSeek's
  mapping is labeled Read tools only, with explanatory tooltips. This is honest
  presentation of the current CLI contract, not a new independent Plan/Build API.
- The standalone rendering fixture now uses the production Inter font instead
  of GPUI's default monospace fallback.

Validation: SDTEST-2830–2832 passed, 14 core session tests passed, all 304 UI
library tests passed, strict core/UI all-target Clippy passed, and desktop and
preview builds passed. EN/FR parsing, key/placeholder parity and diff whitespace
checks passed. The native render fixture with a background approval demonstrates
its urgent marker/count even when its project is collapsed; evidence is
`/tmp/shelldeck-t3-render/attention.png` and `expanded.png`. The continuity flow was exercised by
GPUI integration tests; its live-provider execution path was not invoked.

Limits of this slice: native continuity protection applies to sessions carrying
an imported history reference. Compatibility is deliberately exact (including a
saved SSH display label); independently revalidating compatible model/access
changes is still future work. Explicitly creating a new draft retains queued
follow-ups on their original conversation. Navigating away drops incompatible
proposed settings but preserves the original context and draft. At the end of
this first slice, draft bundles, unified history, remote model catalogs, work-log
grouping and row menus were still open; the second pass below addresses draft
bundles, work-log grouping and row menus.

### Second parity pass: activity, direct actions and durable drafts

The next implementation keeps the same underlying features and execution model:

- Successful contiguous tool/command/read/test/diff events from a known turn
  share an inline disclosure. Original trace IDs and payloads remain intact;
  failures, pending/running/unknown operations, prose and history boundaries stay
  outside the fold. Inspector selection expands its exact trace and uses a scroll
  anchor instead of jumping to the end of the chat. Group expansion survives
  adjacent successful operations joining the group. Unlike T3's inner work-log
  viewport, expanded output uses ShellDeck's single conversation scroll owner.
- Progress plans show completed/total and the current step; every step remains
  available on expansion. Approvals, questions, authentication recovery, artifacts
  and turn-level Changes actions remain independently visible.
- Each conversation has a direct actions menu (ellipsis/right-click): rename,
  pin, duplicate, copy, Markdown/JSON export, archive/restore. It reuses the Library
  handlers, validates current run state, and retains background drafts/context.
  Archive has a persistent dismissible Undo action. Imported remote conversations
  label copy/export/duplicate as **local transcript** operations; an empty local
  transcript disables those actions. This does not implement full-server export.
- Versioned draft bundles retain the execution context, reasoning selection,
  attachment snapshots and follow-ups alongside existing text drafts. Version 1
  libraries remain readable. Restored follow-ups are paused; Review never sends.
  Saved images use bounded private retained assets and require a check before
  sending after restart. Unavailable attachments remain visible for repair.

This closes only the corresponding slices in the gap table. Unified native/local
history scrolling, broader target-scoped model discovery, true Plan/Build mode
and the other recorded interaction gaps remain separate work. No terminal,
SSH-folder, browser, file, diff, export or approval capability was removed.

Second-pass verification: 24 core session/storage tests and 311 UI library tests
passed. Subsequent presentation fixes passed final-source strict core/UI
all-target Clippy, desktop/preview builds and native checks. EN/FR key and
placeholder parity and diff whitespace checks passed. SDTEST-2833–2841 cover
background row actions, archive/Undo, duplication context, durable attachments,
paused follow-ups, shutdown draft persistence and lossless work grouping.

The isolated native preview was exercised in dark and Solarized Light themes,
including a 900-pixel-wide window. Clicks verified work expansion, right-click
and ellipsis menus, rename with Enter, archive and Undo. Native inspection found
and fixed two additional issues: wrapped user messages clipped their action row,
and an invisible draft-marker tooltip intercepted the neighboring ellipsis.
Final evidence: `/tmp/shelldeck-parity-render/verified-draft-menu.png`;
additional captures: `dark.png`, `expanded-light.png`, `renamed.png`,
`archived.png` and `undo.png` in the same directory. The preview uses Inter.

Draft recovery is covered by automated tests, not a live provider submission.
Retained image assets are capped at 128 MiB/4096 files, with no automatic asset
deletion. A restored attachment check is all-or-nothing; unavailable items must
be removed or reattached before checking again. The running user application
was not restarted and this pass did not publish a release.
