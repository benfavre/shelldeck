//! The frame of every User-mode screen (Accueil, Mes sites, Partage, Mes
//! demandes, Mes informations): the same margins, the same head (a small
//! eyebrow, the title, actions to the right) and the same gap between blocks,
//! so moving from one tab to another never shifts the content or changes the
//! type. Palette colours only.
use crate::{scale::px, theme::ShellDeckColors};
use gpui::{prelude::*, *};

/// Side margin of a User screen; [`PAGE_PAD_X_COMPACT`] in a narrow window.
pub const PAGE_PAD_X: f32 = 30.0;
pub const PAGE_PAD_X_COMPACT: f32 = 16.0;
/// Space above the head and below the last block.
pub const PAGE_PAD_TOP: f32 = 26.0;
pub const PAGE_PAD_BOTTOM: f32 = 24.0;
/// Space between the head and the blocks, and between two blocks.
pub const PAGE_GAP: f32 = 20.0;

/// Up to this logical width a User screen uses its compact margins.
const COMPACT_MAX_LOGICAL_WIDTH: f32 = 600.0;

/// Whether the window is narrow enough for compact margins, in logical
/// pixels: a doubled UI scale needs twice the width.
pub fn is_compact(window: &Window) -> bool {
    let scale = window.rem_size().to_f64() as f32 / crate::scale::REM_BASE;
    (window.viewport_size().width.to_f64() as f32) / scale.max(0.1) <= COMPACT_MAX_LOGICAL_WIDTH
}

pub fn pad_x(compact: bool) -> f32 {
    if compact {
        PAGE_PAD_X_COMPACT
    } else {
        PAGE_PAD_X
    }
}

/// The page column: margins and gap, the caller adds the head and blocks.
pub fn page(compact: bool) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(PAGE_GAP))
        .px(px(pad_x(compact)))
        .pt(px(PAGE_PAD_TOP))
        .pb(px(PAGE_PAD_BOTTOM))
}

/// Size of the title of a User screen.
pub fn title_size(compact: bool) -> f32 {
    if compact {
        24.0
    } else {
        28.0
    }
}

/// A run of the title in the title type; a title may be made of several runs
/// (the home page marks the name).
pub fn title_text(label: impl Into<SharedString>, compact: bool) -> Div {
    div()
        .text_size(px(title_size(compact)))
        .font_weight(FontWeight::EXTRA_BOLD)
        .text_color(ShellDeckColors::text_primary())
        .whitespace_nowrap()
        .child(label.into())
}

/// The head of a User screen: the eyebrow over the title, the actions to the
/// right (wrapping under the title when the window is narrow).
pub fn head(
    eyebrow: impl Into<SharedString>,
    title: impl IntoElement,
    actions: Option<AnyElement>,
) -> Div {
    div()
        .flex()
        .flex_wrap()
        .items_center()
        .justify_between()
        .gap(px(16.0))
        .flex_shrink_0()
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(6.0))
                .min_w(px(0.0))
                .child(
                    div()
                        .text_size(px(14.0))
                        .text_color(ShellDeckColors::text_muted())
                        .child(eyebrow.into()),
                )
                .child(title),
        )
        .children(actions.map(|actions| {
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap(px(10.0))
                .child(actions)
        }))
}

/// The title of a card or section inside a User screen.
pub fn section_title(label: impl Into<SharedString>) -> Div {
    div()
        .text_size(px(18.0))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(ShellDeckColors::text_primary())
        .child(label.into())
}
