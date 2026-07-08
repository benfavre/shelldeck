# Overflow & text containment

GPUI does **not** wrap or clip text by default. A `max_w` on a flex row is
**not enough** — unconstrained flex children grow with their content and bleed
past the cap. This bit us on Support message bubbles and toast notifications.

## Flex rows/columns (toasts, toolbars, action bars, list rows)

1. **`min_w(px(0.0))` / `min_h(px(0.0))`** on any `flex_1` / `flex_grow` child
   that must shrink inside a capped parent. Without it, long strings ignore
   `max_w` on the ancestor.
2. **`overflow_hidden()`** on the capped shell (the pill, toast, row chrome).
3. **`flex_shrink_0()`** on icons, badges, kebab handles — only the text
   column shrinks.

## Multi-line body text (messages, toasts, error banners)

gpui uses the parent's **available width** as `wrap_width`. Feed a **Definite**
width per line:

```rust
let mut body = div().flex().flex_col().text_size(px(13.0));
for line in text.split('\n') {
    let display: SharedString = if line.is_empty() { " ".into() } else { line.into() };
    body = body.child(div().max_w(px(320.0)).child(display));
}
```

Reference: `SupportView::render_message` in `support_view.rs`.

**Do not** slap `w_full()` + `min_w(0)` on bubble text without testing —
it stretches the bubble past `max_w` and breaks right-alignment (see comment
in `render_message`).

## Single-line labels (toolbar, tabs, table cells)

- **`truncate()`** or **`line_clamp(1)`** when the label must stay on one row.
- Separate **label** from **metadata** (keyboard hint, badge) with distinct
  styling — see terminal `toolbar_btn` (`terminal_view.rs`).

## Scroll vs clip

| Need | Use |
|------|-----|
| Long thread / list | `overflow_y_scroll` + `min_h(0)` on the scroll column |
| Fixed chip / toast | `max_w` + wrap or `line_clamp(n)` — no scroll inside a toast |
| Horizontal tab strip | `overflow_x_scroll` at min width, not unbounded flex |

## Overlays (toasts, modals, dropdowns)

- Anchor container: `max_w` matching the child cap (`ToastContainer` → 420px).
- Stack from the edge with `items_end` but each child still needs its own
  `max_w` + overflow rules above.

## Centered modals (form overlays)

**Prefer adabraka `Dialog`** — see `.agents/ui-components.md`. It caps height at
~85% viewport and handles focus / backdrop / escape.

**Hand-rolled overlay (legacy only — do not add new ones):**

```rust
div() // backdrop: absolute inset-0, occlude, backdrop color
    .child(
        div()
            .flex()
            .flex_col()
            .w(px(500.0))
            .max_h(px(580.0)) // or relative(0.85) — MUST cap height
            .overflow_hidden()
            .child(header.flex_shrink_0())
            .child(
                div()
                    .flex_grow()
                    .min_h(px(0.0)) // required for flex scroll
                    .overflow_y_scroll()
                    .child(form_fields),
            )
            .child(footer.flex_shrink_0()),
    )
```

**Failure mode:** `items_center` + unbounded `flex_col` + many chip rows → modal
taller than the window → footer buttons clipped with no scroll. Happened on
`script_form.rs` ("New Script").

References: `variable_prompt.rs` (scroll body), `script_form.rs` (interim fix),
`patches/adabraka-ui/src/overlays/dialog.rs` (target).

## Dropdown / popover panels

- Use adabraka `Select` / `Combobox` with `.anchored().occlude()` — not
  `div().absolute()` without `.occlude()` (transparent bleed-through).

## Before shipping UI with dynamic text

Ask: *what is the longest realistic string?* (API error, file path, email, UUID)
If it can exceed ~40 chars, apply containment in the same PR — don't rely on
short demo copy.
