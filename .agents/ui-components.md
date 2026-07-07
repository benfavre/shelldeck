# UI components

**Prefer `adabraka-ui` components over hand-rolled `div`s.** Only build a
custom widget when adabraka-ui genuinely has no equivalent. When you do
hand-roll, factor it into a reusable helper if the same shape appears (or
will obviously appear) in more than one place — otherwise leave it inline.

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

**Current migration backlog** (non-binding hints from the audit — where
the biggest wins live if you're already touching the area):

| Custom today | adabraka target |
|---|---|
| Chips / pills / status badges | `Badge` |
| Kebabs, theme / account / site / mode / sidebar switchers | `PopoverMenu` / `ContextMenu` |
| Destroy / delete confirms (bext, connections) | `ConfirmDialog` |
| Connection pickers (port_forward, script_form) | `Select` / `Combobox` |
| Support / Jean / Bext section tabs | `Tabs` |
| Right-side sheet chrome (workspace + connection_form) | `Sheet` |
| OIDC buttons (login_form), toolbar chips (sites_view) | `Button` variants |
| Dashboard stat cards | `Card` |
