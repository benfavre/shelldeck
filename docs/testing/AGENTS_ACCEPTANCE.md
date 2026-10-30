# Agents workflow acceptance

This is the completion gate for the user's Agents experience, not a list of
features to copy wholesale. Existing ShellDeck/Automonique ownership, native SSH,
permission boundaries and local work remain authoritative.

Reference reviewed: [T3 Code](https://github.com/pingdotgg/t3code/tree/6575ce32508bd7908444241f3719aa33aaf77f90)
(snapshot 2026-10-30), particularly `docs/user/thread-sidebar.md`,
`docs/user/composer.md`, `docs/user/remote-access.md` and their implementation.
The user's supplied screenshots also define the intended compact layout,
project/session sidebar, composer, and browser beside the conversation.

## Required journeys

| Journey | Acceptance conditions | Evidence / remaining gate |
| --- | --- | --- |
| Open the app | Agents is the default; Agents is second in the full shell rail; shell toggle works in Agents mode; resizing retains navigation and composer. | Source reviewed and native fixture clicked: Agents startup, second activity after Connections (below brand), sidebar toggle, 900px resize; composer remains usable. |
| Add/open a project | Local folder picker or saved SSH connection plus remote folder browser; standard SSH folders require no helper install; successful add lands in Agents; Cancel returns without losing draft. | Native fixture clicked: Add project → local folder → Agents; canonical re-add retains one project/name and selects existing chat. SSH browsing covered by earlier onboarding work; not repeated against a live host in this pass. |
| Start another task | New conversation has explicit project, server, agent and model; existing project sessions are selectable; archived sessions do not reopen implicitly; nested checkouts do not duplicate rows. | Navigator/project-picker corrections and regression coverage in this pass. |
| Manage the list | Search matches project/server/session; collapsed groups remain manageable; active/archived state is clear; archive and restore have distinct actions; hiding an SSH list does not delete server history. | Native fixture clicked: archive → archived filter → restore retains unsent draft; pins above folders, hidden attention and current-row retention, provider-mixed SSH folders and temporary search reveal. SDTEST-2859–2861 cover exact folder scope, pin/unpin, paging, drafts and legacy collapse migration. SSH page/hide state regressions remain. No implied parity with T3 snoozing/drag ordering. |
| Discover remote work | Automonique and native SSH are clearly separate and grouped; selecting a saved SSH server loads fresh metadata; last successful fetch and failures are visible; large histories and older inventory pages remain accessible. | Paged inventory and large-file fixtures plus live read-only verification. Candidate/time/output bounds must remain explicit. |
| Reconnect and read | Exact server/provider/native ID/directory; full provider-retained history is reachable in pages; reading never runs tools; source reference survives restart; explicit revalidation restores resume authority. | SDUC-562/563 parser/storage/UI regressions and native reconnect walkthrough. Deleted/compacted server content cannot be recreated. |
| Continue after external changes | Refresh can read current server history without duplicating retained local messages; compaction recovery preserves draft and resume identity; late responses cannot switch conversations. | SDTEST-2818 and private refreshed-history/back-to-chat walkthrough. |
| Compose and choose a model | Context is visible, controls retain valid draft choices; availability/loading/error states are honest; model options come from actual provider support; failed attachment intake leaves a recoverable draft. | Composer/model/context tests in this pass; live provider catalog claims require a real provider read. |
| Send, work, stop, retry | Transcript remains ordered, no transport lifecycle spam; draft/queue survives failed admission; tool/approval activity remains usable; stopping is distinct from failure; background streams do not steal focus. | Core/UI runtime and queue tests plus controlled native fixture. A fixture is not evidence of a real provider turn. |
| Inspect the work | Files/Changes/Terminal actions are coherent; browser is a companion pane; no duplicate toolbars/errors; unavailable capabilities explain an actionable next step. | Earlier inspector/browser walkthrough; current source regression review. Live Chromium and provider/browser control require a working helper, not a screenshot fixture. |
| Recover account/network failures | Errors are local to the failed operation; last known list/catalog is retained; refresh is meaningful; logout cancels/invalidate pending reads; no silent new chat on failed native resume. | Request identity/epoch/destination tests and error-state walkthrough. |
| Reopen after restart | Local text and drafts preserved; saved SSH choice can refresh once visible; collapse choice survives; native history reads passively; original conversation resumes only after revalidation. | Automated preference/source round-trips; private full-app restart retained selected project, unsent draft and sidebar visibility. SSH auto-refresh-on-restore is state-tested; resume revalidation remains explicit. |

## Completion rule

For each touched journey: inspect the reference behavior, identify the gap, fix
its root cause, exercise the success path and at least one relevant recovery
path, then record concrete evidence. Test counts alone do not satisfy a journey.
A reference screenshot is not proof that an interaction works. Distinguish:
**source reviewed**, **automated regression**, **native fixture clicked**, and
**live verified**. Do not mark an entire app or provider integration complete
because one layer passed. Record unverified gates before handing work back.

Run UI tests with isolated XDG directories. Private native fixtures must not
reuse the real profile or submit production changes. Preserve existing local
changes and provider histories. New user examples add regressions to the same
journey, rather than replacing this acceptance list with another one-off fix.

## Evidence recorded 2026-10-30

- **Automated regression:** 302 UI library tests passed on the final source, including
  SDTEST-2819–2822 and 2826–2829. Core remote discovery/history and preference
  checks passed; paged Python fixtures cover 451 sessions, concurrent new/updated
  files, foreign cursors, malformed input and large Jcode metadata. Core/UI strict
  all-target Clippy passed (existing patched-dependency warnings remain). Desktop
  and preview builds passed; EN/FR key and placeholder parity passed.
- **Native fixture clicked:** the real desktop binary ran with a disposable
  profile, hidden real home and isolated network against a local account stub.
  Project creation/reopen, archive/restore, project search, sidebar toggle, resize
  and app restart were exercised without sending a provider prompt. Screenshots:
  `/tmp/shelldeck-acceptance-qa/07-project-agents.png`,
  `09-reopened-project.png`, `10-archived.png`, `11-restored-search.png`,
  `12-shell-toggle-narrow.png`, `17-final-sidebar.png`, `18-help-narrow.png`.
  Setup/sign-in instructions now expand on demand; unassigned chats have their
  own collapsible group. The provider-free preview also clicked inventory
  continuation (Claude count 1 → 2), native reconnect, retained history paging
  (97–160 → 33–96 of 160) and Hide server while preserving the imported chat.
  Evidence: `/tmp/shelldeck-acceptance-preview/02-older-inventory.png`,
  `04-history.png`, `05-older-history-hidden-server.png`. Preview typography
  lacks the full app's loaded fonts; use the full-binary captures for visual QA.
- **Live verified, read-only:** the actual SSH native inventory returned 5,261
  unique conversations across 15 pages, zero duplicates and zero omission
  warnings. The 57,846,771-byte Jcode conversation appeared on page 13. This
  replaces the previous scan's 200-per-provider terminal cap and 8 MiB Jcode
  omission. Paging still enforces documented filesystem/time/output bounds and
  reports real omissions. No provider was launched or history modified.

### Explicit remaining gates

This pass does not certify every app feature or platform. Live generation, tool
approval, cancellation and reconnect-send must be exercised per supported
provider before a provider-wide release claim. The model catalog is discovered
for local Codex; other contexts retain Automatic/custom model entry and must not
claim inferred reasoning support. Live Automonique runtime control and Chromium
agent interaction were not re-executed in this pass. Windows/macOS packaging and
Linux desktop portals require their own native checks. The preceding browser and
transcript audits retain their separately recorded evidence.

## T3-first follow-up

[T3_CODE_REVIEW_2026-10-30.md](T3_CODE_REVIEW_2026-10-30.md) records the reference
source and actual release walkthrough, the remaining gaps, and the deliberate
exceptions (ordinary SSH, full retained native history, conservative permissions).
The first follow-up implements stable navigation order, attention-aware collapsed
groups and explicit imported-conversation context transitions. Do not mistake
that slice for complete parity with the reference.

### Second T3 parity pass

- Completed tool activity and plans now fold inline without dropping trace
  payloads or hiding pending work, failures or recovery controls. Inspector
  navigation reveals its exact trace rather than scrolling to the chat end.
- Sidebar actions support rename, pin, duplicate, copy/export and archive with
  Undo. Native clicks verified both menu entry points, rename with Enter and
  archive/Undo. Background actions preserve the selected conversation's draft.
- Draft bundles retain context, effort, attachments and paused follow-ups across
  restart. Restored images require verification and restored follow-ups never
  execute automatically. Automated regressions cover these recovery contracts.
- Validation: 24 core session/storage tests and 311 UI library tests passed;
  final presentation fixes passed strict all-target core/UI Clippy and desktop
  and preview builds. EN/FR key/placeholder parity and whitespace checks passed.
  Native dark/light and 900-pixel-wide renders caught and resolved message action
  clipping and an invisible tooltip blocking the first row's ellipsis menu.
  Final capture: `/tmp/shelldeck-parity-render/verified-draft-menu.png`.

No live provider prompt was submitted for this pass. The remaining gates above
still apply; the T3 review tracks remaining parity work. The real running app
was left untouched while the isolated preview was tested.

### Encoded transcript follow-up — 2026-10-30

- **Source reviewed:** the pinned T3 `TimelineWorkEntryRow.tsx` initializes tool
  disclosure collapsed and presents plain output in monospace. Native retained
  history now follows that presentation while keeping the conversation as its
  sole vertical scroll owner. Long base64/hex runs fold behind a size notice in
  both native history and locally retained messages/trace output. Reveal/hide and
  copy-original controls operate on presentation/original text respectively;
  snapshot hashes, stored entries and export text are unchanged.
- **Automated regression:** all 313 isolated-profile UI tests passed, including
  SDTEST-2842/2843. All five native-history Python fixtures passed. UI compilation,
  strict all-target UI Clippy, preview build, FR/EN key/placeholder parity and
  whitespace checks passed.
- **Native fixture clicked:** `SHELLDECK_PREVIEW_BLOBS=1` reproduces an encoded
  image diff and an inline data URI without a provider. On the desktop with a
  disposable XDG profile, hidden real home and disabled network, verified initial
  tool collapse, expanded literal output with the data folded, explicit original
  reveal, hide again, and a usable composer at 900×700 in Dark and Solarized Light.
  Captures: `/tmp/shelldeck-transcript-qa/04-history.png`, `05-tool-folded.png`,
  `06-original-visible.png`, `07-rehidden.png`, `08-narrow.png`, and
  `11-light-tool.png`. Xvfb's black rendering could not validate this fixture;
  the final captures use the desktop display and the same isolated test profile.
  Clipboard copying was source-reviewed; the native walkthrough exercised the
  reveal/hide controls. No live SSH history or provider turn was exercised here.

### T3-inspired test-suite increment — 2026-10-30

The test-source comparison at upstream `9b1d351` is recorded in
`T3_CODE_REVIEW_2026-10-30.md` (§ Test-suite follow-up). SDTEST-2844–2847 add
production-handler scenarios with a private clipboard, source/page replacement,
1,000-event tool replays for all four provider choices, durable failure ordering,
and a wrapping/streaming/Unicode corpus. The corpus reproduced a short wrapped
base64 tail leaking into display; the final rule includes that tail without
consuming the readable prose after the block. Stale copy/reveal callbacks now
resolve their exact retained source before changing presentation or clipboard.

All 317 isolated-profile UI tests, all three Python discovery/history suites,
strict all-target UI Clippy and the preview build passed locally. CI now has an
independent `agent-history` job for those reader suites; no remote CI run is
claimed. Handler tests execute the same methods bound by the controls, without
rendering GPUI or writing the desktop clipboard. The subsequent rendered
interaction increment below covers actual control dispatch and layout. The prior
native screenshots retain their narrower manual-fixture meaning.

The rebuilt native preview also rechecked the updated source-bound reveal/hide
callbacks on the desktop at 900×700 with a disposable profile, hidden real home
and disabled network. The inline native attachment revealed its complete text,
hid it again, and kept the composer accessible. Evidence:
`/tmp/shelldeck-t3-native-qa/03-reveal.png` and `04-hide.png`. Exact clipboard
content and stale-action rejection were exercised through the private test
clipboard by SDTEST-2844/2845; the desktop clipboard was not used.


### Rendered interaction suite — 2026-10-30

SDTEST-1713 and SDTEST-2848–2854 use one shared GPUI harness around the production
cockpit. The suite dispatches actual clicks, keyboard input, Enter and wheel
events, checks emitted requests and drafts, and measures rendered bounds. The
reveal/copy matrix covers local and native transcripts at 1280×900, 900×700 and
620×560 in Dark and Solarized Light. Other journeys cover conversation focus,
streaming while reading older content, Jump to latest, footer retry and paging,
once-only queued admission, durable draft reload and pending question/approval
responses. Clipboard tests use GPUI's private clipboard.

The tests reproduced and drove fixes for Send/Stop sizing (both now 28 px),
new/restored composer focus, footer Retry visibility during loading/error layout
changes, and duplicate approval decisions. Approval controls remain disabled
until the exact interaction resolves or its transport rejects the submission;
uncorrelated failures cannot release a pending decision. Restart tests use their
own durable stores and never execute a restored draft.

This layer exercises real layout and input dispatch on GPUI's test platform.
Native GPU paint evidence and real provider runs remain separate: the suite does
not claim to test external processes or the broader split-pane/restart journey
SDTEST-2480/2482. CI runs it by default with serialized tests and disposable XDG
roots, alongside the independent Python reader suites.

Validation: `cargo test --workspace --locked --offline -- --test-threads=1`
passed 1,130 tests (325 UI); nine existing live/doc tests were ignored. All
three Python reader/discovery suites passed. Strict workspace all-target Clippy,
formatting, translation key/placeholder parity and whitespace checks passed.
The native preview was exercised on the real desktop with a disposable profile,
hidden real home and disabled network. Tool expansion, original reveal/hide and
fixed composer containment were checked at 900×700 and 620×560 in Dark, plus
folded tool/attachment rendering in Solarized Light at both sizes. Evidence:
`/tmp/shelldeck-rendered-suite-qa/02-folded.png`, `03-tool-expanded.png`,
`04-raw.png`, `05-narrow.png`, `06-light.png`, `07-light-narrow.png`.
The desktop clipboard and real providers were not used. These screenshots show
native GPU paint; SDTEST-2848–2854 separately verify layout and event behavior.


### Release follow-up — current-chat focus and missing SSH recovery

SDTEST-2855 reproduced typing staying in navigator search after selecting the
already-open chat. Selection now requests surface focus even when its session ID
is unchanged, without resetting unsent context choices. SDTEST-2738 uses the
rendered project/reconnect pickers, actual search input and commit controls
across EN/FR, Dark/Solarized Light and 900×700/620×560. Missing targets block
click/Enter execution while preserving editable multiline drafts; chat switching
retains separate local/remote drafts; no-match searches recover; explicit repair
admits exactly one request for the selected server and original remote folder.
The harness records the admitted request without running SSH or a provider.
Its locale is restored on drop, including after a failed assertion.

The follow-up workspace sweep passed 1,132 tests, including 327 UI tests; the
same nine existing live/doc tests remain ignored. Strict workspace all-target
Clippy passed. The French Solarized Light preview at 900×700 showed the retained
remote draft, disabled Send, both project and Reconnect SSH controls, and exact
folder `/srv/customer project` (`/tmp/shelldeck-release-qa/02-missing-selected.png`).
This walkthrough also exposed an overflowing older-conversations sidebar label;
its visible text is now shorter in both languages, with the full action in a
tooltip. The preview supports `SHELLDECK_PREVIEW_MISSING_SSH=1` and
`SHELLDECK_PREVIEW_FR=1` for repeating this case without SSH/provider access.

### Screenshot follow-up — native titles and cropped command cards

SDTEST-2856 reproduced `agent session name is too long` with the screenshot's
240-byte French title before the correction. Discovery accepts longer titles
than local session names. Native import now bounds only the display label to
160 bytes with a UTF-8-safe ellipsis; manual naming and control-character checks
remain strict. The matrix covers all four provider variants, multibyte and
boundary-length titles, saved transcript/rename preservation, exact resume
authority and distinct conversations sharing a display prefix. SDTEST-2857
clicks the production reconnect button for all three native formats at 900×700
and 620×560, loads a validated history page, reuses the same chat and preserves
both local and native drafts without emitting Run.

Technical trace cards, expanded completed-work groups, transcript bodies and
run details now explicitly retain their natural height. A command icon cannot
shrink, and wrapped titles have an explicit full-width header and line height.
SDTEST-2858 covers Dark/Solarized Light at 900×700, 620×560 and 900×400 with and
without multiline Unicode output. It checks complete child bounds, separation
from the preparing indicator and run details, real wheel reachability and a
fixed composer with completed work collapsed/expanded. The initial Linux
fixture and simple headless geometry did not reproduce the exact compressed
Windows card from the user screenshot; this is layout hardening with regression
coverage, not a claim of a Windows GPU reproduction.

The wheel harness waits for GPUI's 70 ms smooth-scroll animation, which uses
`std::time::Instant` rather than the test dispatcher clock. Checking geometry
before that animation settled caused an intermittent offscreen-click failure;
the harness now moves the pointer and waits 80 ms before measuring. No assertion
was widened and no production scrolling code was changed.

Native Linux GPU walkthroughs used fresh isolated XDG profiles, a masked real
home and disabled networking. The synthetic browser traces did not execute a
browser, SSH command or provider. Dark at 900×700 shows the complete warning,
preparation indicator and run details (`/tmp/shelldeck-chat-qa/03-warning-dark.png`);
900×400 keeps the complete warning reachable by wheel while the composer stays
fixed (`06-short-scrolled-dark.png`). Solarized Light at 620×560 covers both
collapsed and expanded completed work
(`/tmp/shelldeck-chat-light-qa/03-warning-light.png`, `04-expanded-light.png`).
The full-title native preview reconnects into retained history at 1280×900
(`/tmp/shelldeck-long-title-qa/02-preview.png`, `03-reconnected.png`). Repeat with
`SHELLDECK_PREVIEW_CROPPED_TRACE=1` or
`SHELLDECK_PREVIEW_SSH=1 SHELLDECK_PREVIEW_LONG_TITLE=1`; add
`SHELLDECK_PREVIEW_FR=1` and optionally `SHELLDECK_PREVIEW_LIGHT=1`.

Final isolated offline workspace sweep: 1,135 tests passed, including 510 core
and 329 UI tests; the same nine existing live/doc tests remain ignored. Strict
workspace all-target Clippy, formatting and whitespace checks passed. Logs:
`/tmp/shelldeck-chat-final-tests.log`, `/tmp/shelldeck-chat-final-clippy.log`.

### Sidebar grouping follow-up — 2026-10-31

Source review of T3 Code's optional project-folder sidebar at `cb9629e` is in
`T3_CODE_REVIEW_2026-10-30.md`. ShellDeck now keeps active pins above catalog
projects, uses exact execution target/workdir folders for uncatalogued chats,
and groups native SSH discovery by server/folder across providers. Catalog
ownership still uses the most specific matching checkout. Group membership
never depends on display names, provider binaries or hashed preference keys.
Collapsed groups retain the current chat/native preview; their badges and
attention describe the other hidden rows. Native server actions are compact
icon controls, and each native folder owns its own visible-row limit.

SDTEST-2859 covers exact target/path scope, identical server/folder labels,
cross-provider grouping, pin/catalog exclusion and stable ordering during
streaming. SDTEST-2860 dispatches real pin/unpin, collapse, search and navigation
controls in EN/Dark and FR/Solarized Light at 900×700 and 620×560. It flushes
separate drafts to the real isolated store and restores a legacy closed catch-all
into closed folder groups without losing the selected chat. SDTEST-2861 clicks
mixed-provider discovery folders, independently grows two folders, retains the
current preview when its folder/server closes, and searches across server/title
terms without mutating saved collapse. All callbacks are fixture-only; neither
Run nor native reconnect execution is admitted by these grouping scenarios.
Existing SDTEST-2744 additionally persists the bounded folder/server/pin keys.

Native Linux GPU walkthrough used fresh XDG profiles, a masked home, no network,
providers or desktop clipboard. The old layout is captured at
`/tmp/shelldeck-sidebar-before/01-sidebar.png`. Dark 1280×900 evidence:
`/tmp/shelldeck-sidebar-qa/01-grouped.png` (pins, catalog and uncatalogued folders),
`02-hidden-attention.png` (closed project with hidden approval),
`03-current-in-closed-project.png` (current chat retained),
`05-native-folder-closed.png` (selected native preview retained), and
`06-search-reveals.png` / `07-cleared-search.png` (temporary reveal and restored
collapse). Solarized Light 620×560 evidence:
`/tmp/shelldeck-sidebar-light-qa/02-narrow-sidebar.png` and
`03-narrow-native-folders.png`; the overlay scrolls independently and the
underlying composer keeps its width. The standalone fixture uses its existing
Ctrl+Shift+B session toggle. Repeat with
`SHELLDECK_PREVIEW_SSH=1 SHELLDECK_PREVIEW_GROUPS=1`; add
`SHELLDECK_PREVIEW_ATTENTION=1`, `SHELLDECK_PREVIEW_FR=1` or
`SHELLDECK_PREVIEW_LIGHT=1` as needed. This is native ShellDeck fixture evidence,
not a new T3 runtime or live-provider walkthrough.

Validation: the isolated offline workspace sweep passed 1,138 tests, including
332 UI tests, with the same nine existing live/doc ignores. The final focused
folder run passed all ten selected scenarios after the migration, source-selection
and visible-count refinements. Strict workspace all-target Clippy and formatting
passed. Logs: `/tmp/shelldeck-sidebar-final-tests.log`,
`/tmp/shelldeck-sidebar-reviewed-tests.log`, `/tmp/shelldeck-sidebar-final-lint.log`.
