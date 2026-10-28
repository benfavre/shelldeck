//! Durable task supervision. All execution and verification stay on Automonique.
use super::*;
use adabraka_ui::overlays::dialog::{Dialog, DialogSize};
use shelldeck_core::config::platform::IdempotencyKey;
use shelldeck_core::config::platform_tasks::*;
use uuid::Uuid;

#[derive(Default)]
pub(super) struct TaskUi {
    pub open: bool,
    target: Option<PlatformReviewTarget>,
    tasks: Vec<TaskSnapshot>,
    selected: Option<OrchestrationTaskId>,
    available: bool,
    error: Option<String>,
    outcome: Option<String>,
    unknown: bool,
    unknown_observed: bool,
    form: Option<TaskForm>,
    confirmation: Option<Entity<Dialog>>,
    pending: Option<PlatformTaskCommand>,
}

#[derive(Clone)]
struct TaskForm {
    objective: Entity<InputState>,
    criterion: Entity<InputState>,
    path: Entity<InputState>,
    expected: Entity<InputState>,
    attempts: Entity<InputState>,
    minutes: Entity<InputState>,
    concurrency: Entity<InputState>,
    total_minutes: Entity<InputState>,
    check_kind: usize,
    criteria: Vec<TaskCriterion>,
    tasks: Vec<TaskSpec>,
    dependencies: BTreeSet<OrchestrationTaskId>,
}

fn input(cx: &mut Context<FleetView>, value: &str) -> Entity<InputState> {
    cx.new(|cx| {
        let mut state = InputState::new(cx);
        state.content = value.to_owned().into();
        state
    })
}

fn task_status_label(task: &TaskSnapshot) -> String {
    match task.status {
        TaskStatus::Queued => t!("fleet.tasks.queued"),
        TaskStatus::Starting => t!("fleet.tasks.starting"),
        TaskStatus::Running => t!("fleet.tasks.running"),
        TaskStatus::AwaitingVerification => t!("fleet.tasks.awaiting_verification"),
        TaskStatus::Verified if task_has_current_verification(task) => t!("fleet.tasks.verified"),
        TaskStatus::Verified => t!("fleet.tasks.unverified"),
        TaskStatus::Failed => t!("fleet.tasks.failed"),
        TaskStatus::PauseRequested => t!("fleet.tasks.pause_requested"),
        TaskStatus::Paused => t!("fleet.tasks.paused"),
        TaskStatus::CancelRequested => t!("fleet.tasks.cancel_requested"),
        TaskStatus::Cancelled => t!("fleet.tasks.cancelled"),
    }
    .to_string()
}

fn control_label(action: TaskControlAction) -> String {
    match action {
        TaskControlAction::Pause => t!("fleet.tasks.pause"),
        TaskControlAction::Resume => t!("fleet.tasks.resume"),
        TaskControlAction::Cancel => t!("fleet.tasks.cancel"),
    }
    .to_string()
}

fn check_label(kind: usize) -> String {
    match kind {
        0 => t!("fleet.tasks.check_registered"),
        1 => t!("fleet.tasks.check_exists"),
        2 => t!("fleet.tasks.check_contains"),
        _ => t!("fleet.tasks.check_sha256"),
    }
    .to_string()
}

fn check_description(check: &TaskCheck) -> String {
    match check {
        TaskCheck::RegisteredCheck { id } => format!("{}: {id}", check_label(0)),
        TaskCheck::ArtifactExists { path } => format!("{}: {path}", check_label(1)),
        TaskCheck::ArtifactContains { path, text } => {
            format!("{}: {path} → {text}", check_label(2))
        }
        TaskCheck::ArtifactSha256 { path, digest } => {
            format!("{}: {path} → {digest}", check_label(3))
        }
    }
}

fn task_controls(task: &TaskSnapshot) -> &'static [TaskControlAction] {
    match task.status {
        TaskStatus::Queued
        | TaskStatus::Starting
        | TaskStatus::Running
        | TaskStatus::AwaitingVerification => {
            &[TaskControlAction::Pause, TaskControlAction::Cancel]
        }
        TaskStatus::Paused | TaskStatus::PauseRequested => {
            &[TaskControlAction::Resume, TaskControlAction::Cancel]
        }
        _ => &[],
    }
}

fn task_load_is_current(previous: &[TaskSnapshot], next: &[TaskSnapshot]) -> bool {
    next.iter().all(|task| {
        previous
            .iter()
            .find(|old| old.spec.id == task.spec.id)
            .is_none_or(|old| task.revision.get() > old.revision.get() || task == old)
    })
}

impl TaskUi {
    fn apply_load(&mut self, target: Option<PlatformReviewTarget>, load: Option<PlatformTaskLoad>) {
        if self.target != target {
            let open = self.open;
            *self = TaskUi {
                open,
                target: target.clone(),
                ..TaskUi::default()
            };
        }
        match load {
            Some(PlatformTaskLoad::Available(tasks, unresolved)) => {
                if target
                    .as_ref()
                    .is_none_or(|target| validate_task_scope(target, &tasks).is_err())
                {
                    self.available = false;
                    self.error = Some(t!("fleet.tasks.context_changed").to_string());
                    return;
                }
                if self.unknown && unresolved.is_empty() {
                    self.outcome = None;
                }
                self.unknown = !unresolved.is_empty();
                self.unknown_observed = self.unknown;
                if self.unknown {
                    self.outcome = Some(format!(
                        "{}: {}",
                        t!("fleet.tasks.unknown"),
                        unresolved.join("; ")
                    ));
                }
                if !task_load_is_current(&self.tasks, &tasks) {
                    self.available = false;
                    self.error = Some(t!("fleet.tasks.stale").to_string());
                    return;
                }
                self.tasks = tasks;
                self.available = true;
                self.error = None;
                if !self
                    .tasks
                    .iter()
                    .any(|task| Some(&task.spec.id) == self.selected.as_ref())
                {
                    self.selected = self.tasks.first().map(|task| task.spec.id.clone());
                }
            }
            Some(PlatformTaskLoad::Unavailable(error)) => {
                self.available = false;
                self.error = Some(error);
            }
            None => {
                self.available = false;
            }
        }
    }

    fn command_is_current(&self, command: &PlatformTaskCommand) -> bool {
        self.available
            && !self.unknown
            && self.target.as_ref() == Some(&command.target())
            && match command {
                PlatformTaskCommand::Create(_) => true,
                PlatformTaskCommand::Control(request) => self.tasks.iter().any(|task| {
                    task.spec.id == request.task
                        && task.revision == request.expected_revision
                        && task_controls(task).contains(&request.action)
                }),
            }
    }

    fn command_result(&mut self, command: &PlatformTaskCommand, result: PlatformTaskCommandResult) {
        if self.target.as_ref() != Some(&command.target()) {
            return;
        }
        match result {
            PlatformTaskCommandResult::Applied(tasks) => {
                for task in tasks {
                    if let Some(old) = self
                        .tasks
                        .iter_mut()
                        .find(|old| old.spec.id == task.spec.id)
                    {
                        if task.revision.get() >= old.revision.get() {
                            *old = task;
                        }
                    } else {
                        self.tasks.push(task);
                    }
                }
                self.unknown = false;
                self.outcome = Some(t!("fleet.tasks.accepted").to_string());
                self.form = None;
            }
            PlatformTaskCommandResult::Refused(reason) => {
                self.unknown = false;
                self.outcome = Some(format!("{}: {reason}", t!("fleet.tasks.refused")));
            }
            PlatformTaskCommandResult::Unknown(reason) => {
                self.unknown = true;
                self.unknown_observed = false;
                self.outcome = Some(format!("{}: {reason}", t!("fleet.tasks.unknown")));
            }
        }
    }
}

impl FleetView {
    pub(super) fn invalidate_tasks(&mut self) {
        self.tasks.available = false;
    }

    pub fn set_tasks(
        &mut self,
        target: Option<PlatformReviewTarget>,
        load: Option<PlatformTaskLoad>,
    ) {
        self.tasks.apply_load(target, load);
    }

    pub fn can_acknowledge_task_dispatches(&self) -> bool {
        self.tasks.unknown
            && self.tasks.unknown_observed
            && self.tasks.available
            && !self.operation_busy
            && !self.loading
    }

    pub fn task_command_is_current(&self, command: &PlatformTaskCommand) -> bool {
        self.tasks.command_is_current(command)
    }

    pub fn set_task_command_result(
        &mut self,
        command: &PlatformTaskCommand,
        result: PlatformTaskCommandResult,
    ) {
        self.operation_busy = false;
        self.tasks.command_result(command, result);
    }

    pub(super) fn render_task_toolbar(&self, cx: &mut Context<Self>) -> AnyElement {
        div()
            .flex()
            .items_center()
            .gap(px(8.0))
            .px(px(12.0))
            .py(px(6.0))
            .border_b_1()
            .border_color(ShellDeckColors::border())
            .child(
                Button::new("platform-tasks-toggle", t!("fleet.tasks.title").to_string())
                    .variant(if self.tasks.open {
                        ButtonVariant::Default
                    } else {
                        ButtonVariant::Ghost
                    })
                    .size(ButtonSize::Sm)
                    .icon(IconSource::from("list-checks"))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.tasks.open = !this.tasks.open;
                        cx.notify();
                    })),
            )
            .child(Badge::new(self.tasks.tasks.len().to_string()).variant(BadgeVariant::Secondary))
            .child(
                div()
                    .text_size(px(11.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(t!("fleet.tasks.server_owned").to_string()),
            )
            .into_any_element()
    }

    pub(super) fn render_tasks(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut body = div()
            .id("platform-tasks-body")
            .flex_1()
            .min_h(px(0.0))
            .min_w(px(0.0))
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap(px(12.0))
            .p(px(12.0));
        if self.tasks.target.is_none() {
            return body
                .child(Alert::info().description(t!("fleet.tasks.select_workspace").to_string()))
                .into_any_element();
        }
        if let Some(error) = &self.tasks.error {
            body = body.child(
                Alert::error()
                    .title(t!("fleet.tasks.unavailable").to_string())
                    .description(error.clone()),
            );
        }
        if let Some(outcome) = &self.tasks.outcome {
            body = body.child(Alert::info().description(outcome.clone()));
        }
        if self.tasks.unknown {
            body = body
                .child(Alert::warning().description(t!("fleet.tasks.unknown_detail").to_string()))
                .child(
                    Button::new(
                        "tasks-inspected-unknown",
                        t!("fleet.tasks.acknowledge").to_string(),
                    )
                    .variant(ButtonVariant::Outline)
                    .size(ButtonSize::Sm)
                    .disabled(!self.can_acknowledge_task_dispatches())
                    .on_click(cx.listener(|this, _, _, cx| {
                        if let Some(target) = &this.tasks.target {
                            cx.emit(FleetViewEvent::AcknowledgeTasks(target.clone()));
                        }
                    })),
                );
        }
        body = body.child(
            div()
                .flex()
                .gap(px(8.0))
                .child(
                    Button::new("tasks-new", t!("fleet.tasks.new_plan").to_string())
                        .size(ButtonSize::Sm)
                        .disabled(
                            !self.tasks.available || self.operation_busy || self.tasks.unknown,
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.new_task_form(cx);
                        })),
                )
                .child(
                    Button::new("tasks-refresh", t!("fleet.refresh").to_string())
                        .variant(ButtonVariant::Outline)
                        .size(ButtonSize::Sm)
                        .disabled(self.loading || self.operation_busy)
                        .on_click(cx.listener(|_, _, _, cx| cx.emit(FleetViewEvent::Refresh))),
                ),
        );
        if let Some(form) = &self.tasks.form {
            body = body.child(self.render_task_form(form, cx));
        }
        if self.tasks.tasks.is_empty() && self.tasks.available {
            body = body.child(
                div()
                    .text_size(px(12.0))
                    .child(t!("fleet.tasks.empty").to_string()),
            );
        }
        for task in &self.tasks.tasks {
            let id = task.spec.id.clone();
            body = body.child(
                Button::new(
                    SharedString::from(format!("task-row-{}", id.as_str())),
                    format!("{} · {}", task_status_label(task), task.spec.objective),
                )
                .variant(if self.tasks.selected.as_ref() == Some(&id) {
                    ButtonVariant::Secondary
                } else {
                    ButtonVariant::Ghost
                })
                .size(ButtonSize::Sm)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.tasks.selected = Some(id.clone());
                    cx.notify();
                })),
            );
        }
        if let Some(task) = self
            .tasks
            .tasks
            .iter()
            .find(|task| Some(&task.spec.id) == self.tasks.selected.as_ref())
        {
            body = body.child(self.render_task_detail(task, cx));
        }
        body.into_any_element()
    }

    fn render_task_detail(&self, task: &TaskSnapshot, cx: &mut Context<Self>) -> AnyElement {
        let mut detail = div()
            .flex()
            .flex_col()
            .gap(px(8.0))
            .p(px(12.0))
            .border_1()
            .border_color(ShellDeckColors::border())
            .rounded(px(6.0))
            .child(
                div()
                    .text_size(px(14.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(task.spec.objective.clone()),
            )
            .child(Badge::new(task_status_label(task)).variant(
                if task_has_current_verification(task) {
                    BadgeVariant::Secondary
                } else {
                    BadgeVariant::Outline
                },
            ))
            .child(
                div()
                    .text_size(px(11.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(
                        t!(
                            "fleet.tasks.limits",
                            attempts = task.attempts_used,
                            maximum = task.spec.max_attempts,
                            minutes = task.spec.attempt_timeout_ms / 60_000,
                            revision = task.revision.get()
                        )
                        .to_string(),
                    ),
            );
        if let Some(reason) = &task.reason {
            detail = detail.child(Alert::info().description(reason.clone()));
        }
        for dependency in &task.spec.dependencies {
            let description = self
                .tasks
                .tasks
                .iter()
                .find(|candidate| &candidate.spec.id == dependency)
                .map(|candidate| {
                    format!(
                        "{} · {}",
                        task_status_label(candidate),
                        candidate.spec.objective
                    )
                })
                .unwrap_or_else(|| t!("fleet.tasks.dependency_unavailable").to_string());
            detail = detail.child(
                div()
                    .text_size(px(11.0))
                    .child(format!("{}: {description}", t!("fleet.tasks.depends_on"))),
            );
        }
        for criterion in &task.spec.criteria {
            let evidence = task.evidence.iter().find(|evidence| {
                evidence.criterion_id == criterion.id
                    && Some(&evidence.attempt_id) == task.attempt_id.as_ref()
                    && Some(&evidence.artifact_revision) == task.artifact_revision.as_ref()
            });
            let status = match evidence {
                Some(e) if e.passed => t!("fleet.tasks.check_passed"),
                Some(_) => t!("fleet.tasks.check_failed"),
                None => t!("fleet.tasks.check_pending"),
            };
            let mut row = div()
                .flex()
                .flex_col()
                .gap(px(4.0))
                .text_size(px(12.0))
                .child(format!("{status} · {}", criterion.description))
                .child(
                    div()
                        .text_size(px(11.0))
                        .text_color(ShellDeckColors::text_muted())
                        .child(check_description(&criterion.check)),
                );
            if let Some(evidence) = evidence {
                row = row.child(evidence.summary.clone()).child(
                    div()
                        .text_size(px(10.0))
                        .text_color(ShellDeckColors::text_muted())
                        .child(format!(
                            "{} · {} · {}",
                            evidence.source,
                            evidence.artifact_revision,
                            crate::i18n::rel_time(evidence.observed_at_ms as f64)
                        )),
                );
            }
            detail = detail.child(row);
        }
        let mut controls = div().flex().gap(px(8.0));
        for action in task_controls(task) {
            let action = *action;
            let id = task.spec.id.clone();
            controls = controls.child(
                Button::new(
                    SharedString::from(format!("task-control-{}", action.as_str())),
                    control_label(action),
                )
                .variant(ButtonVariant::Outline)
                .size(ButtonSize::Sm)
                .disabled(!self.tasks.available || self.operation_busy || self.tasks.unknown)
                .on_click(
                    cx.listener(move |this, _, _, cx| this.prepare_task_control(&id, action, cx)),
                ),
            );
        }
        detail.child(controls).into_any_element()
    }

    fn new_task_form(&mut self, cx: &mut Context<Self>) {
        if !self.tasks.available || self.operation_busy || self.tasks.unknown {
            return;
        }
        self.tasks.form = Some(TaskForm {
            objective: input(cx, ""),
            criterion: input(cx, ""),
            path: input(cx, ""),
            expected: input(cx, ""),
            attempts: input(cx, "2"),
            minutes: input(cx, "10"),
            concurrency: input(cx, "2"),
            total_minutes: input(cx, "30"),
            check_kind: 1,
            criteria: Vec::new(),
            tasks: Vec::new(),
            dependencies: BTreeSet::new(),
        });
        self.tasks.outcome = None;
        cx.notify();
    }

    fn render_task_form(&self, form: &TaskForm, cx: &mut Context<Self>) -> AnyElement {
        let field = |label: String, state: &Entity<InputState>| {
            div()
                .flex()
                .flex_col()
                .gap(px(4.0))
                .child(div().text_size(px(11.0)).child(label))
                .child(Input::new(state).size(InputSize::Sm))
        };
        let mut body = div()
            .flex()
            .flex_col()
            .gap(px(9.0))
            .p(px(12.0))
            .border_1()
            .border_color(ShellDeckColors::border())
            .rounded(px(6.0))
            .child(Alert::info().description(t!("fleet.tasks.plan_help").to_string()))
            .child(field(
                t!("fleet.tasks.objective").to_string(),
                &form.objective,
            ))
            .child(field(
                t!("fleet.tasks.criterion").to_string(),
                &form.criterion,
            ));
        let mut checks = div().flex().flex_wrap().gap(px(6.0));
        for kind in 1..4 {
            checks = checks.child(
                Button::new(
                    SharedString::from(format!("task-check-{kind}")),
                    check_label(kind),
                )
                .size(ButtonSize::Sm)
                .variant(if form.check_kind == kind {
                    ButtonVariant::Default
                } else {
                    ButtonVariant::Outline
                })
                .on_click(cx.listener(move |this, _, _, cx| {
                    if let Some(form) = &mut this.tasks.form {
                        form.check_kind = kind;
                    }
                    cx.notify();
                })),
            );
        }
        body = body.child(checks).child(field(
            if form.check_kind == 0 {
                t!("fleet.tasks.check_id")
            } else {
                t!("fleet.tasks.artifact_path")
            }
            .to_string(),
            &form.path,
        ));
        if form.check_kind >= 2 {
            body = body.child(field(
                t!("fleet.tasks.expected").to_string(),
                &form.expected,
            ));
        }
        for criterion in &form.criteria {
            body = body.child(div().text_size(px(11.0)).child(format!(
                "{} · {}",
                criterion.description,
                check_description(&criterion.check)
            )));
        }
        body = body.child(
            Button::new("task-add-check", t!("fleet.tasks.add_check").to_string())
                .variant(ButtonVariant::Outline)
                .size(ButtonSize::Sm)
                .on_click(cx.listener(|this, _, _, cx| {
                    if let Err(error) = this.add_task_criterion(cx) {
                        this.tasks.outcome = Some(error);
                    }
                    cx.notify();
                })),
        );
        body = body
            .child(field(
                t!("fleet.tasks.max_attempts").to_string(),
                &form.attempts,
            ))
            .child(field(
                t!("fleet.tasks.attempt_minutes").to_string(),
                &form.minutes,
            ));
        if !form.tasks.is_empty() {
            body = body.child(
                div()
                    .text_size(px(11.0))
                    .child(t!("fleet.tasks.choose_dependencies").to_string()),
            );
        }
        for task in &form.tasks {
            let id = task.id.clone();
            body = body.child(
                Button::new(
                    SharedString::from(format!("task-dependency-{}", id.as_str())),
                    task.objective.clone(),
                )
                .variant(if form.dependencies.contains(&id) {
                    ButtonVariant::Default
                } else {
                    ButtonVariant::Outline
                })
                .size(ButtonSize::Sm)
                .on_click(cx.listener(move |this, _, _, cx| {
                    if let Some(form) = &mut this.tasks.form {
                        if !form.dependencies.remove(&id) {
                            form.dependencies.insert(id.clone());
                        }
                    }
                    cx.notify();
                })),
            );
        }
        body = body
            .child(
                Button::new("task-add-step", t!("fleet.tasks.add_task").to_string())
                    .variant(ButtonVariant::Outline)
                    .size(ButtonSize::Sm)
                    .on_click(cx.listener(|this, _, _, cx| {
                        if let Err(error) = this.add_plan_task(cx) {
                            this.tasks.outcome = Some(error);
                        }
                        cx.notify();
                    })),
            )
            .child(
                div()
                    .text_size(px(11.0))
                    .child(t!("fleet.tasks.plan_count", count = form.tasks.len()).to_string()),
            )
            .child(field(
                t!("fleet.tasks.concurrency").to_string(),
                &form.concurrency,
            ))
            .child(field(
                t!("fleet.tasks.total_minutes").to_string(),
                &form.total_minutes,
            ))
            .child(
                div()
                    .flex()
                    .gap(px(8.0))
                    .child(
                        Button::new("task-preview-plan", t!("fleet.tasks.preview").to_string())
                            .size(ButtonSize::Sm)
                            .on_click(cx.listener(|this, _, _, cx| {
                                if let Err(error) = this.prepare_task_plan(cx) {
                                    this.tasks.outcome = Some(error);
                                }
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("task-discard-form", t!("scripts.cancel").to_string())
                            .variant(ButtonVariant::Ghost)
                            .size(ButtonSize::Sm)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.tasks.form = None;
                                cx.notify();
                            })),
                    ),
            );
        body.into_any_element()
    }

    fn add_task_criterion(&mut self, cx: &mut Context<Self>) -> Result<(), String> {
        let form = self
            .tasks
            .form
            .as_mut()
            .ok_or_else(|| t!("fleet.tasks.context_changed").to_string())?;
        let path = form.path.read(cx).content().trim().to_owned();
        let expected = form.expected.read(cx).content().to_owned();
        let check = match form.check_kind {
            0 => TaskCheck::RegisteredCheck { id: path },
            1 => TaskCheck::ArtifactExists { path },
            2 => TaskCheck::ArtifactContains {
                path,
                text: expected,
            },
            _ => TaskCheck::ArtifactSha256 {
                path,
                digest: expected.trim().to_owned(),
            },
        };
        let criterion = TaskCriterion {
            id: Uuid::new_v4().to_string(),
            description: form.criterion.read(cx).content().trim().to_owned(),
            check,
        };
        criterion
            .validate()
            .map_err(|_| t!("fleet.tasks.invalid_check").to_string())?;
        if form.criteria.len() >= 16 {
            return Err(t!("fleet.tasks.invalid_check").to_string());
        }
        form.criteria.push(criterion);
        for state in [&form.criterion, &form.path, &form.expected] {
            state.update(cx, |state, cx| {
                state.content = "".into();
                cx.notify();
            });
        }
        Ok(())
    }

    fn add_plan_task(&mut self, cx: &mut Context<Self>) -> Result<(), String> {
        let form = self
            .tasks
            .form
            .as_ref()
            .ok_or_else(|| t!("fleet.tasks.context_changed").to_string())?;
        if !form.criterion.read(cx).content().trim().is_empty() || form.criteria.is_empty() {
            self.add_task_criterion(cx)?;
        }
        let form = self.tasks.form.as_mut().unwrap();
        let invalid = || t!("fleet.tasks.invalid_plan").to_string();
        let task = TaskSpec {
            id: OrchestrationTaskId::new(Uuid::new_v4().to_string()).map_err(|_| invalid())?,
            objective: form.objective.read(cx).content().trim().to_owned(),
            criteria: form.criteria.clone(),
            dependencies: form.dependencies.iter().cloned().collect(),
            max_attempts: form
                .attempts
                .read(cx)
                .content()
                .trim()
                .parse()
                .map_err(|_| invalid())?,
            attempt_timeout_ms: form
                .minutes
                .read(cx)
                .content()
                .trim()
                .parse::<u64>()
                .map_err(|_| invalid())?
                .checked_mul(60_000)
                .ok_or_else(invalid)?,
        };
        task.validate().map_err(|_| invalid())?;
        if form.tasks.len() >= 64 {
            return Err(invalid());
        }
        form.tasks.push(task);
        form.criteria.clear();
        form.dependencies.clear();
        form.objective.update(cx, |state, cx| {
            state.content = "".into();
            cx.notify();
        });
        Ok(())
    }

    fn prepare_task_plan(&mut self, cx: &mut Context<Self>) -> Result<(), String> {
        let target = self
            .tasks
            .target
            .clone()
            .ok_or_else(|| t!("fleet.tasks.context_changed").to_string())?;
        if !self.tasks.available || self.tasks.unknown {
            return Err(t!("fleet.tasks.unavailable").to_string());
        }
        if self
            .tasks
            .form
            .as_ref()
            .is_some_and(|form| !form.objective.read(cx).content().trim().is_empty())
        {
            self.add_plan_task(cx)?;
        }
        let form = self
            .tasks
            .form
            .as_ref()
            .ok_or_else(|| t!("fleet.tasks.invalid_plan").to_string())?;
        let invalid = || t!("fleet.tasks.invalid_plan").to_string();
        let plan = TaskPlan {
            run_id: OrchestrationRunId::new(Uuid::new_v4().to_string()).map_err(|_| invalid())?,
            tasks: form.tasks.clone(),
            max_concurrent: form
                .concurrency
                .read(cx)
                .content()
                .trim()
                .parse()
                .map_err(|_| invalid())?,
            time_limit_ms: form
                .total_minutes
                .read(cx)
                .content()
                .trim()
                .parse::<u64>()
                .map_err(|_| invalid())?
                .checked_mul(60_000)
                .ok_or_else(invalid)?,
        };
        plan.validate().map_err(|_| invalid())?;
        let request = TaskCreateRequest {
            project: target.project.clone(),
            workspace: task_workspace(&target).map_err(|_| invalid())?,
            idempotency_key: IdempotencyKey::new(Uuid::new_v4().to_string())
                .map_err(|_| invalid())?,
            plan,
        };
        self.confirm_task_command(PlatformTaskCommand::Create(request), cx);
        Ok(())
    }

    fn prepare_task_control(
        &mut self,
        id: &OrchestrationTaskId,
        action: TaskControlAction,
        cx: &mut Context<Self>,
    ) {
        let Some(task) = self.tasks.tasks.iter().find(|task| &task.spec.id == id) else {
            return;
        };
        if !task_controls(task).contains(&action) || self.tasks.unknown {
            return;
        }
        let request = TaskControlRequest {
            project: task.project.clone(),
            workspace: task.workspace.clone(),
            task: id.clone(),
            expected_revision: task.revision,
            idempotency_key: IdempotencyKey::new(Uuid::new_v4().to_string())
                .expect("UUID is an idempotency key"),
            action,
        };
        self.confirm_task_command(PlatformTaskCommand::Control(request), cx);
    }

    fn confirm_task_command(&mut self, command: PlatformTaskCommand, cx: &mut Context<Self>) {
        if !self.task_command_is_current(&command) {
            return;
        }
        let target = command.target();
        let mut body = div()
            .id("tasks-confirm-body")
            .min_h(px(0.0))
            .max_h(px(320.0))
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap(px(8.0))
            .text_size(px(12.0))
            .child(
                t!(
                    "fleet.tasks.confirm_scope",
                    project = target.project.as_str(),
                    workspace = target.workspace.id()
                )
                .to_string(),
            );
        match &command {
            PlatformTaskCommand::Create(request) => {
                body = body.child(
                    t!(
                        "fleet.tasks.confirm_plan",
                        count = request.plan.tasks.len(),
                        concurrency = request.plan.max_concurrent,
                        minutes = request.plan.time_limit_ms / 60_000
                    )
                    .to_string(),
                );
                for task in &request.plan.tasks {
                    body = body.child(
                        div()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(task.objective.clone()),
                    );
                    body = body.child(
                        t!(
                            "fleet.tasks.confirm_task_limits",
                            attempts = task.max_attempts,
                            minutes = task.attempt_timeout_ms / 60_000
                        )
                        .to_string(),
                    );
                    for dependency in &task.dependencies {
                        if let Some(prerequisite) = request
                            .plan
                            .tasks
                            .iter()
                            .find(|candidate| &candidate.id == dependency)
                        {
                            body = body.child(format!(
                                "{}: {}",
                                t!("fleet.tasks.depends_on"),
                                prerequisite.objective
                            ));
                        }
                    }
                    for criterion in &task.criteria {
                        body = body.child(format!(
                            "{} · {}",
                            criterion.description,
                            check_description(&criterion.check)
                        ));
                    }
                }
                body = body.child(t!("fleet.tasks.confirm_execution").to_string());
            }
            PlatformTaskCommand::Control(request) => {
                let objective = self
                    .tasks
                    .tasks
                    .iter()
                    .find(|task| task.spec.id == request.task)
                    .map(|task| task.spec.objective.as_str())
                    .unwrap_or("");
                body = body.child(
                    t!(
                        "fleet.tasks.confirm_control",
                        action = control_label(request.action),
                        objective = objective,
                        revision = request.expected_revision.get()
                    )
                    .to_string(),
                );
            }
        }
        self.tasks.pending = Some(command);
        let close = cx.entity();
        let confirm = cx.entity();
        let cancel = cx.entity();
        self.tasks.confirmation = Some(cx.new(move |cx| {
            Dialog::new(cx)
                .title(t!("fleet.tasks.confirm_title").to_string())
                .size(DialogSize::Lg)
                .on_close(move |_, cx| {
                    close.update(cx, |this, cx| {
                        this.tasks.confirmation = None;
                        this.tasks.pending = None;
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
                            Button::new("tasks-confirm-cancel", t!("scripts.cancel").to_string())
                                .variant(ButtonVariant::Ghost)
                                .size(ButtonSize::Sm)
                                .on_click(move |_, _, cx| {
                                    cancel.update(cx, |this, cx| {
                                        this.tasks.confirmation = None;
                                        this.tasks.pending = None;
                                        cx.notify();
                                    })
                                }),
                        )
                        .child(
                            Button::new(
                                "tasks-confirm-execute",
                                t!("fleet.tasks.confirm").to_string(),
                            )
                            .size(ButtonSize::Sm)
                            .on_click(move |_, _, cx| {
                                confirm.update(cx, |this, cx| {
                                    this.tasks.confirmation = None;
                                    if let Some(command) = this.tasks.pending.take() {
                                        if this.task_command_is_current(&command)
                                            && !this.tasks.unknown
                                        {
                                            cx.emit(FleetViewEvent::TaskCommand(command));
                                        } else {
                                            this.tasks.outcome =
                                                Some(t!("fleet.tasks.context_changed").to_string());
                                        }
                                    }
                                    cx.notify();
                                })
                            }),
                        ),
                )
        }));
        cx.notify();
    }

    pub(super) fn task_confirmation(&self) -> Option<Entity<Dialog>> {
        self.tasks.confirmation.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::{task_controls, TaskUi};
    use automonique_protocol::{
        platform_v2::{ProjectId, UserWorkspaceId, WorkContextIdentity},
        primitives::Revision,
    };
    use shelldeck_core::config::platform::IdempotencyKey;
    use shelldeck_core::config::platform_review::PlatformReviewTarget;
    use shelldeck_core::config::platform_tasks::*;

    fn fixture() -> TaskSnapshot {
        TaskSnapshot {
            project: ProjectId::new("project-fixture").unwrap(),
            workspace: UserWorkspaceId::new("workspace-fixture").unwrap(),
            run_id: OrchestrationRunId::new("run-fixture").unwrap(),
            spec: TaskSpec {
                id: OrchestrationTaskId::new("task-fixture").unwrap(),
                objective: "Produce the answer artifact".into(),
                criteria: vec![TaskCriterion {
                    id: "answer".into(),
                    description: "Answer exists".into(),
                    check: TaskCheck::ArtifactExists {
                        path: "answer.txt".into(),
                    },
                }],
                dependencies: vec![],
                max_attempts: 2,
                attempt_timeout_ms: 60_000,
            },
            status: TaskStatus::Queued,
            revision: Revision::new(2).unwrap(),
            attempts_used: 0,
            attempt_id: None,
            artifact_revision: None,
            evidence: vec![],
            reason: None,
        }
    }
    fn scope(task: &TaskSnapshot) -> PlatformReviewTarget {
        PlatformReviewTarget {
            project: task.project.clone(),
            workspace: WorkContextIdentity::UserWorkspace(task.workspace.clone()),
        }
    }

    // SDTEST-2704 / SDUC-560
    #[test]
    fn sdtest_2704_task_observations_cannot_regress_revision_or_cross_workspace_and_account_boundaries(
    ) {
        let mut state = TaskUi::default();
        let task = fixture();
        let target = scope(&task);
        state.apply_load(
            Some(target.clone()),
            Some(PlatformTaskLoad::Available(vec![task.clone()], vec![])),
        );
        assert!(state.available);
        assert!(
            state.pending.is_none(),
            "observing tasks cannot prepare an effect"
        );
        let mut older = task.clone();
        older.revision = Revision::FIRST;
        state.apply_load(
            Some(target.clone()),
            Some(PlatformTaskLoad::Available(vec![older], vec![])),
        );
        assert!(!state.available);
        assert_eq!(state.tasks[0].revision, task.revision);
        let mut foreign = target;
        foreign.workspace =
            WorkContextIdentity::UserWorkspace(UserWorkspaceId::new("foreign").unwrap());
        state.apply_load(
            Some(foreign),
            Some(PlatformTaskLoad::Available(vec![task], vec![])),
        );
        assert!(!state.available);
        assert!(state.tasks.is_empty());
        state.apply_load(None, None);
        assert!(state.target.is_none());
        assert!(state.outcome.is_none());
        assert!(state.pending.is_none());
    }

    // SDTEST-2705 / SDUC-560
    #[test]
    fn sdtest_2705_task_controls_use_exact_current_revision_and_unknown_dispatch_never_replays() {
        let mut state = TaskUi::default();
        let mut task = fixture();
        let target = scope(&task);
        state.apply_load(
            Some(target.clone()),
            Some(PlatformTaskLoad::Available(vec![task.clone()], vec![])),
        );
        let mut command = PlatformTaskCommand::Control(TaskControlRequest {
            project: task.project.clone(),
            workspace: task.workspace.clone(),
            task: task.spec.id.clone(),
            expected_revision: task.revision,
            idempotency_key: IdempotencyKey::new("request-fixture").unwrap(),
            action: TaskControlAction::Pause,
        });
        assert!(state.command_is_current(&command));
        task.revision = Revision::new(3).unwrap();
        task.status = TaskStatus::PauseRequested;
        state.apply_load(
            Some(target.clone()),
            Some(PlatformTaskLoad::Available(vec![task.clone()], vec![])),
        );
        assert!(!state.command_is_current(&command));
        let PlatformTaskCommand::Control(request) = &mut command else {
            unreachable!()
        };
        request.expected_revision = task.revision;
        request.action = TaskControlAction::Resume;
        assert!(state.command_is_current(&command));
        for status in [
            TaskStatus::Verified,
            TaskStatus::Failed,
            TaskStatus::Cancelled,
            TaskStatus::CancelRequested,
        ] {
            task.status = status;
            assert!(task_controls(&task).is_empty());
        }
        state.command_result(
            &command,
            PlatformTaskCommandResult::Unknown("connection lost".into()),
        );
        assert!(state.unknown);
        assert!(!state.unknown_observed);
        assert!(
            state.pending.is_none(),
            "unknown result must not prepare a retry"
        );
        task.status = TaskStatus::PauseRequested;
        state.apply_load(
            Some(target),
            Some(PlatformTaskLoad::Available(
                vec![task],
                vec!["resume: task-fixture".into()],
            )),
        );
        assert!(
            state.unknown_observed,
            "fresh observation permits explicit inspection acknowledgment"
        );
        assert!(
            state.unknown,
            "observing status is not correlated proof of success"
        );
    }
}
