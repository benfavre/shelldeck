# Contextual AI

ShellDeck AI is opt-in and provider-neutral. All integrations go through
`shelldeck_core::ai::AiClient`; views must not invoke Claude, Codex, Aider,
OpenAI, or Anthropic directly.

## Safety contract

- An AI call starts only after an explicit user action. No background
  suggestions, automatic analysis, or telemetry.
- Output is a draft. Never execute a generated command, send a reply, mutate a
  request, or overwrite a script without a separate user confirmation.
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
