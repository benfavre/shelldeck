# Chat latency and behavior audit — 2026-10-30

Scope: native Agents send/admission, local and SSH CLI streams, local Codex
app-server, Automonique ACP, completion, cancellation, background conversations,
and failed-launch recovery. The contextual AI Assistant was also inspected; its
`AiClient::complete` path returns a complete response rather than streaming.

## Findings and changes

| Path | Previous behavior | Change |
|---|---|---|
| Read-only local chat | Full repository observation before provider startup and again before completion | No eager Git snapshot. Changes remains available on demand. Local write-enabled runs retain exact before/after boundaries. |
| Stream delivery | Independent 30 ms Codex / 50 ms CLI and ACP polling loops | Shared 16 ms delivery interval. This reduces the configured polling wait, not provider inference time. |
| Bursty output | Drain an unbounded number of queued events in one UI callback; restart the persistence timer for each event | At most 64 events per frame; one library-save scheduling call and repaint notification per batch. |
| Completion | Separate, duplicated event/done handling | Retain completion until every buffered event is delivered, including worker disconnects and exact batch boundaries. |
| Background sessions | Every run could request follow-latest on the visible scroll owner | Only the selected, visible local session follows its own output/completion. |
| Rejected launch | Retained run options, stale context-control entities, missing save scheduling | Release inputs, refresh controls, retain the transcript, persist the failed status, allow a new attempt. |
| Orphan completion | Missing session could retain an active-run slot | Release the stale runtime entry. |

The application records debug timing for bridge startup-to-first-text,
bridge startup-to-drained-completion, and pre/post Git observations. These are
local diagnostics containing durations, without prompts, responses, paths,
credentials or external telemetry. Bridge timings begin after worker launch;
they are not an end-to-end Send-to-paint measurement.

The 16 ms interval is a scheduling target, not a real-time guarantee. Delivery
remains frame-paced polling. A heavy burst is intentionally spread across
frames: the regression fixture drains 1,000 events in 16 batches while preserving
all text before completion. This limits UI starvation; it is not a hard memory
bound on the producer channel.

## Behavior retained

- Stop has its existing priority/cancellation path; clean interruption remains
  Cancelled, worker loss remains Failed, and queued turns advance only after
  successful completion.
- Run UUIDs fence late output and separate concurrent conversations.
- Provider text normalization still suppresses duplicate final snapshots and
  retains split-token redaction. No privacy protection was removed to reduce
  first-token delay.
- Write-enabled local turns still wait for review snapshots so queued edits
  cannot contaminate before/after attribution.
- Context changes still invalidate provider resume state. Admission and write
  confirmations are unchanged.

## Validation

- 284 UI tests passed, including SDTEST-2793–2795 and existing stopping,
  admission, queue, authorization and session-context checks.
- 89 core agent tests passed, including SDTEST-2792, stream normalization,
  protocol handling, process cleanup and session persistence.
- All-target workspace Clippy, formatting, diff checks and the Linux native
  development build passed.
- Native controlled CLI fixture: streamed text appears progressively, the
  final provider snapshot does not duplicate text, normal completion releases
  the composer, and Stop preserves partial output with Run stopped status.
  Both read-only runs issued zero eager Git observations.
- Two local debug-build samples delivered first text to the UI update about
  46 ms and 22 ms after the fixture emitted it. Bridge-ready to first text was
  263 ms and 241 ms, including the fixture's deliberate 200 ms first-text
  delay. These are diagnostic samples under Xvfb/software rendering, not a
  release benchmark, paint measurement, or before/after provider comparison.
  Normal completion reached the UI about 72 ms after fixture completion,
  including the existing local process-exit polling interval.
- Evidence: `/tmp/shelldeck-chat-latency-20261030/04-streaming.png`,
  `05-stop.png` (normal completion; the attempted stop came after completion),
  `06-before-stop.png`, `07-cancelled.png`, and `native.log`. Fixture timings
  are `/tmp/shelldeck-feature-audit-20261030/fixture-times.log`. The fixture
  used an isolated profile/network namespace; no real provider call ran.

## Remaining latency sources

Provider startup/authentication/model computation and SSH connection setup are
outside these UI delivery changes. This audit does not benchmark live paid
models or claim a provider speedup. Exact Git review can still delay write-mode
startup/completion on large repositories. The contextual AI Assistant's
non-streaming completion API would require a separate transport-level streaming
change. Windows/macOS runtime behavior requires their native runners.

## Follow-up: DeepSeek credential-consent failure

The actual SSH target has Jcode `v0.79.35-dev (b6c8d8a51)`. Its noninteractive
runner refuses to read an existing OpenCode DeepSeek credential source until the
user approves that source. The ordinary interactive launcher also failed because
its daemon had no interactive confirmation channel. `jcode login --provider
deepseek` requests a new key; `jcode repl --provider deepseek --no-update
--no-selfdev` correctly presents the existing-source trust question. This was
verified over the project's SSH configuration without reading credential files.

ShellDeck now recognizes this diagnostic in the current failed turn and offers
concise recovery guidance, Copy command, and Open target terminal. It does not
approve consent, copy credentials, or replay the request implicitly. The raw
provider diagnostic remains in retained history, with a concise lifecycle label
on screen. SDTEST-2796 includes recovery from saved history after restart.

The user explicitly approved the existing-source trust prompt. That exposed a
second issue in the installed Jcode build: `load_api_key_from_env_or_config`
returns early when `~/.config/jcode/deepseek.env` is absent, before calling the
approved external-source fallback. Created that file exclusively with mode 0600
and a comment only (no key), allowing Jcode to reach the trusted OpenCode source.
The original credential file was unchanged; no credential was copied. Jcode's
auth status then reported DeepSeek available. A single read-only live request,
with base tools disabled and an instruction to reply `OK` without inspecting
files, returned `OK` and exit code 0 on 2026-10-30.

The real output additionally revealed that this Jcode version uses
`text_delta` with a `text` field and a final `done` record. ShellDeck now parses
both alongside the older event forms (SDTEST-2797); previously those records
were only generic activity. The final snapshot uses the existing suppression
of duplicate text after streaming.

Validation: the new parser regression, all 285 UI tests, workspace Clippy, and
the native build passed. After the layout adjustment, the focused recovery
regression and workspace Clippy passed again. Native isolated-profile checks
at 1280×900 and 740×900 verified wrapped recovery copy, opening a terminal in
the selected fixture project's folder without executing the setup command,
and a successful two-delta Jcode fixture followed by `done`: one assistant
answer, completed status, and no stale recovery card on the new turn. Evidence
is `/tmp/shelldeck-deepseek-qa/11-wrap-wide.png`, `12-wrap-narrow.png`,
`13-open-terminal.png`, and `15-stream-complete.png`. The fixture's namespace
had no real credentials/network access; the separate real server smoke check
above verified authentication and provider response.

## Follow-up: Jcode transcript and run details

Read the installed target's Jcode NDJSON emitter to verify field shapes.
Connection phases, message boundaries and token records no longer become generic
chat rows. Tokens and start/done model fields populate structured run details;
tool_start/exec/done share an identified tool entry. tool_input JSON fragments
are never answer deltas. Jcode text_replace/done records apply authoritative
answer snapshots, including corrected or trailing text, without duplicate prose.
Errors take precedence over result bodies. The final snapshot is included in
first-output timing for snapshot-only responses.

Metadata now records its originating turn and resets at new-run/context
boundaries. Persistence retains only metadata for the latest matching turn and
context, dropping legacy unproven model labels. Legacy Jcode transport rows
are hidden only in presentation using their recorded provider; original history
and meaningful activity remain intact.

Validation: 92 core agent tests and all 286 then-existing UI tests passed;
SDTEST-2799/2800 cover Jcode protocol replay and durable metadata provenance.
SDTEST-2801 checks legacy trace presentation. A private Xvfb fixture in Solarized
Light reproduced the user's greeting: one answer, Run completed, and collapsed
Run details showing deepseek-v4-flash and the expected 2348 input/8 output tokens.
Evidence: /tmp/shelldeck-jcode-qa/03-answer.png and 05-accurate-details.png.
The native check used an isolated profile, synthetic provider, and no paid call.
