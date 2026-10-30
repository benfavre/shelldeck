//! Inline disclosure for completed work; live and failed operations stay visible.
use super::*;

fn completed_work(trace: &AgentTraceEvent) -> bool {
    matches!(
        trace.detail,
        AgentTraceKind::Command {
            status: AgentTraceStatus::Succeeded,
            ..
        } | AgentTraceKind::FileRead {
            status: AgentTraceStatus::Succeeded,
            ..
        } | AgentTraceKind::Diff {
            status: AgentTraceStatus::Succeeded,
            ..
        } | AgentTraceKind::Test {
            status: AgentTraceStatus::Succeeded,
            ..
        } | AgentTraceKind::Tool {
            status: AgentTraceStatus::Succeeded,
            ..
        }
    )
}

/// Never move work across prose, a context boundary, or a turn. Keep original
/// trace identities and output so inspector links can open the exact operation.
pub(super) fn group_completed_work(items: Vec<TimelineItem>) -> Vec<TimelineItem> {
    let mut grouped = Vec::with_capacity(items.len());
    for item in items {
        match item {
            TimelineItem::Trace(trace) if completed_work(&trace) => {
                if let Some(TimelineItem::CompletedWork(traces)) = grouped.last_mut() {
                    if traces.last().is_some_and(|last: &AgentTraceEvent| {
                        last.turn_id.is_some() && last.turn_id == trace.turn_id
                    }) {
                        traces.push(trace);
                        continue;
                    }
                }
                grouped.push(TimelineItem::CompletedWork(vec![trace]));
            }
            other => grouped.push(other),
        }
    }
    grouped
}

impl AgentConsoleView {
    pub(super) fn render_completed_work(
        &self,
        session_id: Uuid,
        traces: &[AgentTraceEvent],
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(first) = traces.first() else {
            return div().into_any_element();
        };
        let group_id = first.id;
        let focused = traces
            .iter()
            .any(|trace| self.focused_trace == Some(trace.id));
        let member_ids: Vec<Uuid> = traces.iter().map(|trace| trace.id).collect();
        let expanded = self.session_ui.get(&session_id).is_some_and(|ui| {
            member_ids
                .iter()
                .any(|id| ui.expanded_activity.contains(id))
        }) || focused;
        let label = if traces.len() == 1 {
            t!("agents.activity.completed_one").to_string()
        } else {
            t!("agents.activity.completed", count = traces.len()).to_string()
        };
        let last = traces.last().unwrap_or(first);
        let (icon, _, title, _, _) = trace_presentation(&last.detail);
        let mut column = div()
            .w_full()
            .min_w(px(0.0))
            .flex_shrink_0()
            .flex()
            .flex_col()
            .gap(px(6.0))
            .child(
                div()
                    .flex()
                    .items_center()
                    .min_w(px(0.0))
                    .gap(px(6.0))
                    .child(
                        Button::new(("agent-work-disclosure", uuid_key(group_id)), label)
                            .debug_selector(|| format!("agent-work-{group_id}"))
                            .variant(ButtonVariant::Ghost)
                            .size(ButtonSize::Sm)
                            .icon(IconSource::from(if expanded {
                                "chevron-down"
                            } else {
                                "chevron-right"
                            }))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                // Disclosure never requests follow-latest or changes execution state.
                                if let Some(ui) = this.session_ui.get_mut(&session_id) {
                                    if expanded {
                                        ui.expanded_activity.retain(|id| !member_ids.contains(id));
                                    } else {
                                        ui.expanded_activity.insert(group_id);
                                    }
                                }
                                if expanded && focused {
                                    this.focused_trace = None;
                                }
                                cx.notify();
                            })),
                    )
                    .child(lucide_icon(icon, 12.0, ShellDeckColors::text_muted()))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .overflow_hidden()
                            .text_ellipsis()
                            .text_size(px(11.0))
                            .text_color(ShellDeckColors::text_muted())
                            .child(title),
                    ),
            );
        if expanded {
            // Keep the conversation as the sole scroll owner. All structured
            // output remains inline, including file changes and test summaries.
            column = column.children(traces.iter().map(|trace| {
                self.render_trace(trace, self.focused_trace == Some(trace.id), window, cx)
            }));
        }
        column.into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // SDTEST-2840 — collapsing successful work preserves order, failures and turn boundaries.
    #[::core::prelude::v1::test]
    fn sdtest_2840_completed_work_preserves_recovery_and_all_output() {
        let context = AgentExecutionContext {
            provider: AgentProvider::Codex,
            target: AgentTarget::Local,
            access: AgentAccessMode::ReadOnly,
            workdir: "/tmp".into(),
            model: None,
        };
        let mut session = AgentSession::new("Inspect and retry", context, 1).unwrap();
        session.begin_run("Check the source and tests", 2).unwrap();
        let turn = session.turns[0].id;
        let next_turn = Uuid::new_v4();
        for (index, status, turn_id) in [
            (0, AgentTraceStatus::Succeeded, turn),
            (1, AgentTraceStatus::Succeeded, turn),
            (2, AgentTraceStatus::Failed, turn),
            (3, AgentTraceStatus::Running, turn),
            (4, AgentTraceStatus::Succeeded, turn),
            (5, AgentTraceStatus::Succeeded, next_turn),
        ] {
            session.trace.push(AgentTraceEvent {
                id: Uuid::new_v4(),
                turn_id: Some(turn_id),
                sequence: index + 10,
                at_ms: index as i64 + 10,
                correlation_id: None,
                detail: AgentTraceKind::Tool {
                    name: format!("operation {index}"),
                    status,
                    summary: Some(format!("Complete output {index}")),
                },
            });
        }
        let grouped = group_completed_work(merged_timeline(&session));
        assert!(matches!(&grouped[1], TimelineItem::CompletedWork(work) if work.len() == 2));
        assert!(
            matches!(&grouped[2], TimelineItem::Trace(trace) if matches!(trace.detail,
            AgentTraceKind::Tool { status: AgentTraceStatus::Failed, .. }))
        );
        assert!(
            matches!(&grouped[3], TimelineItem::Trace(trace) if matches!(trace.detail,
            AgentTraceKind::Tool { status: AgentTraceStatus::Running, .. }))
        );
        assert!(matches!(&grouped[4], TimelineItem::CompletedWork(work) if work.len() == 1));
        assert!(matches!(&grouped[5], TimelineItem::CompletedWork(work) if work.len() == 1));
        let output: Vec<_> = grouped
            .into_iter()
            .flat_map(|item| match item {
                TimelineItem::Trace(trace) => vec![trace],
                TimelineItem::CompletedWork(traces) => traces,
                _ => vec![],
            })
            .collect();
        assert_eq!(
            output, session.trace,
            "disclosure must retain every original payload and identity"
        );
    }
}
