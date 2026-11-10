//! Project folders for conversations that have no catalog checkout.
//! Group identities are presentation state; execution always uses the session.
use super::navigator::{
    collapsed_session_is_visible, collapsed_session_summary, navigator_session_matches_query,
    navigator_worktree, project_group_collapsed, session_group_summary, toggle_project_group,
    NavigatorState,
};
use super::*;
use std::collections::BTreeMap;

// Shared geometry keeps embedded section controls aligned with complete header rows.
fn navigation_header_layout(id: impl Into<ElementId>, group: SharedString) -> Stateful<Div> {
    div()
        .id(id)
        .group(group)
        .flex()
        .items_center()
        .min_w_0()
        .gap(px(6.0))
        .h(px(34.0))
        .px(px(4.0))
        .rounded(px(8.0))
}

pub(super) fn navigation_header_surface(
    id: impl Into<ElementId>,
    group: SharedString,
) -> Stateful<Div> {
    navigation_header_layout(id, group).hover(|style| style.bg(ShellDeckColors::hover_bg()))
}

pub(super) fn navigator_pin(session: &AgentSession) -> bool {
    session.pinned && !session.archived
}

pub(super) fn navigation_children(key: &str, children: AnyElement) -> AnyElement {
    // Layout only: adabraka Tree owns selection/expansion, while these retained
    // conversation controls already own both through the canonical session model.
    div()
        .debug_selector(|| format!("agent-tree-children-{key}"))
        .min_w_0()
        .flex_shrink_0()
        .ml(px(16.0))
        .pl(px(14.0))
        .child(children)
        .into_any_element()
}

pub(super) fn directory_group_key(namespace: &str, target: &AgentTarget, path: &str) -> String {
    let authority = match target {
        AgentTarget::Local => "local".to_owned(),
        AgentTarget::Ssh { connection_id, .. } => connection_id.to_string(),
    };
    // Hash only the bounded preference key. Group membership and execution use
    // the complete path and target, never this display-state identifier.
    format!("{namespace}:{authority}:{:016x}", stable_string_key(path))
}

pub(super) fn directory_name(target: &AgentTarget, path: &str) -> String {
    let name = match target {
        AgentTarget::Local => std::path::Path::new(path)
            .file_name()
            .and_then(|name| name.to_str()),
        AgentTarget::Ssh { .. } => path.trim_end_matches('/').rsplit('/').next(),
    };
    name.filter(|name| !name.is_empty())
        .unwrap_or(path)
        .to_owned()
}

struct ConversationFolder<'a> {
    target: &'a AgentTarget,
    path: &'a str,
    sessions: Vec<&'a AgentSession>,
}

fn conversation_folders<'a>(
    sessions: impl Iterator<Item = &'a AgentSession>,
    catalog: &[AgentProjectGroup],
) -> Vec<ConversationFolder<'a>> {
    let mut folders = BTreeMap::new();
    for session in sessions {
        if navigator_pin(session) || navigator_worktree(session, catalog).is_some() {
            continue;
        }
        let authority = match session.context.target {
            AgentTarget::Local => None,
            AgentTarget::Ssh { connection_id, .. } => Some(connection_id),
        };
        let key = (authority, session.context.workdir.as_str());
        folders
            .entry(key)
            .or_insert_with(|| ConversationFolder {
                target: &session.context.target,
                path: &session.context.workdir,
                sessions: Vec::new(),
            })
            .sessions
            .push(session);
    }
    // Stream timestamps never move a folder or change the order of its rows.
    folders.into_values().collect()
}

impl AgentConsoleView {
    pub(super) fn navigator_group_keys(&self) -> Vec<String> {
        let mut keys = self
            .project_groups
            .iter()
            .map(|group| group.id.clone())
            .collect::<Vec<_>>();
        keys.extend(
            self.project_groups
                .iter()
                .filter(|group| group.worktrees.len() > 1)
                .flat_map(|group| &group.worktrees)
                .map(|tree| format!("worktree-{}", tree.id)),
        );
        keys.push("pinned-sessions".into());
        keys.extend(
            conversation_folders(self.sessions.sessions().iter(), &self.project_groups)
                .into_iter()
                .map(|folder| directory_group_key("chat-folder", folder.target, folder.path)),
        );
        keys
    }

    pub(super) fn migrate_legacy_folder_collapse(&mut self) {
        if !self
            .library
            .preferences
            .collapsed_project_ids
            .iter()
            .any(|id| id == "unassigned-sessions")
        {
            return;
        }
        self.library
            .preferences
            .collapsed_project_ids
            .retain(|id| id != "unassigned-sessions");
        for folder in conversation_folders(self.sessions.sessions().iter(), &self.project_groups) {
            let key = directory_group_key("chat-folder", folder.target, folder.path);
            if !self
                .library
                .preferences
                .collapsed_project_ids
                .contains(&key)
            {
                self.library.preferences.toggle_project_collapsed(&key);
            }
        }
    }

    pub(super) fn render_project_session_rows(
        &self,
        worktree: &AgentWorktreeOption,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        self.render_session_rows_matching(
            |session| {
                !navigator_pin(session)
                    && navigator_worktree(session, &self.project_groups)
                        .is_some_and(|owner| owner.id == worktree.id)
            },
            cx,
        )
    }

    pub(super) fn navigator_selected_session(&self) -> Option<Uuid> {
        if self.remote_sessions.active || self.ssh_sessions.preview.is_some() {
            None
        } else {
            self.sessions.selected_id()
        }
    }

    // Match project headers: one 24px icon slot, 6px gap and a label with 4px padding.
    pub(super) fn render_navigation_section_toggle(
        &self,
        id: &'static str,
        label: String,
        collapsed: bool,
        searching: bool,
        icon: &'static str,
        cx: &Context<Self>,
        toggle: fn(&mut Self, &mut Context<Self>),
    ) -> Stateful<Div> {
        let group: SharedString = format!("{id}-hover").into();
        navigation_header_layout(SharedString::from(format!("{id}-header")), group.clone())
            .child(
                Button::new(SharedString::from(format!("{id}-chevron")), "")
                    .variant(ButtonVariant::Ghost)
                    .hover_background(gpui::transparent_black())
                    .size(ButtonSize::Icon)
                    .p_0()
                    .w(px(24.0))
                    .h(px(24.0))
                    .flex_shrink_0()
                    .icon(if collapsed {
                        "chevron-right"
                    } else {
                        "chevron-down"
                    })
                    .icon_visible_on_hover(group)
                    .icon_when_not_hovered(icon)
                    .disabled(searching)
                    .on_click(cx.listener(move |this, _, _, cx| toggle(this, cx))),
            )
            .child(
                Button::new(id, label)
                    .debug_selector(move || id.into())
                    .variant(ButtonVariant::Ghost)
                    .hover_background(gpui::transparent_black())
                    .size(ButtonSize::Sm)
                    .h(px(30.0))
                    .px(px(4.0))
                    .flex_1()
                    .min_w_0()
                    .justify_start()
                    .overflow_hidden()
                    .text_ellipsis()
                    .disabled(searching)
                    .on_click(cx.listener(move |this, _, _, cx| toggle(this, cx))),
            )
    }

    pub(super) fn render_navigation_group_header(
        &self,
        key: &str,
        label: String,
        tooltip: String,
        hidden: (usize, NavigatorState),
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let query = self.navigator_search.read(cx).content().to_string();
        let collapsed = project_group_collapsed(&self.library.preferences, key, &query);
        let key = key.to_owned();
        let summary_key = key.clone();
        let selector = format!("agent-folder-toggle-{key}");
        let hover_group: SharedString = format!("agent-group-hover-{key}").into();
        let mut header = navigation_header_surface(
            SharedString::from(format!("agent-group-header-{key}")),
            hover_group.clone(),
        )
        .w_full()
        .child(
            Button::new(SharedString::from(selector.clone()), label)
                .debug_selector(move || selector.clone())
                .variant(ButtonVariant::Ghost)
                .hover_background(gpui::transparent_black())
                .icon_visible_on_hover(hover_group)
                .icon_when_not_hovered(if key == "pinned-sessions" {
                    "pin"
                } else if key.starts_with("ssh-server:") {
                    "server"
                } else if key.starts_with("worktree-") {
                    "git-branch"
                } else if key.starts_with("chat-folder:") && !key.starts_with("chat-folder:local:")
                {
                    "globe"
                } else {
                    "folder"
                })
                .size(ButtonSize::Sm)
                .h(px(30.0))
                .px(px(4.0))
                .justify_start()
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .text_ellipsis()
                .icon(IconSource::from(if collapsed {
                    "chevron-right"
                } else {
                    "chevron-down"
                }))
                .tooltip(tooltip)
                .disabled(!query.trim().is_empty())
                .on_click(cx.listener(move |this, _, _, cx| {
                    let query = this.navigator_search.read(cx).content().to_string();
                    if toggle_project_group(&mut this.library.preferences, &key, &query) {
                        this.schedule_library_save(cx);
                        cx.notify();
                    }
                })),
        );
        if collapsed && hidden.0 > 0 {
            header = header.child(collapsed_session_summary(&summary_key, hidden.0, hidden.1));
        }
        header.into_any_element()
    }

    fn render_conversation_folder(
        &self,
        key: &str,
        label: String,
        tooltip: String,
        sessions: &[&AgentSession],
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let query = self.navigator_search.read(cx).content().to_string();
        let collapsed = project_group_collapsed(&self.library.preferences, key, &query);
        let selected = self.navigator_selected_session();
        let hidden = session_group_summary(
            sessions
                .iter()
                .copied()
                .filter(|session| !collapsed_session_is_visible(session, selected)),
        );
        let ids = sessions
            .iter()
            .map(|session| session.id)
            .collect::<HashSet<_>>();
        div()
            .debug_selector(|| format!("agent-folder-{key}"))
            .w_full()
            .flex()
            .flex_col()
            .flex_shrink_0()
            .min_w_0()
            .gap(px(2.0))
            .pt(px(4.0))
            .child(self.render_navigation_group_header(key, label, tooltip, hidden, cx))
            .child(navigation_children(
                key,
                self.render_session_rows_matching(
                    |session| {
                        ids.contains(&session.id)
                            && (!collapsed || collapsed_session_is_visible(session, selected))
                    },
                    cx,
                ),
            ))
            .into_any_element()
    }

    pub(super) fn render_pinned_conversations(&self, cx: &mut Context<Self>) -> AnyElement {
        let query = self.navigator_search.read(cx).content().to_string();
        let sessions = self
            .sessions
            .library_sessions(self.library.show_archived())
            .into_iter()
            .filter(|session| {
                navigator_pin(session)
                    && navigator_session_matches_query(session, &self.project_groups, &query)
            })
            .collect::<Vec<_>>();
        if sessions.is_empty() {
            return div().into_any_element();
        }
        self.render_conversation_folder(
            "pinned-sessions",
            t!("agents.navigator.pinned").to_string(),
            t!("agents.navigator.pinned").to_string(),
            &sessions,
            cx,
        )
    }

    pub(super) fn render_uncatalogued_folders(&self, cx: &mut Context<Self>) -> AnyElement {
        let query = self.navigator_search.read(cx).content().to_string();
        let folders = conversation_folders(
            self.sessions
                .library_sessions(self.library.show_archived())
                .into_iter()
                .filter(|session| {
                    navigator_session_matches_query(session, &self.project_groups, &query)
                }),
            &self.project_groups,
        );
        let mut list = div().flex().flex_col().min_w_0().gap(px(4.0));
        for folder in &folders {
            let name = directory_name(folder.target, folder.path);
            let duplicate = folders.iter().any(|other| {
                other.path != folder.path
                    && same_target_authority(other.target, folder.target)
                    && directory_name(other.target, other.path) == name
            });
            let title = if duplicate {
                folder.path.to_owned()
            } else {
                name
            };
            let tooltip = format!("{}\n{}", target_label(folder.target), folder.path);
            list = list.child(self.render_conversation_folder(
                &directory_group_key("chat-folder", folder.target, folder.path),
                title,
                tooltip,
                &folder.sessions,
                cx,
            ));
        }
        list.into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // SDTEST-2859 / SDUC-562, SDUC-564: target identity owns a folder, not its label or provider.
    #[::core::prelude::v1::test]
    fn sdtest_2859_folder_groups_keep_machine_path_and_catalog_identity() {
        let server = |id| AgentTarget::Ssh {
            connection_id: Uuid::from_u128(id),
            connection_label: "Same display name".into(),
        };
        let mut sessions = AgentSessionCollection::default();
        let mut clock = 0;
        let mut create = |target, path: &str, provider| {
            clock += 1;
            sessions
                .create(
                    path,
                    AgentExecutionContext {
                        target,
                        workdir: path.into(),
                        provider,
                        access: AgentAccessMode::ReadOnly,
                        model: None,
                    },
                    clock,
                )
                .unwrap()
        };
        let first = create(server(1), "/srv/customer/api", AgentProvider::Codex);
        let second = create(server(1), "/srv/customer/api", AgentProvider::Claude);
        let other_server = create(server(2), "/srv/customer/api", AgentProvider::Jcode);
        let other_path = create(server(1), "/srv/other/api", AgentProvider::Codex);
        let local = create(
            AgentTarget::Local,
            "/srv/customer/api",
            AgentProvider::Codex,
        );
        let pinned = create(server(1), "/srv/customer/api", AgentProvider::Codex);
        sessions.set_pinned(pinned, true, 2).unwrap();
        let folder_ids = |sessions: &AgentSessionCollection, catalog: &[AgentProjectGroup]| {
            conversation_folders(sessions.library_sessions(false).into_iter(), catalog)
                .iter()
                .map(|folder| {
                    folder
                        .sessions
                        .iter()
                        .map(|session| session.id)
                        .collect::<Vec<_>>()
                })
                .collect::<Vec<_>>()
        };
        let before = folder_ids(&sessions, &[]);
        assert_eq!(before.len(), 4);
        assert!(before.contains(&vec![second, first]));
        for id in [other_server, other_path, local] {
            assert!(before.contains(&vec![id]));
        }
        sessions
            .begin_run(first, "Stream without moving the folder", 3)
            .unwrap();
        sessions
            .apply_stream_event(first, AgentStreamEvent::Text("New output".into()), 4)
            .unwrap();
        assert_eq!(folder_ids(&sessions, &[]), before);
        let catalog = vec![AgentProjectGroup {
            id: "registered".into(),
            label: "Customer API".into(),
            worktrees: vec![AgentWorktreeOption {
                id: "checkout".into(),
                label: "Customer API".into(),
                path: "/srv/customer".into(),
                target: server(1),
                branch: None,
                is_primary: true,
            }],
        }];
        let owned = folder_ids(&sessions, &catalog);
        assert_eq!(owned.len(), 3);
        assert!(!owned
            .iter()
            .flatten()
            .any(|id| *id == first || *id == second || *id == pinned));
        assert_ne!(
            directory_group_key("chat-folder", &server(1), "/srv/customer/api"),
            directory_group_key("chat-folder", &server(2), "/srv/customer/api")
        );
        assert_ne!(
            directory_group_key("chat-folder", &server(1), "/srv/customer/api"),
            directory_group_key("chat-folder", &server(1), "/srv/other/api")
        );
        assert_eq!(directory_name(&server(1), "/srv/customer/api/"), "api");
        assert_eq!(directory_name(&server(1), "/"), "/");
    }
}
