# Spacing — no visual collisions, ever

**Every UI element must have breathing room from its neighbours.** No character
glued to a border, no chevron touching a rail, no button flush against another
control, no toggle knob overlapping its track edge, no line-number stuck to a
gutter divider. If two visual elements can visually touch in *any* built-in
theme at *any* font size, you have a spacing bug — treat it as blocking.

## Why this rule exists

**Incident (2026-07):** the file-editor gutter got refactored to have a
proper `[breakpoint][number][fold][code]` column layout with a 1px vertical
rail between gutter and code. The fold column was sized at exactly `cell_w`
and the chevron `▾` was painted at the RIGHT edge of that cell — so the
chevron visually kissed the vertical rail with zero padding. It looked
broken, felt cramped, and the maintainer had to point it out three times
across three screenshots before it was noticed. The fix took two lines
(widen the fold column to `cell_w * 1.5`, paint the chevron in its LEFT half)
but the round-trip cost was measured in tens of minutes.

The lesson: **treat spacing as part of the contract, not a polish pass.**
It's cheaper to bake in breathing room than to fix a collision after the
maintainer sees it.

## The rule, applied to every axis

### Horizontal — glyph / element vs. border

- **Never paint a text glyph or icon flush against an adjacent border, rail,
  or cell edge.** Reserve at least ~half a monospace cell (or ~4-6px) of
  padding between the glyph's bounding box and the next visual element.
- **Icon in a decorated column** (gutter, sidebar chip, badge) → size the
  column at ≥1.5× the icon width, paint the icon centered in the left or
  right half (not stretched across the full column).
- **Two horizontally-adjacent controls** (buttons, chips, kebabs) → keep
  `gap(px(6.0))` or more between them; `gap(px(2.0))` is a collision waiting
  to happen at scale 1.2×.

### Horizontal — text run vs. gutter / rail

- Line numbers, fold chevrons, breakpoints and code all live in different
  columns. **Each column owns its width**; a glyph painted at the boundary
  belongs to the column, not the rail.
- Right-aligned line numbers must leave at least 1 cell of padding before the
  next column starts.

### Vertical — cell-height alignment

- `shape_line().paint(origin, cell_h, …)` places the glyph at the baseline
  the shaper picks — which may not match the cell's own baseline for
  atypical Unicode glyphs (chevrons, arrows, math symbols). If you paint a
  standalone glyph in a cell, **check on both a short line-height (1.2) and
  a long one (1.6) that the glyph stays visually centered**. If it drifts,
  switch to a glyph with standard metrics (Geometric Shapes block: `▾ ▸ ▼
  ▶ ▽ ▷ ◇ …`) or paint the mark procedurally with `paint_quad`.

### Vertical — inter-row spacing

- Row height comes from `cell_h = font_size * line_height`. It's the same
  for every visible row — no row may render taller because of a paint
  side-effect. If a specific row appears larger, look for a paint that
  used `shaped.height` instead of `cell_h`, or an off-by-one in the row
  loop.

### Toggles / knobs

- The knob's track edge must never coincide with the visible toggle bounds.
  Leave inner padding (adabraka `Toggle` already does this — hand-rolled
  toggles must match it or the OFF state looks like two adjacent color
  blocks).

### Modals / dialogs / sheets

- Content must never touch the modal frame; leave at least `p(px(16.0))`
  inside. Buttons in the footer keep `gap(px(8.0))` at minimum.
- See [`overflow.md`](overflow.md) for the height cap + scroll body rules
  — this file is about *padding*, that one about *containment*.

## Pre-commit checklist for any paint / layout change

Before you consider a paint change done, walk this list:

1. **Zoom in** (mental or actual screenshot at 2× / 3×) on every edge
   between two elements you touched. Ask: *can I see a ≥2px gap?* If not,
   fix it.
2. **Toggle every relevant setting** that changes cell size (font size 10
   / 14 / 20, line-height 1.2 / 1.5 / 1.7, terminal font family swap,
   theme swap). Collisions often only appear at one specific combo.
3. **Try both a dense and a sparse theme** (Solarized Light + Tokyo Night
   are the two extremes we ship). A cell that looks fine on Dark can
   collapse into a monochrome blob on Solarized Light.
4. **Longest realistic content**: line numbers for a 10 000-line file
   (5 digits + padding), Unicode names, long file paths in the tab bar.
5. **If the change touches the gutter, the sidebar, a modal, or a toast**
   → screenshot the corner and inspect the boundary you added. Don't
   ship it based on the local dev machine feel alone.

## Related files

- [`overflow.md`](overflow.md) — text clipping, wrap, scroll body rules
  (padding is here; overflow is there).
- [`ui-components.md`](ui-components.md) — prefer adabraka components that
  come with tuned padding out of the box.
- [`theming.md`](theming.md) — the colors that make a collision visible
  or invisible; a "no space bug" that only shows in Solarized Light is
  still a space bug.
