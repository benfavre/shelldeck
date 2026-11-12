use super::*;

/// State of the session file autosave (`Workspace::ensure_workspace_autosave`).
/// One workspace per process.
mod workspace_autosave {
    use parking_lot::Mutex;
    use std::sync::atomic::AtomicBool;
    use std::time::Duration;

    /// How often the open tabs are compared with the file.
    pub(super) const PERIOD: Duration = Duration::from_secs(10);
    pub(super) static STARTED: AtomicBool = AtomicBool::new(false);
    /// Set at shutdown: the file then holds the tabs as they were.
    pub(super) static STOPPED: AtomicBool = AtomicBool::new(false);
    /// What the file holds, as far as this process knows.
    pub(super) static LAST_SAVED: Mutex<Option<String>> = Mutex::new(None);

    /// Whether `current` has to be written: it differs from what was
    /// written last, or nothing was written yet by this process.
    pub(super) fn changed(last: Option<&str>, current: &str) -> bool {
        last != Some(current)
    }
}

impl Workspace {
    /// Réapplique tout le contexte mutable qui appartient à la fenêtre, et
    /// non au workspace retenu. L'entité PTY reste identique; seuls ses
    /// réglages, intégrations et aperçus sont remis à jour.
    pub(super) fn hydrate_active_terminal_runtime(&mut self, cx: &mut Context<Self>) {
        let theme_name = self
            .terminal_theme_preview
            .as_deref()
            .unwrap_or(&self.app_config.terminal.theme);
        let config = workspaces::WorkspaceTerminalConfig {
            theme: TerminalTheme::by_name(theme_name),
            font_size: self.app_config.terminal.font_size,
            font_family: self.app_config.terminal.font_family.clone(),
            default_shell: self.app_config.terminal.default_shell.clone(),
            cursor_style: self.app_config.terminal.cursor_style.clone(),
            cursor_blink: self.app_config.terminal.cursor_blink,
            scrollback_lines: self.app_config.terminal.scrollback_lines,
            // `total_width` inclut le rail et respecte l'état replié courant.
            sidebar_width: self.sidebar.read(cx).total_width(),
            menu_bar_visible: self.app_config.general.menu_bar_visible,
            persistent_sessions: self.app_config.general.session_daemon,
        };
        workspaces::apply_terminal_config(&self.terminal, &config, cx);
        self.sync_scripts_to_terminal_toolbar(cx);
        self.sync_ai_affordances(cx);
    }

    pub(super) fn update_dashboard_stats(&mut self, cx: &mut Context<Self>) {
        let terminal_count = self.terminal.read(cx).tab_count();
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

        self.dashboard.update(cx, |d, _| {
            d.active_terminals = terminal_count;
        });
        self.status_bar.update(cx, |bar, _| {
            bar.set_counts(active_connections, running_scripts);
        });
        // Also push the fresh state to the tray. The tray publisher
        // dedups against its last state, so this is cheap even when
        // update_dashboard_stats runs on every unrelated tick.
        self.publish_tray_state(cx);
    }

    /// Persist the current set of open terminal tabs so they can be restored on
    /// the next launch (when `auto_connect_on_startup` is enabled). Saved
    /// unconditionally and best-effort: failures are logged, never fatal.
    pub(super) fn save_workspace_state(&self, cx: &Context<Self>) {
        let state = self.capture_workspace_state(cx);
        *workspace_autosave::LAST_SAVED.lock() = serde_json::to_string(&state).ok();
        if let Err(e) = state.save() {
            tracing::warn!("Failed to save workspace state: {}", e);
        }
    }

    /// With the session daemon, the shells outlive a crash of the
    /// application but the session file used to be written at shutdown
    /// and at sign-out only: after a crash, tabs opened since the last
    /// clean quit came back without their splits, and the list of panes
    /// still to end on SSH servers was lost. While the setting is on, the
    /// file is written again a few seconds after it stopped matching
    /// what is open. Started once, by the first change of the tab set.
    pub(super) fn ensure_workspace_autosave(&self, cx: &mut Context<Self>) {
        use std::sync::atomic::Ordering;
        if !self.app_config.general.session_daemon
            || workspace_autosave::STARTED.swap(true, Ordering::SeqCst)
        {
            return;
        }
        cx.spawn(async move |this, cx: &mut AsyncApp| loop {
            cx.background_executor()
                .timer(workspace_autosave::PERIOD)
                .await;
            if this
                .update(cx, |workspace, cx| workspace.autosave_workspace_state(cx))
                .is_err()
            {
                break;
            }
        })
        .detach();
    }

    fn autosave_workspace_state(&self, cx: &Context<Self>) {
        use std::sync::atomic::Ordering;
        if !self.app_config.general.session_daemon
            || workspace_autosave::STOPPED.load(Ordering::SeqCst)
        {
            return;
        }
        let state = self.capture_workspace_state(cx);
        let Ok(text) = serde_json::to_string(&state) else {
            return;
        };
        let mut last = workspace_autosave::LAST_SAVED.lock();
        if !workspace_autosave::changed(last.as_deref(), &text) {
            return;
        }
        match state.save() {
            Ok(()) => *last = Some(text),
            Err(e) => tracing::warn!("Failed to autosave workspace state: {}", e),
        }
    }

    fn capture_workspace_state(
        &self,
        cx: &Context<Self>,
    ) -> shelldeck_core::config::WorkspaceState {
        use shelldeck_core::config::workspace_state::{TabState, TabType};
        use shelldeck_core::config::WorkspaceState;

        let terminal = self.terminal.read(cx);
        let sessions = terminal.session_states();
        let active_tab = terminal.active_tab_index();
        // Tabs made of session daemon panes also save their native layout
        // by pane id: those panes outlive the app, and the next start
        // rebuilds the tabs around the ones still running.
        let mut daemon_layouts = if terminal.restores_daemon_panes() {
            terminal.daemon_tab_layouts()
        } else {
            Vec::new()
        };
        daemon_layouts.resize(sessions.len(), None);
        // SSH tabs made of panes of their host's daemon: their splits, by
        // pane id on that host, for the next connection.
        let mut remote_layouts = if self.app_config.general.session_daemon {
            terminal.remote_tab_layouts()
        } else {
            Vec::new()
        };
        remote_layouts.resize(sessions.len(), None);

        let tabs: Vec<TabState> = sessions
            .into_iter()
            .zip(daemon_layouts.into_iter().zip(remote_layouts))
            .enumerate()
            .map(|(i, ((title, connection_id), (daemon, remote)))| {
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
                    daemon: daemon.filter(|_| connection_id.is_none()),
                    remote: remote.filter(|_| connection_id.is_some()),
                }
            })
            .collect();

        WorkspaceState {
            tabs,
            active_tab,
            sidebar_visible: self.sidebar_visible,
            // Remote daemon panes still to be ended: kept until their
            // server is reached, in this run or a later one.
            #[cfg(unix)]
            daemon_cleanup: super::remote_daemon::cleanup_to_save(),
            #[cfg(not(unix))]
            daemon_cleanup: Vec::new(),
        }
    }

    /// Gracefully shut down: close all terminal sessions, stop tunnels, stop background tasks.
    pub fn shutdown(&mut self, cx: &mut Context<Self>) {
        tracing::info!("Inklura Desktop shutting down gracefully...");
        // Persist open tabs before tearing sessions down so there is something
        // to restore next launch. Best-effort; never blocks shutdown.
        self.save_workspace_state(cx);
        // Nothing after this point may write the file again: the sessions
        // are about to be closed, and an autosave would record no tab.
        workspace_autosave::STOPPED.store(true, std::sync::atomic::Ordering::SeqCst);
        self.agent_console.update(cx, |view, cx| {
            if let Err(error) = view.flush_library(cx) {
                tracing::warn!(%error,"Unable to save agent library on shutdown");
            }
        });
        self.stop_authenticated_runtime(cx);
        // Stop git polling
        self._git_poll_task = None;
        // Clear forms if open
        self.connection_form = None;
        self._form_sub = None;
        self.login_form = None;
        self._login_form_sub = None;
        self.post_login_splash = None;
        self.onboarding = None;
        self._onboarding_sub = None;
        self.port_forward_form = None;
        self._pf_form_sub = None;
        self.script_form = None;
        self._script_form_sub = None;
        tracing::info!("Shutdown cleanup complete");
    }

    /// Stop runtime resources that must never survive an account boundary.
    /// Unlike `shutdown`, this keeps the Workspace itself alive for login.
    pub(super) fn stop_authenticated_runtime(&mut self, cx: &mut Context<Self>) {
        self.ai_dock_assistant
            .update(cx, |view, cx| view.set_shared_agent_console(None, cx));
        self.personal_share.update(cx, |view, cx| view.clear(cx));
        self.workspace_hub
            .update(cx, |hub, cx| hub.cancel_remote_folder_picker(cx));
        // Retire setup attempts before stopping completed tunnels.
        self.tunnel_starts.cancel_all();
        self.port_forwards.update(cx, |pf, cx| {
            for forward in &mut pf.forwards {
                forward.status = shelldeck_core::models::port_forward::ForwardStatus::Inactive;
            }
            cx.notify();
        });
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
        self.stop_all_agent_runs();
        self.agent_console
            .update(cx, |view, cx| view.set_browser_account("", None, cx));
        // Close all terminal sessions (drops channels, threads exit)
        self.terminal.update(cx, |terminal, _| {
            terminal.close_all_sessions();
        });
        self.ai_script_runs.clear();
        self.ai_terminal_runs.clear();
        self.ai_diagnostic_sequences.clear();
        self.ai_action_confirmation = None;
    }

    pub fn set_active_view(&mut self, view: ActiveView, cx: &mut Context<Self>) {
        if view.is_retired() {
            return;
        }
        self.close_titlebar_menus();
        self.active_view = view;
        self.on_active_view_changed(cx);
        cx.notify();
    }

    /// Open the Monique console (palette / action). Switches to Dev mode for
    /// super-admins so the console is actually on screen.
    pub fn open_monique_console(&mut self, cx: &mut Context<Self>) {
        if !self.enter_dev_mode(cx) {
            return;
        }
        if !self.has_monique() {
            self.show_toast(
                t!("toast.monique.not_configured").to_string(),
                ToastLevel::Warning,
                cx,
            );
            return;
        }
        self.active_view = ActiveView::MoniqueConsole;
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
        if matches!(link, DeepLink::Agents) {
            self.request_agents_presentation(cx);
            return;
        }
        if !matches!(link, DeepLink::Assistant) {
            self.show_main_presentation(cx);
        }
        if !matches!(link, DeepLink::Assistant) && !self.signed_in() {
            self.show_login_form(cx);
            return;
        }
        match link {
            // The application-level CompanionRuntime owns this target. It
            // authenticates before forwarding, so keep this arm inert if
            // another caller accidentally routes it into Workspace.
            DeepLink::Assistant | DeepLink::Agents => {}
            DeepLink::ExtensionSettings => {
                self.settings.update(cx, |settings, cx| {
                    settings.set_active_tab(crate::settings::SettingsTab::General, cx);
                });
                self.refresh_inklura_extension_panel(cx);
                self.open_settings(cx);
            }
            DeepLink::OpenConnection(id) => {
                if !self.enter_dev_mode(cx) {
                    return;
                }
                if !self.connections.iter().any(|c| c.id == id) {
                    self.show_toast(
                        t!("toast.deeplink.connection_not_found").to_string(),
                        ToastLevel::Warning,
                        cx,
                    );
                    return;
                }
                // The Connections panel is retired: the link lands on Agents,
                // whose target picker lists the same saved hosts.
                self.switch_to_section(SidebarSection::Agents);
                self.sidebar.update(cx, |s, cx| {
                    s.set_active_section(SidebarSection::Agents);
                    s.focus_connection(id);
                    cx.notify();
                });
                self.on_active_view_changed(cx);
                cx.notify();
            }
            DeepLink::SshConnect(id) => {
                if !self.enter_dev_mode(cx) {
                    return;
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
                    self.set_active_view(ActiveView::Terminal, cx);
                    cx.notify();
                } else {
                    self.show_toast(
                        t!("toast.deeplink.connection_not_found").to_string(),
                        ToastLevel::Warning,
                        cx,
                    );
                }
            }
            DeepLink::TunnelStart(_) => {
                self.show_toast(
                    t!(
                        "navigation.retired_tool",
                        app_name = shelldeck_core::APP_NAME
                    )
                    .to_string(),
                    ToastLevel::Info,
                    cx,
                );
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
                        v.set_section(crate::support_view::SupportSection::Requests, cx);
                        cx.notify();
                    });
                }
                self.select_issue(id, cx);
                cx.notify();
            }
            DeepLink::OpenTicket(id) => {
                if !self.can_access_mode(AppMode::Support) {
                    self.show_toast(
                        t!("toast.deeplink.support_only").to_string(),
                        ToastLevel::Warning,
                        cx,
                    );
                    return;
                }
                self.set_mode(AppMode::Support, cx);
                self.support.update(cx, |v, cx| {
                    v.set_section(crate::support_view::SupportSection::Tickets, cx);
                    cx.notify();
                });
                self.select_support_ticket(id, cx);
                cx.notify();
            }
            DeepLink::PlatformSession(session_id) => {
                self.pending_fleet_session_focus = Some(session_id);
                self.open_fleet(cx);
                self.focus_pending_fleet_session(cx);
            }
        }
    }

    /// Key handling for the User-mode "Ask Monique" composer.
    pub(super) fn handle_monique_ask_key(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) {
        let key = event.keystroke.key.as_str();
        match key {
            "enter" => {
                if event.keystroke.modifiers.shift {
                    self.monique_ask_input.push('\n');
                    cx.notify();
                } else {
                    self.submit_monique_ask(cx);
                }
            }
            "backspace" => {
                self.monique_ask_input.pop();
                cx.notify();
            }
            _ => {
                if let Some(ref kc) = event.keystroke.key_char {
                    if !event.keystroke.modifiers.control && !event.keystroke.modifiers.alt {
                        self.monique_ask_input.push_str(kc);
                        cx.notify();
                    }
                } else if key.len() == 1
                    && !event.keystroke.modifiers.control
                    && !event.keystroke.modifiers.alt
                {
                    self.monique_ask_input.push_str(key);
                    cx.notify();
                }
            }
        }
    }

    pub(super) fn submit_monique_ask(&mut self, cx: &mut Context<Self>) {
        let text = self.monique_ask_input.trim().to_string();
        if text.is_empty() {
            return;
        }
        self.monique_ask_input.clear();
        self.prepare_monique_dispatch(text, cx);
        cx.notify();
    }

    pub(super) fn populate_script_editor_connections(&self, cx: &mut Context<Self>) {
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
        let opening = !self.command_palette.read(cx).visible;
        if opening {
            self.close_titlebar_menus();
        }
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

    pub(super) fn contextual_sidebar_visible(&self, cx: &App) -> bool {
        if self.active_view == ActiveView::Agents && !self.settings_open {
            self.agent_console.read(cx).navigator_visible()
        } else {
            self.sidebar_visible
        }
    }

    pub fn toggle_sidebar(&mut self, cx: &mut Context<Self>) {
        if !self.enter_dev_mode(cx) {
            return;
        }
        if self.active_view == ActiveView::Agents {
            self.agent_console
                .update(cx, |view, cx| view.toggle_navigator(cx));
            cx.notify();
            return;
        }
        self.sidebar_visible = !self.sidebar_visible;
        // Toggles the *panel*; the activity rail stays put, so the effective
        // width comes from the sidebar itself rather than being recomputed
        // here (it alone knows whether the rail is showing).
        let effective_width = self.sidebar.update(cx, |sidebar, _cx| {
            sidebar.toggle_collapsed();
            sidebar.total_width()
        });
        self.terminal.update(cx, |terminal, _cx| {
            terminal.set_sidebar_width(effective_width);
        });
        cx.notify();
    }

    pub fn switch_to_section(&mut self, section: SidebarSection) {
        if section.is_retired() {
            return;
        }
        self.close_titlebar_menus();
        self.active_view = match section {
            SidebarSection::Connections => ActiveView::Dashboard,
            SidebarSection::Workspaces => ActiveView::Workspaces,
            SidebarSection::Terminals => ActiveView::Terminal,
            SidebarSection::Agents => ActiveView::Agents,
            SidebarSection::Scripts => ActiveView::Scripts,
            SidebarSection::PortForwards => ActiveView::PortForwards,
            SidebarSection::ServerSync => ActiveView::ServerSync,
            SidebarSection::Sites => ActiveView::Sites,
            SidebarSection::Recent => ActiveView::Recent,
            SidebarSection::FileEditor => ActiveView::FileEditor,
            SidebarSection::MoniqueConsole => ActiveView::MoniqueConsole,
            SidebarSection::Fleet => ActiveView::Fleet,
            SidebarSection::BextCloud => ActiveView::BextCloud,
            SidebarSection::Support | SidebarSection::Requests => ActiveView::Support,
            SidebarSection::Settings => ActiveView::Settings,
        };
    }

    /// The Support console is on screen: Support mode, or its view inside
    /// Dev. Gates its polls, its AI context and where new requests open.
    pub(super) fn support_surface_visible(&self) -> bool {
        super::polling::support_console_on_screen(
            self.effective_mode(),
            self.active_view,
            self.can_access_mode(AppMode::Support),
        )
    }

    pub(super) fn activate_dev_section(&mut self, section: SidebarSection, cx: &mut Context<Self>) {
        if section.is_retired() {
            return;
        }
        if support_rail_target(section).is_some() && !self.can_access_mode(AppMode::Support) {
            return;
        }
        if !self.enter_dev_mode(cx) {
            return;
        }
        self.switch_to_section(section);
        self.sidebar.update(cx, |sidebar, cx| {
            sidebar.set_active_section(section);
            cx.notify();
        });
        self.show_support_rail_section(section, cx);
        self.on_active_view_changed(cx);
        cx.notify();
    }

    /// The Support or Requests rail icon was chosen in Dev: put the console
    /// on the matching tab and load its data. No-op for other sections.
    pub(super) fn show_support_rail_section(
        &mut self,
        section: SidebarSection,
        cx: &mut Context<Self>,
    ) {
        let Some(target) = support_rail_target(section) else {
            return;
        };
        self.support.update(cx, |view, cx| {
            view.set_section(target, cx);
            cx.notify();
        });
        self.refresh_issues(cx);
        if target != crate::support_view::SupportSection::Requests {
            self.refresh_support(cx);
        }
    }

    /// Called when the active Dev view changes — (re)start the Monique poll if the
    /// console just became visible.
    pub(super) fn on_active_view_changed(&mut self, cx: &mut Context<Self>) {
        self.close_titlebar_menus();
        // Native navigation from the dedicated Agents surface must reveal its
        // destination in the full Dev shell, including when the saved mode was
        // User or Support before the transient Agents launch.
        if self.agents_presentation_active() && self.active_view != ActiveView::Agents {
            self.presentation = WorkspacePresentation::Main;
            self.agents_focus_pending = false;
            self.set_mode(AppMode::Dev, cx);
        }
        if self.active_view != ActiveView::Workspaces
            || self.settings_open
            || !self.can_access_mode(AppMode::Dev)
            || self.effective_mode() != AppMode::Dev
        {
            self.workspace_hub
                .update(cx, |hub, cx| hub.cancel_remote_folder_picker(cx));
        }
        let workspace_agent = (self.active_view == ActiveView::Workspaces)
            .then(|| self.workspace_hub.read(cx).active_agent_session_id())
            .flatten();
        self.sync_agent_companion_host(cx);
        let workflows_authorized = self.can_access_mode(AppMode::Dev);
        let companion_owns_agents = self.companion_owns_agents(cx);
        let agent_visible = (companion_owns_agents
            && self.ai_dock_assistant.read(cx).shared_agent_chat_visible())
            || (!companion_owns_agents
                && super::presentation::agent_surface_visible(
                    workflows_authorized,
                    self.effective_mode(),
                    self.settings_open,
                    self.active_view == ActiveView::Agents || workspace_agent.is_some(),
                ));
        self.agent_console.update(cx, |view, cx| {
            view.set_workflows_authorized(workflows_authorized, cx);
            if let Some(session_id) = workspace_agent {
                view.select_session(session_id, cx);
            }
            view.set_surface_visible(agent_visible, cx);
        });
        self.sync_monique_poll(cx);
        self.sync_fleet_view_poll(cx);
        self.sync_bext_poll(cx);
        // The Support console can be a Dev view: its polls follow it.
        self.sync_support_poll(cx);
        self.sync_monique_activity_poll(cx);
        self.sync_issues_poll(cx);
        self.refresh_command_palette(cx);
    }

    pub(super) fn sync_terminal_tab_count(&self, cx: &mut Context<Self>) {
        self.ensure_workspace_autosave(cx);
        let count = self.terminal.read(cx).tabs.len();
        self.sidebar.update(cx, |sidebar, _| {
            sidebar.set_terminal_tab_count(count);
        });
    }

    pub fn next_tab(&mut self, cx: &mut Context<Self>) {
        if !self.enter_dev_mode(cx) {
            return;
        }
        self.terminal.update(cx, |t, cx| {
            t.next_tab();
            cx.notify();
        });
    }

    pub fn prev_tab(&mut self, cx: &mut Context<Self>) {
        if !self.enter_dev_mode(cx) {
            return;
        }
        self.terminal.update(cx, |t, cx| {
            t.prev_tab();
            cx.notify();
        });
    }

    pub fn close_active_tab(&mut self, cx: &mut Context<Self>) {
        if !self.enter_dev_mode(cx) {
            return;
        }
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

    /// Restore the previously-saved session on startup.
    ///
    /// With `general.session_daemon` on, the panes an earlier run left
    /// running come back first, whatever `auto_connect_on_startup` says:
    /// they are live processes, not saved state. Listing them is a round
    /// trip to the daemon, so it runs on the background executor and the
    /// restore continues when the answer is in; no daemon is ever started
    /// for it. With the setting off the restore is synchronous, as before.
    pub fn restore_session(&mut self, cx: &mut Context<Self>) {
        // An install without the daemon files gets them now (in the
        // background; the terminals restored below do not wait for it).
        self.ensure_session_daemon_files(false, cx);
        let wants_panes = self.app_config.general.session_daemon
            && self.terminal.update(cx, |terminal, _| {
                terminal.set_persistent_sessions(true);
                terminal.restores_daemon_panes()
            });
        if !wants_panes {
            self.finish_restore_session(
                crate::terminal_view::RestorablePanes::Live(Vec::new()),
                cx,
            );
            return;
        }
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let panes = cx
                .background_executor()
                .spawn(async { crate::terminal_view::restorable_daemon_panes() })
                .await;
            let _ = this.update(cx, |workspace, cx| {
                workspace.finish_restore_session(panes, cx);
            });
        })
        .detach();
    }

    /// Second half of [`restore_session`](Self::restore_session).
    /// `live_panes`: session daemon panes still running and not shown yet.
    ///
    /// Daemon tabs are rebuilt from the layout saved by pane id (tabs,
    /// split tree, ratios, focused pane); a live pane with no saved slot
    /// becomes a tab of its own and a saved slot whose pane is gone is
    /// dropped. When `auto_connect_on_startup` is enabled, the other saved
    /// tabs are restored in their saved order: local tabs reopen a
    /// default-shell terminal, SSH tabs reconnect via the existing
    /// `connect_ssh` path if the connection still exists. A saved daemon
    /// tab none of whose panes is alive counts as such a local tab, so
    /// with no live pane at all (restart of the machine, setting turned
    /// off) the local tabs are reopened exactly as without the daemon;
    /// when some panes are alive, they are the local tabs and no fresh
    /// shell is added. Failures are logged, never fatal.
    ///
    /// When the daemon was left by another version of the app
    /// (`RestorablePanes::OtherVersion`), its panes cannot be listed or
    /// attached. The saved daemon tabs come back all the same, each pane
    /// showing that it is managed by another version: the daemon and the
    /// panes are left running, and the saved layout keeps their place.
    fn finish_restore_session(
        &mut self,
        panes: crate::terminal_view::RestorablePanes,
        cx: &mut Context<Self>,
    ) {
        use crate::terminal_view::RestorablePanes;
        use shelldeck_core::config::workspace_state::{RestoredTab, TabType};
        use shelldeck_core::config::WorkspaceState;

        let restore_saved = self.app_config.general.auto_connect_on_startup;
        let other_version = panes == RestorablePanes::OtherVersion;
        let mut live_panes = match panes {
            RestorablePanes::Live(panes) => panes,
            RestorablePanes::OtherVersion => Vec::new(),
        };
        if !restore_saved && live_panes.is_empty() && !other_version {
            return;
        }

        let state = match WorkspaceState::load() {
            Ok(state) => state,
            Err(e) => {
                tracing::warn!("Failed to load workspace state for restore: {}", e);
                // Live panes still come back, one tab each.
                WorkspaceState::default()
            }
        };
        if other_version {
            live_panes = state.saved_daemon_panes();
            if !restore_saved && live_panes.is_empty() {
                return;
            }
        }

        let plan = state.daemon_restore_plan(&live_panes);
        if plan.tabs.is_empty() {
            // Nothing saved and nothing running: leave the normal default
            // (empty) startup untouched.
            return;
        }

        let mut restored = 0usize;
        let mut restored_saved = 0usize;
        // Terminal tab opened for each planned tab, with the pane to focus.
        let mut opened: Vec<Option<(Uuid, Option<Uuid>)>> = Vec::with_capacity(plan.tabs.len());
        for planned in &plan.tabs {
            let before = self.terminal.read(cx).tabs.len();
            let mut focused = None;
            match planned {
                RestoredTab::Daemon { title, layout } => {
                    focused = layout.focused;
                    self.terminal.update(cx, |terminal, cx| {
                        terminal.restore_daemon_tab(title.as_deref(), layout, cx);
                    });
                }
                RestoredTab::Saved(_) if !restore_saved => {}
                RestoredTab::Saved(index) => {
                    let tab = &state.tabs[*index];
                    match tab.tab_type {
                        // The reattached panes are the local tabs; a fresh
                        // shell per saved tab would duplicate them.
                        TabType::Local if !live_panes.is_empty() => {}
                        TabType::Local => {
                            self.terminal.update(cx, |terminal, cx| {
                                terminal.spawn_local_terminal(cx);
                            });
                        }
                        TabType::Ssh => {
                            let connection = tab.connection_id.and_then(|conn_id| {
                                self.connections.iter().find(|c| c.id == conn_id).cloned()
                            });
                            match connection {
                                // Its splits come back once the host's
                                // daemon answers with the panes still there.
                                Some(conn) => self.connect_ssh_restoring(
                                    conn,
                                    tab.remote
                                        .clone()
                                        .filter(|_| self.app_config.general.session_daemon),
                                    cx,
                                ),
                                None => tracing::warn!(
                                    "Skipping SSH tab restore: connection {:?} no longer exists",
                                    tab.connection_id
                                ),
                            }
                        }
                    }
                }
            }
            let terminal = self.terminal.read(cx);
            let tab = (terminal.tabs.len() > before)
                .then(|| terminal.tabs.last().map(|tab| (tab.id, focused)))
                .flatten();
            if tab.is_some() {
                restored += 1;
                if matches!(planned, RestoredTab::Saved(_)) {
                    restored_saved += 1;
                }
            }
            opened.push(tab);
        }

        if restored == 0 {
            return;
        }

        // Restore sidebar visibility if it differs from the current state.
        // Only with saved tabs: live panes alone do not bring the saved
        // window state back.
        if restored_saved > 0 && state.sidebar_visible != self.sidebar_visible {
            self.toggle_sidebar(cx);
        }

        // Restore the active tab: the saved one when it came back, else
        // the nearest restored tab at or before its place.
        let target = plan
            .active
            .and_then(|index| opened.get(index).copied().flatten())
            .or_else(|| {
                let at = state.active_tab.min(opened.len() - 1);
                opened[..=at].iter().rev().find_map(|tab| *tab)
            })
            .or_else(|| opened.iter().find_map(|tab| *tab));
        if let Some((tab_id, focused)) = target {
            self.terminal.update(cx, |terminal, _| {
                terminal.activate_restored_tab(tab_id, focused);
            });
        }

        if restored_saved > 0 {
            self.active_view = ActiveView::Terminal;
        }
        self.update_dashboard_stats(cx);
        self.sync_terminal_tab_count(cx);
        cx.notify();
    }

    pub fn open_new_terminal(&mut self, cx: &mut Context<Self>) {
        if !self.enter_dev_mode(cx) {
            return;
        }
        self.terminal.update(cx, |terminal, cx| {
            terminal.spawn_local_terminal(cx);
        });
        self.set_active_view(ActiveView::Terminal, cx);
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

    /// Start periodic git status polling. Work is suspended while the main
    /// window is hidden in the tray, and the status bar only repaints when the
    /// displayed value actually changes.
    pub fn start_git_polling(&mut self, main_window: AnyWindowHandle, cx: &mut Context<Self>) {
        if self._git_poll_task.is_some() {
            return;
        }
        let weak_bar = self.status_bar.downgrade();
        self._git_poll_task = Some(cx.spawn(async move |_this, cx: &mut AsyncApp| loop {
            cx.background_executor()
                .timer(std::time::Duration::from_secs(15))
                .await;

            let main_window_visible = main_window
                .update(cx, |_, window, _| window.is_window_visible())
                .unwrap_or(false);
            if !main_window_visible {
                continue;
            }

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
        }));
    }
}

/// The Support console tab a Dev rail icon opens: Support lands on the
/// console's Accueil, as Support mode does; Requests on its Demandes tab.
pub(super) fn support_rail_target(
    section: SidebarSection,
) -> Option<crate::support_view::SupportSection> {
    match section {
        SidebarSection::Support => Some(crate::support_view::SupportSection::Home),
        SidebarSection::Requests => Some(crate::support_view::SupportSection::Requests),
        _ => None,
    }
}
