# ShellDeck feature audit — 2026-10-30

Scope: the native ShellDeck application, its seven workspace crates, and the
existing packaging/CLI contracts. This review builds on the Agents navigator
and startup fixes already present in the working tree. It is not a claim that
every provider, operating system, or production workflow has been exercised.

## Corrections

| Area | Defect and resulting behavior | Regression evidence |
| --- | --- | --- |
| Account permissions | A `whoami` response without display fields could retain obsolete capabilities. Display-name fallback now preserves only display text; server roles and capabilities always replace the previous values. | SDTEST-2750 |
| Authentication | Redirects could replay authentication requests. The account client refuses redirects, including password-preserving 307/308 responses and bearer redirects. | SDTEST-2751 |
| Cloud profile merge | A remote UUID collision could convert a local/manual profile into cloud-owned data eligible for later removal. Local ownership now survives collisions and subsequent cloud pruning. | SDTEST-2752 |
| Cloud payload validation | A missing connections field could be interpreted as an authoritative empty list. Only an explicit valid collection can reconcile cloud profiles. | SDTEST-2753 |
| Support and request drafts | A late send completion could erase text or attachments edited after submission. Completion clears only the matching, unchanged submitted draft. | SDTEST-2754 |
| SSH execution | Output EOF could arrive before the command's exit status, and a cancelled operation could still dispatch after waiting for the shared handle. Status is retained after EOF; cancellation gates dispatch. | SDTEST-2760–2761 |
| Script execution | Old pollers could update a replacement run; missing remote status looked successful; cancellation while connecting and worker-start failure could leave runs unsettled. Execution ownership, cancellation, and failure completion now remain consistent. | SDTEST-2762–2765 |
| Local script children | Cancelling only the shell left descendants running or holding output pipes open. Script workers now use the existing owned process-tree supervision for cancellation and completion. | SDTEST-2766–2768; Windows case pending native execution |
| Final script output | Completion could arrive between draining output and checking the result, dropping the last lines. Both pollers now observe completion before the final output drain. | SDTEST-2769 |
| Remote session identity | Sessions sharing an ID across authorities could share selection/drafts or target the wrong retained pane. Navigation and controls now use the complete session coordinate; ambiguous ID-only navigation is refused. | SDTEST-2770 |
| Remote follow-ups | A late acceptance could clear a different session's draft or newer unsent edits. It now clears only the exact unchanged submission. | SDTEST-2771 |
| Browser evidence | Saved context could authorize sharing a screenshot from a different displayed draft context. Sharing now checks the selected session, displayed context, and source instance. | SDTEST-2772 |
| Updater lifecycle | Checks/downloads/installations could overlap or stale work could install after disabling updates. Operations are serialized and generation-checked; an installation already swapping files may finish. | SDTEST-2780 |
| Update downloads | Interrupted downloads could leave partial files; a zero-sized payload bypassed size bounds. Owned temporary files clean themselves up, existing destinations survive failures, and signed size must be positive. | SDTEST-2781 |
| Update platform | A correctly signed manifest for a different platform could pass signature validation. Platform identity is now checked before accepting it. | SDTEST-2782 |
| Window chrome | Narrow windows clipped Terminal/Help. A compact scrolling menu uses the same authorized command inventory and dispatcher, opens at the top, and retains manual scrolling. English mode labels no longer display the French `Utilisateur`. | SDTEST-2785–2786; native narrow-window smoke |
| About navigation | The About menu command opened General settings. It now selects the About page before opening Settings. | Native command smoke; shared settings navigation |

The existing Agents improvements are retained: independently collapsible project
groups, a collapsed-by-default remote section, persisted expansion choices,
search without rewriting those choices, bounded remote pages, attachable-session
filtering, readable fallback labels, and explicit directory-only sessions.
Unavailable observation is not presented as a working chat session.

## Review coverage

| Feature group | Checks performed | Practical limit |
| --- | --- | --- |
| Startup, navigation, settings, scaling and localization | Native Dev/Support/User surfaces in an isolated X11 profile; wide and narrow settings; English/French key and placeholder parity; menu capability tests. | Actual Wayland portal, OS tray/autostart, and global shortcuts remain desktop-dependent. |
| Connections, SSH, terminals and tunnels | Connection/settings UI; complete SSH and terminal suites, including in-memory SSH, cancellation, parsing and tunnel contracts. | No production SSH sessions or tunnels opened by this audit. |
| Scripts and history | Script UI and complete runtime suite; disposable local shell/child-process cancellation and completion checks. | Remote descendant handling depends on the SSH server; intentionally detached daemons are not claimed as supervised. |
| Projects and workspaces | Onboarding UI and existing creation, repair, retained-surface and lifecycle tests. | No production checkout, remote worktree or project data mutated. |
| Agents, Fleet, tasks and review | Full core/UI suites plus source review of session identity, draft ownership, provider boundaries, review operations and remote directory handling. | Live paid provider execution and task side effects were not triggered. |
| Browser and file editor | Browser IPC/context tests; packaging tests; source review of editor load/save fencing and retained file context. | Bundled native CEF runtime was not available for its live helper smoke; no claim of live browser coverage. No standalone SFTP transfer manager exists in these crates. |
| Account, cloud sync and modes | HTTP contract tests, role/capability refresh review, collision/malformed-payload regressions, synthetic Manage account fixtures. | Fixtures do not establish production service availability. |
| Support, requests and user sites | Synthetic populated lists and User home/sites/requests/account UI; complete API and draft lifecycle tests. | No real ticket/reply submitted or customer record changed. |
| Server sync, detected sites and recent activity | Native empty-state/navigation checks and existing contracts. | No rsync, remote scan or destructive server operation run. |
| Updates, CLI and packaging | Signature/platform/size/lifecycle tests; loopback interrupted download; CLI and packaging contract scripts. | No actual self-replacement of the user's installed app; NSIS-dependent case requires its toolchain. |

## Validation record

The final workspace suite passed **1,068 distinct tests**: main 102, browser 13,
core 493, SSH 69, terminal 94, UI 279, updater 18. Two child-process test result
lines in the core log are not double-counted. Seven opt-in live tests and two
documentation examples remain ignored. The patched UI dependency's compact-menu
regression passed separately.

The Linux development build and both CLI launch-contract tests passed.
All-target Clippy with `-D warnings`, formatting, and diff whitespace checks
passed. Existing warnings inside patched dependencies remain visible. Packaging
contracts passed: Agents 3 (one NSIS-dependent skip), browser 4, private
dependency access 3. Translation checks found no missing literal keys among
2,228 references, no English/French key mismatch, and no placeholder mismatch.

Native screenshots cover Dev navigation/settings and populated Support/User
surfaces at 1400×940, with settings/request details and the compact menu at
620×760. The menu starts at File, scrolls through Terminal to Help, and preserves
its scroll position. Artifacts and logs are under
`/tmp/shelldeck-feature-audit-20261030/` and `/tmp/shelldeck-audit-*.log` on the
audit machine; they are temporary local evidence, not shipped application data.

Test profiles use disposable absolute `XDG_CONFIG_HOME`, `XDG_DATA_HOME` and `XDG_CACHE_HOME`.
Native fixtures additionally use a private network/mount namespace and synthetic
Manage data, without access to the user's profile or credential service.

Windows and macOS builds/native workflows still need their CI runners. In
particular SDTEST-2768 is intentionally not marked passing from Linux. Existing
opt-in live tests and ignored documentation examples remain unexecuted. The test
inventories retain their outstanding gaps rather than treating this audit as
blanket certification.

This audit does not publish a release or change the version number.

## Follow-up interaction and UX review

A second pass used actual mouse/keyboard input in the isolated native app,
including a saved local fixture project. Evidence is retained in
`/tmp/shelldeck-ux-review-20261030/`.

Corrections from this pass:

- Split Add project from New workspace. The project picker always opens project
  registration even when a checkout already exists; Workspaces is highlighted
  while the form is open, and Cancel returns to the prior Agents conversation.
- Use consistent Add project wording. New project conversations use New session
  rather than repeating the project name as their initial title.
- Replace the Scripts search imitation with the shared input, including normal
  caret/selection/editing and a clear button. Keep search input separate from
  inline code editing and give the input the full list width.
- Remove the duplicate Automatic reasoning suffix in the model trigger. Explicit
  reasoning and the full tooltip/picker remain available.
- Share Files/Changes tabs and the close control between both inspector views.
  Changes loads immediately from its tab, retains navigation back to Files, and
  puts wrapping/refresh controls on a separate row so the title cannot collapse
  into a vertical stack of letters.
- Hide the redundant port-forward table header when there are no forwards.
- Explain non-Git folders in Changes without raw Git stderr or a contradictory
  refresh hint.
- Keep short Recent Activity lists at the top. The shared variable list now
  clamps stale scroll positions correctly after filtering or resizing.
- Give shared inputs an explicit full-width layout, fixing invisible search
  text inside nested panels. Verified typing, select-all replacement and clear
  at 1280×900 and 740×900.
- Keep one Save/Cancel toolbar while editing a script. Hide Run until editing
  finishes, so it cannot silently execute the previously saved body; retain
  Stop when an execution is already running.

Walkthrough coverage includes provider/model/project popovers, project add and
cancel with empty/populated catalogs, project creation into Agents, Scripts
selection/search/editing, connection and script forms, port-forward presets and
form, Server Sync's empty state, Files/Changes, Support filters and ticket detail,
and narrow-window navigation. No production connection, provider run, support
reply, sync, or port forward was started. Windows/macOS and live integration
limits from the first pass still apply.

Regression SDTEST-2790 and the complete Linux UI suite passed (280 tests).
SDTEST-2791 covers shared variable-list scroll bounds on both axes. All 19
shared-component tests also passed. All-target Clippy, formatting, diff checks,
EN/FR key parity (2,692 keys), and the native development build passed after the
final UI edits. Visual checks complement these tests; they do not certify every
possible layout or live service condition.

Final visual evidence includes `27-add-existing-catalog.png` and
`28-cancel-return.png` (project routing), `48-changes.png` and `49-file.png`
(inspector/editor), `50-forward.png` (empty forwards), `52-search.png` through
`54-search-clear.png` (search editing/clear at both sizes), `55-recent.png` and
`56-recent-filter.png` (list alignment/filtering), `59-edit-search-isolated.png`
(search does not edit the script draft), `60-collapse.png` (project collapse),
and `62-edit-final.png` / `63-edit-cancel.png` (single editing toolbar and exit).
These files are in the temporary evidence directory above.

## Remote session connection and resume

Added an explicit **Check connection** flow for directory entries. It reads typed
retained history and command state, then offers **Review and resume** for sessions
confirmed by the runtime. Confirmation uses the existing idempotent follow-up
lane without inventing a live attachment or control lease. **Disconnect view**
stops polling and preserves the draft without stopping the remote session.

Validation: all 289 UI tests passed; targeted core/UI all-targets strict Clippy
and app build passed. SDTEST-2802/2803 cover stale/refused/foreign reads, revision
fencing, duplicate confirmation, uncertain receipt reconciliation, draft retention,
disconnect, live-attachment handoff and account reset. EN/FR keys are checked.
Native isolated fixture verified the connection, loaded transcript, review at
760 px, and reconciling state with an editable draft. Evidence:
`/tmp/shelldeck-remote-qa/03-checked.png`, `04-review-narrow.png`,
`05-reconciling.png`. No real provider run was submitted by the fixture.

### Connection diagnosis and gateway repair

Selecting a remote directory session now checks its command state and retained
history automatically. Explicit retry also refreshes gateway capabilities, so
server upgrades are recognized without restarting the app. The UI distinguishes
a missing gateway resume method from a runtime that cannot confirm the session.
Empty retained history has an explanation beside the available resume composer.

Live diagnosis found the selected microbe session fresh at revision 2, with no
pending approvals or stored events. Manage omitted `session_follow_up` from its
capabilities and rejected client-bound receipt lookups. The canonical gateway
now validates and forwards these typed frames, preserving runtime authorization,
revision checks and receipt custody. Legacy JSON/generic execute paths were not
expanded. Three gateway source files were deployed with baseline hash checks and
backups at `/home/infra-sj278/.local/state/shelldeck-gateway-backups/20261030-98b2ce06e191`.
API source revalidation required no production restart.

Validation: 42 gateway tests on the local source snapshot; 38 tests on the
production repository variant, preserving its concurrent changes. All 13 focused
Platform client tests and all 290 UI tests passed. A live typed probe for a
random nonexistent session reached the runtime and returned `target_not_owned`;
the user's actual session was only read, never resumed. Native isolated preview
verified automatic readiness and the empty-history composer at 1280x900:
`/tmp/shelldeck-connect-recovery-qa/01-auto-connected.png`.

Final checks: strict all-target core/UI Clippy and workspace formatting passed.
The deployed gateway's own test run passed all 38 tests and its local health
endpoint returned HTTP 200.
The Linux development app build also passed and was reopened with these changes.

## Native remote sessions and authoritative refresh (initial pass; superseded below)

The earlier Remote sessions surface only listed Automonique directory records.
It could not discover existing native CLI conversations on an SSH server. Agents
now has an explicit SSH server picker and bounded read-only discovery for Claude,
Codex and Jcode, with server/provider grouping, search, collapse, pagination,
last-refresh times and partial/offline states. Reconnect verifies the native ID,
provider, directory and model before preparing an exact read-only native resume.
No provider is started during discovery or reconnect. Prior native transcripts
are not copied into ShellDeck; new messages appear from reconnection onward.

Automonique refresh now always fetches the complete current directory even when
resource subscriptions emit no events. A per-session history read failure no
longer discards a successful directory refresh. Manual refresh and rapid session
selection queue behind in-flight operations instead of silently disappearing.

Live read-only discovery through ShellDeck's own SSH client with the project SSH
configuration found 443 conversations (Claude 200, Codex 43, Jcode 200). The
600-entry/200-per-provider bounds and oversized-Jcode exclusions are visible as
partial-results warnings. No native conversation was submitted or modified.
All 293 Linux UI tests passed; focused core parsing/resume tests and the Python
fixture passed. SSH native resume IDs remain memory-only. The workspace
orchestrator's existing tests were extracted to keep it below 2,000 lines.

Final verification: core/SSH/UI all-target strict Clippy, formatting and the
Linux development build passed. The final reconnect UI regression passed after
the native-screen refinements. Native provider-free fixture verified grouped
lists, selection, reconnect and the ready composer at 1280px and 760px, with
evidence under `/tmp/shelldeck-native-sessions-qa/04-final-list.png`,
`05-narrow-review.png` and `06-narrow-ready.png`. The live SSH check fetched
metadata only and did not send a provider message.

### Native transcript reconnect (SDUC-563)

Reconnection now reads Codex, Claude and Jcode transcripts directly over SSH,
with exact source validation and bounded pages instead of an empty new timeline.
Passive source references survive restart; native execution still requires an
explicit fresh reconnect. Old tool events are never replayed into live state.
Normal reload preserves the imported prefix, and changed/compacted histories have
a separate read-only refreshed view preserving local turns.

Validation: reader fixtures, live read-only samples for all three providers
(including a 57.8 MB Jcode file), source/callback fencing, saved-reference recovery,
and isolated native UI reconnect/pagination. Prior imported conversations without
a persisted source reference need to be selected once more from SSH conversations.
Only content retained by the provider can be loaded; server-deleted or compacted
content cannot be reconstructed. No provider execution or credential changes
were performed as part of history validation.

Final checks: 294 UI tests passed; the follow-up compaction/reconnect recovery
regression passed; strict core/UI all-target Clippy and desktop build passed.
Native visual checks at 1280×900 and 800×900 verified reconnect, the newest,
middle and first history pages, and a composer outside the scrolling history.
Evidence is in `/tmp/shelldeck-history-qa/`.

## Whole-journey acceptance pass (SDUC-564)

The completion criteria now live in [AGENTS_ACCEPTANCE.md](AGENTS_ACCEPTANCE.md)
and are required by `.agents/agent-console.md`. T3 Code was inspected at pinned
commit `6575ce32508bd7908444241f3719aa33aaf77f90`, alongside the user's earlier
requirements. This is workflow verification, not a claim of feature parity.

Fixed adjacent navigation defects: direct project entry and per-project new chat,
canonical project reuse, newest exact active conversation selection, nested
checkout ownership, project-aware search, distinct archive/restore actions and
retained drafts. Unassigned chats have a separate collapsible group; setup help
is collapsed on empty chats. Model discovery rejects stale scopes and retains
known catalogs on failure; unsupported effort choices are no longer fabricated.
Attachment/approval callbacks remain bound to their original context; queued
turns are consumed only after admission succeeds.

Native inventories now page through older conversations, including large Jcode
files; selecting a server refreshes immediately and the saved selection refreshes
once on restoration. Refresh errors retain the known list, continuation errors
retain the cursor, and Hide server invalidates pending callbacks without deleting
remote history. Live read-only discovery found 5,261 unique sessions across 15
pages without warnings or duplicates. Final 302 UI tests, focused core/Python
checks, strict linting, translation parity and both desktop/preview builds passed.
See the acceptance document for clicked native paths and unverified live gates.
