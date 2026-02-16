use adabraka_ui::prelude::*;
use gpui::*;
use shelldeck_core::models::port_forward::{ForwardDirection, ForwardStatus, PortForward};
use uuid::Uuid;

use crate::theme::ShellDeckColors;

use shelldeck_core::models::port_forward::PortForward as PortForwardModel;

#[derive(Debug, Clone)]
pub enum PortForwardEvent {
    StartForward(Uuid),
    StopForward(Uuid),
    AddForward,
    EditForward(Uuid),
    AddPresetForward(PortForwardModel),
}

impl EventEmitter<PortForwardEvent> for PortForwardView {}

pub struct PortForwardView {
    pub forwards: Vec<PortForward>,
}

impl Default for PortForwardView {
    fn default() -> Self {
        Self::new()
    }
}

impl PortForwardView {
    pub fn new() -> Self {
        Self {
            forwards: Vec::new(),
        }
    }

    fn render_forward_row(
        &self,
        forward: &PortForward,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let status_color = match forward.status {
            ForwardStatus::Active => ShellDeckColors::success(),
            ForwardStatus::Inactive => ShellDeckColors::text_muted(),
            ForwardStatus::Error => ShellDeckColors::error(),
            ForwardStatus::Stopping => ShellDeckColors::warning(),
        };

        let direction_arrow = match forward.direction {
            ForwardDirection::LocalToRemote => "-->",
            ForwardDirection::RemoteToLocal => "<--",
            ForwardDirection::Dynamic => "<=>",
        };

        let direction_label = match forward.direction {
            ForwardDirection::LocalToRemote => "Local -> Remote",
            ForwardDirection::RemoteToLocal => "Remote -> Local",
            ForwardDirection::Dynamic => "SOCKS Proxy",
        };

        div()
            .flex()
            .items_center()
            .w_full()
            .px(px(16.0))
            .py(px(10.0))
            .border_b_1()
            .border_color(ShellDeckColors::border())
            .hover(|el| el.bg(ShellDeckColors::hover_bg()))
            // Status dot
            .child(
                div()
                    .w(px(40.0))
                    .flex()
                    .items_center()
                    .child(div().w(px(8.0)).h(px(8.0)).rounded_full().bg(status_color)),
            )
            // Label
            .child(
                div()
                    .w(px(160.0))
                    .text_size(px(13.0))
                    .text_color(ShellDeckColors::text_primary())
                    .font_weight(FontWeight::MEDIUM)
                    .child(
                        forward
                            .label
                            .clone()
                            .unwrap_or_else(|| "Unnamed".to_string()),
                    ),
            )
            // Direction
            .child(
                div()
                    .w(px(120.0))
                    .text_size(px(11.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(direction_label.to_string()),
            )
            // Local endpoint
            .child(
                div()
                    .w(px(140.0))
                    .text_size(px(12.0))
                    .text_color(ShellDeckColors::primary())
                    .font_family("JetBrains Mono")
                    .child(format!("{}:{}", forward.local_host, forward.local_port)),
            )
            // Arrow
            .child(
                div()
                    .w(px(40.0))
                    .text_size(px(12.0))
                    .text_color(ShellDeckColors::text_muted())
                    .text_center()
                    .child(direction_arrow.to_string()),
            )
            // Remote endpoint
            .child(
                div()
                    .w(px(140.0))
                    .text_size(px(12.0))
                    .text_color(ShellDeckColors::warning())
                    .font_family("JetBrains Mono")
                    .child(format!("{}:{}", forward.remote_host, forward.remote_port)),
            )
            // Bytes transferred
            .child(
                div()
                    .w(px(80.0))
                    .text_size(px(11.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(format_bytes(forward.bytes_sent + forward.bytes_received)),
            )
            // Actions
            .child({
                let fwd_id = forward.id;
                let is_active = forward.status == ForwardStatus::Active;
                let (label, event) = if is_active {
                    ("Stop", PortForwardEvent::StopForward(fwd_id))
                } else {
                    ("Start", PortForwardEvent::StartForward(fwd_id))
                };
                div()
                    .flex()
                    .items_center()
                    .gap(px(4.0))
                    .child(
                        div()
                            .id(ElementId::from(SharedString::from(format!(
                                "fwd-edit-{}",
                                fwd_id
                            ))))
                            .cursor_pointer()
                            .px(px(6.0))
                            .py(px(3.0))
                            .rounded(px(4.0))
                            .text_size(px(11.0))
                            .text_color(ShellDeckColors::text_muted())
                            .hover(|el| {
                                el.bg(ShellDeckColors::primary().opacity(0.15))
                                    .text_color(ShellDeckColors::primary())
                            })
                            .on_click(cx.listener(move |_this, _: &ClickEvent, _, cx| {
                                cx.emit(PortForwardEvent::EditForward(fwd_id));
                            }))
                            .child("Edit"),
                    )
                    .child(
                        div()
                            .id(ElementId::from(SharedString::from(format!(
                                "fwd-action-{}",
                                fwd_id
                            ))))
                            .cursor_pointer()
                            .px(px(6.0))
                            .py(px(3.0))
                            .rounded(px(4.0))
                            .text_size(px(11.0))
                            .text_color(ShellDeckColors::text_muted())
                            .hover(|el| {
                                if is_active {
                                    el.bg(ShellDeckColors::error().opacity(0.15))
                                        .text_color(ShellDeckColors::error())
                                } else {
                                    el.bg(ShellDeckColors::success().opacity(0.15))
                                        .text_color(ShellDeckColors::success())
                                }
                            })
                            .on_click(cx.listener(move |_this, _: &ClickEvent, _, cx| {
                                cx.emit(event.clone());
                            }))
                            .child(label),
                    )
            })
    }

    fn render_header_row() -> impl IntoElement {
        div()
            .flex()
            .items_center()
            .w_full()
            .px(px(16.0))
            .py(px(8.0))
            .border_b_1()
            .border_color(ShellDeckColors::border())
            .bg(ShellDeckColors::bg_sidebar())
            .child(
                div()
                    .w(px(40.0))
                    .text_size(px(10.0))
                    .text_color(ShellDeckColors::text_muted())
                    .font_weight(FontWeight::BOLD)
                    .child(""),
            )
            .child(
                div()
                    .w(px(160.0))
                    .text_size(px(10.0))
                    .text_color(ShellDeckColors::text_muted())
                    .font_weight(FontWeight::BOLD)
                    .child("LABEL"),
            )
            .child(
                div()
                    .w(px(120.0))
                    .text_size(px(10.0))
                    .text_color(ShellDeckColors::text_muted())
                    .font_weight(FontWeight::BOLD)
                    .child("DIRECTION"),
            )
            .child(
                div()
                    .w(px(140.0))
                    .text_size(px(10.0))
                    .text_color(ShellDeckColors::text_muted())
                    .font_weight(FontWeight::BOLD)
                    .child("LOCAL"),
            )
            .child(
                div()
                    .w(px(40.0))
                    .text_size(px(10.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(""),
            )
            .child(
                div()
                    .w(px(140.0))
                    .text_size(px(10.0))
                    .text_color(ShellDeckColors::text_muted())
                    .font_weight(FontWeight::BOLD)
                    .child("REMOTE"),
            )
            .child(
                div()
                    .w(px(80.0))
                    .text_size(px(10.0))
                    .text_color(ShellDeckColors::text_muted())
                    .font_weight(FontWeight::BOLD)
                    .child("TRAFFIC"),
            )
            .child(
                div()
                    .text_size(px(10.0))
                    .text_color(ShellDeckColors::text_muted())
                    .font_weight(FontWeight::BOLD)
                    .child("ACTIONS"),
            )
    }

    fn render_port_map(&self) -> impl IntoElement {
        // Visual port map showing connections between local and remote
        let mut map = div()
            .flex()
            .flex_col()
            .gap(px(4.0))
            .p(px(16.0))
            .bg(ShellDeckColors::bg_surface())
            .rounded(px(8.0))
            .border_1()
            .border_color(ShellDeckColors::border());

        // Header
        map = map.child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .mb(px(8.0))
                .child(
                    div()
                        .text_size(px(12.0))
                        .font_weight(FontWeight::BOLD)
                        .text_color(ShellDeckColors::primary())
                        .child("LOCAL"),
                )
                .child(
                    div()
                        .text_size(px(12.0))
                        .font_weight(FontWeight::BOLD)
                        .text_color(ShellDeckColors::warning())
                        .child("REMOTE"),
                ),
        );

        let active_forwards: Vec<_> = self
            .forwards
            .iter()
            .filter(|f| f.status == ForwardStatus::Active)
            .collect();

        for forward in &active_forwards {
            let arrow = match forward.direction {
                ForwardDirection::LocalToRemote => "-------->",
                ForwardDirection::RemoteToLocal => "<--------",
                ForwardDirection::Dynamic => "<------->",
            };

            map = map.child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .py(px(4.0))
                    .child(
                        div()
                            .text_size(px(12.0))
                            .text_color(ShellDeckColors::primary())
                            .font_family("JetBrains Mono")
                            .child(format!(":{}", forward.local_port)),
                    )
                    .child(
                        div()
                            .text_size(px(10.0))
                            .text_color(ShellDeckColors::text_muted())
                            .child(arrow.to_string()),
                    )
                    .child(
                        div()
                            .text_size(px(12.0))
                            .text_color(ShellDeckColors::warning())
                            .font_family("JetBrains Mono")
                            .child(format!(":{}", forward.remote_port)),
                    ),
            );
        }

        if active_forwards.is_empty() {
            map = map.child(
                div()
                    .text_size(px(12.0))
                    .text_color(ShellDeckColors::text_muted())
                    .text_center()
                    .py(px(12.0))
                    .child("No active port forwards"),
            );
        }

        map
    }

    #[allow(clippy::too_many_arguments)]
    fn render_preset_card(
        id: &str,
        title: &str,
        description: &str,
        local_label: &str,
        remote_label: &str,
        arrow: &str,
        preset: PortForwardModel,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        div()
            .id(ElementId::from(SharedString::from(id.to_string())))
            .flex()
            .flex_col()
            .gap(px(8.0))
            .p(px(14.0))
            .w(px(260.0))
            .bg(ShellDeckColors::bg_surface())
            .rounded(px(8.0))
            .border_1()
            .border_color(ShellDeckColors::border())
            .cursor_pointer()
            .hover(|el| {
                el.border_color(ShellDeckColors::primary().opacity(0.5))
                    .bg(ShellDeckColors::primary().opacity(0.04))
            })
            .on_click(cx.listener(move |_this, _: &ClickEvent, _, cx| {
                cx.emit(PortForwardEvent::AddPresetForward(preset.clone()));
            }))
            // Title
            .child(
                div()
                    .text_size(px(13.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(ShellDeckColors::text_primary())
                    .child(title.to_string()),
            )
            // Description
            .child(
                div()
                    .text_size(px(11.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(description.to_string()),
            )
            // Port mapping visualization
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .mt(px(2.0))
                    .child(
                        div()
                            .text_size(px(11.0))
                            .font_family("JetBrains Mono")
                            .text_color(ShellDeckColors::primary())
                            .child(local_label.to_string()),
                    )
                    .child(
                        div()
                            .text_size(px(10.0))
                            .text_color(ShellDeckColors::text_muted())
                            .child(arrow.to_string()),
                    )
                    .child(
                        div()
                            .text_size(px(11.0))
                            .font_family("JetBrains Mono")
                            .text_color(ShellDeckColors::warning())
                            .child(remote_label.to_string()),
                    ),
            )
    }

    fn render_presets(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let nil = Uuid::nil();
        div()
            .flex()
            .flex_col()
            .gap(px(10.0))
            .px(px(24.0))
            .py(px(12.0))
            // Section header
            .child(
                div()
                    .text_size(px(11.0))
                    .font_weight(FontWeight::BOLD)
                    .text_color(ShellDeckColors::text_muted())
                    .child("PRESETS"),
            )
            // Preset cards row
            .child(
                div()
                    .flex()
                    .gap(px(12.0))
                    .flex_wrap()
                    .child(Self::render_preset_card(
                        "preset-opencode",
                        "OpenCode Web",
                        "Forward remote OpenCode web UI to local browser",
                        "localhost:4096",
                        "remote:4096",
                        "-->",
                        PortForwardModel::opencode_preset(nil),
                        cx,
                    ))
                    .child(Self::render_preset_card(
                        "preset-chrome-devtools",
                        "Chrome DevTools",
                        "Expose local Chrome DevTools to remote server",
                        "localhost:9222",
                        "remote:9222",
                        "<--",
                        PortForwardModel::chrome_devtools_preset(nil),
                        cx,
                    ))
                    .child(Self::render_preset_card(
                        "preset-dev-server",
                        "Dev Server",
                        "Forward remote dev server port 3060 to local",
                        "localhost:3060",
                        "remote:3060",
                        "-->",
                        PortForwardModel::dev_server_preset(nil),
                        cx,
                    )),
            )
    }
}

impl Render for PortForwardView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(ShellDeckColors::bg_primary())
            .id("port-forward-view")
            .overflow_y_scroll()
            // Header
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .px(px(24.0))
                    .py(px(16.0))
                    .border_b_1()
                    .border_color(ShellDeckColors::border())
                    .child(
                        div()
                            .text_size(px(18.0))
                            .font_weight(FontWeight::BOLD)
                            .text_color(ShellDeckColors::text_primary())
                            .child("Port Forwards"),
                    )
                    .child(
                        div().flex().gap(px(8.0)).child(
                            div()
                                .id("add-forward-btn")
                                .on_click(cx.listener(|_this, _: &ClickEvent, _, cx| {
                                    cx.emit(PortForwardEvent::AddForward);
                                }))
                                .child(
                                    Button::new("add-forward", "+ Add Forward")
                                        .variant(ButtonVariant::Default),
                                ),
                        ),
                    ),
            )
            // Presets
            .child(self.render_presets(cx))
            // Port map
            .child(
                div()
                    .px(px(24.0))
                    .py(px(16.0))
                    .child(self.render_port_map()),
            )
            // Table
            .child(
                div()
                    .flex()
                    .flex_col()
                    .mx(px(24.0))
                    .rounded(px(8.0))
                    .border_1()
                    .border_color(ShellDeckColors::border())
                    .overflow_hidden()
                    .child(Self::render_header_row())
                    .children(self.forwards.iter().map(|f| self.render_forward_row(f, cx))),
            )
    }
}

fn format_bytes(bytes: u64) -> String {
    if bytes == 0 {
        return "0 B".to_string();
    }
    let units = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit_idx = 0;
    while value >= 1024.0 && unit_idx < units.len() - 1 {
        value /= 1024.0;
        unit_idx += 1;
    }
    if unit_idx == 0 {
        format!("{} B", bytes)
    } else {
        format!("{:.1} {}", value, units[unit_idx])
    }
}
