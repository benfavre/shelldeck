use crate::scale::px;
use crate::theme::ShellDeckColors;
use gpui::prelude::*;
use gpui::*;

/// The ShellDeck brand badge — a rounded square in the theme's primary color
/// with a bold white `>_` prompt glyph. Single source of truth: use this in
/// the titlebar, sidebar, About screen, and anywhere else the mark appears.
///
/// `size` drives every proportion (rounding, glyph size, shadow), so the mark
/// stays visually coherent across scales.
pub fn brand_badge(size: f32) -> impl IntoElement {
    let mut inner = div()
        .text_size(px(size * 0.5))
        .font_weight(FontWeight::BOLD)
        .text_color(gpui::white())
        .child(">_");

    // Small badges (< 22px) don't have room for the shadow to read cleanly.
    let mut badge = div()
        .flex()
        .items_center()
        .justify_center()
        .w(px(size))
        .h(px(size))
        .rounded(px(size * 0.25))
        .bg(ShellDeckColors::primary());
    if size >= 24.0 {
        badge = badge.shadow_sm();
        inner = inner.line_height(px(size * 0.6));
    }
    badge.child(inner)
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
