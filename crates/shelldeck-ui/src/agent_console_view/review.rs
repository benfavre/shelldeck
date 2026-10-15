//! Actual repository review, kept separate from provider-reported activity.

use super::*;
use std::collections::BTreeSet;
use std::sync::Arc;

use adabraka_ui::overlays::dialog::{Dialog as ReviewDialog, DialogSize};
use shelldeck_core::agent_review::{
    attribute_shared_turn, AgentReviewBackend, ChangeAttribution, FileDiff, RepositorySnapshot,
    ReviewAction, ReviewPlan, ReviewSection, ReviewTurn,
};

pub(super) struct AgentReviewUi {
    sessions: HashMap<Uuid, SessionReviewUi>,
    pending: Option<PendingReviewPlan>,
    dialog: Option<Entity<ReviewDialog>>,
    authority_error: Option<String>,
    wrap_lines: bool,
}

struct PendingReviewPlan {
    session_id: Uuid,
    context: AgentExecutionContext,
    snapshot_id: Uuid,
    directory: PathBuf,
    plan: ReviewPlan,
}

impl AgentReviewUi {
    pub(super) fn new() -> Self {
        Self {
            sessions: HashMap::new(),
            pending: None,
            dialog: None,
            authority_error: None,
            wrap_lines: true,
        }
    }
}

#[derive(Default)]
struct SessionReviewUi {
    snapshot: Option<Arc<RepositorySnapshot>>,
    before: Option<RepositorySnapshot>,
    after: Option<RepositorySnapshot>,
    turns: Vec<Arc<ReviewTurn>>,
    loading: bool,
    mutating: bool,
    generation: u64,
    error: Option<String>,
    baseline_error: Option<String>,
    context: Option<AgentExecutionContext>,
    expanded: BTreeSet<String>,
    diff_pages: HashMap<String, usize>,
    commit_input: Option<Entity<InputState>>,
    comment: Option<ReviewLineComment>,
}

struct ReviewLineComment {
    path: String,
    old_line: Option<u32>,
    new_line: Option<u32>,
    snapshot_id: Uuid,
    input: Entity<InputState>,
}

impl AgentConsoleView {
    fn review_context_matches(
        &self,
        session_id: Uuid,
        context: &AgentExecutionContext,
        cx: &Context<Self>,
    ) -> bool {
        self.sessions.selected_id() == Some(session_id)
            && matches!(context.target, AgentTarget::Local)
            && self.displayed_context(cx) == *context
            && self
                .review
                .sessions
                .get(&session_id)
                .and_then(|state| state.context.as_ref())
                == Some(context)
    }

    fn reject_review_context(&mut self, cx: &mut Context<Self>) {
        self.review.pending = None;
        self.review.dialog = None;
        // A session-specific error would disappear behind the very context
        // mismatch that caused the refusal. Keep this notice on the panel.
        self.review.authority_error = Some(t!("agents.review.context_changed").to_string());
        cx.notify();
    }

    /// Workspace supplies this result before spawning a local provider process.
    pub fn set_review_before_run(
        &mut self,
        session_id: Uuid,
        result: Result<RepositorySnapshot, String>,
        cx: &mut Context<Self>,
    ) {
        let context = self
            .sessions
            .get(session_id)
            .map(|session| session.context.clone());
        let state = self.review.sessions.entry(session_id).or_default();
        state.before = None;
        state.after = None;
        state.context = context;
        state.baseline_error = None;
        state.generation = state.generation.wrapping_add(1);
        match result {
            Ok(snapshot) => state.before = Some(snapshot),
            Err(error) => state.baseline_error = Some(error),
        }
        cx.notify();
    }

    /// Workspace captures this after process completion and before admitting the
    /// next queued run. No asynchronously late read can contaminate that turn.
    pub fn set_review_after_run(
        &mut self,
        session_id: Uuid,
        result: Result<RepositorySnapshot, String>,
        cx: &mut Context<Self>,
    ) {
        let state = self.review.sessions.entry(session_id).or_default();
        state.after = None;
        match result {
            Ok(snapshot) => state.after = Some(snapshot),
            Err(error) => state.baseline_error = Some(error),
        }
        cx.notify();
    }

    pub(super) fn finish_review_run(&mut self, session_id: Uuid, cx: &mut Context<Self>) {
        let state = self.review.sessions.entry(session_id).or_default();
        state.loading = false;
        let before = state.before.take();
        let after = state.after.take();
        if let Some(after) = after {
            let after = Arc::new(after);
            state.snapshot = Some(after.clone());
            if let Some(before) = before {
                // Provider path/diff traces are hints, never ownership evidence.
                // Adapters with exact tool edit bytes can pass OwnedFileEdit to
                // attribute_turn; current CLI trace formats do not supply them.
                match attribute_shared_turn(Arc::new(before), after, &[]) {
                    Ok(turn) => {
                        state.turns.push(Arc::new(turn));
                        // Exact snapshots can be large; retain at most two turns
                        // in memory. The conversation itself remains durable.
                        if state.turns.len() > 2 {
                            state.turns.remove(0);
                        }
                    }
                    Err(error) => state.baseline_error = Some(error.to_string()),
                }
            }
        } else if state.baseline_error.is_none() {
            state.baseline_error = Some(t!("agents.review.no_baseline").to_string());
        }
        cx.notify();
    }

    pub(super) fn refresh_review(&mut self, cx: &mut Context<Self>) {
        let Some(session_id) = self.sessions.selected_id() else {
            return;
        };
        let context = self.displayed_context(cx);
        self.review.authority_error = None;
        let state = self.review.sessions.entry(session_id).or_default();
        if state.loading || state.mutating {
            return;
        }
        state.generation = state.generation.wrapping_add(1);
        let generation = state.generation;
        state.context = Some(context.clone());
        state.error = None;
        if !matches!(context.target, AgentTarget::Local) {
            state.snapshot = None;
            state.error = Some(t!("agents.review.remote").to_string());
            cx.notify();
            return;
        }
        state.loading = true;
        let workdir = PathBuf::from(&context.workdir);
        let task = cx.background_executor().spawn(async move {
            AgentReviewBackend::default()
                .observe(&workdir)
                .map_err(|error| error.to_string())
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                let Some(state) = this.review.sessions.get_mut(&session_id) else {
                    return;
                };
                if state.generation != generation || state.context.as_ref() != Some(&context) {
                    return;
                }
                state.loading = false;
                state.expanded.clear();
                state.comment = None;
                match result {
                    Ok(snapshot) => state.snapshot = Some(Arc::new(snapshot)),
                    Err(error) => {
                        state.snapshot = None;
                        state.error = Some(error);
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn prepare_review_action(
        &mut self,
        session_id: Uuid,
        action: ReviewAction,
        cx: &mut Context<Self>,
    ) {
        if self
            .sessions
            .get(session_id)
            .is_none_or(|session| session.status.is_active())
        {
            return;
        }
        let Some(state) = self.review.sessions.get(&session_id) else {
            return;
        };
        let Some(context) = state.context.clone() else {
            self.reject_review_context(cx);
            return;
        };
        if !self.review_context_matches(session_id, &context, cx) {
            self.reject_review_context(cx);
            return;
        }
        if state.loading || state.mutating {
            return;
        }
        let Some(snapshot) = state.snapshot.clone() else {
            return;
        };
        match AgentReviewBackend::default().plan(&snapshot, action) {
            Ok(plan) => self.present_review_plan(session_id, context, snapshot, plan, cx),
            Err(error) => {
                self.review.sessions.entry(session_id).or_default().error = Some(error.to_string());
                cx.notify();
            }
        }
    }

    fn prepare_review_undo(
        &mut self,
        session_id: Uuid,
        turn_id: Uuid,
        path: String,
        cx: &mut Context<Self>,
    ) {
        if self
            .sessions
            .get(session_id)
            .is_none_or(|session| session.status.is_active())
        {
            return;
        }
        let Some(state) = self.review.sessions.get(&session_id) else {
            return;
        };
        let Some(context) = state.context.clone() else {
            self.reject_review_context(cx);
            return;
        };
        if !self.review_context_matches(session_id, &context, cx) {
            self.reject_review_context(cx);
            return;
        }
        if state.loading || state.mutating {
            return;
        }
        let Some(snapshot) = state.snapshot.clone() else {
            return;
        };
        let Some(turn) = state.turns.iter().find(|turn| turn.id == turn_id).cloned() else {
            return;
        };
        match AgentReviewBackend::default().plan_undo(&snapshot, &turn, &path) {
            Ok(plan) => self.present_review_plan(session_id, context, snapshot, plan, cx),
            Err(error) => {
                self.review.sessions.entry(session_id).or_default().error = Some(error.to_string());
                cx.notify();
            }
        }
    }

    fn present_review_plan(
        &mut self,
        session_id: Uuid,
        context: AgentExecutionContext,
        snapshot: Arc<RepositorySnapshot>,
        plan: ReviewPlan,
        cx: &mut Context<Self>,
    ) {
        if !self.review_context_matches(session_id, &context, cx)
            || self
                .review
                .sessions
                .get(&session_id)
                .and_then(|state| state.snapshot.as_ref())
                .is_none_or(|current| current.id != snapshot.id)
        {
            self.reject_review_context(cx);
            return;
        }
        self.review.authority_error = None;
        let parent = cx.entity();
        let close_parent = parent.clone();
        let confirm_parent = parent.clone();
        let cancel_parent = parent;
        let details = plan.summary.clone();
        let paths = plan.paths.join("\n");
        self.review.pending = Some(PendingReviewPlan {
            session_id,
            context,
            snapshot_id: snapshot.id,
            directory: snapshot.root.clone(),
            plan,
        });
        self.review.dialog = Some(cx.new(move |dialog_cx| {
            ReviewDialog::new(dialog_cx)
                .title(t!("agents.review.confirm_title").to_string())
                .description(t!("agents.review.confirm_body").to_string())
                .size(DialogSize::Lg)
                .on_close(move |_, cx| {
                    close_parent.update(cx, |this, cx| {
                        this.review.pending = None;
                        this.review.dialog = None;
                        cx.notify();
                    });
                })
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .min_w(px(0.0))
                        .gap(px(10.0))
                        .child(details)
                        .child(
                            div()
                                .font_family("JetBrains Mono")
                                .text_size(px(11.0))
                                .child(paths),
                        ),
                )
                .footer(
                    div()
                        .flex()
                        .justify_end()
                        .gap(px(8.0))
                        .child(
                            Button::new(
                                "agent-review-confirm-cancel",
                                t!("agents.review.cancel").to_string(),
                            )
                            .variant(ButtonVariant::Ghost)
                            .on_click(move |_, _, cx| {
                                cancel_parent.update(cx, |this, cx| {
                                    this.review.pending = None;
                                    this.review.dialog = None;
                                    cx.notify();
                                });
                            }),
                        )
                        .child(
                            Button::new(
                                "agent-review-confirm-apply",
                                t!("agents.review.confirm").to_string(),
                            )
                            .on_click(move |_, _, cx| {
                                confirm_parent.update(cx, |this, cx| this.execute_review_plan(cx));
                            }),
                        ),
                )
        }));
        cx.notify();
    }

    fn execute_review_plan(&mut self, cx: &mut Context<Self>) {
        let Some(pending) = self.review.pending.take() else {
            return;
        };
        let session_id = pending.session_id;
        self.review.dialog = None;
        if !self.review_context_matches(session_id, &pending.context, cx)
            || self
                .review
                .sessions
                .get(&session_id)
                .and_then(|state| state.snapshot.as_ref())
                .is_none_or(|snapshot| snapshot.id != pending.snapshot_id)
        {
            self.reject_review_context(cx);
            return;
        }
        if self
            .sessions
            .get(session_id)
            .is_none_or(|session| session.status.is_active())
        {
            self.review.sessions.entry(session_id).or_default().error =
                Some(t!("agents.review.busy").to_string());
            cx.notify();
            return;
        }
        let context = pending.context;
        let state = self.review.sessions.entry(session_id).or_default();
        if state.mutating {
            return;
        }
        state.mutating = true;
        state.error = None;
        state.generation = state.generation.wrapping_add(1);
        let generation = state.generation;
        let directory = pending.directory;
        let plan = pending.plan;
        let task = cx.background_executor().spawn(async move {
            let backend = AgentReviewBackend::default();
            backend
                .execute(&plan, plan.confirm())
                .map_err(|error| error.to_string())?;
            backend
                .observe(&directory)
                .map_err(|error| error.to_string())
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                let Some(state) = this.review.sessions.get_mut(&session_id) else {
                    return;
                };
                state.mutating = false;
                if state.generation != generation || state.context.as_ref() != Some(&context) {
                    return;
                }
                state.comment = None;
                match result {
                    Ok(snapshot) => {
                        state.snapshot = Some(Arc::new(snapshot));
                        state.error = None;
                    }
                    Err(error) => state.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// The root view renders this as its last child, so the dialog covers the
    /// entire agent workspace rather than only the inspector's allocated width.
    pub(super) fn render_review_dialog(&self) -> Option<AnyElement> {
        self.review
            .dialog
            .clone()
            .map(IntoElement::into_any_element)
    }

    fn begin_review_comment(
        &mut self,
        session_id: Uuid,
        snapshot_id: Uuid,
        path: String,
        old_line: Option<u32>,
        new_line: Option<u32>,
        cx: &mut Context<Self>,
    ) {
        let input = input_state(cx, String::new());
        let state = self.review.sessions.entry(session_id).or_default();
        if state.snapshot.as_ref().map(|snapshot| snapshot.id) != Some(snapshot_id) {
            return;
        }
        state.comment = Some(ReviewLineComment {
            path,
            old_line,
            new_line,
            snapshot_id,
            input,
        });
        cx.notify();
    }

    fn send_review_comment(&mut self, session_id: Uuid, cx: &mut Context<Self>) {
        let Some(state) = self.review.sessions.get_mut(&session_id) else {
            return;
        };
        let Some(comment) = state.comment.take() else {
            return;
        };
        if state.snapshot.as_ref().map(|snapshot| snapshot.id) != Some(comment.snapshot_id) {
            return;
        }
        let text = comment.input.read(cx).content().trim().to_owned();
        if text.is_empty() {
            state.comment = Some(comment);
            return;
        }
        let side = match (comment.old_line, comment.new_line) {
            (_, Some(line)) => format!("{}:{line} (+)", comment.path),
            (Some(line), None) => format!("{}:{line} (-)", comment.path),
            _ => comment.path,
        };
        let feedback = t!("agents.review.feedback", anchor = side, comment = text).to_string();
        if let Some(ui) = self.session_ui.get(&session_id) {
            let existing = ui.prompt.read(cx).content().to_owned();
            let combined = if existing.trim().is_empty() {
                feedback
            } else {
                format!("{existing}\n\n{feedback}")
            };
            ui.prompt
                .update(cx, |input, cx| input.replace_content(combined, cx));
        }
        self.schedule_library_save(cx);
        cx.notify();
    }

    pub(super) fn render_review_panel(
        &mut self,
        session: &AgentSession,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let session_id = session.id;
        let context = self.displayed_context(cx);
        let context_matches = self
            .review
            .sessions
            .get(&session_id)
            .and_then(|state| state.context.as_ref())
            == Some(&context);
        let state = self.review.sessions.entry(session_id).or_default();
        if state.commit_input.is_none() {
            state.commit_input = Some(input_state(cx, String::new()));
        }
        let loading = state.loading || state.mutating;
        let snapshot = if context_matches {
            state.snapshot.clone()
        } else {
            None
        };
        let error = if context_matches {
            state.error.clone()
        } else {
            None
        };
        let commit_input = state.commit_input.clone().expect("initialized above");
        let expanded = state.expanded.clone();
        let wrap_lines = self.review.wrap_lines;
        let mut body = div()
            .id("agent-real-review-body")
            .flex()
            .flex_col()
            .flex_1()
            .min_h(px(0.0))
            .min_w(px(0.0))
            .overflow_y_scroll()
            .p(px(8.0))
            .gap(px(8.0));
        if let Some(error) = &self.review.authority_error {
            body = body.child(
                div()
                    .text_size(px(11.0))
                    .text_color(ShellDeckColors::warning())
                    .child(error.clone()),
            );
        }
        let disabled = loading || session.status.is_active();
        let header = div()
            .flex()
            .items_center()
            .gap(px(6.0))
            .px(px(10.0))
            .py(px(8.0))
            .border_b_1()
            .border_color(ShellDeckColors::border())
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .text_size(px(12.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(t!("agents.review.title").to_string()),
            )
            .child(
                Button::new("agent-review-wrap", t!("agents.review.wrap").to_string())
                    .size(ButtonSize::Sm)
                    .variant(if wrap_lines {
                        ButtonVariant::Secondary
                    } else {
                        ButtonVariant::Ghost
                    })
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.review.wrap_lines = !this.review.wrap_lines;
                        cx.notify();
                    })),
            )
            .child(
                Button::new(
                    "agent-review-refresh",
                    t!("agents.review.refresh").to_string(),
                )
                .icon(IconSource::from("refresh-cw"))
                .size(ButtonSize::Sm)
                .variant(ButtonVariant::Ghost)
                .disabled(loading)
                .on_click(cx.listener(|this, _, _, cx| this.refresh_review(cx))),
            )
            .child(
                Button::new("agent-review-close", "")
                    .size(ButtonSize::Icon)
                    .variant(ButtonVariant::Ghost)
                    .icon(IconSource::from("x"))
                    .tooltip(t!("agents.parity.close_panel").to_string())
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.tools.panel_visible = false;
                        this.schedule_library_save(cx);
                        cx.notify();
                    })),
            );
        if !matches!(context.target, AgentTarget::Local) {
            body = body.child(inspector_empty_state(
                "server",
                t!("agents.review.remote").to_string(),
            ));
        } else {
            if loading {
                body = body.child(
                    div()
                        .text_size(px(11.0))
                        .text_color(ShellDeckColors::text_muted())
                        .child(t!("agents.review.loading").to_string()),
                );
            }
            if let Some(error) = error {
                body = body.child(
                    div()
                        .text_size(px(11.0))
                        .text_color(ShellDeckColors::error())
                        .child(error),
                );
            }
            if let Some(snapshot) = snapshot {
                body = body.child(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap(px(6.0))
                        .child(
                            Badge::new(
                                snapshot
                                    .branch
                                    .clone()
                                    .unwrap_or_else(|| t!("agents.review.detached").to_string()),
                            )
                            .variant(BadgeVariant::Outline),
                        )
                        .child(
                            div()
                                .min_w(px(0.0))
                                .text_size(px(10.0))
                                .text_color(ShellDeckColors::text_muted())
                                .child(snapshot.root.display().to_string()),
                        ),
                );
                for refusal in snapshot.refusals.iter().take(4) {
                    body = body.child(
                        div()
                            .text_size(px(10.0))
                            .text_color(ShellDeckColors::warning())
                            .child(refusal.clone()),
                    );
                }
                if snapshot.diffs.is_empty() {
                    body = body.child(inspector_empty_state(
                        "git-branch",
                        t!("agents.review.empty").to_string(),
                    ));
                }
                for (ordinal, diff) in snapshot.diffs.iter().enumerate() {
                    let key = format!("{}:{:?}", diff.path, diff.section);
                    let open = expanded.contains(&key);
                    let diff_path = diff.path.clone();
                    let action_path = diff_path.clone();
                    let mut paths = vec![diff_path.clone()];
                    if let Some(original) = &diff.original_path {
                        paths.push(original.clone());
                    }
                    let action_paths = paths;
                    let section = diff.section;
                    let counts = format_counts(diff);
                    let mut card = div()
                        .flex()
                        .flex_col()
                        .w_full()
                        .min_w(px(0.0))
                        .overflow_hidden()
                        .border_1()
                        .border_color(ShellDeckColors::border())
                        .rounded(px(6.0));
                    let mut controls = div()
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .gap(px(5.0))
                        .p(px(6.0))
                        .child(
                            Button::new(
                                SharedString::from(format!("agent-review-file-{ordinal}")),
                                diff.path.clone(),
                            )
                            .variant(ButtonVariant::Ghost)
                            .size(ButtonSize::Sm)
                            .icon(IconSource::from(if open {
                                "chevron-down"
                            } else {
                                "chevron-right"
                            }))
                            .on_click(cx.listener(
                                move |this, _, _, cx| {
                                    let state = this.review.sessions.entry(session_id).or_default();
                                    if !state.expanded.remove(&key) {
                                        state.expanded.insert(key.clone());
                                    }
                                    cx.notify();
                                },
                            )),
                        )
                        .child(
                            Badge::new(section_label(diff.section)).variant(BadgeVariant::Outline),
                        )
                        .child(
                            div()
                                .text_size(px(10.0))
                                .text_color(ShellDeckColors::text_muted())
                                .child(counts),
                        );
                    if diff.conflicted {
                        controls = controls.child(
                            Badge::new(t!("agents.review.conflict").to_string())
                                .variant(BadgeVariant::Destructive),
                        );
                    }
                    controls = controls.child(
                        Button::new(
                            SharedString::from(format!("agent-review-mutate-{ordinal}")),
                            if section == ReviewSection::Staged {
                                t!("agents.review.unstage").to_string()
                            } else {
                                t!("agents.review.stage").to_string()
                            },
                        )
                        .variant(ButtonVariant::Secondary)
                        .size(ButtonSize::Sm)
                        .disabled(disabled || diff.conflicted || !snapshot.can_mutate())
                        .on_click(cx.listener(move |this, _, _, cx| {
                            let action = if section == ReviewSection::Staged {
                                ReviewAction::UnstageFiles {
                                    paths: action_paths.clone(),
                                }
                            } else {
                                ReviewAction::StageFiles {
                                    paths: action_paths.clone(),
                                }
                            };
                            this.prepare_review_action(session_id, action, cx);
                        })),
                    );
                    card = card.child(controls);
                    if open {
                        let copied = diff.unified.clone();
                        card = card.child(
                            div().flex().justify_end().child(
                                Button::new(
                                    SharedString::from(format!("agent-review-copy-{ordinal}")),
                                    t!("agents.review.copy_diff").to_string(),
                                )
                                .variant(ButtonVariant::Ghost)
                                .size(ButtonSize::Sm)
                                .on_click(move |_, _, cx| {
                                    cx.write_to_clipboard(ClipboardItem::new_string(copied.clone()))
                                }),
                            ),
                        );
                        card = card.child(self.render_unified_diff(
                            session_id,
                            snapshot.id,
                            diff,
                            ordinal,
                            wrap_lines,
                            cx,
                        ));
                        let comment = self
                            .review
                            .sessions
                            .get(&session_id)
                            .and_then(|state| state.comment.as_ref())
                            .filter(|comment| comment.path == action_path)
                            .map(|comment| {
                                (comment.input.clone(), comment.new_line.or(comment.old_line))
                            });
                        if let Some((input, line)) = comment {
                            card = card.child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .p(px(6.0))
                                    .gap(px(5.0))
                                    .border_t_1()
                                    .border_color(ShellDeckColors::border())
                                    .child(
                                        div()
                                            .text_size(px(10.0))
                                            .text_color(ShellDeckColors::text_muted())
                                            .child(format!("{}:{}", diff.path, line.unwrap_or(0))),
                                    )
                                    .child(Input::new(&input).size(InputSize::Sm).placeholder(
                                        t!("agents.review.comment_placeholder").to_string(),
                                    ))
                                    .child(
                                        Button::new(
                                            SharedString::from(format!(
                                                "agent-review-comment-send-{ordinal}"
                                            )),
                                            t!("agents.review.add_comment").to_string(),
                                        )
                                        .size(ButtonSize::Sm)
                                        .variant(ButtonVariant::Secondary)
                                        .on_click(
                                            cx.listener(move |this, _, _, cx| {
                                                this.send_review_comment(session_id, cx)
                                            }),
                                        ),
                                    ),
                            );
                        }
                    }
                    body = body.child(card);
                }
                if snapshot.staged_count() > 0 {
                    let input = commit_input.clone();
                    body = body.child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(6.0))
                            .border_t_1()
                            .border_color(ShellDeckColors::border())
                            .pt(px(10.0))
                            .child(
                                Input::new(&commit_input)
                                    .size(InputSize::Sm)
                                    .placeholder(t!("agents.review.commit_message").to_string()),
                            )
                            .child(
                                div()
                                    .text_size(px(10.0))
                                    .text_color(ShellDeckColors::text_muted())
                                    .child(t!("agents.review.commit_notice").to_string()),
                            )
                            .child(
                                Button::new(
                                    "agent-review-commit",
                                    t!("agents.review.commit").to_string(),
                                )
                                .size(ButtonSize::Sm)
                                .disabled(disabled || !snapshot.can_mutate())
                                .on_click(cx.listener(
                                    move |this, _, _, cx| {
                                        let message = input.read(cx).content().to_owned();
                                        this.prepare_review_action(
                                            session_id,
                                            ReviewAction::Commit { message },
                                            cx,
                                        );
                                    },
                                )),
                            ),
                    );
                }
            } else if !loading {
                body = body.child(inspector_empty_state(
                    "git-branch",
                    t!("agents.review.load_hint").to_string(),
                ));
            }
        }
        div()
            .flex()
            .flex_col()
            .size_full()
            .min_w(px(0.0))
            .min_h(px(0.0))
            .child(header)
            .child(body)
            .into_any_element()
    }

    fn render_unified_diff(
        &self,
        session_id: Uuid,
        snapshot_id: Uuid,
        diff: &FileDiff,
        ordinal: usize,
        wrap: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        // adabraka has no source-diff gutter/comment widget. A semantic row
        // renderer keeps old/new anchors distinct, including deleted lines.
        const PAGE_LINES: usize = 300;
        let parsed = parse_unified_lines(&diff.unified);
        let page_key = format!("{}:{:?}", diff.path, diff.section);
        let pages = parsed.len().div_ceil(PAGE_LINES).max(1);
        let page = self
            .review
            .sessions
            .get(&session_id)
            .and_then(|state| state.diff_pages.get(&page_key))
            .copied()
            .unwrap_or(0)
            .min(pages - 1);
        let mut lines = div()
            .id(SharedString::from(format!("agent-review-diff-{ordinal}")))
            .w_full()
            .min_w(px(0.0))
            .overflow_x_scroll()
            .font_family("JetBrains Mono")
            .text_size(px(10.0));
        if pages > 1 {
            let previous_key = page_key.clone();
            let next_key = page_key;
            lines = lines.child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .justify_between()
                    .gap(px(4.0))
                    .p(px(5.0))
                    .child(
                        Button::new(
                            SharedString::from(format!("agent-diff-prev-{ordinal}")),
                            t!("agents.review.previous").to_string(),
                        )
                        .size(ButtonSize::Sm)
                        .variant(ButtonVariant::Ghost)
                        .disabled(page == 0)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.review
                                .sessions
                                .entry(session_id)
                                .or_default()
                                .diff_pages
                                .insert(previous_key.clone(), page.saturating_sub(1));
                            cx.notify();
                        })),
                    )
                    .child(t!("agents.review.page", current = page + 1, total = pages).to_string())
                    .child(
                        Button::new(
                            SharedString::from(format!("agent-diff-next-{ordinal}")),
                            t!("agents.review.next").to_string(),
                        )
                        .size(ButtonSize::Sm)
                        .variant(ButtonVariant::Ghost)
                        .disabled(page + 1 >= pages)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.review
                                .sessions
                                .entry(session_id)
                                .or_default()
                                .diff_pages
                                .insert(next_key.clone(), page + 1);
                            cx.notify();
                        })),
                    ),
            );
        }
        for (index, line) in parsed
            .into_iter()
            .enumerate()
            .skip(page * PAGE_LINES)
            .take(PAGE_LINES)
        {
            let color = match line.kind {
                DiffDisplayKind::Added => ShellDeckColors::success(),
                DiffDisplayKind::Removed => ShellDeckColors::error(),
                DiffDisplayKind::Meta => ShellDeckColors::text_muted(),
                DiffDisplayKind::Context => ShellDeckColors::text_primary(),
            };
            let tint = match line.kind {
                DiffDisplayKind::Added => ShellDeckColors::success().opacity(0.08),
                DiffDisplayKind::Removed => ShellDeckColors::error().opacity(0.08),
                _ => ShellDeckColors::bg_surface().opacity(0.0),
            };
            let mut text = div()
                .flex_1()
                .min_w(px(0.0))
                .text_color(color)
                .child(line.text.clone());
            if !wrap {
                text = text.whitespace_nowrap();
            }
            let mut row = div()
                .id(SharedString::from(format!(
                    "agent-review-line-{ordinal}-{index}"
                )))
                .flex()
                .min_w(px(0.0))
                .bg(tint)
                .child(diff_gutter(line.old_line))
                .child(diff_gutter(line.new_line))
                .child(text);
            if line.old_line.is_some() || line.new_line.is_some() {
                let path = diff.path.clone();
                row = row
                    .cursor_pointer()
                    .hover(|style| style.bg(ShellDeckColors::hover_bg()))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.begin_review_comment(
                            session_id,
                            snapshot_id,
                            path.clone(),
                            line.old_line,
                            line.new_line,
                            cx,
                        )
                    }));
            }
            lines = lines.child(row);
        }
        lines.into_any_element()
    }

    pub(super) fn render_turn_change_cards(
        &self,
        session: &AgentSession,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mut cards = div()
            .flex()
            .flex_col()
            .gap(px(8.0))
            .w_full()
            .min_w(px(0.0))
            .max_w(px(900.0))
            .mx_auto()
            .px(px(14.0))
            .py(px(6.0));
        let Some(state) = self.review.sessions.get(&session.id) else {
            return cards.into_any_element();
        };
        if let Some(error) = &state.baseline_error {
            cards = cards.child(
                div()
                    .text_size(px(11.0))
                    .text_color(ShellDeckColors::warning())
                    .child(t!("agents.review.baseline_unavailable", reason = error).to_string()),
            );
        }
        for turn in &state.turns {
            if turn.changes.is_empty() {
                continue;
            }
            let session_id = session.id;
            let turn_id = turn.id;
            let mut card = div()
                .flex()
                .flex_col()
                .w_full()
                .min_w(px(0.0))
                .overflow_hidden()
                .border_1()
                .border_color(ShellDeckColors::border())
                .rounded(px(7.0))
                .bg(ShellDeckColors::bg_surface());
            card = card.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .px(px(10.0))
                    .py(px(7.0))
                    .border_b_1()
                    .border_color(ShellDeckColors::border())
                    .child(lucide_icon(
                        "git-branch",
                        15.0,
                        ShellDeckColors::text_muted(),
                    ))
                    .child(div().flex_1().min_w(px(0.0)).text_size(px(12.0)).child(
                        t!("agents.review.turn_changes", count = turn.changes.len()).to_string(),
                    ))
                    .child(
                        Button::new(
                            SharedString::from(format!("agent-turn-review-{turn_id}")),
                            t!("agents.review.review").to_string(),
                        )
                        .variant(ButtonVariant::Secondary)
                        .size(ButtonSize::Sm)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if this.sessions.selected_id() == Some(session_id) {
                                this.tools.active = None;
                                this.tools.panel_visible = true;
                                this.inspector_tab = AgentInspectorTab::Changes;
                                this.refresh_review(cx);
                            }
                        })),
                    ),
            );
            card = card.child(
                div()
                    .px(px(10.0))
                    .py(px(5.0))
                    .text_size(px(10.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(t!("agents.review.unattributed").to_string()),
            );
            for (ordinal, change) in turn.changes.iter().enumerate() {
                let path = change.path.clone();
                let can_undo = change.attribution == ChangeAttribution::ProvenAgentEdit
                    && !session.status.is_active();
                let mut row = div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap(px(7.0))
                    .px(px(10.0))
                    .py(px(5.0))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .overflow_hidden()
                            .text_ellipsis()
                            .text_size(px(11.0))
                            .child(change.path.clone()),
                    );
                if change.baseline_dirty {
                    row = row.child(
                        Badge::new(t!("agents.review.baseline_dirty").to_string())
                            .variant(BadgeVariant::Outline),
                    );
                }
                row = row.child(
                    Button::new(
                        SharedString::from(format!("agent-turn-undo-{turn_id}-{ordinal}")),
                        t!("agents.review.undo").to_string(),
                    )
                    .variant(ButtonVariant::Ghost)
                    .size(ButtonSize::Sm)
                    .disabled(!can_undo)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.prepare_review_undo(session_id, turn_id, path.clone(), cx)
                    })),
                );
                card = card.child(row);
            }
            if turn
                .changes
                .iter()
                .all(|change| change.attribution != ChangeAttribution::ProvenAgentEdit)
            {
                card = card.child(
                    div()
                        .px(px(10.0))
                        .py(px(6.0))
                        .text_size(px(10.0))
                        .text_color(ShellDeckColors::text_muted())
                        .child(t!("agents.review.undo_unavailable").to_string()),
                );
            }
            cards = cards.child(card);
        }
        cards.into_any_element()
    }
}

fn diff_gutter(line: Option<u32>) -> Div {
    div()
        .w(px(30.0))
        .flex_shrink_0()
        .pr(px(4.0))
        .text_align(TextAlign::Right)
        .text_color(ShellDeckColors::text_muted())
        .child(line.map(|line| line.to_string()).unwrap_or_default())
}

fn section_label(section: ReviewSection) -> String {
    match section {
        ReviewSection::Staged => t!("agents.review.staged").to_string(),
        ReviewSection::Unstaged => t!("agents.review.unstaged").to_string(),
        ReviewSection::Untracked => t!("agents.review.untracked").to_string(),
    }
}

fn format_counts(diff: &FileDiff) -> String {
    match (diff.counts.additions, diff.counts.deletions) {
        (Some(added), Some(removed)) => format!("+{added} −{removed}"),
        _ => t!("agents.review.unknown").to_string(),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DiffDisplayKind {
    Meta,
    Context,
    Added,
    Removed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct DiffDisplayLine {
    old_line: Option<u32>,
    new_line: Option<u32>,
    kind: DiffDisplayKind,
    text: String,
}

fn parse_unified_lines(diff: &str) -> Vec<DiffDisplayLine> {
    let mut old_line: Option<u32> = None;
    let mut new_line: Option<u32> = None;
    let mut result = Vec::new();
    for text in diff.lines() {
        if text.starts_with("diff --git ") {
            old_line = None;
            new_line = None;
        }
        if text.starts_with("@@ ") {
            let mut pieces = text.split_whitespace();
            let _ = pieces.next();
            old_line = pieces
                .next()
                .and_then(|value| value.strip_prefix('-'))
                .and_then(|value| value.split(',').next())
                .and_then(|value| value.parse().ok());
            new_line = pieces
                .next()
                .and_then(|value| value.strip_prefix('+'))
                .and_then(|value| value.split(',').next())
                .and_then(|value| value.parse().ok());
            result.push(DiffDisplayLine {
                old_line: None,
                new_line: None,
                kind: DiffDisplayKind::Meta,
                text: text.to_owned(),
            });
            continue;
        }
        let kind = if old_line.is_none() || new_line.is_none() {
            DiffDisplayKind::Meta
        } else if text.starts_with('+') {
            DiffDisplayKind::Added
        } else if text.starts_with('-') {
            DiffDisplayKind::Removed
        } else if text.starts_with(' ') {
            DiffDisplayKind::Context
        } else {
            DiffDisplayKind::Meta
        };
        let old = matches!(kind, DiffDisplayKind::Removed | DiffDisplayKind::Context)
            .then_some(old_line)
            .flatten();
        let new = matches!(kind, DiffDisplayKind::Added | DiffDisplayKind::Context)
            .then_some(new_line)
            .flatten();
        if old.is_some() {
            old_line = old_line.and_then(|line| line.checked_add(1));
        }
        if new.is_some() {
            new_line = new_line.and_then(|line| line.checked_add(1));
        }
        result.push(DiffDisplayLine {
            old_line: old,
            new_line: new,
            kind,
            text: text.to_owned(),
        });
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;
    // SDTEST-2273 — SDUC-552
    #[test]
    fn sdtest_2273_diff_anchors_keep_deleted_new_context_and_no_newline_lines_distinct() {
        let lines = parse_unified_lines(
            "--- a/file\n+++ b/file\n@@ -10,2 +20,2 @@\n-old\n+new\n context\n\\ No newline at end of file\n@@ -0,0 +1 @@\n+first\n",
        );
        assert_eq!((lines[3].old_line, lines[3].new_line), (Some(10), None));
        assert_eq!((lines[4].old_line, lines[4].new_line), (None, Some(20)));
        assert_eq!((lines[5].old_line, lines[5].new_line), (Some(11), Some(21)));
        assert_eq!((lines[6].old_line, lines[6].new_line), (None, None));
        assert_eq!((lines[8].old_line, lines[8].new_line), (None, Some(1)));
    }
}
