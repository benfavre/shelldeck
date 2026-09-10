//! Projection of the Dev Agents cockpit into the Assistant.
//!
//! The Assistant never owns a process or a trace: it observes
//! `AgentConsoleView`, whose `AgentSessionCollection` stays the authority, and
//! offers one explicit route back to that cockpit. The only mutations it
//! starts are the user's own local Git index and commit actions (`git.rs`).

use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

use adabraka_ui::components::icon_source::IconSource;
use adabraka_ui::prelude::{
    Button, ButtonSize, ButtonVariant, Spinner, SpinnerSize, SpinnerVariant,
};
use gpui::prelude::*;
use gpui::*;
use shelldeck_core::agent_session::{
    AgentMessage, AgentMessageRole, AgentSession, AgentSessionAttention, AgentSessionStatus,
    AgentTraceEvent, AgentTraceKind, AgentTraceStatus,
};

use super::{format_duration, AiActivity, AiAssistantEvent, AiAssistantView};
use crate::agent_console_view::{merged_timeline, AgentConsoleView, TimelineItem};
use crate::icons::{lucide_icon, lucide_path};
use crate::scale::px;
use crate::t;
use crate::theme::ShellDeckColors;

mod files;
mod git;
mod signals;
mod steps;

pub(super) use git::AgentGitPanel;

/// Paths, branches and diffs use the same face as the Agents cockpit.
const MONO: &str = "JetBrains Mono";

#[derive(Debug, Default)]
struct ObservedFile {
    read: bool,
    additions: u32,
    deletions: u32,
    /// Time of the latest event touching the path.
    at_ms: i64,
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

    /// Changed paths of the observed session: the local working tree when it
    /// has been read, otherwise the diffs recorded in the trace.
    pub(super) fn observed_changed_file_count(&self, cx: &App) -> usize {
        if !self.agent_observability_enabled {
            return 0;
        }
        let Some(console) = self.agent_console.as_ref() else {
            return 0;
        };
        let console = console.read(cx);
        let Some(session) = select_observed_session(console) else {
            return 0;
        };
        if let Some(tree) = self.agent_git.tree_for(&session.context.workdir) {
            return tree.files.len();
        }
        observed_files(session)
            .values()
            .filter(|file| file.additions > 0 || file.deletions > 0)
            .count()
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
                    .child(lucide_icon("folder", 11.0, ShellDeckColors::text_muted()))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .font_family(MONO)
                            .child(session.context.workdir.clone()),
                    )
                    .children(
                        elapsed
                            .clone()
                            .map(|elapsed| div().flex_shrink_0().font_family(MONO).child(elapsed)),
                    ),
            );

        let content = match activity {
            AiActivity::AgentFiles => self.render_files(&session, cx),
            AiActivity::AgentGit => self.render_git(&session, cx),
            _ => self.render_activity(&session, elapsed.unwrap_or_else(|| format_duration(0)), cx),
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

    fn render_activity(
        &self,
        session: &AgentSession,
        duration: String,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mut root = div().flex().flex_col().gap(px(10.0));
        if let Some(card) = self.render_attention_card(cx) {
            root = root.child(card);
        }
        root = root
            .child(signals::render_metrics(
                duration,
                signals::tool_event_count(session),
                observed_files(session).len(),
            ))
            .child(panel_label(t!("ai.observability.steps_title").to_string()))
            .child(steps::render_steps(
                &steps::session_steps(session),
                now_ms(),
            ))
            .child(panel_label(
                t!("ai.observability.timeline_title").to_string(),
            ));

        // The same ordered thread as the Agents cockpit: the agent's own
        // messages between the actions they explain.
        let mut timeline = div()
            .flex()
            .flex_col()
            .gap(px(3.0))
            .border_l_1()
            .border_color(ShellDeckColors::border())
            .ml(px(5.0))
            .pl(px(12.0));
        let items = merged_timeline(session);
        if items.is_empty() {
            timeline = timeline.child(empty_line(
                t!("ai.observability.empty_activity").to_string(),
            ));
        } else {
            let start = items.len().saturating_sub(MAX_TIMELINE_ITEMS);
            for item in &items[start..] {
                timeline = timeline.child(match item {
                    TimelineItem::Trace(trace) => trace_row(trace),
                    TimelineItem::Message(message) => message_row(message),
                });
            }
        }
        root.child(timeline).into_any_element()
    }
}

/// Most recent thread rows shown in Suivi; the Agents cockpit keeps the rest.
const MAX_TIMELINE_ITEMS: usize = 24;

fn panel_label(label: String) -> AnyElement {
    div()
        .pt(px(2.0))
        .text_size(px(9.0))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(ShellDeckColors::text_muted())
        .child(label.to_uppercase())
        .into_any_element()
}

fn message_row(message: &AgentMessage) -> AnyElement {
    let (icon, color) = match message.role {
        AgentMessageRole::User => ("user", ShellDeckColors::text_muted()),
        AgentMessageRole::Agent => ("bot", ShellDeckColors::primary()),
        AgentMessageRole::Error => ("circle-alert", ShellDeckColors::error()),
    };
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
                .flex_shrink_0()
                .size(gpui::px(12.0))
                .text_color(color),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .line_clamp(3)
                .text_size(px(10.0))
                .text_color(if message.role == AgentMessageRole::Error {
                    ShellDeckColors::error()
                } else {
                    ShellDeckColors::text_primary()
                })
                .child(message.text.replace('\n', " ")),
        )
        .child(
            div()
                .flex_shrink_0()
                .font_family(MONO)
                .text_size(px(8.5))
                .text_color(ShellDeckColors::text_muted())
                .child(crate::i18n::rel_time(message.at_ms as f64)),
        )
        .into_any_element()
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

fn observed_files(session: &AgentSession) -> BTreeMap<String, ObservedFile> {
    let mut files = BTreeMap::new();
    for trace in &session.trace {
        match &trace.detail {
            AgentTraceKind::FileRead { path, .. } => {
                let file = files
                    .entry(path.clone())
                    .or_insert_with(ObservedFile::default);
                file.read = true;
                file.at_ms = file.at_ms.max(trace.at_ms);
            }
            AgentTraceKind::Diff {
                path,
                additions,
                deletions,
                ..
            } => {
                let file = files
                    .entry(path.clone())
                    .or_insert_with(ObservedFile::default);
                file.additions = *additions;
                file.deletions = *deletions;
                file.at_ms = file.at_ms.max(trace.at_ms);
            }
            _ => {}
        }
    }
    files
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
                .font_family(MONO)
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
        assert_eq!((read.additions, read.deletions, read.at_ms), (0, 0, 2));
        let changed = files.get("src/lib.rs").unwrap();
        assert!(!changed.read);
        assert_eq!(
            (changed.additions, changed.deletions, changed.at_ms),
            (12, 4, 3)
        );
    }
}
