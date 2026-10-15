use super::*;

impl AgentConsoleView {
    pub(super) fn render_inspector(
        &mut self,
        session: &AgentSession,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let remote = matches!(session.context.target, AgentTarget::Ssh { .. });
        let files = inspector_files(session);
        let changed_count = files.iter().filter(|file| file.diff.is_some()).count();
        let files_selected = self.inspector_tab == AgentInspectorTab::Files;
        let changes_selected = self.inspector_tab == AgentInspectorTab::Changes;
        let remote_parent = remote
            .then(|| {
                self.inspector_remote_path
                    .as_deref()
                    .unwrap_or(&session.context.workdir)
            })
            .and_then(remote_parent_path);

        let tabs = div()
            .h(px(42.0))
            .flex()
            .items_center()
            .gap(px(4.0))
            .px(px(8.0))
            .border_b_1()
            .border_color(ShellDeckColors::border())
            .child(
                Button::new(
                    "agent-inspector-files",
                    t!("agents.inspector.files").to_string(),
                )
                .variant(if files_selected {
                    ButtonVariant::Secondary
                } else {
                    ButtonVariant::Ghost
                })
                .size(ButtonSize::Sm)
                .icon(IconSource::from("file-text"))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.inspector_tab = AgentInspectorTab::Files;
                    cx.notify();
                })),
            )
            .child(
                Button::new(
                    "agent-inspector-changes",
                    t!("agents.inspector.changes").to_string(),
                )
                .variant(if changes_selected {
                    ButtonVariant::Secondary
                } else {
                    ButtonVariant::Ghost
                })
                .size(ButtonSize::Sm)
                .icon(IconSource::from("git-branch"))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.inspector_tab = AgentInspectorTab::Changes;
                    cx.notify();
                })),
            )
            .child(div().flex_1())
            .child(
                Button::new("agent-files-close", "")
                    .variant(ButtonVariant::Ghost)
                    .size(ButtonSize::Icon)
                    .icon(IconSource::from("x"))
                    .tooltip(t!("agents.parity.close_panel").to_string())
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.tools.panel_visible = false;
                        cx.notify();
                    })),
            );

        let mut list = div()
            .id("agent-inspector-list")
            .flex()
            .flex_col()
            .flex_1()
            .min_h(px(0.0))
            .overflow_y_scroll()
            .px(px(7.0))
            .py(px(6.0))
            .gap(px(4.0));
        if files_selected && !remote {
            let entries = self.inspector_local_browser.visible_entries();
            if entries.is_empty() {
                list = list.child(inspector_empty_state(
                    "file-text",
                    t!("agents.inspector.empty_files").to_string(),
                ));
            } else {
                for entry in entries {
                    let path = entry.path.clone();
                    let event_path = path.clone();
                    let is_dir = entry.is_dir;
                    let session_id = session.id;
                    let icon = if is_dir {
                        if entry.is_expanded {
                            "chevron-down"
                        } else {
                            "chevron-right"
                        }
                    } else {
                        "file-text"
                    };
                    list = list.child(
                        div()
                            .id(SharedString::from(format!("agent-file-{}", path.display())))
                            .h(px(25.0))
                            .w_full()
                            .flex()
                            .items_center()
                            .gap(px(5.0))
                            .pl(px(6.0 + entry.depth as f32 * 12.0))
                            .pr(px(6.0))
                            .rounded(px(4.0))
                            .cursor_pointer()
                            .hover(|style| style.bg(ShellDeckColors::hover_bg()))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if is_dir {
                                    this.inspector_local_browser.toggle_dir(&path);
                                    cx.notify();
                                } else {
                                    cx.emit(AgentConsoleEvent::OpenLocalFile {
                                        session_id,
                                        path: event_path.clone(),
                                    });
                                }
                            }))
                            .child(lucide_icon(icon, 11.0, ShellDeckColors::text_muted()))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w(px(0.0))
                                    .overflow_hidden()
                                    .whitespace_nowrap()
                                    .text_ellipsis()
                                    .text_size(px(10.0))
                                    .text_color(if is_dir {
                                        ShellDeckColors::text_primary()
                                    } else {
                                        ShellDeckColors::text_muted()
                                    })
                                    .child(entry.name),
                            ),
                    );
                }
            }
        } else if files_selected {
            let (session_id, connection_id) = match &session.context.target {
                AgentTarget::Ssh { connection_id, .. } => (session.id, *connection_id),
                AgentTarget::Local => unreachable!("local files use the local browser"),
            };
            let request_path = self
                .inspector_remote_path
                .clone()
                .unwrap_or_else(|| session.context.workdir.clone());
            if self.inspector_remote_loading {
                list = list.child(inspector_empty_state(
                    "refresh-cw",
                    t!("agents.inspector.remote_loading").to_string(),
                ));
            } else if let Some(error) = self.inspector_remote_error.clone() {
                let retry_path = request_path.clone();
                list = list
                    .child(inspector_empty_state("circle-alert", error))
                    .child(
                        Button::new(
                            "agent-remote-files-retry",
                            t!("agents.inspector.retry").to_string(),
                        )
                        .variant(ButtonVariant::Secondary)
                        .size(ButtonSize::Sm)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.request_remote_directory(
                                session_id,
                                connection_id,
                                retry_path.clone(),
                                cx,
                            );
                        })),
                    );
            } else if self.inspector_remote_entries.is_empty() {
                let load_path = request_path.clone();
                list = list
                    .child(inspector_empty_state(
                        "server",
                        t!("agents.inspector.remote_empty").to_string(),
                    ))
                    .child(
                        Button::new(
                            "agent-remote-files-load",
                            t!("agents.inspector.remote_load").to_string(),
                        )
                        .variant(ButtonVariant::Secondary)
                        .size(ButtonSize::Sm)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.request_remote_directory(
                                session_id,
                                connection_id,
                                load_path.clone(),
                                cx,
                            );
                        })),
                    );
            } else {
                for entry in self.inspector_remote_entries.clone() {
                    let path = entry.path.clone();
                    let is_dir = entry.is_dir;
                    let mut row = div()
                        .id(SharedString::from(format!(
                            "agent-remote-file-{}",
                            entry.path
                        )))
                        .h(px(25.0))
                        .w_full()
                        .flex()
                        .items_center()
                        .gap(px(6.0))
                        .px(px(6.0))
                        .rounded(px(4.0))
                        .text_size(px(10.0))
                        .text_color(if is_dir {
                            ShellDeckColors::text_primary()
                        } else {
                            ShellDeckColors::text_muted()
                        })
                        .child(lucide_icon(
                            if is_dir { "archive" } else { "file-text" },
                            11.0,
                            ShellDeckColors::text_muted(),
                        ))
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.0))
                                .overflow_hidden()
                                .whitespace_nowrap()
                                .text_ellipsis()
                                .child(entry.name),
                        );
                    if is_dir {
                        row = row
                            .cursor_pointer()
                            .hover(|style| style.bg(ShellDeckColors::hover_bg()))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.request_remote_directory(
                                    session_id,
                                    connection_id,
                                    path.clone(),
                                    cx,
                                );
                            }));
                    } else {
                        row = row
                            .cursor_pointer()
                            .hover(|style| style.bg(ShellDeckColors::hover_bg()))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.tools.active = Some(tools::AgentToolSurface::Preview);
                                this.tools.preview_session = Some(session_id);
                                let request_id = Uuid::new_v4();
                                this.tools.preview_request = request_id;
                                this.tools.preview_context = Some(this.displayed_context(cx));
                                this.tools.preview_title = path.clone();
                                this.tools.remote_preview = None;
                                this.tools.loading = true;
                                this.tools.error = None;
                                cx.emit(AgentConsoleEvent::OpenRemoteFile {
                                    session_id,
                                    request_id,
                                    connection_id,
                                    path: path.clone(),
                                });
                                cx.notify();
                            }));
                    }
                    list = list.child(row);
                }
            }
        } else {
            let visible = files
                .iter()
                .filter(|file| file.diff.is_some())
                .collect::<Vec<_>>();
            if visible.is_empty() {
                list = list.child(inspector_empty_state(
                    "git-branch",
                    t!("agents.inspector.empty_changes").to_string(),
                ));
            }
            for file in visible {
                let diff = file.diff.as_ref().expect("changed file has a diff");
                let trace_id = diff.trace_id;
                let selected = self.focused_trace == Some(trace_id);
                let mut row = div()
                    .id(("agent-inspector-trace", uuid_key(trace_id)))
                    .w_full()
                    .flex()
                    .flex_col()
                    .gap(px(4.0))
                    .px(px(8.0))
                    .py(px(7.0))
                    .overflow_hidden()
                    .border_1()
                    .border_color(if selected {
                        ShellDeckColors::primary()
                    } else {
                        ShellDeckColors::border()
                    })
                    .rounded(px(5.0))
                    .bg(if selected {
                        ShellDeckColors::primary().opacity(0.08)
                    } else {
                        ShellDeckColors::bg_surface()
                    })
                    .cursor_pointer()
                    .hover(|style| style.bg(ShellDeckColors::hover_bg()))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.focused_trace = Some(trace_id);
                        pin_to_latest(&this.scroll);
                        cx.notify();
                    }))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.0))
                            .child(lucide_icon("git-branch", 12.0, ShellDeckColors::warning()))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w(px(0.0))
                                    .overflow_hidden()
                                    .whitespace_nowrap()
                                    .text_ellipsis()
                                    .font_family("JetBrains Mono")
                                    .text_size(px(9.5))
                                    .text_color(ShellDeckColors::text_primary())
                                    .child(file.path.clone()),
                            )
                            .child(
                                div()
                                    .flex_shrink_0()
                                    .font_family("JetBrains Mono")
                                    .text_size(px(8.5))
                                    .text_color(ShellDeckColors::text_muted())
                                    .child(format!("+{} −{}", diff.additions, diff.deletions)),
                            ),
                    );
                if let Some(preview) = diff.preview.as_ref() {
                    row = row.child(
                        div()
                            .max_h(px(88.0))
                            .overflow_hidden()
                            .font_family("JetBrains Mono")
                            .text_size(px(8.5))
                            .line_height(relative(1.3))
                            .text_color(ShellDeckColors::text_muted())
                            .whitespace_normal()
                            .child(preview.clone()),
                    );
                }
                list = list.child(row);
            }
        }

        div()
            .w_full()
            .h_full()
            .flex()
            .flex_col()
            .flex_shrink_0()
            .border_l_1()
            .border_color(ShellDeckColors::border())
            .bg(ShellDeckColors::bg_sidebar())
            .child(tabs)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(4.0))
                    .px(px(10.0))
                    .py(px(8.0))
                    .border_b_1()
                    .border_color(ShellDeckColors::border())
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.0))
                            .when_some(remote_parent, |row, parent_path| {
                                let (session_id, connection_id) = match &session.context.target {
                                    AgentTarget::Ssh { connection_id, .. } => {
                                        (session.id, *connection_id)
                                    }
                                    AgentTarget::Local => {
                                        unreachable!("a remote parent requires an SSH target")
                                    }
                                };
                                row.child(
                                    Button::new("agent-remote-files-parent", "")
                                        .variant(ButtonVariant::Ghost)
                                        .size(ButtonSize::Icon)
                                        .icon(IconSource::from("arrow-up"))
                                        .tooltip(t!("agents.inspector.remote_parent").to_string())
                                        .w(px(24.0))
                                        .h(px(24.0))
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.request_remote_directory(
                                                session_id,
                                                connection_id,
                                                parent_path.clone(),
                                                cx,
                                            );
                                        })),
                                )
                            })
                            .child(
                                Badge::new(if remote {
                                    t!("agents.inspector.remote").to_string()
                                } else {
                                    t!("agents.inspector.local").to_string()
                                })
                                .variant(BadgeVariant::Secondary),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w(px(0.0))
                                    .overflow_hidden()
                                    .whitespace_nowrap()
                                    .text_ellipsis()
                                    .font_family("JetBrains Mono")
                                    .text_size(px(9.0))
                                    .text_color(ShellDeckColors::text_muted())
                                    .child(session.context.workdir.clone()),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(5.0))
                            .text_size(px(9.0))
                            .text_color(ShellDeckColors::text_muted())
                            .child(format!(
                                "{} · {} {}",
                                t!("agents.inspector.observed"),
                                files.len(),
                                t!("agents.inspector.files")
                            ))
                            .when(changed_count > 0, |row| {
                                row.child(format!(
                                    "· {changed_count} {}",
                                    t!("agents.inspector.changes")
                                ))
                            }),
                    ),
            )
            .child(list)
            .when(changes_selected, |panel| {
                panel.child(
                    div()
                        .flex_shrink_0()
                        .px(px(10.0))
                        .py(px(7.0))
                        .border_t_1()
                        .border_color(ShellDeckColors::border())
                        .text_size(px(8.5))
                        .text_color(ShellDeckColors::text_muted())
                        .child(t!("agents.inspector.focus_hint").to_string()),
                )
            })
            .into_any_element()
    }
}
