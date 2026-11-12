//! Native Monique console. Network work stays in `Workspace`; this view only
//! renders the typed Automonique snapshot and emits explicit user actions.

use adabraka_ui::components::input::{Input, InputSize, InputState};
use adabraka_ui::prelude::Markdown;
use gpui::prelude::*;
use gpui::*;
use shelldeck_core::config::monique::{
    MoniqueAgentAccount, MoniqueAgentAccounts, MoniqueAgentAuthAction, MoniqueAgentLoginSession,
    MoniqueChatAction, MoniqueChatHistory, MoniqueChatMessage, MoniqueChatResponse, MoniqueProcess,
    MoniqueProcesses, MoniqueStatus,
};
use std::collections::BTreeMap;

use crate::scale::px;
use crate::t;
use crate::theme::ShellDeckColors;

#[derive(Debug, Clone)]
pub enum MoniqueViewEvent {
    Refresh,
    Send(String),
    ResolveAction {
        action_id: String,
        approved: bool,
    },
    NewChat,
    AgentAction(MoniqueAgentAuthAction),
    OpenAuthorization(String),
    /// Open the request a job works on (an `https` Manage or issue page).
    OpenJobLink(String),
}

/// How many jobs the list shows, and how many output lines an expanded job.
const JOB_ROWS: usize = 30;
const JOB_OUTPUT_LINES: usize = 40;

/// When a job last moved: `updated_at`, else its latest output line (epoch
/// ms, 0 when unknown).
fn job_moved_at_ms(job: &MoniqueProcess) -> f64 {
    job.updated_at
        .as_deref()
        .or(job.created_at.as_deref())
        .and_then(|at| chrono::DateTime::parse_from_rfc3339(at).ok())
        .map(|at| at.timestamp_millis() as f64)
        .or_else(|| {
            job.output
                .iter()
                .map(|line| line.at_ms)
                .max()
                .map(|ms| ms as f64)
        })
        .unwrap_or(0.0)
}

impl EventEmitter<MoniqueViewEvent> for MoniqueView {}

pub struct MoniqueView {
    status: Option<MoniqueStatus>,
    processes: Option<MoniqueProcesses>,
    messages: Vec<MoniqueChatMessage>,
    pending_actions: Vec<MoniqueChatAction>,
    agent_accounts: MoniqueAgentAccounts,
    composer: Entity<InputState>,
    account_label: Entity<InputState>,
    authorization_codes: BTreeMap<String, Entity<InputState>>,
    confirm_logout: Option<String>,
    confirm_remove: Option<String>,
    loading: bool,
    account_loading: bool,
    error: Option<String>,
    scroll: ScrollHandle,
    /// The job list shows only jobs still in progress.
    jobs_active_only: bool,
    /// The job whose output is unfolded.
    expanded_job: Option<String>,
}

impl MoniqueView {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            status: None,
            processes: None,
            messages: Vec::new(),
            pending_actions: Vec::new(),
            agent_accounts: MoniqueAgentAccounts::default(),
            composer: cx.new(InputState::new),
            account_label: cx.new(InputState::new),
            authorization_codes: BTreeMap::new(),
            confirm_logout: None,
            confirm_remove: None,
            loading: false,
            account_loading: false,
            error: None,
            scroll: ScrollHandle::new(),
            jobs_active_only: true,
            expanded_job: None,
        }
    }

    /// Unfold the output of `job_id` (from the status bar or a notice).
    pub fn focus_job(&mut self, job_id: Option<String>) {
        if let Some(id) = job_id {
            self.jobs_active_only = false;
            self.expanded_job = Some(id);
        }
    }

    pub fn set_loading(&mut self, loading: bool) {
        self.loading = loading;
        if loading {
            self.error = None;
        }
    }

    pub fn set_error(&mut self, error: impl Into<String>) {
        self.error = Some(error.into());
        self.loading = false;
        self.account_loading = false;
    }

    pub fn set_account_error(&mut self, error: impl Into<String>) {
        self.error = Some(error.into());
        self.account_loading = false;
    }

    pub fn set_snapshot(
        &mut self,
        status: MoniqueStatus,
        processes: MoniqueProcesses,
        history: MoniqueChatHistory,
    ) {
        self.status = Some(status);
        self.processes = Some(processes);
        self.messages = history.messages;
        self.pending_actions = history.pending_actions;
        self.loading = false;
        self.error = None;
    }

    /// Jobs only (the background poll behind the status bar).
    pub fn set_processes(&mut self, processes: MoniqueProcesses) {
        self.processes = Some(processes);
    }

    pub fn apply_response(&mut self, response: MoniqueChatResponse) {
        if !response.answer.trim().is_empty() {
            self.messages.push(MoniqueChatMessage {
                role: "assistant".to_string(),
                content: response.answer,
                created_at_ms: chrono::Utc::now().timestamp_millis(),
            });
        }
        if let Some(action) = response.action {
            self.pending_actions.retain(|item| item.id != action.id);
            self.pending_actions.push(action);
        }
        self.loading = false;
        self.error = None;
    }

    pub fn set_agent_accounts(&mut self, accounts: MoniqueAgentAccounts, cx: &mut Context<Self>) {
        self.authorization_codes.retain(|session_id, _| {
            accounts
                .login_sessions
                .iter()
                .any(|session| session.id == *session_id && session.accepts_authorization_code)
        });
        for session in &accounts.login_sessions {
            if session.accepts_authorization_code
                && !self.authorization_codes.contains_key(&session.id)
            {
                self.authorization_codes
                    .insert(session.id.clone(), cx.new(InputState::new));
            }
        }
        self.agent_accounts = accounts;
        self.account_loading = false;
        self.error = None;
    }

    pub fn set_account_loading(&mut self, loading: bool) {
        self.account_loading = loading;
        if loading {
            self.error = None;
        }
    }

    pub fn has_active_login(&self) -> bool {
        self.agent_accounts
            .login_sessions
            .iter()
            .any(MoniqueAgentLoginSession::active)
    }

    pub fn chat_busy(&self) -> bool {
        self.loading
    }

    pub fn clear_chat(&mut self) {
        self.messages.clear();
        self.pending_actions.clear();
        self.loading = false;
        self.error = None;
    }

    pub fn pending_action_count(&self) -> usize {
        self.pending_actions.len()
    }

    fn submit(&mut self, cx: &mut Context<Self>) {
        if self.loading {
            return;
        }
        let message = self.composer.read(cx).content().trim().to_string();
        if message.is_empty() {
            return;
        }
        self.composer.update(cx, |state, cx| state.reset(cx));
        self.messages.push(MoniqueChatMessage {
            role: "user".to_string(),
            content: message.clone(),
            created_at_ms: chrono::Utc::now().timestamp_millis(),
        });
        self.account_loading = true;
        cx.emit(MoniqueViewEvent::Send(message));
        cx.notify();
    }

    fn button(
        id: impl Into<ElementId>,
        label: impl Into<SharedString>,
        cx: &mut Context<Self>,
        handler: impl Fn(&mut Self, &mut Context<Self>) + 'static,
    ) -> Stateful<Div> {
        div()
            .id(id)
            .px(px(10.0))
            .py(px(6.0))
            .rounded(px(7.0))
            .border_1()
            .border_color(ShellDeckColors::border())
            .bg(ShellDeckColors::bg_primary())
            .text_size(px(12.0))
            .text_color(ShellDeckColors::text_primary())
            .cursor_pointer()
            .hover(|style| style.bg(ShellDeckColors::hover_bg()))
            .child(label.into())
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| handler(this, cx)))
    }

    fn metric(label: String, value: String) -> impl IntoElement {
        div()
            .min_w(px(104.0))
            .flex_1()
            .flex()
            .flex_col()
            .gap(px(3.0))
            .p(px(10.0))
            .rounded(px(8.0))
            .border_1()
            .border_color(ShellDeckColors::border())
            .bg(ShellDeckColors::bg_surface())
            .child(
                div()
                    .text_size(px(10.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(label),
            )
            .child(
                div()
                    .text_size(px(17.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(ShellDeckColors::text_primary())
                    .child(value),
            )
    }

    fn render_header(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let ready = self.status.as_ref().is_some_and(MoniqueStatus::ready);
        let generation = self
            .status
            .as_ref()
            .and_then(|status| status.generation)
            .map_or_else(|| "—".to_string(), |value| value.to_string());
        div()
            .flex()
            .items_center()
            .gap(px(10.0))
            .px(px(14.0))
            .py(px(10.0))
            .border_b_1()
            .border_color(ShellDeckColors::border())
            .child(div().size(px(8.0)).rounded_full().bg(if ready {
                ShellDeckColors::success()
            } else {
                ShellDeckColors::warning()
            }))
            .child(
                div()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .text_size(px(14.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(ShellDeckColors::text_primary())
                            .child(t!("monique.title").to_string()),
                    )
                    .child(
                        div()
                            .text_size(px(11.0))
                            .text_color(ShellDeckColors::text_muted())
                            .child(format!(
                                "{} · {} {generation}",
                                if ready {
                                    t!("monique.status.ready")
                                } else {
                                    t!("monique.status.check")
                                },
                                t!("monique.generation")
                            )),
                    ),
            )
            .child(Self::button(
                "monique-new-chat",
                t!("monique.new_chat").to_string(),
                cx,
                |_this, cx| cx.emit(MoniqueViewEvent::NewChat),
            ))
            .child(Self::button(
                "monique-refresh",
                t!("monique.refresh").to_string(),
                cx,
                |_this, cx| cx.emit(MoniqueViewEvent::Refresh),
            ))
    }

    fn render_metrics(&self) -> impl IntoElement {
        let status = self.status.as_ref();
        let stats = self.processes.as_ref().map(|snapshot| &snapshot.stats);
        div()
            .flex()
            .flex_wrap()
            .gap(px(8.0))
            .px(px(14.0))
            .pt(px(12.0))
            .child(Self::metric(
                t!("monique.metric.running").to_string(),
                status
                    .and_then(|value| value.running)
                    .unwrap_or(0)
                    .to_string(),
            ))
            .child(Self::metric(
                t!("monique.metric.queued").to_string(),
                stats.map_or(0, |value| value.queued).to_string(),
            ))
            .child(Self::metric(
                t!("monique.metric.completed").to_string(),
                stats.map_or(0, |value| value.completed).to_string(),
            ))
            .child(Self::metric(
                t!("monique.metric.reconciliation").to_string(),
                status
                    .and_then(|value| value.reconciliation_pending)
                    .unwrap_or(0)
                    .to_string(),
            ))
    }

    fn job_status_color(job: &MoniqueProcess) -> Hsla {
        if job.is_failed() {
            ShellDeckColors::error()
        } else if job.is_finished() {
            ShellDeckColors::success()
        } else if job.status == "running" {
            ShellDeckColors::primary()
        } else {
            ShellDeckColors::warning()
        }
    }

    fn job_status_label(job: &MoniqueProcess) -> String {
        match job.status.as_str() {
            "running" => t!("monique.jobs.status.running").to_string(),
            "queued" => t!("monique.jobs.status.queued").to_string(),
            "completed" | "succeeded" | "done" => t!("monique.jobs.status.completed").to_string(),
            "failed" | "error" => t!("monique.jobs.status.failed").to_string(),
            "cancelled" | "canceled" => t!("monique.jobs.status.cancelled").to_string(),
            "" => t!("monique.jobs.status.unknown").to_string(),
            other => other.replace('_', " "),
        }
    }

    fn render_job(
        &self,
        index: usize,
        job: &MoniqueProcess,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let color = Self::job_status_color(job);
        let expanded = self.expanded_job.as_deref() == Some(job.id.as_str());
        let moved = crate::i18n::rel_time(job_moved_at_ms(job));
        let mut meta = vec![job.provider.clone(), job.runtime.clone()];
        meta.retain(|part| !part.is_empty());
        if !moved.is_empty() {
            meta.push(moved);
        }
        let job_id = job.id.clone();
        let mut header = div()
            .flex()
            .items_center()
            .gap(px(8.0))
            .child(
                div()
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .gap(px(5.0))
                    .px(px(6.0))
                    .py(px(1.0))
                    .rounded(px(6.0))
                    .bg(color.opacity(0.14))
                    .child(div().size(px(6.0)).rounded_full().bg(color))
                    .child(
                        div()
                            .text_size(px(10.0))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(color)
                            .child(Self::job_status_label(job)),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .truncate()
                    .text_size(px(12.0))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(ShellDeckColors::text_primary())
                    .child(job.label()),
            )
            .child(
                div()
                    .flex_shrink_0()
                    .text_size(px(10.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(meta.join(" · ")),
            );
        if let Some(link) = job.link().map(str::to_string) {
            header = header.child(
                div()
                    .id(("monique-job-link", index))
                    .flex_shrink_0()
                    .px(px(6.0))
                    .py(px(1.0))
                    .rounded(px(6.0))
                    .text_size(px(10.0))
                    .text_color(ShellDeckColors::primary())
                    .cursor_pointer()
                    .hover(|style| style.bg(ShellDeckColors::hover_bg()))
                    .child(t!("monique.jobs.open").to_string())
                    .on_click(cx.listener(move |_this, _: &ClickEvent, _, cx| {
                        cx.stop_propagation();
                        cx.emit(MoniqueViewEvent::OpenJobLink(link.clone()));
                    })),
            );
        }
        let mut row = div()
            .id(("monique-job", index))
            .flex()
            .flex_col()
            .gap(px(3.0))
            .px(px(10.0))
            .py(px(7.0))
            .rounded(px(7.0))
            .cursor_pointer()
            .hover(|style| style.bg(ShellDeckColors::hover_bg()))
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.expanded_job = if this.expanded_job.as_deref() == Some(job_id.as_str()) {
                    None
                } else {
                    Some(job_id.clone())
                };
                cx.notify();
            }))
            .child(header);
        if expanded {
            row = row.bg(ShellDeckColors::bg_surface());
            let start = job.output.len().saturating_sub(JOB_OUTPUT_LINES);
            let mut log = div()
                .id(("monique-job-output", index))
                .max_h(px(200.0))
                .overflow_y_scroll()
                .mt(px(4.0))
                .p(px(8.0))
                .rounded(px(6.0))
                .bg(ShellDeckColors::bg_primary())
                .border_1()
                .border_color(ShellDeckColors::border())
                .flex()
                .flex_col()
                .gap(px(2.0))
                .font_family("JetBrains Mono")
                .text_size(px(11.0));
            if job.output.is_empty() {
                log = log.child(
                    div()
                        .text_color(ShellDeckColors::text_muted())
                        .child(t!("monique.jobs.no_output").to_string()),
                );
            }
            for line in &job.output[start..] {
                let color = if line.kind == "error" || line.kind == "stderr" {
                    ShellDeckColors::error()
                } else {
                    ShellDeckColors::text_primary()
                };
                let mut text = line.text.trim_end().to_string();
                if line.truncated {
                    text.push_str(" …");
                }
                log = log.child(div().text_color(color).child(text));
            }
            row = row.child(log);
        } else if let Some(last) = job.last_output() {
            row = row.child(
                div()
                    .pl(px(2.0))
                    .truncate()
                    .font_family("JetBrains Mono")
                    .text_size(px(11.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(last.to_string()),
            );
        }
        row
    }

    /// Monique's jobs: in progress first, each with its state, what it works
    /// on, when it last moved and its latest output; a click unfolds the
    /// output log.
    fn render_jobs(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let empty = MoniqueProcesses::default();
        let snapshot = self.processes.as_ref().unwrap_or(&empty);
        let active = snapshot.active_count();
        let total = snapshot.jobs.len();
        let chip = |id: &'static str, label: String, selected: bool, active_only: bool| {
            div()
                .id(id)
                .px(px(8.0))
                .py(px(2.0))
                .rounded(px(6.0))
                .border_1()
                .border_color(if selected {
                    ShellDeckColors::primary().opacity(0.4)
                } else {
                    ShellDeckColors::border()
                })
                .bg(if selected {
                    ShellDeckColors::primary().opacity(0.12)
                } else {
                    ShellDeckColors::bg_primary()
                })
                .text_size(px(11.0))
                .text_color(ShellDeckColors::text_primary())
                .cursor_pointer()
                .child(label)
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.jobs_active_only = active_only;
                    cx.notify();
                }))
        };
        let header = div()
            .flex()
            .items_center()
            .gap(px(6.0))
            .child(
                div()
                    .flex_1()
                    .text_size(px(12.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(ShellDeckColors::text_primary())
                    .child(t!("monique.jobs.title").to_string()),
            )
            .child(chip(
                "monique-jobs-active",
                t!("monique.jobs.filter_active", count = active).to_string(),
                self.jobs_active_only,
                true,
            ))
            .child(chip(
                "monique-jobs-all",
                t!("monique.jobs.filter_all", count = total).to_string(),
                !self.jobs_active_only,
                false,
            ));
        let jobs = snapshot.ordered_jobs(self.jobs_active_only);
        let mut list = div()
            .id("monique-jobs-list")
            .max_h(px(280.0))
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap(px(2.0));
        if jobs.is_empty() {
            list = list.child(
                div()
                    .px(px(10.0))
                    .py(px(8.0))
                    .text_size(px(12.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(if self.jobs_active_only {
                        t!("monique.jobs.none_active").to_string()
                    } else {
                        t!("monique.jobs.none").to_string()
                    }),
            );
        }
        for (index, job) in jobs.into_iter().take(JOB_ROWS).enumerate() {
            list = list.child(self.render_job(index, job, cx));
        }
        div()
            .mx(px(14.0))
            .mt(px(12.0))
            .p(px(10.0))
            .rounded(px(8.0))
            .border_1()
            .border_color(ShellDeckColors::border())
            .bg(ShellDeckColors::bg_sidebar())
            .flex()
            .flex_col()
            .gap(px(6.0))
            .child(header)
            .child(list)
    }

    fn start_agent_login(&mut self, provider: &str, cx: &mut Context<Self>) {
        let entered = self.account_label.read(cx).content().trim().to_string();
        let label = if entered.is_empty() {
            match provider {
                "claude" => t!("monique.accounts.default_claude").to_string(),
                _ => t!("monique.accounts.default_codex").to_string(),
            }
        } else {
            entered
        };
        self.account_label.update(cx, |state, cx| state.reset(cx));
        self.loading = true;
        cx.emit(MoniqueViewEvent::AgentAction(
            MoniqueAgentAuthAction::StartLogin {
                provider: provider.to_string(),
                label,
                account_id: None,
            },
        ));
        cx.notify();
    }

    fn render_agent_account(
        &self,
        account: &MoniqueAgentAccount,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let select_id = account.id.clone();
        let refresh_id = account.id.clone();
        let login_id = account.id.clone();
        let login_provider = account.provider.clone();
        let login_label = account.label.clone();
        let logout_id = account.id.clone();
        let remove_id = account.id.clone();
        let confirming = self.confirm_remove.as_deref() == Some(account.id.as_str());
        let confirming_logout = self.confirm_logout.as_deref() == Some(account.id.as_str());
        let can_select = account.can_select() && !account.worker_selected;
        let state = if account.worker_selected {
            t!("monique.accounts.active").to_string()
        } else {
            account.status.clone()
        };
        let mut actions = div().flex().flex_wrap().gap(px(5.0));
        if can_select {
            actions = actions.child(Self::button(
                ElementId::from(SharedString::from(format!(
                    "monique-account-select-{select_id}"
                ))),
                t!("monique.accounts.use").to_string(),
                cx,
                move |_this, cx| {
                    cx.emit(MoniqueViewEvent::AgentAction(
                        MoniqueAgentAuthAction::Select {
                            account_id: select_id.clone(),
                        },
                    ))
                },
            ));
        }
        actions = actions
            .child(Self::button(
                ElementId::from(SharedString::from(format!(
                    "monique-account-refresh-{refresh_id}"
                ))),
                t!("monique.accounts.verify").to_string(),
                cx,
                move |_this, cx| {
                    cx.emit(MoniqueViewEvent::AgentAction(
                        MoniqueAgentAuthAction::Refresh {
                            account_id: refresh_id.clone(),
                        },
                    ))
                },
            ))
            .child(Self::button(
                ElementId::from(SharedString::from(format!(
                    "monique-account-login-{login_id}"
                ))),
                t!("monique.accounts.sign_in_again").to_string(),
                cx,
                move |_this, cx| {
                    cx.emit(MoniqueViewEvent::AgentAction(
                        MoniqueAgentAuthAction::StartLogin {
                            provider: login_provider.clone(),
                            label: login_label.clone(),
                            account_id: Some(login_id.clone()),
                        },
                    ))
                },
            ))
            .child(Self::button(
                ElementId::from(SharedString::from(format!(
                    "monique-account-logout-{logout_id}"
                ))),
                if confirming_logout {
                    t!("monique.accounts.confirm_sign_out").to_string()
                } else {
                    t!("monique.accounts.sign_out").to_string()
                },
                cx,
                move |this, cx| {
                    if this.confirm_logout.as_deref() == Some(logout_id.as_str()) {
                        this.confirm_logout = None;
                        cx.emit(MoniqueViewEvent::AgentAction(
                            MoniqueAgentAuthAction::Logout {
                                account_id: logout_id.clone(),
                                confirm: true,
                            },
                        ));
                    } else {
                        this.confirm_logout = Some(logout_id.clone());
                        cx.notify();
                    }
                },
            ));
        if !account.worker_selected {
            actions = actions.child(Self::button(
                ElementId::from(SharedString::from(format!(
                    "monique-account-remove-{remove_id}"
                ))),
                if confirming {
                    t!("monique.accounts.confirm_remove").to_string()
                } else {
                    t!("monique.accounts.remove").to_string()
                },
                cx,
                move |this, cx| {
                    if this.confirm_remove.as_deref() == Some(remove_id.as_str()) {
                        this.confirm_remove = None;
                        cx.emit(MoniqueViewEvent::AgentAction(
                            MoniqueAgentAuthAction::Remove {
                                account_id: remove_id.clone(),
                                confirm: true,
                            },
                        ));
                    } else {
                        this.confirm_remove = Some(remove_id.clone());
                        cx.notify();
                    }
                },
            ));
        }
        div()
            .flex()
            .flex_col()
            .gap(px(5.0))
            .p(px(9.0))
            .rounded(px(8.0))
            .border_1()
            .border_color(if account.worker_selected {
                ShellDeckColors::success()
            } else {
                ShellDeckColors::border()
            })
            .bg(ShellDeckColors::bg_surface())
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(
                        div()
                            .flex_1()
                            .text_size(px(12.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(ShellDeckColors::text_primary())
                            .child(account.label.clone()),
                    )
                    .child(
                        div()
                            .text_size(px(10.0))
                            .text_color(if account.can_select() {
                                ShellDeckColors::success()
                            } else {
                                ShellDeckColors::warning()
                            })
                            .child(state),
                    ),
            )
            .child(
                div()
                    .text_size(px(10.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(format!("{} · {}", account.provider_name, account.method)),
            )
            .child(actions)
    }

    fn render_login_session(
        &self,
        session: &MoniqueAgentLoginSession,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let open_url = session.safe_authorization_url().map(str::to_string);
        let cancel_id = session.id.clone();
        let submit_id = session.id.clone();
        let mut actions = div().flex().flex_wrap().items_center().gap(px(6.0));
        if let Some(url) = open_url {
            actions = actions.child(Self::button(
                ElementId::from(SharedString::from(format!(
                    "monique-login-open-{}",
                    session.id
                ))),
                t!("monique.accounts.continue_login").to_string(),
                cx,
                move |_this, cx| cx.emit(MoniqueViewEvent::OpenAuthorization(url.clone())),
            ));
        }
        if let Some(code) = &session.user_code {
            actions = actions.child(
                div()
                    .px(px(8.0))
                    .py(px(5.0))
                    .rounded(px(6.0))
                    .bg(ShellDeckColors::selected_bg())
                    .text_size(px(12.0))
                    .child(code.clone()),
            );
        }
        if session.accepts_authorization_code {
            let entity = cx.entity();
            if let Some(code_state) = self.authorization_codes.get(&session.id).cloned() {
                let code_state_for_submit = code_state.clone();
                actions = actions
                    .child(
                        div().w(px(210.0)).child(
                            Input::new(&code_state)
                                .size(InputSize::Sm)
                                .placeholder(t!("monique.accounts.authorization_code").to_string()),
                        ),
                    )
                    .child(Self::button(
                        ElementId::from(SharedString::from(format!(
                            "monique-login-submit-{submit_id}"
                        ))),
                        t!("monique.accounts.submit_code").to_string(),
                        cx,
                        move |_this, cx| {
                            entity.update(cx, |_this, cx| {
                                let code =
                                    code_state_for_submit.read(cx).content().trim().to_string();
                                if !code.is_empty() {
                                    code_state_for_submit.update(cx, |state, cx| state.reset(cx));
                                    cx.emit(MoniqueViewEvent::AgentAction(
                                        MoniqueAgentAuthAction::SubmitAuthorizationCode {
                                            session_id: submit_id.clone(),
                                            code,
                                        },
                                    ));
                                }
                            });
                        },
                    ));
            }
        }
        if session.active() {
            actions = actions.child(Self::button(
                ElementId::from(SharedString::from(format!(
                    "monique-login-cancel-{cancel_id}"
                ))),
                t!("monique.accounts.cancel").to_string(),
                cx,
                move |_this, cx| {
                    cx.emit(MoniqueViewEvent::AgentAction(
                        MoniqueAgentAuthAction::CancelLogin {
                            session_id: cancel_id.clone(),
                        },
                    ))
                },
            ));
        }
        div()
            .flex()
            .flex_col()
            .gap(px(6.0))
            .p(px(9.0))
            .rounded(px(8.0))
            .border_1()
            .border_color(ShellDeckColors::warning())
            .child(
                div()
                    .text_size(px(11.0))
                    .text_color(ShellDeckColors::text_primary())
                    .child(format!("{} · {}", session.provider, session.status)),
            )
            .child(actions)
    }

    fn render_agent_accounts(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let account_count = self.agent_accounts.accounts.len();
        let maximum = self.agent_accounts.max_accounts;
        let at_capacity = maximum > 0 && account_count >= maximum;
        let codex_available = self
            .agent_accounts
            .providers
            .iter()
            .any(|provider| provider.id == "codex" && provider.available);
        let claude_available = self
            .agent_accounts
            .providers
            .iter()
            .any(|provider| provider.id == "claude" && provider.available);
        let capacity = if maximum > 0 {
            format!("{account_count} / {maximum}")
        } else {
            account_count.to_string()
        };
        let mut list = div().flex().flex_col().gap(px(7.0));
        for session in &self.agent_accounts.login_sessions {
            list = list.child(self.render_login_session(session, cx));
        }
        for account in &self.agent_accounts.accounts {
            list = list.child(self.render_agent_account(account, cx));
        }
        if self.agent_accounts.accounts.is_empty() && self.agent_accounts.login_sessions.is_empty()
        {
            list = list.child(
                div()
                    .py(px(8.0))
                    .text_size(px(11.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(t!("monique.accounts.empty").to_string()),
            );
        }
        div()
            .id("monique-agent-accounts")
            .mx(px(14.0))
            .mt(px(10.0))
            .p(px(10.0))
            .max_h(px(290.0))
            .overflow_y_scroll()
            .rounded(px(9.0))
            .border_1()
            .border_color(ShellDeckColors::border())
            .bg(ShellDeckColors::bg_primary())
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(7.0))
                    .mb(px(8.0))
                    .child(
                        div()
                            .flex_1()
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .text_size(px(12.0))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(t!("monique.accounts.title").to_string()),
                            )
                            .child(
                                div()
                                    .text_size(px(10.0))
                                    .text_color(ShellDeckColors::text_muted())
                                    .child(t!("monique.accounts.boundary").to_string()),
                            ),
                    )
                    .child(
                        div()
                            .text_size(px(10.0))
                            .text_color(ShellDeckColors::text_muted())
                            .child(if self.account_loading {
                                t!("monique.accounts.updating").to_string()
                            } else {
                                capacity
                            }),
                    )
                    .when(!at_capacity, |header| {
                        header.child(
                            div().w(px(180.0)).child(
                                Input::new(&self.account_label)
                                    .size(InputSize::Sm)
                                    .placeholder(t!("monique.accounts.alias").to_string()),
                            ),
                        )
                    })
                    .when(codex_available && !at_capacity, |header| {
                        header.child(Self::button(
                            "monique-add-codex",
                            t!("monique.accounts.add_codex").to_string(),
                            cx,
                            |this, cx| this.start_agent_login("codex", cx),
                        ))
                    })
                    .when(claude_available && !at_capacity, |header| {
                        header.child(Self::button(
                            "monique-add-claude",
                            t!("monique.accounts.add_claude").to_string(),
                            cx,
                            |this, cx| this.start_agent_login("claude", cx),
                        ))
                    }),
            )
            .child(list)
    }

    fn render_action(action: &MoniqueChatAction, cx: &mut Context<Self>) -> impl IntoElement {
        let approve_id = action.id.clone();
        let reject_id = action.id.clone();
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
                    .text_size(px(12.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(ShellDeckColors::text_primary())
                    .child(action.title.clone()),
            )
            .child(
                div()
                    .text_size(px(12.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(action.detail.clone()),
            )
            .when(!action.impact.is_empty(), |card| {
                card.child(
                    div()
                        .text_size(px(11.0))
                        .text_color(ShellDeckColors::warning())
                        .child(action.impact.clone()),
                )
            })
            .child(
                div()
                    .flex()
                    .gap(px(6.0))
                    .child(Self::button(
                        ElementId::from(SharedString::from(format!(
                            "monique-approve-{approve_id}"
                        ))),
                        t!("monique.action.approve").to_string(),
                        cx,
                        move |_this, cx| {
                            cx.emit(MoniqueViewEvent::ResolveAction {
                                action_id: approve_id.clone(),
                                approved: true,
                            })
                        },
                    ))
                    .child(Self::button(
                        ElementId::from(SharedString::from(format!("monique-reject-{reject_id}"))),
                        t!("monique.action.reject").to_string(),
                        cx,
                        move |_this, cx| {
                            cx.emit(MoniqueViewEvent::ResolveAction {
                                action_id: reject_id.clone(),
                                approved: false,
                            })
                        },
                    )),
            )
    }

    fn render_message(message: &MoniqueChatMessage) -> impl IntoElement {
        let assistant = message.role == "assistant";
        div()
            .flex()
            .flex_col()
            .gap(px(4.0))
            .p(px(10.0))
            .rounded(px(9.0))
            .border_1()
            .border_color(ShellDeckColors::border())
            .bg(if assistant {
                ShellDeckColors::bg_surface()
            } else {
                ShellDeckColors::selected_bg()
            })
            .child(
                div()
                    .text_size(px(10.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(ShellDeckColors::text_muted())
                    .child(if assistant {
                        "MONIQUE".to_string()
                    } else {
                        t!("monique.you").to_string()
                    }),
            )
            .child(
                Markdown::new(message.content.clone())
                    .base_font_size(gpui::px(13.0))
                    .compact()
                    .w_full()
                    .min_w(px(0.0)),
            )
    }

    fn render_conversation(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut list = div()
            .id("monique-conversation")
            .track_scroll(&self.scroll)
            .flex_1()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap(px(8.0))
            .p(px(14.0));
        if self.messages.is_empty() {
            list = list.child(
                div()
                    .py(px(24.0))
                    .text_size(px(13.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(t!("monique.empty").to_string()),
            );
        } else {
            for message in &self.messages {
                list = list.child(Self::render_message(message));
            }
        }
        for action in &self.pending_actions {
            list = list.child(Self::render_action(action, cx));
        }
        if self.loading {
            list = list.child(
                div()
                    .py(px(8.0))
                    .text_size(px(12.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(t!("monique.thinking").to_string()),
            );
        }
        list
    }

    fn render_composer(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let entity = cx.entity();
        div()
            .flex()
            .items_center()
            .gap(px(8.0))
            .px(px(14.0))
            .py(px(10.0))
            .border_t_1()
            .border_color(ShellDeckColors::border())
            .child(
                div().flex_1().child(
                    Input::new(&self.composer)
                        .size(InputSize::Sm)
                        .placeholder(t!("monique.placeholder").to_string())
                        .on_enter(move |_value, cx| {
                            entity.update(cx, |this, cx| this.submit(cx));
                        }),
                ),
            )
            .child(Self::button(
                "monique-send",
                t!("monique.send").to_string(),
                cx,
                |this, cx| this.submit(cx),
            ))
    }
}

impl Render for MoniqueView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut root = div()
            .id("monique-view-root")
            .size_full()
            .flex()
            .flex_col()
            .bg(ShellDeckColors::bg_primary())
            .child(self.render_header(cx))
            .child(self.render_metrics())
            .child(self.render_jobs(cx))
            .child(self.render_agent_accounts(cx))
            .child(self.render_conversation(cx))
            .child(self.render_composer(cx));
        if let Some(error) = &self.error {
            root = root.child(
                div()
                    .absolute()
                    .bottom(px(64.0))
                    .left(px(14.0))
                    .right(px(14.0))
                    .p(px(9.0))
                    .rounded(px(8.0))
                    .bg(ShellDeckColors::error())
                    .text_size(px(12.0))
                    .text_color(white())
                    .child(error.clone()),
            );
        }
        root
    }
}
