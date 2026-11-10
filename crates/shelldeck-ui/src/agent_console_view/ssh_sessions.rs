//! Explicit SSH discovery and native CLI resume, separate from the Platform directory.
use super::navigation_groups::{directory_group_key, directory_name};
use super::navigator::{project_group_collapsed, NavigatorState};
use super::*;
use shelldeck_core::agent_remote_sessions::{RemoteAgentSession, RemoteAgentSessionInventory};
use std::collections::BTreeMap;

#[derive(Default)]
pub(super) struct SshSessionsUi {
    selector: Option<Entity<Select<Uuid>>>,
    chosen: Option<Uuid>,
    servers: BTreeMap<Uuid, ServerSessions>,
    local: Option<ServerSessions>,
    collapsed: bool,
    pub(super) preview: Option<(Option<Uuid>, RemoteAgentSession)>,
    reconnecting: Option<Uuid>,
    reconnect_error: Option<String>,
}
#[derive(Default)]
struct ServerSessions {
    request: Uuid,
    requested_cursor: Option<String>,
    loading: bool,
    inventory: Option<RemoteAgentSessionInventory>,
    checked_at: Option<i64>,
    error: Option<String>,
    visible_limits: HashMap<String, usize>,
}

fn timestamp(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms)
        .map(|at| {
            at.with_timezone(&chrono::Local)
                .format("%d/%m %H:%M")
                .to_string()
        })
        .unwrap_or_default()
}
fn matches_session(session: &RemoteAgentSession, server: &str, query: &str) -> bool {
    let fields = [
        session.title.as_str(),
        session.workdir.as_str(),
        session.session_id.as_str(),
        server,
        session.provider.display_name(),
    ]
    .map(str::to_lowercase);
    query
        .split_whitespace()
        .all(|term| fields.iter().any(|field| field.contains(term)))
}
impl AgentConsoleView {
    pub(super) fn rebuild_ssh_session_selector(&mut self, cx: &mut Context<Self>) {
        self.ssh_sessions
            .servers
            .retain(|id, _| self.connections.iter().any(|c| c.id == *id));
        if self.ssh_sessions.preview.as_ref().is_some_and(|(id, _)| {
            id.is_some_and(|id| !self.connections.iter().any(|c| c.id == id))
        }) {
            self.clear_ssh_session_preview();
        }
        if !self
            .connections
            .iter()
            .any(|c| Some(c.id) == self.ssh_sessions.chosen)
        {
            self.ssh_sessions.chosen = self
                .library
                .preferences
                .ssh_selected_server
                .filter(|id| self.connections.iter().any(|c| c.id == *id));
        }
        let options = self
            .connections
            .iter()
            .map(|c| SelectOption::new(c.id, format!("{} · {}", c.label, c.host)))
            .collect::<Vec<_>>();
        let index = self
            .connections
            .iter()
            .position(|c| Some(c.id) == self.ssh_sessions.chosen);
        let parent = cx.entity();
        self.ssh_sessions.selector = Some(cx.new(move |cx| {
            Select::new(cx)
                .options(options)
                .selected_index(index)
                .searchable(true)
                .placeholder(t!("agents.ssh_sessions.choose_server").to_string())
                .search_placeholder(t!("agents.target.search").to_string())
                .on_change(move |id, _, cx| {
                    parent.update(cx, |this, cx| {
                        this.scan_ssh_sessions(*id, cx);
                    })
                })
        }));
    }
    pub(super) fn restore_ssh_preferences(&mut self, cx: &mut Context<Self>) {
        self.ssh_sessions.chosen = self.library.preferences.ssh_selected_server;
        self.ssh_sessions.collapsed = self.library.preferences.ssh_sessions_collapsed;
        self.rebuild_ssh_session_selector(cx);
    }
    pub(super) fn ensure_ssh_discovery(&mut self, cx: &mut Context<Self>) {
        if self.library.preferences.local_sessions_enabled
            && self.ssh_sessions.local.is_none()
            && self.connection_repair_storage_ready()
            && !project_group_collapsed(&self.library.preferences, "native-local", "")
        {
            self.scan_native_source(None, None, cx);
        }
        if self.ssh_sessions.collapsed || !self.connection_repair_storage_ready() {
            return;
        }
        if let Some(id) = self.library.preferences.ssh_selected_server {
            if !self.ssh_sessions.servers.contains_key(&id)
                && self.connections.iter().any(|c| c.id == id)
            {
                self.scan_ssh_sessions(id, cx);
            }
        }
    }
    fn hide_ssh_server(&mut self, id: Uuid, cx: &mut Context<Self>) {
        self.ssh_sessions.servers.remove(&id);
        if self.ssh_sessions.chosen == Some(id) {
            self.ssh_sessions.chosen = None;
            self.library.preferences.ssh_selected_server = None;
        }
        self.rebuild_ssh_session_selector(cx);
        self.schedule_library_save(cx);
        cx.notify();
    }
    pub fn reset_ssh_sessions(&mut self, cx: &mut Context<Self>) {
        self.ssh_sessions = SshSessionsUi::default();
        self.native_history.clear();
        cx.notify();
    }
    pub(crate) fn clear_ssh_session_preview(&mut self) {
        self.ssh_sessions.preview = None;
        self.ssh_sessions.reconnecting = None;
        self.ssh_sessions.reconnect_error = None;
    }
    pub fn scan_ssh_sessions(&mut self, connection_id: Uuid, cx: &mut Context<Self>) {
        if !self.connections.iter().any(|c| c.id == connection_id) {
            return;
        }
        self.request_ssh_session_page(connection_id, None, cx);
    }
    fn request_ssh_session_page(
        &mut self,
        connection_id: Uuid,
        cursor: Option<String>,
        cx: &mut Context<Self>,
    ) {
        self.ssh_sessions.chosen = Some(connection_id);
        self.library.preferences.ssh_selected_server = Some(connection_id);
        self.ssh_sessions.collapsed = false;
        self.library.preferences.ssh_sessions_collapsed = false;
        self.rebuild_ssh_session_selector(cx);
        let server = self.ssh_sessions.servers.entry(connection_id).or_default();
        if server.loading {
            return;
        }
        server.request = Uuid::new_v4();
        server.loading = true;
        server.requested_cursor = cursor.clone();
        server.error = None;
        cx.emit(AgentConsoleEvent::DiscoverSshSessions {
            connection_id,
            request_id: server.request,
            cursor,
        });
        self.schedule_library_save(cx);
        cx.notify();
    }
    pub fn set_ssh_sessions(
        &mut self,
        connection_id: Uuid,
        request_id: Uuid,
        result: Result<RemoteAgentSessionInventory, String>,
        cx: &mut Context<Self>,
    ) {
        self.set_native_sessions(Some(connection_id), request_id, result, cx);
    }
    pub fn set_native_sessions(
        &mut self,
        connection_id: Option<Uuid>,
        request_id: Uuid,
        result: Result<RemoteAgentSessionInventory, String>,
        cx: &mut Context<Self>,
    ) {
        let server = match connection_id {
            Some(id) => self.ssh_sessions.servers.get_mut(&id),
            None => self.ssh_sessions.local.as_mut(),
        };
        let Some(server) = server else {
            return;
        };
        if server.request != request_id || !server.loading {
            return;
        }
        server.loading = false;
        match result {
            Ok(mut inventory) => {
                if server.requested_cursor.is_some() {
                    if let Some(previous) = server.inventory.take() {
                        let mut sessions = previous.sessions;
                        for incoming in inventory.sessions {
                            if let Some(existing) = sessions.iter_mut().find(|s| {
                                s.provider == incoming.provider
                                    && s.session_id == incoming.session_id
                            }) {
                                *existing = incoming;
                            } else {
                                sessions.push(incoming);
                            }
                        }
                        inventory.sessions = sessions;
                        for warning in previous.warnings {
                            if !inventory.warnings.contains(&warning) {
                                inventory.warnings.push(warning);
                            }
                        }
                    }
                }
                server.inventory = Some(inventory);
                server.checked_at = Some(now_ms());
                server.error = None;
            }
            Err(error) => server.error = Some(error),
        }
        cx.notify();
    }
    pub fn complete_ssh_reconnect(
        &mut self,
        request_id: Uuid,
        result: Result<shelldeck_core::agent_remote_history::RemoteHistoryPage, String>,
        cx: &mut Context<Self>,
    ) {
        if self.ssh_sessions.reconnecting != Some(request_id) {
            return;
        }
        self.ssh_sessions.reconnecting = None;
        let page = match result {
            Ok(page) => page,
            Err(error) => {
                self.ssh_sessions.reconnect_error = Some(error);
                cx.notify();
                return;
            }
        };
        let Some((connection_id, remote)) = self.ssh_sessions.preview.clone() else {
            return;
        };
        let target = if let Some(connection_id) = connection_id {
            let Some(connection) = self.connections.iter().find(|c| c.id == connection_id) else {
                return;
            };
            AgentTarget::Ssh {
                connection_id,
                connection_label: connection.label.clone(),
            }
        } else {
            AgentTarget::Local
        };
        if page.provider != remote.provider
            || page.session_id != remote.session_id
            || page.workdir != remote.workdir
        {
            self.ssh_sessions.reconnect_error =
                Some(t!("agents.ssh_sessions.session_missing").to_string());
            cx.notify();
            return;
        }
        let context = AgentExecutionContext {
            provider: remote.provider,
            target,
            workdir: remote.workdir.clone(),
            access: AgentAccessMode::ReadOnly,
            model: page.model.clone(),
        };
        if !self.connection_repair_storage_ready() {
            self.ssh_sessions.reconnect_error =
                Some(t!("agents.ssh_sessions.library_loading").to_string());
            cx.notify();
            return;
        }
        self.retain_draft_context(cx);
        match self.sessions.reconnect_remote(
            remote.title.clone(),
            context,
            remote.session_id.clone(),
            now_ms(),
        ) {
            Ok(id) => {
                self.install_native_history(id, page);
                self.session_ui
                    .entry(id)
                    .or_insert_with(|| AgentSessionUi::new(cx));
                self.clear_ssh_session_preview();
                self.set_remote_session_active(false, cx);
                self.apply_selected_context(cx);
                self.schedule_library_save(cx);
                self.surface_focus_requested = true;
            }
            Err(error) => self.ssh_sessions.reconnect_error = Some(error.to_string()),
        }
        cx.notify();
    }
    pub(super) fn render_ssh_session_rows(&self, cx: &mut Context<Self>) -> AnyElement {
        let query = self
            .navigator_search
            .read(cx)
            .content()
            .trim()
            .to_lowercase();
        let expanded = !self.ssh_sessions.collapsed || !query.is_empty();
        let mut list = div()
            .id("agent-ssh-section")
            .flex()
            .flex_col()
            .gap(px(4.0))
            .py(px(8.0))
            .border_b_1()
            .border_color(ShellDeckColors::border())
            .child(
                self.render_navigation_section_toggle(
                    "ssh-sessions-toggle",
                    t!("agents.ssh_sessions.title").to_string(),
                    !expanded,
                    "server",
                    cx,
                    |this, cx| {
                        this.ssh_sessions.collapsed = !this.ssh_sessions.collapsed;
                        this.library.preferences.ssh_sessions_collapsed =
                            this.ssh_sessions.collapsed;
                        this.schedule_library_save(cx);
                        cx.notify();
                    },
                )
                .hover(|style| style.bg(ShellDeckColors::hover_bg())),
            );
        if !expanded {
            return list.into_any_element();
        }
        if let Some(select) = &self.ssh_sessions.selector {
            list = list.child(select.clone());
        }
        if self.ssh_sessions.servers.is_empty() {
            list = list.child(
                div()
                    .text_size(px(10.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(t!("agents.ssh_sessions.intro").to_string()),
            );
        }
        for connection in &self.connections {
            if let Some(server) = self.ssh_sessions.servers.get(&connection.id) {
                list = list.child(self.render_native_source_rows(
                    Some(connection.id),
                    connection.label.clone(),
                    connection.host.clone(),
                    server,
                    &query,
                    cx,
                ));
            }
        }
        list.into_any_element()
    }
    fn render_native_source_rows(
        &self,
        id: Option<Uuid>,
        label: String,
        host: String,
        server: &ServerSessions,
        query: &str,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mut list = div().flex().flex_col().gap(px(4.0)).min_w_0();
        if !query.is_empty()
            && server.inventory.as_ref().is_some_and(|i| {
                !i.sessions
                    .iter()
                    .any(|session| matches_session(session, &label, query))
            })
            && !server.loading
        {
            return list.into_any_element();
        }
        let source_key = id.map_or_else(|| "local".to_string(), |id| id.to_string());
        let server_key = id.map_or_else(
            || "native-local".to_string(),
            |id| format!("ssh-server:{id}"),
        );
        let server_collapsed =
            project_group_collapsed(&self.library.preferences, &server_key, query);
        let selected = self
            .ssh_sessions
            .preview
            .as_ref()
            .filter(|(connection, _)| *connection == id)
            .map(|(_, session)| session);
        let count = server
            .inventory
            .as_ref()
            .map_or(0, |inventory| inventory.sessions.len());
        list = list.child(
            div()
                .flex()
                .items_center()
                .min_w_0()
                .gap(px(2.0))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .child(self.render_navigation_group_header(
                            &server_key,
                            label.clone(),
                            format!("{}\n{}", label, host),
                            (count, NavigatorState::Quiet),
                            cx,
                        )),
                )
                .child(
                    Button::new(
                        SharedString::from(format!("native-refresh-{source_key}")),
                        "",
                    )
                    .debug_selector({
                        let selector = format!("native-refresh-{source_key}");
                        move || selector.clone()
                    })
                    .variant(ButtonVariant::Ghost)
                    .size(ButtonSize::Icon)
                    .w(px(24.0))
                    .h(px(24.0))
                    .flex_shrink_0()
                    .icon(IconSource::from("refresh-cw"))
                    .tooltip(if id.is_none() {
                        t!("agents.local_sessions.refresh").to_string()
                    } else {
                        t!("agents.ssh_sessions.refresh_server").to_string()
                    })
                    .disabled(server.loading)
                    .on_click(
                        cx.listener(move |this, _, _, cx| this.scan_native_source(id, None, cx)),
                    ),
                )
                .child(
                    Button::new(SharedString::from(format!("native-hide-{source_key}")), "")
                        .debug_selector({
                            let selector = format!("native-hide-{source_key}");
                            move || selector.clone()
                        })
                        .variant(ButtonVariant::Ghost)
                        .size(ButtonSize::Icon)
                        .w(px(24.0))
                        .h(px(24.0))
                        .flex_shrink_0()
                        .icon(IconSource::from("eye-off"))
                        .tooltip(if id.is_none() {
                            t!("agents.local_sessions.hide").to_string()
                        } else {
                            t!("agents.ssh_sessions.hide_server").to_string()
                        })
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.hide_native_source(id, cx);
                        })),
                ),
        );
        if !server_collapsed {
            let label = if server.loading {
                if id.is_none() {
                    t!("agents.local_sessions.loading").to_string()
                } else {
                    t!("agents.ssh_sessions.connecting").to_string()
                }
            } else if let Some(at) = server.checked_at {
                t!("agents.ssh_sessions.updated", time = timestamp(at)).to_string()
            } else {
                t!("agents.ssh_sessions.not_connected").to_string()
            };
            list = list.child(
                div()
                    .text_size(px(10.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(label),
            );
            if let Some(error) = &server.error {
                list = list.child(
                    div()
                        .text_size(px(10.0))
                        .text_color(ShellDeckColors::warning())
                        .whitespace_normal()
                        .child(t!("agents.ssh_sessions.failed", error = error).to_string()),
                );
            }
        }
        let Some(inventory) = &server.inventory else {
            return list.into_any_element();
        };
        if !server_collapsed {
            if inventory.truncated && inventory.next_cursor.is_none() {
                list = list.child(
                    div()
                        .text_size(px(10.0))
                        .text_color(ShellDeckColors::warning())
                        .child(t!("agents.ssh_sessions.truncated").to_string()),
                );
            }
            for warning in &inventory.warnings {
                list = list.child(
                    div()
                        .text_size(px(10.0))
                        .whitespace_normal()
                        .text_color(ShellDeckColors::text_muted())
                        .child(warning.clone()),
                );
            }
            if inventory.sessions.is_empty() {
                list = list.child(div().text_size(px(10.0)).child(if id.is_none() {
                    t!("agents.local_sessions.empty").to_string()
                } else {
                    t!("agents.ssh_sessions.empty").to_string()
                }));
            }
            if inventory.next_cursor.is_some() {
                list = list.child(
                    Button::new(
                        SharedString::from(format!("native-older-{source_key}")),
                        t!("agents.ssh_sessions.load_older").to_string(),
                    )
                    .tooltip(if id.is_none() {
                        t!("agents.local_sessions.older_hint").to_string()
                    } else {
                        t!("agents.ssh_sessions.load_older_hint").to_string()
                    })
                    .variant(ButtonVariant::Ghost)
                    .size(ButtonSize::Sm)
                    .disabled(server.loading)
                    .on_click(
                        cx.listener(move |this, _, _, cx| this.load_older_native_sessions(id, cx)),
                    ),
                );
            }
        }
        let target = id.map_or(AgentTarget::Local, |connection_id| AgentTarget::Ssh {
            connection_id,
            connection_label: label.clone(),
        });
        let mut folders: BTreeMap<&str, Vec<&RemoteAgentSession>> = BTreeMap::new();
        for session in &inventory.sessions {
            if matches_session(session, &label, query) {
                folders.entry(&session.workdir).or_default().push(session);
            }
        }
        for (path, sessions) in &folders {
            let is_selected = |session: &&RemoteAgentSession| {
                selected.is_some_and(|current| {
                    current.provider == session.provider
                        && current.session_id == session.session_id
                        && current.workdir == session.workdir
                })
            };
            if server_collapsed {
                continue;
            }
            let key = directory_group_key("ssh-folder", &target, path);
            let collapsed =
                server_collapsed || project_group_collapsed(&self.library.preferences, &key, query);
            let name = directory_name(&target, path);
            let duplicate = folders
                .keys()
                .any(|other| other != path && directory_name(&target, other) == name);
            let title = if duplicate { (*path).to_owned() } else { name };
            let hidden = sessions.len();
            let mut folder = div()
                .debug_selector(|| format!("agent-folder-{key}"))
                .flex()
                .flex_col()
                .flex_shrink_0()
                .min_w_0()
                .pl(px(8.0))
                .gap(px(2.0));
            if !server_collapsed {
                folder = folder.child(self.render_navigation_group_header(
                    &key,
                    title,
                    format!("{}\n{}", label, path),
                    (hidden, NavigatorState::Quiet),
                    cx,
                ));
            }
            let mut rows = div().flex().flex_col().min_w_0().gap(px(2.0));
            let limit = server.visible_limits.get(&key).copied().unwrap_or(10);
            for session in sessions.iter().enumerate().filter_map(|(index, session)| {
                let selected = is_selected(session);
                (!collapsed && (index < limit || selected)).then_some(*session)
            }) {
                let remote = session.clone();
                let provider = session.provider;
                let selected = is_selected(&session);
                let selector = format!(
                    "agent-ssh-conversation-{source_key}-{:?}-{}",
                    session.provider, session.session_id
                );
                let icon_selector = format!("{selector}-provider");
                let provider_mark = div()
                    .id(SharedString::from(icon_selector.clone()))
                    .size(px(14.0))
                    .flex_shrink_0()
                    .debug_selector(move || icon_selector.clone())
                    .cursor_pointer()
                    .child(crate::icons::agent_provider_icon(
                        provider,
                        14.0,
                        ShellDeckColors::text_muted(),
                    ));
                // The row owns one highlight across the provider, label and padding;
                // the canonical Button retains keyboard focus and activation.
                rows = rows.child(
                    div()
                        .id(SharedString::from(format!("{selector}-row")))
                        .debug_selector(|| selector.clone())
                        // Include the existing icon gutter while retaining label alignment.
                        .ml(px(-17.0))
                        .flex()
                        .items_center()
                        .gap(px(7.0))
                        .px(px(8.0))
                        .h(px(32.0))
                        .rounded(px(5.0))
                        .min_w_0()
                        .cursor_pointer()
                        .when(selected, |row| {
                            row.bg(ShellDeckColors::selected_bg().opacity(0.65))
                        })
                        .when(!selected, |row| {
                            row.hover(|s| s.bg(ShellDeckColors::hover_bg()))
                        })
                        .on_click(cx.listener({
                            let remote = remote.clone();
                            move |this, _, _, cx| {
                                this.preview_native_session(id, remote.clone(), cx)
                            }
                        }))
                        .child(provider_mark)
                        .child(
                            Button::new(
                                SharedString::from(selector.clone()),
                                session.title.clone(),
                            )
                            .debug_selector(move || format!("{selector}-label"))
                            .variant(ButtonVariant::Ghost)
                            .hover_background(gpui::transparent_black())
                            .px(px(0.0))
                            .flex_1()
                            .size(ButtonSize::Sm)
                            .justify_start()
                            .w_full()
                            .min_w_0()
                            .overflow_hidden()
                            .text_ellipsis()
                            .h(px(32.0))
                            .text_size(px(13.0))
                            .tooltip(format!(
                                "{}\n{}\n{}\n{}",
                                session.title,
                                session.provider.display_name(),
                                session.workdir,
                                timestamp(session.updated_at_ms as i64)
                            ))
                            .on_click(cx.listener(
                                move |this, _, _, cx| {
                                    cx.stop_propagation();
                                    this.preview_native_session(id, remote.clone(), cx);
                                },
                            )),
                        ),
                );
            }
            let shown = sessions.len().min(limit)
                + sessions
                    .iter()
                    .skip(limit)
                    .filter(|session| is_selected(session))
                    .count();
            if !collapsed && sessions.len() > shown {
                let selector = format!("agent-ssh-more-{key}");
                let page_key = key.clone();
                rows = rows.child(
                    Button::new(
                        SharedString::from(selector.clone()),
                        t!(
                            "agents.remote.show_more",
                            shown = shown,
                            total = sessions.len()
                        )
                        .to_string(),
                    )
                    .debug_selector(move || selector.clone())
                    .variant(ButtonVariant::Ghost)
                    .size(ButtonSize::Sm)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(server) = match id {
                            Some(id) => this.ssh_sessions.servers.get_mut(&id),
                            None => this.ssh_sessions.local.as_mut(),
                        } {
                            *server.visible_limits.entry(page_key.clone()).or_insert(10) += 20;
                        }
                        cx.notify();
                    })),
                );
            }
            list = list.child(folder.child(super::navigation_groups::navigation_children(
                &key,
                rows.into_any_element(),
            )));
        }
        list.into_any_element()
    }
    fn preview_native_session(
        &mut self,
        id: Option<Uuid>,
        remote: RemoteAgentSession,
        cx: &mut Context<Self>,
    ) {
        self.retain_draft_context(cx);
        self.ssh_sessions.preview = Some((id, remote));
        self.set_remote_session_active(false, cx);
        self.ssh_sessions.reconnect_error = None;
        self.ssh_sessions.reconnecting = None;
        self.sessions.set_surface_visible(false);
        cx.notify();
    }
    fn hide_native_source(&mut self, id: Option<Uuid>, cx: &mut Context<Self>) {
        if let Some(id) = id {
            self.hide_ssh_server(id, cx);
        } else {
            self.ssh_sessions.local = None;
            self.library.preferences.local_sessions_enabled = false;
        }
        if self
            .ssh_sessions
            .preview
            .as_ref()
            .is_some_and(|(source, _)| *source == id)
        {
            self.clear_ssh_session_preview();
            self.sessions
                .set_surface_visible(self.remote_sessions.surface_visible);
        }
        self.schedule_library_save(cx);
        cx.notify();
    }
    pub(super) fn render_local_session_rows(&self, cx: &mut Context<Self>) -> AnyElement {
        let query = self
            .navigator_search
            .read(cx)
            .content()
            .trim()
            .to_lowercase();
        if let Some(server) = &self.ssh_sessions.local {
            self.render_native_source_rows(
                None,
                t!("agents.local_sessions.title").to_string(),
                t!("agents.target.local").to_string(),
                server,
                &query,
                cx,
            )
        } else {
            div()
                .py(px(8.0))
                .child(
                    Button::new(
                        "discover-local-sessions",
                        t!("agents.local_sessions.discover").to_string(),
                    )
                    .debug_selector(|| "agent-local-discover".into())
                    .variant(ButtonVariant::Ghost)
                    .size(ButtonSize::Sm)
                    .icon(IconSource::from("terminal"))
                    .w_full()
                    .justify_start()
                    .tooltip(t!("agents.local_sessions.help").to_string())
                    .on_click(cx.listener(|this, _, _, cx| this.scan_local_sessions(cx))),
                )
                .into_any_element()
        }
    }
    fn load_older_native_sessions(&mut self, id: Option<Uuid>, cx: &mut Context<Self>) {
        let server = match id {
            Some(id) => self.ssh_sessions.servers.get(&id),
            None => self.ssh_sessions.local.as_ref(),
        };
        let cursor = server
            .and_then(|s| s.inventory.as_ref())
            .and_then(|i| i.next_cursor.clone());
        if cursor.is_some() {
            self.scan_native_source(id, cursor, cx);
        }
    }
    pub fn scan_local_sessions(&mut self, cx: &mut Context<Self>) {
        self.scan_native_source(None, None, cx);
    }
    fn scan_native_source(
        &mut self,
        id: Option<Uuid>,
        cursor: Option<String>,
        cx: &mut Context<Self>,
    ) {
        if let Some(id) = id {
            self.request_ssh_session_page(id, cursor, cx);
            return;
        }
        self.library.preferences.local_sessions_enabled = true;
        self.schedule_library_save(cx);
        let server = self.ssh_sessions.local.get_or_insert_with(Default::default);
        if server.loading {
            return;
        }
        server.loading = true;
        server.error = None;
        server.request = Uuid::new_v4();
        server.requested_cursor = cursor.clone();
        cx.emit(AgentConsoleEvent::DiscoverLocalSessions {
            request_id: server.request,
            cursor,
        });
        cx.notify();
    }
    pub(super) fn render_ssh_session_preview(
        &mut self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some((connection_id, session)) = self.ssh_sessions.preview.clone() else {
            return div().into_any_element();
        };
        let label = self
            .connections
            .iter()
            .find(|c| Some(c.id) == connection_id)
            .map(|c| format!("{} · {}", c.label, c.host))
            .unwrap_or_else(|| {
                if connection_id.is_none() {
                    t!("agents.target.local").to_string()
                } else {
                    t!("agents.target.missing").to_string()
                }
            });
        let model = session.model.clone();
        let busy = self.ssh_sessions.reconnecting.is_some();
        let mut details = div()
            .id("ssh-session-preview")
            .debug_selector(|| "agent-ssh-preview".into())
            .flex_1()
            .min_w_0()
            .h_full()
            .overflow_y_scroll()
            .p(px(24.0))
            .flex()
            .flex_col()
            .gap(px(16.0))
            .child(
                Button::new(
                    "ssh-preview-navigator",
                    t!("agents.parity.toggle_sessions").to_string(),
                )
                .debug_selector(|| "agent-ssh-preview-navigator".into())
                .variant(ButtonVariant::Ghost)
                .size(ButtonSize::Sm)
                .justify_start()
                .icon(IconSource::from("panel-left"))
                .on_click(cx.listener(|this, _, _, cx| this.toggle_navigator(cx))),
            )
            .child(
                div()
                    .text_size(px(18.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(session.title.clone()),
            )
            .child(div().text_size(px(13.0)).child(format!(
                "{} · {}",
                label,
                session.provider.display_name()
            )))
            .child(
                div()
                    .text_size(px(12.0))
                    .whitespace_normal()
                    .child(session.workdir.clone()),
            )
            .child(
                div()
                    .text_size(px(11.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(format!(
                        "{} · {}",
                        timestamp(session.updated_at_ms as i64),
                        session.session_id
                    )),
            )
            .children(model.map(|value| div().text_size(px(12.0)).child(value)))
            .child(
                div()
                    .max_w(px(650.0))
                    .whitespace_normal()
                    .child(if connection_id.is_none() {
                        t!("agents.local_sessions.resume_help").to_string()
                    } else {
                        t!("agents.ssh_sessions.resume_help").to_string()
                    }),
            )
            .child(
                div()
                    .flex()
                    .gap(px(8.0))
                    .child(
                        Button::new(
                            "ssh-session-reconnect",
                            t!("agents.ssh_sessions.reconnect").to_string(),
                        )
                        .debug_selector(|| "agent-ssh-reconnect".into())
                        .icon(IconSource::from("plug"))
                        .loading(busy)
                        .disabled(busy)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if this.ssh_sessions.reconnecting.is_none() {
                                let request_id = Uuid::new_v4();
                                this.ssh_sessions.reconnecting = Some(request_id);
                                this.ssh_sessions.reconnect_error = None;
                                if let Some(connection_id) = connection_id {
                                    cx.emit(AgentConsoleEvent::ReconnectSshSession {
                                        connection_id,
                                        request_id,
                                        session: session.clone(),
                                    });
                                } else {
                                    cx.emit(AgentConsoleEvent::ReconnectLocalSession {
                                        request_id,
                                        session: session.clone(),
                                    });
                                }
                                cx.notify();
                            }
                        })),
                    )
                    .child(
                        Button::new(
                            "ssh-session-back",
                            t!("agents.ssh_sessions.back").to_string(),
                        )
                        .variant(ButtonVariant::Ghost)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.clear_ssh_session_preview();
                            this.sessions
                                .set_surface_visible(this.remote_sessions.surface_visible);
                            cx.notify();
                        })),
                    ),
            );
        if let Some(error) = &self.ssh_sessions.reconnect_error {
            details = details.child(
                div()
                    .whitespace_normal()
                    .text_color(ShellDeckColors::error())
                    .child(error.clone()),
            );
        }
        let mut root = div()
            .flex()
            .size_full()
            .min_w_0()
            .min_h_0()
            .bg(ShellDeckColors::bg_primary())
            .text_color(ShellDeckColors::text_primary());
        if self.tools.navigator_wide && self.tools.navigator_visible {
            root = root.child(self.render_navigator(window, cx));
        }
        root = root.child(details);
        if !self.tools.navigator_wide && self.tools.navigator_overlay {
            root = root.child(self.render_navigator_overlay(window, cx));
        }
        if let Some(dialog) = self.render_project_removal(cx) {
            root = root.child(dialog);
        }
        root.into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn history_page() -> shelldeck_core::agent_remote_history::RemoteHistoryPage {
        shelldeck_core::agent_remote_history::RemoteHistoryPage {
            provider: AgentProvider::Codex,
            session_id: "thread-original".into(),
            workdir: "/srv/project".into(),
            model: Some("gpt-5.1-codex".into()),
            snapshot: format!("v1:0:{}", "0".repeat(64)),
            total_entries: 0,
            start: 0,
            end: 0,
            entries: vec![],
            warnings: vec![],
        }
    }
    fn remote() -> RemoteAgentSession {
        RemoteAgentSession {
            provider: AgentProvider::Codex,
            session_id: "thread-original".into(),
            workdir: "/srv/project".into(),
            title: "Fix deployment".into(),
            updated_at_ms: 1,
            model: None,
        }
    }
    // SDTEST-2811 / SDUC-562, SDUC-563 — out-of-order scans cannot relabel an
    // inventory; exact history supplies the model omitted by metadata discovery,
    // and reconnect keeps the existing cutoff without starting another run.
    #[::core::prelude::v1::test]
    fn ssh_discovery_reconnect_preserves_native_identity_and_rejects_stale_callbacks() {
        let mut app = gpui::TestAppContext::single();
        let view = app.new(AgentConsoleView::new);
        view.update(&mut app,|view,cx|{
            view.library=AgentLibraryUi::new(cx);
            let id=Uuid::new_v4();
            view.set_connections(vec![AgentConnectionOption{id,label:"Server A".into(),host:"fixture.invalid".into()}],cx);
            let local=view.sessions.selected_id().unwrap();
            view.session_ui.get(&local).unwrap().prompt.update(cx,|state,_|state.content="keep my draft".into());
            view.scan_ssh_sessions(id,cx);
            let request=view.ssh_sessions.servers[&id].request;
            view.set_ssh_sessions(id,Uuid::new_v4(),Ok(RemoteAgentSessionInventory::default()),cx);
            assert!(view.ssh_sessions.servers[&id].loading);
            view.set_ssh_sessions(id,request,Ok(RemoteAgentSessionInventory{sessions:vec![remote()],warnings:vec![],truncated:false,next_cursor:None}),cx);
            assert!(!view.ssh_sessions.servers[&id].loading);
            view.scan_ssh_sessions(id,cx);
            let retry=view.ssh_sessions.servers[&id].request;
            view.set_ssh_sessions(id,request,Ok(RemoteAgentSessionInventory::default()),cx);
            assert_eq!(view.ssh_sessions.servers[&id].inventory.as_ref().unwrap().sessions.len(),1);
            view.set_ssh_sessions(id,retry,Err("offline".into()),cx);
            assert!(view.ssh_sessions.servers[&id].error.is_some());
            assert!(view.ssh_sessions.servers[&id].checked_at.is_some());
            let pending=Uuid::new_v4();
            view.ssh_sessions.preview=Some((Some(id),remote()));view.ssh_sessions.reconnecting=Some(pending);
            view.complete_ssh_reconnect(Uuid::new_v4(),Ok(history_page()),cx);
            assert_eq!(view.sessions.selected_id(),Some(local));
            view.complete_ssh_reconnect(pending,Ok(history_page()),cx);
            let imported=view.sessions.selected().unwrap();
            assert_eq!(imported.run_request("continue").resume_session.as_deref(),Some("thread-original"));
            assert!(matches!(imported.context.target,AgentTarget::Ssh{connection_id,..}if connection_id==id));
            assert_eq!(imported.context.workdir,"/srv/project");
            assert_eq!(imported.context.model.as_deref(), Some("gpt-5.1-codex"));
            assert_eq!(imported.remote_history.as_ref().unwrap().model.as_deref(), Some("gpt-5.1-codex"));
            assert!(!imported.status.is_active());
            let imported_id = imported.id;
            let snapshot = imported.remote_history.as_ref().unwrap().snapshot.clone();
            let count = view.sessions.sessions().len();
            // Discovery only sees an early title and commonly omits model.
            // That must not lose the saved transcript cutoff on reconnect.
            assert!(remote().model.is_none());
            assert_eq!(view.native_history_query(id, &remote()).snapshot, snapshot);
            let pending = Uuid::new_v4();
            view.ssh_sessions.preview = Some((Some(id), remote()));
            view.ssh_sessions.reconnecting = Some(pending);
            view.complete_ssh_reconnect(pending, Ok(history_page()), cx);
            assert_eq!(view.sessions.selected_id(), Some(imported_id));
            assert_eq!(view.sessions.sessions().len(), count);
            assert_eq!(view.sessions.selected().unwrap().remote_history.as_ref().unwrap().snapshot, snapshot);
            assert_eq!(view.session_ui.get(&local).unwrap().prompt.read(cx).content(),"keep my draft");
            assert!(view.foreground_runs.is_empty());
            view.reset_ssh_sessions(cx);
            view.set_ssh_sessions(id,retry,Ok(RemoteAgentSessionInventory::default()),cx);
            assert!(view.ssh_sessions.servers.is_empty());
        });
    }
    // SDTEST-2829 / SDUC-564 — selected server restores once, pages merge by
    // identity, offline reads retain known data, hiding invalidates late reads.
    #[::core::prelude::v1::test]
    fn saved_server_discovery_pages_preserve_data_and_hide_cancels_loading() {
        let mut app = gpui::TestAppContext::single();
        let view = app.new(AgentConsoleView::new);
        view.update(&mut app, |view, cx| {
            view.library = AgentLibraryUi::new(cx);
            let id = Uuid::new_v4();
            view.set_connections(
                vec![AgentConnectionOption {
                    id,
                    label: "Server".into(),
                    host: "fixture.invalid".into(),
                }],
                cx,
            );
            view.library.preferences.ssh_selected_server = Some(id);
            view.restore_ssh_preferences(cx);
            view.ensure_ssh_discovery(cx);
            let first = view.ssh_sessions.servers[&id].request;
            view.ensure_ssh_discovery(cx);
            assert_eq!(view.ssh_sessions.servers[&id].request, first);
            view.set_ssh_sessions(
                id,
                first,
                Ok(RemoteAgentSessionInventory {
                    sessions: vec![remote()],
                    next_cursor: Some("page2".into()),
                    ..Default::default()
                }),
                cx,
            );
            view.load_older_native_sessions(Some(id), cx);
            let page2 = view.ssh_sessions.servers[&id].request;
            assert_eq!(
                view.ssh_sessions.servers[&id].requested_cursor.as_deref(),
                Some("page2")
            );
            let mut changed = remote();
            changed.title = "Updated title".into();
            let mut older = remote();
            older.session_id = "older-thread".into();
            view.set_ssh_sessions(
                id,
                page2,
                Ok(RemoteAgentSessionInventory {
                    sessions: vec![changed, older],
                    next_cursor: Some("page3".into()),
                    ..Default::default()
                }),
                cx,
            );
            let inventory = view.ssh_sessions.servers[&id].inventory.as_ref().unwrap();
            assert_eq!(inventory.sessions.len(), 2);
            assert_eq!(inventory.sessions[0].title, "Updated title");
            view.load_older_native_sessions(Some(id), cx);
            let failed = view.ssh_sessions.servers[&id].request;
            view.set_ssh_sessions(id, failed, Err("offline".into()), cx);
            assert_eq!(
                view.ssh_sessions.servers[&id]
                    .inventory
                    .as_ref()
                    .unwrap()
                    .sessions
                    .len(),
                2
            );
            assert_eq!(
                view.ssh_sessions.servers[&id]
                    .inventory
                    .as_ref()
                    .unwrap()
                    .next_cursor
                    .as_deref(),
                Some("page3")
            );
            view.scan_ssh_sessions(id, cx);
            let late = view.ssh_sessions.servers[&id].request;
            view.hide_ssh_server(id, cx);
            view.set_ssh_sessions(id, late, Ok(RemoteAgentSessionInventory::default()), cx);
            view.ensure_ssh_discovery(cx);
            assert!(view.ssh_sessions.servers.is_empty());
            assert!(view.library.preferences.ssh_selected_server.is_none());
        });
    }
}
