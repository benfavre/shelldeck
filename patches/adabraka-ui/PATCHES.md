# Patches — adabraka-ui

**Vendored from**: `adabraka-ui` v0.3.9
**Upstream**: https://github.com/Augani/adabraka-ui
**Last synced**: 2026-07-07 (v0.3.0 → v0.3.9)

Total markers in code: **38**
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

### SDPATCH-009 — `multi_line` flag + API on `InputState` / `Input`

- **Files / symbols**:
  - `src/components/input_state.rs` — `InputState` (`multi_line` field),
    `InputState::new` (init the flag), `InputState::multi_line` (builder),
    `InputState::paste` (keep `\n` when multi_line),
    `InputState::enter` (insert `\n` when multi_line)
  - `src/components/input.rs` — `Input` (`multi_line`, `min_rows` fields),
    `Input::new` (init the fields), `Input::multi_line` (builder),
    `Input::min_rows` (builder),
    `<Input as RenderOnce>::render` (propagate flag to state + swap
    `.h(height).items_center()` for `.min_h(min_rows * line_h + pad)
    .items_start()` in the HStack container)
- **Markers**:
  - `src/components/input_state.rs:193` — `// ShellDeck patch: SDPATCH-009 — when true, the input behaves as a`
  - `src/components/input_state.rs:246` — `// ShellDeck patch: SDPATCH-009 — flag. SDPATCH-010 — layouts +`
  - `src/components/input_state.rs:256` — `/// ShellDeck patch: SDPATCH-009 — enable multi-line textarea mode. See`
  - `src/components/input_state.rs:869` — `// ShellDeck patch: SDPATCH-009 — keep embedded newlines when in`
  - `src/components/input_state.rs:900` — `// ShellDeck patch: SDPATCH-009 — in multi_line mode, Enter inserts a`
  - `src/components/input.rs:131` — `// ShellDeck patch: SDPATCH-009 — multi_line mirrors the same-named flag`
  - `src/components/input.rs:179` — `// ShellDeck patch: SDPATCH-009 — default single-line; opt in with`
  - `src/components/input.rs:186` — `/// ShellDeck patch: SDPATCH-009 — turn this Input into a multi-line`
  - `src/components/input.rs:195` — `/// ShellDeck patch: SDPATCH-009 — visible height of the textarea, in`
  - `src/components/input.rs:501` — `// ShellDeck patch: SDPATCH-009 — propagate the wrapper's flag to`
  - `src/components/input.rs:769` — `// ShellDeck patch: SDPATCH-009 — in multi_line mode`
- **Why**: adabraka's `Input` is strictly single-line — `enter` always
  emits `InputEvent::Enter`, `paste` strips `\n`. Its sibling `Textarea`
  is a `RenderOnce` display-only stub with no state backing. ShellDeck's
  User-mode "Nouvelle demande" needs a real textarea for the Détails
  field, and the `.agents/ui-components.md` rules require extending
  adabraka rather than forking a private widget in `shelldeck-ui`. This
  patch is the "surface" half — a `multi_line: bool` on `InputState` and
  `Input`, plus a `min_rows` for visible height on `Input`, plus the
  behavior swaps in `enter` / `paste` / the render container's sizing.
  The rendering half (shape_text with wrap, cursor placement, click
  mapping) lives in **SDPATCH-010** — the two are kept separate so a
  future refactor can retire one without disturbing the other.
- **Upstream status**: not filed yet — big enough to be worth a real
  design conversation upstream before filing (they may prefer a
  separate `TextareaState` type instead of a flag on `InputState`).

### SDPATCH-010 — wrap-aware multi_line rendering via gpui `shape_text`

- **Files / symbols**:
  - `src/components/input_state.rs` — `InputState` (fields
    `wrapped_layouts`, `wrapped_line_count`, `line_height`),
    `InputState::index_for_mouse_position` (multi_line click mapping via
    `WrappedLine::closest_index_for_position`),
    `PrepaintState` (field `wrapped_lines`),
    `<InputTextElement as gpui::Element>::request_layout` (multi_line
    height uses last-paint wrapped-visual-line count),
    `<InputTextElement as gpui::Element>::prepaint` (multi_line branch
    calls `window.text_system().shape_text` with
    `wrap_width = Some(bounds.width)` and places the caret via
    `WrappedLine::position_for_index`),
    `<InputTextElement as gpui::Element>::paint` (multi_line branch
    paints each `WrappedLine` at its cumulative Y, then feeds the
    visual-line count back to the state + `cx.notify()` when it changes
    so the next `request_layout` reserves enough height)
- **Markers**:
  - `src/components/input_state.rs:199` — `// ShellDeck patch: SDPATCH-010 — one \`WrappedLine\` per \`\\n\`-segment,`
  - `src/components/input_state.rs:975` — `// ShellDeck patch: SDPATCH-010 — multi_line click mapping. Walk the`
  - `src/components/input_state.rs:1290` — `// ShellDeck patch: SDPATCH-010 — populated in multi_line mode with one`
  - `src/components/input_state.rs:1332` — `// ShellDeck patch: SDPATCH-010 — reserve enough vertical space in`
  - `src/components/input_state.rs:1359` — `// ShellDeck patch: SDPATCH-010 — multi_line prepaint path. gpui's`
  - `src/components/input_state.rs:1633` — `// ShellDeck patch: SDPATCH-010 — multi_line paint path. Each`
- **Why**: the initial SDPATCH-009 landed with a naive multi_line
  renderer that called `shape_line` once per `\n`-separated segment.
  `shape_line` doesn't wrap — a long paragraph without hard breaks was
  laid out as a single visual line running past the input's right edge
  (visible bug, screenshot linked from the ShellDeck session on
  2026-07-07). gpui already ships `TextSystem::shape_text(text, fs,
  runs, Some(wrap_width), None)` which returns
  `Vec<WrappedLine>` — one per `\n`-segment with `wrap_boundaries` for
  each soft-wrap. This patch replaces the `shape_line`-per-segment
  approach with a single `shape_text` call at the input's inner width,
  walks the returned `WrappedLine`s (each carries a
  `WrappedLineLayout` via Deref) to place the caret with
  `position_for_index`, paints each with `WrappedLine::paint(...,
  TextAlign::Left, None, ...)`, and stores the resulting
  visual-line count on the state so the next `request_layout` reserves
  enough vertical room (previous frame's count → this frame's reserved
  height, one-frame lag). Click mapping walks the same layouts via
  `closest_index_for_position`. Selection quads still only render when
  both ends land on the same visual sub-line — cross-sub-line
  selection is a follow-up.
- **Upstream status**: not filed yet — bundles with SDPATCH-009's
  design conversation.

### SDPATCH-011 — fix leaking `cx.subscribe` in `Input::render` (duplicate event dispatch)

- **Files / symbols**:
  - `src/components/input_state.rs` — `InputState` (fields
    `on_change_cb`, `on_enter_cb`, `on_focus_cb`, `on_blur_cb`,
    `on_validate_cb`), `InputState::new` (init the slots),
    `InputState::replace_text_in_range` (fires `on_change_cb`),
    `InputState::enter` (fires `on_enter_cb`),
    `InputState::escape` (fires `on_blur_cb`),
    `InputState::on_focus` (fires `on_focus_cb`),
    `InputState::on_blur` (fires `on_blur_cb`)
  - `src/components/input.rs` — `<Input as RenderOnce>::render` (drops
    the `cx.subscribe(...).detach()` block and writes the callbacks
    into the state's slots in place)
- **Markers**:
  - `src/components/input_state.rs:193` — `// ShellDeck patch: SDPATCH-011 — direct callback slots for the Input`
  - `src/components/input_state.rs:260` — `// ShellDeck patch: SDPATCH-011 — initialise the direct callback`
  - `src/components/input_state.rs:395` — `// ShellDeck patch: SDPATCH-011 — direct callback slot fires exactly`
  - `src/components/input_state.rs:937` — `// ShellDeck patch: SDPATCH-011 — invoke the direct callback`
  - `src/components/input_state.rs:950` — `// ShellDeck patch: SDPATCH-011 — direct callback slot fires here so`
  - `src/components/input_state.rs:969` — `// ShellDeck patch: SDPATCH-011 — direct callback slot.`
  - `src/components/input_state.rs:991` — `// ShellDeck patch: SDPATCH-011 — direct callback slot.`
  - `src/components/input.rs:588` — `// ShellDeck patch: SDPATCH-011 — replace the leaking \`cx.subscribe\``
- **Why**: `Input::render` in v0.3.9 (and upstream `main` at the time of
  writing) calls `cx.subscribe(&state, ...).detach()` on every render
  pass. `Subscription::detach()` only cancels the drop-unsubscribe
  callback — the underlying listener stays alive until the observed
  entity is dropped. Every render therefore appends a new listener; after
  N frames a single Enter press invokes the `on_enter` handler N times.
  In-session repro (2026-07-07): sending one Support reply produced
  ~400 duplicated `send_composer` calls. The fix swaps the pub/sub for
  five Rc-boxed callback slots on `InputState`; each render calls
  `state.update` to write the current wrapper closures into the slots
  (replace, not append), and the InputState action handlers invoke the
  slot directly, exactly once per event. Existing subscribers to
  `InputEvent::*` still work — we did not drop the `cx.emit(...)` calls,
  only added the direct call alongside.
- **Upstream status**: not filed yet — clear bug with a small repro,
  worth a PR (the reproducer is `on_enter` called N times after N
  renders of the same Input).

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
- **2026-07-07** — SDPATCH-011: fixed the leaking `cx.subscribe` in
  `Input::render` (each render pass appended a fresh listener → single
  Enter press invoked `on_enter` N times). Swapped for five direct
  `Rc<Fn>` slots on `InputState` populated via `state.update`. Marker
  count 30 → 38 (8 new markers).
- **2026-07-07** — SDPATCH-010: replaced the multi_line renderer's
  `shape_line`-per-`\n`-segment with gpui's `shape_text` at the input's
  inner width so long paragraphs actually wrap instead of running past
  the right edge. Moved 5 rendering markers from SDPATCH-009 to
  SDPATCH-010 (SDPATCH-009 now covers the surface — flag, builder,
  container-sizing swap; SDPATCH-010 covers the shape/paint guts).
  Added 1 new marker (`wrapped_layouts` field replaces the old
  `last_layouts: Vec<ShapedLine>` with `Vec<WrappedLine>` + a
  `wrapped_line_count` for the request_layout feedback loop). Net
  marker count 29 → 30.

## Retired patches

*(empty for now)*
