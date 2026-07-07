//! Lucide icon helpers.
//!
//! SVGs live in `crates/shelldeck/assets/icons/lucide/` and are embedded at
//! startup (`shelldeck::Assets` + `set_icon_base_path("icons/lucide")`).
//! See that directory's `README.md` for the inventory and add procedure.

use adabraka_ui::components::icon::{Icon, IconSize};
use gpui::{prelude::*, SharedString, *};

/// GPUI asset path for `svg().path(lucide_path("x"))` — inherits parent
/// `text_color` (handy for hover states on a wrapping div).
pub fn lucide_path(name: &str) -> SharedString {
    SharedString::from(format!("icons/lucide/{name}.svg"))
}

/// Render a named Lucide icon at `size_px` logical pixels, tinted with `color`.
pub fn lucide_icon(name: &str, size_px: f32, color: Hsla) -> impl IntoElement {
    Icon::new(name)
        .size(IconSize::Custom(px(size_px)))
        .color(color)
}
