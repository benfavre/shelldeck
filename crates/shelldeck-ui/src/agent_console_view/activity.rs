//! Inline disclosure for completed work; live and failed operations stay visible.
use super::*;
use std::{cell::RefCell, rc::Rc};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct FloatingGroup {
    session: Uuid,
    group: Uuid,
}

pub(super) struct ActivityUi {
    floating: Option<FloatingGroup>,
    pending_reveal: Option<Uuid>,
    pending_open: Option<Uuid>,
    anchor: ScrollAnchor,
}
impl ActivityUi {
    pub(super) fn new(scroll: ScrollHandle) -> Self {
        Self {
            floating: None,
            pending_reveal: None,
            pending_open: None,
            anchor: ScrollAnchor::for_handle(scroll),
        }
    }
}
#[derive(Clone, Copy)]
pub(super) struct ActivityBounds {
    group: Uuid,
    header: Bounds<Pixels>,
    bottom: Pixels,
}
pub(super) type ActivityFrame = Rc<RefCell<Vec<ActivityBounds>>>;

/// A collapsed preview is one bounded line. The original multiline command
/// remains in the expanded trace, copy/export and retained model.
fn operation_preview(detail: &AgentTraceKind) -> (&'static str, String) {
    let (icon, text) = match detail {
        AgentTraceKind::Command { command, .. } => ("terminal", command.as_str()),
        AgentTraceKind::FileRead { path, .. } => ("search", path.as_str()),
        AgentTraceKind::Diff { path, .. } => ("git-branch", path.as_str()),
        AgentTraceKind::Test { name, .. } => ("check-check", name.as_str()),
        AgentTraceKind::Tool { name, .. } => ("settings", name.as_str()),
        AgentTraceKind::Activity { label } => ("activity", label.as_str()),
    };
    let line = text
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("");
    let mut chars = line.trim().chars().filter(|ch| !ch.is_control());
    let mut preview = chars.by_ref().take(180).collect::<String>();
    if chars.next().is_some() || text.lines().skip(1).any(|line| !line.trim().is_empty()) {
        preview.push('…');
    }
    (icon, preview)
}

#[cfg(test)]
thread_local! {pub(super) static MOTION_FRAMES:std::cell::Cell<usize>=const {std::cell::Cell::new(0)};}

/// The visible dot owns motion; command text stays opaque. No recurring task
/// runs while the indicator is offscreen, completed or under reduced motion.
pub(super) fn running_dot(color: Hsla, reduced: bool) -> impl IntoElement {
    static EPOCH: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    canvas(
        |_, _, _| (),
        move |bounds, (), window, _cx| {
            if !bounds.intersects(&window.content_mask().bounds) {
                return;
            }

            let alpha = if reduced {
                1.0
            } else {
                let phase = EPOCH
                    .get_or_init(std::time::Instant::now)
                    .elapsed()
                    .as_secs_f32()
                    / 1.2;
                window.request_animation_frame();
                #[cfg(test)]
                MOTION_FRAMES.with(|count| count.set(count.get() + 1));
                0.75 + 0.25 * (phase * std::f32::consts::TAU).sin()
            };
            window.paint_quad(
                fill(bounds, color.opacity(alpha)).corner_radii(bounds.size.width * 0.5),
            );
        },
    )
    .size(px(6.0))
    .flex_shrink_0()
}

pub(super) fn completed_work(trace: &AgentTraceEvent) -> bool {
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

pub(super) fn operation_status(detail: &AgentTraceKind) -> Option<AgentTraceStatus> {
    match detail {
        AgentTraceKind::Command { status, .. }
        | AgentTraceKind::FileRead { status, .. }
        | AgentTraceKind::Diff { status, .. }
        | AgentTraceKind::Test { status, .. }
        | AgentTraceKind::Tool { status, .. } => Some(*status),
        AgentTraceKind::Activity { .. } => None,
    }
}

pub(super) fn status_label(status: AgentTraceStatus) -> String {
    t!(match status {
        AgentTraceStatus::Running => "agents.activity.running",
        AgentTraceStatus::Pending => "agents.activity.pending",
        AgentTraceStatus::Succeeded => "agents.activity.done",
        AgentTraceStatus::Failed => "agents.activity.failed",
        AgentTraceStatus::Cancelled => "agents.activity.cancelled",
        AgentTraceStatus::Unknown => "agents.activity.unknown",
    })
    .to_string()
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
        traces: &[&AgentTraceEvent],
        frame: ActivityFrame,
        inline: bool,
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
        let (icon, title) = operation_preview(&last.detail);
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
                    .flex_col()
                    .items_start()
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
                            .flex_shrink_0()
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
                                if expanded {
                                    this.activity.pending_reveal = Some(group_id);
                                } else {
                                    this.activity.pending_open = Some(group_id);
                                }
                                cx.notify();
                            })),
                    )
                    .child(
                        div()
                            .debug_selector(|| format!("agent-work-preview-{group_id}"))
                            .w_full()
                            .min_w(px(0.0))
                            .flex()
                            .items_center()
                            .gap(px(8.0))
                            .pl(px(10.0))
                            .pb(px(4.0))
                            .child(lucide_icon(icon, 12.0, ShellDeckColors::text_muted()))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .overflow_hidden()
                                    .whitespace_nowrap()
                                    .text_ellipsis()
                                    .text_size(px(11.0))
                                    .text_color(ShellDeckColors::text_muted())
                                    .child(title),
                            ),
                    ),
            );
        if expanded && inline {
            // Keep the conversation as the sole scroll owner. All structured
            // output remains inline, including file changes and test summaries.
            column = column.children(traces.iter().map(|trace| {
                self.render_trace(trace, self.focused_trace == Some(trace.id), window, cx)
            }));
            column = column.on_children_prepainted(move |bounds, _, _| {
                if let (Some(header), Some(last)) = (bounds.first(), bounds.last()) {
                    frame.borrow_mut().push(ActivityBounds {
                        group: group_id,
                        header: *header,
                        bottom: last.bottom(),
                    });
                }
            });
        }
        column
            .id(("agent-work-group", uuid_key(group_id)))
            .anchor_scroll(
                (inline && self.activity.pending_reveal == Some(group_id))
                    .then(|| self.activity.anchor.clone()),
            )
            .into_any_element()
    }

    pub(super) fn render_activity_viewport(
        &mut self,
        session: Uuid,
        output: impl IntoElement,
        frame: ActivityFrame,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let reveal = self.activity.pending_reveal.take();
        let open = self.activity.pending_open.take();
        if let Some(id) = reveal.or(open.filter(|_| self.timeline.virtualized)) {
            if self.timeline.virtualized {
                self.timeline.reveal(id, true);
            } else {
                self.activity.anchor.scroll_to(window, cx);
            }
            window.on_next_frame(|window, _| window.refresh());
            window.request_animation_frame();
        }
        let weak = cx.entity().downgrade();
        let scroll = self.scroll.clone();
        let mut viewport = div()
            .relative()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .min_w_0()
            .child(output)
            .on_children_prepainted(move |_, _, cx| {
                let bounds = scroll.bounds();
                let legacy = frame
                    .borrow()
                    .iter()
                    .filter(|group| {
                        group.header.bottom() < bounds.top()
                            && group.bottom > bounds.top() + gpui::px(40.0)
                    })
                    .max_by(|a, b| {
                        a.header
                            .top()
                            .partial_cmp(&b.header.top())
                            .unwrap_or(std::cmp::Ordering::Equal)
                    })
                    .map(|group| FloatingGroup {
                        session,
                        group: group.group,
                    });
                let _ = weak.update(cx, |view, cx| {
                    let active = if view.timeline.virtualized {
                        view.timeline
                            .floating_group()
                            .map(|group| FloatingGroup { session, group })
                    } else {
                        legacy
                    };
                    if view.sessions.selected_id() == Some(session)
                        && view.activity.floating != active
                    {
                        view.activity.floating = active;
                        cx.notify();
                    }
                });
            });
        if let Some(group) = self
            .activity
            .floating
            .filter(|group| group.session == session)
        {
            viewport = viewport.child(
                div()
                    .absolute()
                    .top(px(8.0))
                    .right(px(24.0))
                    .occlude()
                    .child(
                        Button::new(
                            "agent-collapse-floating",
                            t!("agents.activity.collapse").to_string(),
                        )
                        .debug_selector(|| "agent-collapse-floating".into())
                        .variant(ButtonVariant::Secondary)
                        .size(ButtonSize::Sm)
                        .icon(IconSource::from("chevrons-up"))
                        .on_click(cx.listener(move |view, _, _, cx| {
                            view.collapse_activity_group(group, cx)
                        })),
                    ),
            );
        }
        viewport.into_any_element()
    }

    fn collapse_activity_group(&mut self, group: FloatingGroup, cx: &mut Context<Self>) {
        if self.sessions.selected_id() != Some(group.session) {
            return;
        }
        let members = self.sessions.get(group.session).and_then(|session| {
            group_completed_work(merged_timeline(session))
                .into_iter()
                .find_map(|item| match item {
                    TimelineItem::CompletedWork(traces)
                        if traces.first().is_some_and(|first| first.id == group.group) =>
                    {
                        Some(traces.iter().map(|trace| trace.id).collect::<HashSet<_>>())
                    }
                    _ => None,
                })
        });
        let Some(members) = members else {
            return;
        };
        if let Some(ui) = self.session_ui.get_mut(&group.session) {
            ui.expanded_activity.retain(|id| !members.contains(id));
        }
        if self.focused_trace.is_some_and(|id| members.contains(&id)) {
            self.focused_trace = None;
        }
        self.activity.floating = None;
        self.activity.pending_reveal = Some(group.group);
        cx.notify();
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
