use super::*;

impl Workspace {
    pub(super) fn handle_dashboard_event(
        &mut self,
        event: &DashboardEvent,
        cx: &mut Context<Self>,
    ) {
        if !self.can_access_mode(AppMode::Dev) {
            return;
        }
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

    pub(super) fn handle_recent_event(&mut self, event: RecentEvent, cx: &mut Context<Self>) {
        if !self.can_access_mode(AppMode::Dev) {
            return;
        }
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

    pub(super) fn open_activity(&mut self, entry: ActivityEntry, cx: &mut Context<Self>) {
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
            ActivityAction::OpenMonique => self.open_monique_console(cx),
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

    pub(super) fn add_activity(
        &mut self,
        message: String,
        kind: ActivityKind,
        cx: &mut Context<Self>,
    ) {
        self.add_activity_entry(ActivityEntry::new(kind, message), cx);
    }

    pub(super) fn add_activity_entry(&mut self, entry: ActivityEntry, cx: &mut Context<Self>) {
        if let Err(e) = ActivityStore::append(&entry) {
            tracing::warn!("Failed to append activity entry: {}", e);
        }
        self.recent_activity.insert(0, entry);
        if self.recent_activity.len() > 500 {
            self.recent_activity.truncate(500);
        }
        self.push_recent_activity(cx);
    }

    pub(super) fn push_recent_activity(&mut self, cx: &mut Context<Self>) {
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
}
