use super::*;

impl Workspace {
    pub(super) fn handle_sidebar_event(&mut self, event: &SidebarEvent, cx: &mut Context<Self>) {
        // Sidebar events may also be synthesized by the native tray. Never
        // rely on the Dev sidebar being absent from the rendered tree as an
        // authorization boundary.
        if !self.can_access_mode(AppMode::Dev) {
            return;
        }
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
                        self.refresh_agent_connections(cx);
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

    pub(super) fn handle_terminal_event(&mut self, event: &TerminalEvent, cx: &mut Context<Self>) {
        if !self.can_access_mode(AppMode::Dev) {
            return;
        }
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

    pub(super) fn handle_update_event(&mut self, event: &AutoUpdateEvent, cx: &mut Context<Self>) {
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

    pub(super) fn handle_settings_event(&mut self, event: &SettingsEvent, cx: &mut Context<Self>) {
        match event {
            SettingsEvent::CloseRequested => {
                self.settings_open = false;
                self.activate_current_mode(cx);
                cx.notify();
            }
            SettingsEvent::ConfigChanged(config) => {
                tracing::info!("Config changed, applying settings");
                let companion_changed = self.app_config.companion != config.companion
                    || self.app_config.clippy != config.clippy;
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
                self.app_config.clippy = config.clippy.clone();
                self.app_config.ai = config.ai.clone();
                *self.ai_companion_config.borrow_mut() = config.ai.clone();
                *self.clippy_companion_config.borrow_mut() = config.clippy.clone();
                let dock_available =
                    self.ai_backend_available() && self.app_config.ai.allows(AiSurface::Global);
                let clippy_available =
                    self.ai_backend_available() && self.app_config.ai.allows(AiSurface::Clippy);
                self.ai_dock_assistant.update(cx, |assistant, cx| {
                    assistant.set_backend(
                        self.app_config.ai.backend,
                        self.app_config.ai.model.clone(),
                        cx,
                    );
                    assistant.set_available(dock_available, cx);
                    assistant.set_clippy_available(clippy_available, cx);
                    assistant.set_clippy_auto_import_clipboard(
                        self.app_config.clippy.auto_import_clipboard_on_shortcut,
                        cx,
                    );
                });
                // The Sheet entity holds the same Clippy state and used to be
                // skipped here, so `ai.surfaces.clippy` was silently ignored
                // in the main window. Its `available` flag is deliberately
                // NOT pushed: unlike the Dock (always Global surface), the
                // Sheet's availability is per-context and validated at open
                // time by `open_ai_assistant_with_context` — pushing the
                // Global-surface value would wrongly disable e.g. a
                // Terminal-context sheet.
                self.ai_assistant.update(cx, |assistant, cx| {
                    assistant.set_backend(
                        self.app_config.ai.backend,
                        self.app_config.ai.model.clone(),
                        cx,
                    );
                    assistant.set_clippy_available(clippy_available, cx);
                    assistant.set_clippy_auto_import_clipboard(
                        self.app_config.clippy.auto_import_clipboard_on_shortcut,
                        cx,
                    );
                });
                self.sync_ai_affordances(cx);
                // Apply terminal settings to running view
                let terminal_theme = TerminalTheme::by_name(&self.app_config.terminal.theme);
                // Apply sidebar width (panel + activity rail).
                self.sidebar_width = self.app_config.general.sidebar_width;
                let total = self.sidebar.read(cx).total_width();
                self.workspace_hub.update(cx, |hub, cx| {
                    hub.configure_terminals(
                        super::workspaces::WorkspaceTerminalConfig {
                            theme: terminal_theme,
                            font_size: self.app_config.terminal.font_size,
                            font_family: self.app_config.terminal.font_family.clone(),
                            default_shell: self.app_config.terminal.default_shell.clone(),
                            cursor_style: self.app_config.terminal.cursor_style.clone(),
                            cursor_blink: self.app_config.terminal.cursor_blink,
                            scrollback_lines: self.app_config.terminal.scrollback_lines,
                            sidebar_width: total,
                            menu_bar_visible: self.app_config.general.menu_bar_visible,
                        },
                        cx,
                    );
                });
                // Apply application UI font (cascades to all child views on re-render)
                self.resolved_ui_font_family =
                    resolve_ui_font_family(&self.app_config.general.ui_font_family, cx);
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
                        publisher(
                            self.app_config.companion.clone(),
                            self.app_config.clippy.clone(),
                        );
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
    pub(super) fn apply_autostart_request(&mut self, desired: bool, cx: &mut Context<Self>) {
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
    pub(super) fn apply_palette(&self, pref: &ThemePreference, cx: &mut Context<Self>) {
        ShellDeckColors::set_theme(pref);
        install_theme(cx.deref_mut(), crate::theme::adabraka_theme_from_palette());
        self.notify_theme_views(cx);
    }

    /// Notify every child view (and self) to re-render with the active palette.
    pub(super) fn notify_theme_views(&self, cx: &mut Context<Self>) {
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
    pub(super) fn preview_palette_action(&mut self, action: &dyn Action, cx: &mut Context<Self>) {
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
            if !self.can_access_mode(AppMode::Dev) {
                return;
            }
            let name = t.name.clone();
            self.preview_terminal_theme(&name, cx);
        } else {
            // A non-theme entry: end any active preview of either kind.
            self.revert_theme_preview(cx);
            self.revert_terminal_theme_preview(cx);
        }
    }

    /// Restore the app theme captured before previewing, if a preview is active.
    pub(super) fn revert_theme_preview(&mut self, cx: &mut Context<Self>) {
        if let Some(orig) = self.theme_before_preview.take() {
            self.apply_palette(&orig, cx);
        }
    }

    /// Apply a terminal color theme to the live terminal without persisting,
    /// remembering the original so it can be restored on dismiss.
    pub(super) fn preview_terminal_theme(&mut self, name: &str, cx: &mut Context<Self>) {
        if self.terminal_theme_before_preview.is_none() {
            self.terminal_theme_before_preview = Some(self.app_config.terminal.theme.clone());
        }
        self.terminal_theme_preview = Some(name.to_owned());
        let theme = TerminalTheme::by_name(name);
        self.terminal.update(cx, |terminal, cx| {
            terminal.set_terminal_theme(&theme);
            cx.notify();
        });
    }

    /// Restore the terminal theme captured before previewing, if active.
    pub(super) fn revert_terminal_theme_preview(&mut self, cx: &mut Context<Self>) {
        self.terminal_theme_preview = None;
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
    pub(super) fn commit_theme_preview(&mut self, pref: ThemePreference, cx: &mut Context<Self>) {
        self.theme_before_preview = None;
        self.settings
            .update(cx, |settings, cx| settings.select_app_theme(pref, cx));
    }

    /// Apply a terminal color theme by name: persist it (which repaints the
    /// live terminal via `ConfigChanged`) and surface a confirmation toast.
    /// Used by the command palette's theme entries.
    pub fn apply_terminal_theme_by_name(&mut self, name: &str, cx: &mut Context<Self>) {
        if !self.enter_dev_mode(cx) {
            return;
        }
        self.settings.update(cx, |settings, cx| {
            settings.select_terminal_theme(name, cx);
        });
        self.show_toast(
            t!("toast.terminal_theme", name = name).to_string(),
            ToastLevel::Info,
            cx,
        );
    }
}
