use super::*;

impl AgentConsoleView {
    pub(super) fn render_navigator(&self, cx: &mut Context<Self>) -> AnyElement {
        let parent = cx.entity();
        let running_count = self.sessions.active_count();
        let unread_count = self
            .sessions
            .sessions()
            .iter()
            .map(|session| u64::from(session.unread_count))
            .sum::<u64>();
        let mut body = div().flex().flex_col().gap(px(4.0)).px(px(6.0)).py(px(7.0));
        body = body.child(self.render_remote_session_rows(cx));
        if self.project_groups.is_empty() {
            body = body.child(self.render_session_rows(None, cx));
        } else {
            for project in &self.project_groups {
                body = body.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.0))
                        .px(px(6.0))
                        .pt(px(6.0))
                        .pb(px(3.0))
                        .child(lucide_icon(
                            "git-branch",
                            12.0,
                            ShellDeckColors::text_muted(),
                        ))
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.0))
                                .overflow_hidden()
                                .text_ellipsis()
                                .text_size(px(10.5))
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(project.label.clone()),
                        ),
                );
                for worktree in &project.worktrees {
                    let worktree_option = worktree.clone();
                    let worktree_selected = !self.remote_sessions.active
                        && self
                            .sessions
                            .selected()
                            .is_some_and(|session| session_worktree_matches(session, worktree));
                    body = body.child(
                        div()
                            .id(("agent-worktree", stable_string_key(&worktree.id)))
                            .flex()
                            .items_center()
                            .gap(px(5.0))
                            .px(px(11.0))
                            .py(px(3.0))
                            .rounded(px(4.0))
                            .cursor_pointer()
                            .when(worktree_selected, |row| {
                                row.bg(ShellDeckColors::selected_bg())
                            })
                            .when(!worktree_selected, |row| {
                                row.hover(|style| style.bg(ShellDeckColors::hover_bg()))
                            })
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.select_worktree_session(&worktree_option, cx);
                            }))
                            .text_size(px(9.5))
                            .text_color(ShellDeckColors::text_muted())
                            .child(lucide_icon(
                                "git-branch",
                                10.0,
                                ShellDeckColors::text_muted(),
                            ))
                            .child(worktree.label.clone())
                            .when_some(worktree.branch.clone(), |row, branch| {
                                row.child(
                                    div()
                                        .flex_1()
                                        .min_w(px(0.0))
                                        .overflow_hidden()
                                        .text_ellipsis()
                                        .font_family("JetBrains Mono")
                                        .text_size(px(8.5))
                                        .child(branch),
                                )
                            })
                            .when(worktree.is_primary, |row| {
                                row.child(
                                    div()
                                        .size(px(5.0))
                                        .rounded_full()
                                        .bg(ShellDeckColors::success()),
                                )
                            }),
                    );
                    body = body.child(self.render_session_rows(Some(worktree), cx));
                }
            }
            let known_worktrees = self
                .project_groups
                .iter()
                .flat_map(|project| project.worktrees.iter())
                .collect::<Vec<_>>();
            body = body.child(self.render_unmatched_session_rows(&known_worktrees, cx));
        }
        div()
            .w(px(f32::from(self.library.preferences.navigator_width)))
            .h_full()
            .flex()
            .flex_col()
            .flex_shrink_0()
            .bg(ShellDeckColors::bg_sidebar())
            .border_r_1()
            .border_color(ShellDeckColors::border())
            .child(
                div()
                    .h(px(42.0))
                    .flex()
                    .items_center()
                    .gap(px(7.0))
                    .px(px(10.0))
                    .border_b_1()
                    .border_color(ShellDeckColors::border())
                    .child(lucide_icon("grid-2x2", 14.0, ShellDeckColors::primary()))
                    .child(
                        div()
                            .flex_1()
                            .text_size(px(11.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(t!("agents.title").to_string()),
                    )
                    .when(running_count > 0, |row| {
                        row.child(
                            Badge::new(running_count.to_string()).variant(BadgeVariant::Secondary),
                        )
                    })
                    .child(
                        Button::new("agent-nav-new", "")
                            .variant(ButtonVariant::Ghost)
                            .size(ButtonSize::Icon)
                            .icon(IconSource::from("plus"))
                            .w(px(26.0))
                            .h(px(26.0))
                            .tooltip(t!("agents.session.new").to_string())
                            .on_click(cx.listener(|this, _, _, cx| this.new_session(cx))),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .px(px(8.0))
                    .py(px(6.0))
                    .border_b_1()
                    .border_color(ShellDeckColors::border())
                    .child(lucide_icon("search", 11.0, ShellDeckColors::text_muted()))
                    .child(
                        div().flex().flex_col().flex_1().min_w(px(0.0)).child(
                            Input::new(&self.navigator_search)
                                .variant(InputVariant::Bare)
                                .size(InputSize::Sm)
                                .w_full()
                                .placeholder(t!("fleet.sessions.search").to_string())
                                .on_change(move |_value, cx| {
                                    parent.update(cx, |_this, cx| cx.notify());
                                }),
                        ),
                    )
                    .child(
                        Button::new("agent-library-archived-filter", "")
                            .variant(if self.library.show_archived() {
                                ButtonVariant::Secondary
                            } else {
                                ButtonVariant::Ghost
                            })
                            .size(ButtonSize::Icon)
                            .icon(IconSource::from("archive"))
                            .tooltip(t!("agents.library.show_archived").to_string())
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.library.preferences.show_archived =
                                    !this.library.preferences.show_archived;
                                this.schedule_library_save(cx);
                                cx.notify();
                            })),
                    ),
            )
            .child(
                div()
                    .id("agent-navigator-scroll")
                    .flex_1()
                    .min_h(px(0.0))
                    .overflow_y_scroll()
                    .child(body),
            )
            .child(
                div()
                    .h(px(30.0))
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .px(px(10.0))
                    .border_t_1()
                    .border_color(ShellDeckColors::border())
                    .text_size(px(9.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(lucide_icon("activity", 11.0, ShellDeckColors::warning()))
                    .child(format!(
                        "{running_count}/{DEFAULT_MAX_CONCURRENT_AGENT_SESSIONS}"
                    ))
                    .child(t!("workspaces.agent.running").to_string())
                    .child(div().flex_1())
                    .when(unread_count > 0, |row| {
                        row.child(lucide_icon("activity", 10.0, ShellDeckColors::primary()))
                            .child(unread_count.to_string())
                            .child(t!("workspaces.attention.unread").to_string())
                    }),
            )
            .into_any_element()
    }

    pub(super) fn render_session_rows(
        &self,
        worktree: Option<&AgentWorktreeOption>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        self.render_session_rows_matching(
            |session| worktree.is_none_or(|worktree| session_worktree_matches(session, worktree)),
            cx,
        )
    }

    fn render_unmatched_session_rows(
        &self,
        known_worktrees: &[&AgentWorktreeOption],
        cx: &mut Context<Self>,
    ) -> AnyElement {
        self.render_session_rows_matching(
            |session| {
                !known_worktrees
                    .iter()
                    .any(|worktree| session_worktree_matches(session, worktree))
            },
            cx,
        )
    }

    fn render_session_rows_matching(
        &self,
        include: impl Fn(&AgentSession) -> bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let query = self
            .navigator_search
            .read(cx)
            .content()
            .trim()
            .to_lowercase();
        let mut list = div().flex().flex_col().gap(px(2.0));
        for session in self
            .sessions
            .search(&query, self.library.show_archived())
            .into_iter()
            .filter(|session| include(session))
        {
            let id = session.id;
            let selected = !self.remote_sessions.active && self.sessions.selected_id() == Some(id);
            let removable = self.sessions.sessions().len() > 1 && !session.status.is_active();
            let mut row = div()
                .id(("agent-nav-session", uuid_key(id)))
                .flex()
                .items_center()
                .gap(px(7.0))
                .h(px(33.0))
                .px(px(8.0))
                .rounded(px(5.0))
                .cursor_pointer()
                .when(selected, |row| row.bg(ShellDeckColors::selected_bg()))
                .when(!selected, |row| {
                    row.hover(|style| style.bg(ShellDeckColors::hover_bg()))
                })
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.select_session(id, cx);
                }))
                .child(
                    div()
                        .size(px(6.0))
                        .rounded_full()
                        .bg(status_color(session.status)),
                )
                .child(lucide_icon(
                    if session.archived {
                        "archive"
                    } else if session.pinned {
                        "pin"
                    } else {
                        provider_icon(session.context.provider)
                    },
                    12.0,
                    ShellDeckColors::text_muted(),
                ))
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.0))
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .text_size(px(10.5))
                        .font_weight(if selected {
                            FontWeight::SEMIBOLD
                        } else {
                            FontWeight::NORMAL
                        })
                        .child(session.name.clone()),
                )
                .when_some(session_elapsed(session), |row, elapsed| {
                    row.child(
                        div()
                            .font_family("JetBrains Mono")
                            .text_size(px(8.5))
                            .text_color(ShellDeckColors::text_muted())
                            .child(elapsed),
                    )
                })
                .when(session.unread_count > 0, |row| {
                    row.child(
                        Badge::new(session.unread_count.to_string())
                            .variant(BadgeVariant::Secondary),
                    )
                });
            if removable {
                row = row.child(
                    div()
                        .id(("agent-close-session", uuid_key(id)))
                        .size(px(19.0))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(4.0))
                        .hover(|style| style.bg(ShellDeckColors::hover_bg()))
                        .on_click(cx.listener(move |this, _event: &ClickEvent, _, cx| {
                            cx.stop_propagation();
                            this.archive_library_session(id, true, cx);
                        }))
                        .child(lucide_icon("x", 10.0, ShellDeckColors::text_muted())),
                );
            }
            list = list.child(row);
        }
        list.into_any_element()
    }

    pub(super) fn render_tabs(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut tabs = div()
            .id("agent-session-tabs")
            .flex()
            .flex_1()
            .min_w(px(0.0))
            .h_full()
            .overflow_x_scroll();
        for session in self.sessions.library_sessions(false) {
            let id = session.id;
            let selected = self.sessions.selected_id() == Some(id);
            let removable = self.sessions.sessions().len() > 1 && !session.status.is_active();
            let mut tab = div()
                .id(("agent-session-tab", uuid_key(id)))
                .h_full()
                .max_w(px(220.0))
                .flex()
                .items_center()
                .gap(px(7.0))
                .px(px(11.0))
                .border_r_1()
                .border_color(ShellDeckColors::border())
                .cursor_pointer()
                .when(selected, |tab| {
                    tab.bg(ShellDeckColors::bg_primary())
                        .border_b_2()
                        .border_color(ShellDeckColors::primary())
                })
                .when(!selected, |tab| {
                    tab.hover(|style| style.bg(ShellDeckColors::hover_bg()))
                })
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.select_session(id, cx);
                }))
                .child(
                    div()
                        .size(px(6.0))
                        .rounded_full()
                        .bg(status_color(session.status)),
                )
                .child(lucide_icon(
                    provider_icon(session.context.provider),
                    12.0,
                    ShellDeckColors::text_muted(),
                ))
                .child(
                    div()
                        .min_w(px(0.0))
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .text_size(px(10.5))
                        .child(session.name.clone()),
                )
                .when(session.attention != AgentSessionAttention::None, |tab| {
                    tab.child(
                        div()
                            .size(px(5.0))
                            .rounded_full()
                            .bg(attention_color(session.attention)),
                    )
                });
            if removable {
                tab = tab.child(
                    div()
                        .id(("agent-close-tab", uuid_key(id)))
                        .size(px(18.0))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(4.0))
                        .hover(|style| style.bg(ShellDeckColors::hover_bg()))
                        .on_click(cx.listener(move |this, _event: &ClickEvent, _, cx| {
                            cx.stop_propagation();
                            this.archive_library_session(id, true, cx);
                        }))
                        .child(lucide_icon("x", 10.0, ShellDeckColors::text_muted())),
                );
            }
            tabs = tabs.child(tab);
        }
        tabs.into_any_element()
    }
}
