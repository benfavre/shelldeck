use adabraka_ui::prelude::{install_theme, Theme};
use gpui::prelude::*;
use gpui::*;
use shelldeck_core::config::app_config::{AppConfig, ThemePreference};
use shelldeck_core::config::store::ConnectionStore;
use shelldeck_core::config::themes::TerminalTheme;
use shelldeck_core::models::connection::{Connection, ConnectionStatus};
use shelldeck_ssh::tunnel::TunnelHandle;
use std::collections::HashMap;
use std::ops::DerefMut;
use uuid::Uuid;

use crate::command_palette::{
    ApplyTerminalTheme, CommandPalette, CommandPaletteEvent, PaletteAction, ToggleCommandPalette,
};
use crate::connection_form::{ConnectionForm, ConnectionFormEvent};
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
}

impl Workspace {
    pub fn new(
        cx: &mut Context<Self>,
        config: AppConfig,
        connections: Vec<Connection>,
        store: ConnectionStore,
    ) -> Self {
        let sidebar = cx.new(|cx| {
            let mut s = SidebarView::new(cx);
            s.set_connections(connections.clone());
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
            let mut actions = vec![
                PaletteAction::new("New Terminal", Some("Ctrl+T"), Box::new(NewTerminal)),
                PaletteAction::new("Toggle Sidebar", Some("Ctrl+B"), Box::new(ToggleSidebar)),
                PaletteAction::new("Open Settings", Some("Ctrl+,"), Box::new(OpenSettings)),
                PaletteAction::new("Close Tab", Some("Ctrl+W"), Box::new(CloseTab)),
                PaletteAction::new("Next Tab", Some("Ctrl+Tab"), Box::new(NextTab)),
                PaletteAction::new("Previous Tab", Some("Ctrl+Shift+Tab"), Box::new(PrevTab)),
                PaletteAction::new("Quit", Some("Ctrl+Q"), Box::new(Quit)),
                PaletteAction::new(
                    "Browse Script Templates",
                    None,
                    Box::new(OpenTemplateBrowser),
                ),
                PaletteAction::new("New Script", None, Box::new(NewScript)),
                PaletteAction::new("Open Server Sync", None, Box::new(OpenServerSync)),
                PaletteAction::new("Open Sites", None, Box::new(OpenSites)),
                PaletteAction::new(
                    "Open File Editor",
                    Some("Ctrl+E"),
                    Box::new(OpenFileEditorView),
                ),
            ];
            // One entry per built-in terminal color theme — switches live.
            for theme in TerminalTheme::builtins() {
                actions.push(PaletteAction::new(
                    &format!("Terminal Theme: {}", theme.name),
                    None,
                    Box::new(ApplyTerminalTheme { name: theme.name }),
                ));
            }
            palette.set_actions(actions);
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
            |_this, _palette, event: &CommandPaletteEvent, cx| match event {
                CommandPaletteEvent::ActionSelected(action) => {
                    cx.dispatch_action(action.as_ref());
                }
                CommandPaletteEvent::Dismissed => {
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
                // Apply auto-update preference
                let auto_update = config.general.auto_update;
                self.auto_updater.update(cx, |updater, cx| {
                    updater.set_enabled(auto_update, cx);
                });
                cx.notify();
            }
            SettingsEvent::ThemeChanged(pref) => {
                tracing::info!("Theme preference changed to {:?}", pref);
                let is_dark = matches!(pref, ThemePreference::Dark | ThemePreference::System);

                // Switch the app-wide ShellDeckColors
                ShellDeckColors::set_dark_mode(is_dark);

                // Switch the adabraka-ui component theme
                let ui_theme = if is_dark {
                    Theme::dark()
                } else {
                    Theme::light()
                };
                install_theme(cx.deref_mut(), ui_theme);

                // Terminal color theme is configured independently (Appearance
                // tab / command palette) and persisted, so it is intentionally
                // left untouched when the app light/dark preference changes.

                // Notify all child views to re-render with new colors
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
        }
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

    /// Gracefully shut down: close all terminal sessions, stop tunnels, stop background tasks.
    pub fn shutdown(&mut self, cx: &mut Context<Self>) {
        tracing::info!("ShellDeck shutting down gracefully...");
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

    pub fn open_new_terminal(&mut self, cx: &mut Context<Self>) {
        self.terminal.update(cx, |terminal, cx| {
            terminal.spawn_local_terminal(cx);
        });
        self.active_view = ActiveView::Terminal;
        self.update_dashboard_stats(cx);
        self.sync_terminal_tab_count(cx);
    }

    /// Start periodic git status polling (every 5 seconds).
    fn start_git_polling(cx: &mut Context<Self>, status_bar: &Entity<StatusBar>) -> gpui::Task<()> {
        let weak_bar = status_bar.downgrade();
        cx.spawn(async move |_this, cx: &mut AsyncApp| loop {
            cx.background_executor()
                .timer(std::time::Duration::from_secs(5))
                .await;

            let git_display = cx
                .background_executor()
                .spawn(async {
                    let cwd = std::env::current_dir().unwrap_or_default();
                    shelldeck_core::git::get_git_status(&cwd).and_then(|s| s.display())
                })
                .await;

            let result = weak_bar.update(cx, |bar, cx| {
                bar.git_status = git_display;
                cx.notify();
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
        handle: &WeakEntity<Self>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let titlebar_bg = ShellDeckColors::bg_sidebar();
        let titlebar_border = ShellDeckColors::border();
        let title_color = ShellDeckColors::text_primary();
        let title_dim = ShellDeckColors::text_muted();
        let btn_w = px(46.0);
        let btn_text = ShellDeckColors::text_muted();
        let btn_hover_bg = ShellDeckColors::hover_bg();

        // Title area — draggable
        let title_area = div()
            .flex_1()
            .h_full()
            .flex()
            .items_center()
            .px_4()
            .gap_2()
            .window_control_area(WindowControlArea::Drag)
            .on_mouse_down(MouseButton::Left, |_e, window, _cx| {
                window.start_window_move();
            })
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::BOLD)
                    .text_color(ShellDeckColors::primary())
                    .child("\u{25C6}"), // ◆
            )
            .child(
                div()
                    .text_xs()
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(title_color)
                    .child("ShellDeck"),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(title_dim)
                    .child(format!("v{}", shelldeck_core::VERSION)),
            );

        // Minimize button
        let minimize_btn = div()
            .id("titlebar-minimize")
            .flex()
            .items_center()
            .justify_center()
            .w(btn_w)
            .h_full()
            .text_sm()
            .text_color(btn_text)
            .hover(|s| s.bg(btn_hover_bg).text_color(gpui::white()))
            .window_control_area(WindowControlArea::Min)
            .on_click(cx.listener(|_this, _event: &ClickEvent, window, _cx| {
                window.minimize_window();
            }))
            .child("\u{2500}"); // ─

        // Maximize / Restore button
        let maximize_icon = if is_maximized { "\u{25A3}" } else { "\u{25A1}" }; // ▣ or □
        let maximize_btn = div()
            .id("titlebar-maximize")
            .flex()
            .items_center()
            .justify_center()
            .w(btn_w)
            .h_full()
            .text_sm()
            .text_color(btn_text)
            .hover(|s| s.bg(btn_hover_bg).text_color(gpui::white()))
            .window_control_area(WindowControlArea::Max)
            .on_click(cx.listener(|_this, _event: &ClickEvent, window, _cx| {
                window.zoom_window();
            }))
            .child(maximize_icon);

        // Close button
        let close_hover_bg = ShellDeckColors::error();
        let h_quit = handle.clone();
        let close_btn = div()
            .id("titlebar-close")
            .flex()
            .items_center()
            .justify_center()
            .w(btn_w)
            .h_full()
            .text_sm()
            .text_color(btn_text)
            .hover(|s| s.bg(close_hover_bg).text_color(gpui::white()))
            .window_control_area(WindowControlArea::Close)
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
        let close_btn = close_btn.child("\u{00D7}"); // ×

        div()
            .flex()
            .items_center()
            .w_full()
            .flex_shrink_0()
            .h(px(38.0))
            .bg(titlebar_bg)
            .border_b_1()
            .border_color(titlebar_border)
            .child(title_area)
            .child(
                div()
                    .flex()
                    .items_center()
                    .h_full()
                    .child(minimize_btn)
                    .child(maximize_btn)
                    .child(close_btn),
            )
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
        let titlebar = Self::render_titlebar(is_maximized, &handle, _cx);

        root = root
            .child(titlebar)
            .child(main_area)
            .child(self.status_bar.clone());

        // Command palette overlay
        root = root.child(self.command_palette.clone());

        // Toast notification overlay
        root = root.child(self.toasts.clone());

        // Modal form overlays — render an occluding backdrop at the workspace
        // level so hover/click on elements behind is properly blocked.
        let has_modal = self.connection_form.is_some()
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
