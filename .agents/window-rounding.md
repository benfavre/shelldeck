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
   over the transparent client inset and square the window again.
8. Apply the same rule to enter/exit animations: animate the panel position,
   not the radius or the outer overlay bounds.
9. A fixed footer with its own opaque background paints after its panel. If it
   reaches an outer window corner, repeat that corner's radius directly on the
   footer as well. The same ownership rule applies recursively to any opaque
   descendant that reaches the window edge.

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
  rounds the full backdrop plus the two opaque panel corners directly.
- AI assistant sheet: the same file contains the earlier right-edge host fix.

The AI sheet's entity wrapper works for that component's paint structure; do
not generalize it to hand-built nested absolute overlays. The User sheet is the
reference for those.

## Visual verification checklist

Test an overlay in all of these states before shipping:

- floating window: top-left, top-right, bottom-left and bottom-right;
- overlay open, opening animation and closing animation;
- maximized window: all corners remain square and edge-to-edge;
- after resizing the floating window;
- light and dark themes, where backdrop wedges have different contrast.

Do not validate only the panel edge. Capture or inspect all four outer corners:
the backdrop also covers the titlebar, content and status bar.
