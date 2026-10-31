use super::*;

impl AgentConsoleView {
    pub(super) fn reveal_trace(&mut self, trace_id: Uuid, cx: &mut Context<Self>) {
        if !self
            .sessions
            .selected()
            .is_some_and(|session| session.trace.iter().any(|trace| trace.id == trace_id))
        {
            return;
        }
        self.focused_trace = Some(trace_id);
        self.pending_trace_reveal = Some(trace_id);
        cx.notify();
    }
    pub(super) fn render_inspector_tabs(&self, cx: &mut Context<Self>) -> Div {
        let files_selected = self.inspector_tab == AgentInspectorTab::Files;
        let changes_selected = self.inspector_tab == AgentInspectorTab::Changes;
        div()
            .min_h(px(42.0))
            .flex_shrink_0()
            .flex_wrap()
            .flex()
            .items_center()
            .gap(px(6.0))
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
                    this.refresh_review(cx);
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
            )
    }

    pub(super) fn render_inspector(
        &mut self,
        session: &AgentSession,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let remote = matches!(session.context.target, AgentTarget::Ssh { .. });
        let files = inspector_files(self.sessions.get(session.id).unwrap_or(session));
        let changed_count = files.iter().filter(|file| file.diff.is_some()).count();
        let files_selected = self.inspector_tab == AgentInspectorTab::Files;
        let changes_selected = self.inspector_tab == AgentInspectorTab::Changes;

        let tabs = self.render_inspector_tabs(cx);

        if files_selected {
            let explorer = self.render_file_explorer(session, cx);
            return div()
                .w_full()
                .h_full()
                .flex()
                .flex_col()
                .min_h_0()
                .min_w_0()
                .bg(ShellDeckColors::bg_sidebar())
                .child(tabs)
                .child(explorer)
                .into_any_element();
        }

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
                    this.reveal_trace(trace_id, cx);
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
