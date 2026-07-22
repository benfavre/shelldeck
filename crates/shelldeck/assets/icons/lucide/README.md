# Lucide icons (ShellDeck subset)

Curated [Lucide](https://lucide.dev/) SVGs embedded in the binary. We ship
**only** the icons we use or expect to use soon — not the full ~1 500-icon set.

**License:** Lucide is [ISC](https://github.com/lucide-icons/lucide/blob/main/LICENSE).
Attribution: “Icons by Lucide” (https://lucide.dev).

## Layout

```
crates/shelldeck/assets/icons/lucide/
├── README.md          ← this file (inventory + how to add)
├── reply.svg          ← one file per icon, kebab-case name = Lucide slug
└── …
```

Runtime path (GPUI `AssetSource` + adabraka-ui): `icons/lucide/{name}.svg`.

At boot, `main.rs` calls `adabraka_ui::set_icon_base_path("icons/lucide")`, so
`Icon::new("reply")` resolves to `icons/lucide/reply.svg`.

## How to add an icon

1. **Pick the slug** on [lucide.dev](https://lucide.dev/icons) (e.g. `paperclip`).
2. **Copy the SVG** from the upstream repo (preferred — keeps stroke width
   consistent):

   ```bash
   git clone --depth=1 https://github.com/lucide-icons/lucide.git .cache/lucide-upstream
   cp .cache/lucide-upstream/icons/paperclip.svg crates/shelldeck/assets/icons/lucide/
   ```

   Or curl: `curl -sSL -o crates/shelldeck/assets/icons/lucide/paperclip.svg \
   https://raw.githubusercontent.com/lucide-icons/lucide/main/icons/paperclip.svg`

3. **Register it** in `crates/shelldeck/src/main.rs` — add the slug to the
   `lucide_assets!(…)` macro (one string per line). Rebuild; the SVG is
   `include_bytes!`’d into the binary.
4. **Document it** in the inventory table below (`reserved` until wired in UI).
5. **Use it** via `shelldeck_ui::icons::lucide_icon("paperclip", 14.0, color)` or
   `Icon::new("paperclip")` from adabraka-ui.

Do **not** commit `.cache/lucide-upstream/` (see root `.gitignore`).

## Usage in views

```rust
use shelldeck_ui::icons::lucide_icon;
use crate::theme::ShellDeckColors;

// …
.child(lucide_icon("refresh-cw", 12.0, ShellDeckColors::text_muted()))
```

Direct `svg().path("icons/lucide/reply.svg")` also works when you do not need
the `Icon` component API.

## Legacy `images/` mapping

These bespoke mono SVGs under `assets/images/` are **candidates for migration**
to Lucide. Brand marks and OIDC logos stay in `images/`.

| Legacy `images/…`   | Lucide slug            | Status        |
|---------------------|------------------------|---------------|
| `search.svg`        | `search`               | **migrated**  |
| `kebab.svg`         | `ellipsis-vertical`    | **migrated**  |
| `close.svg`         | `x`                    | **migrated** (except titlebar → keep `images/close.svg`) |
| `plus.svg`          | `plus`                 | **migrated** (except titlebar ± scale → keep `images/`) |
| `minus.svg`         | `minus`                | **migrated** (except titlebar ± scale → keep `images/`) |
| `chevron-down.svg`  | `chevron-down`         | **migrated** (except titlebar site chip → keep `images/`) |
| `refresh.svg`       | `refresh-cw`           | **migrated**  |
| `pin.svg`           | `pin`                  | **migrated**  |
| `pin-outline.svg`   | —                      | keep legacy   |
| `external-link.svg` | `external-link`        | reserved      |
| `minimize.svg`      | `minimize-2`           | **titlebar only** — keep `images/` |
| `maximize.svg`      | `maximize-2`           | **titlebar only** — keep `images/` |
| `restore.svg`       | —                      | keep legacy   |

## Inventory (76 icons)

Slug is the filename without `.svg`. **Category** is for humans only.

| Slug | Category | Used / reserved for |
|------|----------|---------------------|
| `activity` | dashboard | recent activity section header |
| `archive` | action | archive an AI conversation |
| `archive-restore` | action | restore an archived AI conversation |
| `arrow-down` | navigation | scroll / sort down |
| `arrow-left-right` | infra | port forwards |
| `arrow-right` | navigation | AI proposal transitions and next actions |
| `arrow-up` | navigation | scroll / sort up |
| `bot` | AI | Jean dispatch action |
| `calendar` | time | date pickers, due dates |
| `check` | action | confirm, done |
| `check-check` | action | read receipts, double-check |
| `chevron-down` | navigation | dropdowns, collapse (sidebar nav) |
| `chevron-left` | navigation | back, pagination |
| `chevron-right` | navigation | forward, pagination |
| `chevron-up` | navigation | expand |
| `circle-alert` | status | warnings (toast, inline) |
| `circle-check` | status | success state |
| `circle-help` | status | help tooltips |
| `clock` | time | timestamps, SLA |
| `cloud` | infra | bext Cloud nav |
| `copy` | action | copy to clipboard |
| `database` | infra | DB connections, sync |
| `download` | action | export, pull |
| `ellipsis` | chrome | horizontal overflow menu |
| `ellipsis-vertical` | chrome | row kebab menus (sidebar, lists) |
| `external-link` | navigation | open in browser |
| `eye` | action | show password / preview |
| `eye-off` | action | hide password |
| `filter` | support | ticket list filters |
| `flag` | support | priority / flag ticket |
| `globe` | infra | sites, public URL |
| `grid-2x2` | view | Sites view — cards mode toggle |
| `inbox` | support | ticket queue empty state |
| `info` | status | info banners |
| `key` | ssh | SSH keys, credentials |
| `keyboard` | dashboard | shortcuts section header |
| `list-checks` | AI | assistant tasks tab and empty state |
| `lock` | security | locked / auth required |
| `mail` | support | email channel |
| `maximize-2` | window | maximize (titlebar) |
| `messages-square` | AI | assistant discussions tab |
| `minimize-2` | window | minimize (titlebar) |
| `minus` | chrome | zoom out, decrement |
| `pencil` | action | edit |
| `pin` | terminal | pin tab |
| `play` | action | execute AI actions and scripts |
| `plus` | chrome | add, zoom in |
| `refresh-cw` | action | **Support refresh**, reload lists |
| `reply` | support | reply composer mode |
| `route` | AI | dispatch an action to Jean or the fleet |
| `rotate-ccw` | action | resume an AI task |
| `scan` | support | interactive screen-area capture |
| `search` | chrome | search inputs |
| `scroll-text` | chrome | Dev sidebar — Scripts |
| `send` | support | send message / reply |
| `server` | ssh | host, fleet instance |
| `settings` | chrome | settings entry |
| `shield` | security | admin / super-admin |
| `shield-check` | security | Sites — SSL-enabled row indicator |
| `sparkles` | AI | assistant actions and AI surfaces |
| `square` | action | stop a running AI task |
| `sticky-note` | support | internal note mode |
| `table` | view | Sites view — table mode toggle |
| `tag` | support | tags, labels |
| `terminal` | ssh | terminal tabs |
| `trash-2` | action | delete / destroy |
| `triangle-alert` | status | errors |
| `upload` | action | import, push |
| `user` | account | single user |
| `user-check` | support | assignee, agent |
| `users` | support | agents list, assign picker |
| `x` | chrome | close, dismiss |
| `zap` | dashboard | quick-connect section header |
