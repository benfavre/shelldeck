# Icons — Lucide subset + legacy `images/`

ShellDeck does **not** bundle all of Lucide (~1 500 SVGs). We ship a curated
subset under `crates/shelldeck/assets/icons/lucide/` and embed only those
files into the binary.

**Canonical inventory + add procedure:** read
[`crates/shelldeck/assets/icons/lucide/README.md`](../crates/shelldeck/assets/icons/lucide/README.md)
before adding or renaming an icon.

## Runtime wiring (already done — do not re-invent)

- Boot: `adabraka_ui::set_icon_base_path("icons/lucide")` in
  `crates/shelldeck/src/main.rs`.
- Embedding: every shipped Lucide slug is listed in the `lucide_assets!(…)`
  macro in the same file (`include_bytes!` per SVG).
- adabraka-ui: `Icon::new("reply")` → `icons/lucide/reply.svg`.
- ShellDeck helper: `shelldeck_ui::icons::{lucide_icon, lucide_path}`.

```rust
use shelldeck_ui::icons::lucide_icon;

.child(lucide_icon("send", 14.0, ShellDeckColors::text_muted()))
```

When the icon must **inherit the parent `text_color`** (e.g. hover on a
wrapping div), prefer `svg().path(lucide_path("minus"))` instead of
`lucide_icon` with a hard-coded color.

> **But the `svg()` node still needs its own `text_color`.** GPUI's `svg.rs`
> **skips painting entirely** when `style.text.color` is `None` on the svg
> element itself — a colour set on an ancestor `div` is not enough, and the
> icon renders as nothing at all (no error, no warning, just an empty box).
> So `svg().path(lucide_path("x")).size(px(16.0)).text_color(some_color)` is
> the minimum; "inherits the parent" means you can *recompute* the colour to
> match the parent's hover state, not that you can omit it.
>
> **Incident (2026-08-06):** the assistant's activity rail shipped with
> `svg().path(lucide_path(icon)).size(…)` and the colour on the wrapping
> `div`. Every rail glyph was invisible — labels showed, icons did not. The
> warning was already written in `icons.rs` next to `script_language_icon`;
> this file did not repeat it.

## Where to use what

| Zone | Source | Notes |
|------|--------|-------|
| App views (Support, sidebar, forms, terminal tabs, …) | **Lucide** (`icons/lucide/…`) | Default for new UI icons |
| Script **language** chips / badges | **Simple Icons** (`icons/simple/…`) | Tech marks (Python, Docker, …) — slugs on `ScriptLanguage::simple_icon()` |
| Script **category** chips | **Lucide** | Semantic slugs on `ScriptCategory::lucide_icon()` |
| **Titlebar / window chrome** | **Legacy** `images/` | Minimize, maximize, restore, close, chevron site chip, ± UI scale — **do not migrate** unless explicitly asked |
| Brand / OIDC logos | `images/logo-*.svg`, `shelldeck-*.svg` | Multi-color or bespoke marks |
| Unpinned tab pin | `images/pin-outline.svg` | No Lucide slug in our subset yet |

## Adding a new icon (checklist)

1. Copy SVG into `crates/shelldeck/assets/icons/lucide/{slug}.svg`
   (from `.cache/lucide-upstream/` or lucide.dev — see README).
2. Add `{slug}` to `lucide_assets!(…)` in `main.rs`.
3. Update the inventory table in the Lucide README.
4. Use `lucide_icon` or `Icon::new("{slug}")` in the view.

Do **not** add new icons under `assets/images/` unless they are brand marks
or titlebar chrome. Do **not** import the full Lucide repo into git.

## Common mistakes

- Re-explaining adabraka `set_icon_base_path` from scratch — it's wired; read
  this file + the Lucide README.
- Migrating titlebar controls to Lucide — user preference is legacy glyphs.
- Using `lucide_icon` with a fixed color inside a hover-sensitive button —
  use `lucide_path` + `svg()` so `currentColor` tracks the parent.
- **Omitting `.text_color()` on a raw `svg()` node.** It does not fall back to
  the parent — it does not paint at all. See the callout above.
- Assuming adabraka's `Icon` inherits the parent colour: it does not either.
  `Icon::render` falls back to `theme.tokens.primary`, so an uncoloured `Icon`
  on a primary-filled button paints primary-on-primary and vanishes.
