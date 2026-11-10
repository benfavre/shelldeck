//! Native Automations is another projection of the same session scheduler.
use super::*;
use shelldeck_core::agent_workflows::CalendarSchedule;

#[derive(Clone, PartialEq, Eq)]
pub struct LocalTaskSession {
    pub id: Uuid,
    pub name: String,
    pub context: AgentExecutionContext,
}

#[cfg(test)]
#[path = "scheduled_tasks/tests.rs"]
mod tests;
#[derive(Clone, Copy)]
pub enum LocalTaskCommand {
    Activate,
    Pause,
    RunNow,
    Delete,
}
impl AgentConsoleView {
    pub fn local_tasks_ready(&self) -> bool {
        self.workflows.authorized && self.workflows.writable && !self.workflows.loading
    }
    pub fn local_task_storage_error(&self) -> Option<&str> {
        self.workflows
            .authorized
            .then_some(self.workflows.error.as_deref())
            .flatten()
    }
    pub fn local_scheduled_tasks(&self) -> Vec<AgentSchedule> {
        if !self.workflows.authorized {
            return Vec::new();
        }
        self.workflows
            .state
            .schedules
            .iter()
            .filter(|task| task.context.target == AgentTarget::Local)
            .cloned()
            .collect()
    }
    pub fn local_task_sessions(&self, cx: &App) -> Vec<LocalTaskSession> {
        if !self.workflows.authorized {
            return Vec::new();
        }
        self.sessions
            .sessions()
            .iter()
            .filter_map(|session| {
                let context = if self.sessions.selected_id() == Some(session.id) {
                    self.displayed_context(cx)
                } else {
                    session.context.clone()
                };
                (!session.archived
                    && context.target == AgentTarget::Local
                    && context.access == AgentAccessMode::ReadOnly)
                    .then(|| LocalTaskSession {
                        id: session.id,
                        name: session.name.clone(),
                        context,
                    })
            })
            .collect()
    }
    fn local_task_context(
        &self,
        session_id: Uuid,
        cx: &App,
    ) -> Result<AgentExecutionContext, String> {
        if !self.local_tasks_ready() {
            return Err(t!("agents.workflows.storage_unavailable").to_string());
        }
        self.local_task_sessions(cx)
            .into_iter()
            .find(|session| session.id == session_id)
            .map(|session| session.context)
            .ok_or_else(|| t!("automations.local_session_required").to_string())
    }
    pub fn save_local_scheduled_task(
        &mut self,
        edit: Option<Uuid>,
        session_id: Uuid,
        title: String,
        prompt: String,
        calendar: CalendarSchedule,
        cx: &mut Context<Self>,
    ) -> Result<Uuid, String> {
        let context = self.local_task_context(session_id, cx)?;
        if edit.is_none()
            && self.workflows.state.goals.len() + self.workflows.state.schedules.len()
                >= MAX_WORKFLOWS
        {
            return Err(t!("automations.local_limit").to_string());
        }
        let mut task =
            AgentSchedule::new_calendar(session_id, context, title, prompt, calendar, now_ms())
                .map_err(|error| error.to_string())?;
        if let Some(id) = edit {
            let existing = self
                .workflows
                .state
                .schedules
                .iter_mut()
                .find(|task| task.id == id)
                .ok_or_else(|| t!("agents.workflows.missing").to_string())?;
            if existing.active_run.is_some()
                || self
                    .workflows
                    .pending
                    .values()
                    .any(|pending| pending.origin == WorkflowOrigin::Schedule(id))
            {
                return Err(t!("agents.workflows.running").to_string());
            }
            if existing.context.target != AgentTarget::Local || existing.calendar.is_none() {
                return Err(t!("agents.workflows.context_changed").to_string());
            }
            task.id = id;
            *existing = task.clone();
        } else {
            self.workflows.state.schedules.push(task.clone());
        }
        self.save_agent_workflows(cx);
        cx.notify();
        Ok(task.id)
    }
    pub fn local_task_command(
        &mut self,
        id: Uuid,
        command: LocalTaskCommand,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        let task = self
            .workflows
            .state
            .schedules
            .iter()
            .find(|task| task.id == id)
            .cloned()
            .ok_or_else(|| t!("agents.workflows.missing").to_string())?;
        if !self.local_tasks_ready() || task.context.target != AgentTarget::Local {
            return Err(t!("agents.workflows.storage_unavailable").to_string());
        }
        let context = if matches!(
            command,
            LocalTaskCommand::Activate | LocalTaskCommand::RunNow
        ) {
            self.local_task_context(task.session_id, cx)?
        } else {
            task.context.clone()
        };
        // Pausing/deleting remains possible after the session's context drifts.
        if matches!(
            command,
            LocalTaskCommand::Activate | LocalTaskCommand::RunNow
        ) && task.context != context
        {
            return Err(t!("agents.workflows.context_changed").to_string());
        }
        if matches!(command, LocalTaskCommand::Pause | LocalTaskCommand::Delete) {
            let pending = self
                .workflows
                .pending
                .iter()
                .find_map(|(session, pending)| {
                    (pending.origin == WorkflowOrigin::Schedule(id)).then_some(*session)
                });
            if self
                .pending_confirm
                .as_ref()
                .is_some_and(|confirm| Some(confirm.session_id) == pending)
            {
                self.pending_confirm = None;
            }
        }
        match command {
            LocalTaskCommand::Activate => {
                if task.is_once() && task.last_run_ms.is_some() {
                    return Err(t!("automations.local_once_done").to_string());
                }
                self.workflows
                    .state
                    .schedules
                    .iter_mut()
                    .find(|task| task.id == id)
                    .unwrap()
                    .enable(now_ms());
            }
            LocalTaskCommand::Pause => {
                self.workflows
                    .pending
                    .retain(|_, pending| pending.origin != WorkflowOrigin::Schedule(id));
                self.pause_workflow(WorkflowOrigin::Schedule(id), WorkflowPauseReason::User, cx);
                if let Some(run) = task.active_run {
                    cx.emit(AgentConsoleEvent::Stop(run));
                }
            }
            LocalTaskCommand::Delete => {
                if task.active_run.is_some() {
                    return Err(t!("agents.workflows.running").to_string());
                }
                self.workflows.state.schedules.retain(|task| task.id != id);
                self.workflows
                    .pending
                    .retain(|_, pending| pending.origin != WorkflowOrigin::Schedule(id));
            }
            LocalTaskCommand::RunNow => {
                if self.pending_confirm.is_some()
                    || !self.workflows.pending.is_empty()
                    || self
                        .sessions
                        .get(task.session_id)
                        .is_none_or(|session| session.status.is_active())
                {
                    return Err(t!("agents.workflows.running").to_string());
                }
                // Explicit test is an ordinary readonly turn. It does not
                // activate the schedule or advance its next due time.
                self.start_pending(
                    PendingRun {
                        source_draft: None,
                        session_id: task.session_id,
                        context,
                        prompt: task.prompt,
                        options: Default::default(),
                    },
                    cx,
                );
            }
        }
        self.save_agent_workflows(cx);
        cx.notify();
        Ok(())
    }
}
