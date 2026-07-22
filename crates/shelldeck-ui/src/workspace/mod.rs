use crate::icons::{ai_provider_badge, lucide_icon, lucide_path};
use adabraka_ui::components::icon_source::IconSource;
use adabraka_ui::components::input::{Input, InputSize, InputState};
use adabraka_ui::overlays::sheet::{Sheet, SheetSize, SheetVariant};
use adabraka_ui::prelude::{
    install_theme, scrollable_vertical, use_theme, AnimatedCollapsible, Button, ButtonSize,
    ButtonVariant, Spinner, SpinnerSize, SpinnerVariant,
};
use gpui::prelude::*;
use gpui::*;
use shelldeck_core::ai::{
    ai_action_disposition, configured_cli_available, create_client, host_context,
    parse_diagnostic_plan, parse_generated_issue_draft, parse_generated_name,
    parse_issue_triage_proposal, test_connection, validate_diagnostic_command, AiActionDisposition,
    AiActionKind, AiActionPayload, AiActionPlan, AiActionPlanSpec, AiActionRisk, AiContext,
    AiIssueTriageProposal, AiSurface, AiTask, AiTaskStatus, AiTaskStore,
};
use shelldeck_core::config::activity::{
    ActivityAction, ActivityEntry, ActivityKind, ActivityStore,
};
use shelldeck_core::config::app_config::{AppConfig, ThemePreference};
use shelldeck_core::config::bext_cloud::{self, BextCloudConfig};
use shelldeck_core::config::bext_instance;
use shelldeck_core::config::cloud_account::{self, AccountInfo, AppMode};
use shelldeck_core::config::deep_link::DeepLink;
use shelldeck_core::config::issues::{self, Issue, IssueInstance};
use shelldeck_core::config::jean_fleet::{
    self, ClaudeExecutor, FleetSnapshot, JeanInstance, JeanJob, RegisterInstance,
};
use shelldeck_core::config::jeanclaude::{self, JeanConfig, JeanState};
use shelldeck_core::config::manage_sites::{self, ManagedSiteInfo, SitesPayload};
use shelldeck_core::config::manage_support;
use shelldeck_core::config::store::ConnectionStore;
use shelldeck_core::config::themes::TerminalTheme;
use shelldeck_core::models::connection::{Connection, ConnectionSource, ConnectionStatus};
use shelldeck_ssh::tunnel::TunnelHandle;
use std::collections::{HashMap, VecDeque};
use std::ops::{DerefMut, Range};
use uuid::Uuid;

use crate::ai_action_dialog::render_ai_action_dialog;
use crate::ai_assistant::{AiAssistantEvent, AiAssistantView};
use crate::ai_workflow::{
    AiNamingKind, AiWorkflowEvent, AiWorkflowInit, AiWorkflowTarget, AiWorkflowView,
};
use crate::bext_cloud_view::{BextCloudView, BextViewEvent};
use crate::command_palette::{
    ApplyAppTheme, ApplyTerminalTheme, CommandPalette, CommandPaletteEvent, OpenManageArea,
    PaletteAction, SetAppMode, ToggleCommandPalette,
};
use crate::connection_form::{ConnectionForm, ConnectionFormEvent};
use crate::dashboard::{DashboardEvent, DashboardView};
use crate::file_editor::view::{FileEditorEvent, FileEditorView};
use crate::fleet_view::{FleetView, FleetViewEvent};
use crate::issue_attachments::{capture_region, draft_from_clipboard_image, AttachmentDraft};
use crate::jean_view::{JeanView, JeanViewEvent};
use crate::login_form::{LoginForm, LoginFormEvent};
use crate::onboarding_view::{OnboardingEvent, OnboardingView};
use crate::port_forward_form::PortForwardForm;
use crate::port_forward_view::{PortForwardEvent, PortForwardView};
use crate::recent_view::{RecentEvent, RecentView};
use crate::script_editor::{ScriptEditorView, ScriptEvent};
use crate::script_form::ScriptForm;
use crate::server_sync_view::{ServerSyncEvent, ServerSyncView};
use crate::settings::{SettingsEvent, SettingsView};
use crate::sidebar::{SidebarEvent, SidebarSection, SidebarView};
use crate::sites_view::{SitesEvent, SitesView};
use crate::status_bar::{StatusBar, StatusBarEvent};
use crate::support_view::{
    issue_status_badge, priority_badge, render_issue_delete_dialog, SupportView, SupportViewEvent,
};
use crate::t;
use crate::template_browser::TemplateBrowser;
use crate::terminal_view::{TerminalEvent, TerminalView};
use crate::theme::ShellDeckColors;
use crate::toast::{ToastContainer, ToastLevel};
use crate::variable_prompt::VariablePrompt;
use shelldeck_update::{AutoUpdateEvent, AutoUpdateStatus, AutoUpdater};

mod discovery;
mod forwards;
mod scripts;
mod server_sync;
mod ssh;

/// Health of the signed-in cloud account, surfaced as the titlebar status dot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccountStatus {
    /// Not yet checked this session (or logged out).
    Unknown,
    /// whoami succeeded — token valid.
    Ok,
    /// Token invalid/revoked — needs re-auth.
    Rejected,
    /// whoami failed on a network error — can't tell.
    Offline,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IssueAttachmentTarget {
    NewRequest,
    Comment,
}

impl AccountStatus {
    fn dot_color(self) -> Hsla {
        match self {
            AccountStatus::Ok => ShellDeckColors::success(),
            AccountStatus::Rejected => ShellDeckColors::error(),
            AccountStatus::Unknown | AccountStatus::Offline => ShellDeckColors::text_muted(),
        }
    }
}

struct WorkspaceTooltip {
    label: SharedString,
}

impl Render for WorkspaceTooltip {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .px(px(8.0))
            .py(px(4.0))
            .rounded(px(4.0))
            .border_1()
            .border_color(ShellDeckColors::border())
            .bg(ShellDeckColors::bg_surface())
            .shadow_md()
            .text_size(px(11.0))
            .font_family(use_theme().tokens.font_family.clone())
            .text_color(ShellDeckColors::text_primary())
            .whitespace_nowrap()
            .child(self.label.clone())
    }
}

/// The three tabs of the User-mode home. `Sites` is the default (matches
/// the pre-tabs layout), `Demandes` migrates the previous inline list into
/// its own surface, and `Infos` is the new "quel compte / quel device"
/// summary — surfaces every field the `/whoami` payload returns so the
/// user can see exactly what ShellDeck knows about them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum UserHomeTab {
    #[default]
    Sites,
    Requests,
    Infos,
}

/// Duration (ms) of the User-mode sheet enter/exit animation. The close
/// handlers use it to keep the sheet mounted while the exit tween plays,
/// then clear the backing state.
const SHEET_ANIM_MS: u64 = 300;

/// Everything the runtime tick needs, gathered on the UI thread then moved into
/// the background executor (all owned + `Send`).
struct RuntimeTickCtx {
    base: String,
    token: String,
    instance_id: String,
    workdir: String,
    model: String,
    autonomy: String,
    version: String,
}

/// One decision of the runtime loop, produced on the UI thread.
enum RuntimeStep {
    /// (base, token, register payload)
    Register(String, String, RegisterInstance),
    /// (base, token, instance id, version) — heartbeat only (a job is busy).
    HeartbeatOnly(String, String, String, String),
    /// Heartbeat + claim (+ auto-execute).
    Tick(RuntimeTickCtx),
}

/// The active content view
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActiveView {
    Dashboard,
    Terminal,
    Scripts,
    PortForwards,
    ServerSync,
    Sites,
    Recent,
    FileEditor,
    JeanConsole,
    Fleet,
    BextCloud,
    Settings,
}

// Actions for keyboard shortcuts
actions!(
    shelldeck,
    [
        OpenQuickConnect,
        NewTerminal,
        CloseTab,
        ToggleSidebar,
        OpenSettings,
        NextTab,
        PrevTab,
        Quit,
        OpenTemplateBrowser,
        NewScript,
        OpenServerSync,
        OpenSites,
        OpenRecent,
        OpenFileEditorView,
        CloudSyncNow,
        SwitchSite,
        OpenJeanConsole,
        JeanTogglePause,
        OpenFleet,
        ToggleJeanRuntime,
        NewRequest,
        OpenSupportRequests,
        OpenBextCloud,
        ConnectBextCloud,
        OpenAiAssistant,
    ]
);

/// Tracks a running tunnel: the handle to stop it, plus the join handle for the
/// background thread that owns the tokio runtime driving the tunnel.
struct ActiveTunnel {
    tunnel_handle: TunnelHandle,
    /// Dropping the JoinHandle does NOT abort the thread -- we use the
    /// TunnelHandle's shutdown channel for that. We keep this so we can
    /// optionally join on cleanup.
    _thread: std::thread::JoinHandle<()>,
}

/// Tracks a running script execution with a cancellation channel.
struct ActiveScript {
    shutdown_tx: tokio::sync::mpsc::Sender<()>,
    _thread: Option<std::thread::JoinHandle<()>>,
}

impl ActiveScript {
    fn stop(&self) {
        let _ = self.shutdown_tx.try_send(());
    }
}

struct AiDiagnosticSequence {
    target: AiWorkflowTarget,
    remaining: VecDeque<String>,
}

pub struct Workspace {
    connections: Vec<Connection>,
    store: ConnectionStore,
    sidebar: Entity<SidebarView>,
    dashboard: Entity<DashboardView>,
    terminal: Entity<TerminalView>,
    scripts: Entity<ScriptEditorView>,
    port_forwards: Entity<PortForwardView>,
    server_sync: Entity<ServerSyncView>,
    sites: Entity<SitesView>,
    recent: Entity<RecentView>,
    file_editor: Entity<FileEditorView>,
    settings: Entity<SettingsView>,
    ai_assistant: Entity<AiAssistantView>,
    ai_dock_assistant: Entity<AiAssistantView>,
    ai_sheet: Option<Entity<Sheet>>,
    ai_workflow: Option<Entity<AiWorkflowView>>,
    ai_workflow_sheet: Option<Entity<Sheet>>,
    ai_tasks: Vec<AiTask>,
    ai_action_confirmation: Option<AiActionPlan>,
    ai_script_runs: HashMap<Uuid, AiActionPlan>,
    ai_terminal_runs: HashMap<Uuid, AiActionPlan>,
    ai_diagnostic_sequences: HashMap<Uuid, AiDiagnosticSequence>,
    status_bar: Entity<StatusBar>,
    command_palette: Entity<CommandPalette>,
    companion_command_palette: Entity<CommandPalette>,
    toasts: Entity<ToastContainer>,
    connection_form: Option<Entity<ConnectionForm>>,
    login_form: Option<Entity<LoginForm>>,
    onboarding: Option<Entity<OnboardingView>>,
    port_forward_form: Option<Entity<PortForwardForm>>,
    script_form: Option<Entity<ScriptForm>>,
    template_browser: Option<Entity<TemplateBrowser>>,
    variable_prompt: Option<Entity<VariablePrompt>>,
    active_view: ActiveView,
    sidebar_visible: bool,
    sidebar_width: f32,
    /// Application UI font family ("System Default" means no override).
    ui_font_family: String,
    /// Application UI base font size in pixels.
    ui_font_size: f32,
    window_active: bool,
    /// Newest-first durable activity cache, mirrored to Dashboard + RecentView.
    recent_activity: Vec<ActivityEntry>,
    pub focus_handle: FocusHandle,
    /// Active tunnels keyed by the PortForward model ID (not the TunnelHandle internal id).
    active_tunnels: HashMap<Uuid, ActiveTunnel>,
    /// Active script executions keyed by script ID.
    active_scripts: HashMap<Uuid, ActiveScript>,
    // Keep subscriptions alive
    _sidebar_sub: Subscription,
    _terminal_sub: Subscription,
    _palette_sub: Subscription,
    _companion_palette_sub: Subscription,
    _settings_sub: Subscription,
    _ai_assistant_sub: Subscription,
    _ai_dock_assistant_sub: Subscription,
    _ai_workflow_sub: Option<Subscription>,
    _scripts_sub: Subscription,
    _forwards_sub: Subscription,
    _server_sync_sub: Subscription,
    _sites_sub: Subscription,
    _recent_sub: Subscription,
    _file_editor_sub: Subscription,
    _form_sub: Option<Subscription>,
    _pf_form_sub: Option<Subscription>,
    _dashboard_sub: Subscription,
    _script_form_sub: Option<Subscription>,
    _template_browser_sub: Option<Subscription>,
    _variable_prompt_sub: Option<Subscription>,
    _git_poll_task: Option<gpui::Task<()>>,
    auto_updater: Entity<AutoUpdater>,
    _update_sub: Subscription,
    _status_bar_sub: Subscription,
    /// Connection ID pending deletion (requires second click to confirm).
    pending_delete: Option<Uuid>,
    /// In-memory copy of the loaded application config. Kept in sync on
    /// `ConfigChanged` so runtime behavior reads the *current* values.
    app_config: AppConfig,
    /// True once the user has been warned about closing with active sessions;
    /// the next close attempt is allowed through (two-step confirm).
    pending_close_confirm: bool,
    /// Whether the titlebar theme-switcher dropdown is open.
    theme_menu_open: bool,
    /// Whether the titlebar account dropdown is open.
    account_menu_open: bool,
    /// Health of the signed-in cloud account (drives the status dot).
    account_status: AccountStatus,
    /// Kept alive while the login modal is open.
    _login_form_sub: Option<Subscription>,
    /// Kept alive while the onboarding tour is open.
    _onboarding_sub: Option<Subscription>,
    /// Cached full whoami response — kept in memory so the User-mode
    /// "Mes informations" tab can surface every field the server sends
    /// (device label, created_at, last_seen_at, …), not just the three
    /// bits `AccountInfo` persists. Refreshed by `check_account_on_startup`
    /// and set on login. Cleared on logout.
    last_whoami: Option<cloud_account::WhoamiInfo>,
    /// Which User-mode home tab is showing (Sites / Demandes / Infos).
    user_home_tab: UserHomeTab,
    /// Cached Inklura Manage sites directory + areas (fetched after sign-in).
    site_directory: Option<SitesPayload>,
    /// Whether the titlebar site-switcher dropdown is open.
    site_menu_open: bool,
    /// Kebab menu open state for a sidebar host row: which connection and where
    /// (window-relative click position). `None` = closed.
    sidebar_kebab_menu: Option<(Uuid, Point<Pixels>)>,
    /// The native Support-mode console.
    support: Entity<SupportView>,
    _support_sub: Subscription,
    /// Background poll while Support mode is visible.
    _support_poll_task: Option<gpui::Task<()>>,
    /// The JeanClaude console (Dev mode).
    jean_view: Entity<JeanView>,
    _jean_sub: Subscription,
    /// Shared `/api/state` cache (feeds jean_view + the Support strip + User card).
    jean_state: Option<JeanState>,
    /// Background poll while a Jean surface is visible.
    _jean_poll_task: Option<gpui::Task<()>>,
    /// User-mode "Demander à JeanClaude" composer buffer + focus.
    jean_ask_input: String,
    jean_ask_focus: FocusHandle,
    /// The Jean fleet view (Dev mode).
    fleet_view: Entity<FleetView>,
    _fleet_sub: Subscription,
    /// Cached fleet snapshot (feeds fleet_view).
    fleet_snapshot: Option<FleetSnapshot>,
    /// Exact Fleet job requested by a deep link, retained across async refresh.
    pending_fleet_job_focus: Option<String>,
    /// Poll while the Fleet view is visible.
    _fleet_view_poll: Option<gpui::Task<()>>,
    /// This machine's registered runtime instance (when the runtime is enabled).
    runtime_instance: Option<JeanInstance>,
    /// Jobs claimed by a `confirm`-autonomy instance, awaiting an explicit
    /// "Exécuter" in the UI. Also gates the loop (concurrency 1).
    runtime_awaiting: Vec<JeanJob>,
    /// True while a job is executing or awaiting confirmation (no new claim).
    runtime_busy: bool,
    /// The register/heartbeat/claim/execute loop (only while enabled + signed in).
    _runtime_loop: Option<gpui::Task<()>>,
    /// Hosted issue-management (requests) cache — shared by User + Support.
    issues_list: Vec<Issue>,
    issues_staff: bool,
    /// Server-side filter state passed to `issues::list_issues` on every
    /// refresh. Fed by `SupportViewEvent::IssuesFilterChanged` — the
    /// SupportView owns the UI state, we cache the values here so the
    /// 15s poll re-uses the current filter instead of resetting to "all".
    issues_filter: issues::IssueListFilter,
    issues_instances: Vec<IssueInstance>,
    issue_detail: Option<Issue>,
    issue_selected: Option<String>,
    /// Request id pending a confirmed soft-delete from the User-mode detail
    /// sheet (drives a confirm modal — owner-or-staff may delete).
    confirm_issue_delete: Option<String>,
    _issues_poll: Option<gpui::Task<()>>,
    /// User-mode "Nouvelle demande" + comment composer states — each hosts
    /// an adabraka `Input` widget (real cursor, selection, undo). Focus is
    /// tracked by each state entity itself; no separate `issue_field` needed.
    /// `issue_body_state` runs in multi-line mode (`Input::multi_line(true)`
    /// via SDPATCH-009) so Détails behaves as a textarea.
    issue_title_state: Entity<InputState>,
    issue_body_state: Entity<InputState>,
    issue_comment_state: Entity<InputState>,
    issue_attachment_url_state: Entity<InputState>,
    issue_new_attachments: Vec<AttachmentDraft>,
    issue_comment_attachments: Vec<AttachmentDraft>,
    issue_attachment_busy: bool,
    issue_attachment_generation: u64,
    issue_ai_prompt_state: Entity<InputState>,
    issue_ai_expanded: bool,
    issue_ai_loading: bool,
    issue_ai_error: Option<String>,
    issue_ai_request_id: u64,
    issue_new_priority: String,
    issue_new_source: &'static str,
    /// User-home "Mes sites" search — filters the compact rows client-side
    /// by label + host + tenant_name. The query is read live from the input
    /// state at render time (same pattern as `SupportView::search_query` —
    /// adabraka `on_change` only fires on programmatic `set_value`, not on
    /// user keystrokes).
    user_sites_search_state: Entity<InputState>,
    /// User-mode: "Nouvelle demande" sheet visibility. The composer used to be
    /// always-visible at the top of `render_user_requests`; it now lives in a
    /// right-side sheet, toggled by the "Nouvelle demande" button in the list
    /// header.
    user_new_request_sheet_open: bool,
    /// While `true` the composer sheet plays its slide-out/fade-out animation.
    /// Cleared (along with `..open`) by a delayed task the close handler spawns.
    user_new_request_sheet_dismissing: bool,
    /// Same for the selected-request detail sheet.
    user_issue_detail_dismissing: bool,
    /// The Dev-mode "bext Cloud" view.
    bext_view: Entity<BextCloudView>,
    _bext_sub: Subscription,
    /// Cached cloud whoami (drives super-admin instances + identity).
    bext_user: Option<bext_cloud::CloudUser>,
    _bext_poll: Option<gpui::Task<()>>,
    /// While the command palette is previewing an app theme, the theme to
    /// restore if the user dismisses without committing. `None` when no preview
    /// is active.
    theme_before_preview: Option<ThemePreference>,
    /// Same idea for a previewed terminal color theme: the terminal theme name
    /// to restore if the palette is dismissed without committing.
    terminal_theme_before_preview: Option<String>,
    /// Optional publisher into the system-tray state channel. Set once
    /// at startup by `main.rs` after `TrayService::new` returns; `None`
    /// when the tray failed to come up (Flatpak sandbox, missing GTK,
    /// etc.) so publishes become no-ops rather than crashing.
    ///
    /// Uses a boxed `Fn` instead of the raw `std::sync::mpsc::Sender`
    /// so `shelldeck-ui` stays independent of the `tray-icon` crate —
    /// the `main.rs` closure keeps the sender internally.
    tray_state_publisher: Option<Box<dyn Fn(TrayCounters) + Send + Sync>>,
    /// Optional OS-notification dispatcher. Same "closure supplied by
    /// `main.rs`" pattern as `tray_state_publisher` so `shelldeck-ui`
    /// stays independent of `notify-rust`. Called from
    /// `publish_tray_state` on positive deltas and from
    /// `apply_tick_result` on Fleet job completion.
    tray_notifier: Option<Box<dyn Fn(TrayNotification) + Send + Sync>>,
    /// Previous tray counters, kept for delta detection. `None` before
    /// the first publish — the first publish seeds the value without
    /// firing notifications so a fresh app launch with pre-existing
    /// unread tickets doesn't dump a spurious "N nouveaux tickets"
    /// notification on startup.
    last_tray_counters: Option<TrayCounters>,
}

/// Snapshot mirror of `shelldeck::tray::TrayState`, kept in
/// `shelldeck-ui` to avoid a dependency on the `shelldeck` binary
/// crate. The `main.rs`-side closure translates one into the other.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TrayCounters {
    pub active_ssh: usize,
    pub open_tunnels: usize,
    pub unread_tickets: usize,
    pub jean_pending: usize,
    pub pinned_connections: Vec<TrayPinnedConnection>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrayPinnedConnection {
    pub id: Uuid,
    pub name: String,
}

/// Notifications the workspace asks the OS to display when a
/// user-relevant delta happens (new ticket arrived, Jean job needs a
/// human, SSH session dropped, Fleet job finished). `main.rs` wires
/// this to `notify-rust`; other UIs (headless tests, mock harness) can
/// stub the notifier with a no-op or a spy.
#[derive(Debug, Clone)]
pub enum TrayNotification {
    /// N new unread support tickets appeared since the last publish.
    NewTickets { count: usize },
    /// N new Jean fleet jobs are awaiting user confirmation.
    JeanPending { count: usize },
    /// N previously-active SSH sessions dropped since the last publish.
    /// Coarse — we don't know *which* host from the counter alone;
    /// finer notifications would need per-session hooks.
    SshDisconnected { count: usize },
    /// A Fleet job finished. `success = false` means the executor
    /// returned a non-zero exit or an error surfaced to the toast.
    FleetJobDone { success: bool },
    /// An AI generation or executable action finished while the main window
    /// was not active.
    AiTaskDone { success: bool },
}

impl Workspace {
    pub fn new(
        cx: &mut Context<Self>,
        config: AppConfig,
        connections: Vec<Connection>,
        store: ConnectionStore,
    ) -> Self {
        crate::i18n::apply_ui_language(&config.general.ui_language);

        // Restore the persisted active-site filter (if any) so the sidebar
        // opens scoped to the last-selected site.
        let initial_site_filter = config
            .cloud_sync
            .active_site_id
            .as_deref()
            .and_then(|s| Uuid::parse_str(s).ok());
        let initial_nav_collapsed = config.general.sidebar_nav_collapsed;
        let initial_pinned_connections = config.pinned_connections.clone();
        let initial_dashboard_pins = initial_pinned_connections.clone();
        let sidebar = cx.new(|cx| {
            let mut s = SidebarView::new(cx);
            s.set_connections(connections.clone());
            s.set_pinned_connections(initial_pinned_connections);
            s.set_site_filter(initial_site_filter);
            s.set_nav_collapsed(initial_nav_collapsed);
            s
        });

        let recent_activity = match ActivityStore::load_recent(500) {
            Ok(entries) => entries,
            Err(e) => {
                tracing::warn!("Failed to load recent activity: {}", e);
                Vec::new()
            }
        };

        let dashboard = cx.new(|_| {
            let mut d = DashboardView::new();
            let quick_connections: Vec<&Connection> = if initial_dashboard_pins.is_empty() {
                connections.iter().take(5).collect()
            } else {
                initial_dashboard_pins
                    .iter()
                    .filter_map(|id| connections.iter().find(|connection| connection.id == *id))
                    .take(5)
                    .collect()
            };
            d.favorite_hosts = quick_connections
                .into_iter()
                .map(|c| {
                    (
                        c.id,
                        c.display_name().to_string(),
                        c.hostname.clone(),
                        c.status == ConnectionStatus::Connected,
                    )
                })
                .collect();
            d.recent_activity = recent_activity.iter().take(8).cloned().collect();
            d
        });

        let terminal = cx.new(TerminalView::new);
        let scripts = cx.new(ScriptEditorView::new);
        let port_forwards = cx.new(|_| PortForwardView::new());
        let server_sync = cx.new(|cx| {
            let mut view = ServerSyncView::new(cx);
            view.set_connections(connections.clone(), cx);
            view.set_profiles(store.sync_profiles.clone());
            view
        });
        let sites = cx.new(|cx| {
            let mut view = SitesView::new(cx);
            view.set_connections(connections.clone());
            view.set_sites(store.managed_sites.clone());
            view
        });
        let recent = cx.new(|cx| {
            let mut view = RecentView::new(cx);
            view.set_entries(recent_activity.clone());
            view
        });
        let file_editor = cx.new(FileEditorView::new);
        // Apply the persisted `[editor]` preferences to the freshly-created
        // editor so they take effect on launch (not just after a later
        // ConfigChanged event from Settings).
        let editor_cfg = config.editor.clone();
        file_editor.update(cx, |ed, cx| {
            ed.apply_editor_config(&editor_cfg, cx);
        });
        let auto_update_enabled = config.general.auto_update;
        let ui_font_family = config.general.ui_font_family.clone();
        let ui_font_size = config.general.ui_font_size;
        let initial_sidebar_width = config.general.sidebar_width;

        // Apply the persisted terminal settings to the freshly-created view so
        // they take effect on launch (not just after a later ConfigChanged).
        {
            let theme = TerminalTheme::by_name(&config.terminal.theme);
            let cfg = &config.terminal;
            let font_family = cfg.font_family.clone();
            let font_size = cfg.font_size;
            let cursor_style = cfg.cursor_style.clone();
            let cursor_blink = cfg.cursor_blink;
            let scrollback = cfg.scrollback_lines;
            terminal.update(cx, |t, _| {
                t.set_terminal_theme(&theme);
                t.set_font_size(font_size);
                t.set_font_family(font_family);
                t.set_cursor_style(&cursor_style);
                t.set_cursor_blink(cursor_blink);
                t.set_scrollback_lines(scrollback);
                t.set_sidebar_width(initial_sidebar_width);
            });
        }

        let app_config = config.clone();
        let settings = cx.new(|settings_cx| SettingsView::new(config, settings_cx));
        let ai_assistant = cx.new(|cx| {
            AiAssistantView::new(
                AiContext::new(
                    AiSurface::Global,
                    t!("ai.context.global").to_string(),
                    serde_json::json!({}),
                ),
                cx,
            )
        });
        let ai_dock_assistant = cx.new(|cx| {
            AiAssistantView::new(
                AiContext::new(
                    AiSurface::Global,
                    t!("ai.context.global").to_string(),
                    serde_json::json!({}),
                ),
                cx,
            )
        });
        let status_bar = cx.new(|_| StatusBar::new());
        let toasts = cx.new(|_| ToastContainer::new());
        let support = cx.new(SupportView::new);
        let jean_view = cx.new(JeanView::new);
        let fleet_view = cx.new(FleetView::new);
        let bext_view = cx.new(BextCloudView::new);
        let mut ai_tasks = AiTaskStore::load().unwrap_or_else(|error| {
            tracing::warn!("Failed to load AI tasks: {error}");
            Vec::new()
        });
        let mut recovered = false;
        for task in &mut ai_tasks {
            if task.status.is_active() {
                task.set_status(AiTaskStatus::Cancelled, None);
                recovered = true;
            }
        }
        if recovered {
            let _ = AiTaskStore::save(&ai_tasks);
        }
        ai_assistant.update(cx, |view, cx| view.set_tasks(ai_tasks.clone(), cx));
        ai_dock_assistant.update(cx, |view, cx| {
            view.set_history_open(false, cx);
            view.set_tasks(ai_tasks.clone(), cx);
        });
        let ai_backend_ready = app_config.ai.is_configured()
            && (!app_config.ai.backend.is_cli() || configured_cli_available(&app_config.ai));
        support.update(cx, |view, cx| {
            view.set_ai_reply_enabled(
                ai_backend_ready && app_config.ai.allows(AiSurface::Support),
                cx,
            );
            view.set_ai_issue_enabled(
                ai_backend_ready && app_config.ai.allows(AiSurface::Issue),
                cx,
            );
        });
        scripts.update(cx, |view, cx| {
            view.set_ai_generation_enabled(
                ai_backend_ready && app_config.ai.allows(AiSurface::Script),
                cx,
            );
        });
        terminal.update(cx, |view, cx| {
            view.set_ai_actions_enabled(
                ai_backend_ready && app_config.ai.allows(AiSurface::Terminal),
                cx,
            );
            view.set_ai_naming_enabled(
                ai_backend_ready && app_config.ai.allows(AiSurface::Naming),
                cx,
            );
        });
        recent.update(cx, |view, _| {
            view.set_ai_enabled(ai_backend_ready && app_config.ai.allows(AiSurface::Recent));
        });

        // Create auto-updater
        let auto_updater = cx.new(|cx| {
            let mut updater = AutoUpdater::new();
            updater.set_enabled(auto_update_enabled, cx);
            updater
        });

        // Create command palette with registered actions
        let command_palette = cx.new(|cx| {
            let mut palette = CommandPalette::new(cx);
            // Initial palette build — no account state yet, so no mode
            // switcher. `refresh_command_palette` will rebuild with the
            // right gating on login / whoami.
            palette.set_actions(Self::base_palette_actions(false, false));
            palette
        });
        let companion_command_palette = cx.new(|cx| {
            let mut palette = CommandPalette::new(cx);
            palette.set_standalone(true);
            palette.set_actions(Self::base_palette_actions(false, false));
            palette
        });

        // Subscribe to sidebar events
        let sidebar_sub = cx.subscribe(&sidebar, |this, _sidebar, event: &SidebarEvent, cx| {
            this.handle_sidebar_event(event, cx);
        });

        // Subscribe to terminal events
        let terminal_sub = cx.subscribe(&terminal, |this, _terminal, event: &TerminalEvent, cx| {
            this.handle_terminal_event(event, cx);
        });

        // Subscribe to command palette events
        let palette_sub = cx.subscribe(
            &command_palette,
            |this, _palette, event: &CommandPaletteEvent, cx| {
                this.handle_command_palette_event(event, cx);
            },
        );
        let companion_palette_sub = cx.subscribe(
            &companion_command_palette,
            |this, _palette, event: &CommandPaletteEvent, cx| {
                this.handle_command_palette_event(event, cx);
            },
        );

        // Subscribe to settings events
        let settings_sub = cx.subscribe(&settings, |this, _settings, event: &SettingsEvent, cx| {
            this.handle_settings_event(event, cx);
        });

        let ai_assistant_sub =
            cx.subscribe(&ai_assistant, |this, view, event: &AiAssistantEvent, cx| {
                this.handle_ai_assistant_event(view, event.clone(), cx);
            });
        let ai_dock_assistant_sub = cx.subscribe(
            &ai_dock_assistant,
            |this, view, event: &AiAssistantEvent, cx| {
                this.handle_ai_assistant_event(view, event.clone(), cx);
            },
        );

        // Subscribe to script editor events
        let scripts_sub = cx.subscribe(&scripts, |this, _scripts, event: &ScriptEvent, cx| {
            this.handle_script_event(event, cx);
        });

        // Subscribe to port forward events
        let forwards_sub = cx.subscribe(
            &port_forwards,
            |this, _forwards, event: &PortForwardEvent, cx| {
                this.handle_forward_event(event, cx);
            },
        );

        // Subscribe to server sync events
        let server_sync_sub =
            cx.subscribe(&server_sync, |this, _view, event: &ServerSyncEvent, cx| {
                this.handle_server_sync_event(event, cx);
            });

        // Subscribe to sites events
        let sites_sub = cx.subscribe(&sites, |this, _view, event: &SitesEvent, cx| {
            this.handle_sites_event(event, cx);
        });

        let recent_sub = cx.subscribe(&recent, |this, _view, event: &RecentEvent, cx| {
            this.handle_recent_event(event.clone(), cx);
        });

        // Subscribe to file editor events
        let file_editor_sub = cx.subscribe(
            &file_editor,
            |_this, _view, _event: &FileEditorEvent, cx| {
                cx.notify();
            },
        );

        // Subscribe to dashboard events
        let dashboard_sub = cx.subscribe(
            &dashboard,
            |this, _dashboard, event: &DashboardEvent, cx| {
                this.handle_dashboard_event(event, cx);
            },
        );

        // Subscribe to auto-updater events
        let update_sub = cx.subscribe(
            &auto_updater,
            |this, _updater, event: &AutoUpdateEvent, cx| {
                this.handle_update_event(event, cx);
            },
        );

        // Subscribe to status bar events (update click)
        let status_bar_sub =
            cx.subscribe(
                &status_bar,
                |this, _bar, event: &StatusBarEvent, cx| match event {
                    StatusBarEvent::UpdateClicked => {
                        this.auto_updater.update(cx, |u, cx| u.trigger_update(cx));
                    }
                },
            );

        let support_sub = cx.subscribe(&support, |this, _view, event: &SupportViewEvent, cx| {
            this.handle_support_event(event.clone(), cx);
        });

        let jean_sub = cx.subscribe(&jean_view, |this, _view, event: &JeanViewEvent, cx| {
            this.handle_jean_event(event.clone(), cx);
        });

        let fleet_sub = cx.subscribe(&fleet_view, |this, _view, event: &FleetViewEvent, cx| {
            this.handle_fleet_event(event.clone(), cx);
        });

        let bext_sub = cx.subscribe(&bext_view, |this, _view, event: &BextViewEvent, cx| {
            this.handle_bext_event(event.clone(), cx);
        });

        // Load saved port forwards into the view
        {
            let saved_forwards = store.port_forwards.clone();
            if !saved_forwards.is_empty() {
                port_forwards.update(cx, |pf, _| {
                    pf.forwards = saved_forwards;
                });
            }
        }

        // Load saved scripts into the view
        {
            let saved_scripts = store.scripts.clone();
            for script in saved_scripts {
                scripts.update(cx, |editor, _| {
                    editor.add_script(script);
                });
            }
        }

        // Start git status polling
        let git_poll_task = Self::start_git_polling(cx, &status_bar);

        Self {
            connections,
            store,
            sidebar,
            dashboard,
            terminal,
            scripts,
            port_forwards,
            server_sync,
            sites,
            recent,
            file_editor,
            settings,
            ai_assistant,
            ai_dock_assistant,
            ai_sheet: None,
            ai_workflow: None,
            ai_workflow_sheet: None,
            ai_tasks,
            ai_action_confirmation: None,
            ai_script_runs: HashMap::new(),
            ai_terminal_runs: HashMap::new(),
            ai_diagnostic_sequences: HashMap::new(),
            status_bar,
            command_palette,
            companion_command_palette,
            toasts,
            connection_form: None,
            login_form: None,
            onboarding: None,
            port_forward_form: None,
            script_form: None,
            template_browser: None,
            variable_prompt: None,
            active_view: ActiveView::Dashboard,
            sidebar_visible: true,
            sidebar_width: initial_sidebar_width,
            ui_font_family,
            ui_font_size,
            window_active: true,
            recent_activity,
            focus_handle: cx.focus_handle(),
            active_tunnels: HashMap::new(),
            active_scripts: HashMap::new(),
            _sidebar_sub: sidebar_sub,
            _terminal_sub: terminal_sub,
            _palette_sub: palette_sub,
            _companion_palette_sub: companion_palette_sub,
            _settings_sub: settings_sub,
            _ai_assistant_sub: ai_assistant_sub,
            _ai_dock_assistant_sub: ai_dock_assistant_sub,
            _ai_workflow_sub: None,
            _scripts_sub: scripts_sub,
            _forwards_sub: forwards_sub,
            _server_sync_sub: server_sync_sub,
            _sites_sub: sites_sub,
            _recent_sub: recent_sub,
            _file_editor_sub: file_editor_sub,
            _dashboard_sub: dashboard_sub,
            _form_sub: None,
            _pf_form_sub: None,
            _script_form_sub: None,
            _template_browser_sub: None,
            _variable_prompt_sub: None,
            _git_poll_task: Some(git_poll_task),
            auto_updater,
            _update_sub: update_sub,
            _status_bar_sub: status_bar_sub,
            pending_delete: None,
            app_config,
            pending_close_confirm: false,
            theme_menu_open: false,
            account_menu_open: false,
            account_status: AccountStatus::Unknown,
            _login_form_sub: None,
            _onboarding_sub: None,
            last_whoami: None,
            user_home_tab: UserHomeTab::Sites,
            site_directory: None,
            site_menu_open: false,
            sidebar_kebab_menu: None,
            support,
            _support_sub: support_sub,
            _support_poll_task: None,
            jean_view,
            _jean_sub: jean_sub,
            jean_state: None,
            _jean_poll_task: None,
            jean_ask_input: String::new(),
            jean_ask_focus: cx.focus_handle(),
            fleet_view,
            _fleet_sub: fleet_sub,
            fleet_snapshot: None,
            pending_fleet_job_focus: None,
            _fleet_view_poll: None,
            runtime_instance: None,
            runtime_awaiting: Vec::new(),
            runtime_busy: false,
            _runtime_loop: None,
            issues_list: Vec::new(),
            issues_staff: false,
            issues_filter: issues::IssueListFilter::default(),
            issues_instances: Vec::new(),
            issue_detail: None,
            issue_selected: None,
            confirm_issue_delete: None,
            _issues_poll: None,
            user_new_request_sheet_open: false,
            user_new_request_sheet_dismissing: false,
            user_issue_detail_dismissing: false,
            issue_title_state: cx.new(InputState::new),
            issue_body_state: cx.new(|cx| InputState::new(cx).multi_line(true)),
            issue_comment_state: cx.new(InputState::new),
            issue_attachment_url_state: cx.new(InputState::new),
            issue_new_attachments: Vec::new(),
            issue_comment_attachments: Vec::new(),
            issue_attachment_busy: false,
            issue_attachment_generation: 0,
            issue_ai_prompt_state: cx.new(|cx| InputState::new(cx).multi_line(true)),
            issue_ai_expanded: false,
            issue_ai_loading: false,
            issue_ai_error: None,
            issue_ai_request_id: 0,
            user_sites_search_state: cx.new(InputState::new),
            issue_new_priority: "normal".to_string(),
            issue_new_source: "user",
            bext_view,
            _bext_sub: bext_sub,
            bext_user: None,
            _bext_poll: None,
            theme_before_preview: None,
            terminal_theme_before_preview: None,
            tray_state_publisher: None,
            tray_notifier: None,
            last_tray_counters: None,
        }
    }

    /// Wire the tray state publisher after tray init. `main.rs` calls
    /// this once at startup with a closure that translates
    /// [`TrayCounters`] into the binary-crate's `tray::TrayState` and
    /// pushes it into the tray thread. `None` means the tray failed to
    /// come up — every subsequent `publish_tray_state` becomes a
    /// no-op.
    pub fn set_tray_state_publisher(&mut self, publisher: Box<dyn Fn(TrayCounters) + Send + Sync>) {
        self.tray_state_publisher = Some(publisher);
    }

    /// Wire the OS-notification dispatcher after tray init. `main.rs`
    /// supplies a closure that translates [`TrayNotification`] into a
    /// `notify-rust` call. `None` means the tray is unavailable —
    /// every subsequent emit is a no-op.
    pub fn set_tray_notifier(&mut self, notifier: Box<dyn Fn(TrayNotification) + Send + Sync>) {
        self.tray_notifier = Some(notifier);
    }

    /// Fire an OS notification if the notifier is wired. Public so
    /// non-counter-driven events (Fleet job completion, future SSH
    /// disconnect hooks with the actual host name) can dispatch
    /// directly without going through `publish_tray_state`.
    pub fn emit_tray_notification(&self, n: TrayNotification) {
        if let Some(notifier) = self.tray_notifier.as_ref() {
            notifier(n);
        }
    }

    /// Compute current tray counters + push into the publisher AND
    /// fire OS notifications for positive deltas (new tickets, Jean
    /// pending) or SSH-disconnect decrements. The first publish just
    /// seeds `last_tray_counters` without notifying — otherwise a
    /// launch with existing unread tickets would spam the OS.
    ///
    /// Cheap enough (four vec-scans + a small notify-rust dispatch on
    /// deltas) to call from every spot that changes user-facing state.
    /// The tray thread diffs the counters against its last known
    /// state, so redundant publishes are silently dropped.
    pub fn publish_tray_state(&mut self, cx: &App) {
        let active_ssh = self
            .connections
            .iter()
            .filter(|c| matches!(c.status, ConnectionStatus::Connected))
            .count();
        let open_tunnels = self.active_tunnels.len();
        let unread_tickets = self.support.read(cx).unread_ticket_count();
        let jean_pending = self.runtime_awaiting.len();
        let pinned_connections = self
            .app_config
            .pinned_connections
            .iter()
            .filter_map(|id| {
                self.connections
                    .iter()
                    .find(|connection| connection.id == *id)
                    .map(|connection| TrayPinnedConnection {
                        id: *id,
                        name: connection.display_name().to_string(),
                    })
            })
            .collect();
        let counters = TrayCounters {
            active_ssh,
            open_tunnels,
            unread_tickets,
            jean_pending,
            pinned_connections,
        };

        // Delta notifications — skipped entirely on the first publish
        // so the seed value doesn't fire a startup burst. Each category
        // is opt-out via `AppConfig.tray.notify_*` (Settings → Général).
        if let Some(prev) = self.last_tray_counters.as_ref() {
            let cfg = &self.app_config.tray;
            if cfg.notify_new_tickets && counters.unread_tickets > prev.unread_tickets {
                self.emit_tray_notification(TrayNotification::NewTickets {
                    count: counters.unread_tickets - prev.unread_tickets,
                });
            }
            if cfg.notify_jean_pending && counters.jean_pending > prev.jean_pending {
                self.emit_tray_notification(TrayNotification::JeanPending {
                    count: counters.jean_pending - prev.jean_pending,
                });
            }
            if cfg.notify_ssh_disconnect && counters.active_ssh < prev.active_ssh {
                self.emit_tray_notification(TrayNotification::SshDisconnected {
                    count: prev.active_ssh - counters.active_ssh,
                });
            }
        }
        self.last_tray_counters = Some(counters.clone());

        if let Some(publisher) = self.tray_state_publisher.as_ref() {
            publisher(counters);
        }
    }

    fn toggle_connection_pin(&mut self, id: Uuid, cx: &mut Context<Self>) {
        let Some(connection) = self
            .connections
            .iter()
            .find(|connection| connection.id == id)
        else {
            return;
        };
        let name = connection.display_name().to_string();
        let unpinned = self.app_config.pinned_connections.contains(&id);
        if unpinned {
            self.app_config
                .pinned_connections
                .retain(|pinned| *pinned != id);
        } else {
            self.app_config.pinned_connections.push(id);
        }

        if let Err(error) = self.app_config.save() {
            if unpinned {
                self.app_config.pinned_connections.push(id);
            } else {
                self.app_config
                    .pinned_connections
                    .retain(|pinned| *pinned != id);
            }
            tracing::error!("Failed to persist pinned connections: {error}");
            self.show_toast(
                t!("toast.connection.pin_failed", error = error.to_string()).to_string(),
                ToastLevel::Error,
                cx,
            );
            return;
        }

        self.sync_settings_config(cx);
        self.sidebar.update(cx, |sidebar, cx| {
            sidebar.set_pinned_connections(self.app_config.pinned_connections.clone());
            cx.notify();
        });
        self.update_dashboard_stats(cx);
        self.show_toast(
            if unpinned {
                t!("toast.connection.unpinned", name = name.as_str()).to_string()
            } else {
                t!("toast.connection.pinned", name = name.as_str()).to_string()
            },
            ToastLevel::Info,
            cx,
        );
    }

    /// Connect a pinned host selected from the system tray.
    pub fn connect_pinned_connection(&mut self, id: Uuid, cx: &mut Context<Self>) {
        self.handle_sidebar_event(&SidebarEvent::ConnectionConnect(id), cx);
    }

    /// Decide whether a window-close request should proceed.
    ///
    /// When `confirm_before_close` is enabled and there are active terminal
    /// sessions or running tunnels, the first close attempt is intercepted: we
    /// warn the user and require a second close to confirm (matching the
    /// Should the close button hide the window to the tray instead of
    /// quitting? True only when the user opted in via Settings **and**
    /// the tray is actually up (no publisher = no tray, so hiding
    /// would strand the app invisible). `main.rs` checks this before
    /// `confirm_window_close` and calls `window.hide_window()` if true.
    pub fn should_hide_to_tray(&self) -> bool {
        self.app_config.tray.close_to_tray && self.tray_state_publisher.is_some()
    }

    /// app's existing two-step "click again to confirm" pattern). Returns
    /// `true` to allow the window to close, `false` to cancel.
    pub fn confirm_window_close(&mut self, cx: &mut Context<Self>) -> bool {
        if !self.app_config.general.confirm_before_close {
            return true;
        }

        let active_terminals = self.terminal.read(cx).tab_count();
        let active_tunnels = self.active_tunnels.len();
        let active_scripts = self.active_scripts.len();
        let has_activity = active_terminals > 0 || active_tunnels > 0 || active_scripts > 0;

        if !has_activity {
            return true;
        }

        if self.pending_close_confirm {
            // Second attempt — allow the close to proceed.
            return true;
        }

        self.pending_close_confirm = true;
        // Push directly so this confirmation is shown even when general
        // notifications are disabled — the user must see why close was blocked.
        let warning = format!(
            "{} active session(s)/tunnel(s) running — close the window again to confirm exit",
            active_terminals + active_tunnels + active_scripts
        );
        self.toasts.update(cx, |toasts, cx| {
            toasts.push(warning, ToastLevel::Warning, cx);
        });
        false
    }

    fn handle_sidebar_event(&mut self, event: &SidebarEvent, cx: &mut Context<Self>) {
        match event {
            SidebarEvent::SectionChanged(section) => {
                self.switch_to_section(*section);
                if *section == SidebarSection::Scripts {
                    self.populate_script_editor_connections(cx);
                }
                self.on_active_view_changed(cx);
                cx.notify();
            }
            SidebarEvent::ConnectionSelected(id) => {
                tracing::info!("Connection selected: {}", id);
                // Check if there's already an open tab for this connection
                let existing_tab = self.terminal.read(cx).find_tab_for_connection(*id);
                if let Some(tab_id) = existing_tab {
                    // Switch to existing tab
                    self.terminal.update(cx, |terminal, cx| {
                        terminal.select_tab(tab_id);
                        cx.notify();
                    });
                } else if let Some(conn) = self.connections.iter().find(|c| c.id == *id) {
                    let title = conn.display_name().to_string();
                    let conn_id = conn.id;
                    self.connect_ssh(conn.clone(), cx);
                    self.add_activity_entry(
                        ActivityEntry::new(
                            ActivityKind::Connection,
                            t!("activity.connecting_to", name = title.as_str()).to_string(),
                        )
                        .with_target(conn_id.to_string(), title)
                        .with_action(ActivityAction::ConnectConnection),
                        cx,
                    );
                }
                self.active_view = ActiveView::Terminal;
                cx.notify();
            }
            SidebarEvent::ConnectionConnect(id) => {
                tracing::info!("Connect requested: {}", id);
                if let Some(conn) = self.connections.iter().find(|c| c.id == *id) {
                    let title = conn.display_name().to_string();
                    let conn_id = conn.id;
                    self.connect_ssh(conn.clone(), cx);
                    self.add_activity_entry(
                        ActivityEntry::new(
                            ActivityKind::Connection,
                            t!("activity.connecting_to", name = title.as_str()).to_string(),
                        )
                        .with_target(conn_id.to_string(), title)
                        .with_action(ActivityAction::ConnectConnection),
                        cx,
                    );
                }
                self.active_view = ActiveView::Terminal;
                cx.notify();
            }
            SidebarEvent::AddConnection => {
                self.show_connection_form(None, cx);
            }
            SidebarEvent::ConnectionEdit(id) => {
                if let Some(conn) = self.connections.iter().find(|c| c.id == *id) {
                    let conn = conn.clone();
                    self.show_connection_form(Some(conn), cx);
                }
            }
            SidebarEvent::QuickConnect => {
                self.show_connection_form(None, cx);
            }
            SidebarEvent::ConnectionDelete(id) => {
                let id = *id;
                if self.pending_delete == Some(id) {
                    // Second click — confirmed, perform deletion
                    self.pending_delete = None;
                    if let Some(conn) = self.connections.iter().find(|c| c.id == id) {
                        let name = conn.display_name().to_string();
                        match self.store.remove_connection(id) {
                            Ok(true) => {
                                tracing::info!("Deleted connection: {}", name);
                            }
                            Ok(false) => {
                                tracing::warn!("Connection {} not found in store", id);
                            }
                            Err(e) => {
                                tracing::error!("Failed to delete connection: {}", e);
                                self.show_toast(
                                    t!("toast.connection.delete_failed", error = e.to_string())
                                        .to_string(),
                                    ToastLevel::Error,
                                    cx,
                                );
                                return;
                            }
                        }
                        self.connections.retain(|c| c.id != id);
                        self.app_config
                            .pinned_connections
                            .retain(|pinned| *pinned != id);
                        if let Err(error) = self.app_config.save() {
                            tracing::error!("Failed to persist removed connection pin: {error}");
                        }
                        self.sync_settings_config(cx);
                        self.sidebar.update(cx, |sidebar, _| {
                            sidebar.set_connections(self.connections.clone());
                            sidebar
                                .set_pinned_connections(self.app_config.pinned_connections.clone());
                        });
                        self.port_forwards.update(cx, |pf, _| {
                            pf.forwards.retain(|f| f.connection_id != id);
                        });
                        self.add_activity(
                            t!("activity.connection_deleted", name = name.as_str()).to_string(),
                            ActivityKind::Connection,
                            cx,
                        );
                        self.show_toast(
                            t!("toast.connection.deleted", name = name.as_str()).to_string(),
                            ToastLevel::Info,
                            cx,
                        );
                        self.update_dashboard_stats(cx);
                        cx.notify();
                    }
                } else {
                    // First click — ask for confirmation
                    if let Some(conn) = self.connections.iter().find(|c| c.id == id) {
                        let name = conn.display_name().to_string();
                        self.pending_delete = Some(id);
                        self.show_toast(
                            t!("toast.connection.delete_confirm", name = name.as_str()).to_string(),
                            ToastLevel::Warning,
                            cx,
                        );
                        cx.notify();
                    }
                }
            }
            SidebarEvent::ConnectionPinToggled(id) => {
                self.toggle_connection_pin(*id, cx);
            }
            SidebarEvent::WidthChanged(width) => {
                self.sidebar_width = *width;
                self.terminal.update(cx, |terminal, _cx| {
                    terminal.set_sidebar_width(*width);
                });
                cx.notify();
            }
            SidebarEvent::ConnectionManageBext(id) => {
                self.manage_bext_for_connection(*id, cx);
            }
            SidebarEvent::OpenConnectionMenu { conn_id, position } => {
                self.sidebar_kebab_menu = Some((*conn_id, *position));
                cx.notify();
            }
            SidebarEvent::NavCollapsedChanged(collapsed) => {
                let collapsed = *collapsed;
                self.app_config.general.sidebar_nav_collapsed = collapsed;
                self.settings.update(cx, |settings, cx| {
                    settings.set_sidebar_nav_collapsed(collapsed, cx);
                });
                cx.notify();
            }
        }
    }

    fn handle_terminal_event(&mut self, event: &TerminalEvent, cx: &mut Context<Self>) {
        match event {
            TerminalEvent::NewTabRequested => {
                tracing::info!("New terminal tab created");
                self.active_view = ActiveView::Terminal;
                self.add_activity_entry(
                    ActivityEntry::new(
                        ActivityKind::Terminal,
                        t!("activity.terminal_opened").to_string(),
                    )
                    .with_action(ActivityAction::OpenTerminal),
                    cx,
                );
                self.update_dashboard_stats(cx);
                self.sync_terminal_tab_count(cx);
                cx.notify();
            }
            TerminalEvent::TabSelected(id) => {
                tracing::info!("Terminal tab selected: {}", id);
            }
            TerminalEvent::TabClosed(id) => {
                tracing::info!("Terminal tab closed: {}", id);
                if let Some(plan) = self.ai_terminal_runs.remove(id) {
                    self.audit_ai_action(&plan, "target_closed", cx);
                }
                self.ai_diagnostic_sequences.remove(id);
                self.add_activity(
                    t!("activity.terminal_closed").to_string(),
                    ActivityKind::Terminal,
                    cx,
                );
                self.update_dashboard_stats(cx);
                self.sync_terminal_tab_count(cx);
                cx.notify();
            }
            TerminalEvent::DuplicateTabRequested(connection_id) => {
                let connection_id = *connection_id;
                if let Some(conn) = self
                    .connections
                    .iter()
                    .find(|c| c.id == connection_id)
                    .cloned()
                {
                    tracing::info!("Duplicating connection tab: {}", conn.display_name());
                    self.connect_ssh(conn, cx);
                    self.active_view = ActiveView::Terminal;
                    self.sync_terminal_tab_count(cx);
                    cx.notify();
                } else {
                    tracing::error!(
                        "Duplicate requested for unknown connection {}",
                        connection_id
                    );
                }
            }
            TerminalEvent::SplitRequested {
                connection_id,
                direction,
            } => {
                let connection_id = *connection_id;
                let direction = *direction;
                if let Some(conn) = self
                    .connections
                    .iter()
                    .find(|c| c.id == connection_id)
                    .cloned()
                {
                    self.connect_ssh_split(conn, direction, cx);
                } else {
                    tracing::error!("Split requested for unknown connection {}", connection_id);
                }
            }
            TerminalEvent::RunScriptRequested(id) => {
                let id = *id;
                if let Some(script) = self
                    .scripts
                    .read(cx)
                    .scripts
                    .iter()
                    .find(|s| s.id == id)
                    .cloned()
                {
                    self.handle_script_event(&ScriptEvent::RunScript(script), cx);
                }
            }
            TerminalEvent::TogglePinScript(id) => {
                let id = *id;
                self.scripts.update(cx, |editor, _| {
                    if let Some(s) = editor.scripts.iter_mut().find(|s| s.id == id) {
                        s.pinned_to_toolbar = !s.pinned_to_toolbar;
                    }
                });
                if let Some(s) = self
                    .scripts
                    .read(cx)
                    .scripts
                    .iter()
                    .find(|s| s.id == id)
                    .cloned()
                {
                    let _ = self.store.update_script(s);
                }
                self.sync_scripts_to_terminal_toolbar(cx);
                cx.notify();
            }
            TerminalEvent::GenerateCommandWithAi(session_id) => {
                self.open_ai_workflow(
                    AiWorkflowTarget::TerminalCommand {
                        session_id: session_id.to_string(),
                    },
                    cx,
                );
            }
            TerminalEvent::DiagnoseWithAi(session_id) => {
                self.open_ai_workflow(
                    AiWorkflowTarget::TerminalDiagnose {
                        session_id: session_id.to_string(),
                    },
                    cx,
                );
            }
            TerminalEvent::SuggestNameWithAi(session_id) => {
                self.open_ai_workflow(
                    AiWorkflowTarget::EntityNaming {
                        kind: AiNamingKind::Terminal,
                        target_id: session_id.to_string(),
                    },
                    cx,
                );
            }
            TerminalEvent::CreateIssueFromContext(session_id) => {
                let context = self.terminal.read(cx).ai_context_data();
                let expected_session = session_id.to_string();
                if context
                    .get("session_id")
                    .and_then(serde_json::Value::as_str)
                    != Some(expected_session.as_str())
                {
                    return;
                }
                let title = context
                    .get("title")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("Terminal");
                let cwd = context
                    .get("cwd")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default();
                let selection = context
                    .get("selection")
                    .and_then(serde_json::Value::as_str)
                    .filter(|value| !value.trim().is_empty());
                let output = selection.unwrap_or_else(|| {
                    context
                        .get("visible_output")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or_default()
                });
                let body = format!(
                    "{}: {}\n{}: {}\n\n{}:\n{}",
                    t!("terminal.issue.session"),
                    title,
                    t!("terminal.issue.cwd"),
                    cwd,
                    t!("terminal.issue.output"),
                    output
                );
                self.open_prefilled_request(
                    t!("terminal.issue.title", terminal = title).to_string(),
                    body,
                    "shelldeck",
                    cx,
                );
            }
            TerminalEvent::StopAiCommand(session_id) => {
                let stopped = self
                    .terminal
                    .update(cx, |terminal, cx| terminal.stop_ai_command(*session_id, cx))
                    .is_ok();
                if stopped {
                    if let Some(plan) = self.ai_terminal_runs.remove(session_id) {
                        self.audit_ai_action(&plan, "cancelled", cx);
                    }
                    self.ai_diagnostic_sequences.remove(session_id);
                    self.show_toast(
                        t!("toast.ai.command_stopped").to_string(),
                        ToastLevel::Info,
                        cx,
                    );
                }
            }
            TerminalEvent::AiCommandFinished {
                session_id,
                exit_code,
                output,
            } => {
                if let Some(plan) = self.ai_terminal_runs.remove(session_id) {
                    let succeeded = exit_code.is_none_or(|code| code == 0);
                    self.audit_ai_action(&plan, if succeeded { "succeeded" } else { "failed" }, cx);
                    self.show_toast(
                        if succeeded {
                            t!("toast.ai.action_succeeded").to_string()
                        } else {
                            t!("toast.ai.command_failed", code = exit_code.unwrap_or(-1))
                                .to_string()
                        },
                        if succeeded {
                            ToastLevel::Success
                        } else {
                            ToastLevel::Error
                        },
                        cx,
                    );
                    if succeeded {
                        if self.ai_diagnostic_sequences.contains_key(session_id) {
                            self.prepare_next_ai_diagnostic_step(*session_id, output.clone(), cx);
                        }
                    } else if let Some(sequence) = self.ai_diagnostic_sequences.remove(session_id) {
                        self.set_ai_workflow_task_status(
                            &sequence.target,
                            AiTaskStatus::Failed,
                            cx,
                        );
                    }
                }
            }
        }
    }

    fn handle_update_event(&mut self, event: &AutoUpdateEvent, cx: &mut Context<Self>) {
        match event {
            AutoUpdateEvent::StatusChanged(status) => {
                let text = status.to_string();
                let show_toast = matches!(
                    status,
                    AutoUpdateStatus::UpdateAvailable(_)
                        | AutoUpdateStatus::Updated(_)
                        | AutoUpdateStatus::Errored(_)
                );

                // Update status bar
                self.status_bar.update(cx, |bar, cx| {
                    bar.update_status = match status {
                        AutoUpdateStatus::Idle => None,
                        _ => Some(text.clone()),
                    };
                    cx.notify();
                });

                // Show toast for notable events
                if show_toast {
                    let level = match status {
                        AutoUpdateStatus::Errored(_) => ToastLevel::Error,
                        AutoUpdateStatus::Updated(_) => ToastLevel::Success,
                        _ => ToastLevel::Info,
                    };
                    self.toasts.update(cx, |toasts, cx| {
                        toasts.push(text, level, cx);
                    });
                }

                cx.notify();
            }
        }
    }

    fn handle_settings_event(&mut self, event: &SettingsEvent, cx: &mut Context<Self>) {
        match event {
            SettingsEvent::ConfigChanged(config) => {
                tracing::info!("Config changed, applying settings");
                if self.app_config.ai != config.ai {
                    self.ai_sheet = None;
                    self.ai_workflow_sheet = None;
                    self.ai_workflow = None;
                    self._ai_workflow_sub = None;
                }
                // Merge settings-owned slices only — see `.agents/session-state.md`.
                self.app_config.general = config.general.clone();
                self.app_config.terminal = config.terminal.clone();
                self.app_config.editor = config.editor.clone();
                self.app_config.tray = config.tray.clone();
                self.app_config.companion = config.companion.clone();
                self.app_config.ai = config.ai.clone();
                let dock_available =
                    self.ai_backend_available() && self.app_config.ai.allows(AiSurface::Global);
                self.ai_dock_assistant.update(cx, |assistant, cx| {
                    assistant.set_backend(
                        self.app_config.ai.backend,
                        self.app_config.ai.model.clone(),
                        cx,
                    );
                    assistant.set_available(dock_available, cx);
                });
                self.sync_ai_affordances(cx);
                // Apply terminal settings to running view
                let terminal_theme = TerminalTheme::by_name(&self.app_config.terminal.theme);
                self.terminal.update(cx, |terminal, cx| {
                    terminal.set_font_size(self.app_config.terminal.font_size);
                    terminal.set_font_family(self.app_config.terminal.font_family.clone());
                    terminal.set_cursor_style(&self.app_config.terminal.cursor_style);
                    terminal.set_cursor_blink(self.app_config.terminal.cursor_blink);
                    terminal.set_scrollback_lines(self.app_config.terminal.scrollback_lines);
                    terminal.set_terminal_theme(&terminal_theme);
                    cx.notify();
                });
                // Apply sidebar width
                self.sidebar_width = self.app_config.general.sidebar_width;
                self.terminal.update(cx, |terminal, _cx| {
                    terminal.set_sidebar_width(self.app_config.general.sidebar_width);
                });
                // Apply application UI font (cascades to all child views on re-render)
                self.ui_font_family = self.app_config.general.ui_font_family.clone();
                self.ui_font_size = self.app_config.general.ui_font_size;
                // The file editor now has its own persisted preferences
                // (font, tab size, line numbers, wrap, blink…). Apply the full
                // slice — the editor merges its own view state and rebuilds
                // the glyph cache lazily.
                let editor_cfg = self.app_config.editor.clone();
                self.file_editor.update(cx, |ed, cx| {
                    ed.apply_editor_config(&editor_cfg, cx);
                });
                // Apply auto-update preference
                let auto_update = self.app_config.general.auto_update;
                self.auto_updater.update(cx, |updater, cx| {
                    updater.set_enabled(auto_update, cx);
                });
                crate::i18n::apply_ui_language(&self.app_config.general.ui_language);
                self.refresh_command_palette(cx);
                cx.notify();
            }
            SettingsEvent::AutostartRequested(desired) => {
                self.apply_autostart_request(*desired, cx);
            }
            SettingsEvent::ShowOnboarding => {
                self.show_onboarding(cx);
            }
            SettingsEvent::AiApiKeyStored { backend, value } => {
                self.update_ai_api_key(*backend, Some(value.clone()), cx);
            }
            SettingsEvent::AiApiKeyDeleted { backend } => {
                self.update_ai_api_key(*backend, None, cx);
            }
            SettingsEvent::AiTestRequested(config) => {
                self.test_ai_connection(config.clone(), cx);
            }
            SettingsEvent::ThemeChanged(pref) => {
                tracing::info!("Theme preference changed to {:?}", pref);

                // Keep the in-memory config in sync with the active theme.
                self.app_config.theme = pref.clone();

                // A committed theme change supersedes any palette preview.
                self.theme_before_preview = None;

                // Apply the palette + matching component theme, then repaint.
                self.apply_palette(pref, cx);

                // Terminal color theme is configured independently (Appearance
                // tab / command palette) and persisted, so it is intentionally
                // left untouched when the app light/dark preference changes.
            }
        }
    }

    fn update_ai_api_key(
        &mut self,
        backend: shelldeck_core::ai::AiBackend,
        value: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let Some(provider) = backend.provider_key() else {
            return;
        };
        self.ai_sheet = None;
        self.refresh_command_palette(cx);
        let provider = provider.to_string();
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let deleting = value.is_none();
            let result = cx
                .background_executor()
                .spawn(async move {
                    match value {
                        None => shelldeck_core::config::keychain::delete_ai_api_key(&provider),
                        Some(value) => shelldeck_core::config::keychain::store_ai_api_key(
                            &provider,
                            value.trim(),
                        ),
                    }
                })
                .await;
            let _ = this.update(cx, |ws, cx| match result {
                Ok(()) => {
                    ws.settings.update(cx, |settings, cx| {
                        settings.reset_ai_connection_state(cx);
                    });
                    ws.show_toast(
                        if deleting {
                            t!("toast.ai.key_deleted").to_string()
                        } else {
                            t!("toast.ai.key_saved").to_string()
                        },
                        ToastLevel::Info,
                        cx,
                    );
                }
                Err(error) => {
                    let message = t!("toast.ai.key_failed", error = error.to_string()).to_string();
                    ws.settings.update(cx, |settings, cx| {
                        settings.set_ai_connection_result(Err(message.clone()), cx);
                    });
                    ws.show_toast(message, ToastLevel::Error, cx);
                }
            });
        })
        .detach();
    }

    fn test_ai_connection(&mut self, config: AppConfig, cx: &mut Context<Self>) {
        let ai_config = config.ai;
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let tested_config = ai_config.clone();
            let result = cx
                .background_executor()
                .spawn(async move { test_connection(&tested_config) })
                .await
                .map(|_| ())
                .map_err(|error| error.to_string());

            let _ = this.update(cx, |workspace, cx| {
                if workspace.app_config.ai != ai_config {
                    workspace.settings.update(cx, |settings, cx| {
                        settings.reset_ai_connection_state(cx);
                    });
                    return;
                }
                workspace.settings.update(cx, |settings, cx| {
                    settings.set_ai_connection_result(result.clone(), cx);
                });
                workspace.refresh_command_palette(cx);
                workspace.show_toast(
                    match &result {
                        Ok(()) => t!("toast.ai.test_ok").to_string(),
                        Err(error) => t!("toast.ai.test_failed", error = error.clone()).to_string(),
                    },
                    if result.is_ok() {
                        ToastLevel::Success
                    } else {
                        ToastLevel::Error
                    },
                    cx,
                );
                cx.notify();
            });
        })
        .detach();
    }

    fn open_ai_assistant(&mut self, cx: &mut Context<Self>) {
        let context = self.current_ai_context(cx);
        if !self.ai_backend_available() || !self.app_config.ai.allows(context.surface) {
            return;
        }
        self.open_ai_assistant_with_context(context, cx);
    }

    /// Prepare the shared assistant entity for the standalone companion Dock.
    /// The Dock intentionally starts with a bounded global context rather than
    /// silently inheriting terminal, ticket or script data from the main UI.
    pub fn prepare_ai_dock(&mut self, cx: &mut Context<Self>) -> Entity<AiAssistantView> {
        let available = self.ai_backend_available() && self.app_config.ai.allows(AiSurface::Global);
        // Both views persist the complete conversation list. Keep only one
        // editor visible so concurrent saves cannot overwrite each other.
        self.ai_sheet = None;
        self.ai_dock_assistant.update(cx, |assistant, cx| {
            assistant.reload_conversations(cx);
            assistant.set_backend(
                self.app_config.ai.backend,
                self.app_config.ai.model.clone(),
                cx,
            );
            assistant.set_context(
                AiContext::new(
                    AiSurface::Global,
                    t!("ai.context.global").to_string(),
                    serde_json::json!({}),
                ),
                cx,
            );
            assistant.set_available(available, cx);
        });
        cx.notify();
        self.ai_dock_assistant.clone()
    }

    pub fn companion_ui_font_family(&self) -> Option<String> {
        (self.ui_font_family != "System Default").then(|| self.ui_font_family.clone())
    }

    /// Refresh the hidden Dock before showing it again without touching its
    /// context request gate, so an in-flight completion remains valid.
    pub fn refresh_ai_dock(&mut self, cx: &mut Context<Self>) {
        let available = self.ai_backend_available() && self.app_config.ai.allows(AiSurface::Global);
        self.ai_sheet = None;
        self.ai_dock_assistant.update(cx, |assistant, cx| {
            assistant.reload_conversations(cx);
            assistant.set_backend(
                self.app_config.ai.backend,
                self.app_config.ai.model.clone(),
                cx,
            );
            assistant.set_available(available, cx);
        });
        cx.notify();
    }

    fn current_ai_context(&self, cx: &App) -> AiContext {
        if self.effective_mode() == AppMode::Support {
            let support = self.support.read(cx);
            return AiContext::new(
                support.ai_surface(),
                t!("ai.context.support").to_string(),
                self.ai_context_data_with_hosts(support.ai_context_data()),
            );
        }
        if self.effective_mode() == AppMode::User && self.user_home_tab == UserHomeTab::Requests {
            return AiContext::new(
                AiSurface::Issue,
                t!("ai.context.issue").to_string(),
                self.ai_context_data_with_hosts(
                    serde_json::to_value(&self.issue_detail)
                        .unwrap_or_else(|_| serde_json::json!({ "issue": null })),
                ),
            );
        }
        match self.active_view {
            ActiveView::Terminal => AiContext::new(
                AiSurface::Terminal,
                t!("ai.context.terminal").to_string(),
                self.ai_context_data_with_hosts(self.terminal.read(cx).ai_context_data()),
            ),
            ActiveView::Scripts => AiContext::new(
                AiSurface::Script,
                t!("ai.context.script").to_string(),
                self.script_ai_context_data(cx),
            ),
            ActiveView::JeanConsole | ActiveView::Fleet => AiContext::new(
                AiSurface::Jean,
                t!("ai.context.jean").to_string(),
                self.ai_context_data_with_hosts(
                    serde_json::to_value(&self.jean_state)
                        .unwrap_or_else(|_| serde_json::json!({ "jean": null })),
                ),
            ),
            ActiveView::Recent | ActiveView::Dashboard => AiContext::new(
                AiSurface::Recent,
                t!("ai.context.recent").to_string(),
                self.ai_context_data_with_hosts(
                    serde_json::to_value(self.recent_activity.iter().take(50).collect::<Vec<_>>())
                        .unwrap_or_else(|_| serde_json::json!([])),
                ),
            ),
            ActiveView::Sites | ActiveView::PortForwards => AiContext::new(
                AiSurface::Naming,
                t!("ai.context.naming").to_string(),
                serde_json::json!({
                    "connections": self.ai_hosts_context_data(),
                    "tunnels": self.store.port_forwards.iter().map(|forward| serde_json::json!({
                        "label": forward.label,
                        "direction": format!("{:?}", forward.direction),
                        "local": format!("{}:{}", forward.local_host, forward.local_port),
                        "remote": format!("{}:{}", forward.remote_host, forward.remote_port),
                    })).collect::<Vec<_>>(),
                }),
            ),
            _ => AiContext::new(
                AiSurface::Global,
                t!("ai.context.global").to_string(),
                self.ai_context_data_with_hosts(serde_json::json!({
                    "active_view": format!("{:?}", self.active_view),
                    "active_site": self.app_config.cloud_sync.active_site_label,
                    "connections": self.connections.len(),
                    "active_tunnels": self.active_tunnels.len(),
                    "active_scripts": self.active_scripts.len(),
                })),
            ),
        }
    }

    fn ai_hosts_context_data(&self) -> serde_json::Value {
        host_context(&self.connections)
    }

    fn ai_context_data_with_hosts(&self, data: serde_json::Value) -> serde_json::Value {
        serde_json::json!({
            "screen": data,
            "hosts": self.ai_hosts_context_data(),
        })
    }

    fn script_ai_context_data(&self, cx: &App) -> serde_json::Value {
        serde_json::json!({
            "script": self.scripts.read(cx).ai_context_data(),
            "hosts": self.ai_hosts_context_data(),
        })
    }

    fn ai_backend_available(&self) -> bool {
        self.app_config.ai.is_configured()
            && (!self.app_config.ai.backend.is_cli()
                || configured_cli_available(&self.app_config.ai))
    }

    fn ai_available_for_current_surface(&self, cx: &App) -> bool {
        self.ai_backend_available()
            && self
                .app_config
                .ai
                .allows(self.current_ai_context(cx).surface)
    }

    fn sync_ai_affordances(&mut self, cx: &mut Context<Self>) {
        let backend_ready = self.ai_backend_available();
        self.support.update(cx, |view, cx| {
            view.set_ai_reply_enabled(
                backend_ready && self.app_config.ai.allows(AiSurface::Support),
                cx,
            );
            view.set_ai_issue_enabled(
                backend_ready && self.app_config.ai.allows(AiSurface::Issue),
                cx,
            );
        });
        self.scripts.update(cx, |view, cx| {
            view.set_ai_generation_enabled(
                backend_ready && self.app_config.ai.allows(AiSurface::Script),
                cx,
            );
        });
        if let Some(form) = self.script_form.as_ref() {
            form.update(cx, |form, cx| {
                form.set_ai_enabled(
                    backend_ready && self.app_config.ai.allows(AiSurface::Script),
                    cx,
                );
                form.set_ai_naming_enabled(
                    backend_ready && self.app_config.ai.allows(AiSurface::Naming),
                    cx,
                );
            });
        }
        self.terminal.update(cx, |view, cx| {
            view.set_ai_actions_enabled(
                backend_ready && self.app_config.ai.allows(AiSurface::Terminal),
                cx,
            );
            view.set_ai_naming_enabled(
                backend_ready && self.app_config.ai.allows(AiSurface::Naming),
                cx,
            );
        });
        self.recent.update(cx, |view, cx| {
            view.set_ai_enabled(backend_ready && self.app_config.ai.allows(AiSurface::Recent));
            cx.notify();
        });
    }

    fn close_ai_workflow(&mut self, cx: &mut Context<Self>) {
        self.ai_workflow_sheet = None;
        self.ai_workflow = None;
        self._ai_workflow_sub = None;
        cx.notify();
    }

    fn sync_ai_tasks(&mut self, cx: &mut Context<Self>) {
        if let Err(error) = AiTaskStore::save(&self.ai_tasks) {
            tracing::warn!("Failed to save AI tasks: {error}");
        }
        let tasks = self.ai_tasks.clone();
        self.ai_assistant
            .update(cx, |view, cx| view.set_tasks(tasks.clone(), cx));
        self.ai_dock_assistant
            .update(cx, |view, cx| view.set_tasks(tasks, cx));
        cx.notify();
    }

    fn begin_ai_workflow_task(
        &mut self,
        target: &AiWorkflowTarget,
        instructions: String,
        cx: &mut Context<Self>,
    ) -> Uuid {
        let context_title = self.ai_workflow_context(target, cx).title;
        let model = if self.app_config.ai.model.trim().is_empty() {
            self.app_config.ai.backend.default_model().to_string()
        } else {
            self.app_config.ai.model.clone()
        };
        if let Some(task) = self.ai_tasks.iter_mut().rev().find(|task| {
            task.capability == target.capability()
                && task.target_id == target.target_id()
                && (!task.status.is_finished() || task.status == AiTaskStatus::Failed)
        }) {
            task.instructions = instructions;
            task.result.clear();
            task.target_kind = Some(target.storage_kind().to_string());
            task.target_label = context_title;
            task.model = model;
            task.set_status(AiTaskStatus::Generating, None);
            let id = task.id;
            self.sync_ai_tasks(cx);
            return id;
        }

        let mut task = AiTask::new(
            target.capability(),
            target.surface(),
            target.target_id(),
            self.app_config.ai.backend,
            instructions,
            String::new(),
        );
        task.target_kind = Some(target.storage_kind().to_string());
        task.target_label = context_title;
        task.model = model;
        task.set_status(AiTaskStatus::Generating, None);
        let id = task.id;
        self.ai_tasks.push(task);
        self.sync_ai_tasks(cx);
        id
    }

    fn finish_ai_workflow_task(
        &mut self,
        task_id: Uuid,
        result: &Result<String, String>,
        cx: &mut Context<Self>,
    ) {
        let completed_in_background = self.ai_workflow.is_none();
        if let Some(task) = self.ai_tasks.iter_mut().find(|task| task.id == task_id) {
            match result {
                Ok(output) => {
                    task.result = output.clone();
                    task.set_status(AiTaskStatus::Ready, None);
                }
                Err(error) => task.set_status(AiTaskStatus::Failed, Some(error.clone())),
            }
            self.sync_ai_tasks(cx);
            if completed_in_background {
                self.show_toast(
                    if result.is_ok() {
                        t!("toast.ai.task_ready").to_string()
                    } else {
                        t!("toast.ai.task_failed").to_string()
                    },
                    if result.is_ok() {
                        ToastLevel::Success
                    } else {
                        ToastLevel::Error
                    },
                    cx,
                );
                if !self.window_active && self.app_config.tray.notify_ai_tasks {
                    self.emit_tray_notification(TrayNotification::AiTaskDone {
                        success: result.is_ok(),
                    });
                }
            }
        }
    }

    fn set_ai_workflow_task_status(
        &mut self,
        target: &AiWorkflowTarget,
        status: AiTaskStatus,
        cx: &mut Context<Self>,
    ) {
        if let Some(task) = self.ai_tasks.iter_mut().rev().find(|task| {
            task.capability == target.capability() && task.target_id == target.target_id()
        }) {
            task.set_status(status, None);
            self.sync_ai_tasks(cx);
        }
    }

    fn set_ai_action_task_status(
        &mut self,
        plan: &AiActionPlan,
        status: AiTaskStatus,
        message: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let task_index = self.ai_tasks.iter().rposition(|task| {
            task.id == plan.id
                || (task.capability == plan.capability
                    && task.target_id == plan.target_id
                    && !task.status.is_finished())
        });
        if let Some(index) = task_index {
            let task = &mut self.ai_tasks[index];
            task.id = plan.id;
            task.target_label = plan.target_label.clone();
            task.model = plan.model.clone();
            task.set_status(status, message);
        } else {
            let mut task = AiTask::from_action(plan, status);
            task.status_message = message;
            self.ai_tasks.push(task);
        }
        self.sync_ai_tasks(cx);
    }

    fn stage_ai_action(&mut self, mut plan: AiActionPlan, cx: &mut Context<Self>) {
        let level = self.app_config.ai.policies.level_for(plan.capability);
        plan.autonomy = level;
        match ai_action_disposition(level, plan.risk) {
            AiActionDisposition::DraftOnly => {
                self.set_ai_action_task_status(&plan, AiTaskStatus::Ready, None, cx);
                self.show_toast(
                    t!("toast.ai.policy_preparation_only").to_string(),
                    ToastLevel::Info,
                    cx,
                );
            }
            AiActionDisposition::Confirm => {
                self.set_ai_action_task_status(&plan, AiTaskStatus::AwaitingConfirmation, None, cx);
                self.ai_action_confirmation = Some(plan);
                cx.notify();
            }
            AiActionDisposition::Execute => {
                self.set_ai_action_task_status(&plan, AiTaskStatus::AwaitingConfirmation, None, cx);
                self.ai_action_confirmation = Some(plan);
                self.confirm_ai_action(cx);
            }
        }
    }

    fn open_ai_workflow(&mut self, target: AiWorkflowTarget, cx: &mut Context<Self>) {
        let surface = target.surface();
        if !self.ai_backend_available() || !self.app_config.ai.allows(surface) {
            return;
        }
        let pending = self
            .ai_tasks
            .iter()
            .rev()
            .find(|draft| {
                draft.capability == target.capability()
                    && draft.target_id == target.target_id()
                    && matches!(draft.status, AiTaskStatus::Ready | AiTaskStatus::Pending)
            })
            .cloned();
        let should_generate = matches!(
            &target,
            AiWorkflowTarget::EntityNaming { .. }
                | AiWorkflowTarget::SupportReply { .. }
                | AiWorkflowTarget::SupportSummary { .. }
                | AiWorkflowTarget::SupportTriage { .. }
                | AiWorkflowTarget::IssueReply { .. }
                | AiWorkflowTarget::IssueSummary { .. }
                | AiWorkflowTarget::IssueTriage { .. }
                | AiWorkflowTarget::ScriptExplain { .. }
                | AiWorkflowTarget::ScriptReview { .. }
                | AiWorkflowTarget::ScriptFix { .. }
                | AiWorkflowTarget::TerminalDiagnose { .. }
        ) && pending.is_none();
        let title = match &target {
            AiWorkflowTarget::EntityNaming { .. } => t!("ai.naming.title").to_string(),
            AiWorkflowTarget::SupportReply { .. } => t!("ai.workflow.support_title").to_string(),
            AiWorkflowTarget::SupportSummary { .. } => {
                t!("ai.workflow.support_summary_title").to_string()
            }
            AiWorkflowTarget::SupportTriage { .. } => {
                t!("ai.workflow.support_triage_title").to_string()
            }
            AiWorkflowTarget::IssueReply { .. } => t!("ai.workflow.issue_reply_title").to_string(),
            AiWorkflowTarget::IssueSummary { .. } => {
                t!("ai.workflow.issue_summary_title").to_string()
            }
            AiWorkflowTarget::IssueTriage { .. } => {
                t!("ai.workflow.issue_triage_title").to_string()
            }
            AiWorkflowTarget::ScriptGenerate { .. } => t!("ai.workflow.script_title").to_string(),
            AiWorkflowTarget::ScriptExplain { .. } => {
                t!("ai.workflow.script_explain_title").to_string()
            }
            AiWorkflowTarget::ScriptReview { .. } => {
                t!("ai.workflow.script_review_title").to_string()
            }
            AiWorkflowTarget::ScriptFix { .. } => t!("ai.workflow.script_fix_title").to_string(),
            AiWorkflowTarget::TerminalCommand { .. } => {
                t!("ai.workflow.terminal_command_title").to_string()
            }
            AiWorkflowTarget::TerminalDiagnose { .. } => {
                t!("ai.workflow.terminal_diagnose_title").to_string()
            }
        };
        let comparison_original = match &target {
            AiWorkflowTarget::ScriptGenerate { script_id }
            | AiWorkflowTarget::ScriptFix { script_id } => Uuid::parse_str(script_id)
                .ok()
                .and_then(|id| self.scripts.read(cx).script_body(id)),
            _ => None,
        };
        let issue_triage_current = match &target {
            AiWorkflowTarget::SupportTriage { .. } => {
                self.support.read(cx).selected_ticket_triage_state()
            }
            AiWorkflowTarget::IssueTriage { issue_id } => self
                .issue_detail
                .as_ref()
                .filter(|issue| &issue.id == issue_id)
                .map(|issue| (issue.priority.clone(), issue.assignee.clone())),
            _ => None,
        };
        let action_policy = self.app_config.ai.policies.level_for(target.capability());
        let workflow = cx.new(|cx| {
            AiWorkflowView::new(
                AiWorkflowInit {
                    target,
                    backend: self.app_config.ai.backend,
                    model: self.app_config.ai.model.clone(),
                    pending,
                    comparison_original,
                    issue_triage_current,
                    action_policy,
                },
                cx,
            )
        });
        let subscription = cx.subscribe(&workflow, |this, _view, event: &AiWorkflowEvent, cx| {
            this.handle_ai_workflow_event(event.clone(), cx);
        });
        let sheet_workflow = workflow.clone();
        let workspace = cx.entity().downgrade();
        self.ai_workflow_sheet = Some(cx.new(move |sheet_cx| {
            Sheet::new(sheet_cx)
                .size(SheetSize::Assistant)
                .variant(SheetVariant::Assistant)
                .title(title)
                .description(t!("ai.workflow.description").to_string())
                .dynamic_content(move || sheet_workflow.clone())
                .on_close(move |_window, cx| {
                    if let Some(workspace) = workspace.upgrade() {
                        workspace.update(cx, |this, cx| this.close_ai_workflow(cx));
                    }
                })
        }));
        self.ai_workflow = Some(workflow.clone());
        self._ai_workflow_sub = Some(subscription);
        if should_generate {
            workflow.update(cx, |view, cx| view.generate(cx));
        }
        cx.notify();
    }

    fn ai_workflow_context(&self, target: &AiWorkflowTarget, cx: &App) -> AiContext {
        match target {
            AiWorkflowTarget::EntityNaming { kind, .. } => {
                let entity = match kind {
                    AiNamingKind::Script => self
                        .script_form
                        .as_ref()
                        .map(|form| form.read(cx).ai_context_data(cx))
                        .unwrap_or_else(|| self.script_ai_context_data(cx)),
                    AiNamingKind::Terminal => self.terminal.read(cx).ai_context_data(),
                    AiNamingKind::Tunnel => self
                        .port_forward_form
                        .as_ref()
                        .map(|form| form.read(cx).ai_context_data(cx))
                        .unwrap_or_else(|| serde_json::json!({ "tunnel": null })),
                    AiNamingKind::Issue => serde_json::json!({
                        "request": {
                            "current_title": self.issue_title_state.read(cx).content().to_string(),
                            "description": self.issue_body_state.read(cx).content().to_string(),
                            "priority": self.issue_new_priority,
                        }
                    }),
                };
                AiContext::new(
                    AiSurface::Naming,
                    t!("ai.context.entity_naming").to_string(),
                    self.ai_context_data_with_hosts(entity),
                )
            }
            AiWorkflowTarget::SupportReply { .. } | AiWorkflowTarget::SupportSummary { .. } => {
                AiContext::new(
                    AiSurface::Support,
                    t!("ai.context.support").to_string(),
                    self.ai_context_data_with_hosts(self.support.read(cx).ai_context_data()),
                )
            }
            AiWorkflowTarget::SupportTriage { .. } => AiContext::new(
                AiSurface::Support,
                t!("ai.context.support").to_string(),
                self.ai_context_data_with_hosts(
                    self.support.read(cx).support_triage_context_data(),
                ),
            ),
            AiWorkflowTarget::IssueReply { .. } | AiWorkflowTarget::IssueSummary { .. } => {
                AiContext::new(
                    AiSurface::Issue,
                    t!("ai.context.issue").to_string(),
                    self.ai_context_data_with_hosts(self.support.read(cx).ai_context_data()),
                )
            }
            AiWorkflowTarget::IssueTriage { .. } => AiContext::new(
                AiSurface::Issue,
                t!("ai.context.issue").to_string(),
                self.ai_context_data_with_hosts(self.support.read(cx).issue_triage_context_data()),
            ),
            AiWorkflowTarget::ScriptGenerate { .. }
            | AiWorkflowTarget::ScriptExplain { .. }
            | AiWorkflowTarget::ScriptReview { .. } => AiContext::new(
                AiSurface::Script,
                t!("ai.context.script").to_string(),
                self.script_ai_context_data(cx),
            ),
            AiWorkflowTarget::ScriptFix { .. } => AiContext::new(
                AiSurface::Script,
                t!("ai.context.script_fix").to_string(),
                serde_json::json!({
                    "script": self.scripts.read(cx).ai_fix_context_data(),
                    "hosts": self.ai_hosts_context_data(),
                }),
            ),
            AiWorkflowTarget::TerminalCommand { .. }
            | AiWorkflowTarget::TerminalDiagnose { .. } => AiContext::new(
                AiSurface::Terminal,
                t!("ai.context.terminal").to_string(),
                self.ai_context_data_with_hosts(self.terminal.read(cx).ai_context_data()),
            ),
        }
    }

    fn prepare_ai_action(
        &mut self,
        target: AiWorkflowTarget,
        result: String,
        cx: &mut Context<Self>,
    ) {
        let model = if self.app_config.ai.model.trim().is_empty() {
            self.app_config.ai.backend.default_model().to_string()
        } else {
            self.app_config.ai.model.clone()
        };
        let plan = match &target {
            AiWorkflowTarget::TerminalCommand { session_id } => {
                if crate::terminal_view::validate_ai_command(&result).is_err() {
                    self.show_toast(
                        t!("toast.ai.command_invalid").to_string(),
                        ToastLevel::Error,
                        cx,
                    );
                    return;
                }
                let context = self.terminal.read(cx).ai_context_data();
                if context
                    .get("session_id")
                    .and_then(serde_json::Value::as_str)
                    != Some(session_id.as_str())
                {
                    self.show_toast(
                        t!("toast.ai.action_target_changed").to_string(),
                        ToastLevel::Error,
                        cx,
                    );
                    return;
                }
                let label = context
                    .get("title")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("Terminal");
                AiActionPlan::new(AiActionPlanSpec {
                    capability: target.capability(),
                    kind: AiActionKind::TerminalCommand,
                    risk: AiActionRisk::High,
                    target_id: session_id.clone(),
                    target_label: label.to_string(),
                    backend: self.app_config.ai.backend,
                    model,
                    timeout_secs: 1_800,
                    payload: AiActionPayload::TerminalCommand { command: result },
                })
            }
            AiWorkflowTarget::ScriptGenerate { script_id }
            | AiWorkflowTarget::ScriptFix { script_id } => {
                let Some(script) = Uuid::parse_str(script_id).ok().and_then(|id| {
                    self.scripts
                        .read(cx)
                        .scripts
                        .iter()
                        .find(|script| script.id == id)
                        .cloned()
                }) else {
                    self.show_toast(
                        t!("toast.ai.action_target_changed").to_string(),
                        ToastLevel::Error,
                        cx,
                    );
                    return;
                };
                AiActionPlan::new(AiActionPlanSpec {
                    capability: target.capability(),
                    kind: AiActionKind::ScriptExecution,
                    risk: AiActionRisk::High,
                    target_id: script.id.to_string(),
                    target_label: script.name,
                    backend: self.app_config.ai.backend,
                    model,
                    timeout_secs: 1_800,
                    payload: AiActionPayload::ScriptExecution {
                        body: shelldeck_core::ai::clean_generated_script_body(&result),
                    },
                })
            }
            AiWorkflowTarget::SupportReply { ticket_id } => {
                let Some((selected_id, label)) = self.support.read(cx).selected_ticket_identity()
                else {
                    return;
                };
                if &selected_id != ticket_id {
                    self.show_toast(
                        t!("toast.ai.action_target_changed").to_string(),
                        ToastLevel::Error,
                        cx,
                    );
                    return;
                }
                AiActionPlan::new(AiActionPlanSpec {
                    capability: target.capability(),
                    kind: AiActionKind::SupportSend,
                    risk: AiActionRisk::Moderate,
                    target_id: ticket_id.clone(),
                    target_label: label,
                    backend: self.app_config.ai.backend,
                    model,
                    timeout_secs: 30,
                    payload: AiActionPayload::SupportSend { body: result },
                })
            }
            _ => return,
        };
        match plan {
            Ok(plan) => self.stage_ai_action(plan, cx),
            Err(error) => self.show_toast(error.to_string(), ToastLevel::Error, cx),
        }
    }

    fn prepare_ai_diagnostic_step(
        &mut self,
        target: AiWorkflowTarget,
        command: String,
        cx: &mut Context<Self>,
    ) {
        let AiWorkflowTarget::TerminalDiagnose { session_id } = &target else {
            return;
        };
        if validate_diagnostic_command(&command).is_err() {
            self.show_toast(
                t!("toast.ai.command_invalid").to_string(),
                ToastLevel::Error,
                cx,
            );
            return;
        }
        let context = self.terminal.read(cx).ai_context_data();
        if context
            .get("session_id")
            .and_then(serde_json::Value::as_str)
            != Some(session_id.as_str())
        {
            self.show_toast(
                t!("toast.ai.action_target_changed").to_string(),
                ToastLevel::Error,
                cx,
            );
            return;
        }
        let label = context
            .get("title")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("Terminal");
        let model = if self.app_config.ai.model.trim().is_empty() {
            self.app_config.ai.backend.default_model().to_string()
        } else {
            self.app_config.ai.model.clone()
        };
        match AiActionPlan::new(AiActionPlanSpec {
            capability: target.capability(),
            kind: AiActionKind::TerminalCommand,
            risk: AiActionRisk::High,
            target_id: session_id.clone(),
            target_label: label.to_string(),
            backend: self.app_config.ai.backend,
            model,
            timeout_secs: 1_800,
            payload: AiActionPayload::TerminalCommand { command },
        }) {
            Ok(plan) => self.stage_ai_action(plan, cx),
            Err(error) => self.show_toast(error.to_string(), ToastLevel::Error, cx),
        }
    }

    fn start_ai_diagnostic_sequence(
        &mut self,
        target: AiWorkflowTarget,
        plan: shelldeck_core::ai::AiDiagnosticPlan,
        cx: &mut Context<Self>,
    ) {
        let AiWorkflowTarget::TerminalDiagnose { session_id } = &target else {
            return;
        };
        let Ok(session_id) = Uuid::parse_str(session_id) else {
            return;
        };
        if plan
            .steps
            .iter()
            .any(|step| validate_diagnostic_command(&step.command).is_err())
        {
            self.show_toast(
                t!("toast.ai.command_invalid").to_string(),
                ToastLevel::Error,
                cx,
            );
            return;
        }
        let remaining = plan
            .steps
            .into_iter()
            .map(|step| step.command)
            .collect::<VecDeque<_>>();
        self.ai_diagnostic_sequences
            .insert(session_id, AiDiagnosticSequence { target, remaining });
        self.prepare_next_ai_diagnostic_step(session_id, String::new(), cx);
    }

    fn prepare_next_ai_diagnostic_step(
        &mut self,
        session_id: Uuid,
        _previous_output: String,
        cx: &mut Context<Self>,
    ) {
        let next = self
            .ai_diagnostic_sequences
            .get_mut(&session_id)
            .and_then(|sequence| {
                sequence
                    .remaining
                    .pop_front()
                    .map(|command| (sequence.target.clone(), command))
            });
        if let Some((target, command)) = next {
            self.prepare_ai_diagnostic_step(target, command, cx);
            return;
        }
        if let Some(sequence) = self.ai_diagnostic_sequences.remove(&session_id) {
            self.set_ai_workflow_task_status(&sequence.target, AiTaskStatus::Succeeded, cx);
            self.show_toast(
                t!("toast.ai.diagnostic_completed").to_string(),
                ToastLevel::Success,
                cx,
            );
        }
    }

    fn prepare_jean_dispatch(&mut self, prompt: String, cx: &mut Context<Self>) {
        if self.effective_jean_config().is_none() || prompt.trim().is_empty() {
            return;
        }
        let model = if self.app_config.ai.model.trim().is_empty() {
            self.app_config.ai.backend.default_model().to_string()
        } else {
            self.app_config.ai.model.clone()
        };
        let (target_id, target_label) = self
            .support
            .read(cx)
            .selected_ticket_identity()
            .unwrap_or_else(|| ("jean".to_string(), "JeanClaude".to_string()));
        match AiActionPlan::new(AiActionPlanSpec {
            capability: shelldeck_core::ai::AiCapability::JeanDispatch,
            kind: AiActionKind::JeanDispatch,
            risk: AiActionRisk::Moderate,
            target_id,
            target_label,
            backend: self.app_config.ai.backend,
            model,
            timeout_secs: 30,
            payload: AiActionPayload::JeanDispatch { prompt },
        }) {
            Ok(plan) => self.stage_ai_action(plan, cx),
            Err(error) => self.show_toast(error.to_string(), ToastLevel::Error, cx),
        }
    }

    fn prepare_fleet_dispatch(
        &mut self,
        issue_id: String,
        instance_id: String,
        cx: &mut Context<Self>,
    ) {
        if !self.issues_staff
            || !self
                .issues_instances
                .iter()
                .any(|instance| instance.id == instance_id)
        {
            self.show_toast(
                t!("toast.ai.action_target_changed").to_string(),
                ToastLevel::Error,
                cx,
            );
            return;
        }
        let Some(issue) = self
            .issues_list
            .iter()
            .find(|issue| issue.id == issue_id)
            .cloned()
        else {
            return;
        };
        let model = if self.app_config.ai.model.trim().is_empty() {
            self.app_config.ai.backend.default_model().to_string()
        } else {
            self.app_config.ai.model.clone()
        };
        match AiActionPlan::new(AiActionPlanSpec {
            capability: shelldeck_core::ai::AiCapability::FleetDispatch,
            kind: AiActionKind::FleetDispatch,
            risk: AiActionRisk::High,
            target_id: issue.id.clone(),
            target_label: issue.title,
            backend: self.app_config.ai.backend,
            model,
            timeout_secs: 30,
            payload: AiActionPayload::FleetDispatch {
                issue_id: issue.id,
                instance_id,
            },
        }) {
            Ok(plan) => self.stage_ai_action(plan, cx),
            Err(error) => self.show_toast(error.to_string(), ToastLevel::Error, cx),
        }
    }

    fn audit_ai_action(&mut self, plan: &AiActionPlan, status: &str, cx: &mut Context<Self>) {
        let task_status = match status {
            "confirmed" | "automatic" | "started" | "submitted" => Some(AiTaskStatus::Executing),
            "succeeded" => Some(AiTaskStatus::Succeeded),
            "failed" | "timed_out" => Some(AiTaskStatus::Failed),
            "cancelled" | "target_changed" | "target_closed" => Some(AiTaskStatus::Cancelled),
            _ => None,
        };
        if let Some(task_status) = task_status {
            self.set_ai_action_task_status(plan, task_status, None, cx);
        }
        if !self.window_active
            && self.app_config.tray.notify_ai_tasks
            && matches!(status, "succeeded" | "failed" | "timed_out")
        {
            self.emit_tray_notification(TrayNotification::AiTaskDone {
                success: status == "succeeded",
            });
        }
        let kind = match plan.kind {
            AiActionKind::TerminalCommand => ActivityKind::Terminal,
            AiActionKind::ScriptExecution => ActivityKind::Script,
            AiActionKind::SupportSend => ActivityKind::Support,
            AiActionKind::JeanDispatch => ActivityKind::Jean,
            AiActionKind::FleetDispatch => ActivityKind::Fleet,
        };
        let actor = self
            .app_config
            .account
            .as_ref()
            .map(|account| account.email.as_str())
            .filter(|email| !email.trim().is_empty())
            .unwrap_or("local-session");
        self.add_activity_entry(
            ActivityEntry::new(
                kind,
                t!(
                    "activity.ai.action",
                    status = status,
                    target = plan.target_label.as_str()
                )
                .to_string(),
            )
            .with_target(plan.target_id.clone(), plan.target_label.clone())
            .with_detail(format!("actor={actor} {}", plan.audit_detail(status))),
            cx,
        );
    }

    fn cancel_ai_action_confirmation(&mut self, cx: &mut Context<Self>) {
        if let Some(plan) = self.ai_action_confirmation.take() {
            if plan.capability == shelldeck_core::ai::AiCapability::TerminalDiagnose {
                if let Ok(session_id) = Uuid::parse_str(&plan.target_id) {
                    self.ai_diagnostic_sequences.remove(&session_id);
                }
            }
            let resumable = self
                .ai_tasks
                .iter()
                .find(|task| task.id == plan.id)
                .is_some_and(|task| !task.result.trim().is_empty());
            self.set_ai_action_task_status(
                &plan,
                if resumable {
                    AiTaskStatus::Ready
                } else {
                    AiTaskStatus::Cancelled
                },
                None,
                cx,
            );
        }
        cx.notify();
    }

    fn track_ai_script_run(&mut self, script_id: Uuid, plan: AiActionPlan, cx: &mut Context<Self>) {
        self.audit_ai_action(&plan, "started", cx);
        let action_id = plan.id;
        let timeout = std::time::Duration::from_secs(plan.timeout_secs);
        self.ai_script_runs.insert(script_id, plan);
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            cx.background_executor().timer(timeout).await;
            let _ = this.update(cx, |workspace, cx| {
                let still_same_action = workspace
                    .ai_script_runs
                    .get(&script_id)
                    .is_some_and(|plan| plan.id == action_id);
                let still_running = workspace.scripts.read(cx).running_script_id == Some(script_id);
                if still_same_action && still_running {
                    if let Some(plan) = workspace.ai_script_runs.remove(&script_id) {
                        workspace.audit_ai_action(&plan, "timed_out", cx);
                    }
                    workspace.handle_script_event(&ScriptEvent::StopScript, cx);
                    workspace.show_toast(
                        t!("toast.ai.action_timed_out").to_string(),
                        ToastLevel::Warning,
                        cx,
                    );
                }
            });
        })
        .detach();
    }

    fn track_ai_terminal_run(
        &mut self,
        session_id: Uuid,
        plan: AiActionPlan,
        cx: &mut Context<Self>,
    ) {
        let action_id = plan.id;
        let timeout = std::time::Duration::from_secs(plan.timeout_secs);
        self.ai_terminal_runs.insert(session_id, plan);
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            cx.background_executor().timer(timeout).await;
            let _ = this.update(cx, |workspace, cx| {
                let still_same_action = workspace
                    .ai_terminal_runs
                    .get(&session_id)
                    .is_some_and(|plan| plan.id == action_id);
                if !still_same_action {
                    return;
                }
                let _ = workspace
                    .terminal
                    .update(cx, |terminal, cx| terminal.stop_ai_command(session_id, cx));
                workspace.ai_diagnostic_sequences.remove(&session_id);
                if let Some(plan) = workspace.ai_terminal_runs.remove(&session_id) {
                    workspace.audit_ai_action(&plan, "timed_out", cx);
                }
                workspace.show_toast(
                    t!("toast.ai.action_timed_out").to_string(),
                    ToastLevel::Warning,
                    cx,
                );
            });
        })
        .detach();
    }

    pub(super) fn finish_ai_script_run(
        &mut self,
        script_id: Uuid,
        status: &str,
        cx: &mut Context<Self>,
    ) {
        if let Some(plan) = self.ai_script_runs.remove(&script_id) {
            self.audit_ai_action(&plan, status, cx);
        }
    }

    fn confirm_ai_action(&mut self, cx: &mut Context<Self>) {
        let Some(plan) = self.ai_action_confirmation.take() else {
            return;
        };
        self.audit_ai_action(
            &plan,
            if plan.autonomy == shelldeck_core::ai::AiAutonomyLevel::Automatic {
                "automatic"
            } else {
                "confirmed"
            },
            cx,
        );
        let payload = plan.payload.clone();
        match payload {
            AiActionPayload::TerminalCommand { command } => {
                let executed = Uuid::parse_str(&plan.target_id)
                    .ok()
                    .is_some_and(|session_id| {
                        self.terminal
                            .update(cx, |terminal, cx| {
                                terminal.execute_ai_command(session_id, &command, cx)
                            })
                            .is_ok()
                    });
                if executed {
                    self.audit_ai_action(&plan, "submitted", cx);
                    if let Ok(session_id) = Uuid::parse_str(&plan.target_id) {
                        self.track_ai_terminal_run(session_id, plan.clone(), cx);
                    }
                    self.close_ai_workflow(cx);
                    self.show_toast(
                        t!("toast.ai.action_submitted").to_string(),
                        ToastLevel::Success,
                        cx,
                    );
                } else {
                    if plan.capability == shelldeck_core::ai::AiCapability::TerminalDiagnose {
                        if let Ok(session_id) = Uuid::parse_str(&plan.target_id) {
                            self.ai_diagnostic_sequences.remove(&session_id);
                        }
                    }
                    self.audit_ai_action(&plan, "target_changed", cx);
                    self.show_toast(
                        t!("toast.ai.action_target_changed").to_string(),
                        ToastLevel::Error,
                        cx,
                    );
                }
            }
            AiActionPayload::ScriptExecution { body } => {
                let Some(mut script) = Uuid::parse_str(&plan.target_id).ok().and_then(|id| {
                    self.scripts
                        .read(cx)
                        .scripts
                        .iter()
                        .find(|script| script.id == id)
                        .cloned()
                }) else {
                    self.audit_ai_action(&plan, "target_changed", cx);
                    self.show_toast(
                        t!("toast.ai.action_target_changed").to_string(),
                        ToastLevel::Error,
                        cx,
                    );
                    return;
                };
                script.body = body;
                let script_id = script.id;
                self.close_ai_workflow(cx);
                self.handle_script_event(&ScriptEvent::RunScript(script), cx);
                if self.scripts.read(cx).running_script_id == Some(script_id) {
                    self.track_ai_script_run(script_id, plan, cx);
                }
            }
            AiActionPayload::SupportSend { body } => {
                let selected = self.support.read(cx).selected_ticket_identity();
                if selected.as_ref().map(|(id, _)| id.as_str()) != Some(plan.target_id.as_str()) {
                    self.audit_ai_action(&plan, "target_changed", cx);
                    self.show_toast(
                        t!("toast.ai.action_target_changed").to_string(),
                        ToastLevel::Error,
                        cx,
                    );
                    return;
                }
                self.close_ai_workflow(cx);
                self.execute_ai_support_send(plan, body, cx);
            }
            AiActionPayload::JeanDispatch { prompt } => {
                self.execute_ai_jean_dispatch(plan, prompt, cx);
            }
            AiActionPayload::FleetDispatch {
                issue_id,
                instance_id,
            } => {
                self.execute_ai_fleet_dispatch(plan, issue_id, instance_id, cx);
            }
        }
        cx.notify();
    }

    fn execute_ai_support_send(
        &mut self,
        plan: AiActionPlan,
        body: String,
        cx: &mut Context<Self>,
    ) {
        if !self.app_config.cloud_sync.is_configured() {
            return;
        }
        let base = self.account_base_url();
        let token = self.app_config.cloud_sync.token.clone();
        let ticket_id = plan.target_id.clone();
        self.support.update(cx, |view, cx| {
            view.set_loading(true);
            cx.notify();
        });
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(
                    async move { manage_support::support_reply(&base, &token, &ticket_id, &body) },
                )
                .await;
            let _ = this.update(cx, |workspace, cx| match result {
                Ok(ticket) => {
                    workspace.support.update(cx, |view, cx| {
                        view.set_detail(ticket, cx);
                        cx.notify();
                    });
                    workspace.audit_ai_action(&plan, "succeeded", cx);
                    workspace.refresh_support(cx);
                    workspace.show_toast(
                        t!("toast.ai.action_succeeded").to_string(),
                        ToastLevel::Success,
                        cx,
                    );
                }
                Err(error) => {
                    workspace.audit_ai_action(&plan, "failed", cx);
                    let message = cloud_account::user_message(&error);
                    workspace.support.update(cx, |view, cx| {
                        view.set_error(message.clone());
                        cx.notify();
                    });
                    workspace.show_toast(message, ToastLevel::Error, cx);
                }
            });
        })
        .detach();
    }

    fn execute_ai_jean_dispatch(
        &mut self,
        plan: AiActionPlan,
        prompt: String,
        cx: &mut Context<Self>,
    ) {
        let Some(config) = self.effective_jean_config() else {
            self.audit_ai_action(&plan, "target_changed", cx);
            return;
        };
        self.audit_ai_action(&plan, "started", cx);
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move { jeanclaude::say(&config, &prompt) })
                .await;
            let _ = this.update(cx, |workspace, cx| {
                match result {
                    Ok(_) => {
                        workspace.audit_ai_action(&plan, "succeeded", cx);
                        workspace.show_toast(
                            t!("toast.jean.sent").to_string(),
                            ToastLevel::Success,
                            cx,
                        );
                    }
                    Err(error) => {
                        workspace.audit_ai_action(&plan, "failed", cx);
                        workspace.show_toast(
                            t!(
                                "toast.jean.error",
                                error = cloud_account::user_message(&error)
                            )
                            .to_string(),
                            ToastLevel::Error,
                            cx,
                        );
                    }
                }
                workspace.refresh_jean_state(cx);
            });
        })
        .detach();
    }

    fn execute_ai_fleet_dispatch(
        &mut self,
        plan: AiActionPlan,
        issue_id: String,
        instance_id: String,
        cx: &mut Context<Self>,
    ) {
        if !self.issues_staff
            || !self
                .issues_instances
                .iter()
                .any(|instance| instance.id == instance_id)
            || !self.issues_list.iter().any(|issue| issue.id == issue_id)
        {
            self.audit_ai_action(&plan, "target_changed", cx);
            self.show_toast(
                t!("toast.ai.action_target_changed").to_string(),
                ToastLevel::Error,
                cx,
            );
            return;
        }
        let Some((base, token)) = self.fleet_base_token() else {
            return;
        };
        self.audit_ai_action(&plan, "started", cx);
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(
                    async move { issues::dispatch_issue(&base, &token, &issue_id, &instance_id) },
                )
                .await;
            let _ = this.update(cx, |workspace, cx| match result {
                Ok(issue) => {
                    workspace.audit_ai_action(&plan, "succeeded", cx);
                    workspace.upsert_issue_in_list(issue.clone());
                    workspace.issue_detail = Some(issue);
                    workspace.push_issues_to_support(cx);
                    workspace.show_toast(
                        t!("toast.ai.action_succeeded").to_string(),
                        ToastLevel::Success,
                        cx,
                    );
                }
                Err(error) => {
                    workspace.audit_ai_action(&plan, "failed", cx);
                    workspace.show_toast(
                        t!(
                            "toast.issue.staff_failed",
                            error = cloud_account::user_message(&error)
                        )
                        .to_string(),
                        ToastLevel::Error,
                        cx,
                    );
                }
            });
        })
        .detach();
    }

    fn handle_ai_workflow_event(&mut self, event: AiWorkflowEvent, cx: &mut Context<Self>) {
        match event {
            AiWorkflowEvent::Generate {
                request_id,
                target,
                instructions,
            } => {
                let base = match &target {
                    AiWorkflowTarget::EntityNaming { .. } => {
                        t!("ai.prompt.entity_name").to_string()
                    }
                    AiWorkflowTarget::SupportReply { .. } => {
                        t!("ai.prompt.support_reply").to_string()
                    }
                    AiWorkflowTarget::SupportSummary { .. } => {
                        t!("ai.prompt.support_summary").to_string()
                    }
                    AiWorkflowTarget::SupportTriage { .. } => {
                        t!("ai.prompt.support_triage").to_string()
                    }
                    AiWorkflowTarget::IssueReply { .. } => t!("ai.prompt.issue_reply").to_string(),
                    AiWorkflowTarget::IssueSummary { .. } => {
                        t!("ai.prompt.issue_summary").to_string()
                    }
                    AiWorkflowTarget::IssueTriage { .. } => {
                        t!("ai.prompt.issue_triage").to_string()
                    }
                    AiWorkflowTarget::ScriptGenerate { .. } => {
                        t!("ai.prompt.script_generate").to_string()
                    }
                    AiWorkflowTarget::ScriptExplain { .. } => {
                        t!("ai.prompt.script_explain").to_string()
                    }
                    AiWorkflowTarget::ScriptReview { .. } => {
                        t!("ai.prompt.script_review").to_string()
                    }
                    AiWorkflowTarget::ScriptFix { .. } => t!("ai.prompt.script_fix").to_string(),
                    AiWorkflowTarget::TerminalCommand { .. } => {
                        t!("ai.prompt.terminal_command_strict").to_string()
                    }
                    AiWorkflowTarget::TerminalDiagnose { .. } => {
                        t!("ai.prompt.terminal_diagnostic_plan").to_string()
                    }
                };
                let prompt = if instructions.trim().is_empty() {
                    base
                } else {
                    format!(
                        "{base}\n\n{}:\n{}",
                        t!("ai.workflow.additional_instructions"),
                        instructions.trim()
                    )
                };
                let context = self.ai_workflow_context(&target, cx);
                let config = self.app_config.ai.clone();
                let structured_issue_triage = matches!(
                    &target,
                    AiWorkflowTarget::SupportTriage { .. } | AiWorkflowTarget::IssueTriage { .. }
                );
                let structured_name = matches!(&target, AiWorkflowTarget::EntityNaming { .. });
                let structured_diagnostic =
                    matches!(&target, AiWorkflowTarget::TerminalDiagnose { .. });
                let issue_triage_repair =
                    t!("ai.prompt.issue_triage_repair", error = "__TRIAGE_ERROR__").to_string();
                let diagnostic_repair = t!(
                    "ai.prompt.terminal_diagnostic_repair",
                    error = "__DIAGNOSTIC_ERROR__"
                )
                .to_string();
                let workflow = self.ai_workflow.as_ref().map(|entity| entity.downgrade());
                let task_id = self.begin_ai_workflow_task(&target, instructions, cx);
                let automatic_support_triage =
                    matches!(&target, AiWorkflowTarget::SupportTriage { .. })
                        && self.app_config.ai.policies.support_triage
                            == shelldeck_core::ai::AiAutonomyLevel::Automatic;
                let generated_target = target.clone();
                cx.spawn(async move |this, cx: &mut AsyncApp| {
                    let result = cx
                        .background_executor()
                        .spawn(async move {
                            let client = create_client(&config)?;
                            let response = client.complete(&prompt, context.clone())?;
                            if structured_name {
                                parse_generated_name(&response.text)?;
                                return Ok::<String, shelldeck_core::ShellDeckError>(response.text);
                            }
                            if structured_diagnostic {
                                return match parse_diagnostic_plan(&response.text) {
                                    Ok(_) => {
                                        Ok::<String, shelldeck_core::ShellDeckError>(response.text)
                                    }
                                    Err(first_error) => {
                                        let repair = diagnostic_repair.replace(
                                            "__DIAGNOSTIC_ERROR__",
                                            &first_error.to_string(),
                                        );
                                        let repaired = client.complete(
                                            &format!("{prompt}\n\n{repair}"),
                                            context.clone(),
                                        )?;
                                        parse_diagnostic_plan(&repaired.text)?;
                                        Ok::<String, shelldeck_core::ShellDeckError>(repaired.text)
                                    }
                                };
                            }
                            if !structured_issue_triage {
                                return Ok::<String, shelldeck_core::ShellDeckError>(response.text);
                            }
                            match parse_issue_triage_proposal(&response.text) {
                                Ok(_) => {
                                    Ok::<String, shelldeck_core::ShellDeckError>(response.text)
                                }
                                Err(first_error) => {
                                    let repair = issue_triage_repair
                                        .replace("__TRIAGE_ERROR__", &first_error.to_string());
                                    let repaired = client
                                        .complete(&format!("{prompt}\n\n{repair}"), context)?;
                                    parse_issue_triage_proposal(&repaired.text)?;
                                    Ok::<String, shelldeck_core::ShellDeckError>(repaired.text)
                                }
                            }
                        })
                        .await
                        .map_err(|error| error.to_string());
                    let _ = this.update(cx, |workspace, cx| {
                        workspace.finish_ai_workflow_task(task_id, &result, cx);
                        if automatic_support_triage {
                            if let (AiWorkflowTarget::SupportTriage { ticket_id }, Ok(raw)) =
                                (&generated_target, &result)
                            {
                                if let Ok(proposal) = parse_issue_triage_proposal(raw) {
                                    workspace.set_ai_workflow_task_status(
                                        &generated_target,
                                        AiTaskStatus::Applied,
                                        cx,
                                    );
                                    workspace.close_ai_workflow(cx);
                                    workspace.apply_support_triage(ticket_id.clone(), proposal, cx);
                                }
                            }
                        }
                    });
                    if let Some(workflow) = workflow.and_then(|workflow| workflow.upgrade()) {
                        let _ = workflow.update(cx, |view, cx| {
                            view.set_result(request_id, result, cx);
                        });
                    }
                })
                .detach();
            }
            AiWorkflowEvent::Accept { target, result } => {
                if let AiWorkflowTarget::EntityNaming { kind, target_id } = &target {
                    let generated = match parse_generated_name(&result) {
                        Ok(generated) => generated,
                        Err(error) => {
                            self.show_toast(
                                t!("toast.ai.name_invalid", error = error.to_string()).to_string(),
                                ToastLevel::Error,
                                cx,
                            );
                            return;
                        }
                    };
                    let applied = match kind {
                        AiNamingKind::Script => self.script_form.as_ref().is_some_and(|form| {
                            form.update(cx, |form, cx| form.apply_ai_name(generated.name, cx));
                            true
                        }),
                        AiNamingKind::Terminal => {
                            Uuid::parse_str(target_id).ok().is_some_and(|id| {
                                self.terminal
                                    .update(cx, |view, cx| {
                                        view.apply_ai_name(id, generated.name, cx)
                                    })
                                    .is_ok()
                            })
                        }
                        AiNamingKind::Tunnel => {
                            self.port_forward_form.as_ref().is_some_and(|form| {
                                form.update(cx, |form, cx| form.apply_ai_name(generated.name, cx));
                                true
                            })
                        }
                        AiNamingKind::Issue => {
                            if self.user_new_request_sheet_open {
                                self.issue_title_state.update(cx, |state, cx| {
                                    state.replace_content(generated.name, cx)
                                });
                                true
                            } else {
                                false
                            }
                        }
                    };
                    if !applied {
                        self.show_toast(
                            t!("toast.ai.name_target_missing").to_string(),
                            ToastLevel::Error,
                            cx,
                        );
                        return;
                    }
                    self.set_ai_workflow_task_status(&target, AiTaskStatus::Applied, cx);
                    self.close_ai_workflow(cx);
                    self.show_toast(
                        t!("toast.ai.name_applied").to_string(),
                        ToastLevel::Success,
                        cx,
                    );
                    return;
                }
                if let AiWorkflowTarget::SupportTriage { ticket_id } = &target {
                    let proposal = match parse_issue_triage_proposal(&result) {
                        Ok(proposal) => proposal,
                        Err(error) => {
                            self.show_toast(
                                t!("toast.ai.triage_invalid", error = error.to_string())
                                    .to_string(),
                                ToastLevel::Error,
                                cx,
                            );
                            return;
                        }
                    };
                    if self.app_config.ai.policies.support_triage
                        == shelldeck_core::ai::AiAutonomyLevel::Preparation
                    {
                        cx.write_to_clipboard(ClipboardItem::new_string(result));
                        self.set_ai_workflow_task_status(&target, AiTaskStatus::Applied, cx);
                        self.close_ai_workflow(cx);
                        self.show_toast(
                            t!("toast.ai.analysis_copied").to_string(),
                            ToastLevel::Success,
                            cx,
                        );
                        return;
                    }
                    self.set_ai_workflow_task_status(&target, AiTaskStatus::Applied, cx);
                    let ticket_id = ticket_id.clone();
                    self.close_ai_workflow(cx);
                    self.apply_support_triage(ticket_id, proposal, cx);
                    return;
                }
                if let AiWorkflowTarget::IssueTriage { issue_id } = &target {
                    let proposal = match parse_issue_triage_proposal(&result) {
                        Ok(proposal) => proposal,
                        Err(error) => {
                            self.show_toast(
                                t!("toast.ai.triage_invalid", error = error.to_string())
                                    .to_string(),
                                ToastLevel::Error,
                                cx,
                            );
                            return;
                        }
                    };
                    self.set_ai_workflow_task_status(&target, AiTaskStatus::Applied, cx);
                    let issue_id = issue_id.clone();
                    self.close_ai_workflow(cx);
                    self.apply_issue_triage(issue_id, proposal, cx);
                    return;
                }
                let copied = matches!(
                    &target,
                    AiWorkflowTarget::SupportSummary { .. }
                        | AiWorkflowTarget::SupportTriage { .. }
                        | AiWorkflowTarget::IssueSummary { .. }
                        | AiWorkflowTarget::ScriptExplain { .. }
                        | AiWorkflowTarget::ScriptReview { .. }
                        | AiWorkflowTarget::TerminalDiagnose { .. }
                );
                match &target {
                    AiWorkflowTarget::EntityNaming { .. } => unreachable!(),
                    AiWorkflowTarget::SupportReply { .. } => {
                        self.support.update(cx, |view, cx| {
                            view.set_composer_draft(result, cx);
                        });
                    }
                    AiWorkflowTarget::IssueReply { .. } => {
                        self.support.update(cx, |view, cx| {
                            view.set_composer_draft(result, cx);
                        });
                    }
                    AiWorkflowTarget::SupportSummary { .. }
                    | AiWorkflowTarget::SupportTriage { .. }
                    | AiWorkflowTarget::IssueSummary { .. }
                    | AiWorkflowTarget::ScriptExplain { .. }
                    | AiWorkflowTarget::ScriptReview { .. } => {
                        cx.write_to_clipboard(ClipboardItem::new_string(result));
                    }
                    AiWorkflowTarget::IssueTriage { .. } => unreachable!(),
                    AiWorkflowTarget::ScriptGenerate { script_id } => {
                        let Some(script_id) = Uuid::parse_str(script_id).ok().filter(|script_id| {
                            self.scripts.read(cx).script_body(*script_id).is_some()
                        }) else {
                            self.show_toast(
                                t!("toast.ai.action_target_changed").to_string(),
                                ToastLevel::Error,
                                cx,
                            );
                            return;
                        };
                        self.active_view = ActiveView::Scripts;
                        self.ai_sheet = None;
                        self.scripts.update(cx, |view, cx| {
                            view.apply_generated_body(script_id, result, cx);
                        });
                    }
                    AiWorkflowTarget::ScriptFix { script_id } => {
                        let Some(script_id) = Uuid::parse_str(script_id).ok().filter(|script_id| {
                            self.scripts.read(cx).script_body(*script_id).is_some()
                        }) else {
                            self.show_toast(
                                t!("toast.ai.action_target_changed").to_string(),
                                ToastLevel::Error,
                                cx,
                            );
                            return;
                        };
                        let body = shelldeck_core::ai::clean_generated_script_body(&result);
                        self.active_view = ActiveView::Scripts;
                        self.ai_sheet = None;
                        self.scripts.update(cx, |view, cx| {
                            view.apply_generated_body(script_id, body, cx);
                        });
                    }
                    AiWorkflowTarget::TerminalCommand { session_id } => {
                        let applied = Uuid::parse_str(session_id).ok().is_some_and(|session_id| {
                            self.terminal
                                .read(cx)
                                .insert_ai_command(session_id, &result)
                                .is_ok()
                        });
                        if !applied {
                            self.show_toast(
                                t!("toast.ai.command_invalid").to_string(),
                                ToastLevel::Error,
                                cx,
                            );
                            return;
                        }
                    }
                    AiWorkflowTarget::TerminalDiagnose { .. } => {
                        let display = match parse_diagnostic_plan(&result) {
                            Ok(plan) => plan.display_text(),
                            Err(error) => {
                                self.show_toast(error.to_string(), ToastLevel::Error, cx);
                                return;
                            }
                        };
                        cx.write_to_clipboard(ClipboardItem::new_string(display));
                    }
                }
                self.set_ai_workflow_task_status(&target, AiTaskStatus::Applied, cx);
                self.close_ai_workflow(cx);
                self.show_toast(
                    if copied {
                        t!("toast.ai.analysis_copied").to_string()
                    } else {
                        t!("toast.ai.draft_applied").to_string()
                    },
                    ToastLevel::Success,
                    cx,
                );
            }
            AiWorkflowEvent::PrepareAction { target, result } => {
                self.prepare_ai_action(target, result, cx);
            }
            AiWorkflowEvent::PrepareDiagnosticStep { target, command } => {
                self.prepare_ai_diagnostic_step(target, command, cx);
            }
            AiWorkflowEvent::PrepareDiagnosticPlan { target, plan } => {
                self.start_ai_diagnostic_sequence(target, plan, cx);
            }
            AiWorkflowEvent::Pending {
                target,
                instructions,
                result,
            } => {
                if let Some(task) = self.ai_tasks.iter_mut().rev().find(|task| {
                    task.capability == target.capability() && task.target_id == target.target_id()
                }) {
                    task.instructions = instructions;
                    task.result = result;
                    task.target_kind = Some(target.storage_kind().to_string());
                    task.set_status(AiTaskStatus::Pending, None);
                } else {
                    let mut task = AiTask::new(
                        target.capability(),
                        target.surface(),
                        target.target_id(),
                        self.app_config.ai.backend,
                        instructions,
                        result,
                    );
                    task.target_kind = Some(target.storage_kind().to_string());
                    self.ai_tasks.push(task);
                }
                self.sync_ai_tasks(cx);
                self.show_toast(
                    t!("toast.ai.draft_pending").to_string(),
                    ToastLevel::Success,
                    cx,
                );
                self.close_ai_workflow(cx);
            }
            AiWorkflowEvent::Cancel => self.close_ai_workflow(cx),
        }
    }

    fn open_ai_assistant_with_context(&mut self, context: AiContext, cx: &mut Context<Self>) {
        if !self.ai_backend_available() || !self.app_config.ai.allows(context.surface) {
            return;
        }
        for handle in cx.windows() {
            if let Some(dock) = handle.downcast::<crate::ai_dock::AiDockView>() {
                let _ = dock.update(cx, |_dock, window, _cx| window.hide_window());
            }
        }
        self.ai_assistant.update(cx, |assistant, cx| {
            assistant.reload_conversations(cx);
            assistant.set_backend(
                self.app_config.ai.backend,
                self.app_config.ai.model.clone(),
                cx,
            );
            assistant.set_context(context, cx);
        });
        let sheet_assistant = self.ai_assistant.clone();
        let workspace = cx.entity().downgrade();
        self.ai_sheet = Some(cx.new(move |sheet_cx| {
            Sheet::new(sheet_cx)
                .width(gpui::px(780.0))
                .variant(SheetVariant::Assistant)
                .title(t!("ai.assistant.title").to_string())
                .dynamic_content(move || sheet_assistant.clone())
                .on_close(move |_window, cx| {
                    if let Some(workspace) = workspace.upgrade() {
                        workspace.update(cx, |this, cx| {
                            this.ai_sheet = None;
                            cx.notify();
                        });
                    }
                })
        }));
        cx.notify();
    }

    fn handle_ai_assistant_event(
        &mut self,
        source: Entity<AiAssistantView>,
        event: AiAssistantEvent,
        cx: &mut Context<Self>,
    ) {
        match event {
            AiAssistantEvent::Submit {
                request_id,
                conversation_id,
                prompt,
                context,
            } => {
                let config = self.app_config.ai.clone();
                let source = source.clone();
                cx.spawn(async move |this, cx: &mut AsyncApp| {
                    let result = cx
                        .background_executor()
                        .spawn(async move {
                            let client = create_client(&config)?;
                            client
                                .complete(&prompt, context)
                                .map(|response| response.text)
                        })
                        .await
                        .map_err(|error| error.to_string());
                    let _ = this.update(cx, |_workspace, cx| {
                        source.update(cx, |assistant, cx| {
                            assistant.set_result(request_id, conversation_id, result, cx);
                        });
                    });
                })
                .detach();
            }
            AiAssistantEvent::ResumeTask(task_id) => {
                let target = self
                    .ai_tasks
                    .iter()
                    .find(|task| task.id == task_id)
                    .and_then(AiWorkflowTarget::from_task);
                if let Some(target) = target {
                    self.ai_sheet = None;
                    self.open_ai_workflow(target, cx);
                } else {
                    self.show_toast(
                        t!("toast.ai.task_unavailable").to_string(),
                        ToastLevel::Warning,
                        cx,
                    );
                }
            }
            AiAssistantEvent::OpenTaskTarget(task_id) => {
                self.open_ai_task_target(task_id, cx);
            }
            AiAssistantEvent::StopTask(task_id) => {
                self.stop_ai_task(task_id, cx);
            }
            AiAssistantEvent::DeleteTask(task_id) => {
                let active = self
                    .ai_tasks
                    .iter()
                    .find(|task| task.id == task_id)
                    .is_some_and(|task| task.status.is_active());
                if !active {
                    self.ai_tasks.retain(|task| task.id != task_id);
                    self.sync_ai_tasks(cx);
                }
            }
        }
    }

    fn open_ai_task_target(&mut self, task_id: Uuid, cx: &mut Context<Self>) {
        let Some(task) = self
            .ai_tasks
            .iter()
            .find(|task| task.id == task_id)
            .cloned()
        else {
            return;
        };
        self.ai_sheet = None;
        match task.surface {
            AiSurface::Support => {
                if self.can_switch_mode() {
                    self.set_mode(AppMode::Support, cx);
                }
                self.support.update(cx, |view, cx| {
                    view.set_section(crate::support_view::SupportSection::Tickets);
                    cx.notify();
                });
                self.select_support_ticket(task.target_id, cx);
            }
            AiSurface::Issue => {
                self.open_support_requests(cx);
                self.select_issue(task.target_id, cx);
            }
            AiSurface::Script => {
                if let Ok(script_id) = Uuid::parse_str(&task.target_id) {
                    self.active_view = ActiveView::Scripts;
                    self.scripts.update(cx, |view, cx| {
                        view.selected_script = Some(script_id);
                        cx.notify();
                    });
                }
            }
            AiSurface::Terminal => {
                if let Ok(session_id) = Uuid::parse_str(&task.target_id) {
                    self.active_view = ActiveView::Terminal;
                    self.terminal.update(cx, |view, cx| {
                        view.select_tab(session_id);
                        cx.notify();
                    });
                }
            }
            AiSurface::Jean => {
                self.active_view = ActiveView::JeanConsole;
            }
            AiSurface::Naming => {
                if task.target_kind.as_deref() == Some("naming_terminal") {
                    if let Ok(session_id) = Uuid::parse_str(&task.target_id) {
                        self.active_view = ActiveView::Terminal;
                        self.terminal.update(cx, |view, cx| {
                            view.select_tab(session_id);
                            cx.notify();
                        });
                    }
                }
            }
            AiSurface::Recent | AiSurface::Global => {}
        }
        cx.notify();
    }

    fn stop_ai_task(&mut self, task_id: Uuid, cx: &mut Context<Self>) {
        if let Some((script_id, _)) = self
            .ai_script_runs
            .iter()
            .find(|(_, plan)| plan.id == task_id)
            .map(|(script_id, plan)| (*script_id, plan.clone()))
        {
            if self.scripts.read(cx).running_script_id == Some(script_id) {
                self.handle_script_event(&ScriptEvent::StopScript, cx);
                return;
            }
        }
        if let Some((session_id, plan)) = self
            .ai_terminal_runs
            .iter()
            .find(|(_, plan)| plan.id == task_id)
            .map(|(session_id, plan)| (*session_id, plan.clone()))
        {
            let stopped = self
                .terminal
                .update(cx, |terminal, cx| terminal.stop_ai_command(session_id, cx))
                .is_ok();
            if stopped {
                self.ai_terminal_runs.remove(&session_id);
                self.audit_ai_action(&plan, "cancelled", cx);
                self.show_toast(
                    t!("toast.ai.command_stopped").to_string(),
                    ToastLevel::Info,
                    cx,
                );
                return;
            }
        }
        self.show_toast(
            t!("toast.ai.task_stop_unavailable").to_string(),
            ToastLevel::Warning,
            cx,
        );
    }

    /// Apply an autostart toggle change: try the OS-level write on a
    /// background thread, then commit the settings field (and save) if
    /// it worked, or toast the error and leave the toggle where the
    /// user found it if it didn't. See `.agents/session-state.md` for
    /// why we route this via a dedicated event instead of the plain
    /// `ConfigChanged` path — we can't roll back a disk write cleanly,
    /// so we simply don't write until the OS confirms.
    fn apply_autostart_request(&mut self, desired: bool, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move { shelldeck_core::config::autostart::apply(desired) })
                .await;

            let _ = this.update(cx, |ws, cx| match result {
                Ok(actual) => {
                    // Commit through Settings — it owns `general.autostart`
                    // (see `.agents/session-state.md`) and its `save_config`
                    // emits `ConfigChanged` so the workspace merges the
                    // updated slice into `app_config` on the next tick.
                    ws.settings.update(cx, |settings, cx| {
                        settings.set_autostart(actual, cx);
                    });
                    ws.show_toast(
                        if actual {
                            t!("toast.autostart.enabled").to_string()
                        } else {
                            t!("toast.autostart.disabled").to_string()
                        },
                        ToastLevel::Info,
                        cx,
                    );
                }
                Err(e) => {
                    tracing::warn!("autostart apply failed: {e}");
                    ws.show_toast(
                        t!("toast.autostart.failed", error = e.to_string()).to_string(),
                        ToastLevel::Error,
                        cx,
                    );
                }
            });
        })
        .detach();
    }

    /// Swap the live `ShellDeckColors` palette and the adabraka-ui component
    /// theme to `pref`, then notify every view so the whole UI repaints. Does
    /// NOT touch `app_config` or persist — callers decide whether to commit.
    fn apply_palette(&self, pref: &ThemePreference, cx: &mut Context<Self>) {
        ShellDeckColors::set_theme(pref);
        install_theme(cx.deref_mut(), crate::theme::adabraka_theme_from_palette());
        self.notify_theme_views(cx);
    }

    /// Notify every child view (and self) to re-render with the active palette.
    fn notify_theme_views(&self, cx: &mut Context<Self>) {
        self.sidebar.update(cx, |_, cx| cx.notify());
        self.dashboard.update(cx, |_, cx| cx.notify());
        self.scripts.update(cx, |_, cx| cx.notify());
        self.port_forwards.update(cx, |_, cx| cx.notify());
        self.server_sync.update(cx, |_, cx| cx.notify());
        self.recent.update(cx, |_, cx| cx.notify());
        self.settings.update(cx, |_, cx| cx.notify());
        self.status_bar.update(cx, |_, cx| cx.notify());
        self.command_palette.update(cx, |_, cx| cx.notify());
        self.toasts.update(cx, |_, cx| cx.notify());
        cx.notify();
    }

    /// Live-preview the action highlighted in the command palette. App-theme
    /// actions apply their palette without persisting; the original theme is
    /// remembered so it can be restored on dismiss. Any other action ends an
    /// active preview (restoring the original theme).
    fn preview_palette_action(&mut self, action: &dyn Action, cx: &mut Context<Self>) {
        if let Some(t) = action.as_any().downcast_ref::<ApplyAppTheme>() {
            // Switching to an app-theme entry ends any terminal-theme preview.
            self.revert_terminal_theme_preview(cx);
            if self.theme_before_preview.is_none() {
                self.theme_before_preview = Some(self.app_config.theme.clone());
            }
            let pref = t.pref.clone();
            self.apply_palette(&pref, cx);
        } else if let Some(t) = action.as_any().downcast_ref::<ApplyTerminalTheme>() {
            // Switching to a terminal-theme entry ends any app-theme preview.
            self.revert_theme_preview(cx);
            let name = t.name.clone();
            self.preview_terminal_theme(&name, cx);
        } else {
            // A non-theme entry: end any active preview of either kind.
            self.revert_theme_preview(cx);
            self.revert_terminal_theme_preview(cx);
        }
    }

    /// Restore the app theme captured before previewing, if a preview is active.
    fn revert_theme_preview(&mut self, cx: &mut Context<Self>) {
        if let Some(orig) = self.theme_before_preview.take() {
            self.apply_palette(&orig, cx);
        }
    }

    /// Apply a terminal color theme to the live terminal without persisting,
    /// remembering the original so it can be restored on dismiss.
    fn preview_terminal_theme(&mut self, name: &str, cx: &mut Context<Self>) {
        if self.terminal_theme_before_preview.is_none() {
            self.terminal_theme_before_preview = Some(self.app_config.terminal.theme.clone());
        }
        let theme = TerminalTheme::by_name(name);
        self.terminal.update(cx, |terminal, cx| {
            terminal.set_terminal_theme(&theme);
            cx.notify();
        });
    }

    /// Restore the terminal theme captured before previewing, if active.
    fn revert_terminal_theme_preview(&mut self, cx: &mut Context<Self>) {
        if let Some(name) = self.terminal_theme_before_preview.take() {
            let theme = TerminalTheme::by_name(&name);
            self.terminal.update(cx, |terminal, cx| {
                terminal.set_terminal_theme(&theme);
                cx.notify();
            });
        }
    }

    /// Commit a previewed app theme: persist it via the settings view (which
    /// re-emits `ThemeChanged`, applying the palette through the normal path).
    fn commit_theme_preview(&mut self, pref: ThemePreference, cx: &mut Context<Self>) {
        self.theme_before_preview = None;
        self.settings
            .update(cx, |settings, cx| settings.select_app_theme(pref, cx));
    }

    /// Apply a terminal color theme by name: persist it (which repaints the
    /// live terminal via `ConfigChanged`) and surface a confirmation toast.
    /// Used by the command palette's theme entries.
    pub fn apply_terminal_theme_by_name(&mut self, name: &str, cx: &mut Context<Self>) {
        self.settings.update(cx, |settings, cx| {
            settings.select_terminal_theme(name, cx);
        });
        self.show_toast(
            t!("toast.terminal_theme", name = name).to_string(),
            ToastLevel::Info,
            cx,
        );
    }

    fn handle_dashboard_event(&mut self, event: &DashboardEvent, cx: &mut Context<Self>) {
        match event {
            DashboardEvent::QuickConnect(id) => {
                if let Some(conn) = self.connections.iter().find(|c| c.id == *id) {
                    let conn = conn.clone();
                    let title = conn.display_name().to_string();
                    let conn_id = conn.id;
                    self.connect_ssh(conn, cx);
                    self.add_activity_entry(
                        ActivityEntry::new(
                            ActivityKind::Connection,
                            t!("activity.quick_connecting_to", name = title.as_str()).to_string(),
                        )
                        .with_target(conn_id.to_string(), title)
                        .with_action(ActivityAction::ConnectConnection),
                        cx,
                    );
                    self.active_view = ActiveView::Terminal;
                    cx.notify();
                } else {
                    self.show_toast(
                        t!("toast.deeplink.connection_not_found").to_string(),
                        ToastLevel::Warning,
                        cx,
                    );
                }
            }
        }
    }

    fn handle_recent_event(&mut self, event: RecentEvent, cx: &mut Context<Self>) {
        match event {
            RecentEvent::Open(entry) => self.open_activity(entry, cx),
            RecentEvent::Analyze(entry) => {
                let context = AiContext::new(
                    AiSurface::Recent,
                    t!("ai.context.recent_event").to_string(),
                    self.ai_context_data_with_hosts(serde_json::json!({
                        "activity": entry,
                    })),
                );
                self.open_ai_assistant_with_context(context, cx);
            }
        }
    }

    fn open_activity(&mut self, entry: ActivityEntry, cx: &mut Context<Self>) {
        match entry.action {
            ActivityAction::None => {}
            ActivityAction::OpenTerminal => {
                self.activate_dev_section(SidebarSection::Terminals, cx);
            }
            ActivityAction::OpenConnection | ActivityAction::ConnectConnection => {
                let Some(id) = entry
                    .target_id
                    .as_deref()
                    .and_then(|id| Uuid::parse_str(id).ok())
                else {
                    return;
                };
                if self.can_switch_mode() {
                    self.set_mode(AppMode::Dev, cx);
                }
                self.sidebar.update(cx, |s, cx| {
                    s.focus_connection(id);
                    cx.notify();
                });
                if entry.action == ActivityAction::ConnectConnection {
                    if let Some(conn) = self.connections.iter().find(|c| c.id == id).cloned() {
                        self.connect_ssh(conn, cx);
                        self.active_view = ActiveView::Terminal;
                    }
                } else {
                    self.active_view = ActiveView::Dashboard;
                }
                self.on_active_view_changed(cx);
                cx.notify();
            }
            ActivityAction::OpenForward => {
                self.activate_dev_section(SidebarSection::PortForwards, cx);
            }
            ActivityAction::OpenScript => {
                let script_id = entry
                    .target_id
                    .as_deref()
                    .and_then(|id| Uuid::parse_str(id).ok());
                self.activate_dev_section(SidebarSection::Scripts, cx);
                if let Some(id) = script_id {
                    self.scripts.update(cx, |editor, cx| {
                        editor.selected_script = Some(id);
                        cx.notify();
                    });
                }
                self.populate_script_editor_connections(cx);
                cx.notify();
            }
            ActivityAction::OpenSupport => {
                if self.can_switch_mode() {
                    self.set_mode(AppMode::Support, cx);
                }
                self.refresh_support(cx);
                cx.notify();
            }
            ActivityAction::OpenTicket => {
                if self.can_switch_mode() {
                    self.set_mode(AppMode::Support, cx);
                }
                if let Some(id) = entry.target_id {
                    self.select_support_ticket(id, cx);
                }
                cx.notify();
            }
            ActivityAction::OpenIssue => {
                if self.can_switch_mode() {
                    if self.issues_staff {
                        self.set_mode(AppMode::Support, cx);
                        self.support.update(cx, |v, cx| {
                            v.set_section(crate::support_view::SupportSection::Requests);
                            cx.notify();
                        });
                    } else {
                        self.set_mode(AppMode::User, cx);
                        self.user_home_tab = UserHomeTab::Requests;
                    }
                }
                if let Some(id) = entry.target_id {
                    self.select_issue(id, cx);
                }
                cx.notify();
            }
            ActivityAction::OpenSite => {
                if self.can_switch_mode() {
                    self.set_mode(AppMode::User, cx);
                }
                if let Some(id) = entry.target_id {
                    self.select_site(Some(id), entry.target_label, cx);
                }
                self.user_home_tab = UserHomeTab::Sites;
                cx.notify();
            }
            ActivityAction::OpenJean => self.open_jean_console(cx),
            ActivityAction::OpenFleet => self.open_fleet(cx),
            ActivityAction::OpenBext => self.open_bext_cloud(cx),
        }
    }

    pub fn show_connection_form(&mut self, conn: Option<Connection>, cx: &mut Context<Self>) {
        let form = cx.new(|form_cx| {
            if let Some(ref c) = conn {
                ConnectionForm::from_connection(c, form_cx)
            } else {
                ConnectionForm::new(form_cx)
            }
        });

        let sub = cx.subscribe(&form, |this, _form, event: &ConnectionFormEvent, cx| {
            match event {
                ConnectionFormEvent::Save(conn) => {
                    tracing::info!("Connection saved: {}", conn.display_name());
                    // Add to connections list
                    if let Some(idx) = this.connections.iter().position(|c| c.id == conn.id) {
                        this.connections[idx] = conn.clone();
                    } else {
                        this.connections.push(conn.clone());
                    }
                    // Persist to store
                    if let Err(e) = this.store.add_connection(conn.clone()) {
                        tracing::error!("Failed to save connection store: {}", e);
                        this.show_toast(
                            t!("toast.connection.save_failed", error = e.to_string()).to_string(),
                            ToastLevel::Error,
                            cx,
                        );
                    }
                    // Update sidebar
                    this.sidebar.update(cx, |sidebar, _| {
                        sidebar.set_connections(this.connections.clone());
                    });
                    let conn_name = conn.display_name().to_string();
                    this.add_activity_entry(
                        ActivityEntry::new(
                            ActivityKind::Connection,
                            t!("activity.connection_added", name = conn_name.as_str()).to_string(),
                        )
                        .with_target(conn.id.to_string(), conn_name)
                        .with_action(ActivityAction::OpenConnection),
                        cx,
                    );
                    this.show_toast(
                        t!(
                            "toast.connection.saved",
                            name = conn.display_name().to_string()
                        )
                        .to_string(),
                        ToastLevel::Success,
                        cx,
                    );
                    // Close form
                    this.connection_form = None;
                    this._form_sub = None;
                    cx.notify();
                }
                ConnectionFormEvent::Cancel => {
                    this.connection_form = None;
                    this._form_sub = None;
                    cx.notify();
                }
            }
        });

        self.connection_form = Some(form);
        self._form_sub = Some(sub);
        cx.notify();
    }

    fn add_activity(&mut self, message: String, kind: ActivityKind, cx: &mut Context<Self>) {
        self.add_activity_entry(ActivityEntry::new(kind, message), cx);
    }

    fn add_activity_entry(&mut self, entry: ActivityEntry, cx: &mut Context<Self>) {
        if let Err(e) = ActivityStore::append(&entry) {
            tracing::warn!("Failed to append activity entry: {}", e);
        }
        self.recent_activity.insert(0, entry);
        if self.recent_activity.len() > 500 {
            self.recent_activity.truncate(500);
        }
        self.push_recent_activity(cx);
    }

    fn push_recent_activity(&mut self, cx: &mut Context<Self>) {
        let dashboard_entries: Vec<ActivityEntry> =
            self.recent_activity.iter().take(8).cloned().collect();
        let recent_entries = self.recent_activity.clone();
        self.dashboard.update(cx, |dashboard, _| {
            dashboard.recent_activity = dashboard_entries;
        });
        self.recent.update(cx, |recent, cx| {
            recent.set_entries(recent_entries);
            cx.notify();
        });
    }

    /// Show a toast notification in the bottom-right corner of the workspace.
    pub fn show_toast(&self, msg: impl Into<String>, level: ToastLevel, cx: &mut Context<Self>) {
        // When notifications are disabled, suppress informational toasts
        // (Info/Success/Warning) but always surface errors so failures are seen.
        if !self.app_config.general.show_notifications && level != ToastLevel::Error {
            return;
        }
        let message = msg.into();
        self.toasts.update(cx, |toasts, cx| {
            toasts.push(message, level, cx);
        });
    }

    /// Pull SSH connection profiles from Inklura Manage on demand.
    ///
    /// If Cloud Sync isn't configured, this just explains how to set it up.
    /// Otherwise the blocking network fetch + merge runs on a background thread
    /// (never the UI thread), and on completion the merged connections are
    /// reloaded into the sidebar/dashboard and a toast reports the stats.
    pub fn cloud_sync_now(&mut self, cx: &mut Context<Self>) {
        let cfg = self.app_config.cloud_sync.clone();
        if !cfg.is_configured() {
            self.show_toast(
                t!("toast.cloud_sync.not_configured").to_string(),
                ToastLevel::Warning,
                cx,
            );
            return;
        }

        self.show_toast(
            t!("toast.cloud_sync.started").to_string(),
            ToastLevel::Info,
            cx,
        );
        let version = shelldeck_core::VERSION;

        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move { shelldeck_core::config::cloud_sync::sync_now(&cfg, version) })
                .await;

            let _ = this.update(cx, |ws, cx| match result {
                Ok(stats) => {
                    ws.reload_connections_after_sync(cx);
                    ws.show_toast(
                        t!(
                            "toast.cloud_sync.done",
                            added = stats.added,
                            updated = stats.updated,
                            removed = stats.removed
                        )
                        .to_string(),
                        ToastLevel::Success,
                        cx,
                    );
                }
                Err(e) => {
                    ws.show_toast(
                        t!("toast.cloud_sync.failed", error = e.to_string()).to_string(),
                        ToastLevel::Error,
                        cx,
                    );
                }
            });
        })
        .detach();
    }

    /// Rebuild the in-memory connection list after Cloud Sync wrote the store.
    ///
    /// Mirrors the startup merge in `main.rs`: reload the persisted store,
    /// re-parse `~/.ssh/config`, and combine them (dedup by alias). Live
    /// connection status from the current list is carried over by id so an
    /// active session doesn't flip back to "disconnected".
    fn reload_connections_after_sync(&mut self, cx: &mut Context<Self>) {
        let store = match ConnectionStore::load() {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("Failed to reload connection store after cloud sync: {}", e);
                return;
            }
        };
        let ssh_connections =
            shelldeck_core::config::ssh_config::parse_ssh_config().unwrap_or_default();

        let mut merged = ssh_connections;
        for conn in &store.connections {
            if !merged.iter().any(|c| c.alias == conn.alias) {
                merged.push(conn.clone());
            }
        }
        // Preserve live status from the current in-memory connections.
        for m in merged.iter_mut() {
            if let Some(cur) = self.connections.iter().find(|c| c.id == m.id) {
                m.status = cur.status.clone();
            }
        }

        self.store = store;
        self.connections = merged;

        let conns = self.connections.clone();
        self.sidebar.update(cx, |sidebar, cx| {
            sidebar.set_connections(conns.clone());
            cx.notify();
        });
        self.server_sync.update(cx, |view, cx| {
            view.set_connections(conns.clone(), cx);
        });
        self.sites.update(cx, |view, _| {
            view.set_connections(conns);
        });
        self.update_dashboard_stats(cx);
        cx.notify();
    }

    // --- Cloud account (Inklura Manage) ---

    /// The account/sync base URL, defaulting to the portal if unset.
    fn account_base_url(&self) -> String {
        let b = self.app_config.cloud_sync.base_url.trim().to_string();
        if b.is_empty() {
            "https://manage.inklura.fr".to_string()
        } else {
            b
        }
    }

    /// Background whoami at startup: refresh the status dot + account name, and
    /// warn once if the token was revoked remotely. No-op when logged out.
    pub fn check_account_on_startup(&mut self, cx: &mut Context<Self>) {
        if !self.app_config.cloud_sync.is_configured() {
            return;
        }
        let base = self.account_base_url();
        let token = self.app_config.cloud_sync.token.clone();
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move { cloud_account::whoami(&base, &token) })
                .await;
            let _ = this.update(cx, |ws, cx| {
                match result {
                    Ok(info) => {
                        ws.account_status = AccountStatus::Ok;
                        let refreshed = info.account_info();
                        if !refreshed.name.trim().is_empty() || !refreshed.email.trim().is_empty() {
                            ws.app_config.account = Some(refreshed);
                            let _ = ws.app_config.save();
                        }
                        // Stash the full whoami — the User "Mes informations"
                        // tab renders every field (device label, created_at,
                        // last_seen_at, …) that `AccountInfo` doesn't persist.
                        ws.last_whoami = Some(info);
                        // Token is valid → load the sites directory + activate
                        // the persisted mode (starts the support poll if needed).
                        ws.refresh_sites(cx);
                        ws.activate_current_mode(cx);
                        ws.maybe_show_onboarding(cx);
                    }
                    Err(e) if cloud_account::is_auth_rejected(&e) => {
                        ws.invalidate_cloud_session(cx);
                        ws.account_status = AccountStatus::Rejected;
                        ws.show_toast(
                            t!("toast.session.expired").to_string(),
                            ToastLevel::Warning,
                            cx,
                        );
                    }
                    Err(_) => {
                        ws.account_status = AccountStatus::Offline;
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Open the Manage sign-up page in the system browser. Wired to the
    /// welcome landing's secondary CTA — ShellDeck requires an account to
    /// launch (no guest mode), so the "Créer un compte" button funnels
    /// prospects to Manage rather than dropping them into an unusable
    /// classic Dev workspace.
    pub fn open_signup(&mut self, cx: &mut Context<Self>) {
        let url = "https://inklura.fr/signup";
        match cloud_account::open_in_browser(url) {
            Ok(_) => self.show_toast(
                t!("toast.opening_browser").to_string(),
                ToastLevel::Info,
                cx,
            ),
            Err(e) => self.show_toast(
                t!(
                    "toast.open_browser_failed",
                    error = cloud_account::user_message(&e)
                )
                .to_string(),
                ToastLevel::Error,
                cx,
            ),
        }
    }

    /// Whether the pre-login welcome landing should intercept the render.
    /// True whenever the user is not signed in — there is no guest path;
    /// every launch of a fresh install lands here, and every logout brings
    /// the user back. Sign-in (or account creation via `open_signup`) is
    /// the only way past.
    fn show_welcome(&self) -> bool {
        !self.signed_in()
    }

    /// Open the password + OIDC login modal.
    pub fn show_login_form(&mut self, cx: &mut Context<Self>) {
        let server = self.account_base_url();
        let device = cloud_account::device_name();
        let form = cx.new(|form_cx| LoginForm::new(server, device, form_cx));

        let sub = cx.subscribe(
            &form,
            |this, _form, event: &LoginFormEvent, cx| match event {
                LoginFormEvent::SubmitPassword { email, password } => {
                    this.start_password_login(email.clone(), password.clone(), cx);
                }
                LoginFormEvent::StartOidc(provider) => {
                    this.start_oidc_login(provider.clone(), cx);
                }
                LoginFormEvent::Cancel => {
                    this.login_form = None;
                    this._login_form_sub = None;
                    cx.notify();
                }
            },
        );

        self.account_menu_open = false;
        self.login_form = Some(form);
        self._login_form_sub = Some(sub);
        cx.notify();
    }

    /// Open the post-login onboarding tour. Callable from Settings replay
    /// or from `maybe_show_onboarding` on first sign-in.
    pub fn show_onboarding(&mut self, cx: &mut Context<Self>) {
        if !self.signed_in() {
            return;
        }
        let can_switch = self.can_switch_mode();
        let form = cx.new(|form_cx| OnboardingView::new(can_switch, form_cx));
        let sub = cx.subscribe(
            &form,
            |this, _form, event: &OnboardingEvent, cx| match event {
                OnboardingEvent::Finished | OnboardingEvent::Skipped => {
                    this.complete_onboarding(cx);
                }
            },
        );
        self.onboarding = Some(form);
        self._onboarding_sub = Some(sub);
        cx.notify();
    }

    /// Show onboarding once per account install when not yet completed.
    fn maybe_show_onboarding(&mut self, cx: &mut Context<Self>) {
        if !self.signed_in() || self.app_config.general.onboarding_completed {
            return;
        }
        if self.onboarding.is_some() {
            return;
        }
        self.show_onboarding(cx);
    }

    /// Close the tour and persist completion (skip counts as done).
    fn complete_onboarding(&mut self, cx: &mut Context<Self>) {
        self.onboarding = None;
        self._onboarding_sub = None;
        if !self.app_config.general.onboarding_completed {
            self.app_config.general.onboarding_completed = true;
            if let Err(e) = self.app_config.save() {
                tracing::error!("Failed to save onboarding_completed: {}", e);
            }
            self.sync_settings_config(cx);
        }
        cx.notify();
    }

    /// Run password login on a background thread, then apply on success.
    fn start_password_login(&mut self, email: String, password: String, cx: &mut Context<Self>) {
        let base = self.account_base_url();
        let device = cloud_account::device_name();
        if let Some(form) = &self.login_form {
            form.update(cx, |f, cx| {
                f.set_busy(true);
                cx.notify();
            });
        }
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(
                    async move { cloud_account::login_password(&base, &email, &password, &device) },
                )
                .await;
            let _ = this.update(cx, |ws, cx| match result {
                Ok((token, account)) => ws.apply_login(token, account, cx),
                Err(e) => {
                    let msg = cloud_account::user_message(&e);
                    if let Some(form) = &ws.login_form {
                        form.update(cx, |f, cx| {
                            f.set_busy(false);
                            f.set_error(msg.clone());
                            cx.notify();
                        });
                    }
                    ws.show_toast(msg, ToastLevel::Error, cx);
                }
            });
        })
        .detach();
    }

    /// Start the browser device-authorize flow: bind a loopback listener, open
    /// the system browser, and wait (background) for the token redirect.
    fn start_oidc_login(&mut self, provider: Option<String>, cx: &mut Context<Self>) {
        let base = self.account_base_url();
        let device = cloud_account::device_name();

        let listener = match std::net::TcpListener::bind("127.0.0.1:0") {
            Ok(l) => l,
            Err(e) => {
                self.show_toast(
                    t!("toast.local_port_open_failed", error = e.to_string()).to_string(),
                    ToastLevel::Error,
                    cx,
                );
                return;
            }
        };
        let port = match listener.local_addr() {
            Ok(a) => a.port(),
            Err(e) => {
                self.show_toast(
                    t!("toast.local_port_read_failed", error = e.to_string()).to_string(),
                    ToastLevel::Error,
                    cx,
                );
                return;
            }
        };
        // Random state: two v4 UUIDs → 64 hex chars, matches [A-Za-z0-9_-]{32,64}.
        let state = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
        let url =
            cloud_account::browser_connect_url(&base, port, &state, &device, provider.as_deref());

        if let Err(e) = cloud_account::open_in_browser(&url) {
            self.show_toast(
                t!(
                    "toast.open_browser_failed",
                    error = cloud_account::user_message(&e)
                )
                .to_string(),
                ToastLevel::Error,
                cx,
            );
            return;
        }

        // Dismiss the login surfaces and show progress.
        self.account_menu_open = false;
        self.login_form = None;
        self._login_form_sub = None;
        self.show_toast(
            t!("toast.browser_auth_waiting").to_string(),
            ToastLevel::Info,
            cx,
        );
        cx.notify();

        let state_for_task = state.clone();
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let outcome = cx
                .background_executor()
                .spawn(async move {
                    let token = cloud_account::browser_connect_listen(
                        listener,
                        &state_for_task,
                        std::time::Duration::from_secs(180),
                    )?;
                    let who = cloud_account::whoami(&base, &token)?;
                    Ok::<(String, AccountInfo), shelldeck_core::ShellDeckError>((
                        token,
                        who.account_info(),
                    ))
                })
                .await;
            let _ = this.update(cx, |ws, cx| match outcome {
                Ok((token, account)) => ws.apply_login(token, account, cx),
                Err(e) => ws.show_toast(
                    t!(
                        "toast.browser_login_failed",
                        error = cloud_account::user_message(&e)
                    )
                    .to_string(),
                    ToastLevel::Error,
                    cx,
                ),
            });
        })
        .detach();
    }

    /// Persist a successful login (enable cloud sync, store token + account),
    /// then sync profiles and report the count.
    fn apply_login(&mut self, token: String, account: AccountInfo, cx: &mut Context<Self>) {
        self.app_config.cloud_sync.enabled = true;
        self.app_config.cloud_sync.token = token;
        self.app_config.account = Some(account.clone());
        if let Err(e) = self.app_config.save() {
            tracing::error!("Failed to save config after login: {}", e);
        }
        self.sync_settings_config(cx);
        self.push_account_to_support(cx);
        self.account_status = AccountStatus::Ok;
        self.login_form = None;
        self._login_form_sub = None;
        self.account_menu_open = false;
        cx.notify();

        // Load the sites directory for the switcher (background, non-blocking).
        self.refresh_sites(cx);
        // Non-super-admins are forced to User mode; activate whatever mode applies.
        self.activate_current_mode(cx);
        // Kick a whoami to populate `last_whoami` (device label, created_at,
        // last_seen_at) — the login response only carries the AccountInfo
        // subset, but "Mes informations" needs the richer payload.
        self.check_account_on_startup(cx);
        self.maybe_show_onboarding(cx);

        let cfg = self.app_config.cloud_sync.clone();
        let name = account.display_name();
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    shelldeck_core::config::cloud_sync::sync_now(&cfg, shelldeck_core::VERSION)
                })
                .await;
            let _ = this.update(cx, |ws, cx| match result {
                Ok(_stats) => {
                    ws.reload_connections_after_sync(cx);
                    let n = ws
                        .connections
                        .iter()
                        .filter(|c| c.source == ConnectionSource::CloudSync)
                        .count();
                    ws.show_toast(
                        t!("toast.login_synced", name = name.as_str(), count = n).to_string(),
                        ToastLevel::Success,
                        cx,
                    );
                }
                Err(e) => {
                    ws.show_toast(
                        t!(
                            "toast.login_sync_failed",
                            name = name.as_str(),
                            error = cloud_account::user_message(&e)
                        )
                        .to_string(),
                        ToastLevel::Warning,
                        cx,
                    );
                }
            });
        })
        .detach();
    }

    /// Keep `SettingsView`'s config snapshot aligned with `app_config`.
    /// Settings persists to disk on many small edits (sidebar nav collapse,
    /// font size, …) and emits `ConfigChanged` — if its copy is stale it
    /// would resurrect a logged-out session. Call after login/logout/session
    /// invalidation and whenever the workspace mutates account/cloud_sync.
    fn sync_settings_config(&mut self, cx: &mut Context<Self>) {
        let snapshot = self.app_config.clone();
        self.settings.update(cx, |settings, cx| {
            // Only rebuild the Select entities whose backing slice actually
            // changed — otherwise a login/logout/mode switch would nuke
            // every open dropdown popover.
            let old = std::mem::replace(&mut settings.config, snapshot);
            settings.sync_selects_if_changed(&old, cx);
            cx.notify();
        });
    }

    /// Clear Inklura Manage credentials and stop cloud-backed polls/views.
    fn invalidate_cloud_session(&mut self, cx: &mut Context<Self>) {
        self.app_config.account = None;
        self.app_config.cloud_sync.token = String::new();
        self.app_config.cloud_sync.enabled = false;
        self.app_config.cloud_sync.active_site_id = None;
        self.app_config.cloud_sync.active_site_label = None;
        if let Err(e) = self.app_config.save() {
            tracing::error!("Failed to save config after session invalidation: {}", e);
        }
        self.sync_settings_config(cx);
        self.push_account_to_support(cx);
        self.account_status = AccountStatus::Unknown;
        self.account_menu_open = false;
        self.last_whoami = None;
        self.user_home_tab = UserHomeTab::Sites;
        self.site_directory = None;
        self.site_menu_open = false;
        self._support_poll_task = None;
        self._issues_poll = None;
        self.sidebar.update(cx, |s, cx| {
            s.set_site_filter(None);
            cx.notify();
        });
        self.activate_current_mode(cx);
    }

    /// Sign out: revoke the token server-side (best-effort), then clear local
    /// account state and disable cloud sync.
    fn logout_account(&mut self, cx: &mut Context<Self>) {
        let base = self.account_base_url();
        let token = self.app_config.cloud_sync.token.clone();
        if !token.is_empty() {
            cx.background_executor()
                .spawn(async move {
                    let _ = cloud_account::logout(&base, &token);
                })
                .detach();
        }

        self.invalidate_cloud_session(cx);
        self.show_toast(t!("toast.logged_out").to_string(), ToastLevel::Info, cx);
        cx.notify();
    }

    // --- Manage sites (site switcher) ---

    /// Fetch the sites directory + areas in the background and cache them.
    /// No-op when logged out; never blocks.
    pub fn refresh_sites(&mut self, cx: &mut Context<Self>) {
        if !self.app_config.cloud_sync.is_configured() {
            return;
        }
        let base = self.account_base_url();
        let token = self.app_config.cloud_sync.token.clone();
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move { manage_sites::fetch_sites(&base, &token) })
                .await;
            let _ = this.update(cx, |ws, cx| match result {
                Ok(payload) => {
                    tracing::info!(
                        "Loaded {} manage sites, {} areas",
                        payload.sites.len(),
                        payload.areas.len()
                    );
                    ws.site_directory = Some(payload);
                    ws.refresh_command_palette(cx);
                    // Server may have just delivered the Jean config (super-admin).
                    ws.update_jean_availability(cx);
                    ws.sync_jean_poll(cx);
                    cx.notify();
                }
                Err(e) => tracing::warn!("Failed to load manage sites: {}", e),
            });
        })
        .detach();
    }

    /// The `ManagedSiteInfo` for the persisted active site, if it's in the cache.
    fn active_site_info(&self) -> Option<ManagedSiteInfo> {
        let id = self.app_config.cloud_sync.active_site_id.as_deref()?;
        self.site_directory
            .as_ref()?
            .sites
            .iter()
            .find(|s| s.site_id == id)
            .cloned()
    }

    /// Select the active site (or `None` for "all sites"): persist it, scope the
    /// sidebar, and close the dropdown.
    fn select_site(
        &mut self,
        site_id: Option<String>,
        label: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let activity_site_id = site_id.clone();
        let activity_label = label.clone();
        self.app_config.cloud_sync.active_site_id = site_id.clone();
        self.app_config.cloud_sync.active_site_label = label;
        if let Err(e) = self.app_config.save() {
            tracing::error!("Failed to save active site: {}", e);
        }
        let filter = site_id.as_deref().and_then(|s| Uuid::parse_str(s).ok());
        self.sidebar.update(cx, |s, cx| {
            s.set_site_filter(filter);
            cx.notify();
        });
        self.refresh_command_palette(cx);
        self.site_menu_open = false;
        if let Some(site_id) = activity_site_id {
            let label = activity_label.unwrap_or_else(|| site_id.clone());
            self.add_activity_entry(
                ActivityEntry::new(
                    ActivityKind::Site,
                    t!("activity.site.selected", label = label.as_str()).to_string(),
                )
                .with_target(site_id, label)
                .with_action(ActivityAction::OpenSite),
                cx,
            );
        }
        cx.notify();
    }

    /// Open a manage area for the active site in the system browser.
    pub fn open_manage_area(&mut self, area_path: String, cx: &mut Context<Self>) {
        let site = match self.active_site_info() {
            Some(s) => s,
            None => {
                self.show_toast(
                    t!("toast.select_active_site_first").to_string(),
                    ToastLevel::Warning,
                    cx,
                );
                return;
            }
        };
        let origin = self
            .site_directory
            .as_ref()
            .map(|p| p.manage_origin.clone())
            .filter(|o| !o.is_empty())
            .unwrap_or_else(|| self.account_base_url());
        let url = manage_sites::manage_area_url(&origin, &site, &area_path);
        self.site_menu_open = false;
        match cloud_account::open_in_browser(&url) {
            Ok(_) => self.show_toast(
                t!("toast.opening_browser").to_string(),
                ToastLevel::Info,
                cx,
            ),
            Err(e) => self.show_toast(
                t!(
                    "toast.open_browser_failed",
                    error = cloud_account::user_message(&e)
                )
                .to_string(),
                ToastLevel::Error,
                cx,
            ),
        }
        cx.notify();
    }

    /// Open the titlebar site switcher (from the command palette / an action).
    pub fn open_site_switcher(&mut self, cx: &mut Context<Self>) {
        if self.site_directory.is_none() {
            self.show_toast(
                t!("toast.login_required_site_switch").to_string(),
                ToastLevel::Warning,
                cx,
            );
            return;
        }
        self.site_menu_open = true;
        self.theme_menu_open = false;
        self.account_menu_open = false;
        cx.notify();
    }

    /// The fixed command-palette entries (everything except the runtime-dependent
    /// manage-area entries, which [`refresh_command_palette`] appends).
    ///
    /// `can_switch_mode` gates the "Mode : …" rows per SDUC-152 / SDTEST-1057
    /// — non-super-admins never see them in the palette, so no dispatched
    /// `SetAppMode` action can silently no-op on their end.
    fn base_palette_actions(can_switch_mode: bool, ai_configured: bool) -> Vec<PaletteAction> {
        let mut actions = vec![
            PaletteAction::new(
                t!("palette.new_terminal").to_string(),
                Some("Ctrl+T"),
                "terminal",
                Box::new(NewTerminal),
            ),
            PaletteAction::new(
                t!("palette.toggle_sidebar").to_string(),
                Some("Ctrl+B"),
                "chevron-left",
                Box::new(ToggleSidebar),
            ),
            PaletteAction::new(
                t!("palette.open_settings").to_string(),
                Some("Ctrl+,"),
                "settings",
                Box::new(OpenSettings),
            ),
            PaletteAction::new(
                t!("palette.close_tab").to_string(),
                Some("Ctrl+W"),
                "x",
                Box::new(CloseTab),
            ),
            PaletteAction::new(
                t!("palette.next_tab").to_string(),
                Some("Ctrl+Tab"),
                "chevron-right",
                Box::new(NextTab),
            ),
            PaletteAction::new(
                t!("palette.prev_tab").to_string(),
                Some("Ctrl+Shift+Tab"),
                "chevron-left",
                Box::new(PrevTab),
            ),
            PaletteAction::new(
                t!("palette.quit").to_string(),
                Some("Ctrl+Q"),
                "x",
                Box::new(Quit),
            ),
            PaletteAction::new(
                t!("palette.browse_templates").to_string(),
                None,
                "scroll-text",
                Box::new(OpenTemplateBrowser),
            ),
            PaletteAction::new(
                t!("palette.new_script").to_string(),
                None,
                "plus",
                Box::new(NewScript),
            ),
            PaletteAction::new(
                t!("palette.open_server_sync").to_string(),
                None,
                "refresh-cw",
                Box::new(OpenServerSync),
            ),
            PaletteAction::new(
                t!("palette.open_sites").to_string(),
                None,
                "globe",
                Box::new(OpenSites),
            ),
            PaletteAction::new(
                t!("palette.open_recent").to_string(),
                None,
                "activity",
                Box::new(OpenRecent),
            ),
            PaletteAction::new(
                t!("palette.open_file_editor").to_string(),
                Some("Ctrl+E"),
                "pencil",
                Box::new(OpenFileEditorView),
            ),
            PaletteAction::new(
                t!("palette.cloud_sync_now").to_string(),
                None,
                "refresh-cw",
                Box::new(CloudSyncNow),
            ),
            PaletteAction::new(
                t!("palette.switch_site").to_string(),
                None,
                "globe",
                Box::new(SwitchSite),
            ),
            PaletteAction::new(
                t!("palette.jean_open").to_string(),
                None,
                "cpu",
                Box::new(OpenJeanConsole),
            ),
            PaletteAction::new(
                t!("palette.jean_pause").to_string(),
                None,
                "clock",
                Box::new(JeanTogglePause),
            ),
            PaletteAction::new(
                t!("palette.fleet_open").to_string(),
                None,
                "box",
                Box::new(OpenFleet),
            ),
            PaletteAction::new(
                t!("palette.fleet_runtime").to_string(),
                None,
                "cpu",
                Box::new(ToggleJeanRuntime),
            ),
            PaletteAction::new(
                t!("palette.new_request").to_string(),
                None,
                "plus",
                Box::new(NewRequest),
            ),
            PaletteAction::new(
                t!("palette.support_requests").to_string(),
                None,
                "inbox",
                Box::new(OpenSupportRequests),
            ),
            PaletteAction::new(
                t!("palette.bext_open").to_string(),
                None,
                "cloud",
                Box::new(OpenBextCloud),
            ),
            PaletteAction::new(
                t!("palette.bext_connect").to_string(),
                None,
                "key",
                Box::new(ConnectBextCloud),
            ),
            PaletteAction::new(
                t!("palette.bext_new_site").to_string(),
                None,
                "cloud",
                Box::new(OpenBextCloud),
            ),
        ];

        if ai_configured {
            actions.push(PaletteAction::new(
                t!("palette.open_ai").to_string(),
                Some("Ctrl+Shift+K"),
                "zap",
                Box::new(OpenAiAssistant),
            ));
        }
        // Mode switcher entries — super-admins only, per SDUC-152 (leaks
        // to a non-super-admin would show an action that then no-ops on
        // dispatch; SDTEST-1057 gates it at construction so nothing leaks
        // to the UI in the first place).
        if can_switch_mode {
            for m in AppMode::all() {
                actions.push(PaletteAction::new(
                    t!("palette.mode", mode = m.label()).to_string(),
                    None,
                    "shield",
                    Box::new(SetAppMode { mode: m }),
                ));
            }
        }
        for pref in ThemePreference::all() {
            actions.push(PaletteAction::new(
                t!("palette.theme", name = pref.display_name()).to_string(),
                None,
                "settings",
                Box::new(ApplyAppTheme { pref: pref.clone() }),
            ));
        }
        for theme in TerminalTheme::builtins() {
            actions.push(PaletteAction::new(
                t!("palette.terminal_theme", name = theme.name).to_string(),
                None,
                "terminal",
                Box::new(ApplyTerminalTheme { name: theme.name }),
            ));
        }
        actions
    }

    /// Rebuild the palette entries, appending "Site actif : <area>" commands for
    /// the active site's manage areas. Called when the site directory loads or
    /// the active site changes.
    fn refresh_command_palette(&mut self, cx: &mut Context<Self>) {
        let mut actions = Self::base_palette_actions(
            self.can_switch_mode(),
            self.ai_available_for_current_surface(cx),
        );
        if let (Some(site), Some(dir)) = (self.active_site_info(), self.site_directory.as_ref()) {
            let label = site.display_label();
            for area in &dir.areas {
                actions.push(PaletteAction::new(
                    t!(
                        "palette.active_site",
                        site = label,
                        area = area.label.as_str()
                    )
                    .to_string(),
                    None,
                    "external-link",
                    Box::new(OpenManageArea {
                        path: area.path.clone(),
                    }),
                ));
            }
        }
        self.command_palette.update(cx, |palette, _| {
            palette.set_actions(actions.clone());
        });
        self.companion_command_palette.update(cx, |palette, _| {
            palette.set_actions(actions);
        });
    }

    fn handle_command_palette_event(
        &mut self,
        event: &CommandPaletteEvent,
        cx: &mut Context<Self>,
    ) {
        match event {
            CommandPaletteEvent::SelectionPreviewed(action) => {
                self.preview_palette_action(action.as_ref(), cx);
            }
            CommandPaletteEvent::ActionSelected(action) => {
                if let Some(theme) = action.as_any().downcast_ref::<ApplyAppTheme>() {
                    self.revert_terminal_theme_preview(cx);
                    self.commit_theme_preview(theme.pref.clone(), cx);
                } else if let Some(theme) = action.as_any().downcast_ref::<ApplyTerminalTheme>() {
                    self.revert_theme_preview(cx);
                    self.terminal_theme_before_preview = None;
                    self.apply_terminal_theme_by_name(&theme.name, cx);
                } else {
                    self.revert_theme_preview(cx);
                    self.revert_terminal_theme_preview(cx);
                    self.execute_palette_action(action.as_ref(), cx);
                }
            }
            CommandPaletteEvent::Dismissed => {
                self.revert_theme_preview(cx);
                self.revert_terminal_theme_preview(cx);
                cx.notify();
            }
        }
    }

    fn execute_palette_action(&mut self, action: &dyn Action, cx: &mut Context<Self>) {
        if action.as_any().is::<NewTerminal>() {
            self.open_new_terminal(cx);
        } else if action.as_any().is::<ToggleSidebar>() {
            self.toggle_sidebar(cx);
        } else if action.as_any().is::<OpenSettings>() {
            self.set_active_view(ActiveView::Settings);
            cx.notify();
        } else if action.as_any().is::<CloseTab>() {
            self.close_active_tab(cx);
        } else if action.as_any().is::<NextTab>() {
            self.next_tab(cx);
        } else if action.as_any().is::<PrevTab>() {
            self.prev_tab(cx);
        } else if action.as_any().is::<Quit>() {
            self.shutdown(cx);
            cx.quit();
        } else if action.as_any().is::<OpenTemplateBrowser>() {
            self.set_active_view(ActiveView::Scripts);
            self.show_template_browser(cx);
        } else if action.as_any().is::<NewScript>() {
            self.set_active_view(ActiveView::Scripts);
            self.show_script_form(cx);
        } else if action.as_any().is::<OpenServerSync>() {
            self.set_active_view(ActiveView::ServerSync);
            cx.notify();
        } else if action.as_any().is::<OpenSites>() {
            self.set_active_view(ActiveView::Sites);
            cx.notify();
        } else if action.as_any().is::<OpenRecent>() {
            self.activate_dev_section(SidebarSection::Recent, cx);
        } else if action.as_any().is::<OpenFileEditorView>() {
            self.set_active_view(ActiveView::FileEditor);
            cx.notify();
        } else if action.as_any().is::<CloudSyncNow>() {
            self.cloud_sync_now(cx);
        } else if action.as_any().is::<SwitchSite>() {
            self.open_site_switcher(cx);
        } else if let Some(area) = action.as_any().downcast_ref::<OpenManageArea>() {
            self.open_manage_area(area.path.clone(), cx);
        } else if let Some(mode) = action.as_any().downcast_ref::<SetAppMode>() {
            self.set_mode(mode.mode, cx);
        } else if action.as_any().is::<OpenJeanConsole>() {
            self.open_jean_console(cx);
        } else if action.as_any().is::<JeanTogglePause>() {
            self.jean_toggle_pause(cx);
        } else if action.as_any().is::<OpenFleet>() {
            self.open_fleet(cx);
        } else if action.as_any().is::<ToggleJeanRuntime>() {
            self.toggle_jean_runtime(cx);
        } else if action.as_any().is::<NewRequest>() {
            self.open_new_request(cx);
        } else if action.as_any().is::<OpenSupportRequests>() {
            self.open_support_requests(cx);
        } else if action.as_any().is::<OpenBextCloud>() {
            self.open_bext_cloud(cx);
        } else if action.as_any().is::<ConnectBextCloud>() {
            self.connect_bext_cloud_action(cx);
        } else if action.as_any().is::<OpenAiAssistant>() {
            self.open_ai_assistant(cx);
        } else {
            cx.dispatch_action(action);
        }
    }

    // --- App modes (User / Support / Dev) ---

    /// Whether the user is signed in to Inklura Manage.
    fn signed_in(&self) -> bool {
        self.app_config.cloud_sync.is_configured() && self.app_config.account.is_some()
    }

    fn is_superadmin(&self) -> bool {
        self.app_config
            .account
            .as_ref()
            .map(|a| a.is_superadmin)
            .unwrap_or(false)
    }

    /// True when the account passes `isManageAdmin` server-side (inclusive
    /// of super-admin). **No longer used for mode gating** — kept only
    /// for consumers that need "is this account a CM admin?" regardless
    /// of ShellDeck-staff status.
    #[allow(dead_code)]
    fn is_admin(&self) -> bool {
        self.app_config
            .account
            .as_ref()
            .map(|a| a.is_admin || a.is_superadmin)
            .unwrap_or(false)
    }

    /// True when the account passes `isInkluraSupport` server-side
    /// (inclusive of super-admin). **The Support-mode gate.** `is_admin`
    /// is deliberately not used here — it would include client
    /// tenant_admins, who are customers.
    fn is_inklura_support(&self) -> bool {
        self.app_config
            .account
            .as_ref()
            .map(|a| a.is_inklura_support || a.is_superadmin)
            .unwrap_or(false)
    }

    /// Signed-in Inklura support OR super-admins may switch modes.
    /// Regular users and client admins see no switcher — forced User.
    pub fn can_switch_mode(&self) -> bool {
        AppMode::can_switch(
            self.signed_in(),
            self.is_inklura_support(),
            self.is_superadmin(),
        )
    }

    /// The surface to present. Logged-out → the welcome landing intercepts
    /// the render before this hits; the User fallback is defensive. Signed-
    /// in super-admin → persisted mode; inklura_support → persisted
    /// clamped to {User, Support}; anyone else (including client admins)
    /// → forced User.
    ///
    /// Delegates to `AppMode::resolve_effective`; that pure fn is under
    /// test in `SDTEST-1052`.
    pub fn effective_mode(&self) -> AppMode {
        AppMode::resolve_effective(
            self.signed_in(),
            self.is_inklura_support(),
            self.is_superadmin(),
            self.app_config.cloud_sync.mode,
        )
    }

    /// Switch the app mode (super-admins only). Dev surfaces are hidden, not
    /// destroyed — running terminal sessions keep going.
    pub fn set_mode(&mut self, mode: AppMode, cx: &mut Context<Self>) {
        if !self.can_switch_mode() || self.app_config.cloud_sync.mode == mode {
            return;
        }
        self.app_config.cloud_sync.mode = mode;
        if let Err(e) = self.app_config.save() {
            tracing::error!("Failed to save app mode: {}", e);
        }
        self.theme_menu_open = false;
        self.account_menu_open = false;
        self.site_menu_open = false;

        // Cross-mode selection carry-over: opening a request in Support then
        // switching to User made the User-mode detail sheet auto-open on top
        // of the (unrelated) User list, because both surfaces share
        // `issue_selected`/`issue_detail`.
        self.reset_issue_selection(cx);

        self.activate_current_mode(cx);
        cx.notify();
    }

    /// Wipe every "which issue row is open" bit — Workspace-side selection
    /// (`issue_selected`, `issue_detail`), the User-mode sheet flags, the
    /// delete confirm dialog, AND the child `SupportView` selection — so
    /// mode switches (and any future "return to a clean list" flow) always
    /// land on an empty state. Any new issue-selection field added to
    /// `Workspace` must be reset here too.
    fn reset_issue_selection(&mut self, cx: &mut Context<Self>) {
        self.issue_selected = None;
        self.issue_detail = None;
        self.user_new_request_sheet_open = false;
        self.user_new_request_sheet_dismissing = false;
        self.user_issue_detail_dismissing = false;
        self.confirm_issue_delete = None;
        self.support.update(cx, |v, cx| {
            v.clear_selection();
            cx.notify();
        });
    }

    /// Start/stop the support poll and load support data for the current mode.
    /// Call after login / startup / a mode change.
    pub fn activate_current_mode(&mut self, cx: &mut Context<Self>) {
        self.sync_support_poll(cx);
        if self.effective_mode() == AppMode::Support && self.app_config.cloud_sync.is_configured() {
            self.refresh_support(cx);
        }
        self.update_jean_availability(cx);
        self.sync_jean_poll(cx);
        self.update_fleet_availability(cx);
        self.sync_fleet_view_poll(cx);
        self.sync_runtime_loop(cx);
        self.sync_issues_poll(cx);
    }

    fn sync_support_poll(&mut self, cx: &mut Context<Self>) {
        let want =
            self.effective_mode() == AppMode::Support && self.app_config.cloud_sync.is_configured();
        if want {
            if self._support_poll_task.is_none() {
                let task = cx.spawn(async move |this, cx: &mut AsyncApp| loop {
                    cx.background_executor()
                        .timer(std::time::Duration::from_secs(30))
                        .await;
                    let keep_going = this
                        .update(cx, |ws, cx| {
                            if ws.effective_mode() == AppMode::Support {
                                ws.refresh_support(cx);
                                true
                            } else {
                                false
                            }
                        })
                        .unwrap_or(false);
                    if !keep_going {
                        break;
                    }
                });
                self._support_poll_task = Some(task);
            }
        } else {
            self._support_poll_task = None;
        }
    }

    fn refresh_support(&mut self, cx: &mut Context<Self>) {
        if !self.app_config.cloud_sync.is_configured() {
            return;
        }
        let base = self.account_base_url();
        let token = self.app_config.cloud_sync.token.clone();
        let need_agents = !self.support.read(cx).has_agents();
        self.support.update(cx, |v, cx| {
            v.set_loading(true);
            cx.notify();
        });
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let (list, agents) = cx
                .background_executor()
                .spawn(async move {
                    let list = manage_support::support_list(&base, &token);
                    let agents = if need_agents {
                        manage_support::support_agents(&base, &token).ok()
                    } else {
                        None
                    };
                    (list, agents)
                })
                .await;
            let _ = this.update(cx, |ws, cx| {
                ws.support.update(cx, |v, cx| {
                    match list {
                        Ok(r) => v.set_list(r.tickets, r.counts, r.me),
                        Err(e) => v.set_error(cloud_account::user_message(&e)),
                    }
                    if let Some(a) = agents {
                        v.set_agents(a);
                    }
                    cx.notify();
                });
                // Support poll changed unread_ticket_count → refresh
                // tray counters. Fires every 30 s while Support is
                // active; the tray dedups.
                ws.publish_tray_state(cx);
            });
        })
        .detach();
    }

    fn select_support_ticket(&mut self, id: String, cx: &mut Context<Self>) {
        let base = self.account_base_url();
        let token = self.app_config.cloud_sync.token.clone();
        self.add_activity_entry(
            ActivityEntry::new(
                ActivityKind::Support,
                t!("activity.support.open_ticket", id = id.as_str()).to_string(),
            )
            .with_target(id.clone(), id.clone())
            .with_action(ActivityAction::OpenTicket),
            cx,
        );
        self.support.update(cx, |v, cx| {
            v.set_loading(true);
            cx.notify();
        });
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let detail = cx
                .background_executor()
                .spawn(async move {
                    let detail = manage_support::support_ticket(&base, &token, &id);
                    // Best-effort mark-read; ignore result.
                    let _ = manage_support::support_read(&base, &token, &id);
                    detail
                })
                .await;
            let _ = this.update(cx, |ws, cx| {
                match detail {
                    Ok(t) => {
                        ws.support.update(cx, |v, cx| {
                            v.set_detail(t, cx);
                            cx.notify();
                        });
                        // Unread counts drift ≤30 s until the poll runs — an
                        // eager `refresh_support` here doubled the HTTP round
                        // trips on every selection.
                    }
                    Err(e) => {
                        let msg = cloud_account::user_message(&e);
                        ws.support.update(cx, |v, cx| {
                            v.set_error(msg);
                            cx.notify();
                        });
                    }
                }
            });
        })
        .detach();
    }

    fn handle_support_event(&mut self, event: SupportViewEvent, cx: &mut Context<Self>) {
        use manage_support as ms;
        match event {
            SupportViewEvent::Refresh => self.refresh_support(cx),
            SupportViewEvent::SelectTicket(id) => self.select_support_ticket(id, cx),
            SupportViewEvent::SuggestReply(ticket_id) => {
                self.open_ai_workflow(AiWorkflowTarget::SupportReply { ticket_id }, cx)
            }
            SupportViewEvent::SummarizeTicket(ticket_id) => {
                self.open_ai_workflow(AiWorkflowTarget::SupportSummary { ticket_id }, cx)
            }
            SupportViewEvent::TriageTicket(ticket_id) => {
                self.open_ai_workflow(AiWorkflowTarget::SupportTriage { ticket_id }, cx)
            }
            SupportViewEvent::SuggestIssueReply(issue_id) => {
                self.open_ai_workflow(AiWorkflowTarget::IssueReply { issue_id }, cx)
            }
            SupportViewEvent::SummarizeIssue(issue_id) => {
                self.open_ai_workflow(AiWorkflowTarget::IssueSummary { issue_id }, cx)
            }
            SupportViewEvent::TriageIssue(issue_id) => {
                self.open_ai_workflow(AiWorkflowTarget::IssueTriage { issue_id }, cx)
            }
            SupportViewEvent::Send {
                id,
                text,
                note,
                attachments,
            } => {
                self.support_action(cx, move |base, token| {
                    if attachments.is_empty() && note {
                        ms::support_note(&base, &token, &id, &text)
                    } else if attachments.is_empty() {
                        ms::support_reply(&base, &token, &id, &text)
                    } else {
                        let uploads = attachments
                            .iter()
                            .map(AttachmentDraft::upload)
                            .collect::<Vec<_>>();
                        let receipts =
                            ms::upload_support_attachments(&base, &token, &id, &uploads)?;
                        if note {
                            ms::support_note_with_attachments(&base, &token, &id, &text, &receipts)
                        } else {
                            ms::support_reply_with_attachments(&base, &token, &id, &text, &receipts)
                        }
                    }
                });
            }
            SupportViewEvent::SetStatus { id, status } => {
                self.support_action(cx, move |b, t| ms::support_status(&b, &t, &id, &status));
            }
            SupportViewEvent::SetPriority { id, priority } => {
                self.support_action(cx, move |b, t| ms::support_priority(&b, &t, &id, &priority));
            }
            SupportViewEvent::Assign { id, assignee } => {
                self.support_action(cx, move |b, t| ms::support_assign(&b, &t, &id, &assignee));
            }
            SupportViewEvent::Resolve { id, resolution } => {
                self.support_action(cx, move |b, t| {
                    ms::support_resolve(&b, &t, &id, &resolution)
                });
            }
            SupportViewEvent::JeanConfirm(thread) => {
                self.jean_action(cx, move |c| jeanclaude::confirm(&c, &thread));
            }
            SupportViewEvent::JeanReject(thread) => {
                self.jean_action(cx, move |c| jeanclaude::reject(&c, &thread));
            }
            SupportViewEvent::SendToJean(text) => self.prepare_jean_dispatch(text, cx),
            SupportViewEvent::ConvertToIssue { title, body } => {
                self.open_prefilled_request(title, body, "support", cx)
            }
            SupportViewEvent::IssuesRefresh => self.refresh_issues(cx),
            SupportViewEvent::SelectIssue(id) => self.select_issue(id, cx),
            SupportViewEvent::IssueComment {
                id,
                body,
                attachments,
            } => self.comment_issue_with_images(id, body, attachments, cx),
            SupportViewEvent::ImportAttachmentUrl { url, generation } => {
                cx.spawn(async move |this, cx: &mut AsyncApp| {
                    let result = cx
                        .background_executor()
                        .spawn(async move { issues::download_issue_image_url(&url) })
                        .await
                        .map_err(|e| cloud_account::user_message(&e))
                        .and_then(|upload| {
                            AttachmentDraft::from_bytes(upload.filename, upload.bytes)
                        });
                    let _ = this.update(cx, |ws, cx| {
                        ws.support.update(cx, |view, cx| {
                            view.finish_attachment_url_import(generation, result, cx)
                        });
                    });
                })
                .detach();
            }
            SupportViewEvent::IssueStatus { id, status } => {
                self.issue_staff_action(cx, move |b, t| issues::set_status(&b, &t, &id, &status))
            }
            SupportViewEvent::IssueAssign { id, assignee } => {
                self.issue_staff_action(cx, move |b, t| issues::assign(&b, &t, &id, &assignee))
            }
            SupportViewEvent::IssuePriority { id, priority } => self
                .issue_staff_action(cx, move |b, t| issues::set_priority(&b, &t, &id, &priority)),
            SupportViewEvent::IssueDispatch { id, instance_id } => {
                self.prepare_fleet_dispatch(id, instance_id, cx)
            }
            SupportViewEvent::IssueGithubPush(id) => {
                self.issue_staff_action(cx, move |b, t| issues::github_push(&b, &t, &id))
            }
            SupportViewEvent::IssueGithubRefresh(id) => {
                self.issue_staff_action(cx, move |b, t| issues::github_refresh(&b, &t, &id))
            }
            SupportViewEvent::IssueDelete(id) => self.delete_issue_now(id, cx),
            SupportViewEvent::IssuesFilterChanged { filter } => {
                self.issues_filter = filter;
                self.refresh_issues(cx);
            }
        }
    }

    /// Run a support write action on the background executor; on success install
    /// the updated ticket + refresh the list, on failure toast the error.
    fn support_action<F>(&mut self, cx: &mut Context<Self>, f: F)
    where
        F: FnOnce(String, String) -> shelldeck_core::Result<manage_support::SupportTicket>
            + Send
            + 'static,
    {
        if !self.app_config.cloud_sync.is_configured() {
            return;
        }
        let base = self.account_base_url();
        let token = self.app_config.cloud_sync.token.clone();
        self.support.update(cx, |v, cx| {
            v.set_loading(true);
            cx.notify();
        });
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move { f(base, token) })
                .await;
            let _ = this.update(cx, |ws, cx| match result {
                Ok(t) => {
                    ws.support.update(cx, |v, cx| {
                        v.set_detail(t, cx);
                        cx.notify();
                    });
                    ws.refresh_support(cx);
                }
                Err(e) => {
                    let msg = cloud_account::user_message(&e);
                    ws.support.update(cx, |v, cx| {
                        v.set_error(msg.clone());
                        cx.notify();
                    });
                    ws.show_toast(msg, ToastLevel::Error, cx);
                }
            });
        })
        .detach();
    }

    // --- JeanClaude client ---

    /// The effective Jean config: a local `[jeanclaude]` override wins, else the
    /// server-delivered config (super-admin only). `None` = feature unavailable.
    ///
    /// Delegates to `JeanConfig::resolve_effective` (the pure fn under test in
    /// SDTEST-1054).
    fn effective_jean_config(&self) -> Option<JeanConfig> {
        let server = self
            .site_directory
            .as_ref()
            .and_then(|s| s.jeanclaude.as_ref());
        JeanConfig::resolve_effective(self.app_config.jeanclaude.as_ref(), server)
    }

    pub fn has_jean(&self) -> bool {
        self.effective_jean_config().is_some()
    }

    /// Whether a Jean surface is currently on screen (so polling is worthwhile).
    fn jean_surface_visible(&self) -> bool {
        if !self.has_jean() {
            return false;
        }
        match self.effective_mode() {
            AppMode::User | AppMode::Support => true,
            AppMode::Dev => self.active_view == ActiveView::JeanConsole,
        }
    }

    /// Reflect Jean availability into the sidebar nav (Dev mode only).
    fn update_jean_availability(&mut self, cx: &mut Context<Self>) {
        let show = self.has_jean() && self.effective_mode() == AppMode::Dev;
        self.sidebar.update(cx, |s, cx| {
            s.set_jean_available(show);
            cx.notify();
        });
    }

    fn sync_jean_poll(&mut self, cx: &mut Context<Self>) {
        if self.jean_surface_visible() {
            // Refresh immediately when a surface becomes visible.
            self.refresh_jean_state(cx);
            if self._jean_poll_task.is_none() {
                let task = cx.spawn(async move |this, cx: &mut AsyncApp| loop {
                    cx.background_executor()
                        .timer(std::time::Duration::from_secs(10))
                        .await;
                    let keep = this
                        .update(cx, |ws, cx| {
                            if ws.jean_surface_visible() {
                                ws.refresh_jean_state(cx);
                                true
                            } else {
                                false
                            }
                        })
                        .unwrap_or(false);
                    if !keep {
                        break;
                    }
                });
                self._jean_poll_task = Some(task);
            }
        } else {
            self._jean_poll_task = None;
        }
    }

    fn refresh_jean_state(&mut self, cx: &mut Context<Self>) {
        let Some(cfg) = self.effective_jean_config() else {
            return;
        };
        self.jean_view.update(cx, |v, cx| {
            v.set_loading(true);
            cx.notify();
        });
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move { jeanclaude::get_state(&cfg) })
                .await;
            let _ = this.update(cx, |ws, cx| match result {
                Ok(state) => {
                    ws.jean_state = Some(state.clone());
                    ws.jean_view.update(cx, |v, cx| {
                        v.set_state(state);
                        cx.notify();
                    });
                    ws.push_jean_brief_to_support(cx);
                }
                Err(e) => {
                    ws.jean_view.update(cx, |v, cx| {
                        v.set_error(cloud_account::user_message(&e));
                        cx.notify();
                    });
                }
            });
        })
        .detach();
    }

    /// Feed the Support-mode Jean strip from the cached state.
    fn push_jean_brief_to_support(&mut self, cx: &mut Context<Self>) {
        let available = self.has_jean();
        let (pending, active) = self
            .jean_state
            .as_ref()
            .map(|s| {
                let pending: Vec<(String, String)> = s
                    .pending
                    .iter()
                    .map(|p| (p.thread_ts.clone(), p.prompt.clone()))
                    .collect();
                let active = s
                    .tickets
                    .iter()
                    .filter(|t| t.is_running() || t.is_queued())
                    .count();
                (pending, active)
            })
            .unwrap_or_default();
        self.support.update(cx, |v, cx| {
            v.set_jean_brief(available, pending, active);
            cx.notify();
        });
    }

    fn handle_jean_event(&mut self, event: JeanViewEvent, cx: &mut Context<Self>) {
        use jeanclaude as j;
        match event {
            JeanViewEvent::Refresh => self.refresh_jean_state(cx),
            JeanViewEvent::SetPaused(p) => self.jean_action(cx, move |c| j::set_paused(&c, p)),
            JeanViewEvent::SetConcurrency(n) => {
                self.jean_action(cx, move |c| j::set_concurrency(&c, n))
            }
            JeanViewEvent::Say(text) => self.jean_say(text, cx),
            JeanViewEvent::Confirm(t) => self.jean_action(cx, move |c| j::confirm(&c, &t)),
            JeanViewEvent::Reject(t) => self.jean_action(cx, move |c| j::reject(&c, &t)),
            JeanViewEvent::Cancel(id) => self.jean_action(cx, move |c| j::cancel(&c, &id)),
            JeanViewEvent::Force(id) => self.jean_action(cx, move |c| j::force_ticket(&c, &id)),
            JeanViewEvent::SelectTicket(id) => self.jean_select_ticket(id, cx),
            JeanViewEvent::LoadHistory { q, status } => self.jean_load_history(q, status, cx),
            JeanViewEvent::LoadTargets => self.jean_load_targets(cx),
            JeanViewEvent::LoadMemory => self.jean_load_memory(cx),
            JeanViewEvent::AddTarget {
                domain,
                ssh_host,
                note,
            } => self.jean_action(cx, move |c| j::add_target(&c, &domain, &ssh_host, &note)),
            JeanViewEvent::RemoveTarget(d) => {
                self.jean_action(cx, move |c| j::remove_target(&c, &d))
            }
            JeanViewEvent::AddMemory { kind, match_, text } => {
                self.jean_action(cx, move |c| j::add_memory(&c, &kind, &match_, &[], &text))
            }
            JeanViewEvent::RemoveMemory(id) => {
                self.jean_action(cx, move |c| j::remove_memory(&c, &id))
            }
        }
    }

    /// Run a Jean write action on the background executor, then refresh state.
    fn jean_action<F>(&mut self, cx: &mut Context<Self>, f: F)
    where
        F: FnOnce(JeanConfig) -> shelldeck_core::Result<()> + Send + 'static,
    {
        let Some(cfg) = self.effective_jean_config() else {
            return;
        };
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let result = cx.background_executor().spawn(async move { f(cfg) }).await;
            let _ = this.update(cx, |ws, cx| {
                if let Err(e) = result {
                    ws.show_toast(
                        t!("toast.jean.error", error = cloud_account::user_message(&e)).to_string(),
                        ToastLevel::Error,
                        cx,
                    );
                }
                ws.refresh_jean_state(cx);
            });
        })
        .detach();
    }

    fn jean_say(&mut self, text: String, cx: &mut Context<Self>) {
        let Some(cfg) = self.effective_jean_config() else {
            return;
        };
        let activity_text = text.clone();
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move { jeanclaude::say(&cfg, &text) })
                .await;
            let _ = this.update(cx, |ws, cx| {
                match result {
                    Ok(_) => {
                        ws.add_activity_entry(
                            ActivityEntry::new(
                                ActivityKind::Jean,
                                t!("activity.jean.sent").to_string(),
                            )
                            .with_detail(activity_text.clone())
                            .with_action(ActivityAction::OpenJean),
                            cx,
                        );
                        ws.show_toast(t!("toast.jean.sent").to_string(), ToastLevel::Success, cx)
                    }
                    Err(e) => ws.show_toast(
                        t!("toast.jean.error", error = cloud_account::user_message(&e)).to_string(),
                        ToastLevel::Error,
                        cx,
                    ),
                }
                ws.refresh_jean_state(cx);
            });
        })
        .detach();
    }

    fn jean_select_ticket(&mut self, id: String, cx: &mut Context<Self>) {
        let Some(cfg) = self.effective_jean_config() else {
            return;
        };
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move { jeanclaude::get_ticket(&cfg, &id) })
                .await;
            let _ = this.update(cx, |ws, cx| match result {
                Ok(t) => ws.jean_view.update(cx, |v, cx| {
                    v.set_detail(t);
                    cx.notify();
                }),
                Err(e) => ws.jean_view.update(cx, |v, cx| {
                    v.set_error(cloud_account::user_message(&e));
                    cx.notify();
                }),
            });
        })
        .detach();
    }

    fn jean_load_history(&mut self, q: String, status: String, cx: &mut Context<Self>) {
        let Some(cfg) = self.effective_jean_config() else {
            return;
        };
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move { jeanclaude::get_history(&cfg, &q, &status, 60) })
                .await;
            let _ = this.update(cx, |ws, cx| match result {
                Ok(h) => ws.jean_view.update(cx, |v, cx| {
                    v.set_history(h);
                    cx.notify();
                }),
                Err(e) => ws.jean_view.update(cx, |v, cx| {
                    v.set_error(cloud_account::user_message(&e));
                    cx.notify();
                }),
            });
        })
        .detach();
    }

    fn jean_load_targets(&mut self, cx: &mut Context<Self>) {
        let Some(cfg) = self.effective_jean_config() else {
            return;
        };
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move { jeanclaude::get_targets(&cfg) })
                .await;
            let _ = this.update(cx, |ws, cx| {
                if let Ok(t) = result {
                    ws.jean_view.update(cx, |v, cx| {
                        v.set_targets(t);
                        cx.notify();
                    });
                }
            });
        })
        .detach();
    }

    fn jean_load_memory(&mut self, cx: &mut Context<Self>) {
        let Some(cfg) = self.effective_jean_config() else {
            return;
        };
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move { jeanclaude::get_memory(&cfg) })
                .await;
            let _ = this.update(cx, |ws, cx| {
                if let Ok(m) = result {
                    ws.jean_view.update(cx, |v, cx| {
                        v.set_memory(m);
                        cx.notify();
                    });
                }
            });
        })
        .detach();
    }

    // --- Jean fleet runtime ---

    /// `(base_url, token)` when signed in to Inklura Manage.
    fn fleet_base_token(&self) -> Option<(String, String)> {
        if self.app_config.cloud_sync.is_configured() {
            Some((
                self.account_base_url(),
                self.app_config.cloud_sync.token.clone(),
            ))
        } else {
            None
        }
    }

    fn fleet_visible(&self) -> bool {
        self.fleet_base_token().is_some()
            && self.effective_mode() == AppMode::Dev
            && self.active_view == ActiveView::Fleet
    }

    fn update_fleet_availability(&mut self, cx: &mut Context<Self>) {
        let show = self.fleet_base_token().is_some() && self.effective_mode() == AppMode::Dev;
        self.sidebar.update(cx, |s, cx| {
            s.set_fleet_available(show);
            cx.notify();
        });
    }

    /// Tenant to register this machine under: the active site's tenant, else the
    /// first known site's, else empty (the server pins it for non-super-admins).
    fn runtime_tenant(&self) -> (String, String) {
        if let Some(active) = self.active_site_info() {
            return (active.tenant_id, active.tenant_name);
        }
        if let Some(dir) = &self.site_directory {
            if let Some(s) = dir.sites.first() {
                return (s.tenant_id.clone(), s.tenant_name.clone());
            }
        }
        (String::new(), String::new())
    }

    fn runtime_workdir_model(&self) -> (String, String) {
        let inst = self.runtime_instance.as_ref();
        let workdir = inst
            .map(|i| i.workdir.clone())
            .filter(|s| !s.trim().is_empty())
            .or_else(|| self.app_config.jean_runtime.workdir.clone())
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| std::env::var("HOME").unwrap_or_else(|_| ".".to_string()));
        let model = inst.map(|i| i.model.clone()).unwrap_or_default();
        (workdir, model)
    }

    fn build_register(&self) -> Option<RegisterInstance> {
        let (tenant_id, tenant_name) = self.runtime_tenant();
        let name = self
            .app_config
            .jean_runtime
            .name
            .clone()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(cloud_account::device_name);
        let (workdir, _) = self.runtime_workdir_model();
        Some(RegisterInstance {
            id: self.app_config.jean_runtime.instance_id.clone(),
            name,
            tenant_id,
            tenant_name,
            site_id: self.app_config.cloud_sync.active_site_id.clone(),
            slack_channel: None,
            workdir,
            model: None,
            // Only set autonomy on the FIRST register (safe default = confirm);
            // later leave it so an admin can flip it to "auto" in the console.
            autonomy: if self.app_config.jean_runtime.instance_id.is_none() {
                Some("confirm".to_string())
            } else {
                None
            },
        })
    }

    fn refresh_fleet_view(&mut self, cx: &mut Context<Self>) {
        let Some((base, token)) = self.fleet_base_token() else {
            return;
        };
        self.fleet_view.update(cx, |v, cx| {
            v.set_loading(true);
            cx.notify();
        });
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move { jean_fleet::get_fleet(&base, &token) })
                .await;
            let _ = this.update(cx, |ws, cx| match result {
                Ok(snap) => {
                    ws.fleet_snapshot = Some(snap.clone());
                    ws.fleet_view.update(cx, |v, cx| {
                        v.set_snapshot(snap);
                        cx.notify();
                    });
                    ws.push_runtime_status_to_fleet(cx);
                    ws.focus_pending_fleet_job(cx);
                }
                Err(e) => ws.fleet_view.update(cx, |v, cx| {
                    v.set_error(cloud_account::user_message(&e));
                    cx.notify();
                }),
            });
        })
        .detach();
    }

    fn push_runtime_status_to_fleet(&mut self, cx: &mut Context<Self>) {
        let enabled = self.app_config.jean_runtime.enabled;
        let my_id = self
            .runtime_instance
            .as_ref()
            .map(|i| i.id.clone())
            .or_else(|| self.app_config.jean_runtime.instance_id.clone());
        let status = if !enabled {
            "désactivé".to_string()
        } else {
            self.runtime_instance
                .as_ref()
                .map(|i| i.status.clone())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "démarrage…".to_string())
        };
        let awaiting = self.runtime_awaiting.clone();
        self.fleet_view.update(cx, |v, cx| {
            v.set_runtime(enabled, my_id, status);
            v.set_awaiting(awaiting);
            cx.notify();
        });
        self.focus_pending_fleet_job(cx);
    }

    fn focus_pending_fleet_job(&mut self, cx: &mut Context<Self>) {
        let Some(job_id) = self.pending_fleet_job_focus.clone() else {
            return;
        };
        let opened = self
            .fleet_view
            .update(cx, |view, cx| view.open_job_by_id(&job_id, cx));
        if opened {
            self.pending_fleet_job_focus = None;
        }
    }

    fn sync_fleet_view_poll(&mut self, cx: &mut Context<Self>) {
        if self.fleet_visible() {
            self.refresh_fleet_view(cx);
            if self._fleet_view_poll.is_none() {
                let task = cx.spawn(async move |this, cx: &mut AsyncApp| loop {
                    cx.background_executor()
                        .timer(std::time::Duration::from_secs(10))
                        .await;
                    let keep = this
                        .update(cx, |ws, cx| {
                            if ws.fleet_visible() {
                                ws.refresh_fleet_view(cx);
                                true
                            } else {
                                false
                            }
                        })
                        .unwrap_or(false);
                    if !keep {
                        break;
                    }
                });
                self._fleet_view_poll = Some(task);
            }
        } else {
            self._fleet_view_poll = None;
        }
    }

    fn handle_fleet_event(&mut self, event: FleetViewEvent, cx: &mut Context<Self>) {
        match event {
            FleetViewEvent::Refresh => self.refresh_fleet_view(cx),
            FleetViewEvent::ToggleRuntime => self.toggle_jean_runtime(cx),
            FleetViewEvent::ApproveJob(id) => self.approve_fleet_job(id, cx),
            FleetViewEvent::RejectJob(id) => self.reject_fleet_job(id, cx),
        }
    }

    /// Enable/disable THIS machine as a fleet runtime. Off by default; enabling
    /// starts the loop. The safety gate is that the loop only *executes* jobs
    /// when enabled AND the instance autonomy is "auto"; "confirm" needs a click.
    pub fn toggle_jean_runtime(&mut self, cx: &mut Context<Self>) {
        if self.fleet_base_token().is_none() {
            self.show_toast(
                t!("toast.jean.login_required_runtime").to_string(),
                ToastLevel::Warning,
                cx,
            );
            return;
        }
        let now = !self.app_config.jean_runtime.enabled;
        self.app_config.jean_runtime.enabled = now;
        if let Err(e) = self.app_config.save() {
            tracing::error!("Failed to save jean_runtime: {}", e);
        }
        if now {
            self.show_toast(
                t!("toast.jean.runtime_on").to_string(),
                ToastLevel::Success,
                cx,
            );
        } else {
            // Best-effort offline heartbeat, then clear local state.
            if let (Some((base, token)), Some(inst)) =
                (self.fleet_base_token(), self.runtime_instance.clone())
            {
                cx.background_executor()
                    .spawn(async move {
                        let _ = jean_fleet::heartbeat(
                            &base,
                            &token,
                            &inst.id,
                            "offline",
                            Some("désactivé"),
                            None,
                        );
                    })
                    .detach();
            }
            self.runtime_instance = None;
            self.runtime_awaiting.clear();
            self.runtime_busy = false;
            self.publish_tray_state(cx);
            self.show_toast(
                t!("toast.jean.runtime_off").to_string(),
                ToastLevel::Info,
                cx,
            );
        }
        self.sync_runtime_loop(cx);
        self.push_runtime_status_to_fleet(cx);
        cx.notify();
    }

    /// Start/stop the runtime loop from config + auth state.
    pub fn sync_runtime_loop(&mut self, cx: &mut Context<Self>) {
        let want = self.app_config.jean_runtime.enabled && self.fleet_base_token().is_some();
        if want {
            if self._runtime_loop.is_none() {
                let task = cx.spawn(async move |this, cx: &mut AsyncApp| {
                    loop {
                        let step = this
                            .update(cx, |ws, cx| ws.runtime_loop_step(cx))
                            .ok()
                            .flatten();
                        let Some(step) = step else {
                            break; // disabled / signed out → stop
                        };
                        match step {
                            RuntimeStep::Register(base, token, reg) => {
                                let r = cx
                                    .background_executor()
                                    .spawn(async move { jean_fleet::register(&base, &token, &reg) })
                                    .await;
                                let _ = this.update(cx, |ws, cx| ws.apply_register(r, cx));
                            }
                            RuntimeStep::HeartbeatOnly(base, token, id, version) => {
                                cx.background_executor()
                                    .spawn(async move {
                                        let _ = jean_fleet::heartbeat(
                                            &base,
                                            &token,
                                            &id,
                                            "online",
                                            None,
                                            Some(&version),
                                        );
                                    })
                                    .await;
                            }
                            RuntimeStep::Tick(tc) => {
                                let r = cx
                                    .background_executor()
                                    .spawn(async move {
                                        jean_fleet::runtime_tick(
                                            &tc.base,
                                            &tc.token,
                                            &tc.instance_id,
                                            &tc.workdir,
                                            &tc.model,
                                            &tc.autonomy,
                                            &tc.version,
                                            &ClaudeExecutor::default(),
                                            std::time::Duration::from_secs(1800),
                                        )
                                    })
                                    .await;
                                let _ = this.update(cx, |ws, cx| ws.apply_tick_result(r, cx));
                            }
                        }
                        cx.background_executor()
                            .timer(std::time::Duration::from_secs(15))
                            .await;
                    }
                });
                self._runtime_loop = Some(task);
            }
        } else {
            self._runtime_loop = None;
        }
    }

    /// Decide this loop iteration's action on the UI thread (keeps all the config
    /// reads + gating in one place). `None` = stop the loop.
    fn runtime_loop_step(&mut self, _cx: &mut Context<Self>) -> Option<RuntimeStep> {
        if !self.app_config.jean_runtime.enabled {
            return None;
        }
        let (base, token) = self.fleet_base_token()?;
        let version = shelldeck_core::VERSION.to_string();

        if self.runtime_instance.is_none() {
            let reg = self.build_register()?;
            return Some(RuntimeStep::Register(base, token, reg));
        }
        let id = self.runtime_instance.as_ref().unwrap().id.clone();
        // Concurrency 1: while a job runs / awaits confirmation, just heartbeat.
        if self.runtime_busy {
            return Some(RuntimeStep::HeartbeatOnly(base, token, id, version));
        }
        let (workdir, model) = self.runtime_workdir_model();
        let autonomy = self.runtime_instance.as_ref().unwrap().autonomy.clone();
        Some(RuntimeStep::Tick(RuntimeTickCtx {
            base,
            token,
            instance_id: id,
            workdir,
            model,
            autonomy,
            version,
        }))
    }

    fn apply_register(&mut self, r: shelldeck_core::Result<JeanInstance>, cx: &mut Context<Self>) {
        match r {
            Ok(inst) => {
                self.app_config.jean_runtime.instance_id = Some(inst.id.clone());
                if let Err(e) = self.app_config.save() {
                    tracing::error!("Failed to persist runtime instance id: {}", e);
                }
                self.runtime_instance = Some(inst);
                self.push_runtime_status_to_fleet(cx);
            }
            Err(e) => {
                self.fleet_view.update(cx, |v, cx| {
                    v.set_error(
                        t!(
                            "toast.jean.register_failed",
                            error = cloud_account::user_message(&e)
                        )
                        .to_string(),
                    );
                    cx.notify();
                });
            }
        }
    }

    fn apply_tick_result(
        &mut self,
        result: shelldeck_core::Result<jean_fleet::TickResult>,
        cx: &mut Context<Self>,
    ) {
        match result {
            Ok(tick) => {
                if let Some(job) = tick.awaiting_confirm {
                    if !self.runtime_awaiting.iter().any(|j| j.id == job.id) {
                        let job_id = job.id.clone();
                        let prompt = job.prompt.clone();
                        self.runtime_awaiting.push(job);
                        self.add_activity_entry(
                            ActivityEntry::new(
                                ActivityKind::Fleet,
                                t!("activity.fleet.awaiting").to_string(),
                            )
                            .with_target(job_id, t!("activity.fleet.job").to_string())
                            .with_detail(prompt)
                            .with_action(ActivityAction::OpenFleet),
                            cx,
                        );
                        self.publish_tray_state(cx);
                    }
                    self.runtime_busy = true;
                    self.show_toast(
                        t!("toast.jean.ticket_awaiting").to_string(),
                        ToastLevel::Warning,
                        cx,
                    );
                }
                self.push_runtime_status_to_fleet(cx);
            }
            Err(e) => {
                self.fleet_view.update(cx, |v, cx| {
                    v.set_error(cloud_account::user_message(&e));
                    cx.notify();
                });
            }
        }
    }

    /// Approve a confirm-mode job: execute it now (running → done/failed).
    fn approve_fleet_job(&mut self, job_id: String, cx: &mut Context<Self>) {
        let Some(job) = self
            .runtime_awaiting
            .iter()
            .find(|j| j.id == job_id)
            .cloned()
        else {
            return;
        };
        let Some((base, token)) = self.fleet_base_token() else {
            return;
        };
        let (workdir, model) = self.runtime_workdir_model();
        self.runtime_awaiting.retain(|j| j.id != job_id);
        self.publish_tray_state(cx);
        // busy stays true through execution.
        self.push_runtime_status_to_fleet(cx);
        self.add_activity_entry(
            ActivityEntry::new(
                ActivityKind::Fleet,
                t!("activity.fleet.running").to_string(),
            )
            .with_target(job.id.clone(), t!("activity.fleet.job").to_string())
            .with_detail(job.prompt.clone())
            .with_action(ActivityAction::OpenFleet),
            cx,
        );
        self.show_toast(
            t!("toast.jean.ticket_running").to_string(),
            ToastLevel::Info,
            cx,
        );
        let job_id_for_activity = job.id.clone();
        let prompt_for_activity = job.prompt.clone();
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let r = cx
                .background_executor()
                .spawn(async move {
                    jean_fleet::execute_job(
                        &base,
                        &token,
                        &job,
                        &workdir,
                        &model,
                        &ClaudeExecutor::default(),
                        std::time::Duration::from_secs(1800),
                    )
                })
                .await;
            let _ = this.update(cx, |ws, cx| {
                let success = r.is_ok();
                if let Err(e) = r {
                    ws.add_activity_entry(
                        ActivityEntry::new(
                            ActivityKind::Fleet,
                            t!("activity.fleet.failed").to_string(),
                        )
                        .with_target(
                            job_id_for_activity.clone(),
                            t!("activity.fleet.job").to_string(),
                        )
                        .with_detail(prompt_for_activity.clone())
                        .with_action(ActivityAction::OpenFleet),
                        cx,
                    );
                    ws.show_toast(
                        t!(
                            "toast.jean.ticket_failed",
                            error = cloud_account::user_message(&e)
                        )
                        .to_string(),
                        ToastLevel::Error,
                        cx,
                    );
                } else {
                    ws.add_activity_entry(
                        ActivityEntry::new(
                            ActivityKind::Fleet,
                            t!("activity.fleet.done").to_string(),
                        )
                        .with_target(
                            job_id_for_activity.clone(),
                            t!("activity.fleet.job").to_string(),
                        )
                        .with_detail(prompt_for_activity.clone())
                        .with_action(ActivityAction::OpenFleet),
                        cx,
                    );
                    ws.show_toast(
                        t!("toast.jean.ticket_done").to_string(),
                        ToastLevel::Success,
                        cx,
                    );
                }
                // Notify the OS whether the job succeeded — the user
                // may have switched away from the ShellDeck window
                // while the executor was running. Muted from Settings
                // → Général via `AppConfig.tray.notify_fleet_done`.
                if ws.app_config.tray.notify_fleet_done {
                    ws.emit_tray_notification(TrayNotification::FleetJobDone { success });
                }
                ws.runtime_busy = false; // free for the next claim
                ws.push_runtime_status_to_fleet(cx);
                ws.refresh_fleet_view(cx);
            });
        })
        .detach();
    }

    /// Reject a confirm-mode job: mark it cancelled server-side.
    fn reject_fleet_job(&mut self, job_id: String, cx: &mut Context<Self>) {
        let Some((base, token)) = self.fleet_base_token() else {
            return;
        };
        let prompt = self
            .runtime_awaiting
            .iter()
            .find(|j| j.id == job_id)
            .map(|j| j.prompt.clone())
            .unwrap_or_default();
        self.runtime_awaiting.retain(|j| j.id != job_id);
        self.runtime_busy = false;
        self.publish_tray_state(cx);
        self.push_runtime_status_to_fleet(cx);
        self.add_activity_entry(
            ActivityEntry::new(
                ActivityKind::Fleet,
                t!("activity.fleet.rejected").to_string(),
            )
            .with_target(job_id.clone(), t!("activity.fleet.job").to_string())
            .with_detail(prompt)
            .with_action(ActivityAction::OpenFleet),
            cx,
        );
        let jid = job_id;
        cx.background_executor()
            .spawn(async move {
                let _ = jean_fleet::update_job(
                    &base,
                    &token,
                    &jid,
                    "cancelled",
                    Some("rejeté depuis ShellDeck"),
                );
            })
            .detach();
        self.refresh_fleet_view(cx);
        cx.notify();
    }

    /// Open the Fleet view (palette / action) in Dev mode.
    pub fn open_fleet(&mut self, cx: &mut Context<Self>) {
        if self.fleet_base_token().is_none() {
            self.show_toast(
                t!("toast.jean.login_required_fleet").to_string(),
                ToastLevel::Warning,
                cx,
            );
            return;
        }
        if self.can_switch_mode() {
            self.set_mode(AppMode::Dev, cx);
        }
        self.active_view = ActiveView::Fleet;
        self.on_active_view_changed(cx);
        cx.notify();
    }

    // --- Hosted issue management (requests) ---

    /// Palette: focus the User-mode "Nouvelle demande" title field.
    pub fn open_new_request(&mut self, cx: &mut Context<Self>) {
        if !self.app_config.cloud_sync.is_configured() {
            self.show_toast(
                t!("toast.issue.login_required_create").to_string(),
                ToastLevel::Warning,
                cx,
            );
            return;
        }
        if self.can_switch_mode() {
            self.set_mode(AppMode::User, cx);
        }
        self.issue_new_source = "user";
        self.user_new_request_sheet_open = true;
        self.sync_issues_poll(cx);
        cx.notify();
    }

    fn open_prefilled_request(
        &mut self,
        title: String,
        body: String,
        source: &'static str,
        cx: &mut Context<Self>,
    ) {
        if !self.app_config.cloud_sync.is_configured() {
            return;
        }
        self.issue_title_state
            .update(cx, |state, cx| state.replace_content(title, cx));
        self.issue_body_state
            .update(cx, |state, cx| state.replace_content(body, cx));
        Self::reset_input(&self.issue_ai_prompt_state.clone(), cx);
        self.issue_new_priority = "normal".to_string();
        self.issue_new_source = source;
        self.issue_ai_expanded = false;
        self.issue_ai_loading = false;
        self.issue_ai_error = None;
        self.issue_ai_request_id = self.issue_ai_request_id.wrapping_add(1);
        if self.can_switch_mode() {
            self.set_mode(AppMode::User, cx);
        }
        self.user_new_request_sheet_open = true;
        self.sync_issues_poll(cx);
        cx.notify();
    }

    /// Reset an `InputState` entity's content back to empty. `set_value` needs
    /// a `Window`, which we don't have in async close callbacks, so we clear
    /// the public `content` field directly (the widget re-reads it on next
    /// paint). Selection state is left at its previous position; since the
    /// content is empty, any range is effectively out of bounds and the
    /// widget clamps it on next input.
    fn reset_input(state: &Entity<InputState>, cx: &mut Context<Self>) {
        state.update(cx, |s, cx| {
            s.reset(cx);
        });
    }

    fn attachment_drafts(&self, target: IssueAttachmentTarget) -> &Vec<AttachmentDraft> {
        match target {
            IssueAttachmentTarget::NewRequest => &self.issue_new_attachments,
            IssueAttachmentTarget::Comment => &self.issue_comment_attachments,
        }
    }

    fn attachment_drafts_mut(
        &mut self,
        target: IssueAttachmentTarget,
    ) -> &mut Vec<AttachmentDraft> {
        match target {
            IssueAttachmentTarget::NewRequest => &mut self.issue_new_attachments,
            IssueAttachmentTarget::Comment => &mut self.issue_comment_attachments,
        }
    }

    fn add_attachment_draft(
        &mut self,
        target: IssueAttachmentTarget,
        draft: AttachmentDraft,
        cx: &mut Context<Self>,
    ) {
        let drafts = self.attachment_drafts_mut(target);
        if drafts.len() >= issues::ISSUE_ATTACHMENT_MAX_COUNT {
            self.show_toast(
                t!(
                    "toast.issue.attachment_limit",
                    count = issues::ISSUE_ATTACHMENT_MAX_COUNT
                )
                .to_string(),
                ToastLevel::Warning,
                cx,
            );
            return;
        }
        drafts.push(draft);
        cx.notify();
    }

    fn import_attachment_paths(
        &mut self,
        target: IssueAttachmentTarget,
        paths: Vec<std::path::PathBuf>,
        generation: u64,
        cx: &mut Context<Self>,
    ) {
        if generation != self.issue_attachment_generation {
            return;
        }
        let remaining =
            issues::ISSUE_ATTACHMENT_MAX_COUNT.saturating_sub(self.attachment_drafts(target).len());
        if remaining == 0 {
            self.show_toast(
                t!(
                    "toast.issue.attachment_limit",
                    count = issues::ISSUE_ATTACHMENT_MAX_COUNT
                )
                .to_string(),
                ToastLevel::Warning,
                cx,
            );
            return;
        }
        self.issue_attachment_busy = true;
        cx.notify();
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let loaded = cx
                .background_executor()
                .spawn(async move {
                    paths
                        .into_iter()
                        .take(remaining)
                        .map(|path| AttachmentDraft::from_path(&path))
                        .collect::<Vec<_>>()
                })
                .await;
            let _ = this.update(cx, |ws, cx| {
                ws.issue_attachment_busy = false;
                if generation != ws.issue_attachment_generation {
                    cx.notify();
                    return;
                }
                for result in loaded {
                    match result {
                        Ok(draft) => ws.add_attachment_draft(target, draft, cx),
                        Err(error) => ws.show_toast(
                            t!("toast.issue.attachment_failed", error = error).to_string(),
                            ToastLevel::Error,
                            cx,
                        ),
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn pick_issue_attachments(
        &mut self,
        target: IssueAttachmentTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let receiver = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some(t!("user.requests.attachments.choose").to_string().into()),
            starting_directory: None,
        });
        let generation = self.issue_attachment_generation;
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = receiver.await else {
                return;
            };
            let _ = this.update(cx, |ws, cx| {
                ws.import_attachment_paths(target, paths, generation, cx)
            });
        })
        .detach();
    }

    fn paste_issue_attachment(
        &mut self,
        target: IssueAttachmentTarget,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(item) = cx.read_from_clipboard() else {
            return false;
        };
        let image = item.entries().iter().find_map(|entry| match entry {
            ClipboardEntry::Image(image) => Some(image),
            _ => None,
        });
        let Some(image) = image else { return false };
        match draft_from_clipboard_image(image) {
            Ok(draft) => self.add_attachment_draft(target, draft, cx),
            Err(error) => self.show_toast(
                t!("toast.issue.attachment_failed", error = error).to_string(),
                ToastLevel::Error,
                cx,
            ),
        }
        true
    }

    fn import_issue_attachment_url(
        &mut self,
        target: IssueAttachmentTarget,
        cx: &mut Context<Self>,
    ) {
        let url = self
            .issue_attachment_url_state
            .read(cx)
            .content()
            .trim()
            .to_string();
        if url.is_empty() || self.issue_attachment_busy {
            return;
        }
        self.issue_attachment_busy = true;
        let generation = self.issue_attachment_generation;
        cx.notify();
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move { issues::download_issue_image_url(&url) })
                .await;
            let _ = this.update(cx, |ws, cx| {
                ws.issue_attachment_busy = false;
                if generation != ws.issue_attachment_generation {
                    cx.notify();
                    return;
                }
                match result.and_then(|upload| {
                    AttachmentDraft::from_bytes(upload.filename, upload.bytes)
                        .map_err(shelldeck_core::ShellDeckError::Connection)
                }) {
                    Ok(draft) => {
                        ws.add_attachment_draft(target, draft, cx);
                        Self::reset_input(&ws.issue_attachment_url_state.clone(), cx);
                    }
                    Err(error) => ws.show_toast(
                        t!(
                            "toast.issue.attachment_failed",
                            error = cloud_account::user_message(&error)
                        )
                        .to_string(),
                        ToastLevel::Error,
                        cx,
                    ),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn capture_issue_attachment(&mut self, target: IssueAttachmentTarget, cx: &mut Context<Self>) {
        if self.issue_attachment_busy {
            return;
        }
        self.issue_attachment_busy = true;
        let generation = self.issue_attachment_generation;
        cx.notify();
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async { capture_region() })
                .await;
            let _ = this.update(cx, |ws, cx| {
                ws.issue_attachment_busy = false;
                if generation != ws.issue_attachment_generation {
                    cx.notify();
                    return;
                }
                match result {
                    Ok(draft) => ws.add_attachment_draft(target, draft, cx),
                    Err(error) => ws.show_toast(
                        t!("toast.issue.attachment_failed", error = error).to_string(),
                        ToastLevel::Warning,
                        cx,
                    ),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Close the "Nouvelle demande" sheet. Plays the exit animation first
    /// (sheet is kept mounted with `dismissing = true`), then clears the state
    /// once the animation duration has elapsed.
    fn close_new_request_sheet(&mut self, cx: &mut Context<Self>) {
        if self.user_new_request_sheet_dismissing || !self.user_new_request_sheet_open {
            return;
        }
        self.user_new_request_sheet_dismissing = true;
        self.issue_attachment_generation = self.issue_attachment_generation.wrapping_add(1);
        self.issue_ai_request_id = self.issue_ai_request_id.wrapping_add(1);
        self.issue_ai_expanded = false;
        self.issue_ai_loading = false;
        self.issue_ai_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(SHEET_ANIM_MS))
                .await;
            let _ = this.update(cx, |ws, cx| {
                ws.user_new_request_sheet_open = false;
                ws.user_new_request_sheet_dismissing = false;
                Self::reset_input(&ws.issue_title_state.clone(), cx);
                Self::reset_input(&ws.issue_body_state.clone(), cx);
                Self::reset_input(&ws.issue_ai_prompt_state.clone(), cx);
                Self::reset_input(&ws.issue_attachment_url_state.clone(), cx);
                ws.issue_new_attachments.clear();
                ws.issue_new_source = "user";
                cx.notify();
            });
        })
        .detach();
    }

    /// Close the User-mode issue detail sheet. Same delayed-unmount pattern
    /// as `close_new_request_sheet`.
    fn close_user_issue_detail(&mut self, cx: &mut Context<Self>) {
        if self.user_issue_detail_dismissing || self.issue_selected.is_none() {
            return;
        }
        self.user_issue_detail_dismissing = true;
        self.issue_attachment_generation = self.issue_attachment_generation.wrapping_add(1);
        cx.notify();
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(SHEET_ANIM_MS))
                .await;
            let _ = this.update(cx, |ws, cx| {
                ws.issue_selected = None;
                ws.issue_detail = None;
                ws.user_issue_detail_dismissing = false;
                Self::reset_input(&ws.issue_comment_state.clone(), cx);
                Self::reset_input(&ws.issue_attachment_url_state.clone(), cx);
                ws.issue_comment_attachments.clear();
                cx.notify();
            });
        })
        .detach();
    }

    /// Palette: open the Support console's Demandes tab.
    pub fn open_support_requests(&mut self, cx: &mut Context<Self>) {
        if !self.app_config.cloud_sync.is_configured() {
            self.show_toast(
                t!("toast.issue.login_required_list").to_string(),
                ToastLevel::Warning,
                cx,
            );
            return;
        }
        if self.can_switch_mode() {
            self.set_mode(AppMode::Support, cx);
        }
        self.support.update(cx, |v, cx| {
            v.set_section(crate::support_view::SupportSection::Requests);
            cx.notify();
        });
        self.refresh_issues(cx);
        cx.notify();
    }

    /// A Jean/issues surface is on screen (User home, or Support mode).
    fn issues_relevant(&self) -> bool {
        self.app_config.cloud_sync.is_configured()
            && matches!(self.effective_mode(), AppMode::User | AppMode::Support)
    }

    fn refresh_issues(&mut self, cx: &mut Context<Self>) {
        let Some((base, token)) = self.fleet_base_token() else {
            return;
        };
        let filter = self.issues_filter.clone();
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move { issues::list_issues(&base, &token, &filter) })
                .await;
            let _ = this.update(cx, |ws, cx| match result {
                Ok(list) => {
                    ws.issues_list = list.issues.clone();
                    ws.issues_staff = list.staff;
                    ws.issues_instances = list.instances.clone();
                    ws.push_issues_to_support(cx);
                    cx.notify();
                }
                Err(e) => ws.show_toast(
                    t!(
                        "toast.issue.list_failed",
                        error = cloud_account::user_message(&e)
                    )
                    .to_string(),
                    ToastLevel::Error,
                    cx,
                ),
            });
        })
        .detach();
    }

    fn push_issues_to_support(&mut self, cx: &mut Context<Self>) {
        let issues = self.issues_list.clone();
        let staff = self.issues_staff;
        let instances = self.issues_instances.clone();
        let detail = self.issue_detail.clone();
        let (acc_name, acc_email) = self
            .app_config
            .account
            .as_ref()
            .map(|a| (a.name.clone(), a.email.clone()))
            .unwrap_or_default();
        self.support.update(cx, |v, cx| {
            v.set_account(&acc_name, &acc_email);
            v.set_issues(issues, staff, instances);
            v.set_issue_detail(detail);
            cx.notify();
        });
    }

    /// Push the current `AppConfig.account` identity to `SupportView` — used
    /// on login/logout transitions so the child's identity cache doesn't
    /// outlive the workspace-owned account state (violation of
    /// `.agents/session-state.md` if it does).
    fn push_account_to_support(&mut self, cx: &mut Context<Self>) {
        let (acc_name, acc_email) = self
            .app_config
            .account
            .as_ref()
            .map(|a| (a.name.clone(), a.email.clone()))
            .unwrap_or_default();
        self.support.update(cx, |v, cx| {
            v.set_account(&acc_name, &acc_email);
            cx.notify();
        });
    }

    fn sync_issues_poll(&mut self, cx: &mut Context<Self>) {
        if self.issues_relevant() {
            self.refresh_issues(cx);
            if self._issues_poll.is_none() {
                let task = cx.spawn(async move |this, cx: &mut AsyncApp| loop {
                    cx.background_executor()
                        .timer(std::time::Duration::from_secs(15))
                        .await;
                    let keep = this
                        .update(cx, |ws, cx| {
                            if ws.issues_relevant() {
                                ws.refresh_issues(cx);
                                true
                            } else {
                                false
                            }
                        })
                        .unwrap_or(false);
                    if !keep {
                        break;
                    }
                });
                self._issues_poll = Some(task);
            }
        } else {
            self._issues_poll = None;
        }
    }

    pub fn select_issue(&mut self, id: String, cx: &mut Context<Self>) {
        let Some((base, token)) = self.fleet_base_token() else {
            return;
        };
        if self.issue_selected.as_deref() != Some(id.as_str()) {
            self.issue_attachment_generation = self.issue_attachment_generation.wrapping_add(1);
            self.issue_comment_attachments.clear();
            Self::reset_input(&self.issue_attachment_url_state.clone(), cx);
        }
        self.issue_selected = Some(id.clone());
        self.add_activity_entry(
            ActivityEntry::new(
                ActivityKind::Issue,
                t!("activity.issue.open", id = id.as_str()).to_string(),
            )
            .with_target(id.clone(), id.clone())
            .with_action(ActivityAction::OpenIssue),
            cx,
        );
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move { issues::get_issue(&base, &token, &id) })
                .await;
            let _ = this.update(cx, |ws, cx| match result {
                Ok(iss) => {
                    ws.issue_detail = Some(iss);
                    ws.push_issues_to_support(cx);
                    cx.notify();
                }
                Err(e) => ws.show_toast(
                    t!(
                        "toast.issue.detail_failed",
                        error = cloud_account::user_message(&e)
                    )
                    .to_string(),
                    ToastLevel::Error,
                    cx,
                ),
            });
        })
        .detach();
    }

    /// Create a request. `source` = "user" (User mode) or "support".
    /// Replace an existing issue in `issues_list` by id, or prepend it if
    /// absent (matches the server's default `updated_at DESC` order for a
    /// freshly-created row). Called after a server-side mutation returns
    /// the updated record so we don't need an eager list refetch.
    fn upsert_issue_in_list(&mut self, iss: Issue) {
        if let Some(pos) = self.issues_list.iter().position(|i| i.id == iss.id) {
            self.issues_list[pos] = iss;
        } else {
            self.issues_list.insert(0, iss);
        }
    }

    /// Drop an issue from `issues_list` by id (soft-delete).
    fn remove_issue_from_list(&mut self, id: &str) {
        self.issues_list.retain(|i| i.id != id);
    }

    fn create_issue_now(
        &mut self,
        title: String,
        body: String,
        priority: String,
        source: &'static str,
        attachments: Vec<AttachmentDraft>,
        cx: &mut Context<Self>,
    ) {
        if self.issue_attachment_busy {
            return;
        }
        let title = title.trim().to_string();
        if title.is_empty() {
            return;
        }
        let Some((base, token)) = self.fleet_base_token() else {
            return;
        };
        self.issue_attachment_busy = true;
        self.issue_attachment_generation = self.issue_attachment_generation.wrapping_add(1);
        cx.notify();
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let created =
                        issues::create_issue(&base, &token, &title, &body, &priority, source)?;
                    if attachments.is_empty() {
                        return Ok::<_, shelldeck_core::ShellDeckError>((created, None));
                    }
                    let uploads = attachments
                        .iter()
                        .map(AttachmentDraft::upload)
                        .collect::<Vec<_>>();
                    match issues::upload_issue_attachments(&base, &token, &created.id, &uploads)
                        .and_then(|receipts| {
                            issues::attach_issue_images(&base, &token, &created.id, &receipts)
                        }) {
                        Ok(updated) => Ok((updated, None)),
                        Err(error) => Ok((created, Some(error))),
                    }
                })
                .await;
            let _ = this.update(cx, |ws, cx| {
                ws.issue_attachment_busy = false;
                match result {
                    Ok((iss, attachment_error)) => {
                        let preserve_attachments = attachment_error.is_some();
                        ws.show_toast(
                            t!("toast.issue.created").to_string(),
                            ToastLevel::Success,
                            cx,
                        );
                        ws.add_activity_entry(
                            ActivityEntry::new(
                                ActivityKind::Issue,
                                t!("activity.issue.created", title = iss.title.as_str())
                                    .to_string(),
                            )
                            .with_target(iss.id.clone(), iss.title.clone())
                            .with_action(ActivityAction::OpenIssue),
                            cx,
                        );
                        // Success: close the composer sheet, clear its buffers,
                        // and pop the detail sheet on the newly-created request.
                        ws.user_new_request_sheet_open = false;
                        Self::reset_input(&ws.issue_title_state.clone(), cx);
                        Self::reset_input(&ws.issue_body_state.clone(), cx);
                        Self::reset_input(&ws.issue_ai_prompt_state.clone(), cx);
                        Self::reset_input(&ws.issue_attachment_url_state.clone(), cx);
                        if preserve_attachments {
                            ws.issue_comment_attachments =
                                std::mem::take(&mut ws.issue_new_attachments);
                        } else {
                            ws.issue_new_attachments.clear();
                        }
                        ws.issue_ai_request_id = ws.issue_ai_request_id.wrapping_add(1);
                        ws.issue_ai_expanded = false;
                        ws.issue_ai_loading = false;
                        ws.issue_ai_error = None;
                        ws.issue_new_source = "user";
                        ws.upsert_issue_in_list(iss.clone());
                        ws.issue_detail = Some(iss.clone());
                        ws.issue_selected = Some(iss.id.clone());
                        ws.push_issues_to_support(cx);
                        if let Some(error) = attachment_error {
                            ws.show_toast(
                                t!(
                                    "toast.issue.attachment_failed_after_create",
                                    error = cloud_account::user_message(&error)
                                )
                                .to_string(),
                                ToastLevel::Warning,
                                cx,
                            );
                        }
                        cx.notify();
                    }
                    Err(e) => ws.show_toast(
                        t!(
                            "toast.issue.create_failed",
                            error = cloud_account::user_message(&e)
                        )
                        .to_string(),
                        ToastLevel::Error,
                        cx,
                    ),
                }
            });
        })
        .detach();
    }

    /// Comment on the selected issue (users can comment on their own requests).
    pub fn comment_issue_now(&mut self, id: String, body: String, cx: &mut Context<Self>) {
        self.comment_issue_with_images(id, body, Vec::new(), cx);
    }

    fn comment_issue_with_images(
        &mut self,
        id: String,
        body: String,
        attachments: Vec<AttachmentDraft>,
        cx: &mut Context<Self>,
    ) {
        if self.issue_attachment_busy {
            return;
        }
        let body = body.trim().to_string();
        if body.is_empty() && attachments.is_empty() {
            return;
        }
        let Some((base, token)) = self.fleet_base_token() else {
            return;
        };
        self.issue_attachment_busy = true;
        self.issue_attachment_generation = self.issue_attachment_generation.wrapping_add(1);
        cx.notify();
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    if attachments.is_empty() {
                        issues::comment_issue(&base, &token, &id, &body)
                    } else {
                        let uploads = attachments
                            .iter()
                            .map(AttachmentDraft::upload)
                            .collect::<Vec<_>>();
                        let receipts =
                            issues::upload_issue_attachments(&base, &token, &id, &uploads)?;
                        issues::comment_issue_with_attachments(&base, &token, &id, &body, &receipts)
                    }
                })
                .await;
            let _ = this.update(cx, |ws, cx| {
                ws.issue_attachment_busy = false;
                match result {
                    Ok(iss) => {
                        ws.upsert_issue_in_list(iss.clone());
                        ws.issue_detail = Some(iss);
                        if let Some((id, title)) = ws
                            .issue_detail
                            .as_ref()
                            .map(|detail| (detail.id.clone(), detail.title.clone()))
                        {
                            ws.add_activity_entry(
                                ActivityEntry::new(
                                    ActivityKind::Issue,
                                    t!("activity.issue.commented", title = title.as_str())
                                        .to_string(),
                                )
                                .with_target(id, title)
                                .with_action(ActivityAction::OpenIssue),
                                cx,
                            );
                        }
                        ws.push_issues_to_support(cx);
                        ws.support.update(cx, |view, cx| {
                            view.clear_composer_after_send(cx);
                        });
                        Self::reset_input(&ws.issue_comment_state.clone(), cx);
                        Self::reset_input(&ws.issue_attachment_url_state.clone(), cx);
                        ws.issue_comment_attachments.clear();
                        cx.notify();
                    }
                    Err(e) => {
                        let message = t!(
                            "toast.issue.comment_failed",
                            error = cloud_account::user_message(&e)
                        )
                        .to_string();
                        ws.support.update(cx, |view, cx| {
                            view.set_error(message.clone());
                            cx.notify();
                        });
                        ws.show_toast(message, ToastLevel::Error, cx);
                    }
                }
            });
        })
        .detach();
    }

    /// Generic staff issue action (status/assign/priority/dispatch/github);
    /// installs the updated issue in the list + refreshes the detail. The
    /// 15 s issues poll catches any drift on other rows.
    pub fn issue_staff_action<F>(&mut self, cx: &mut Context<Self>, f: F)
    where
        F: FnOnce(String, String) -> shelldeck_core::Result<Issue> + Send + 'static,
    {
        let Some((base, token)) = self.fleet_base_token() else {
            return;
        };
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move { f(base, token) })
                .await;
            let _ = this.update(cx, |ws, cx| match result {
                Ok(iss) => {
                    ws.upsert_issue_in_list(iss.clone());
                    ws.issue_detail = Some(iss);
                    if let Some((id, title)) = ws
                        .issue_detail
                        .as_ref()
                        .map(|detail| (detail.id.clone(), detail.title.clone()))
                    {
                        ws.add_activity_entry(
                            ActivityEntry::new(
                                ActivityKind::Issue,
                                t!("activity.issue.updated", title = title.as_str()).to_string(),
                            )
                            .with_target(id, title)
                            .with_action(ActivityAction::OpenIssue),
                            cx,
                        );
                    }
                    ws.push_issues_to_support(cx);
                    cx.notify();
                }
                Err(e) => ws.show_toast(
                    t!(
                        "toast.issue.staff_failed",
                        error = cloud_account::user_message(&e)
                    )
                    .to_string(),
                    ToastLevel::Error,
                    cx,
                ),
            });
        })
        .detach();
    }

    fn apply_issue_triage(
        &mut self,
        issue_id: String,
        proposal: AiIssueTriageProposal,
        cx: &mut Context<Self>,
    ) {
        if !self.issues_staff {
            self.show_toast(
                t!("toast.ai.triage_staff_only").to_string(),
                ToastLevel::Error,
                cx,
            );
            return;
        }
        let Some(current) = self
            .issue_detail
            .as_ref()
            .filter(|issue| issue.id == issue_id)
            .or_else(|| self.issues_list.iter().find(|issue| issue.id == issue_id))
        else {
            self.show_toast(
                t!("toast.ai.triage_obsolete").to_string(),
                ToastLevel::Error,
                cx,
            );
            return;
        };
        if let Some(assignee) = proposal.assignee.as_deref() {
            if !self.support.read(cx).is_known_issue_assignee(assignee) {
                self.show_toast(
                    t!("toast.ai.triage_unknown_assignee", assignee = assignee).to_string(),
                    ToastLevel::Error,
                    cx,
                );
                return;
            }
        }
        let priority = proposal
            .priority
            .filter(|priority| priority != &current.priority);
        let assignee = proposal
            .assignee
            .filter(|assignee| !assignee.eq_ignore_ascii_case(current.assignee.trim()));
        let change_count = usize::from(priority.is_some()) + usize::from(assignee.is_some());
        if change_count == 0 {
            self.show_toast(
                t!("toast.ai.triage_no_changes").to_string(),
                ToastLevel::Info,
                cx,
            );
            return;
        }
        let Some((base, token)) = self.fleet_base_token() else {
            return;
        };
        self.show_toast(
            t!("toast.ai.triage_applying").to_string(),
            ToastLevel::Info,
            cx,
        );
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let mut updated = None;
                    if let Some(priority) = priority {
                        updated = Some(issues::set_priority(&base, &token, &issue_id, &priority)?);
                    }
                    if let Some(assignee) = assignee {
                        updated = Some(issues::assign(&base, &token, &issue_id, &assignee)?);
                    }
                    updated.ok_or_else(|| {
                        shelldeck_core::ShellDeckError::Config(
                            "issue triage has no applicable changes".to_string(),
                        )
                    })
                })
                .await;
            let _ = this.update(cx, |ws, cx| match result {
                Ok(issue) => {
                    ws.upsert_issue_in_list(issue.clone());
                    ws.issue_detail = Some(issue.clone());
                    ws.push_issues_to_support(cx);
                    ws.add_activity_entry(
                        ActivityEntry::new(
                            ActivityKind::Issue,
                            t!("activity.issue.updated", title = issue.title.as_str()).to_string(),
                        )
                        .with_target(issue.id, issue.title)
                        .with_action(ActivityAction::OpenIssue),
                        cx,
                    );
                    ws.show_toast(
                        t!("toast.ai.triage_applied", count = change_count).to_string(),
                        ToastLevel::Success,
                        cx,
                    );
                    cx.notify();
                }
                Err(error) => {
                    ws.show_toast(
                        t!(
                            "toast.ai.triage_failed",
                            error = cloud_account::user_message(&error)
                        )
                        .to_string(),
                        ToastLevel::Error,
                        cx,
                    );
                    ws.refresh_issues(cx);
                }
            });
        })
        .detach();
    }

    fn apply_support_triage(
        &mut self,
        ticket_id: String,
        proposal: AiIssueTriageProposal,
        cx: &mut Context<Self>,
    ) {
        let selected = self.support.read(cx).selected_ticket_identity();
        if selected.as_ref().map(|(id, _)| id.as_str()) != Some(ticket_id.as_str()) {
            self.show_toast(
                t!("toast.ai.triage_obsolete").to_string(),
                ToastLevel::Error,
                cx,
            );
            return;
        }
        if let Some(assignee) = proposal.assignee.as_deref() {
            if !self.support.read(cx).is_known_support_assignee(assignee) {
                self.show_toast(
                    t!("toast.ai.triage_unknown_assignee", assignee = assignee).to_string(),
                    ToastLevel::Error,
                    cx,
                );
                return;
            }
        }
        let Some((current_priority, current_assignee)) =
            self.support.read(cx).selected_ticket_triage_state()
        else {
            return;
        };
        let priority = proposal
            .priority
            .filter(|priority| !priority.eq_ignore_ascii_case(current_priority.trim()));
        let assignee = proposal
            .assignee
            .filter(|assignee| !assignee.eq_ignore_ascii_case(current_assignee.trim()));
        let change_count = usize::from(priority.is_some()) + usize::from(assignee.is_some());
        if change_count == 0 {
            self.show_toast(
                t!("toast.ai.triage_no_changes").to_string(),
                ToastLevel::Info,
                cx,
            );
            return;
        }
        let base = self.account_base_url();
        let token = self.app_config.cloud_sync.token.clone();
        self.show_toast(
            t!("toast.ai.triage_applying").to_string(),
            ToastLevel::Info,
            cx,
        );
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let mut updated = None;
                    if let Some(priority) = priority {
                        updated = Some(manage_support::support_priority(
                            &base, &token, &ticket_id, &priority,
                        )?);
                    }
                    if let Some(assignee) = assignee {
                        updated = Some(manage_support::support_assign(
                            &base, &token, &ticket_id, &assignee,
                        )?);
                    }
                    updated.ok_or_else(|| {
                        shelldeck_core::ShellDeckError::Config(
                            "support triage has no applicable changes".to_string(),
                        )
                    })
                })
                .await;
            let _ = this.update(cx, |workspace, cx| match result {
                Ok(ticket) => {
                    let label = ticket.subject.clone();
                    let id = ticket.id.clone();
                    workspace.support.update(cx, |view, cx| {
                        view.set_detail(ticket, cx);
                    });
                    workspace.add_activity_entry(
                        ActivityEntry::new(
                            ActivityKind::Support,
                            t!("activity.support.updated", subject = label.as_str()).to_string(),
                        )
                        .with_target(id, label)
                        .with_action(ActivityAction::OpenTicket),
                        cx,
                    );
                    workspace.show_toast(
                        t!("toast.ai.triage_applied", count = change_count).to_string(),
                        ToastLevel::Success,
                        cx,
                    );
                    workspace.refresh_support(cx);
                }
                Err(error) => workspace.show_toast(
                    t!(
                        "toast.ai.triage_failed",
                        error = cloud_account::user_message(&error)
                    )
                    .to_string(),
                    ToastLevel::Error,
                    cx,
                ),
            });
        })
        .detach();
    }

    /// Soft-delete a request (owner-or-staff). On success the row is
    /// removed from the local list, the detail pane closed, and any drift
    /// is caught by the 15 s issues poll.
    fn delete_issue_now(&mut self, id: String, cx: &mut Context<Self>) {
        let Some((base, token)) = self.fleet_base_token() else {
            return;
        };
        let deleted_id = id.clone();
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move { issues::delete_issue(&base, &token, &id) })
                .await;
            let _ = this.update(cx, |ws, cx| match result {
                Ok(_) => {
                    ws.add_activity(
                        t!("activity.issue.deleted", id = deleted_id.as_str()).to_string(),
                        ActivityKind::Issue,
                        cx,
                    );
                    ws.remove_issue_from_list(&deleted_id);
                    if ws.issue_selected.as_deref() == Some(deleted_id.as_str()) {
                        ws.issue_selected = None;
                        ws.issue_detail = None;
                    }
                    ws.push_issues_to_support(cx);
                    ws.show_toast(
                        t!("toast.issue.deleted").to_string(),
                        ToastLevel::Success,
                        cx,
                    );
                    cx.notify();
                }
                Err(e) => ws.show_toast(
                    t!(
                        "toast.issue.delete_failed",
                        error = cloud_account::user_message(&e)
                    )
                    .to_string(),
                    ToastLevel::Error,
                    cx,
                ),
            });
        })
        .detach();
    }

    /// Whether the given issue was filed by the currently signed-in user
    /// (matching `requested_by` against the account name or email — the
    /// server stores `actor = user_name || user_email` so we accept either).
    /// Comparison is trimmed + case-insensitive to tolerate cosmetic drift
    /// between the token payload and whoami.
    fn is_my_issue(&self, iss: &Issue) -> bool {
        let Some(a) = self.app_config.account.as_ref() else {
            return false;
        };
        let rb = iss.requested_by.trim().to_ascii_lowercase();
        if rb.is_empty() {
            return false;
        }
        let name = a.name.trim().to_ascii_lowercase();
        let email = a.email.trim().to_ascii_lowercase();
        (!name.is_empty() && rb == name) || (!email.is_empty() && rb == email)
    }

    /// Destructive confirm modal for soft-deleting a request from User mode.
    fn render_delete_issue_modal(&self, id: String, cx: &mut Context<Self>) -> impl IntoElement {
        let entity = cx.entity();
        let title: SharedString = self
            .issue_detail
            .as_ref()
            .filter(|i| i.id == id)
            .map(|i| i.title.clone())
            .or_else(|| {
                self.issues_list
                    .iter()
                    .find(|i| i.id == id)
                    .map(|i| i.title.clone())
            })
            .unwrap_or_default()
            .into();

        let close_entity = entity.clone();
        let confirm_entity = entity;
        let confirm_id = id;

        render_issue_delete_dialog(
            title,
            "ws-iss-del",
            move |cx| {
                close_entity.update(cx, |this, cx| {
                    this.confirm_issue_delete = None;
                    cx.notify();
                });
            },
            move |cx| {
                let id = confirm_id.clone();
                confirm_entity.update(cx, |this, cx| {
                    this.confirm_issue_delete = None;
                    this.delete_issue_now(id, cx);
                    cx.notify();
                });
            },
        )
    }

    /// Submit the "Nouvelle demande" composer sheet: read the Input states,
    /// hand them to `create_issue_now`. Called from the "Créer" button and
    /// from the Title `Input::on_enter`.
    fn generate_new_request_with_ai(&mut self, cx: &mut Context<Self>) {
        if self.issue_ai_loading
            || !self.ai_backend_available()
            || !self.app_config.ai.allows(AiSurface::Issue)
        {
            return;
        }
        let instructions = self
            .issue_ai_prompt_state
            .read(cx)
            .content()
            .trim()
            .to_string();
        if instructions.is_empty() {
            self.issue_ai_error = Some(t!("user.requests.ai.required").to_string());
            cx.notify();
            return;
        }

        self.issue_ai_request_id = self.issue_ai_request_id.wrapping_add(1);
        let request_id = self.issue_ai_request_id;
        self.issue_ai_loading = true;
        self.issue_ai_error = None;
        let context = AiContext::new(
            AiSurface::Issue,
            t!("ai.context.issue_form").to_string(),
            serde_json::json!({
                "draft": {
                    "title": self.issue_title_state.read(cx).content().to_string(),
                    "description": self.issue_body_state.read(cx).content().to_string(),
                    "priority": self.issue_new_priority.clone(),
                },
                "hosts": self.ai_hosts_context_data(),
            }),
        );
        let prompt = format!(
            "{}\n\n{}:\n{}",
            t!("ai.prompt.issue_generate_form"),
            t!("ai.workflow.additional_instructions"),
            instructions
        );
        let config = self.app_config.ai.clone();
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let client = create_client(&config)?;
                    let response = client.complete(&prompt, context.clone())?;
                    match parse_generated_issue_draft(&response.text) {
                        Ok(draft) => Ok(draft),
                        Err(first_error) => {
                            let repair_prompt = format!(
                                "{}\n\n{}",
                                prompt,
                                t!(
                                    "ai.prompt.issue_generate_repair",
                                    error = first_error.to_string()
                                )
                            );
                            let repaired = client.complete(&repair_prompt, context)?;
                            parse_generated_issue_draft(&repaired.text)
                        }
                    }
                })
                .await
                .map_err(|error| error.to_string());
            let _ = this.update(cx, |ws, cx| {
                if request_id != ws.issue_ai_request_id || !ws.user_new_request_sheet_open {
                    return;
                }
                ws.issue_ai_loading = false;
                match result {
                    Ok(draft) => {
                        ws.issue_title_state
                            .update(cx, |state, cx| state.replace_content(draft.title, cx));
                        ws.issue_body_state
                            .update(cx, |state, cx| state.replace_content(draft.description, cx));
                        ws.issue_new_priority = draft.priority;
                        ws.issue_ai_error = None;
                    }
                    Err(error) => ws.issue_ai_error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn submit_new_request(&mut self, cx: &mut Context<Self>) {
        let title = self.issue_title_state.read(cx).content().to_string();
        let body = self.issue_body_state.read(cx).content().to_string();
        let prio = self.issue_new_priority.clone();
        let source = self.issue_new_source;
        let attachments = self.issue_new_attachments.clone();
        self.create_issue_now(title, body, prio, source, attachments, cx);
    }

    /// Submit the comment composer on the currently-open detail sheet.
    fn submit_issue_comment(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.issue_selected.clone() else {
            return;
        };
        let body = self.issue_comment_state.read(cx).content().to_string();
        let attachments = self.issue_comment_attachments.clone();
        if body.trim().is_empty() && attachments.is_empty() {
            return;
        }
        self.comment_issue_with_images(id, body, attachments, cx);
    }

    // --- bext Cloud (control plane + single-instance SDK) ---

    fn bext_visible(&self) -> bool {
        self.effective_mode() == AppMode::Dev && self.active_view == ActiveView::BextCloud
    }

    /// Open the bext Cloud view (palette / sidebar).
    pub fn open_bext_cloud(&mut self, cx: &mut Context<Self>) {
        if self.effective_mode() != AppMode::Dev && self.can_switch_mode() {
            self.set_mode(AppMode::Dev, cx);
        }
        self.active_view = ActiveView::BextCloud;
        self.on_active_view_changed(cx);
        cx.notify();
    }

    /// Palette: open the view and immediately start the cloud connect flow.
    pub fn connect_bext_cloud_action(&mut self, cx: &mut Context<Self>) {
        self.open_bext_cloud(cx);
        self.connect_bext(cx);
    }

    /// Per-connection "Gérer bext": open the Instance tab. v1 targets the local
    /// loopback SDK (remote reach via SSH tunnel is a follow-up).
    pub fn manage_bext_for_connection(&mut self, conn_id: Uuid, cx: &mut Context<Self>) {
        let app_id = self
            .connections
            .iter()
            .find(|c| c.id == conn_id)
            .map(|c| c.alias.clone())
            .filter(|a| !a.is_empty())
            .unwrap_or_else(|| "default".to_string());
        if self.effective_mode() != AppMode::Dev && self.can_switch_mode() {
            self.set_mode(AppMode::Dev, cx);
        }
        self.active_view = ActiveView::BextCloud;
        let base = "http://127.0.0.1".to_string();
        self.bext_view.update(cx, |v, cx| {
            v.open_instance(base.clone(), app_id.clone(), cx);
            cx.notify();
        });
        self.show_toast(
            t!("toast.bext.local_instance").to_string(),
            ToastLevel::Info,
            cx,
        );
        self.refresh_bext_instance(base, app_id, cx);
        cx.notify();
    }

    fn sync_bext_poll(&mut self, cx: &mut Context<Self>) {
        if self.bext_visible() {
            self.refresh_bext_cloud(cx);
            if self._bext_poll.is_none() {
                let task = cx.spawn(async move |this, cx: &mut AsyncApp| loop {
                    cx.background_executor()
                        .timer(std::time::Duration::from_secs(15))
                        .await;
                    let keep = this
                        .update(cx, |ws, cx| {
                            if ws.bext_visible() {
                                ws.refresh_bext_cloud(cx);
                                true
                            } else {
                                false
                            }
                        })
                        .unwrap_or(false);
                    if !keep {
                        break;
                    }
                });
                self._bext_poll = Some(task);
            }
        } else {
            self._bext_poll = None;
        }
    }

    fn refresh_bext_cloud(&mut self, cx: &mut Context<Self>) {
        let cfg = self.app_config.bext_cloud.clone();
        if !cfg.is_connected() {
            self.bext_view.update(cx, |v, cx| {
                v.set_connection(false, None);
                cx.notify();
            });
            return;
        }
        self.bext_view.update(cx, |v, cx| {
            v.set_loading(true);
            cx.notify();
        });
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let bundle = cx
                .background_executor()
                .spawn(async move {
                    // Fan out whoami / sites / dashboard onto three OS
                    // threads — the bext_cloud client is reqwest-blocking,
                    // so the previous serial chain cost ~3× round-trip.
                    // Instances stays serial after whoami since it's only
                    // fetched for super-admin.
                    let cfg_w = cfg.clone();
                    let cfg_s = cfg.clone();
                    let cfg_d = cfg.clone();
                    let who_h = std::thread::spawn(move || bext_cloud::whoami(&cfg_w));
                    let sites_h = std::thread::spawn(move || bext_cloud::list_sites(&cfg_s));
                    let dash_h = std::thread::spawn(move || bext_cloud::dashboard(&cfg_d));
                    let who = who_h.join().expect("bext whoami thread panicked");
                    let sites = sites_h.join().expect("bext list_sites thread panicked");
                    let dash = dash_h.join().expect("bext dashboard thread panicked");
                    let is_super = who.as_ref().map(|u| u.is_super_admin).unwrap_or(false);
                    let instances = if is_super {
                        bext_cloud::list_instances(&cfg).ok()
                    } else {
                        None
                    };
                    (who, sites, dash, instances)
                })
                .await;
            let _ = this.update(cx, |ws, cx| {
                let (who, sites, dash, instances) = bundle;
                match who {
                    Ok(u) => {
                        ws.bext_user = Some(u.clone());
                        ws.bext_view.update(cx, |v, cx| {
                            v.set_connection(true, Some(u));
                            cx.notify();
                        });
                    }
                    Err(e) => {
                        ws.bext_view.update(cx, |v, cx| {
                            v.set_error(cloud_account::user_message(&e));
                            cx.notify();
                        });
                    }
                }
                if let Ok(s) = sites {
                    ws.bext_view.update(cx, |v, cx| {
                        v.set_sites(s);
                        cx.notify();
                    });
                }
                if let Ok(d) = dash {
                    ws.bext_view.update(cx, |v, cx| {
                        v.set_stats(d.stats);
                        cx.notify();
                    });
                }
                if let Some(insts) = instances {
                    ws.bext_view.update(cx, |v, cx| {
                        v.set_instances(insts.instances);
                        cx.notify();
                    });
                }
            });
        })
        .detach();
    }

    fn connect_bext(&mut self, cx: &mut Context<Self>) {
        let base = {
            let b = self.app_config.bext_cloud.base_url.trim().to_string();
            if b.is_empty() {
                "https://cloud.bext.dev".to_string()
            } else {
                b
            }
        };
        let listener = match std::net::TcpListener::bind("127.0.0.1:0") {
            Ok(l) => l,
            Err(e) => {
                self.show_toast(
                    t!("toast.local_port_open_failed", error = e.to_string()).to_string(),
                    ToastLevel::Error,
                    cx,
                );
                return;
            }
        };
        let port = match listener.local_addr() {
            Ok(a) => a.port(),
            Err(e) => {
                self.show_toast(
                    t!("toast.local_port_read_failed", error = e.to_string()).to_string(),
                    ToastLevel::Error,
                    cx,
                );
                return;
            }
        };
        let url = bext_cloud::cli_login_url(&base, port);
        if let Err(e) = cloud_account::open_in_browser(&url) {
            self.show_toast(
                t!(
                    "toast.open_browser_failed",
                    error = cloud_account::user_message(&e)
                )
                .to_string(),
                ToastLevel::Error,
                cx,
            );
            return;
        }
        self.show_toast(
            t!("toast.bext.connect_waiting").to_string(),
            ToastLevel::Info,
            cx,
        );
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let outcome = cx
                .background_executor()
                .spawn(async move {
                    bext_cloud::browser_connect_listen(
                        listener,
                        std::time::Duration::from_secs(180),
                    )
                })
                .await;
            let _ = this.update(cx, |ws, cx| match outcome {
                Ok(conn) => {
                    ws.app_config.bext_cloud.token = conn.token;
                    ws.app_config.bext_cloud.email = conn.email;
                    ws.app_config.bext_cloud.name = conn.name;
                    if let Err(e) = ws.app_config.save() {
                        tracing::error!("Failed to save bext_cloud config: {}", e);
                    }
                    ws.show_toast(
                        t!(
                            "toast.bext.connected",
                            email = ws.app_config.bext_cloud.email.as_str()
                        )
                        .to_string(),
                        ToastLevel::Success,
                        cx,
                    );
                    ws.refresh_bext_cloud(cx);
                }
                Err(e) => ws.show_toast(
                    t!(
                        "toast.bext.connect_failed",
                        error = cloud_account::user_message(&e)
                    )
                    .to_string(),
                    ToastLevel::Error,
                    cx,
                ),
            });
        })
        .detach();
    }

    fn disconnect_bext(&mut self, cx: &mut Context<Self>) {
        self.app_config.bext_cloud.token = String::new();
        self.app_config.bext_cloud.email = String::new();
        self.app_config.bext_cloud.name = String::new();
        if let Err(e) = self.app_config.save() {
            tracing::error!("Failed to save bext_cloud config: {}", e);
        }
        self.bext_user = None;
        self.bext_view.update(cx, |v, cx| {
            v.set_connection(false, None);
            cx.notify();
        });
        self.show_toast(
            t!("toast.bext.disconnected").to_string(),
            ToastLevel::Info,
            cx,
        );
        cx.notify();
    }

    fn bext_cloud_action<F>(&mut self, cx: &mut Context<Self>, f: F)
    where
        F: FnOnce(BextCloudConfig) -> shelldeck_core::Result<()> + Send + 'static,
    {
        let cfg = self.app_config.bext_cloud.clone();
        if !cfg.is_connected() {
            return;
        }
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let r = cx.background_executor().spawn(async move { f(cfg) }).await;
            let _ = this.update(cx, |ws, cx| {
                match r {
                    Ok(_) => ws.show_toast(
                        t!("toast.bext.action_ok").to_string(),
                        ToastLevel::Success,
                        cx,
                    ),
                    Err(e) => ws.show_toast(
                        t!("toast.bext.error", error = cloud_account::user_message(&e)).to_string(),
                        ToastLevel::Error,
                        cx,
                    ),
                }
                ws.refresh_bext_cloud(cx);
            });
        })
        .detach();
    }

    fn refresh_bext_instance(&mut self, base: String, app_id: String, cx: &mut Context<Self>) {
        let (b2, a2) = (base.clone(), app_id.clone());
        self.bext_view.update(cx, |v, cx| {
            v.set_loading(true);
            cx.notify();
        });
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let r = cx
                .background_executor()
                .spawn(async move {
                    let inst = bext_instance::BextInstance::new(base, app_id);
                    bext_instance::list_sites(&inst)
                })
                .await;
            let _ = this.update(cx, |ws, cx| match r {
                Ok(sites) => ws.bext_view.update(cx, |v, cx| {
                    v.set_instance_sites(sites.sites, b2.clone(), a2.clone(), cx);
                    cx.notify();
                }),
                Err(e) => ws.bext_view.update(cx, |v, cx| {
                    v.set_error(cloud_account::user_message(&e));
                    cx.notify();
                }),
            });
        })
        .detach();
    }

    fn bext_instance_action<F>(
        &mut self,
        base: String,
        app_id: String,
        cx: &mut Context<Self>,
        f: F,
    ) where
        F: FnOnce(&bext_instance::BextInstance) -> shelldeck_core::Result<()> + Send + 'static,
    {
        let (b2, a2) = (base.clone(), app_id.clone());
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let r = cx
                .background_executor()
                .spawn(async move {
                    let inst = bext_instance::BextInstance::new(base, app_id);
                    f(&inst)
                })
                .await;
            let _ = this.update(cx, |ws, cx| {
                match r {
                    Ok(_) => ws.show_toast(
                        t!("toast.bext.instance_action_ok").to_string(),
                        ToastLevel::Success,
                        cx,
                    ),
                    Err(e) => ws.show_toast(
                        t!(
                            "toast.bext.instance_error",
                            error = cloud_account::user_message(&e)
                        )
                        .to_string(),
                        ToastLevel::Error,
                        cx,
                    ),
                }
                ws.refresh_bext_instance(b2.clone(), a2.clone(), cx);
            });
        })
        .detach();
    }

    fn handle_bext_event(&mut self, event: BextViewEvent, cx: &mut Context<Self>) {
        match event {
            BextViewEvent::Connect => self.connect_bext(cx),
            BextViewEvent::Disconnect => self.disconnect_bext(cx),
            BextViewEvent::RefreshCloud => self.refresh_bext_cloud(cx),
            BextViewEvent::CreateSite { name, title } => {
                let t = if title.trim().is_empty() {
                    None
                } else {
                    Some(title)
                };
                self.bext_cloud_action(cx, move |cfg| {
                    bext_cloud::create_site(&cfg, &name, t.as_deref()).map(|_| ())
                });
            }
            BextViewEvent::SiteAction { slug, action } => {
                self.bext_cloud_action(cx, move |cfg| {
                    bext_cloud::site_action(&cfg, &slug, &action, None).map(|_| ())
                });
            }
            BextViewEvent::OpenSite(domain) => {
                let url = if domain.starts_with("http") {
                    domain
                } else {
                    format!("https://{}", domain)
                };
                if let Err(e) = cloud_account::open_in_browser(&url) {
                    self.show_toast(
                        t!(
                            "toast.open_failed_generic",
                            error = cloud_account::user_message(&e)
                        )
                        .to_string(),
                        ToastLevel::Error,
                        cx,
                    );
                }
            }
            BextViewEvent::RefreshInstance { base, app_id } => {
                self.refresh_bext_instance(base, app_id, cx)
            }
            BextViewEvent::InstanceCreate {
                base,
                app_id,
                slug,
                title,
            } => {
                let t = if title.trim().is_empty() {
                    None
                } else {
                    Some(title)
                };
                self.bext_instance_action(base, app_id, cx, move |inst| {
                    bext_instance::create_site(inst, &slug, t.as_deref(), None, None).map(|_| ())
                });
            }
            BextViewEvent::InstanceGoLive {
                base,
                app_id,
                slug,
                domain,
            } => {
                self.bext_instance_action(base, app_id, cx, move |inst| {
                    bext_instance::go_live(inst, &slug, &domain).map(|_| ())
                });
            }
            BextViewEvent::InstanceDestroy { base, app_id, slug } => {
                self.bext_instance_action(base, app_id, cx, move |inst| {
                    bext_instance::destroy_site(inst, &slug).map(|_| ())
                });
            }
        }
    }

    fn update_dashboard_stats(&mut self, cx: &mut Context<Self>) {
        let terminal_count = self.terminal.read(cx).tab_count();
        let active_forwards = self.active_tunnels.len();
        let running_scripts = if self.scripts.read(cx).is_running() {
            1
        } else {
            0
        };
        let active_connections = self
            .connections
            .iter()
            .filter(|c| {
                matches!(
                    c.status,
                    shelldeck_core::models::connection::ConnectionStatus::Connected
                )
            })
            .count();

        let quick_connections: Vec<&Connection> = if self.app_config.pinned_connections.is_empty() {
            self.connections.iter().take(5).collect()
        } else {
            self.app_config
                .pinned_connections
                .iter()
                .filter_map(|id| {
                    self.connections
                        .iter()
                        .find(|connection| connection.id == *id)
                })
                .take(5)
                .collect()
        };
        let favorite_hosts: Vec<(Uuid, String, String, bool)> = quick_connections
            .into_iter()
            .map(|c| {
                (
                    c.id,
                    c.display_name().to_string(),
                    c.hostname.clone(),
                    c.status == ConnectionStatus::Connected,
                )
            })
            .collect();

        self.dashboard.update(cx, |d, _| {
            d.active_terminals = terminal_count;
            d.active_forwards = active_forwards;
            d.running_scripts = running_scripts;
            d.active_connections = active_connections;
            d.favorite_hosts = favorite_hosts;
        });
        self.status_bar.update(cx, |bar, _| {
            bar.set_counts(terminal_count, active_forwards, running_scripts);
        });
        // Also push the fresh state to the tray. The tray publisher
        // dedups against its last state, so this is cheap even when
        // update_dashboard_stats runs on every unrelated tick.
        self.publish_tray_state(cx);
    }

    /// Persist the current set of open terminal tabs so they can be restored on
    /// the next launch (when `auto_connect_on_startup` is enabled). Saved
    /// unconditionally and best-effort: failures are logged, never fatal.
    fn save_workspace_state(&self, cx: &Context<Self>) {
        use shelldeck_core::config::workspace_state::{TabState, TabType};
        use shelldeck_core::config::WorkspaceState;

        let terminal = self.terminal.read(cx);
        let sessions = terminal.session_states();
        let active_tab = terminal.active_tab_index();

        let tabs: Vec<TabState> = sessions
            .into_iter()
            .enumerate()
            .map(|(i, (title, connection_id))| {
                let tab_type = if connection_id.is_some() {
                    TabType::Ssh
                } else {
                    TabType::Local
                };
                TabState {
                    id: i.to_string(),
                    title,
                    tab_type,
                    connection_id,
                    // Local tabs are spawned with the default shell, which the
                    // terminal session does not track, so leave this unset.
                    shell: None,
                }
            })
            .collect();

        let state = WorkspaceState {
            tabs,
            active_tab,
            sidebar_visible: self.sidebar_visible,
        };

        if let Err(e) = state.save() {
            tracing::warn!("Failed to save workspace state: {}", e);
        }
    }

    /// Gracefully shut down: close all terminal sessions, stop tunnels, stop background tasks.
    pub fn shutdown(&mut self, cx: &mut Context<Self>) {
        tracing::info!("ShellDeck shutting down gracefully...");
        // Persist open tabs before tearing sessions down so there is something
        // to restore next launch. Best-effort; never blocks shutdown.
        self.save_workspace_state(cx);
        // Stop all active tunnels
        for (fwd_id, tunnel) in self.active_tunnels.drain() {
            tracing::info!("Stopping tunnel for forward {}", fwd_id);
            tunnel.tunnel_handle.stop();
        }
        // Stop all active scripts
        for (script_id, active) in self.active_scripts.drain() {
            tracing::info!("Stopping script {}", script_id);
            active.stop();
        }
        // Close all terminal sessions (drops channels, threads exit)
        self.terminal.update(cx, |terminal, _| {
            terminal.close_all_sessions();
        });
        // Stop git polling
        self._git_poll_task = None;
        // Clear forms if open
        self.connection_form = None;
        self._form_sub = None;
        self.login_form = None;
        self._login_form_sub = None;
        self.onboarding = None;
        self._onboarding_sub = None;
        self.port_forward_form = None;
        self._pf_form_sub = None;
        self.script_form = None;
        self._script_form_sub = None;
        tracing::info!("Shutdown cleanup complete");
    }

    pub fn set_active_view(&mut self, view: ActiveView) {
        self.active_view = view;
    }

    /// Open the JeanClaude console (palette / action). Switches to Dev mode for
    /// super-admins so the console is actually on screen.
    pub fn open_jean_console(&mut self, cx: &mut Context<Self>) {
        if !self.has_jean() {
            self.show_toast(
                t!("toast.jean.not_configured").to_string(),
                ToastLevel::Warning,
                cx,
            );
            return;
        }
        if self.can_switch_mode() {
            self.set_mode(AppMode::Dev, cx);
        }
        self.active_view = ActiveView::JeanConsole;
        self.on_active_view_changed(cx);
        cx.notify();
    }

    /// Route a `shelldeck://…` deep link (already parsed by
    /// `shelldeck_core::config::deep_link`) onto the right surface. Called
    /// from `main.rs` when the OS hands the URL to us — either as the arg
    /// that launched this process, or forwarded from a second launch by the
    /// single-instance guard. Best-effort: an unresolvable target (unknown
    /// UUID, no permission) toasts instead of failing.
    pub fn open_deep_link(&mut self, link: DeepLink, cx: &mut Context<Self>) {
        tracing::info!("deep link: {link:?}");
        match link {
            DeepLink::OpenConnection(id) => {
                if !self.connections.iter().any(|c| c.id == id) {
                    self.show_toast(
                        t!("toast.deeplink.connection_not_found").to_string(),
                        ToastLevel::Warning,
                        cx,
                    );
                    return;
                }
                if self.can_switch_mode() {
                    self.set_mode(AppMode::Dev, cx);
                }
                self.switch_to_section(SidebarSection::Connections);
                self.sidebar.update(cx, |s, cx| {
                    s.focus_connection(id);
                    cx.notify();
                });
                self.on_active_view_changed(cx);
                cx.notify();
            }
            DeepLink::SshConnect(id) => {
                if self.can_switch_mode() {
                    self.set_mode(AppMode::Dev, cx);
                }
                if let Some(conn) = self.connections.iter().find(|c| c.id == id).cloned() {
                    let title = conn.display_name().to_string();
                    let conn_id = conn.id;
                    self.connect_ssh(conn, cx);
                    self.add_activity_entry(
                        ActivityEntry::new(
                            ActivityKind::Connection,
                            t!("activity.connecting_to", name = title.as_str()).to_string(),
                        )
                        .with_target(conn_id.to_string(), title)
                        .with_action(ActivityAction::ConnectConnection),
                        cx,
                    );
                    self.active_view = ActiveView::Terminal;
                    cx.notify();
                } else {
                    self.show_toast(
                        t!("toast.deeplink.connection_not_found").to_string(),
                        ToastLevel::Warning,
                        cx,
                    );
                }
            }
            DeepLink::TunnelStart(id) => {
                if self.can_switch_mode() {
                    self.set_mode(AppMode::Dev, cx);
                }
                self.switch_to_section(SidebarSection::PortForwards);
                self.on_active_view_changed(cx);
                self.handle_forward_event(&PortForwardEvent::StartForward(id), cx);
                cx.notify();
            }
            DeepLink::OpenSite(id) => {
                if self.can_switch_mode() {
                    self.set_mode(AppMode::User, cx);
                }
                let label = self
                    .site_directory
                    .as_ref()
                    .and_then(|p| p.sites.iter().find(|s| s.site_id == id))
                    .map(|s| s.display_label());
                self.select_site(Some(id), label, cx);
                cx.notify();
            }
            DeepLink::OpenIssue(id) => {
                if self.can_switch_mode() {
                    self.set_mode(AppMode::Support, cx);
                    self.support.update(cx, |v, cx| {
                        v.set_section(crate::support_view::SupportSection::Requests);
                        cx.notify();
                    });
                }
                self.select_issue(id, cx);
                cx.notify();
            }
            DeepLink::OpenTicket(id) => {
                if !self.can_switch_mode() {
                    self.show_toast(
                        t!("toast.deeplink.support_only").to_string(),
                        ToastLevel::Warning,
                        cx,
                    );
                    return;
                }
                self.set_mode(AppMode::Support, cx);
                self.support.update(cx, |v, cx| {
                    v.set_section(crate::support_view::SupportSection::Tickets);
                    cx.notify();
                });
                self.select_support_ticket(id, cx);
                cx.notify();
            }
            DeepLink::JeanConfirm(job_id) => {
                self.pending_fleet_job_focus = Some(job_id);
                self.open_fleet(cx);
                self.focus_pending_fleet_job(cx);
            }
        }
    }

    /// Key handling for the User-mode "Demander à JeanClaude" composer.
    fn handle_jean_ask_key(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) {
        let key = event.keystroke.key.as_str();
        match key {
            "enter" => {
                if event.keystroke.modifiers.shift {
                    self.jean_ask_input.push('\n');
                    cx.notify();
                } else {
                    self.submit_jean_ask(cx);
                }
            }
            "backspace" => {
                self.jean_ask_input.pop();
                cx.notify();
            }
            _ => {
                if let Some(ref kc) = event.keystroke.key_char {
                    if !event.keystroke.modifiers.control && !event.keystroke.modifiers.alt {
                        self.jean_ask_input.push_str(kc);
                        cx.notify();
                    }
                } else if key.len() == 1
                    && !event.keystroke.modifiers.control
                    && !event.keystroke.modifiers.alt
                {
                    self.jean_ask_input.push_str(key);
                    cx.notify();
                }
            }
        }
    }

    fn submit_jean_ask(&mut self, cx: &mut Context<Self>) {
        let text = self.jean_ask_input.trim().to_string();
        if text.is_empty() {
            return;
        }
        let name = self
            .app_config
            .account
            .as_ref()
            .map(|a| a.display_name())
            .unwrap_or_default();
        let full = shelldeck_core::config::jeanclaude::format_via_shelldeck(&name, &text);
        self.jean_ask_input.clear();
        self.jean_say(full, cx);
        cx.notify();
    }

    /// Toggle JeanClaude's paused state (palette / action).
    pub fn jean_toggle_pause(&mut self, cx: &mut Context<Self>) {
        if !self.has_jean() {
            return;
        }
        let paused = self
            .jean_state
            .as_ref()
            .map(|s| s.bot.paused)
            .unwrap_or(false);
        self.jean_action(cx, move |c| jeanclaude::set_paused(&c, !paused));
    }

    fn populate_script_editor_connections(&self, cx: &mut Context<Self>) {
        let conns: Vec<(Uuid, String)> = self
            .connections
            .iter()
            .map(|c| (c.id, c.display_name().to_string()))
            .collect();
        self.scripts.update(cx, |editor, _| {
            editor.set_connections(conns);
        });
    }

    /// Public entry point for external callers (system tray, IPC deep
    /// links, remote triggers) to toggle the command palette without
    /// touching the private `command_palette` field.
    pub fn toggle_command_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.command_palette.update(cx, |palette, cx| {
            palette.toggle(window, cx);
            cx.notify();
        });
        cx.notify();
    }

    pub fn prepare_companion_command_palette(
        &mut self,
        cx: &mut Context<Self>,
    ) -> Entity<CommandPalette> {
        self.refresh_command_palette(cx);
        self.companion_command_palette.clone()
    }

    pub fn toggle_sidebar(&mut self, cx: &mut Context<Self>) {
        self.sidebar_visible = !self.sidebar_visible;
        self.sidebar.update(cx, |sidebar, _cx| {
            sidebar.toggle_collapsed();
        });
        let effective_width = if self.sidebar_visible {
            self.sidebar_width
        } else {
            0.0
        };
        self.terminal.update(cx, |terminal, _cx| {
            terminal.set_sidebar_width(effective_width);
        });
    }

    pub fn switch_to_section(&mut self, section: SidebarSection) {
        self.active_view = match section {
            SidebarSection::Connections => ActiveView::Dashboard,
            SidebarSection::Terminals => ActiveView::Terminal,
            SidebarSection::Scripts => ActiveView::Scripts,
            SidebarSection::PortForwards => ActiveView::PortForwards,
            SidebarSection::ServerSync => ActiveView::ServerSync,
            SidebarSection::Sites => ActiveView::Sites,
            SidebarSection::Recent => ActiveView::Recent,
            SidebarSection::FileEditor => ActiveView::FileEditor,
            SidebarSection::JeanConsole => ActiveView::JeanConsole,
            SidebarSection::Fleet => ActiveView::Fleet,
            SidebarSection::BextCloud => ActiveView::BextCloud,
            SidebarSection::Settings => ActiveView::Settings,
        };
    }

    fn activate_dev_section(&mut self, section: SidebarSection, cx: &mut Context<Self>) {
        if self.can_switch_mode() {
            self.set_mode(AppMode::Dev, cx);
        }
        self.switch_to_section(section);
        self.sidebar.update(cx, |sidebar, cx| {
            sidebar.set_active_section(section);
            cx.notify();
        });
        self.on_active_view_changed(cx);
        cx.notify();
    }

    /// Called when the active Dev view changes — (re)start the Jean poll if the
    /// console just became visible.
    fn on_active_view_changed(&mut self, cx: &mut Context<Self>) {
        self.sync_jean_poll(cx);
        self.sync_fleet_view_poll(cx);
        self.sync_bext_poll(cx);
        self.refresh_command_palette(cx);
    }

    fn sync_terminal_tab_count(&self, cx: &mut Context<Self>) {
        let count = self.terminal.read(cx).tabs.len();
        self.sidebar.update(cx, |sidebar, _| {
            sidebar.set_terminal_tab_count(count);
        });
    }

    pub fn next_tab(&mut self, cx: &mut Context<Self>) {
        self.terminal.update(cx, |t, cx| {
            t.next_tab();
            cx.notify();
        });
    }

    pub fn prev_tab(&mut self, cx: &mut Context<Self>) {
        self.terminal.update(cx, |t, cx| {
            t.prev_tab();
            cx.notify();
        });
    }

    pub fn close_active_tab(&mut self, cx: &mut Context<Self>) {
        self.terminal.update(cx, |t, cx| {
            if let Some(tab) = t.tabs.get(t.pane.active_index) {
                let id = tab.id;
                t.close_tab(id);
            }
            cx.notify();
        });
        self.update_dashboard_stats(cx);
        self.sync_terminal_tab_count(cx);
    }

    /// Restore the previously-saved session on startup when
    /// `auto_connect_on_startup` is enabled. Local tabs reopen a default-shell
    /// terminal; SSH tabs reconnect via the existing `connect_ssh` path if the
    /// connection still exists. No-op (and no behavior change) when the flag is
    /// off or there is nothing to restore. Failures are logged, never fatal.
    pub fn restore_session(&mut self, cx: &mut Context<Self>) {
        use shelldeck_core::config::workspace_state::TabType;
        use shelldeck_core::config::WorkspaceState;

        if !self.app_config.general.auto_connect_on_startup {
            return;
        }

        let state = match WorkspaceState::load() {
            Ok(state) => state,
            Err(e) => {
                tracing::warn!("Failed to load workspace state for restore: {}", e);
                return;
            }
        };

        if state.tabs.is_empty() {
            // Nothing saved — leave the normal default (empty) startup untouched.
            return;
        }

        let mut restored = 0usize;
        for tab in &state.tabs {
            match tab.tab_type {
                TabType::Local => {
                    self.terminal.update(cx, |terminal, cx| {
                        terminal.spawn_local_terminal(cx);
                    });
                    restored += 1;
                }
                TabType::Ssh => {
                    let Some(conn_id) = tab.connection_id else {
                        tracing::warn!("Skipping SSH tab restore: missing connection id");
                        continue;
                    };
                    if let Some(conn) = self.connections.iter().find(|c| c.id == conn_id).cloned() {
                        self.connect_ssh(conn, cx);
                        restored += 1;
                    } else {
                        tracing::warn!(
                            "Skipping SSH tab restore: connection {} no longer exists",
                            conn_id
                        );
                    }
                }
            }
        }

        if restored == 0 {
            return;
        }

        // Restore sidebar visibility if it differs from the current state.
        if state.sidebar_visible != self.sidebar_visible {
            self.toggle_sidebar(cx);
        }

        // Restore the active tab (clamped to the number of tabs actually
        // recreated, since some saved tabs may have been skipped).
        self.terminal.update(cx, |terminal, _| {
            if let Some(tab) = terminal
                .tabs
                .get(state.active_tab.min(terminal.tabs.len() - 1))
            {
                let id = tab.id;
                terminal.select_tab(id);
            }
        });

        self.active_view = ActiveView::Terminal;
        self.update_dashboard_stats(cx);
        self.sync_terminal_tab_count(cx);
        cx.notify();
    }

    pub fn open_new_terminal(&mut self, cx: &mut Context<Self>) {
        self.terminal.update(cx, |terminal, cx| {
            terminal.spawn_local_terminal(cx);
        });
        self.active_view = ActiveView::Terminal;
        self.add_activity_entry(
            ActivityEntry::new(
                ActivityKind::Terminal,
                t!("activity.terminal_opened").to_string(),
            )
            .with_action(ActivityAction::OpenTerminal),
            cx,
        );
        self.update_dashboard_stats(cx);
        self.sync_terminal_tab_count(cx);
    }

    /// Start periodic git status polling (every 15 seconds). Only repaints the
    /// status bar when the status string actually changed.
    fn start_git_polling(cx: &mut Context<Self>, status_bar: &Entity<StatusBar>) -> gpui::Task<()> {
        let weak_bar = status_bar.downgrade();
        cx.spawn(async move |_this, cx: &mut AsyncApp| loop {
            cx.background_executor()
                .timer(std::time::Duration::from_secs(15))
                .await;

            let git_display = cx
                .background_executor()
                .spawn(async {
                    let cwd = std::env::current_dir().unwrap_or_default();
                    shelldeck_core::git::get_git_status(&cwd).and_then(|s| s.display())
                })
                .await;

            let result = weak_bar.update(cx, |bar, cx| {
                if bar.git_status != git_display {
                    bar.git_status = git_display;
                    cx.notify();
                }
            });
            if result.is_err() {
                break;
            }
        })
    }
}

/// Uniform slot height for the User-home compact site rows. Includes the
/// visible card (~56px) + 4px padding top/bottom, which reads as an 8px
/// gap between adjacent rows without breaking `uniform_list`'s
/// uniform-height contract. Any change here must also update the
/// `others_count * SITE_ROW_H` calc in `render_user_home`.
const SITE_ROW_H: f32 = 64.0;

/// Uniform slot for User-mode request rows. The inner row occupies 38px and
/// the remaining 4px preserves the existing visual gap while allowing GPUI
/// to render only the visible range.
const USER_REQUEST_ROW_H: f32 = 42.0;

/// Lucide slug for a Manage area key. Kept in one place so the User-home
/// site cards and any future palette entries share the same visual vocab.
/// Return `None` for area keys we ship with no dedicated icon — the chip
/// then renders label-only.
fn manage_area_icon(key: &str) -> Option<&'static str> {
    Some(match key {
        "dashboard" => "activity",
        "cms" => "scroll-text",
        "helpdesk" => "mail",
        "ecommerce" => "box",
        "settings" => "settings",
        "shelldeck" => "terminal",
        _ => return None,
    })
}

/// Parse a `#rrggbb` (or `rrggbb`) string into an opaque `Hsla`. Returns
/// `None` on any malformed input — the site card falls back to the neutral
/// border colour in that case.
fn parse_brand_hex(hex: &Option<String>) -> Option<Hsla> {
    let raw = hex.as_ref()?.trim();
    let raw = raw.trim_start_matches('#');
    if raw.len() != 6 || !raw.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let r = u8::from_str_radix(&raw[0..2], 16).ok()?;
    let g = u8::from_str_radix(&raw[2..4], 16).ok()?;
    let b = u8::from_str_radix(&raw[4..6], 16).ok()?;
    Some(Hsla::from(rgba(
        (r as u32) << 24 | (g as u32) << 16 | (b as u32) << 8 | 0xFF,
    )))
}

fn resize_edge(pos: Point<Pixels>, border: Pixels, size: Size<Pixels>) -> Option<ResizeEdge> {
    if pos.y < border && pos.x < border {
        Some(ResizeEdge::TopLeft)
    } else if pos.y < border && pos.x > size.width - border {
        Some(ResizeEdge::TopRight)
    } else if pos.y < border {
        Some(ResizeEdge::Top)
    } else if pos.y > size.height - border && pos.x < border {
        Some(ResizeEdge::BottomLeft)
    } else if pos.y > size.height - border && pos.x > size.width - border {
        Some(ResizeEdge::BottomRight)
    } else if pos.y > size.height - border {
        Some(ResizeEdge::Bottom)
    } else if pos.x < border {
        Some(ResizeEdge::Left)
    } else if pos.x > size.width - border {
        Some(ResizeEdge::Right)
    } else {
        None
    }
}

impl Workspace {
    /// Render the custom window titlebar with drag area and window controls.
    #[allow(clippy::too_many_arguments)]
    fn render_titlebar(
        is_maximized: bool,
        theme_menu_open: bool,
        account_menu_open: bool,
        account: Option<AccountInfo>,
        account_status: AccountStatus,
        site_menu_open: bool,
        active_site_label: Option<String>,
        sites_loaded: bool,
        mode_switch: Option<AppMode>,
        ui_font_size: f32,
        ai_configured: bool,
        ai_task_count: usize,
        handle: &WeakEntity<Self>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let titlebar_bg = ShellDeckColors::bg_sidebar();
        let titlebar_border = ShellDeckColors::border();
        let title_color = ShellDeckColors::text_primary();
        let title_dim = ShellDeckColors::text_muted();
        let accent = ShellDeckColors::primary();
        let btn_text = ShellDeckColors::text_muted();
        let btn_hover_bg = ShellDeckColors::hover_bg();

        // Title area — draggable
        let title_area = div()
            .flex_1()
            .h_full()
            .flex()
            .items_center()
            .px(px(10.0))
            .gap(px(8.0))
            .window_control_area(WindowControlArea::Drag)
            .on_mouse_down(MouseButton::Left, |_e, window, _cx| {
                window.start_window_move();
            })
            .child(crate::brand::brand_badge(20.0))
            .child(crate::brand::brand_wordmark(12.0))
            .child(
                // Version pill
                div()
                    .px(px(6.0))
                    .py(px(1.0))
                    .rounded(px(4.0))
                    .bg(ShellDeckColors::badge_bg())
                    .text_color(title_dim)
                    .text_size(px(10.0))
                    .font_weight(FontWeight::MEDIUM)
                    .child(format!("v{}", shelldeck_core::VERSION)),
            );

        // A window-control button with a rounded hover affordance and an SVG
        // glyph. `icon_path` points at an embedded asset (see main.rs Assets).
        //
        // GPUI's `svg()` element paints with its OWN `style.text.color` — it
        // does not inherit from the parent — so we set it explicitly on the
        // SVG and swap it on group hover to whiten the icon over the red
        // close background.
        let control_btn =
            |id: &'static str, icon_path: &'static str, area: WindowControlArea, danger: bool| {
                let hover_bg = if danger {
                    ShellDeckColors::error()
                } else {
                    btn_hover_bg
                };
                let group_name = SharedString::from(format!("ctrl-{id}"));
                div()
                    .id(id)
                    .group(group_name.clone())
                    .flex()
                    .items_center()
                    .justify_center()
                    .size(px(28.0))
                    .rounded(px(6.0))
                    .hover(|s| s.bg(hover_bg))
                    .window_control_area(area)
                    .child(
                        svg()
                            .path(icon_path)
                            .size(px(12.0))
                            .text_color(btn_text)
                            .group_hover(group_name, |s| s.text_color(gpui::white())),
                    )
            };

        let minimize_btn = control_btn(
            "titlebar-minimize",
            "images/minimize.svg",
            WindowControlArea::Min,
            false,
        )
        .on_click(cx.listener(|_this, _event: &ClickEvent, window, _cx| {
            window.minimize_window();
        }));

        let maximize_icon = if is_maximized {
            "images/restore.svg"
        } else {
            "images/maximize.svg"
        };
        let maximize_btn = control_btn(
            "titlebar-maximize",
            maximize_icon,
            WindowControlArea::Max,
            false,
        )
        .on_click(cx.listener(|_this, _event: &ClickEvent, window, _cx| {
            window.zoom_window();
        }));

        let h_quit = handle.clone();
        let close_btn = control_btn(
            "titlebar-close",
            "images/close.svg",
            WindowControlArea::Close,
            true,
        )
        .on_click(
            move |_event: &ClickEvent, window: &mut Window, cx: &mut App| {
                if let Some(ws) = h_quit.upgrade() {
                    if ws.read(cx).should_hide_to_tray() {
                        window.hide_window();
                        return;
                    }
                    let should_close = ws.update(cx, |ws, cx| ws.confirm_window_close(cx));
                    if should_close {
                        ws.update(cx, |ws, cx| ws.shutdown(cx));
                        cx.quit();
                    }
                }
            },
        );

        // Theme switcher — a 2x2 palette swatch that reflects the active theme
        // and toggles the dropdown menu.
        let mut theme_btn = div()
            .id("titlebar-theme")
            .flex()
            .items_center()
            .justify_center()
            .size(px(28.0))
            .rounded(px(6.0))
            .cursor_pointer()
            .hover(|s| s.bg(btn_hover_bg))
            .child(
                div()
                    .size(px(14.0))
                    .rounded(px(4.0))
                    .overflow_hidden()
                    .flex()
                    .flex_wrap()
                    .child(div().size(px(7.0)).bg(ShellDeckColors::primary()))
                    .child(div().size(px(7.0)).bg(ShellDeckColors::success()))
                    .child(div().size(px(7.0)).bg(ShellDeckColors::warning()))
                    .child(div().size(px(7.0)).bg(ShellDeckColors::error())),
            )
            .on_click(cx.listener(|this, _event: &ClickEvent, _window, cx| {
                this.theme_menu_open = !this.theme_menu_open;
                cx.notify();
            }));
        if theme_menu_open {
            theme_btn = theme_btn.bg(ShellDeckColors::hover_bg());
        }

        // Account chip — "Se connecter" when logged out, otherwise an
        // avatar-initial + name with a health status dot. Toggles the account
        // dropdown.
        let mut account_btn = div()
            .id("titlebar-account")
            .flex()
            .items_center()
            .gap(px(6.0))
            .h(px(28.0))
            .px(px(7.0))
            .rounded(px(6.0))
            .cursor_pointer()
            .hover(|s| s.bg(btn_hover_bg));

        if let Some(acct) = &account {
            let dot = account_status.dot_color();
            account_btn = account_btn
                .child(
                    div()
                        .relative()
                        .child(
                            div()
                                .size(px(18.0))
                                .rounded_full()
                                .bg(accent.opacity(0.20))
                                .flex()
                                .items_center()
                                .justify_center()
                                .text_size(px(10.0))
                                .font_weight(FontWeight::BOLD)
                                .text_color(accent)
                                .child(acct.initial()),
                        )
                        .child(
                            div()
                                .absolute()
                                .bottom(px(-1.0))
                                .right(px(-1.0))
                                .size(px(7.0))
                                .rounded_full()
                                .bg(dot)
                                .border_1()
                                .border_color(titlebar_bg),
                        ),
                )
                .child(
                    div()
                        .max_w(px(96.0))
                        .overflow_hidden()
                        .text_size(px(11.0))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(title_color)
                        .child(acct.display_name()),
                );
        } else {
            account_btn = account_btn
                .child(
                    div()
                        .size(px(18.0))
                        .rounded_full()
                        .bg(ShellDeckColors::badge_bg())
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_size(px(10.0))
                        .text_color(title_dim)
                        .child("\u{25CB}"), // ○ placeholder avatar
                )
                .child(
                    div()
                        .text_size(px(11.0))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(title_dim)
                        .child(crate::t!("account.sign_in").to_string()),
                );
        }

        account_btn =
            account_btn.on_click(cx.listener(|this, _event: &ClickEvent, _window, cx| {
                this.account_menu_open = !this.account_menu_open;
                if this.account_menu_open {
                    this.theme_menu_open = false;
                }
                cx.notify();
            }));
        if account_menu_open {
            account_btn = account_btn.bg(ShellDeckColors::hover_bg());
        }

        // Mode switcher — a three-segment control, super-admins only.
        let mode_switcher = mode_switch.map(|current| {
            let mut seg = div()
                .flex()
                .items_center()
                .gap(px(1.0))
                .p(px(2.0))
                .rounded(px(6.0))
                .bg(ShellDeckColors::badge_bg());
            for m in AppMode::all() {
                let active = m == current;
                let mut btn = div()
                    .id(ElementId::from(SharedString::from(format!(
                        "titlebar-mode-{}",
                        m.label()
                    ))))
                    .px(px(8.0))
                    .py(px(3.0))
                    .rounded(px(5.0))
                    .text_size(px(11.0))
                    .font_weight(FontWeight::MEDIUM)
                    .cursor_pointer()
                    .child(m.label().to_string())
                    .on_click(cx.listener(move |this, _event: &ClickEvent, _window, cx| {
                        this.set_mode(m, cx);
                    }));
                if active {
                    btn = btn
                        .bg(ShellDeckColors::bg_surface())
                        .text_color(ShellDeckColors::text_primary());
                } else {
                    btn = btn
                        .text_color(title_dim)
                        .hover(|s| s.text_color(title_color));
                }
                seg = seg.child(btn);
            }
            seg
        });

        // Site chip — shown only when signed in and the sites directory has
        // loaded. Displays the active site label or "Tous les sites".
        let show_site_chip = account.is_some() && sites_loaded;
        let site_chip = if show_site_chip {
            let label = active_site_label.unwrap_or_else(|| "Tous les sites".to_string());
            let mut chip = div()
                .id("titlebar-site")
                .flex()
                .items_center()
                .gap(px(5.0))
                .h(px(28.0))
                .px(px(8.0))
                .rounded(px(6.0))
                .cursor_pointer()
                .hover(|s| s.bg(btn_hover_bg))
                .child(
                    div()
                        .text_size(px(11.0))
                        .text_color(title_dim)
                        .child("\u{25C9}"), // ◉ site glyph
                )
                .child(
                    div()
                        .max_w(px(120.0))
                        .overflow_hidden()
                        .text_size(px(11.0))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(title_color)
                        .child(label),
                )
                .child(
                    svg()
                        .path("images/chevron-down.svg")
                        .size(px(9.0))
                        .text_color(title_dim),
                )
                .on_click(cx.listener(|this, _event: &ClickEvent, _window, cx| {
                    this.site_menu_open = !this.site_menu_open;
                    if this.site_menu_open {
                        this.theme_menu_open = false;
                        this.account_menu_open = false;
                    }
                    cx.notify();
                }));
            if site_menu_open {
                chip = chip.bg(ShellDeckColors::hover_bg());
            }
            Some(chip)
        } else {
            None
        };

        // UI scale controls — a compact −/value/+ group that adjusts the app
        // font size (which drives proportional UI scaling) live.
        let scale_btn = |id: &'static str, icon_path: &'static str| {
            let group_name = SharedString::from(format!("scale-{id}"));
            div()
                .id(id)
                .group(group_name.clone())
                .flex()
                .items_center()
                .justify_center()
                .size(px(22.0))
                .rounded(px(5.0))
                .cursor_pointer()
                .hover(|s| s.bg(btn_hover_bg))
                .child(
                    svg()
                        .path(icon_path)
                        .size(px(11.0))
                        .text_color(btn_text)
                        .group_hover(group_name, |s| {
                            s.text_color(ShellDeckColors::text_primary())
                        }),
                )
        };
        let dec_btn = scale_btn("titlebar-scale-down", "images/minus.svg").on_click(cx.listener(
            |this, _event: &ClickEvent, _window, cx| {
                this.settings
                    .update(cx, |settings, cx| settings.adjust_ui_font_size(-1.0, cx));
                cx.notify();
            },
        ));
        let inc_btn = scale_btn("titlebar-scale-up", "images/plus.svg").on_click(cx.listener(
            |this, _event: &ClickEvent, _window, cx| {
                this.settings
                    .update(cx, |settings, cx| settings.adjust_ui_font_size(1.0, cx));
                cx.notify();
            },
        ));
        let scale_group = div()
            .flex()
            .items_center()
            .gap(px(1.0))
            .child(dec_btn)
            .child(
                div()
                    .min_w(px(30.0))
                    .flex()
                    .justify_center()
                    .text_size(px(11.0))
                    .text_color(title_dim)
                    .child(format!("{}px", ui_font_size as i32)),
            )
            .child(inc_btn);

        let ai_button = ai_configured.then(|| {
            let tooltip: SharedString = t!("ai.assistant.open").to_string().into();
            let workspace = handle.clone();
            div()
                .id("titlebar-ai")
                .flex()
                .items_center()
                .justify_center()
                .h(px(28.0))
                .w(if ai_task_count == 0 {
                    px(28.0)
                } else {
                    px(44.0)
                })
                .gap(px(4.0))
                .rounded(px(6.0))
                .border_1()
                .border_color(ShellDeckColors::primary().opacity(0.40))
                .bg(ShellDeckColors::primary().opacity(0.12))
                .cursor_pointer()
                .hover(|el| el.bg(ShellDeckColors::primary().opacity(0.22)))
                .tooltip(move |_, cx| {
                    cx.new(|_| WorkspaceTooltip {
                        label: tooltip.clone(),
                    })
                    .into()
                })
                .on_click(move |_, _, cx| {
                    if let Some(workspace) = workspace.upgrade() {
                        workspace.update(cx, |this, cx| this.open_ai_assistant(cx));
                    }
                })
                .child(
                    svg()
                        .path(lucide_path("sparkles"))
                        .size(px(14.0))
                        .text_color(ShellDeckColors::primary()),
                )
                .when(ai_task_count > 0, |button| {
                    button.child(
                        div()
                            .flex()
                            .items_center()
                            .justify_center()
                            .min_w(px(14.0))
                            .h(px(14.0))
                            .px(px(3.0))
                            .rounded_full()
                            .bg(ShellDeckColors::primary())
                            .text_size(px(9.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(white())
                            .child(if ai_task_count > 99 {
                                "99+".to_string()
                            } else {
                                ai_task_count.to_string()
                            }),
                    )
                })
        });

        // Subtle vertical divider between the chrome control clusters.
        let divider = || div().w(px(1.0)).h(px(16.0)).mx(px(4.0)).bg(titlebar_border);

        div()
            .flex()
            .items_center()
            .w_full()
            .flex_shrink_0()
            .h(px(40.0))
            .bg(titlebar_bg)
            .border_b_1()
            .border_color(titlebar_border)
            .child(title_area)
            .child(
                div()
                    .flex()
                    .items_center()
                    .h_full()
                    .gap(px(4.0))
                    .pr(px(8.0))
                    .child(scale_group)
                    .children(ai_button)
                    .child(divider())
                    .child(account_btn)
                    .children(mode_switcher)
                    .children(site_chip)
                    .child(theme_btn)
                    .child(divider())
                    .child(minimize_btn)
                    .child(maximize_btn)
                    .child(close_btn),
            )
    }

    /// Render the titlebar theme-switcher dropdown: a full-window backdrop that
    /// dismisses on click, plus an anchored panel listing every app theme.
    fn render_theme_menu(&self, cx: &mut Context<Self>) -> impl IntoElement {
        use shelldeck_core::config::app_config::ThemePreference;

        let current = self.app_config.theme.clone();

        let mut panel = div()
            .id("theme-menu-panel")
            .absolute()
            .top(px(46.0))
            .right(px(12.0))
            .w(px(212.0))
            .max_h(px(440.0))
            .overflow_y_scroll()
            .bg(ShellDeckColors::bg_surface())
            .border_1()
            .border_color(ShellDeckColors::border())
            .rounded(px(10.0))
            .shadow(
                vec![BoxShadow {
                    color: hsla(0.0, 0.0, 0.0, 0.45),
                    offset: point(px(0.0), px(4.0)),
                    blur_radius: px(20.0),
                    spread_radius: px(0.0),
                    inset: false,
                }]
                .into(),
            )
            .p(px(4.0))
            .flex()
            .flex_col()
            .gap(px(1.0))
            // Clicks inside the panel must not bubble to the dismiss backdrop.
            .on_mouse_down(MouseButton::Left, |_e, _window, cx: &mut App| {
                cx.stop_propagation();
            });

        for pref in ThemePreference::all() {
            let pref = pref.clone();
            let is_active = current == pref;
            let p = crate::theme::palette_for(&pref);
            let label = pref.display_name().to_string();

            let mut item = div()
                .id(ElementId::from(SharedString::from(format!(
                    "theme-menu-{label}"
                ))))
                .flex()
                .items_center()
                .gap(px(8.0))
                .px(px(8.0))
                .py(px(5.0))
                .rounded(px(6.0))
                .cursor_pointer()
                .hover(|s| s.bg(ShellDeckColors::hover_bg()))
                // A mini swatch showing the theme's background + accent.
                .child(
                    div()
                        .size(px(16.0))
                        .rounded(px(4.0))
                        .bg(p.bg_primary)
                        .border_1()
                        .border_color(p.border)
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(div().size(px(8.0)).rounded(px(2.0)).bg(p.primary)),
                )
                .child(
                    div()
                        .flex_1()
                        .text_size(px(12.0))
                        .text_color(if is_active {
                            ShellDeckColors::primary()
                        } else {
                            ShellDeckColors::text_primary()
                        })
                        .font_weight(if is_active {
                            FontWeight::SEMIBOLD
                        } else {
                            FontWeight::NORMAL
                        })
                        .child(label),
                );

            if is_active {
                item = item.child(
                    div()
                        .text_size(px(12.0))
                        .text_color(ShellDeckColors::primary())
                        .child("\u{2713}"),
                );
            }

            item = item.on_click(cx.listener(move |this, _event: &ClickEvent, _window, cx| {
                let pref = pref.clone();
                this.settings.update(cx, |settings, cx| {
                    settings.select_app_theme(pref, cx);
                });
                this.theme_menu_open = false;
                cx.notify();
            }));

            panel = panel.child(item);
        }

        // Transparent full-window backdrop — a click anywhere outside the panel
        // closes the menu.
        div()
            .id("theme-menu-backdrop")
            .occlude()
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _e, _window, cx| {
                    this.theme_menu_open = false;
                    cx.notify();
                }),
            )
            .child(panel)
    }

    /// Render the titlebar account dropdown: a dismiss backdrop plus an anchored
    /// panel. Logged out shows the sign-in options (password modal + OIDC);
    /// logged in shows the account, sync, and sign-out controls.
    fn render_account_menu(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let shadow = vec![BoxShadow {
            color: hsla(0.0, 0.0, 0.0, 0.45),
            offset: point(px(0.0), px(4.0)),
            blur_radius: px(20.0),
            spread_radius: px(0.0),
            inset: false,
        }];

        let mut panel = div()
            .id("account-menu-panel")
            .absolute()
            .top(px(46.0))
            .right(px(12.0))
            .w(px(288.0))
            .bg(ShellDeckColors::bg_surface())
            .border_1()
            .border_color(ShellDeckColors::border())
            .rounded(px(10.0))
            .shadow(shadow.into())
            .p(px(12.0))
            .flex()
            .flex_col()
            .gap(px(8.0))
            // Clicks inside must not bubble to the dismiss backdrop.
            .on_mouse_down(MouseButton::Left, |_e, _window, cx: &mut App| {
                cx.stop_propagation();
            });

        // A full-width secondary (outlined) menu button.
        let secondary_btn = |id: &'static str, label: String| {
            div()
                .id(id)
                .w_full()
                .px(px(10.0))
                .py(px(8.0))
                .rounded(px(6.0))
                .border_1()
                .border_color(ShellDeckColors::border())
                .bg(ShellDeckColors::bg_primary())
                .text_size(px(13.0))
                .text_color(ShellDeckColors::text_primary())
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .hover(|s| s.bg(ShellDeckColors::hover_bg()))
                .child(label)
        };

        if let Some(acct) = self.app_config.account.clone() {
            // --- LOGGED IN ---
            panel = panel.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.0))
                    .pb(px(8.0))
                    .border_b_1()
                    .border_color(ShellDeckColors::border())
                    .child(
                        div()
                            .size(px(34.0))
                            .rounded_full()
                            .bg(ShellDeckColors::primary().opacity(0.20))
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_size(px(15.0))
                            .font_weight(FontWeight::BOLD)
                            .text_color(ShellDeckColors::primary())
                            .child(acct.initial()),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .overflow_hidden()
                            .child(
                                div()
                                    .text_size(px(13.0))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(ShellDeckColors::text_primary())
                                    .child(acct.display_name()),
                            )
                            .child(
                                div()
                                    .text_size(px(11.0))
                                    .text_color(ShellDeckColors::text_muted())
                                    .child(acct.email.clone()),
                            ),
                    ),
            );

            let status_label = match self.account_status {
                AccountStatus::Ok => "Connecté",
                AccountStatus::Rejected => "Session expirée — reconnectez-vous",
                AccountStatus::Offline => "Hors ligne",
                AccountStatus::Unknown => "Vérification…",
            };
            let info_row = |label: String, value: String| {
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(px(8.0))
                    .child(
                        div()
                            .text_size(px(11.0))
                            .text_color(ShellDeckColors::text_muted())
                            .child(label),
                    )
                    .child(
                        div()
                            .max_w(px(180.0))
                            .overflow_hidden()
                            .text_size(px(11.0))
                            .text_color(ShellDeckColors::text_primary())
                            .child(value),
                    )
            };
            panel = panel
                .child(info_row("Serveur".to_string(), self.account_base_url()))
                .child(info_row(
                    "Appareil".to_string(),
                    cloud_account::device_name(),
                ))
                .child(info_row(
                    t!("user.sites.active").to_string(),
                    self.app_config
                        .cloud_sync
                        .active_site_label
                        .clone()
                        .unwrap_or_else(|| "Tous les sites".to_string()),
                ))
                .child(info_row(
                    t!("settings.cloud_sync.status.label").to_string(),
                    status_label.to_string(),
                ));

            panel = panel.child(
                secondary_btn("account-sync", t!("user.sync").to_string()).on_click(cx.listener(
                    |this, _: &ClickEvent, _, cx| {
                        this.account_menu_open = false;
                        this.cloud_sync_now(cx);
                    },
                )),
            );
            panel = panel.child(
                secondary_btn("account-logout", t!("user.account.logout").to_string())
                    .text_color(ShellDeckColors::error())
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.logout_account(cx);
                    })),
            );
        } else {
            // --- LOGGED OUT ---
            panel = panel
                .child(
                    div()
                        .text_size(px(13.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(ShellDeckColors::text_primary())
                        .child(t!("user.account.title").to_string()),
                )
                .child(
                    div()
                        .text_size(px(11.0))
                        .text_color(ShellDeckColors::text_muted())
                        .child(t!("user.account.hint").to_string()),
                );

            // Primary: open the password + OIDC login modal.
            panel = panel.child(
                div()
                    .id("account-signin")
                    .w_full()
                    .px(px(10.0))
                    .py(px(9.0))
                    .rounded(px(6.0))
                    .bg(ShellDeckColors::primary())
                    .text_size(px(13.0))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(white())
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .child(crate::t!("account.sign_in").to_string())
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.show_login_form(cx);
                    })),
            );

            // Divider.
            panel = panel.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(div().flex_1().h(px(1.0)).bg(ShellDeckColors::border()))
                    .child(
                        div()
                            .text_size(px(10.0))
                            .text_color(ShellDeckColors::text_muted())
                            .child(t!("user.account.or_one_click").to_string()),
                    )
                    .child(div().flex_1().h(px(1.0)).bg(ShellDeckColors::border())),
            );

            panel = panel
                .child(
                    secondary_btn("account-oidc-sso", t!("login.oidc_sso").to_string()).on_click(
                        cx.listener(|this, _: &ClickEvent, _, cx| {
                            this.start_oidc_login(Some("sso".to_string()), cx);
                        }),
                    ),
                )
                .child(
                    div()
                        .flex()
                        .gap(px(8.0))
                        .child(div().flex_1().child(
                            secondary_btn("account-oidc-google", "Google".to_string()).on_click(
                                cx.listener(|this, _: &ClickEvent, _, cx| {
                                    this.start_oidc_login(Some("google".to_string()), cx);
                                }),
                            ),
                        ))
                        .child(div().flex_1().child(
                            secondary_btn("account-oidc-github", "GitHub".to_string()).on_click(
                                cx.listener(|this, _: &ClickEvent, _, cx| {
                                    this.start_oidc_login(Some("github".to_string()), cx);
                                }),
                            ),
                        )),
                );
        }

        // Dismiss backdrop.
        div()
            .id("account-menu-backdrop")
            .occlude()
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _e, _window, cx| {
                    this.account_menu_open = false;
                    cx.notify();
                }),
            )
            .child(panel)
    }

    /// Render the titlebar site-switcher dropdown: "Tous les sites" + the site
    /// list (active pinned, connection-bearing next, capped) + "Ouvrir dans
    /// Manage" area links for the active site.
    fn render_site_menu(&self, cx: &mut Context<Self>) -> impl IntoElement {
        const CAP: usize = 20;
        let payload = self.site_directory.clone().unwrap_or_default();
        let active_id = self.app_config.cloud_sync.active_site_id.clone();

        // Which sites have at least one synced connection.
        let conn_site_ids: std::collections::HashSet<String> = self
            .connections
            .iter()
            .filter_map(|c| c.site_id.map(|id| id.to_string()))
            .collect();

        // Sort: active first, then connection-bearing, then alphabetical.
        let mut sites: Vec<&ManagedSiteInfo> = payload.sites.iter().collect();
        sites.sort_by(|a, b| {
            let a_active = active_id.as_deref() == Some(a.site_id.as_str());
            let b_active = active_id.as_deref() == Some(b.site_id.as_str());
            let a_conn = conn_site_ids.contains(&a.site_id);
            let b_conn = conn_site_ids.contains(&b.site_id);
            b_active.cmp(&a_active).then(b_conn.cmp(&a_conn)).then(
                a.display_label()
                    .to_lowercase()
                    .cmp(&b.display_label().to_lowercase()),
            )
        });
        let total = sites.len();
        let hidden = total.saturating_sub(CAP);

        let row =
            |id: ElementId, label: String, active: bool, badge: Option<String>| -> Stateful<Div> {
                let mut r = div()
                    .id(id)
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .px(px(8.0))
                    .py(px(6.0))
                    .rounded(px(6.0))
                    .cursor_pointer()
                    .hover(|s| s.bg(ShellDeckColors::hover_bg()));
                if active {
                    r = r.bg(ShellDeckColors::selected_bg());
                }
                r = r.child(
                    div()
                        .flex_1()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_size(px(12.0))
                        .text_color(ShellDeckColors::text_primary())
                        .child(label),
                );
                if let Some(b) = badge {
                    r = r.child(
                        div()
                            .flex_shrink_0()
                            .px(px(5.0))
                            .py(px(1.0))
                            .rounded(px(8.0))
                            .bg(ShellDeckColors::badge_bg())
                            .text_size(px(10.0))
                            .text_color(ShellDeckColors::text_muted())
                            .child(b),
                    );
                }
                if active {
                    r = r.child(
                        div()
                            .flex_shrink_0()
                            .text_size(px(12.0))
                            .text_color(ShellDeckColors::primary())
                            .child("\u{2713}"),
                    );
                }
                r
            };

        let shadow = vec![BoxShadow {
            color: hsla(0.0, 0.0, 0.0, 0.45),
            offset: point(px(0.0), px(4.0)),
            blur_radius: px(20.0),
            spread_radius: px(0.0),
            inset: false,
        }];

        let mut panel = div()
            .id("site-menu-panel")
            .absolute()
            .top(px(46.0))
            .right(px(12.0))
            .w(px(300.0))
            .max_h(px(480.0))
            .overflow_y_scroll()
            .bg(ShellDeckColors::bg_surface())
            .border_1()
            .border_color(ShellDeckColors::border())
            .rounded(px(10.0))
            .shadow(shadow.into())
            .p(px(6.0))
            .flex()
            .flex_col()
            .gap(px(1.0))
            .on_mouse_down(MouseButton::Left, |_e, _window, cx: &mut App| {
                cx.stop_propagation();
            });

        panel = panel.child(Self::render_site_section_header(&format!(
            "SITES ({})",
            total
        )));

        // "Tous les sites" (clear the filter).
        panel = panel.child(
            row(
                ElementId::from(SharedString::from("site-all")),
                "Tous les sites".to_string(),
                active_id.is_none(),
                None,
            )
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.select_site(None, None, cx);
            })),
        );

        for site in sites.iter().take(CAP) {
            let sid = site.site_id.clone();
            let label = site.display_label();
            let is_active = active_id.as_deref() == Some(sid.as_str());
            let badge = if conn_site_ids.contains(&sid) {
                Some("connexions".to_string())
            } else {
                None
            };
            let elem_id = ElementId::from(SharedString::from(format!("site-{}", sid)));
            let sid_for_click = sid.clone();
            let label_for_click = label.clone();
            panel = panel.child(row(elem_id, label, is_active, badge).on_click(cx.listener(
                move |this, _: &ClickEvent, _, cx| {
                    this.select_site(
                        Some(sid_for_click.clone()),
                        Some(label_for_click.clone()),
                        cx,
                    );
                },
            )));
        }

        if hidden > 0 {
            panel = panel.child(
                div()
                    .px(px(8.0))
                    .py(px(6.0))
                    .text_size(px(11.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(format!(
                        "+{} autres sites (les sites avec connexions sont priorisés)",
                        hidden
                    )),
            );
        }

        // "Ouvrir dans Manage" — area links for the active site.
        if let Some(active_site) = self.active_site_info() {
            if !payload.areas.is_empty() {
                panel = panel.child(Self::render_site_section_header(&format!(
                    "OUVRIR DANS MANAGE — {}",
                    active_site.display_label()
                )));
                for area in &payload.areas {
                    let path = area.path.clone();
                    panel = panel.child(
                        div()
                            .id(ElementId::from(SharedString::from(format!(
                                "area-{}",
                                area.key
                            ))))
                            .flex()
                            .items_center()
                            .gap(px(8.0))
                            .px(px(8.0))
                            .py(px(6.0))
                            .rounded(px(6.0))
                            .cursor_pointer()
                            .hover(|s| s.bg(ShellDeckColors::hover_bg()))
                            .child(
                                div()
                                    .flex_1()
                                    .overflow_hidden()
                                    .whitespace_nowrap()
                                    .text_size(px(12.0))
                                    .text_color(ShellDeckColors::text_primary())
                                    .child(area.label.clone()),
                            )
                            .child(
                                div()
                                    .flex_shrink_0()
                                    .text_size(px(11.0))
                                    .text_color(ShellDeckColors::text_muted())
                                    .child("\u{2197}"), // ↗
                            )
                            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                this.open_manage_area(path.clone(), cx);
                            })),
                    );
                }
            }
        }

        // Dismiss backdrop.
        div()
            .id("site-menu-backdrop")
            .occlude()
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _e, _window, cx| {
                    this.site_menu_open = false;
                    cx.notify();
                }),
            )
            .child(panel)
    }

    /// Render the sidebar kebab (⋮) row-action menu: a backdrop that dismisses
    /// on click plus an anchored panel with SSH / Edit / bext / Delete for the
    /// clicked connection. Positioned at the kebab's window-relative click
    /// coordinates.
    fn render_sidebar_kebab_menu(
        &self,
        conn_id: Uuid,
        pos: Point<Pixels>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let conn_name = self
            .connections
            .iter()
            .find(|c| c.id == conn_id)
            .map(|c| c.display_name().to_string())
            .unwrap_or_else(|| "Connection".to_string());

        let shadow = vec![BoxShadow {
            color: hsla(0.0, 0.0, 0.0, 0.35),
            offset: point(px(0.0), px(4.0)),
            blur_radius: px(16.0),
            spread_radius: px(0.0),
            inset: false,
        }];

        // Header (connection name) — reminds the user which row is targeted.
        let header = div()
            .px(px(10.0))
            .py(px(6.0))
            .text_size(px(11.0))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(ShellDeckColors::text_muted())
            .overflow_hidden()
            .whitespace_nowrap()
            .child(conn_name);

        #[allow(clippy::type_complexity)]
        // local closure param; a type alias would need Self, disallowed here
        let item = |id: &'static str,
                    label: &'static str,
                    accent: gpui::Hsla,
                    danger: bool,
                    on_click: Box<dyn Fn(&mut Self, &mut Context<Self>)>|
         -> gpui::Stateful<Div> {
            let hover_bg = if danger {
                ShellDeckColors::error().opacity(0.12)
            } else {
                accent.opacity(0.12)
            };
            let hover_text = if danger {
                ShellDeckColors::error()
            } else {
                accent
            };
            div()
                .id(ElementId::from(SharedString::from(format!(
                    "kebab-item-{id}-{conn_id}"
                ))))
                .flex()
                .items_center()
                .gap(px(8.0))
                .px(px(10.0))
                .py(px(6.0))
                .rounded(px(5.0))
                .text_size(px(12.0))
                .text_color(if danger {
                    ShellDeckColors::error()
                } else {
                    ShellDeckColors::text_primary()
                })
                .cursor_pointer()
                .hover(move |el| el.bg(hover_bg).text_color(hover_text))
                .child(label)
                .on_click(cx.listener(move |this, _event: &ClickEvent, _window, cx| {
                    cx.stop_propagation();
                    this.sidebar_kebab_menu = None;
                    on_click(this, cx);
                }))
        };

        let panel = div()
            .id("sidebar-kebab-panel")
            .occlude()
            .w(px(200.0))
            .bg(ShellDeckColors::bg_surface())
            .border_1()
            .border_color(ShellDeckColors::border())
            .rounded(px(8.0))
            .shadow(shadow.into())
            .p(px(4.0))
            .flex()
            .flex_col()
            .gap(px(1.0))
            // Clicks inside the panel must not bubble to the dismiss backdrop.
            .on_mouse_down(MouseButton::Left, |_e, _window, cx: &mut App| {
                cx.stop_propagation();
            })
            .child(header)
            .child(div().h(px(1.0)).my(px(2.0)).bg(ShellDeckColors::border()))
            .child(item(
                "ssh",
                "Connect (SSH)",
                ShellDeckColors::success(),
                false,
                Box::new(move |this, cx| {
                    if let Some(conn) = this.connections.iter().find(|c| c.id == conn_id) {
                        let conn = conn.clone();
                        this.connect_ssh(conn, cx);
                    }
                    this.active_view = ActiveView::Terminal;
                    cx.notify();
                }),
            ))
            .child(item(
                "edit",
                "Edit…",
                ShellDeckColors::primary(),
                false,
                Box::new(move |this, cx| {
                    if let Some(conn) = this.connections.iter().find(|c| c.id == conn_id) {
                        let conn = conn.clone();
                        this.show_connection_form(Some(conn), cx);
                    }
                }),
            ))
            .child(item(
                "bext",
                "Manage bext…",
                ShellDeckColors::primary(),
                false,
                Box::new(move |this, cx| {
                    this.manage_bext_for_connection(conn_id, cx);
                }),
            ))
            .child(div().h(px(1.0)).my(px(2.0)).bg(ShellDeckColors::border()))
            .child(item(
                "del",
                "Delete",
                ShellDeckColors::error(),
                true,
                Box::new(move |this, cx| {
                    // Reuse the two-step confirm flow from the existing handler.
                    this.handle_sidebar_event(&SidebarEvent::ConnectionDelete(conn_id), cx);
                }),
            ));

        // Transparent full-window backdrop — click anywhere outside dismisses.
        // The panel itself is wrapped in `deferred(anchored())` with
        // `snap_to_window_with_margin` so it flips inside the viewport when
        // the click position would otherwise push the menu off-screen
        // (previously the bottom items got clipped by the status bar).
        div()
            .id("sidebar-kebab-backdrop")
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _e, _window, cx| {
                    this.sidebar_kebab_menu = None;
                    cx.notify();
                }),
            )
            .child(
                deferred(
                    anchored()
                        .position(pos + point(gpui::px(0.0), gpui::px(4.0)))
                        .anchor(gpui::Corner::TopLeft)
                        .snap_to_window_with_margin(gpui::px(8.0))
                        .child(panel),
                )
                .with_priority(2),
            )
    }

    fn render_site_section_header(label: &str) -> impl IntoElement {
        div()
            .px(px(8.0))
            .pt(px(8.0))
            .pb(px(4.0))
            .text_size(px(10.0))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(ShellDeckColors::text_muted())
            .child(label.to_string())
    }

    /// Open a manage area in the browser for a specific site (User-mode rows).
    fn open_area_for_site(
        &mut self,
        site: ManagedSiteInfo,
        area_path: String,
        cx: &mut Context<Self>,
    ) {
        let origin = self
            .site_directory
            .as_ref()
            .map(|p| p.manage_origin.clone())
            .filter(|o| !o.is_empty())
            .unwrap_or_else(|| self.account_base_url());
        let url = manage_sites::manage_area_url(&origin, &site, &area_path);
        match cloud_account::open_in_browser(&url) {
            Ok(_) => self.show_toast(
                t!("toast.opening_browser").to_string(),
                ToastLevel::Info,
                cx,
            ),
            Err(e) => self.show_toast(
                t!(
                    "toast.open_browser_failed",
                    error = cloud_account::user_message(&e)
                )
                .to_string(),
                ToastLevel::Error,
                cx,
            ),
        }
    }

    /// Split the site directory into `(active, others)` — the active site
    /// as a full "rich" card, everyone else as compact virtualised rows.
    /// Applies the live search query, then sorts (active pinned, then
    /// connection-bearing, then alpha) so the compact list has a stable
    /// order. The active site is only returned when it *also* passes the
    /// filter — a filter that hides the current active means the top card
    /// disappears (the sidebar filter itself stays untouched).
    fn partition_user_sites(
        &self,
        cx: &mut Context<Self>,
    ) -> (
        Option<manage_sites::ManagedSiteInfo>,
        Vec<manage_sites::ManagedSiteInfo>,
    ) {
        let payload = self.site_directory.clone().unwrap_or_default();
        let active_id = self.app_config.cloud_sync.active_site_id.clone();
        let conn_site_ids: std::collections::HashSet<String> = self
            .connections
            .iter()
            .filter_map(|c| c.site_id.map(|id| id.to_string()))
            .collect();
        let q = self
            .user_sites_search_state
            .read(cx)
            .content()
            .trim()
            .to_lowercase();
        let mut sites: Vec<manage_sites::ManagedSiteInfo> = payload
            .sites
            .iter()
            .filter(|s| {
                q.is_empty()
                    || s.display_label().to_lowercase().contains(&q)
                    || s.host.to_lowercase().contains(&q)
                    || s.tenant_name.to_lowercase().contains(&q)
            })
            .cloned()
            .collect();
        sites.sort_by(|a, b| {
            let a_conn = conn_site_ids.contains(&a.site_id);
            let b_conn = conn_site_ids.contains(&b.site_id);
            b_conn.cmp(&a_conn).then(
                a.display_label()
                    .to_lowercase()
                    .cmp(&b.display_label().to_lowercase()),
            )
        });
        let active = active_id
            .as_deref()
            .and_then(|id| sites.iter().position(|s| s.site_id == id))
            .map(|idx| sites.remove(idx));
        (active, sites)
    }

    /// Full "rich" site card — reserved for the currently-active site. This
    /// is the only place areas + wp-admin chip render (the compact rows keep
    /// paint budget low by omitting them). Extracted from the pre-virt loop
    /// verbatim; only the `is_active = true` branch stays here (the compact
    /// row handles inactive sites now).
    fn render_active_site_card(
        &self,
        site: &manage_sites::ManagedSiteInfo,
        area_buttons: &[manage_sites::ManageArea],
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let sid = site.site_id.clone();
        let label = site.display_label();
        let mut card = div()
            .flex()
            .flex_col()
            .gap(px(8.0))
            .p(px(12.0))
            .rounded(px(10.0))
            .border_1()
            .border_color(ShellDeckColors::primary())
            .bg(ShellDeckColors::bg_sidebar());

        // Row 1: identity + "Site actif" pill.
        card = card.child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap(px(10.0))
                .child({
                    let mut identity = div().flex().flex_col().min_w(px(0.0)).overflow_hidden();
                    let mut label_row = div().flex().items_center().gap(px(6.0)).child(
                        div()
                            .text_size(px(14.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(ShellDeckColors::text_primary())
                            .truncate()
                            .child(label.clone()),
                    );
                    if site.is_wordpress == Some(true) {
                        label_row = label_row.child(
                            div()
                                .px(px(5.0))
                                .py(px(1.0))
                                .rounded(px(4.0))
                                .bg(ShellDeckColors::primary().opacity(0.12))
                                .text_size(px(10.0))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(ShellDeckColors::primary())
                                .flex_shrink_0()
                                .child("WP"),
                        );
                    }
                    identity = identity.child(label_row).child(
                        div()
                            .text_size(px(11.0))
                            .text_color(ShellDeckColors::text_muted())
                            .child(if site.host.is_empty() {
                                site.tenant_name.clone()
                            } else {
                                site.host.clone()
                            }),
                    );
                    identity
                })
                .child(
                    div()
                        .px(px(10.0))
                        .py(px(5.0))
                        .rounded(px(6.0))
                        .text_size(px(12.0))
                        .font_weight(FontWeight::MEDIUM)
                        .flex_shrink_0()
                        .bg(ShellDeckColors::primary().opacity(0.15))
                        .text_color(ShellDeckColors::primary())
                        .child(t!("user.sites.active").to_string()),
                ),
        );

        // Row 2: wp-admin shortcut (if any) + area deep-links.
        let mut areas_row = div().flex().flex_wrap().gap(px(6.0));
        if let Some(wp_url) = site.wp_admin_url.as_ref().filter(|u| !u.is_empty()) {
            let wp_url_owned = wp_url.clone();
            areas_row = areas_row.child(
                div()
                    .id(ElementId::from(SharedString::from(format!(
                        "uh-wp-{}",
                        sid
                    ))))
                    .flex()
                    .items_center()
                    .gap(px(5.0))
                    .px(px(8.0))
                    .py(px(4.0))
                    .rounded(px(6.0))
                    .border_1()
                    .border_color(ShellDeckColors::primary().opacity(0.35))
                    .bg(ShellDeckColors::primary().opacity(0.08))
                    .text_size(px(11.0))
                    .text_color(ShellDeckColors::primary())
                    .cursor_pointer()
                    .hover(|s| s.bg(ShellDeckColors::primary().opacity(0.14)))
                    .child(lucide_icon(
                        "external-link",
                        11.0,
                        ShellDeckColors::primary(),
                    ))
                    .child("wp-admin")
                    .on_click(cx.listener(move |_this, _: &ClickEvent, _, _cx| {
                        let _ =
                            shelldeck_core::config::cloud_account::open_in_browser(&wp_url_owned);
                    })),
            );
        }
        for area in area_buttons {
            let site_clone = site.clone();
            let path = area.path.clone();
            let mut chip = div()
                .id(ElementId::from(SharedString::from(format!(
                    "uh-area-{}-{}",
                    sid, area.key
                ))))
                .flex()
                .items_center()
                .gap(px(5.0))
                .px(px(8.0))
                .py(px(4.0))
                .rounded(px(6.0))
                .border_1()
                .border_color(ShellDeckColors::border())
                .bg(ShellDeckColors::bg_primary())
                .text_size(px(11.0))
                .text_color(ShellDeckColors::text_muted())
                .cursor_pointer()
                .hover(|s| {
                    s.bg(ShellDeckColors::hover_bg())
                        .text_color(ShellDeckColors::text_primary())
                });
            if let Some(slug) = manage_area_icon(&area.key) {
                chip = chip.child(
                    svg()
                        .path(lucide_path(slug))
                        .size(px(11.0))
                        .text_color(ShellDeckColors::text_muted()),
                );
            }
            areas_row = areas_row.child(chip.child(area.label.clone()).on_click(cx.listener(
                move |this, _: &ClickEvent, _, cx| {
                    this.open_area_for_site(site_clone.clone(), path.clone(), cx);
                },
            )));
        }
        card.child(areas_row)
    }

    /// Fixed-height compact row for a non-active site. The full slot
    /// (`SITE_ROW_H = 64px`) contains an inner card that's ~56px tall with
    /// 4px padding top/bottom, giving an 8px visual gap between adjacent
    /// rows without breaking `uniform_list`'s uniform-height contract.
    /// Width fills the parent (`w_full`) so rows land on the same right
    /// edge as the active card above. Areas + wp-admin chip are dropped
    /// here on purpose — activation promotes the site to the top card.
    fn render_compact_site_row(
        &self,
        site: &manage_sites::ManagedSiteInfo,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let sid = site.site_id.clone();
        let label = site.display_label();
        let brand = parse_brand_hex(&site.brand_color);
        let border_color = brand
            .map(|c| c.opacity(0.45))
            .unwrap_or(ShellDeckColors::border());
        let sid_for_click = sid.clone();
        let label_for_click = label.clone();

        div().w_full().h(px(SITE_ROW_H)).py(px(4.0)).child(
            div()
                .w_full()
                .h_full()
                .flex()
                .items_center()
                .gap(px(10.0))
                .px(px(12.0))
                .rounded(px(10.0))
                .border_1()
                .border_color(border_color)
                .bg(ShellDeckColors::bg_sidebar())
                .child({
                    let mut identity = div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w(px(0.0))
                        .overflow_hidden();
                    let mut label_row = div().flex().items_center().gap(px(6.0)).child(
                        div()
                            .text_size(px(14.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(ShellDeckColors::text_primary())
                            .truncate()
                            .child(label.clone()),
                    );
                    if site.is_wordpress == Some(true) {
                        label_row = label_row.child(
                            div()
                                .px(px(5.0))
                                .py(px(1.0))
                                .rounded(px(4.0))
                                .bg(ShellDeckColors::primary().opacity(0.12))
                                .text_size(px(10.0))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(ShellDeckColors::primary())
                                .flex_shrink_0()
                                .child("WP"),
                        );
                    }
                    identity = identity.child(label_row).child(
                        div()
                            .text_size(px(11.0))
                            .text_color(ShellDeckColors::text_muted())
                            .truncate()
                            .child(if site.host.is_empty() {
                                site.tenant_name.clone()
                            } else {
                                site.host.clone()
                            }),
                    );
                    identity
                })
                .child(
                    div()
                        .id(ElementId::from(SharedString::from(format!(
                            "uh-act-{}",
                            sid
                        ))))
                        .px(px(10.0))
                        .py(px(5.0))
                        .rounded(px(6.0))
                        .text_size(px(12.0))
                        .font_weight(FontWeight::MEDIUM)
                        .flex_shrink_0()
                        .border_1()
                        .border_color(ShellDeckColors::border())
                        .bg(ShellDeckColors::bg_primary())
                        .text_color(ShellDeckColors::text_primary())
                        .cursor_pointer()
                        .hover(|s| s.bg(ShellDeckColors::hover_bg()))
                        .child(t!("user.sites.activate").to_string())
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.select_site(
                                Some(sid_for_click.clone()),
                                Some(label_for_click.clone()),
                                cx,
                            );
                        })),
                ),
        )
    }

    /// Tab bar for the User-mode home. Three tabs (Sites / Demandes /
    /// Infos), same visual shape as `SupportView::render_section_tabs`
    /// (compact_filter_button + icon, `Default` variant when active).
    fn render_user_home_tab_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let tab = |label: String,
                   icon: &'static str,
                   target: UserHomeTab,
                   this_tab: UserHomeTab,
                   cx: &mut Context<Self>| {
            let active = this_tab == target;
            let entity = cx.entity();
            adabraka_ui::components::button::Button::new(
                ElementId::from(SharedString::from(format!("uh-tab-{target:?}"))),
                label,
            )
            .size(adabraka_ui::components::button::ButtonSize::Sm)
            .h(gpui::px(26.0))
            .px(gpui::px(10.0))
            .variant(if active {
                ButtonVariant::Default
            } else {
                ButtonVariant::Outline
            })
            .icon(IconSource::from(icon))
            .on_click(move |_, _, cx| {
                entity.update(cx, |this, cx| {
                    this.user_home_tab = target;
                    cx.notify();
                });
            })
        };
        let current = self.user_home_tab;
        div()
            .flex()
            .items_center()
            .gap(px(6.0))
            .px(px(16.0))
            .pt(px(4.0))
            .pb(px(8.0))
            .border_b_1()
            .border_color(ShellDeckColors::border())
            .child(tab(
                t!("user.tabs.sites").to_string(),
                "globe",
                UserHomeTab::Sites,
                current,
                cx,
            ))
            .child(tab(
                t!("user.tabs.requests").to_string(),
                "tag",
                UserHomeTab::Requests,
                current,
                cx,
            ))
            .child(tab(
                t!("user.tabs.infos").to_string(),
                "user",
                UserHomeTab::Infos,
                current,
                cx,
            ))
    }

    /// User-mode "Mes informations" tab — surfaces every field the
    /// `/whoami` payload returned (device label, created_at, last_seen_at,
    /// role) plus the account bits and directory stats. Deliberately
    /// read-only so it can't accidentally mutate credentials.
    fn render_user_infos_tab(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let account = self.app_config.account.clone().unwrap_or_default();
        let server = self.account_base_url();
        let payload = self.site_directory.clone().unwrap_or_default();
        let whoami = self.last_whoami.clone().unwrap_or_default();

        // Small helper: one "field row" (label muted small, value primary
        // wrapping). Copies the shape of the ticket detail meta rows so
        // the visual language stays the same across surfaces.
        let field = |label: String, value: String, icon: &'static str| {
            div()
                .flex()
                .items_start()
                .gap(px(10.0))
                .py(px(8.0))
                .child(
                    div()
                        .size(px(28.0))
                        .rounded(px(6.0))
                        .bg(ShellDeckColors::primary().opacity(0.10))
                        .flex()
                        .items_center()
                        .justify_center()
                        .flex_shrink_0()
                        .child(lucide_icon(icon, 13.0, ShellDeckColors::primary())),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .min_w(px(0.0))
                        .flex_1()
                        .child(
                            div()
                                .text_size(px(11.0))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(ShellDeckColors::text_muted())
                                .child(label.to_uppercase()),
                        )
                        .child(
                            div()
                                .text_size(px(13.0))
                                .text_color(ShellDeckColors::text_primary())
                                .child(if value.trim().is_empty() {
                                    t!("user.infos.unknown").to_string()
                                } else {
                                    value
                                }),
                        ),
                )
        };

        // Section chrome — same p/rounded/border/bg as other User-mode cards.
        let section = |title: String, icon: &'static str, body: gpui::Div| {
            div()
                .flex()
                .flex_col()
                .m(px(16.0))
                .mb(px(0.0))
                .rounded(px(12.0))
                .border_1()
                .border_color(ShellDeckColors::border())
                .bg(ShellDeckColors::bg_sidebar())
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.0))
                        .px(px(16.0))
                        .py(px(12.0))
                        .border_b_1()
                        .border_color(ShellDeckColors::border())
                        .child(lucide_icon(icon, 15.0, ShellDeckColors::primary()))
                        .child(
                            div()
                                .text_size(px(14.0))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(ShellDeckColors::text_primary())
                                .child(title),
                        ),
                )
                .child(div().flex().flex_col().px(px(16.0)).py(px(4.0)).child(body))
        };

        let role_label = if account.is_superadmin {
            t!("user.infos.role.superadmin").to_string()
        } else if account.is_inklura_support {
            t!("user.infos.role.inklura_support").to_string()
        } else if account.is_admin {
            t!("user.infos.role.admin").to_string()
        } else {
            t!("user.infos.role.user").to_string()
        };

        // Session — device + role + timestamps returned by whoami.
        let session_body = div()
            .flex()
            .flex_col()
            .child(field(
                t!("user.infos.field.device").to_string(),
                whoami.label.clone().unwrap_or_default(),
                "keyboard",
            ))
            .child(field(
                t!("user.infos.field.role").to_string(),
                role_label,
                "shield",
            ))
            .child(field(
                t!("user.infos.field.since").to_string(),
                whoami.created_at.clone().unwrap_or_default(),
                "calendar",
            ))
            .child(field(
                t!("user.infos.field.last_seen").to_string(),
                whoami.last_seen_at.clone().unwrap_or_default(),
                "clock",
            ));

        // Account — identity + Manage server.
        let account_body = div()
            .flex()
            .flex_col()
            .child(field(
                t!("user.infos.field.name").to_string(),
                account.display_name(),
                "user",
            ))
            .child(field(
                t!("user.infos.field.email").to_string(),
                account.email.clone(),
                "mail",
            ))
            .child(field(
                t!("user.infos.field.server").to_string(),
                server,
                "globe",
            ));

        // Scope — tenant + sites the server exposed to us.
        let tenant_name = payload
            .sites
            .first()
            .map(|s| s.tenant_name.clone())
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_default();
        let sites_count = payload.sites.len();
        let scope_body = div()
            .flex()
            .flex_col()
            .child(field(
                t!("user.infos.field.tenant").to_string(),
                tenant_name,
                "users",
            ))
            .child(field(
                t!("user.infos.field.sites_available", count = sites_count).to_string(),
                t!("user.infos.field.sites_count", count = sites_count).to_string(),
                "globe",
            ));

        // Roles — one badge per entry in the CM role bag. Surfaces every
        // custom role (`content_editor`, `customer_service`, …) the tenant
        // admin defined in Manage, not just the hardcoded super-admin /
        // admin tiers the mode gate uses. See `.agents/roles.md` for the
        // "bag is the truth, predicates are shortcuts" rule.
        let roles_body = {
            let mut container = div().flex().flex_col().py(px(4.0));
            if account.roles.is_empty() {
                container = container.child(
                    div()
                        .py(px(8.0))
                        .text_size(px(12.0))
                        .text_color(ShellDeckColors::text_muted())
                        .child(t!("user.infos.roles.empty").to_string()),
                );
            } else {
                let mut row = div().flex().flex_wrap().gap(px(6.0)).py(px(8.0));
                for role in &account.roles {
                    row = row.child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(4.0))
                            .px(px(8.0))
                            .py(px(3.0))
                            .rounded(px(6.0))
                            .bg(ShellDeckColors::primary().opacity(0.12))
                            .border_1()
                            .border_color(ShellDeckColors::primary().opacity(0.35))
                            .text_size(px(11.0))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(ShellDeckColors::primary())
                            .child(lucide_icon("shield", 10.0, ShellDeckColors::primary()))
                            .child(role.clone()),
                    );
                }
                container = container.child(row);
            }
            container
        };

        let _ = cx; // no listeners here — the tab is read-only.
        div()
            .id("user-infos-tab")
            .flex()
            .flex_col()
            .pb(px(16.0))
            .child(section(
                t!("user.infos.section.session").to_string(),
                "shield",
                session_body,
            ))
            .child(section(
                t!("user.infos.section.roles").to_string(),
                "shield",
                roles_body,
            ))
            .child(section(
                t!("user.infos.section.account").to_string(),
                "user",
                account_body,
            ))
            .child(section(
                t!("user.infos.section.scope").to_string(),
                "users",
                scope_body,
            ))
    }

    /// User mode: a manage-centric home — account header + "Mes sites" list with
    /// per-site Activer + area deep links.
    /// Pre-login welcome landing — intercepts the render whenever the user
    /// is not signed in (there is no guest path). Two-part layout:
    ///
    /// 1. **Hero** — ShellDeck brand icon + title + tagline + two CTAs
    ///    (sign in / create account).
    /// 2. **Inklura marketing** — the Inklura brand block + value props
    ///    lifted from inklura.fr, so a first-time visitor understands
    ///    what they're being invited into before creating an account.
    ///
    /// Kept inside a `scrollable_vertical` because on small windows the
    /// marketing block would push the CTAs offscreen.
    fn render_welcome_screen(&self, cx: &mut Context<Self>) -> impl IntoElement {
        // Small helper for the four Inklura value-prop cards — same shape
        // so the row reads as a set.
        fn stat_card(icon: &'static str, value: String, label: String) -> impl IntoElement {
            div()
                .flex()
                .flex_col()
                .items_center()
                .gap(px(4.0))
                .w(px(150.0))
                .px(px(12.0))
                .py(px(14.0))
                .rounded(px(10.0))
                .border_1()
                .border_color(ShellDeckColors::border())
                .bg(ShellDeckColors::bg_sidebar())
                .child(lucide_icon(icon, 22.0, ShellDeckColors::primary()))
                .child(
                    div()
                        .text_size(px(18.0))
                        .font_weight(FontWeight::BOLD)
                        .text_color(ShellDeckColors::text_primary())
                        .child(value),
                )
                .child(
                    div()
                        .text_size(px(11.0))
                        .text_color(ShellDeckColors::text_muted())
                        .child(label),
                )
        }

        let entity = cx.entity();

        // Hero — brand + CTAs.
        let hero = div()
            .flex()
            .flex_col()
            .items_center()
            .gap(px(16.0))
            .pt(px(48.0))
            .pb(px(32.0))
            .child(
                // ShellDeck brand mark — PNG (not SVG) because GPUI renders
                // SVGs in currentColor and the mark's multi-fill palette
                // (teal frame + dark inner + light glyph) would collapse
                // to a single tint. The PNG raster preserves every colour.
                img("images/shelldeck-icon.png").w(px(72.0)).h(px(72.0)),
            )
            .child(
                div()
                    .text_size(px(24.0))
                    .font_weight(FontWeight::BOLD)
                    .text_color(ShellDeckColors::text_primary())
                    .child(t!("welcome.title").to_string()),
            )
            .child(
                div()
                    .max_w(px(460.0))
                    .text_size(px(13.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(t!("welcome.tagline").to_string()),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(8.0))
                    .mt(px(8.0))
                    .child(
                        // Primary CTA — funnels to the existing LoginForm modal.
                        div()
                            .id("welcome-sign-in")
                            .flex()
                            .items_center()
                            .gap(px(8.0))
                            .px(px(20.0))
                            .py(px(10.0))
                            .rounded(px(10.0))
                            .bg(ShellDeckColors::primary())
                            .text_size(px(14.0))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(white())
                            .cursor_pointer()
                            .child(
                                svg()
                                    .path(lucide_path("external-link"))
                                    .size(px(14.0))
                                    .text_color(white()),
                            )
                            .child(t!("welcome.sign_in").to_string())
                            .on_click({
                                let entity = entity.clone();
                                move |_, _, cx| {
                                    entity.update(cx, |this, cx| this.show_login_form(cx));
                                }
                            }),
                    )
                    .child(
                        // Secondary CTA — opens Manage signup in the browser.
                        div()
                            .id("welcome-signup")
                            .flex()
                            .items_center()
                            .gap(px(6.0))
                            .px(px(14.0))
                            .py(px(6.0))
                            .rounded(px(8.0))
                            .text_size(px(12.0))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(ShellDeckColors::text_muted())
                            .cursor_pointer()
                            .hover(|s| {
                                s.bg(ShellDeckColors::hover_bg())
                                    .text_color(ShellDeckColors::text_primary())
                            })
                            .child(lucide_icon(
                                "external-link",
                                11.0,
                                ShellDeckColors::text_muted(),
                            ))
                            .child(t!("welcome.create_account").to_string())
                            .on_click({
                                let entity = entity.clone();
                                move |_, _, cx| {
                                    entity.update(cx, |this, cx| this.open_signup(cx));
                                }
                            }),
                    ),
            );

        // Inklura marketing block — content lifted from inklura.fr so the
        // messaging stays in sync with the marketing site. Not a full
        // marketing page; just enough for a first-time visitor to know
        // what they're being invited into.
        let inklura = div()
            .flex()
            .flex_col()
            .items_center()
            .gap(px(14.0))
            .mt(px(8.0))
            .pt(px(24.0))
            .pb(px(48.0))
            .px(px(32.0))
            .border_t_1()
            .border_color(ShellDeckColors::border())
            .child(
                // Inklura brand square — same 28×42 mark on #146BFF ground
                // as the login modal, for visual consistency across the
                // pre-auth surfaces.
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .w(px(28.0))
                    .h(px(42.0))
                    .rounded(px(8.0))
                    .bg(rgb(0x146BFF))
                    .child(
                        svg()
                            .path("images/logo-inklura.svg")
                            .w(px(28.0))
                            .h(px(42.0))
                            .text_color(gpui::white()),
                    ),
            )
            .child(
                div()
                    .text_size(px(20.0))
                    .font_weight(FontWeight::BOLD)
                    .text_color(ShellDeckColors::text_primary())
                    .child(t!("welcome.inklura.title").to_string()),
            )
            .child(
                div()
                    .max_w(px(560.0))
                    .text_size(px(13.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(t!("welcome.inklura.subtitle").to_string()),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .justify_center()
                    .gap(px(10.0))
                    .mt(px(6.0))
                    .child(stat_card(
                        "zap",
                        t!("welcome.inklura.stat.savings.value").to_string(),
                        t!("welcome.inklura.stat.savings.label").to_string(),
                    ))
                    .child(stat_card(
                        "clock",
                        t!("welcome.inklura.stat.time.value").to_string(),
                        t!("welcome.inklura.stat.time.label").to_string(),
                    ))
                    .child(stat_card(
                        "shield",
                        t!("welcome.inklura.stat.uptime.value").to_string(),
                        t!("welcome.inklura.stat.uptime.label").to_string(),
                    ))
                    .child(stat_card(
                        "users",
                        t!("welcome.inklura.stat.clients.value").to_string(),
                        t!("welcome.inklura.stat.clients.label").to_string(),
                    )),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.0))
                    .mt(px(8.0))
                    .text_size(px(11.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(lucide_icon("check", 11.0, ShellDeckColors::success()))
                    .child(t!("welcome.inklura.trust").to_string()),
            );

        // "Réalisé par WD29" footer — same shape as the Settings > About
        // signature so a first-time visitor sees the same attribution
        // whether they land here or hit About after signing in.
        const LOGO_H: f32 = 20.0;
        let made_by = div()
            .flex()
            .items_center()
            .justify_center()
            .gap(px(8.0))
            .py(px(20.0))
            .text_color(ShellDeckColors::text_muted())
            .child(
                div()
                    .flex()
                    .items_center()
                    .h(px(LOGO_H))
                    .text_size(px(11.0))
                    .line_height(px(LOGO_H))
                    .child(t!("settings.about.made_by").to_string()),
            )
            .child(
                div().flex().items_center().h(px(LOGO_H)).child(
                    svg()
                        .path("images/wd29-logo.svg")
                        .w(px(56.0))
                        .h(px(LOGO_H))
                        .flex_shrink_0()
                        .text_color(ShellDeckColors::text_muted()),
                ),
            );

        // Full page — scrolls if the three blocks don't fit the window.
        div()
            .size_full()
            .bg(ShellDeckColors::bg_primary())
            .child(scrollable_vertical(
                div()
                    .id("welcome-body")
                    .flex()
                    .flex_col()
                    .items_center()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .items_center()
                            .w_full()
                            .child(hero)
                            .child(inklura)
                            .child(made_by),
                    ),
            ))
    }

    fn render_user_home(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let account = self.app_config.account.clone().unwrap_or_default();
        let server = self.account_base_url();
        let payload = self.site_directory.clone().unwrap_or_default();

        // Preferred area buttons for each site row (subset of the directory).
        let preferred = [
            "dashboard",
            "cms",
            "helpdesk",
            "ecommerce",
            "settings",
            "shelldeck",
        ];
        let area_buttons: Vec<manage_sites::ManageArea> = preferred
            .iter()
            .filter_map(|k| payload.areas.iter().find(|a| a.key == *k).cloned())
            .collect();

        // Header card.
        let header = div()
            .flex()
            .items_center()
            .justify_between()
            .gap(px(12.0))
            .p(px(16.0))
            .m(px(16.0))
            .rounded(px(12.0))
            .border_1()
            .border_color(ShellDeckColors::border())
            .bg(ShellDeckColors::bg_sidebar())
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(12.0))
                    .child(
                        div()
                            .size(px(40.0))
                            .rounded_full()
                            .bg(ShellDeckColors::primary().opacity(0.20))
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_size(px(17.0))
                            .font_weight(FontWeight::BOLD)
                            .text_color(ShellDeckColors::primary())
                            .child(account.initial()),
                    )
                    .child({
                        let mut name_row = div().flex().items_center().gap(px(8.0)).child(
                            div()
                                .text_size(px(16.0))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(ShellDeckColors::text_primary())
                                .child(account.display_name()),
                        );
                        // Super-admin badge (`shield` + label, primary tint)
                        // — surfaces the role the token was minted with so
                        // the user knows why they see Support/Dev options.
                        if account.is_superadmin {
                            name_row = name_row.child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(4.0))
                                    .px(px(6.0))
                                    .py(px(1.0))
                                    .rounded(px(6.0))
                                    .bg(ShellDeckColors::primary().opacity(0.14))
                                    .border_1()
                                    .border_color(ShellDeckColors::primary().opacity(0.35))
                                    .text_size(px(10.0))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(ShellDeckColors::primary())
                                    .child(lucide_icon("shield", 10.0, ShellDeckColors::primary()))
                                    .child(t!("user.badge.super_admin").to_string()),
                            );
                        }
                        div().flex().flex_col().child(name_row).child(
                            div()
                                .text_size(px(12.0))
                                .text_color(ShellDeckColors::text_muted())
                                .child(format!("{} · {}", account.email, server)),
                        )
                    }),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(
                        div()
                            .id("uh-open-manage")
                            .flex()
                            .items_center()
                            .gap(px(6.0))
                            .px(px(12.0))
                            .py(px(8.0))
                            .rounded(px(8.0))
                            .bg(ShellDeckColors::primary())
                            .text_size(px(13.0))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(white())
                            .cursor_pointer()
                            .child(
                                svg()
                                    .path(lucide_path("external-link"))
                                    .size(px(12.0))
                                    .text_color(white()),
                            )
                            .child(t!("user.open_manage").to_string())
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.open_manage_area("/manage".to_string(), cx);
                            })),
                    )
                    .child(
                        div()
                            .id("uh-sync")
                            .flex()
                            .items_center()
                            .gap(px(6.0))
                            .px(px(12.0))
                            .py(px(8.0))
                            .rounded(px(8.0))
                            .border_1()
                            .border_color(ShellDeckColors::border())
                            .bg(ShellDeckColors::bg_primary())
                            .text_size(px(13.0))
                            .text_color(ShellDeckColors::text_primary())
                            .cursor_pointer()
                            .hover(|s| s.bg(ShellDeckColors::hover_bg()))
                            .child(lucide_icon(
                                "refresh-cw",
                                12.0,
                                ShellDeckColors::text_muted(),
                            ))
                            .child(t!("user.sync").to_string())
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.cloud_sync_now(cx);
                            })),
                    ),
            );

        // Sites: filter by search, sort (conn-bearing first, then alpha),
        // split into (active-card, others-for-virt-list). Recomputed inside
        // the `uniform_list` processor as well — cheap enough on 300 sites
        // (< 1ms) and keeps the model authoritative.
        let (active_site, others_sites) = self.partition_user_sites(cx);
        let others_count = others_sites.len();

        let mut list = div()
            .id("user-home-sites")
            .flex()
            .flex_col()
            .gap(px(8.0))
            .px(px(16.0));

        if active_site.is_none() && others_count == 0 {
            // Centered CTA card instead of a passive mumble line — makes it
            // clear the next action is to open Manage (or Synchroniser if the
            // sites were just created).
            let empty_card = div()
                .flex()
                .flex_col()
                .items_center()
                .gap(px(12.0))
                .p(px(28.0))
                .rounded(px(12.0))
                .border_1()
                .border_color(ShellDeckColors::border())
                .bg(ShellDeckColors::bg_sidebar())
                .child(
                    div()
                        .size(px(44.0))
                        .rounded_full()
                        .bg(ShellDeckColors::primary().opacity(0.15))
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_size(px(20.0))
                        .text_color(ShellDeckColors::primary())
                        .child(">_"),
                )
                .child(
                    div()
                        .text_size(px(14.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(ShellDeckColors::text_primary())
                        .child(t!("user.sites.empty.title").to_string()),
                )
                .child(
                    div()
                        .text_size(px(12.0))
                        .text_color(ShellDeckColors::text_muted())
                        .child(t!("user.sites.empty.hint").to_string()),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.0))
                        .mt(px(4.0))
                        .child(
                            div()
                                .id("uh-empty-open-manage")
                                .flex()
                                .items_center()
                                .gap(px(6.0))
                                .px(px(14.0))
                                .py(px(8.0))
                                .rounded(px(8.0))
                                .bg(ShellDeckColors::primary())
                                .text_size(px(13.0))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(white())
                                .cursor_pointer()
                                .child(
                                    svg()
                                        .path(lucide_path("external-link"))
                                        .size(px(12.0))
                                        .text_color(white()),
                                )
                                .child(t!("user.open_manage").to_string())
                                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                    this.open_manage_area("/manage".to_string(), cx);
                                })),
                        )
                        .child(
                            div()
                                .id("uh-empty-sync")
                                .flex()
                                .items_center()
                                .gap(px(6.0))
                                .px(px(14.0))
                                .py(px(8.0))
                                .rounded(px(8.0))
                                .border_1()
                                .border_color(ShellDeckColors::border())
                                .bg(ShellDeckColors::bg_primary())
                                .text_size(px(13.0))
                                .text_color(ShellDeckColors::text_primary())
                                .cursor_pointer()
                                .hover(|s| s.bg(ShellDeckColors::hover_bg()))
                                .child(lucide_icon(
                                    "refresh-cw",
                                    12.0,
                                    ShellDeckColors::text_muted(),
                                ))
                                .child(t!("user.sync").to_string())
                                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                    this.cloud_sync_now(cx);
                                })),
                        ),
                );
            list = list.child(empty_card);
        }

        // Active site sits at the top as a full "rich" card (identity +
        // wp-admin shortcut + all six area deep-links). It's the only card
        // that owns the areas — Activer on any other row promotes that
        // site here.
        if let Some(site) = active_site.as_ref() {
            list = list.child(self.render_active_site_card(site, &area_buttons, cx));
        }

        // Everyone else is a fixed-height compact row inside a virtualised
        // `uniform_list`. Height per row is deliberately uniform so GPUI's
        // virtualiser knows how many rows fit the viewport without probing
        // each one — that's the whole point of this refactor: paint budget
        // becomes O(visible) instead of O(sites).
        if others_count > 0 {
            const MAX_LIST_H: f32 = 600.0;
            const MIN_LIST_H: f32 = 120.0;
            let visible_h = (others_count as f32 * SITE_ROW_H).clamp(MIN_LIST_H, MAX_LIST_H);
            list = list.child(
                div().w_full().h(px(visible_h)).child(
                    uniform_list(
                        "user-home-sites-virt",
                        others_count,
                        cx.processor(|this, range: Range<usize>, _window, cx| {
                            let (_, others) = this.partition_user_sites(cx);
                            let mut items: Vec<AnyElement> = Vec::new();
                            for i in range {
                                if let Some(site) = others.get(i) {
                                    items.push(
                                        this.render_compact_site_row(site, cx).into_any_element(),
                                    );
                                }
                            }
                            items
                        }),
                    )
                    .w_full()
                    .h_full(),
                ),
            );
        }

        // Page body: account header, "Mes sites" section, optional Jean card,
        // "Mes demandes" section. Everything stacks at natural height; the
        // whole page scrolls if the content overflows.
        let tab = self.user_home_tab;
        let tab_bar = self.render_user_home_tab_bar(cx);

        // Body composition: header (persistent) + tab bar + tab content.
        // Each tab owns its own inner scroll. Previously the whole page
        // scrolled as one; splitting kept the header visible while the
        // active tab scrolls, and let the Sites tab embed a virtualised
        // list without competing with an outer scroll.
        let mut body = div()
            .id("user-home-body")
            .flex()
            .flex_col()
            .pb(px(24.0))
            .child(header)
            .child(tab_bar);
        match tab {
            UserHomeTab::Sites => {
                body = body
                    .child({
                        // Section header: title on the left, live search on
                        // the right (only when there are enough sites to
                        // make it worth it — small tenants keep the row
                        // uncluttered).
                        let mut row = div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap(px(8.0))
                            .px(px(16.0))
                            .pt(px(8.0))
                            .pb(px(6.0))
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(8.0))
                                    .child(lucide_icon(
                                        "globe",
                                        16.0,
                                        ShellDeckColors::text_muted(),
                                    ))
                                    .child(
                                        div()
                                            .text_size(px(18.0))
                                            .font_weight(FontWeight::BOLD)
                                            .text_color(ShellDeckColors::text_primary())
                                            .child(t!("user.sites.title").to_string()),
                                    ),
                            );
                        if payload.sites.len() > 5 {
                            let entity = cx.entity();
                            row = row.child(
                                div().w(px(260.0)).child(
                                    Input::new(&self.user_sites_search_state)
                                        .size(InputSize::Sm)
                                        .placeholder(t!("user.sites.search").to_string())
                                        .prefix(lucide_icon(
                                            "search",
                                            12.0,
                                            ShellDeckColors::text_muted(),
                                        ))
                                        .on_change(move |_, cx| {
                                            entity.update(cx, |_, cx| cx.notify());
                                        }),
                                ),
                            );
                        }
                        row
                    })
                    .child(list)
                    .children(if self.has_jean() {
                        Some(self.render_jean_ask_card(cx))
                    } else {
                        None
                    });
            }
            UserHomeTab::Requests => {
                body = body.child(self.render_user_requests(cx));
            }
            UserHomeTab::Infos => {
                body = body.child(self.render_user_infos_tab(cx));
            }
        }

        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(ShellDeckColors::bg_primary())
            .child(scrollable_vertical(body))
    }

    /// One row of the "Mes demandes" list — status badge, title, priority,
    /// optional GitHub number, and a hover-only red trash icon that opens
    /// the delete confirm. The hover kebab is hand-rolled (matches the
    /// sidebar's per-row action pattern) because adabraka `IconButton`
    /// derives its ElementId from the icon name and would collide across
    /// rows.
    fn render_user_request_row(&self, iss: &Issue, cx: &mut Context<Self>) -> impl IntoElement {
        let id = iss.id.clone();
        let selected = self.issue_selected.as_deref() == Some(iss.id.as_str());
        let group_name = SharedString::from(format!("uiss-row-{}", iss.id));
        let mut row = div()
            .id(ElementId::from(SharedString::from(format!(
                "uiss-{}",
                iss.id
            ))))
            .group(group_name.clone())
            .w_full()
            .flex()
            .items_center()
            .gap(px(8.0))
            .px(px(10.0))
            .py(px(7.0))
            .rounded(px(8.0))
            .border_1()
            .border_color(if selected {
                ShellDeckColors::primary()
            } else {
                ShellDeckColors::border()
            })
            .cursor_pointer()
            .hover(|s| s.bg(ShellDeckColors::hover_bg()))
            .on_click({
                let id = id.clone();
                cx.listener(move |this, _: &ClickEvent, _, cx| this.select_issue(id.clone(), cx))
            })
            .child(issue_status_badge(&iss.status))
            .child(
                div()
                    .flex_1()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_size(px(13.0))
                    .text_color(ShellDeckColors::text_primary())
                    .child(iss.title.clone()),
            )
            .child(priority_badge(&iss.priority));
        if let Some(g) = &iss.github {
            row = row.child(
                div()
                    .flex_shrink_0()
                    .text_size(px(10.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(t!("user.github_issue", number = g.number).to_string()),
            );
        }
        let del_id = iss.id.clone();
        row.child(
            div()
                .id(ElementId::from(SharedString::from(format!(
                    "uiss-del-{}",
                    iss.id
                ))))
                .flex_shrink_0()
                .flex()
                .items_center()
                .justify_center()
                .w(px(22.0))
                .h(px(22.0))
                .rounded(px(4.0))
                .cursor_pointer()
                .text_color(ShellDeckColors::error())
                .opacity(0.0)
                .group_hover(group_name.clone(), |el| el.opacity(1.0))
                .hover(|el| el.bg(ShellDeckColors::error().opacity(0.15)))
                .child(
                    svg()
                        .path(lucide_path("trash-2"))
                        .size(px(13.0))
                        .text_color(ShellDeckColors::error()),
                )
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    cx.stop_propagation();
                    this.confirm_issue_delete = Some(del_id.clone());
                    cx.notify();
                })),
        )
    }

    /// User-mode "Mes demandes": a list of the tenant's requests. Selecting a
    /// row opens the detail as a right-side sheet; the "+ Nouvelle demande"
    /// button in the header opens the composer as another right-side sheet.
    /// Both live at the workspace root — they slide over the list without
    /// pushing it down (the pre-sheet layout used to append them inline).
    fn render_user_requests(&self, cx: &mut Context<Self>) -> impl IntoElement {
        // User mode is the "as-a-normal-user" surface — even for a
        // super-admin viewing it, we only surface requests *they* filed.
        // (The server hands staff every in-scope request without a
        // `requested_by` filter — cf. `issuesInScope` in the manage repo — so
        // the "Mes demandes" label would otherwise be misleading.)
        let mine_count = self
            .issues_list
            .iter()
            .filter(|i| self.is_my_issue(i))
            .count();
        let list = if mine_count == 0 {
            div()
                .mt(px(8.0))
                .child(
                    div()
                        .py(px(8.0))
                        .text_size(px(12.0))
                        .text_color(ShellDeckColors::text_muted())
                        .child(t!("user.requests.empty").to_string()),
                )
                .into_any_element()
        } else {
            const MAX_LIST_H: f32 = 600.0;
            const MIN_LIST_H: f32 = 120.0;
            let visible_h = (mine_count as f32 * USER_REQUEST_ROW_H).clamp(MIN_LIST_H, MAX_LIST_H);
            div()
                .w_full()
                .h(px(visible_h))
                .mt(px(8.0))
                .child(
                    uniform_list(
                        "user-requests-virt",
                        mine_count,
                        cx.processor(|this, range: Range<usize>, _window, cx| {
                            let mine_indices = this
                                .issues_list
                                .iter()
                                .enumerate()
                                .filter(|(_, issue)| this.is_my_issue(issue))
                                .map(|(index, _)| index)
                                .collect::<Vec<_>>();
                            range
                                .filter_map(|index| mine_indices.get(index).copied())
                                .filter_map(|index| this.issues_list.get(index))
                                .map(|issue| {
                                    div()
                                        .w_full()
                                        .pb(px(4.0))
                                        .child(this.render_user_request_row(issue, cx))
                                        .into_any_element()
                                })
                                .collect::<Vec<_>>()
                        }),
                    )
                    .w_full()
                    .h_full(),
                )
                .into_any_element()
        };

        // Section header: title + "Nouvelle demande" button.
        let header = div()
            .flex()
            .items_center()
            .justify_between()
            .mb(px(4.0))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(lucide_icon("tag", 16.0, ShellDeckColors::text_muted()))
                    .child(
                        div()
                            .text_size(px(18.0))
                            .font_weight(FontWeight::BOLD)
                            .text_color(ShellDeckColors::text_primary())
                            .child(t!("user.requests.title").to_string()),
                    ),
            )
            .child(
                div()
                    .id("user-new-request-btn")
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .px(px(10.0))
                    .py(px(6.0))
                    .rounded(px(6.0))
                    .bg(ShellDeckColors::primary())
                    .text_size(px(12.0))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(white())
                    .cursor_pointer()
                    .child(
                        svg()
                            .path("icons/lucide/plus.svg")
                            .size(px(11.0))
                            .text_color(white()),
                    )
                    .child(t!("user.requests.new").to_string())
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.open_new_request(cx);
                    })),
            );

        div()
            .flex()
            .flex_col()
            .gap(px(4.0))
            .m(px(16.0))
            .child(header)
            .child(list)
    }

    /// Full-screen dimmed backdrop + right-anchored panel that wraps some inner
    /// content. Shared chrome for the two User-mode issue sheets (composer +
    /// detail). Clicking the backdrop or the header × triggers `on_close`;
    /// inner clicks are stopped so the backdrop doesn't dismiss.
    ///
    /// `dismissing = true` plays the exit animation (slide back off-screen
    /// right + fade out); `false` plays the enter animation.
    #[allow(clippy::too_many_arguments)]
    fn render_user_sheet<C: IntoElement + 'static>(
        &self,
        id: &'static str,
        title: String,
        icon: Option<&'static str>,
        dismissing: bool,
        inner: C,
        on_close: impl Fn(&mut Self, &mut Context<Self>) + Clone + 'static,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        use std::time::Duration;
        const SHEET_WIDTH: f32 = 480.0;
        const ANIM_MS: u64 = SHEET_ANIM_MS;

        let close_bg = on_close.clone();
        div()
            .id(id)
            .occlude()
            .absolute()
            .top_0()
            .left_0()
            .right_0()
            .bottom_0()
            .bg(ShellDeckColors::backdrop())
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _e, _window, cx| {
                    close_bg(this, cx);
                }),
            )
            .child(
                div()
                    .absolute()
                    .top_0()
                    .right_0()
                    .bottom_0()
                    .flex()
                    .flex_col()
                    .w(px(SHEET_WIDTH))
                    .bg(ShellDeckColors::bg_surface())
                    .border_l_1()
                    .border_color(ShellDeckColors::border())
                    .shadow_xl()
                    .overflow_hidden()
                    .on_mouse_down(MouseButton::Left, |_e, _window, cx: &mut App| {
                        cx.stop_propagation();
                    })
                    // Sheet header: title + close button.
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .flex_shrink_0()
                            .px(px(20.0))
                            .py(px(14.0))
                            .border_b_1()
                            .border_color(ShellDeckColors::border())
                            .child({
                                let mut row = div().flex().items_center().gap(px(8.0));
                                if let Some(slug) = icon {
                                    row = row.child(lucide_icon(
                                        slug,
                                        16.0,
                                        ShellDeckColors::primary(),
                                    ));
                                }
                                row.child(
                                    div()
                                        .text_size(px(16.0))
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(ShellDeckColors::text_primary())
                                        .child(title.clone()),
                                )
                            })
                            .child({
                                let close = on_close.clone();
                                div()
                                    .id("user-sheet-close")
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .cursor_pointer()
                                    .text_color(ShellDeckColors::text_muted())
                                    .hover(|el| el.text_color(ShellDeckColors::text_primary()))
                                    .child(
                                        svg()
                                            .path("icons/lucide/x.svg")
                                            .size(px(14.0))
                                            .text_color(ShellDeckColors::text_muted()),
                                    )
                                    .on_click(cx.listener(
                                        move |this, _: &ClickEvent, _window, cx| {
                                            close(this, cx);
                                        },
                                    ))
                            }),
                    )
                    // Body — scrollable if the content overflows the sheet.
                    .child(
                        div()
                            .id("user-sheet-body")
                            .flex_grow()
                            .min_h(px(0.0))
                            .overflow_y_scroll()
                            .p(px(16.0))
                            .child(inner),
                    )
                    // Slide (300ms). On enter: ease_out_quint (very smooth
                    // decel), from `right = -SHEET_WIDTH` to 0. On exit:
                    // ease_in_quint reversed. Encoding the direction in the
                    // id makes GPUI treat enter vs exit as distinct
                    // animations and restart cleanly on each flip.
                    .with_animation(
                        SharedString::from(format!(
                            "{id}-slide-{}",
                            if dismissing { "out" } else { "in" }
                        )),
                        Animation::new(Duration::from_millis(ANIM_MS)).with_easing(if dismissing {
                            (|t: f32| t * t * t * t * t) as fn(f32) -> f32 // ease_in_quint
                        } else {
                            (|t: f32| 1.0 - (1.0 - t).powi(5)) as fn(f32) -> f32
                            // ease_out_quint
                        }),
                        move |el, delta| {
                            let d = delta.clamp(0.0, 1.0);
                            let offset = if dismissing {
                                -SHEET_WIDTH * d
                            } else {
                                -SHEET_WIDTH * (1.0 - d)
                            };
                            el.right(gpui::px(offset))
                        },
                    ),
            )
    }

    /// The "Nouvelle demande" composer rendered as a right-side sheet.
    fn render_issue_attachment_picker(
        &self,
        target: IssueAttachmentTarget,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let drafts = self.attachment_drafts(target).clone();
        let mut previews = div().flex().flex_wrap().gap(px(8.0));
        for (index, draft) in drafts.iter().enumerate() {
            let filename = draft.filename.clone();
            previews = previews.child(
                div()
                    .id(ElementId::from(SharedString::from(format!(
                        "issue-attachment-{target:?}-{index}"
                    ))))
                    .relative()
                    .w(px(76.0))
                    .h(px(76.0))
                    .rounded(px(7.0))
                    .overflow_hidden()
                    .border_1()
                    .border_color(ShellDeckColors::border())
                    .tooltip(move |_window, cx| {
                        cx.new(|_| WorkspaceTooltip {
                            label: filename.clone().into(),
                        })
                        .into()
                    })
                    .child(
                        img(draft.image.clone())
                            .size_full()
                            .object_fit(ObjectFit::Cover),
                    )
                    .child(
                        div()
                            .id(ElementId::from(SharedString::from(format!(
                                "issue-attachment-remove-{target:?}-{index}"
                            ))))
                            .absolute()
                            .top(px(4.0))
                            .right(px(4.0))
                            .size(px(20.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded_full()
                            .bg(ShellDeckColors::backdrop())
                            .cursor_pointer()
                            .child(lucide_icon("x", 12.0, white()))
                            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                let drafts = this.attachment_drafts_mut(target);
                                if index < drafts.len() {
                                    drafts.remove(index);
                                }
                                cx.notify();
                            })),
                    ),
            );
        }

        let url_input = Input::new(&self.issue_attachment_url_state)
            .size(InputSize::Sm)
            .placeholder(t!("user.requests.attachments.url_placeholder").to_string())
            .on_enter({
                let entity = cx.entity();
                move |_value, cx| {
                    entity.update(cx, |ws, cx| ws.import_issue_attachment_url(target, cx))
                }
            });

        div()
            .id(ElementId::from(SharedString::from(format!(
                "issue-attachment-picker-{target:?}"
            ))))
            .flex()
            .flex_col()
            .gap(px(8.0))
            .p(px(10.0))
            .rounded(px(8.0))
            .border_1()
            .border_color(ShellDeckColors::border())
            .bg(ShellDeckColors::bg_primary().opacity(0.55))
            .on_key_down(cx.listener(move |this, event: &KeyDownEvent, _, cx| {
                let mods = event.keystroke.modifiers;
                if event.keystroke.key.eq_ignore_ascii_case("v")
                    && (mods.control || mods.platform)
                    && this.paste_issue_attachment(target, cx)
                {
                    cx.stop_propagation();
                }
            }))
            .on_drop(cx.listener(move |this, paths: &ExternalPaths, _, cx| {
                let generation = this.issue_attachment_generation;
                this.import_attachment_paths(target, paths.paths().to_vec(), generation, cx);
            }))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_size(px(11.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(ShellDeckColors::text_primary())
                            .child(t!("user.requests.attachments.title").to_string()),
                    )
                    .child(
                        div()
                            .text_size(px(10.0))
                            .text_color(ShellDeckColors::text_muted())
                            .child(t!("user.requests.attachments.drop_hint").to_string()),
                    ),
            )
            .when(!drafts.is_empty(), |el| el.child(previews))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .child(
                        Button::new(
                            SharedString::from(format!("issue-file-{target:?}")),
                            t!("user.requests.attachments.file").to_string(),
                        )
                        .size(ButtonSize::Sm)
                        .variant(ButtonVariant::Outline)
                        .icon(IconSource::from("upload"))
                        .disabled(self.issue_attachment_busy)
                        .on_click(cx.listener(
                            move |this, _, window, cx| {
                                this.pick_issue_attachments(target, window, cx);
                            },
                        )),
                    )
                    .child(
                        Button::new(
                            SharedString::from(format!("issue-paste-{target:?}")),
                            t!("user.requests.attachments.paste").to_string(),
                        )
                        .size(ButtonSize::Sm)
                        .variant(ButtonVariant::Outline)
                        .icon(IconSource::from("clipboard-paste"))
                        .disabled(self.issue_attachment_busy)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if !this.paste_issue_attachment(target, cx) {
                                this.show_toast(
                                    t!("toast.issue.clipboard_no_image").to_string(),
                                    ToastLevel::Warning,
                                    cx,
                                );
                            }
                        })),
                    )
                    .child(
                        Button::new(
                            SharedString::from(format!("issue-capture-{target:?}")),
                            t!("user.requests.attachments.capture").to_string(),
                        )
                        .size(ButtonSize::Sm)
                        .variant(ButtonVariant::Outline)
                        .icon(IconSource::from("plus"))
                        .disabled(self.issue_attachment_busy)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.capture_issue_attachment(target, cx);
                        })),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .child(div().flex_1().min_w(px(0.0)).child(url_input))
                    .child(
                        Button::new(
                            SharedString::from(format!("issue-url-{target:?}")),
                            t!("user.requests.attachments.add_url").to_string(),
                        )
                        .size(ButtonSize::Sm)
                        .variant(ButtonVariant::Outline)
                        .icon(IconSource::from("globe"))
                        .disabled(self.issue_attachment_busy)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.import_issue_attachment_url(target, cx);
                        })),
                    ),
            )
    }

    fn render_stored_attachments(
        &self,
        attachments: &[issues::IssueAttachment],
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mut row = div().flex().flex_wrap().gap(px(6.0));
        for attachment in attachments {
            let url = if attachment.viewer_url.is_empty() {
                attachment.url.clone()
            } else {
                attachment.viewer_url.clone()
            };
            row = row.child(
                div()
                    .id(ElementId::from(SharedString::from(format!(
                        "stored-attachment-{}",
                        attachment.id
                    ))))
                    .flex()
                    .items_center()
                    .gap(px(5.0))
                    .max_w(px(210.0))
                    .px(px(8.0))
                    .py(px(6.0))
                    .rounded(px(6.0))
                    .border_1()
                    .border_color(ShellDeckColors::border())
                    .text_size(px(11.0))
                    .text_color(ShellDeckColors::primary())
                    .cursor_pointer()
                    .child(lucide_icon("globe", 12.0, ShellDeckColors::primary()))
                    .child(div().truncate().child(attachment.filename.clone()))
                    .on_click(cx.listener(move |_this, _: &ClickEvent, _, _| {
                        let _ = cloud_account::open_in_browser(&url);
                    })),
            );
        }
        row.into_any_element()
    }

    fn render_user_new_request_sheet(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let priorities = ["low", "normal", "high", "urgent"];
        let mut prio_row = div().flex().items_center().gap(px(6.0));
        for p in priorities {
            let active = self.issue_new_priority == p;
            // Colored adabraka Badge picks up the severity mapping; the
            // wrapper div carries the click-target + a soft ring on the
            // selected option so the picker still reads as a choice, not a
            // read-only tag.
            let mut chip = div()
                .id(ElementId::from(SharedString::from(format!(
                    "iss-np-sheet-{p}"
                ))))
                .p(px(2.0))
                .rounded_full()
                .cursor_pointer()
                .child(priority_badge(p))
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.issue_new_priority = p.to_string();
                    cx.notify();
                }));
            if active {
                chip = chip.border_2().border_color(ShellDeckColors::primary());
            } else {
                chip = chip
                    .border_2()
                    .border_color(gpui::transparent_black())
                    .opacity(0.55);
            }
            prio_row = prio_row.child(chip);
        }

        // Real Input widgets — cursor, selection, undo, Enter to submit.
        // Sm size (32px h / 8px padx / 13px font) matches the compact look
        // the fake-input divs used before the migration.
        let title_input = Input::new(&self.issue_title_state)
            .size(InputSize::Sm)
            .placeholder(t!("user.requests.title_placeholder").to_string())
            .on_enter({
                let entity = cx.entity();
                move |_value, cx| {
                    entity.update(cx, |ws, cx| ws.submit_new_request(cx));
                }
            });
        let body_input = Input::new(&self.issue_body_state)
            .size(InputSize::Sm)
            .placeholder(t!("user.requests.body_placeholder").to_string())
            .multi_line(true)
            .min_rows(4);

        let ai_enabled = self.ai_backend_available() && self.app_config.ai.allows(AiSurface::Issue);
        let mut inner = div().flex().flex_col().gap(px(10.0));
        if ai_enabled {
            let model = if self.app_config.ai.model.trim().is_empty() {
                self.app_config.ai.backend.default_model().to_string()
            } else {
                self.app_config.ai.model.clone()
            };
            let expanded = self.issue_ai_expanded;
            let trigger = div()
                .flex()
                .items_center()
                .justify_between()
                .gap(px(8.0))
                .w_full()
                .px(px(10.0))
                .py(px(8.0))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.0))
                        .min_w(px(0.0))
                        .child(lucide_icon("sparkles", 14.0, ShellDeckColors::primary()))
                        .child(
                            div()
                                .truncate()
                                .text_size(px(12.0))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(ShellDeckColors::primary())
                                .child(t!("user.requests.ai.title").to_string()),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.0))
                        .flex_shrink_0()
                        .child(ai_provider_badge(self.app_config.ai.backend, &model))
                        .child(
                            svg()
                                .path(lucide_path("chevron-down"))
                                .size(px(13.0))
                                .text_color(ShellDeckColors::text_muted())
                                .with_transformation(gpui::Transformation::rotate(gpui::radians(
                                    if expanded {
                                        0.0
                                    } else {
                                        -std::f32::consts::FRAC_PI_2
                                    },
                                ))),
                        ),
                );

            let mut content = div()
                .flex()
                .flex_col()
                .gap(px(8.0))
                .px(px(10.0))
                .pb(px(10.0))
                .child(
                    div()
                        .flex()
                        .items_end()
                        .gap(px(8.0))
                        .child(
                            div().flex_1().min_w(px(0.0)).child(
                                Input::new(&self.issue_ai_prompt_state)
                                    .size(InputSize::Sm)
                                    .multi_line(true)
                                    .min_rows(2)
                                    .max_rows(4)
                                    .placeholder(t!("user.requests.ai.placeholder").to_string())
                                    .disabled(self.issue_ai_loading),
                            ),
                        )
                        .child(
                            Button::new(
                                "user-request-ai-generate",
                                t!("user.requests.ai.generate").to_string(),
                            )
                            .variant(ButtonVariant::Ai)
                            .size(ButtonSize::Sm)
                            .min_w(px(104.0))
                            .disabled(self.issue_ai_loading)
                            .icon(IconSource::from("sparkles"))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.generate_new_request_with_ai(cx);
                            })),
                        ),
                );
            if self.issue_ai_loading {
                content = content.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.0))
                        .text_size(px(11.0))
                        .text_color(ShellDeckColors::text_muted())
                        .child(
                            Spinner::new()
                                .size(SpinnerSize::Xs)
                                .variant(SpinnerVariant::Primary),
                        )
                        .child(t!("user.requests.ai.generating").to_string()),
                );
            }
            if let Some(error) = &self.issue_ai_error {
                content = content.child(
                    div()
                        .text_size(px(11.0))
                        .text_color(ShellDeckColors::error())
                        .child(error.clone()),
                );
            }

            let entity = cx.entity();
            let mut ai_block = AnimatedCollapsible::new()
                .open(expanded)
                .show_icon(false)
                .trigger(trigger)
                .on_toggle(move |open, _, cx| {
                    entity.update(cx, |workspace, cx| {
                        workspace.issue_ai_expanded = open;
                        cx.notify();
                    });
                })
                .rounded(px(6.0))
                .border_1()
                .border_color(ShellDeckColors::primary().opacity(0.35))
                .bg(ShellDeckColors::primary().opacity(0.07));
            if expanded {
                ai_block = ai_block.content(content);
            }
            inner = inner.child(ai_block);
        }

        inner = inner
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(div().flex_1().min_w(px(0.0)).child(title_input))
                    .when(
                        self.ai_backend_available() && self.app_config.ai.allows(AiSurface::Naming),
                        |row| {
                            row.child(
                                Button::new("request-ai-name", t!("ai.naming.action").to_string())
                                    .variant(ButtonVariant::Ai)
                                    .size(ButtonSize::Sm)
                                    .icon(IconSource::from("sparkles"))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.open_ai_workflow(
                                            AiWorkflowTarget::EntityNaming {
                                                kind: AiNamingKind::Issue,
                                                target_id: "new-request".to_string(),
                                            },
                                            cx,
                                        );
                                    })),
                            )
                        },
                    ),
            )
            .child(body_input)
            .child(self.render_issue_attachment_picker(IssueAttachmentTarget::NewRequest, cx))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .mt(px(4.0))
                    .child(prio_row)
                    .child(
                        div()
                            .id("iss-create")
                            .px(px(14.0))
                            .py(px(8.0))
                            .rounded(px(6.0))
                            .bg(ShellDeckColors::primary())
                            .text_size(px(13.0))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(white())
                            .cursor_pointer()
                            .child(t!("user.requests.create").to_string())
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.submit_new_request(cx);
                            })),
                    ),
            );

        self.render_user_sheet(
            "user-new-request-sheet",
            t!("user.requests.new").to_string(),
            Some("plus"),
            self.user_new_request_sheet_dismissing,
            inner,
            |this, cx| this.close_new_request_sheet(cx),
            cx,
        )
    }

    /// The selected-request detail rendered as a right-side sheet.
    fn render_user_issue_detail_sheet(
        &self,
        iss: Issue,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let inner = self.render_user_issue_detail(&iss, cx);
        self.render_user_sheet(
            "user-issue-detail-sheet",
            t!("user.requests.detail_title").to_string(),
            Some("tag"),
            self.user_issue_detail_dismissing,
            inner,
            |this, cx| this.close_user_issue_detail(cx),
            cx,
        )
    }

    fn render_user_issue_detail(&self, iss: &Issue, cx: &mut Context<Self>) -> impl IntoElement {
        let mut thread = div().flex().flex_col().gap(px(6.0)).mt(px(8.0));
        if !iss.body.trim().is_empty() {
            thread = thread.child(
                div()
                    .p(px(10.0))
                    .rounded(px(8.0))
                    .bg(ShellDeckColors::bg_primary())
                    .border_1()
                    .border_color(ShellDeckColors::border())
                    .text_size(px(13.0))
                    .text_color(ShellDeckColors::text_primary())
                    .child(iss.body.clone()),
            );
        }
        if !iss.attachments.is_empty() {
            thread = thread.child(self.render_stored_attachments(&iss.attachments, cx));
        }
        for c in &iss.comments {
            thread = thread.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(2.0))
                    .p(px(9.0))
                    .rounded(px(8.0))
                    .bg(if c.is_note() {
                        ShellDeckColors::warning().opacity(0.10)
                    } else {
                        ShellDeckColors::bg_sidebar()
                    })
                    .child(
                        div()
                            .text_size(px(10.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(ShellDeckColors::text_muted())
                            .child(if c.is_note() {
                                c.kind.clone()
                            } else {
                                c.author.clone()
                            }),
                    )
                    .child(
                        div()
                            .text_size(px(13.0))
                            .text_color(ShellDeckColors::text_primary())
                            .child(c.body.clone()),
                    ),
            );
            if !c.attachments.is_empty() {
                thread = thread.child(self.render_stored_attachments(&c.attachments, cx));
            }
        }

        // Detail content flows directly inside the sheet chrome — no inner box
        // (bg / border / rounded) so the sheet reads as a single surface, not
        // "a card inside a card".
        div()
            .flex()
            .flex_col()
            .gap(px(8.0))
            .mt(px(10.0))
            .child(
                div()
                    .flex()
                    .items_start()
                    .gap(px(8.0))
                    .min_w(px(0.0))
                    .overflow_hidden()
                    .child(div().flex_shrink_0().child(issue_status_badge(&iss.status)))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .line_clamp(3)
                            .text_size(px(14.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(ShellDeckColors::text_primary())
                            .child(iss.title.clone()),
                    )
                    .children(iss.github.as_ref().map(|g| {
                        div()
                            .id("uiss-gh")
                            .flex_shrink_0()
                            .text_size(px(11.0))
                            .text_color(ShellDeckColors::primary())
                            .cursor_pointer()
                            .child(t!("user.github_issue", number = g.number).to_string())
                            .on_click({
                                let url = g.url.clone();
                                cx.listener(move |_t, _: &ClickEvent, _, _cx| {
                                    let _ = cloud_account::open_in_browser(&url);
                                })
                            })
                    })),
            )
            .child(thread)
            .child(self.render_issue_attachment_picker(IssueAttachmentTarget::Comment, cx))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .child(
                        div().flex_1().child(
                            Input::new(&self.issue_comment_state)
                                .size(InputSize::Sm)
                                .placeholder(t!("user.requests.comment_placeholder").to_string())
                                .on_enter({
                                    let entity = cx.entity();
                                    move |_value, cx| {
                                        entity.update(cx, |ws, cx| ws.submit_issue_comment(cx));
                                    }
                                }),
                        ),
                    )
                    .child(
                        div()
                            .id("uiss-comment-send")
                            .flex()
                            .items_center()
                            .gap(px(6.0))
                            .px(px(12.0))
                            .py(px(7.0))
                            .rounded(px(6.0))
                            .bg(ShellDeckColors::primary())
                            .text_size(px(12.0))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(white())
                            .cursor_pointer()
                            .child(
                                svg()
                                    .path(lucide_path("send"))
                                    .size(px(11.0))
                                    .text_color(white()),
                            )
                            .child(t!("user.requests.send").to_string())
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.submit_issue_comment(cx);
                            })),
                    ),
            )
            .when(self.is_my_issue(iss), |el| {
                el.child(
                    div().mt(px(8.0)).flex().justify_end().child(
                        Button::new("uiss-delete", t!("support.menu.delete").to_string())
                            .variant(ButtonVariant::Destructive)
                            .icon(IconSource::from("trash-2"))
                            .on_click({
                                let id = iss.id.clone();
                                cx.listener(move |this, _: &ClickEvent, _, cx| {
                                    this.confirm_issue_delete = Some(id.clone());
                                    cx.notify();
                                })
                            }),
                    ),
                )
            })
    }

    /// User-mode "Demander à JeanClaude" card: a composer that files a request
    /// through Jean's Slack intake, plus a read-only recent-activity list.
    fn render_jean_ask_card(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let input_display = if self.jean_ask_input.is_empty() {
            div()
                .text_color(ShellDeckColors::text_muted())
                .child(t!("user.jean.ask_placeholder").to_string())
        } else {
            div()
                .text_color(ShellDeckColors::text_primary())
                .child(self.jean_ask_input.clone())
        };

        let mut activity = div().flex().flex_col().gap(px(2.0)).mt(px(6.0));
        if let Some(state) = &self.jean_state {
            for t in state.tickets.iter().take(10) {
                activity = activity.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.0))
                        .py(px(2.0))
                        .child(
                            div()
                                .flex_shrink_0()
                                .px(px(5.0))
                                .rounded(px(6.0))
                                .bg(ShellDeckColors::badge_bg())
                                .text_size(px(10.0))
                                .text_color(ShellDeckColors::text_muted())
                                .child(t.status.clone()),
                        )
                        .child(
                            div()
                                .flex_1()
                                .overflow_hidden()
                                .whitespace_nowrap()
                                .text_size(px(11.0))
                                .text_color(ShellDeckColors::text_muted())
                                .child(t.prompt.clone()),
                        ),
                );
            }
        }

        div()
            .m(px(16.0))
            .p(px(14.0))
            .rounded(px(12.0))
            .border_1()
            .border_color(ShellDeckColors::border())
            .bg(ShellDeckColors::bg_sidebar())
            .flex()
            .flex_col()
            .gap(px(8.0))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(lucide_icon("zap", 15.0, ShellDeckColors::primary()))
                    .child(
                        div()
                            .text_size(px(15.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(ShellDeckColors::text_primary())
                            .child(t!("user.jean.ask_title").to_string()),
                    ),
            )
            .child(
                div()
                    .id("jean-ask-input")
                    .track_focus(&self.jean_ask_focus)
                    .on_key_down(
                        cx.listener(|this, e: &KeyDownEvent, _w, cx| {
                            this.handle_jean_ask_key(e, cx)
                        }),
                    )
                    .w_full()
                    .min_h(px(56.0))
                    .px(px(10.0))
                    .py(px(8.0))
                    .rounded(px(8.0))
                    .bg(ShellDeckColors::bg_primary())
                    .border_1()
                    .border_color(ShellDeckColors::border())
                    .text_size(px(13.0))
                    .cursor_text()
                    .child(input_display),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_size(px(11.0))
                            .text_color(ShellDeckColors::text_muted())
                            .child(t!("user.jean.confirm_hint").to_string()),
                    )
                    .child(
                        div()
                            .id("jean-ask-send")
                            .flex()
                            .items_center()
                            .gap(px(6.0))
                            .px(px(12.0))
                            .py(px(7.0))
                            .rounded(px(6.0))
                            .bg(ShellDeckColors::primary())
                            .text_size(px(13.0))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(white())
                            .cursor_pointer()
                            .child(
                                svg()
                                    .path(lucide_path("send"))
                                    .size(px(12.0))
                                    .text_color(white()),
                            )
                            .child(t!("user.requests.send").to_string())
                            .on_click(
                                cx.listener(|this, _: &ClickEvent, _, cx| this.submit_jean_ask(cx)),
                            ),
                    ),
            )
            .child(
                div()
                    .text_size(px(11.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(ShellDeckColors::text_muted())
                    .mt(px(4.0))
                    .child(t!("user.jean.recent_activity").to_string()),
            )
            .child(activity)
    }
}

impl Render for Workspace {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        self.window_active = _window.is_window_active();
        _window.set_client_inset(px(5.0));

        // Drive proportional UI scaling from the App Font Size setting. Every
        // view that styles via `crate::scale::px` (i.e. rems) tracks this rem
        // size; the terminal grid and window chrome use absolute pixels and are
        // intentionally unaffected.
        {
            use crate::scale::{rem_size_for_scale, scale_for_font_size};
            let scale = scale_for_font_size(self.ui_font_size);
            _window.set_rem_size(px(rem_size_for_scale(scale)));
        }

        // Check if script editor wants to open the template browser
        if self.scripts.read(_cx).template_browser_open && self.template_browser.is_none() {
            self.scripts.update(_cx, |editor, _| {
                editor.template_browser_open = false;
            });
            self.show_template_browser(_cx);
        }

        let handle = _cx.entity().downgrade();
        let is_maximized = _window.is_maximized();

        let sidebar_resizing = self.sidebar_visible && self.sidebar.read(_cx).is_resizing();

        let output_resizing = (self.active_view == ActiveView::Scripts
            && self.scripts.read(_cx).is_output_resizing())
            || (self.active_view == ActiveView::ServerSync
                && (self.server_sync.read(_cx).is_panel_dragging()
                    || self.server_sync.read(_cx).is_log_resizing()
                    || self.server_sync.read(_cx).is_discovery_resizing()));

        // Build main content area — flex_grow fills between titlebar and status bar
        let mut main_area = div().flex().flex_grow().min_h(px(0.0)).overflow_hidden();

        // Pre-login landing: intercepts before `effective_mode()` gets a say.
        // Only shown on a fresh install (or after an explicit config wipe) —
        // once the user picks "Se connecter" (logs in) or "Continuer en
        // local" (`welcome_bypass = true`), we never come back here.
        if self.show_welcome() {
            main_area = main_area.child(self.render_welcome_screen(_cx));
            // Fall through to render titlebar + status bar chrome around
            // the welcome — no sidebar, no mode-specific children.
        } else {
            // The app mode selects the whole surface. User/Support are full-pane
            // manage surfaces (no sidebar); Dev is the classic terminal workspace.
            // Dev views (terminal sessions etc.) are hidden, never destroyed.
            match self.effective_mode() {
                AppMode::Support => {
                    main_area = main_area.child(self.support.clone());
                }
                AppMode::User => {
                    main_area = main_area.child(self.render_user_home(_cx));
                }
                AppMode::Dev => {
                    if self.sidebar_visible {
                        main_area = main_area.child(self.sidebar.clone());
                    }

                    let mut content = div().flex_grow().w_full().min_h(px(0.0)).overflow_hidden();
                    if !output_resizing && !sidebar_resizing {
                        content = content.block_mouse_except_scroll();
                    }

                    match self.active_view {
                        ActiveView::Dashboard => content = content.child(self.dashboard.clone()),
                        ActiveView::Terminal => content = content.child(self.terminal.clone()),
                        ActiveView::Scripts => content = content.child(self.scripts.clone()),
                        ActiveView::PortForwards => {
                            content = content.child(self.port_forwards.clone())
                        }
                        ActiveView::ServerSync => content = content.child(self.server_sync.clone()),
                        ActiveView::Sites => content = content.child(self.sites.clone()),
                        ActiveView::Recent => content = content.child(self.recent.clone()),
                        ActiveView::FileEditor => content = content.child(self.file_editor.clone()),
                        ActiveView::JeanConsole => content = content.child(self.jean_view.clone()),
                        ActiveView::Fleet => content = content.child(self.fleet_view.clone()),
                        ActiveView::BextCloud => content = content.child(self.bext_view.clone()),
                        ActiveView::Settings => content = content.child(self.settings.clone()),
                    }

                    main_area = main_area.child(content);
                }
            }
        } // end of `else` (not-welcome branch)

        let h1 = handle.clone();
        let h2 = handle.clone();
        let h3 = handle.clone();
        let h4 = handle.clone();
        let h5 = handle.clone();
        let h6 = handle.clone();
        let h7 = handle.clone();
        let h8 = handle.clone();
        let h9 = handle.clone();
        let h10 = handle.clone();
        let h11 = handle.clone();
        let h12 = handle.clone();
        let h13 = handle.clone();
        let h14 = handle.clone();
        let h15 = handle.clone();
        let h16 = handle.clone();
        let h17 = handle.clone();

        let mut root = div()
            .relative()
            .flex()
            .flex_col()
            .size_full()
            .bg(ShellDeckColors::bg_primary())
            .id("workspace-root")
            .track_focus(&self.focus_handle)
            .on_action(move |_: &NewTerminal, _window, cx| {
                if let Some(ws) = h1.upgrade() {
                    ws.update(cx, |ws, cx| {
                        ws.open_new_terminal(cx);
                        cx.notify();
                    });
                }
            })
            .on_action(move |_: &ToggleSidebar, _window, cx| {
                if let Some(ws) = h2.upgrade() {
                    ws.update(cx, |ws, cx| {
                        ws.toggle_sidebar(cx);
                        cx.notify();
                    });
                }
            })
            .on_action(move |_: &OpenSettings, _window, cx| {
                if let Some(ws) = h3.upgrade() {
                    ws.update(cx, |ws, cx| {
                        ws.set_active_view(ActiveView::Settings);
                        cx.notify();
                    });
                }
            })
            .on_action(move |_: &Quit, _window, cx| {
                if let Some(ws) = h4.upgrade() {
                    ws.update(cx, |ws, cx| {
                        ws.shutdown(cx);
                        cx.quit();
                    });
                }
            })
            .on_action(move |_: &ToggleCommandPalette, window, cx| {
                if let Some(ws) = h5.upgrade() {
                    ws.update(cx, |ws, cx| {
                        ws.command_palette.update(cx, |palette, cx| {
                            palette.toggle(window, cx);
                            cx.notify();
                        });
                        cx.notify();
                    });
                }
            })
            .on_action(move |_: &OpenQuickConnect, _window, cx| {
                if let Some(ws) = h6.upgrade() {
                    ws.update(cx, |ws, cx| {
                        ws.show_connection_form(None, cx);
                    });
                }
            })
            .on_action(move |_: &NextTab, _window, cx| {
                if let Some(ws) = h7.upgrade() {
                    ws.update(cx, |ws, cx| ws.next_tab(cx));
                }
            })
            .on_action(move |_: &PrevTab, _window, cx| {
                if let Some(ws) = h8.upgrade() {
                    ws.update(cx, |ws, cx| ws.prev_tab(cx));
                }
            })
            .on_action(move |_: &CloseTab, _window, cx| {
                if let Some(ws) = h9.upgrade() {
                    ws.update(cx, |ws, cx| ws.close_active_tab(cx));
                }
            })
            .on_action(move |_: &OpenTemplateBrowser, _window, cx| {
                if let Some(ws) = h10.upgrade() {
                    ws.update(cx, |ws, cx| {
                        ws.set_active_view(ActiveView::Scripts);
                        ws.show_template_browser(cx);
                        cx.notify();
                    });
                }
            })
            .on_action(move |_: &NewScript, _window, cx| {
                if let Some(ws) = h11.upgrade() {
                    ws.update(cx, |ws, cx| {
                        ws.set_active_view(ActiveView::Scripts);
                        ws.show_script_form(cx);
                        cx.notify();
                    });
                }
            })
            .on_action(move |_: &OpenServerSync, _window, cx| {
                if let Some(ws) = h12.upgrade() {
                    ws.update(cx, |ws, cx| {
                        ws.set_active_view(ActiveView::ServerSync);
                        cx.notify();
                    });
                }
            })
            .on_action(move |_: &OpenSites, _window, cx| {
                if let Some(ws) = h13.upgrade() {
                    ws.update(cx, |ws, cx| {
                        ws.set_active_view(ActiveView::Sites);
                        cx.notify();
                    });
                }
            })
            .on_action(move |_: &OpenFileEditorView, _window, cx| {
                if let Some(ws) = h14.upgrade() {
                    ws.update(cx, |ws, cx| {
                        ws.set_active_view(ActiveView::FileEditor);
                        cx.notify();
                    });
                }
            })
            .on_action(move |action: &ApplyTerminalTheme, _window, cx| {
                if let Some(ws) = h15.upgrade() {
                    let name = action.name.clone();
                    ws.update(cx, |ws, cx| {
                        ws.apply_terminal_theme_by_name(&name, cx);
                    });
                }
            })
            .on_action(move |_: &OpenRecent, _window, cx| {
                if let Some(ws) = h16.upgrade() {
                    ws.update(cx, |ws, cx| {
                        ws.activate_dev_section(SidebarSection::Recent, cx);
                    });
                }
            })
            .on_action(move |_: &OpenAiAssistant, _window, cx| {
                if let Some(ws) = h17.upgrade() {
                    ws.update(cx, |ws, cx| ws.open_ai_assistant(cx));
                }
            });

        // Apply the configured application UI font family on the root so it
        // cascades to every child view; "System Default" leaves GPUI's
        // default font untouched. (UI scale is driven by the rem size set at
        // the top of render.)
        if self.ui_font_family != "System Default" {
            root = root.font_family(self.ui_font_family.clone());
        }

        // Window chrome: clip children to the root so the custom titlebar and
        // status bar follow the window's rounded corners. When floating (not
        // maximized) draw a soft drop shadow and a 1px frame inside the 5px
        // client inset; when maximized the window is edge-to-edge with square
        // corners and no frame.
        root = root.overflow_hidden();
        if is_maximized {
            root = root.rounded(px(0.0));
        } else {
            root = root
                .rounded(px(10.0))
                .border_1()
                .border_color(ShellDeckColors::border())
                .shadow(
                    vec![BoxShadow {
                        color: hsla(0.0, 0.0, 0.0, 0.45),
                        offset: point(px(0.0), px(2.0)),
                        blur_radius: px(16.0),
                        spread_radius: px(0.0),
                        inset: false,
                    }]
                    .into(),
                );
        }

        // Sidebar resize drag
        if sidebar_resizing {
            let h_move = handle.clone();
            let h_up = handle.clone();
            root = root
                .cursor_col_resize()
                .on_mouse_move(
                    move |event: &MouseMoveEvent, _window: &mut Window, cx: &mut App| {
                        if let Some(ws) = h_move.upgrade() {
                            ws.update(cx, |ws, cx| {
                                let new_width = event.position.x.to_f64() as f32;
                                let clamped = new_width.clamp(180.0, 400.0);
                                ws.sidebar_width = clamped;
                                ws.sidebar.update(cx, |sidebar, _| {
                                    sidebar.set_width(clamped);
                                });
                                ws.terminal.update(cx, |terminal, _| {
                                    terminal.set_sidebar_width(clamped);
                                });
                                cx.notify();
                            });
                        }
                    },
                )
                .on_mouse_up(
                    MouseButton::Left,
                    move |_event: &MouseUpEvent, _window: &mut Window, cx: &mut App| {
                        if let Some(ws) = h_up.upgrade() {
                            ws.update(cx, |ws, cx| {
                                ws.sidebar.update(cx, |sidebar, _| {
                                    sidebar.stop_resizing();
                                });
                                cx.notify();
                            });
                        }
                    },
                );
        }

        // Output panel resize drag (scripts or server sync)
        if output_resizing {
            let h_move = handle.clone();
            let h_up = handle.clone();
            let is_sync_panel_drag = self.active_view == ActiveView::ServerSync
                && self.server_sync.read(_cx).is_panel_dragging();
            let is_sync_log_resize = self.active_view == ActiveView::ServerSync
                && self.server_sync.read(_cx).is_log_resizing();
            let is_sync_discovery_resize = self.active_view == ActiveView::ServerSync
                && self.server_sync.read(_cx).is_discovery_resizing();

            if is_sync_panel_drag {
                root = root.cursor_col_resize();
            } else {
                root = root.cursor_row_resize();
            }

            root = root
                .on_mouse_move(
                    move |event: &MouseMoveEvent, window: &mut Window, cx: &mut App| {
                        if let Some(ws) = h_move.upgrade() {
                            ws.update(cx, |ws, cx| {
                                if is_sync_panel_drag {
                                    let window_width = window.viewport_size().width.to_f64() as f32;
                                    let mouse_x = event.position.x.to_f64() as f32;
                                    let sidebar_w = if ws.sidebar_visible {
                                        ws.sidebar_width
                                    } else {
                                        0.0
                                    };
                                    let content_w = window_width - sidebar_w;
                                    if content_w > 0.0 {
                                        let ratio =
                                            ((mouse_x - sidebar_w) / content_w).clamp(0.2, 0.8);
                                        ws.server_sync.update(cx, |view, _| {
                                            view.panel_ratio = ratio;
                                        });
                                    }
                                } else if is_sync_log_resize {
                                    let window_height =
                                        window.viewport_size().height.to_f64() as f32;
                                    let mouse_y = event.position.y.to_f64() as f32;
                                    let new_height =
                                        (window_height - 28.0 - mouse_y).clamp(60.0, 600.0);
                                    ws.server_sync.update(cx, |view, _| {
                                        view.log_panel_height = new_height;
                                    });
                                } else if is_sync_discovery_resize {
                                    let window_height =
                                        window.viewport_size().height.to_f64() as f32;
                                    let mouse_y = event.position.y.to_f64() as f32;
                                    // Discovery panel grows upward from the bottom of the server panel
                                    let new_height =
                                        (window_height - 28.0 - mouse_y).clamp(60.0, 400.0);
                                    ws.server_sync.update(cx, |view, _| {
                                        if view.source_panel.discovery_resizing {
                                            view.source_panel.discovery_panel_height = new_height;
                                        }
                                        if view.dest_panel.discovery_resizing {
                                            view.dest_panel.discovery_panel_height = new_height;
                                        }
                                    });
                                } else {
                                    let window_height =
                                        window.viewport_size().height.to_f64() as f32;
                                    let mouse_y = event.position.y.to_f64() as f32;
                                    let new_height = window_height - 28.0 - mouse_y;
                                    ws.scripts.update(cx, |editor, _| {
                                        editor.set_output_height(new_height);
                                    });
                                }
                                cx.notify();
                            });
                        }
                    },
                )
                .on_mouse_up(
                    MouseButton::Left,
                    move |_event: &MouseUpEvent, _window: &mut Window, cx: &mut App| {
                        if let Some(ws) = h_up.upgrade() {
                            ws.update(cx, |ws, cx| {
                                ws.scripts.update(cx, |editor, _| {
                                    editor.stop_output_resizing();
                                });
                                ws.server_sync.update(cx, |view, _| {
                                    view.panel_dragging = false;
                                    view.log_panel_resizing = false;
                                    view.stop_discovery_resizing();
                                });
                                cx.notify();
                            });
                        }
                    },
                );
        }

        // Edge resize handling (when not maximized and not already resizing)
        if !is_maximized && !sidebar_resizing && !output_resizing {
            let border = px(5.0);
            root = root
                .child(
                    canvas(
                        |_bounds, window, _cx| {
                            window.insert_hitbox(
                                Bounds::new(
                                    point(px(0.0), px(0.0)),
                                    window.window_bounds().get_bounds().size,
                                ),
                                HitboxBehavior::Normal,
                            )
                        },
                        move |_bounds, hitbox, window, _cx| {
                            let mouse = window.mouse_position();
                            let size = window.window_bounds().get_bounds().size;
                            let Some(edge) = resize_edge(mouse, border, size) else {
                                return;
                            };
                            window.set_cursor_style(
                                match edge {
                                    ResizeEdge::Top | ResizeEdge::Bottom => {
                                        CursorStyle::ResizeUpDown
                                    }
                                    ResizeEdge::Left | ResizeEdge::Right => {
                                        CursorStyle::ResizeLeftRight
                                    }
                                    ResizeEdge::TopLeft | ResizeEdge::BottomRight => {
                                        CursorStyle::ResizeUpLeftDownRight
                                    }
                                    ResizeEdge::TopRight | ResizeEdge::BottomLeft => {
                                        CursorStyle::ResizeUpRightDownLeft
                                    }
                                },
                                &hitbox,
                            );
                        },
                    )
                    .size_full()
                    .absolute(),
                )
                .on_mouse_move(|_e, window, _cx| {
                    window.refresh();
                })
                .on_mouse_down(MouseButton::Left, move |e, window, _cx| {
                    let size = window.window_bounds().get_bounds().size;
                    if let Some(edge) = resize_edge(e.position, px(5.0), size) {
                        window.start_window_resize(edge);
                    }
                });
        }

        // Custom titlebar with drag area + window controls
        let titlebar = Self::render_titlebar(
            is_maximized,
            self.theme_menu_open,
            self.account_menu_open,
            self.app_config.account.clone(),
            self.account_status,
            self.site_menu_open,
            self.app_config.cloud_sync.active_site_label.clone(),
            self.site_directory.is_some(),
            if self.can_switch_mode() {
                Some(self.effective_mode())
            } else {
                None
            },
            self.ui_font_size,
            self.ai_available_for_current_surface(_cx),
            self.ai_tasks
                .iter()
                .filter(|task| {
                    task.status.is_active()
                        || matches!(task.status, AiTaskStatus::Ready | AiTaskStatus::Pending)
                })
                .count(),
            &handle,
            _cx,
        );

        root = root
            .child(titlebar)
            .child(main_area)
            .child(self.status_bar.clone());

        // Titlebar theme-switcher dropdown overlay
        if self.theme_menu_open {
            root = root.child(self.render_theme_menu(_cx));
        }

        // Titlebar account dropdown overlay
        if self.account_menu_open {
            root = root.child(self.render_account_menu(_cx));
        }

        // Titlebar site-switcher dropdown overlay
        if self.site_menu_open {
            root = root.child(self.render_site_menu(_cx));
        }

        // Sidebar kebab (⋮) row-action menu
        if let Some((conn_id, pos)) = self.sidebar_kebab_menu {
            root = root.child(self.render_sidebar_kebab_menu(conn_id, pos, _cx));
        }

        // User-mode "Mes demandes" sheets: composer + selected-request detail.
        // Both live at workspace root so they slide over the list without
        // pushing it down (their inline predecessors did the pushing).
        if matches!(self.effective_mode(), AppMode::User) {
            if self.user_new_request_sheet_open {
                root = root.child(self.render_user_new_request_sheet(_cx));
            } else if let Some(iss) = self.issue_detail.clone() {
                if self.issue_selected.as_deref() == Some(iss.id.as_str()) {
                    root = root.child(self.render_user_issue_detail_sheet(iss, _cx));
                }
            }
        }

        // Command palette overlay
        root = root.child(self.command_palette.clone());

        if let Some(sheet) = &self.ai_sheet {
            root = root.child(sheet.clone());
        }
        if let Some(sheet) = &self.ai_workflow_sheet {
            root = root.child(sheet.clone());
        }

        // Toast notification overlay
        root = root.child(self.toasts.clone());

        // Modal form overlays — render an occluding backdrop at the workspace
        // level so hover/click on elements behind is properly blocked.
        let has_modal = self.connection_form.is_some()
            || self.login_form.is_some()
            || self.onboarding.is_some()
            || self.port_forward_form.is_some()
            || self.script_form.is_some()
            || self.template_browser.is_some()
            || self.variable_prompt.is_some();

        if has_modal {
            let mut modal_layer = div()
                .id("modal-backdrop")
                .occlude()
                .absolute()
                .top_0()
                .left_0()
                .size_full();

            if let Some(ref form) = self.connection_form {
                modal_layer = modal_layer.child(form.clone());
            }
            if let Some(ref form) = self.login_form {
                modal_layer = modal_layer.child(form.clone());
            }
            if let Some(ref form) = self.onboarding {
                modal_layer = modal_layer.child(form.clone());
            }
            if let Some(ref form) = self.port_forward_form {
                modal_layer = modal_layer.child(form.clone());
            }
            if let Some(ref form) = self.script_form {
                modal_layer = modal_layer.child(form.clone());
            }
            if let Some(ref browser) = self.template_browser {
                modal_layer = modal_layer.child(browser.clone());
            }
            if let Some(ref prompt) = self.variable_prompt {
                modal_layer = modal_layer.child(prompt.clone());
            }

            root = root.child(modal_layer);
        }

        // User-mode delete-issue confirm modal (surfaces outside modal_backdrop
        // since UiDialog provides its own backdrop + occlude).
        if let Some(id) = self.confirm_issue_delete.clone() {
            root = root.child(self.render_delete_issue_modal(id, _cx));
        }

        if let Some(plan) = self.ai_action_confirmation.clone() {
            let workspace = _cx.entity().downgrade();
            let close_workspace = workspace.clone();
            root = root.child(render_ai_action_dialog(
                plan,
                move |cx| {
                    if let Some(workspace) = close_workspace.upgrade() {
                        workspace.update(cx, |workspace, cx| {
                            workspace.cancel_ai_action_confirmation(cx);
                        });
                    }
                },
                move |cx| {
                    if let Some(workspace) = workspace.upgrade() {
                        workspace.update(cx, |workspace, cx| workspace.confirm_ai_action(cx));
                    }
                },
            ));
        }

        root
    }
}
