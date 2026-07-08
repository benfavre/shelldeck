//! Jean fleet view (Dev mode) — see the whole tenant/site fleet and control
//! whether THIS machine hosts a Jean runtime.
//!
//! Top: the runtime toggle (with a prominent safety warning — enabling it lets
//! ShellDeck run Claude Code jobs here) + this machine's live status, then any
//! jobs awaiting confirmation (Exécuter / Rejeter), the instances list, and the
//! recent-jobs feed. The view is a pure renderer; the workspace does all I/O.

use crate::scale::px;
use gpui::prelude::*;
use gpui::*;

use shelldeck_core::config::jean_fleet::{FleetSnapshot, JeanInstance, JeanJob};

use crate::theme::ShellDeckColors;

#[derive(Debug, Clone)]
pub enum FleetViewEvent {
    Refresh,
    ToggleRuntime,
    ApproveJob(String),
    RejectJob(String),
}

impl EventEmitter<FleetViewEvent> for FleetView {}

pub struct FleetView {
    snapshot: FleetSnapshot,
    /// This machine's registered instance id (to mark "cette machine").
    my_id: Option<String>,
    runtime_enabled: bool,
    my_status: String,
    awaiting: Vec<JeanJob>,
    loading: bool,
    error: Option<String>,
}

impl FleetView {
    pub fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            snapshot: FleetSnapshot::default(),
            my_id: None,
            runtime_enabled: false,
            my_status: "désactivé".to_string(),
            awaiting: Vec::new(),
            loading: false,
            error: None,
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

    fn render_runtime_card(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let enabled = self.runtime_enabled;
        let (toggle_bg, toggle_label) = if enabled {
            (ShellDeckColors::error(), "Désactiver ce runtime")
        } else {
            (ShellDeckColors::primary(), "Activer ce runtime")
        };

        div()
            .flex()
            .flex_col()
            .gap(px(8.0))
            .m(px(16.0))
            .p(px(14.0))
            .rounded(px(12.0))
            .border_1()
            .border_color(if enabled {
                ShellDeckColors::warning()
            } else {
                ShellDeckColors::border()
            })
            .bg(ShellDeckColors::bg_sidebar())
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_size(px(15.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(ShellDeckColors::text_primary())
                            .child("Runtime Jean sur cette machine"),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.0))
                            .child(div().size(px(8.0)).rounded_full().bg(if enabled {
                                ShellDeckColors::success()
                            } else {
                                ShellDeckColors::text_muted()
                            }))
                            .child(
                                div()
                                    .text_size(px(12.0))
                                    .text_color(ShellDeckColors::text_muted())
                                    .child(self.my_status.clone()),
                            ),
                    ),
            )
            .child(
                div()
                    .text_size(px(12.0))
                    .text_color(ShellDeckColors::warning())
                    .child(
                        "⚠ ShellDeck exécutera des tickets Claude Code sur cette machine \
                         (accès fichiers / édition / commandes dans le dossier de travail). \
                         Les instances « confirm » exigent une validation par ticket ; \
                         « auto » exécute immédiatement.",
                    ),
            )
            .child(
                div()
                    .id("fleet-runtime-toggle")
                    .px(px(12.0))
                    .py(px(8.0))
                    .rounded(px(8.0))
                    .bg(toggle_bg)
                    .text_size(px(13.0))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(white())
                    .cursor_pointer()
                    .w(px(200.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(toggle_label.to_string())
                    .on_click(cx.listener(|_this, _: &ClickEvent, _, cx| {
                        cx.emit(FleetViewEvent::ToggleRuntime)
                    })),
            )
    }

    fn render_awaiting(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut col = div().flex().flex_col().gap(px(6.0)).mx(px(16.0));
        if self.awaiting.is_empty() {
            return col;
        }
        col = col.child(
            div()
                .text_size(px(11.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(ShellDeckColors::warning())
                .child("TICKETS EN ATTENTE DE VALIDATION (cette machine)"),
        );
        for job in &self.awaiting {
            let id_ok = job.id.clone();
            let id_no = job.id.clone();
            col = col.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(6.0))
                    .p(px(10.0))
                    .rounded(px(8.0))
                    .border_1()
                    .border_color(ShellDeckColors::warning())
                    .bg(ShellDeckColors::warning().opacity(0.08))
                    .child(
                        div()
                            .text_size(px(13.0))
                            .text_color(ShellDeckColors::text_primary())
                            .child(job.prompt.clone()),
                    )
                    .child(
                        div()
                            .text_size(px(11.0))
                            .text_color(ShellDeckColors::text_muted())
                            .child(format!("source : {} · {}", job.source, job.requested_by)),
                    )
                    .child(
                        div()
                            .flex()
                            .gap(px(6.0))
                            .child(
                                div()
                                    .id(ElementId::from(SharedString::from(format!(
                                        "fleet-exec-{id_ok}"
                                    ))))
                                    .px(px(10.0))
                                    .py(px(5.0))
                                    .rounded(px(6.0))
                                    .bg(ShellDeckColors::success())
                                    .text_size(px(12.0))
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(white())
                                    .cursor_pointer()
                                    .child("Exécuter")
                                    .on_click(cx.listener(move |_t, _: &ClickEvent, _, cx| {
                                        cx.emit(FleetViewEvent::ApproveJob(id_ok.clone()))
                                    })),
                            )
                            .child(
                                div()
                                    .id(ElementId::from(SharedString::from(format!(
                                        "fleet-rej-{id_no}"
                                    ))))
                                    .px(px(10.0))
                                    .py(px(5.0))
                                    .rounded(px(6.0))
                                    .border_1()
                                    .border_color(ShellDeckColors::border())
                                    .text_size(px(12.0))
                                    .text_color(ShellDeckColors::error())
                                    .cursor_pointer()
                                    .hover(|s| s.bg(ShellDeckColors::hover_bg()))
                                    .child("Rejeter")
                                    .on_click(cx.listener(move |_t, _: &ClickEvent, _, cx| {
                                        cx.emit(FleetViewEvent::RejectJob(id_no.clone()))
                                    })),
                            ),
                    ),
            );
        }
        col
    }

    fn render_instance(&self, inst: &JeanInstance) -> impl IntoElement {
        let is_me = self.my_id.as_deref() == Some(inst.id.as_str());
        let dot = match inst.status.as_str() {
            "online" => ShellDeckColors::success(),
            "busy" => ShellDeckColors::warning(),
            "offline" => ShellDeckColors::error(),
            _ => ShellDeckColors::text_muted(),
        };
        let scope = if let Some(label) = &inst.site_label {
            format!("{} · {}", inst.tenant_name, label)
        } else {
            inst.tenant_name.clone()
        };
        let hb = if inst.last_seen_at > 0.0 {
            rel_time(inst.last_seen_at)
        } else {
            "jamais vu".to_string()
        };

        div()
            .flex()
            .items_center()
            .gap(px(10.0))
            .px(px(10.0))
            .py(px(7.0))
            .rounded(px(6.0))
            .child(div().size(px(8.0)).rounded_full().bg(dot).flex_shrink_0())
            .child(
                div()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .min_w(px(0.0))
                    .overflow_hidden()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.0))
                            .child(
                                div()
                                    .text_size(px(13.0))
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(ShellDeckColors::text_primary())
                                    .child(inst.name.clone()),
                            )
                            .child(
                                div()
                                    .px(px(5.0))
                                    .rounded(px(6.0))
                                    .bg(ShellDeckColors::badge_bg())
                                    .text_size(px(9.0))
                                    .text_color(ShellDeckColors::text_muted())
                                    .child(inst.runtime.clone()),
                            )
                            .children(if is_me {
                                Some(
                                    div()
                                        .px(px(5.0))
                                        .rounded(px(6.0))
                                        .bg(ShellDeckColors::primary().opacity(0.18))
                                        .text_size(px(9.0))
                                        .text_color(ShellDeckColors::primary())
                                        .child("cette machine"),
                                )
                            } else {
                                None
                            }),
                    )
                    .child(
                        div()
                            .text_size(px(11.0))
                            .text_color(ShellDeckColors::text_muted())
                            .child(format!("{} · {} · {}", scope, inst.autonomy, hb)),
                    ),
            )
    }

    fn render_job(job: &JeanJob) -> impl IntoElement {
        let color = match job.status.as_str() {
            "running" | "claimed" => ShellDeckColors::warning(),
            "done" => ShellDeckColors::success(),
            "failed" => ShellDeckColors::error(),
            _ => ShellDeckColors::text_muted(),
        };
        div()
            .flex()
            .items_center()
            .gap(px(8.0))
            .px(px(10.0))
            .py(px(5.0))
            .child(
                div()
                    .flex_shrink_0()
                    .px(px(5.0))
                    .rounded(px(6.0))
                    .bg(color.opacity(0.15))
                    .text_size(px(10.0))
                    .text_color(color)
                    .child(job.status.clone()),
            )
            .child(
                div()
                    .flex_1()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_size(px(12.0))
                    .text_color(ShellDeckColors::text_primary())
                    .child(job.prompt.clone()),
            )
            .child(
                div()
                    .flex_shrink_0()
                    .text_size(px(10.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(job.source.clone()),
            )
    }

    fn section(label: &str) -> impl IntoElement {
        div()
            .px(px(16.0))
            .pt(px(10.0))
            .pb(px(4.0))
            .text_size(px(11.0))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(ShellDeckColors::text_muted())
            .child(label.to_string())
    }
}

impl Render for FleetView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let stats = &self.snapshot.stats;

        let header = div()
            .flex()
            .items_center()
            .justify_between()
            .px(px(16.0))
            .py(px(12.0))
            .border_b_1()
            .border_color(ShellDeckColors::border())
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.0))
                    .child(
                        div()
                            .text_size(px(16.0))
                            .font_weight(FontWeight::BOLD)
                            .text_color(ShellDeckColors::text_primary())
                            .child("Flotte Jean"),
                    )
                    .child(
                        div()
                            .text_size(px(11.0))
                            .text_color(ShellDeckColors::text_muted())
                            .child(if self.loading {
                                "chargement…".to_string()
                            } else {
                                format!(
                                    "{} en ligne / {} · {} en attente · {} en cours",
                                    stats.online, stats.total, stats.pending, stats.running
                                )
                            }),
                    ),
            )
            .child(
                div()
                    .id("fleet-refresh")
                    .px(px(8.0))
                    .py(px(4.0))
                    .rounded(px(6.0))
                    .text_size(px(12.0))
                    .text_color(ShellDeckColors::text_muted())
                    .cursor_pointer()
                    .hover(|s| s.bg(ShellDeckColors::hover_bg()))
                    .child("↻ Actualiser")
                    .on_click(
                        cx.listener(|_t, _: &ClickEvent, _, cx| cx.emit(FleetViewEvent::Refresh)),
                    ),
            );

        let mut instances = div().flex().flex_col().px(px(8.0));
        if self.snapshot.instances.is_empty() {
            instances = instances.child(
                div()
                    .px(px(16.0))
                    .py(px(8.0))
                    .text_size(px(12.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child("Aucune instance."),
            );
        } else {
            for inst in &self.snapshot.instances {
                instances = instances.child(self.render_instance(inst));
            }
        }

        let mut jobs = div().flex().flex_col().px(px(8.0)).pb(px(16.0));
        if self.snapshot.jobs.is_empty() {
            jobs = jobs.child(
                div()
                    .px(px(16.0))
                    .py(px(8.0))
                    .text_size(px(12.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child("Aucun ticket récent."),
            );
        } else {
            for job in self.snapshot.jobs.iter().take(40) {
                jobs = jobs.child(Self::render_job(job));
            }
        }

        let body = div()
            .id("fleet-body")
            .flex_1()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .child(self.render_runtime_card(cx))
            .child(self.render_awaiting(cx))
            .child(Self::section("Instances"))
            .child(instances)
            .child(Self::section("Tickets récents"))
            .child(jobs);

        let mut root = div()
            .size_full()
            .flex()
            .flex_col()
            .bg(ShellDeckColors::bg_primary())
            .child(header)
            .child(body);

        if let Some(err) = &self.error {
            root = root.child(
                div()
                    .absolute()
                    .bottom(px(12.0))
                    .left(px(12.0))
                    .px(px(12.0))
                    .py(px(8.0))
                    .rounded(px(8.0))
                    .bg(ShellDeckColors::error())
                    .text_size(px(12.0))
                    .text_color(white())
                    .child(err.clone()),
            );
        }
        root
    }
}

fn rel_time(at_ms: f64) -> String {
    crate::i18n::rel_time(at_ms)
}
