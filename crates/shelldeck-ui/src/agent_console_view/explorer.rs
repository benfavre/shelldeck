use super::*;

const EXPLORER_PAGE_SIZE: usize = 300;

pub(super) struct AgentExplorerUi {
    pub show_hidden: bool,
    visible_limit: usize,
}

impl Default for AgentExplorerUi {
    fn default() -> Self {
        Self {
            show_hidden: false,
            visible_limit: EXPLORER_PAGE_SIZE,
        }
    }
}

impl AgentExplorerUi {
    pub fn reset_context(&mut self) {
        self.visible_limit = EXPLORER_PAGE_SIZE;
    }
}

#[derive(Clone)]
struct ExplorerEntry {
    path: String,
    local_path: Option<PathBuf>,
    name: String,
    is_dir: bool,
    is_expanded: bool,
    depth: usize,
}

impl AgentConsoleView {
    pub(super) fn render_file_explorer(
        &mut self,
        session: &AgentSession,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let remote = matches!(session.context.target, AgentTarget::Ssh { .. });
        let session_id = session.id;
        let selected_path = self.selected_agent_file_path(cx);
        let current_path = if remote {
            self.inspector_remote_path
                .clone()
                .unwrap_or_else(|| session.context.workdir.clone())
        } else {
            self.inspector_local_browser.root().display().to_string()
        };
        let remote_root = self.inspector_remote_root.as_deref();
        let parent_path = remote_root.and_then(|root| explorer_remote_parent(root, &current_path));
        let remote_connection = match session.context.target {
            AgentTarget::Ssh { connection_id, .. } => Some(connection_id),
            AgentTarget::Local => None,
        };

        let refresh_path = current_path.clone();
        let toolbar = div()
            .flex()
            .items_center()
            .gap(px(6.0))
            .px(px(10.0))
            .pt(px(8.0))
            .pb(px(6.0))
            .child(
                Badge::new(if remote {
                    t!("agents.inspector.remote").to_string()
                } else {
                    t!("agents.inspector.local").to_string()
                })
                .variant(BadgeVariant::Secondary),
            )
            .child(div().flex_1().min_w_0())
            .when_some(parent_path, |row, parent| {
                row.child(
                    Button::new("agent-explorer-parent", "")
                        .variant(ButtonVariant::Ghost)
                        .size(ButtonSize::Icon)
                        .w(px(28.0))
                        .h(px(28.0))
                        .icon(IconSource::from("arrow-up"))
                        .tooltip(t!("agents.explorer.parent").to_string())
                        .disabled(self.inspector_remote_loading)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if let Some(connection_id) = remote_connection {
                                this.request_remote_directory(
                                    session_id,
                                    connection_id,
                                    parent.clone(),
                                    cx,
                                );
                            }
                        })),
                )
            })
            .child(
                Button::new("agent-explorer-hidden", "")
                    .variant(ButtonVariant::Ghost)
                    .size(ButtonSize::Icon)
                    .w(px(28.0))
                    .h(px(28.0))
                    .selected(self.explorer.show_hidden)
                    .icon(IconSource::from(if self.explorer.show_hidden {
                        "eye"
                    } else {
                        "eye-off"
                    }))
                    .tooltip(if self.explorer.show_hidden {
                        t!("agents.explorer.hide_hidden").to_string()
                    } else {
                        t!("agents.explorer.show_hidden").to_string()
                    })
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.explorer.show_hidden = !this.explorer.show_hidden;
                        cx.notify();
                    })),
            )
            .child(
                Button::new("agent-explorer-refresh", "")
                    .variant(ButtonVariant::Ghost)
                    .size(ButtonSize::Icon)
                    .w(px(28.0))
                    .h(px(28.0))
                    .icon(IconSource::from("refresh-cw"))
                    .tooltip(t!("agents.explorer.refresh").to_string())
                    .disabled(remote && self.inspector_remote_loading)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(connection_id) = remote_connection {
                            this.request_remote_directory(
                                session_id,
                                connection_id,
                                refresh_path.clone(),
                                cx,
                            );
                        } else {
                            // The shared local tree reads its expanded directories afresh.
                            cx.notify();
                        }
                    })),
            );

        let mut location = div()
            .w_full()
            .flex_shrink_0()
            .flex()
            .flex_col()
            .min_w_0()
            .gap(px(4.0))
            .px(px(10.0))
            .pb(px(8.0))
            .border_b_1()
            .border_color(ShellDeckColors::border());
        if let (Some(root), Some(connection_id)) = (remote_root, remote_connection) {
            let mut crumbs = div()
                .id("agent-explorer-breadcrumbs")
                .flex()
                .items_center()
                .gap(px(6.0))
                .min_w_0()
                .overflow_x_scroll();
            for (index, (label, path)) in explorer_remote_breadcrumbs(root, &current_path)
                .into_iter()
                .enumerate()
            {
                if index > 0 {
                    crumbs = crumbs.child(lucide_icon(
                        "chevron-right",
                        10.0,
                        ShellDeckColors::text_muted(),
                    ));
                }
                let selected = path == current_path;
                let destination = path.clone();
                crumbs = crumbs.child(
                    Button::new(("agent-explorer-crumb", index), label)
                        .variant(ButtonVariant::Ghost)
                        .size(ButtonSize::Sm)
                        .h(px(26.0))
                        .max_w(px(150.0))
                        .px(px(6.0))
                        .tooltip(path)
                        .selected(selected)
                        .disabled(self.inspector_remote_loading)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.request_remote_directory(
                                session_id,
                                connection_id,
                                destination.clone(),
                                cx,
                            );
                        })),
                );
            }
            location = location.child(crumbs);
        }
        let location_name = if remote {
            current_path
                .rsplit('/')
                .find(|part| !part.is_empty())
                .unwrap_or("/")
                .to_string()
        } else {
            std::path::Path::new(&current_path)
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned()
        };
        let copy_path = current_path.clone();
        location = location.child(
            Button::new("agent-explorer-location", location_name)
                .variant(ButtonVariant::Ghost)
                .size(ButtonSize::Sm)
                .w_full()
                .min_w_0()
                .justify_start()
                .icon(IconSource::from("copy"))
                .tooltip(t!("agents.explorer.copy_path", path = current_path.clone()).to_string())
                .on_click(move |_, _, cx| {
                    cx.write_to_clipboard(ClipboardItem::new_string(copy_path.clone()))
                }),
        );

        let mut list = div()
            .id("agent-inspector-list")
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .min_w_0()
            .overflow_y_scroll()
            .px(px(7.0))
            .py(px(6.0))
            .gap(px(2.0));
        let (entries, listing_truncated) = if remote {
            (
                self.inspector_remote_entries
                    .iter()
                    .filter(|entry| explorer_entry_visible(&entry.name, self.explorer.show_hidden))
                    .filter(|entry| {
                        remote_root.is_some_and(|root| explorer_remote_within(root, &entry.path))
                    })
                    .map(|entry| ExplorerEntry {
                        path: entry.path.clone(),
                        local_path: None,
                        name: entry.name.clone(),
                        is_dir: entry.is_dir,
                        is_expanded: false,
                        depth: 0,
                    })
                    .collect::<Vec<_>>(),
                self.inspector_remote_truncated,
            )
        } else {
            let listing = self
                .inspector_local_browser
                .visible_listing_with_hidden(self.explorer.show_hidden);
            (
                listing
                    .entries
                    .into_iter()
                    .map(|entry| ExplorerEntry {
                        path: entry.path.display().to_string(),
                        local_path: Some(entry.path),
                        name: entry.name,
                        is_dir: entry.is_dir,
                        is_expanded: entry.is_expanded,
                        depth: entry.depth,
                    })
                    .collect::<Vec<_>>(),
                listing.truncated,
            )
        };
        let visible_count = entries.len();
        if remote && self.inspector_remote_loading {
            list = list.child(inspector_empty_state(
                "refresh-cw",
                t!("agents.inspector.remote_loading").to_string(),
            ));
        } else if remote && self.inspector_remote_error.is_some() {
            list = list.child(inspector_empty_state(
                "circle-alert",
                self.inspector_remote_error.clone().unwrap_or_default(),
            ));
        } else if remote && self.inspector_remote_root.is_none() {
            let load_path = current_path;
            list = list
                .child(inspector_empty_state(
                    "server",
                    t!("agents.explorer.unloaded").to_string(),
                ))
                .child(
                    Button::new(
                        "agent-explorer-load",
                        t!("agents.inspector.remote_load").to_string(),
                    )
                    .variant(ButtonVariant::Secondary)
                    .size(ButtonSize::Sm)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(connection_id) = remote_connection {
                            this.request_remote_directory(
                                session_id,
                                connection_id,
                                load_path.clone(),
                                cx,
                            );
                        }
                    })),
                );
        } else if entries.is_empty() {
            list = list.child(inspector_empty_state(
                "file-text",
                t!("agents.explorer.empty").to_string(),
            ));
        } else {
            for entry in entries.into_iter().take(self.explorer.visible_limit) {
                let selected =
                    explorer_file_selected(selected_path.as_deref(), &entry.path, entry.is_dir);
                let keyboard_entry = entry.clone();
                let click_entry = entry.clone();
                let icon = if entry.is_dir {
                    if entry.is_expanded {
                        "chevron-down"
                    } else {
                        "chevron-right"
                    }
                } else {
                    "file-text"
                };
                list = list.child(
                    Button::new(
                        SharedString::from(format!("agent-explorer-entry-{}", entry.path)),
                        entry.name,
                    )
                    .variant(ButtonVariant::Ghost)
                    .size(ButtonSize::Sm)
                    .w_full()
                    .h(px(30.0))
                    .min_w_0()
                    .justify_start()
                    .pl(px(8.0 + entry.depth as f32 * 14.0))
                    .pr(px(8.0))
                    .selected(selected)
                    .icon(IconSource::from(icon))
                    .tooltip(entry.path)
                    .on_key_down(cx.listener(move |this, event: &KeyDownEvent, _, cx| {
                        if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                            cx.stop_propagation();
                            this.activate_explorer_entry(session_id, keyboard_entry.clone(), cx);
                        }
                    }))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.activate_explorer_entry(session_id, click_entry.clone(), cx);
                    })),
                );
            }
            if visible_count > self.explorer.visible_limit {
                list = list.child(
                    Button::new(
                        "agent-explorer-more",
                        t!(
                            "agents.explorer.show_more",
                            count = visible_count - self.explorer.visible_limit
                        )
                        .to_string(),
                    )
                    .variant(ButtonVariant::Ghost)
                    .size(ButtonSize::Sm)
                    .icon(IconSource::from("chevron-down"))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.explorer.visible_limit = this
                            .explorer
                            .visible_limit
                            .saturating_add(EXPLORER_PAGE_SIZE);
                        cx.notify();
                    })),
                );
            }
        }
        let mut panel = div()
            .w_full()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .min_w_0()
            .child(toolbar)
            .child(location)
            .child(list);
        if listing_truncated {
            panel = panel.child(
                div()
                    .flex_shrink_0()
                    .px(px(10.0))
                    .py(px(6.0))
                    .text_size(px(10.0))
                    .text_color(ShellDeckColors::warning())
                    .child(t!("agents.explorer.truncated").to_string()),
            );
        }
        panel
            .child(
                div()
                    .flex_shrink_0()
                    .px(px(10.0))
                    .py(px(8.0))
                    .border_t_1()
                    .border_color(ShellDeckColors::border())
                    .text_size(px(10.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(t!("agents.explorer.count", count = visible_count).to_string()),
            )
            .into_any_element()
    }

    fn activate_explorer_entry(
        &mut self,
        session_id: Uuid,
        entry: ExplorerEntry,
        cx: &mut Context<Self>,
    ) {
        if self.sessions.selected_id() != Some(session_id) {
            return;
        }
        match self.displayed_context(cx).target {
            AgentTarget::Local => {
                let Some(path) = entry.local_path else { return };
                if entry.is_dir {
                    self.inspector_local_browser.toggle_dir(&path);
                } else {
                    self.open_local_agent_preview(path, cx);
                }
            }
            AgentTarget::Ssh { connection_id, .. } => {
                if !self
                    .inspector_remote_root
                    .as_deref()
                    .is_some_and(|root| explorer_remote_within(root, &entry.path))
                {
                    return;
                }
                if entry.is_dir {
                    self.request_remote_directory(session_id, connection_id, entry.path, cx);
                } else {
                    self.open_remote_agent_preview(session_id, connection_id, entry.path, cx);
                }
            }
        }
        cx.notify();
    }
}

fn explorer_entry_visible(name: &str, show_hidden: bool) -> bool {
    !matches!(name, "." | ".." | ".git") && (show_hidden || !name.starts_with('.'))
}

fn explorer_file_selected(active_path: Option<&str>, path: &str, is_dir: bool) -> bool {
    !is_dir && active_path == Some(path)
}

fn explorer_remote_within(root: &str, path: &str) -> bool {
    use shelldeck_core::config::workspace_catalog::RemotePosixPath;
    if RemotePosixPath::new(root.to_string()).is_err()
        || RemotePosixPath::new(path.to_string()).is_err()
    {
        return false;
    }
    let root = root.trim_end_matches('/');
    path == root || (path.starts_with(root) && path.as_bytes().get(root.len()) == Some(&b'/'))
}

fn explorer_remote_parent(root: &str, path: &str) -> Option<String> {
    if !explorer_remote_within(root, path)
        || path.trim_end_matches('/') == root.trim_end_matches('/')
    {
        return None;
    }
    let parent = path.trim_end_matches('/').rsplit_once('/')?.0;
    let parent = if parent.is_empty() { "/" } else { parent };
    explorer_remote_within(root, parent).then(|| parent.to_string())
}

fn explorer_remote_breadcrumbs(root: &str, path: &str) -> Vec<(String, String)> {
    if !explorer_remote_within(root, path) {
        return Vec::new();
    }
    let root = root.trim_end_matches('/');
    let label = root
        .rsplit('/')
        .next()
        .filter(|label| !label.is_empty())
        .unwrap_or("/");
    let mut crumbs = vec![(
        label.to_string(),
        if root.is_empty() {
            "/".to_string()
        } else {
            root.to_string()
        },
    )];
    let mut current = root.to_string();
    for part in path[root.len()..]
        .split('/')
        .filter(|part| !part.is_empty())
    {
        current.push('/');
        current.push_str(part);
        crumbs.push((part.to_string(), current.clone()));
    }
    crumbs
}

#[cfg(test)]
mod tests {
    use super::*;

    // SDTEST-2500 — SDUC-551
    #[::core::prelude::v1::test]
    fn sdtest_2500_remote_explorer_parent_stops_at_the_session_root() {
        assert_eq!(
            explorer_remote_parent("/srv/project", "/srv/project/src/deep"),
            Some("/srv/project/src".into())
        );
        assert_eq!(
            explorer_remote_parent("/srv/project", "/srv/project/src"),
            Some("/srv/project".into())
        );
        assert_eq!(
            explorer_remote_parent("/srv/project/", "/srv/project/"),
            None
        );
        assert_eq!(
            explorer_remote_parent("/srv/project", "/srv/project-other/src"),
            None
        );
        assert_eq!(
            explorer_remote_parent("/srv/project", "/srv/project/../secret"),
            None
        );
        assert_eq!(explorer_remote_parent("/", "/src"), Some("/".into()));
    }

    // SDTEST-2501 — SDUC-551
    #[::core::prelude::v1::test]
    fn sdtest_2501_remote_explorer_breadcrumbs_preserve_exact_paths_and_root() {
        assert_eq!(
            explorer_remote_breadcrumbs("/srv/a project", "/srv/a project/src/café"),
            vec![
                ("a project".into(), "/srv/a project".into()),
                ("src".into(), "/srv/a project/src".into()),
                ("café".into(), "/srv/a project/src/café".into()),
            ]
        );
        assert_eq!(
            explorer_remote_breadcrumbs("/", "/etc"),
            vec![("/".into(), "/".into()), ("etc".into(), "/etc".into())]
        );
        assert!(explorer_remote_breadcrumbs("/srv/a", "/srv/ab/file").is_empty());
        assert!(explorer_remote_breadcrumbs("~", "/home/user").is_empty());
    }

    // SDTEST-2502 — SDUC-551
    #[::core::prelude::v1::test]
    fn sdtest_2502_explorer_hidden_toggle_exposes_project_configuration_without_git_internals() {
        assert!(!explorer_entry_visible(".env", false));
        assert!(explorer_entry_visible(".env", true));
        assert!(explorer_entry_visible(".agents", true));
        assert!(explorer_entry_visible("Cargo.toml", false));
        for name in [".", "..", ".git"] {
            assert!(!explorer_entry_visible(name, true));
        }
    }

    // SDTEST-2503 — SDUC-551
    #[::core::prelude::v1::test]
    fn sdtest_2503_explorer_highlights_only_the_active_editor_file() {
        let active = Some("/srv/project/main.rs");
        assert!(explorer_file_selected(
            active,
            "/srv/project/main.rs",
            false
        ));
        assert!(!explorer_file_selected(
            active,
            "/srv/project/main.rs.bak",
            false
        ));
        assert!(!explorer_file_selected(
            active,
            "/srv/project/main.rs",
            true
        ));
        assert!(!explorer_file_selected(None, "/srv/project/main.rs", false));
    }
}
