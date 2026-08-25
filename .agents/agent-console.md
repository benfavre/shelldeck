# Agent console presentation

The Dev Agents surface combines provider JSONL control records with a Markdown
conversation. Keep those two layers visually and semantically separate.

- Claude `type="system"` is not synonymous with Ready. Only
  `subtype="init"` becomes `AgentStreamEvent::Ready`; hook and status records
  must not flood the visible trace.
- Consecutive identical activity labels collapse. Keep technical activity out
  of the transcript flow: a round header button opens a bounded popover with
  the twelve newest labels, so progress never pushes the conversation away.
- The transcript is chat prose, not a document. Render it with
  `Markdown::compact()` to retain the shared 8 px conversation rhythm used by
  Support, Monique, and the assistant.
- Bound the Markdown renderer to the transcript column with a zero minimum
  width and clipped overflow. Its horizontal-rule child is absolutely sized
  and otherwise escapes the 860 px conversation measure on wide windows.
- Keep the transcript/activity column as the single scroll owner and the
  composer as its non-shrinking sibling. Do not nest another transcript scroll
  or place the composer inside it.
- Keep the prompt on the shared `Composer`; its frame inherits the same theme
  radius, border, shadow, hover and focus tokens as ShellDeck `Input`. Do not
  add Agent-only card chrome around it.
- Treat provider, target, permissions, and workdir as one execution context.
  The first three use adabraka `Select::context_label` inside one divided
  frame; workdir is the editable final row. The model override belongs in the
  Composer footer because it applies to the next message, and must remain a
  free-form CLI value rather than a hard-coded shortlist.
