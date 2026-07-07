# Patches — adabraka-ui

**Vendored from**: `adabraka-ui` v0.3.9
**Upstream**: https://github.com/Augani/adabraka-ui
**Last synced**: 2026-07-07 (v0.3.0 → v0.3.9)

Total markers in code: **13**
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

## Retired patches

*(empty for now)*
