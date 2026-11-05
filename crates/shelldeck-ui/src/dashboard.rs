use crate::scale::px;
use adabraka_ui::display::card::Card;
use gpui::prelude::*;
use gpui::*;
use shelldeck_core::config::activity::{ActivityEntry, ActivityKind};

use crate::icons::lucide_icon;
use crate::shortcut_reference::{
    render_shortcut_rows, shortcuts_for, ShortcutGroup, ShortcutSurface,
};
use crate::t;
use crate::theme::ShellDeckColors;

pub struct DashboardView {
    pub active_terminals: usize,
    pub recent_activity: Vec<ActivityEntry>,
}

impl Default for DashboardView {
    fn default() -> Self {
        Self::new()
    }
}

impl DashboardView {
    pub fn new() -> Self {
        Self {
            active_terminals: 0,
            recent_activity: Vec::new(),
        }
    }

    fn render_stat_card(icon: &str, title: &str, value: usize, color: Hsla) -> impl IntoElement {
        Card::new()
            .content(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(12.0))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.0))
                            .child(lucide_icon(
                                icon,
                                14.0,
                                ShellDeckColors::text_muted().opacity(0.7),
                            ))
                            .child(
                                div()
                                    .text_size(px(12.0))
                                    .text_color(ShellDeckColors::text_muted())
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(title.to_string()),
                            ),
                    )
                    .child(
                        div()
                            .text_size(px(32.0))
                            .text_color(color)
                            .font_weight(FontWeight::BOLD)
                            .child(value.to_string()),
                    ),
            )
            .min_w(px(180.0))
    }

    fn activity_color(kind: ActivityKind) -> Hsla {
        match kind {
            ActivityKind::Terminal | ActivityKind::Connection => ShellDeckColors::success(),
            ActivityKind::Forward | ActivityKind::Site | ActivityKind::Bext => {
                ShellDeckColors::primary()
            }
            ActivityKind::Script | ActivityKind::Monique | ActivityKind::Fleet => {
                ShellDeckColors::warning()
            }
            ActivityKind::Support | ActivityKind::Issue => ShellDeckColors::primary_hover(),
            ActivityKind::Error => ShellDeckColors::error(),
        }
    }

    fn render_activity_item(event: &ActivityEntry) -> impl IntoElement {
        let color = Self::activity_color(event.kind);
        let at_ms = event.at.timestamp_millis() as f64;
        div()
            .flex()
            .items_center()
            .gap(px(12.0))
            .w_full()
            .px(px(12.0))
            .py(px(8.0))
            .rounded(px(6.0))
            .hover(|el| el.bg(ShellDeckColors::hover_bg()))
            // Status dot
            .child(
                div()
                    .w(px(6.0))
                    .h(px(6.0))
                    .rounded_full()
                    .bg(color)
                    .flex_shrink_0(),
            )
            // Message
            .child(
                div()
                    .flex_grow()
                    .text_size(px(13.0))
                    .text_color(ShellDeckColors::text_primary())
                    .child(event.message.clone()),
            )
            // Timestamp
            .child(
                div()
                    .text_size(px(11.0))
                    .text_color(ShellDeckColors::text_muted())
                    .flex_shrink_0()
                    .child(crate::i18n::rel_time(at_ms)),
            )
    }
}

impl Render for DashboardView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let mut container = div()
            .flex()
            .flex_col()
            .size_full()
            .p(px(32.0))
            .gap(px(24.0))
            .id("dashboard")
            .overflow_y_scroll()
            .bg(ShellDeckColors::bg_primary())
            // Welcome header
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(4.0))
                    .child(
                        div()
                            .text_size(px(24.0))
                            .font_weight(FontWeight::BOLD)
                            .text_color(ShellDeckColors::text_primary())
                            .child(t!("dashboard.welcome").to_string()),
                    )
                    .child(
                        div()
                            .text_size(px(14.0))
                            .text_color(ShellDeckColors::text_muted())
                            .child(t!("dashboard.subtitle").to_string()),
                    ),
            )
            // Stats row
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap(px(16.0))
                    .child(Self::render_stat_card(
                        "terminal",
                        t!("dashboard.stats.terminals").as_ref(),
                        self.active_terminals,
                        ShellDeckColors::primary(),
                    )),
            );

        // Activity feed
        let mut activity_panel = div()
            .flex()
            .flex_col()
            .rounded(px(8.0))
            .bg(ShellDeckColors::bg_surface())
            .border_1()
            .border_color(ShellDeckColors::border());

        if self.recent_activity.is_empty() {
            activity_panel = activity_panel.child(
                div()
                    .p(px(24.0))
                    .text_size(px(13.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(t!("dashboard.no_activity").to_string()),
            );
        } else {
            activity_panel = activity_panel
                .children(self.recent_activity.iter().map(Self::render_activity_item));
        }

        container = container.child(
            div()
                .flex()
                .flex_col()
                .gap(px(8.0))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.0))
                        .child(lucide_icon("activity", 16.0, ShellDeckColors::text_muted()))
                        .child(
                            div()
                                .text_size(px(16.0))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(ShellDeckColors::text_primary())
                                .child(t!("dashboard.recent_activity").to_string()),
                        ),
                )
                .child(activity_panel),
        );

        // Keyboard shortcuts reference. Both Dev surfaces consume this same
        // ordered catalogue; only the two-column grouping belongs here.
        let shortcuts = shortcuts_for(ShortcutSurface::Dashboard, true);
        let navigation_shortcuts = shortcuts
            .iter()
            .copied()
            .filter(|shortcut| shortcut.group == ShortcutGroup::Navigation);
        let terminal_shortcuts = shortcuts
            .iter()
            .copied()
            .filter(|shortcut| shortcut.group == ShortcutGroup::Terminal);

        container = container.child(
            div()
                .flex()
                .flex_col()
                .gap(px(8.0))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.0))
                        .child(lucide_icon("keyboard", 16.0, ShellDeckColors::text_muted()))
                        .child(
                            div()
                                .text_size(px(16.0))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(ShellDeckColors::text_primary())
                                .child(t!("shortcuts.title").to_string()),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .gap(px(16.0))
                        .child(
                            // Column 1: Navigation
                            div()
                                .flex()
                                .flex_col()
                                .flex_1()
                                .rounded(px(8.0))
                                .bg(ShellDeckColors::bg_surface())
                                .border_1()
                                .border_color(ShellDeckColors::border())
                                .p(px(12.0))
                                .gap(px(2.0))
                                .child(
                                    div()
                                        .text_size(px(12.0))
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(ShellDeckColors::text_muted())
                                        .mb(px(4.0))
                                        .child(t!("shortcuts.group.navigation").to_string()),
                                )
                                .child(render_shortcut_rows(navigation_shortcuts)),
                        )
                        .child(
                            // Column 2: Terminal
                            div()
                                .flex()
                                .flex_col()
                                .flex_1()
                                .rounded(px(8.0))
                                .bg(ShellDeckColors::bg_surface())
                                .border_1()
                                .border_color(ShellDeckColors::border())
                                .p(px(12.0))
                                .gap(px(2.0))
                                .child(
                                    div()
                                        .text_size(px(12.0))
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(ShellDeckColors::text_muted())
                                        .mb(px(4.0))
                                        .child(t!("shortcuts.group.terminal").to_string()),
                                )
                                .child(render_shortcut_rows(terminal_shortcuts)),
                        ),
                ),
        );

        container
    }
}
