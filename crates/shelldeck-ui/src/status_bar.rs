use adabraka_ui::prelude::use_theme;
use gpui::*;

use crate::icons::lucide_icon;
use crate::t;
use crate::theme::ShellDeckColors;

fn status_bar_uses_compact_layout(viewport_width: Pixels, rem_size: Pixels) -> bool {
    viewport_width < crate::scale::px(800.0).to_pixels(rem_size)
}

struct StatusBarTooltip {
    label: SharedString,
}

impl Render for StatusBarTooltip {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .px(px(8.0))
            .py(px(4.0))
            .rounded(px(4.0))
            .border_1()
            .border_color(ShellDeckColors::border())
            .bg(ShellDeckColors::bg_surface())
            .shadow_md()
            .text_size(px(11.0))
            .font_family(use_theme().tokens.font_family.clone())
            .text_color(ShellDeckColors::text_primary())
            .whitespace_nowrap()
            .child(self.label.clone())
    }
}

#[derive(Debug, Clone)]
pub enum StatusBarEvent {
    UpdateClicked,
}

pub struct StatusBar {
    pub active_connections: usize,
    pub active_forwards: usize,
    pub running_scripts: usize,
    pub notification: Option<String>,
    pub git_status: Option<String>,
    pub update_status: Option<String>,
}

impl EventEmitter<StatusBarEvent> for StatusBar {}

impl Default for StatusBar {
    fn default() -> Self {
        Self::new()
    }
}

impl StatusBar {
    pub fn new() -> Self {
        Self {
            active_connections: 0,
            active_forwards: 0,
            running_scripts: 0,
            notification: None,
            git_status: None,
            update_status: None,
        }
    }

    pub fn set_counts(&mut self, connections: usize, forwards: usize, scripts: usize) {
        self.active_connections = connections;
        self.active_forwards = forwards;
        self.running_scripts = scripts;
    }

    pub fn set_notification(&mut self, msg: Option<String>) {
        self.notification = msg;
    }

    fn status_item(metric: StatusMetric, count: usize) -> impl IntoElement {
        let tooltip_label: SharedString = status_count_label(metric, count).into();
        div()
            .id(metric.element_id())
            .flex()
            .items_center()
            .gap(px(4.0))
            .h(px(20.0))
            .px(px(4.0))
            .rounded(px(4.0))
            .hover(|style| style.bg(ShellDeckColors::hover_bg()))
            .tooltip(move |_, cx| {
                cx.new(|_| StatusBarTooltip {
                    label: tooltip_label.clone(),
                })
                .into()
            })
            .child(lucide_icon(
                metric.icon(),
                12.0,
                ShellDeckColors::text_muted(),
            ))
            .child(
                div()
                    .text_size(px(11.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(count.to_string()),
            )
    }

    fn trailing_status(&self, compact: bool, cx: &mut Context<Self>) -> Option<Stateful<Div>> {
        let (text, color, is_update) = if let Some(ref update) = self.update_status {
            (update.clone(), ShellDeckColors::primary(), true)
        } else if let Some(ref notification) = self.notification {
            (notification.clone(), ShellDeckColors::text_muted(), false)
        } else if !compact {
            (
                format!("ShellDeck v{}", shelldeck_core::VERSION),
                ShellDeckColors::text_muted(),
                false,
            )
        } else {
            return None;
        };

        let mut element = div()
            .id("update-status")
            .min_w(px(0.0))
            .text_size(px(11.0))
            .text_color(color)
            .child(text);
        if compact {
            element = element.truncate();
        }
        Some(if is_update {
            element.cursor_pointer().on_click(cx.listener(
                |_this, _event: &ClickEvent, _window, cx| {
                    cx.emit(StatusBarEvent::UpdateClicked);
                },
            ))
        } else {
            element
        })
    }
}

#[derive(Clone, Copy)]
pub(crate) enum StatusMetric {
    ActiveConnections,
    ActiveForwards,
    RunningScripts,
}

impl StatusMetric {
    fn icon(self) -> &'static str {
        match self {
            Self::ActiveConnections => "server",
            Self::ActiveForwards => "route",
            Self::RunningScripts => "play",
        }
    }

    fn element_id(self) -> &'static str {
        match self {
            Self::ActiveConnections => "status-active-connections",
            Self::ActiveForwards => "status-active-forwards",
            Self::RunningScripts => "status-running-scripts",
        }
    }
}

/// Localized, explicit status-bar counter.
///
/// The values are live activity counts, not inventory totals. Keeping that
/// meaning in the label prevents `0 scripts` from reading as an empty script
/// library when it actually means that no script is currently running.
pub(crate) fn status_count_label(metric: StatusMetric, count: usize) -> String {
    match (metric, count) {
        (StatusMetric::ActiveConnections, 1) => t!("status_bar.connections.one").to_string(),
        (StatusMetric::ActiveConnections, _) => {
            t!("status_bar.connections.many", count = count).to_string()
        }
        (StatusMetric::ActiveForwards, 1) => t!("status_bar.forwards.one").to_string(),
        (StatusMetric::ActiveForwards, _) => {
            t!("status_bar.forwards.many", count = count).to_string()
        }
        (StatusMetric::RunningScripts, 1) => t!("status_bar.scripts.one").to_string(),
        (StatusMetric::RunningScripts, _) => {
            t!("status_bar.scripts.many", count = count).to_string()
        }
    }
}

impl Render for StatusBar {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let is_maximized = window.is_maximized();
        let compact =
            status_bar_uses_compact_layout(window.viewport_size().width, window.rem_size());
        let mut bar = div()
            .relative()
            .flex()
            .flex_shrink_0()
            .w_full()
            .h(px(28.0))
            .items_center()
            .justify_between()
            .px(px(if compact { 8.0 } else { 12.0 }))
            .bg(ShellDeckColors::bg_sidebar());
        // This surface owns the bottom window background, so it also owns the
        // floating window's bottom radius. Parent overflow clipping is
        // rectangular in GPUI and cannot provide this mask for us.
        if !is_maximized {
            bar = bar.rounded_b(use_theme().tokens.radius_xl);
        }
        bar = bar.border_t_1().border_color(ShellDeckColors::border());

        if !compact {
            if let Some(ref git) = self.git_status {
                // The centered layer is outside the flex flow: the trailing
                // flex region can grow without pushing the branch against the
                // activity counters. Later siblings retain pointer priority.
                bar = bar.child(
                    div()
                        .absolute()
                        .top_0()
                        .bottom_0()
                        .left_0()
                        .right_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(
                            div()
                                .text_size(px(11.0))
                                .text_color(ShellDeckColors::primary())
                                .child(git.clone()),
                        ),
                );
            }
        }

        // Activity counters paint after the non-interactive centered layer so
        // their hover targets and tooltips retain pointer priority.
        bar = bar.child(
            div()
                .flex()
                .items_center()
                .flex_shrink_0()
                .gap(px(if compact { 6.0 } else { 10.0 }))
                .child(Self::status_item(
                    StatusMetric::ActiveConnections,
                    self.active_connections,
                ))
                .child(Self::status_item(
                    StatusMetric::ActiveForwards,
                    self.active_forwards,
                ))
                .child(Self::status_item(
                    StatusMetric::RunningScripts,
                    self.running_scripts,
                )),
        );

        let trailing_status = self.trailing_status(compact, cx);
        let show_trailing = !compact || trailing_status.is_some();
        if show_trailing {
            let mut trailing = div()
                .flex()
                .flex_1()
                .min_w(px(0.0))
                .justify_end()
                .overflow_hidden()
                .items_center()
                .gap(px(12.0));
            if !compact {
                trailing = trailing.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(4.0))
                        .px(px(6.0))
                        .py(px(1.0))
                        .rounded(px(4.0))
                        .bg(ShellDeckColors::hint_bg())
                        .child(
                            div()
                                .text_size(px(10.0))
                                .text_color(ShellDeckColors::text_muted())
                                .child(if cfg!(target_os = "macos") {
                                    "\u{2318}\u{21E7}P"
                                } else {
                                    "Ctrl+Shift+P"
                                }),
                        )
                        .child(
                            div()
                                .text_size(px(10.0))
                                .text_color(ShellDeckColors::text_muted())
                                .child(t!("status_bar.command_palette").to_string()),
                        ),
                );
            }
            if let Some(status) = trailing_status {
                trailing = trailing.child(status);
            }
            bar = bar.child(trailing);
        }

        bar
    }
}

#[cfg(test)]
mod tests {
    use super::{status_bar_uses_compact_layout, StatusMetric};

    // SDTEST-1736 — D-07 / SDUC-443. Compact status metadata must disappear
    // at a logical breakpoint, not at an accidental device-pixel width.
    #[test]
    fn status_bar_compact_breakpoint_tracks_ui_scale() {
        assert!(status_bar_uses_compact_layout(
            gpui::px(799.0),
            gpui::px(16.0)
        ));
        assert!(!status_bar_uses_compact_layout(
            gpui::px(800.0),
            gpui::px(16.0)
        ));
        assert!(status_bar_uses_compact_layout(
            gpui::px(1_599.0),
            gpui::px(32.0)
        ));
        assert!(!status_bar_uses_compact_layout(
            gpui::px(1_600.0),
            gpui::px(32.0)
        ));
    }

    // SDTEST-1741 — D-07. Each compact counter keeps a distinct semantic
    // Lucide glyph; its localized prose is exposed by the runtime tooltip.
    #[test]
    fn status_metrics_have_distinct_semantic_icons() {
        assert_eq!(StatusMetric::ActiveConnections.icon(), "server");
        assert_eq!(StatusMetric::ActiveForwards.icon(), "route");
        assert_eq!(StatusMetric::RunningScripts.icon(), "play");
    }
}
