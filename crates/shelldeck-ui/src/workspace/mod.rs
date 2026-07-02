use adabraka_ui::prelude::{install_theme, Theme};
use gpui::prelude::*;
use gpui::*;
use shelldeck_core::config::app_config::{AppConfig, ThemePreference};
use shelldeck_core::config::cloud_account::{self, AccountInfo};
use shelldeck_core::config::manage_sites::{self, ManagedSiteInfo, SitesPayload};
use shelldeck_core::config::store::ConnectionStore;
use shelldeck_core::config::themes::TerminalTheme;
use shelldeck_core::models::connection::{Connection, ConnectionSource, ConnectionStatus};
use shelldeck_ssh::tunnel::TunnelHandle;
use std::collections::HashMap;
use std::ops::DerefMut;
use uuid::Uuid;

use crate::command_palette::{
    ApplyAppTheme, ApplyTerminalTheme, CommandPalette, CommandPaletteEvent, OpenManageArea,
    PaletteAction, ToggleCommandPalette,
};
use crate::connection_form::{ConnectionForm, ConnectionFormEvent};
use crate::login_form::{LoginForm, LoginFormEvent};
use crate::dashboard::{ActivityEvent, ActivityType, DashboardEvent, DashboardView};
use crate::port_forward_form::PortForwardForm;
use crate::port_forward_view::{PortForwardEvent, PortForwardView};
use crate::script_editor::{ScriptEditorView, ScriptEvent};
use crate::script_form::ScriptForm;
use crate::server_sync_view::{ServerSyncEvent, ServerSyncView};
use crate::settings::{SettingsEvent, SettingsView};
use crate::sidebar::{SidebarEvent, SidebarSection, SidebarView};
use crate::sites_view::{SitesEvent, SitesView};
use crate::status_bar::{StatusBar, StatusBarEvent};
use crate::template_browser::TemplateBrowser;
use crate::file_editor::view::{FileEditorEvent, FileEditorView};
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

impl AccountStatus {
    fn dot_color(self) -> Hsla {
        match self {
            AccountStatus::Ok => ShellDeckColors::success(),
            AccountStatus::Rejected => ShellDeckColors::error(),
            AccountStatus::Unknown | AccountStatus::Offline => ShellDeckColors::text_muted(),
        }
    }
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
    FileEditor,
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
        OpenFileEditorView,
        CloudSyncNow,
        SwitchSite,
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
    file_editor: Entity<FileEditorView>,
    settings: Entity<SettingsView>,
    status_bar: Entity<StatusBar>,
    command_palette: Entity<CommandPalette>,
    toasts: Entity<ToastContainer>,
    connection_form: Option<Entity<ConnectionForm>>,
    login_form: Option<Entity<LoginForm>>,
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
    pub focus_handle: FocusHandle,
    /// Active tunnels keyed by the PortForward model ID (not the TunnelHandle internal id).
    active_tunnels: HashMap<Uuid, ActiveTunnel>,
    /// Active script executions keyed by script ID.
    active_scripts: HashMap<Uuid, ActiveScript>,
    // Keep subscriptions alive
    _sidebar_sub: Subscription,
    _terminal_sub: Subscription,
    _palette_sub: Subscription,
    _settings_sub: Subscription,
    _scripts_sub: Subscription,
    _forwards_sub: Subscription,
    _server_sync_sub: Subscription,
    _sites_sub: Subscription,
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
    /// Cached Inklura Manage sites directory + areas (fetched after sign-in).
    site_directory: Option<SitesPayload>,
    /// Whether the titlebar site-switcher dropdown is open.
    site_menu_open: bool,
    /// While the command palette is previewing an app theme, the theme to
    /// restore if the user dismisses without committing. `None` when no preview
    /// is active.
    theme_before_preview: Option<ThemePreference>,
    /// Same idea for a previewed terminal color theme: the terminal theme name
    /// to restore if the palette is dismissed without committing.
    terminal_theme_before_preview: Option<String>,
}

impl Workspace {
    pub fn new(
        cx: &mut Context<Self>,
        config: AppConfig,
        connections: Vec<Connection>,
        store: ConnectionStore,
    ) -> Self {
        // Restore the persisted active-site filter (if any) so the sidebar
        // opens scoped to the last-selected site.
        let initial_site_filter = config
            .cloud_sync
            .active_site_id
            .as_deref()
            .and_then(|s| Uuid::parse_str(s).ok());
        let sidebar = cx.new(|cx| {
            let mut s = SidebarView::new(cx);
            s.set_connections(connections.clone());
            s.set_site_filter(initial_site_filter);
            s
        });

        let dashboard = cx.new(|_| {
            let mut d = DashboardView::new();
            d.favorite_hosts = connections
                .iter()
                .take(5)
                .map(|c| {
                    (
                        c.display_name().to_string(),
                        c.hostname.clone(),
                        c.status == ConnectionStatus::Connected,
                    )
                })
                .collect();
            d
        });

        let terminal = cx.new(TerminalView::new);
        let scripts = cx.new(ScriptEditorView::new);
        let port_forwards = cx.new(|_| PortForwardView::new());
        let server_sync = cx.new(|cx| {
            let mut view = ServerSyncView::new(cx);
            view.set_connections(connections.clone());
            view.set_profiles(store.sync_profiles.clone());
            view
        });
        let sites = cx.new(|cx| {
            let mut view = SitesView::new(cx);
            view.set_connections(connections.clone());
            view.set_sites(store.managed_sites.clone());
            view
        });
        let file_editor = cx.new(FileEditorView::new);
        // The editor scales with the app font size, like the rest of the UI.
        file_editor.update(cx, |ed, _| {
            ed.set_font_size(config.general.ui_font_size);
            ed.set_font_family(config.general.ui_font_family.clone());
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
        let settings = cx.new(|_| SettingsView::new(config));
        let status_bar = cx.new(|_| StatusBar::new());
        let toasts = cx.new(|_| ToastContainer::new());

        // Create auto-updater
        let auto_updater = cx.new(|cx| {
            let mut updater = AutoUpdater::new();
            updater.set_enabled(auto_update_enabled, cx);
            updater
        });

        // Create command palette with registered actions
        let command_palette = cx.new(|cx| {
            let mut palette = CommandPalette::new(cx);
            palette.set_actions(Self::base_palette_actions());
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
            |this, _palette, event: &CommandPaletteEvent, cx| match event {
                CommandPaletteEvent::SelectionPreviewed(action) => {
                    this.preview_palette_action(action.as_ref(), cx);
                }
                CommandPaletteEvent::ActionSelected(action) => {
                    if let Some(t) = action.as_any().downcast_ref::<ApplyAppTheme>() {
                        // Commit the previewed app theme (persists it).
                        this.revert_terminal_theme_preview(cx);
                        this.commit_theme_preview(t.pref.clone(), cx);
                    } else if let Some(t) = action.as_any().downcast_ref::<ApplyTerminalTheme>() {
                        // Commit the previewed terminal theme (persists it).
                        this.revert_theme_preview(cx);
                        this.terminal_theme_before_preview = None;
                        this.apply_terminal_theme_by_name(&t.name, cx);
                    } else {
                        // Any other command: drop any active previews first.
                        this.revert_theme_preview(cx);
                        this.revert_terminal_theme_preview(cx);
                        cx.dispatch_action(action.as_ref());
                    }
                }
                CommandPaletteEvent::Dismissed => {
                    this.revert_theme_preview(cx);
                    this.revert_terminal_theme_preview(cx);
                    cx.notify();
                }
            },
        );

        // Subscribe to settings events
        let settings_sub = cx.subscribe(&settings, |this, _settings, event: &SettingsEvent, cx| {
            this.handle_settings_event(event, cx);
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

        // Subscribe to file editor events
        let file_editor_sub =
            cx.subscribe(&file_editor, |_this, _view, _event: &FileEditorEvent, cx| {
                cx.notify();
            });

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
        let status_bar_sub = cx.subscribe(
            &status_bar,
            |this, _bar, event: &StatusBarEvent, cx| match event {
                StatusBarEvent::UpdateClicked => {
                    this.auto_updater.update(cx, |u, cx| u.trigger_update(cx));
                }
            },
        );

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
            file_editor,
            settings,
            status_bar,
            command_palette,
            toasts,
            connection_form: None,
            login_form: None,
            port_forward_form: None,
            script_form: None,
            template_browser: None,
            variable_prompt: None,
            active_view: ActiveView::Dashboard,
            sidebar_visible: true,
            sidebar_width: initial_sidebar_width,
            ui_font_family,
            ui_font_size,
            focus_handle: cx.focus_handle(),
            active_tunnels: HashMap::new(),
            active_scripts: HashMap::new(),
            _sidebar_sub: sidebar_sub,
            _terminal_sub: terminal_sub,
            _palette_sub: palette_sub,
            _settings_sub: settings_sub,
            _scripts_sub: scripts_sub,
            _forwards_sub: forwards_sub,
            _server_sync_sub: server_sync_sub,
            _sites_sub: sites_sub,
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
            site_directory: None,
            site_menu_open: false,
            theme_before_preview: None,
            terminal_theme_before_preview: None,
        }
    }

    /// Decide whether a window-close request should proceed.
    ///
    /// When `confirm_before_close` is enabled and there are active terminal
    /// sessions or running tunnels, the first close attempt is intercepted: we
    /// warn the user and require a second close to confirm (matching the
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
                    self.connect_ssh(conn.clone(), cx);
                    self.add_activity(
                        format!("Connecting to {}", title),
                        ActivityType::Connection,
                        cx,
                    );
                }
                self.active_view = ActiveView::Terminal;
                cx.notify();
            }
            SidebarEvent::ConnectionConnect(id) => {
                tracing::info!("Connect requested: {}", id);
                if let Some(conn) = self.connections.iter().find(|c| c.id == *id) {
                    self.connect_ssh(conn.clone(), cx);
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
                                    format!("Failed to delete: {}", e),
                                    ToastLevel::Error,
                                    cx,
                                );
                                return;
                            }
                        }
                        self.connections.retain(|c| c.id != id);
                        self.sidebar.update(cx, |sidebar, _| {
                            sidebar.set_connections(self.connections.clone());
                        });
                        self.port_forwards.update(cx, |pf, _| {
                            pf.forwards.retain(|f| f.connection_id != id);
                        });
                        self.add_activity(
                            format!("Deleted connection: {}", name),
                            ActivityType::Connection,
                            cx,
                        );
                        self.show_toast(
                            format!("Deleted connection: {}", name),
                            ToastLevel::Info,
                            cx,
                        );
                        cx.notify();
                    }
                } else {
                    // First click — ask for confirmation
                    if let Some(conn) = self.connections.iter().find(|c| c.id == id) {
                        let name = conn.display_name().to_string();
                        self.pending_delete = Some(id);
                        self.show_toast(
                            format!("Click delete again to confirm removing \"{}\"", name),
                            ToastLevel::Warning,
                            cx,
                        );
                        cx.notify();
                    }
                }
            }
            SidebarEvent::WidthChanged(width) => {
                self.sidebar_width = *width;
                self.terminal.update(cx, |terminal, _cx| {
                    terminal.set_sidebar_width(*width);
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
                self.update_dashboard_stats(cx);
                self.sync_terminal_tab_count(cx);
                cx.notify();
            }
            TerminalEvent::TabSelected(id) => {
                tracing::info!("Terminal tab selected: {}", id);
            }
            TerminalEvent::TabClosed(id) => {
                tracing::info!("Terminal tab closed: {}", id);
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
                // Keep the in-memory config in sync so runtime behavior
                // (notifications gate, tmux auto-attach, etc.) reads current values.
                self.app_config = config.clone();
                // Apply terminal settings to running view
                let terminal_theme = TerminalTheme::by_name(&config.terminal.theme);
                self.terminal.update(cx, |terminal, cx| {
                    terminal.set_font_size(config.terminal.font_size);
                    terminal.set_font_family(config.terminal.font_family.clone());
                    terminal.set_cursor_style(&config.terminal.cursor_style);
                    terminal.set_cursor_blink(config.terminal.cursor_blink);
                    terminal.set_scrollback_lines(config.terminal.scrollback_lines);
                    terminal.set_terminal_theme(&terminal_theme);
                    cx.notify();
                });
                // Apply sidebar width
                self.sidebar_width = config.general.sidebar_width;
                self.terminal.update(cx, |terminal, _cx| {
                    terminal.set_sidebar_width(config.general.sidebar_width);
                });
                // Apply application UI font (cascades to all child views on re-render)
                self.ui_font_family = config.general.ui_font_family.clone();
                self.ui_font_size = config.general.ui_font_size;
                // The file editor's text scales with the app font size too.
                let ed_size = config.general.ui_font_size;
                let ed_family = config.general.ui_font_family.clone();
                self.file_editor.update(cx, |ed, cx| {
                    ed.set_font_size(ed_size);
                    ed.set_font_family(ed_family);
                    cx.notify();
                });
                // Apply auto-update preference
                let auto_update = config.general.auto_update;
                self.auto_updater.update(cx, |updater, cx| {
                    updater.set_enabled(auto_update, cx);
                });
                cx.notify();
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

    /// Swap the live `ShellDeckColors` palette and the adabraka-ui component
    /// theme to `pref`, then notify every view so the whole UI repaints. Does
    /// NOT touch `app_config` or persist — callers decide whether to commit.
    fn apply_palette(&self, pref: &ThemePreference, cx: &mut Context<Self>) {
        ShellDeckColors::set_theme(pref);
        let ui_theme = if pref.is_dark() {
            Theme::dark()
        } else {
            Theme::light()
        };
        install_theme(cx.deref_mut(), ui_theme);
        self.notify_theme_views(cx);
    }

    /// Notify every child view (and self) to re-render with the active palette.
    fn notify_theme_views(&self, cx: &mut Context<Self>) {
        self.sidebar.update(cx, |_, cx| cx.notify());
        self.dashboard.update(cx, |_, cx| cx.notify());
        self.scripts.update(cx, |_, cx| cx.notify());
        self.port_forwards.update(cx, |_, cx| cx.notify());
        self.server_sync.update(cx, |_, cx| cx.notify());
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
        self.show_toast(format!("Terminal theme: {}", name), ToastLevel::Info, cx);
    }

    fn handle_dashboard_event(&mut self, event: &DashboardEvent, cx: &mut Context<Self>) {
        match event {
            DashboardEvent::QuickConnect(alias) => {
                // Find connection by alias/display name and connect
                if let Some(conn) = self
                    .connections
                    .iter()
                    .find(|c| c.display_name() == alias.as_str())
                {
                    let conn = conn.clone();
                    let title = conn.display_name().to_string();
                    self.connect_ssh(conn, cx);
                    self.add_activity(
                        format!("Quick connecting to {}", title),
                        ActivityType::Connection,
                        cx,
                    );
                    self.active_view = ActiveView::Terminal;
                    cx.notify();
                } else {
                    self.show_toast(
                        format!("Connection '{}' not found", alias),
                        ToastLevel::Warning,
                        cx,
                    );
                }
            }
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
                            format!("Failed to save connection: {}", e),
                            ToastLevel::Error,
                            cx,
                        );
                    }
                    // Update sidebar
                    this.sidebar.update(cx, |sidebar, _| {
                        sidebar.set_connections(this.connections.clone());
                    });
                    this.add_activity(
                        format!("Added connection: {}", conn.display_name()),
                        ActivityType::Connection,
                        cx,
                    );
                    this.show_toast(
                        format!("Connection saved: {}", conn.display_name()),
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

    fn add_activity(&mut self, message: String, event_type: ActivityType, cx: &mut Context<Self>) {
        let event = ActivityEvent {
            icon: match event_type {
                ActivityType::Connection => "server",
                ActivityType::Forward => "arrow",
                ActivityType::Script => "play",
                ActivityType::Error => "alert",
            },
            message,
            timestamp: chrono::Local::now().format("%H:%M:%S").to_string(),
            event_type,
        };
        self.dashboard.update(cx, |dashboard, _| {
            dashboard.recent_activity.insert(0, event);
            if dashboard.recent_activity.len() > 50 {
                dashboard.recent_activity.truncate(50);
            }
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
                "Cloud Sync isn't configured. Enable it and add a token in the [cloud_sync] \
                 section of shelldeck.toml (get a token at manage.inklura.fr/manage/shelldeck).",
                ToastLevel::Warning,
                cx,
            );
            return;
        }

        self.show_toast("Cloud Sync started…", ToastLevel::Info, cx);
        let version = shelldeck_core::VERSION;

        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    shelldeck_core::config::cloud_sync::sync_now(&cfg, version)
                })
                .await;

            let _ = this.update(cx, |ws, cx| match result {
                Ok(stats) => {
                    ws.reload_connections_after_sync(cx);
                    ws.show_toast(
                        format!(
                            "Cloud Sync: {} added, {} updated, {} removed",
                            stats.added, stats.updated, stats.removed
                        ),
                        ToastLevel::Success,
                        cx,
                    );
                }
                Err(e) => {
                    ws.show_toast(format!("Cloud Sync failed: {}", e), ToastLevel::Error, cx);
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
        self.server_sync.update(cx, |view, _| {
            view.set_connections(conns.clone());
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
                        // Token is valid → load the sites directory too.
                        ws.refresh_sites(cx);
                    }
                    Err(e) if cloud_account::is_auth_rejected(&e) => {
                        ws.account_status = AccountStatus::Rejected;
                        ws.show_toast(
                            "Session Inklura expirée — reconnectez-vous depuis le menu compte.",
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

    /// Open the password + OIDC login modal.
    pub fn show_login_form(&mut self, cx: &mut Context<Self>) {
        let server = self.account_base_url();
        let device = cloud_account::device_name();
        let form = cx.new(|form_cx| LoginForm::new(server, device, form_cx));

        let sub = cx.subscribe(&form, |this, _form, event: &LoginFormEvent, cx| match event {
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
        });

        self.account_menu_open = false;
        self.login_form = Some(form);
        self._login_form_sub = Some(sub);
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
                .spawn(async move {
                    cloud_account::login_password(&base, &email, &password, &device)
                })
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
                    format!("Impossible d'ouvrir un port local : {}", e),
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
                    format!("Impossible de lire le port local : {}", e),
                    ToastLevel::Error,
                    cx,
                );
                return;
            }
        };
        // Random state: two v4 UUIDs → 64 hex chars, matches [A-Za-z0-9_-]{32,64}.
        let state = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
        let url = cloud_account::browser_connect_url(&base, port, &state, &device, provider.as_deref());

        if let Err(e) = cloud_account::open_in_browser(&url) {
            self.show_toast(
                format!("Impossible d'ouvrir le navigateur : {}", cloud_account::user_message(&e)),
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
            "En attente d'autorisation dans le navigateur…",
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
                    format!("Connexion navigateur échouée : {}", cloud_account::user_message(&e)),
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
        self.account_status = AccountStatus::Ok;
        self.login_form = None;
        self._login_form_sub = None;
        self.account_menu_open = false;
        cx.notify();

        // Load the sites directory for the switcher (background, non-blocking).
        self.refresh_sites(cx);

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
                        format!("Connecté en tant que {} — {} profils synchronisés", name, n),
                        ToastLevel::Success,
                        cx,
                    );
                }
                Err(e) => {
                    ws.show_toast(
                        format!(
                            "Connecté en tant que {}. Synchronisation échouée : {}",
                            name,
                            cloud_account::user_message(&e)
                        ),
                        ToastLevel::Warning,
                        cx,
                    );
                }
            });
        })
        .detach();
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

        self.app_config.account = None;
        self.app_config.cloud_sync.token = String::new();
        self.app_config.cloud_sync.enabled = false;
        self.app_config.cloud_sync.active_site_id = None;
        self.app_config.cloud_sync.active_site_label = None;
        if let Err(e) = self.app_config.save() {
            tracing::error!("Failed to save config after logout: {}", e);
        }
        self.account_status = AccountStatus::Unknown;
        self.account_menu_open = false;
        self.site_directory = None;
        self.site_menu_open = false;
        self.sidebar.update(cx, |s, cx| {
            s.set_site_filter(None);
            cx.notify();
        });
        self.show_toast("Déconnecté d'Inklura Manage.", ToastLevel::Info, cx);
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
        cx.notify();
    }

    /// Open a manage area for the active site in the system browser.
    pub fn open_manage_area(&mut self, area_path: String, cx: &mut Context<Self>) {
        let site = match self.active_site_info() {
            Some(s) => s,
            None => {
                self.show_toast(
                    "Sélectionnez d'abord un site actif pour ouvrir Manage.",
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
            Ok(_) => self.show_toast("Ouverture dans le navigateur…", ToastLevel::Info, cx),
            Err(e) => self.show_toast(
                format!(
                    "Impossible d'ouvrir le navigateur : {}",
                    cloud_account::user_message(&e)
                ),
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
                "Connectez-vous à Inklura Manage pour changer de site.",
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
    fn base_palette_actions() -> Vec<PaletteAction> {
        let mut actions = vec![
            PaletteAction::new("New Terminal", Some("Ctrl+T"), Box::new(NewTerminal)),
            PaletteAction::new("Toggle Sidebar", Some("Ctrl+B"), Box::new(ToggleSidebar)),
            PaletteAction::new("Open Settings", Some("Ctrl+,"), Box::new(OpenSettings)),
            PaletteAction::new("Close Tab", Some("Ctrl+W"), Box::new(CloseTab)),
            PaletteAction::new("Next Tab", Some("Ctrl+Tab"), Box::new(NextTab)),
            PaletteAction::new("Previous Tab", Some("Ctrl+Shift+Tab"), Box::new(PrevTab)),
            PaletteAction::new("Quit", Some("Ctrl+Q"), Box::new(Quit)),
            PaletteAction::new("Browse Script Templates", None, Box::new(OpenTemplateBrowser)),
            PaletteAction::new("New Script", None, Box::new(NewScript)),
            PaletteAction::new("Open Server Sync", None, Box::new(OpenServerSync)),
            PaletteAction::new("Open Sites", None, Box::new(OpenSites)),
            PaletteAction::new("Open File Editor", Some("Ctrl+E"), Box::new(OpenFileEditorView)),
            PaletteAction::new("Cloud Sync Now", None, Box::new(CloudSyncNow)),
            PaletteAction::new("Switch Active Site", None, Box::new(SwitchSite)),
        ];
        for pref in ThemePreference::all() {
            actions.push(PaletteAction::new(
                &format!("Theme: {}", pref.display_name()),
                None,
                Box::new(ApplyAppTheme { pref: pref.clone() }),
            ));
        }
        for theme in TerminalTheme::builtins() {
            actions.push(PaletteAction::new(
                &format!("Terminal Theme: {}", theme.name),
                None,
                Box::new(ApplyTerminalTheme { name: theme.name }),
            ));
        }
        actions
    }

    /// Rebuild the palette entries, appending "Site actif : <area>" commands for
    /// the active site's manage areas. Called when the site directory loads or
    /// the active site changes.
    fn refresh_command_palette(&mut self, cx: &mut Context<Self>) {
        let mut actions = Self::base_palette_actions();
        if let (Some(site), Some(dir)) = (self.active_site_info(), self.site_directory.as_ref()) {
            let label = site.display_label();
            for area in &dir.areas {
                actions.push(PaletteAction::new(
                    &format!("Site actif ({}) : {}", label, area.label),
                    None,
                    Box::new(OpenManageArea {
                        path: area.path.clone(),
                    }),
                ));
            }
        }
        self.command_palette.update(cx, |palette, _| {
            palette.set_actions(actions);
        });
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

        let favorite_hosts: Vec<(String, String, bool)> = self
            .connections
            .iter()
            .take(5)
            .map(|c| {
                (
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
        self.port_forward_form = None;
        self._pf_form_sub = None;
        self.script_form = None;
        self._script_form_sub = None;
        tracing::info!("Shutdown cleanup complete");
    }

    pub fn set_active_view(&mut self, view: ActiveView) {
        self.active_view = view;
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
            SidebarSection::FileEditor => ActiveView::FileEditor,
            SidebarSection::Settings => ActiveView::Settings,
        };
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
            if let Some(tab) = terminal.tabs.get(state.active_tab.min(terminal.tabs.len() - 1)) {
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
    fn render_titlebar(
        is_maximized: bool,
        theme_menu_open: bool,
        account_menu_open: bool,
        account: Option<AccountInfo>,
        account_status: AccountStatus,
        site_menu_open: bool,
        active_site_label: Option<String>,
        sites_loaded: bool,
        ui_font_size: f32,
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

        // Brand mark — a small accent-tinted rounded badge with the diamond glyph.
        let brand_badge = div()
            .flex()
            .items_center()
            .justify_center()
            .size(px(20.0))
            .rounded(px(5.0))
            .bg(accent.opacity(0.18))
            .text_xs()
            .font_weight(FontWeight::BOLD)
            .text_color(accent)
            .child("\u{25C6}"); // ◆

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
            .child(brand_badge)
            .child(
                div()
                    .text_xs()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(title_color)
                    .child("ShellDeck"),
            )
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

        // A window-control button with a rounded hover affordance.
        let control_btn = |id: &'static str,
                           glyph: &'static str,
                           area: WindowControlArea,
                           danger: bool| {
            let hover_bg = if danger {
                ShellDeckColors::error()
            } else {
                btn_hover_bg
            };
            div()
                .id(id)
                .flex()
                .items_center()
                .justify_center()
                .size(px(28.0))
                .rounded(px(6.0))
                .text_sm()
                .text_color(btn_text)
                .hover(|s| s.bg(hover_bg).text_color(gpui::white()))
                .window_control_area(area)
                .child(glyph)
        };

        let minimize_btn = control_btn(
            "titlebar-minimize",
            "\u{2500}", // ─
            WindowControlArea::Min,
            false,
        )
        .on_click(cx.listener(|_this, _event: &ClickEvent, window, _cx| {
            window.minimize_window();
        }));

        let maximize_icon = if is_maximized { "\u{25A3}" } else { "\u{25A1}" }; // ▣ or □
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
            "\u{00D7}", // ×
            WindowControlArea::Close,
            true,
        )
        .on_click(
            move |_event: &ClickEvent, _window: &mut Window, cx: &mut App| {
                if let Some(ws) = h_quit.upgrade() {
                    ws.update(cx, |ws, cx| {
                        ws.shutdown(cx);
                        cx.quit();
                    });
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
                        .child("Se connecter"),
                );
        }

        account_btn = account_btn.on_click(cx.listener(|this, _event: &ClickEvent, _window, cx| {
            this.account_menu_open = !this.account_menu_open;
            if this.account_menu_open {
                this.theme_menu_open = false;
            }
            cx.notify();
        }));
        if account_menu_open {
            account_btn = account_btn.bg(ShellDeckColors::hover_bg());
        }

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
                    div()
                        .text_size(px(8.0))
                        .text_color(title_dim)
                        .child("\u{25BC}"), // ▼
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
        let scale_btn = |id: &'static str, glyph: &'static str| {
            div()
                .id(id)
                .flex()
                .items_center()
                .justify_center()
                .size(px(22.0))
                .rounded(px(5.0))
                .text_sm()
                .text_color(btn_text)
                .cursor_pointer()
                .hover(|s| s.bg(btn_hover_bg).text_color(ShellDeckColors::text_primary()))
                .child(glyph)
        };
        let dec_btn = scale_btn("titlebar-scale-down", "\u{2212}") // −
            .on_click(cx.listener(|this, _event: &ClickEvent, _window, cx| {
                this.settings
                    .update(cx, |settings, cx| settings.adjust_ui_font_size(-1.0, cx));
                cx.notify();
            }));
        let inc_btn = scale_btn("titlebar-scale-up", "+").on_click(cx.listener(
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

        // Subtle vertical divider between the chrome control clusters.
        let divider = || {
            div()
                .w(px(1.0))
                .h(px(16.0))
                .mx(px(4.0))
                .bg(titlebar_border)
        };

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
                    .child(divider())
                    .child(account_btn)
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
            .shadow(vec![BoxShadow {
                color: hsla(0.0, 0.0, 0.0, 0.45),
                offset: point(px(0.0), px(4.0)),
                blur_radius: px(20.0),
                spread_radius: px(0.0),
                inset: false,
            }])
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
            .shadow(shadow)
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
            let info_row = |label: &str, value: String| {
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(px(8.0))
                    .child(
                        div()
                            .text_size(px(11.0))
                            .text_color(ShellDeckColors::text_muted())
                            .child(label.to_string()),
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
                .child(info_row("Serveur", self.account_base_url()))
                .child(info_row("Appareil", cloud_account::device_name()))
                .child(info_row(
                    "Site actif",
                    self.app_config
                        .cloud_sync
                        .active_site_label
                        .clone()
                        .unwrap_or_else(|| "Tous les sites".to_string()),
                ))
                .child(info_row("Statut", status_label.to_string()));

            panel = panel.child(
                secondary_btn("account-sync", "Synchroniser".to_string()).on_click(cx.listener(
                    |this, _: &ClickEvent, _, cx| {
                        this.account_menu_open = false;
                        this.cloud_sync_now(cx);
                    },
                )),
            );
            panel = panel.child(
                secondary_btn("account-logout", "Se déconnecter".to_string())
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
                        .child("Compte Inklura Manage"),
                )
                .child(
                    div()
                        .text_size(px(11.0))
                        .text_color(ShellDeckColors::text_muted())
                        .child("Connectez-vous pour synchroniser vos connexions SSH."),
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
                    .child("Se connecter")
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
                            .child("ou en un clic"),
                    )
                    .child(div().flex_1().h(px(1.0)).bg(ShellDeckColors::border())),
            );

            panel = panel
                .child(
                    secondary_btn("account-oidc-sso", "Continuer avec SSO 1clic.pro".to_string())
                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                            this.start_oidc_login(Some("sso".to_string()), cx);
                        })),
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
            b_active
                .cmp(&a_active)
                .then(b_conn.cmp(&a_conn))
                .then(
                    a.display_label()
                        .to_lowercase()
                        .cmp(&b.display_label().to_lowercase()),
                )
        });
        let total = sites.len();
        let hidden = total.saturating_sub(CAP);

        let row = |id: ElementId,
                   label: String,
                   active: bool,
                   badge: Option<String>|
         -> Stateful<Div> {
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
            .shadow(shadow)
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
}

impl Render for Workspace {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
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

        if self.sidebar_visible {
            main_area = main_area.child(self.sidebar.clone());
        }

        let mut content = div().flex_grow().w_full().min_h(px(0.0)).overflow_hidden();
        if !output_resizing && !sidebar_resizing {
            content = content.block_mouse_except_scroll();
        }

        match self.active_view {
            ActiveView::Dashboard => {
                content = content.child(self.dashboard.clone());
            }
            ActiveView::Terminal => {
                content = content.child(self.terminal.clone());
            }
            ActiveView::Scripts => {
                content = content.child(self.scripts.clone());
            }
            ActiveView::PortForwards => {
                content = content.child(self.port_forwards.clone());
            }
            ActiveView::ServerSync => {
                content = content.child(self.server_sync.clone());
            }
            ActiveView::Sites => {
                content = content.child(self.sites.clone());
            }
            ActiveView::FileEditor => {
                content = content.child(self.file_editor.clone());
            }
            ActiveView::Settings => {
                content = content.child(self.settings.clone());
            }
        }

        main_area = main_area.child(content);

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
                            palette.toggle(window);
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
                .shadow(vec![BoxShadow {
                    color: hsla(0.0, 0.0, 0.0, 0.45),
                    offset: point(px(0.0), px(2.0)),
                    blur_radius: px(16.0),
                    spread_radius: px(0.0),
                    inset: false,
                }]);
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
            self.ui_font_size,
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

        // Command palette overlay
        root = root.child(self.command_palette.clone());

        // Toast notification overlay
        root = root.child(self.toasts.clone());

        // Modal form overlays — render an occluding backdrop at the workspace
        // level so hover/click on elements behind is properly blocked.
        let has_modal = self.connection_form.is_some()
            || self.login_form.is_some()
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

        root
    }
}
