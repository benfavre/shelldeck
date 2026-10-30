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
    collapsed: bool,
    pub(super) preview: Option<(Uuid, RemoteAgentSession)>,
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
        if self
            .ssh_sessions
            .preview
            .as_ref()
            .is_some_and(|(id, _)| !self.connections.iter().any(|c| c.id == *id))
        {
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
    fn load_older_ssh_sessions(&mut self, connection_id: Uuid, cx: &mut Context<Self>) {
        let cursor = self
            .ssh_sessions
            .servers
            .get(&connection_id)
            .and_then(|s| s.inventory.as_ref())
            .and_then(|i| i.next_cursor.clone());
        if cursor.is_some() {
            self.request_ssh_session_page(connection_id, cursor, cx);
        }
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
        let Some(server) = self.ssh_sessions.servers.get_mut(&connection_id) else {
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
        let Some(connection) = self.connections.iter().find(|c| c.id == connection_id) else {
            return;
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
            target: AgentTarget::Ssh {
                connection_id,
                connection_label: connection.label.clone(),
            },
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
            .flex()
            .flex_col()
            .gap(px(4.0))
            .px(px(6.0))
            .py(px(8.0))
            .border_b_1()
            .border_color(ShellDeckColors::border())
            .child(
                Button::new(
                    "ssh-sessions-toggle",
                    t!("agents.ssh_sessions.title").to_string(),
                )
                .variant(ButtonVariant::Ghost)
                .size(ButtonSize::Sm)
                .justify_start()
                .icon(IconSource::from(if expanded {
                    "chevron-down"
                } else {
                    "chevron-right"
                }))
                .disabled(!query.is_empty())
                .on_click(cx.listener(|this, _, _, cx| {
                    this.ssh_sessions.collapsed = !this.ssh_sessions.collapsed;
                    this.library.preferences.ssh_sessions_collapsed = this.ssh_sessions.collapsed;
                    this.schedule_library_save(cx);
                    cx.notify();
                })),
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
            let Some(server) = self.ssh_sessions.servers.get(&connection.id) else {
                continue;
            };
            let id = connection.id;
            if !query.is_empty()
                && server.inventory.as_ref().is_some_and(|i| {
                    !i.sessions
                        .iter()
                        .any(|session| matches_session(session, &connection.label, &query))
                })
                && !server.loading
            {
                continue;
            }
            let server_key = format!("ssh-server:{id}");
            let server_collapsed =
                project_group_collapsed(&self.library.preferences, &server_key, &query);
            let selected = self
                .ssh_sessions
                .preview
                .as_ref()
                .filter(|(connection, _)| *connection == id)
                .map(|(_, session)| session);
            let count = server.inventory.as_ref().map_or(0, |inventory| {
                inventory
                    .sessions
                    .iter()
                    .filter(|session| {
                        !selected.is_some_and(|current| {
                            current.provider == session.provider
                                && current.session_id == session.session_id
                                && current.workdir == session.workdir
                        })
                    })
                    .count()
            });
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
                                connection.label.clone(),
                                format!("{}\n{}", connection.label, connection.host),
                                (count, NavigatorState::Quiet),
                                cx,
                            )),
                    )
                    .child(
                        Button::new(SharedString::from(format!("ssh-refresh-{id}")), "")
                            .variant(ButtonVariant::Ghost)
                            .size(ButtonSize::Icon)
                            .w(px(24.0))
                            .h(px(24.0))
                            .flex_shrink_0()
                            .icon(IconSource::from("refresh-cw"))
                            .tooltip(t!("agents.ssh_sessions.refresh_server").to_string())
                            .disabled(server.loading)
                            .on_click(
                                cx.listener(move |this, _, _, cx| this.scan_ssh_sessions(id, cx)),
                            ),
                    )
                    .child(
                        Button::new(SharedString::from(format!("ssh-hide-{id}")), "")
                            .variant(ButtonVariant::Ghost)
                            .size(ButtonSize::Icon)
                            .w(px(24.0))
                            .h(px(24.0))
                            .flex_shrink_0()
                            .icon(IconSource::from("eye-off"))
                            .tooltip(t!("agents.ssh_sessions.hide_server").to_string())
                            .on_click(
                                cx.listener(move |this, _, _, cx| this.hide_ssh_server(id, cx)),
                            ),
                    ),
            );
            if !server_collapsed {
                let label = if server.loading {
                    t!("agents.ssh_sessions.connecting").to_string()
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
                continue;
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
                    list = list.child(
                        div()
                            .text_size(px(10.0))
                            .child(t!("agents.ssh_sessions.empty").to_string()),
                    );
                }
                if inventory.next_cursor.is_some() {
                    list = list.child(
                        Button::new(
                            SharedString::from(format!("ssh-older-{id}")),
                            t!("agents.ssh_sessions.load_older").to_string(),
                        )
                        .tooltip(t!("agents.ssh_sessions.load_older_hint").to_string())
                        .variant(ButtonVariant::Ghost)
                        .size(ButtonSize::Sm)
                        .disabled(server.loading)
                        .on_click(
                            cx.listener(move |this, _, _, cx| this.load_older_ssh_sessions(id, cx)),
                        ),
                    );
                }
            }
            let target = AgentTarget::Ssh {
                connection_id: id,
                connection_label: connection.label.clone(),
            };
            let mut folders: BTreeMap<&str, Vec<&RemoteAgentSession>> = BTreeMap::new();
            for session in &inventory.sessions {
                if matches_session(session, &connection.label, &query) {
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
                if server_collapsed && !sessions.iter().any(is_selected) {
                    continue;
                }
                let key = directory_group_key("ssh-folder", &target, path);
                let collapsed = server_collapsed
                    || project_group_collapsed(&self.library.preferences, &key, &query);
                let name = directory_name(&target, path);
                let duplicate = folders
                    .keys()
                    .any(|other| other != path && directory_name(&target, other) == name);
                let title = if duplicate { (*path).to_owned() } else { name };
                let hidden = sessions.len().saturating_sub(
                    sessions
                        .iter()
                        .filter(|session| is_selected(session))
                        .count(),
                );
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
                        format!("{}\n{}", connection.label, path),
                        (hidden, NavigatorState::Quiet),
                        cx,
                    ));
                }
                let mut rows = div().flex().flex_col().min_w_0().gap(px(2.0));
                let limit = server.visible_limits.get(&key).copied().unwrap_or(10);
                for session in sessions.iter().enumerate().filter_map(|(index, session)| {
                    let selected = is_selected(session);
                    ((!collapsed && index < limit) || selected).then_some(*session)
                }) {
                    let remote = session.clone();
                    let selected = is_selected(&session);
                    let selector = format!(
                        "agent-ssh-conversation-{id}-{:?}-{}",
                        session.provider, session.session_id
                    );
                    rows = rows.child(
                        Button::new(SharedString::from(selector.clone()), session.title.clone())
                            .debug_selector(move || selector.clone())
                            .variant(if selected {
                                ButtonVariant::Secondary
                            } else {
                                ButtonVariant::Ghost
                            })
                            .size(ButtonSize::Sm)
                            .justify_start()
                            .w_full()
                            .min_w_0()
                            .overflow_hidden()
                            .text_ellipsis()
                            .icon(IconSource::from(provider_icon(session.provider)))
                            .tooltip(format!(
                                "{}\n{}\n{}\n{}",
                                session.title,
                                session.provider.display_name(),
                                session.workdir,
                                timestamp(session.updated_at_ms as i64)
                            ))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.retain_draft_context(cx);
                                this.ssh_sessions.preview = Some((id, remote.clone()));
                                this.set_remote_session_active(false, cx);
                                this.ssh_sessions.reconnect_error = None;
                                this.ssh_sessions.reconnecting = None;
                                this.sessions.set_surface_visible(false);
                                cx.notify();
                            })),
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
                            if let Some(server) = this.ssh_sessions.servers.get_mut(&id) {
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
        }
        list.into_any_element()
    }
    pub(super) fn render_ssh_session_preview(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let Some((connection_id, session)) = self.ssh_sessions.preview.clone() else {
            return div().into_any_element();
        };
        let label = self
            .connections
            .iter()
            .find(|c| c.id == connection_id)
            .map(|c| format!("{} · {}", c.label, c.host))
            .unwrap_or_else(|| t!("agents.target.missing").to_string());
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
                    .child(t!("agents.ssh_sessions.resume_help").to_string()),
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
                                cx.emit(AgentConsoleEvent::ReconnectSshSession {
                                    connection_id,
                                    request_id,
                                    session: session.clone(),
                                });
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
            root = root.child(self.render_navigator(cx));
        }
        root = root.child(details);
        if !self.tools.navigator_wide && self.tools.navigator_overlay {
            root = root.child(self.render_navigator_overlay(cx));
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
            view.ssh_sessions.preview=Some((id,remote()));view.ssh_sessions.reconnecting=Some(pending);
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
            view.ssh_sessions.preview = Some((id, remote()));
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
            view.load_older_ssh_sessions(id, cx);
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
            view.load_older_ssh_sessions(id, cx);
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
