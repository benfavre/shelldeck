# UI components

**Prefer `adabraka-ui` components over hand-rolled `div`s.** Only build a
custom widget when adabraka-ui genuinely has no equivalent. When you do
hand-roll, factor it into a reusable helper if the same shape appears (or
will obviously appear) in more than one place — otherwise leave it inline.

## Harmonization — same shape, same look

**When two surfaces do the same job, they render with the same widgets, in
the same layout.** If Support > Tickets has a filter bar (search input +
`IconButton` "filter" + count `Badge` + chips row of `compact_filter_button`
with `selected(active)`), Support > Demandes must use the *identical*
building blocks and the *identical* spacing/order. No visual re-invention
per surface.

**Incident (2026-07):** the Demandes filter bar shipped with a custom
rounded pill "Filtres" button and custom filled chip divs, next to the
tickets bar built from `IconButton` + adabraka `Badge` + `compact_filter_button`.
Two filter bars, two designs, one app. The user flagged it as
regression-worthy: *"il faut harmoniser les designs d'éléments similaires,
sauf demande exceptionnelle."*

**Why:** every re-invented shape is a visual drift, a maintenance fork,
and a re-review cost for the user. The mental model becomes "this looks
different, why?" — which is the wrong question to force on your reader.

**How to apply:**

- **Before building a new surface**, look for the closest sibling surface
  in `shelldeck-ui/src/` that does the same job (filter, list, kebab
  menu, form, empty state, badge …). Copy its structure and building
  blocks; don't design a fresh one.
- **If you catch yourself writing `div().px(…).py(…).rounded(…).bg(…)`
  for a chip / pill / button-like affordance**, stop and check whether
  the sibling surface uses an adabraka helper (`compact_filter_button`,
  `Badge`, `IconButton`, `Label`, `Checkbox`, …). Reuse that helper.
- **Container spacing (px/py/gap/border-b) must match too.** A filter bar
  is not just "input + chips" — the specific `px(10) pt(8) pb(6)` matters
  because it lines up with adjacent panes. Copy the exact values.
- **Exceptions require a spelled-out reason.** If your new surface
  genuinely can't reuse the sibling's shape (different information
  density, different interaction contract), leave a one-line comment
  next to the divergence explaining *why* — so the next reader doesn't
  paper over your decision with a drift-fix.
- **If the sibling itself is doing something ugly**, migrate the sibling
  first, then use the new shape in both places. Don't fork "the ugly
  one" and "the pretty one" — one hits both.

The rule generalizes: **filter bars, kebab menus, empty states, confirm
dialogs, section headers, sheet chrome, row hover actions — any element
that appears in ≥2 surfaces must share the exact same widgets.** When in
doubt, grep for the sibling.

**Why:** adabraka-ui already ships ~85 components with consistent theming,
keyboard handling, focus management, and hover/active states wired up. A
fresh `div` for a chip / menu / dialog quietly loses those behaviors and
drifts the visual language over time. An audit of the current codebase
found most chips, dropdowns, sheet chrome, section tabs, confirms, and
stat cards are custom `div`s — an existing source of inconsistency we
don't want to grow.

**How to apply:**

1. **Search first.** Before starting a `div().flex()...`, check whether
   adabraka-ui has the shape you need. Grep the vendored crate at
   `patches/adabraka-ui/`. High-signal names: `Button`, `Input`, `Toggle`,
   `Sheet`, `PopoverMenu`, `ContextMenu`, `Dialog`, `AlertDialog`,
   `ConfirmDialog`, `Select`, `Dropdown`, `Combobox`, `Tabs`, `Card`,
   `Badge`, `Alert`, `Separator`, `Checkbox`, `Radio`, `Tooltip`,
   `HoverCard`.
2. **Match, don't fork.** If adabraka's version is 90% right, extend it
   via its variant/config API or add the tweak inside
   `patches/adabraka-ui/`. Don't copy-paste it into a private variant in
   `shelldeck-ui`.
3. **Falling back to a hand-built widget is the exception, not the
   default.** When it happens, leave a one-line comment on the custom
   widget saying *why* adabraka couldn't cover it, so a future migration
   attempt doesn't waste the same investigation.
4. **Reusability is judgment-based, not automatic.**
   - Same custom shape used (or clearly about to be used) in ≥2 places →
     pull it into a small helper (a free function returning
     `impl IntoElement`, or a tiny `#[derive(IntoElement)]` component).
   - One-off flourish tied to a single view → leave it inline. No
     premature abstraction — three similar lines beat a bad abstraction.
5. **Genuinely missing shapes go upstream.** If a shape is missing and
   would be useful across the app, add it to `patches/adabraka-ui/`
   rather than duplicating a helper in `shelldeck-ui`. Treat the vendored
   crate as ours to extend.

## Modals & form overlays

**Incident (2026-07):** `ScriptForm` ("New Script") used a hand-rolled
`absolute()` backdrop + centered `div`. No `max_h`, no scroll body — language /
category chip rows + body editor pushed the footer off-screen.

**Rule:** centered modals (create/edit forms, variable prompts, login) →
**`adabraka_ui::overlays::dialog::Dialog`** (or `ConfirmDialog` for yes/no).
Side panels → **`Sheet`**. Do **not** add new `*-form-overlay` div stacks.

`Dialog` already ships focus trap, backdrop dismiss, escape, close button, and
**`max_h(relative(0.85))`** per `DialogSize` — the sizing bug we hit manually.

**If you must keep a legacy overlay temporarily** (large migration), follow
`.agents/overflow.md` § Centered modals — same PR, no exceptions. Add a
one-line `// TODO: migrate to adabraka Dialog` on the overlay root.

**Enum pickers (Language / Category / Target chips)** are fine as inline
`Badge` / toggle rows when the set is small and fixed — not every row of pills
needs a `Select`. Connection / server pickers **do** need `Select` / `Combobox`.

**Current migration backlog** (non-binding hints from the audit — where
the biggest wins live if you're already touching the area):

| Custom today | adabraka target | Notes |
|---|---|---|
| Centered form modals (`script_form`, `port_forward_form`, `variable_prompt`, `login_form`, `connection_form`) | `Dialog` | Prefer `DialogSize::Md` + scroll body; see `patches/adabraka-ui/src/overlays/dialog.rs` |
| Right-side sheet chrome (workspace + connection_form) | `Sheet` | |
| Chips / pills / status badges | `Badge` | Small fixed enums OK inline until touched |
| Kebabs, theme / account / site / mode / sidebar switchers | `PopoverMenu` / `ContextMenu` | |
| Destroy / delete confirms (bext, connections) | `ConfirmDialog` | `support_view` already uses `UiDialog` for some flows |
| ~~Connection pickers (port_forward, script_form, server_sync)~~ | ~~`Select` / `Combobox`~~ | **Done** — `connection_combobox.rs` + `Select` in server_sync |
| Support / Jean / Bext section tabs | `Tabs` | |
| OIDC buttons (login_form), toolbar chips (sites_view) | `Button` variants | |
| Dashboard stat cards | `Card` | |
