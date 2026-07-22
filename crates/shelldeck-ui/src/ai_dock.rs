use adabraka_ui::components::icon_source::IconSource;
use adabraka_ui::prelude::{Button, ButtonSize, ButtonVariant};
use gpui::prelude::*;
use gpui::{div, AnyWindowHandle, Context, Entity, Render, Subscription, Window};

use crate::ai_assistant::AiAssistantView;
use crate::scale::px;
use crate::t;
use crate::theme::ShellDeckColors;

pub fn dock_window_title() -> String {
    t!("ai.dock.title").to_string()
}

pub fn dock_tray_label() -> String {
    t!("ai.dock.tray_open").to_string()
}

/// Compact root view hosted by the screen-edge Assistant Dock.
///
/// The actual conversation surface remains `AiAssistantView`, shared with the
/// Workspace so requests, conversations and tasks survive while this window is
/// hidden. The native window has no system chrome and cannot move or resize;
/// this wrapper supplies its only close control.
pub struct AiDockView {
    assistant: Entity<AiAssistantView>,
    main_window: AnyWindowHandle,
    font_family: Option<String>,
    activation_armed: bool,
    _activation_sub: Subscription,
}

impl AiDockView {
    pub fn new(
        assistant: Entity<AiAssistantView>,
        main_window: AnyWindowHandle,
        font_family: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
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
            assistant,
            main_window,
            font_family,
            activation_armed: false,
            _activation_sub: activation_sub,
        }
    }

    pub fn focus_composer(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.assistant
            .update(cx, |assistant, cx| assistant.focus_composer(window, cx));
    }
}

impl Render for AiDockView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let main_window = self.main_window;
        let escape_assistant = self.assistant.clone();
        let toolbar = div()
            .flex()
            .items_center()
            .justify_between()
            .gap(px(8.0))
            .h(px(44.0))
            .px(px(12.0))
            .flex_shrink_0()
            .border_b_1()
            .border_color(ShellDeckColors::border())
            .bg(ShellDeckColors::bg_surface())
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(7.0))
                    .min_w_0()
                    .text_size(px(12.0))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(ShellDeckColors::text_primary())
                    .child(t!("ai.dock.title").to_string()),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .flex_shrink_0()
                    .child(
                        Button::new(
                            "ai-dock-open-shelldeck",
                            t!("ai.dock.open_shelldeck").to_string(),
                        )
                        .variant(ButtonVariant::Ghost)
                        .size(ButtonSize::Sm)
                        .icon(IconSource::from("external-link"))
                        .on_click(move |_, dock_window, cx| {
                            if let Err(error) = main_window.update(cx, |_, main_window, _| {
                                main_window.show_window();
                                main_window.activate_window();
                            }) {
                                tracing::warn!(
                                    error = %error,
                                    "AI Dock could not activate the main window"
                                );
                            }
                            dock_window.remove_window();
                        }),
                    )
                    .child(
                        Button::new("ai-dock-hide", "")
                            .variant(ButtonVariant::Ghost)
                            .size(ButtonSize::Sm)
                            .tooltip(t!("ai.dock.hide").to_string())
                            .icon(IconSource::from("x"))
                            .on_click(|_, window, _| window.remove_window()),
                    ),
            );

        let mut root = div()
            .id("ai-dock-root")
            .flex()
            .flex_col()
            .size_full()
            .min_w_0()
            .min_h(px(0.0))
            .overflow_hidden()
            .bg(ShellDeckColors::bg_primary())
            .capture_key_down(move |event: &gpui::KeyDownEvent, window, cx| {
                if event.keystroke.key.eq_ignore_ascii_case("escape")
                    && !escape_assistant.read(cx).has_open_dialog()
                {
                    window.remove_window();
                    cx.stop_propagation();
                }
            })
            .child(toolbar)
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_w_0()
                    .min_h(px(0.0))
                    .child(self.assistant.clone()),
            );
        if let Some(font_family) = &self.font_family {
            root = root.font_family(font_family.clone());
        }
        root
    }
}
