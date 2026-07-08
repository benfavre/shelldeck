use crate::scale::px;
use crate::theme::ShellDeckColors;
use gpui::prelude::*;
use gpui::*;

/// Monolith logo — square PNG (`shelldeck-icon.png`, logo variant, not dock rounding).
/// GPUI `svg()` is monochrome-only; multi-color marks use a raster asset.
pub fn brand_badge(size: f32) -> impl IntoElement {
    div().flex_shrink_0().w(px(size)).h(px(size)).child(
        img("images/shelldeck-icon.png")
            .w_full()
            .h_full()
            .object_fit(ObjectFit::Contain),
    )
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
