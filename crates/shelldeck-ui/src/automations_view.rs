//! Manage workflows and desktop-local agent tasks share one task surface.
//! Local runs stay owned by the canonical AgentConsole session scheduler.
mod local;
mod rendering;
use crate::theme::ShellDeckColors;
use adabraka_ui::components::input::InputState;
use gpui::*;
use serde_json::{json, Value};
use shelldeck_core::config::manage_automations::{
    self as api, AutomationData, AutomationError, TaskDraft, TaskEditor,
};

#[derive(Clone, PartialEq, Eq)]
pub struct AutomationContext {
    pub base: String,
    pub token: String,
    pub site: String,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Tasks,
    Workflows,
    Runs,
}
pub enum AutomationEvent {
    AuthRejected,
    Reconnect,
    OpenAgent(Option<uuid::Uuid>),
}
impl EventEmitter<AutomationEvent> for AutomationsView {}

struct TaskForm {
    id: Option<String>,
    revision: Option<String>,
    name: Entity<InputState>,
    prompt: Entity<InputState>,
    cron: Entity<InputState>,
    timezone: Entity<InputState>,
    once: bool,
    target: TaskTarget,
    local_id: Option<uuid::Uuid>,
    session_id: Option<uuid::Uuid>,
    session_select: Option<Entity<adabraka_ui::components::select::Select<uuid::Uuid>>>,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum TaskTarget {
    Manage,
    Local,
}
#[derive(Clone)]
enum DeleteTarget {
    Manage(String),
    Local(uuid::Uuid),
}
pub struct AutomationsView {
    context: Option<AutomationContext>,
    site_label: String,
    generation: u64,
    data: Option<AutomationData>,
    error: Option<AutomationError>,
    busy: bool,
    tab: Tab,
    page: usize,
    search: Entity<InputState>,
    editor: Option<TaskEditor>,
    form: Option<TaskForm>,
    confirm_delete: Option<DeleteTarget>,
    local_agents: Option<Entity<crate::agent_console_view::AgentConsoleView>>,
    local_editor: Option<uuid::Uuid>,
    _local_subscription: Option<Subscription>,
    local_session_options: Vec<crate::agent_console_view::LocalTaskSession>,
    request: Option<Task<()>>,
    poll: Option<Task<()>>,
    _search_subscription: Subscription,
}
enum Operation {
    Save(TaskDraft),
    Edit(String),
    Action(Value),
    Open(String, Option<String>),
}
enum Outcome {
    Editor(TaskEditor),
    Action(Value),
    Url(String),
}

impl AutomationsView {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let search = cx.new(InputState::new);
        let subscription = cx.observe(&search, |_, _, cx| cx.notify());
        Self {
            context: None,
            site_label: String::new(),
            generation: 0,
            data: None,
            error: None,
            busy: false,
            tab: Tab::Tasks,
            page: 1,
            search,
            editor: None,
            form: None,
            confirm_delete: None,
            local_agents: None,
            local_editor: None,
            _local_subscription: None,
            local_session_options: Vec::new(),
            request: None,
            poll: None,
            _search_subscription: subscription,
        }
    }
    pub fn set_context(
        &mut self,
        context: Option<AutomationContext>,
        label: String,
        active: bool,
        cx: &mut Context<Self>,
    ) {
        let changed = self.context != context;
        if changed {
            self.generation = self.generation.wrapping_add(1);
            self.context = context;
            self.request = None;
            self.poll = None;
            self.data = None;
            self.error = None;
            self.busy = false;
            self.editor = None;
            self.local_editor = None;
            self.form = None;
            self.confirm_delete = None;
            self.page = 1;
            self.search
                .update(cx, |state, cx| state.replace_content("", cx));
            cx.notify();
        }
        self.site_label = label;
        if !active {
            self.poll = None;
            return;
        }
        if self.context.is_some() && changed {
            self.refresh(cx);
        }
        if self.context.is_some() && self.poll.is_none() {
            if self.data.is_none() && self.error.is_none() && !self.busy {
                self.refresh(cx);
            }
            self.poll = Some(cx.spawn(async move |this, cx: &mut AsyncApp| loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_secs(60))
                    .await;
                if this.update(cx, |view, cx| view.refresh(cx)).is_err() {
                    break;
                }
            }));
        }
    }
    fn fail(&mut self, error: AutomationError, cx: &mut Context<Self>) {
        if error.status == 401 {
            self.data = None;
            self.editor = None;
            self.form = None;
            cx.emit(AutomationEvent::AuthRejected);
        }
        self.error = Some(error);
        self.busy = false;
        cx.notify();
    }
    fn refresh(&mut self, cx: &mut Context<Self>) {
        if self.busy || self.form.is_some() {
            return;
        }
        let Some(context) = self.context.clone() else {
            return;
        };
        let generation = self.generation;
        let page = self.page;
        self.busy = true;
        self.request = Some(cx.spawn(async move |this, cx: &mut AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(
                    async move { api::fetch(&context.base, &context.token, &context.site, page) },
                )
                .await;
            let _ = this.update(cx, |view, cx| {
                if view.generation != generation {
                    return;
                }
                view.busy = false;
                match result {
                    Ok(data) => {
                        view.data = Some(data);
                        view.error = None;
                        cx.notify();
                    }
                    Err(error) => view.fail(error, cx),
                }
            });
        }));
        cx.notify();
    }
    fn operation(&mut self, operation: Operation, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let Some(context) = self.context.clone() else {
            return;
        };
        let generation = self.generation;
        self.busy = true;
        self.error = None;
        self.request = Some(cx.spawn(async move |this, cx: &mut AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    match operation {
                        Operation::Save(draft) => {
                            api::save(&context.base, &context.token, &context.site, &draft)
                                .map(Outcome::Editor)
                        }
                        Operation::Edit(id) => {
                            api::editor(&context.base, &context.token, &context.site, &id)
                                .map(Outcome::Editor)
                        }
                        Operation::Action(body) => {
                            api::action(&context.base, &context.token, &context.site, body)
                                .map(Outcome::Action)
                        }
                        Operation::Open(destination, id) => api::open_url(
                            &context.base,
                            &context.token,
                            &context.site,
                            &destination,
                            id.as_deref(),
                        )
                        .map(Outcome::Url),
                    }
                })
                .await;
            let _ = this.update(cx, |view, cx| {
                if view.generation != generation {
                    return;
                }
                view.busy = false;
                match result {
                    Ok(Outcome::Editor(editor)) => {
                        view.editor = Some(editor);
                        view.form = None;
                        cx.notify();
                    }
                    Ok(Outcome::Action(value)) => {
                        view.form = None;
                        if value.get("editor").is_some() {
                            match serde_json::from_value(value["editor"].clone()) {
                                Ok(editor) => view.editor = Some(editor),
                                Err(_) => view.editor = None,
                            }
                        } else {
                            view.editor = None;
                        }
                        if value.get("execution_id").is_some_and(|id| !id.is_null()) {
                            view.tab = Tab::Runs;
                        }
                        view.refresh(cx);
                    }
                    Ok(Outcome::Url(url)) => {
                        if let Err(error) =
                            shelldeck_core::config::cloud_account::open_in_browser(&url)
                        {
                            view.fail(
                                AutomationError {
                                    status: 0,
                                    needs_reconnect: false,
                                    message: error.to_string(),
                                },
                                cx,
                            );
                        }
                        cx.notify();
                    }
                    Err(error) => view.fail(error, cx),
                }
            });
        }));
        cx.notify();
    }
    fn new_form(&mut self, draft: TaskDraft, cx: &mut Context<Self>) {
        self.local_editor = None;
        let local_sessions = self.local_sessions(cx);
        let selected = self
            .local_agents
            .as_ref()
            .and_then(|agents| agents.read(cx).selected_session_id());
        let local_session = local_sessions
            .iter()
            .find(|session| Some(session.id) == selected)
            .or_else(|| local_sessions.first())
            .map(|session| session.id);
        let field = |value: String, multiline: bool, cx: &mut Context<Self>| {
            cx.new(|cx| {
                let mut state = InputState::new(cx).multi_line(multiline);
                state.content = value.into();
                state
            })
        };
        self.form = Some(TaskForm {
            id: draft.id,
            revision: draft.revision,
            name: field(draft.name, false, cx),
            prompt: field(draft.prompt, true, cx),
            cron: field(draft.cron, false, cx),
            timezone: field(draft.timezone, false, cx),
            once: draft.once,
            target: if self.context.is_some() {
                TaskTarget::Manage
            } else {
                TaskTarget::Local
            },
            local_id: None,
            session_id: local_session,
            session_select: None,
        });
        self.rebuild_local_session_select(cx);
        self.error = None;
        cx.notify();
    }
    pub fn start_task(&mut self, index: Option<usize>, cx: &mut Context<Self>) {
        let mut draft = TaskDraft {
            cron: "0 9 * * 1".into(),
            timezone: "Europe/Paris".into(),
            ..Default::default()
        };
        if let Some(index) = index {
            draft.name = t!(format!("automations.template.{index}.title")).to_string();
            draft.prompt = t!(format!("automations.template.{index}.prompt")).to_string();
        }
        self.new_form(draft, cx);
    }
    fn save_form(&mut self, cx: &mut Context<Self>) {
        let Some(form) = &self.form else {
            return;
        };
        let value = |field: &Entity<InputState>| field.read(cx).content().trim().to_string();
        let draft = TaskDraft {
            id: form.id.clone(),
            revision: form.revision.clone(),
            name: value(&form.name),
            prompt: value(&form.prompt),
            cron: value(&form.cron),
            timezone: value(&form.timezone),
            once: form.once,
        };
        if draft.name.is_empty()
            || draft.name.chars().count() > 100
            || draft.prompt.is_empty()
            || draft.prompt.chars().count() > 8000
            || draft.cron.split_whitespace().count() != 5
            || draft.timezone.is_empty()
        {
            self.fail(
                AutomationError {
                    status: 0,
                    needs_reconnect: false,
                    message: t!("automations.validation").to_string(),
                },
                cx,
            );
            return;
        }
        if form.target == TaskTarget::Local {
            self.save_local_form(draft, cx);
        } else {
            self.operation(Operation::Save(draft), cx);
        }
    }
    fn change_tab(&mut self, tab: Tab, cx: &mut Context<Self>) {
        self.tab = tab;
        self.page = 1;
        self.editor = None;
        self.local_editor = None;
        self.form = None;
        self.refresh(cx);
        cx.notify();
    }
}

#[cfg(test)]
mod tests;
