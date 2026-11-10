use super::rendering::{date, field, status};
use super::*;
use crate::agent_console_view::{AgentConsoleView, LocalTaskCommand, LocalTaskSession};
use crate::scale::px;
use adabraka_ui::components::{
    button::{Button, ButtonSize, ButtonVariant},
    select::{Select, SelectOption},
};
use gpui::prelude::*;
use shelldeck_core::agent_workflows::{AgentSchedule, CalendarSchedule};
use uuid::Uuid;

impl AutomationsView {
    pub fn set_local_agents(&mut self, agents: Entity<AgentConsoleView>, cx: &mut Context<Self>) {
        self._local_subscription = Some(cx.observe(&agents, |this, _, cx| {
            if this.local_sessions(cx) != this.local_session_options {
                this.rebuild_local_session_select(cx);
            }
            cx.notify();
        }));
        self.local_agents = Some(agents);
        self.rebuild_local_session_select(cx);
    }
    pub(super) fn local_sessions(&self, cx: &App) -> Vec<LocalTaskSession> {
        self.local_agents
            .as_ref()
            .map(|agents| agents.read(cx).local_task_sessions(cx))
            .unwrap_or_default()
    }
    pub(super) fn local_tasks(&self, cx: &App) -> Vec<AgentSchedule> {
        self.local_agents
            .as_ref()
            .map(|agents| agents.read(cx).local_scheduled_tasks())
            .unwrap_or_default()
    }
    pub(super) fn local_ready(&self, cx: &App) -> bool {
        self.local_agents
            .as_ref()
            .is_some_and(|agents| agents.read(cx).local_tasks_ready())
    }
    pub(super) fn rebuild_local_session_select(&mut self, cx: &mut Context<Self>) {
        self.local_session_options = self.local_sessions(cx);
        let Some(form) = &mut self.form else { return };
        let options: Vec<_> = self
            .local_session_options
            .iter()
            .map(|session| {
                SelectOption::new(
                    session.id,
                    format!(
                        "{} · {}",
                        session.name,
                        session.context.provider.display_name()
                    ),
                )
            })
            .collect();
        let selected = options
            .iter()
            .position(|option| Some(option.value) == form.session_id)
            .or_else(|| (!options.is_empty()).then_some(0));
        form.session_id = selected.map(|index| options[index].value);
        let parent = cx.entity();
        form.session_select = Some(cx.new(move |cx| {
            Select::new(cx)
                .options(options)
                .selected_index(selected)
                .searchable(true)
                .placeholder(t!("automations.local_session_required").to_string())
                .search_placeholder(t!("automations.search").to_string())
                .on_change(move |id, _, cx| {
                    parent.update(cx, |this, cx| {
                        if let Some(form) = &mut this.form {
                            form.session_id = Some(*id);
                        }
                        cx.notify();
                    })
                })
        }));
    }
    fn local_error(&mut self, message: String, cx: &mut Context<Self>) {
        self.fail(
            AutomationError {
                status: 0,
                needs_reconnect: false,
                message,
            },
            cx,
        );
    }
    pub(super) fn save_local_form(&mut self, draft: TaskDraft, cx: &mut Context<Self>) {
        let Some(form) = &self.form else { return };
        let Some(agents) = self.local_agents.clone() else {
            return;
        };
        let Some(session_id) = form.session_id else {
            self.local_error(t!("automations.local_session_required").to_string(), cx);
            return;
        };
        let edit = form.local_id;
        let calendar = CalendarSchedule {
            cron: draft.cron,
            timezone: draft.timezone,
            once: draft.once,
        };
        if calendar
            .next_after(chrono::Utc::now().timestamp_millis())
            .is_err()
        {
            self.local_error(t!("automations.validation").to_string(), cx);
            return;
        }
        match agents.update(cx, |agents, cx| {
            agents.save_local_scheduled_task(
                edit,
                session_id,
                draft.name,
                draft.prompt,
                calendar,
                cx,
            )
        }) {
            Ok(id) => {
                self.form = None;
                self.local_editor = Some(id);
                self.error = None;
                cx.notify();
            }
            Err(error) => self.local_error(error, cx),
        }
    }
    pub(super) fn local_command(
        &mut self,
        id: Uuid,
        command: LocalTaskCommand,
        cx: &mut Context<Self>,
    ) {
        let Some(agents) = self.local_agents.clone() else {
            return;
        };
        let session = self
            .local_tasks(cx)
            .iter()
            .find(|task| task.id == id)
            .map(|task| task.session_id);
        match agents.update(cx, |agents, cx| agents.local_task_command(id, command, cx)) {
            Ok(()) => {
                self.error = None;
                if matches!(command, LocalTaskCommand::Delete) {
                    self.local_editor = None;
                }
                if matches!(command, LocalTaskCommand::RunNow) {
                    cx.emit(AutomationEvent::OpenAgent(session));
                }
                cx.notify();
            }
            Err(error) => self.local_error(error, cx),
        }
    }
    fn edit_local_task(&mut self, id: Uuid, cx: &mut Context<Self>) {
        let Some(task) = self.local_tasks(cx).into_iter().find(|task| task.id == id) else {
            return;
        };
        let Some(calendar) = task.calendar else {
            return;
        };
        self.new_form(
            TaskDraft {
                name: task.title,
                prompt: task.prompt,
                cron: calendar.cron,
                timezone: calendar.timezone,
                once: calendar.once,
                ..Default::default()
            },
            cx,
        );
        if let Some(form) = &mut self.form {
            form.target = TaskTarget::Local;
            form.local_id = Some(id);
            form.session_id = Some(task.session_id);
        }
        self.rebuild_local_session_select(cx);
    }
    pub(super) fn render_execution_target(&self, cx: &mut Context<Self>) -> Div {
        let form = self.form.as_ref().unwrap();
        let locked = form.id.is_some() || form.local_id.is_some();
        let mut choices = div().flex().gap(px(8.0));
        for (index, target, key) in [
            (0, TaskTarget::Manage, "automations.target_manage"),
            (1, TaskTarget::Local, "automations.target_local"),
        ] {
            choices = choices.child(
                Button::new(("automation-target", index as usize), t!(key).to_string())
                    .debug_selector(move || format!("automation-target-{index}"))
                    .variant(ButtonVariant::Secondary)
                    .selected(form.target == target)
                    .disabled(
                        self.busy
                            || locked
                            || (target == TaskTarget::Manage && self.context.is_none()),
                    )
                    .on_click(cx.listener(move |view, _, _, cx| {
                        if let Some(form) = &mut view.form {
                            form.target = target;
                        }
                        view.error = None;
                        cx.notify();
                    })),
            );
        }
        let mut body = div().flex().flex_col().gap(px(12.0)).child(field(
            t!("automations.execution_target").to_string(),
            choices,
        ));
        if form.target == TaskTarget::Local {
            body = body.child(hint("automations.local_hint"));
            if let Some(error) = self
                .local_agents
                .as_ref()
                .and_then(|agents| agents.read(cx).local_task_storage_error())
            {
                body = body.child(
                    div()
                        .text_size(px(12.0))
                        .text_color(ShellDeckColors::error())
                        .child(error.to_string()),
                );
            }
            if let Some(select) = &form.session_select {
                body = body.child(field(
                    t!("automations.local_session").to_string(),
                    select.clone(),
                ));
            }
            if let Some(session) = self
                .local_sessions(cx)
                .iter()
                .find(|session| Some(session.id) == form.session_id)
            {
                body = body.child(
                    div()
                        .text_size(px(12.0))
                        .text_color(ShellDeckColors::text_muted())
                        .child(format!(
                            "{} · {} · {}",
                            session.context.provider.display_name(),
                            t!("agents.access.read_only"),
                            session.context.workdir
                        )),
                );
            }
            body = body.child(
                Button::new(
                    "automation-configure-agent",
                    t!("automations.configure_agent").to_string(),
                )
                .size(ButtonSize::Sm)
                .variant(ButtonVariant::Ghost)
                .on_click(cx.listener(|_, _, _, cx| cx.emit(AutomationEvent::OpenAgent(None)))),
            );
        } else {
            body = body.child(hint("automations.manage_hint"));
        }
        body
    }
    pub(super) fn render_local_rows(&self, query: &str, cx: &mut Context<Self>) -> Vec<AnyElement> {
        self.local_tasks(cx)
            .into_iter()
            .filter(|task| {
                format!("{} {} {}", task.title, task.prompt, task.context.workdir)
                    .to_lowercase()
                    .contains(query)
            })
            .map(|task| {
                let id = task.id;
                let (label, color) = status(local_status(&task));
                div()
                    .flex()
                    .flex_col()
                    .gap(px(10.0))
                    .p(px(18.0))
                    .w_full()
                    .rounded(px(10.0))
                    .border_1()
                    .border_color(ShellDeckColors::border())
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .items_center()
                            .gap(px(12.0))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w(px(160.0))
                                    .text_size(px(15.0))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(task.title.clone()),
                            )
                            .child(div().text_size(px(12.0)).text_color(color).child(label)),
                    )
                    .child(
                        div()
                            .text_size(px(11.0))
                            .text_color(ShellDeckColors::primary())
                            .child(format!(
                                "{} · {}",
                                t!("automations.target_local"),
                                task.context.provider.display_name()
                            )),
                    )
                    .child(
                        div()
                            .text_size(px(12.0))
                            .text_color(ShellDeckColors::text_muted())
                            .line_clamp(2)
                            .child(task.prompt.clone()),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .gap(px(8.0))
                            .items_center()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w(px(180.0))
                                    .text_size(px(11.0))
                                    .text_color(ShellDeckColors::text_muted())
                                    .child(
                                        t!("automations.next_run", date = local_next_date(&task))
                                            .to_string(),
                                    ),
                            )
                            .child(
                                Button::new(
                                    SharedString::from(format!("local-task-{id}")),
                                    t!("automations.details").to_string(),
                                )
                                .size(ButtonSize::Sm)
                                .variant(ButtonVariant::Secondary)
                                .on_click(cx.listener(
                                    move |view, _, _, cx| {
                                        view.local_editor = Some(id);
                                        cx.notify();
                                    },
                                )),
                            ),
                    )
                    .into_any_element()
            })
            .collect()
    }
    pub(super) fn render_local_editor(&self, cx: &mut Context<Self>) -> Div {
        let Some(task) = self
            .local_tasks(cx)
            .into_iter()
            .find(|task| Some(task.id) == self.local_editor)
        else {
            return div().child(t!("agents.workflows.missing").to_string());
        };
        let id = task.id;
        let session = task.session_id;
        let (label, color) = status(local_status(&task));
        let once_done = task.is_once() && task.last_run_ms.is_some();
        let editable = task.calendar.is_some()
            && task.context.access == shelldeck_core::agent_runtime::AgentAccessMode::ReadOnly;
        let mut body = div()
            .flex()
            .flex_col()
            .w_full()
            .max_w(px(800.0))
            .gap(px(20.0))
            .child(
                div()
                    .text_size(px(24.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(task.title.clone()),
            )
            .child(div().text_size(px(13.0)).text_color(color).child(label))
            .child(hint("automations.local_hint"))
            .child(div().text_size(px(13.0)).child(task.prompt.clone()))
            .child(
                div()
                    .text_size(px(12.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(format!(
                        "{} · {}",
                        task.context.provider.display_name(),
                        task.context.workdir
                    )),
            )
            .child(
                div()
                    .text_size(px(12.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(t!("automations.next_run", date = local_next_date(&task)).to_string()),
            );
        if let Some(calendar) = &task.calendar {
            body = body.child(
                div()
                    .text_size(px(12.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(format!(
                        "{} · {}{}",
                        calendar.cron,
                        calendar.timezone,
                        if calendar.once {
                            format!(" · {}", t!("automations.once"))
                        } else {
                            String::new()
                        }
                    )),
            );
        }
        if let Some(reason) = task.pause_reason {
            body = body.child(
                div()
                    .text_size(px(12.0))
                    .text_color(ShellDeckColors::warning())
                    .child(t!(format!("agents.workflows.reason_{}", reason.key())).to_string()),
            );
        }
        let mut actions = div().flex().flex_wrap().gap(px(8.0));
        if editable {
            actions = actions.child(
                Button::new(
                    "automation-local-edit",
                    t!("automations.edit_task").to_string(),
                )
                .variant(ButtonVariant::Secondary)
                .disabled(task.active_run.is_some() || !self.local_ready(cx))
                .on_click(cx.listener(move |view, _, _, cx| view.edit_local_task(id, cx))),
            );
            if !once_done || task.enabled {
                let command = if task.enabled {
                    LocalTaskCommand::Pause
                } else {
                    LocalTaskCommand::Activate
                };
                actions = actions.child(
                    Button::new(
                        "automation-local-toggle",
                        t!(if task.enabled {
                            "automations.pause"
                        } else {
                            "automations.activate"
                        })
                        .to_string(),
                    )
                    .disabled(!self.local_ready(cx))
                    .on_click(
                        cx.listener(move |view, _, _, cx| view.local_command(id, command, cx)),
                    ),
                );
            }
            actions = actions.child(
                Button::new(
                    "automation-local-run",
                    t!("automations.run_now").to_string(),
                )
                .variant(ButtonVariant::Secondary)
                .disabled(task.active_run.is_some() || !self.local_ready(cx))
                .on_click(cx.listener(move |view, _, _, cx| {
                    view.local_command(id, LocalTaskCommand::RunNow, cx)
                })),
            );
        }
        body.child(
            actions
                .child(
                    Button::new(
                        "automation-local-results",
                        t!("automations.open_agent").to_string(),
                    )
                    .variant(ButtonVariant::Ghost)
                    .on_click(cx.listener(move |_, _, _, cx| {
                        cx.emit(AutomationEvent::OpenAgent(Some(session)))
                    })),
                )
                .child(
                    Button::new(
                        "automation-local-delete",
                        t!("automations.delete").to_string(),
                    )
                    .variant(ButtonVariant::Ghost)
                    .disabled(task.active_run.is_some() || !self.local_ready(cx))
                    .on_click(cx.listener(move |view, _, _, cx| {
                        view.confirm_delete = Some(DeleteTarget::Local(id));
                        cx.notify();
                    })),
                ),
        )
    }
    pub(super) fn render_local_history(
        &self,
        query: &str,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let Some(agents) = &self.local_agents else {
            return Vec::new();
        };
        let tasks = self.local_tasks(cx);
        let mut turns: Vec<_> = agents
            .read(cx)
            .sessions()
            .iter()
            .filter(|session| {
                tasks.iter().any(|task| task.session_id == session.id)
                    && session.name.to_lowercase().contains(query)
            })
            .flat_map(|session| session.turns.iter().map(move |turn| (session, turn)))
            .collect();
        turns.sort_by_key(|(_, turn)| std::cmp::Reverse(turn.started_at_ms));
        turns
            .into_iter()
            .take(20)
            .map(|(session, turn)| {
                let id = session.id;
                let (label, color) = status(match turn.status {
                    shelldeck_core::agent_session::AgentSessionStatus::Completed => "COMPLETED",
                    shelldeck_core::agent_session::AgentSessionStatus::Failed => "FAILED",
                    shelldeck_core::agent_session::AgentSessionStatus::Cancelled => "CANCELLED",
                    _ => "RUNNING",
                });
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap(px(12.0))
                    .p(px(16.0))
                    .border_b_1()
                    .border_color(ShellDeckColors::border())
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(180.0))
                            .text_size(px(13.0))
                            .child(format!(
                                "{} · {}",
                                t!("automations.target_local"),
                                session.name
                            )),
                    )
                    .child(div().text_size(px(12.0)).text_color(color).child(label))
                    .child(
                        div()
                            .text_size(px(12.0))
                            .text_color(ShellDeckColors::text_muted())
                            .child(local_date(turn.started_at_ms)),
                    )
                    .child(
                        Button::new(
                            SharedString::from(format!("local-history-{}", turn.id)),
                            t!("automations.open_agent").to_string(),
                        )
                        .size(ButtonSize::Sm)
                        .variant(ButtonVariant::Ghost)
                        .on_click(cx.listener(move |_, _, _, cx| {
                            cx.emit(AutomationEvent::OpenAgent(Some(id)))
                        })),
                    )
                    .into_any_element()
            })
            .collect()
    }
}
fn hint(key: &str) -> Div {
    div()
        .text_size(px(12.0))
        .text_color(ShellDeckColors::text_muted())
        .child(t!(key).to_string())
}
fn local_date(ms: i64) -> String {
    date(
        chrono::DateTime::from_timestamp_millis(ms)
            .map(|date| date.to_rfc3339())
            .as_deref(),
    )
}
fn local_next_date(task: &AgentSchedule) -> String {
    if task.enabled && task.next_due_ms != i64::MAX {
        local_date(task.next_due_ms)
    } else {
        t!("automations.not_scheduled").to_string()
    }
}
fn local_status(task: &AgentSchedule) -> &'static str {
    if task.active_run.is_some() {
        "RUNNING"
    } else if task.enabled {
        "ACTIVE"
    } else if task.pause_reason.is_some() {
        "PAUSED"
    } else if task.is_once() && task.last_run_ms.is_some() {
        "COMPLETED"
    } else {
        "DRAFT"
    }
}
