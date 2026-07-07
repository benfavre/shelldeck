# Patches — adabraka-ui

**Vendored from**: `adabraka-ui` v0.3.9
**Upstream**: https://github.com/Augani/adabraka-ui
**Last synced**: 2026-07-07 (v0.3.0 → v0.3.9)

Total markers in code: **29**
(sum of the per-entry `Markers` lists below; SDPATCH-008 is an adapter and
carries no marker of its own — see its entry).

## Patches

### SDPATCH-001 — `InputState::set_value` no-op when selection is empty

- **Files / symbols**:
  - `src/components/input_state.rs` — `InputState::set_value`
- **Markers**:
  - `src/components/input_state.rs:319` — `/// ShellDeck patch: the upstream implementation was a no-op when the`
- **Why**: Upstream calls `replace_text_in_range(None, new_value, …)`
  which uses `selected_range` as the range to replace. With an empty
  selection (the normal case) that range is 0-width, so replacing it
  with `""` leaves the content untouched — the built-in
  `.clearable(true)` × chip didn't actually clear anything. We select
  the whole existing content first so the replacement overwrites it.
- **Upstream status**: not filed yet — clear reproducer, worth a PR.

### SDPATCH-002 — Cursor / selection colors from `theme.tokens.ring`

- **Files / symbols**:
  - `src/components/input_state.rs` — `InputTextElement::prepaint`
- **Markers**:
  - `src/components/input_state.rs:1344` — `// ShellDeck patch: cursor / selection colors from the active theme`
- **Why**: Upstream hardcodes `rgb(0x0066ff)` for the caret and
  `rgba(0x3311ff30)` for the selection. Both should come from the active
  theme's `ring` token so the input matches the surrounding app palette
  (ShellDeck ships a beige theme where `#0066ff` is jarring).
- **Upstream status**: not filed yet — pure token wiring, easy PR.

### SDPATCH-003 — Cursor blink

- **Files / symbols**:
  - `src/components/input_state.rs` — `InputTextElement::paint`
    (+ static `INPUT_BLINK_EPOCH` in the same file)
- **Markers**:
  - `src/components/input_state.rs:1438` — `// ShellDeck patch: blink the caret while focused.`
- **Why**: Upstream draws the caret as a static filled rectangle — no
  blink. We request an animation frame each paint and modulate visibility
  on a 500 ms on / 500 ms off cycle keyed to a monotonic epoch so every
  focused input on screen blinks in phase.
- **Upstream status**: not filed yet — feature addition worth upstreaming.

### SDPATCH-004 — Horizontal caret-follow scroll

- **Files / symbols**:
  - `src/components/input_state.rs` — `PrepaintState` (adds `scroll_offset` field)
  - `src/components/input_state.rs` — `InputTextElement::prepaint`
  - `src/components/input_state.rs` — `InputTextElement::paint`
- **Markers**:
  - `src/components/input_state.rs:1214` — `/// ShellDeck patch: horizontal scroll offset applied to the shaped line`
  - `src/components/input_state.rs:1350` — `// ShellDeck patch: horizontal scroll — when the caret would be past`
  - `src/components/input_state.rs:1427` — `// ShellDeck patch: shift the whole line by the horizontal scroll`
- **Why**: Upstream just clips content that overflows the input width, so
  when you type past the visible width the caret disappears off the right
  and you're typing blind. We compute an offset so the caret stays ~4 px
  from the right edge, store it on `PrepaintState`, and apply it to the
  shaped line + caret + selection during paint.
- **Upstream status**: not filed yet — real usability bug worth a PR.

### SDPATCH-005 — Clear-button `×` reliability

- **Files / symbols**:
  - `src/components/input.rs` — `Input::render` (the `.when(show_clear, …)` branch)
- **Markers**:
  - `src/components/input.rs:779` — ``// ShellDeck patch: `.occlude()` blocks``
- **Why**: Upstream renders the × chip as a plain unstateful div with
  `on_mouse_down` — the event was swallowed by the input's own text-area
  mouse handler and the click just moved the caret to position 0 without
  clearing. We gave the chip a stateful id scoped to the state's entity
  id, an explicit 20×20 hit box, `.occlude()` to block passthrough, and
  switched to `.on_click`. Combined with SDPATCH-001, the chip now
  actually clears the input.
- **Upstream status**: not filed yet — pairs with SDPATCH-001 in a PR.

### SDPATCH-006 — Word-level actions on `InputState`

- **Files / symbols**:
  - `src/components/input_state.rs` — actions macro (adds `LeftWord`,
    `RightWord`, `SelectLeftWord`, `SelectRightWord`, `BackspaceWord`,
    `DeleteWord`)
  - `src/components/input_state.rs` — impl block hosting `left_word`,
    `right_word`, `select_left_word`, `select_right_word`,
    `backspace_word`, `delete_word`
  - `src/components/input_state.rs` — `previous_word_boundary`
  - `src/components/input_state.rs` — `next_word_boundary`
- **Markers**:
  - `src/components/input_state.rs:34` — `// ShellDeck patch: word-level navigation and delete (Ctrl+←/→ and`
  - `src/components/input_state.rs:732` — `// ShellDeck patch: word-level cursor movement + delete.`
  - `src/components/input_state.rs:1011` — `/// ShellDeck patch: jump to the start of the previous unicode word.`
  - `src/components/input_state.rs:1020` — `/// ShellDeck patch: jump to the end of the next unicode word.`
- **Why**: Upstream doesn't ship word-level cursor movement at all. Word
  boundaries use `unicode-segmentation::unicode_word_indices` so
  non-ASCII input behaves.
- **Upstream status**: not filed yet — feature addition worth
  upstreaming.

### SDPATCH-007 — Word-nav keybindings

- **Files / symbols**:
  - `src/components/input.rs` — `init` (adds `KeyBinding`s to
    `"Input"` context)
  - `src/components/input.rs` — `Input::render` (registers the six
    new actions on the container)
- **Markers**:
  - `src/components/input.rs:48` — `// ShellDeck patch: word-level navigation and delete.`
  - `src/components/input.rs:728` — `// ShellDeck patch: word-level actions.`
- **Why**: Wires SDPATCH-006 into the `"Input"` key_context. Ctrl+←/→/
  Backspace/Delete on Linux/Windows, Alt+←/→/Backspace/Delete on macOS,
  Ctrl+Shift+←/→ for selection.
- **Upstream status**: pairs with SDPATCH-006 in the same PR.

### SDPATCH-008 — `file_upload` adapter for new `starting_directory`

- **Files / symbols**:
  - `src/components/file_upload.rs` — every `PathPromptOptions { … }`
    struct literal
- **Markers**: none — this is a purely mechanical adaptation to keep
  `file_upload` building after `patches/adabraka-gpui` (SDPATCH-101)
  added the `starting_directory` field on `PathPromptOptions`. It has
  no marker because there's no *reason* to preserve on top of upstream
  code — the entry exists so that a sync catches every struct literal
  that needs `starting_directory: None` re-added.
- **Why**: `PathPromptOptions` gained a new field in
  `patches/adabraka-gpui/src/platform.rs`; every literal construction of
  the struct now needs to initialise it or the crate fails to build.
- **Upstream status**: N/A — companion patch. Removed automatically if
  we ever retire SDPATCH-101.

### SDPATCH-009 — `multi_line` mode on `InputState` / `Input` (real textarea)

- **Files / symbols**:
  - `src/components/input_state.rs` — `InputState` (fields `multi_line`,
    `last_layouts`, `line_height`), `InputState::new`,
    `InputState::multi_line` (builder), `InputState::paste`,
    `InputState::enter`, `InputState::index_for_mouse_position`,
    `PrepaintState` (field `multi_lines`),
    `<InputTextElement as gpui::Element>::request_layout`,
    `<InputTextElement as gpui::Element>::prepaint`,
    `<InputTextElement as gpui::Element>::paint`
  - `src/components/input.rs` — `Input` (fields `multi_line`, `min_rows`),
    `Input::new`, `Input::multi_line` (builder), `Input::min_rows`
    (builder), `<Input as RenderOnce>::render` (state propagation +
    HStack sizing swap in the container)
- **Markers**:
  - `src/components/input_state.rs:193` — `// ShellDeck patch: SDPATCH-009 — when true, the input behaves as a`
  - `src/components/input_state.rs:240` — `// ShellDeck patch: SDPATCH-009 — see the `multi_line` /`
  - `src/components/input_state.rs:248` — `/// ShellDeck patch: SDPATCH-009 — enable multi-line textarea mode. See`
  - `src/components/input_state.rs:861` — `// ShellDeck patch: SDPATCH-009 — keep embedded newlines when in`
  - `src/components/input_state.rs:892` — `// ShellDeck patch: SDPATCH-009 — in multi_line mode, Enter inserts a`
  - `src/components/input_state.rs:967` — `// ShellDeck patch: SDPATCH-009 — multi_line click mapping. When we`
  - `src/components/input_state.rs:1274` — `// ShellDeck patch: SDPATCH-009 — populated in multi_line mode with one`
  - `src/components/input_state.rs:1314` — `// ShellDeck patch: SDPATCH-009 — reserve one line per `\n` segment`
  - `src/components/input_state.rs:1337` — `// ShellDeck patch: SDPATCH-009 — multi_line prepaint path. Shape each`
  - `src/components/input_state.rs:1597` — `// ShellDeck patch: SDPATCH-009 — multi_line paint path. Paint each`
  - `src/components/input.rs:131` — `// ShellDeck patch: SDPATCH-009 — multi_line mirrors the same-named flag`
  - `src/components/input.rs:179` — `// ShellDeck patch: SDPATCH-009 — default single-line; opt in with`
  - `src/components/input.rs:186` — `/// ShellDeck patch: SDPATCH-009 — turn this Input into a multi-line`
  - `src/components/input.rs:195` — `/// ShellDeck patch: SDPATCH-009 — visible height of the textarea, in`
  - `src/components/input.rs:501` — `// ShellDeck patch: SDPATCH-009 — propagate the wrapper's flag to`
  - `src/components/input.rs:769` — `// ShellDeck patch: SDPATCH-009 — in multi_line mode`
- **Why**: adabraka's `Input` is strictly single-line — `InputState::enter`
  always emits `InputEvent::Enter`, `paste` strips `\n`, and
  `InputTextElement` shapes exactly one line with a fixed
  `window.line_height()` layout. Its sibling `Textarea` is a
  `RenderOnce` display-only stub with no state backing. ShellDeck's User-
  mode "Nouvelle demande" needs a real textarea for the Détails field,
  and the ShellDeck `.agents/ui-components.md` rules require extending
  adabraka rather than forking a private widget in `shelldeck-ui`. This
  patch adds a `multi_line: bool` on `InputState` / `Input` that:
  Enter inserts `\n` into the content, paste keeps embedded newlines,
  `request_layout` reserves `n_lines * line_height`, `prepaint` shapes
  one line per `\n`-segment and places the caret on the right line,
  `paint` stacks the shaped lines and snapshots `last_layouts` +
  `line_height` on the state for click mapping, and
  `index_for_mouse_position` uses the click's `y` to pick a line and the
  click's `x` against that line's shaped run. Cross-line selection
  quads are intentionally not drawn (would need Vec<PaintQuad>) — a
  follow-up patch can add them. Up/Down line navigation is also
  deferred; arrows still walk across `\n` via the existing byte-level
  left/right handlers, and mouse click always lands on the right line.
- **Upstream status**: not filed yet — big enough to be worth a real
  design conversation upstream before filing (they may prefer a
  separate `TextareaState` type instead of a flag on `InputState`).

## Preserved files (do not overwrite on sync)

- `PATCHES.md` (this file)
- `CLAUDE.md` — upstream's own working notes; kept in-tree because
  vendored, but not our territory to rewrite. Overwrite it only if the
  upstream version genuinely changed.

## Sync log

- **2026-07-07** — initial inventory. Marker count 13 = 1+1+1+3+1+4+2
  (SDPATCH-008 carries none by design). No sync yet.
- **2026-07-07** — synced v0.3.0 → v0.3.9. All eight SDPATCHes replayed
  clean — only line-number shifts and small mechanical adaptations
  (v0.3.9 refactored `PrepaintState`, but the shape SDPATCH-004 targets
  survived intact; `Input::render` grew a bit around SDPATCH-005's
  clear-button hunk without semantic conflict). Marker count stays at
  13. v0.3.9 also natively pins `adabraka-gpui = "0.5"`, so the temp
  pin bump we introduced during the `adabraka-gpui` sync
  (see `4a6c705`) becomes a no-op / redundant.
- **2026-07-07** — added SDPATCH-009 (real textarea via `multi_line` on
  `InputState`/`Input`). Marker count 13 → 29 (16 new markers). No sync
  in this entry — pure additive extension. `Input::render` now branches
  in the HStack container between `.h(height)` + `.items_center()` (the
  upstream single-line box) and `.min_h(min_rows*line_h+pad)` +
  `.items_start()` (textarea box); a future upstream refactor of that
  container will need eyes on that hunk.

## Retired patches

*(empty for now)*
