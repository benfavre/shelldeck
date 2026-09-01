//! Worktree-first coding-agent cockpit for Dev mode.
//!
//! Durable session/message/trace state is owned by `shelldeck-core`. This view
//! adds GPUI drafts, navigation, execution controls, and run-id routing while
//! Workspace retains process and SSH lifetimes.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use adabraka_ui::components::confirm_dialog::Dialog as UiDialog;
use adabraka_ui::components::icon_source::IconSource;
use adabraka_ui::components::input::{Input, InputSize, InputVariant};
use adabraka_ui::components::input_state::InputState;
use adabraka_ui::components::select::{Select, SelectOption};
use adabraka_ui::overlays::popover::PopoverContent;
use adabraka_ui::prelude::{
    Badge, BadgeVariant, Button, ButtonSize, ButtonVariant, Composer, Markdown, Popover,
};
use chrono::Utc;
use gpui::prelude::*;
use gpui::*;
use shelldeck_core::agent_runtime::{
    AgentAccessMode, AgentProvider, AgentRunRequest, AgentStreamEvent, AgentTarget,
    AGENT_REMOTE_HOME_WORKDIR, MAX_AGENT_MODEL_BYTES, MAX_AGENT_PROMPT_BYTES,
    MAX_AGENT_WORKDIR_BYTES,
};
use shelldeck_core::agent_session::{
    AgentExecutionContext, AgentMessage, AgentMessageRole, AgentSession, AgentSessionAttention,
    AgentSessionCollection, AgentSessionStatus, AgentTraceEvent, AgentTraceKind, AgentTraceStatus,
    DEFAULT_MAX_CONCURRENT_AGENT_SESSIONS,
};
use uuid::Uuid;

use crate::file_editor::file_browser::FileBrowserPanel;
use crate::follow_scroll::{follow_latest_if_at_end, pin_to_latest};
use crate::icons::lucide_icon;
use crate::monolith::{animated_loading_text, animated_monolith, MonolithMotion};
use crate::scale::px;
use crate::t;
use crate::theme::ShellDeckColors;

const MAX_ACTIVITY_BYTES: usize = 4 * 1024;
const NAVIGATOR_WIDTH: f32 = 238.0;
const INSPECTOR_WIDTH: f32 = 286.0;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum AgentInspectorTab {
    #[default]
    Files,
    Changes,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct InspectorFile {
    path: String,
    latest_trace_id: Uuid,
    latest_sequence: u64,
    diff: Option<InspectorDiff>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct InspectorDiff {
    trace_id: Uuid,
    sequence: u64,
    additions: u32,
    deletions: u32,
    preview: Option<String>,
}

#[derive(Debug, Clone)]
pub enum AgentConsoleEvent {
    Run(AgentRunRequest),
    Stop(Uuid),
    CloseSession(Uuid),
    OpenLocalFile {
        session_id: Uuid,
        path: PathBuf,
    },
    BrowseRemoteDirectory {
        session_id: Uuid,
        connection_id: Uuid,
        path: String,
    },
}

impl EventEmitter<AgentConsoleEvent> for AgentConsoleView {}

#[derive(Debug, Clone)]
pub struct AgentConnectionOption {
    pub id: Uuid,
    pub label: String,
    pub host: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentRemoteFileEntry {
    pub path: String,
    pub name: String,
    pub is_dir: bool,
}

/// Project/worktree data supplied by the workspace catalog. Sessions are
/// associated by their execution-context workdir, so the UI has no duplicate
/// checkout identity model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentProjectGroup {
    pub id: String,
    pub label: String,
    pub worktrees: Vec<AgentWorktreeOption>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentWorktreeOption {
    pub id: String,
    pub label: String,
    pub path: String,
    pub target: AgentTarget,
    pub branch: Option<String>,
    pub is_primary: bool,
}

struct AgentSessionUi {
    prompt: Entity<InputState>,
    run_id: Option<Uuid>,
    received_delta: bool,
    received_output: bool,
}

impl AgentSessionUi {
    fn new(cx: &mut Context<AgentConsoleView>) -> Self {
        Self {
            prompt: cx.new(|cx| InputState::new(cx).multi_line(true)),
            run_id: None,
            received_delta: false,
            received_output: false,
        }
    }
}

struct PendingRun {
    session_id: Uuid,
    context: AgentExecutionContext,
    prompt: String,
}

#[derive(Clone)]
enum TimelineItem {
    Message(AgentMessage),
    Trace(AgentTraceEvent),
}

impl TimelineItem {
    fn sequence(&self) -> u64 {
        match self {
            Self::Message(message) => message.sequence,
            Self::Trace(trace) => trace.sequence,
        }
    }

    fn at_ms(&self) -> i64 {
        match self {
            Self::Message(message) => message.at_ms,
            Self::Trace(trace) => trace.at_ms,
        }
    }
}

pub struct AgentConsoleView {
    provider: AgentProvider,
    access: AgentAccessMode,
    connections: Vec<AgentConnectionOption>,
    selected_connection: Option<Uuid>,
    provider_select: Entity<Select<AgentProvider>>,
    access_select: Entity<Select<AgentAccessMode>>,
    target_select: Entity<Select<Option<Uuid>>>,
    default_local_workdir: String,
    workdir_state: Entity<InputState>,
    model_state: Entity<InputState>,
    navigator_search: Entity<InputState>,
    sessions: AgentSessionCollection,
    session_ui: HashMap<Uuid, AgentSessionUi>,
    run_sessions: HashMap<Uuid, Uuid>,
    project_groups: Vec<AgentProjectGroup>,
    context_locks: HashSet<Uuid>,
    pending_confirm: Option<PendingRun>,
    context_expanded: bool,
    inspector_tab: AgentInspectorTab,
    focused_trace: Option<Uuid>,
    inspector_local_browser: FileBrowserPanel,
    inspector_remote_path: Option<String>,
    inspector_remote_pending_path: Option<String>,
    inspector_remote_session: Option<Uuid>,
    inspector_remote_connection: Option<Uuid>,
    inspector_remote_entries: Vec<AgentRemoteFileEntry>,
    inspector_remote_loading: bool,
    inspector_remote_error: Option<String>,
    scroll: ScrollHandle,
}

fn input_state(cx: &mut Context<AgentConsoleView>, initial: String) -> Entity<InputState> {
    cx.new(|cx| {
        let mut state = InputState::new(cx);
        state.content = initial.into();
        state
    })
}

impl AgentConsoleView {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let parent = cx.entity();
        let provider_select =
            Self::build_provider_select(AgentProvider::Claude, false, parent.clone(), cx);
        let access_select =
            Self::build_access_select(AgentAccessMode::ReadOnly, false, parent.clone(), cx);
        let target_select = Self::build_target_select(&[], None, false, parent, cx);
        let default_workdir = std::env::current_dir()
            .ok()
            .map(|path| path.display().to_string())
            .filter(|path| !path.is_empty())
            .unwrap_or_else(|| "/tmp".to_string());
        let context = AgentExecutionContext {
            provider: AgentProvider::Claude,
            target: AgentTarget::Local,
            access: AgentAccessMode::ReadOnly,
            workdir: default_workdir.clone(),
            model: None,
        };
        let mut sessions = AgentSessionCollection::default();
        let session_id = sessions
            .create(t!("agents.session.new").to_string(), context, now_ms())
            .expect("default agent execution context is valid");
        let mut session_ui = HashMap::new();
        session_ui.insert(session_id, AgentSessionUi::new(cx));
        let mut inspector_local_browser = FileBrowserPanel::new();
        inspector_local_browser.set_root(PathBuf::from(&default_workdir));
        Self {
            provider: AgentProvider::Claude,
            access: AgentAccessMode::ReadOnly,
            connections: Vec::new(),
            selected_connection: None,
            provider_select,
            access_select,
            target_select,
            default_local_workdir: default_workdir.clone(),
            workdir_state: input_state(cx, default_workdir),
            model_state: input_state(cx, String::new()),
            navigator_search: input_state(cx, String::new()),
            sessions,
            session_ui,
            run_sessions: HashMap::new(),
            project_groups: Vec::new(),
            context_locks: HashSet::new(),
            pending_confirm: None,
            context_expanded: false,
            inspector_tab: AgentInspectorTab::Files,
            focused_trace: None,
            inspector_local_browser,
            inspector_remote_path: None,
            inspector_remote_pending_path: None,
            inspector_remote_session: None,
            inspector_remote_connection: None,
            inspector_remote_entries: Vec::new(),
            inspector_remote_loading: false,
            inspector_remote_error: None,
            scroll: ScrollHandle::new(),
        }
    }

    fn build_provider_select(
        selected: AgentProvider,
        disabled: bool,
        parent: Entity<Self>,
        cx: &mut Context<Self>,
    ) -> Entity<Select<AgentProvider>> {
        let options = vec![
            SelectOption::new(AgentProvider::Claude, "Claude Code")
                .with_icon("icons/simple/claudecode.svg"),
            SelectOption::new(AgentProvider::Codex, "Codex").with_icon("icons/simple/openai.svg"),
            SelectOption::new(AgentProvider::Automonique, "Automonique ACP")
                .with_icon("icons/lucide/bot.svg"),
            SelectOption::new(AgentProvider::Jcode, "Jcode").with_icon("icons/lucide/sparkles.svg"),
            SelectOption::new(AgentProvider::DeepSeek, "DeepSeek")
                .with_icon("icons/lucide/bot.svg"),
        ];
        let selected_index = options.iter().position(|option| option.value == selected);
        cx.new(move |select_cx| {
            Select::new(select_cx)
                .options(options)
                .selected_index(selected_index)
                .disabled(disabled)
                .context_label(t!("agents.provider").to_string())
                .on_change(move |provider, window, cx| {
                    parent.update(cx, |this, cx| {
                        this.provider = *provider;
                        if *provider == AgentProvider::Automonique {
                            this.model_state.update(cx, |state, cx| {
                                state.set_value("", window, cx);
                            });
                        }
                        cx.notify();
                    });
                })
        })
    }

    fn build_access_select(
        selected: AgentAccessMode,
        disabled: bool,
        parent: Entity<Self>,
        cx: &mut Context<Self>,
    ) -> Entity<Select<AgentAccessMode>> {
        let options = vec![
            SelectOption::new(
                AgentAccessMode::ReadOnly,
                t!("agents.access.read_only").to_string(),
            )
            .with_icon("icons/lucide/lock.svg"),
            SelectOption::new(
                AgentAccessMode::WorkspaceWrite,
                t!("agents.access.workspace_write").to_string(),
            )
            .with_icon("icons/lucide/pencil.svg"),
            SelectOption::new(
                AgentAccessMode::FullAccess,
                t!("agents.access.full").to_string(),
            )
            .with_icon("icons/lucide/shield.svg"),
        ];
        let selected_index = options.iter().position(|option| option.value == selected);
        cx.new(move |select_cx| {
            Select::new(select_cx)
                .options(options)
                .selected_index(selected_index)
                .disabled(disabled)
                .context_label(t!("agents.access.label").to_string())
                .on_change(move |access, _window, cx| {
                    parent.update(cx, |this, cx| {
                        this.access = *access;
                        cx.notify();
                    });
                })
        })
    }

    fn build_target_select(
        connections: &[AgentConnectionOption],
        selected: Option<Uuid>,
        disabled: bool,
        parent: Entity<Self>,
        cx: &mut Context<Self>,
    ) -> Entity<Select<Option<Uuid>>> {
        let mut options = vec![
            SelectOption::new(None, t!("agents.target.local").to_string())
                .with_icon("icons/lucide/cpu.svg"),
        ];
        options.extend(connections.iter().map(|connection| {
            SelectOption::new(
                Some(connection.id),
                format!("{} — {}", connection.label, connection.host),
            )
            .with_icon("icons/lucide/server.svg")
        }));
        let selected_index = selected
            .and_then(|id| options.iter().position(|option| option.value == Some(id)))
            .or(Some(0));
        cx.new(move |select_cx| {
            Select::new(select_cx)
                .options(options)
                .selected_index(selected_index)
                .disabled(disabled)
                .searchable(true)
                .search_placeholder(t!("agents.target.search").to_string())
                .context_label(t!("agents.target.label").to_string())
                .on_change(move |connection_id, window, cx| {
                    parent.update(cx, |this, cx| {
                        if let Some(next_workdir) = target_switch_workdir(
                            this.selected_connection,
                            *connection_id,
                            this.workdir_state.read(cx).content(),
                            &this.default_local_workdir,
                        ) {
                            this.workdir_state.update(cx, |state, cx| {
                                state.set_value(next_workdir, window, cx);
                            });
                        }
                        this.selected_connection = *connection_id;
                        cx.notify();
                    });
                })
        })
    }

    pub fn set_connections(
        &mut self,
        connections: Vec<AgentConnectionOption>,
        cx: &mut Context<Self>,
    ) {
        if self
            .selected_connection
            .is_some_and(|id| !connections.iter().any(|connection| connection.id == id))
        {
            self.selected_connection = None;
        }
        self.connections = connections;
        let locked = self
            .sessions
            .selected_id()
            .is_some_and(|id| self.context_locks.contains(&id));
        self.target_select = Self::build_target_select(
            &self.connections,
            self.selected_connection,
            locked,
            cx.entity(),
            cx,
        );
        cx.notify();
    }

    pub fn set_project_groups(
        &mut self,
        project_groups: Vec<AgentProjectGroup>,
        cx: &mut Context<Self>,
    ) {
        self.project_groups = project_groups;
        cx.notify();
    }

    pub fn set_remote_directory_listing(
        &mut self,
        session_id: Uuid,
        connection_id: Uuid,
        requested_path: String,
        resolved_path: String,
        result: Result<Vec<AgentRemoteFileEntry>, String>,
        cx: &mut Context<Self>,
    ) {
        let selected_matches = self.sessions.selected().is_some_and(|session| {
            session.id == session_id
                && matches!(
                    session.context.target,
                    AgentTarget::Ssh {
                        connection_id: selected,
                        ..
                    } if selected == connection_id
                )
        });
        if !selected_matches
            || self.inspector_remote_connection != Some(connection_id)
            || self.inspector_remote_session != Some(session_id)
            || self.inspector_remote_pending_path.as_deref() != Some(requested_path.as_str())
        {
            return;
        }
        self.inspector_remote_loading = false;
        self.inspector_remote_pending_path = None;
        match result {
            Ok(entries) => {
                self.inspector_remote_path = Some(resolved_path);
                self.inspector_remote_entries = entries;
                self.inspector_remote_error = None;
            }
            Err(error) => {
                self.inspector_remote_entries.clear();
                self.inspector_remote_error = Some(bounded_utf8(error, MAX_ACTIVITY_BYTES));
            }
        }
        cx.notify();
    }

    fn request_remote_directory(
        &mut self,
        session_id: Uuid,
        connection_id: Uuid,
        path: String,
        cx: &mut Context<Self>,
    ) {
        self.inspector_remote_session = Some(session_id);
        self.inspector_remote_connection = Some(connection_id);
        self.inspector_remote_path = Some(path.clone());
        self.inspector_remote_pending_path = Some(path.clone());
        self.inspector_remote_loading = true;
        self.inspector_remote_error = None;
        cx.emit(AgentConsoleEvent::BrowseRemoteDirectory {
            session_id,
            connection_id,
            path,
        });
        cx.notify();
    }

    pub fn sessions(&self) -> &[AgentSession] {
        self.sessions.sessions()
    }

    pub fn selected_session_id(&self) -> Option<Uuid> {
        self.sessions.selected_id()
    }

    pub fn session_id_for_run(&self, run_id: Uuid) -> Option<Uuid> {
        self.run_sessions.get(&run_id).copied()
    }

    pub fn set_session_context_locked(
        &mut self,
        session_id: Uuid,
        locked: bool,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.sessions.get(session_id).is_none() {
            return false;
        }
        if locked {
            self.context_locks.insert(session_id);
        } else {
            self.context_locks.remove(&session_id);
        }
        if self.sessions.selected_id() == Some(session_id) {
            self.apply_selected_context(cx);
        }
        cx.notify();
        true
    }

    pub fn select_session(&mut self, session_id: Uuid, cx: &mut Context<Self>) -> bool {
        if self.sessions.select(session_id).is_err() {
            return false;
        }
        self.apply_selected_context(cx);
        pin_to_latest(&self.scroll);
        cx.notify();
        true
    }

    pub fn create_session(
        &mut self,
        name: impl Into<String>,
        context: AgentExecutionContext,
        cx: &mut Context<Self>,
    ) -> Result<Uuid, String> {
        let session_id = self
            .sessions
            .create(name, context, now_ms())
            .map_err(|error| error.to_string())?;
        self.session_ui.insert(session_id, AgentSessionUi::new(cx));
        self.apply_selected_context(cx);
        pin_to_latest(&self.scroll);
        cx.notify();
        Ok(session_id)
    }

    pub fn begin_run(&mut self, request: AgentRunRequest, cx: &mut Context<Self>) {
        let session_id = self
            .run_sessions
            .get(&request.id)
            .copied()
            .or_else(|| self.sessions.selected_id());
        let Some(session_id) = session_id else { return };
        self.run_sessions.insert(request.id, session_id);
        if let Some(ui) = self.session_ui.get_mut(&session_id) {
            ui.run_id = Some(request.id);
            ui.received_delta = false;
            ui.received_output = false;
            ui.prompt.update(cx, |state, cx| state.reset(cx));
        }
        cx.notify();
    }

    pub fn set_surface_visible(&mut self, visible: bool, cx: &mut Context<Self>) {
        if self.sessions.set_surface_visible(visible) {
            cx.notify();
        }
    }

    pub fn reject_run(&mut self, error: String, cx: &mut Context<Self>) {
        let run_id = self
            .sessions
            .selected_id()
            .and_then(|session_id| self.session_ui.get(&session_id)?.run_id)
            .or_else(|| self.run_sessions.keys().next().copied());
        if let Some(run_id) = run_id {
            self.reject_run_for(run_id, error, cx);
        }
    }

    pub fn reject_run_for(&mut self, run_id: Uuid, error: String, cx: &mut Context<Self>) {
        let Some(session_id) = self.run_sessions.remove(&run_id) else {
            return;
        };
        let _ = self.sessions.apply_stream_event(
            session_id,
            AgentStreamEvent::Error(bounded_utf8(error, MAX_ACTIVITY_BYTES)),
            now_ms(),
        );
        let _ = self
            .sessions
            .finish(session_id, AgentSessionStatus::Failed, now_ms());
        if let Some(ui) = self.session_ui.get_mut(&session_id) {
            ui.run_id = None;
        }
        cx.notify();
    }

    /// Compatibility route for the previous singleton workspace runtime.
    /// Parallel runtimes must call `push_stream_event_for`.
    pub fn push_stream_event(&mut self, event: AgentStreamEvent, cx: &mut Context<Self>) {
        if let Some(run_id) = self.run_sessions.keys().next().copied() {
            self.push_stream_event_for(run_id, event, cx);
        }
    }

    pub fn push_stream_event_for(
        &mut self,
        run_id: Uuid,
        event: AgentStreamEvent,
        cx: &mut Context<Self>,
    ) {
        let Some(session_id) = self.run_sessions.get(&run_id).copied() else {
            return;
        };
        follow_latest_if_at_end(&self.scroll);
        let suppress_complete_jcode = matches!(event, AgentStreamEvent::Text(_))
            && self
                .session_ui
                .get(&session_id)
                .is_some_and(|ui| ui.received_delta)
            && self.sessions.get(session_id).is_some_and(|session| {
                matches!(
                    session.context.provider,
                    AgentProvider::Jcode | AgentProvider::DeepSeek
                )
            });
        if suppress_complete_jcode {
            return;
        }
        if let Some(ui) = self.session_ui.get_mut(&session_id) {
            match &event {
                AgentStreamEvent::Text(text) | AgentStreamEvent::TextDelta(text) => {
                    if !text.is_empty() {
                        ui.received_output = true;
                    }
                    if matches!(event, AgentStreamEvent::TextDelta(_)) {
                        ui.received_delta = true;
                    }
                }
                AgentStreamEvent::Error(_) => ui.received_output = true,
                _ => {}
            }
        }
        let _ = self
            .sessions
            .apply_stream_event(session_id, event, now_ms());
        cx.notify();
    }

    pub fn finish_run(
        &mut self,
        run_id: Uuid,
        exit_code: Option<i32>,
        error: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let Some(session_id) = self.run_sessions.remove(&run_id) else {
            return;
        };
        let error_present = error.is_some();
        let error_label = bounded_utf8(error.clone().unwrap_or_default(), MAX_ACTIVITY_BYTES);
        if let Some(error) = error {
            let _ = self.sessions.apply_stream_event(
                session_id,
                AgentStreamEvent::Error(bounded_utf8(error, MAX_ACTIVITY_BYTES)),
                now_ms(),
            );
        }
        let status = terminal_session_status(exit_code, error_present);
        let activity = match (exit_code, error_present) {
            (Some(0), false) => t!("agents.status.completed").to_string(),
            (Some(code), _) => t!("agents.status.exit", code = code).to_string(),
            (None, true) => t!("agents.toast.failed", error = error_label).to_string(),
            (None, false) => t!("agents.status.stopped").to_string(),
        };
        let _ = self.sessions.apply_stream_event(
            session_id,
            AgentStreamEvent::Activity(activity),
            now_ms(),
        );
        let _ = self.sessions.finish(session_id, status, now_ms());
        if let Some(ui) = self.session_ui.get_mut(&session_id) {
            ui.run_id = None;
        }
        follow_latest_if_at_end(&self.scroll);
        cx.notify();
    }

    fn apply_selected_context(&mut self, cx: &mut Context<Self>) {
        let Some(session) = self.sessions.selected() else {
            return;
        };
        self.provider = session.context.provider;
        self.access = session.context.access;
        self.selected_connection = match &session.context.target {
            AgentTarget::Local => None,
            AgentTarget::Ssh { connection_id, .. } => Some(*connection_id),
        };
        let locked = self.context_locks.contains(&session.id);
        self.provider_select = Self::build_provider_select(self.provider, locked, cx.entity(), cx);
        self.access_select = Self::build_access_select(self.access, locked, cx.entity(), cx);
        self.target_select = Self::build_target_select(
            &self.connections,
            self.selected_connection,
            locked,
            cx.entity(),
            cx,
        );
        self.workdir_state = input_state(cx, session.context.workdir.clone());
        self.model_state = input_state(cx, session.context.model.clone().unwrap_or_default());
        match &session.context.target {
            AgentTarget::Local => {
                let root = PathBuf::from(&session.context.workdir);
                if root.is_dir() && self.inspector_local_browser.root() != root.as_path() {
                    self.inspector_local_browser.set_root(root);
                }
                self.inspector_remote_connection = None;
                self.inspector_remote_path = None;
                self.inspector_remote_pending_path = None;
                self.inspector_remote_session = None;
                self.inspector_remote_entries.clear();
                self.inspector_remote_error = None;
                self.inspector_remote_loading = false;
            }
            AgentTarget::Ssh { connection_id, .. } => {
                if self.inspector_remote_session != Some(session.id)
                    || self.inspector_remote_connection != Some(*connection_id)
                {
                    self.inspector_remote_session = Some(session.id);
                    self.inspector_remote_connection = Some(*connection_id);
                    self.inspector_remote_path = None;
                    self.inspector_remote_pending_path = None;
                    self.inspector_remote_entries.clear();
                    self.inspector_remote_error = None;
                    self.inspector_remote_loading = false;
                }
            }
        }
    }

    fn context_from_controls(&self, cx: &Context<Self>) -> Result<AgentExecutionContext, String> {
        if let Some(session_id) = self.sessions.selected_id() {
            if self.context_locks.contains(&session_id) {
                return self
                    .sessions
                    .get(session_id)
                    .map(|session| session.context.clone())
                    .ok_or_else(|| t!("agents.error.runtime_lost").to_string());
            }
        }
        let workdir = self.workdir_state.read(cx).content().trim().to_string();
        let model = self.model_state.read(cx).content().trim().to_string();
        if !workdir_is_valid_for_target(&workdir, self.selected_connection.is_some()) {
            return Err(t!("agents.error.workdir_absolute").to_string());
        }
        if workdir.len() > MAX_AGENT_WORKDIR_BYTES {
            return Err(t!("agents.error.workdir_too_long").to_string());
        }
        if model.len() > MAX_AGENT_MODEL_BYTES {
            return Err(t!("agents.error.model_too_long").to_string());
        }
        let target = match self.selected_connection {
            None => AgentTarget::Local,
            Some(connection_id) => {
                let connection = self
                    .connections
                    .iter()
                    .find(|connection| connection.id == connection_id)
                    .ok_or_else(|| t!("agents.error.target_missing").to_string())?;
                AgentTarget::Ssh {
                    connection_id,
                    connection_label: connection.label.clone(),
                }
            }
        };
        let context = AgentExecutionContext {
            provider: self.provider,
            target,
            access: self.access,
            workdir,
            model: (self.provider != AgentProvider::Automonique && !model.is_empty())
                .then_some(model),
        };
        context
            .run_request("validate")
            .validate()
            .map_err(|error| error.to_string())?;
        Ok(context)
    }

    fn pending_from_draft(&self, cx: &Context<Self>) -> Result<PendingRun, String> {
        let session_id = self
            .sessions
            .selected_id()
            .ok_or_else(|| t!("agents.error.prompt_empty").to_string())?;
        let prompt = self
            .session_ui
            .get(&session_id)
            .expect("selected session has UI state")
            .prompt
            .read(cx)
            .content()
            .trim()
            .to_string();
        if prompt.is_empty() {
            return Err(t!("agents.error.prompt_empty").to_string());
        }
        if prompt.len() > MAX_AGENT_PROMPT_BYTES {
            return Err(t!("agents.error.prompt_too_long").to_string());
        }
        Ok(PendingRun {
            session_id,
            context: self.context_from_controls(cx)?,
            prompt,
        })
    }

    fn submit(&mut self, cx: &mut Context<Self>) {
        let selected_active = self
            .sessions
            .selected()
            .is_some_and(|session| session.status.is_active());
        if selected_active {
            return;
        }
        match self.pending_from_draft(cx) {
            Ok(pending) if pending.context.access == AgentAccessMode::ReadOnly => {
                self.start_pending(pending, cx)
            }
            Ok(pending) => {
                self.pending_confirm = Some(pending);
                cx.notify();
            }
            Err(error) => self.apply_draft_error(error, cx),
        }
    }

    fn start_pending(&mut self, pending: PendingRun, cx: &mut Context<Self>) {
        let session_id = pending.session_id;
        let Some(session) = self.sessions.get_mut(session_id) else {
            self.apply_draft_error(t!("agents.error.runtime_lost").to_string(), cx);
            return;
        };
        if let Err(error) = session.set_context(pending.context, now_ms()) {
            self.apply_draft_error(error.to_string(), cx);
            return;
        }
        if session.messages.is_empty() {
            let _ = session.rename(session_name(&pending.prompt), now_ms());
        }
        match self
            .sessions
            .begin_run(session_id, pending.prompt, now_ms())
        {
            Ok(request) => {
                self.run_sessions.insert(request.id, session_id);
                if let Some(ui) = self.session_ui.get_mut(&session_id) {
                    ui.run_id = Some(request.id);
                    ui.received_delta = false;
                    ui.received_output = false;
                }
                cx.emit(AgentConsoleEvent::Run(request));
            }
            Err(error) => self.apply_draft_error(error.to_string(), cx),
        }
    }

    fn apply_draft_error(&mut self, error: String, cx: &mut Context<Self>) {
        if let Some(session_id) = self.sessions.selected_id() {
            let _ = self.sessions.apply_stream_event(
                session_id,
                AgentStreamEvent::Error(error),
                now_ms(),
            );
        }
        cx.notify();
    }

    fn new_session(&mut self, cx: &mut Context<Self>) {
        let Ok(context) = self.context_from_controls(cx) else {
            return;
        };
        let _ = self.create_session(t!("agents.session.new").to_string(), context, cx);
    }

    pub fn select_worktree_session(
        &mut self,
        worktree: &AgentWorktreeOption,
        cx: &mut Context<Self>,
    ) -> Option<Uuid> {
        if let Some(session_id) = worktree_bound_session(self.sessions.sessions(), worktree) {
            self.select_session(session_id, cx);
            return Some(session_id);
        }
        let base = self.sessions.selected()?.context.clone();
        let context = context_for_worktree(&base, worktree);
        self.create_session(worktree.label.clone(), context, cx)
            .ok()
    }

    fn request_stop(&mut self, run_id: Uuid, cx: &mut Context<Self>) {
        let Some(session_id) = self.run_sessions.get(&run_id).copied() else {
            return;
        };
        let Some(session) = self.sessions.get_mut(session_id) else {
            return;
        };
        if session.request_stop(now_ms()) {
            cx.emit(AgentConsoleEvent::Stop(run_id));
            cx.notify();
        }
    }

    fn close_session(&mut self, session_id: Uuid, cx: &mut Context<Self>) {
        if self.sessions.sessions().len() <= 1 {
            return;
        }
        if self.sessions.remove(session_id).is_ok() {
            self.session_ui.remove(&session_id);
            self.context_locks.remove(&session_id);
            self.apply_selected_context(cx);
            cx.emit(AgentConsoleEvent::CloseSession(session_id));
            cx.notify();
        }
    }

    fn acknowledge_selected_attention(&mut self, cx: &mut Context<Self>) {
        let Some(session_id) = self.sessions.selected_id() else {
            return;
        };
        if let Some(session) = self.sessions.get_mut(session_id) {
            session.acknowledge_attention();
            cx.notify();
        }
    }

    fn render_confirmation(&self, pending: &PendingRun, cx: &mut Context<Self>) -> AnyElement {
        let target = target_label(&pending.context.target);
        let access = access_label(pending.context.access);
        let destructive = pending.context.access == AgentAccessMode::FullAccess;
        UiDialog::new()
            .width(gpui::px(520.0))
            .header(
                div()
                    .flex()
                    .items_center()
                    .gap(px(9.0))
                    .px(px(16.0))
                    .py(px(14.0))
                    .border_b_1()
                    .border_color(ShellDeckColors::border())
                    .child(lucide_icon(
                        "shield-check",
                        17.0,
                        if destructive {
                            ShellDeckColors::error()
                        } else {
                            ShellDeckColors::warning()
                        },
                    ))
                    .child(
                        div()
                            .text_size(px(15.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(t!("agents.confirm.title").to_string()),
                    ),
            )
            .content(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(10.0))
                    .px(px(16.0))
                    .py(px(14.0))
                    .text_size(px(12.0))
                    .child(t!("agents.confirm.description").to_string())
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .gap(px(6.0))
                            .child(
                                Badge::new(pending.context.provider.display_name())
                                    .variant(BadgeVariant::Outline),
                            )
                            .child(Badge::new(target).variant(BadgeVariant::Outline))
                            .child(Badge::new(access).variant(BadgeVariant::Outline)),
                    )
                    .child(
                        div()
                            .rounded(px(7.0))
                            .bg(ShellDeckColors::bg_surface())
                            .overflow_hidden()
                            .px(px(10.0))
                            .py(px(8.0))
                            .font_family("JetBrains Mono")
                            .text_size(px(11.0))
                            .line_clamp(3)
                            .child(pending.context.workdir.clone()),
                    ),
            )
            .footer(
                div()
                    .flex()
                    .justify_end()
                    .gap(px(8.0))
                    .p(px(12.0))
                    .border_t_1()
                    .border_color(ShellDeckColors::border())
                    .child(
                        Button::new("agents-confirm-cancel", t!("scripts.cancel").to_string())
                            .variant(ButtonVariant::Ghost)
                            .size(ButtonSize::Sm)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.pending_confirm = None;
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("agents-confirm-run", t!("agents.run").to_string())
                            .variant(if destructive {
                                ButtonVariant::Destructive
                            } else {
                                ButtonVariant::Default
                            })
                            .size(ButtonSize::Sm)
                            .icon(IconSource::from("play"))
                            .on_click(cx.listener(|this, _, _, cx| {
                                if let Some(pending) = this.pending_confirm.take() {
                                    this.start_pending(pending, cx);
                                }
                            })),
                    ),
            )
            .on_backdrop_click({
                let parent = cx.entity();
                move |_, cx| {
                    parent.update(cx, |this, cx| {
                        this.pending_confirm = None;
                        cx.notify();
                    });
                }
            })
            .into_any_element()
    }

    fn render_model_popover(&self, cx: &mut Context<Self>) -> AnyElement {
        let locked = self
            .sessions
            .selected_id()
            .is_some_and(|id| self.context_locks.contains(&id));
        let configured = self.model_state.read(cx).content().trim().to_string();
        let display = if configured.is_empty() {
            t!("agents.model.auto").to_string()
        } else {
            configured
        };
        let trigger = div()
            .id("agent-model-trigger")
            .h(px(26.0))
            .max_w(px(180.0))
            .flex()
            .items_center()
            .gap(px(5.0))
            .px(px(6.0))
            .rounded(px(6.0))
            .cursor_pointer()
            .text_size(px(10.5))
            .text_color(ShellDeckColors::text_muted())
            .hover(|style| style.bg(ShellDeckColors::hover_bg()))
            .child(
                div()
                    .min_w(px(0.0))
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .child(display),
            )
            .child(lucide_icon(
                "chevron-down",
                11.0,
                ShellDeckColors::text_muted(),
            ));
        let model_state = self.model_state.clone();
        let parent = cx.entity();
        Popover::new("agent-model-popover")
            .anchor(Corner::BottomRight)
            .trigger(trigger)
            .content(move |window, cx| {
                let model_state = model_state.clone();
                let parent = parent.clone();
                cx.new(move |content_cx| {
                    PopoverContent::new(window, content_cx, move |_window, _cx| {
                        let notify_parent = parent.clone();
                        div()
                            .w(px(260.0))
                            .flex()
                            .flex_col()
                            .gap(px(7.0))
                            .child(
                                div()
                                    .text_size(px(11.0))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(t!("agents.model.label").to_string()),
                            )
                            .child(
                                Input::new(&model_state)
                                    .size(InputSize::Sm)
                                    .disabled(locked)
                                    .placeholder(t!("agents.model.auto").to_string())
                                    .on_change(move |_value, cx| {
                                        notify_parent.update(cx, |_this, cx| cx.notify());
                                    }),
                            )
                            .child(
                                div()
                                    .text_size(px(10.0))
                                    .text_color(ShellDeckColors::text_muted())
                                    .child(t!("agents.model.hint").to_string()),
                            )
                            .into_any_element()
                    })
                })
            })
            .into_any_element()
    }

    fn render_navigator(&self, cx: &mut Context<Self>) -> AnyElement {
        let parent = cx.entity();
        let running_count = self.sessions.active_count();
        let unread_count = self
            .sessions
            .sessions()
            .iter()
            .map(|session| u64::from(session.unread_count))
            .sum::<u64>();
        let mut body = div().flex().flex_col().gap(px(4.0)).px(px(6.0)).py(px(7.0));
        if self.project_groups.is_empty() {
            body = body.child(self.render_session_rows(None, cx));
        } else {
            for project in &self.project_groups {
                body = body.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.0))
                        .px(px(6.0))
                        .pt(px(6.0))
                        .pb(px(3.0))
                        .child(lucide_icon(
                            "git-branch",
                            12.0,
                            ShellDeckColors::text_muted(),
                        ))
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.0))
                                .overflow_hidden()
                                .text_ellipsis()
                                .text_size(px(10.5))
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(project.label.clone()),
                        ),
                );
                for worktree in &project.worktrees {
                    let worktree_option = worktree.clone();
                    let worktree_selected = self
                        .sessions
                        .selected()
                        .is_some_and(|session| session_worktree_matches(session, worktree));
                    body = body.child(
                        div()
                            .id(("agent-worktree", stable_string_key(&worktree.id)))
                            .flex()
                            .items_center()
                            .gap(px(5.0))
                            .px(px(11.0))
                            .py(px(3.0))
                            .rounded(px(4.0))
                            .cursor_pointer()
                            .when(worktree_selected, |row| {
                                row.bg(ShellDeckColors::selected_bg())
                            })
                            .when(!worktree_selected, |row| {
                                row.hover(|style| style.bg(ShellDeckColors::hover_bg()))
                            })
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.select_worktree_session(&worktree_option, cx);
                            }))
                            .text_size(px(9.5))
                            .text_color(ShellDeckColors::text_muted())
                            .child(lucide_icon(
                                "git-branch",
                                10.0,
                                ShellDeckColors::text_muted(),
                            ))
                            .child(worktree.label.clone())
                            .when_some(worktree.branch.clone(), |row, branch| {
                                row.child(
                                    div()
                                        .flex_1()
                                        .min_w(px(0.0))
                                        .overflow_hidden()
                                        .text_ellipsis()
                                        .font_family("JetBrains Mono")
                                        .text_size(px(8.5))
                                        .child(branch),
                                )
                            })
                            .when(worktree.is_primary, |row| {
                                row.child(
                                    div()
                                        .size(px(5.0))
                                        .rounded_full()
                                        .bg(ShellDeckColors::success()),
                                )
                            }),
                    );
                    body = body.child(self.render_session_rows(Some(worktree), cx));
                }
            }
            let known_worktrees = self
                .project_groups
                .iter()
                .flat_map(|project| project.worktrees.iter())
                .collect::<Vec<_>>();
            body = body.child(self.render_unmatched_session_rows(&known_worktrees, cx));
        }
        div()
            .w(px(NAVIGATOR_WIDTH))
            .h_full()
            .flex()
            .flex_col()
            .flex_shrink_0()
            .bg(ShellDeckColors::bg_sidebar())
            .border_r_1()
            .border_color(ShellDeckColors::border())
            .child(
                div()
                    .h(px(42.0))
                    .flex()
                    .items_center()
                    .gap(px(7.0))
                    .px(px(10.0))
                    .border_b_1()
                    .border_color(ShellDeckColors::border())
                    .child(lucide_icon("grid-2x2", 14.0, ShellDeckColors::primary()))
                    .child(
                        div()
                            .flex_1()
                            .text_size(px(11.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(t!("agents.title").to_string()),
                    )
                    .when(running_count > 0, |row| {
                        row.child(
                            Badge::new(running_count.to_string()).variant(BadgeVariant::Secondary),
                        )
                    })
                    .child(
                        Button::new("agent-nav-new", "")
                            .variant(ButtonVariant::Ghost)
                            .size(ButtonSize::Icon)
                            .icon(IconSource::from("plus"))
                            .w(px(26.0))
                            .h(px(26.0))
                            .tooltip(t!("agents.session.new").to_string())
                            .on_click(cx.listener(|this, _, _, cx| this.new_session(cx))),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .px(px(8.0))
                    .py(px(6.0))
                    .border_b_1()
                    .border_color(ShellDeckColors::border())
                    .child(lucide_icon("search", 11.0, ShellDeckColors::text_muted()))
                    .child(
                        div().flex_1().min_w(px(0.0)).child(
                            Input::new(&self.navigator_search)
                                .variant(InputVariant::Bare)
                                .size(InputSize::Sm)
                                .placeholder(t!("fleet.sessions.search").to_string())
                                .on_change(move |_value, cx| {
                                    parent.update(cx, |_this, cx| cx.notify());
                                }),
                        ),
                    ),
            )
            .child(
                div()
                    .id("agent-navigator-scroll")
                    .flex_1()
                    .min_h(px(0.0))
                    .overflow_y_scroll()
                    .child(body),
            )
            .child(
                div()
                    .h(px(30.0))
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .px(px(10.0))
                    .border_t_1()
                    .border_color(ShellDeckColors::border())
                    .text_size(px(9.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(lucide_icon("activity", 11.0, ShellDeckColors::warning()))
                    .child(format!(
                        "{running_count}/{DEFAULT_MAX_CONCURRENT_AGENT_SESSIONS}"
                    ))
                    .child(t!("workspaces.agent.running").to_string())
                    .child(div().flex_1())
                    .when(unread_count > 0, |row| {
                        row.child(lucide_icon("activity", 10.0, ShellDeckColors::primary()))
                            .child(unread_count.to_string())
                            .child(t!("workspaces.attention.unread").to_string())
                    }),
            )
            .into_any_element()
    }

    fn render_session_rows(
        &self,
        worktree: Option<&AgentWorktreeOption>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        self.render_session_rows_matching(
            |session| worktree.is_none_or(|worktree| session_worktree_matches(session, worktree)),
            cx,
        )
    }

    fn render_unmatched_session_rows(
        &self,
        known_worktrees: &[&AgentWorktreeOption],
        cx: &mut Context<Self>,
    ) -> AnyElement {
        self.render_session_rows_matching(
            |session| {
                !known_worktrees
                    .iter()
                    .any(|worktree| session_worktree_matches(session, worktree))
            },
            cx,
        )
    }

    fn render_session_rows_matching(
        &self,
        include: impl Fn(&AgentSession) -> bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let query = self
            .navigator_search
            .read(cx)
            .content()
            .trim()
            .to_ascii_lowercase();
        let mut list = div().flex().flex_col().gap(px(2.0));
        for session in self.sessions.sessions().iter().filter(|session| {
            include(session)
                && (query.is_empty()
                    || session.name.to_ascii_lowercase().contains(&query)
                    || session
                        .context
                        .workdir
                        .to_ascii_lowercase()
                        .contains(&query)
                    || session
                        .context
                        .provider
                        .display_name()
                        .to_ascii_lowercase()
                        .contains(&query))
        }) {
            let id = session.id;
            let selected = self.sessions.selected_id() == Some(id);
            let removable = self.sessions.sessions().len() > 1 && !session.status.is_active();
            let mut row = div()
                .id(("agent-nav-session", uuid_key(id)))
                .flex()
                .items_center()
                .gap(px(7.0))
                .h(px(33.0))
                .px(px(8.0))
                .rounded(px(5.0))
                .cursor_pointer()
                .when(selected, |row| row.bg(ShellDeckColors::selected_bg()))
                .when(!selected, |row| {
                    row.hover(|style| style.bg(ShellDeckColors::hover_bg()))
                })
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.select_session(id, cx);
                }))
                .child(
                    div()
                        .size(px(6.0))
                        .rounded_full()
                        .bg(status_color(session.status)),
                )
                .child(lucide_icon(
                    provider_icon(session.context.provider),
                    12.0,
                    ShellDeckColors::text_muted(),
                ))
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.0))
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .text_size(px(10.5))
                        .font_weight(if selected {
                            FontWeight::SEMIBOLD
                        } else {
                            FontWeight::NORMAL
                        })
                        .child(session.name.clone()),
                )
                .when_some(session_elapsed(session), |row, elapsed| {
                    row.child(
                        div()
                            .font_family("JetBrains Mono")
                            .text_size(px(8.5))
                            .text_color(ShellDeckColors::text_muted())
                            .child(elapsed),
                    )
                })
                .when(session.unread_count > 0, |row| {
                    row.child(
                        Badge::new(session.unread_count.to_string())
                            .variant(BadgeVariant::Secondary),
                    )
                });
            if removable {
                row = row.child(
                    div()
                        .id(("agent-close-session", uuid_key(id)))
                        .size(px(19.0))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(4.0))
                        .hover(|style| style.bg(ShellDeckColors::hover_bg()))
                        .on_click(cx.listener(move |this, _event: &ClickEvent, _, cx| {
                            cx.stop_propagation();
                            this.close_session(id, cx);
                        }))
                        .child(lucide_icon("x", 10.0, ShellDeckColors::text_muted())),
                );
            }
            list = list.child(row);
        }
        list.into_any_element()
    }

    fn render_tabs(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut tabs = div()
            .id("agent-session-tabs")
            .flex()
            .flex_1()
            .min_w(px(0.0))
            .h_full()
            .overflow_x_scroll();
        for session in self.sessions.sessions() {
            let id = session.id;
            let selected = self.sessions.selected_id() == Some(id);
            let removable = self.sessions.sessions().len() > 1 && !session.status.is_active();
            let mut tab = div()
                .id(("agent-session-tab", uuid_key(id)))
                .h_full()
                .max_w(px(220.0))
                .flex()
                .items_center()
                .gap(px(7.0))
                .px(px(11.0))
                .border_r_1()
                .border_color(ShellDeckColors::border())
                .cursor_pointer()
                .when(selected, |tab| {
                    tab.bg(ShellDeckColors::bg_primary())
                        .border_b_2()
                        .border_color(ShellDeckColors::primary())
                })
                .when(!selected, |tab| {
                    tab.hover(|style| style.bg(ShellDeckColors::hover_bg()))
                })
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.select_session(id, cx);
                }))
                .child(
                    div()
                        .size(px(6.0))
                        .rounded_full()
                        .bg(status_color(session.status)),
                )
                .child(lucide_icon(
                    provider_icon(session.context.provider),
                    12.0,
                    ShellDeckColors::text_muted(),
                ))
                .child(
                    div()
                        .min_w(px(0.0))
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .text_size(px(10.5))
                        .child(session.name.clone()),
                )
                .when(session.attention != AgentSessionAttention::None, |tab| {
                    tab.child(
                        div()
                            .size(px(5.0))
                            .rounded_full()
                            .bg(attention_color(session.attention)),
                    )
                });
            if removable {
                tab = tab.child(
                    div()
                        .id(("agent-close-tab", uuid_key(id)))
                        .size(px(18.0))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(4.0))
                        .hover(|style| style.bg(ShellDeckColors::hover_bg()))
                        .on_click(cx.listener(move |this, _event: &ClickEvent, _, cx| {
                            cx.stop_propagation();
                            this.close_session(id, cx);
                        }))
                        .child(lucide_icon("x", 10.0, ShellDeckColors::text_muted())),
                );
            }
            tabs = tabs.child(tab);
        }
        tabs.into_any_element()
    }

    fn render_trace(trace: &AgentTraceEvent, focused: bool) -> AnyElement {
        let (icon, color, title, detail, stats) = trace_presentation(&trace.detail);
        div()
            .id(("agent-trace", uuid_key(trace.id)))
            .w_full()
            .flex()
            .items_start()
            .gap(px(8.0))
            .px(px(9.0))
            .py(px(7.0))
            .border_1()
            .border_color(if focused {
                ShellDeckColors::primary()
            } else {
                ShellDeckColors::border()
            })
            .rounded(px(5.0))
            .bg(if focused {
                ShellDeckColors::primary().opacity(0.08)
            } else {
                ShellDeckColors::bg_surface()
            })
            .child(
                div()
                    .mt(px(1.0))
                    .size(px(20.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(4.0))
                    .bg(color.opacity(0.10))
                    .child(lucide_icon(icon, 11.0, color)),
            )
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .flex()
                    .flex_col()
                    .gap(px(2.0))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(7.0))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w(px(0.0))
                                    .overflow_hidden()
                                    .text_ellipsis()
                                    .text_size(px(10.0))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(color)
                                    .child(title),
                            )
                            .when_some(stats, |row, stats| {
                                row.child(
                                    div()
                                        .font_family("JetBrains Mono")
                                        .text_size(px(9.0))
                                        .text_color(ShellDeckColors::text_muted())
                                        .child(stats),
                                )
                            }),
                    )
                    .when_some(detail, |column, detail| {
                        column.child(
                            div()
                                .font_family("JetBrains Mono")
                                .text_size(px(10.0))
                                .line_height(relative(1.35))
                                .text_color(ShellDeckColors::text_muted())
                                .whitespace_normal()
                                .child(detail),
                        )
                    }),
            )
            .into_any_element()
    }

    fn render_inspector(&mut self, session: &AgentSession, cx: &mut Context<Self>) -> AnyElement {
        let remote = matches!(session.context.target, AgentTarget::Ssh { .. });
        let files = inspector_files(session);
        let changed_count = files.iter().filter(|file| file.diff.is_some()).count();
        let files_selected = self.inspector_tab == AgentInspectorTab::Files;
        let changes_selected = self.inspector_tab == AgentInspectorTab::Changes;
        let remote_parent = remote
            .then(|| {
                self.inspector_remote_path
                    .as_deref()
                    .unwrap_or(&session.context.workdir)
            })
            .and_then(remote_parent_path);

        let tabs = div()
            .h(px(42.0))
            .flex()
            .items_center()
            .gap(px(4.0))
            .px(px(8.0))
            .border_b_1()
            .border_color(ShellDeckColors::border())
            .child(
                Button::new(
                    "agent-inspector-files",
                    t!("agents.inspector.files").to_string(),
                )
                .variant(if files_selected {
                    ButtonVariant::Secondary
                } else {
                    ButtonVariant::Ghost
                })
                .size(ButtonSize::Sm)
                .icon(IconSource::from("file-text"))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.inspector_tab = AgentInspectorTab::Files;
                    cx.notify();
                })),
            )
            .child(
                Button::new(
                    "agent-inspector-changes",
                    t!("agents.inspector.changes").to_string(),
                )
                .variant(if changes_selected {
                    ButtonVariant::Secondary
                } else {
                    ButtonVariant::Ghost
                })
                .size(ButtonSize::Sm)
                .icon(IconSource::from("git-branch"))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.inspector_tab = AgentInspectorTab::Changes;
                    cx.notify();
                })),
            );

        let mut list = div()
            .id("agent-inspector-list")
            .flex()
            .flex_col()
            .flex_1()
            .min_h(px(0.0))
            .overflow_y_scroll()
            .px(px(7.0))
            .py(px(6.0))
            .gap(px(4.0));
        if files_selected && !remote {
            let entries = self.inspector_local_browser.visible_entries();
            if entries.is_empty() {
                list = list.child(inspector_empty_state(
                    "file-text",
                    t!("agents.inspector.empty_files").to_string(),
                ));
            } else {
                for entry in entries {
                    let path = entry.path.clone();
                    let event_path = path.clone();
                    let is_dir = entry.is_dir;
                    let session_id = session.id;
                    let icon = if is_dir {
                        if entry.is_expanded {
                            "chevron-down"
                        } else {
                            "chevron-right"
                        }
                    } else {
                        "file-text"
                    };
                    list = list.child(
                        div()
                            .id(SharedString::from(format!("agent-file-{}", path.display())))
                            .h(px(25.0))
                            .w_full()
                            .flex()
                            .items_center()
                            .gap(px(5.0))
                            .pl(px(6.0 + entry.depth as f32 * 12.0))
                            .pr(px(6.0))
                            .rounded(px(4.0))
                            .cursor_pointer()
                            .hover(|style| style.bg(ShellDeckColors::hover_bg()))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if is_dir {
                                    this.inspector_local_browser.toggle_dir(&path);
                                    cx.notify();
                                } else {
                                    cx.emit(AgentConsoleEvent::OpenLocalFile {
                                        session_id,
                                        path: event_path.clone(),
                                    });
                                }
                            }))
                            .child(lucide_icon(icon, 11.0, ShellDeckColors::text_muted()))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w(px(0.0))
                                    .overflow_hidden()
                                    .whitespace_nowrap()
                                    .text_ellipsis()
                                    .text_size(px(10.0))
                                    .text_color(if is_dir {
                                        ShellDeckColors::text_primary()
                                    } else {
                                        ShellDeckColors::text_muted()
                                    })
                                    .child(entry.name),
                            ),
                    );
                }
            }
        } else if files_selected {
            let (session_id, connection_id) = match &session.context.target {
                AgentTarget::Ssh { connection_id, .. } => (session.id, *connection_id),
                AgentTarget::Local => unreachable!("local files use the local browser"),
            };
            let request_path = self
                .inspector_remote_path
                .clone()
                .unwrap_or_else(|| session.context.workdir.clone());
            if self.inspector_remote_loading {
                list = list.child(inspector_empty_state(
                    "refresh-cw",
                    t!("agents.inspector.remote_loading").to_string(),
                ));
            } else if let Some(error) = self.inspector_remote_error.clone() {
                let retry_path = request_path.clone();
                list = list
                    .child(inspector_empty_state("circle-alert", error))
                    .child(
                        Button::new(
                            "agent-remote-files-retry",
                            t!("agents.inspector.retry").to_string(),
                        )
                        .variant(ButtonVariant::Secondary)
                        .size(ButtonSize::Sm)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.request_remote_directory(
                                session_id,
                                connection_id,
                                retry_path.clone(),
                                cx,
                            );
                        })),
                    );
            } else if self.inspector_remote_entries.is_empty() {
                let load_path = request_path.clone();
                list = list
                    .child(inspector_empty_state(
                        "server",
                        t!("agents.inspector.remote_empty").to_string(),
                    ))
                    .child(
                        Button::new(
                            "agent-remote-files-load",
                            t!("agents.inspector.remote_load").to_string(),
                        )
                        .variant(ButtonVariant::Secondary)
                        .size(ButtonSize::Sm)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.request_remote_directory(
                                session_id,
                                connection_id,
                                load_path.clone(),
                                cx,
                            );
                        })),
                    );
            } else {
                for entry in self.inspector_remote_entries.clone() {
                    let path = entry.path.clone();
                    let is_dir = entry.is_dir;
                    let mut row = div()
                        .id(SharedString::from(format!(
                            "agent-remote-file-{}",
                            entry.path
                        )))
                        .h(px(25.0))
                        .w_full()
                        .flex()
                        .items_center()
                        .gap(px(6.0))
                        .px(px(6.0))
                        .rounded(px(4.0))
                        .text_size(px(10.0))
                        .text_color(if is_dir {
                            ShellDeckColors::text_primary()
                        } else {
                            ShellDeckColors::text_muted()
                        })
                        .child(lucide_icon(
                            if is_dir { "archive" } else { "file-text" },
                            11.0,
                            ShellDeckColors::text_muted(),
                        ))
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.0))
                                .overflow_hidden()
                                .whitespace_nowrap()
                                .text_ellipsis()
                                .child(entry.name),
                        );
                    if is_dir {
                        row = row
                            .cursor_pointer()
                            .hover(|style| style.bg(ShellDeckColors::hover_bg()))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.request_remote_directory(
                                    session_id,
                                    connection_id,
                                    path.clone(),
                                    cx,
                                );
                            }));
                    } else {
                        row = row.child(
                            Badge::new(t!("agents.inspector.remote").to_string())
                                .variant(BadgeVariant::Secondary),
                        );
                    }
                    list = list.child(row);
                }
            }
        } else {
            let visible = files
                .iter()
                .filter(|file| file.diff.is_some())
                .collect::<Vec<_>>();
            if visible.is_empty() {
                list = list.child(inspector_empty_state(
                    "git-branch",
                    t!("agents.inspector.empty_changes").to_string(),
                ));
            }
            for file in visible {
                let diff = file.diff.as_ref().expect("changed file has a diff");
                let trace_id = diff.trace_id;
                let selected = self.focused_trace == Some(trace_id);
                let mut row = div()
                    .id(("agent-inspector-trace", uuid_key(trace_id)))
                    .w_full()
                    .flex()
                    .flex_col()
                    .gap(px(4.0))
                    .px(px(8.0))
                    .py(px(7.0))
                    .overflow_hidden()
                    .border_1()
                    .border_color(if selected {
                        ShellDeckColors::primary()
                    } else {
                        ShellDeckColors::border()
                    })
                    .rounded(px(5.0))
                    .bg(if selected {
                        ShellDeckColors::primary().opacity(0.08)
                    } else {
                        ShellDeckColors::bg_surface()
                    })
                    .cursor_pointer()
                    .hover(|style| style.bg(ShellDeckColors::hover_bg()))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.focused_trace = Some(trace_id);
                        pin_to_latest(&this.scroll);
                        cx.notify();
                    }))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.0))
                            .child(lucide_icon("git-branch", 12.0, ShellDeckColors::warning()))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w(px(0.0))
                                    .overflow_hidden()
                                    .whitespace_nowrap()
                                    .text_ellipsis()
                                    .font_family("JetBrains Mono")
                                    .text_size(px(9.5))
                                    .text_color(ShellDeckColors::text_primary())
                                    .child(file.path.clone()),
                            )
                            .child(
                                div()
                                    .flex_shrink_0()
                                    .font_family("JetBrains Mono")
                                    .text_size(px(8.5))
                                    .text_color(ShellDeckColors::text_muted())
                                    .child(format!("+{} −{}", diff.additions, diff.deletions)),
                            ),
                    );
                if let Some(preview) = diff.preview.as_ref() {
                    row = row.child(
                        div()
                            .max_h(px(88.0))
                            .overflow_hidden()
                            .font_family("JetBrains Mono")
                            .text_size(px(8.5))
                            .line_height(relative(1.3))
                            .text_color(ShellDeckColors::text_muted())
                            .whitespace_normal()
                            .child(preview.clone()),
                    );
                }
                list = list.child(row);
            }
        }

        div()
            .w(px(INSPECTOR_WIDTH))
            .h_full()
            .flex()
            .flex_col()
            .flex_shrink_0()
            .border_l_1()
            .border_color(ShellDeckColors::border())
            .bg(ShellDeckColors::bg_sidebar())
            .child(tabs)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(4.0))
                    .px(px(10.0))
                    .py(px(8.0))
                    .border_b_1()
                    .border_color(ShellDeckColors::border())
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.0))
                            .when_some(remote_parent, |row, parent_path| {
                                let (session_id, connection_id) = match &session.context.target {
                                    AgentTarget::Ssh { connection_id, .. } => {
                                        (session.id, *connection_id)
                                    }
                                    AgentTarget::Local => {
                                        unreachable!("a remote parent requires an SSH target")
                                    }
                                };
                                row.child(
                                    Button::new("agent-remote-files-parent", "")
                                        .variant(ButtonVariant::Ghost)
                                        .size(ButtonSize::Icon)
                                        .icon(IconSource::from("arrow-up"))
                                        .tooltip(t!("agents.inspector.remote_parent").to_string())
                                        .w(px(24.0))
                                        .h(px(24.0))
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.request_remote_directory(
                                                session_id,
                                                connection_id,
                                                parent_path.clone(),
                                                cx,
                                            );
                                        })),
                                )
                            })
                            .child(
                                Badge::new(if remote {
                                    t!("agents.inspector.remote").to_string()
                                } else {
                                    t!("agents.inspector.local").to_string()
                                })
                                .variant(BadgeVariant::Secondary),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w(px(0.0))
                                    .overflow_hidden()
                                    .whitespace_nowrap()
                                    .text_ellipsis()
                                    .font_family("JetBrains Mono")
                                    .text_size(px(9.0))
                                    .text_color(ShellDeckColors::text_muted())
                                    .child(session.context.workdir.clone()),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(5.0))
                            .text_size(px(9.0))
                            .text_color(ShellDeckColors::text_muted())
                            .child(format!(
                                "{} · {} {}",
                                t!("agents.inspector.observed"),
                                files.len(),
                                t!("agents.inspector.files")
                            ))
                            .when(changed_count > 0, |row| {
                                row.child(format!(
                                    "· {changed_count} {}",
                                    t!("agents.inspector.changes")
                                ))
                            }),
                    ),
            )
            .child(list)
            .when(changes_selected, |panel| {
                panel.child(
                    div()
                        .flex_shrink_0()
                        .px(px(10.0))
                        .py(px(7.0))
                        .border_t_1()
                        .border_color(ShellDeckColors::border())
                        .text_size(px(8.5))
                        .text_color(ShellDeckColors::text_muted())
                        .child(t!("agents.inspector.focus_hint").to_string()),
                )
            })
            .into_any_element()
    }

    fn render_context(&self, narrow: bool, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let locked = self
            .sessions
            .selected_id()
            .is_some_and(|id| self.context_locks.contains(&id));
        let workdir_focused = self
            .workdir_state
            .read(cx)
            .focus_handle(cx)
            .is_focused(window);
        let provider = context_select_cell(self.provider_select.clone());
        let target = context_select_cell(self.target_select.clone());
        let access = context_select_cell(self.access_select.clone());
        let selects = if narrow {
            div()
                .flex()
                .flex_col()
                .child(provider)
                .child(target.border_t_1().border_color(ShellDeckColors::border()))
                .child(access.border_t_1().border_color(ShellDeckColors::border()))
        } else {
            div()
                .flex()
                .child(
                    provider
                        .border_r_1()
                        .border_color(ShellDeckColors::border()),
                )
                .child(target.border_r_1().border_color(ShellDeckColors::border()))
                .child(access)
        };
        div()
            .flex()
            .flex_col()
            .border_b_1()
            .border_color(ShellDeckColors::border())
            .bg(ShellDeckColors::bg_surface())
            .child(selects)
            .child(
                div()
                    .h(px(34.0))
                    .flex()
                    .items_center()
                    .gap(px(7.0))
                    .px(px(9.0))
                    .border_t_1()
                    .border_color(ShellDeckColors::border())
                    .child(lucide_icon("archive", 12.0, ShellDeckColors::text_muted()))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .h(px(28.0))
                            .overflow_hidden()
                            .border_1()
                            .border_color(if workdir_focused {
                                ShellDeckColors::primary()
                            } else {
                                gpui::transparent_black()
                            })
                            .rounded(px(5.0))
                            .child(
                                Input::new(&self.workdir_state)
                                    .variant(InputVariant::Bare)
                                    .size(InputSize::Sm)
                                    .disabled(locked)
                                    .placeholder("/srv/project"),
                            ),
                    ),
            )
            .into_any_element()
    }
}

impl Render for AgentConsoleView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let wide = window.viewport_size().width >= px(840.0).to_pixels(window.rem_size());
        let inspector_wide =
            window.viewport_size().width >= px(1180.0).to_pixels(window.rem_size());
        let session = self
            .sessions
            .selected()
            .cloned()
            .expect("cockpit retains one session");
        if matches!(session.context.target, AgentTarget::Local) {
            let root = PathBuf::from(&session.context.workdir);
            if root.is_dir() && self.inspector_local_browser.root() != root.as_path() {
                self.inspector_local_browser.set_root(root);
            }
        }
        let session_id = session.id;
        let session_name = session.name.clone();
        let context = session.context.clone();
        let status = session.status;
        let attention = session.attention;
        let elapsed = session_elapsed(&session);
        let timeline = merged_timeline(&session);
        let running = self.session_ui.get(&session_id).and_then(|ui| ui.run_id);
        let received_output = self
            .session_ui
            .get(&session_id)
            .is_some_and(|ui| ui.received_output);
        let prompt = self
            .session_ui
            .get(&session_id)
            .expect("selected session has UI state")
            .prompt
            .clone();
        let has_content = !timeline.is_empty();
        let context_expanded = self.context_expanded;

        let submit = {
            let parent = cx.entity();
            move |cx: &mut App| parent.update(cx, |this, cx| this.submit(cx))
        };
        let header = div()
            .h(px(42.0))
            .flex()
            .items_center()
            .flex_shrink_0()
            .bg(ShellDeckColors::bg_sidebar())
            .border_b_1()
            .border_color(ShellDeckColors::border())
            .child(self.render_tabs(cx))
            .child(
                Button::new("agent-add-tab", "")
                    .variant(ButtonVariant::Ghost)
                    .size(ButtonSize::Icon)
                    .icon(IconSource::from("plus"))
                    .w(px(30.0))
                    .h(px(30.0))
                    .tooltip(t!("agents.session.new").to_string())
                    .on_click(cx.listener(|this, _, _, cx| this.new_session(cx))),
            );
        let metadata = div()
            .h(if wide { px(42.0) } else { px(38.0) })
            .flex()
            .items_center()
            .gap(px(8.0))
            .px(px(12.0))
            .flex_shrink_0()
            .border_b_1()
            .border_color(ShellDeckColors::border())
            .child(lucide_icon(
                provider_icon(context.provider),
                14.0,
                ShellDeckColors::primary(),
            ))
            .when(!wide, |row| {
                row.child(
                    div()
                        .max_w(px(260.0))
                        .overflow_hidden()
                        .text_ellipsis()
                        .text_size(px(11.5))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(session_name),
                )
            })
            .when(wide, |row| {
                row.child(
                    div()
                        .text_size(px(11.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(context.provider.display_name()),
                )
            })
            .child(div().size(px(6.0)).rounded_full().bg(status_color(status)))
            .when_some(elapsed, |row, elapsed| {
                row.child(
                    div()
                        .font_family("JetBrains Mono")
                        .text_size(px(9.0))
                        .text_color(ShellDeckColors::text_muted())
                        .child(elapsed),
                )
            })
            .when(attention != AgentSessionAttention::None, |row| {
                row.child(
                    div()
                        .size(px(5.0))
                        .rounded_full()
                        .bg(attention_color(attention)),
                )
                .child(
                    Button::new("agent-acknowledge-attention", "")
                        .variant(ButtonVariant::Ghost)
                        .size(ButtonSize::Icon)
                        .icon(IconSource::from("check"))
                        .w(px(24.0))
                        .h(px(24.0))
                        .tooltip(t!("agents.attention.acknowledge").to_string())
                        .on_click(
                            cx.listener(|this, _, _, cx| this.acknowledge_selected_attention(cx)),
                        ),
                )
            })
            .child(div().flex_1())
            .when(wide, |row| {
                row.child(
                    Badge::new(target_label(&context.target)).variant(BadgeVariant::Secondary),
                )
                .child(
                    div()
                        .max_w(px(300.0))
                        .min_w(px(0.0))
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .font_family("JetBrains Mono")
                        .text_size(px(9.0))
                        .text_color(ShellDeckColors::text_muted())
                        .child(context.workdir.clone()),
                )
            })
            .when(!wide, |row| {
                row.child(
                    div()
                        .text_size(px(9.5))
                        .text_color(ShellDeckColors::text_muted())
                        .child(context.provider.display_name()),
                )
            })
            .child(
                Button::new("agent-context-toggle", "")
                    .variant(ButtonVariant::Ghost)
                    .size(ButtonSize::Icon)
                    .icon(IconSource::from(if context_expanded {
                        "panel-right-close"
                    } else {
                        "settings"
                    }))
                    .w(px(28.0))
                    .h(px(28.0))
                    .tooltip(t!("agents.context.title").to_string())
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.context_expanded = !this.context_expanded;
                        cx.notify();
                    })),
            );

        let mut output = div()
            .id(("agent-console-output", uuid_key(session_id)))
            .flex()
            .flex_col()
            .flex_1()
            .min_h(px(0.0))
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
            .px(if wide { px(24.0) } else { px(12.0) })
            .py(px(16.0));
        if !has_content {
            output = output.child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .flex_1()
                    .gap(px(8.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(lucide_icon("bot", 27.0, ShellDeckColors::primary()))
                    .child(
                        div()
                            .text_size(px(14.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(ShellDeckColors::text_primary())
                            .child(t!("agents.empty.title").to_string()),
                    )
                    .child(
                        div()
                            .max_w(px(520.0))
                            .text_size(px(11.0))
                            .text_align(TextAlign::Center)
                            .child(t!("agents.empty.description").to_string()),
                    ),
            );
        } else {
            let mut timeline_view = div()
                .w_full()
                .max_w(px(920.0))
                .mx_auto()
                .flex()
                .flex_col()
                .gap(px(8.0));
            for item in &timeline {
                match item {
                    TimelineItem::Message(message) => {
                        let color = match message.role {
                            AgentMessageRole::User => ShellDeckColors::text_muted(),
                            AgentMessageRole::Agent => ShellDeckColors::primary(),
                            AgentMessageRole::Error => ShellDeckColors::error(),
                        };
                        let label = match message.role {
                            AgentMessageRole::User => t!("agents.role.you").to_string(),
                            AgentMessageRole::Agent => context.provider.display_name().to_string(),
                            AgentMessageRole::Error => t!("agents.activity.title").to_string(),
                        };
                        timeline_view = timeline_view.child(
                            div()
                                .id(("agent-message", uuid_key(message.id)))
                                .w_full()
                                .min_w(px(0.0))
                                .overflow_hidden()
                                .py(px(5.0))
                                .when(message.role == AgentMessageRole::Error, |message| {
                                    message
                                        .px(px(9.0))
                                        .rounded(px(5.0))
                                        .bg(ShellDeckColors::error().opacity(0.10))
                                })
                                .child(
                                    div()
                                        .mb(px(4.0))
                                        .text_size(px(9.5))
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(color)
                                        .child(label),
                                )
                                .child(
                                    Markdown::new(message.text.clone())
                                        .base_font_size(px(12.0).to_pixels(window.rem_size()))
                                        .compact()
                                        .w_full()
                                        .min_w_0()
                                        .whitespace_normal(),
                                ),
                        );
                    }
                    TimelineItem::Trace(trace) => {
                        timeline_view = timeline_view.child(Self::render_trace(
                            trace,
                            self.focused_trace == Some(trace.id),
                        ));
                    }
                }
            }
            if status.is_active() && !received_output {
                timeline_view = timeline_view.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(7.0))
                        .py(px(4.0))
                        .text_size(px(11.0))
                        .text_color(ShellDeckColors::text_muted())
                        .child(animated_monolith(
                            "agent-console-thinking",
                            28.0,
                            MonolithMotion::Thinking,
                            cx,
                        ))
                        .child(animated_loading_text(
                            "agent-console-thinking-text",
                            t!("agents.status.preparing").to_string(),
                            cx,
                        )),
                );
            }
            output = output.child(timeline_view);
        }

        let mut composer = Composer::new(format!("agent-composer-{session_id}"), &prompt)
            .placeholder(t!("agents.prompt.placeholder").to_string())
            .min_rows(2)
            .max_rows(8)
            .disabled(status.is_active())
            .on_commit(submit);
        if context.provider != AgentProvider::Automonique {
            composer = composer.option(self.render_model_popover(cx));
        }
        if let Some(run_id) = running {
            composer = composer.without_commit().option(
                Button::new(("agent-stop", uuid_key(run_id)), "")
                    .variant(ButtonVariant::Destructive)
                    .size(ButtonSize::Icon)
                    .icon(IconSource::from("square"))
                    .tooltip(t!("agents.stop").to_string())
                    .w(px(28.0))
                    .h(px(28.0))
                    .rounded_full()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.request_stop(run_id, cx);
                    })),
            );
        }
        let mut content = div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w(px(0.0))
            .min_h(px(0.0));
        if !wide {
            content = content.child(header);
        }
        content = content.child(metadata);
        if context_expanded {
            content = content.child(self.render_context(!wide, window, cx));
        }
        content = content.child(output).child(
            div()
                .flex_shrink_0()
                .w_full()
                .px(if wide { px(18.0) } else { px(10.0) })
                .pt(px(7.0))
                .pb(px(11.0))
                .border_t_1()
                .border_color(ShellDeckColors::border())
                .child(div().w_full().max_w(px(940.0)).mx_auto().child(composer)),
        );
        let mut root = div()
            .relative()
            .flex()
            .size_full()
            .min_h(px(0.0))
            .bg(ShellDeckColors::bg_primary());
        if wide {
            root = root.child(self.render_navigator(cx));
        }
        root = root.child(content);
        if inspector_wide {
            root = root.child(self.render_inspector(&session, cx));
        }
        if let Some(pending) = self.pending_confirm.as_ref() {
            root = root.child(self.render_confirmation(pending, cx));
        }
        root
    }
}

fn context_select_cell<T: Clone + 'static>(select: Entity<Select<T>>) -> Div {
    div()
        .flex()
        .flex_1()
        .min_w(px(0.0))
        .overflow_hidden()
        .child(select)
}

fn inspector_empty_state(icon: &'static str, label: String) -> Div {
    div()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .flex_1()
        .gap(px(8.0))
        .px(px(18.0))
        .text_align(TextAlign::Center)
        .text_size(px(10.5))
        .text_color(ShellDeckColors::text_muted())
        .child(lucide_icon(icon, 22.0, ShellDeckColors::text_muted()))
        .child(label)
}

fn now_ms() -> i64 {
    Utc::now().timestamp_millis()
}

fn uuid_key(id: Uuid) -> u64 {
    (id.as_u128() ^ (id.as_u128() >> 64)) as u64
}

fn stable_string_key(value: &str) -> u64 {
    value
        .as_bytes()
        .iter()
        .fold(0xcbf29ce484222325, |hash, byte| {
            (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
        })
}

fn session_elapsed(session: &AgentSession) -> Option<String> {
    let started = session.started_at_ms?;
    let finished = session.finished_at_ms.unwrap_or_else(now_ms);
    let seconds = finished.saturating_sub(started) / 1_000;
    Some(if seconds < 60 {
        format!("{seconds}s")
    } else if seconds < 3_600 {
        format!("{}m {:02}s", seconds / 60, seconds % 60)
    } else {
        format!("{}h {:02}m", seconds / 3_600, (seconds % 3_600) / 60)
    })
}

fn merged_timeline(session: &AgentSession) -> Vec<TimelineItem> {
    let mut timeline = Vec::with_capacity(session.messages.len() + session.trace.len());
    timeline.extend(session.messages.iter().cloned().map(TimelineItem::Message));
    timeline.extend(session.trace.iter().cloned().map(TimelineItem::Trace));
    timeline.sort_by(|left, right| match (left.sequence(), right.sequence()) {
        // Pre-sequence durable sessions deserialize with zero. Preserve their
        // historical interleaving, then place all newly sequenced rows after
        // that recovered prefix.
        (0, 0) => left.at_ms().cmp(&right.at_ms()),
        (0, _) => std::cmp::Ordering::Less,
        (_, 0) => std::cmp::Ordering::Greater,
        (left, right) => left.cmp(&right),
    });
    timeline
}

fn target_switch_workdir(
    previous_connection: Option<Uuid>,
    next_connection: Option<Uuid>,
    current_workdir: &str,
    local_workdir: &str,
) -> Option<String> {
    match (previous_connection, next_connection) {
        (None, Some(_)) if current_workdir == local_workdir => {
            Some(AGENT_REMOTE_HOME_WORKDIR.to_string())
        }
        (Some(_), None) => Some(local_workdir.to_string()),
        _ => None,
    }
}

fn workdir_is_valid_for_target(workdir: &str, remote: bool) -> bool {
    if workdir.is_empty() {
        return false;
    }
    if remote {
        workdir == AGENT_REMOTE_HOME_WORKDIR || workdir.starts_with('/')
    } else {
        std::path::Path::new(workdir).is_absolute()
    }
}

fn remote_parent_path(path: &str) -> Option<String> {
    if path == AGENT_REMOTE_HOME_WORKDIR || path == "/" {
        return None;
    }
    let path = path.trim_end_matches('/');
    let separator = path.rfind('/')?;
    Some(if separator == 0 {
        "/".to_string()
    } else {
        path[..separator].to_string()
    })
}

fn inspector_files(session: &AgentSession) -> Vec<InspectorFile> {
    let mut by_path = HashMap::<String, InspectorFile>::new();
    for trace in &session.trace {
        match &trace.detail {
            AgentTraceKind::FileRead { path, .. } => {
                let entry = by_path
                    .entry(path.clone())
                    .or_insert_with(|| InspectorFile {
                        path: path.clone(),
                        latest_trace_id: trace.id,
                        latest_sequence: trace.sequence,
                        diff: None,
                    });
                if trace.sequence >= entry.latest_sequence {
                    entry.latest_trace_id = trace.id;
                    entry.latest_sequence = trace.sequence;
                }
            }
            AgentTraceKind::Diff {
                path,
                additions,
                deletions,
                preview,
                ..
            } => {
                let entry = by_path
                    .entry(path.clone())
                    .or_insert_with(|| InspectorFile {
                        path: path.clone(),
                        latest_trace_id: trace.id,
                        latest_sequence: trace.sequence,
                        diff: None,
                    });
                if trace.sequence >= entry.latest_sequence {
                    entry.latest_trace_id = trace.id;
                    entry.latest_sequence = trace.sequence;
                }
                if entry
                    .diff
                    .as_ref()
                    .is_none_or(|diff| trace.sequence >= diff.sequence)
                {
                    entry.diff = Some(InspectorDiff {
                        trace_id: trace.id,
                        sequence: trace.sequence,
                        additions: *additions,
                        deletions: *deletions,
                        preview: preview.clone(),
                    });
                }
            }
            _ => {}
        }
    }
    let mut files = by_path.into_values().collect::<Vec<_>>();
    files.sort_by(|left, right| {
        right
            .latest_sequence
            .cmp(&left.latest_sequence)
            .then_with(|| left.path.cmp(&right.path))
    });
    files
}

fn session_name(prompt: &str) -> String {
    let first = prompt.lines().next().unwrap_or_default().trim();
    let mut name = bounded_utf8(first.to_string(), 72);
    if name.len() < first.len() {
        name.push('…');
    }
    if name.is_empty() {
        t!("agents.session.new").to_string()
    } else {
        name
    }
}

fn provider_icon(provider: AgentProvider) -> &'static str {
    match provider {
        AgentProvider::Claude => "bot",
        AgentProvider::Codex => "terminal",
        AgentProvider::Automonique => "bot",
        AgentProvider::Jcode => "sparkles",
        AgentProvider::DeepSeek => "bot",
    }
}

fn target_label(target: &AgentTarget) -> String {
    match target {
        AgentTarget::Local => t!("agents.target.local").to_string(),
        AgentTarget::Ssh {
            connection_label, ..
        } => connection_label.clone(),
    }
}

fn session_worktree_matches(session: &AgentSession, worktree: &AgentWorktreeOption) -> bool {
    if !same_target_authority(&session.context.target, &worktree.target) {
        return false;
    }
    match &worktree.target {
        AgentTarget::Local => std::path::Path::new(&session.context.workdir)
            .starts_with(std::path::Path::new(&worktree.path)),
        AgentTarget::Ssh { .. } => {
            session.context.workdir == worktree.path
                || session
                    .context
                    .workdir
                    .strip_prefix(worktree.path.trim_end_matches('/'))
                    .is_some_and(|suffix| suffix.starts_with('/'))
        }
    }
}

fn same_target_authority(left: &AgentTarget, right: &AgentTarget) -> bool {
    match (left, right) {
        (AgentTarget::Local, AgentTarget::Local) => true,
        (
            AgentTarget::Ssh {
                connection_id: left,
                ..
            },
            AgentTarget::Ssh {
                connection_id: right,
                ..
            },
        ) => left == right,
        _ => false,
    }
}

fn worktree_bound_session(
    sessions: &[AgentSession],
    worktree: &AgentWorktreeOption,
) -> Option<Uuid> {
    sessions
        .iter()
        .find(|session| session_worktree_matches(session, worktree))
        .map(|session| session.id)
}

fn context_for_worktree(
    base: &AgentExecutionContext,
    worktree: &AgentWorktreeOption,
) -> AgentExecutionContext {
    AgentExecutionContext {
        target: worktree.target.clone(),
        workdir: worktree.path.clone(),
        ..base.clone()
    }
}

fn terminal_session_status(exit_code: Option<i32>, error_present: bool) -> AgentSessionStatus {
    if error_present {
        AgentSessionStatus::Failed
    } else {
        match exit_code {
            Some(0) => AgentSessionStatus::Completed,
            Some(_) => AgentSessionStatus::Failed,
            None => AgentSessionStatus::Cancelled,
        }
    }
}

fn access_label(access: AgentAccessMode) -> String {
    match access {
        AgentAccessMode::ReadOnly => t!("agents.access.read_only").to_string(),
        AgentAccessMode::WorkspaceWrite => t!("agents.access.workspace_write").to_string(),
        AgentAccessMode::FullAccess => t!("agents.access.full").to_string(),
    }
}

fn status_color(status: AgentSessionStatus) -> Hsla {
    match status {
        AgentSessionStatus::Idle | AgentSessionStatus::Cancelled => ShellDeckColors::text_muted(),
        AgentSessionStatus::Starting
        | AgentSessionStatus::Running
        | AgentSessionStatus::Stopping => ShellDeckColors::warning(),
        AgentSessionStatus::Completed => ShellDeckColors::success(),
        AgentSessionStatus::Failed => ShellDeckColors::error(),
    }
}

fn attention_color(attention: AgentSessionAttention) -> Hsla {
    match attention {
        AgentSessionAttention::None => ShellDeckColors::text_muted(),
        AgentSessionAttention::Unread => ShellDeckColors::primary(),
        AgentSessionAttention::NeedsAttention => ShellDeckColors::error(),
    }
}

fn trace_status_color(status: AgentTraceStatus) -> Hsla {
    match status {
        AgentTraceStatus::Succeeded => ShellDeckColors::success(),
        AgentTraceStatus::Failed | AgentTraceStatus::Cancelled => ShellDeckColors::error(),
        AgentTraceStatus::Pending | AgentTraceStatus::Running => ShellDeckColors::warning(),
        AgentTraceStatus::Unknown => ShellDeckColors::text_muted(),
    }
}

fn trace_presentation(
    detail: &AgentTraceKind,
) -> (&'static str, Hsla, String, Option<String>, Option<String>) {
    match detail {
        AgentTraceKind::Command {
            command,
            status,
            exit_code,
            summary,
        } => (
            "terminal",
            trace_status_color(*status),
            command.clone(),
            summary.clone(),
            exit_code.map(|code| code.to_string()),
        ),
        AgentTraceKind::FileRead {
            path,
            line_start,
            line_end,
            ..
        } => (
            "search",
            ShellDeckColors::text_muted(),
            path.clone(),
            None,
            match (line_start, line_end) {
                (Some(start), Some(end)) => Some(format!("{start}:{end}")),
                (Some(start), None) => Some(start.to_string()),
                _ => None,
            },
        ),
        AgentTraceKind::Diff {
            path,
            additions,
            deletions,
            preview,
            ..
        } => (
            "git-branch",
            ShellDeckColors::warning(),
            path.clone(),
            preview.clone(),
            Some(format!("+{additions} −{deletions}")),
        ),
        AgentTraceKind::Test {
            name,
            status,
            summary,
        } => (
            "check-check",
            trace_status_color(*status),
            name.clone(),
            summary.clone(),
            None,
        ),
        AgentTraceKind::Tool {
            name,
            status,
            summary,
        } => (
            "settings",
            trace_status_color(*status),
            name.clone(),
            summary.clone(),
            None,
        ),
        AgentTraceKind::Activity { label } => (
            "activity",
            ShellDeckColors::text_muted(),
            label.clone(),
            None,
            None,
        ),
    }
}

fn bounded_utf8(mut value: String, max_bytes: usize) -> String {
    if value.len() <= max_bytes {
        return value;
    }
    let mut end = max_bytes;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value.truncate(end);
    value
}

#[cfg(test)]
mod tests {
    use super::{
        context_for_worktree, inspector_files, remote_parent_path, session_name,
        target_switch_workdir, terminal_session_status, trace_presentation,
        workdir_is_valid_for_target, worktree_bound_session, AgentWorktreeOption,
    };
    use shelldeck_core::agent_runtime::{
        AgentAccessMode, AgentProvider, AgentTarget, AGENT_REMOTE_HOME_WORKDIR,
    };
    use shelldeck_core::agent_session::{
        AgentExecutionContext, AgentSession, AgentSessionStatus, AgentTraceEvent, AgentTraceKind,
        AgentTraceStatus,
    };

    // SDTEST-1882
    #[test]
    fn sdtest_1882_cockpit_trace_presentations_cover_structured_events() {
        let command = AgentTraceKind::Command {
            command: "cargo test".to_string(),
            status: AgentTraceStatus::Succeeded,
            exit_code: Some(0),
            summary: None,
        };
        let diff = AgentTraceKind::Diff {
            path: "src/main.rs".to_string(),
            status: AgentTraceStatus::Succeeded,
            additions: 4,
            deletions: 2,
            preview: None,
        };
        assert_eq!(trace_presentation(&command).0, "terminal");
        assert_eq!(trace_presentation(&command).4.as_deref(), Some("0"));
        assert_eq!(trace_presentation(&diff).4.as_deref(), Some("+4 −2"));
        assert!(session_name(&"x".repeat(100)).ends_with('…'));
    }

    // SDTEST-1899
    #[test]
    fn sdtest_1899_terminal_errors_always_fail_and_only_clean_stop_cancels() {
        assert_eq!(
            terminal_session_status(Some(0), true),
            AgentSessionStatus::Failed
        );
        assert_eq!(
            terminal_session_status(None, true),
            AgentSessionStatus::Failed
        );
        assert_eq!(
            terminal_session_status(None, false),
            AgentSessionStatus::Cancelled
        );
        assert_eq!(
            terminal_session_status(Some(0), false),
            AgentSessionStatus::Completed
        );
        assert_eq!(
            terminal_session_status(Some(17), false),
            AgentSessionStatus::Failed
        );
    }

    // SDTEST-1900
    #[test]
    fn sdtest_1900_worktree_selection_reuses_bound_session_or_clones_context() {
        let base = AgentExecutionContext {
            provider: AgentProvider::Codex,
            target: AgentTarget::Local,
            access: AgentAccessMode::ReadOnly,
            workdir: "/srv/original".to_string(),
            model: Some("gpt-5".to_string()),
        };
        let worktree = AgentWorktreeOption {
            id: "feature".to_string(),
            label: "feature".to_string(),
            path: "/srv/feature".to_string(),
            target: AgentTarget::Local,
            branch: Some("feature".to_string()),
            is_primary: false,
        };
        let bound = AgentSession::new("bound", context_for_worktree(&base, &worktree), 1)
            .expect("valid session");
        let bound_id = bound.id;
        assert_eq!(worktree_bound_session(&[bound], &worktree), Some(bound_id));
        let next_worktree = AgentWorktreeOption {
            path: "/srv/next".to_string(),
            ..worktree
        };
        let next = context_for_worktree(&base, &next_worktree);
        assert_eq!(base.workdir, "/srv/original");
        assert_eq!(next.workdir, "/srv/next");
        assert_eq!(next.provider, AgentProvider::Codex);
        assert_eq!(next.model.as_deref(), Some("gpt-5"));

        let remote_a = uuid::Uuid::new_v4();
        let remote_b = uuid::Uuid::new_v4();
        let remote_context = AgentExecutionContext {
            target: AgentTarget::Ssh {
                connection_id: remote_a,
                connection_label: "host-a".to_string(),
            },
            workdir: "/srv/shared".to_string(),
            ..base.clone()
        };
        let remote_session =
            AgentSession::new("remote", remote_context, 1).expect("valid SSH session");
        let other_host = AgentWorktreeOption {
            target: AgentTarget::Ssh {
                connection_id: remote_b,
                connection_label: "host-b".to_string(),
            },
            path: "/srv/shared".to_string(),
            ..next_worktree
        };
        assert_eq!(worktree_bound_session(&[remote_session], &other_host), None);
    }

    // SDTEST-1901
    #[test]
    fn sdtest_1901_stop_request_enters_stopping_before_runtime_completion() {
        let context = AgentExecutionContext {
            provider: AgentProvider::Claude,
            target: AgentTarget::Local,
            access: AgentAccessMode::ReadOnly,
            workdir: "/srv/project".to_string(),
            model: None,
        };
        let mut session = AgentSession::new("stop", context, 1).expect("valid session");
        session.begin_run("inspect", 2).expect("run starts");
        assert!(session.request_stop(3));
        assert_eq!(session.status, AgentSessionStatus::Stopping);
    }

    // SDTEST-1902
    #[test]
    fn sdtest_1902_inspector_projects_latest_files_and_diff_focus_from_trace() {
        let context = AgentExecutionContext {
            provider: AgentProvider::Claude,
            target: AgentTarget::Local,
            access: AgentAccessMode::ReadOnly,
            workdir: "/srv/project".to_string(),
            model: None,
        };
        let mut session = AgentSession::new("inspect", context, 1).expect("valid session");
        let read_id = uuid::Uuid::new_v4();
        let diff_id = uuid::Uuid::new_v4();
        session.trace = vec![
            AgentTraceEvent {
                id: read_id,
                sequence: 1,
                at_ms: 2,
                correlation_id: None,
                detail: AgentTraceKind::FileRead {
                    path: "src/lib.rs".to_string(),
                    status: AgentTraceStatus::Succeeded,
                    line_start: Some(1),
                    line_end: Some(40),
                },
            },
            AgentTraceEvent {
                id: diff_id,
                sequence: 2,
                at_ms: 3,
                correlation_id: None,
                detail: AgentTraceKind::Diff {
                    path: "src/lib.rs".to_string(),
                    status: AgentTraceStatus::Succeeded,
                    additions: 7,
                    deletions: 2,
                    preview: Some("+safe change".to_string()),
                },
            },
            AgentTraceEvent {
                id: uuid::Uuid::new_v4(),
                sequence: 3,
                at_ms: 4,
                correlation_id: None,
                detail: AgentTraceKind::FileRead {
                    path: "README.md".to_string(),
                    status: AgentTraceStatus::Succeeded,
                    line_start: None,
                    line_end: None,
                },
            },
        ];

        let files = inspector_files(&session);
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].path, "README.md");
        let changed = files
            .iter()
            .find(|file| file.path == "src/lib.rs")
            .expect("changed file remains in the projection");
        assert_eq!(changed.latest_trace_id, diff_id);
        assert_eq!(
            changed.diff.as_ref().map(|diff| diff.trace_id),
            Some(diff_id)
        );
        assert_eq!(changed.diff.as_ref().map(|diff| diff.additions), Some(7));
    }

    // SDTEST-1903
    #[test]
    fn sdtest_1903_target_switch_defaults_and_validation_are_target_aware() {
        let host = uuid::Uuid::new_v4();
        assert_eq!(
            target_switch_workdir(None, Some(host), "/home/me/project", "/home/me/project"),
            Some(AGENT_REMOTE_HOME_WORKDIR.to_string())
        );
        assert_eq!(
            target_switch_workdir(None, Some(host), "/srv/explicit", "/home/me/project"),
            None,
            "an explicit remote/worktree path is retained"
        );
        assert_eq!(
            target_switch_workdir(
                Some(host),
                None,
                AGENT_REMOTE_HOME_WORKDIR,
                "/home/me/project"
            ),
            Some("/home/me/project".to_string())
        );
        assert!(workdir_is_valid_for_target(AGENT_REMOTE_HOME_WORKDIR, true));
        assert!(workdir_is_valid_for_target("/srv/project", true));
        assert!(!workdir_is_valid_for_target("relative/project", true));
        assert!(!workdir_is_valid_for_target(
            AGENT_REMOTE_HOME_WORKDIR,
            false
        ));
        assert!(workdir_is_valid_for_target("/home/me/project", false));
        assert_eq!(
            remote_parent_path("/home/me/project/"),
            Some("/home/me".to_string())
        );
        assert_eq!(remote_parent_path("/home"), Some("/".to_string()));
        assert_eq!(remote_parent_path("/"), None);
    }
}
