//! Terminal start surface. Auto margins center fitting content and collapse to
//! zero when it overflows, keeping the top reachable in short windows.
use super::*;
use crate::command_palette::ToggleCommandPalette;
use crate::icons::lucide_icon;
use crate::monolith::{animated_monolith, MonolithMotion};
use crate::scale::px;
use crate::shortcut_reference::{
    display_accelerator, render_shortcut_rows, shortcuts_for, ShortcutGroup, ShortcutSurface,
    NEW_TERMINAL_BINDING,
};
use crate::workspace::{OpenAutomations, OpenQuickConnect, OpenSettings};
use adabraka_ui::components::button::{Button, ButtonSize, ButtonVariant};
use adabraka_ui::components::kbd::{KBDSize, KBD};

impl TerminalView {
    // Button has one label, while these launchers need a logo, policy caption
    // and command/accelerator. Keep the existing custom launcher shape here.
    fn launcher_card(
        id: &'static str,
        title: String,
        caption: String,
        command: SharedString,
        accelerator: Option<String>,
        logo: AnyElement,
    ) -> Stateful<Div> {
        let mut footer = div().flex().items_center().justify_between().gap(px(12.0));
        if let Some(accelerator) = accelerator {
            footer = footer.child(KBD::new(accelerator).size(KBDSize::Sm));
        } else {
            footer = footer.child(
                div()
                    .text_size(px(11.0))
                    .font_family(use_theme().tokens.font_mono.clone())
                    .text_color(ShellDeckColors::text_muted())
                    .child(
                        command
                            .split_whitespace()
                            .next()
                            .unwrap_or_default()
                            .to_string(),
                    ),
            );
        }
        footer = footer.child(lucide_icon(
            "arrow-right",
            14.0,
            ShellDeckColors::text_muted(),
        ));
        div()
            .id(id)
            .debug_selector(move || id.to_string())
            .flex()
            .flex_col()
            .justify_between()
            .gap(px(12.0))
            .flex_1()
            .min_w(px(220.0))
            .max_w(px(300.0))
            .min_h(px(136.0))
            .p(px(16.0))
            .rounded(px(10.0))
            .bg(ShellDeckColors::bg_surface())
            .border_1()
            .border_color(ShellDeckColors::border())
            .cursor_pointer()
            .hover(|el| {
                el.border_color(ShellDeckColors::primary())
                    .bg(ShellDeckColors::hover_bg())
            })
            .active(|el| el.bg(ShellDeckColors::selected_bg()))
            .child(
                div().flex().items_center().gap(px(10.0)).child(logo).child(
                    div()
                        .min_w(px(0.0))
                        .text_size(px(14.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(ShellDeckColors::text_primary())
                        .child(title),
                ),
            )
            .child(
                div()
                    .min_w(px(0.0))
                    .text_size(px(12.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(caption),
            )
            .child(footer)
            .tooltip(move |_, cx| {
                cx.new(|_| TerminalToolbarTooltip {
                    label: command.clone(),
                })
                .into()
            })
    }

    pub(super) fn render_empty_state(&self, cx: &mut Context<Self>) -> impl IntoElement {
        if let Some(message) = self.required_cwd_unavailable_message() {
            return div()
                .id("terminal-workspace-folder-unavailable")
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .size_full()
                .p(px(24.0))
                .gap(px(8.0))
                .bg(ShellDeckColors::bg_primary())
                .child(
                    div()
                        .text_size(px(16.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(ShellDeckColors::text_primary())
                        .child(t!("terminal.empty.workspace_unavailable_title").to_string()),
                )
                .child(
                    div()
                        .max_w(px(620.0))
                        .text_center()
                        .text_size(px(12.0))
                        .text_color(ShellDeckColors::text_muted())
                        .child(message),
                );
        }

        let mut launchers = div()
            .flex()
            .flex_wrap()
            .justify_center()
            .gap(px(12.0))
            .w_full()
            .child(
                Self::launcher_card(
                    "empty-launch-terminal",
                    t!("terminal.empty.open_terminal").to_string(),
                    t!("terminal.empty.open_terminal_description").to_string(),
                    t!("terminal.empty.open_terminal_cmd").to_string().into(),
                    Some(display_accelerator(NEW_TERMINAL_BINDING)),
                    div()
                        .flex()
                        .items_center()
                        .justify_center()
                        .flex_shrink_0()
                        .size(px(30.0))
                        .rounded(px(8.0))
                        .bg(ShellDeckColors::primary().opacity(0.12))
                        .child(lucide_icon("terminal", 18.0, ShellDeckColors::primary()))
                        .into_any_element(),
                )
                .on_click(cx.listener(|this, _, window, cx| {
                    this.spawn_local_terminal(cx);
                    this.focus_handle.focus(window);
                    cx.emit(TerminalEvent::NewTabRequested);
                    cx.notify();
                })),
            );

        if self.claude_available {
            launchers = launchers.child(
                Self::launcher_card(
                    "empty-launch-claude",
                    t!("terminal.empty.launch_claude").to_string(),
                    t!("terminal.empty.launch_claude_caption").to_string(),
                    CLAUDE_CLI_COMMAND.into(),
                    None,
                    Self::claude_logo(30.0).into_any_element(),
                )
                .on_click(cx.listener(|this, _, window, cx| {
                    this.launch_claude(cx);
                    this.focus_handle.focus(window);
                })),
            );
        }
        if self.codex_available {
            launchers = launchers.child(
                Self::launcher_card(
                    "empty-launch-codex",
                    t!("terminal.empty.launch_codex").to_string(),
                    t!("terminal.empty.launch_codex_caption").to_string(),
                    CODEX_CLI_COMMAND.into(),
                    None,
                    Self::codex_logo(30.0).into_any_element(),
                )
                .on_click(cx.listener(|this, _, window, cx| {
                    this.launch_codex(cx);
                    this.focus_handle.focus(window);
                })),
            );
        }

        let quick_actions = div()
            .flex()
            .flex_wrap()
            .justify_center()
            .gap(px(8.0))
            .child(
                Button::new(
                    "terminal-empty-connect",
                    t!("terminal.empty.connect_ssh").to_string(),
                )
                .debug_selector(|| "terminal-empty-connect".into())
                .variant(ButtonVariant::Ghost)
                .size(ButtonSize::Sm)
                .icon("server")
                .on_click(cx.listener(|this, _, window, cx| {
                    // Actions bubble along the focused view's dispatch path.
                    this.focus_handle.focus(window);
                    window.dispatch_action(Box::new(OpenQuickConnect), cx);
                })),
            )
            .child(
                Button::new(
                    "terminal-empty-palette",
                    t!("shortcuts.command_palette").to_string(),
                )
                .debug_selector(|| "terminal-empty-palette".into())
                .variant(ButtonVariant::Ghost)
                .size(ButtonSize::Sm)
                .icon("search")
                .on_click(cx.listener(|this, _, window, cx| {
                    this.focus_handle.focus(window);
                    window.dispatch_action(Box::new(ToggleCommandPalette), cx);
                })),
            )
            .child(
                Button::new(
                    "terminal-empty-settings",
                    t!("shortcuts.settings").to_string(),
                )
                .debug_selector(|| "terminal-empty-settings".into())
                .variant(ButtonVariant::Ghost)
                .size(ButtonSize::Sm)
                .icon("settings")
                .on_click(cx.listener(|this, _, window, cx| {
                    this.focus_handle.focus(window);
                    window.dispatch_action(Box::new(OpenSettings), cx);
                })),
            );

        let quick_actions = quick_actions.child(
            Button::new(
                "terminal-empty-automations",
                t!("automations.tasks").to_string(),
            )
            .debug_selector(|| "terminal-empty-automations".into())
            .variant(ButtonVariant::Ghost)
            .size(ButtonSize::Sm)
            .icon("calendar")
            .on_click(cx.listener(|this, _, window, cx| {
                this.focus_handle.focus(window);
                window.dispatch_action(Box::new(OpenAutomations), cx);
            })),
        );

        let shortcuts = shortcuts_for(ShortcutSurface::TerminalEmpty, true);
        let mut columns = div().flex().flex_wrap().gap(px(24.0)).w_full();
        for (group, label) in [
            (ShortcutGroup::Navigation, "shortcuts.group.navigation"),
            (ShortcutGroup::Terminal, "shortcuts.group.terminal"),
        ] {
            columns = columns.child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w(px(240.0))
                    .gap(px(6.0))
                    .child(
                        div()
                            .text_size(px(11.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(ShellDeckColors::text_muted())
                            .child(t!(label).to_string()),
                    )
                    .child(render_shortcut_rows(
                        shortcuts.iter().copied().filter(|item| item.group == group),
                    )),
            );
        }

        div()
            .id("terminal-empty-scroll")
            .debug_selector(|| "terminal-empty-scroll".into())
            .track_focus(&self.focus_handle)
            .flex()
            .flex_col()
            .items_center()
            .size_full()
            .min_h(px(0.0))
            .overflow_y_scroll()
            .p(px(24.0))
            .bg(ShellDeckColors::bg_primary())
            .child(
                div()
                    .debug_selector(|| "terminal-welcome-content".into())
                    .flex()
                    .flex_col()
                    .items_center()
                    .flex_shrink_0()
                    .w_full()
                    .max_w(px(800.0))
                    .my_auto()
                    .gap(px(24.0))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .items_center()
                            .w_full()
                            .gap(px(10.0))
                            .child(animated_monolith(
                                "terminal-empty-typing",
                                72.0,
                                MonolithMotion::TerminalTyping,
                                cx,
                            ))
                            .child(
                                div()
                                    .text_size(px(26.0))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_center()
                                    .text_color(ShellDeckColors::text_primary())
                                    .child(t!("terminal.empty.welcome_title").to_string()),
                            )
                            .child(
                                div()
                                    .max_w(px(520.0))
                                    .text_size(px(13.0))
                                    .text_center()
                                    .text_color(ShellDeckColors::text_muted())
                                    .child(t!("terminal.empty.welcome_hint").to_string()),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .w_full()
                            .gap(px(12.0))
                            .child(launchers)
                            .child(quick_actions),
                    )
                    .child(
                        div()
                            .debug_selector(|| "terminal-empty-shortcuts".into())
                            .flex()
                            .flex_col()
                            .w_full()
                            .gap(px(12.0))
                            .p(px(20.0))
                            .rounded(px(10.0))
                            .border_1()
                            .border_color(ShellDeckColors::border())
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(8.0))
                                    .child(lucide_icon(
                                        "keyboard",
                                        16.0,
                                        ShellDeckColors::text_muted(),
                                    ))
                                    .child(
                                        div()
                                            .text_size(px(13.0))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(ShellDeckColors::text_primary())
                                            .child(t!("shortcuts.title").to_string()),
                                    ),
                            )
                            .child(columns),
                    ),
            )
    }
}

#[cfg(test)]
mod tests;
