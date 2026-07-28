use crate::i18n::rel_time;
use crate::icons::{ai_provider_badge, lucide_icon, lucide_path};
use adabraka_ui::components::icon_button::IconButton;
use adabraka_ui::components::icon_source::IconSource;
use adabraka_ui::components::input::{Input, InputSize, InputState, Paste};
use adabraka_ui::navigation::menu::{
    MenuBar as AdabrakaMenuBar, MenuBarItem, MenuItem as AdabrakaMenuItem,
};
use adabraka_ui::overlays::sheet::{Sheet, SheetSize, SheetVariant};
use adabraka_ui::prelude::{
    AnimatedCollapsible, Badge, BadgeVariant, Button, ButtonSize, ButtonVariant, Select,
    SelectOption, Spinner, SpinnerSize, SpinnerVariant, install_theme, scrollable_vertical,
    use_theme,
};
use gpui::prelude::*;
use gpui::*;
// Shadows `gpui::px` from the glob above so every length this module styles
// with is a rem, and therefore tracks the window rem size the App Font Size
// setting drives. Real-pixel call sites (window inset, rem size itself, box
// shadows, sidebar width, mouse-position math) must say `gpui::px` explicitly.
use crate::scale::px;
use shelldeck_core::ai::{
    AiActionDisposition, AiActionKind, AiActionPayload, AiActionPlan, AiActionPlanSpec,
    AiActionRisk, AiConfig, AiContext, AiIssueTriageProposal, AiSurface, AiTask, AiTaskStatus,
    AiTaskStore, ai_action_disposition, configured_cli_available, create_client, host_context,
    parse_diagnostic_plan, parse_generated_issue_draft, parse_generated_name,
    parse_issue_triage_proposal, test_connection, validate_diagnostic_command,
};
use shelldeck_core::config::activity::{
    ActivityAction, ActivityEntry, ActivityKind, ActivityStore,
};
use shelldeck_core::config::app_config::{AppConfig, CompanionConfig, ThemePreference};
use shelldeck_core::config::bext_cloud;
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
use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::ops::{DerefMut, Range};
use std::rc::Rc;
use uuid::Uuid;

use crate::ai_action_dialog::render_ai_action_dialog;
use crate::ai_assistant::{AiAssistantEvent, AiAssistantView};
use crate::ai_companion::AiCompanionEvent;
use crate::ai_workflow::{
    AiNamingKind, AiWorkflowEvent, AiWorkflowInit, AiWorkflowTarget, AiWorkflowView,
};
use crate::attachment_annotator::AttachmentAnnotator;
use crate::bext_cloud_view::{BextCloudView, BextViewEvent};
use crate::command_palette::{
    ApplyAppTheme, ApplyTerminalTheme, CommandPalette, CommandPaletteEvent, OpenManageArea,
    PaletteAction, SetAppMode, ToggleCommandPalette,
};
use crate::connection_form::{ConnectionForm, ConnectionFormEvent};
use crate::dashboard::{DashboardEvent, DashboardView};
use crate::file_editor::view::{FileEditorEvent, FileEditorView};
use crate::fleet_view::{FleetView, FleetViewEvent};
use crate::issue_attachments::{
    AttachmentDraft, AttachmentLightbox, capture_region, draft_from_clipboard_image,
    render_attachment_draft_gallery, render_stored_attachment_gallery,
};
use crate::jean_view::{JeanView, JeanViewEvent};
use crate::login_form::{LoginForm, LoginFormEvent};
use crate::onboarding_view::{OnboardingEvent, OnboardingView};
use crate::port_forward_form::PortForwardForm;
use crate::port_forward_view::{PortForwardEvent, PortForwardView};
use crate::recent_view::{RecentEvent, RecentView};
use crate::script_editor::{ScriptEditorView, ScriptEvent};
use crate::script_form::ScriptForm;
use crate::server_sync_view::{ServerSyncEvent, ServerSyncView};
use crate::settings::{
    CompanionShortcutStatuses, SettingsEvent, SettingsView, ShortcutRegistrationStatus,
};
use crate::sidebar::{SidebarEvent, SidebarSection, SidebarView};
use crate::sites_view::{SitesEvent, SitesView};
use crate::status_bar::{StatusBar, StatusBarEvent};
use crate::support_view::{
    SupportView, SupportViewEvent, issue_status_badge, priority_badge,
    render_attachment_delete_dialog, render_issue_delete_dialog,
};
use crate::t;
use crate::template_browser::TemplateBrowser;
use crate::terminal_view::{TerminalEvent, TerminalView};
use crate::theme::ShellDeckColors;
use crate::toast::{ToastContainer, ToastLevel};
use crate::variable_prompt::VariablePrompt;
use shelldeck_update::{AutoUpdateEvent, AutoUpdateStatus, AutoUpdater};

mod ai;
mod account;
mod bext;
mod chrome;
mod discovery;
mod fleet;
mod forwards;
mod jean;
mod menu;
mod modes;
mod navigation;
mod palette;
mod panels;
mod requests;
mod request_views;
mod scripts;
mod server_sync;
mod ssh;
mod sites;
mod support;
mod user_home;

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

struct NewIssueDraft {
    title: String,
    body: String,
    priority: String,
    source: &'static str,
    site_id: String,
    site_label: String,
    attachments: Vec<AttachmentDraft>,
}

/// Full-window transition shown after Manage accepts a login and while the
/// first cloud profile sync prepares the signed-in workspace.
#[derive(Debug, Clone)]
struct PostLoginSplash {
    display_name: String,
    dismissing: bool,
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

/// User-mode navigation. `Home` is the landing dashboard; the remaining tabs
/// keep sites, requests, and account/device details as focused work surfaces.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum UserHomeTab {
    #[default]
    Home,
    Sites,
    Requests,
    Infos,
}

/// Duration (ms) of the User-mode sheet enter/exit animation. The close
/// handlers use it to keep the sheet mounted while the exit tween plays,
/// then clear the backing state.
const SHEET_ANIM_MS: u64 = 300;

/// Keeps very fast logins from flashing the splash for only a frame or two.
const POST_LOGIN_SPLASH_MIN_MS: u64 = 3_000;
const POST_LOGIN_SPLASH_FADE_MS: u64 = 380;

fn post_login_splash_remaining(elapsed: std::time::Duration) -> Option<std::time::Duration> {
    std::time::Duration::from_millis(POST_LOGIN_SPLASH_MIN_MS)
        .checked_sub(elapsed)
        .filter(|remaining| !remaining.is_zero())
}

/// The wink is deliberately brief, matching a natural blink instead of
/// cross-fading constantly between the two Monolith expressions.
fn post_login_wink_opacity(delta: f32) -> f32 {
    if (0.72..=0.82).contains(&delta) {
        1.0
    } else {
        0.0
    }
}

/// A deterministic loading curve that feels like staged application startup:
/// quick discovery, slower profile preparation, then a short finalization.
/// Keeping it monotonic prevents the percentage from ever moving backwards.
fn post_login_simulated_progress(delta: f32) -> f32 {
    let delta = delta.clamp(0.0, 1.0);
    let stages = [
        (0.00, 0.00),
        (0.10, 0.18),
        (0.28, 0.39),
        (0.47, 0.56),
        (0.68, 0.79),
        (0.86, 0.93),
        (0.96, 0.98),
        (1.00, 1.00),
    ];

    for pair in stages.windows(2) {
        let (start_t, start_progress) = pair[0];
        let (end_t, end_progress) = pair[1];
        if delta <= end_t {
            let local = ((delta - start_t) / (end_t - start_t)).clamp(0.0, 1.0);
            // Smooth each stage without removing the deliberate speed changes.
            let eased = local * local * (3.0 - 2.0 * local);
            return start_progress + (end_progress - start_progress) * eased;
        }
    }

    1.0
}

fn post_login_splash_opacity(dismissing: bool, delta: f32) -> f32 {
    if dismissing {
        1.0 - delta.clamp(0.0, 1.0)
    } else {
        1.0
    }
}

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
    ai_companion_config: Rc<RefCell<AiConfig>>,
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
    post_login_splash: Option<PostLoginSplash>,
    onboarding: Option<Entity<OnboardingView>>,
    port_forward_form: Option<Entity<PortForwardForm>>,
    script_form: Option<Entity<ScriptForm>>,
    template_browser: Option<Entity<TemplateBrowser>>,
    variable_prompt: Option<Entity<VariablePrompt>>,
    active_view: ActiveView,
    /// Settings is a global personal surface, available from every app mode.
    /// Keeping this separate from `active_view` preserves the hidden Dev view
    /// (including live terminal tabs) while Settings is open.
    settings_open: bool,
    /// The application menu row. Present in every mode (and on the welcome
    /// screen); its contents are rebuilt from `menu_bar_spec` whenever the
    /// state it reads changes. See `crate::menu_bar`.
    menu_bar: Entity<AdabrakaMenuBar>,
    /// Last published global-shortcut registration statuses. Kept here so a
    /// transition into a failed state can be announced once, rather than the
    /// failure living only in the Settings pane.
    companion_shortcut_statuses: CompanionShortcutStatuses,
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
    /// Posted request image awaiting permanent deletion confirmation.
    confirm_attachment_delete: Option<(String, String)>,
    /// Native full-screen preview for images attached to the open request.
    issue_attachment_lightbox: Option<Entity<AttachmentLightbox>>,
    /// Annotation editor opened after an interactive area capture.
    issue_capture_annotator: Option<Entity<AttachmentAnnotator>>,
    _issues_poll: Option<gpui::Task<()>>,
    /// User-mode "Nouvelle demande" + comment composer states — each hosts
    /// an adabraka `Input` widget (real cursor, selection, undo). Focus is
    /// tracked by each state entity itself; no separate `issue_field` needed.
    /// `issue_body_state` runs in multi-line mode (`Input::multi_line(true)`
    /// via SDPATCH-009) so Détails behaves as a textarea.
    issue_title_state: Entity<InputState>,
    issue_body_state: Entity<InputState>,
    /// Searchable target-site picker for the New Request sheet. The selected
    /// id is resolved back through `site_directory` before submission so a
    /// stale or forged option can never reach Manage.
    issue_site_select: Entity<Select<String>>,
    issue_new_site_id: Option<String>,
    issue_comment_state: Entity<InputState>,
    issue_attachment_url_state: Entity<InputState>,
    /// Reveals the optional URL importer only when explicitly requested.
    issue_attachment_url_open: bool,
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
    /// Publishes Settings-owned companion changes back to the binary-level
    /// runtime, which owns the platform global-hotkey registrations.
    companion_config_publisher: Option<Box<dyn Fn(CompanionConfig) + Send + Sync>>,
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
    pub ai_tasks_running: usize,
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
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrayNotification {
    /// N new unread support tickets appeared since the last publish.
    NewTickets { count: usize },
    /// N new Jean fleet jobs are awaiting user confirmation.
    JeanPending { count: usize },
    /// One active SSH transport disappeared without a normal shell exit or an
    /// explicit tab close.
    SshDisconnected { name: String },
    /// A Fleet job finished. `success = false` means the executor
    /// returned a non-zero exit or an error surfaced to the toast.
    FleetJobDone { success: bool },
    /// An AI generation or executable action finished while the main window
    /// was not active.
    AiTaskDone { success: bool },
}

impl TrayNotification {
    pub fn localized_text(&self) -> (String, String) {
        match self {
            Self::NewTickets { count } => (
                t!("notification.support.summary").to_string(),
                if *count == 1 {
                    t!("notification.support.one").to_string()
                } else {
                    t!("notification.support.many", count = *count).to_string()
                },
            ),
            Self::JeanPending { count } => (
                t!("notification.jean.summary").to_string(),
                if *count == 1 {
                    t!("notification.jean.one").to_string()
                } else {
                    t!("notification.jean.many", count = *count).to_string()
                },
            ),
            Self::SshDisconnected { name } => (
                t!("notification.ssh.summary").to_string(),
                t!("notification.ssh.connection_lost", name = name).to_string(),
            ),
            Self::FleetJobDone { success } => (
                t!("notification.fleet.summary").to_string(),
                if *success {
                    t!("notification.fleet.success").to_string()
                } else {
                    t!("notification.fleet.failed").to_string()
                },
            ),
            Self::AiTaskDone { success } => (
                t!("notification.ai.summary").to_string(),
                if *success {
                    t!("notification.ai.success").to_string()
                } else {
                    t!("notification.ai.failed").to_string()
                },
            ),
        }
    }
}

impl Workspace {
    pub fn new(
        cx: &mut Context<Self>,
        config: AppConfig,
        connections: Vec<Connection>,
        store: ConnectionStore,
        ai_dock_assistant: Entity<AiAssistantView>,
        ai_tasks: Vec<AiTask>,
        ai_companion_config: Rc<RefCell<AiConfig>>,
    ) -> Self {
        crate::i18n::apply_ui_language(&config.general.ui_language);
        let issue_site_select = Self::build_issue_site_select(&[], None, cx);

        // Restore the persisted active-site filter (if any) so the sidebar
        // opens scoped to the last-selected site.
        let initial_site_filter = config
            .cloud_sync
            .active_site_id
            .as_deref()
            .and_then(|s| Uuid::parse_str(s).ok());
        let initial_pinned_connections = config.pinned_connections.clone();
        let initial_dashboard_pins = initial_pinned_connections.clone();
        let sidebar = cx.new(|cx| {
            let mut s = SidebarView::new(cx);
            s.set_connections(connections.clone());
            s.set_pinned_connections(initial_pinned_connections);
            s.set_site_filter(initial_site_filter);
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
            let menu_bar_visible = config.general.menu_bar_visible;
            terminal.update(cx, |t, _| {
                t.set_menu_bar_visible(menu_bar_visible);
                t.set_terminal_theme(&theme);
                t.set_font_size(font_size);
                t.set_font_family(font_family);
                t.set_cursor_style(&cursor_style);
                t.set_cursor_blink(cursor_blink);
                t.set_scrollback_lines(scrollback);
                // Panel + activity rail: the rail is on unless the persisted
                // "navigation collapsed" preference hides it.
                t.set_sidebar_width(initial_sidebar_width + crate::sidebar::RAIL_WIDTH);
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
        let status_bar = cx.new(|_| StatusBar::new());
        let toasts = cx.new(|_| ToastContainer::new());
        // Built empty: the real items need `&mut Context<Workspace>` for their
        // click handlers, so `rebuild_menu_bar` fills it in on first render.
        let menu_bar = cx.new(|_| {
            AdabrakaMenuBar::new(Vec::new())
                .row_height(gpui::px(crate::menu_bar::MENU_BAR_HEIGHT))
                .menu_min_width(gpui::px(240.0))
        });
        let support = cx.new(SupportView::new);
        let jean_view = cx.new(JeanView::new);
        let fleet_view = cx.new(FleetView::new);
        let bext_view = cx.new(BextCloudView::new);
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
            palette.set_actions(Self::base_palette_actions(&[], AppMode::User, false));
            palette
        });
        let companion_command_palette = cx.new(|cx| {
            let mut palette = CommandPalette::new(cx);
            palette.set_standalone(true);
            palette.set_actions(Self::base_palette_actions(&[], AppMode::User, false));
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
            ai_companion_config,
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
            post_login_splash: None,
            onboarding: None,
            port_forward_form: None,
            script_form: None,
            template_browser: None,
            variable_prompt: None,
            active_view: ActiveView::Dashboard,
            settings_open: false,
            menu_bar,
            companion_shortcut_statuses: CompanionShortcutStatuses::default(),
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
            _git_poll_task: None,
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
            user_home_tab: UserHomeTab::Home,
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
            confirm_attachment_delete: None,
            issue_attachment_lightbox: None,
            issue_capture_annotator: None,
            _issues_poll: None,
            user_new_request_sheet_open: false,
            user_new_request_sheet_dismissing: false,
            user_issue_detail_dismissing: false,
            issue_title_state: cx.new(InputState::new),
            issue_body_state: cx.new(|cx| InputState::new(cx).multi_line(true)),
            issue_site_select,
            issue_new_site_id: None,
            issue_comment_state: cx.new(InputState::new),
            issue_attachment_url_state: cx.new(InputState::new),
            issue_attachment_url_open: false,
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
            companion_config_publisher: None,
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

    /// Wire dynamic companion settings to the binary-level runtime.
    ///
    /// The UI crate persists the preference, while `main.rs` owns the native
    /// global-hotkey APIs. Keeping the bridge as a callback preserves that
    /// dependency boundary.
    pub fn set_companion_config_publisher(
        &mut self,
        publisher: Box<dyn Fn(CompanionConfig) + Send + Sync>,
    ) {
        self.companion_config_publisher = Some(publisher);
    }

    pub fn set_companion_shortcut_statuses(
        &mut self,
        statuses: CompanionShortcutStatuses,
        cx: &mut Context<Self>,
    ) {
        // A refused grab used to be indistinguishable from a working one: the
        // status landed in the Settings pane and nowhere else, so a global
        // shortcut that silently never registered just looked like a shortcut
        // that "rarely works". Announce the transition into a failed state
        // once, where the user actually is.
        for (kind, message) in shortcut_failure_toasts(&self.companion_shortcut_statuses, &statuses)
        {
            let _ = kind;
            self.show_toast(message, ToastLevel::Warning, cx);
        }
        self.companion_shortcut_statuses = statuses.clone();
        self.settings.update(cx, |settings, cx| {
            settings.set_companion_shortcut_statuses(statuses, cx);
        });
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
    /// pending). SSH transport loss is emitted by the individual session
    /// lifecycle because a counter cannot distinguish expected exits. The
    /// first publish just seeds `last_tray_counters` without notifying — otherwise a
    /// launch with existing unread tickets would spam the OS.
    ///
    /// Cheap enough (a few vec-scans + a small notify-rust dispatch on
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
        let ai_tasks_running = self
            .ai_tasks
            .iter()
            .filter(|task| task.status.is_running())
            .count();
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
            ai_tasks_running,
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
                if *section == SidebarSection::Settings {
                    self.open_settings(cx);
                    return;
                }
                self.settings_open = false;
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
                // The terminal is offset by rail + panel, not the panel alone.
                let total = self.sidebar.read(cx).total_width();
                self.terminal.update(cx, |terminal, _cx| {
                    terminal.set_sidebar_width(total);
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
            SidebarEvent::PanelItemSelected { section, id } => {
                self.handle_panel_item_selected(*section, *id, cx);
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
            SettingsEvent::CloseRequested => {
                self.settings_open = false;
                self.activate_current_mode(cx);
                cx.notify();
            }
            SettingsEvent::ConfigChanged(config) => {
                tracing::info!("Config changed, applying settings");
                let companion_changed = self.app_config.companion != config.companion;
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
                *self.ai_companion_config.borrow_mut() = config.ai.clone();
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
                // Apply sidebar width (panel + activity rail).
                self.sidebar_width = self.app_config.general.sidebar_width;
                let total = self.sidebar.read(cx).total_width();
                self.terminal.update(cx, |terminal, _cx| {
                    terminal.set_sidebar_width(total);
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
                self.publish_tray_state(cx);
                if companion_changed {
                    if let Some(publisher) = self.companion_config_publisher.as_ref() {
                        publisher(self.app_config.companion.clone());
                    }
                }
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
                if !self.enter_dev_mode(cx) {
                    return;
                }
                let Some(id) = entry
                    .target_id
                    .as_deref()
                    .and_then(|id| Uuid::parse_str(id).ok())
                else {
                    return;
                };
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
                if !self.can_access_mode(AppMode::Support) {
                    return;
                }
                self.set_mode(AppMode::Support, cx);
                self.refresh_support(cx);
                cx.notify();
            }
            ActivityAction::OpenTicket => {
                if !self.can_access_mode(AppMode::Support) {
                    return;
                }
                self.set_mode(AppMode::Support, cx);
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
        if !self.enter_dev_mode(cx) {
            return;
        }
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
        self.start_cloud_sync(true, cx);
    }

    /// Run the configured startup sync after the main Workspace exists.
    ///
    /// Unlike the manual action, this does not announce a redundant
    /// "started" toast; completion and failures remain visible.
    pub fn cloud_sync_on_startup(&mut self, cx: &mut Context<Self>) {
        if self.app_config.cloud_sync.sync_on_startup {
            self.start_cloud_sync(false, cx);
        }
    }

    fn start_cloud_sync(&mut self, announce_started: bool, cx: &mut Context<Self>) {
        let cfg = self.app_config.cloud_sync.clone();
        if !cfg.is_configured() {
            if announce_started {
                self.show_toast(
                    t!("toast.cloud_sync.not_configured").to_string(),
                    ToastLevel::Warning,
                    cx,
                );
            }
            return;
        }

        if announce_started {
            self.show_toast(
                t!("toast.cloud_sync.started").to_string(),
                ToastLevel::Info,
                cx,
            );
        }
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

/// Which companion shortcut a failure toast is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShortcutToastKind {
    AiDock,
    CommandPalette,
}

/// Whether a registration status means the shortcut will **not** fire while
/// ShellDeck is unfocused.
///
/// `Applying` and `PendingPortal` are in-flight, not failures — the Wayland
/// portal answers asynchronously, and toasting mid-flight would fire on every
/// launch. `Disabled` is the user's own choice.
fn shortcut_status_is_failure(status: &ShortcutRegistrationStatus) -> bool {
    matches!(
        status,
        ShortcutRegistrationStatus::Conflict | ShortcutRegistrationStatus::Error(_)
    )
}

/// Failure toasts to show for a status transition.
///
/// Only *entering* a failed state produces a toast, so a status republished
/// unchanged (the companion config channel re-syncs on every settings save)
/// does not re-toast. Pure so the transition logic is testable without GPUI.
fn shortcut_failure_toasts(
    previous: &CompanionShortcutStatuses,
    next: &CompanionShortcutStatuses,
) -> Vec<(ShortcutToastKind, String)> {
    let mut toasts = Vec::new();
    let mut check = |kind: ShortcutToastKind,
                     before: &ShortcutRegistrationStatus,
                     after: &ShortcutRegistrationStatus,
                     label: &str| {
        if !shortcut_status_is_failure(after) || before == after {
            return;
        }
        let reason = match after {
            ShortcutRegistrationStatus::Conflict => t!("shortcut.failure.conflict").to_string(),
            ShortcutRegistrationStatus::Error(error) => {
                if crate::settings::shortcut_error_is_portal_missing(error) {
                    t!("shortcut.failure.portal_missing").to_string()
                } else {
                    error.clone()
                }
            }
            _ => return,
        };
        toasts.push((
            kind,
            t!(
                "shortcut.failure.toast",
                shortcut = label.to_string(),
                reason = reason
            )
            .to_string(),
        ));
    };
    check(
        ShortcutToastKind::AiDock,
        &previous.ai_dock,
        &next.ai_dock,
        &t!("shortcut.name.ai_dock"),
    );
    check(
        ShortcutToastKind::CommandPalette,
        &previous.command_palette,
        &next.command_palette,
        &t!("shortcut.name.command_palette"),
    );
    toasts
}

/// Deterministic stand-in id for a Manage site whose `site_id` is not a UUID.
///
/// Sidebar rows need a stable id for their `ElementId` and for selection
/// matching. Manage sends `site_id` as a string that is normally a UUID; when
/// it is not, hashing the id and name keeps the row stable across renders
/// instead of it churning an id every frame. Not a real v5 UUID — the `uuid`
/// crate is built with `v4` + `serde` only — and never persisted.
fn uuid_from_key(site_id: &str, name: &str) -> Uuid {
    use std::hash::{Hash, Hasher};

    let mut hi = std::collections::hash_map::DefaultHasher::new();
    site_id.hash(&mut hi);
    name.hash(&mut hi);
    let mut lo = std::collections::hash_map::DefaultHasher::new();
    name.hash(&mut lo);
    site_id.hash(&mut lo);
    Uuid::from_u128(((hi.finish() as u128) << 64) | lo.finish() as u128)
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

    fn render_post_login_splash(&self, splash: &PostLoginSplash) -> impl IntoElement {
        use std::time::Duration;

        let mascot = div()
            .relative()
            .flex_shrink_0()
            .w(px(188.0))
            .h(px(188.0))
            .child(
                img("images/brand/svg/expressions/dark-default-logo.svg")
                    .w_full()
                    .h_full()
                    .object_fit(ObjectFit::Contain),
            )
            .child(
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full()
                    .child(
                        img("images/brand/svg/expressions/dark-wink-logo.svg")
                            .w_full()
                            .h_full()
                            .object_fit(ObjectFit::Contain),
                    )
                    .with_animation(
                        "post-login-mascot-wink",
                        Animation::new(Duration::from_millis(4_200)).repeat(),
                        |el, delta| el.opacity(post_login_wink_opacity(delta)),
                    ),
            )
            .with_animation(
                "post-login-mascot-float",
                Animation::new(Duration::from_millis(2_600))
                    .repeat()
                    .with_easing(ease_in_out),
                |el, delta| {
                    let y = (delta * std::f32::consts::TAU).sin() * 5.0;
                    el.top(px(y))
                },
            );

        let progress_bar = div()
            .relative()
            .w(px(220.0))
            .h(px(5.0))
            .overflow_hidden()
            .rounded_full()
            .bg(ShellDeckColors::primary().opacity(0.14))
            .child(
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .h_full()
                    .w(px(0.0))
                    .rounded_full()
                    .bg(ShellDeckColors::primary())
                    .with_animation(
                        "post-login-progress-bar",
                        Animation::new(Duration::from_millis(POST_LOGIN_SPLASH_MIN_MS)),
                        |el, delta| el.w(px(220.0 * post_login_simulated_progress(delta))),
                    ),
            );

        let progress_percentage = div()
            .min_w(px(34.0))
            .text_right()
            .text_size(px(11.0))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(ShellDeckColors::primary())
            .with_animation(
                "post-login-progress-percentage",
                Animation::new(Duration::from_millis(POST_LOGIN_SPLASH_MIN_MS)),
                |el, delta| {
                    let percentage = (post_login_simulated_progress(delta) * 100.0).round() as u8;
                    el.child(format!("{percentage}%"))
                },
            );

        div()
            .id("post-login-splash")
            .occlude()
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .overflow_hidden()
            .bg(ShellDeckColors::bg_primary())
            .on_mouse_down(MouseButton::Left, |_event, _window, cx| {
                cx.stop_propagation();
            })
            .child(
                div()
                    .absolute()
                    .w(px(420.0))
                    .h(px(420.0))
                    .rounded_full()
                    .bg(ShellDeckColors::primary().opacity(0.07)),
            )
            .child(
                div()
                    .relative()
                    .flex()
                    .flex_col()
                    .items_center()
                    .w_full()
                    .max_w(px(480.0))
                    .px(px(28.0))
                    .child(mascot)
                    .child(
                        div()
                            .mt(px(22.0))
                            .text_size(px(25.0))
                            .font_weight(FontWeight::BOLD)
                            .text_color(ShellDeckColors::text_primary())
                            .text_center()
                            .child(
                                t!(
                                    "account.splash.welcome",
                                    name = splash.display_name.as_str()
                                )
                                .to_string(),
                            ),
                    )
                    .child(
                        div()
                            .mt(px(8.0))
                            .text_size(px(14.0))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(ShellDeckColors::text_muted())
                            .text_center()
                            .child(t!("account.splash.preparing").to_string()),
                    )
                    .child(
                        div()
                            .mt(px(22.0))
                            .flex()
                            .flex_col()
                            .items_center()
                            .gap(px(8.0))
                            .child(
                                div()
                                    .w(px(220.0))
                                    .flex()
                                    .items_center()
                                    .justify_between()
                                    .text_size(px(11.0))
                                    .text_color(ShellDeckColors::text_muted())
                                    .child(t!("account.splash.syncing").to_string())
                                    .child(progress_percentage),
                            )
                            .child(progress_bar),
                    ),
            )
            .with_animation(
                SharedString::from(format!(
                    "post-login-splash-{}",
                    if splash.dismissing {
                        "fade-out"
                    } else {
                        "visible"
                    }
                )),
                Animation::new(Duration::from_millis(POST_LOGIN_SPLASH_FADE_MS))
                    .with_easing(ease_in_out),
                {
                    let dismissing = splash.dismissing;
                    move |el, delta| el.opacity(post_login_splash_opacity(dismissing, delta))
                },
            )
    }
}

impl Render for Workspace {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        self.window_active = _window.is_window_active();
        // Window chrome geometry is in real device-independent pixels.
        _window.set_client_inset(gpui::px(5.0));

        // Drive proportional UI scaling from the App Font Size setting. Every
        // view that styles via `crate::scale::px` (i.e. rems) tracks this rem
        // size; the terminal grid and window chrome use absolute pixels and are
        // intentionally unaffected.
        {
            use crate::scale::{rem_size_for_scale, scale_for_font_size};
            let scale = scale_for_font_size(self.ui_font_size);
            // The rem size itself is necessarily absolute — it is the unit
            // every `crate::scale::px` above resolves against.
            _window.set_rem_size(gpui::px(rem_size_for_scale(scale)));
        }

        // The menu row reads a dozen pieces of state (mode, sign-in, sidebar,
        // Jean/Fleet availability, AI); rebuilding it here keeps it honest
        // without a subscription per input.
        self.rebuild_menu_bar(_cx);
        // Same reasoning: the contextual panel reads several live entities
        // with no shared change signal.
        if self.effective_mode() == AppMode::Dev {
            self.refresh_sidebar_panels(_cx);
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
        // There is no guest/local bypass; logout returns here as well.
        if self.show_welcome() {
            main_area = main_area.child(self.render_welcome_screen(_cx));
            // Fall through to render titlebar + status bar chrome around
            // the welcome — no sidebar, no mode-specific children.
        } else if self.settings_open {
            main_area = main_area.child(self.settings.clone());
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
                    // Always rendered: the activity rail stays on screen even
                    // when the panel is collapsed (VS Code layout). The
                    // sidebar itself decides what to draw from its own
                    // collapsed / nav-collapsed state.
                    main_area = main_area.child(self.sidebar.clone());

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
                        ws.open_settings(cx);
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
        // maximized) draw a 1px frame inside the 5px client inset; when
        // maximized the window is edge-to-edge with square corners and no
        // frame. The floating radius matches the standard Card component.
        root = root.overflow_hidden();
        if is_maximized {
            root = root.rounded(px(0.0));
        } else {
            root = root
                .rounded(use_theme().tokens.radius_lg)
                .border_1()
                .border_color(ShellDeckColors::border());
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
                                // The pointer is in window space; the panel
                                // starts to the right of the activity rail.
                                let rail = ws.sidebar.read(cx).rail_offset();
                                let new_width = event.position.x.to_f64() as f32 - rail;
                                let clamped = new_width.clamp(180.0, 400.0);
                                ws.sidebar_width = clamped;
                                let total = ws.sidebar.update(cx, |sidebar, _| {
                                    sidebar.set_width(clamped);
                                    sidebar.total_width()
                                });
                                ws.terminal.update(cx, |terminal, _| {
                                    terminal.set_sidebar_width(total);
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
                                    // Rail + panel: the content area starts to
                                    // the right of both.
                                    let sidebar_w = ws.sidebar.read(cx).total_width();
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
            // Window-edge resize hit-testing works in real screen pixels.
            let border = gpui::px(5.0);
            root = root
                .child(
                    canvas(
                        |_bounds, window, _cx| {
                            window.insert_hitbox(
                                Bounds::new(
                                    point(gpui::px(0.0), gpui::px(0.0)),
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
                    if let Some(edge) = resize_edge(e.position, gpui::px(5.0), size) {
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
                Some((self.effective_mode(), self.allowed_modes()))
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

        // The application menu row sits between the titlebar and the content
        // in every mode, including the pre-login welcome screen (where
        // `menu_bar_spec` reduces it to sign-in / quit / zoom / about).
        root = root.child(titlebar);
        if self.app_config.general.menu_bar_visible {
            root = root.child(self.menu_bar.clone());
        }
        root = root.child(main_area).child(self.status_bar.clone());

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
        if !self.settings_open && matches!(self.effective_mode(), AppMode::User) {
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
        if let Some((issue_id, attachment_id)) = self.confirm_attachment_delete.clone() {
            root = root.child(self.render_delete_attachment_modal(issue_id, attachment_id, _cx));
        }

        if let Some(lightbox) = &self.issue_attachment_lightbox {
            root = root.child(lightbox.clone());
        }

        if let Some(annotator) = &self.issue_capture_annotator {
            root = root.child(annotator.clone());
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

        // The post-login transition is intentionally last: it covers window
        // chrome, toasts and first-run onboarding until the initial sync has
        // produced a coherent signed-in workspace.
        if let Some(splash) = &self.post_login_splash {
            root = root.child(self.render_post_login_splash(splash));
        }

        root
    }
}

#[cfg(test)]
mod tests {
    use super::{
        POST_LOGIN_SPLASH_MIN_MS, ShortcutToastKind, post_login_splash_remaining,
        post_login_simulated_progress, post_login_splash_opacity, post_login_wink_opacity,
        shortcut_failure_toasts, shortcut_status_is_failure,
    };
    use crate::settings::{CompanionShortcutStatuses, ShortcutRegistrationStatus};
    use crate::t;

    fn statuses(
        ai_dock: ShortcutRegistrationStatus,
        command_palette: ShortcutRegistrationStatus,
    ) -> CompanionShortcutStatuses {
        CompanionShortcutStatuses {
            ai_dock,
            command_palette,
        }
    }

    #[test]
    fn post_login_splash_has_a_minimum_visible_duration() {
        let early = post_login_splash_remaining(std::time::Duration::from_millis(200))
            .expect("a fast sync should keep the splash visible");
        assert_eq!(
            early,
            std::time::Duration::from_millis(POST_LOGIN_SPLASH_MIN_MS - 200)
        );
        assert!(
            post_login_splash_remaining(std::time::Duration::from_millis(
                POST_LOGIN_SPLASH_MIN_MS
            ))
            .is_none()
        );
    }

    #[test]
    fn post_login_mascot_only_winks_briefly() {
        assert_eq!(post_login_wink_opacity(0.2), 0.0);
        assert_eq!(post_login_wink_opacity(0.75), 1.0);
        assert_eq!(post_login_wink_opacity(0.9), 0.0);
    }

    #[test]
    fn post_login_simulated_progress_is_staged_and_monotonic() {
        let checkpoints: Vec<f32> = (0..=100)
            .map(|step| post_login_simulated_progress(step as f32 / 100.0))
            .collect();
        assert_eq!(checkpoints[0], 0.0);
        assert_eq!(checkpoints[100], 1.0);
        assert!(
            checkpoints
                .windows(2)
                .all(|pair| pair[1] >= pair[0]),
            "simulated progress must never move backwards"
        );
        assert!(post_login_simulated_progress(0.1) >= 0.17);
        assert!(post_login_simulated_progress(0.86) < 0.95);
    }

    #[test]
    fn post_login_splash_only_fades_when_dismissing() {
        assert_eq!(post_login_splash_opacity(false, 0.75), 1.0);
        assert_eq!(post_login_splash_opacity(true, 0.0), 1.0);
        assert_eq!(post_login_splash_opacity(true, 0.5), 0.5);
        assert_eq!(post_login_splash_opacity(true, 1.0), 0.0);
    }

    // SDTEST-1415 — only a genuinely failed registration counts. `Applying`
    // and `PendingPortal` are in-flight: the Wayland GlobalShortcuts portal
    // answers asynchronously, so treating either as a failure would toast on
    // every single launch before the compositor has replied. `Disabled` is the
    // user's own choice and never a failure.
    #[test]
    fn only_conflict_and_error_count_as_failures() {
        assert!(shortcut_status_is_failure(
            &ShortcutRegistrationStatus::Conflict
        ));
        assert!(shortcut_status_is_failure(
            &ShortcutRegistrationStatus::Error("BadAccess".into())
        ));

        for in_flight in [
            ShortcutRegistrationStatus::Disabled,
            ShortcutRegistrationStatus::Applying,
            ShortcutRegistrationStatus::Registered,
            ShortcutRegistrationStatus::PendingPortal,
        ] {
            assert!(
                !shortcut_status_is_failure(&in_flight),
                "{in_flight:?} must not toast"
            );
        }
    }

    // SDTEST-1416 — the companion config channel republishes statuses on every
    // settings save, so an unchanged failure must stay silent. Only the
    // transition *into* a failure is announced, or a user with a permanently
    // conflicting shortcut gets a toast every time they touch Settings.
    #[test]
    fn only_the_transition_into_failure_toasts() {
        let ok = statuses(
            ShortcutRegistrationStatus::Registered,
            ShortcutRegistrationStatus::Registered,
        );
        let failed = statuses(
            ShortcutRegistrationStatus::Error("BadAccess".into()),
            ShortcutRegistrationStatus::Registered,
        );

        // Entering the failure announces it once.
        let toasts = shortcut_failure_toasts(&ok, &failed);
        assert_eq!(toasts.len(), 1);
        assert_eq!(toasts[0].0, ShortcutToastKind::AiDock);
        // The platform's own reason must reach the user, not be swallowed.
        assert!(toasts[0].1.contains("BadAccess"));

        // Republishing the same failure is silent.
        assert!(shortcut_failure_toasts(&failed, &failed).is_empty());
        // Recovering is silent too — nothing to warn about.
        assert!(shortcut_failure_toasts(&failed, &ok).is_empty());
    }

    // SDTEST-1417 — the two shortcuts are independent; one failing must not
    // mask or duplicate the other, and both failing at once reports both.
    #[test]
    fn each_shortcut_reports_independently() {
        let ok = statuses(
            ShortcutRegistrationStatus::Registered,
            ShortcutRegistrationStatus::Registered,
        );
        let both = statuses(
            ShortcutRegistrationStatus::Error("BadAccess".into()),
            ShortcutRegistrationStatus::Conflict,
        );

        let toasts = shortcut_failure_toasts(&ok, &both);
        assert_eq!(toasts.len(), 2);
        let kinds: Vec<_> = toasts.iter().map(|(kind, _)| *kind).collect();
        assert!(kinds.contains(&ShortcutToastKind::AiDock));
        assert!(kinds.contains(&ShortcutToastKind::CommandPalette));

        // Palette-only failure does not mention the dock.
        let palette_only = statuses(
            ShortcutRegistrationStatus::Registered,
            ShortcutRegistrationStatus::Conflict,
        );
        let toasts = shortcut_failure_toasts(&ok, &palette_only);
        assert_eq!(toasts.len(), 1);
        assert_eq!(toasts[0].0, ShortcutToastKind::CommandPalette);
    }

    // SDTEST-1418 — a Wayland session without the Global Shortcuts portal is
    // the one failure that is environmental rather than a bad key choice, and
    // ashpd reports it as an English D-Bus sentence. It must reach the user as
    // the translated explanation; every other platform error still arrives
    // verbatim, since we cannot guess what those mean.
    #[test]
    fn portal_absence_is_explained_but_other_errors_pass_through() {
        let ok = statuses(
            ShortcutRegistrationStatus::Registered,
            ShortcutRegistrationStatus::Registered,
        );
        let portal_missing = statuses(
            ShortcutRegistrationStatus::Error(
                "A portal frontend implementing `org.freedesktop.portal.GlobalShortcuts` \
                 was not found"
                    .into(),
            ),
            ShortcutRegistrationStatus::Registered,
        );

        let toasts = shortcut_failure_toasts(&ok, &portal_missing);
        assert_eq!(toasts.len(), 1);
        assert!(
            toasts[0]
                .1
                .contains(&t!("shortcut.failure.portal_missing").to_string())
        );
        assert!(!toasts[0].1.contains("portal frontend"));

        let other = statuses(
            ShortcutRegistrationStatus::Error(
                "Could not resolve keycode for key: nosuchkey".into(),
            ),
            ShortcutRegistrationStatus::Registered,
        );
        let toasts = shortcut_failure_toasts(&ok, &other);
        assert!(toasts[0].1.contains("Could not resolve keycode"));
    }
}
