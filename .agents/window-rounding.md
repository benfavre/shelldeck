# Window rounding and overlay clipping

ShellDeck uses a transparent, client-decorated GPUI window. Its visible outer
shape is drawn by the Workspace root rather than by the operating system:

- floating window: `radius_xl`, a 1 px border and `overflow_hidden()`;
- maximized window: zero radius and no border.

This makes sheets and other absolute overlays a recurring trap. In GPUI, a
nested `absolute().inset_0()` element can paint with a rectangular clip even
when an ancestor is rounded and has `overflow_hidden()`. The result is a square
patch, a second arc or a dark wedge in one or more outer window corners.

## Correct overlay pattern

Apply the radius directly to every opaque layer that owns an outer corner. A
rounded host around a nested `absolute()` overlay is not sufficient: the child
can escape that host's rounded clip.

```rust
let mut backdrop = div()
    .absolute()
    .inset_0()
    .overflow_hidden();

if !is_maximized {
    backdrop = backdrop.rounded(use_theme().tokens.radius_xl);
}

let mut panel = div()
    .absolute()
    .top_0()
    .right_0()
    .bottom_0()
    .overflow_hidden();

if !is_maximized {
    // The opaque panel paints after the backdrop, so it owns the two right
    // corners and must repeat the exact same radius on those edges.
    panel = panel
        .rounded_tr(use_theme().tokens.radius_xl)
        .rounded_br(use_theme().tokens.radius_xl);
}

root = root.child(backdrop.child(panel));
```

The ownership is explicit: the backdrop owns the left corners; the opaque side
panel paints over the backdrop and therefore owns the right corners. Both use
the same theme token and the same outer bounds.

## Rules

1. Keep the Workspace root rounding in `workspace/render.rs`; it is still the
   canonical shape for ordinary chrome and content.
2. Any full-window absolute sheet/backdrop added at the Workspace root gets the
   floating-window radius directly on the element that paints the backdrop.
3. Use `use_theme().tokens.radius_xl`, never a duplicated raw radius. Every
   corner-owning layer must match the floating Workspace exactly.
4. Set no radius while maximized. A floating-only radius on a maximized window
   leaves transparent holes at the screen corners.
5. If an opaque panel reaches an outer edge, round the corners it owns directly
   (`rounded_tr`/`rounded_br` for a right-side sheet). A parent clip alone is
   not reliable for a nested absolute child in GPUI.
6. Do not give backdrop and panel different radius values or slightly different
   outer bounds; that creates a dark wedge between their geometries.
7. Keep shadows inside the clipped overlay. A shadow mounted outside it can paint
   over the transparent client inset and square the window again. In particular,
   do not use `shadow_xl()` or a non-inset `BoxShadow` on an edge-to-edge panel.
   Keep the existing border at the panel/content seam, or paint a deliberately
   internal seam shadow that cannot reach an exposed outer corner.
8. Apply the same rule to enter/exit animations: animate the panel position,
   not the radius or the outer overlay bounds.
9. A fixed footer with its own opaque background paints after its panel. If it
   reaches an outer window corner, repeat that corner's radius directly on the
   footer as well. The same ownership rule applies recursively to any opaque
   descendant that reaches the window edge.
10. **The opaque layer that reaches an outer corner owns it — count the
    ancestors, not the intentions.** A rounded ancestor with
    `overflow_hidden()` does *not* reliably clip an opaque descendant's fill,
    even two of them with the identical radius. Measured on 2026-08-20: the
    assistant composer sat inside a Sheet panel *and* a Sheet body, both
    `rounded_br(radius_xl)` and both `overflow_hidden()`, and its flat fill
    still covered the arc — only the window's own 1 px border survived.

    This file previously stated the opposite for that exact case, on the
    assumption that the panel clipped the composer. It did not. If a layer's
    background reaches the corner, give that layer the radius.

    The grey-triangle failure is real but narrower than it was written: it
    happens when the inner layer's radius or outer bounds differ from the
    owner's, cutting a second, larger arc and exposing the backdrop between
    them. Same radius token and same bounds produce one silhouette, not two.

### Removing the bottom chrome transfers corner ownership

The status bar is the bottom-most opaque layer in Dev mode and owns the two
bottom corners. UX-004 stopped mounting it in User, Support and the welcome
screen — which silently promoted each mode's own root to bottom-most layer
without giving it the radius. All three squared off the bottom of the window,
permanently, in every theme.

Nobody noticed for weeks because a flat bottom edge on a light surface over a
light desktop is nearly invisible. It only became obvious once the overlay
backdrops were fixed: a correctly rounded dark backdrop puts the square edge
underneath it in high contrast.

**How to apply:**

- **Whenever you add, remove or conditionally hide a full-width chrome row at
  the top or bottom of the window, ask which layer now touches that edge** and
  give it the radius. `shelldeck_ui::overlay::round_window_bottom` exists for
  exactly this and takes the maximized flag.
- **Fixing an overlay is not fixing the window.** A backdrop and the surface
  beneath it are two separate corner owners. After correcting one, re-measure
  with the other visible.
- `shelldeck_ui::overlay::window_backdrop` is the single remaining caller of
  `ShellDeckColors::backdrop()`. Build new full-window layers with it rather
  than recomposing the chain — seven of nine hand-written copies had dropped
  the radius.

### Measuring: take the background reference on the same row

The ramp method below is right, but the reference matters. Sampling one
background colour for the whole screenshot fails on a desktop with a gradient
wallpaper or another window nearby: corners get compared against a colour that
never appears next to them, and read as square when they are round — or, worse,
as round when they are flat.

Capture the window **plus a margin**, then for each row take the reference from
a pixel just outside the window **on that same row**, and raise the window above
any other app first. Measuring a second ShellDeck instance stacked on top of the
one under test produced a full set of green ramps for a window that was visibly
broken.

### Settle it by measuring, not by looking

A corner is eight pixels. Screenshot the window and print, for each of the
four corners, how many background pixels precede the first opaque one on each
of the first ten rows:

```
tl: [8, 6, 4, 3, 2, 2, 1, 1, 0, 0]   ← a real arc
br: [8, 6, 1, 1, 1, 1, 1, 1, 0, 0]   ← flat fill over the arc
```

A smooth decreasing ramp is a rounded corner. A ramp that collapses to a
constant is an opaque layer painting over it. All zeros is a square corner,
which is what a maximized window must show on all four. Comparing a suspect
corner against a known-good one on the same screenshot removes any argument
about theme, scale or anti-aliasing.

## Rounded image cards

GPUI image elements are independent paint layers and can escape a rounded
container just like absolute overlays. Apply the radius directly to the image.
For edge-to-edge thumbnails, keep one silhouette owner: no differently rounded
background, visible border or artificial inset underneath the bitmap. If a
design genuinely requires a border, treat that as a separate tested component;
do not simulate clipping by adding spacing around the media.

## Existing references

- User request composer/detail sheets:
  `crates/shelldeck-ui/src/workspace/request_views.rs::render_user_sheet`
  rounds the full backdrop plus the two opaque panel corners directly and has no
  exterior panel shadow.
- Connection form:
  `crates/shelldeck-ui/src/connection_form.rs::render` follows the same pattern.
- AI assistant sheets:
  `patches/adabraka-ui/src/overlays/sheet.rs::Sheet::render` rounds every opaque
  Assistant surface that owns a corner and omits the variant's exterior shadow.
  Its full-window backdrop directly owns all four corners; there is deliberately
  no extra rounded host wrapper in `workspace/render.rs`.

The AI sheet's entity wrapper works for that component's paint structure; do
not generalize it to hand-built nested absolute overlays. The User sheet is the
reference for those.

## Translucent rectangle outside a correct arc

When the opaque corner itself follows the correct curve but a faint square or
halo remains outside it, inspect the shadow before changing any radius. GPUI
paints a standard exterior shadow as its own rectangular layer outside the
element's rounded clip. Neither another `rounded_*()` nor `overflow_hidden()`
on that same panel clips the already-exterior paint.

Use this binary isolation sequence:

1. Render the full-window backdrop alone.
2. Add only the empty panel shell, without children, border or shadow.
3. Restore the real sheet content while keeping the shadow disabled.
4. Restore the shadow last. If the rectangle returns only here, remove that
   exterior shadow from the edge-to-edge variant; do not compensate with extra
   radii, padding or nested wrappers.

This sequence isolated the 16 px Assistant `BoxShadow`: backdrop, panel and
composer geometry were all correct, while the shadow alone recreated the
bottom-right translucent rectangle. An audit should then search specifically
for other `top_0().right_0().bottom_0()` panels with `shadow_xl()`/`.shadow()`;
centered dialogs, cards, menus and popovers are not the same case and keep their
normal elevation.

If removing the shadow leaves only a small grey triangle when a full-width
footer/composer is present, temporarily hide that descendant. A clean corner
without it indicates two competing rounded silhouettes rather than a missing
clip. Keep the outer Sheet corner owner and remove the redundant inner radius.

## Visual verification checklist

Test an overlay in all of these states before shipping:

- floating window: top-left, top-right, bottom-left and bottom-right;
- overlay open, opening animation and closing animation;
- maximized window: all corners remain square and edge-to-edge;
- after resizing the floating window;
- light and dark themes, where backdrop wedges have different contrast.

Do not validate only the panel edge. Capture or inspect all four outer corners:
the backdrop also covers the titlebar, content and status bar.
