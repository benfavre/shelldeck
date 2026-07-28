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

mod account;
mod activity;
mod ai;
mod bext;
mod chrome;
mod cloud_sync;
mod discovery;
mod events;
mod fleet;
mod forwards;
mod jean;
mod menu;
mod modes;
mod navigation;
mod overlays;
mod palette;
mod panels;
mod render;
mod request_views;
mod requests;
mod scripts;
mod server_sync;
mod sites;
mod ssh;
mod support;
mod tray;
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

#[cfg(test)]
mod tests {
    use super::{
        POST_LOGIN_SPLASH_MIN_MS, ShortcutToastKind, post_login_simulated_progress,
        post_login_splash_opacity, post_login_splash_remaining, post_login_wink_opacity,
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
            post_login_splash_remaining(std::time::Duration::from_millis(POST_LOGIN_SPLASH_MIN_MS))
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
            checkpoints.windows(2).all(|pair| pair[1] >= pair[0]),
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
