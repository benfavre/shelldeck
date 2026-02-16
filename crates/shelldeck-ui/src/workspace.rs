use adabraka_ui::prelude::{install_theme, Theme};
use gpui::prelude::*;
use gpui::*;
use shelldeck_core::config::app_config::{AppConfig, ThemePreference};
use shelldeck_core::config::store::ConnectionStore;
use shelldeck_core::config::themes::TerminalTheme;
use shelldeck_core::models::connection::{Connection, ConnectionStatus};
use shelldeck_core::models::managed_site::ManagedSite;
use shelldeck_core::models::port_forward::{ForwardDirection, ForwardStatus};
use shelldeck_core::models::script::{ScriptLanguage, ScriptTarget};
use shelldeck_core::models::script_runner::build_command;
use shelldeck_core::models::server_sync::SyncProfile;
use shelldeck_core::models::templates::all_templates;
use shelldeck_ssh::client::SshClient;
use shelldeck_ssh::tunnel::TunnelHandle;
use shelldeck_terminal::session::TerminalSession;
use std::collections::HashMap;
use std::ops::DerefMut;
use uuid::Uuid;

use crate::command_palette::{
    CommandPalette, CommandPaletteEvent, PaletteAction, ToggleCommandPalette,
};
use crate::connection_form::{ConnectionForm, ConnectionFormEvent};
use crate::dashboard::{ActivityEvent, ActivityType, DashboardEvent, DashboardView};
use crate::port_forward_form::{PortForwardForm, PortForwardFormEvent};
use crate::port_forward_view::{PortForwardEvent, PortForwardView};
use crate::script_editor::{ScriptEditorView, ScriptEvent};
use crate::script_form::{ScriptForm, ScriptFormEvent};
use crate::server_sync_view::{PanelSide, ServerSyncEvent, ServerSyncView, LOCAL_MACHINE_ID};
use crate::settings::{SettingsEvent, SettingsView};
use crate::sidebar::{SidebarEvent, SidebarSection, SidebarView};
use crate::sites_view::{SitesEvent, SitesView};
use crate::status_bar::StatusBar;
use crate::template_browser::{TemplateBrowser, TemplateBrowserEvent};
use crate::terminal_view::{SplitDirection, TerminalEvent, TerminalView};
use crate::theme::ShellDeckColors;
use crate::toast::{ToastContainer, ToastLevel};
use crate::variable_prompt::{VariablePrompt, VariablePromptEvent};
use shelldeck_update::{AutoUpdateEvent, AutoUpdateStatus, AutoUpdater};

/// The active content view
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActiveView {
    Dashboard,
    Terminal,
    Scripts,
    PortForwards,
    ServerSync,
    Sites,
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
    _form_sub: Option<Subscription>,
    _pf_form_sub: Option<Subscription>,
    _dashboard_sub: Subscription,
    _script_form_sub: Option<Subscription>,
    _template_browser_sub: Option<Subscription>,
    _variable_prompt_sub: Option<Subscription>,
    _git_poll_task: Option<gpui::Task<()>>,
    auto_updater: Entity<AutoUpdater>,
    _update_sub: Subscription,
    /// Connection ID pending deletion (requires second click to confirm).
    pending_delete: Option<Uuid>,
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
        let auto_update_enabled = config.general.auto_update;
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
            palette.set_actions(vec![
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
            ]);
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
            sidebar_width: 260.0,
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
            _dashboard_sub: dashboard_sub,
            _form_sub: None,
            _pf_form_sub: None,
            _script_form_sub: None,
            _template_browser_sub: None,
            _variable_prompt_sub: None,
            _git_poll_task: Some(git_poll_task),
            auto_updater,
            _update_sub: update_sub,
            pending_delete: None,
        }
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
                cx.notify();
            }
            TerminalEvent::TabSelected(id) => {
                tracing::info!("Terminal tab selected: {}", id);
            }
            TerminalEvent::TabClosed(id) => {
                tracing::info!("Terminal tab closed: {}", id);
                self.update_dashboard_stats(cx);
                cx.notify();
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
                // Apply terminal settings to running view
                self.terminal.update(cx, |terminal, cx| {
                    terminal.set_font_size(config.terminal.font_size);
                    terminal.set_font_family(config.terminal.font_family.clone());
                    terminal.set_cursor_style(&config.terminal.cursor_style);
                    cx.notify();
                });
                // Apply sidebar width
                self.sidebar_width = config.general.sidebar_width;
                self.terminal.update(cx, |terminal, _cx| {
                    terminal.set_sidebar_width(config.general.sidebar_width);
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

                // Apply a terminal theme matching the preference
                let terminal_theme = if is_dark {
                    TerminalTheme::dark()
                } else {
                    TerminalTheme::light()
                };
                self.terminal.update(cx, |terminal, cx| {
                    terminal.set_terminal_theme(&terminal_theme);
                    cx.notify();
                });

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
            SettingsEvent::TerminalThemeChanged(theme) => {
                tracing::info!("Terminal theme changed to: {}", theme.name);
                self.terminal.update(cx, |terminal, cx| {
                    terminal.set_terminal_theme(theme);
                    cx.notify();
                });
                cx.notify();
            }
        }
    }

    fn handle_script_event(&mut self, event: &ScriptEvent, cx: &mut Context<Self>) {
        match event {
            ScriptEvent::RunScript(script) => {
                // Guard: don't start if already running
                if self.scripts.read(cx).is_running() {
                    self.show_toast("A script is already running", ToastLevel::Warning, cx);
                    return;
                }

                // Check for template variables — show prompt if any exist
                let resolved = script.resolved_variables();
                if !resolved.is_empty() {
                    self.show_variable_prompt(script.clone(), resolved, cx);
                    return;
                }

                tracing::info!("Running script: {}", script.name);
                let cmd = build_command(script, None);
                let script_name = script.name.clone();
                let script_id = script.id;
                let connection_id = match &script.target {
                    ScriptTarget::Remote(cid) => Some(*cid),
                    _ => None,
                };

                // Create execution record
                let record = shelldeck_core::models::execution::ExecutionRecord::new(
                    script_id,
                    connection_id,
                );

                let display_cmd = if matches!(script.language, ScriptLanguage::Shell) {
                    format!("$ {}", script.body)
                } else {
                    format!("$ [{}] {}", script.language.label(), cmd.ssh_command)
                };

                self.scripts.update(cx, |editor, _| {
                    editor.running_script_id = Some(script_id);
                    editor.execution_output.clear();
                    editor.execution_output.push(display_cmd);
                    editor.history.push(record);
                });

                // Update last_run / run_count on the script
                self.scripts.update(cx, |editor, _| {
                    if let Some(s) = editor.scripts.iter_mut().find(|s| s.id == script_id) {
                        s.last_run = Some(chrono::Utc::now());
                        s.run_count += 1;
                    }
                });
                // Persist run stats to store
                if let Some(s) = self
                    .scripts
                    .read(cx)
                    .scripts
                    .iter()
                    .find(|s| s.id == script_id)
                    .cloned()
                {
                    let _ = self.store.update_script(s);
                }

                self.add_activity(
                    format!("Running script: {}", script_name),
                    ActivityType::Script,
                    cx,
                );
                self.show_toast(
                    format!("Running script: {}", script_name),
                    ToastLevel::Info,
                    cx,
                );
                self.update_dashboard_stats(cx);

                // Route based on script target
                match &script.target {
                    ScriptTarget::Remote(connection_id) => {
                        let connection = self
                            .connections
                            .iter()
                            .find(|c| c.id == *connection_id)
                            .cloned();
                        if let Some(conn) = connection {
                            self.run_script_remote(
                                cmd.ssh_command.clone(),
                                script_name,
                                script_id,
                                conn,
                                cx,
                            );
                        } else {
                            tracing::error!(
                                "Connection {} not found for remote script",
                                connection_id
                            );
                            self.scripts.update(cx, |editor, cx| {
                                editor.running_script_id = None;
                                editor
                                    .execution_output
                                    .push(format!("Error: Connection {} not found", connection_id));
                                cx.notify();
                            });
                            self.show_toast("Remote connection not found", ToastLevel::Error, cx);
                            self.update_dashboard_stats(cx);
                        }
                    }
                    ScriptTarget::Local | ScriptTarget::AskOnRun => {
                        self.run_script_local_cmd(
                            cmd.local_binary.clone(),
                            cmd.local_args.clone(),
                            cmd.env_vars.clone(),
                            script_name,
                            script_id,
                            cx,
                        );
                    }
                }

                // Sync favorites/recent to terminal toolbar
                self.sync_scripts_to_terminal_toolbar(cx);

                cx.notify();
            }
            ScriptEvent::StopScript => {
                let script_id = self.scripts.read(cx).running_script_id;
                if let Some(sid) = script_id {
                    if let Some(active) = self.active_scripts.remove(&sid) {
                        active.stop();
                    }

                    self.scripts.update(cx, |editor, cx| {
                        editor.running_script_id = None;
                        editor
                            .execution_output
                            .push("[Script cancelled]".to_string());
                        // Finalize the last execution record
                        if let Some(record) = editor.history.last_mut() {
                            record.finish(-1);
                        }
                        cx.notify();
                    });

                    self.show_toast("Script cancelled", ToastLevel::Info, cx);
                    self.update_dashboard_stats(cx);
                    cx.notify();
                }
            }
            ScriptEvent::AddScript => {
                self.show_script_form(cx);
            }
            ScriptEvent::EditScript(id) => {
                if let Some(script) = self
                    .scripts
                    .read(cx)
                    .scripts
                    .iter()
                    .find(|s| s.id == *id)
                    .cloned()
                {
                    self.show_script_form_edit(&script, cx);
                }
            }
            ScriptEvent::UpdateScript(script) => {
                tracing::info!("Script updated (inline): {}", script.name);
                // Update in store
                match self.store.update_script(script.clone()) {
                    Ok(true) => {}
                    Ok(false) => {
                        if let Err(e) = self.store.add_script(script.clone()) {
                            tracing::error!("Failed to save script: {}", e);
                            self.show_toast(
                                format!("Failed to save script: {}", e),
                                ToastLevel::Error,
                                cx,
                            );
                        }
                    }
                    Err(e) => {
                        tracing::error!("Failed to update script: {}", e);
                        self.show_toast(
                            format!("Failed to update script: {}", e),
                            ToastLevel::Error,
                            cx,
                        );
                    }
                }
                // Update in script editor view
                self.scripts.update(cx, |editor, _| {
                    if let Some(existing) = editor.scripts.iter_mut().find(|s| s.id == script.id) {
                        *existing = script.clone();
                    }
                });
                self.add_activity(
                    format!("Updated script: {}", script.name),
                    ActivityType::Script,
                    cx,
                );
                self.show_toast(
                    format!("Script updated: {}", script.name),
                    ToastLevel::Success,
                    cx,
                );
                cx.notify();
            }
            ScriptEvent::ClearOutput => {}
            ScriptEvent::ToggleFavorite(id) => {
                let id = *id;
                self.scripts.update(cx, |editor, _| {
                    if let Some(s) = editor.scripts.iter_mut().find(|s| s.id == id) {
                        s.is_favorite = !s.is_favorite;
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
            ScriptEvent::DeleteScript(id) => {
                let id = *id;
                let name = self
                    .scripts
                    .read(cx)
                    .scripts
                    .iter()
                    .find(|s| s.id == id)
                    .map(|s| s.name.clone());
                self.scripts.update(cx, |editor, _| {
                    editor.scripts.retain(|s| s.id != id);
                    if editor.selected_script == Some(id) {
                        editor.selected_script = None;
                    }
                });
                let _ = self.store.remove_script(id);
                if let Some(name) = name {
                    self.show_toast(format!("Deleted script: {}", name), ToastLevel::Info, cx);
                }
                self.sync_scripts_to_terminal_toolbar(cx);
                cx.notify();
            }
            ScriptEvent::ImportTemplate(template_id) => {
                let template_id = template_id.clone();
                if let Some(tmpl) = all_templates().iter().find(|t| t.id == template_id) {
                    let script = tmpl.to_script();
                    let name = script.name.clone();
                    self.scripts.update(cx, |editor, _| {
                        editor.scripts.push(script.clone());
                    });
                    let _ = self.store.add_script(script);
                    self.show_toast(
                        format!("Imported template: {}", name),
                        ToastLevel::Success,
                        cx,
                    );
                    self.sync_scripts_to_terminal_toolbar(cx);
                    cx.notify();
                }
            }
            ScriptEvent::RunScriptById(id) => {
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
        }
    }

    fn run_script_local_cmd(
        &mut self,
        binary: String,
        args: Vec<String>,
        env_vars: Vec<(String, String)>,
        script_name: String,
        script_id: Uuid,
        cx: &mut Context<Self>,
    ) {
        use std::io::{BufRead, BufReader};
        use std::process::{Command, Stdio};

        let (shutdown_tx, shutdown_rx) = tokio::sync::mpsc::channel::<()>(1);
        let (stream_tx, stream_rx) = std::sync::mpsc::channel::<String>();
        let (done_tx, done_rx) = std::sync::mpsc::channel::<Option<i32>>();

        let thread_handle = std::thread::Builder::new()
            .name(format!("script-local-{}", script_id))
            .spawn(move || {
                let mut cmd = Command::new(&binary);
                cmd.args(&args)
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped());
                for (k, v) in &env_vars {
                    cmd.env(k, v);
                }
                let mut child = match cmd.spawn() {
                    Ok(c) => c,
                    Err(e) => {
                        let _ = stream_tx.send(format!("Error: {}", e));
                        let _ = done_tx.send(None);
                        return;
                    }
                };

                // Spawn reader threads for stdout and stderr
                let stdout = child.stdout.take();
                let stderr = child.stderr.take();
                let stream_tx2 = stream_tx.clone();

                let stdout_thread = std::thread::spawn(move || {
                    if let Some(stdout) = stdout {
                        for line in BufReader::new(stdout).lines() {
                            match line {
                                Ok(l) => {
                                    let _ = stream_tx.send(l);
                                }
                                Err(_) => break,
                            }
                        }
                    }
                });

                let stderr_thread = std::thread::spawn(move || {
                    if let Some(stderr) = stderr {
                        let mut first = true;
                        for line in BufReader::new(stderr).lines() {
                            match line {
                                Ok(l) => {
                                    if first {
                                        let _ = stream_tx2.send("--- stderr ---".to_string());
                                        first = false;
                                    }
                                    let _ = stream_tx2.send(l);
                                }
                                Err(_) => break,
                            }
                        }
                    }
                });

                // Create a blocking receiver for the shutdown signal
                let mut shutdown_rx = shutdown_rx;

                // Poll for completion or cancellation
                loop {
                    match child.try_wait() {
                        Ok(Some(status)) => {
                            let _ = stdout_thread.join();
                            let _ = stderr_thread.join();
                            let _ = done_tx.send(status.code());
                            return;
                        }
                        Ok(None) => {}
                        Err(e) => {
                            let _ = stdout_thread.join();
                            let _ = stderr_thread.join();
                            let _ = done_tx.send(None);
                            tracing::error!("Error waiting for child process: {}", e);
                            return;
                        }
                    }

                    // Check shutdown (non-blocking)
                    match shutdown_rx.try_recv() {
                        Ok(()) => {
                            // Kill the child process
                            let _ = child.kill();
                            let _ = child.wait(); // reap
                            let _ = stdout_thread.join();
                            let _ = stderr_thread.join();
                            let _ = done_tx.send(Some(-1));
                            return;
                        }
                        Err(tokio::sync::mpsc::error::TryRecvError::Empty) => {}
                        Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                            // Sender dropped — treat as cancellation
                            let _ = child.kill();
                            let _ = child.wait();
                            let _ = stdout_thread.join();
                            let _ = stderr_thread.join();
                            let _ = done_tx.send(Some(-1));
                            return;
                        }
                    }

                    std::thread::sleep(std::time::Duration::from_millis(50));
                }
            })
            .expect("Failed to spawn local script thread");

        self.active_scripts.insert(
            script_id,
            ActiveScript {
                shutdown_tx,
                _thread: Some(thread_handle),
            },
        );

        // UI poller: drains output and handles completion
        let scripts_handle = self.scripts.downgrade();
        let script_name_done = script_name;
        cx.spawn(async move |_this, cx: &mut AsyncApp| {
            loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(50))
                    .await;

                // Drain output lines
                let mut lines = Vec::new();
                while let Ok(line) = stream_rx.try_recv() {
                    lines.push(line);
                }

                if !lines.is_empty() {
                    let _ = scripts_handle.update(cx, |editor, cx| {
                        for line in &lines {
                            editor.execution_output.push(line.clone());
                        }
                        // Also append to execution record
                        if let Some(record) = editor.history.last_mut() {
                            for line in &lines {
                                record.append_output(line);
                                record.append_output("\n");
                            }
                        }
                        cx.notify();
                    });
                }

                // Check if done
                match done_rx.try_recv() {
                    Ok(exit_code) => {
                        let _ = scripts_handle.update(cx, |editor, cx| {
                            editor.running_script_id = None;
                            let code = exit_code.unwrap_or(-1);
                            editor.execution_output.push(format!("Exit code: {}", code));
                            // Finalize execution record
                            if let Some(record) = editor.history.last_mut() {
                                record.finish(code);
                            }
                            cx.notify();
                        });
                        let _ = _this.update(cx, |ws, cx| {
                            ws.active_scripts.remove(&script_id);
                            ws.update_dashboard_stats(cx);
                            match exit_code {
                                Some(0) => {
                                    ws.show_toast(
                                        format!(
                                            "Script '{}' completed successfully",
                                            script_name_done
                                        ),
                                        ToastLevel::Success,
                                        cx,
                                    );
                                }
                                Some(code) => {
                                    ws.show_toast(
                                        format!(
                                            "Script '{}' exited with code {}",
                                            script_name_done, code
                                        ),
                                        ToastLevel::Error,
                                        cx,
                                    );
                                }
                                None => {
                                    ws.show_toast(
                                        format!("Script '{}' failed to execute", script_name_done),
                                        ToastLevel::Error,
                                        cx,
                                    );
                                }
                            }
                        });
                        break;
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => {}
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        // Thread exited without sending done — clean up
                        let _ = scripts_handle.update(cx, |editor, cx| {
                            editor.running_script_id = None;
                            editor
                                .execution_output
                                .push("[Script thread exited unexpectedly]".to_string());
                            cx.notify();
                        });
                        let _ = _this.update(cx, |ws, cx| {
                            ws.active_scripts.remove(&script_id);
                            ws.update_dashboard_stats(cx);
                        });
                        break;
                    }
                }
            }
        })
        .detach();
    }

    fn run_script_remote(
        &mut self,
        body: String,
        script_name: String,
        script_id: Uuid,
        connection: Connection,
        cx: &mut Context<Self>,
    ) {
        let host_display = connection.display_name().to_string();

        self.scripts.update(cx, |editor, _| {
            editor
                .execution_output
                .push(format!("[remote: {}]", host_display));
        });

        let (shutdown_tx, shutdown_rx) = tokio::sync::mpsc::channel::<()>(1);
        let (stream_tx, stream_rx) = std::sync::mpsc::channel::<String>();
        let (done_tx, done_rx) = std::sync::mpsc::channel::<Option<i32>>();

        let thread_handle = std::thread::Builder::new()
            .name(format!("script-remote-{}", script_id))
            .spawn(move || {
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("Failed to create tokio runtime for remote script");

                rt.block_on(async move {
                    let client = SshClient::new();
                    let session = match client.connect(&connection).await {
                        Ok(s) => s,
                        Err(e) => {
                            let _ = stream_tx.send(format!("Error: SSH connection failed: {}", e));
                            let _ = done_tx.send(None);
                            return;
                        }
                    };

                    // Create channel for exec_cancellable output
                    let (output_tx, mut output_rx) =
                        tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();

                    // Forward tokio output_rx -> std stream_tx
                    let fwd_stream_tx = stream_tx.clone();
                    let fwd_task = tokio::spawn(async move {
                        while let Some(data) = output_rx.recv().await {
                            let text = String::from_utf8_lossy(&data);
                            for line in text.lines() {
                                let _ = fwd_stream_tx.send(line.to_string());
                            }
                        }
                    });

                    let result = session
                        .exec_cancellable(&body, output_tx, shutdown_rx)
                        .await;

                    // Wait for forwarding to flush
                    let _ = fwd_task.await;

                    match result {
                        Ok(exit_code) => {
                            let _ = done_tx.send(exit_code.map(|c| c as i32));
                        }
                        Err(e) => {
                            let _ = stream_tx.send(format!("Error: {}", e));
                            let _ = done_tx.send(None);
                        }
                    }
                });
            })
            .expect("Failed to spawn remote script thread");

        self.active_scripts.insert(
            script_id,
            ActiveScript {
                shutdown_tx,
                _thread: Some(thread_handle),
            },
        );

        // UI poller
        let scripts_handle = self.scripts.downgrade();
        let script_name_done = script_name;
        cx.spawn(async move |_this, cx: &mut AsyncApp| {
            loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(50))
                    .await;

                // Drain output lines
                let mut lines = Vec::new();
                while let Ok(line) = stream_rx.try_recv() {
                    lines.push(line);
                }

                if !lines.is_empty() {
                    let _ = scripts_handle.update(cx, |editor, cx| {
                        for line in &lines {
                            editor.execution_output.push(line.clone());
                        }
                        if let Some(record) = editor.history.last_mut() {
                            for line in &lines {
                                record.append_output(line);
                                record.append_output("\n");
                            }
                        }
                        cx.notify();
                    });
                }

                // Check if done
                match done_rx.try_recv() {
                    Ok(exit_code) => {
                        let _ = scripts_handle.update(cx, |editor, cx| {
                            editor.running_script_id = None;
                            let code = exit_code.unwrap_or(-1);
                            editor.execution_output.push(format!("Exit code: {}", code));
                            if let Some(record) = editor.history.last_mut() {
                                record.finish(code);
                            }
                            cx.notify();
                        });
                        let _ = _this.update(cx, |ws, cx| {
                            ws.active_scripts.remove(&script_id);
                            ws.update_dashboard_stats(cx);
                            match exit_code {
                                Some(0) | None => {
                                    ws.show_toast(
                                        format!(
                                            "Script '{}' completed on {}",
                                            script_name_done, host_display
                                        ),
                                        ToastLevel::Success,
                                        cx,
                                    );
                                }
                                Some(code) => {
                                    ws.show_toast(
                                        format!(
                                            "Script '{}' exited with code {} on {}",
                                            script_name_done, code, host_display
                                        ),
                                        ToastLevel::Error,
                                        cx,
                                    );
                                }
                            }
                        });
                        break;
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => {}
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        let _ = scripts_handle.update(cx, |editor, cx| {
                            editor.running_script_id = None;
                            editor
                                .execution_output
                                .push("[Remote script thread exited unexpectedly]".to_string());
                            cx.notify();
                        });
                        let _ = _this.update(cx, |ws, cx| {
                            ws.active_scripts.remove(&script_id);
                            ws.update_dashboard_stats(cx);
                        });
                        break;
                    }
                }
            }
        })
        .detach();
    }

    /// Push favorite and recent scripts to the terminal toolbar.
    fn sync_scripts_to_terminal_toolbar(&self, cx: &mut Context<Self>) {
        let scripts = &self.scripts.read(cx).scripts;

        let favorites: Vec<(Uuid, String, ScriptLanguage)> = scripts
            .iter()
            .filter(|s| s.is_favorite)
            .map(|s| (s.id, s.name.clone(), s.language.clone()))
            .collect();

        let mut recent: Vec<_> = scripts
            .iter()
            .filter(|s| s.last_run.is_some())
            .collect::<Vec<_>>();
        recent.sort_by_key(|s| std::cmp::Reverse(s.last_run));
        let recent: Vec<(Uuid, String, ScriptLanguage)> = recent
            .into_iter()
            .take(5)
            .map(|s| (s.id, s.name.clone(), s.language.clone()))
            .collect();

        self.terminal.update(cx, |tv, _| {
            tv.set_scripts(favorites, recent);
        });
    }

    fn handle_forward_event(&mut self, event: &PortForwardEvent, cx: &mut Context<Self>) {
        match event {
            PortForwardEvent::StartForward(id) => {
                let forward_id = *id;
                tracing::info!("Start forward requested: {}", forward_id);

                // Look up the port forward configuration
                let forward = {
                    let pf_view = self.port_forwards.read(cx);
                    pf_view
                        .forwards
                        .iter()
                        .find(|f| f.id == forward_id)
                        .cloned()
                };
                let forward = match forward {
                    Some(f) => f,
                    None => {
                        tracing::error!("Port forward not found: {}", forward_id);
                        self.add_activity(
                            format!("Port forward not found: {}", forward_id),
                            ActivityType::Error,
                            cx,
                        );
                        self.show_toast("Port forward not found", ToastLevel::Error, cx);
                        return;
                    }
                };

                // Don't start if already active
                if self.active_tunnels.contains_key(&forward_id) {
                    tracing::warn!("Port forward {} is already active", forward_id);
                    return;
                }

                // Look up the connection for this forward
                let connection = self
                    .connections
                    .iter()
                    .find(|c| c.id == forward.connection_id)
                    .cloned();
                let connection = match connection {
                    Some(c) => c,
                    None => {
                        tracing::error!(
                            "Connection {} not found for port forward {}",
                            forward.connection_id,
                            forward_id
                        );
                        self.port_forwards.update(cx, |pf, _| {
                            if let Some(f) = pf.forwards.iter_mut().find(|f| f.id == forward_id) {
                                f.status = ForwardStatus::Error;
                            }
                        });
                        self.add_activity(
                            "Connection not found for port forward".to_string(),
                            ActivityType::Error,
                            cx,
                        );
                        self.show_toast(
                            "Connection not found for port forward",
                            ToastLevel::Error,
                            cx,
                        );
                        cx.notify();
                        return;
                    }
                };

                let label = forward
                    .label
                    .clone()
                    .unwrap_or_else(|| forward.description());

                // Update status to show we're starting
                self.port_forwards.update(cx, |pf, _| {
                    if let Some(f) = pf.forwards.iter_mut().find(|f| f.id == forward_id) {
                        f.status = ForwardStatus::Active;
                    }
                });

                self.add_activity(
                    format!("Starting port forward: {}", label),
                    ActivityType::Forward,
                    cx,
                );
                self.show_toast(
                    format!("Starting port forward: {}", label),
                    ToastLevel::Info,
                    cx,
                );

                // Use a channel to send the TunnelHandle back from the background thread
                let (result_tx, result_rx) =
                    std::sync::mpsc::channel::<Result<TunnelHandle, String>>();

                let direction = forward.direction;
                let local_port = forward.local_port;
                let remote_host = forward.remote_host.clone();
                let remote_port = forward.remote_port;
                let local_host = forward.local_host.clone();

                // Spawn a dedicated thread with its own tokio runtime for the SSH tunnel.
                // The thread stays alive as long as the tunnel is running; the tokio runtime
                // drives the TcpListener accept loop inside start_local_forward/start_remote_forward.
                let thread_handle = std::thread::Builder::new()
                    .name(format!("tunnel-{}", forward_id))
                    .spawn(move || {
                        let rt = tokio::runtime::Builder::new_current_thread()
                            .enable_all()
                            .build()
                            .expect("Failed to create tokio runtime for tunnel");

                        rt.block_on(async move {
                            // Establish SSH connection
                            let client = SshClient::new();
                            let mut session = match client.connect(&connection).await {
                                Ok(s) => s,
                                Err(e) => {
                                    let msg = format!("SSH connection failed: {}", e);
                                    tracing::error!("{}", msg);
                                    let _ = result_tx.send(Err(msg));
                                    return;
                                }
                            };
                            tracing::info!(
                                "SSH connected for tunnel to {}",
                                connection.display_name()
                            );

                            let shared_handle = session.shared_handle();
                            let mut tunnel_manager = shelldeck_ssh::tunnel::TunnelManager::new();

                            let tunnel_result = match direction {
                                ForwardDirection::LocalToRemote => {
                                    tunnel_manager
                                        .start_local_forward(
                                            shared_handle,
                                            local_port,
                                            remote_host,
                                            remote_port,
                                        )
                                        .await
                                }
                                ForwardDirection::RemoteToLocal => {
                                    let forwarded_rx = session
                                        .take_forwarded_tcpip_rx()
                                        .expect("forwarded_tcpip_rx already taken");
                                    tunnel_manager
                                        .start_remote_forward(
                                            shared_handle,
                                            remote_port,
                                            local_host,
                                            local_port,
                                            forwarded_rx,
                                        )
                                        .await
                                }
                                ForwardDirection::Dynamic => {
                                    // Dynamic/SOCKS not yet implemented in TunnelManager,
                                    // fall back to local forward as a reasonable approximation
                                    tunnel_manager
                                        .start_local_forward(
                                            shared_handle,
                                            local_port,
                                            remote_host,
                                            remote_port,
                                        )
                                        .await
                                }
                            };

                            match tunnel_result {
                                Ok(_tunnel_id) => {
                                    // Create a proxy shutdown channel so the UI thread can
                                    // signal this background thread to tear down the tunnel.
                                    let (thread_shutdown_tx, mut thread_shutdown_rx) =
                                        tokio::sync::mpsc::channel::<()>(1);

                                    // Build a proxy TunnelHandle that shares the real tunnel's
                                    // Arc-wrapped status and byte counters but uses the
                                    // thread-level shutdown channel.
                                    let tunnel_ref = &tunnel_manager.tunnels()
                                        [tunnel_manager.tunnels().len() - 1];
                                    let proxy_handle = TunnelHandle::new_proxy(
                                        tunnel_ref.id,
                                        tunnel_ref.status.clone(),
                                        tunnel_ref.bytes_sent.clone(),
                                        tunnel_ref.bytes_received.clone(),
                                        thread_shutdown_tx,
                                    );

                                    let _ = result_tx.send(Ok(proxy_handle));

                                    // Park this thread -- keep the tokio runtime alive so the
                                    // tunnel's spawned tasks continue to run. Wait for shutdown.
                                    thread_shutdown_rx.recv().await;

                                    // Shutdown received -- stop all tunnels and exit
                                    tracing::info!("Stopping tunnels for forward {}", forward_id);
                                    tunnel_manager.stop_all();

                                    // Give tunnel tasks a moment to clean up
                                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                                }
                                Err(e) => {
                                    let msg = format!("Tunnel start failed: {}", e);
                                    tracing::error!("{}", msg);
                                    let _ = result_tx.send(Err(msg));
                                }
                            }
                        });
                    })
                    .expect("Failed to spawn tunnel thread");

                // Now wait for the result from the background thread.
                // We use cx.spawn to avoid blocking the UI thread.
                let pf_handle = self.port_forwards.downgrade();
                let dashboard_handle = self.dashboard.downgrade();
                let weak_self = cx.entity().downgrade();
                let label_for_activity = label.clone();

                cx.spawn(async move |_this, cx: &mut AsyncApp| {
                    // Wait for the result on the background executor so we don't block GPUI
                    let result = cx
                        .background_executor()
                        .spawn(async move {
                            // The SSH connection + tunnel setup happens on the dedicated thread.
                            // We give it a generous timeout.
                            result_rx.recv_timeout(std::time::Duration::from_secs(30))
                        })
                        .await;

                    match result {
                        Ok(Ok(tunnel_handle)) => {
                            tracing::info!(
                                "Tunnel started successfully for forward {}",
                                forward_id
                            );

                            // Store the active tunnel in the workspace
                            let _ = weak_self.update(cx, |ws, cx| {
                                ws.active_tunnels.insert(
                                    forward_id,
                                    ActiveTunnel {
                                        tunnel_handle,
                                        _thread: thread_handle,
                                    },
                                );

                                // Update forward status to Active
                                ws.port_forwards.update(cx, |pf, _| {
                                    if let Some(f) =
                                        pf.forwards.iter_mut().find(|f| f.id == forward_id)
                                    {
                                        f.status = ForwardStatus::Active;
                                    }
                                });

                                ws.add_activity(
                                    format!("Port forward active: {}", label_for_activity),
                                    ActivityType::Forward,
                                    cx,
                                );
                                ws.show_toast(
                                    format!("Port forward active: {}", label_for_activity),
                                    ToastLevel::Success,
                                    cx,
                                );
                                ws.update_dashboard_stats(cx);
                                cx.notify();
                            });
                        }
                        Ok(Err(err_msg)) => {
                            tracing::error!(
                                "Tunnel failed for forward {}: {}",
                                forward_id,
                                err_msg
                            );

                            let _ = pf_handle.update(cx, |pf, cx| {
                                if let Some(f) = pf.forwards.iter_mut().find(|f| f.id == forward_id)
                                {
                                    f.status = ForwardStatus::Error;
                                }
                                cx.notify();
                            });

                            let _ = dashboard_handle.update(cx, |dashboard, _| {
                                dashboard.recent_activity.insert(
                                    0,
                                    ActivityEvent {
                                        icon: "alert",
                                        message: format!("Port forward failed: {}", err_msg),
                                        timestamp: chrono::Local::now()
                                            .format("%H:%M:%S")
                                            .to_string(),
                                        event_type: ActivityType::Error,
                                    },
                                );
                                if dashboard.recent_activity.len() > 50 {
                                    dashboard.recent_activity.truncate(50);
                                }
                            });

                            let _ = weak_self.update(cx, |ws, cx| {
                                ws.show_toast(
                                    format!("Port forward failed: {}", err_msg),
                                    ToastLevel::Error,
                                    cx,
                                );
                            });
                        }
                        Err(_timeout) => {
                            tracing::error!("Tunnel setup timed out for forward {}", forward_id);

                            let _ = pf_handle.update(cx, |pf, cx| {
                                if let Some(f) = pf.forwards.iter_mut().find(|f| f.id == forward_id)
                                {
                                    f.status = ForwardStatus::Error;
                                }
                                cx.notify();
                            });

                            let _ = dashboard_handle.update(cx, |dashboard, _| {
                                dashboard.recent_activity.insert(
                                    0,
                                    ActivityEvent {
                                        icon: "alert",
                                        message: format!(
                                            "Port forward timed out: {}",
                                            label_for_activity
                                        ),
                                        timestamp: chrono::Local::now()
                                            .format("%H:%M:%S")
                                            .to_string(),
                                        event_type: ActivityType::Error,
                                    },
                                );
                                if dashboard.recent_activity.len() > 50 {
                                    dashboard.recent_activity.truncate(50);
                                }
                            });

                            let _ = weak_self.update(cx, |ws, cx| {
                                ws.show_toast(
                                    format!("Port forward timed out: {}", label_for_activity),
                                    ToastLevel::Warning,
                                    cx,
                                );
                            });
                        }
                    }
                })
                .detach();

                cx.notify();
            }
            PortForwardEvent::StopForward(id) => {
                let forward_id = *id;
                tracing::info!("Stop forward requested: {}", forward_id);

                // Look up and remove the active tunnel
                if let Some(active_tunnel) = self.active_tunnels.remove(&forward_id) {
                    // Signal the tunnel to stop. This sends through the shutdown channel
                    // which causes the background thread's tokio runtime to stop the
                    // TunnelManager and exit.
                    active_tunnel.tunnel_handle.stop();

                    // Capture final byte counts before we drop the handle
                    let (final_sent, final_recv) = active_tunnel.tunnel_handle.total_bytes();

                    // Update forward status to Inactive
                    self.port_forwards.update(cx, |pf, _| {
                        if let Some(f) = pf.forwards.iter_mut().find(|f| f.id == forward_id) {
                            f.status = ForwardStatus::Inactive;
                            f.bytes_sent = final_sent;
                            f.bytes_received = final_recv;
                        }
                    });

                    let label = {
                        let pf_view = self.port_forwards.read(cx);
                        pf_view
                            .forwards
                            .iter()
                            .find(|f| f.id == forward_id)
                            .and_then(|f| f.label.clone())
                            .unwrap_or_else(|| format!("forward {}", forward_id))
                    };

                    self.add_activity(
                        format!("Stopped port forward: {}", label),
                        ActivityType::Forward,
                        cx,
                    );
                    self.show_toast(
                        format!("Stopped port forward: {}", label),
                        ToastLevel::Info,
                        cx,
                    );

                    tracing::info!("Port forward {} stopped", forward_id);
                } else {
                    tracing::warn!("No active tunnel found for forward {}", forward_id);

                    // Even if we don't have a tracked tunnel, reset status to Inactive
                    self.port_forwards.update(cx, |pf, _| {
                        if let Some(f) = pf.forwards.iter_mut().find(|f| f.id == forward_id) {
                            f.status = ForwardStatus::Inactive;
                        }
                    });

                    self.add_activity(
                        "Port forward stop requested (no active tunnel)".to_string(),
                        ActivityType::Forward,
                        cx,
                    );
                }

                self.update_dashboard_stats(cx);
                cx.notify();
            }
            PortForwardEvent::AddForward => {
                self.show_port_forward_form(cx);
            }
            PortForwardEvent::EditForward(id) => {
                if let Some(fwd) = self
                    .port_forwards
                    .read(cx)
                    .forwards
                    .iter()
                    .find(|f| f.id == *id)
                    .cloned()
                {
                    self.show_port_forward_form_edit(&fwd, cx);
                }
            }
            PortForwardEvent::AddPresetForward(preset) => {
                // Open the form pre-filled with preset values so the user can pick a connection
                self.show_port_forward_form_edit(preset, cx);
            }
        }
    }

    fn handle_server_sync_event(&mut self, event: &ServerSyncEvent, cx: &mut Context<Self>) {
        match event {
            ServerSyncEvent::ListFiles {
                connection_id,
                path,
                panel,
            } => {
                let conn_id = *connection_id;
                let path = path.clone();
                let panel = *panel;

                if conn_id == LOCAL_MACHINE_ID {
                    self.list_local_files(path, panel, cx);
                } else if let Some(conn) =
                    self.connections.iter().find(|c| c.id == conn_id).cloned()
                {
                    self.list_remote_files(conn, path, panel, cx);
                }
            }
            ServerSyncEvent::DiscoverServices {
                connection_id,
                panel,
            } => {
                let conn_id = *connection_id;
                let panel = *panel;

                if conn_id == LOCAL_MACHINE_ID {
                    self.discover_local_services(panel, cx);
                } else if let Some(conn) =
                    self.connections.iter().find(|c| c.id == conn_id).cloned()
                {
                    self.discover_remote_services(conn, panel, cx);
                }
            }
            ServerSyncEvent::StartSync(profile) => {
                self.start_sync_operation(profile.clone(), cx);
            }
            ServerSyncEvent::CancelSync(op_id) => {
                let op_id = *op_id;
                // Signal cancel via active_scripts mechanism
                if let Some(active) = self.active_scripts.get(&op_id) {
                    active.stop();
                }
                self.server_sync.update(cx, |view, _| {
                    if let Some(ref mut op) = view.active_operation {
                        if op.id == op_id {
                            op.status =
                                shelldeck_core::models::server_sync::SyncOperationStatus::Cancelled;
                        }
                    }
                });
                cx.notify();
            }
            ServerSyncEvent::SaveProfile(profile) => {
                let profile = profile.clone();
                let _ = self.store.add_sync_profile(profile.clone());
                self.server_sync.update(cx, |view, _| {
                    view.set_profiles(self.store.sync_profiles.clone());
                });
                cx.notify();
            }
            ServerSyncEvent::DeleteProfile(id) => {
                let _ = self.store.remove_sync_profile(*id);
                self.server_sync.update(cx, |view, _| {
                    view.set_profiles(self.store.sync_profiles.clone());
                    if view.selected_profile == Some(*id) {
                        view.selected_profile = None;
                    }
                });
                cx.notify();
            }
            ServerSyncEvent::ExecSync {
                source_connection_id: _,
                command: _,
                operation_id,
                item_id,
            } => {
                // Individual sync command execution — handled as part of start_sync_operation
                tracing::debug!("ExecSync for item {:?} on op {:?}", item_id, operation_id);
            }
        }
    }

    fn handle_sites_event(&mut self, event: &SitesEvent, cx: &mut Context<Self>) {
        match event {
            SitesEvent::ScanServer(conn_id) => {
                let conn_id = *conn_id;
                if let Some(conn) = self.connections.iter().find(|c| c.id == conn_id).cloned() {
                    self.sites.update(cx, |view, _| {
                        view.scans_pending += 1;
                    });
                    self.discover_for_sites(conn, cx);
                }
            }
            SitesEvent::ScanAllServers => {
                let conns: Vec<Connection> = self.connections.clone();
                let count = conns.len() as u32;
                self.sites.update(cx, |view, _| {
                    view.scans_pending += count;
                });
                for conn in conns {
                    self.discover_for_sites(conn, cx);
                }
            }
            SitesEvent::RemoveSite(id) => {
                let _ = self.store.remove_managed_site(*id);
                self.sites.update(cx, |view, _| {
                    view.set_sites(self.store.managed_sites.clone());
                });
                cx.notify();
            }
            SitesEvent::ToggleFavorite(id) => {
                let id = *id;
                if let Some(site) = self.store.managed_sites.iter_mut().find(|s| s.id == id) {
                    site.favorite = !site.favorite;
                }
                let _ = self.store.save();
                self.sites.update(cx, |view, _| {
                    view.set_sites(self.store.managed_sites.clone());
                });
                cx.notify();
            }
            SitesEvent::UpdateTags(id, tags) => {
                let id = *id;
                let tags = tags.clone();
                if let Some(site) = self.store.managed_sites.iter_mut().find(|s| s.id == id) {
                    site.tags = tags;
                }
                let _ = self.store.save();
                self.sites.update(cx, |view, _| {
                    view.set_sites(self.store.managed_sites.clone());
                });
                cx.notify();
            }
            SitesEvent::OpenInBrowser(url) => {
                let _ = open::that(url);
            }
            SitesEvent::SshToServer(conn_id) => {
                let conn_id = *conn_id;
                if let Some(conn) = self.connections.iter().find(|c| c.id == conn_id).cloned() {
                    self.connect_ssh(conn, cx);
                }
            }
            SitesEvent::AddToSync(site_id) => {
                let _ = site_id;
                self.set_active_view(ActiveView::ServerSync);
                cx.notify();
            }
            SitesEvent::CheckSiteStatus(site_id) => {
                let site_id = *site_id;
                if let Some(site) = self.store.managed_sites.iter().find(|s| s.id == site_id) {
                    let conn_id = site.connection_id;
                    let port = site.port();
                    if let Some(conn) = self.connections.iter().find(|c| c.id == conn_id).cloned() {
                        if let Some(port) = port {
                            let sites_handle = self.sites.downgrade();
                            let check_cmd = format!(
                                "ss -tlnp 2>/dev/null | grep -q ':{} ' && echo ONLINE || echo OFFLINE",
                                port
                            );

                            let (done_tx, done_rx) = std::sync::mpsc::channel::<(bool, String)>();

                            std::thread::Builder::new()
                                .name("site-status-check".to_string())
                                .spawn(move || {
                                    let rt = tokio::runtime::Builder::new_current_thread()
                                        .enable_all()
                                        .build()
                                        .expect("tokio runtime for status check");
                                    rt.block_on(async move {
                                        let client = SshClient::new();
                                        match client.connect(&conn).await {
                                            Ok(session) => match session.exec(&check_cmd).await {
                                                Ok(result) => {
                                                    let output =
                                                        String::from_utf8_lossy(&result.stdout)
                                                            .trim()
                                                            .to_string();
                                                    let online = output.contains("ONLINE");
                                                    let _ = done_tx.send((online, String::new()));
                                                }
                                                Err(e) => {
                                                    let _ = done_tx.send((false, e.to_string()));
                                                }
                                            },
                                            Err(e) => {
                                                let _ = done_tx.send((false, e.to_string()));
                                            }
                                        }
                                    });
                                })
                                .expect("spawn status-check thread");

                            cx.spawn(async move |_ws, cx: &mut AsyncApp| {
                                loop {
                                    cx.background_executor()
                                        .timer(std::time::Duration::from_millis(50))
                                        .await;
                                    if let Ok((online, err_msg)) = done_rx.try_recv() {
                                        let _ = sites_handle.update(cx, |view, cx| {
                                            if let Some(site) = view.sites.iter_mut().find(|s| s.id == site_id) {
                                                site.last_checked = Some(chrono::Utc::now());
                                                if err_msg.is_empty() {
                                                    site.status = if online {
                                                        shelldeck_core::models::managed_site::SiteStatus::Online
                                                    } else {
                                                        shelldeck_core::models::managed_site::SiteStatus::Offline
                                                    };
                                                } else {
                                                    site.status = shelldeck_core::models::managed_site::SiteStatus::Error(err_msg);
                                                }
                                            }
                                            cx.notify();
                                        });
                                        break;
                                    }
                                }
                            }).detach();
                        }
                    }
                }
            }
            SitesEvent::ClearAllSites => {
                self.store.managed_sites.clear();
                let _ = self.store.save();
                self.sites.update(cx, |view, _| {
                    view.set_sites(Vec::new());
                });
                cx.notify();
            }
            SitesEvent::RefreshSites => {
                self.sites.update(cx, |view, _| {
                    view.set_sites(self.store.managed_sites.clone());
                });
                cx.notify();
            }
        }
    }

    fn list_remote_files(
        &mut self,
        connection: Connection,
        path: String,
        panel: PanelSide,
        cx: &mut Context<Self>,
    ) {
        use shelldeck_core::models::discovery;

        let cmd = discovery::ls_command(&path);
        let fallback_cmd = discovery::ls_command_fallback(&path);
        let path_clone = path.clone();

        let (stream_tx, stream_rx) = std::sync::mpsc::channel::<String>();
        let (done_tx, done_rx) = std::sync::mpsc::channel::<bool>();

        std::thread::Builder::new()
            .name("sync-ls".to_string())
            .spawn(move || {
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("tokio runtime for ls");

                rt.block_on(async move {
                    let client = SshClient::new();
                    let session = match client.connect(&connection).await {
                        Ok(s) => s,
                        Err(e) => {
                            let _ = stream_tx.send(format!("Error: {}", e));
                            let _ = done_tx.send(false);
                            return;
                        }
                    };

                    // Try stat-based command first
                    let output = session.exec(&cmd).await;
                    match output {
                        Ok(result) => {
                            let out = String::from_utf8_lossy(&result.stdout).to_string();
                            if !out.trim().is_empty() {
                                let _ = stream_tx.send(format!("STAT:{}", out));
                                let _ = done_tx.send(true);
                            } else {
                                // Fallback to ls
                                match session.exec(&fallback_cmd).await {
                                    Ok(result) => {
                                        let out =
                                            String::from_utf8_lossy(&result.stdout).to_string();
                                        let _ = stream_tx.send(format!("LS:{}", out));
                                        let _ = done_tx.send(true);
                                    }
                                    Err(e) => {
                                        let _ = stream_tx.send(format!("Error: {}", e));
                                        let _ = done_tx.send(false);
                                    }
                                }
                            }
                        }
                        Err(_) => {
                            // Fallback to ls
                            match session.exec(&fallback_cmd).await {
                                Ok(result) => {
                                    let out = String::from_utf8_lossy(&result.stdout).to_string();
                                    let _ = stream_tx.send(format!("LS:{}", out));
                                    let _ = done_tx.send(true);
                                }
                                Err(e) => {
                                    let _ = stream_tx.send(format!("Error: {}", e));
                                    let _ = done_tx.send(false);
                                }
                            }
                        }
                    }
                });
            })
            .expect("spawn ls thread");

        let sync_handle = self.server_sync.downgrade();
        cx.spawn(async move |_this, cx: &mut AsyncApp| {
            loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(50))
                    .await;

                if let Ok(data) = stream_rx.try_recv() {
                    let path_for_parse = path_clone.clone();
                    let entries = if let Some(stripped) = data.strip_prefix("STAT:") {
                        discovery::parse_stat_output(stripped, &path_for_parse)
                    } else if let Some(stripped) = data.strip_prefix("LS:") {
                        discovery::parse_ls_output(stripped, &path_for_parse)
                    } else {
                        // Error
                        Vec::new()
                    };

                    let _ = sync_handle.update(cx, |view, cx| {
                        view.set_file_entries(panel, path_for_parse, entries);
                        cx.notify();
                    });
                }

                if done_rx.try_recv().is_ok() {
                    break;
                }
            }
        })
        .detach();
    }

    fn list_local_files(&mut self, path: String, panel: PanelSide, cx: &mut Context<Self>) {
        use shelldeck_core::models::discovery;
        let entries = discovery::list_local_files(&path);
        self.server_sync.update(cx, |view, cx| {
            view.set_file_entries(panel, path, entries);
            cx.notify();
        });
    }

    fn discover_local_services(&mut self, panel: PanelSide, cx: &mut Context<Self>) {
        use shelldeck_core::models::discovery;

        let (tx, rx) = std::sync::mpsc::channel::<(
            Vec<shelldeck_core::models::DiscoveredSite>,
            Vec<shelldeck_core::models::DiscoveredDatabase>,
        )>();

        std::thread::Builder::new()
            .name("sync-discover-local".to_string())
            .spawn(move || {
                let nginx_output = std::process::Command::new("sh")
                    .arg("-c")
                    .arg(discovery::nginx_discover_command())
                    .output()
                    .ok()
                    .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
                    .unwrap_or_default();
                let sites = discovery::parse_nginx_configs(&nginx_output);

                let mysql_output = std::process::Command::new("sh")
                    .arg("-c")
                    .arg(discovery::mysql_discover_command(""))
                    .output()
                    .ok()
                    .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
                    .unwrap_or_default();
                let mysql_dbs = discovery::parse_mysql_discovery(&mysql_output);

                let pg_output = std::process::Command::new("sh")
                    .arg("-c")
                    .arg(discovery::pg_discover_command("-U postgres"))
                    .output()
                    .ok()
                    .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
                    .unwrap_or_default();
                let pg_dbs = discovery::parse_pg_discovery(&pg_output);

                let mut all_dbs = mysql_dbs;
                all_dbs.extend(pg_dbs);

                let _ = tx.send((sites, all_dbs));
            })
            .expect("spawn local discover thread");

        let sync_handle = self.server_sync.downgrade();
        cx.spawn(async move |_this, cx: &mut AsyncApp| loop {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(50))
                .await;

            if let Ok((sites, dbs)) = rx.try_recv() {
                let _ = sync_handle.update(cx, |view, cx| {
                    view.set_discovered_sites(panel, sites);
                    view.set_discovered_databases(panel, dbs);
                    view.panel_state_mut(panel).discovery_loading = false;
                    cx.notify();
                });
                break;
            }
        })
        .detach();
    }

    fn discover_remote_services(
        &mut self,
        connection: Connection,
        panel: PanelSide,
        cx: &mut Context<Self>,
    ) {
        use shelldeck_core::models::discovery;

        let disc_conn_id = connection.id;
        let disc_conn_name = connection.display_name().to_string();

        let nginx_cmd = discovery::nginx_discover_command().to_string();
        let mysql_cmd = discovery::mysql_discover_command("");
        let pg_cmd = discovery::pg_discover_command("-U postgres");

        let (stream_tx, stream_rx) = std::sync::mpsc::channel::<(String, String)>();
        let (done_tx, done_rx) = std::sync::mpsc::channel::<bool>();

        let thread_disc_conn_name = disc_conn_name.clone();
        std::thread::Builder::new()
            .name("sync-discover".to_string())
            .spawn(move || {
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("tokio runtime for discover");

                rt.block_on(async move {
                    let client = SshClient::new();
                    let session = match tokio::time::timeout(
                        std::time::Duration::from_secs(15),
                        client.connect(&connection),
                    )
                    .await
                    {
                        Ok(Ok(s)) => s,
                        Ok(Err(e)) => {
                            let _ = stream_tx.send(("error".to_string(), format!("Error: {}", e)));
                            let _ = done_tx.send(false);
                            return;
                        }
                        Err(_) => {
                            let _ = stream_tx.send((
                                "error".to_string(),
                                format!("Connection timed out for {}", thread_disc_conn_name),
                            ));
                            let _ = done_tx.send(false);
                            return;
                        }
                    };

                    let exec_timeout = std::time::Duration::from_secs(30);

                    // Discover nginx
                    match tokio::time::timeout(exec_timeout, session.exec(&nginx_cmd)).await {
                        Ok(Ok(result)) => {
                            let output = String::from_utf8_lossy(&result.stdout).to_string();
                            let _ = stream_tx.send(("nginx".to_string(), output));
                        }
                        Ok(Err(e)) => tracing::debug!(
                            "nginx discover exec error on {}: {}",
                            thread_disc_conn_name,
                            e
                        ),
                        Err(_) => {
                            tracing::warn!("nginx discover timed out on {}", thread_disc_conn_name)
                        }
                    }

                    // Discover MySQL
                    match tokio::time::timeout(exec_timeout, session.exec(&mysql_cmd)).await {
                        Ok(Ok(result)) => {
                            let output = String::from_utf8_lossy(&result.stdout).to_string();
                            let _ = stream_tx.send(("mysql".to_string(), output));
                        }
                        Ok(Err(e)) => tracing::debug!(
                            "mysql discover exec error on {}: {}",
                            thread_disc_conn_name,
                            e
                        ),
                        Err(_) => {
                            tracing::warn!("mysql discover timed out on {}", thread_disc_conn_name)
                        }
                    }

                    // Discover PostgreSQL
                    match tokio::time::timeout(exec_timeout, session.exec(&pg_cmd)).await {
                        Ok(Ok(result)) => {
                            let output = String::from_utf8_lossy(&result.stdout).to_string();
                            let _ = stream_tx.send(("pg".to_string(), output));
                        }
                        Ok(Err(e)) => tracing::debug!(
                            "pg discover exec error on {}: {}",
                            thread_disc_conn_name,
                            e
                        ),
                        Err(_) => {
                            tracing::warn!("pg discover timed out on {}", thread_disc_conn_name)
                        }
                    }

                    let _ = done_tx.send(true);
                });
            })
            .expect("spawn discover thread");

        let sync_handle = self.server_sync.downgrade();
        cx.spawn(async move |ws_handle, cx: &mut AsyncApp| {
            let mut auto_sites: Vec<ManagedSite> = Vec::new();
            let wall_clock_start = std::time::Instant::now();
            let wall_clock_limit = std::time::Duration::from_secs(90);

            loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(50))
                    .await;

                while let Ok((kind, data)) = stream_rx.try_recv() {
                    match kind.as_str() {
                        "nginx" => {
                            let sites = discovery::parse_nginx_configs(&data);
                            for s in &sites {
                                auto_sites.push(ManagedSite::from_nginx(
                                    disc_conn_id,
                                    &disc_conn_name,
                                    s.clone(),
                                ));
                            }
                            let _ = sync_handle.update(cx, |view, cx| {
                                view.set_discovered_sites(panel, sites);
                                cx.notify();
                            });
                        }
                        "mysql" => {
                            let dbs = discovery::parse_mysql_discovery(&data);
                            for d in &dbs {
                                auto_sites.push(ManagedSite::from_database(
                                    disc_conn_id,
                                    &disc_conn_name,
                                    d.clone(),
                                ));
                            }
                            if !dbs.is_empty() {
                                let _ = sync_handle.update(cx, |view, cx| {
                                    let mut all =
                                        view.panel_state_mut(panel).discovered_databases.clone();
                                    all.extend(dbs);
                                    view.set_discovered_databases(panel, all);
                                    cx.notify();
                                });
                            }
                        }
                        "pg" => {
                            let dbs = discovery::parse_pg_discovery(&data);
                            for d in &dbs {
                                auto_sites.push(ManagedSite::from_database(
                                    disc_conn_id,
                                    &disc_conn_name,
                                    d.clone(),
                                ));
                            }
                            if !dbs.is_empty() {
                                let _ = sync_handle.update(cx, |view, cx| {
                                    let mut all =
                                        view.panel_state_mut(panel).discovered_databases.clone();
                                    all.extend(dbs);
                                    view.set_discovered_databases(panel, all);
                                    cx.notify();
                                });
                            }
                        }
                        _ => {}
                    }
                }

                if done_rx.try_recv().is_ok() {
                    let _ = sync_handle.update(cx, |view, cx| {
                        view.panel_state_mut(panel).discovery_loading = false;
                        cx.notify();
                    });
                    if !auto_sites.is_empty() {
                        let _ = ws_handle.update(cx, |ws, cx| {
                            let _ = ws.store.add_managed_sites_bulk(auto_sites);
                            ws.sites.update(cx, |view, _| {
                                view.set_sites(ws.store.managed_sites.clone());
                            });
                            cx.notify();
                        });
                    }
                    break;
                }

                // Wall-clock safety: abort if background thread is stuck
                if wall_clock_start.elapsed() > wall_clock_limit {
                    tracing::warn!(
                        "Sync discover poller timed out after 90s for {}",
                        disc_conn_name
                    );
                    let _ = sync_handle.update(cx, |view, cx| {
                        view.panel_state_mut(panel).discovery_loading = false;
                        cx.notify();
                    });
                    if !auto_sites.is_empty() {
                        let _ = ws_handle.update(cx, |ws, cx| {
                            let _ = ws.store.add_managed_sites_bulk(auto_sites);
                            ws.sites.update(cx, |view, _| {
                                view.set_sites(ws.store.managed_sites.clone());
                            });
                            cx.notify();
                        });
                    }
                    break;
                }
            }
        })
        .detach();
    }

    fn discover_for_sites(&mut self, connection: Connection, cx: &mut Context<Self>) {
        use shelldeck_core::models::discovery;

        let conn_id = connection.id;
        let conn_name = connection.display_name().to_string();

        let nginx_cmd = discovery::nginx_discover_command().to_string();
        let mysql_cmd = discovery::mysql_discover_command("");
        let pg_cmd = discovery::pg_discover_command("-U postgres");

        let (stream_tx, stream_rx) = std::sync::mpsc::channel::<(String, String)>();
        let (done_tx, done_rx) = std::sync::mpsc::channel::<bool>();

        let thread_conn_name = conn_name.clone();
        std::thread::Builder::new()
            .name("sites-discover".to_string())
            .spawn(move || {
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("tokio runtime for sites discover");

                rt.block_on(async move {
                    let client = SshClient::new();
                    let session = match tokio::time::timeout(
                        std::time::Duration::from_secs(15),
                        client.connect(&connection),
                    )
                    .await
                    {
                        Ok(Ok(s)) => s,
                        Ok(Err(e)) => {
                            tracing::warn!("Sites discover failed for {}: {}", thread_conn_name, e);
                            let _ = done_tx.send(false);
                            return;
                        }
                        Err(_) => {
                            tracing::warn!(
                                "Sites discover timed out connecting to {}",
                                thread_conn_name
                            );
                            let _ = done_tx.send(false);
                            return;
                        }
                    };

                    let exec_timeout = std::time::Duration::from_secs(30);

                    match tokio::time::timeout(exec_timeout, session.exec(&nginx_cmd)).await {
                        Ok(Ok(result)) => {
                            let output = String::from_utf8_lossy(&result.stdout).to_string();
                            let _ = stream_tx.send(("nginx".to_string(), output));
                        }
                        Ok(Err(e)) => tracing::debug!(
                            "nginx discover exec error on {}: {}",
                            thread_conn_name,
                            e
                        ),
                        Err(_) => {
                            tracing::warn!("nginx discover timed out on {}", thread_conn_name)
                        }
                    }

                    match tokio::time::timeout(exec_timeout, session.exec(&mysql_cmd)).await {
                        Ok(Ok(result)) => {
                            let output = String::from_utf8_lossy(&result.stdout).to_string();
                            let _ = stream_tx.send(("mysql".to_string(), output));
                        }
                        Ok(Err(e)) => tracing::debug!(
                            "mysql discover exec error on {}: {}",
                            thread_conn_name,
                            e
                        ),
                        Err(_) => {
                            tracing::warn!("mysql discover timed out on {}", thread_conn_name)
                        }
                    }

                    match tokio::time::timeout(exec_timeout, session.exec(&pg_cmd)).await {
                        Ok(Ok(result)) => {
                            let output = String::from_utf8_lossy(&result.stdout).to_string();
                            let _ = stream_tx.send(("pg".to_string(), output));
                        }
                        Ok(Err(e)) => {
                            tracing::debug!("pg discover exec error on {}: {}", thread_conn_name, e)
                        }
                        Err(_) => tracing::warn!("pg discover timed out on {}", thread_conn_name),
                    }

                    let _ = done_tx.send(true);
                });
            })
            .expect("spawn sites-discover thread");

        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let mut new_sites: Vec<ManagedSite> = Vec::new();
            let wall_clock_start = std::time::Instant::now();
            let wall_clock_limit = std::time::Duration::from_secs(90);

            loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(50))
                    .await;

                while let Ok((kind, data)) = stream_rx.try_recv() {
                    match kind.as_str() {
                        "nginx" => {
                            let sites = discovery::parse_nginx_configs(&data);
                            for site in sites {
                                new_sites.push(ManagedSite::from_nginx(conn_id, &conn_name, site));
                            }
                        }
                        "mysql" => {
                            let dbs = discovery::parse_mysql_discovery(&data);
                            for db in dbs {
                                new_sites.push(ManagedSite::from_database(conn_id, &conn_name, db));
                            }
                        }
                        "pg" => {
                            let dbs = discovery::parse_pg_discovery(&data);
                            for db in dbs {
                                new_sites.push(ManagedSite::from_database(conn_id, &conn_name, db));
                            }
                        }
                        _ => {}
                    }
                }

                if done_rx.try_recv().is_ok() {
                    let _ = this.update(cx, |ws, cx| {
                        let _ = ws.store.add_managed_sites_bulk(new_sites);
                        ws.sites.update(cx, |view, _| {
                            view.scans_pending = view.scans_pending.saturating_sub(1);
                            view.set_sites(ws.store.managed_sites.clone());
                        });
                        cx.notify();
                    });
                    break;
                }

                // Wall-clock safety: abort if background thread is stuck
                if wall_clock_start.elapsed() > wall_clock_limit {
                    tracing::warn!(
                        "Sites discover poller timed out after 90s for conn {}",
                        conn_name
                    );
                    let _ = this.update(cx, |ws, cx| {
                        let _ = ws.store.add_managed_sites_bulk(new_sites);
                        ws.sites.update(cx, |view, _| {
                            view.scans_pending = view.scans_pending.saturating_sub(1);
                            view.set_sites(ws.store.managed_sites.clone());
                        });
                        cx.notify();
                    });
                    break;
                }
            }
        })
        .detach();
    }

    fn start_sync_operation(&mut self, profile: SyncProfile, cx: &mut Context<Self>) {
        use chrono::Utc;
        use shelldeck_core::models::discovery;
        use shelldeck_core::models::server_sync::*;

        let op_id = Uuid::new_v4();
        let item_progress: Vec<SyncProgress> = profile
            .items
            .iter()
            .map(|item| SyncProgress {
                item_id: item.id,
                status: SyncOperationStatus::Pending,
                bytes_transferred: 0,
                total_bytes: None,
                files_transferred: 0,
                total_files: None,
                current_file: None,
                error_message: None,
            })
            .collect();

        let operation = SyncOperation {
            id: op_id,
            profile_id: profile.id,
            status: SyncOperationStatus::Connecting,
            item_progress,
            log_lines: Vec::new(),
            started_at: Utc::now(),
            finished_at: None,
        };

        self.server_sync.update(cx, |view, cx| {
            view.active_operation = Some(operation);
            view.log_lines
                .push(format!("[sync] Starting sync operation {}", op_id));
            cx.notify();
        });

        // Get connection info
        let source_conn = self
            .connections
            .iter()
            .find(|c| c.id == profile.source_connection_id)
            .cloned();
        let dest_conn = self
            .connections
            .iter()
            .find(|c| c.id == profile.dest_connection_id)
            .cloned();

        let (source_conn, dest_conn) = match (source_conn, dest_conn) {
            (Some(s), Some(d)) => (s, d),
            _ => {
                self.server_sync.update(cx, |view, cx| {
                    view.append_log(
                        "[sync] Error: source or destination connection not found".to_string(),
                    );
                    if let Some(ref mut op) = view.active_operation {
                        op.status = SyncOperationStatus::Failed;
                    }
                    cx.notify();
                });
                return;
            }
        };

        // Build commands for each item
        let mut commands: Vec<(Uuid, String)> = Vec::new();
        for item in &profile.items {
            if !item.enabled {
                continue;
            }
            let cmd = match &item.kind {
                SyncItemKind::Directory {
                    source_path,
                    dest_path,
                    exclude_patterns,
                } => discovery::rsync_command(
                    source_path,
                    &dest_conn.user,
                    &dest_conn.hostname,
                    dest_path,
                    &profile.options,
                    exclude_patterns,
                ),
                SyncItemKind::Database {
                    ref name,
                    engine,
                    ref source_credentials,
                    ref dest_credentials,
                } => match engine {
                    DatabaseEngine::Mysql => discovery::mysql_sync_command(
                        name,
                        source_credentials,
                        &dest_conn.user,
                        &dest_conn.hostname,
                        dest_credentials,
                        profile.options.compress,
                    ),
                    DatabaseEngine::Postgresql => discovery::pg_sync_command(
                        name,
                        source_credentials,
                        &dest_conn.user,
                        &dest_conn.hostname,
                        dest_credentials,
                        profile.options.compress,
                    ),
                },
                SyncItemKind::NginxSite {
                    ref site,
                    ref sync_config,
                    ref sync_root,
                } => {
                    let mut cmds = Vec::new();
                    if *sync_root && !site.root.is_empty() {
                        cmds.push(discovery::rsync_command(
                            &site.root,
                            &dest_conn.user,
                            &dest_conn.hostname,
                            &site.root,
                            &profile.options,
                            &[],
                        ));
                    }
                    if *sync_config && !site.config_path.is_empty() {
                        cmds.push(discovery::rsync_command(
                            &site.config_path,
                            &dest_conn.user,
                            &dest_conn.hostname,
                            &site.config_path,
                            &profile.options,
                            &[],
                        ));
                    }
                    cmds.join(" && ")
                }
            };
            commands.push((item.id, cmd));
        }

        let total_items = commands.len();
        let (shutdown_tx, _shutdown_rx) = tokio::sync::mpsc::channel::<()>(1);
        let (stream_tx, stream_rx) = std::sync::mpsc::channel::<(Uuid, String)>();
        let (done_tx, done_rx) = std::sync::mpsc::channel::<(Uuid, bool)>();

        let thread_handle = std::thread::Builder::new()
            .name(format!("sync-op-{}", op_id))
            .spawn(move || {
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("tokio runtime for sync");

                rt.block_on(async move {
                    let client = SshClient::new();
                    let session = match client.connect(&source_conn).await {
                        Ok(s) => s,
                        Err(e) => {
                            let _ =
                                stream_tx.send((Uuid::nil(), format!("[sync] SSH Error: {}", e)));
                            return;
                        }
                    };

                    for (item_id, cmd) in &commands {
                        let _ = stream_tx.send((*item_id, format!("[sync] Running: {}", cmd)));

                        let (output_tx, mut output_rx) =
                            tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();

                        let fwd_tx = stream_tx.clone();
                        let fwd_item_id = *item_id;
                        let fwd_task = tokio::spawn(async move {
                            while let Some(data) = output_rx.recv().await {
                                let text = String::from_utf8_lossy(&data);
                                for line in text.lines() {
                                    let _ = fwd_tx.send((fwd_item_id, line.to_string()));
                                }
                            }
                        });

                        let (_cancel_tx, cancel_rx) = tokio::sync::mpsc::channel::<()>(1);
                        let result = session.exec_cancellable(cmd, output_tx, cancel_rx).await;
                        let _ = fwd_task.await;

                        match result {
                            Ok(_) => {
                                let _ = done_tx.send((*item_id, true));
                            }
                            Err(e) => {
                                let _ = stream_tx.send((*item_id, format!("[sync] Error: {}", e)));
                                let _ = done_tx.send((*item_id, false));
                            }
                        }
                    }
                });
            })
            .expect("spawn sync thread");

        self.active_scripts.insert(
            op_id,
            ActiveScript {
                shutdown_tx,
                _thread: Some(thread_handle),
            },
        );

        // UI poller
        let sync_handle = self.server_sync.downgrade();
        cx.spawn(async move |_this, cx: &mut AsyncApp| {
            let mut all_done = std::collections::HashSet::new();

            loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(50))
                    .await;

                let mut lines = Vec::new();
                while let Ok((item_id, line)) = stream_rx.try_recv() {
                    lines.push((item_id, line));
                }

                while let Ok((item_id, success)) = done_rx.try_recv() {
                    all_done.insert(item_id);
                    let status = if success {
                        SyncOperationStatus::Completed
                    } else {
                        SyncOperationStatus::Failed
                    };
                    let _ = sync_handle.update(cx, |view, cx| {
                        if let Some(ref mut op) = view.active_operation {
                            if let Some(prog) =
                                op.item_progress.iter_mut().find(|p| p.item_id == item_id)
                            {
                                prog.status = status;
                            }
                        }
                        cx.notify();
                    });
                }

                if !lines.is_empty() {
                    let _ = sync_handle.update(cx, |view, cx| {
                        for (item_id, line) in &lines {
                            view.log_lines.push(line.clone());
                            // Parse rsync progress if applicable
                            if line.contains('%') {
                                if let Some(pct_str) =
                                    line.split_whitespace().find(|w| w.ends_with('%'))
                                {
                                    if let Ok(pct) = pct_str.trim_end_matches('%').parse::<f64>() {
                                        if let Some(ref mut op) = view.active_operation {
                                            if let Some(prog) = op
                                                .item_progress
                                                .iter_mut()
                                                .find(|p| p.item_id == *item_id)
                                            {
                                                prog.status = SyncOperationStatus::Running;
                                                prog.total_bytes = Some(100);
                                                prog.bytes_transferred = pct as u64;
                                            }
                                        }
                                    }
                                }
                            }
                            // Update current file
                            if let Some(ref mut op) = view.active_operation {
                                if let Some(prog) =
                                    op.item_progress.iter_mut().find(|p| p.item_id == *item_id)
                                {
                                    if !line.starts_with("[sync]") {
                                        prog.current_file = Some(line.clone());
                                    }
                                }
                            }
                        }
                        cx.notify();
                    });
                }

                if all_done.len() >= total_items {
                    let _ = sync_handle.update(cx, |view, cx| {
                        if let Some(ref mut op) = view.active_operation {
                            let all_success = op
                                .item_progress
                                .iter()
                                .all(|p| p.status == SyncOperationStatus::Completed);
                            op.status = if all_success {
                                SyncOperationStatus::Completed
                            } else {
                                SyncOperationStatus::Failed
                            };
                            op.finished_at = Some(Utc::now());
                        }
                        view.log_lines
                            .push("[sync] Operation complete.".to_string());
                        view.wizard_active = false;
                        cx.notify();
                    });
                    break;
                }
            }
        })
        .detach();
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

    fn show_port_forward_form(&mut self, cx: &mut Context<Self>) {
        let connections: Vec<(Uuid, String, String)> = self
            .connections
            .iter()
            .map(|c| (c.id, c.display_name().to_string(), c.hostname.clone()))
            .collect();

        let form = cx.new(|form_cx| PortForwardForm::new(connections, form_cx));

        let sub = cx.subscribe(&form, |this, _form, event: &PortForwardFormEvent, cx| {
            match event {
                PortForwardFormEvent::Save(forward) => {
                    tracing::info!("Port forward created: {}", forward.description());
                    // Persist to store
                    if let Err(e) = this.store.add_port_forward(forward.clone()) {
                        tracing::error!("Failed to save port forward: {}", e);
                        this.show_toast(
                            format!("Failed to save port forward: {}", e),
                            ToastLevel::Error,
                            cx,
                        );
                    }
                    // Update the view
                    this.port_forwards.update(cx, |pf, _| {
                        pf.forwards.push(forward.clone());
                    });
                    this.add_activity(
                        format!("Added port forward: {}", forward.description()),
                        ActivityType::Forward,
                        cx,
                    );
                    this.show_toast(
                        format!("Port forward created: {}", forward.description()),
                        ToastLevel::Success,
                        cx,
                    );
                    // Close form
                    this.port_forward_form = None;
                    this._pf_form_sub = None;
                    cx.notify();
                }
                PortForwardFormEvent::Cancel => {
                    this.port_forward_form = None;
                    this._pf_form_sub = None;
                    cx.notify();
                }
            }
        });

        self.port_forward_form = Some(form);
        self._pf_form_sub = Some(sub);
        cx.notify();
    }

    fn show_template_browser(&mut self, cx: &mut Context<Self>) {
        let browser = cx.new(TemplateBrowser::new);

        let sub = cx.subscribe(
            &browser,
            |this, _browser, event: &TemplateBrowserEvent, cx| match event {
                TemplateBrowserEvent::Import(script) => {
                    let name = script.name.clone();
                    this.scripts.update(cx, |editor, _| {
                        editor.scripts.push(script.clone());
                    });
                    let _ = this.store.add_script(script.clone());
                    this.show_toast(
                        format!("Imported template: {}", name),
                        ToastLevel::Success,
                        cx,
                    );
                    this.sync_scripts_to_terminal_toolbar(cx);
                    this.template_browser = None;
                    this._template_browser_sub = None;
                    cx.notify();
                }
                TemplateBrowserEvent::Cancel => {
                    this.template_browser = None;
                    this._template_browser_sub = None;
                    cx.notify();
                }
            },
        );

        self.template_browser = Some(browser);
        self._template_browser_sub = Some(sub);
        cx.notify();
    }

    fn show_variable_prompt(
        &mut self,
        script: shelldeck_core::models::script::Script,
        variables: Vec<shelldeck_core::models::script::ScriptVariable>,
        cx: &mut Context<Self>,
    ) {
        let script_clone = script.clone();
        let prompt = cx.new(|cx| VariablePrompt::new(script_clone, variables, cx));

        let sub = cx.subscribe(
            &prompt,
            |this, _prompt, event: &VariablePromptEvent, cx| match event {
                VariablePromptEvent::Run(script, values) => {
                    this.variable_prompt = None;
                    this._variable_prompt_sub = None;
                    this.run_script_with_values(script.clone(), values.clone(), cx);
                    cx.notify();
                }
                VariablePromptEvent::Cancel => {
                    this.variable_prompt = None;
                    this._variable_prompt_sub = None;
                    cx.notify();
                }
            },
        );

        self.variable_prompt = Some(prompt);
        self._variable_prompt_sub = Some(sub);
        cx.notify();
    }

    fn run_script_with_values(
        &mut self,
        script: shelldeck_core::models::script::Script,
        values: std::collections::HashMap<String, String>,
        cx: &mut Context<Self>,
    ) {
        tracing::info!("Running script with variables: {}", script.name);
        let cmd = build_command(&script, Some(&values));
        let script_name = script.name.clone();
        let script_id = script.id;
        let connection_id = match &script.target {
            ScriptTarget::Remote(cid) => Some(*cid),
            _ => None,
        };

        let record =
            shelldeck_core::models::execution::ExecutionRecord::new(script_id, connection_id);

        let display_cmd = if matches!(script.language, ScriptLanguage::Shell) {
            format!(
                "$ {}",
                shelldeck_core::models::script_runner::substitute_variables(&script.body, &values)
            )
        } else {
            format!("$ [{}] {}", script.language.label(), cmd.ssh_command)
        };

        let values_for_store = values.clone();
        self.scripts.update(cx, |editor, _| {
            editor.running_script_id = Some(script_id);
            editor.execution_output.clear();
            editor.execution_output.push(display_cmd);
            editor.history.push(record);
            // Store the variable values for display in the variables bar
            editor.last_var_values.insert(script_id, values_for_store);
        });

        self.scripts.update(cx, |editor, _| {
            if let Some(s) = editor.scripts.iter_mut().find(|s| s.id == script_id) {
                s.last_run = Some(chrono::Utc::now());
                s.run_count += 1;
            }
        });
        if let Some(s) = self
            .scripts
            .read(cx)
            .scripts
            .iter()
            .find(|s| s.id == script_id)
            .cloned()
        {
            let _ = self.store.update_script(s);
        }

        self.add_activity(
            format!("Running script: {}", script_name),
            ActivityType::Script,
            cx,
        );
        self.show_toast(
            format!("Running script: {}", script_name),
            ToastLevel::Info,
            cx,
        );
        self.update_dashboard_stats(cx);

        match &script.target {
            ScriptTarget::Remote(connection_id) => {
                let connection = self
                    .connections
                    .iter()
                    .find(|c| c.id == *connection_id)
                    .cloned();
                if let Some(conn) = connection {
                    self.run_script_remote(
                        cmd.ssh_command.clone(),
                        script_name,
                        script_id,
                        conn,
                        cx,
                    );
                } else {
                    tracing::error!("Connection {} not found for remote script", connection_id);
                    self.scripts.update(cx, |editor, cx| {
                        editor.running_script_id = None;
                        editor
                            .execution_output
                            .push(format!("Error: Connection {} not found", connection_id));
                        cx.notify();
                    });
                    self.show_toast("Remote connection not found", ToastLevel::Error, cx);
                    self.update_dashboard_stats(cx);
                }
            }
            ScriptTarget::Local | ScriptTarget::AskOnRun => {
                self.run_script_local_cmd(
                    cmd.local_binary.clone(),
                    cmd.local_args.clone(),
                    cmd.env_vars.clone(),
                    script_name,
                    script_id,
                    cx,
                );
            }
        }

        self.sync_scripts_to_terminal_toolbar(cx);
        cx.notify();
    }

    fn show_script_form(&mut self, cx: &mut Context<Self>) {
        let connections: Vec<(Uuid, String, String)> = self
            .connections
            .iter()
            .map(|c| (c.id, c.display_name().to_string(), c.hostname.clone()))
            .collect();

        let form = cx.new(|form_cx| ScriptForm::new(connections, form_cx));

        let sub = cx.subscribe(&form, |this, _form, event: &ScriptFormEvent, cx| {
            match event {
                ScriptFormEvent::Save(script) => {
                    tracing::info!("Script created: {}", script.name);
                    // Persist to store
                    if let Err(e) = this.store.add_script(script.clone()) {
                        tracing::error!("Failed to save script: {}", e);
                        this.show_toast(
                            format!("Failed to save script: {}", e),
                            ToastLevel::Error,
                            cx,
                        );
                    }
                    // Add to script editor view
                    this.scripts.update(cx, |editor, _| {
                        editor.add_script(script.clone());
                    });
                    this.add_activity(
                        format!("Added script: {}", script.name),
                        ActivityType::Script,
                        cx,
                    );
                    this.show_toast(
                        format!("Script created: {}", script.name),
                        ToastLevel::Success,
                        cx,
                    );
                    // Close form
                    this.script_form = None;
                    this._script_form_sub = None;
                    cx.notify();
                }
                ScriptFormEvent::Cancel => {
                    this.script_form = None;
                    this._script_form_sub = None;
                    cx.notify();
                }
            }
        });

        self.script_form = Some(form);
        self._script_form_sub = Some(sub);
        cx.notify();
    }

    fn show_port_forward_form_edit(
        &mut self,
        forward: &shelldeck_core::models::port_forward::PortForward,
        cx: &mut Context<Self>,
    ) {
        let connections: Vec<(Uuid, String, String)> = self
            .connections
            .iter()
            .map(|c| (c.id, c.display_name().to_string(), c.hostname.clone()))
            .collect();

        let forward = forward.clone();
        let form =
            cx.new(|form_cx| PortForwardForm::from_port_forward(&forward, connections, form_cx));

        let sub = cx.subscribe(&form, |this, _form, event: &PortForwardFormEvent, cx| {
            match event {
                PortForwardFormEvent::Save(forward) => {
                    tracing::info!("Port forward updated: {}", forward.description());
                    // Update in store
                    match this.store.update_port_forward(forward.clone()) {
                        Ok(true) => {}
                        Ok(false) => {
                            // Not found in store, add it
                            if let Err(e) = this.store.add_port_forward(forward.clone()) {
                                tracing::error!("Failed to save port forward: {}", e);
                                this.show_toast(
                                    format!("Failed to save port forward: {}", e),
                                    ToastLevel::Error,
                                    cx,
                                );
                            }
                        }
                        Err(e) => {
                            tracing::error!("Failed to update port forward: {}", e);
                            this.show_toast(
                                format!("Failed to update port forward: {}", e),
                                ToastLevel::Error,
                                cx,
                            );
                        }
                    }
                    // Update the view
                    this.port_forwards.update(cx, |pf, _| {
                        if let Some(existing) = pf.forwards.iter_mut().find(|f| f.id == forward.id)
                        {
                            *existing = forward.clone();
                        }
                    });
                    this.add_activity(
                        format!("Updated port forward: {}", forward.description()),
                        ActivityType::Forward,
                        cx,
                    );
                    this.show_toast(
                        format!("Port forward updated: {}", forward.description()),
                        ToastLevel::Success,
                        cx,
                    );
                    // Close form
                    this.port_forward_form = None;
                    this._pf_form_sub = None;
                    cx.notify();
                }
                PortForwardFormEvent::Cancel => {
                    this.port_forward_form = None;
                    this._pf_form_sub = None;
                    cx.notify();
                }
            }
        });

        self.port_forward_form = Some(form);
        self._pf_form_sub = Some(sub);
        cx.notify();
    }

    fn show_script_form_edit(
        &mut self,
        script: &shelldeck_core::models::script::Script,
        cx: &mut Context<Self>,
    ) {
        let connections: Vec<(Uuid, String, String)> = self
            .connections
            .iter()
            .map(|c| (c.id, c.display_name().to_string(), c.hostname.clone()))
            .collect();

        let script = script.clone();
        let form = cx.new(|form_cx| ScriptForm::from_script(&script, connections, form_cx));

        let sub = cx.subscribe(&form, |this, _form, event: &ScriptFormEvent, cx| {
            match event {
                ScriptFormEvent::Save(script) => {
                    tracing::info!("Script updated: {}", script.name);
                    // Update in store
                    match this.store.update_script(script.clone()) {
                        Ok(true) => {}
                        Ok(false) => {
                            // Not found in store, add it
                            if let Err(e) = this.store.add_script(script.clone()) {
                                tracing::error!("Failed to save script: {}", e);
                                this.show_toast(
                                    format!("Failed to save script: {}", e),
                                    ToastLevel::Error,
                                    cx,
                                );
                            }
                        }
                        Err(e) => {
                            tracing::error!("Failed to update script: {}", e);
                            this.show_toast(
                                format!("Failed to update script: {}", e),
                                ToastLevel::Error,
                                cx,
                            );
                        }
                    }
                    // Update in script editor view
                    this.scripts.update(cx, |editor, _| {
                        if let Some(existing) =
                            editor.scripts.iter_mut().find(|s| s.id == script.id)
                        {
                            *existing = script.clone();
                        }
                    });
                    this.add_activity(
                        format!("Updated script: {}", script.name),
                        ActivityType::Script,
                        cx,
                    );
                    this.show_toast(
                        format!("Script updated: {}", script.name),
                        ToastLevel::Success,
                        cx,
                    );
                    // Close form
                    this.script_form = None;
                    this._script_form_sub = None;
                    cx.notify();
                }
                ScriptFormEvent::Cancel => {
                    this.script_form = None;
                    this._script_form_sub = None;
                    cx.notify();
                }
            }
        });

        self.script_form = Some(form);
        self._script_form_sub = Some(sub);
        cx.notify();
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
            SidebarSection::Scripts => ActiveView::Scripts,
            SidebarSection::PortForwards => ActiveView::PortForwards,
            SidebarSection::ServerSync => ActiveView::ServerSync,
            SidebarSection::Sites => ActiveView::Sites,
            SidebarSection::Settings => ActiveView::Settings,
        };
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
    }

    pub fn open_new_terminal(&mut self, cx: &mut Context<Self>) {
        self.terminal.update(cx, |terminal, cx| {
            terminal.spawn_local_terminal(cx);
        });
        self.active_view = ActiveView::Terminal;
        self.update_dashboard_stats(cx);
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

    /// Update a connection's status and refresh sidebar.
    fn set_connection_status(
        &mut self,
        conn_id: Uuid,
        status: ConnectionStatus,
        cx: &mut Context<Self>,
    ) {
        if let Some(conn) = self.connections.iter_mut().find(|c| c.id == conn_id) {
            conn.status = status;
        }
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
    }

    /// Initiate an SSH connection to `connection`.
    fn connect_ssh(&mut self, connection: Connection, cx: &mut Context<Self>) {
        let title = connection.display_name().to_string();
        let conn_id = connection.id;

        let (rows, cols) = self.terminal.read(cx).grid_size();

        let (mut session, data_tx, input_rx) =
            TerminalSession::spawn_ssh(title.clone(), rows, cols);

        let (resize_tx, resize_rx) = tokio::sync::mpsc::unbounded_channel::<(u16, u16)>();
        session.set_resize_fn(Box::new(move |rows, cols| {
            let _ = resize_tx.send((rows, cols));
        }));

        self.terminal.update(cx, |terminal, cx| {
            terminal.add_session_with_connection(session, Some(conn_id));
            terminal.ensure_refresh_running(cx);
            cx.notify();
        });

        // Mark as connecting
        self.set_connection_status(conn_id, ConnectionStatus::Connecting, cx);

        // Channel for SSH status feedback
        let (status_tx, status_rx) = std::sync::mpsc::channel::<Result<(), String>>();

        let conn = connection;
        std::thread::Builder::new()
            .name(format!("ssh-{}", title))
            .spawn(move || {
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("Failed to create tokio runtime");

                rt.block_on(async move {
                    let client = SshClient::new();
                    let ssh_session = match client.connect(&conn).await {
                        Ok(s) => s,
                        Err(e) => {
                            let msg = format!("SSH connection failed for {}: {}", conn.display_name(), e);
                            tracing::error!("{}", msg);
                            let _ = status_tx.send(Err(msg));
                            return;
                        }
                    };
                    tracing::info!("SSH connected to {}", conn.display_name());

                    let channel = match ssh_session.open_shell(rows as u32, cols as u32).await {
                        Ok(ch) => ch,
                        Err(e) => {
                            let msg = format!("Failed to open SSH shell for {}: {}", conn.display_name(), e);
                            tracing::error!("{}", msg);
                            let _ = status_tx.send(Err(msg));
                            return;
                        }
                    };
                    tracing::info!("SSH shell opened for {}", conn.display_name());

                    // Notify success
                    let _ = status_tx.send(Ok(()));

                    let (mut channel_reader, mut channel_writer) = channel.split();

                    let mut input_rx = input_rx;
                    let write_task = tokio::spawn(async move {
                        use tokio::io::AsyncWriteExt;
                        while let Some(data) = input_rx.recv().await {
                            if channel_writer.write_all(&data).await.is_err() {
                                break;
                            }
                        }
                        tracing::info!("SSH write loop ended");
                    });

                    let mut resize_rx = resize_rx;
                    let read_task = tokio::spawn(async move {
                        use shelldeck_ssh::session::SshChannelData;
                        loop {
                            tokio::select! {
                                biased;
                                Some((r, c)) = resize_rx.recv() => {
                                    if let Err(e) = channel_reader.resize(r as u32, c as u32).await {
                                        tracing::warn!("SSH resize failed: {}", e);
                                    }
                                }
                                msg = channel_reader.read() => {
                                    match msg {
                                        Some(SshChannelData::Data(data)) => {
                                            if data_tx.send(data).is_err() {
                                                break;
                                            }
                                        }
                                        Some(SshChannelData::Eof) | None => break,
                                    }
                                }
                            }
                        }
                        tracing::info!("SSH read loop ended");
                    });

                    tokio::select! {
                        _ = read_task => {}
                        _ = write_task => {}
                    }

                    tracing::info!("SSH session ended for {}", conn.display_name());
                });
            })
            .expect("Failed to spawn SSH thread");

        // Spawn a GPUI task to listen for SSH status feedback
        let weak = cx.entity().downgrade();
        cx.spawn(async move |_this, cx: &mut AsyncApp| {
            // Poll in a non-blocking way on the background executor
            let result = cx
                .background_executor()
                .spawn(async move { status_rx.recv().ok() })
                .await;

            if let Some(status) = result {
                let _ = weak.update(cx, |ws, cx| {
                    match status {
                        Ok(()) => {
                            ws.set_connection_status(conn_id, ConnectionStatus::Connected, cx);
                            ws.show_toast(
                                format!("Connected to {}", title),
                                ToastLevel::Success,
                                cx,
                            );
                        }
                        Err(msg) => {
                            ws.set_connection_status(
                                conn_id,
                                ConnectionStatus::Error(msg.clone()),
                                cx,
                            );
                            ws.show_toast(msg, ToastLevel::Error, cx);
                        }
                    }
                    cx.notify();
                });
            }
        })
        .detach();
    }

    /// Initiate an SSH connection for a split pane on the current tab.
    fn connect_ssh_split(
        &mut self,
        connection: Connection,
        direction: SplitDirection,
        cx: &mut Context<Self>,
    ) {
        let title = format!("{} (split)", connection.display_name());
        let conn_id = connection.id;

        let (rows, cols) = self.terminal.read(cx).grid_size();

        let (mut session, data_tx, input_rx) =
            TerminalSession::spawn_ssh(title.clone(), rows, cols);

        let (resize_tx, resize_rx) = tokio::sync::mpsc::unbounded_channel::<(u16, u16)>();
        session.set_resize_fn(Box::new(move |rows, cols| {
            let _ = resize_tx.send((rows, cols));
        }));

        // Inject the session into the terminal view's split
        let terminal = self.terminal.clone();
        terminal.update(cx, |terminal, cx| {
            terminal.set_split_session(session, direction, cx);
        });

        let conn = connection;
        std::thread::Builder::new()
            .name(format!("ssh-split-{}", title))
            .spawn(move || {
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("Failed to create tokio runtime");

                rt.block_on(async move {
                    let client = SshClient::new();
                    let ssh_session = match client.connect(&conn).await {
                        Ok(s) => s,
                        Err(e) => {
                            tracing::error!("SSH split connection failed for {}: {}", conn.display_name(), e);
                            return;
                        }
                    };
                    tracing::info!("SSH split connected to {}", conn.display_name());

                    let channel = match ssh_session.open_shell(rows as u32, cols as u32).await {
                        Ok(ch) => ch,
                        Err(e) => {
                            tracing::error!("Failed to open SSH split shell for {}: {}", conn.display_name(), e);
                            return;
                        }
                    };
                    tracing::info!("SSH split shell opened for {}", conn.display_name());

                    let (mut channel_reader, mut channel_writer) = channel.split();

                    let mut input_rx = input_rx;
                    let write_task = tokio::spawn(async move {
                        use tokio::io::AsyncWriteExt;
                        while let Some(data) = input_rx.recv().await {
                            if channel_writer.write_all(&data).await.is_err() {
                                break;
                            }
                        }
                        tracing::info!("SSH split write loop ended");
                    });

                    let mut resize_rx = resize_rx;
                    let read_task = tokio::spawn(async move {
                        use shelldeck_ssh::session::SshChannelData;
                        loop {
                            tokio::select! {
                                biased;
                                Some((r, c)) = resize_rx.recv() => {
                                    if let Err(e) = channel_reader.resize(r as u32, c as u32).await {
                                        tracing::warn!("SSH split resize failed: {}", e);
                                    }
                                }
                                msg = channel_reader.read() => {
                                    match msg {
                                        Some(SshChannelData::Data(data)) => {
                                            if data_tx.send(data).is_err() {
                                                break;
                                            }
                                        }
                                        Some(SshChannelData::Eof) | None => break,
                                    }
                                }
                            }
                        }
                        tracing::info!("SSH split read loop ended");
                    });

                    tokio::select! {
                        _ = read_task => {}
                        _ = write_task => {}
                    }

                    tracing::info!("SSH split session ended for {}", conn.display_name());
                });
            })
            .expect("Failed to spawn SSH split thread");

        self.show_toast(
            format!("Connecting split to {}", conn_id),
            ToastLevel::Info,
            cx,
        );
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
            .child(div().text_xs().text_color(title_dim).child("v0.1.3"));

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
            });

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
