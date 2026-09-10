//! Small persistent signals: Suivi metrics, the attention alert and the
//! summary line under the composer.

use std::collections::HashSet;

use adabraka_ui::prelude::{
    Button, ButtonSize, ButtonVariant, Spinner, SpinnerSize, SpinnerVariant,
};
use gpui::prelude::*;
use gpui::*;
use shelldeck_core::agent_session::{
    AgentSession, AgentSessionAttention, AgentSessionStatus, AgentTraceKind,
};
use uuid::Uuid;

use super::super::{AiAssistantEvent, AiAssistantView};
use super::{select_observed_session, usage, MONO};
use crate::icons::lucide_icon;
use crate::scale::px;
use crate::t;
use crate::theme::ShellDeckColors;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct ObservationSignals {
    pub running: usize,
    pub waiting: usize,
    pub completed: usize,
    /// First other session asking for the user, unless postponed.
    pub attention: Option<(Uuid, String)>,
}

/// Work counts across every session. A session waiting for the user counts
/// as waiting rather than running, and only a session other than the one on
/// screen can raise the alert: the observed one already shows its own state.
pub(super) fn observation_signals(
    sessions: &[AgentSession],
    observed: Option<Uuid>,
    postponed: &HashSet<Uuid>,
) -> ObservationSignals {
    let mut signals = ObservationSignals::default();
    for session in sessions {
        if session.attention == AgentSessionAttention::NeedsAttention {
            signals.waiting += 1;
            if signals.attention.is_none()
                && Some(session.id) != observed
                && !postponed.contains(&session.id)
            {
                signals.attention = Some((session.id, session.name.clone()));
            }
        } else if session.status.is_active() {
            signals.running += 1;
        } else if session.status == AgentSessionStatus::Completed {
            signals.completed += 1;
        }
    }
    signals
}

/// Commands, tools and tests recorded in a trace.
pub(super) fn tool_event_count(session: &AgentSession) -> usize {
    session
        .trace
        .iter()
        .filter(|event| {
            matches!(
                event.detail,
                AgentTraceKind::Command { .. }
                    | AgentTraceKind::Tool { .. }
                    | AgentTraceKind::Test { .. }
            )
        })
        .count()
}

pub(super) fn render_metrics(duration: String, tools: usize, files: usize) -> AnyElement {
    let metric = |value: String, label: String| {
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w(px(0.0))
            .p(px(7.0))
            .rounded(px(7.0))
            .border_1()
            .border_color(ShellDeckColors::border())
            .bg(ShellDeckColors::bg_surface())
            .child(
                div()
                    .truncate()
                    .font_family(MONO)
                    .text_size(px(12.0))
                    .font_weight(FontWeight::BOLD)
                    .text_color(ShellDeckColors::text_primary())
                    .child(value),
            )
            .child(
                div()
                    .truncate()
                    .text_size(px(8.5))
                    .text_color(ShellDeckColors::text_muted())
                    .child(label),
            )
    };
    div()
        .flex()
        .gap(px(6.0))
        .child(metric(
            duration,
            t!("ai.observability.metric_duration").to_string(),
        ))
        .child(metric(
            tools.to_string(),
            t!("ai.observability.metric_tools").to_string(),
        ))
        .child(metric(
            files.to_string(),
            t!("ai.observability.metric_files").to_string(),
        ))
        .into_any_element()
}

impl AiAssistantView {
    fn current_signals(&self, cx: &App) -> Option<ObservationSignals> {
        if !self.agent_observability_enabled {
            return None;
        }
        let console = self.agent_console.as_ref()?.read(cx);
        let observed = select_observed_session(console).map(|session| session.id);
        Some(observation_signals(
            console.sessions(),
            observed,
            &self.agent_attention_postponed,
        ))
    }

    /// Left half of the composer footnote: running and waiting sessions, the
    /// running session's tokens, the size of the local diff and an account
    /// window close to its limit. Nothing is shown while all of it is absent.
    pub(in crate::ai_assistant) fn render_observation_footnote(
        &self,
        cx: &App,
    ) -> Option<AnyElement> {
        let signals = self.current_signals(cx)?;
        let changed = self.agent_git.changed_count();
        let tokens = self.running_session_tokens(cx);
        let quota_alert = self.current_quota_alert(cx);
        if signals.running == 0
            && signals.waiting == 0
            && changed == 0
            && tokens.is_none()
            && quota_alert.is_none()
        {
            return None;
        }
        let mut row = div()
            .flex()
            .items_center()
            .gap(px(8.0))
            .min_w(px(0.0))
            .overflow_hidden();
        if signals.running > 0 {
            row = row.child(
                div()
                    .flex()
                    .flex_shrink_0()
                    .items_center()
                    .gap(px(4.0))
                    .child(
                        Spinner::new()
                            .size(SpinnerSize::Xs)
                            .variant(SpinnerVariant::Primary),
                    )
                    .child(
                        t!("ai.observability.signal_running", count = signals.running).to_string(),
                    ),
            );
        }
        if signals.waiting > 0 {
            row = row.child(
                div()
                    .flex_shrink_0()
                    .text_color(ShellDeckColors::warning())
                    .child(
                        t!("ai.observability.signal_waiting", count = signals.waiting).to_string(),
                    ),
            );
        }
        if let Some(tokens) = tokens {
            row = row.child(
                div().flex_shrink_0().child(
                    t!(
                        "ai.observability.usage_tokens",
                        value = usage::format_token_count(tokens, usage::uses_french())
                    )
                    .to_string(),
                ),
            );
        }
        if changed > 0 {
            let (additions, deletions) = self.agent_git.line_totals();
            row =
                row.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(4.0))
                        .min_w(px(0.0))
                        .child(lucide_icon(
                            "git-branch",
                            11.0,
                            ShellDeckColors::text_muted(),
                        ))
                        .child(div().truncate().child(
                            t!("ai.observability.git_file_count", count = changed).to_string(),
                        ))
                        .child(
                            div()
                                .flex_shrink_0()
                                .text_color(ShellDeckColors::success())
                                .child(format!("+{additions}")),
                        )
                        .child(
                            div()
                                .flex_shrink_0()
                                .text_color(ShellDeckColors::error())
                                .child(format!("−{deletions}")),
                        ),
                );
        }
        if let Some((provider, quota)) = quota_alert {
            row = row.child(
                div()
                    .flex_shrink_0()
                    .text_color(ShellDeckColors::error())
                    .child(
                        t!(
                            "ai.observability.usage_quota_alert",
                            provider = provider.display_name(),
                            window = usage::window_label(quota.window),
                            percent =
                                usage::format_percent(quota.used_percent, usage::uses_french())
                        )
                        .to_string(),
                    ),
            );
        }
        Some(row.into_any_element())
    }

    pub(super) fn render_attention_card(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let (session_id, name) = self.current_signals(cx)?.attention?;
        Some(
            div()
                .flex()
                .flex_col()
                .gap(px(6.0))
                .p(px(9.0))
                .rounded(px(8.0))
                .border_1()
                .border_color(ShellDeckColors::warning().opacity(0.42))
                .bg(ShellDeckColors::warning().opacity(0.08))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.0))
                        .text_size(px(10.5))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(lucide_icon(
                            "triangle-alert",
                            12.0,
                            ShellDeckColors::warning(),
                        ))
                        .child(t!("ai.observability.attention_title").to_string()),
                )
                .child(
                    div()
                        .truncate()
                        .text_size(px(10.0))
                        .text_color(ShellDeckColors::text_muted())
                        .child(name),
                )
                .child(
                    div()
                        .flex()
                        .gap(px(6.0))
                        .child(
                            Button::new(
                                "ai-attention-review",
                                t!("ai.observability.attention_review").to_string(),
                            )
                            .variant(ButtonVariant::Default)
                            .size(ButtonSize::Sm)
                            .on_click(cx.listener(|_, _, _, cx| {
                                cx.emit(AiAssistantEvent::OpenAgents);
                            })),
                        )
                        .child(
                            Button::new(
                                "ai-attention-later",
                                t!("ai.observability.attention_later").to_string(),
                            )
                            .variant(ButtonVariant::Ghost)
                            .size(ButtonSize::Sm)
                            .on_click(cx.listener(
                                move |this, _, _, cx| {
                                    this.agent_attention_postponed.insert(session_id);
                                    cx.notify();
                                },
                            )),
                        ),
                )
                .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::observation_signals;
    use shelldeck_core::agent_runtime::{AgentAccessMode, AgentProvider, AgentTarget};
    use shelldeck_core::agent_session::{
        AgentExecutionContext, AgentSession, AgentSessionAttention, AgentSessionStatus,
    };
    use std::collections::HashSet;

    fn session(
        name: &str,
        status: AgentSessionStatus,
        attention: AgentSessionAttention,
    ) -> AgentSession {
        let mut session = AgentSession::new(
            name,
            AgentExecutionContext {
                provider: AgentProvider::Codex,
                target: AgentTarget::Local,
                access: AgentAccessMode::ReadOnly,
                workdir: "/tmp/project".to_string(),
                model: None,
            },
            1,
        )
        .unwrap();
        session.status = status;
        session.attention = attention;
        session
    }

    // SDTEST-1929
    #[test]
    fn sdtest_1929_signals_count_work_and_surface_only_other_unpostponed_attention() {
        let observed = session(
            "observed",
            AgentSessionStatus::Running,
            AgentSessionAttention::NeedsAttention,
        );
        let waiting = session(
            "waiting",
            AgentSessionStatus::Running,
            AgentSessionAttention::NeedsAttention,
        );
        let sessions = vec![
            observed.clone(),
            waiting.clone(),
            session(
                "running",
                AgentSessionStatus::Starting,
                AgentSessionAttention::None,
            ),
            session(
                "done",
                AgentSessionStatus::Completed,
                AgentSessionAttention::Unread,
            ),
            session(
                "failed",
                AgentSessionStatus::Failed,
                AgentSessionAttention::None,
            ),
        ];

        let signals = observation_signals(&sessions, Some(observed.id), &HashSet::new());
        assert_eq!(
            (signals.running, signals.waiting, signals.completed),
            (1, 2, 1)
        );
        assert_eq!(signals.attention, Some((waiting.id, "waiting".to_string())));

        let postponed: HashSet<_> = [waiting.id].into();
        assert_eq!(
            observation_signals(&sessions, Some(observed.id), &postponed).attention,
            None
        );
    }
}
