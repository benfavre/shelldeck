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
use crate::monolith::{animated_monolith, MonolithMotion};
use crate::scale::px;
use crate::t;
use crate::theme::ShellDeckColors;

#[path = "agent_console_view/activity.rs"]
mod activity;
#[path = "agent_console_view/auth_recovery.rs"]
mod auth_recovery;
#[path = "agent_console_view/composer.rs"]
mod composer;
#[path = "agent_console_view/composer_controls.rs"]
mod composer_controls;
#[path = "agent_console_view/editor.rs"]
mod editor;
mod timeline;
pub use editor::save_local_document;
pub use editor::{AgentFileOrigin, AgentFileSaveRequest};
#[path = "agent_console_view/connection_repair.rs"]
mod connection_repair;
#[path = "agent_console_view/explorer.rs"]
mod explorer;
#[path = "agent_console_view/inspector.rs"]
mod inspector;
#[cfg(test)]
#[path = "agent_console_view/interaction_tests.rs"]
mod interaction_tests;
#[path = "agent_console_view/library.rs"]
mod library;
#[path = "agent_console_view/live_browser.rs"]
mod live_browser;
#[path = "agent_console_view/live_input.rs"]
mod live_input;
#[path = "agent_console_view/model.rs"]
mod model;
#[path = "agent_console_view/native_history.rs"]
mod native_history;
mod navigation_groups;
#[path = "agent_console_view/navigator.rs"]
mod navigator;
#[path = "agent_console_view/progress.rs"]
mod progress;
#[path = "agent_console_view/project_picker.rs"]
mod project_picker;
#[path = "agent_console_view/remote_sessions.rs"]
mod remote_sessions;
#[path = "agent_console_view/ssh_sessions.rs"]
mod ssh_sessions;
#[path = "agent_console_view/transcript.rs"]
mod transcript;
use live_input::PendingLiveInput;
pub use native_history::NativeHistoryRequest;
#[path = "agent_console_view/review.rs"]
mod review;
#[path = "agent_console_view/tools.rs"]
mod tools;
use composer::AgentComposerUi;
use library::AgentLibraryUi;
use review::AgentReviewUi;
use tools::AgentToolsUi;
#[path = "agent_console_view/workflows.rs"]
mod workflows;
use workflows::AgentWorkflowsUi;
#[path = "agent_console_view/delivery.rs"]
mod delivery;
use delivery::AgentDeliveryUi;

actions!(
    agent_cockpit,
    [
        AgentNewChat,
        AgentFocusComposer,
        AgentToggleFiles,
        AgentToggleSessions,
        AgentJumpLatest
    ]
);

const MAX_ACTIVITY_BYTES: usize = 4 * 1024;

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
    NavigatorChanged,
    ReconnectProject {
        checkout_id: shelldeck_core::config::workspace_catalog::CatalogCheckoutId,
        expected_connection: Uuid,
        connection_id: Uuid,
    },
    RefreshRemoteSessions,
    LoadSshHistory(NativeHistoryRequest),
    DiscoverSshSessions {
        connection_id: Uuid,
        request_id: Uuid,
        cursor: Option<String>,
    },
    ReconnectSshSession {
        connection_id: Uuid,
        request_id: Uuid,
        session: shelldeck_core::agent_remote_sessions::RemoteAgentSession,
    },
    OpenPlatformSession(shelldeck_core::config::platform::ResourceCoordinate),
    SaveFile(AgentFileSaveRequest),
    Run(AgentRunRequest),
    Stop(Uuid),
    CloseSession(Uuid),
    OpenTerminal {
        context: AgentExecutionContext,
    },
    OpenWorkspaceLauncher,
    OpenProjectSetup,
    CheckReadiness {
        check_id: Uuid,
        context: AgentExecutionContext,
    },
    Steer {
        run_id: Uuid,
        text: String,
    },
    QuestionAnswer {
        run_id: Uuid,
        question_id: String,
        answer: String,
    },
    ApprovalDecision {
        run_id: Uuid,
        request_id: String,
        option_id: Option<String>,
    },
    OpenRemoteFile {
        context: AgentExecutionContext,
        session_id: Uuid,
        request_id: Uuid,
        connection_id: Uuid,
        path: String,
    },
    OpenLocalFile {
        session_id: Uuid,
        path: PathBuf,
    },
    BrowseRemoteDirectory(AgentRemoteDirectoryRequest),
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

#[derive(Debug, Clone)]
pub struct AgentRemoteDirectoryRequest {
    pub session_id: Uuid,
    pub connection_id: Uuid,
    pub request_id: Uuid,
    pub context: AgentExecutionContext,
    pub root: String,
    pub path: String,
}

pub struct AgentRemoteDirectoryListing {
    pub request: AgentRemoteDirectoryRequest,
    pub root: String,
    pub truncated: bool,
    pub resolved_path: String,
    pub result: Result<Vec<AgentRemoteFileEntry>, String>,
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
    _draft_subscription: Subscription,
    prompt: Entity<InputState>,
    run_id: Option<Uuid>,
    received_delta: bool,
    received_output: bool,
    run_details_expanded: bool,
    plan_expanded: bool,
    expanded_activity: HashSet<Uuid>,
    raw_transcripts: HashSet<transcript::TranscriptId>,
    pending_approval: Option<String>,
}

impl AgentSessionUi {
    fn new(cx: &mut Context<AgentConsoleView>) -> Self {
        let prompt = cx.new(|cx| InputState::new(cx).multi_line(true));
        let subscription = cx.subscribe(&prompt, |this, _, event, cx| {
            if matches!(
                event,
                adabraka_ui::components::input_state::InputEvent::Change
            ) {
                this.schedule_library_save(cx);
                // Refresh Send readiness immediately while the draft is edited.
                cx.notify();
            }
        });
        Self {
            _draft_subscription: subscription,
            prompt,
            run_id: None,
            received_delta: false,
            received_output: false,
            run_details_expanded: false,
            plan_expanded: false,
            expanded_activity: HashSet::new(),
            raw_transcripts: HashSet::new(),
            pending_approval: None,
        }
    }
}

struct PendingRun {
    session_id: Uuid,
    context: AgentExecutionContext,
    prompt: String,
    options: shelldeck_core::agent_runtime::AgentInputOptions,
    source_draft: Option<String>,
}

#[derive(Clone)]
enum TimelineItem {
    Message(AgentMessage),
    Trace(AgentTraceEvent),
    CompletedWork(Vec<AgentTraceEvent>),
    Boundary(shelldeck_core::agent_session::AgentHistoryBoundary),
}

impl TimelineItem {
    fn sequence(&self) -> u64 {
        match self {
            Self::Message(message) => message.sequence,
            Self::Trace(trace) => trace.sequence,
            Self::CompletedWork(traces) => traces.first().map_or(0, |trace| trace.sequence),
            Self::Boundary(boundary) => boundary.sequence,
        }
    }

    fn at_ms(&self) -> i64 {
        match self {
            Self::Message(message) => message.at_ms,
            Self::Trace(trace) => trace.at_ms,
            Self::CompletedWork(traces) => traces.first().map_or(0, |trace| trace.at_ms),
            Self::Boundary(boundary) => boundary.at_ms,
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
    model_discovery: model::AgentModelDiscoveryUi,
    navigator_search: Entity<InputState>,
    sessions: AgentSessionCollection,
    remote_sessions: remote_sessions::RemoteAgentSessionsUi,
    ssh_sessions: ssh_sessions::SshSessionsUi,
    native_history: HashMap<Uuid, native_history::NativeHistoryUi>,
    session_ui: HashMap<Uuid, AgentSessionUi>,
    run_sessions: HashMap<Uuid, Uuid>,
    foreground_runs: HashSet<Uuid>,
    pending_live_inputs: HashMap<Uuid, PendingLiveInput>,
    project_groups: Vec<AgentProjectGroup>,
    context_locks: HashSet<Uuid>,
    pending_confirm: Option<PendingRun>,
    context_expanded: bool,
    agent_setup_expanded: bool,
    inspector_tab: AgentInspectorTab,
    focused_trace: Option<Uuid>,
    focused_trace_anchor: ScrollAnchor,
    pending_trace_reveal: Option<Uuid>,
    surface_focus_requested: bool,
    inspector_local_browser: FileBrowserPanel,
    inspector_remote_request: Uuid,
    inspector_remote_root: Option<String>,
    inspector_remote_truncated: bool,
    inspector_remote_path: Option<String>,
    inspector_remote_pending_path: Option<String>,
    inspector_remote_session: Option<Uuid>,
    inspector_remote_connection: Option<Uuid>,
    inspector_remote_entries: Vec<AgentRemoteFileEntry>,
    inspector_remote_loading: bool,
    inspector_remote_error: Option<String>,
    scroll: ScrollHandle,
    library: AgentLibraryUi,
    composer_ui: AgentComposerUi,
    review: AgentReviewUi,
    tools: AgentToolsUi,
    editors: editor::AgentEditorsUi,
    live_browsers: live_browser::AgentLiveBrowsersUi,
    activity: activity::ActivityUi,
    timeline: timeline::TimelineUi,
    #[cfg(test)]
    rendered_transcripts: std::cell::Cell<usize>,
    explorer: explorer::AgentExplorerUi,
    workflows: AgentWorkflowsUi,
    delivery: AgentDeliveryUi,
}

fn input_state(cx: &mut Context<AgentConsoleView>, initial: String) -> Entity<InputState> {
    let state = cx.new(|cx| {
        let mut state = InputState::new(cx);
        state.content = initial.into();
        state
    });
    // Manual model/path edits must immediately refresh continuity and Send readiness.
    cx.subscribe(&state, |this, _, event, cx| {
        if matches!(
            event,
            adabraka_ui::components::input_state::InputEvent::Change
        ) {
            this.schedule_library_save(cx);
            cx.notify();
        }
    })
    .detach();
    state
}

fn selected_agent_target(
    selected: Option<Uuid>,
    connections: &[AgentConnectionOption],
) -> AgentTarget {
    match selected {
        None => AgentTarget::Local,
        Some(connection_id) => AgentTarget::Ssh {
            connection_id,
            connection_label: connections
                .iter()
                .find(|item| item.id == connection_id)
                .map(|item| item.label.clone())
                .unwrap_or_else(|| t!("agents.target.missing").to_string()),
        },
    }
}

fn validate_agent_target(
    target: &AgentTarget,
    connections: &[AgentConnectionOption],
) -> Result<(), String> {
    match target {
        AgentTarget::Ssh { connection_id, .. }
            if !connections.iter().any(|item| item.id == *connection_id) =>
        {
            Err(t!("agents.error.target_missing").to_string())
        }
        _ => Ok(()),
    }
}

fn agent_target_options(
    connections: &[AgentConnectionOption],
    selected: Option<Uuid>,
) -> (Vec<SelectOption<Option<Uuid>>>, Option<usize>) {
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
    if let Some(id) = selected.filter(|id| !connections.iter().any(|item| item.id == *id)) {
        options.push(
            SelectOption::new(Some(id), format!("{} ({id})", t!("agents.target.missing")))
                .with_icon("icons/lucide/server.svg"),
        );
    }
    let index = options.iter().position(|option| option.value == selected);
    (options, index)
}

impl AgentConsoleView {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let modifier = if cfg!(target_os = "macos") {
            "cmd"
        } else {
            "ctrl"
        };
        cx.bind_keys([
            KeyBinding::new(
                &format!("{modifier}-shift-o"),
                AgentNewChat,
                Some("AgentConsole"),
            ),
            KeyBinding::new(
                &format!("{modifier}-l"),
                AgentFocusComposer,
                Some("AgentConsole"),
            ),
            KeyBinding::new(
                &format!("{modifier}-shift-e"),
                AgentToggleFiles,
                Some("AgentConsole"),
            ),
            KeyBinding::new(
                &format!("{modifier}-shift-b"),
                AgentToggleSessions,
                Some("AgentConsole"),
            ),
            KeyBinding::new(
                &format!("{modifier}-shift-j"),
                AgentJumpLatest,
                Some("AgentConsole"),
            ),
        ]);
        let parent = cx.entity();
        let provider_select =
            Self::build_provider_select(AgentProvider::Claude, false, parent.clone(), cx);
        let access_select = Self::build_access_select(
            AgentProvider::Claude,
            AgentAccessMode::ReadOnly,
            false,
            parent.clone(),
            cx,
        );
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
        let scroll = ScrollHandle::new();
        let mut view = Self {
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
            model_discovery: model::AgentModelDiscoveryUi::new(),
            navigator_search: input_state(cx, String::new()),
            sessions,
            remote_sessions: remote_sessions::RemoteAgentSessionsUi::default(),
            ssh_sessions: Default::default(),
            native_history: Default::default(),
            session_ui,
            run_sessions: HashMap::new(),
            foreground_runs: HashSet::new(),
            pending_live_inputs: HashMap::new(),
            project_groups: Vec::new(),
            context_locks: HashSet::new(),
            pending_confirm: None,
            context_expanded: false,
            agent_setup_expanded: false,
            inspector_tab: AgentInspectorTab::Files,
            focused_trace: None,
            activity: activity::ActivityUi::new(scroll.clone()),
            timeline: timeline::TimelineUi::default(),
            #[cfg(test)]
            rendered_transcripts: std::cell::Cell::new(0),
            focused_trace_anchor: ScrollAnchor::for_handle(scroll.clone()),
            pending_trace_reveal: None,
            surface_focus_requested: false,
            inspector_local_browser,
            inspector_remote_request: Uuid::new_v4(),
            inspector_remote_root: None,
            inspector_remote_truncated: false,
            inspector_remote_path: None,
            inspector_remote_pending_path: None,
            inspector_remote_session: None,
            inspector_remote_connection: None,
            inspector_remote_entries: Vec::new(),
            inspector_remote_loading: false,
            inspector_remote_error: None,
            scroll,
            library: AgentLibraryUi::new(cx),
            composer_ui: AgentComposerUi::new(cx),
            review: AgentReviewUi::new(),
            tools: AgentToolsUi::new(cx),
            editors: editor::AgentEditorsUi::default(),
            live_browsers: live_browser::AgentLiveBrowsersUi::default(),
            explorer: explorer::AgentExplorerUi::default(),
            workflows: AgentWorkflowsUi::new(cx),
            delivery: AgentDeliveryUi::new(cx),
        };
        view.restore_library(cx);
        view
    }

    /// Activate the retained middle pane when the production window is shown.
    /// Focus is applied during rendering, once that pane belongs to a window.
    pub fn request_surface_focus(&mut self, cx: &mut Context<Self>) {
        self.surface_focus_requested = true;
        cx.notify();
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
                .on_change(move |provider, _window, cx| {
                    parent.update(cx, |this, cx| {
                        this.change_draft_provider(*provider, cx);
                    });
                })
        })
    }

    fn build_access_select(
        provider: AgentProvider,
        selected: AgentAccessMode,
        disabled: bool,
        parent: Entity<Self>,
        cx: &mut Context<Self>,
    ) -> Entity<Select<AgentAccessMode>> {
        let options = vec![
            SelectOption::new(
                AgentAccessMode::ReadOnly,
                composer_controls::provider_access_label(provider, AgentAccessMode::ReadOnly),
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
                        this.change_draft_access(*access, cx);
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
        let (options, selected_index) = agent_target_options(connections, selected);
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
                        if this.context_controls_locked() {
                            return;
                        }
                        if connection_id
                            .is_some_and(|id| !this.connections.iter().any(|item| item.id == id))
                        {
                            return;
                        }
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
                        this.schedule_library_save(cx);
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
        // Discovery cannot grant a different execution target. A missing SSH
        // UUID remains selected until the user explicitly chooses another target.
        self.connections = connections;
        self.rebuild_ssh_session_selector(cx);
        let locked = self.context_controls_locked();
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
        listing: AgentRemoteDirectoryListing,
        cx: &mut Context<Self>,
    ) {
        let AgentRemoteDirectoryListing {
            request:
                AgentRemoteDirectoryRequest {
                    session_id,
                    connection_id,
                    request_id,
                    context,
                    path: requested_path,
                    ..
                },
            root,
            truncated,
            resolved_path,
            result,
        } = listing;
        let selected_matches = self.sessions.selected_id() == Some(session_id)
            && matches!(self.displayed_context(cx).target,AgentTarget::Ssh{connection_id:selected,..}if selected==connection_id);
        if !selected_matches
            || self.inspector_remote_request != request_id
            || self.displayed_context(cx) != context
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
                self.inspector_remote_root = Some(root);
                self.inspector_remote_truncated = truncated;
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
        self.inspector_remote_request = Uuid::new_v4();
        self.inspector_remote_session = Some(session_id);
        self.inspector_remote_connection = Some(connection_id);
        self.inspector_remote_path = Some(path.clone());
        self.inspector_remote_pending_path = Some(path.clone());
        self.inspector_remote_loading = true;
        self.inspector_remote_error = None;
        cx.emit(AgentConsoleEvent::BrowseRemoteDirectory(
            AgentRemoteDirectoryRequest {
                request_id: self.inspector_remote_request,
                context: self.displayed_context(cx),
                root: self
                    .inspector_remote_root
                    .clone()
                    .unwrap_or_else(|| self.displayed_context(cx).workdir),
                session_id,
                connection_id,
                path,
            },
        ));
        cx.notify();
    }

    pub fn sessions(&self) -> &[AgentSession] {
        self.sessions.sessions()
    }

    pub fn selected_session_id(&self) -> Option<Uuid> {
        self.sessions.selected_id()
    }

    pub fn is_run_starting(&self, run_id: Uuid) -> bool {
        self.run_sessions
            .get(&run_id)
            .and_then(|id| self.sessions.get(*id))
            .is_some_and(|s| s.status == AgentSessionStatus::Starting)
    }

    pub fn session_id_for_run(&self, run_id: Uuid) -> Option<Uuid> {
        self.run_sessions.get(&run_id).copied()
    }

    pub fn run_requests_focus(&self, run_id: Uuid) -> bool {
        !self.remote_sessions.active
            && self.foreground_runs.contains(&run_id)
            && self.run_sessions.get(&run_id).copied() == self.sessions.selected_id()
    }

    pub fn admit_starting_run_workdir(
        &mut self,
        request: &mut AgentRunRequest,
        workdir: &str,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        let session_id = self
            .run_sessions
            .get(&request.id)
            .copied()
            .ok_or_else(|| t!("agents.error.runtime_lost").to_string())?;
        if self.session_ui.get(&session_id).and_then(|ui| ui.run_id) != Some(request.id) {
            return Err(t!("agents.error.runtime_lost").to_string());
        }
        let changed = self
            .sessions
            .get_mut(session_id)
            .ok_or_else(|| t!("agents.error.runtime_lost").to_string())?
            .admit_starting_workdir(request, workdir, now_ms())
            .map_err(|error| error.to_string())?;
        if changed {
            if self.sessions.selected_id() == Some(session_id) {
                self.apply_selected_context(cx);
            }
            self.schedule_library_save(cx);
            cx.notify();
        }
        Ok(())
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
        if self.sessions.get(session_id).is_none() {
            return false;
        }
        let unchanged = self.sessions.selected_id() == Some(session_id);
        self.clear_ssh_session_preview();
        self.close_navigator_overlay(cx);
        if !unchanged {
            self.retain_draft_context(cx);
        }
        if self.sessions.select(session_id).is_err() {
            return false;
        }
        self.set_remote_session_active(false, cx);
        if !unchanged {
            self.apply_selected_context(cx);
        }
        if self.sessions.surface_visible() {
            self.request_surface_focus(cx);
        }
        self.schedule_library_save(cx);
        self.timeline.pin(&self.scroll);
        cx.notify();
        true
    }

    pub fn create_session(
        &mut self,
        name: impl Into<String>,
        context: AgentExecutionContext,
        cx: &mut Context<Self>,
    ) -> Result<Uuid, String> {
        self.retain_draft_context(cx);
        let session_id = self
            .sessions
            .create(name, context, now_ms())
            .map_err(|error| error.to_string())?;
        self.session_ui.insert(session_id, AgentSessionUi::new(cx));
        self.clear_ssh_session_preview();
        self.close_navigator_overlay(cx);
        self.set_remote_session_active(false, cx);
        self.apply_selected_context(cx);
        if self.sessions.surface_visible() {
            self.request_surface_focus(cx);
        }
        self.schedule_library_save(cx);
        self.timeline.pin(&self.scroll);
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
            ui.pending_approval = None;
            ui.received_delta = false;
            ui.received_output = false;
        }
        if self.sessions.selected_id() == Some(session_id) && self.sessions.surface_visible() {
            self.timeline.pin(&self.scroll);
        }
        self.schedule_library_save(cx);
        cx.notify();
    }

    pub fn set_surface_visible(&mut self, visible: bool, cx: &mut Context<Self>) {
        self.remote_sessions.surface_visible = visible;
        if self.sessions.set_surface_visible(
            visible && !self.remote_sessions.active && self.ssh_sessions.preview.is_none(),
        ) {
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
        self.retire_browser_run(run_id);
        self.take_run_options(run_id);
        self.pending_live_inputs.remove(&run_id);
        self.foreground_runs.remove(&run_id);
        self.workflow_finish_run(run_id, AgentSessionStatus::Failed, cx);
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
            ui.pending_approval = None;
        }
        if self.sessions.selected_id() == Some(session_id) {
            self.apply_selected_context(cx);
        }
        self.schedule_library_save(cx);
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
        self.push_stream_events_for(run_id, std::iter::once(event), cx);
    }

    /// One persistence timer and repaint per bounded runtime batch.
    pub fn push_stream_events_for(
        &mut self,
        run_id: Uuid,
        events: impl IntoIterator<Item = AgentStreamEvent>,
        cx: &mut Context<Self>,
    ) {
        let mut changed = false;
        for event in events {
            changed |= self.apply_stream_event_for(run_id, event, cx);
        }
        if changed {
            self.schedule_library_save(cx);
            cx.notify();
        }
    }

    fn apply_stream_event_for(
        &mut self,
        run_id: Uuid,
        event: AgentStreamEvent,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(session_id) = self.run_sessions.get(&run_id).copied() else {
            return false;
        };
        if self.sessions.selected_id() == Some(session_id) {
            self.timeline.epoch = self.timeline.epoch.wrapping_add(1);
        }
        self.workflow_observe_event(run_id, &event, cx);
        self.acknowledge_live_input(run_id, session_id, &event, cx);
        if let AgentStreamEvent::InteractionResolved { id } = &event {
            if let Some(ui) = self.session_ui.get_mut(&session_id) {
                if ui.pending_approval.as_ref() == Some(id) {
                    ui.pending_approval = None;
                }
            }
        }
        if self.sessions.selected_id() == Some(session_id) && self.sessions.surface_visible() {
            self.timeline.follow(&self.scroll);
        }
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
            return false;
        }
        if let Some(ui) = self.session_ui.get_mut(&session_id) {
            match &event {
                AgentStreamEvent::Text(text)
                | AgentStreamEvent::TextDelta(text)
                | AgentStreamEvent::TextSnapshot(text) => {
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
        true
    }

    pub fn finish_run(
        &mut self,
        run_id: Uuid,
        exit_code: Option<i32>,
        error: Option<String>,
        cx: &mut Context<Self>,
    ) {
        self.retire_browser_run(run_id);
        self.take_run_options(run_id);
        self.pending_live_inputs.remove(&run_id);
        self.foreground_runs.remove(&run_id);
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
        self.workflow_finish_run(run_id, status, cx);
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
            ui.pending_approval = None;
        }
        if self.sessions.selected_id() == Some(session_id) && self.sessions.surface_visible() {
            self.timeline.follow(&self.scroll);
        }
        if self.sessions.selected_id() == Some(session_id) {
            self.apply_selected_context(cx);
        }
        self.finish_review_run(session_id, cx);
        self.schedule_library_save(cx);
        self.drain_queued(session_id, cx);
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
        let locked = self.context_locks.contains(&session.id) || session.status.is_active();
        self.provider_select = Self::build_provider_select(self.provider, locked, cx.entity(), cx);
        self.access_select =
            Self::build_access_select(self.provider, self.access, locked, cx.entity(), cx);
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
                if self.inspector_local_browser.root() != root.as_path() {
                    self.inspector_local_browser.set_root(root);
                }
                self.inspector_remote_connection = None;
                self.inspector_remote_root = None;
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
                    self.inspector_remote_root = None;
                    self.inspector_remote_path = None;
                    self.inspector_remote_pending_path = None;
                    self.inspector_remote_entries.clear();
                    self.inspector_remote_error = None;
                    self.inspector_remote_loading = false;
                }
            }
        }
    }

    fn context_controls_locked(&self) -> bool {
        self.sessions.selected().is_some_and(|session| {
            self.context_locks.contains(&session.id) || session.status.is_active()
        })
    }

    /// Retain valid unsent selections when navigating away, using the same
    /// context boundary that protects provider resume tokens at submission.
    fn retain_draft_context(&mut self, cx: &mut Context<Self>) {
        if self.context_controls_locked() {
            return;
        }
        if let Ok(context) = self.context_from_controls(cx) {
            if let Some(session) = self
                .sessions
                .selected_id()
                .and_then(|id| self.sessions.get_mut(id))
            {
                let _ = session.set_context(context, now_ms());
            }
        }
    }

    fn change_draft_provider(&mut self, provider: AgentProvider, cx: &mut Context<Self>) {
        if self.context_controls_locked() || self.provider == provider {
            return;
        }
        self.provider = provider;
        // A model identifier and reasoning choice belong to their provider.
        self.model_state
            .update(cx, |state, cx| state.replace_content(String::new(), cx));
        if let Some(id) = self.sessions.selected_id() {
            self.composer_ui.effort.remove(&id);
        }
        self.provider_select = Self::build_provider_select(provider, false, cx.entity(), cx);
        self.access_select =
            Self::build_access_select(provider, self.access, false, cx.entity(), cx);
        self.schedule_library_save(cx);
        cx.notify();
    }

    fn change_draft_access(&mut self, access: AgentAccessMode, cx: &mut Context<Self>) {
        if self.context_controls_locked() {
            return;
        }
        self.access = access;
        self.access_select =
            Self::build_access_select(self.provider, access, false, cx.entity(), cx);
        self.schedule_library_save(cx);
        cx.notify();
    }

    fn context_from_controls(&self, cx: &Context<Self>) -> Result<AgentExecutionContext, String> {
        if let Some(session_id) = self.sessions.selected_id() {
            if self.context_locks.contains(&session_id) {
                let context = self
                    .sessions
                    .get(session_id)
                    .map(|session| session.context.clone())
                    .ok_or_else(|| t!("agents.error.runtime_lost").to_string())?;
                validate_agent_target(&context.target, &self.connections)?;
                return Ok(context);
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
        let target = selected_agent_target(self.selected_connection, &self.connections);
        validate_agent_target(&target, &self.connections)?;
        let context = AgentExecutionContext {
            provider: self.provider,
            target,
            access: self.access,
            workdir,
            model: (self.provider != AgentProvider::Automonique && !model.is_empty())
                .then_some(model),
        };
        if let Some(id) = self.sessions.selected_id() {
            self.validate_native_context_transition(id, &context)?;
        }
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
        if self
            .native_history
            .get(&session_id)
            .is_some_and(|state| state.fresh_view)
        {
            return Err(t!("agents.native_history.fresh_help").to_string());
        }
        if self.sessions.get(session_id).is_some_and(|session| {
            Self::native_context_needs_reconnect(session, &self.displayed_context(cx))
        }) {
            return Err(t!("agents.native_history.reconnect_required").to_string());
        }
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
            prompt: self.prepare_composer_prompt(session_id, &prompt, cx)?,
            options: self.composer_input_options(session_id)?,
            source_draft: Some(prompt),
        })
    }

    fn submit(&mut self, cx: &mut Context<Self>) {
        if let Some((session_id, question_id)) = self.sessions.selected().and_then(|session| {
            session
                .question
                .as_ref()
                .filter(|question| question.response_supported)
                .map(|question| (session.id, question.id.clone()))
        }) {
            if let Some(run_id) = self.session_ui.get(&session_id).and_then(|ui| ui.run_id) {
                let answer = self
                    .session_ui
                    .get(&session_id)
                    .unwrap()
                    .prompt
                    .read(cx)
                    .content()
                    .trim()
                    .to_string();
                if !answer.is_empty() && answer.len() <= MAX_AGENT_PROMPT_BYTES {
                    if !self.reserve_live_input(
                        run_id,
                        PendingLiveInput::question(Some(answer.clone()), question_id.clone()),
                        cx,
                    ) {
                        return;
                    }
                    cx.emit(AgentConsoleEvent::QuestionAnswer {
                        run_id,
                        question_id,
                        answer,
                    });
                    self.schedule_library_save(cx);
                }
                return;
            }
        }
        if self.queue_active_draft(cx) {
            return;
        }
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
        if let Err(error) =
            self.validate_native_context_transition(pending.session_id, &pending.context)
        {
            self.composer_ui.errors.insert(pending.session_id, error);
            cx.notify();
            return;
        }
        if let Err(error) = validate_agent_target(&pending.context.target, &self.connections) {
            self.apply_session_draft_error(pending.session_id, error, cx);
            return;
        }
        if let Err(error) = self.validate_discovered_model_input(&pending.context, &pending.options)
        {
            self.apply_session_draft_error(pending.session_id, error, cx);
            return;
        }
        let foreground = pending.source_draft.is_some();
        let clear_draft = pending.source_draft.as_ref().is_some_and(|source| {
            self.session_ui
                .get(&pending.session_id)
                .is_some_and(|ui| ui.prompt.read(cx).content().trim() == source)
                && self
                    .prepare_composer_prompt(pending.session_id, source, cx)
                    .ok()
                    .as_ref()
                    == Some(&pending.prompt)
                && self
                    .composer_input_options(pending.session_id)
                    .ok()
                    .as_ref()
                    == Some(&pending.options)
        });
        let session_id = pending.session_id;
        let Some(session) = self.sessions.get_mut(session_id) else {
            self.apply_session_draft_error(
                session_id,
                t!("agents.error.runtime_lost").to_string(),
                cx,
            );
            return;
        };
        if let Err(error) = session.set_context(pending.context, now_ms()) {
            self.apply_session_draft_error(session_id, error.to_string(), cx);
            return;
        }
        if session.messages.is_empty() && session.provider_session_id().is_none() {
            let _ = session.rename(session_name(&pending.prompt), now_ms());
        }
        match self
            .sessions
            .begin_run(session_id, pending.prompt, now_ms())
        {
            Ok(request) => {
                self.run_sessions.insert(request.id, session_id);
                if foreground {
                    self.foreground_runs.insert(request.id);
                }
                if let Some(ui) = self.session_ui.get_mut(&session_id) {
                    ui.run_id = Some(request.id);
                    ui.received_delta = false;
                    ui.received_output = false;
                }
                self.set_run_options(request.id, pending.options);
                if !self.workflow_register_run(&request, cx) {
                    self.reject_run_for(
                        request.id,
                        t!("agents.workflows.context_changed").to_string(),
                        cx,
                    );
                    return;
                }
                if !foreground {
                    self.consume_admitted_queued(&request);
                }
                if clear_draft {
                    if let Some(ui) = self.session_ui.get(&session_id) {
                        ui.prompt.update(cx, |state, cx| state.reset(cx));
                    }
                    self.clear_composer_context(session_id);
                }
                if self.sessions.selected_id() == Some(session_id) {
                    self.apply_selected_context(cx);
                }
                cx.emit(AgentConsoleEvent::Run(request));
                self.schedule_library_save(cx);
            }
            Err(error) => self.apply_session_draft_error(session_id, error.to_string(), cx),
        }
    }

    fn apply_draft_error(&mut self, error: String, cx: &mut Context<Self>) {
        if let Some(session_id) = self.sessions.selected_id() {
            self.apply_session_draft_error(session_id, error, cx);
        }
    }

    fn apply_session_draft_error(
        &mut self,
        session_id: Uuid,
        error: String,
        cx: &mut Context<Self>,
    ) {
        let _ =
            self.sessions
                .apply_stream_event(session_id, AgentStreamEvent::Error(error), now_ms());
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
        let base = self.displayed_context(cx);
        let context = context_for_worktree(&base, worktree);
        self.create_session(t!("agents.session.new").to_string(), context, cx)
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
            self.retire_browser_run(run_id);
            cx.emit(AgentConsoleEvent::Stop(run_id));
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
                                this.workflow_cancel_pending();
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
                        this.workflow_cancel_pending();
                        cx.notify();
                    });
                }
            })
            .into_any_element()
    }

    fn render_trace(
        &self,
        trace: &AgentTraceEvent,
        focused: bool,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (icon, color, title, detail, stats) = trace_presentation(&trace.detail);
        let status = activity::operation_status(&trace.detail);
        let animate = status == Some(AgentTraceStatus::Running)
            && self.sessions.surface_visible()
            && self.sessions.selected().is_some_and(|session| {
                matches!(
                    session.status,
                    AgentSessionStatus::Starting | AgentSessionStatus::Running
                ) && trace.turn_id == session.current_turn_id()
            });
        let title = if matches!(trace.detail, AgentTraceKind::Activity { .. }) {
            Self::concise_activity_label(title)
        } else {
            title
        };
        // Plain lifecycle activity belongs in the timeline, but does not need
        // the command/tool card chrome used for inspectable technical work.
        if matches!(trace.detail, AgentTraceKind::Activity { .. }) && !focused {
            return div()
                .id(("agent-trace", uuid_key(trace.id)))
                .w_full()
                .flex_shrink_0()
                .min_w(px(0.0))
                .flex()
                .items_start()
                .gap(px(6.0))
                .py(px(3.0))
                .text_size(px(10.5))
                .text_color(color)
                .child(lucide_icon(icon, 12.0, color))
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.0))
                        .whitespace_normal()
                        .child(title),
                )
                .into_any_element();
        }
        div()
            .id(("agent-trace", uuid_key(trace.id)))
            .anchor_scroll(focused.then(|| self.focused_trace_anchor.clone()))
            .debug_selector(|| format!("agent-trace-{}", trace.id))
            .w_full()
            .min_w_0()
            // Retained output must scroll at its natural height. Hidden overflow
            // otherwise permits a flex parent to squeeze the card to its padding.
            .flex_shrink_0()
            .min_h(px(36.0))
            .overflow_hidden()
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
                    .debug_selector(|| format!("agent-trace-{}-icon", trace.id))
                    .mt(px(1.0))
                    .size(px(20.0))
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(4.0))
                    .bg(color.opacity(0.10))
                    .child(lucide_icon(icon, 11.0, color)),
            )
            .child(
                div()
                    .debug_selector(|| format!("agent-trace-{}-content", trace.id))
                    .flex_1()
                    .min_w(px(0.0))
                    .flex()
                    .flex_col()
                    .gap(px(2.0))
                    .child(
                        div()
                            .w_full()
                            .flex()
                            .items_center()
                            .gap(px(7.0))
                            .child(
                                div()
                                    .debug_selector(|| format!("agent-trace-{}-title", trace.id))
                                    .flex_1()
                                    .min_w(px(0.0))
                                    .whitespace_normal()
                                    .text_size(px(10.0))
                                    .line_height(relative(1.4))
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
                            })
                            .when_some(status, |row, status| {
                                let mut label = div()
                                    .flex()
                                    .items_center()
                                    .gap(px(6.0))
                                    .flex_shrink_0()
                                    .debug_selector(|| {
                                        format!("agent-operation-status-{}", trace.id)
                                    })
                                    .text_size(px(10.0))
                                    .text_color(color);
                                if animate {
                                    label = label.child(activity::running_dot(
                                        color,
                                        cx.prefers_reduced_motion(),
                                    ));
                                }
                                row.child(label.child(activity::status_label(status)))
                            }),
                    )
                    .when_some(detail, |column, detail| {
                        column.child(self.render_transcript_text(
                            transcript::TranscriptId::Trace(trace.id),
                            &detail,
                            false,
                            window,
                            cx,
                        ))
                    }),
            )
            .into_any_element()
    }

    fn render_context(&self, narrow: bool, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let locked = self.context_controls_locked();
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
        if self.ssh_sessions.preview.is_some() {
            let wide = window.viewport_size().width.to_f64() as f32
                >= px(840.0).to_pixels(window.rem_size()).to_f64() as f32;
            self.update_navigator_layout(wide, cx);
            return self.render_ssh_session_preview(cx);
        }
        if self.remote_sessions.active && self.remote_sessions.available {
            let wide = window.viewport_size().width.to_f64() as f32
                >= px(840.0).to_pixels(window.rem_size()).to_f64() as f32;
            self.update_navigator_layout(wide, cx);
            return self.render_remote_session(cx);
        }
        self.ensure_ssh_discovery(cx);
        self.ensure_native_history(cx);
        if self.surface_focus_requested
            && self.pending_confirm.is_none()
            && self.workflows.dialog.is_none()
            && self.delivery.dialog.is_none()
            && self.review.dialog.is_none()
        {
            self.surface_focus_requested = false;
            if !self.focus_visible_browser(cx) && !self.focus_visible_editor(window, cx) {
                if let Some(ui) = self
                    .sessions
                    .selected_id()
                    .and_then(|id| self.session_ui.get(&id))
                {
                    ui.prompt.read(cx).focus_handle(cx).focus(window);
                }
            }
        }
        let width = if self.tools.measured_width > 0.0 {
            self.tools.measured_width
        } else {
            window.viewport_size().width.to_f64() as f32
        };
        let wide = width >= px(840.0).to_pixels(window.rem_size()).to_f64() as f32;
        let inspector_wide = width >= px(1180.0).to_pixels(window.rem_size()).to_f64() as f32;
        self.update_navigator_layout(wide, cx);
        self.tools.inspector_wide = inspector_wide;
        let mut session = self
            .sessions
            .selected()
            .map(AgentSession::presentation_snapshot)
            .expect("cockpit retains one session");
        session.context = self.displayed_context(cx);
        if self.tools.inspector_context.as_ref() != Some(&session.context) {
            self.inspector_remote_entries.clear();
            self.inspector_remote_path = None;
            self.inspector_remote_pending_path = None;
            self.inspector_remote_session = None;
            self.inspector_remote_connection = None;
            self.inspector_remote_error = None;
            self.inspector_remote_loading = false;
            self.tools.instructions_request = Uuid::new_v4();
            self.tools.loading = false;
            self.tools.error = None;
            self.tools.instructions.clear();
            if matches!(
                self.tools.active,
                Some(tools::AgentToolSurface::Instructions)
            ) {
                self.tools.active = None;
            }
            self.tools.inspector_context = Some(session.context.clone());
            self.inspector_remote_root = None;
            self.inspector_remote_truncated = false;
            self.explorer.reset_context();
        }

        if matches!(session.context.target, AgentTarget::Local) {
            let root = PathBuf::from(&session.context.workdir);
            if self.inspector_local_browser.root() != root.as_path() {
                self.inspector_local_browser.set_root(root);
            }
        }
        if self.tools.panel_visible
            && self.tools.active.is_none()
            && self.inspector_tab == AgentInspectorTab::Files
            && self.inspector_remote_root.is_none()
            && !self.inspector_remote_loading
            && self.inspector_remote_error.is_none()
        {
            if let AgentTarget::Ssh { connection_id, .. } = session.context.target {
                self.request_remote_directory(
                    session.id,
                    connection_id,
                    session.context.workdir.clone(),
                    cx,
                );
            }
        }
        let session_id = session.id;
        let session_name = session.name.clone();
        let context = session.context.clone();
        let status = session.status;
        let attention = session.attention;
        let elapsed = session_elapsed(&session);
        let timeline = self.timeline.project(
            self.sessions.get(session_id).expect("selected session"),
            &self.session_ui[&session_id],
            self.focused_trace,
            window.rem_size(),
            self.native_history.get(&session_id),
        );
        let has_content = !timeline.is_empty() || self.timeline.virtualized;
        let context_expanded = self.context_expanded;

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
        let workdir_label = std::path::Path::new(&context.workdir)
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or(&context.workdir)
            .to_owned();
        let metadata = div()
            .h(px(46.0))
            .flex()
            .items_center()
            .flex_shrink_0()
            .min_w_0()
            .gap(px(8.0))
            .px(px(12.0))
            .border_b_1()
            .border_color(ShellDeckColors::border())
            .when(wide, |row| {
                row.child(
                    Button::new("agent-header-sessions", "")
                        .variant(ButtonVariant::Ghost)
                        .size(ButtonSize::Icon)
                        .w(px(28.0))
                        .h(px(28.0))
                        .icon(IconSource::from("messages-square"))
                        .tooltip(t!("agents.parity.toggle_sessions").to_string())
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.toggle_navigator(cx);
                        })),
                )
            })
            .child(lucide_icon(
                provider_icon(context.provider),
                14.0,
                ShellDeckColors::text_muted(),
            ))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .when(wide, |row| {
                        row.child(
                            div()
                                .max_w(px(150.0))
                                .min_w_0()
                                .truncate()
                                .text_size(px(11.0))
                                .text_color(ShellDeckColors::text_muted())
                                .child(workdir_label),
                        )
                        .child(div().text_color(ShellDeckColors::text_muted()).child("/"))
                    })
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(px(12.0))
                            .font_weight(FontWeight::MEDIUM)
                            .child(session_name),
                    ),
            )
            .child(
                div()
                    .flex_shrink_0()
                    .size(px(6.0))
                    .rounded_full()
                    .bg(status_color(status)),
            )
            .when_some(elapsed, |row, elapsed| {
                row.child(
                    div()
                        .flex_shrink_0()
                        .text_size(px(10.0))
                        .text_color(ShellDeckColors::text_muted())
                        .child(elapsed),
                )
            })
            .when(attention != AgentSessionAttention::None, |row| {
                row.child(
                    Button::new("agent-acknowledge-attention", "")
                        .variant(ButtonVariant::Ghost)
                        .size(ButtonSize::Icon)
                        .icon(IconSource::from("check"))
                        .w(px(28.0))
                        .h(px(28.0))
                        .tooltip(t!("agents.attention.acknowledge").to_string())
                        .on_click(
                            cx.listener(|this, _, _, cx| this.acknowledge_selected_attention(cx)),
                        ),
                )
            })
            .child(self.render_quick_tools(cx))
            .child(
                Button::new("agent-context-toggle", "")
                    .variant(if context_expanded {
                        ButtonVariant::Secondary
                    } else {
                        ButtonVariant::Ghost
                    })
                    .size(ButtonSize::Icon)
                    .icon(IconSource::from("settings"))
                    .w(px(28.0))
                    .h(px(28.0))
                    .tooltip(t!("agents.context.title").to_string())
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.context_expanded = !this.context_expanded;
                        cx.notify();
                    })),
            );

        let activity_frame = activity::ActivityFrame::default();
        let mut output = div()
            .id(("agent-console-output", uuid_key(session_id)))
            .debug_selector(|| "agent-timeline-scroll".into())
            .flex()
            .flex_col()
            .flex_1()
            .min_h(px(0.0))
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
            .px(if wide { px(24.0) } else { px(12.0) })
            .py(px(16.0));
        if !self.timeline.virtualized && session.remote_history.is_some() {
            output = output.child(self.render_native_history(&session, window, cx));
        }
        if !has_content && session.remote_history.is_none() {
            let subscription_setup = crate::cli_subscription_setup::render_subscription_setup(
                self.provider.binary(),
                self.selected_connection.is_some(),
            );
            let mut empty_state = div()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
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
                );
            if subscription_setup.is_some() && self.agent_setup_expanded {
                empty_state = empty_state.flex_shrink_0().py(px(16.0));
            } else {
                empty_state = empty_state.flex_1();
            }
            if subscription_setup.is_some() {
                empty_state = empty_state.child(
                    Button::new("agent-setup-toggle", t!("agents.setup.toggle").to_string())
                        .variant(ButtonVariant::Ghost)
                        .size(ButtonSize::Sm)
                        .icon(IconSource::from(if self.agent_setup_expanded {
                            "chevron-up"
                        } else {
                            "chevron-down"
                        }))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.agent_setup_expanded = !this.agent_setup_expanded;
                            cx.notify();
                        })),
                );
            }
            output = output.child(empty_state);
            if self.agent_setup_expanded {
                if let Some(setup) = subscription_setup {
                    output = output.child(div().w_full().max_w(px(520.0)).mx_auto().child(setup));
                }
            }
        } else if !self.timeline.virtualized {
            let mut timeline_view = div()
                .flex_shrink_0()
                .w_full()
                .max_w(px(800.0))
                .mx_auto()
                .flex()
                .flex_col()
                .gap(px(8.0));
            for item in &timeline {
                timeline_view = timeline_view.child(self.render_indexed_timeline_row(
                    item,
                    activity_frame.clone(),
                    true,
                    window,
                    cx,
                ));
            }
            timeline_view = timeline_view.child(self.render_timeline_footer(&session, cx));
            output = output.child(timeline_view);
            if let Some(trace_id) = self.pending_trace_reveal.take() {
                if self.focused_trace == Some(trace_id)
                    && self.sessions.get(session_id).is_some_and(|session| {
                        session.trace.iter().any(|trace| trace.id == trace_id)
                    })
                {
                    // Schedule from render: the expanded trace must first acquire
                    // its new anchor bounds during this frame's prepaint.
                    if self.timeline.virtualized {
                        self.timeline.reveal(trace_id, false);
                    } else {
                        self.focused_trace_anchor.scroll_to(window, cx);
                    }
                    window.on_next_frame(|window, _| window.refresh());
                    window.request_animation_frame();
                }
            }
        }

        if self.timeline.virtualized {
            if let Some(id) = self.pending_trace_reveal.take() {
                self.timeline.reveal(id, false);
            }
        }
        let output = if self.timeline.virtualized {
            self.render_virtual_timeline(wide, cx)
        } else {
            output.into_any_element()
        };
        let composer = self.render_composer(&session, window, cx);
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
        if let Err(error) =
            validate_agent_target(&self.displayed_context(cx).target, &self.connections)
        {
            content = content.child(
                div()
                    .px(px(14.0))
                    .py(px(8.0))
                    .text_size(px(11.0))
                    .text_color(ShellDeckColors::error())
                    .child(error),
            );
        }
        if self.tools.toolbar_expanded {
            content = content.child(self.render_workspace_tools(cx));
        }
        if let Some(error) = &self.library.error {
            content = content.child(
                div()
                    .px(px(14.0))
                    .py(px(8.0))
                    .text_size(px(11.0))
                    .text_color(ShellDeckColors::error())
                    .child(error.clone()),
            );
        }
        if context_expanded {
            content = content.child(self.render_context(!wide, window, cx));
        }
        let away = !self.timeline.at_end(&self.scroll);
        content = content.child(self.render_middle_switcher(cx));
        if self.agent_editor_visible(cx) {
            content = content.child(self.render_middle_editor(cx));
        } else {
            content = content.child(self.render_activity_viewport(
                session_id,
                output,
                activity_frame,
                window,
                cx,
            ));
            if away {
                content = content.child(
                    div().flex().justify_center().flex_shrink_0().child(
                        Button::new("agent-jump-latest", t!("agents.parity.latest").to_string())
                            .debug_selector(|| "agent-jump-latest".into())
                            .variant(ButtonVariant::Secondary)
                            .size(ButtonSize::Sm)
                            .h(px(28.0))
                            .icon(IconSource::from("arrow-down"))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.timeline.pin(&this.scroll);
                                cx.notify();
                            })),
                    ),
                );
            }
            content = content.child(
                div()
                    .debug_selector(|| "agent-composer-shell".into())
                    .flex_shrink_0()
                    .w_full()
                    .px(if wide { px(18.0) } else { px(10.0) })
                    .pt(px(7.0))
                    .pb(px(11.0))
                    .child(div().w_full().max_w(px(800.0)).mx_auto().child(composer)),
            );
        }
        let mut root = div()
            .id("agent-cockpit")
            .key_context("AgentConsole")
            .on_action(cx.listener(|this, _: &AgentNewChat, _, cx| this.new_session(cx)))
            .on_action(cx.listener(|this, _: &AgentFocusComposer, window, cx| {
                this.show_agent_conversation(cx);
                if let Some(id) = this.sessions.selected_id() {
                    if let Some(ui) = this.session_ui.get(&id) {
                        ui.prompt.read(cx).focus_handle(cx).focus(window);
                    }
                }
            }))
            .on_action(cx.listener(|this, _: &AgentToggleFiles, _, cx| {
                this.tools.panel_visible = !this.tools.panel_visible;
                this.schedule_library_save(cx);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &AgentToggleSessions, _, cx| {
                this.toggle_navigator(cx);
            }))
            .on_action(cx.listener(|this, _: &AgentJumpLatest, _, cx| {
                this.timeline.pin(&this.scroll);
                cx.notify();
            }))
            .relative()
            .flex()
            .size_full()
            .min_h(px(0.0))
            .bg(ShellDeckColors::bg_primary())
            .text_color(ShellDeckColors::text_primary());
        if wide && self.tools.navigator_visible {
            root = root.child(self.render_navigator(cx));
        }
        if self.live_browser_visible(cx) {
            let available_width = width / window.rem_size().to_f64() as f32 * 16.0
                - if wide && self.tools.navigator_visible {
                    f32::from(self.library.preferences.navigator_width)
                } else {
                    0.0
                };
            root = root.child(
                div()
                    .flex_1()
                    .h_full()
                    .min_w_0()
                    .min_h_0()
                    .child(self.render_browser_split(content, available_width, cx)),
            );
        } else {
            root = root.child(content);
        }
        if self.tools.panel_visible {
            let panel = if self.tools.active.is_some() {
                self.render_tools_panel(&session, cx)
            } else if self.inspector_tab == AgentInspectorTab::Changes {
                self.render_review_panel(&session, cx)
            } else {
                self.render_inspector(&session, cx)
            };
            let width = self
                .tools
                .panel_width
                .min((width / window.rem_size().to_f64() as f32 * 16.0 - 80.0).max(240.0));
            let panel = div()
                .w(px(width))
                .h_full()
                .flex_shrink_0()
                .min_h_0()
                .border_l_1()
                .border_color(ShellDeckColors::border())
                .bg(ShellDeckColors::bg_sidebar())
                .child(panel);
            if inspector_wide {
                root = root.child(panel);
            } else {
                root = root.child(
                    div()
                        .absolute()
                        .right_0()
                        .top_0()
                        .bottom_0()
                        .occlude()
                        .child(panel),
                );
            }
        }
        if !wide && self.tools.navigator_overlay {
            root = root.child(self.render_navigator_overlay(cx));
        }
        if let Some(dialog) = self.render_workflow_dialog() {
            root = root.child(dialog);
        }
        if let Some(dialog) = self.render_delivery_dialog() {
            root = root.child(dialog);
        }
        if let Some(dialog) = self.render_review_dialog() {
            root = root.child(dialog);
        }
        if let Some(overlay) = self.render_composer_overlay() {
            root = root.child(overlay);
        }
        if let Some(pending) = self.pending_confirm.as_ref() {
            root = root.child(self.render_confirmation(pending, cx));
        }
        let measure = cx.entity().downgrade();
        root = root.child(
            canvas(
                move |bounds, _, cx| {
                    let width = bounds.size.width.to_f64() as f32;
                    // Layout is still being painted here. Invalidate afterward
                    // so a resize schedules the responsive frame without input.
                    cx.defer(move |cx| {
                        let _ = measure.update(cx, |this, cx| {
                            if (this.tools.measured_width - width).abs() > 1.0 {
                                this.tools.measured_width = width;
                                cx.notify();
                            }
                        });
                    });
                },
                |_, _, _, _| {},
            )
            .absolute()
            .size_full(),
        );
        root.into_any_element()
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

fn trace_is_visible(session: &AgentSession, trace: &AgentTraceEvent) -> bool {
    let provider = trace
        .turn_id
        .and_then(|id| session.turns.iter().find(|turn| turn.id == id))
        .map(|turn| turn.context.provider);
    // Hide bookkeeping retained by older Jcode parsers without deleting history.
    !(matches!(
        provider,
        Some(AgentProvider::Jcode | AgentProvider::DeepSeek)
    ) && matches!(&trace.detail, AgentTraceKind::Activity { label } if matches!(label.as_str(), "start" | "connection type" | "connection phase" | "message end" | "tokens" | "upstream provider" | "session" | "tool input")))
}

fn merged_timeline(session: &AgentSession) -> Vec<TimelineItem> {
    let mut timeline = Vec::with_capacity(session.messages.len() + session.trace.len());
    timeline.extend(session.messages.iter().cloned().map(TimelineItem::Message));
    timeline.extend(
        session
            .trace
            .iter()
            .filter(|trace| trace_is_visible(session, trace))
            .cloned()
            .map(TimelineItem::Trace),
    );
    timeline.extend(
        session
            .boundaries
            .iter()
            .cloned()
            .map(TimelineItem::Boundary),
    );
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
        .filter(|session| !session.archived && session_worktree_matches(session, worktree))
        .max_by_key(|session| {
            (
                session.context.workdir.trim_end_matches('/')
                    == worktree.path.trim_end_matches('/'),
                session.updated_at_ms,
                session.id,
            )
        })
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

fn session_status_label(status: AgentSessionStatus) -> String {
    t!(match status {
        AgentSessionStatus::Idle => "agents.session_state.ready",
        AgentSessionStatus::Starting => "agents.session_state.starting",
        AgentSessionStatus::Running => "agents.session_state.running",
        AgentSessionStatus::Stopping => "agents.session_state.stopping",
        AgentSessionStatus::Completed => "agents.session_state.completed",
        AgentSessionStatus::Failed => "agents.session_state.failed",
        AgentSessionStatus::Cancelled => "agents.session_state.cancelled",
    })
    .to_string()
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
            status,
        } => (
            "search",
            trace_status_color(*status),
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
            status,
        } => (
            "git-branch",
            trace_status_color(*status),
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
    // SDTEST-2801 — clean legacy Jcode bookkeeping without hiding real work or other providers.
    #[test]
    fn legacy_jcode_timeline_hides_only_known_transport_rows() {
        use super::{merged_timeline, AgentStreamEvent};
        let mut session = AgentSession::new(
            "Legacy",
            AgentExecutionContext {
                provider: AgentProvider::DeepSeek,
                target: AgentTarget::Local,
                access: AgentAccessMode::ReadOnly,
                workdir: std::env::temp_dir().display().to_string(),
                model: None,
            },
            0,
        )
        .unwrap();
        session.begin_run("hello", 1).unwrap();
        session.apply_stream_event(
            AgentStreamEvent::Activity("connection phase".into()),
            2,
            true,
        );
        session.apply_stream_event(
            AgentStreamEvent::Activity("Compacting context".into()),
            3,
            true,
        );
        assert_eq!(merged_timeline(&session).len(), 2); // user message and real activity
        assert_eq!(session.trace.len(), 2); // retained diagnostics are untouched
        session.turns.last_mut().unwrap().context.provider = AgentProvider::Claude;
        assert_eq!(merged_timeline(&session).len(), 3);
    }

    // SDTEST-2795 — failed admission releases inputs and unlocks a new attempt.
    #[test]
    fn rejected_chat_run_releases_options_and_retains_conversation() {
        use super::*;
        use gpui::{AppContext, TestAppContext};
        let mut app = TestAppContext::single();
        let view = app.update(|cx| cx.new(AgentConsoleView::new));
        view.update(&mut app, |view, cx| {
            let id = view.sessions.selected_id().unwrap();
            let request = view.sessions.begin_run(id, "Keep this request", 1).unwrap();
            view.run_sessions.insert(request.id, id);
            view.set_run_options(
                request.id,
                shelldeck_core::agent_runtime::AgentInputOptions {
                    image_paths: vec![PathBuf::from("/fixture/attachment.png")],
                    ..Default::default()
                },
            );
            view.reject_run_for(request.id, "Provider not installed".into(), cx);
            assert!(view.take_run_options(request.id).image_paths.is_empty());
            assert!(!view.context_controls_locked());
            assert_eq!(
                view.sessions.get(id).unwrap().status,
                AgentSessionStatus::Failed
            );
            assert!(view
                .sessions
                .get(id)
                .unwrap()
                .messages
                .iter()
                .any(|m| m.text == "Keep this request"));
            assert!(view.sessions.begin_run(id, "Try again", 2).is_ok());
        });
    }

    // SDTEST-2794 — batched/background output stays with its run; late output
    // cannot revive a finished turn or contaminate the visible conversation.
    #[test]
    fn batched_chat_events_preserve_session_identity_and_terminal_fence() {
        use super::*;
        use gpui::{AppContext, TestAppContext};
        let mut app = TestAppContext::single();
        let view = app.update(|cx| cx.new(AgentConsoleView::new));
        view.update(&mut app, |view, cx| {
            let visible = view.sessions.selected_id().unwrap();
            let context = view.sessions.get(visible).unwrap().context.clone();
            let background = view.sessions.create("Background", context, 1).unwrap();
            view.sessions.select(visible).unwrap();
            view.sessions.set_surface_visible(true);
            let request = view.sessions.begin_run(background, "hello", 2).unwrap();
            view.run_sessions.insert(request.id, background);
            view.push_stream_events_for(
                request.id,
                [
                    AgentStreamEvent::Text("Hello".into()),
                    AgentStreamEvent::TextDelta(" there".into()),
                    AgentStreamEvent::TextDelta("!".into()),
                ],
                cx,
            );
            assert!(view.sessions.get(visible).unwrap().messages.is_empty());
            let session = view.sessions.get(background).unwrap();
            assert_eq!(session.messages.last().unwrap().text, "Hello there!");
            assert!(session.unread_count > 0);
            view.finish_run(request.id, None, None, cx);
            let messages = view.sessions.get(background).unwrap().messages.len();
            view.push_stream_events_for(
                request.id,
                [AgentStreamEvent::TextDelta("late".into())],
                cx,
            );
            let session = view.sessions.get(background).unwrap();
            assert_eq!(session.status, AgentSessionStatus::Cancelled);
            assert_eq!(session.messages.len(), messages);
            assert_eq!(view.sessions.selected_id(), Some(visible));
        });
    }

    // SDTEST-2734 — disappeared SSH identities remain SSH across refresh,
    // selection display and controls; only explicit selection can change target.
    #[test]
    fn missing_ssh_refresh_never_retargets_controls_or_display_to_local() {
        use gpui::{AppContext, TestAppContext};
        let mut app = TestAppContext::single();
        let view = app.update(|cx| cx.new(super::AgentConsoleView::new));
        view.update(&mut app, |view, cx| {
            let id = uuid::Uuid::new_v4();
            let connection = super::AgentConnectionOption { id, label: "Saved SSH".into(), host: "fixture.invalid".into() };
            view.set_connections(vec![connection.clone()], cx);
            view.selected_connection = Some(id);
            view.workdir_state.update(cx, |state, cx| state.replace_content("/srv/project", cx));
            let context = view.context_from_controls(cx).unwrap();
            let selected = view.sessions.selected_id().unwrap();
            view.sessions.get_mut(selected).unwrap().set_context(context, 1).unwrap();
            view.set_connections(Vec::new(), cx);
            assert_eq!(view.selected_connection, Some(id));
            assert!(matches!(view.displayed_context(cx).target, AgentTarget::Ssh { connection_id, .. } if connection_id == id));
            assert!(view.context_from_controls(cx).is_err());
            let (options, index) = super::agent_target_options(&view.connections, view.selected_connection);
            assert_eq!(options[index.unwrap()].value, Some(id));
            let replacement = super::AgentConnectionOption { id: uuid::Uuid::new_v4(), ..connection };
            view.set_connections(vec![replacement.clone()], cx);
            assert_eq!(view.selected_connection, Some(id));
            assert!(view.context_from_controls(cx).is_err());
            view.selected_connection = Some(replacement.id); // explicit dropdown choice
            assert!(matches!(view.context_from_controls(cx).unwrap().target, AgentTarget::Ssh { connection_id, .. } if connection_id == replacement.id));
        });
    }

    // SDTEST-2735 — immutable bound context and pending confirmation cannot
    // bypass target membership after the saved SSH identity disappears.
    #[test]
    fn missing_bound_ssh_blocks_pending_run_without_mutating_its_context() {
        use gpui::{AppContext, TestAppContext};
        let mut app = TestAppContext::single();
        let view = app.update(|cx| cx.new(super::AgentConsoleView::new));
        view.update(&mut app, |view, cx| {
            let id = view.sessions.selected_id().unwrap();
            let context = AgentExecutionContext {
                provider: AgentProvider::Claude,
                access: AgentAccessMode::ReadOnly,
                target: AgentTarget::Ssh {
                    connection_id: uuid::Uuid::new_v4(),
                    connection_label: "Missing bound SSH".into(),
                },
                workdir: "/srv/bound".into(),
                model: None,
            };
            view.sessions
                .get_mut(id)
                .unwrap()
                .set_context(context.clone(), 1)
                .unwrap();
            view.context_locks.insert(id);
            view.apply_selected_context(cx);
            view.set_connections(Vec::new(), cx);
            assert!(view.context_controls_locked());
            assert_eq!(view.displayed_context(cx), context);
            assert!(view.context_from_controls(cx).is_err());
            view.start_pending(
                super::PendingRun {
                    session_id: id,
                    context: context.clone(),
                    prompt: "Never launch locally".into(),
                    options: Default::default(),
                    source_draft: None,
                },
                cx,
            );
            assert_eq!(view.sessions.get(id).unwrap().context, context);
            assert!(!view.sessions.get(id).unwrap().status.is_active());
            assert!(view.run_sessions.is_empty());
            assert_eq!(
                view.sessions
                    .get(id)
                    .unwrap()
                    .messages
                    .last()
                    .map(|message| message.text.as_str()),
                Some(crate::t!("agents.error.target_missing").as_ref())
            );
        });
    }

    use super::{
        context_for_worktree, inspector_files, session_name, target_switch_workdir,
        terminal_session_status, trace_presentation, workdir_is_valid_for_target,
        worktree_bound_session, AgentWorktreeOption,
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
                turn_id: None,
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
                turn_id: None,
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
                turn_id: None,
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
    }
}
