//! User-started goal continuation and opt-in desktop-local recurring tasks.
use super::*;
use adabraka_ui::components::alert::Alert;
use adabraka_ui::components::checkbox::Checkbox;
use adabraka_ui::overlays::dialog::{Dialog as WorkflowDialog, DialogSize};
use shelldeck_core::agent_workflows::{
    check_provider_readiness, AgentGoal, AgentSchedule, AgentWorkflowStore, AgentWorkflows,
    AuthenticationReadiness, GoalObservation, GoalStatus, LocalReadinessExecutor,
    ProviderReadiness, WorkflowPauseReason, MAX_WORKFLOWS, SCHEDULE_RUN_LIMIT_MS,
};
use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    Arc,
};
use std::time::Duration;

#[derive(Clone, Copy, PartialEq, Eq)]
enum WorkflowOrigin {
    Goal(Uuid),
    Schedule(Uuid),
}
struct PendingWorkflow {
    origin: WorkflowOrigin,
    context: AgentExecutionContext,
    prompt: String,
}
struct WorkflowRun {
    origin: WorkflowOrigin,
    observation: GoalObservation,
}
#[derive(Clone, Copy)]
enum FormKind {
    Goal(Option<Uuid>),
    Schedule(Option<Uuid>),
}
#[derive(Clone)]
struct WorkflowForm {
    kind: FormKind,
    session_id: Uuid,
    context: AgentExecutionContext,
    objective: Entity<InputState>,
    verification: Entity<InputState>,
    progress: Entity<InputState>,
    title: Entity<InputState>,
    turns: Entity<InputState>,
    minutes: Entity<InputState>,
    interval: Entity<InputState>,
    automatic: Arc<AtomicBool>,
}
pub(super) struct AgentWorkflowsUi {
    state: AgentWorkflows,
    store: AgentWorkflowStore,
    authorized: bool,
    loading: bool,
    writable: bool,
    error: Option<String>,
    feedback: Entity<WorkflowFeedback>,
    pub(super) dialog: Option<Entity<WorkflowDialog>>,
    form: Option<WorkflowForm>,
    pending: HashMap<Uuid, PendingWorkflow>,
    runs: HashMap<Uuid, WorkflowRun>,
    poll_task: Option<Task<()>>,
    save_task: Option<Task<()>>,
    generation: Arc<AtomicU64>,
    write_lock: Arc<parking_lot::Mutex<()>>,
    readiness: Option<(AgentExecutionContext, Result<ProviderReadiness, String>)>,
    readiness_pending: Option<(Uuid, AgentExecutionContext)>,
}
impl AgentWorkflowsUi {
    pub(super) fn new(cx: &mut Context<AgentConsoleView>) -> Self {
        let store = AgentWorkflowStore::default();
        let restore_store = store.clone();
        cx.spawn(async move |this, cx| {
            let loaded = cx
                .background_executor()
                .spawn(async move { restore_store.load() })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.workflows.loading = false;
                match loaded {
                    Ok(state) => {
                        this.workflows.state = state;
                        this.workflows.writable = true;
                    }
                    Err(error) => this.workflows.error = Some(error.to_string()),
                }
                this.ensure_workflow_poll(cx);
                cx.notify();
            });
        })
        .detach();
        Self {
            state: AgentWorkflows::default(),
            store,
            authorized: false,
            loading: true,
            writable: false,
            error: None,
            feedback: cx.new(|_| WorkflowFeedback { error: None }),
            dialog: None,
            form: None,
            pending: HashMap::new(),
            runs: HashMap::new(),
            poll_task: None,
            save_task: None,
            generation: Arc::new(AtomicU64::new(0)),
            write_lock: Arc::new(parking_lot::Mutex::new(())),
            readiness: None,
            readiness_pending: None,
        }
    }
}
impl AgentConsoleView {
    /// Workspace owns this role gate and must refresh it before dispatch and
    /// after login/logout/mode/capability changes. Visibility alone is no grant.
    pub fn set_workflows_authorized(&mut self, authorized: bool, cx: &mut Context<Self>) {
        if self.workflows.authorized == authorized {
            return;
        }
        self.workflows.authorized = authorized;
        if !authorized {
            let stops = self
                .workflows
                .state
                .goals
                .iter_mut()
                .filter(|goal| {
                    goal.status == GoalStatus::Running
                        || (goal.status == GoalStatus::Ready
                            && goal.automatic_continuation
                            && goal.turns_used > 0)
                })
                .filter_map(|goal| {
                    let run_id = goal.active_run;
                    goal.pause(WorkflowPauseReason::AuthorizationLost, now_ms());
                    run_id
                })
                .collect::<Vec<_>>();
            let mut stops = stops;
            for task in &mut self.workflows.state.schedules {
                if let Some(run_id) = task.active_run {
                    stops.push(run_id);
                    task.pause(WorkflowPauseReason::AuthorizationLost);
                }
            }
            if !self.workflows.pending.is_empty() {
                self.pending_confirm = None;
            }
            self.workflows.pending.clear();
            for run_id in stops {
                cx.emit(AgentConsoleEvent::Stop(run_id));
            }
            self.save_agent_workflows(cx);
        }
        self.ensure_workflow_poll(cx);
        cx.notify();
    }
    fn save_agent_workflows(&mut self, cx: &mut Context<Self>) {
        if !self.workflows.writable {
            return;
        }
        let state = self.workflows.state.clone();
        let store = self.workflows.store.clone();
        let generation = self.workflows.generation.fetch_add(1, Ordering::SeqCst) + 1;
        let current = self.workflows.generation.clone();
        let lock = self.workflows.write_lock.clone();
        self.workflows.save_task = Some(cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let _guard = lock.lock();
                    if current.load(Ordering::SeqCst) != generation {
                        return Ok(());
                    }
                    store.save(&state).map_err(|error| error.to_string())
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if let Err(error) = result {
                    this.workflows.error = Some(error);
                    cx.notify();
                }
            });
        }));
    }
    fn ensure_workflow_poll(&mut self, cx: &mut Context<Self>) {
        if self.workflows.poll_task.is_some() {
            return;
        }
        self.workflows.poll_task = Some(cx.spawn(async move |this, cx| loop {
            cx.background_executor().timer(Duration::from_secs(1)).await;
            if this
                .update(cx, |this, cx| this.poll_agent_workflows(cx))
                .is_err()
            {
                break;
            }
        }));
    }
    /// Every tick rechecks identity/context. Workspace also revalidates the
    /// role and SSH connection immediately before receiving the normal Run.
    pub(super) fn poll_agent_workflows(&mut self, cx: &mut Context<Self>) {
        if self.workflows.loading || !self.workflows.authorized {
            return;
        }
        let now = now_ms();
        let mut changed = false;
        let mut stops = Vec::new();
        for goal in &mut self.workflows.state.goals {
            if goal.status == GoalStatus::Running && goal.elapsed_at(now) >= goal.time_limit_ms {
                goal.pause(WorkflowPauseReason::Budget, now);
                if let Some(run_id) = goal.active_run {
                    stops.push(run_id);
                }
                changed = true;
            }
        }
        for task in &mut self.workflows.state.schedules {
            if task.active_run.is_some()
                && task.pause_reason != Some(WorkflowPauseReason::Budget)
                && task
                    .last_run_ms
                    .is_some_and(|started| now.saturating_sub(started) >= SCHEDULE_RUN_LIMIT_MS)
            {
                task.pause(WorkflowPauseReason::Budget);
                if let Some(run_id) = task.active_run {
                    stops.push(run_id);
                }
                changed = true;
            }
        }
        for run_id in stops {
            cx.emit(AgentConsoleEvent::Stop(run_id));
        }
        if self.pending_confirm.is_none() && self.workflows.pending.is_empty() {
            let selected = self.sessions.selected_id();
            let displayed = self.displayed_context(cx);
            let contexts = self
                .sessions
                .sessions()
                .iter()
                .map(|session| {
                    (
                        session.id,
                        (
                            if selected == Some(session.id) {
                                displayed.clone()
                            } else {
                                session.context.clone()
                            },
                            session.status.is_active(),
                            session.archived,
                        ),
                    )
                })
                .collect::<HashMap<_, _>>();
            let mut dispatch = None;
            for task in &mut self.workflows.state.schedules {
                let was_enabled = task.enabled;
                // Busy sessions wait; a changed context or lost binding pauses.
                if contexts
                    .get(&task.session_id)
                    .is_some_and(|(_, busy, _)| *busy)
                {
                    continue;
                }
                if let Some(next) = task.claim_due(now, |id, context| {
                    contexts.get(&id).is_some_and(|(actual, busy, archived)| {
                        actual == context && !busy && !archived
                    })
                }) {
                    dispatch = Some(next);
                    changed = true;
                    break;
                }
                if was_enabled != task.enabled {
                    changed = true;
                }
            }
            if let Some(dispatch) = dispatch {
                self.dispatch_workflow(
                    WorkflowOrigin::Schedule(dispatch.workflow_id),
                    dispatch.session_id,
                    dispatch.context,
                    dispatch.prompt,
                    cx,
                );
            }
        }
        if self.pending_confirm.is_none() && self.workflows.pending.is_empty() {
            let drifted = self
                .workflows
                .state
                .goals
                .iter()
                .filter(|goal| {
                    goal.automatic_continuation
                        && goal.turns_used > 0
                        && goal.status == GoalStatus::Ready
                })
                .filter_map(|goal| {
                    let current = self.sessions.get(goal.session_id).map(|session| {
                        if self.sessions.selected_id() == Some(goal.session_id) {
                            self.displayed_context(cx)
                        } else {
                            session.context.clone()
                        }
                    });
                    (current.as_ref() != Some(&goal.context)).then_some(goal.id)
                })
                .collect::<Vec<_>>();
            for id in drifted {
                self.pause_workflow(
                    WorkflowOrigin::Goal(id),
                    WorkflowPauseReason::ContextChanged,
                    cx,
                );
            }
            let continuation = self
                .workflows
                .state
                .goals
                .iter()
                .filter(|goal| {
                    goal.automatic_continuation
                        && goal.turns_used > 0
                        && goal.status == GoalStatus::Ready
                })
                .find_map(|goal| {
                    let session = self.sessions.get(goal.session_id)?;
                    if session.status.is_active() || session.archived {
                        return None;
                    }
                    let current = if self.sessions.selected_id() == Some(goal.session_id) {
                        self.displayed_context(cx)
                    } else {
                        session.context.clone()
                    };
                    if !goal.may_automatically_continue(&current, now) {
                        return None;
                    }
                    Some(goal.id)
                });
            if let Some(id) = continuation {
                self.continue_goal_turn(id, false, cx);
            }
        }
        if changed {
            self.save_agent_workflows(cx);
            cx.notify();
        }
    }
    fn dispatch_workflow(
        &mut self,
        origin: WorkflowOrigin,
        session_id: Uuid,
        context: AgentExecutionContext,
        prompt: String,
        cx: &mut Context<Self>,
    ) {
        if !self.workflows.authorized || self.pending_confirm.is_some() {
            return;
        }
        if self
            .sessions
            .get(session_id)
            .is_none_or(|s| s.status.is_active() || s.archived)
        {
            return;
        }
        let current = if self.sessions.selected_id() == Some(session_id) {
            self.displayed_context(cx)
        } else {
            self.sessions.get(session_id).unwrap().context.clone()
        };
        if current != context {
            self.pause_workflow(origin, WorkflowPauseReason::ContextChanged, cx);
            return;
        }
        self.workflows.pending.insert(
            session_id,
            PendingWorkflow {
                origin,
                context: context.clone(),
                prompt: prompt.clone(),
            },
        );
        let pending = PendingRun {
            source_draft: None,
            session_id,
            context,
            prompt,
            options: Default::default(),
        };
        if pending.context.access == AgentAccessMode::ReadOnly {
            self.start_pending(pending, cx);
            if let Some(failed) = self.workflows.pending.remove(&session_id) {
                self.pause_workflow(failed.origin, WorkflowPauseReason::Error, cx);
            }
        } else {
            self.pending_confirm = Some(pending);
            cx.notify();
        }
    }
    /// Call after runtime admission succeeds, before emitting Run. This does
    /// not bypass Workspace's final live-role/target check.
    pub(super) fn workflow_register_run(
        &mut self,
        request: &AgentRunRequest,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(session_id) = self.run_sessions.get(&request.id).copied() else {
            return false;
        };
        let Some(pending) = self.workflows.pending.remove(&session_id) else {
            return true;
        };
        if pending.context != AgentExecutionContext::from_request(request)
            || pending.prompt != request.prompt
        {
            return true;
        }
        if !self.workflows.authorized {
            return false;
        }
        match pending.origin {
            WorkflowOrigin::Goal(id) => {
                let Some(goal) = self
                    .workflows
                    .state
                    .goals
                    .iter_mut()
                    .find(|goal| goal.id == id)
                else {
                    return false;
                };
                if goal
                    .begin_turn(request.id, &pending.context, now_ms())
                    .is_err()
                {
                    goal.pause(WorkflowPauseReason::ContextChanged, now_ms());
                    return false;
                }
            }
            WorkflowOrigin::Schedule(id) => {
                if let Some(task) = self
                    .workflows
                    .state
                    .schedules
                    .iter_mut()
                    .find(|task| task.id == id)
                {
                    if !task.enabled {
                        return false;
                    }
                    task.active_run = Some(request.id);
                } else {
                    return false;
                }
            }
        }
        self.workflows.runs.insert(
            request.id,
            WorkflowRun {
                origin: pending.origin,
                observation: GoalObservation::default(),
            },
        );
        self.save_agent_workflows(cx);
        self.ensure_workflow_poll(cx);
        true
    }
    pub(super) fn workflow_cancel_pending(&mut self) {
        self.workflows.pending.clear();
    }
    pub(super) fn workflow_observe_event(
        &mut self,
        run_id: Uuid,
        event: &AgentStreamEvent,
        cx: &mut Context<Self>,
    ) {
        let Some(run) = self.workflows.runs.get_mut(&run_id) else {
            return;
        };
        if let Some(reason) = run.observation.observe(event) {
            let origin = run.origin;
            self.pause_workflow(origin, reason, cx);
        }
    }
    pub(super) fn workflow_finish_run(
        &mut self,
        run_id: Uuid,
        status: AgentSessionStatus,
        cx: &mut Context<Self>,
    ) {
        let Some(run) = self.workflows.runs.remove(&run_id) else {
            return;
        };
        match run.origin {
            WorkflowOrigin::Goal(id) => {
                if let Some(goal) = self
                    .workflows
                    .state
                    .goals
                    .iter_mut()
                    .find(|goal| goal.id == id)
                {
                    goal.finish_turn(
                        run_id,
                        status == AgentSessionStatus::Completed,
                        run.observation.explicitly_completed(id),
                        now_ms(),
                    );
                }
            }
            WorkflowOrigin::Schedule(id) => {
                if let Some(task) = self
                    .workflows
                    .state
                    .schedules
                    .iter_mut()
                    .find(|task| task.id == id)
                {
                    task.active_run = None;
                    if status != AgentSessionStatus::Completed && task.pause_reason.is_none() {
                        task.pause(WorkflowPauseReason::Error);
                    }
                }
            }
        }
        self.save_agent_workflows(cx);
        cx.notify();
    }
    fn pause_workflow(
        &mut self,
        origin: WorkflowOrigin,
        reason: WorkflowPauseReason,
        cx: &mut Context<Self>,
    ) {
        match origin {
            WorkflowOrigin::Goal(id) => {
                if let Some(goal) = self
                    .workflows
                    .state
                    .goals
                    .iter_mut()
                    .find(|goal| goal.id == id)
                {
                    goal.pause(reason, now_ms());
                }
            }
            WorkflowOrigin::Schedule(id) => {
                if let Some(task) = self
                    .workflows
                    .state
                    .schedules
                    .iter_mut()
                    .find(|task| task.id == id)
                {
                    task.pause(reason);
                }
            }
        }
        self.save_agent_workflows(cx);
        cx.notify();
    }
    fn start_goal_turn(&mut self, id: Uuid, cx: &mut Context<Self>) {
        self.continue_goal_turn(id, true, cx);
    }
    fn continue_goal_turn(&mut self, id: Uuid, user_initiated: bool, cx: &mut Context<Self>) {
        let Some(goal) = self
            .workflows
            .state
            .goals
            .iter()
            .find(|goal| goal.id == id)
            .cloned()
        else {
            return;
        };
        let current = if self.sessions.selected_id() == Some(goal.session_id) {
            self.displayed_context(cx)
        } else {
            let Some(session) = self.sessions.get(goal.session_id) else {
                return;
            };
            session.context.clone()
        };
        match goal.next_prompt(&current, now_ms()) {
            Ok(prompt) => {
                if user_initiated {
                    self.workflows.dialog = None;
                    self.workflows.form = None;
                }
                self.dispatch_workflow(
                    WorkflowOrigin::Goal(id),
                    goal.session_id,
                    goal.context,
                    prompt,
                    cx,
                );
            }
            Err(error) => {
                self.workflows.feedback.update(cx, |feedback, cx| {
                    feedback.error = Some(error.to_string());
                    cx.notify();
                });
                self.workflows.error = Some(error.to_string());
                cx.notify();
            }
        }
    }
    fn toggle_workflow(&mut self, origin: WorkflowOrigin, cx: &mut Context<Self>) {
        let pending_session = self
            .workflows
            .pending
            .iter()
            .find_map(|(session, pending)| (pending.origin == origin).then_some(*session));
        if let Some(session) = pending_session {
            self.workflows.pending.remove(&session);
            if self
                .pending_confirm
                .as_ref()
                .is_some_and(|pending| pending.session_id == session)
            {
                self.pending_confirm = None;
            }
        }
        match origin {
            WorkflowOrigin::Goal(id) => {
                let run_id = self
                    .workflows
                    .state
                    .goals
                    .iter()
                    .find(|g| g.id == id)
                    .and_then(|g| g.active_run);
                self.pause_workflow(origin, WorkflowPauseReason::User, cx);
                if let Some(run_id) = run_id {
                    cx.emit(AgentConsoleEvent::Stop(run_id));
                }
            }
            WorkflowOrigin::Schedule(id) => {
                if let Some(task) = self
                    .workflows
                    .state
                    .schedules
                    .iter_mut()
                    .find(|task| task.id == id)
                {
                    if task.enabled {
                        let run_id = task.active_run;
                        task.pause(WorkflowPauseReason::User);
                        if let Some(run_id) = run_id {
                            cx.emit(AgentConsoleEvent::Stop(run_id));
                        }
                    } else {
                        task.enable(now_ms());
                    }
                }
                self.save_agent_workflows(cx);
            }
        }
        self.show_workflow_manager(cx);
    }
    fn remove_workflow(&mut self, origin: WorkflowOrigin, cx: &mut Context<Self>) {
        match origin {
            WorkflowOrigin::Goal(id) => self
                .workflows
                .state
                .goals
                .retain(|goal| goal.id != id || goal.active_run.is_some()),
            WorkflowOrigin::Schedule(id) => self
                .workflows
                .state
                .schedules
                .retain(|task| task.id != id || task.active_run.is_some()),
        }
        self.save_agent_workflows(cx);
        self.show_workflow_manager(cx);
    }
    pub(super) fn render_workflow_controls(&self, cx: &mut Context<Self>) -> AnyElement {
        let session = self.sessions.selected_id();
        let goals = self
            .workflows
            .state
            .goals
            .iter()
            .filter(|goal| Some(goal.session_id) == session)
            .count();
        let tasks = self
            .workflows
            .state
            .schedules
            .iter()
            .filter(|task| Some(task.session_id) == session)
            .count();
        let context = self.displayed_context(cx);
        let mut row = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(6.0))
            .px(px(10.0))
            .py(px(4.0))
            .text_size(px(10.0));
        row = row.child(
            Button::new(
                "agent-workflows",
                t!("agents.workflows.manage", goals = goals, tasks = tasks).to_string(),
            )
            .variant(ButtonVariant::Ghost)
            .size(ButtonSize::Sm)
            .icon(IconSource::from("list-checks"))
            .disabled(self.workflows.loading || !self.workflows.writable)
            .on_click(cx.listener(|this, _, _, cx| this.show_workflow_manager(cx))),
        );
        row = row.child(
            Button::new(
                "agent-readiness",
                t!("agents.workflows.check_readiness").to_string(),
            )
            .variant(ButtonVariant::Ghost)
            .size(ButtonSize::Sm)
            .icon(IconSource::from("circle-check"))
            .disabled(self.workflows.readiness_pending.is_some())
            .on_click(cx.listener(|this, _, _, cx| this.check_agent_readiness(cx))),
        );
        if let Some((checked, result)) = &self.workflows.readiness {
            if checked == &context {
                let label = match result {
                    Ok(result) if !result.installed => {
                        t!("agents.workflows.not_installed").to_string()
                    }
                    Ok(result) => {
                        let auth = match result.authentication {
                            AuthenticationReadiness::Authenticated => {
                                t!("agents.workflows.authenticated").to_string()
                            }
                            AuthenticationReadiness::SignInRequired => {
                                t!("agents.workflows.sign_in_required").to_string()
                            }
                            AuthenticationReadiness::Unavailable => {
                                t!("agents.workflows.auth_unavailable").to_string()
                            }
                        };
                        format!(
                            "{} · {} · {}",
                            context.provider.display_name(),
                            result.version.as_deref().unwrap_or("—"),
                            auth
                        )
                    }
                    Err(error) => error.clone(),
                };
                row = row.child(div().text_color(ShellDeckColors::text_muted()).child(label));
            }
        }
        if let Some(goal) =
            self.workflows.state.goals.iter().find(|goal| {
                Some(goal.session_id) == session && goal.status != GoalStatus::Complete
            })
        {
            row = row.child(
                Badge::new(
                    t!(
                        "agents.workflows.goal_budget",
                        used = goal.turns_used,
                        total = goal.turn_limit
                    )
                    .to_string(),
                )
                .variant(BadgeVariant::Outline),
            );
            if goal.status != GoalStatus::Running && goal.active_run.is_none() {
                let id = goal.id;
                row = row.child(
                    Button::new(
                        "agent-goal-continue",
                        t!("agents.workflows.start_continue").to_string(),
                    )
                    .variant(ButtonVariant::Ghost)
                    .size(ButtonSize::Sm)
                    .disabled(!self.workflows.authorized || goal.budget_exhausted(now_ms()))
                    .on_click(cx.listener(move |this, _, _, cx| this.start_goal_turn(id, cx))),
                );
            }
        }
        if let Some(error) = &self.workflows.error {
            row = row.child(
                div()
                    .text_color(ShellDeckColors::text_muted())
                    .line_clamp(1)
                    .child(error.clone()),
            );
        }
        row.into_any_element()
    }
    fn check_agent_readiness(&mut self, cx: &mut Context<Self>) {
        if self.workflows.readiness_pending.is_some() {
            return;
        }
        let context = self.displayed_context(cx);
        let check_id = Uuid::new_v4();
        self.workflows.readiness_pending = Some((check_id, context.clone()));
        if matches!(context.target, AgentTarget::Ssh { .. }) {
            cx.emit(AgentConsoleEvent::CheckReadiness { check_id, context });
        } else {
            cx.spawn(async move |this, cx| {
                let result = cx
                    .background_executor()
                    .spawn(
                        async move { check_provider_readiness(&context, &LocalReadinessExecutor) },
                    )
                    .await;
                let _ = this.update(cx, |this, cx| {
                    this.set_agent_readiness_result(check_id, result, cx)
                });
            })
            .detach();
        }
        cx.notify();
    }
    /// Only sanitized status/version may cross this callback. Never forward
    /// raw auth output from an SSH provider process.
    pub fn set_agent_readiness_result(
        &mut self,
        check_id: Uuid,
        result: Result<ProviderReadiness, String>,
        cx: &mut Context<Self>,
    ) {
        let Some((pending, context)) = self.workflows.readiness_pending.take() else {
            return;
        };
        if pending != check_id {
            self.workflows.readiness_pending = Some((pending, context));
            return;
        }
        self.workflows.readiness = Some((context, result));
        cx.notify();
    }
    fn show_workflow_manager(&mut self, cx: &mut Context<Self>) {
        self.workflows.form = None;
        let Some(session_id) = self.sessions.selected_id() else {
            return;
        };
        let parent = cx.entity();
        let close = parent.clone();
        let mut content = div()
            .flex()
            .flex_col()
            .gap(px(12.0))
            .child(self.workflows.feedback.clone())
            .child(t!("agents.workflows.desktop_only").to_string());
        for goal in self
            .workflows
            .state
            .goals
            .iter()
            .filter(|g| g.session_id == session_id)
        {
            let id = goal.id;
            let start = parent.clone();
            let edit = parent.clone();
            let pause = parent.clone();
            let remove = parent.clone();
            let reason = goal
                .pause_reason
                .map(|r| {
                    let key = format!("agents.workflows.reason_{}", r.key());
                    t!(key.as_str()).to_string()
                })
                .unwrap_or_default();
            let status = match goal.status {
                GoalStatus::Ready => "ready",
                GoalStatus::Running => "running",
                GoalStatus::Paused => "paused",
                GoalStatus::Complete => "complete",
                GoalStatus::BudgetExhausted => "budget_exhausted",
            };
            let mut controls = div().flex().flex_wrap().gap(px(5.0));
            if goal.active_run.is_none() && goal.status != GoalStatus::Complete {
                controls = controls.child(
                    Button::new(
                        SharedString::from(format!("goal-start-{id}")),
                        t!("agents.workflows.start_continue").to_string(),
                    )
                    .size(ButtonSize::Sm)
                    .disabled(!self.workflows.authorized || goal.budget_exhausted(now_ms()))
                    .on_click(move |_, _, cx| {
                        start.update(cx, |this, cx| this.start_goal_turn(id, cx))
                    }),
                );
            }
            controls = controls
                .child(
                    Button::new(
                        SharedString::from(format!("goal-edit-{id}")),
                        t!("agents.workflows.edit").to_string(),
                    )
                    .variant(ButtonVariant::Ghost)
                    .size(ButtonSize::Sm)
                    .disabled(goal.active_run.is_some())
                    .on_click(move |_, _, cx| {
                        edit.update(cx, |this, cx| {
                            this.open_workflow_form(FormKind::Goal(Some(id)), cx)
                        })
                    }),
                )
                .child(
                    Button::new(
                        SharedString::from(format!("goal-pause-{id}")),
                        t!("agents.workflows.pause").to_string(),
                    )
                    .variant(ButtonVariant::Ghost)
                    .size(ButtonSize::Sm)
                    .on_click(move |_, _, cx| {
                        pause.update(cx, |this, cx| {
                            this.toggle_workflow(WorkflowOrigin::Goal(id), cx)
                        })
                    }),
                )
                .child(
                    Button::new(
                        SharedString::from(format!("goal-remove-{id}")),
                        t!("agents.workflows.remove").to_string(),
                    )
                    .variant(ButtonVariant::Ghost)
                    .size(ButtonSize::Sm)
                    .disabled(goal.active_run.is_some())
                    .on_click(move |_, _, cx| {
                        remove.update(cx, |this, cx| {
                            this.remove_workflow(WorkflowOrigin::Goal(id), cx)
                        })
                    }),
                );
            content = content.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(5.0))
                    .child(
                        div()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(goal.objective.clone()),
                    )
                    .child(
                        Badge::new({
                            let key = format!("agents.workflows.status_{status}");
                            t!(key.as_str()).to_string()
                        })
                        .variant(BadgeVariant::Outline),
                    )
                    .child(goal.verification.clone())
                    .child(reason)
                    .child(workflow_context_label(&goal.context))
                    .child(
                        t!(
                            "agents.workflows.progress_budget",
                            used = goal.turns_used,
                            total = goal.turn_limit,
                            minutes = goal.elapsed_at(now_ms()) / 60_000,
                            limit = goal.time_limit_ms / 60_000
                        )
                        .to_string(),
                    )
                    .child(goal.progress.clone())
                    .child(controls),
            );
        }
        for task in self
            .workflows
            .state
            .schedules
            .iter()
            .filter(|task| task.session_id == session_id)
        {
            let id = task.id;
            let edit = parent.clone();
            let toggle = parent.clone();
            let remove = parent.clone();
            let toggle_label = if task.enabled {
                t!("agents.workflows.pause").to_string()
            } else {
                t!("agents.workflows.enable").to_string()
            };
            let due = chrono::DateTime::from_timestamp_millis(task.next_due_ms)
                .map(|at| {
                    at.with_timezone(&chrono::Local)
                        .format("%Y-%m-%d %H:%M")
                        .to_string()
                })
                .unwrap_or_else(|| "—".into());
            let reason = task
                .pause_reason
                .map(|r| {
                    let key = format!("agents.workflows.reason_{}", r.key());
                    t!(key.as_str()).to_string()
                })
                .unwrap_or_default();
            content = content.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(5.0))
                    .child(
                        div()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(task.title.clone()),
                    )
                    .child(task.prompt.clone())
                    .child(workflow_context_label(&task.context))
                    .child(
                        t!(
                            "agents.workflows.schedule_due",
                            minutes = task.interval_ms / 60_000,
                            due = due
                        )
                        .to_string(),
                    )
                    .child(reason)
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .gap(px(5.0))
                            .child(
                                Button::new(
                                    SharedString::from(format!("task-edit-{id}")),
                                    t!("agents.workflows.edit").to_string(),
                                )
                                .variant(ButtonVariant::Ghost)
                                .size(ButtonSize::Sm)
                                .disabled(task.active_run.is_some())
                                .on_click(move |_, _, cx| {
                                    edit.update(cx, |this, cx| {
                                        this.open_workflow_form(FormKind::Schedule(Some(id)), cx)
                                    })
                                }),
                            )
                            .child(
                                Button::new(
                                    SharedString::from(format!("task-toggle-{id}")),
                                    toggle_label,
                                )
                                .variant(ButtonVariant::Outline)
                                .size(ButtonSize::Sm)
                                .on_click(move |_, _, cx| {
                                    toggle.update(cx, |this, cx| {
                                        this.toggle_workflow(WorkflowOrigin::Schedule(id), cx)
                                    })
                                }),
                            )
                            .child(
                                Button::new(
                                    SharedString::from(format!("task-remove-{id}")),
                                    t!("agents.workflows.remove").to_string(),
                                )
                                .variant(ButtonVariant::Ghost)
                                .size(ButtonSize::Sm)
                                .disabled(task.active_run.is_some())
                                .on_click(move |_, _, cx| {
                                    remove.update(cx, |this, cx| {
                                        this.remove_workflow(WorkflowOrigin::Schedule(id), cx)
                                    })
                                }),
                            ),
                    ),
            );
        }
        let new_goal = parent.clone();
        let new_schedule = parent;
        let at_limit = self.workflows.state.goals.len() + self.workflows.state.schedules.len()
            >= MAX_WORKFLOWS;
        self.workflows.dialog = Some(cx.new(move |dialog_cx| {
            WorkflowDialog::new(dialog_cx)
                .title(t!("agents.workflows.title").to_string())
                .size(DialogSize::Lg)
                .on_close(move |_, cx| {
                    close.update(cx, |this, cx| {
                        this.workflows.dialog = None;
                        this.workflows.form = None;
                        cx.notify();
                    })
                })
                .child(content)
                .footer(
                    div()
                        .flex()
                        .justify_end()
                        .gap(px(8.0))
                        .child(
                            Button::new(
                                "agent-new-goal",
                                t!("agents.workflows.new_goal").to_string(),
                            )
                            .size(ButtonSize::Sm)
                            .disabled(at_limit)
                            .on_click(move |_, _, cx| {
                                new_goal.update(cx, |this, cx| {
                                    this.open_workflow_form(FormKind::Goal(None), cx)
                                })
                            }),
                        )
                        .child(
                            Button::new(
                                "agent-new-task",
                                t!("agents.workflows.new_schedule").to_string(),
                            )
                            .size(ButtonSize::Sm)
                            .variant(ButtonVariant::Outline)
                            .disabled(at_limit)
                            .on_click(move |_, _, cx| {
                                new_schedule.update(cx, |this, cx| {
                                    this.open_workflow_form(FormKind::Schedule(None), cx)
                                })
                            }),
                        ),
                )
        }));
        cx.notify();
    }
    fn open_workflow_form(&mut self, kind: FormKind, cx: &mut Context<Self>) {
        let Some(session_id) = self.sessions.selected_id() else {
            return;
        };
        let mut context = self.displayed_context(cx);
        let mut automatic = false;
        let (mut objective, mut verification, mut progress, mut title) =
            (String::new(), String::new(), String::new(), String::new());
        let (mut turns, mut minutes, mut interval) = ("8".into(), "30".into(), "60".into());
        match kind {
            FormKind::Goal(Some(id)) => {
                if let Some(goal) = self.workflows.state.goals.iter().find(|g| g.id == id) {
                    context = goal.context.clone();
                    objective = goal.objective.clone();
                    verification = goal.verification.clone();
                    progress = goal.progress.clone();
                    automatic = goal.automatic_continuation;
                    turns = goal.turn_limit.to_string();
                    minutes = (goal.time_limit_ms / 60_000).to_string();
                }
            }
            FormKind::Schedule(Some(id)) => {
                if let Some(task) = self.workflows.state.schedules.iter().find(|t| t.id == id) {
                    context = task.context.clone();
                    title = task.title.clone();
                    objective = task.prompt.clone();
                    interval = (task.interval_ms / 60_000).to_string();
                }
            }
            _ => {}
        }
        let form = WorkflowForm {
            kind,
            session_id,
            context: context.clone(),
            objective: input_state(cx, objective),
            verification: input_state(cx, verification),
            progress: input_state(cx, progress),
            title: input_state(cx, title),
            turns: input_state(cx, turns),
            minutes: input_state(cx, minutes),
            interval: input_state(cx, interval),
            automatic: Arc::new(AtomicBool::new(automatic)),
        };
        let mut body = div()
            .flex()
            .flex_col()
            .gap(px(9.0))
            .child(self.workflows.feedback.clone())
            .child(workflow_context_label(&context));
        if matches!(kind, FormKind::Schedule(_)) {
            body = body
                .child(workflow_field(
                    "agents.workflows.task_title",
                    &form.title,
                    false,
                ))
                .child(t!("agents.workflows.desktop_only").to_string());
        }
        body = body.child(workflow_field(
            "agents.workflows.objective",
            &form.objective,
            true,
        ));
        match kind {
            FormKind::Goal(_) => {
                body = body
                    .child(workflow_field(
                        "agents.workflows.verification",
                        &form.verification,
                        true,
                    ))
                    .child(workflow_field(
                        "agents.workflows.progress",
                        &form.progress,
                        true,
                    ))
                    .child(workflow_field(
                        "agents.workflows.turn_limit",
                        &form.turns,
                        false,
                    ))
                    .child(workflow_field(
                        "agents.workflows.time_limit",
                        &form.minutes,
                        false,
                    ))
                    .child(t!("agents.workflows.explicit_continuation").to_string());
            }
            FormKind::Schedule(_) => {
                body = body
                    .child(workflow_field(
                        "agents.workflows.interval",
                        &form.interval,
                        false,
                    ))
                    .child(t!("agents.workflows.enable_separately").to_string());
            }
        }
        if matches!(kind, FormKind::Goal(_)) {
            let toggle = cx.new(|_| WorkflowAutomaticToggle {
                checked: form.automatic.clone(),
                allowed: context.access == AgentAccessMode::ReadOnly,
            });
            body = body
                .child(toggle)
                .child(t!("agents.workflows.automatic_hint").to_string());
        }
        self.workflows.form = Some(form);
        let parent = cx.entity();
        let close = parent.clone();
        let save = parent.clone();
        let cancel = parent;
        self.workflows.dialog = Some(cx.new(move |dialog_cx| {
            WorkflowDialog::new(dialog_cx)
                .title(t!("agents.workflows.configure").to_string())
                .size(DialogSize::Lg)
                .on_close(move |_, cx| {
                    close.update(cx, |this, cx| {
                        this.workflows.dialog = None;
                        this.workflows.form = None;
                        cx.notify();
                    })
                })
                .child(body)
                .footer(
                    div()
                        .flex()
                        .justify_end()
                        .gap(px(8.0))
                        .child(
                            Button::new("agent-workflow-cancel", t!("scripts.cancel").to_string())
                                .variant(ButtonVariant::Ghost)
                                .size(ButtonSize::Sm)
                                .on_click(move |_, _, cx| {
                                    cancel.update(cx, |this, cx| this.show_workflow_manager(cx))
                                }),
                        )
                        .child(
                            Button::new(
                                "agent-workflow-save",
                                t!("agents.workflows.save").to_string(),
                            )
                            .size(ButtonSize::Sm)
                            .on_click(move |_, _, cx| {
                                save.update(cx, |this, cx| this.save_workflow_form(cx))
                            }),
                        ),
                )
        }));
        cx.notify();
    }
    fn save_workflow_form(&mut self, cx: &mut Context<Self>) {
        let Some(form) = self.workflows.form.clone() else {
            return;
        };
        let read = |input: &Entity<InputState>| input.read(cx).content().trim().to_string();
        let result = (|| -> Result<(), String> {
            if !self.workflows.writable {
                return Err(t!("agents.workflows.storage_unavailable").to_string());
            }
            let current = self.displayed_context(cx);
            if current != form.context {
                return Err(t!("agents.workflows.context_changed").to_string());
            }
            match form.kind {
                FormKind::Goal(edit_id) => {
                    let turns = read(&form.turns)
                        .parse::<u32>()
                        .map_err(|_| t!("agents.workflows.invalid_budget").to_string())?;
                    let minutes = read(&form.minutes)
                        .parse::<i64>()
                        .map_err(|_| t!("agents.workflows.invalid_budget").to_string())?;
                    let mut goal = AgentGoal::new(
                        form.session_id,
                        form.context.clone(),
                        read(&form.objective),
                        read(&form.verification),
                        turns,
                        minutes.saturating_mul(60_000),
                    )
                    .map_err(|e| e.to_string())?;
                    goal.progress = read(&form.progress);
                    goal.automatic_continuation = form.automatic.load(Ordering::SeqCst);
                    goal.validate().map_err(|e| e.to_string())?;
                    if let Some(id) = edit_id {
                        let Some(existing) =
                            self.workflows.state.goals.iter_mut().find(|g| g.id == id)
                        else {
                            return Err(t!("agents.workflows.missing").to_string());
                        };
                        if existing.active_run.is_some() {
                            return Err(t!("agents.workflows.running").to_string());
                        }
                        goal.id = id;
                        goal.turns_used = existing.turns_used;
                        goal.elapsed_ms = existing.elapsed_ms;
                        goal.status = GoalStatus::Paused;
                        goal.pause_reason = Some(WorkflowPauseReason::User);
                        *existing = goal;
                    } else {
                        self.workflows.state.goals.push(goal);
                    }
                }
                FormKind::Schedule(edit_id) => {
                    let interval = read(&form.interval)
                        .parse::<i64>()
                        .map_err(|_| t!("agents.workflows.invalid_interval").to_string())?;
                    let mut task = AgentSchedule::new(
                        form.session_id,
                        form.context.clone(),
                        read(&form.title),
                        read(&form.objective),
                        interval.saturating_mul(60_000),
                        now_ms(),
                    )
                    .map_err(|e| e.to_string())?;
                    if let Some(id) = edit_id {
                        let Some(existing) = self
                            .workflows
                            .state
                            .schedules
                            .iter_mut()
                            .find(|t| t.id == id)
                        else {
                            return Err(t!("agents.workflows.missing").to_string());
                        };
                        if existing.active_run.is_some() {
                            return Err(t!("agents.workflows.running").to_string());
                        }
                        task.id = id;
                        task.last_run_ms = existing.last_run_ms;
                        *existing = task;
                    } else {
                        self.workflows.state.schedules.push(task);
                    }
                }
            }
            Ok(())
        })();
        match result {
            Ok(()) => {
                self.workflows.error = None;
                self.save_agent_workflows(cx);
                self.show_workflow_manager(cx);
            }
            Err(error) => {
                self.workflows.feedback.update(cx, |feedback, cx| {
                    feedback.error = Some(error.clone());
                    cx.notify();
                });
                self.workflows.error = Some(error);
                cx.notify();
            }
        }
    }
    pub(super) fn render_workflow_dialog(&self) -> Option<AnyElement> {
        self.workflows
            .dialog
            .as_ref()
            .map(|dialog| dialog.clone().into_any_element())
    }
}
fn workflow_field(key: &str, input: &Entity<InputState>, multiline: bool) -> AnyElement {
    let mut field = Input::new(input).size(InputSize::Sm);
    if multiline {
        field = field.multi_line(true).min_rows(2).max_rows(5);
    }
    div()
        .flex()
        .flex_col()
        .gap(px(4.0))
        .child(div().text_size(px(11.0)).child(t!(key).to_string()))
        .child(field)
        .into_any_element()
}
fn workflow_context_label(context: &AgentExecutionContext) -> String {
    let access = match context.access {
        AgentAccessMode::ReadOnly => t!("agents.access.read_only").to_string(),
        AgentAccessMode::WorkspaceWrite => t!("agents.access.workspace_write").to_string(),
        AgentAccessMode::FullAccess => t!("agents.access.full").to_string(),
    };
    format!(
        "{} · {} · {} · {}",
        context.provider.display_name(),
        context.target.label(),
        context.workdir,
        access
    )
}

struct WorkflowAutomaticToggle {
    checked: Arc<AtomicBool>,
    allowed: bool,
}
impl Render for WorkflowAutomaticToggle {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        Checkbox::new("agent-goal-automatic")
            .checked(self.checked.load(Ordering::SeqCst))
            .disabled(!self.allowed)
            .label(t!("agents.workflows.automatic_continuation").to_string())
            .on_click(cx.listener(|this, checked, _, cx| {
                if this.allowed {
                    this.checked.store(*checked, Ordering::SeqCst);
                    cx.notify();
                }
            }))
    }
}

struct WorkflowFeedback {
    error: Option<String>,
}
impl Render for WorkflowFeedback {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        match &self.error {
            Some(error) => Alert::error().description(error.clone()).into_any_element(),
            None => div().into_any_element(),
        }
    }
}
