# Application chrome — scaling, menu bar, sidebar rail

The three fixed surfaces that frame every mode: the **UI scale**, the
**application menu row**, and the **Dev sidebar**. They share one property —
the terminal grid's geometry is computed against them, so changing their size
silently mis-sizes every terminal unless the matching constant is updated too.

## 1. Proportional UI scale — who scales, who does not

The App Font Size setting drives `Window::set_rem_size`. A view participates by
shadowing GPUI's `px` with `crate::scale::px`, which returns **rems**:

```rust
use gpui::*;            // brings gpui::px via the glob
use crate::scale::px;   // shadows it within this file — everything is now rems
```

**Every view file that renders chrome must have that import.** `workspace/mod.rs`
did not until 2026-07, which is exactly why User mode (whose entire surface is
`Workspace::render_user_home`) ignored the setting while the sidebar and
Support view tracked it.

**Deliberately absolute (`gpui::px`) — do not convert:**

| Call site | Why |
|---|---|
| `Window::set_client_inset` | window chrome geometry, device pixels |
| `Window::set_rem_size` | it *is* the unit everything else resolves against |
| `BoxShadow { offset, blur_radius, spread_radius }` | typed `Pixels`; will not accept `Rems` |
| `resize_edge` / window-edge hitboxes / `Bounds` | screen-space hit-testing |
| `TerminalView` grid math, `sidebar_width` | see §3 |
| `sidebar::RAIL_WIDTH` and the rail's own icons | see §3 |
| adabraka `IconButton::size` / `Sheet::width` | upstream API takes `Pixels` |

**Known gap:** adabraka-ui sizes its own components in absolute `px` internally
(`ButtonSize::Sm => (px(36.0), …)` in `patches/adabraka-ui/src/components/button.rs`).
So adabraka widgets keep a fixed size at any UI scale while surrounding
hand-rolled chrome grows. Fixing it means converting ~2 500 call sites across
146 files in the vendored fork — a deliberate, separately-scoped change, not
something to start halfway through an unrelated task.

## 2. The application menu row

- Spec: `crates/shelldeck-ui/src/menu_bar.rs` — `menu_bar_spec(MenuBarContext)`
  is a **pure function** from app state to a `Vec<MenuSpec>`. No GPUI, no I/O,
  fully unit-tested. Put new menu logic here, not in the renderer.
- Row widget: adabraka `MenuBar` (`patches/adabraka-ui/src/navigation/menu.rs`),
  fixed by **SDPATCH-025** — upstream tracked `active_menu` but never rendered a
  dropdown for it.
- Wiring: `Workspace::rebuild_menu_bar` (called every render — the spec is cheap
  and the alternative is a dozen invalidation paths) and
  `Workspace::execute_menu_command`.

**Rules:**

- **Commands route through `execute_palette_action`** whenever an `actions!`
  entry exists, so the menu bar, the command palette and the keybindings stay
  one code path. Only terminal-owned actions (copy, paste, find, clear, splits,
  terminal zoom) are dispatched into the focus path with
  `window.dispatch_action`, because only the focused pane knows the selection.
- **Never show a command the account cannot reach.** Gate on the same
  predicates as `.agents/roles.md`. An empty or erroring menu item is worse
  than an absent one.
- **Icons must exist in the bundled Lucide subset** (`.agents/icons.md`). The
  subset is ~78 files, not all of Lucide — check before using a slug.
- **`MENU_BAR_HEIGHT` is consumed by `TerminalView::menu_bar_height`.** Change
  one, change the other, and keep the `menu_bar_visible` flag flowing into
  `TerminalView::set_menu_bar_visible`.
- Menu labels live in `locales/{fr,en}.toml` under `menu.*`. Note the TOML
  dotted-key trap: `menu.file` cannot be both a string and a table, which is
  why the titles are namespaced `menu.title.file`.

## 3. The Dev sidebar — rail + panel

VS Code layout, implemented in `crates/shelldeck-ui/src/sidebar.rs`:

```
[ rail 48px ][ panel 180–400px ][ content ]
   always        contextual, collapses via Cmd/Ctrl+B
```

**The rail lists activities, not destinations.** `SidebarSection::rail_activities()`
is the authoritative list. A section earns a rail slot when selecting it puts
something *in the panel*; JeanClaude, Fleet and bext Cloud are places you go,
reached from the Aller menu and the palette, and Settings is pinned separately
at the bottom. Adding a rail icon for a section with nothing behind it is the
mistake this list exists to prevent.

**The panel is contextual — it follows `active_section`.** Connections keeps
its bespoke renderer (groups, pins, per-row hover actions, site badges);
everything else feeds `set_panel_items(section, Vec<PanelItem>)` and renders
through the one generic row. `Workspace::refresh_sidebar_panels` builds those
rows per render, and `handle_panel_item_selected` routes a click back to the
existing entry point for that activity.

**Incident (2026-07-25):** the first cut of the rail wired `active_section` to
the *main view* only. The panel kept rendering the host list under a header
naming whatever activity was selected, so picking "Scripts" showed a header
reading SCRIPTS above a list of SSH hosts. A rail whose panel does not follow
it is not an activity bar — it is a row of buttons with a mislabelled list
attached. If you add a rail activity, add its panel rows in the same change.

`SidebarSection::has_panel()` marks the activities that have rows. One that
does not (Server Sync today) hides the panel entirely so its main view gets the
full width — and `total_width()` must account for that, which is why
`sidebar_total_width` takes `section_has_panel`.

- **`nav_collapsed` hides the rail** and restores the in-panel navigation list.
  The two never render together — the rail *is* the navigation. Toggled from a
  chevron in the **panel header** and from **Affichage → Barre d'activités**
  (`Workspace::toggle_activity_bar`).

  **Sidebar chrome controls belong in the panel header, not in the list.** The
  rail toggle first shipped as a full-width bordered strip sitting directly
  above the hosts list. Bordered top and bottom and glued to the `HÔTES`
  header, it read as that section's own collapse control rather than as
  sidebar chrome — and once the panel became contextual it appeared above every
  activity, offering to "hide the navigation" from the middle of a list of
  scripts. A control that acts on the whole sidebar goes in the header (or the
  View menu); a control that acts on a list goes in that list.

  The panel header names the active activity, so list-level headers must not
  repeat it: `HÔTES` under `CONNEXIONS` labelled the same list twice, and the
  `sidebar.hosts` key was retired with it.
- **`collapsed` hides the panel**, leaving the rail. This is what
  `ToggleSidebar` / `Workspace::toggle_sidebar` drives.
- **`SidebarView::total_width()` is the number the terminal needs** — rail plus
  panel, per their independent visibility. Never pass `width()` (the panel
  alone) to `TerminalView::set_sidebar_width`; that is how the grid ends up
  drawn underneath the rail. `sidebar_total_width` is extracted as a pure
  function so the arithmetic is unit-tested without a GPUI context.
- **The resize drag is in window space.** Subtract `rail_offset()` before
  clamping, or dragging jumps by the rail width.
- **The rail and its glyphs are absolute pixels on purpose.** The terminal grid
  is sized against the sidebar total; a rem-sized rail would have to thread the
  UI scale through every consumer, and a rem-sized 18px glyph would outgrow a
  fixed 48px rail at 2× and collide with its edges (`.agents/spacing.md`).

## Checklist when touching any of this

- [ ] Added a chrome row / changed a chrome height? → update the matching
      constant consumed by `TerminalView::content_area`.
- [ ] Changed sidebar composition? → `total_width()` still correct in all four
      rail/panel states, and the drag still subtracts `rail_offset()`.
- [ ] New view file rendering chrome? → `use crate::scale::px;` unless it is on
      the deliberately-absolute list above.
- [ ] New menu entry? → unique id, bundled icon slug, capability-gated, `fr` +
      `en` keys, and routed through `execute_palette_action` if an action exists.

## Related files

- `crates/shelldeck-ui/src/scale.rs` — the rem helpers.
- `crates/shelldeck-ui/src/menu_bar.rs` — menu spec + tests.
- `crates/shelldeck-ui/src/sidebar.rs` — rail, panel, width math + tests.
- `patches/adabraka-ui/PATCHES.md` — SDPATCH-025 (`MenuBar` dropdown).
- [`spacing.md`](spacing.md), [`overflow.md`](overflow.md),
  [`ui-components.md`](ui-components.md), [`icons.md`](icons.md).
