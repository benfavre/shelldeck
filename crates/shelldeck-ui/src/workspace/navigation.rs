use super::*;

impl Workspace {
    pub(super) fn update_dashboard_stats(&mut self, cx: &mut Context<Self>) {
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
            bar.set_counts(active_connections, active_forwards, running_scripts);
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
        // Close all terminal sessions (drops channels, threads exit)
        self.terminal.update(cx, |terminal, _| {
            terminal.close_all_sessions();
        });
        self.ai_script_runs.clear();
        self.ai_terminal_runs.clear();
        self.ai_diagnostic_sequences.clear();
        self.ai_action_confirmation = None;
    }

    pub fn set_active_view(&mut self, view: ActiveView) {
        self.close_titlebar_menus();
        self.active_view = view;
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
        if !matches!(link, DeepLink::Assistant) && !self.signed_in() {
            self.show_login_form(cx);
            return;
        }
        match link {
            // The application-level CompanionRuntime owns this target. It
            // authenticates before forwarding, so keep this arm inert if
            // another caller accidentally routes it into Workspace.
            DeepLink::Assistant => {}
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
                self.switch_to_section(SidebarSection::Connections);
                self.sidebar.update(cx, |s, cx| {
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
                if !self.enter_dev_mode(cx) {
                    return;
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
                    v.set_section(crate::support_view::SupportSection::Tickets);
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

    pub fn toggle_sidebar(&mut self, cx: &mut Context<Self>) {
        if !self.enter_dev_mode(cx) {
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
    }

    pub fn switch_to_section(&mut self, section: SidebarSection) {
        self.close_titlebar_menus();
        self.active_view = match section {
            SidebarSection::Connections => ActiveView::Dashboard,
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
            SidebarSection::Settings => ActiveView::Settings,
        };
    }

    pub(super) fn activate_dev_section(&mut self, section: SidebarSection, cx: &mut Context<Self>) {
        if !self.enter_dev_mode(cx) {
            return;
        }
        self.switch_to_section(section);
        self.sidebar.update(cx, |sidebar, cx| {
            sidebar.set_active_section(section);
            cx.notify();
        });
        self.on_active_view_changed(cx);
        cx.notify();
    }

    /// Called when the active Dev view changes — (re)start the Monique poll if the
    /// console just became visible.
    pub(super) fn on_active_view_changed(&mut self, cx: &mut Context<Self>) {
        self.close_titlebar_menus();
        self.sync_monique_poll(cx);
        self.sync_fleet_view_poll(cx);
        self.sync_bext_poll(cx);
        self.refresh_command_palette(cx);
    }

    pub(super) fn sync_terminal_tab_count(&self, cx: &mut Context<Self>) {
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
        if !self.enter_dev_mode(cx) {
            return;
        }
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
