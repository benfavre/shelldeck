use gpui::prelude::*;
use gpui::{div, AnyWindowHandle, Context, Entity, Render, Subscription, Window};

use crate::command_palette::{action_opens_main_window, CommandPalette, CommandPaletteEvent};
use crate::theme::ShellDeckColors;

/// Borderless root for the system-wide ShellDeck command palette.
pub struct CommandPaletteWindowView {
    palette: Entity<CommandPalette>,
    font_family: Option<String>,
    activation_armed: bool,
    _palette_sub: Subscription,
    _activation_sub: Subscription,
}

impl CommandPaletteWindowView {
    pub fn new(
        palette: Entity<CommandPalette>,
        main_window: AnyWindowHandle,
        palette_window: AnyWindowHandle,
        font_family: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let palette_sub = cx.subscribe(
            &palette,
            move |_this, _palette, event: &CommandPaletteEvent, cx| {
                if matches!(
                    event,
                    CommandPaletteEvent::ActionSelected(_) | CommandPaletteEvent::Dismissed
                ) {
                    if let CommandPaletteEvent::ActionSelected(action) = event {
                        if action_opens_main_window(action.as_ref()) {
                            if let Err(error) = main_window.update(cx, |_, window, _| {
                                window.show_window();
                                window.activate_window();
                            }) {
                                tracing::warn!(
                                    error = %error,
                                    "command palette could not activate the main window"
                                );
                            }
                        }
                    }
                    if let Err(error) =
                        palette_window.update(cx, |_, window, _| window.remove_window())
                    {
                        tracing::debug!(
                            error = %error,
                            "command palette window was already closed"
                        );
                    }
                }
            },
        );
        let activation_sub = cx.observe_window_activation(window, |this, window, _cx| {
            if window.is_window_active() {
                this.activation_armed = true;
            } else {
                let should_close = this.activation_armed && window.is_window_visible();
                this.activation_armed = false;
                if should_close {
                    window.remove_window();
                }
            }
        });
        Self {
            palette,
            font_family,
            activation_armed: false,
            _palette_sub: palette_sub,
            _activation_sub: activation_sub,
        }
    }

    pub fn show(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.palette.update(cx, |palette, cx| {
            palette.show(window, cx);
        });
    }
}

impl Render for CommandPaletteWindowView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let escape_palette = self.palette.clone();
        let mut root = div()
            .size_full()
            .min_w_0()
            .min_h(gpui::px(0.0))
            .overflow_hidden()
            .bg(ShellDeckColors::bg_surface())
            .capture_key_down(move |event: &gpui::KeyDownEvent, window, cx| {
                if event.keystroke.key.eq_ignore_ascii_case("escape") {
                    escape_palette.update(cx, |palette, cx| palette.dismiss(cx));
                    window.remove_window();
                    cx.stop_propagation();
                }
            })
            .child(self.palette.clone());
        if let Some(font_family) = &self.font_family {
            root = root.font_family(font_family.clone());
        }
        root
    }
}
