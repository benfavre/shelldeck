//! Read-only projection of the Dev Agents cockpit into the Assistant.
//!
//! The assistant never owns a process, trace, file, or Git mutation. It
//! observes `AgentConsoleView`, whose `AgentSessionCollection` remains the
//! authority, and offers one explicit route back to that cockpit.

use std::collections::BTreeMap;
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use adabraka_ui::components::icon_source::IconSource;
use adabraka_ui::prelude::{
    Button, ButtonSize, ButtonVariant, Spinner, SpinnerSize, SpinnerVariant,
};
use gpui::prelude::*;
use gpui::*;
use shelldeck_core::agent_runtime::AgentTarget;
use shelldeck_core::agent_session::{
    AgentSession, AgentSessionAttention, AgentSessionStatus, AgentTraceEvent, AgentTraceKind,
    AgentTraceStatus,
};

use super::{format_duration, AiActivity, AiAssistantEvent, AiAssistantView};
use crate::agent_console_view::AgentConsoleView;
use crate::icons::{lucide_icon, lucide_path};
use crate::scale::px;
use crate::t;
use crate::theme::ShellDeckColors;

#[derive(Default)]
struct ObservedFile {
    read: bool,
    additions: u32,
    deletions: u32,
    status: AgentTraceStatus,
}

impl AiAssistantView {
    /// Bind only for Dev-capable accounts. The observer makes both the Sheet
    /// and external Dock repaint when the authoritative console receives a
    /// stream event, including while either assistant surface was hidden.
    pub fn bind_agent_console(
        &mut self,
        console: Entity<AgentConsoleView>,
        cx: &mut Context<Self>,
    ) {
        self._agent_console_observer = Some(cx.observe(&console, |this, _, cx| {
            this.refresh_agent_git(cx);
            cx.notify();
        }));
        self.agent_console = Some(console);
        self.refresh_agent_git(cx);
        cx.notify();
    }

    pub fn set_agent_observability_enabled(&mut self, enabled: bool, cx: &mut Context<Self>) {
        self.agent_observability_enabled = enabled && self.agent_console.is_some();
        if !self.agent_observability_enabled
            && matches!(
                self.active_tab,
                AiActivity::AgentActivity | AiActivity::AgentFiles | AiActivity::AgentGit
            )
        {
            self.active_tab = AiActivity::Chat;
            self.sync_loading();
        }
        if self.agent_observability_enabled {
            self.refresh_agent_git(cx);
        }
        cx.notify();
    }

    fn observed_session(&self, cx: &App) -> Option<AgentSession> {
        if !self.agent_observability_enabled {
            return None;
        }
        let console = self.agent_console.as_ref()?.read(cx);
        select_observed_session(console).cloned()
    }

    pub(super) fn observed_agent_active_count(&self, cx: &App) -> usize {
        if !self.agent_observability_enabled {
            return 0;
        }
        self.agent_console
            .as_ref()
            .map(|console| {
                console
                    .read(cx)
                    .sessions()
                    .iter()
                    .filter(|session| session.status.is_active())
                    .count()
            })
            .unwrap_or(0)
    }

    pub(super) fn observed_changed_file_count(&self, cx: &App) -> usize {
        if !self.agent_observability_enabled {
            return 0;
        }
        self.agent_console
            .as_ref()
            .and_then(|console| {
                let console = console.read(cx);
                select_observed_session(console).map(|session| {
                    observed_files(session)
                        .values()
                        .filter(|file| file.additions > 0 || file.deletions > 0)
                        .count()
                })
            })
            .unwrap_or(0)
    }

    pub(super) fn refresh_agent_git(&mut self, cx: &mut Context<Self>) {
        let Some(session) = self.observed_session(cx) else {
            self.agent_git_status = None;
            return;
        };
        if !matches!(session.context.target, AgentTarget::Local) {
            self.agent_git_status = None;
            return;
        }
        let workdir = session.context.workdir;
        if self
            .agent_git_last_refresh
            .as_ref()
            .is_some_and(|(path, at)| path == &workdir && at.elapsed() < Duration::from_secs(2))
        {
            return;
        }
        self.agent_git_last_refresh = Some((workdir.clone(), std::time::Instant::now()));
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let lookup_path = workdir.clone();
            let status = cx
                .background_executor()
                .spawn(async move { shelldeck_core::git::get_git_status(Path::new(&lookup_path)) })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this
                    .observed_session(cx)
                    .is_some_and(|session| session.context.workdir == workdir)
                {
                    this.agent_git_status = status.map(|status| (workdir, status));
                    cx.notify();
                }
            });
        })
        .detach();
    }

    pub(super) fn render_agent_observability(
        &self,
        activity: AiActivity,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let tabs = [
            (AiActivity::AgentActivity, "activity", "ai.rail.activity"),
            (AiActivity::AgentFiles, "file-text", "ai.rail.files"),
            (AiActivity::AgentGit, "git-branch", "ai.rail.git"),
        ];
        let mut header = div()
            .flex()
            .items_center()
            .gap(px(4.0))
            .h(px(44.0))
            .px(px(10.0))
            .flex_shrink_0()
            .border_b_1()
            .border_color(ShellDeckColors::border());
        for (tab, icon, label) in tabs {
            header = header.child(
                Button::new(
                    SharedString::from(format!("ai-observability-{tab:?}")),
                    t!(label).to_string(),
                )
                .variant(if activity == tab {
                    ButtonVariant::Secondary
                } else {
                    ButtonVariant::Ghost
                })
                .size(ButtonSize::Sm)
                .icon(IconSource::from(icon))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.active_tab = tab;
                    this.sync_loading();
                    if tab == AiActivity::AgentGit {
                        this.refresh_agent_git(cx);
                    }
                    cx.notify();
                })),
            );
        }

        let Some(session) = self.observed_session(cx) else {
            return div()
                .flex()
                .flex_col()
                .size_full()
                .child(header)
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .items_center()
                        .justify_center()
                        .gap(px(9.0))
                        .px(px(24.0))
                        .text_align(TextAlign::Center)
                        .child(lucide_icon("bot", 24.0, ShellDeckColors::text_muted()))
                        .child(
                            div()
                                .text_size(px(13.0))
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(t!("ai.observability.empty_title").to_string()),
                        )
                        .child(
                            div()
                                .max_w(px(320.0))
                                .text_size(px(11.0))
                                .text_color(ShellDeckColors::text_muted())
                                .child(t!("ai.observability.empty_description").to_string()),
                        )
                        .child(open_agents_button(cx)),
                )
                .into_any_element();
        };

        let status = agent_status_label(session.status);
        let status_color = agent_status_color(session.status, session.attention);
        let model = session
            .context
            .model
            .clone()
            .unwrap_or_else(|| t!("agents.model.auto").to_string());
        let elapsed = session.started_at_ms.map(|started| {
            let ended = session.finished_at_ms.unwrap_or_else(now_ms);
            format_duration(ended.saturating_sub(started).max(0) as u64)
        });
        let summary = div()
            .flex()
            .flex_col()
            .gap(px(7.0))
            .mx(px(10.0))
            .mt(px(10.0))
            .p(px(10.0))
            .rounded(px(8.0))
            .border_1()
            .border_color(ShellDeckColors::border())
            .bg(ShellDeckColors::bg_surface())
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(7.0))
                    .min_w_0()
                    .child(
                        div()
                            .relative()
                            .flex()
                            .items_center()
                            .justify_center()
                            .size(px(24.0))
                            .rounded(px(6.0))
                            .bg(ShellDeckColors::bg_primary())
                            .child(lucide_icon("bot", 13.0, ShellDeckColors::text_muted()))
                            .when(session.status.is_active(), |icon| {
                                icon.child(
                                    div()
                                        .absolute()
                                        .right(gpui::px(-2.0))
                                        .bottom(gpui::px(-2.0))
                                        .child(
                                            Spinner::new()
                                                .size(SpinnerSize::Xs)
                                                .variant(SpinnerVariant::Primary),
                                        ),
                                )
                            }),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w_0()
                            .child(
                                div()
                                    .truncate()
                                    .text_size(px(12.0))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(session.name.clone()),
                            )
                            .child(
                                div()
                                    .truncate()
                                    .text_size(px(9.5))
                                    .text_color(ShellDeckColors::text_muted())
                                    .child(format!(
                                        "{} · {model} · {}",
                                        session.context.provider.display_name(),
                                        session.context.target.label()
                                    )),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(5.0))
                            .px(px(7.0))
                            .py(px(3.0))
                            .rounded_full()
                            .bg(status_color.opacity(0.12))
                            .text_size(px(9.5))
                            .text_color(status_color)
                            .child(status),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .min_w_0()
                    .text_size(px(9.5))
                    .text_color(ShellDeckColors::text_muted())
                    .child(lucide_icon(
                        "file-text",
                        11.0,
                        ShellDeckColors::text_muted(),
                    ))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .child(session.context.workdir.clone()),
                    )
                    .children(elapsed.map(|elapsed| {
                        div()
                            .flex_shrink_0()
                            .font_family("monospace")
                            .child(elapsed)
                    })),
            );

        let content = match activity {
            AiActivity::AgentFiles => render_files(&session),
            AiActivity::AgentGit => self.render_git(&session, cx),
            _ => render_activity(&session),
        };

        div()
            .flex()
            .flex_col()
            .size_full()
            .min_h(px(0.0))
            .child(header)
            .child(summary)
            .child(
                div()
                    .id("ai-observability-scroll")
                    .flex_1()
                    .min_h(px(0.0))
                    .overflow_y_scroll()
                    .px(px(10.0))
                    .py(px(10.0))
                    .child(content),
            )
            .child(
                div()
                    .flex()
                    .justify_end()
                    .px(px(10.0))
                    .py(px(8.0))
                    .border_t_1()
                    .border_color(ShellDeckColors::border())
                    .child(open_agents_button(cx)),
            )
            .into_any_element()
    }

    fn render_git(&self, session: &AgentSession, _cx: &App) -> AnyElement {
        let files = observed_files(session);
        let changes = files
            .iter()
            .filter(|(_, file)| file.additions > 0 || file.deletions > 0)
            .collect::<Vec<_>>();
        let additions: u32 = changes.iter().map(|(_, file)| file.additions).sum();
        let deletions: u32 = changes.iter().map(|(_, file)| file.deletions).sum();
        let status = self
            .agent_git_status
            .as_ref()
            .filter(|(path, _)| path == &session.context.workdir)
            .map(|(_, status)| status);
        let branch = status
            .and_then(|status| status.branch.clone())
            .unwrap_or_else(|| t!("ai.observability.git_unknown").to_string());

        let mut root = div().flex().flex_col().gap(px(8.0)).child(
            div()
                .flex()
                .flex_col()
                .gap(px(6.0))
                .p(px(10.0))
                .rounded(px(8.0))
                .border_1()
                .border_color(ShellDeckColors::border())
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.0))
                        .min_w_0()
                        .font_family("monospace")
                        .text_size(px(10.5))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(lucide_icon(
                            "git-branch",
                            12.0,
                            ShellDeckColors::text_muted(),
                        ))
                        .child(div().flex_1().min_w_0().truncate().child(branch)),
                )
                .child(
                    div()
                        .flex()
                        .gap(px(8.0))
                        .text_size(px(9.5))
                        .text_color(ShellDeckColors::text_muted())
                        .child(format!(
                            "{} {}",
                            changes.len(),
                            t!("ai.observability.files_count")
                        ))
                        .child(
                            div()
                                .text_color(ShellDeckColors::success())
                                .child(format!("+{additions}")),
                        )
                        .child(
                            div()
                                .text_color(ShellDeckColors::error())
                                .child(format!("−{deletions}")),
                        )
                        .children(status.map(|status| {
                            div().child(format!(
                                "{} {} · {} {} · {} {}",
                                status.staged,
                                t!("ai.observability.staged"),
                                status.modified,
                                t!("ai.observability.modified"),
                                status.untracked,
                                t!("ai.observability.untracked")
                            ))
                        })),
                ),
        );
        for (path, file) in changes {
            root = root.child(file_row(path, file));
        }
        root.child(
            div()
                .flex()
                .gap(px(7.0))
                .p(px(9.0))
                .rounded(px(7.0))
                .bg(ShellDeckColors::warning().opacity(0.10))
                .text_size(px(10.0))
                .text_color(ShellDeckColors::text_muted())
                .child(lucide_icon(
                    "shield-check",
                    12.0,
                    ShellDeckColors::warning(),
                ))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .child(t!("ai.observability.git_explicit").to_string()),
                ),
        )
        .into_any_element()
    }
}

fn select_observed_session(console: &AgentConsoleView) -> Option<&AgentSession> {
    console
        .sessions()
        .iter()
        .filter(|session| session.status.is_active())
        .max_by_key(|session| session.updated_at_ms)
        .or_else(|| {
            console
                .selected_session_id()
                .and_then(|id| console.sessions().iter().find(|session| session.id == id))
        })
        .or_else(|| {
            console
                .sessions()
                .iter()
                .max_by_key(|session| session.updated_at_ms)
        })
}

fn render_activity(session: &AgentSession) -> AnyElement {
    let mut timeline = div()
        .flex()
        .flex_col()
        .gap(px(3.0))
        .border_l_1()
        .border_color(ShellDeckColors::border())
        .ml(px(5.0))
        .pl(px(12.0));
    let traces = session.trace.iter().rev().take(16).collect::<Vec<_>>();
    if traces.is_empty() {
        timeline = timeline.child(empty_line(
            t!("ai.observability.empty_activity").to_string(),
        ));
    } else {
        for trace in traces.into_iter().rev() {
            timeline = timeline.child(trace_row(trace));
        }
    }
    timeline.into_any_element()
}

fn render_files(session: &AgentSession) -> AnyElement {
    let files = observed_files(session);
    let mut root = div()
        .flex()
        .flex_col()
        .gap(px(3.0))
        .font_family("monospace")
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(6.0))
                .px(px(6.0))
                .py(px(5.0))
                .text_size(px(10.0))
                .font_weight(FontWeight::SEMIBOLD)
                .child(lucide_icon(
                    "file-text",
                    12.0,
                    ShellDeckColors::text_muted(),
                ))
                .child(
                    Path::new(&session.context.workdir)
                        .file_name()
                        .and_then(|name| name.to_str())
                        .unwrap_or(&session.context.workdir)
                        .to_string(),
                ),
        );
    if files.is_empty() {
        root = root.child(empty_line(t!("ai.observability.empty_files").to_string()));
    } else {
        for (path, file) in &files {
            root = root.child(file_row(path, file));
        }
    }
    root.into_any_element()
}

fn observed_files(session: &AgentSession) -> BTreeMap<String, ObservedFile> {
    let mut files = BTreeMap::new();
    for trace in &session.trace {
        match &trace.detail {
            AgentTraceKind::FileRead { path, status, .. } => {
                let file = files
                    .entry(path.clone())
                    .or_insert_with(ObservedFile::default);
                file.read = true;
                file.status = *status;
            }
            AgentTraceKind::Diff {
                path,
                status,
                additions,
                deletions,
                ..
            } => {
                let file = files
                    .entry(path.clone())
                    .or_insert_with(ObservedFile::default);
                file.additions = *additions;
                file.deletions = *deletions;
                file.status = *status;
            }
            _ => {}
        }
    }
    files
}

fn file_row(path: &str, file: &ObservedFile) -> AnyElement {
    let changed = file.additions > 0 || file.deletions > 0;
    let marker = if changed { "M" } else { "L" };
    let marker_color = if changed {
        ShellDeckColors::warning()
    } else {
        ShellDeckColors::primary()
    };
    let depth = Path::new(path)
        .components()
        .count()
        .saturating_sub(1)
        .min(3) as f32;
    div()
        .flex()
        .items_center()
        .gap(px(6.0))
        .min_w_0()
        .pl(px(6.0 + depth * 10.0))
        .pr(px(5.0))
        .py(px(5.0))
        .rounded(px(5.0))
        .hover(|style| style.bg(ShellDeckColors::hover_bg()))
        .text_size(px(9.5))
        .child(lucide_icon(
            "file-text",
            11.0,
            ShellDeckColors::text_muted(),
        ))
        .child(div().flex_1().min_w_0().truncate().child(path.to_string()))
        .when(changed, |row| {
            row.child(
                div()
                    .flex_shrink_0()
                    .text_color(ShellDeckColors::success())
                    .child(format!("+{}", file.additions)),
            )
            .child(
                div()
                    .flex_shrink_0()
                    .text_color(ShellDeckColors::error())
                    .child(format!("−{}", file.deletions)),
            )
        })
        .child(
            div()
                .flex()
                .items_center()
                .justify_center()
                .flex_shrink_0()
                .size(px(16.0))
                .rounded(px(4.0))
                .bg(marker_color.opacity(0.12))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(marker_color)
                .child(marker),
        )
        .into_any_element()
}

fn trace_row(trace: &AgentTraceEvent) -> AnyElement {
    let (icon, title, detail, status) = match &trace.detail {
        AgentTraceKind::Command {
            command,
            status,
            summary,
            ..
        } => ("terminal", command.clone(), summary.clone(), *status),
        AgentTraceKind::FileRead { path, status, .. } => ("search", path.clone(), None, *status),
        AgentTraceKind::Diff {
            path,
            status,
            additions,
            deletions,
            ..
        } => (
            "git-branch",
            path.clone(),
            Some(format!("+{additions} −{deletions}")),
            *status,
        ),
        AgentTraceKind::Test {
            name,
            status,
            summary,
        } => ("check-check", name.clone(), summary.clone(), *status),
        AgentTraceKind::Tool {
            name,
            status,
            summary,
        } => ("settings", name.clone(), summary.clone(), *status),
        AgentTraceKind::Activity { label } => {
            ("activity", label.clone(), None, AgentTraceStatus::Unknown)
        }
    };
    let color = trace_status_color(status);
    div()
        .relative()
        .flex()
        .gap(px(7.0))
        .min_w_0()
        .py(px(6.0))
        .child(
            div()
                .absolute()
                .left(gpui::px(-17.0))
                .top(gpui::px(11.0))
                .size(gpui::px(9.0))
                .rounded_full()
                .border_2()
                .border_color(ShellDeckColors::bg_primary())
                .bg(color),
        )
        .child(
            svg()
                .path(lucide_path(icon))
                .size(gpui::px(12.0))
                .text_color(color),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .child(div().truncate().text_size(px(10.0)).child(title))
                .children(detail.map(|detail| {
                    div()
                        .mt(px(2.0))
                        .truncate()
                        .text_size(px(9.0))
                        .text_color(ShellDeckColors::text_muted())
                        .child(detail)
                })),
        )
        .child(
            div()
                .flex_shrink_0()
                .font_family("monospace")
                .text_size(px(8.5))
                .text_color(ShellDeckColors::text_muted())
                .child(crate::i18n::rel_time(trace.at_ms as f64)),
        )
        .into_any_element()
}

fn open_agents_button(cx: &mut Context<AiAssistantView>) -> Button {
    Button::new(
        "ai-observability-open-agents",
        t!("ai.observability.open_agents").to_string(),
    )
    .variant(ButtonVariant::Secondary)
    .size(ButtonSize::Sm)
    .icon(IconSource::from("external-link"))
    .on_click(cx.listener(|_, _, _, cx| cx.emit(AiAssistantEvent::OpenAgents)))
}

fn empty_line(message: String) -> AnyElement {
    div()
        .px(px(8.0))
        .py(px(18.0))
        .text_align(TextAlign::Center)
        .text_size(px(10.0))
        .text_color(ShellDeckColors::text_muted())
        .child(message)
        .into_any_element()
}

fn agent_status_label(status: AgentSessionStatus) -> String {
    let key = match status {
        AgentSessionStatus::Idle => "ai.observability.status.idle",
        AgentSessionStatus::Starting => "ai.observability.status.starting",
        AgentSessionStatus::Running => "ai.observability.status.running",
        AgentSessionStatus::Stopping => "ai.observability.status.stopping",
        AgentSessionStatus::Completed => "ai.observability.status.completed",
        AgentSessionStatus::Failed => "ai.observability.status.failed",
        AgentSessionStatus::Cancelled => "ai.observability.status.cancelled",
    };
    t!(key).to_string()
}

fn agent_status_color(status: AgentSessionStatus, attention: AgentSessionAttention) -> Hsla {
    if attention == AgentSessionAttention::NeedsAttention || status == AgentSessionStatus::Failed {
        ShellDeckColors::error()
    } else if status.is_active() {
        ShellDeckColors::warning()
    } else if status == AgentSessionStatus::Completed {
        ShellDeckColors::success()
    } else {
        ShellDeckColors::text_muted()
    }
}

fn trace_status_color(status: AgentTraceStatus) -> Hsla {
    match status {
        AgentTraceStatus::Succeeded => ShellDeckColors::success(),
        AgentTraceStatus::Failed | AgentTraceStatus::Cancelled => ShellDeckColors::error(),
        AgentTraceStatus::Pending | AgentTraceStatus::Running => ShellDeckColors::warning(),
        AgentTraceStatus::Unknown => ShellDeckColors::text_muted(),
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(i64::MAX as u128) as i64
}

#[cfg(test)]
mod tests {
    use super::observed_files;
    use shelldeck_core::agent_runtime::{AgentAccessMode, AgentProvider, AgentTarget};
    use shelldeck_core::agent_session::{
        AgentExecutionContext, AgentSession, AgentTraceEvent, AgentTraceKind, AgentTraceStatus,
    };
    use uuid::Uuid;

    // SDTEST-1923
    #[test]
    fn assistant_file_projection_uses_structured_agent_trace_without_inventing_changes() {
        let mut session = AgentSession::new(
            "Audit",
            AgentExecutionContext {
                provider: AgentProvider::Codex,
                target: AgentTarget::Local,
                access: AgentAccessMode::ReadOnly,
                workdir: "/tmp/project".to_string(),
                model: Some("gpt-test".to_string()),
            },
            1,
        )
        .unwrap();
        session.trace = vec![
            AgentTraceEvent {
                id: Uuid::new_v4(),
                sequence: 1,
                at_ms: 2,
                correlation_id: None,
                detail: AgentTraceKind::FileRead {
                    path: "src/main.rs".to_string(),
                    status: AgentTraceStatus::Succeeded,
                    line_start: Some(1),
                    line_end: Some(20),
                },
            },
            AgentTraceEvent {
                id: Uuid::new_v4(),
                sequence: 2,
                at_ms: 3,
                correlation_id: None,
                detail: AgentTraceKind::Diff {
                    path: "src/lib.rs".to_string(),
                    status: AgentTraceStatus::Succeeded,
                    additions: 12,
                    deletions: 4,
                    preview: None,
                },
            },
        ];

        let files = observed_files(&session);
        assert_eq!(files.len(), 2);
        let read = files.get("src/main.rs").unwrap();
        assert!(read.read);
        assert_eq!((read.additions, read.deletions), (0, 0));
        let changed = files.get("src/lib.rs").unwrap();
        assert!(!changed.read);
        assert_eq!((changed.additions, changed.deletions), (12, 4));
    }
}
