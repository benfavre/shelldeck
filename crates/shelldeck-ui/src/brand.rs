use crate::scale::px;
use crate::theme::ShellDeckColors;
use gpui::prelude::*;
use gpui::SharedString;
use gpui::*;

/// Monolith logo — 128px PNG per theme (`brand/png/themes/monolith-{slug}-128.png`).
/// GPUI `svg()` is monochrome-only; multi-color marks use a raster asset.
/// Falls back to the dark badge for unknown slugs (should be unreachable —
/// slugs are enumerated in `ThemePreference::brand_slug`).
pub fn brand_badge(size: f32) -> impl IntoElement {
    let slug = ShellDeckColors::palette_slug();
    let path: SharedString = format!("images/brand/png/themes/monolith-{slug}-128.png").into();
    div()
        .flex_shrink_0()
        .w(px(size))
        .h(px(size))
        .child(img(path).w_full().h_full().object_fit(ObjectFit::Contain))
}

/// Same mark at an **absolute** pixel size, for fixed-width chrome that must
/// not grow with the UI scale — currently the sidebar activity rail, whose
/// width the terminal grid geometry depends on.
pub fn brand_badge_abs(size: f32) -> impl IntoElement {
    let slug = ShellDeckColors::palette_slug();
    let path: SharedString = format!("images/brand/png/themes/monolith-{slug}-128.png").into();
    div()
        .flex_shrink_0()
        .w(gpui::px(size))
        .h(gpui::px(size))
        .child(img(path).w_full().h_full().object_fit(ObjectFit::Contain))
}

/// Monochrome Monolith mark (`shelldeck-mark.svg`) — muted contexts, `currentColor`.
pub fn brand_mark(width: f32, height: f32) -> impl IntoElement {
    svg()
        .path("images/shelldeck-mark.svg")
        .w(px(width))
        .h(px(height))
        .flex_shrink_0()
        .text_color(ShellDeckColors::text_muted())
}

/// Absolute-pixel variant for fixed chrome such as the Dev activity rail.
pub fn brand_mark_abs(width: f32, height: f32) -> impl IntoElement {
    svg()
        .path("images/shelldeck-mark.svg")
        .w(gpui::px(width))
        .h(gpui::px(height))
        .flex_shrink_0()
        .text_color(ShellDeckColors::text_muted().opacity(0.72))
}

/// The ShellDeck wordmark — "Shell" in the primary text color, "Deck" in the
/// brand accent color. `text_size` sets the point size of both halves.
pub fn brand_wordmark(text_size: f32) -> impl IntoElement {
    div()
        .flex()
        .items_baseline()
        .child(
            div()
                .text_size(px(text_size))
                .font_weight(FontWeight::BOLD)
                .text_color(ShellDeckColors::text_primary())
                .child("Shell"),
        )
        .child(
            div()
                .text_size(px(text_size))
                .font_weight(FontWeight::BOLD)
                .text_color(ShellDeckColors::primary())
                .child("Deck"),
        )
}
