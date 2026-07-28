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
mod scripts;
mod server_sync;
mod ssh;
mod sites;
mod support;

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
        mode_switch: Option<(AppMode, &'static [AppMode])>,
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

        // Settings is a personal surface shared by User, Support and Dev.
        let settings_btn = account.as_ref().map(|_| {
            let settings_handle = handle.clone();
            IconButton::new("settings")
                .variant(ButtonVariant::Ghost)
                .size(gpui::px(28.0))
                .icon_size(gpui::px(14.0))
                .on_click(move |_, _, cx| {
                    if let Some(ws) = settings_handle.upgrade() {
                        ws.update(cx, |ws, cx| ws.open_settings(cx));
                    }
                })
        });

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

        // Mode switcher — only the exact modes granted to this account.
        let mode_switcher = mode_switch.map(|(current, allowed_modes)| {
            let mut seg = div()
                .flex()
                .items_center()
                .gap(px(1.0))
                .p(px(2.0))
                .rounded(px(6.0))
                .bg(ShellDeckColors::badge_bg());
            for &m in allowed_modes {
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

        let mut titlebar = div()
            .flex()
            .items_center()
            .w_full()
            .flex_shrink_0()
            .h(px(40.0))
            .bg(titlebar_bg);
        // Rounded clipping does not propagate to child backgrounds in GPUI.
        // This element owns the titlebar background, so it must own the
        // floating window's top radius as well.
        if !is_maximized {
            titlebar = titlebar.rounded_t(use_theme().tokens.radius_lg);
        }
        titlebar
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
                    .children(settings_btn)
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
                    // BoxShadow fields are typed `Pixels` — real pixels, not rems.
                    offset: point(gpui::px(0.0), gpui::px(4.0)),
                    blur_radius: gpui::px(20.0),
                    spread_radius: gpui::px(0.0),
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
            // BoxShadow fields are typed `Pixels` — real pixels, not rems.
            offset: point(gpui::px(0.0), gpui::px(4.0)),
            blur_radius: gpui::px(20.0),
            spread_radius: gpui::px(0.0),
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
            // BoxShadow fields are typed `Pixels` — real pixels, not rems.
            offset: point(gpui::px(0.0), gpui::px(4.0)),
            blur_radius: gpui::px(20.0),
            spread_radius: gpui::px(0.0),
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
            // BoxShadow fields are typed `Pixels` — real pixels, not rems.
            offset: point(gpui::px(0.0), gpui::px(4.0)),
            blur_radius: gpui::px(16.0),
            spread_radius: gpui::px(0.0),
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

    /// Tab bar for the User-mode home. Same visual shape as
    /// `SupportView::render_section_tabs`
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
            .h(px(26.0))
            .px(px(10.0))
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
                t!("user.tabs.home").to_string(),
                "house",
                UserHomeTab::Home,
                current,
                cx,
            ))
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

    fn render_user_overview(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let sites = self
            .site_directory
            .as_ref()
            .map(|payload| payload.sites.len())
            .unwrap_or(0);
        let open_requests = self
            .issues_list
            .iter()
            .filter(|issue| !matches!(issue.status.as_str(), "closed" | "resolved"))
            .count();

        let stat = |icon: &'static str, value: usize, label: String| {
            adabraka_ui::display::card::Card::new()
                .content(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(12.0))
                        .child(
                            div()
                                .size(px(38.0))
                                .rounded(px(10.0))
                                .bg(ShellDeckColors::primary().opacity(0.12))
                                .flex()
                                .items_center()
                                .justify_center()
                                .child(lucide_icon(icon, 18.0, ShellDeckColors::primary())),
                        )
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .child(
                                    div()
                                        .text_size(px(24.0))
                                        .font_weight(FontWeight::BOLD)
                                        .text_color(ShellDeckColors::text_primary())
                                        .child(value.to_string()),
                                )
                                .child(
                                    div()
                                        .text_size(px(12.0))
                                        .text_color(ShellDeckColors::text_muted())
                                        .child(label),
                                ),
                        ),
                )
                .min_w(px(180.0))
                .flex_1()
        };

        let entity = cx.entity();
        let sites_action = Button::new("home-open-sites", t!("user.home.open_sites").to_string())
            .variant(ButtonVariant::Outline)
            .icon(IconSource::from("globe"))
            .on_click(move |_, _, cx| {
                entity.update(cx, |this, cx| {
                    this.user_home_tab = UserHomeTab::Sites;
                    cx.notify();
                });
            });
        let entity = cx.entity();
        let requests_action = Button::new(
            "home-open-requests",
            t!("user.home.open_requests").to_string(),
        )
        .variant(ButtonVariant::Outline)
        .icon(IconSource::from("tag"))
        .on_click(move |_, _, cx| {
            entity.update(cx, |this, cx| {
                this.user_home_tab = UserHomeTab::Requests;
                cx.notify();
            });
        });
        let entity = cx.entity();
        let new_request = Button::new("home-new-request", t!("user.home.new_request").to_string())
            .icon(IconSource::from("plus"))
            .on_click(move |_, _, cx| {
                entity.update(cx, |this, cx| this.open_new_request(cx));
            });

        let recent_requests = if self.issues_list.is_empty() {
            div()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap(px(8.0))
                .min_h(px(132.0))
                .child(lucide_icon("inbox", 24.0, ShellDeckColors::text_muted()))
                .child(
                    div()
                        .text_size(px(13.0))
                        .text_color(ShellDeckColors::text_muted())
                        .child(t!("user.home.recent_empty").to_string()),
                )
                .into_any_element()
        } else {
            let rows = self
                .issues_list
                .iter()
                .take(3)
                .cloned()
                .enumerate()
                .map(|(index, issue)| {
                    let issue_id = issue.id.clone();
                    let entity = cx.entity();
                    let updated = rel_time(issue.updated_at);
                    div()
                        .id(("home-recent-request", index))
                        .flex()
                        .items_center()
                        .gap(px(10.0))
                        .px(px(2.0))
                        .py(px(10.0))
                        .border_b_1()
                        .border_color(ShellDeckColors::border().opacity(0.65))
                        .cursor_pointer()
                        .hover(|style| style.bg(ShellDeckColors::hover_bg()))
                        .on_click(move |_, _, cx| {
                            entity.update(cx, |this, cx| {
                                this.user_home_tab = UserHomeTab::Requests;
                                this.select_issue(issue_id.clone(), cx);
                                cx.notify();
                            });
                        })
                        .child(
                            div()
                                .size(px(30.0))
                                .rounded(px(8.0))
                                .bg(ShellDeckColors::primary().opacity(0.10))
                                .flex()
                                .items_center()
                                .justify_center()
                                .flex_shrink_0()
                                .child(lucide_icon("inbox", 14.0, ShellDeckColors::primary())),
                        )
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .gap(px(3.0))
                                .min_w(px(0.0))
                                .flex_1()
                                .child(
                                    div()
                                        .text_size(px(13.0))
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(ShellDeckColors::text_primary())
                                        .overflow_hidden()
                                        .child(issue.title),
                                )
                                .child(
                                    div()
                                        .text_size(px(11.0))
                                        .text_color(ShellDeckColors::text_muted())
                                        .child(updated),
                                ),
                        )
                        .child(issue_status_badge(&issue.status))
                })
                .collect::<Vec<_>>();
            div().flex().flex_col().children(rows).into_any_element()
        };

        let account_status = match self.account_status {
            AccountStatus::Ok => t!("user.home.status.connected").to_string(),
            AccountStatus::Rejected => t!("user.home.status.expired").to_string(),
            AccountStatus::Offline => t!("user.home.status.offline").to_string(),
            AccountStatus::Unknown => t!("user.home.status.checking").to_string(),
        };
        let active_site = self
            .app_config
            .cloud_sync
            .active_site_label
            .clone()
            .filter(|label| !label.trim().is_empty())
            .unwrap_or_else(|| t!("user.home.no_active_site").to_string());
        let status_row = |icon: &'static str, label: String, value: String, color: Hsla| {
            div()
                .flex()
                .items_center()
                .gap(px(10.0))
                .py(px(7.0))
                .child(
                    div()
                        .size(px(28.0))
                        .rounded(px(7.0))
                        .bg(color.opacity(0.10))
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(lucide_icon(icon, 13.0, color)),
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
                                .text_color(ShellDeckColors::text_muted())
                                .child(label),
                        )
                        .child(
                            div()
                                .text_size(px(13.0))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(ShellDeckColors::text_primary())
                                .overflow_hidden()
                                .child(value),
                        ),
                )
        };
        let entity = cx.entity();
        let sync_action = Button::new("home-sync", t!("user.home.sync").to_string())
            .variant(ButtonVariant::Outline)
            .size(ButtonSize::Sm)
            .icon(IconSource::from("refresh-cw"))
            .on_click(move |_, _, cx| {
                entity.update(cx, |this, cx| this.cloud_sync_now(cx));
            });

        div()
            .id("user-overview-scroll")
            .flex()
            .flex_col()
            .flex_1()
            .overflow_y_scroll()
            .gap(px(16.0))
            .p(px(16.0))
            .child(
                div()
                    .relative()
                    .w_full()
                    .h(px(132.0))
                    .flex_shrink_0()
                    .overflow_hidden()
                    .rounded(use_theme().tokens.radius_lg)
                    .border_1()
                    .border_color(ShellDeckColors::primary().opacity(0.40))
                    // Match the surrounding page so GPUI's rectangular
                    // background paint cannot show behind the curved border.
                    // The dark artwork itself stays safely inset below.
                    .bg(ShellDeckColors::bg_primary())
                    .child(
                        img("images/home/user-dashboard-colorful-watermark-v2.webp")
                            .absolute()
                            .inset_0()
                            .size_full()
                            // The asset is exported at this exact aspect ratio
                            // with its gradient and Card-equivalent alpha
                            // corners baked in, so GPUI has nothing to mask.
                            .object_fit(ObjectFit::Fill),
                    )
                    .child(
                        div()
                            .relative()
                            .ml_auto()
                            .w(relative(0.52))
                            .h_full()
                            .flex()
                            .flex_col()
                            .justify_center()
                            .items_start()
                            .gap(px(7.0))
                            .px(px(24.0))
                            .child(
                                div()
                                    .px(px(8.0))
                                    .py(px(3.0))
                                    .rounded_full()
                                    .bg(ShellDeckColors::primary().opacity(0.22))
                                    .border_1()
                                    .border_color(ShellDeckColors::primary().opacity(0.45))
                                    .text_size(px(10.0))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(hsla(0.47, 0.78, 0.72, 1.0))
                                    .child(
                                        t!("user.home.directory_count", count = sites).to_string(),
                                    ),
                            )
                            .child(
                                div()
                                    .text_size(px(21.0))
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(white())
                                    .child(t!("user.home.title").to_string()),
                            )
                            .child(
                                div()
                                    .max_w(px(430.0))
                                    .text_size(px(12.0))
                                    .text_color(white().opacity(0.72))
                                    .child(t!("user.home.subtitle").to_string()),
                            ),
                    ),
            )
            .child(
                div()
                    .flex()
                    .gap(px(12.0))
                    .child(stat("globe", sites, t!("user.home.sites").to_string()))
                    .child(stat(
                        "inbox",
                        open_requests,
                        t!("user.home.open_requests_count").to_string(),
                    )),
            )
            .child(
                adabraka_ui::display::card::Card::new().content(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap(px(12.0))
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .gap(px(4.0))
                                .child(
                                    div()
                                        .text_size(px(14.0))
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(ShellDeckColors::text_primary())
                                        .child(t!("user.home.quick_actions").to_string()),
                                )
                                .child(
                                    div()
                                        .text_size(px(12.0))
                                        .text_color(ShellDeckColors::text_muted())
                                        .child(t!("user.home.quick_actions_hint").to_string()),
                                ),
                        )
                        .child(
                            div()
                                .flex()
                                .flex_wrap()
                                .justify_end()
                                .gap(px(8.0))
                                .child(sites_action)
                                .child(requests_action)
                                .child(new_request),
                        ),
                ),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_start()
                    .gap(px(12.0))
                    .child(
                        adabraka_ui::display::card::Card::new()
                            .content(
                                div()
                                    .flex()
                                    .flex_col()
                                    .child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .justify_between()
                                            .gap(px(8.0))
                                            .pb(px(6.0))
                                            .child(
                                                div()
                                                    .flex()
                                                    .items_center()
                                                    .gap(px(8.0))
                                                    .child(lucide_icon(
                                                        "tag",
                                                        15.0,
                                                        ShellDeckColors::primary(),
                                                    ))
                                                    .child(
                                                        div()
                                                            .text_size(px(14.0))
                                                            .font_weight(FontWeight::SEMIBOLD)
                                                            .text_color(
                                                                ShellDeckColors::text_primary(),
                                                            )
                                                            .child(
                                                                t!("user.home.recent_requests")
                                                                    .to_string(),
                                                            ),
                                                    ),
                                            )
                                            .child(
                                                div()
                                                    .text_size(px(11.0))
                                                    .text_color(ShellDeckColors::text_muted())
                                                    .child(
                                                        t!("user.home.latest_three").to_string(),
                                                    ),
                                            ),
                                    )
                                    .child(recent_requests),
                            )
                            .min_w(px(300.0))
                            .flex_1(),
                    )
                    .child(
                        adabraka_ui::display::card::Card::new()
                            .content(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap(px(5.0))
                                    .child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .gap(px(8.0))
                                            .pb(px(6.0))
                                            .child(lucide_icon(
                                                "activity",
                                                15.0,
                                                ShellDeckColors::primary(),
                                            ))
                                            .child(
                                                div()
                                                    .text_size(px(14.0))
                                                    .font_weight(FontWeight::SEMIBOLD)
                                                    .text_color(ShellDeckColors::text_primary())
                                                    .child(
                                                        t!("user.home.workspace_status")
                                                            .to_string(),
                                                    ),
                                            ),
                                    )
                                    .child(status_row(
                                        "shield",
                                        t!("user.home.account").to_string(),
                                        account_status,
                                        self.account_status.dot_color(),
                                    ))
                                    .child(status_row(
                                        "globe",
                                        t!("user.home.active_site").to_string(),
                                        active_site,
                                        ShellDeckColors::primary(),
                                    ))
                                    .child(status_row(
                                        "database",
                                        t!("user.home.directory").to_string(),
                                        t!("user.home.directory_count", count = sites).to_string(),
                                        ShellDeckColors::success(),
                                    ))
                                    .child(
                                        div().flex().justify_end().pt(px(6.0)).child(sync_action),
                                    ),
                            )
                            .min_w(px(260.0))
                            .flex_1(),
                    ),
            )
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
            UserHomeTab::Home => {
                body = body.child(self.render_user_overview(cx));
            }
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

    /// Return a label that always carries a visible Unicode ellipsis when it
    /// exceeds the badge's known character budget. GPUI currently clips text
    /// nodes inside flex badges before painting its CSS-style ellipsis.
    fn ellipsize_badge_label(label: &str, max_chars: usize) -> String {
        let mut chars = label.chars();
        let prefix: String = chars.by_ref().take(max_chars.saturating_sub(1)).collect();
        if chars.next().is_some() {
            format!("{prefix}…")
        } else {
            label.to_string()
        }
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
                    .min_w(px(0.0))
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .truncate()
                    .text_size(px(13.0))
                    .text_color(ShellDeckColors::text_primary())
                    .child(iss.title.clone()),
            );
        if let Some(site_label) = iss
            .site_label
            .as_ref()
            .filter(|label| !label.trim().is_empty())
        {
            row = row.child(
                Badge::new(Self::ellipsize_badge_label(site_label, 17))
                    .variant(BadgeVariant::Outline)
                    .max_w(px(140.0))
                    .overflow_hidden(),
            );
        }
        row = row.child(priority_badge(&iss.priority));
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
        let entity = cx.entity().downgrade();
        let previews =
            render_attachment_draft_gallery(&drafts, "issue-attachment-draft", move |index, cx| {
                if let Some(entity) = entity.upgrade() {
                    entity.update(cx, |this, cx| {
                        let drafts = this.attachment_drafts_mut(target);
                        if index < drafts.len() {
                            drafts.remove(index);
                        }
                        cx.notify();
                    });
                }
            });

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
            .pt(px(9.0))
            .border_t_1()
            .border_color(ShellDeckColors::border())
            .on_key_down(cx.listener(move |this, event: &KeyDownEvent, _, cx| {
                let mods = event.keystroke.modifiers;
                if event.keystroke.key.eq_ignore_ascii_case("v")
                    && (mods.control || mods.platform)
                    && this.paste_issue_attachment(target, cx)
                {
                    cx.stop_propagation();
                }
            }))
            .on_action(cx.listener(move |this, _: &Paste, _, cx| {
                if this.paste_issue_attachment(target, cx) {
                    cx.stop_propagation();
                } else {
                    cx.propagate();
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
                    .gap(px(10.0))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.0))
                            .child(
                                div()
                                    .text_size(px(11.0))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(ShellDeckColors::text_primary())
                                    .child(t!("user.requests.attachments.title").to_string()),
                            )
                            .child(
                                Badge::new(format!(
                                    "{}/{}",
                                    drafts.len(),
                                    issues::ISSUE_ATTACHMENT_MAX_COUNT
                                ))
                                .variant(BadgeVariant::Secondary),
                            ),
                    )
                    .child(
                        div()
                            .min_w(px(0.0))
                            .truncate()
                            .text_size(px(10.0))
                            .text_color(ShellDeckColors::text_muted())
                            .child(t!("user.requests.attachments.drop_hint").to_string()),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .flex_wrap()
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
                        .icon(IconSource::from("scan"))
                        .disabled(self.issue_attachment_busy)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.capture_issue_attachment(target, cx);
                        })),
                    ),
            )
            .when(!drafts.is_empty(), |el| el.child(previews))
            .when(!self.issue_attachment_url_open, |el| {
                el.child(
                    Button::new(
                        SharedString::from(format!("issue-url-toggle-{target:?}")),
                        t!("user.requests.attachments.url_toggle").to_string(),
                    )
                    .size(ButtonSize::Sm)
                    .variant(ButtonVariant::Ghost)
                    .icon(IconSource::from("globe"))
                    .disabled(self.issue_attachment_busy)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.issue_attachment_url_open = true;
                        cx.notify();
                    })),
                )
            })
            .when(self.issue_attachment_url_open, |el| {
                el.child(
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
                            .on_click(cx.listener(
                                move |this, _, _, cx| {
                                    this.import_issue_attachment_url(target, cx);
                                },
                            )),
                        )
                        .child(
                            IconButton::new("x")
                                .variant(ButtonVariant::Ghost)
                                .size(gpui::px(32.0))
                                .icon_size(gpui::px(13.0))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.issue_attachment_url_open = false;
                                    Self::reset_input(&this.issue_attachment_url_state.clone(), cx);
                                    cx.notify();
                                })),
                        ),
                )
            })
    }

    fn render_stored_attachments(
        &self,
        attachments: &[issues::IssueAttachment],
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let entity = cx.entity().downgrade();
        let lightbox_attachments = attachments.to_vec();
        let delete_entity = entity.clone();
        let delete_attachments = attachments.to_vec();
        let issue_id = self
            .issue_detail
            .as_ref()
            .map(|issue| issue.id.clone())
            .unwrap_or_default();
        render_stored_attachment_gallery(
            attachments,
            "stored-attachment",
            move |index, cx| {
                let Some(entity) = entity.upgrade() else {
                    return;
                };
                let close_entity = entity.downgrade();
                let attachments = lightbox_attachments.clone();
                let lightbox = cx.new(|cx| {
                    AttachmentLightbox::new(
                        attachments,
                        index,
                        move |cx| {
                            if let Some(entity) = close_entity.upgrade() {
                                entity.update(cx, |this, cx| {
                                    this.issue_attachment_lightbox = None;
                                    cx.notify();
                                });
                            }
                        },
                        cx,
                    )
                });
                entity.update(cx, |this, cx| {
                    this.issue_attachment_lightbox = Some(lightbox);
                    cx.notify();
                });
            },
            Some(Rc::new(move |index, cx| {
                let Some(attachment) = delete_attachments.get(index) else {
                    return;
                };
                if let Some(entity) = delete_entity.upgrade() {
                    entity.update(cx, |this, cx| {
                        this.confirm_attachment_delete =
                            Some((issue_id.clone(), attachment.id.clone()));
                        cx.notify();
                    });
                }
            })),
        )
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
        let mut inner = div().flex().flex_col().gap(px(10.0)).on_action(cx.listener(
            |this, _: &Paste, _, cx| {
                if this.paste_issue_attachment(IssueAttachmentTarget::NewRequest, cx) {
                    cx.stop_propagation();
                } else {
                    cx.propagate();
                }
            },
        ));
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
                    .flex_col()
                    .gap(px(5.0))
                    .child(
                        div()
                            .text_size(px(11.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(ShellDeckColors::text_muted())
                            .child(t!("user.requests.site_label").to_string()),
                    )
                    .child(self.issue_site_select.clone()),
            )
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
            .on_action(cx.listener(|this, _: &Paste, _, cx| {
                if this.paste_issue_attachment(IssueAttachmentTarget::Comment, cx) {
                    cx.stop_propagation();
                } else {
                    cx.propagate();
                }
            }))
            .child(
                div()
                    .flex()
                    .w_full()
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
                    .children(
                        iss.site_label
                            .as_ref()
                            .filter(|label| !label.trim().is_empty())
                            .map(|label| {
                                Badge::new(Self::ellipsize_badge_label(label, 13))
                                    .variant(BadgeVariant::Outline)
                                    .max_w(px(120.0))
                                    .flex_shrink_0()
                                    .overflow_hidden()
                            }),
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
