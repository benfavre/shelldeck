//! Jean fleet supervision view.
//!
//! The view keeps operational work prominent: local runtime controls and
//! instances live in the left rail, while approvals and recent jobs use the
//! larger right pane. The workspace remains responsible for all I/O.

use adabraka_ui::components::alert::Alert;
use adabraka_ui::components::button::{Button, ButtonSize, ButtonVariant};
use adabraka_ui::components::icon_source::IconSource;
use adabraka_ui::components::toggle::{Toggle, ToggleSize};
use adabraka_ui::display::badge::{Badge, BadgeVariant};
use adabraka_ui::overlays::sheet::{Sheet, SheetSize};
use gpui::prelude::*;
use gpui::*;
use std::ops::Range;

use shelldeck_core::config::jean_fleet::{FleetSnapshot, JeanInstance, JeanJob};

use crate::i18n::rel_time;
use crate::icons::lucide_icon;
use crate::scale::px;
use crate::t;
use crate::theme::ShellDeckColors;

#[derive(Debug, Clone)]
pub enum FleetViewEvent {
    Refresh,
    ToggleRuntime,
    ApproveJob(String),
    RejectJob(String),
}

impl EventEmitter<FleetViewEvent> for FleetView {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum JobFilter {
    All,
    Awaiting,
    Running,
    Done,
    Failed,
}

impl JobFilter {
    const ALL: [Self; 5] = [
        Self::All,
        Self::Awaiting,
        Self::Running,
        Self::Done,
        Self::Failed,
    ];

    fn label(self) -> String {
        match self {
            Self::All => t!("fleet.filter.all").to_string(),
            Self::Awaiting => t!("fleet.filter.awaiting").to_string(),
            Self::Running => t!("fleet.filter.running").to_string(),
            Self::Done => t!("fleet.filter.done").to_string(),
            Self::Failed => t!("fleet.filter.failed").to_string(),
        }
    }

    fn matches(self, job: &JeanJob) -> bool {
        match self {
            Self::All => true,
            Self::Awaiting => matches!(job.status.as_str(), "pending" | "claimed"),
            Self::Running => job.status == "running",
            Self::Done => job.status == "done",
            Self::Failed => matches!(job.status.as_str(), "failed" | "cancelled"),
        }
    }
}

pub struct FleetView {
    snapshot: FleetSnapshot,
    /// This machine's registered instance id (to mark "cette machine").
    my_id: Option<String>,
    runtime_enabled: bool,
    my_status: String,
    awaiting: Vec<JeanJob>,
    loading: bool,
    error: Option<String>,
    job_filter: JobFilter,
    show_runtime_warning: bool,
    job_detail_sheet: Option<Entity<Sheet>>,
}

impl FleetView {
    pub fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            snapshot: FleetSnapshot::default(),
            my_id: None,
            runtime_enabled: false,
            my_status: t!("fleet.runtime.disabled").to_string(),
            awaiting: Vec::new(),
            loading: false,
            error: None,
            job_filter: JobFilter::All,
            show_runtime_warning: false,
            job_detail_sheet: None,
        }
    }

    pub fn set_snapshot(&mut self, snapshot: FleetSnapshot) {
        self.snapshot = snapshot;
        self.loading = false;
        self.error = None;
    }

    pub fn set_runtime(&mut self, enabled: bool, my_id: Option<String>, status: impl Into<String>) {
        self.runtime_enabled = enabled;
        self.my_id = my_id;
        self.my_status = status.into();
    }

    pub fn set_awaiting(&mut self, awaiting: Vec<JeanJob>) {
        self.awaiting = awaiting;
    }

    pub fn set_loading(&mut self, loading: bool) {
        self.loading = loading;
    }

    pub fn set_error(&mut self, msg: impl Into<String>) {
        self.error = Some(msg.into());
        self.loading = false;
    }

    fn compact_filter_button(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Button {
        Button::new(id, label)
            .size(ButtonSize::Sm)
            .h(gpui::px(26.0))
            .px(gpui::px(8.0))
    }

    fn section_header(icon: &'static str, label: String, count: usize) -> impl IntoElement {
        div()
            .flex()
            .items_center()
            .justify_between()
            .gap(px(8.0))
            .px(px(14.0))
            .py(px(10.0))
            .border_b_1()
            .border_color(ShellDeckColors::border())
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .min_w(px(0.0))
                    .child(lucide_icon(icon, 15.0, ShellDeckColors::text_muted()))
                    .child(
                        div()
                            .truncate()
                            .text_size(px(13.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(ShellDeckColors::text_primary())
                            .child(label),
                    ),
            )
            .child(Badge::new(count.to_string()).variant(BadgeVariant::Secondary))
    }

    fn prompt_preview(prompt: &str) -> Div {
        let mut preview = div()
            .w_full()
            .min_w(px(0.0))
            .overflow_hidden()
            .flex()
            .flex_col()
            .text_size(px(12.0))
            .text_color(ShellDeckColors::text_primary());
        let mut rendered = 0;
        for line in prompt
            .lines()
            .filter(|line| !line.trim().is_empty())
            .take(2)
        {
            preview = preview.child(
                div()
                    .w_full()
                    .min_w(px(0.0))
                    .truncate()
                    .child(line.to_string()),
            );
            rendered += 1;
        }
        while rendered < 2 {
            preview = preview.child(div().child(" "));
            rendered += 1;
        }
        preview
    }

    fn render_header(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let stats = &self.snapshot.stats;
        let entity = cx.entity();

        div()
            .flex()
            .items_center()
            .justify_between()
            .gap(px(16.0))
            .px(px(16.0))
            .py(px(10.0))
            .border_b_1()
            .border_color(ShellDeckColors::border())
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(12.0))
                    .min_w(px(0.0))
                    .child(
                        div()
                            .truncate()
                            .text_size(px(17.0))
                            .font_weight(FontWeight::BOLD)
                            .text_color(ShellDeckColors::text_primary())
                            .child(t!("fleet.title").to_string()),
                    )
                    .child(
                        div().flex().items_center().gap(px(6.0)).children([
                            Badge::new(t!("fleet.metric.online", count = stats.online).to_string())
                                .variant(BadgeVariant::Outline),
                            Badge::new(
                                t!("fleet.metric.awaiting", count = stats.pending).to_string(),
                            )
                            .variant(if stats.pending > 0 {
                                BadgeVariant::Warning
                            } else {
                                BadgeVariant::Outline
                            }),
                            Badge::new(
                                t!("fleet.metric.running", count = stats.running).to_string(),
                            )
                            .variant(BadgeVariant::Outline),
                        ]),
                    ),
            )
            .child(
                Button::new("fleet-refresh", t!("fleet.refresh").to_string())
                    .variant(ButtonVariant::Ghost)
                    .size(ButtonSize::Sm)
                    .h(px(32.0))
                    .icon(IconSource::from("refresh-cw"))
                    .loading(self.loading)
                    .tooltip(t!("fleet.refresh").to_string())
                    .on_click(move |_, _, cx| {
                        entity.update(cx, |_this, cx| cx.emit(FleetViewEvent::Refresh));
                    }),
            )
    }

    fn render_runtime_panel(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let enabled = self.runtime_enabled;
        let toggle_label = if enabled {
            t!("fleet.runtime.disable").to_string()
        } else {
            t!("fleet.runtime.enable").to_string()
        };
        let toggle_entity = cx.entity();
        let warning_entity = cx.entity();

        let mut panel = div()
            .flex()
            .flex_col()
            .gap(px(12.0))
            .p(px(14.0))
            .border_b_1()
            .border_color(ShellDeckColors::border())
            .child(
                div()
                    .flex()
                    .items_start()
                    .justify_between()
                    .gap(px(10.0))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.0))
                            .min_w(px(0.0))
                            .child(lucide_icon("cpu", 17.0, ShellDeckColors::primary()))
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .min_w(px(0.0))
                                    .gap(px(2.0))
                                    .child(
                                        div()
                                            .truncate()
                                            .text_size(px(13.0))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(ShellDeckColors::text_primary())
                                            .child(t!("fleet.runtime.local_title").to_string()),
                                    )
                                    .child(
                                        div()
                                            .truncate()
                                            .text_size(px(11.0))
                                            .text_color(ShellDeckColors::text_muted())
                                            .child(self.my_status.clone()),
                                    ),
                            ),
                    )
                    .child(
                        Button::new("fleet-runtime-info", "")
                            .variant(ButtonVariant::Ghost)
                            .size(ButtonSize::Icon)
                            .w(px(28.0))
                            .h(px(28.0))
                            .icon(IconSource::from("info"))
                            .tooltip(t!("fleet.runtime.safety_details").to_string())
                            .on_click(move |_, _, cx| {
                                warning_entity.update(cx, |this, cx| {
                                    this.show_runtime_warning = !this.show_runtime_warning;
                                    cx.notify();
                                });
                            }),
                    ),
            )
            .child(
                Toggle::new("fleet-runtime-toggle")
                    .checked(enabled)
                    .label(toggle_label)
                    .size(ToggleSize::Sm)
                    .on_click(move |_, _, cx| {
                        toggle_entity
                            .update(cx, |_this, cx| cx.emit(FleetViewEvent::ToggleRuntime));
                    }),
            );

        if self.show_runtime_warning {
            panel = panel.child(
                Alert::warning()
                    .title(t!("fleet.runtime.safety_title").to_string())
                    .description(t!("fleet.runtime.warning").to_string()),
            );
        }

        panel
    }

    fn instance_status(status: &str) -> (&'static str, Hsla) {
        match status {
            "online" => ("fleet.status.online", ShellDeckColors::success()),
            "busy" => ("fleet.status.busy", ShellDeckColors::warning()),
            "offline" => ("fleet.status.offline", ShellDeckColors::error()),
            _ => ("fleet.status.unknown", ShellDeckColors::text_muted()),
        }
    }

    fn render_instance(&self, inst: &JeanInstance) -> impl IntoElement {
        let is_me = self.my_id.as_deref() == Some(inst.id.as_str());
        let (status_key, status_color) = Self::instance_status(&inst.status);
        let scope = if let Some(label) = &inst.site_label {
            format!("{} · {}", inst.tenant_name, label)
        } else {
            inst.tenant_name.clone()
        };
        let heartbeat = if inst.last_seen_at > 0.0 {
            rel_time(inst.last_seen_at)
        } else {
            t!("fleet.instance.never_seen").to_string()
        };
        let runtime_icon = if inst.is_shelldeck() {
            "terminal"
        } else {
            "server"
        };

        div()
            .flex()
            .items_start()
            .gap(px(10.0))
            .w_full()
            .px(px(12.0))
            .py(px(9.0))
            .border_b_1()
            .border_color(ShellDeckColors::border().opacity(0.65))
            .hover(|style| style.bg(ShellDeckColors::hover_bg()))
            .child(div().mt(px(2.0)).flex_shrink_0().child(lucide_icon(
                runtime_icon,
                16.0,
                status_color,
            )))
            .child(
                div()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .min_w(px(0.0))
                    .overflow_hidden()
                    .gap(px(5.0))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.0))
                            .min_w(px(0.0))
                            .child(
                                div()
                                    .min_w(px(0.0))
                                    .truncate()
                                    .text_size(px(13.0))
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(ShellDeckColors::text_primary())
                                    .child(inst.name.clone()),
                            )
                            .children(if is_me {
                                Some(
                                    Badge::new(t!("fleet.instance.this_machine").to_string())
                                        .variant(BadgeVariant::Default)
                                        .text_size(px(10.0))
                                        .px(px(7.0)),
                                )
                            } else {
                                None
                            }),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .flex_wrap()
                            .gap(px(5.0))
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(5.0))
                                    .text_size(px(11.0))
                                    .text_color(status_color)
                                    .child(div().size(px(6.0)).rounded_full().bg(status_color))
                                    .child(t!(status_key).to_string()),
                            )
                            .child(
                                Badge::new(inst.runtime.clone())
                                    .variant(BadgeVariant::Outline)
                                    .text_size(px(10.0))
                                    .px(px(7.0)),
                            )
                            .child(
                                Badge::new(inst.autonomy.clone())
                                    .variant(BadgeVariant::Secondary)
                                    .text_size(px(10.0))
                                    .px(px(7.0)),
                            ),
                    )
                    .child(
                        div()
                            .truncate()
                            .text_size(px(11.0))
                            .text_color(ShellDeckColors::text_muted())
                            .child(format!("{} · {}", scope, heartbeat)),
                    ),
            )
    }

    fn render_instances(&self) -> impl IntoElement {
        let mut instances: Vec<&JeanInstance> = self.snapshot.instances.iter().collect();
        instances.sort_by_key(|instance| {
            let is_me = self.my_id.as_deref() == Some(instance.id.as_str());
            let status_rank = match instance.status.as_str() {
                "busy" => 0,
                "online" => 1,
                "offline" => 3,
                _ => 2,
            };
            (!is_me, status_rank)
        });

        let mut list = div()
            .id("fleet-instances-scroll")
            .flex_1()
            .min_h(px(0.0))
            .overflow_y_scroll()
            .flex()
            .flex_col();
        if instances.is_empty() {
            list = list.child(
                div()
                    .px(px(14.0))
                    .py(px(18.0))
                    .text_size(px(12.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(t!("fleet.instances.empty").to_string()),
            );
        } else {
            for instance in instances {
                list = list.child(self.render_instance(instance));
            }
        }
        list
    }

    fn render_left_rail(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .w(px(330.0))
            .min_w(px(280.0))
            .max_w(px(360.0))
            .h_full()
            .flex_shrink_0()
            .flex()
            .flex_col()
            .min_h(px(0.0))
            .border_r_1()
            .border_color(ShellDeckColors::border())
            .bg(ShellDeckColors::bg_sidebar())
            .child(self.render_runtime_panel(cx))
            .child(Self::section_header(
                "server",
                t!("fleet.instances.section").to_string(),
                self.snapshot.instances.len(),
            ))
            .child(self.render_instances())
    }

    fn render_awaiting(&self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        if self.awaiting.is_empty() {
            return None;
        }

        let mut list = div()
            .flex()
            .flex_col()
            .gap(px(8.0))
            .px(px(14.0))
            .pb(px(12.0));
        for job in &self.awaiting {
            let approve_id = job.id.clone();
            let reject_id = job.id.clone();
            let approve_entity = cx.entity();
            let reject_entity = cx.entity();
            list = list.child(
                div()
                    .flex()
                    .items_start()
                    .gap(px(10.0))
                    .w_full()
                    .min_w(px(0.0))
                    .p(px(10.0))
                    .rounded(px(6.0))
                    .border_1()
                    .border_color(ShellDeckColors::warning().opacity(0.55))
                    .bg(ShellDeckColors::warning().opacity(0.08))
                    .child(div().flex_shrink_0().mt(px(2.0)).child(lucide_icon(
                        "circle-alert",
                        16.0,
                        ShellDeckColors::warning(),
                    )))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .flex()
                            .flex_col()
                            .gap(px(4.0))
                            .child(Self::prompt_preview(&job.prompt))
                            .child(
                                div()
                                    .truncate()
                                    .text_size(px(10.0))
                                    .text_color(ShellDeckColors::text_muted())
                                    .child(
                                        t!(
                                            "fleet.awaiting.source",
                                            source = job.source.as_str(),
                                            requested_by = job.requested_by.as_str()
                                        )
                                        .to_string(),
                                    ),
                            ),
                    )
                    .child(
                        div()
                            .flex_shrink_0()
                            .flex()
                            .items_center()
                            .gap(px(6.0))
                            .child(
                                Button::new(
                                    ElementId::from(SharedString::from(format!(
                                        "fleet-exec-{approve_id}"
                                    ))),
                                    t!("fleet.awaiting.execute").to_string(),
                                )
                                .size(ButtonSize::Sm)
                                .h(px(30.0))
                                .icon(IconSource::from("check"))
                                .on_click(move |_, _, cx| {
                                    approve_entity.update(cx, |_this, cx| {
                                        cx.emit(FleetViewEvent::ApproveJob(approve_id.clone()))
                                    });
                                }),
                            )
                            .child(
                                Button::new(
                                    ElementId::from(SharedString::from(format!(
                                        "fleet-reject-{reject_id}"
                                    ))),
                                    t!("fleet.awaiting.reject").to_string(),
                                )
                                .variant(ButtonVariant::Outline)
                                .size(ButtonSize::Sm)
                                .h(px(30.0))
                                .icon(IconSource::from("x"))
                                .on_click(move |_, _, cx| {
                                    reject_entity.update(cx, |_this, cx| {
                                        cx.emit(FleetViewEvent::RejectJob(reject_id.clone()))
                                    });
                                }),
                            ),
                    ),
            );
        }

        Some(
            div()
                .flex()
                .flex_col()
                .border_b_1()
                .border_color(ShellDeckColors::border())
                .child(Self::section_header(
                    "shield-check",
                    t!("fleet.awaiting.title").to_string(),
                    self.awaiting.len(),
                ))
                .child(list),
        )
    }

    fn job_status(job: &JeanJob) -> (&'static str, BadgeVariant, Hsla) {
        match job.status.as_str() {
            "pending" => (
                "fleet.job.pending",
                BadgeVariant::Warning,
                ShellDeckColors::warning(),
            ),
            "claimed" => (
                "fleet.job.claimed",
                BadgeVariant::Warning,
                ShellDeckColors::warning(),
            ),
            "running" => (
                "fleet.job.running",
                BadgeVariant::Default,
                ShellDeckColors::primary(),
            ),
            "done" => (
                "fleet.job.done",
                BadgeVariant::Outline,
                ShellDeckColors::success(),
            ),
            "failed" => (
                "fleet.job.failed",
                BadgeVariant::Destructive,
                ShellDeckColors::error(),
            ),
            "cancelled" => (
                "fleet.job.cancelled",
                BadgeVariant::Secondary,
                ShellDeckColors::text_muted(),
            ),
            _ => (
                "fleet.status.unknown",
                BadgeVariant::Secondary,
                ShellDeckColors::text_muted(),
            ),
        }
    }

    fn open_job_detail(&mut self, job: JeanJob, cx: &mut Context<Self>) {
        let title = t!("fleet.job.detail.title").to_string();
        let description = format!("{} · {}", job.source, rel_time(job.updated_at));
        let detail_job = job.clone();
        let fleet = cx.entity().downgrade();
        self.job_detail_sheet = Some(cx.new(move |sheet_cx| {
            Sheet::new(sheet_cx)
                .size(SheetSize::Lg)
                .title(title)
                .description(description)
                .dynamic_content(move || Self::render_job_detail_content(&detail_job))
                .on_close(move |_window, cx| {
                    if let Some(fleet) = fleet.upgrade() {
                        fleet.update(cx, |this, cx| {
                            this.job_detail_sheet = None;
                            cx.notify();
                        });
                    }
                })
        }));
        cx.notify();
    }

    fn detail_field(label: String, value: String) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .min_w(px(0.0))
            .gap(px(3.0))
            .child(
                div()
                    .text_size(px(10.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(ShellDeckColors::text_muted())
                    .child(label),
            )
            .child(
                div()
                    .min_w(px(0.0))
                    .overflow_hidden()
                    .text_size(px(12.0))
                    .text_color(ShellDeckColors::text_primary())
                    .child(value),
            )
    }

    fn multiline_block(text: &str) -> Div {
        let mut block = div()
            .w_full()
            .min_w(px(0.0))
            .overflow_hidden()
            .flex()
            .flex_col()
            .gap(px(3.0))
            .text_size(px(12.0))
            .text_color(ShellDeckColors::text_primary());
        for line in text.split('\n') {
            let display: SharedString = if line.is_empty() {
                " ".into()
            } else {
                line.to_string().into()
            };
            block = block.child(
                div()
                    .w_full()
                    .min_w(px(0.0))
                    .overflow_hidden()
                    .child(display),
            );
        }
        block
    }

    fn render_job_detail_content(job: &JeanJob) -> impl IntoElement {
        let (status_key, status_variant, _) = Self::job_status(job);
        let no_result = t!("fleet.job.detail.no_result").to_string();
        let result = job
            .result
            .as_deref()
            .filter(|value| !value.is_empty())
            .unwrap_or(&no_result)
            .to_string();

        div()
            .id("fleet-job-detail-scroll")
            .size_full()
            .min_h(px(0.0))
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap(px(18.0))
            .p(px(20.0))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(Badge::new(t!(status_key).to_string()).variant(status_variant))
                    .child(
                        div()
                            .text_size(px(11.0))
                            .text_color(ShellDeckColors::text_muted())
                            .child(rel_time(job.updated_at)),
                    ),
            )
            .child(
                div()
                    .grid()
                    .grid_cols(2)
                    .gap(px(14.0))
                    .child(Self::detail_field(
                        t!("fleet.job.detail.source").to_string(),
                        job.source.clone(),
                    ))
                    .child(Self::detail_field(
                        t!("fleet.job.detail.requested_by").to_string(),
                        job.requested_by.clone(),
                    ))
                    .child(Self::detail_field(
                        t!("fleet.job.detail.instance").to_string(),
                        job.instance_id.clone(),
                    ))
                    .child(Self::detail_field(
                        t!("fleet.job.detail.identifier").to_string(),
                        job.id.clone(),
                    )),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(8.0))
                    .child(
                        div()
                            .text_size(px(11.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(ShellDeckColors::text_muted())
                            .child(t!("fleet.job.detail.prompt").to_string()),
                    )
                    .child(
                        div()
                            .w_full()
                            .min_w(px(0.0))
                            .overflow_hidden()
                            .p(px(12.0))
                            .rounded(px(6.0))
                            .border_1()
                            .border_color(ShellDeckColors::border())
                            .bg(ShellDeckColors::bg_surface())
                            .child(Self::multiline_block(&job.prompt)),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(8.0))
                    .child(
                        div()
                            .text_size(px(11.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(ShellDeckColors::text_muted())
                            .child(t!("fleet.job.detail.result").to_string()),
                    )
                    .child(
                        div()
                            .w_full()
                            .min_w(px(0.0))
                            .overflow_hidden()
                            .p(px(12.0))
                            .rounded(px(6.0))
                            .border_1()
                            .border_color(ShellDeckColors::border())
                            .bg(ShellDeckColors::bg_surface())
                            .child(Self::multiline_block(&result)),
                    ),
            )
    }

    fn render_job(&self, job: &JeanJob, cx: &mut Context<Self>) -> impl IntoElement {
        let (status_key, status_variant, status_color) = Self::job_status(job);
        let job_for_detail = job.clone();
        let metadata = if job.requested_by.is_empty() {
            job.source.clone()
        } else {
            format!("{} · {}", job.source, job.requested_by)
        };

        div()
            .id(ElementId::from(SharedString::from(format!(
                "fleet-job-{}",
                job.id
            ))))
            .w_full()
            .min_w(px(0.0))
            .flex()
            .items_start()
            .gap(px(10.0))
            .px(px(14.0))
            .py(px(10.0))
            .border_b_1()
            .border_color(ShellDeckColors::border().opacity(0.65))
            .cursor_pointer()
            .hover(|style| style.bg(ShellDeckColors::hover_bg()))
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.open_job_detail(job_for_detail.clone(), cx);
            }))
            .child(
                div()
                    .mt(px(4.0))
                    .size(px(7.0))
                    .rounded_full()
                    .flex_shrink_0()
                    .bg(status_color),
            )
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .overflow_hidden()
                    .flex()
                    .flex_col()
                    .gap(px(6.0))
                    .child(Self::prompt_preview(&job.prompt).font_weight(FontWeight::MEDIUM))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(7.0))
                            .min_w(px(0.0))
                            .overflow_hidden()
                            .text_size(px(10.0))
                            .text_color(ShellDeckColors::text_muted())
                            .child(div().flex_shrink_0().child(
                                Badge::new(t!(status_key).to_string()).variant(status_variant),
                            ))
                            .child(div().flex_1().min_w(px(0.0)).truncate().child(metadata)),
                    ),
            )
            .child(
                div()
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(
                        div()
                            .text_size(px(10.0))
                            .text_color(ShellDeckColors::text_muted())
                            .child(rel_time(job.updated_at)),
                    )
                    .child(lucide_icon("ellipsis", 15.0, ShellDeckColors::text_muted())),
            )
    }

    fn render_job_filters(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut filters = div()
            .flex()
            .items_center()
            .flex_wrap()
            .gap(px(6.0))
            .px(px(14.0))
            .py(px(8.0))
            .border_b_1()
            .border_color(ShellDeckColors::border());
        for filter in JobFilter::ALL {
            let entity = cx.entity();
            filters = filters.child(
                Self::compact_filter_button(
                    ElementId::from(SharedString::from(format!("fleet-filter-{filter:?}"))),
                    filter.label(),
                )
                .variant(ButtonVariant::Outline)
                .selected(self.job_filter == filter)
                .on_click(move |_, _, cx| {
                    entity.update(cx, |this, cx| {
                        this.job_filter = filter;
                        cx.notify();
                    });
                }),
            );
        }
        filters
    }

    fn render_jobs(&self, cx: &mut Context<Self>) -> AnyElement {
        let filtered_count = self
            .snapshot
            .jobs
            .iter()
            .filter(|job| self.job_filter.matches(job))
            .count();
        if filtered_count == 0 {
            div()
                .id("fleet-jobs-empty")
                .flex_1()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap(px(8.0))
                .p(px(24.0))
                .child(lucide_icon("clock", 24.0, ShellDeckColors::text_muted()))
                .child(
                    div()
                        .text_size(px(12.0))
                        .text_color(ShellDeckColors::text_muted())
                        .child(t!("fleet.jobs.empty_filter").to_string()),
                )
                .into_any_element()
        } else {
            uniform_list(
                "fleet-jobs-list",
                filtered_count,
                cx.processor(|this, range: Range<usize>, _window, cx| {
                    let filtered_indices = this
                        .snapshot
                        .jobs
                        .iter()
                        .enumerate()
                        .filter(|(_, job)| this.job_filter.matches(job))
                        .map(|(index, _)| index)
                        .collect::<Vec<_>>();
                    range
                        .filter_map(|index| filtered_indices.get(index).copied())
                        .filter_map(|index| this.snapshot.jobs.get(index))
                        .map(|job| this.render_job(job, cx).into_any_element())
                        .collect::<Vec<_>>()
                }),
            )
            .flex_1()
            .min_h(px(0.0))
            .w_full()
            .into_any_element()
        }
    }

    fn render_jobs_pane(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut pane = div()
            .flex_1()
            .min_w(px(0.0))
            .min_h(px(0.0))
            .flex()
            .flex_col()
            .child(self.render_job_filters(cx));

        if let Some(awaiting) = self.render_awaiting(cx) {
            pane = pane.child(awaiting);
        }
        if let Some(error) = &self.error {
            pane = pane.child(
                div().px(px(14.0)).pt(px(10.0)).child(
                    Alert::error()
                        .title(t!("fleet.error.title").to_string())
                        .description(error.clone()),
                ),
            );
        }

        pane.child(Self::section_header(
            "activity",
            t!("fleet.jobs.section").to_string(),
            self.snapshot.jobs.len(),
        ))
        .child(self.render_jobs(cx))
    }
}

impl Render for FleetView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut root = div()
            .size_full()
            .min_w(px(0.0))
            .min_h(px(0.0))
            .flex()
            .flex_col()
            .bg(ShellDeckColors::bg_primary())
            .child(self.render_header(cx))
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .min_h(px(0.0))
                    .flex()
                    .child(self.render_left_rail(cx))
                    .child(self.render_jobs_pane(cx)),
            );

        if let Some(sheet) = &self.job_detail_sheet {
            root = root.child(sheet.clone());
        }
        root
    }
}
