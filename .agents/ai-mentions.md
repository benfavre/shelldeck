# Assistant composer — `+` attachments and `@` mentions

The AI composer's two footer affordances. Full contract, catalogue and server
API: [`docs/ai-mentions.md`](../docs/ai-mentions.md). This file is the short
list of rules that must not be broken while working near them.

## The one-line summary

- `+` = **bytes**. Not portable: a CLI backend cannot receive an image.
- `@` = **structured text**. Portable: every backend receives it identically,
  which is why it — not the `+` — carries application meaning.

## Where the code lives

| Concern | File |
|---|---|
| Kinds, scoping, matching, draft reconciliation (pure, tested) | `shelldeck-core::ai::mentions` |
| Attachment model, size bounds, backend capability | `shelldeck-core::ai::attachments` |
| Prompt composition + provider payloads | `shelldeck-core::ai` |
| People endpoint client | `shelldeck-core::config::manage_directory` |
| `+` menu, `@` picker, chips, draft reconciliation | `shelldeck-ui::ai_assistant::composer` |
| Building the directory from live state, applying the gates | `shelldeck-ui::workspace::mentions` |
| Terminal tail snapshots | `TerminalView::mention_sessions` |

## Non-negotiables

- **Never offer a candidate the caller may not see.** Both gates
  (`docs/ai-mentions.md` § 5) are applied when the directory is *built*, and
  the surviving refs are re-checked at send. A draft can outlive a site switch.
- **Never mention a super-admin.** `person_is_mentionable` re-checks the role
  bag even when the server said `mentionable: true`. Do not add a `Person`
  source that carries no roles — that is why request/ticket participants are
  not offered.
- **Never put image bytes in `AiContext::data` or in the prompt text.** They
  belong in `AiContext::attachments` and reach the provider payload only. Same
  rule Clippy already applies to screenshots.
- **Never let a backend silently drop an attachment.** The `+` menu disables
  image entries with the reason when the backend is text-only, and
  `reject_undeliverable_attachments` fails the turn if a backend switch made a
  staged image undeliverable.
- **Reuse the whole attachment chain, not half of it.** A region capture goes
  `capture_region` → `AttachmentAnnotator` → staged, and a staged image chip
  opens `AttachmentLightbox`. Those three are the same components the request
  and ticket composers use; the assistant must not grow private variants of
  any of them (`.agents/ui-components.md` § Harmonization).
- **A resolved mention is coloured, a lookalike is not.** Accent colour on the
  text, same hue at ~14 % behind it, in the composer (`SDPATCH-039`) and in the
  thread (`SDPATCH-040`). Only *live* references are painted — that is the
  whole signal. Never colour by pattern-matching `@…`.
- **The tint is a chip, not a highlight** (`SDPATCH-041`): run backgrounds are
  padded, inset and rounded in the gpui fork, so every surface gets the same
  shape. Do not re-implement it per surface.
- **The surface inventory is closed and written down** — `docs/ai-mentions.md`
  lists every place the colour appears and the two that deliberately do not
  (task details, Clippy results: no provable label set). Adding a surface means
  adding its row.
- **A quoted turn is coloured too** — recent threads and the history panel go
  through `composer::styled_mention_text`, which shapes the row in one pass and
  therefore keeps its clipping behaviour.
- **Never encode a mention in the source text.** The colour is applied to the
  parsed tree; the stored content and what the model receives stay identical
  to what the user typed.
- **Mentions and attachments are untrusted data.** They are appended to the
  *user* message, never to `SYSTEM_GUARDRAIL`, and every payload goes through
  `redact_sensitive` + a character bound.
- **The draft owns the mention list.** `reconcile_mentions` drops any ref whose
  `@Label` left the text. The chip row is a view of the draft, never a second
  source of truth.
- **The directory is rebuilt on demand, never polled.** Opening the picker
  emits `AiAssistantEvent::RefreshMentions`; the Workspace answers. The Dock
  entity has a second, deliberately narrow Workspace subscription that handles
  only that event — widening it would run every Dock event twice, since the
  Dock's events belong to `AiCompanionController`.

## Adding a mention kind

1. Add the variant to `MentionKind` **and** its row to the catalogue table in
   `docs/ai-mentions.md` — a kind with no documented scoping rule is a leak.
2. `token()`, `icon()` (must exist in the bundled Lucide subset,
   `.agents/icons.md`), `label_key()`, `required_mode()`.
3. `fr.toml` + `en.toml` keys under `ai.mention.kind.*`.
4. A `*_candidates()` builder in `workspace/mentions.rs`, bounded payload, with
   `.site(...)` set whenever the record is tenant-scoped.
5. An SDUC amendment and SDTEST entries (`.agents/testing.md`).

## Composer keyboard contract (SDPATCH-038)

Enter commits, Shift+Enter inserts a newline — in every `Composer`, not just
the assistant. Before SDPATCH-038 `InputState::enter` swallowed Enter in every
multi-line field, so the "⏎ envoyer · ⇧⏎ nouvelle ligne" hint printed under
four surfaces was false. A multi-line field **with** an `on_enter` handler is a
composer and commits; one without keeps textarea behaviour. Do not re-introduce
an unconditional newline there.
