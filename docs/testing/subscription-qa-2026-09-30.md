# Subscription CLI verification — 2026-09-30

ShellDeck already delegates Claude Code and Codex authentication to their
installed CLIs. Neither CLI branch calls the OS-keychain API-key path. This
pass proves that the contextual assistant works through subscription logins and
makes that setup discoverable in Settings and the Agents empty state.

## Live subscription check

SDTEST-1935 ran the real ShellDeck `create_client` → `complete` →
`test_connection` path against Codex CLI 0.159.2 and Claude Code 2.1.285.
A child test process removed `OPENAI_API_KEY`, `CODEX_API_KEY`,
`ANTHROPIC_API_KEY`, and `ANTHROPIC_AUTH_TOKEN`, preserving the providers’
normal OAuth storage. CLI status first confirmed ChatGPT authentication for
Codex and Claude.ai/Max for Claude Code. Both returned exactly
`SHELLDECK_AI_OK`. No API key was entered or stored in ShellDeck.

Repeat only on a computer already signed in with both eligible subscriptions:

```sh
PKG_CONFIG_PATH=/usr/lib/x86_64-linux-gnu/pkgconfig \
SHELLDECK_LIVE_SUBSCRIPTION=1 \
cargo test -p shelldeck-core --lib \
  ai::tests::live_subscription_clis_complete_without_api_keys \
  -- --ignored --exact --nocapture
```

This test is ignored by default: it needs the user's subscriptions and sends
two real, minimal completions. It never starts a login, replaces credentials,
or runs a write-capable coding agent. Provider CLIs may refresh their own tokens.

## Native interface check

The actual app ran on a private X11 display with a temporary XDG app profile,
fictional Manage account and API-key variables removed. At 1200 × 850 and
580 × 600:

- [Both subscription choices](screenshots/2026-09-30/subscriptions/backend-options.png)
  fit without clipping; the API providers remain separate options.
- [Claude Code](screenshots/2026-09-30/subscriptions/claude-settings.png) and
  [Codex](screenshots/2026-09-30/subscriptions/codex-settings.png) show their
  sign-in command, account-status command, official guide, quota reminder,
  and API-credential billing warning. Neither shows an API-key input.
- Both copy buttons produced the exact command in the private display's
  clipboard: `claude auth login --claudeai` and `codex login`.
- [Compact Settings](screenshots/2026-09-30/subscriptions/codex-compact.png)
  retains the setup controls through scrolling.
- [Agents](screenshots/2026-09-30/subscriptions/agents-setup.png) uses the same
  component. Its guide is fully visible at desktop size; for an SSH target,
  setup belongs on the selected host rather than forwarding local credentials.

The normal workspace suite still passes 853 tests; seven live/infrastructure
cases and two doc examples are ignored. Linux build/check, Clippy with
`--no-deps -- -D warnings`, formatting and whitespace checks pass.
SDTEST-1936 remains Red for automated native render/click coverage.

## Boundaries

Authentication and billing remain owned by the CLI. Configured API credentials
can select API billing; the interface tells users to check their active account.
This pass does not force a different login, promise access outside a plan's
entitlements, or remove the independent API backends. Native browser sign-in,
remote subscription login and full writable coding-agent journeys were not
live-tested. No production ticket, request, source file or remote service was
modified by the two completion checks.

Official behavior was checked against [Codex authentication](https://developers.openai.com/codex/auth),
[Codex CLI commands](https://developers.openai.com/codex/cli/reference),
[Claude Code authentication](https://code.claude.com/docs/en/authentication),
and [Claude Code credential precedence](https://code.claude.com/docs/en/env-vars).
