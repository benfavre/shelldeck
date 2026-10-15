//! Shared client-side window controls. These use the same legacy glyphs as the
//! main titlebar; adabraka buttons do not expose native window-control regions.
use crate::{scale::px, theme::ShellDeckColors};
use gpui::{prelude::*, *};

pub(crate) fn control_button(
    id: &'static str,
    icon_path: &'static str,
    area: WindowControlArea,
    danger: bool,
) -> Stateful<Div> {
    let hover_bg = if danger {
        ShellDeckColors::error()
    } else {
        ShellDeckColors::hover_bg()
    };
    let group_name = SharedString::from(format!("ctrl-{id}"));
    div()
        .id(id)
        .group(group_name.clone())
        .flex()
        .items_center()
        .justify_center()
        .size(px(28.0))
        .rounded(px(6.0))
        .hover(move |s| s.bg(hover_bg))
        .window_control_area(area)
        .child(
            svg()
                .path(icon_path)
                .size(px(12.0))
                .text_color(ShellDeckColors::text_muted())
                .group_hover(group_name, |s| s.text_color(white())),
        )
}

pub(crate) fn resize_edge(
    pos: Point<Pixels>,
    border: Pixels,
    size: Size<Pixels>,
) -> Option<ResizeEdge> {
    match (
        pos.x < border,
        pos.x > size.width - border,
        pos.y < border,
        pos.y > size.height - border,
    ) {
        (true, _, true, _) => Some(ResizeEdge::TopLeft),
        (_, true, true, _) => Some(ResizeEdge::TopRight),
        (true, _, _, true) => Some(ResizeEdge::BottomLeft),
        (_, true, _, true) => Some(ResizeEdge::BottomRight),
        (_, _, true, _) => Some(ResizeEdge::Top),
        (_, _, _, true) => Some(ResizeEdge::Bottom),
        (true, _, _, _) => Some(ResizeEdge::Left),
        (_, true, _, _) => Some(ResizeEdge::Right),
        _ => None,
    }
}

pub(crate) fn resize_cursor_layer() -> impl IntoElement {
    canvas(
        |_bounds, window, _cx| {
            window.insert_hitbox(
                Bounds::new(point(gpui::px(0.0), gpui::px(0.0)), window.viewport_size()),
                HitboxBehavior::Normal,
            )
        },
        |_bounds, hitbox, window, _cx| {
            if let Some(edge) = resize_edge(
                window.mouse_position(),
                gpui::px(5.0),
                window.viewport_size(),
            ) {
                window.set_cursor_style(
                    match edge {
                        ResizeEdge::Top | ResizeEdge::Bottom => CursorStyle::ResizeUpDown,
                        ResizeEdge::Left | ResizeEdge::Right => CursorStyle::ResizeLeftRight,
                        ResizeEdge::TopLeft | ResizeEdge::BottomRight => {
                            CursorStyle::ResizeUpLeftDownRight
                        }
                        ResizeEdge::TopRight | ResizeEdge::BottomLeft => {
                            CursorStyle::ResizeUpRightDownLeft
                        }
                    },
                    &hitbox,
                );
            }
        },
    )
    .size_full()
    .absolute()
}
