//! Keep the cached Projects pane independently resizable in every cockpit.
use super::*;

#[derive(Clone, Copy)]
pub(super) struct NavigatorResize {
    pointer_x: Pixels,
    width: f32,
    scale: f32,
}

impl AgentConsoleView {
    fn navigator_max_width(&self, window: &Window) -> f32 {
        let scale = window.rem_size().to_f64() as f32 / 16.0;
        let available = if self.tools.measured_width > 0.0 {
            self.tools.measured_width
        } else {
            window.viewport_size().width.to_f64() as f32
        } / scale;
        let reserve = if self.tools.navigator_wide {
            CONVERSATION_MIN_WIDTH
        } else {
            32.0
        };
        (available - reserve).clamp(180.0, 480.0)
    }

    pub(super) fn navigator_width(&self, window: &Window) -> f32 {
        f32::from(self.library.preferences.navigator_width)
            .clamp(180.0, 480.0)
            .min(self.navigator_max_width(window))
    }

    pub(super) fn finish_navigator_resize(&mut self, cx: &mut Context<Self>) {
        if self.tools.navigator_resize.take().is_some() {
            self.schedule_library_save(cx);
            cx.notify();
        }
    }

    fn resize_navigator(
        &mut self,
        event: &MouseMoveEvent,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        let Some(drag) = self.tools.navigator_resize else {
            return;
        };
        if event.pressed_button != Some(MouseButton::Left) {
            self.finish_navigator_resize(cx);
            return;
        }
        // A pointer delta avoids assumptions about the rail, client inset or
        // which retained workspace pane hosts this console. Preferences use rems.
        let delta = (event.position.x - drag.pointer_x).to_f64() as f32 / drag.scale;
        let width = (drag.width + delta)
            .clamp(180.0, self.navigator_max_width(window))
            .round() as u16;
        if self.library.preferences.navigator_width != width {
            self.library.preferences.navigator_width = width;
            cx.notify();
        }
    }

    pub(super) fn render_navigator_resize_handle(&self, cx: &mut Context<Self>) -> AnyElement {
        let moving = cx.entity().downgrade();
        let ending = moving.clone();
        // adabraka's split groups own proportional pane sizes. This edge keeps
        // our persisted logical width and cached navigator, including overlays.
        div()
            .id("agent-navigator-resize-handle")
            .debug_selector(|| "agent-navigator-resize-handle".into())
            .absolute()
            .right_0()
            .top_0()
            .bottom_0()
            .w(px(6.0))
            .occlude()
            .cursor_col_resize()
            .hover(|style| style.bg(ShellDeckColors::primary().opacity(0.35)))
            .when(self.tools.navigator_resize.is_some(), |handle| {
                handle.bg(ShellDeckColors::primary().opacity(0.5))
            })
            .tooltip(|_, cx| {
                cx.new(|_| super::navigator::NavigatorTooltip(t!("agents.navigator.resize").to_string()))
                    .into()
            })
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    if event.click_count == 2 {
                        this.tools.navigator_resize = None;
                        this.library.preferences.navigator_width =
                            shelldeck_core::agent_session::storage::AgentConsolePreferences::default()
                                .navigator_width;
                        this.schedule_library_save(cx);
                    } else {
                        this.tools.navigator_resize = Some(NavigatorResize {
                            pointer_x: event.position.x,
                            width: this.navigator_width(window),
                            scale: window.rem_size().to_f64() as f32 / 16.0,
                        });
                    }
                    cx.notify();
                }),
            )
            .child(
                canvas(
                    |_, _, _| (),
                    move |_, _, window, _| {
                        let moving = moving.clone();
                        window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
                            if phase.bubble()
                                && moving.read_with(cx, |this, _| this.tools.navigator_resize.is_some()).unwrap_or(false)
                            {
                                let _ = moving.update(cx, |this, cx| this.resize_navigator(event, window, cx));
                            }
                        });
                        let ending = ending.clone();
                        // Window events keep dragging and release working after
                        // the pointer crosses the pane edge or leaves the root.
                        window.on_mouse_event(move |event: &MouseUpEvent, phase, _, cx| {
                            if phase.bubble() && event.button == MouseButton::Left {
                                let _ = ending.update(cx, |this, cx| this.finish_navigator_resize(cx));
                            }
                        });
                    },
                )
                .absolute()
                .size_full(),
            )
            .into_any_element()
    }
}
