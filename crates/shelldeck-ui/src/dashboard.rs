use crate::scale::px;
use adabraka_ui::display::card::Card;
use gpui::prelude::*;
use gpui::*;
use shelldeck_core::config::activity::{ActivityEntry, ActivityKind};
use uuid::Uuid;

use crate::icons::lucide_icon;
use crate::t;
use crate::theme::ShellDeckColors;

/// Events emitted by the dashboard.
#[derive(Debug, Clone)]
pub enum DashboardEvent {
    /// User clicked a quick-connect host button.
    QuickConnect(Uuid),
}

impl EventEmitter<DashboardEvent> for DashboardView {}

pub struct DashboardView {
    pub active_connections: usize,
    pub active_terminals: usize,
    pub running_scripts: usize,
    pub active_forwards: usize,
    pub recent_activity: Vec<ActivityEntry>,
    pub favorite_hosts: Vec<(Uuid, String, String, bool)>,
}

impl Default for DashboardView {
    fn default() -> Self {
        Self::new()
    }
}

impl DashboardView {
    pub fn new() -> Self {
        Self {
            active_connections: 0,
            active_terminals: 0,
            running_scripts: 0,
            active_forwards: 0,
            recent_activity: Vec::new(),
            favorite_hosts: Vec::new(),
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
            ActivityKind::Script | ActivityKind::Jean | ActivityKind::Fleet => {
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

    fn render_shortcut_item(keys: &str, description: &str) -> impl IntoElement {
        div()
            .flex()
            .items_center()
            .justify_between()
            .px(px(12.0))
            .py(px(6.0))
            .rounded(px(4.0))
            .hover(|el| el.bg(ShellDeckColors::hover_bg()))
            .child(
                div()
                    .text_size(px(12.0))
                    .text_color(ShellDeckColors::text_primary())
                    .child(description.to_string()),
            )
            .child(
                div()
                    .px(px(6.0))
                    .py(px(2.0))
                    .rounded(px(4.0))
                    .bg(ShellDeckColors::primary().opacity(0.12))
                    .text_size(px(11.0))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(ShellDeckColors::primary())
                    .child(keys.to_string()),
            )
    }

    fn render_quick_connect_button(
        id: Uuid,
        alias: &str,
        hostname: &str,
        is_connected: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let alias_str = alias.to_string();
        let status_color = if is_connected {
            ShellDeckColors::status_connected()
        } else {
            ShellDeckColors::status_disconnected()
        };

        div()
            .id(ElementId::from(SharedString::from(format!(
                "qc-{}",
                alias_str
            ))))
            .flex()
            .items_center()
            .gap(px(8.0))
            .px(px(12.0))
            .py(px(8.0))
            .rounded(px(8.0))
            .bg(ShellDeckColors::bg_surface())
            .border_1()
            .border_color(ShellDeckColors::border())
            .cursor_pointer()
            .hover(|el| {
                el.border_color(ShellDeckColors::primary())
                    .bg(ShellDeckColors::primary().opacity(0.08))
            })
            .on_click(cx.listener(move |_this, _: &ClickEvent, _, cx| {
                cx.emit(DashboardEvent::QuickConnect(id));
            }))
            // Status indicator
            .child(div().w(px(8.0)).h(px(8.0)).rounded_full().bg(status_color))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .text_size(px(13.0))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(ShellDeckColors::text_primary())
                            .child(alias.to_string()),
                    )
                    .child(
                        div()
                            .text_size(px(11.0))
                            .text_color(ShellDeckColors::text_muted())
                            .child(hostname.to_string()),
                    ),
            )
    }
}

impl Render for DashboardView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
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
                        "server",
                        t!("dashboard.stats.connections").as_ref(),
                        self.active_connections,
                        ShellDeckColors::success(),
                    ))
                    .child(Self::render_stat_card(
                        "terminal",
                        t!("dashboard.stats.terminals").as_ref(),
                        self.active_terminals,
                        ShellDeckColors::primary(),
                    ))
                    .child(Self::render_stat_card(
                        "scroll-text",
                        t!("dashboard.stats.scripts").as_ref(),
                        self.running_scripts,
                        ShellDeckColors::warning(),
                    ))
                    .child(Self::render_stat_card(
                        "arrow-left-right",
                        t!("dashboard.stats.forwards").as_ref(),
                        self.active_forwards,
                        ShellDeckColors::primary_hover(),
                    )),
            );

        // Quick connect section
        if !self.favorite_hosts.is_empty() {
            let hosts = self.favorite_hosts.clone();
            let mut host_buttons = div().flex().flex_wrap().gap(px(8.0));
            for (id, alias, hostname, is_connected) in &hosts {
                host_buttons = host_buttons.child(Self::render_quick_connect_button(
                    *id,
                    alias,
                    hostname,
                    *is_connected,
                    cx,
                ));
            }
            container = container.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(12.0))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.0))
                            .child(lucide_icon("zap", 16.0, ShellDeckColors::text_muted()))
                            .child(
                                div()
                                    .text_size(px(16.0))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(ShellDeckColors::text_primary())
                                    .child(t!("dashboard.quick_connect").to_string()),
                            ),
                    )
                    .child(host_buttons),
            );
        }

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

        // Keyboard shortcuts reference
        let (ctrl, cmd) = if cfg!(target_os = "macos") {
            ("\u{2318}", "\u{2318}")
        } else {
            ("Ctrl+", "Ctrl+")
        };
        let shift = if cfg!(target_os = "macos") {
            "\u{21E7}"
        } else {
            "Shift+"
        };

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
                                .child(t!("dashboard.shortcuts.title").to_string()),
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
                                        .child(t!("dashboard.shortcuts.navigation").to_string()),
                                )
                                .child(Self::render_shortcut_item(
                                    &format!("{}T", cmd),
                                    t!("dashboard.shortcut.new_terminal").as_ref(),
                                ))
                                .child(Self::render_shortcut_item(
                                    "Ctrl+Tab",
                                    t!("dashboard.shortcut.next_tab").as_ref(),
                                ))
                                .child(Self::render_shortcut_item(
                                    &format!("{}B", cmd),
                                    t!("dashboard.shortcut.toggle_sidebar").as_ref(),
                                ))
                                .child(Self::render_shortcut_item(
                                    &format!("{},", cmd),
                                    t!("dashboard.shortcut.settings").as_ref(),
                                ))
                                .child(Self::render_shortcut_item(
                                    &format!("{}{}P", cmd, shift),
                                    t!("dashboard.shortcut.command_palette").as_ref(),
                                )),
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
                                        .child(t!("dashboard.shortcuts.terminal").to_string()),
                                )
                                .child(Self::render_shortcut_item(
                                    &format!("{}{}C", ctrl, shift),
                                    t!("dashboard.shortcut.copy").as_ref(),
                                ))
                                .child(Self::render_shortcut_item(
                                    &format!("{}{}V", ctrl, shift),
                                    t!("dashboard.shortcut.paste").as_ref(),
                                ))
                                .child(Self::render_shortcut_item(
                                    &format!("{}F", cmd),
                                    t!("dashboard.shortcut.search").as_ref(),
                                ))
                                .child(Self::render_shortcut_item(
                                    &format!("{}L", cmd),
                                    t!("dashboard.shortcut.clear").as_ref(),
                                ))
                                .child(Self::render_shortcut_item(
                                    &format!("{}{}D", ctrl, shift),
                                    t!("dashboard.shortcut.split").as_ref(),
                                ))
                                .child(Self::render_shortcut_item(
                                    &format!("{}= / {}-", cmd, cmd),
                                    t!("dashboard.shortcut.zoom").as_ref(),
                                )),
                        ),
                ),
        );

        container
    }
}
