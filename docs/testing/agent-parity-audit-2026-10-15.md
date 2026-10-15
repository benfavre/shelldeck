# Agents workspace parity implementation audit — 2026-10-15

The Agents experience now includes durable conversations, richer input and live
local Codex interaction, actual local Git review, explicit Git delivery, browser
snapshots, adjacent workspace tools, and bounded local goals/recurring tasks.
This is a substantial implementation pass over the screenshot/source audit. It
is not full ChatGPT parity: the remaining limitations below are part of the
current product contract.

## Implementation and boundaries

| Area | Implemented | Current boundary |
|---|---|---|
| Execution context | Header, Files, previews and actions use the selected effective context. Active/catalog-bound runs keep their captured authority; stale asynchronous results cannot replace a different session/context. Mapped SSH checkout admission updates the starting turn and retires an incompatible resume handle. | Remote transports retain their existing capability limits. A host/path label does not create remote authority. |
| Conversation continuity | Private bounded JSON storage, debounced saves plus shutdown flush, drafts/preferences, interrupted-run recovery, rename, pin, archive/restore, transcript search, fork and Markdown/JSON export. Corrupt/future/oversized history is preserved on failure. | Provider process/resume handles are not persisted. Queued runs/attachments are in memory. Workspace tab layouts and retained tool-pane arrangements are not durable. No cross-device/cloud conversation continuation or hosted share link. |
| Context changes | Provider/target/access/workdir/model changes create visible history boundaries. Forks retain readable history without inheriting an active provider session. | Old messages are not silently replayed into a new model context. Automatic summarization/context reconstruction is not implemented. |
| Composer | File picker, workdir-scoped local file mentions, explicit text/clipboard context, local Codex images, screenshots, image preview/annotation and removal. Inputs remain editable while work runs. Queued turns support edit/reorder/remove and retain their captured context/options. | File mentions use a picker rather than an inline fuzzy `@` menu; remote file mentions/folder/selection/terminal-output context capture and drag/drop are not complete. Images are local-Codex-only. Attachments/queues do not survive restart. |
| Models and response actions | Explicit local Codex `model/list` discovery, searchable advertised choices and model-specific reasoning levels, plus manual model override. Named discovered models validate captured image/effort inputs before launch. Response copy/quote and user-message edit-to-draft. | Discovery is local-Codex-only and does not prove account entitlement or inference success. Automatic model selection remains owned by Codex configuration; a catalog recommendation is not its resolved value. Other providers, SSH, unknown custom models and failed discovery retain manual entry/provider validation. Editing prepares another turn; no transcript rewrite, dedicated regenerate/feedback or nested side-chat workflow. |
| Live execution | Local Codex app-server streaming, Send now steering, typed questions, exact one-shot approvals, prioritized Stop and rejected-input draft preservation. Claude partial content and Codex snapshots avoid duplicate visible answers. | SSH Codex and other provider transports do not gain the local Codex interaction protocol. A queued follow-up is not live steering. Full sandbox/approval policy configuration remains provider-specific. |
| Progress and activity | Structured plan steps/status, available token/context/cost metadata, source/artifact links, questions, tool activity and advertised subagent status. Streaming secrets are redacted across chunk boundaries. | Usage is shown only when provided. No universal provider/model capability discovery, explicit delegated-agent launch manager, full tool-output inspection suite, or durable agent desktop-attention inbox. |
| Git review | Actual bounded local status/diffs for staged, unstaged, untracked, deleted, renamed, binary and conflicted files. Diff line numbers, wrapping/pagination, turn-change cards and line feedback to the draft. Confirmed file staging/unstaging and commit plans revalidate exact state and use Git locks. | Local only; no SSH Git transport or hunk staging. Observed timing does not prove agent authorship. Undo requires exact tool-edit evidence, which current CLI traces generally lack, so those Undo controls remain unavailable. Comments are session feedback rather than hosted code-review comments. |
| Delivery | Branch/commit/origin/upstream/divergence/dirty-tree summary. Explicit GitHub PR/check/readiness fetch. Push preview and PR title/body/base form lead to a separate concrete confirmation. Push uses a captured SHA/destination; PR uses prepared `--body-file`, explicit head/repository, duplicate checks and post-create author/head/base verification. | Local only. Requires a complete review snapshot, named committed branch, one HTTPS/SSH origin and matching fetch/push destination. Multiple destinations, fork push URLs, noncanonical GitHub origins and very large/incomplete snapshots refuse. No force push, merge, automatic PR creation, or automatic remote mutation. |
| GitHub identity | Only local `gh`. `gh auth status` plus `gh api user` before publishing, actor-change refusal, and created-author verification. The ShellDeck repository applies its contributor allowlist; unrelated projects use their own account identity. | The UI does not manage account credentials or invent a remote executor. Git/SSH transport authentication still follows the user's configured tools. |
| Workspaces/tools | Discoverable Files, Changes, Terminal, Browser, Sources, Instructions, Library, Goals/tasks and Delivery panels. Local files open beside the conversation; bounded remote text preview is routed through the existing SSH target. Panels can be resized/hidden. Workspace launcher creates/selects branches/worktrees instead of a dirty checkout switch. | Project guidance inspection is local only. Source/tool metadata is limited to explicit context and provider-reported metadata. No plugin installation/MCP authentication or complete skill/tool discovery UI. |
| Browser | Explicit HTTP(S) Chromium capture under a deadline, fresh private profile, bounded rendered DOM/PNG, URL back/forward/reload, desktop/mobile size, DOM filtering, screenshot lightbox/annotation, and attachment of feedback/DOM/screenshots. | A snapshot preview, not an embedded live browser: no authenticated cookie continuity, forms/live clicks, console/network tools, element-anchored comments or continuous agent-controlled verification loop. Requires installed Chrome/Chromium. |
| Artifacts | Provider-reported source/artifact links, local editor/image previews and browser screenshot feedback. | No full versioned artifact gallery, rendered Word/PDF/spreadsheet workflow, document annotations or universal provenance store. |
| Goals/tasks | Objectives, verification criteria, notes, bounded turns/time, explicit pause/start, scoped completion marker and restart recovery. Read-only automatic continuation is opt-in and requires Start. Recurring tasks save paused, enable explicitly, coalesce missed runs and revalidate authority/context at dispatch. CLI readiness checks installation/version/auth without returning raw credentials. | The desktop app must remain open. No cloud scheduler or cross-device goal runtime. Each write-access run needs confirmation. Questions, approvals, errors and context/budget changes pause continuation. |
| Navigation and accessibility | Search, session/library controls, jump-latest, keyboard entry points, adjacent panels and pane-width-aware layout state. | Native keyboard/focus/short-pane acceptance remains open. No full timeline virtualization, global search-result highlighting/jump navigation, voice/dictation or voice conversation. |

## Delivery safety and fidelity

The delivery backend uses the same injected `ReviewExecutor` boundary as local
review. Unit tests do not invoke real `git push` or `gh pr create`. Preparation
may create a private local PR body file and perform bounded read-only network
checks; it cannot execute the preview's remote effect. A confirmation is scoped
to one plan, expires after five minutes and cannot be replayed.

Before effect, delivery re-reads the original/canonical workdir, repository
identity, branch, HEAD, exact snapshot/index and effective origin/URL-rewrite
configuration. Effective fetch/push destinations are read with `git remote
get-url --all`; differing or multiple destinations refuse. The captured SHA
and URL are passed as separate arguments without a shell or force flag. A PR
requires the published branch to match that SHA and no matching open PR, then
rechecks those conditions before creation. The returned URL, repository,
branch, author, base and commit are verified. A timeout/failure after dispatch
is reported as potentially accepted remotely and requires a status refresh,
without an automatic retry. Delivery itself does not stage, commit, checkout,
stash, reset or modify working files.

This does not turn GitHub into an atomic compare-and-swap PR API. A remote
branch can still move concurrently; returned PR identity is checked and a
mismatch is surfaced. Current review budgets (including 128 MiB of retained
file content and 20,000 files) can make delivery unavailable on large checkouts.
The UI must state that refusal rather than treat an incomplete snapshot as a
clean repository.

## Source map

| Contract | Primary implementation |
|---|---|
| SDUC-550 | [agent_session.rs](../../crates/shelldeck-core/src/agent_session.rs), [agent_session_storage.rs](../../crates/shelldeck-core/src/agent_session_storage.rs), [library.rs](../../crates/shelldeck-ui/src/agent_console_view/library.rs) |
| SDUC-551 | [composer.rs](../../crates/shelldeck-ui/src/agent_console_view/composer.rs), [model.rs](../../crates/shelldeck-ui/src/agent_console_view/model.rs), [model_discovery.rs](../../crates/shelldeck-core/src/agent_codex/model_discovery.rs), [progress.rs](../../crates/shelldeck-ui/src/agent_console_view/progress.rs), [tools.rs](../../crates/shelldeck-ui/src/agent_console_view/tools.rs), [agent_runtime_stream.rs](../../crates/shelldeck-core/src/agent_runtime_stream.rs), [agent_runtime_protocol.rs](../../crates/shelldeck-core/src/agent_runtime_protocol.rs) |
| SDUC-552 | [agent_review.rs](../../crates/shelldeck-core/src/agent_review.rs), [review.rs](../../crates/shelldeck-ui/src/agent_console_view/review.rs) |
| SDUC-553 | [agent_browser.rs](../../crates/shelldeck-core/src/agent_browser.rs), [browser.rs](../../crates/shelldeck-ui/src/agent_console_view/browser.rs) |
| SDUC-554 | [agent_codex.rs](../../crates/shelldeck-core/src/agent_codex.rs), [Codex tests](../../crates/shelldeck-core/src/agent_codex/tests.rs), [workspace/agents.rs](../../crates/shelldeck-ui/src/workspace/agents.rs) |
| SDUC-555 | [agent_workflows.rs](../../crates/shelldeck-core/src/agent_workflows.rs), [workflows.rs](../../crates/shelldeck-ui/src/agent_console_view/workflows.rs) |
| SDUC-556 | [agent_delivery.rs](../../crates/shelldeck-core/src/agent_delivery.rs), [delivery.rs](../../crates/shelldeck-ui/src/agent_console_view/delivery.rs) |

French and English UI strings live in the canonical core locales, including
39 matching `agents.delivery.*` keys with identical interpolation variables.
Raw provider/network diagnostics and user-entered content remain unlocalized.

## Verification record

Final isolated-profile execution on Linux (pinned toolchain, locked dependencies):

- `cargo test --locked -p shelldeck-core -p shelldeck-ui --lib`, followed by
  `cargo test --locked -p shelldeck-core --lib` after the checkout-size fix:
  **474 core passed, six existing live tests ignored; 216 UI passed, three
  baseline UI failures**. All added history, stream/protocol, review, browser,
  delivery, live-input, mapped-checkout, queue and model-catalog tests passed.
- The three failures are SDTEST-1763, SDTEST-1764 and SDTEST-1770 in the unchanged
  native worktree lifecycle. All three reproduce in the pre-implementation
  test executable `shelldeck_ui-e0cbdc52f07561f4`, using another disposable XDG
  profile, with the same worktree-authority refusal. They are not hidden or
  marked passing; the full suite therefore still exits unsuccessfully.
- `cargo check --locked -p shelldeck` and `cargo build --locked -p shelldeck`
  passed for the integrated application; the final executable includes the
  checkout-size limits.
- Local installed Codex app-server discovery returned **seven advertised
  models** using only initialization and `model/list`. No inference, thread or
  turn was started. The returned catalog is not an entitlement guarantee.
- A real localhost Chromium capture verified JavaScript-rendered DOM and PNG
  output. Browser unit tests use an injected executor; Git delivery tests never
  execute a live push or create a real PR.
- French/English keys and placeholders, embedded icon availability, formatting
  and `git diff --check` were checked. No account configuration, toolchain,
  lockfile, release tag or live remote was changed by this parity work.

Logs are retained under `/tmp/shelldeck-parity-final-tests.log`,
`/tmp/shelldeck-parity-core-final.log` and
`/tmp/shelldeck-parity-baseline-tests.log`. Per-test inventories remain in
[core](tests-core.md) and [UI/app](tests-ui-and-app.md). Compilation and static
screenshots do not certify live native inference or other platforms.

## Remaining acceptance and feature work

Before calling this production-ready, perform the native journeys recorded as
SDTEST-2480..2483: recovery and target switching, image/queue editing and focus,
short/split pane layout, review/confirmation freshness, real installed local
Codex questions/approvals/steering/Stop, Chromium capture and local goal/task
lifecycle. Real push/PR acceptance requires a separately authorized throwaway
remote and must preserve unrelated dirty work. macOS/Windows builds and native
platform behavior remain release gates.

Broader feature parity still needs an interactive authenticated browser,
remote Git/instruction support, durable workspace layouts, agent-owned undo
proof, cross-provider model discovery, tool/skill discovery, complete artifact/document handling, voice,
cloud scheduling/continuation, hosted sharing, and deeper review/attention and
conversation navigation workflows. These are missing features, not implied
by the successful offline tests.

## Reference checks

The screenshot comparison is grounded in the current official desktop docs:
[Browser](https://learn.chatgpt.com/docs/browser) describes an interactive shared
browser and signed-in browser profiles; ShellDeck's snapshot capture does not
satisfy that contract. [Models](https://learn.chatgpt.com/docs/models) describes
the composer model/reasoning selector. The implementation uses the installed
CLI's [app-server model/list contract](https://learn.chatgpt.com/docs/app-server)
for advertised local Codex choices, without hard-coding account availability.
Other providers retain an explicit override field. A model catalog is not proof
that an inference request is entitled or will succeed.

## Reproducible native preview

`crates/shelldeck-ui/examples/agent_console_preview.rs` seeds a disposable
conversation library and opens the real Agents view without launching a provider,
SSH connection, terminal process, or GitHub action. It requires all three XDG
homes under the system temporary directory and closes itself after 45 seconds.
`SHELLDECK_PREVIEW_WIDTH=700` exercises the narrow layout;
`SHELLDECK_PREVIEW_PANEL=1` starts with Files visible. Run it on a desktop with
DRI3 presentation support; Xvfb cannot present this GPUI Vulkan surface.

```sh
shelldeck_preview_profile=$(mktemp -d -t shelldeck-agent-preview.XXXXXX)
XDG_CONFIG_HOME="$shelldeck_preview_profile/config" \
XDG_DATA_HOME="$shelldeck_preview_profile/data" \
XDG_CACHE_HOME="$shelldeck_preview_profile/cache" \
PKG_CONFIG_PATH=/usr/lib/x86_64-linux-gnu/pkgconfig \
cargo run --locked -p shelldeck-ui --example agent_console_preview
```

The initial 1280×900 and 700×900 captures verified a visible composer and a
single transcript scroll area. Review found and corrected toolbar wrapping,
unread recovery while visible, narrow history access, and explicit execution
scope labels. This fixture does not certify live inference, authenticated
browser workflows, delivery, or platform-specific native behavior.

Final native fixture captures: [desktop](agent-parity-preview-wide.png) and
[narrow](agent-parity-preview-narrow.png). Narrow Library opened with search,
conversation selection and management controls; its close button worked. Files
then opened in the same overlay with its own close control. The host, workdir
and permission scope stayed visible, and the composer remained outside the
transcript scroll region. Preview windows and their Xvfb probe were closed after
capture. These are fixture-only interaction checks, leaving the broader
SDTEST-2480..2483 journeys open.

The real ShellDeck checkout was also observed with `agent_review_probe`, without
Git writes or network access: **1,502 files, 72 status/diff entries, complete
snapshot, zero refusals**. The initial 2 MiB/file and 64 MiB total defaults could
not represent this checkout (about 78 MiB tracked, including a 5.5 MiB asset).
Default limits are now **8 MiB/file and 128 MiB total**, with the same incomplete
snapshot refusal above those bounds. SDTEST-2494 verifies that retaining a large
asset does not weaken stale-state refusal before an index mutation.
