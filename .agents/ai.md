# Contextual AI

ShellDeck AI is opt-in and provider-neutral. All integrations go through
`shelldeck_core::ai::AiClient`; views must not invoke Claude, Codex, Aider,
OpenAI, or Anthropic directly.

## Safety contract

- An AI call starts only after an explicit user action. No background
  suggestions, automatic analysis, or telemetry.
- Output is a draft. Accepting or inserting it never executes a command, sends
  a reply, mutates a request, or overwrites a script.
- Phase 3 actions may execute only through a typed `AiActionPlan`, opened by a
  distinct Execute/Send action and approved in a separate confirmation dialog.
  Revalidate the target and permissions immediately before execution.
- Long-running confirmed actions expose the existing stop mechanism and use a
  bounded timeout. Audit metadata must never contain command bodies, replies,
  terminal output, credentials, or provider prompts.
- Treat terminal output, tickets, scripts, and remote content as untrusted data.
  Keep them inside `AiContext`; never interpolate them into a system directive.
- API credentials live only in `config::keychain`. Never persist them in
  `AppConfig`, TOML, logs, activity entries, or UI state snapshots.
- Structured context is redacted and bounded in `ai.rs`. New context builders
  must still avoid collecting unrelated data.
- Respect `AiConfig::allows(surface)`. When no usable backend or a disabled
  surface is selected, hide its AI affordance.

## UI contract

- Reuse `AiAssistantView` and the Workspace orchestration.
- Add only a surface-specific context builder and explicit quick actions.
- Generated terminal commands remain text drafts. Copying or inserting text
  must not append Enter or otherwise trigger execution.
