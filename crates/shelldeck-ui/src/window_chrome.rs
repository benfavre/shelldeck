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

pub(crate) fn resize_cursor_updates<T: 'static>(
    window: &Window,
    cx: &Context<T>,
) -> impl Fn(&MouseMoveEvent, &mut Window, &mut App) + 'static {
    let previous_edge = std::cell::Cell::new(resize_edge(
        window.mouse_position(),
        gpui::px(5.0),
        window.viewport_size(),
    ));
    let parent = cx.entity().downgrade();
    move |event: &MouseMoveEvent, window: &mut Window, cx: &mut App| {
        let edge = resize_edge(event.position, gpui::px(5.0), window.viewport_size());
        if previous_edge.replace(edge) != edge {
            let _ = parent.update(cx, |_, cx| cx.notify());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::Cell, rc::Rc};

    struct CachedContent(Rc<Cell<usize>>);
    impl Render for CachedContent {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            self.0.set(self.0.get() + 1);
            div().size_full()
        }
    }
    struct ResizeProbe(Entity<CachedContent>, Rc<Cell<usize>>);
    impl Render for ResizeProbe {
        fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            self.1.set(self.1.get() + 1);
            div()
                .relative()
                .size_full()
                .child(AnyView::from(self.0.clone()).cached(StyleRefinement::default().size_full()))
                .child(resize_cursor_layer())
                .on_mouse_move(resize_cursor_updates(window, cx))
        }
    }

    // SDTEST-2879 / SDUC-564 — pointer motion and resize-cursor changes retain expensive content.
    #[::core::prelude::v1::test]
    fn window_resize_pointer_updates_preserve_cached_content() {
        let mut app = TestAppContext::single();
        let content_renders = Rc::new(Cell::new(0));
        let root_renders = Rc::new(Cell::new(0));
        let counts = (content_renders.clone(), root_renders.clone());
        let handle =
            app.add_window(move |_, cx| ResizeProbe(cx.new(|_| CachedContent(counts.0)), counts.1));
        let root = handle.root(&mut app).unwrap();
        let mut window = VisualTestContext::from_window(*handle, &app);
        window.simulate_resize(size(gpui::px(640.0), gpui::px(480.0)));
        for _ in 0..3 {
            root.update(&mut app, |_, cx| cx.notify());
            window.run_until_parked();
        }
        let before = (content_renders.get(), root_renders.get());
        assert!(before.0 > 0);
        for (x, y) in [
            (100.0, 100.0),
            (200.0, 200.0),
            (2.0, 2.0),
            (2.0, 100.0),
            (200.0, 200.0),
        ] {
            window.simulate_mouse_move(point(gpui::px(x), gpui::px(y)), None, Modifiers::none());
            window.run_until_parked();
        }
        assert!(
            root_renders.get() > before.1,
            "edge transitions must update the cursor layer"
        );
        assert_eq!(
            content_renders.get(),
            before.0,
            "cursor changes must preserve cached transcript paint"
        );
    }
}
