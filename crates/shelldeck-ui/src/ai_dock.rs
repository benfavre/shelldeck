use adabraka_ui::prelude::use_theme;
use gpui::prelude::*;
use gpui::{div, Context, Entity, Render, Subscription, Window};

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrayLabels {
    pub show: String,
    pub assistant: String,
    pub clippy: String,
    pub palette: String,
    pub choose_character: String,
    pub pause_character: String,
    pub return_character: String,
    pub quit: String,
    pub pinned: String,
    pub no_pinned: String,
}

impl TrayLabels {
    pub fn localized() -> Self {
        Self {
            show: t!("tray.show").to_string(),
            assistant: dock_tray_label(),
            clippy: t!("tray.clippy").to_string(),
            palette: t!("tray.palette").to_string(),
            choose_character: t!("tray.character.choose").to_string(),
            pause_character: t!("tray.character.pause").to_string(),
            return_character: t!("tray.character.return_to_dock").to_string(),
            quit: t!("tray.quit").to_string(),
            pinned: t!("tray.pinned").to_string(),
            no_pinned: t!("tray.no_pinned").to_string(),
        }
    }
}

impl Default for TrayLabels {
    fn default() -> Self {
        Self::localized()
    }
}

pub fn tray_counter_ssh(n: usize) -> String {
    match n {
        0 => t!("tray.counter.ssh.zero").to_string(),
        1 => t!("tray.counter.ssh.one").to_string(),
        n => t!("tray.counter.ssh.many", count = n).to_string(),
    }
}

pub fn tray_counter_tunnels(n: usize) -> String {
    match n {
        0 => t!("tray.counter.tunnels.zero").to_string(),
        1 => t!("tray.counter.tunnels.one").to_string(),
        n => t!("tray.counter.tunnels.many", count = n).to_string(),
    }
}

pub fn tray_counter_tickets(n: usize) -> String {
    match n {
        0 => t!("tray.counter.tickets.zero").to_string(),
        1 => t!("tray.counter.tickets.one").to_string(),
        n => t!("tray.counter.tickets.many", count = n).to_string(),
    }
}

pub fn tray_counter_jean(n: usize) -> String {
    match n {
        0 => t!("tray.counter.jean.zero").to_string(),
        1 => t!("tray.counter.jean.one").to_string(),
        n => t!("tray.counter.jean.many", count = n).to_string(),
    }
}

pub fn tray_counter_ai_tasks(n: usize) -> String {
    match n {
        0 => t!("tray.counter.ai_tasks.zero").to_string(),
        1 => t!("tray.counter.ai_tasks.one").to_string(),
        n => t!("tray.counter.ai_tasks.many", count = n).to_string(),
    }
}

/// Compact root view hosted by the screen-edge Assistant Dock.
///
/// The actual conversation surface remains `AiAssistantView`, shared with the
/// Workspace so requests, conversations and tasks survive while this window is
/// hidden. The native window has no system chrome and cannot move or resize;
/// this wrapper owns only the native-window lifecycle and exposed corners. The
/// shared assistant view supplies the Dock's single header inside its content
/// column, leaving the activity rail full-height as specified by the prototype.
pub struct AiDockView {
    assistant: Entity<AiAssistantView>,
    font_family: Option<String>,
    activation_armed: bool,
    _activation_sub: Subscription,
}

impl AiDockView {
    pub fn new(
        assistant: Entity<AiAssistantView>,
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
    fn render(&mut self, window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        // Match the floating main window's native client geometry. The Dock
        // stays flush with the screen on the right; its GPUI chrome only rounds
        // the two exposed corners on the left.
        window.set_client_inset(gpui::px(5.0));
        let corner_radius = use_theme().tokens.radius_xl;
        let escape_assistant = self.assistant.clone();

        let mut root = div()
            .id("ai-dock-root")
            .flex()
            .flex_col()
            .size_full()
            .min_w_0()
            .min_h(px(0.0))
            .overflow_hidden()
            .rounded_tl(corner_radius)
            .rounded_bl(corner_radius)
            .border_l_1()
            .border_t_1()
            .border_b_1()
            .border_color(ShellDeckColors::border())
            .bg(ShellDeckColors::bg_primary())
            .capture_key_down(move |event: &gpui::KeyDownEvent, window, cx| {
                if event.keystroke.key.eq_ignore_ascii_case("escape")
                    && !escape_assistant.read(cx).has_open_dialog()
                {
                    window.remove_window();
                    cx.stop_propagation();
                }
            })
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
