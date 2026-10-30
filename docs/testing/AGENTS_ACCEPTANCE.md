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
| Manage the list | Search matches project/server/session; collapsed groups remain manageable; active/archived state is clear; archive and restore have distinct actions; hiding an SSH list does not delete server history. | Native fixture clicked: archive → archived filter → restore retains unsent draft; project search excludes unrelated chats; Other conversations collapses separately. SSH page/hide state regressions. No implied parity with T3 snoozing/drag ordering. |
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
