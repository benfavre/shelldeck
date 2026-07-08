# Theming & colors

The user **switches the app theme at runtime** (Settings → Appearance, titlebar
picker, command palette). ShellDeck ships several built-in palettes (Dark, Light,
Solarized, …). **Never assume a fixed light or dark look** when picking colors.

## Two layers

| Layer | Config | Tokens |
|-------|--------|--------|
| **App chrome** (sidebar, forms, dashboards, adabraka widgets) | `AppConfig.theme` (`ThemePreference`) | `ShellDeckColors::*` |
| **Terminal grid** | `AppConfig.terminal.theme` | `TerminalTheme` / session grid colors |

UI work in `shelldeck-ui` is almost always **app chrome**. Do not hardcode terminal
ANSI hex in non-terminal views.

## Rules for new UI

1. **Custom GPUI (`div`, labels, borders)** → `ShellDeckColors` only:
   `bg_primary`, `bg_surface`, `bg_sidebar`, `text_primary`, `text_muted`,
   `border`, `primary`, `success`, `warning`, `error`, `hover_bg`, `selected_bg`,
   `hint_bg`, `backdrop`, …
2. **adabraka-ui** (`Select`, `Input`, `Button`, …) → prefer the component over
   hand-rolled widgets; its tokens are **re-synced on every theme change** via
   `adabraka_theme_from_palette()` in `theme.rs` / `Workspace::apply_palette`.
3. **Tinted surfaces** → opacity on a semantic color, not a raw hex:
   `ShellDeckColors::primary().opacity(0.12)`, `ShellDeckColors::error().opacity(0.1)`.
4. **Forbidden in view code** (except inside `theme.rs` palette definitions):
   - `rgb(0x…)`, `hsla(0, 0, 0.13, 1)` literals for backgrounds/text/borders
   - shadcn / GPUI default grays that ignore the active palette
   - copying a color from a screenshot of one theme
5. **Contrast** → test mentally on **both** a dark and a light built-in theme
   (e.g. Dark + Solarized Light). If a chip/badge only works on beige, fix the token.

## Switch flow (do not break)

```
User picks theme → Workspace::apply_palette / select_app_theme
  → ShellDeckColors::set_palette(...)
  → adabraka_ui::theme::init_theme(adabraka_theme_from_palette(), cx)
  → cx.refresh_windows()
```

New global UI state must not cache `Hsla` from the old palette across a theme switch.

## adabraka-ui preference

Dropdowns, inputs, buttons, dialogs, tabs: **use adabraka first** (see
`.agents/ui-components.md`). Hand-rolled menus often drift from the synced adabraka
theme and reintroduce “wrong theme” bugs after a palette change.

## adabraka accent tokens

- **`accent`** = teinte légère `primary` (pas le plein primary) ;
  **`accent_foreground`** = `text_primary` — lisible sur tous les thèmes.
- Surbrillance Select/Combobox : toujours `accent` + `accent_foreground` ensemble.

## Adding a new semantic color

1. Add field to `Palette` + derive it in `build()` / each `*_palette()` seed.
2. Add `ShellDeckColors::foo()` accessor.
3. Wire into `adabraka_theme_from_palette()` if adabraka widgets need it.
4. Never add a one-off color only in a single view file.
