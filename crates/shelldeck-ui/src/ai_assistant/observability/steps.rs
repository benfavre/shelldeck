//! Suivi steps: the observed trace read as Comprendre, Planifier, Modifier,
//! Vérifier and Résumer.
//!
//! The steps are a reading aid derived only from typed events and messages.
//! Nothing here guesses progress the trace does not show.

use std::collections::{BTreeMap, HashSet};

use gpui::prelude::*;
use gpui::*;
use shelldeck_core::agent_session::{
    AgentMessageRole, AgentSession, AgentSessionStatus, AgentTraceKind, AgentTraceStatus,
};

use super::super::format_duration;
use super::MONO;
use crate::scale::px;
use crate::t;
use crate::theme::ShellDeckColors;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum StepKind {
    Understand,
    Plan,
    Modify,
    Verify,
    Summarize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum StepState {
    Pending,
    Running,
    Done,
    Failed,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct StepCounts {
    pub events: usize,
    pub reads: usize,
    pub files: usize,
    pub additions: u32,
    pub deletions: u32,
    pub passed: usize,
    pub failed: usize,
    pub running: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Step {
    pub kind: StepKind,
    pub state: StepState,
    pub first_ms: Option<i64>,
    pub last_ms: Option<i64>,
    pub counts: StepCounts,
}

/// Commands that only look at the working tree.
const READ_COMMANDS: &[&str] = &[
    "rg",
    "grep",
    "find",
    "fd",
    "ls",
    "tree",
    "cat",
    "head",
    "tail",
    "wc",
    "sed -n",
    "git status",
    "git diff",
    "git log",
    "git show",
    "git blame",
];

/// Commands that check the work without being a test run. Test runs already
/// arrive as `Test` events from the runtime.
const CHECK_COMMANDS: &[&str] = &[
    "cargo check",
    "cargo clippy",
    "cargo build",
    "cargo fmt --check",
    "tsc",
    "eslint",
    "ruff",
    "mypy",
    "go vet",
    "npm run lint",
    "npm run build",
    "pnpm lint",
    "pnpm build",
    "bun run build",
    "make check",
];

/// The command a shell wrapper (`bash -lc '…'`) actually runs.
fn command_body(command: &str) -> &str {
    let trimmed = command.trim();
    for wrapper in ["bash -lc ", "bash -c ", "sh -c ", "zsh -lc ", "zsh -c "] {
        if let Some(rest) = trimmed.strip_prefix(wrapper) {
            return rest.trim().trim_matches(['\'', '"']).trim();
        }
    }
    trimmed
}

fn starts_with_command(body: &str, candidates: &[&str]) -> bool {
    candidates.iter().any(|candidate| {
        body == *candidate
            || body
                .strip_prefix(candidate)
                .is_some_and(|rest| rest.starts_with(' '))
    })
}

fn tool_step(name: &str) -> Option<StepKind> {
    let name = name.to_ascii_lowercase();
    let has = |keys: &[&str]| keys.iter().any(|key| name.contains(key));
    if has(&[
        "read", "grep", "glob", "search", "find", "list", "view", "fetch",
    ]) {
        Some(StepKind::Understand)
    } else if has(&["todo", "plan", "task", "think"]) {
        Some(StepKind::Plan)
    } else if has(&[
        "edit", "write", "patch", "create", "delete", "move", "rename",
    ]) {
        Some(StepKind::Modify)
    } else if has(&["test", "lint", "check", "build"]) {
        Some(StepKind::Verify)
    } else {
        None
    }
}

fn record(step: &mut Step, at_ms: i64) {
    step.counts.events += 1;
    step.first_ms = Some(step.first_ms.map_or(at_ms, |first| first.min(at_ms)));
    step.last_ms = Some(step.last_ms.map_or(at_ms, |last| last.max(at_ms)));
}

fn index(kind: StepKind) -> usize {
    match kind {
        StepKind::Understand => 0,
        StepKind::Plan => 1,
        StepKind::Modify => 2,
        StepKind::Verify => 3,
        StepKind::Summarize => 4,
    }
}

/// The five steps of a session. An event that no rule classifies (an
/// unknown command, an activity label) belongs with the work around it: the
/// step of the previous event. Agent messages followed by more work are
/// planning notes; an agent message after the last event is the summary.
pub(super) fn session_steps(session: &AgentSession) -> Vec<Step> {
    let mut steps: Vec<Step> = [
        StepKind::Understand,
        StepKind::Plan,
        StepKind::Modify,
        StepKind::Verify,
        StepKind::Summarize,
    ]
    .into_iter()
    .map(|kind| Step {
        kind,
        state: StepState::Pending,
        first_ms: None,
        last_ms: None,
        counts: StepCounts::default(),
    })
    .collect();

    let mut read_paths = HashSet::new();
    let mut changed: BTreeMap<&str, (u32, u32)> = BTreeMap::new();
    let mut previous: Option<StepKind> = None;
    for event in &session.trace {
        let classified = match &event.detail {
            AgentTraceKind::FileRead { path, .. } => {
                read_paths.insert(path.as_str());
                steps[index(StepKind::Understand)].counts.reads += 1;
                Some(StepKind::Understand)
            }
            AgentTraceKind::Diff {
                path,
                additions,
                deletions,
                ..
            } => {
                changed.insert(path.as_str(), (*additions, *deletions));
                Some(StepKind::Modify)
            }
            AgentTraceKind::Test { status, .. } => {
                let counts = &mut steps[index(StepKind::Verify)].counts;
                match status {
                    AgentTraceStatus::Succeeded => counts.passed += 1,
                    AgentTraceStatus::Failed | AgentTraceStatus::Cancelled => counts.failed += 1,
                    AgentTraceStatus::Pending
                    | AgentTraceStatus::Running
                    | AgentTraceStatus::Unknown => counts.running += 1,
                }
                Some(StepKind::Verify)
            }
            AgentTraceKind::Command { command, .. } => {
                let body = command_body(command);
                if starts_with_command(body, CHECK_COMMANDS) {
                    Some(StepKind::Verify)
                } else if starts_with_command(body, READ_COMMANDS) {
                    Some(StepKind::Understand)
                } else {
                    None
                }
            }
            AgentTraceKind::Tool { name, .. } => tool_step(name),
            AgentTraceKind::Activity { .. } => None,
        };
        let fallback = if matches!(event.detail, AgentTraceKind::Activity { .. }) {
            StepKind::Plan
        } else {
            StepKind::Understand
        };
        let kind = classified.or(previous).unwrap_or(fallback);
        record(&mut steps[index(kind)], event.at_ms);
        previous = Some(kind);
    }
    steps[index(StepKind::Understand)].counts.files = read_paths.len();
    let modify = &mut steps[index(StepKind::Modify)].counts;
    modify.files = changed.len();
    for (additions, deletions) in changed.values() {
        modify.additions = modify.additions.saturating_add(*additions);
        modify.deletions = modify.deletions.saturating_add(*deletions);
    }

    let last_trace_ms = session.trace.iter().map(|event| event.at_ms).max();
    for message in session
        .messages
        .iter()
        .filter(|message| message.role == AgentMessageRole::Agent)
    {
        let after_work = last_trace_ms.is_none_or(|last| message.at_ms >= last);
        let kind = if after_work {
            StepKind::Summarize
        } else {
            StepKind::Plan
        };
        record(&mut steps[index(kind)], message.at_ms);
    }

    let active = session.status.is_active();
    for step in steps.iter_mut().take(4) {
        step.state = if step.counts.events == 0 {
            StepState::Pending
        } else if active && Some(step.kind) == previous {
            StepState::Running
        } else if step.kind == StepKind::Verify && step.counts.failed > 0 {
            StepState::Failed
        } else {
            StepState::Done
        };
    }
    let summary = &mut steps[index(StepKind::Summarize)];
    summary.state = match session.status {
        AgentSessionStatus::Completed => StepState::Done,
        AgentSessionStatus::Failed | AgentSessionStatus::Cancelled => StepState::Failed,
        _ if active && summary.counts.events > 0 => StepState::Running,
        _ => StepState::Pending,
    };
    steps
}

fn step_title(kind: StepKind) -> String {
    let key = match kind {
        StepKind::Understand => "ai.observability.step.understand",
        StepKind::Plan => "ai.observability.step.plan",
        StepKind::Modify => "ai.observability.step.modify",
        StepKind::Verify => "ai.observability.step.verify",
        StepKind::Summarize => "ai.observability.step.summarize",
    };
    t!(key).to_string()
}

fn step_detail(step: &Step) -> Option<String> {
    if step.counts.events == 0 {
        return None;
    }
    let counts = &step.counts;
    Some(match step.kind {
        StepKind::Understand => t!(
            "ai.observability.step.understand_detail",
            reads = counts.reads,
            files = counts.files
        )
        .to_string(),
        StepKind::Plan => {
            t!("ai.observability.step.plan_detail", count = counts.events).to_string()
        }
        StepKind::Modify => t!(
            "ai.observability.step.modify_detail",
            files = counts.files,
            additions = counts.additions,
            deletions = counts.deletions
        )
        .to_string(),
        StepKind::Verify => t!(
            "ai.observability.step.verify_detail",
            passed = counts.passed,
            failed = counts.failed,
            running = counts.running
        )
        .to_string(),
        StepKind::Summarize => t!("ai.observability.step.summarize_detail").to_string(),
    })
}

/// Completed share of the test runs, when any are known.
fn verify_progress(counts: &StepCounts) -> Option<f32> {
    let done = counts.passed + counts.failed;
    let total = done + counts.running;
    (total > 0).then(|| done as f32 / total as f32)
}

pub(super) fn render_steps(steps: &[Step], now_ms: i64) -> AnyElement {
    let mut list = div()
        .flex()
        .flex_col()
        .ml(px(5.0))
        .pl(px(14.0))
        .border_l_1()
        .border_color(ShellDeckColors::border());
    for step in steps {
        let dot = match step.state {
            StepState::Done => ShellDeckColors::success(),
            StepState::Running => ShellDeckColors::primary(),
            StepState::Failed => ShellDeckColors::error(),
            StepState::Pending => ShellDeckColors::border(),
        };
        let timing = match (step.first_ms, step.last_ms) {
            (Some(first), Some(last)) => {
                let end = if step.state == StepState::Running {
                    now_ms.max(last)
                } else {
                    last
                };
                format_duration(end.saturating_sub(first).max(0) as u64)
            }
            _ => t!("ai.observability.step.pending").to_string(),
        };
        let mut row = div()
            .relative()
            .flex()
            .flex_col()
            .gap(px(2.0))
            .pb(px(10.0))
            .child(
                div()
                    .absolute()
                    .left(gpui::px(-19.0))
                    .top(gpui::px(3.0))
                    .size(gpui::px(9.0))
                    .rounded_full()
                    .border_2()
                    .border_color(ShellDeckColors::bg_primary())
                    .bg(dot),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .min_w(px(0.0))
                    .text_size(px(10.0))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .truncate()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(if step.state == StepState::Pending {
                                ShellDeckColors::text_muted()
                            } else {
                                ShellDeckColors::text_primary()
                            })
                            .child(step_title(step.kind)),
                    )
                    .child(
                        div()
                            .flex_shrink_0()
                            .font_family(MONO)
                            .text_size(px(8.5))
                            .text_color(ShellDeckColors::text_muted())
                            .child(timing),
                    ),
            );
        if let Some(detail) = step_detail(step) {
            row = row.child(
                div().flex().min_w(px(0.0)).child(
                    div()
                        .flex_1()
                        .min_w(px(0.0))
                        .truncate()
                        .text_size(px(9.0))
                        .text_color(ShellDeckColors::text_muted())
                        .child(detail),
                ),
            );
        }
        if step.kind == StepKind::Verify && step.state == StepState::Running {
            if let Some(fraction) = verify_progress(&step.counts) {
                row = row.child(
                    div()
                        .mt(px(4.0))
                        .h(gpui::px(3.0))
                        .w_full()
                        .rounded_full()
                        .overflow_hidden()
                        .bg(ShellDeckColors::border())
                        .child(
                            div()
                                .h_full()
                                .w(relative(fraction))
                                .rounded_full()
                                .bg(ShellDeckColors::primary()),
                        ),
                );
            }
        }
        list = list.child(row);
    }
    list.into_any_element()
}

#[cfg(test)]
mod tests {
    use super::{session_steps, StepKind, StepState};
    use shelldeck_core::agent_runtime::{AgentAccessMode, AgentProvider, AgentTarget};
    use shelldeck_core::agent_session::{
        AgentExecutionContext, AgentMessage, AgentMessageRole, AgentSession, AgentSessionStatus,
        AgentTraceEvent, AgentTraceKind, AgentTraceStatus,
    };
    use uuid::Uuid;

    fn event(at_ms: i64, detail: AgentTraceKind) -> AgentTraceEvent {
        AgentTraceEvent {
            id: Uuid::new_v4(),
            sequence: at_ms as u64,
            at_ms,
            correlation_id: None,
            detail,
        }
    }

    fn agent_message(at_ms: i64, text: &str) -> AgentMessage {
        AgentMessage {
            id: Uuid::new_v4(),
            sequence: at_ms as u64,
            role: AgentMessageRole::Agent,
            text: text.to_string(),
            at_ms,
        }
    }

    fn state(steps: &[super::Step], kind: StepKind) -> (StepState, usize) {
        let step = steps.iter().find(|step| step.kind == kind).unwrap();
        (step.state, step.counts.events)
    }

    // SDTEST-1930
    #[test]
    fn sdtest_1930_steps_follow_the_typed_trace_without_inventing_progress() {
        let mut session = AgentSession::new(
            "Cache",
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
        let idle = session_steps(&session);
        assert!(idle
            .iter()
            .all(|step| step.state == StepState::Pending && step.counts.events == 0));

        session.status = AgentSessionStatus::Running;
        session.messages = vec![agent_message(15, "Je limite l’invalidation.")];
        session.trace = vec![
            event(
                10,
                AgentTraceKind::FileRead {
                    path: "src/grid.rs".to_string(),
                    status: AgentTraceStatus::Succeeded,
                    line_start: None,
                    line_end: None,
                },
            ),
            event(
                12,
                AgentTraceKind::Command {
                    command: "bash -lc 'rg invalidate src'".to_string(),
                    status: AgentTraceStatus::Succeeded,
                    exit_code: Some(0),
                    summary: None,
                },
            ),
            event(
                14,
                AgentTraceKind::Activity {
                    label: "Thinking".to_string(),
                },
            ),
            event(
                20,
                AgentTraceKind::Diff {
                    path: "src/cache.rs".to_string(),
                    status: AgentTraceStatus::Succeeded,
                    additions: 42,
                    deletions: 11,
                    preview: None,
                },
            ),
            event(
                21,
                AgentTraceKind::Command {
                    command: "mkdir -p out".to_string(),
                    status: AgentTraceStatus::Succeeded,
                    exit_code: Some(0),
                    summary: None,
                },
            ),
            event(
                30,
                AgentTraceKind::Test {
                    name: "cargo test -p shelldeck-terminal".to_string(),
                    status: AgentTraceStatus::Succeeded,
                    summary: None,
                },
            ),
            event(
                32,
                AgentTraceKind::Test {
                    name: "cargo test -p shelldeck-ui".to_string(),
                    status: AgentTraceStatus::Running,
                    summary: None,
                },
            ),
        ];

        let running = session_steps(&session);
        // The read, the wrapped `rg` and the activity that follows them.
        assert_eq!(state(&running, StepKind::Understand), (StepState::Done, 3));
        // An agent message followed by more work is a planning note.
        assert_eq!(state(&running, StepKind::Plan), (StepState::Done, 1));
        // An unclassified command after a change stays with the change.
        assert_eq!(state(&running, StepKind::Modify), (StepState::Done, 2));
        let modify = &running[2].counts;
        assert_eq!(
            (modify.files, modify.additions, modify.deletions),
            (1, 42, 11)
        );
        assert_eq!(state(&running, StepKind::Verify), (StepState::Running, 2));
        assert_eq!(
            (running[3].counts.passed, running[3].counts.running),
            (1, 1)
        );
        assert_eq!(
            state(&running, StepKind::Summarize),
            (StepState::Pending, 0)
        );

        session.trace.push(event(
            40,
            AgentTraceKind::Test {
                name: "cargo test -p shelldeck-ui".to_string(),
                status: AgentTraceStatus::Failed,
                summary: None,
            },
        ));
        session
            .messages
            .push(agent_message(50, "Un test échoue encore."));
        session.status = AgentSessionStatus::Completed;
        let finished = session_steps(&session);
        assert_eq!(state(&finished, StepKind::Verify), (StepState::Failed, 3));
        assert_eq!(state(&finished, StepKind::Summarize), (StepState::Done, 1));
    }
}
