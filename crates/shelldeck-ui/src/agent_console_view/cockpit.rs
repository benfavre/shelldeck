//! Cockpit chrome drawn from the owner's mock-up (SDUC-595): the sidebar head
//! (Nouvelle session, search field, Tout / Local / SSH), project badges, the
//! imported-conversations group, the footer, and the small shared pieces the
//! session header, the start screen and the details panel are built from.
use super::navigator::NavigatorTargetFilter;
use super::*;

/// Rounded corners of the cockpit's cards, buttons and fields.
pub(super) const CARD_RADIUS: f32 = 10.0;
/// Rounded corners of the large surfaces (the main card, the composer).
pub(super) const SURFACE_RADIUS: f32 = 12.0;
/// Preference key of the « Conversations importées » group. It starts closed,
/// as in the mock-up, so the saved key records that it was opened.
pub(super) const IMPORTED_GROUP_OPEN_KEY: &str = "imported-conversations-open";

/// A small rounded label: « SSH », « Local », « 12 », « Prêt à relire ».
pub(super) fn pill(label: impl Into<SharedString>, tone: Hsla, filled: bool) -> Div {
    div()
        .flex_shrink_0()
        .flex()
        .items_center()
        .h(px(20.0))
        .px(px(7.0))
        .rounded(px(6.0))
        .text_size(px(10.5))
        .font_weight(FontWeight::MEDIUM)
        .when(filled, |pill| pill.bg(tone.opacity(0.12)).text_color(tone))
        .when(!filled, |pill| {
            pill.bg(ShellDeckColors::hover_bg())
                .text_color(ShellDeckColors::text_muted())
        })
        .child(label.into())
}

/// A status dot followed by its words (« Connecté », « Prêt »).
pub(super) fn status_pill(label: impl Into<SharedString>, color: Hsla) -> Div {
    div()
        .flex_shrink_0()
        .flex()
        .items_center()
        .gap(px(6.0))
        .h(px(26.0))
        .px(px(10.0))
        .rounded(px(13.0))
        .bg(color.opacity(0.10))
        .text_size(px(11.5))
        .text_color(ShellDeckColors::text_primary())
        .child(div().size(px(7.0)).rounded_full().bg(color))
        .child(label.into())
}

/// The robot on a pale blue tile, as in the mock-up's start screen and replies.
pub(super) fn agent_tile(size: f32) -> Div {
    div()
        .flex_shrink_0()
        .flex()
        .items_center()
        .justify_center()
        .size(px(size))
        .rounded(px(size * 0.28))
        .bg(ShellDeckColors::primary().opacity(0.10))
        .child(lucide_icon("bot", size * 0.48, ShellDeckColors::primary()))
}

/// The heading of a card or a details section.
pub(super) fn section_heading(label: impl Into<SharedString>) -> Div {
    div()
        .text_size(px(13.0))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(ShellDeckColors::text_primary())
        .child(label.into())
}

/// A bordered dropdown face: icon, label, chevron. Used for the project and
/// machine choosers and for the composer's agent, permission and model menus.
pub(super) fn chip(
    id: impl Into<ElementId>,
    icon: Option<AnyElement>,
    label: impl Into<SharedString>,
    height: f32,
) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .flex_shrink_0()
        .items_center()
        .gap(px(8.0))
        .h(px(height))
        .min_w_0()
        .px(px(12.0))
        .rounded(px(CARD_RADIUS))
        .border_1()
        .border_color(ShellDeckColors::border())
        .cursor_pointer()
        .hover(|style| style.bg(ShellDeckColors::hover_bg()))
        .text_size(px(13.0))
        .text_color(ShellDeckColors::text_primary())
        .when_some(icon, |chip, icon| chip.child(icon))
        .child(div().min_w_0().truncate().child(label.into()))
        .child(lucide_icon(
            "chevron-down",
            14.0,
            ShellDeckColors::text_muted(),
        ))
}

pub(super) fn target_badge(target: &AgentTarget) -> Div {
    pill(
        t!(match target {
            AgentTarget::Local => "agents.cockpit.badge_local",
            AgentTarget::Ssh { .. } => "agents.cockpit.badge_ssh",
        })
        .to_string(),
        ShellDeckColors::text_muted(),
        false,
    )
}

impl AgentConsoleView {
    pub(super) fn open_navigator_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.library.search_open = true;
        window.focus(&self.navigator_search.read(cx).focus_handle(cx));
        cx.notify();
    }

    /// Nouvelle session, the search field and the machine filter, then « Projets ».
    pub(super) fn render_navigator_head(
        &self,
        query: &str,
        parent: Entity<Self>,
        search_blur_parent: Entity<Self>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mut head = div()
            .flex()
            .flex_col()
            .flex_shrink_0()
            .gap(px(12.0))
            .px(px(12.0))
            .pt(px(12.0))
            .pb(px(6.0))
            .child(
                Button::new("agent-nav-new", t!("agents.session.new").to_string())
                    .debug_selector(|| "agent-nav-new".into())
                    .variant(ButtonVariant::Default)
                    .icon(IconSource::from("plus"))
                    .w_full()
                    .h(px(38.0))
                    .rounded(px(CARD_RADIUS))
                    .on_click(cx.listener(|this, _, _, cx| this.new_session(cx))),
            );
        let field = div()
            .flex()
            .items_center()
            .gap(px(8.0))
            .h(px(36.0))
            .px(px(10.0))
            .rounded(px(CARD_RADIUS))
            .border_1()
            .border_color(ShellDeckColors::border())
            .bg(ShellDeckColors::bg_sidebar());
        if self.library.search_open {
            head = head.child(
                field
                    .border_color(ShellDeckColors::primary())
                    .child(lucide_icon("search", 14.0, ShellDeckColors::text_muted()))
                    .child(
                        div().flex().flex_col().flex_1().min_w(px(0.0)).child(
                            Input::new(&self.navigator_search)
                                .debug_selector("agent-nav-search")
                                .variant(InputVariant::Bare)
                                .size(InputSize::Sm)
                                .w_full()
                                .placeholder(t!("agents.cockpit.search").to_string())
                                .on_change(move |_value, cx| {
                                    parent.update(cx, |_this, cx| cx.notify());
                                })
                                .on_blur(move |_, cx| {
                                    search_blur_parent.update(cx, |view, cx| {
                                        view.library.search_open = false;
                                        view.navigator_search.update(cx, |state, cx| {
                                            state.replace_content(String::new(), cx)
                                        });
                                        view.request_surface_focus(cx);
                                        cx.notify();
                                    });
                                }),
                        ),
                    )
                    .child(
                        Button::new("agent-nav-clear-search", "")
                            .debug_selector(|| "agent-nav-clear-search".into())
                            .variant(ButtonVariant::Ghost)
                            .size(ButtonSize::Icon)
                            .icon("x")
                            .w(px(24.0))
                            .h(px(24.0))
                            .when(query.is_empty(), |button| button.invisible())
                            .tooltip(t!("agents.navigator.clear_search").to_string())
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.navigator_search.update(cx, |state, cx| {
                                    state.replace_content(String::new(), cx)
                                });
                                window.focus(&this.navigator_search.read(cx).focus_handle(cx));
                                cx.notify();
                            })),
                    ),
            );
        } else {
            // At rest the field is a button that looks like one: the real input
            // only exists while searching, so blur and Escape keep their meaning.
            head = head.child(
                field
                    .id("agent-nav-search-trigger")
                    .debug_selector(|| "agent-nav-search-trigger".into())
                    .cursor_text()
                    .hover(|style| style.border_color(ShellDeckColors::primary().opacity(0.5)))
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.open_navigator_search(window, cx);
                    }))
                    .child(lucide_icon("search", 14.0, ShellDeckColors::text_muted()))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(px(12.5))
                            .text_color(ShellDeckColors::text_muted())
                            .child(t!("agents.cockpit.search").to_string()),
                    ),
            );
        }
        head = head.child(self.render_target_filter(cx));
        head = head.child(
            div()
                .flex()
                .items_center()
                .gap(px(6.0))
                .pt(px(6.0))
                .child(
                    div()
                        .flex_1()
                        .pl(px(2.0))
                        .text_size(px(12.5))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(ShellDeckColors::text_primary())
                        .child(t!("agents.navigator.projects").to_string()),
                )
                .child(self.navigator_actions_button(cx))
                .child(
                    Button::new("agent-nav-add-project-quick", "")
                        .debug_selector(|| "agent-nav-add-project-quick".into())
                        .variant(ButtonVariant::Ghost)
                        .size(ButtonSize::Icon)
                        .icon(IconSource::from("plus"))
                        .w(px(26.0))
                        .h(px(26.0))
                        .tooltip(t!("agents.cockpit.add_project").to_string())
                        .on_click(
                            cx.listener(|_, _, _, cx| cx.emit(AgentConsoleEvent::OpenProjectSetup)),
                        ),
                ),
        );
        head.into_any_element()
    }

    fn render_target_filter(&self, cx: &mut Context<Self>) -> AnyElement {
        let current = self.library.target_filter;
        let mut row = div()
            .flex()
            .items_center()
            .gap(px(4.0))
            .p(px(3.0))
            .rounded(px(CARD_RADIUS))
            .bg(ShellDeckColors::hover_bg().opacity(0.7));
        for (filter, id, key) in [
            (
                NavigatorTargetFilter::All,
                "agent-nav-filter-all",
                "agents.cockpit.filter_all",
            ),
            (
                NavigatorTargetFilter::Local,
                "agent-nav-filter-local",
                "agents.cockpit.filter_local",
            ),
            (
                NavigatorTargetFilter::Ssh,
                "agent-nav-filter-ssh",
                "agents.cockpit.filter_ssh",
            ),
        ] {
            let active = current == filter;
            row = row.child(
                div()
                    .id(id)
                    .debug_selector(move || id.into())
                    .flex_1()
                    .flex()
                    .items_center()
                    .justify_center()
                    .h(px(28.0))
                    .rounded(px(7.0))
                    .cursor_pointer()
                    .text_size(px(12.5))
                    .when(active, |segment| {
                        segment
                            .bg(ShellDeckColors::primary().opacity(0.12))
                            .text_color(ShellDeckColors::primary())
                            .font_weight(FontWeight::SEMIBOLD)
                    })
                    .when(!active, |segment| {
                        segment
                            .text_color(ShellDeckColors::text_primary())
                            .hover(|style| style.bg(ShellDeckColors::bg_sidebar().opacity(0.7)))
                    })
                    .tooltip({
                        let label = t!("agents.cockpit.filter_label").to_string();
                        move |_, cx| {
                            cx.new(|_| super::navigator::NavigatorTooltip(label.clone()))
                                .into()
                        }
                    })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if this.library.target_filter != filter {
                            this.library.target_filter = filter;
                            cx.emit(AgentConsoleEvent::NavigatorChanged);
                            cx.notify();
                        }
                    }))
                    .child(t!(key).to_string()),
            );
        }
        row.into_any_element()
    }

    fn toggle_imported_conversations(&mut self, cx: &mut Context<Self>) {
        if !self.navigator_search.read(cx).content().trim().is_empty() {
            return;
        }
        let ids = &mut self.library.preferences.collapsed_project_ids;
        if let Some(index) = ids.iter().position(|id| id == IMPORTED_GROUP_OPEN_KEY) {
            ids.remove(index);
        } else {
            ids.push(IMPORTED_GROUP_OPEN_KEY.to_string());
        }
        self.schedule_library_save(cx);
        cx.notify();
    }

    /// « Conversations importées »: local CLI discovery, saved-server
    /// conversations and the Platform directory, behind one collapsible header.
    pub(super) fn render_imported_conversations(&self, cx: &mut Context<Self>) -> AnyElement {
        let filter = self.library.target_filter;
        let query = self.navigator_search.read(cx).content().to_string();
        let collapsed = query.trim().is_empty()
            && !self
                .library
                .preferences
                .collapsed_project_ids
                .iter()
                .any(|id| id == IMPORTED_GROUP_OPEN_KEY);
        let count = self
            .sessions
            .library_sessions(self.library.show_archived())
            .iter()
            .filter(|session| session.remote_history.is_some())
            .count();
        let mut group = div().flex().flex_col().min_w_0().gap(px(4.0)).mt(px(8.0));
        let header_group: SharedString = "agent-imported-hover".into();
        group = group.child(
            super::navigation_groups::navigation_header_surface(
                "agent-imported-header",
                header_group,
            )
            .debug_selector(|| "agent-imported-toggle".into())
            .cursor_pointer()
            .on_click(cx.listener(|this, _, _, cx| {
                this.toggle_imported_conversations(cx);
            }))
            .child(div().w(px(24.0)).flex().justify_center().child(lucide_icon(
                if collapsed {
                    "chevron-right"
                } else {
                    "chevron-down"
                },
                14.0,
                ShellDeckColors::text_muted(),
            )))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .px(px(4.0))
                    .text_size(px(13.0))
                    .font_weight(FontWeight::MEDIUM)
                    .child(t!("agents.cockpit.imported").to_string()),
            )
            .when(count > 0, |header| {
                header.child(pill(
                    count.to_string(),
                    ShellDeckColors::text_muted(),
                    false,
                ))
            }),
        );
        if collapsed {
            return group.into_any_element();
        }
        let mut body = div().flex().flex_col().min_w_0().gap(px(4.0));
        if filter != NavigatorTargetFilter::Ssh {
            body = body.child(self.render_local_session_rows(cx));
        }
        if filter != NavigatorTargetFilter::Local {
            body = body
                .child(self.render_ssh_session_rows(cx))
                .child(self.render_remote_session_rows(cx));
        }
        group.child(body).into_any_element()
    }

    /// The running count, then Paramètres and Aide, as in the mock-up's foot.
    pub(super) fn render_navigator_foot(&self, cx: &mut Context<Self>) -> AnyElement {
        div()
            .flex()
            .flex_col()
            .flex_shrink_0()
            .gap(px(2.0))
            .mx(px(10.0))
            .pt(px(6.0))
            .pb(px(8.0))
            .border_t_1()
            .border_color(ShellDeckColors::border())
            .child(self.render_activity_footer(cx))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .child(
                        Button::new(
                            "agent-footer-settings",
                            t!("agents.cockpit.settings").to_string(),
                        )
                        .debug_selector(|| "agent-footer-settings".into())
                        .variant(ButtonVariant::Ghost)
                        .size(ButtonSize::Sm)
                        .icon("settings")
                        .h(px(32.0))
                        .on_click(
                            cx.listener(|_, _, _, cx| cx.emit(AgentConsoleEvent::OpenSettings)),
                        ),
                    )
                    .child(
                        Button::new("agent-footer-help", t!("agents.cockpit.help").to_string())
                            .debug_selector(|| "agent-footer-help".into())
                            .variant(ButtonVariant::Ghost)
                            .size(ButtonSize::Sm)
                            .icon("circle-help")
                            .h(px(32.0))
                            .on_click(
                                cx.listener(|_, _, _, cx| cx.emit(AgentConsoleEvent::OpenHelp)),
                            ),
                    ),
            )
            .into_any_element()
    }
}

/// One test of the latest turn that ran any, for « Vérifications ».
pub(super) struct TurnCheck {
    pub name: String,
    pub status: AgentTraceStatus,
}

/// The tests of the most recent turn that reported tests, in order.
pub(super) fn latest_turn_checks(session: &AgentSession) -> Vec<TurnCheck> {
    let Some(turn) = session
        .trace
        .iter()
        .rev()
        .find(|trace| matches!(trace.detail, AgentTraceKind::Test { .. }))
        .map(|trace| trace.turn_id)
    else {
        return Vec::new();
    };
    session
        .trace
        .iter()
        .filter(|trace| trace.turn_id == turn)
        .filter_map(|trace| match &trace.detail {
            AgentTraceKind::Test { name, status, .. } => Some(TurnCheck {
                name: name.clone(),
                status: *status,
            }),
            _ => None,
        })
        .collect()
}

/// One file of the latest turn that edited any: path and its +/− counts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct TurnFileChange {
    pub path: String,
    pub additions: u32,
    pub deletions: u32,
}

/// The files the most recent editing turn changed, merged by path in first-edit order.
pub(super) fn latest_turn_file_changes(session: &AgentSession) -> Vec<TurnFileChange> {
    let Some(turn) = session
        .trace
        .iter()
        .rev()
        .find(|trace| matches!(trace.detail, AgentTraceKind::Diff { .. }))
        .map(|trace| trace.turn_id)
    else {
        return Vec::new();
    };
    let mut files: Vec<TurnFileChange> = Vec::new();
    for trace in session.trace.iter().filter(|trace| trace.turn_id == turn) {
        if let AgentTraceKind::Diff {
            path,
            additions,
            deletions,
            status,
            ..
        } = &trace.detail
        {
            if matches!(
                status,
                AgentTraceStatus::Failed | AgentTraceStatus::Cancelled
            ) {
                continue;
            }
            match files.iter_mut().find(|file| &file.path == path) {
                Some(file) => {
                    file.additions = file.additions.saturating_add(*additions);
                    file.deletions = file.deletions.saturating_add(*deletions);
                }
                None => files.push(TurnFileChange {
                    path: path.clone(),
                    additions: *additions,
                    deletions: *deletions,
                }),
            }
        }
    }
    files
}

pub(super) fn check_icon(status: AgentTraceStatus) -> AnyElement {
    let (icon, color) = match status {
        AgentTraceStatus::Succeeded => ("check", ShellDeckColors::success()),
        AgentTraceStatus::Failed => ("x", ShellDeckColors::error()),
        AgentTraceStatus::Running | AgentTraceStatus::Pending => {
            ("loader-circle", ShellDeckColors::warning())
        }
        AgentTraceStatus::Cancelled | AgentTraceStatus::Unknown => {
            ("minus", ShellDeckColors::text_muted())
        }
    };
    div()
        .flex_shrink_0()
        .flex()
        .items_center()
        .justify_center()
        .size(px(20.0))
        .rounded_full()
        .bg(color.opacity(0.16))
        .child(lucide_icon(icon, 12.0, color))
        .into_any_element()
}

fn details_row(label: String, value: impl IntoElement) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(12.0))
        .min_h(px(26.0))
        .child(
            div()
                .w(px(110.0))
                .flex_shrink_0()
                .text_size(px(12.5))
                .text_color(ShellDeckColors::text_muted())
                .child(label),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_size(px(12.5))
                .child(value),
        )
}

fn details_section() -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(10.0))
        .py(px(16.0))
        .border_b_1()
        .border_color(ShellDeckColors::border())
}

fn compact_count(value: u64) -> String {
    match value {
        0..=999 => value.to_string(),
        1_000..=999_999 => format!("{:.1} k", value as f64 / 1_000.0),
        _ => format!("{:.1} M", value as f64 / 1_000_000.0),
    }
}

impl AgentConsoleView {
    /// « Détails de la session », opened by the header's Détails button.
    pub(super) fn render_session_details(
        &self,
        session: &AgentSession,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let context = &session.context;
        let full = self.sessions.get(session.id);
        let target_ok = validate_agent_target(&context.target, &self.connections).is_ok();
        let mut body = div()
            .id("agent-session-details-scroll")
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .px(px(18.0));

        // Environnement
        let (machine, machine_state) = match &context.target {
            AgentTarget::Local => (
                t!("agents.cockpit.this_machine").to_string(),
                t!("agents.cockpit.ready").to_string(),
            ),
            AgentTarget::Ssh {
                connection_label, ..
            } => (
                connection_label.clone(),
                if target_ok {
                    t!("agents.cockpit.connected").to_string()
                } else {
                    t!("agents.cockpit.disconnected").to_string()
                },
            ),
        };
        let dot = if target_ok {
            ShellDeckColors::success()
        } else {
            ShellDeckColors::error()
        };
        let folder = std::path::Path::new(&context.workdir)
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or(&context.workdir)
            .to_owned();
        let branch = full
            .and_then(|full| super::navigator::navigator_worktree(full, &self.project_groups))
            .and_then(|tree| tree.branch.clone());
        let mut environment = details_section()
            .child(section_heading(
                t!("agents.cockpit.environment").to_string(),
            ))
            .child(
                div()
                    .flex()
                    .items_start()
                    .gap(px(10.0))
                    .child(
                        div()
                            .pt(px(5.0))
                            .child(div().size(px(8.0)).rounded_full().bg(dot)),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w_0()
                            .gap(px(2.0))
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(8.0))
                                    .child(
                                        div()
                                            .min_w_0()
                                            .truncate()
                                            .text_size(px(13.5))
                                            .font_weight(FontWeight::MEDIUM)
                                            .child(machine),
                                    )
                                    .child(target_badge(&context.target)),
                            )
                            .child(
                                div()
                                    .text_size(px(11.5))
                                    .text_color(ShellDeckColors::text_muted())
                                    .child(machine_state),
                            ),
                    ),
            )
            .child(
                div()
                    .id("agent-details-folder")
                    .flex()
                    .items_center()
                    .gap(px(10.0))
                    .tooltip({
                        let path = context.workdir.clone();
                        move |_, cx| {
                            cx.new(|_| super::navigator::NavigatorTooltip(path.clone()))
                                .into()
                        }
                    })
                    .child(lucide_icon("folder", 15.0, ShellDeckColors::text_muted()))
                    .child(div().min_w_0().truncate().text_size(px(13.0)).child(folder)),
            );
        if let Some(branch) = branch {
            environment = environment.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.0))
                    .child(lucide_icon(
                        "git-branch",
                        15.0,
                        ShellDeckColors::text_muted(),
                    ))
                    .child(div().min_w_0().truncate().text_size(px(13.0)).child(branch)),
            );
        }
        body = body.child(environment);

        // Agent
        let idle = !session.status.is_active() && session.status != AgentSessionStatus::Failed;
        let (state_label, state_color) = if idle {
            (
                t!("agents.cockpit.ready").to_string(),
                ShellDeckColors::success(),
            )
        } else {
            (
                session_status_label(session.status),
                status_color(session.status),
            )
        };
        let model = context
            .model
            .clone()
            .filter(|model| !model.trim().is_empty())
            .unwrap_or_else(|| t!("agents.cockpit.model_auto").to_string());
        body = body.child(
            details_section()
                .child(section_heading(t!("agents.cockpit.agent").to_string()))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(10.0))
                        .child(crate::icons::agent_provider_icon(
                            context.provider,
                            18.0,
                            ShellDeckColors::text_primary(),
                        ))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_size(px(13.5))
                                .font_weight(FontWeight::MEDIUM)
                                .child(context.provider.display_name().to_string()),
                        )
                        .child(status_pill(state_label, state_color)),
                )
                .child(details_row(t!("agents.cockpit.model").to_string(), model))
                .child(details_row(
                    t!("agents.cockpit.permissions").to_string(),
                    access_label(context.access),
                ))
                .child(
                    div().flex().child(
                        Button::new(
                            "agent-details-configure",
                            t!("agents.cockpit.configure").to_string(),
                        )
                        .debug_selector(|| "agent-details-configure".into())
                        .variant(ButtonVariant::Link)
                        .size(ButtonSize::Sm)
                        .px(px(0.0))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.context_expanded = true;
                            cx.notify();
                        })),
                    ),
                ),
        );

        // Contexte
        let usage = session.usage.as_ref();
        let ratio = usage.and_then(|usage| match (usage.context_used, usage.context_size) {
            (Some(used), Some(size)) if size > 0 => Some((used as f64 / size as f64).min(1.0)),
            _ => None,
        });
        let mut context_section = details_section().child(
            div()
                .flex()
                .items_center()
                .child(
                    div()
                        .flex_1()
                        .child(section_heading(t!("agents.cockpit.context").to_string())),
                )
                .when_some(ratio, |row, ratio| {
                    row.child(
                        div()
                            .text_size(px(12.5))
                            .text_color(ShellDeckColors::text_muted())
                            .child(format!("{} %", (ratio * 100.0).round() as u32)),
                    )
                }),
        );
        match ratio {
            Some(ratio) => {
                context_section = context_section
                    .child(
                        div()
                            .debug_selector(|| "agent-details-context-meter".into())
                            .h(px(8.0))
                            .w_full()
                            .rounded_full()
                            .bg(ShellDeckColors::hover_bg())
                            .child(
                                div()
                                    .h_full()
                                    .w(relative(ratio.max(0.02) as f32))
                                    .rounded_full()
                                    .bg(ShellDeckColors::primary()),
                            ),
                    )
                    .child(
                        div()
                            .text_size(px(11.5))
                            .text_color(ShellDeckColors::text_muted())
                            .child(t!("agents.cockpit.context_free").to_string()),
                    );
            }
            None => {
                context_section = context_section.child(
                    div()
                        .text_size(px(11.5))
                        .text_color(ShellDeckColors::text_muted())
                        .child(t!("agents.cockpit.context_unknown").to_string()),
                );
            }
        }
        body = body.child(context_section);

        // Vérifications
        let checks = full.map(latest_turn_checks).unwrap_or_default();
        let passed = checks
            .iter()
            .filter(|check| check.status == AgentTraceStatus::Succeeded)
            .count();
        let mut checks_section = details_section().child(
            div()
                .flex()
                .items_center()
                .child(
                    div()
                        .flex_1()
                        .child(section_heading(t!("agents.cockpit.checks").to_string())),
                )
                .when(!checks.is_empty(), |row| {
                    row.child(pill(
                        format!("{passed}/{}", checks.len()),
                        if passed == checks.len() {
                            ShellDeckColors::success()
                        } else {
                            ShellDeckColors::warning()
                        },
                        true,
                    ))
                }),
        );
        if checks.is_empty() {
            checks_section = checks_section.child(
                div()
                    .text_size(px(11.5))
                    .text_color(ShellDeckColors::text_muted())
                    .child(t!("agents.cockpit.checks_empty").to_string()),
            );
        }
        for check in &checks {
            checks_section = checks_section.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.0))
                    .child(check_icon(check.status))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(px(13.0))
                            .child(check.name.clone()),
                    ),
            );
        }
        body = body.child(checks_section);

        // Outils et connexions / Ressources système, folded.
        body = body.child(self.render_details_fold(
            "agent-details-tools",
            "wrench",
            t!("agents.cockpit.tools").to_string(),
            self.tools.details_tools_open,
            |this| &mut this.tools.details_tools_open,
            cx,
        ));
        if self.tools.details_tools_open {
            let mut tools = div().flex().flex_wrap().gap(px(6.0)).pb(px(12.0));
            for (id, icon, key) in [
                ("files", "file-text", "agents.inspector.files"),
                ("review", "git-branch", "agents.inspector.changes"),
                ("terminal", "terminal", "agents.parity.terminal"),
                ("browser", "globe", "agents.live_browser.title"),
                ("instructions", "list-checks", "agents.parity.instructions"),
                ("sources", "link", "agents.parity.sources"),
            ] {
                tools = tools.child(
                    Button::new(
                        SharedString::from(format!("agent-details-tool-{id}")),
                        t!(key).to_string(),
                    )
                    .variant(ButtonVariant::Outline)
                    .size(ButtonSize::Sm)
                    .icon(IconSource::from(icon))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.activate_workspace_tool(id, cx);
                        cx.notify();
                    })),
                );
            }
            body = body.child(tools);
        }
        body = body.child(self.render_details_fold(
            "agent-details-resources",
            "layers",
            t!("agents.cockpit.resources").to_string(),
            self.tools.details_resources_open,
            |this| &mut this.tools.details_resources_open,
            cx,
        ));
        if self.tools.details_resources_open {
            let mut resources = div().flex().flex_col().gap(px(4.0)).pb(px(12.0));
            if let Some(usage) = usage {
                let tokens = usage.input_tokens.unwrap_or(0) + usage.output_tokens.unwrap_or(0);
                if tokens > 0 {
                    resources = resources.child(details_row(
                        t!("agents.cockpit.tokens").to_string(),
                        format!(
                            "{} ↑ · {} ↓",
                            compact_count(usage.input_tokens.unwrap_or(0)),
                            compact_count(usage.output_tokens.unwrap_or(0))
                        ),
                    ));
                }
                if let Some(cost) = &usage.cost_amount {
                    resources = resources.child(details_row(
                        t!("agents.cockpit.cost").to_string(),
                        format!("{cost} {}", usage.cost_currency.clone().unwrap_or_default()),
                    ));
                }
            }
            if let Some(elapsed) = session_elapsed(session) {
                resources = resources.child(details_row(
                    t!("agents.cockpit.duration").to_string(),
                    elapsed,
                ));
            }
            body = body.child(resources);
        }

        div()
            .debug_selector(|| "agent-session-details".into())
            .flex()
            .flex_col()
            .size_full()
            .min_h_0()
            .child(
                div()
                    .flex()
                    .items_center()
                    .flex_shrink_0()
                    .h(px(54.0))
                    .px(px(18.0))
                    .border_b_1()
                    .border_color(ShellDeckColors::border())
                    .child(
                        div()
                            .flex_1()
                            .text_size(px(15.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(t!("agents.cockpit.details_title").to_string()),
                    )
                    .child(
                        Button::new("agent-details-close", "")
                            .debug_selector(|| "agent-details-close".into())
                            .variant(ButtonVariant::Ghost)
                            .size(ButtonSize::Icon)
                            .icon(IconSource::from("x"))
                            .w(px(28.0))
                            .h(px(28.0))
                            .tooltip(t!("agents.cockpit.details_close").to_string())
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.tools.panel_visible = false;
                                this.schedule_library_save(cx);
                                cx.notify();
                            })),
                    ),
            )
            .child(body)
            .into_any_element()
    }

    fn render_details_fold(
        &self,
        id: &'static str,
        icon: &'static str,
        label: String,
        open: bool,
        state: fn(&mut Self) -> &mut bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        div()
            .id(id)
            .debug_selector(move || id.into())
            .flex()
            .items_center()
            .gap(px(10.0))
            .h(px(48.0))
            .border_b_1()
            .border_color(ShellDeckColors::border())
            .cursor_pointer()
            .on_click(cx.listener(move |this, _, _, cx| {
                let open = state(this);
                *open = !*open;
                cx.notify();
            }))
            .child(lucide_icon(icon, 16.0, ShellDeckColors::text_primary()))
            .child(
                div()
                    .flex_1()
                    .text_size(px(13.5))
                    .font_weight(FontWeight::MEDIUM)
                    .child(label),
            )
            .child(lucide_icon(
                if open { "chevron-up" } else { "chevron-down" },
                15.0,
                ShellDeckColors::text_muted(),
            ))
            .into_any_element()
    }
}

impl AgentConsoleView {
    /// The start screen of an empty session (SDUC-595): the robot tile, the
    /// question, the composer with its project and machine choosers, three
    /// starting points and the sessions to pick up again.
    pub(super) fn render_start_screen(
        &self,
        width: f32,
        session: &AgentSession,
        composer: AnyElement,
        setup: Option<AnyElement>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mut column = div()
            .debug_selector(|| "agent-start-screen".into())
            .flex_shrink_0()
            .w(px(width))
            .mx_auto()
            .flex()
            .flex_col()
            .gap(px(18.0))
            .py(px(24.0))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(10.0))
                    .pb(px(8.0))
                    .child(agent_tile(68.0))
                    .child(
                        div()
                            .pt(px(10.0))
                            .text_size(px(28.0))
                            .font_weight(FontWeight::BOLD)
                            .text_color(ShellDeckColors::text_primary())
                            .text_align(TextAlign::Center)
                            .child(t!("agents.empty.title").to_string()),
                    )
                    .child(
                        div()
                            .max_w(px(560.0))
                            .text_size(px(15.0))
                            .text_color(ShellDeckColors::text_muted())
                            .text_align(TextAlign::Center)
                            .child(t!("agents.empty.description").to_string()),
                    ),
            );
        if let Some(setup) = setup {
            column = column.child(setup);
        }
        column = column.child(composer);
        let mut suggestions = div().flex().flex_wrap().gap(px(12.0)).pt(px(4.0));
        for (id, icon, label, prompt) in [
            (
                "agent-suggest-analyze",
                "chart-no-axes-column",
                "agents.cockpit.suggest_analyze",
                "agents.cockpit.prompt_analyze",
            ),
            (
                "agent-suggest-fix",
                "wrench",
                "agents.cockpit.suggest_fix",
                "agents.cockpit.prompt_fix",
            ),
            (
                "agent-suggest-feature",
                "sparkles",
                "agents.cockpit.suggest_feature",
                "agents.cockpit.prompt_feature",
            ),
        ] {
            let session_id = session.id;
            suggestions = suggestions.child(
                div()
                    .id(id)
                    .debug_selector(move || id.into())
                    .flex_1()
                    .min_w(px(180.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .gap(px(10.0))
                    .h(px(52.0))
                    .px(px(14.0))
                    .rounded(px(CARD_RADIUS))
                    .border_1()
                    .border_color(ShellDeckColors::border())
                    .cursor_pointer()
                    .hover(|style| {
                        style
                            .bg(ShellDeckColors::hover_bg())
                            .border_color(ShellDeckColors::primary().opacity(0.4))
                    })
                    .on_click(cx.listener(move |this, _, window, cx| {
                        let Some(ui) = this.session_ui.get(&session_id) else {
                            return;
                        };
                        let prompt_state = ui.prompt.clone();
                        let text = t!(prompt).to_string();
                        prompt_state.update(cx, |state, cx| {
                            state.replace_content(text, cx);
                        });
                        prompt_state.read(cx).focus_handle(cx).focus(window);
                        cx.notify();
                    }))
                    .child(lucide_icon(icon, 17.0, ShellDeckColors::text_primary()))
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .text_size(px(14.0))
                            .child(t!(label).to_string()),
                    ),
            );
        }
        column = column.child(suggestions);

        let mut recent = self
            .sessions
            .library_sessions(false)
            .into_iter()
            .filter(|other| other.id != session.id && !other.messages.is_empty())
            .collect::<Vec<_>>();
        recent.sort_by_key(|other| std::cmp::Reverse(other.updated_at_ms));
        if !recent.is_empty() {
            let mut list = div()
                .flex()
                .flex_col()
                .gap(px(8.0))
                .mt(px(10.0))
                .pt(px(22.0))
                .border_t_1()
                .border_color(ShellDeckColors::border())
                .child(
                    section_heading(t!("agents.cockpit.resume").to_string()).text_size(px(15.0)),
                );
            for other in recent.into_iter().take(3) {
                let id = other.id;
                let project = super::navigator::navigator_worktree(other, &self.project_groups)
                    .and_then(|tree| {
                        self.project_groups
                            .iter()
                            .find(|group| group.worktrees.iter().any(|w| w.id == tree.id))
                            .map(|group| group.label.clone())
                    })
                    .unwrap_or_else(|| {
                        super::navigation_groups::directory_name(
                            &other.context.target,
                            &other.context.workdir,
                        )
                    });
                list = list.child(
                    div()
                        .id(("agent-resume", uuid_key(id)))
                        .debug_selector(move || format!("agent-resume-{id}"))
                        .flex()
                        .items_center()
                        .gap(px(14.0))
                        .px(px(16.0))
                        .py(px(11.0))
                        .rounded(px(CARD_RADIUS))
                        .border_1()
                        .border_color(ShellDeckColors::border())
                        .cursor_pointer()
                        .hover(|style| style.bg(ShellDeckColors::hover_bg()))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.select_session(id, cx);
                        }))
                        .child(lucide_icon(
                            "message-square",
                            18.0,
                            ShellDeckColors::text_muted(),
                        ))
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .flex_1()
                                .min_w_0()
                                .gap(px(2.0))
                                .child(
                                    div()
                                        .truncate()
                                        .text_size(px(14.0))
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .child(other.name.clone()),
                                )
                                .child(
                                    div()
                                        .truncate()
                                        .text_size(px(12.5))
                                        .text_color(ShellDeckColors::text_muted())
                                        .child(project),
                                ),
                        )
                        .child(
                            div()
                                .flex_shrink_0()
                                .text_size(px(12.0))
                                .text_color(ShellDeckColors::text_muted())
                                .child(crate::i18n::rel_time(other.updated_at_ms as f64)),
                        ),
                );
            }
            column = column.child(list);
        }
        column.into_any_element()
    }
}

/// Indent of an agent reply's text under its header (tile + gap).
pub(super) const REPLY_INDENT: f32 = 46.0;

/// « 2 min 06 s », « 45 s », « 1 h 05 min ».
pub(super) fn format_turn_duration(ms: i64) -> String {
    let secs = (ms.max(0) + 500) / 1000;
    if secs < 60 {
        format!("{secs} s")
    } else if secs < 3600 {
        format!("{} min {:02} s", secs / 60, secs % 60)
    } else {
        format!("{} h {:02} min", secs / 3600, (secs % 3600) / 60)
    }
}

impl AgentConsoleView {
    /// The header of a reply, drawn above the first agent message of a turn:
    /// the robot tile, the agent's name and the turn's state and duration.
    pub(super) fn reply_header(&self, message: &AgentMessage, cx: &App) -> Option<AnyElement> {
        if message.role != AgentMessageRole::Agent {
            return None;
        }
        let session = self.sessions.selected()?;
        let first = session.messages.iter().find(|other| {
            other.role == AgentMessageRole::Agent && other.turn_id == message.turn_id
        })?;
        if first.id != message.id {
            return None;
        }
        let provider = message
            .provider
            .unwrap_or(self.displayed_context(cx).provider)
            .display_name()
            .to_string();
        let state = message
            .turn_id
            .and_then(|id| session.turns.iter().find(|turn| turn.id == id))
            .map(
                |turn| match (turn.status.is_active(), turn.finished_at_ms) {
                    (false, Some(finished)) => {
                        let word = if turn.status == AgentSessionStatus::Completed {
                            t!("agents.cockpit.finished").to_string()
                        } else {
                            session_status_label(turn.status)
                        };
                        format!(
                            "{word} • {}",
                            format_turn_duration(finished - turn.started_at_ms)
                        )
                    }
                    _ => session_status_label(turn.status),
                },
            );
        Some(
            div()
                .debug_selector(move || format!("agent-reply-header-{}", message.id))
                .flex()
                .items_center()
                .gap(px(12.0))
                .mb(px(8.0))
                .child(agent_tile(34.0))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .min_w_0()
                        .child(
                            div()
                                .text_size(px(14.0))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(ShellDeckColors::text_primary())
                                .child(provider),
                        )
                        .when_some(state, |column, state| {
                            column.child(
                                div()
                                    .text_size(px(12.0))
                                    .text_color(ShellDeckColors::text_muted())
                                    .child(state),
                            )
                        }),
                )
                .into_any_element(),
        )
    }
}

fn last_turn_reported(session: &AgentSession, kind: fn(&AgentTraceKind) -> bool) -> bool {
    let last_turn = session.turns.last().map(|turn| turn.id);
    last_turn.is_some()
        && session
            .trace
            .iter()
            .rev()
            .find(|trace| kind(&trace.detail))
            .is_some_and(|trace| trace.turn_id == last_turn)
}

impl AgentConsoleView {
    /// Under a finished reply: the files it changed, ready to review, and the
    /// outcome of its checks (SDUC-595).
    pub(super) fn render_turn_summary(
        &self,
        session: &AgentSession,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mut summary = div().flex().flex_col().gap(px(12.0)).min_w_0();
        if session.status.is_active() {
            return summary.into_any_element();
        }
        // Local write turns already get the repository review cards; do not
        // list the same files twice.
        let reviewed = self.review_lists_changes(session.id);
        let files = if !reviewed
            && last_turn_reported(session, |kind| matches!(kind, AgentTraceKind::Diff { .. }))
        {
            latest_turn_file_changes(session)
        } else {
            Vec::new()
        };
        if !files.is_empty() {
            let title = if files.len() == 1 {
                t!("agents.cockpit.file_changed").to_string()
            } else {
                t!("agents.cockpit.files_changed", count = files.len()).to_string()
            };
            let mut card = div()
                .debug_selector(|| "agent-turn-files".into())
                .flex()
                .flex_col()
                .min_w_0()
                .rounded(px(CARD_RADIUS))
                .border_1()
                .border_color(ShellDeckColors::border())
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(10.0))
                        .h(px(48.0))
                        .px(px(14.0))
                        .border_b_1()
                        .border_color(ShellDeckColors::border())
                        .child(lucide_icon("code", 17.0, ShellDeckColors::text_primary()))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_size(px(14.0))
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(title),
                        )
                        .child(pill(
                            t!("agents.cockpit.ready_to_review").to_string(),
                            ShellDeckColors::success(),
                            true,
                        )),
                );
            let mut rows = div().flex().flex_col().px(px(14.0)).py(px(4.0));
            for (index, file) in files.iter().enumerate() {
                let name = std::path::Path::new(&file.path)
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or(&file.path)
                    .to_owned();
                let path = file.path.clone();
                rows = rows.child(
                    div()
                        .id(("agent-turn-file", index))
                        .flex()
                        .items_center()
                        .gap(px(12.0))
                        .h(px(38.0))
                        .when(index > 0, |row| {
                            row.border_t_1()
                                .border_color(ShellDeckColors::border().opacity(0.6))
                        })
                        .tooltip(move |_, cx| {
                            cx.new(|_| super::navigator::NavigatorTooltip(path.clone()))
                                .into()
                        })
                        .child(lucide_icon("file", 16.0, ShellDeckColors::text_muted()))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_size(px(13.5))
                                .child(name),
                        )
                        .child(
                            div()
                                .w(px(52.0))
                                .text_align(TextAlign::Right)
                                .text_size(px(13.0))
                                .text_color(ShellDeckColors::success())
                                .child(format!("+{}", file.additions)),
                        )
                        .child(
                            div()
                                .w(px(52.0))
                                .text_align(TextAlign::Right)
                                .text_size(px(13.0))
                                .text_color(ShellDeckColors::error())
                                .child(format!("−{}", file.deletions)),
                        ),
                );
            }
            card = card.child(rows).child(
                div().px(px(14.0)).pt(px(6.0)).pb(px(14.0)).child(
                    Button::new(
                        "agent-turn-review-changes",
                        t!("agents.cockpit.review_changes").to_string(),
                    )
                    .debug_selector(|| "agent-turn-review-changes".into())
                    .variant(ButtonVariant::Default)
                    .icon(IconSource::from("arrow-right"))
                    .icon_position(adabraka_ui::components::button::IconPosition::End)
                    .h(px(38.0))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.activate_workspace_tool("review", cx);
                        cx.notify();
                    })),
                ),
            );
            summary = summary.child(card);
        }
        if last_turn_reported(session, |kind| matches!(kind, AgentTraceKind::Test { .. })) {
            let checks = latest_turn_checks(session);
            let failed = checks
                .iter()
                .filter(|check| check.status == AgentTraceStatus::Failed)
                .count();
            let finished = checks.iter().all(|check| {
                !matches!(
                    check.status,
                    AgentTraceStatus::Running | AgentTraceStatus::Pending
                )
            });
            if !checks.is_empty() && finished {
                let (status, text) = if failed == 0 {
                    (
                        AgentTraceStatus::Succeeded,
                        t!("agents.cockpit.checks_passed", count = checks.len()).to_string(),
                    )
                } else {
                    (
                        AgentTraceStatus::Failed,
                        t!(
                            "agents.cockpit.checks_failed",
                            failed = failed,
                            count = checks.len()
                        )
                        .to_string(),
                    )
                };
                summary = summary.child(
                    div()
                        .debug_selector(|| "agent-turn-checks".into())
                        .flex()
                        .items_center()
                        .gap(px(12.0))
                        .py(px(4.0))
                        .child(check_icon(status))
                        .child(div().text_size(px(14.0)).child(text))
                        .child(
                            div()
                                .id("agent-turn-checks-results")
                                .debug_selector(|| "agent-turn-checks-results".into())
                                .text_size(px(13.5))
                                .text_color(ShellDeckColors::text_muted())
                                .underline()
                                .cursor_pointer()
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.tools.active = None;
                                    this.inspector_tab = AgentInspectorTab::Details;
                                    this.tools.panel_visible = true;
                                    this.schedule_library_save(cx);
                                    cx.notify();
                                }))
                                .child(t!("agents.cockpit.see_results").to_string()),
                        ),
                );
            }
        }
        summary.into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session_with(traces: &[(u128, AgentTraceKind)], turns: &[u128]) -> AgentSession {
        let context = AgentExecutionContext {
            provider: AgentProvider::Codex,
            target: AgentTarget::Local,
            access: AgentAccessMode::ReadOnly,
            workdir: "/tmp/project".into(),
            model: None,
        };
        let mut collection = AgentSessionCollection::default();
        let id = collection.create("Fixture", context, 1).unwrap();
        let mut session = collection.get(id).unwrap().clone();
        session.turns = turns
            .iter()
            .map(|turn| shelldeck_core::agent_session::AgentTurn {
                id: Uuid::from_u128(*turn),
                context: session.context.clone(),
                started_at_ms: 10,
                finished_at_ms: Some(20),
                status: AgentSessionStatus::Completed,
            })
            .collect();
        session.trace = traces
            .iter()
            .enumerate()
            .map(|(index, (turn, detail))| AgentTraceEvent {
                id: Uuid::new_v4(),
                sequence: index as u64 + 1,
                turn_id: Some(Uuid::from_u128(*turn)),
                at_ms: index as i64,
                correlation_id: None,
                detail: detail.clone(),
            })
            .collect();
        session
    }

    fn diff(
        path: &str,
        additions: u32,
        deletions: u32,
        status: AgentTraceStatus,
    ) -> AgentTraceKind {
        AgentTraceKind::Diff {
            path: path.into(),
            status,
            additions,
            deletions,
            preview: None,
        }
    }

    fn test(name: &str, status: AgentTraceStatus) -> AgentTraceKind {
        AgentTraceKind::Test {
            name: name.into(),
            status,
            summary: None,
        }
    }

    // SDTEST-3194 / SDUC-595: the changed-files card lists the latest editing
    // turn only, one row per path with summed counts, without failed edits.
    #[::core::prelude::v1::test]
    fn sdtest_3194_turn_files_merge_paths_of_the_latest_editing_turn() {
        use AgentTraceStatus::*;
        let session = session_with(
            &[
                (1, diff("old.rs", 9, 9, Succeeded)),
                (2, diff("src/a.rs", 3, 1, Succeeded)),
                (2, diff("src/b.rs", 5, 0, Succeeded)),
                (2, diff("src/a.rs", 2, 4, Succeeded)),
                (2, diff("src/c.rs", 7, 7, Failed)),
            ],
            &[1, 2],
        );
        assert_eq!(
            latest_turn_file_changes(&session),
            vec![
                TurnFileChange {
                    path: "src/a.rs".into(),
                    additions: 5,
                    deletions: 5
                },
                TurnFileChange {
                    path: "src/b.rs".into(),
                    additions: 5,
                    deletions: 0
                },
            ]
        );
        assert!(last_turn_reported(&session, |kind| matches!(
            kind,
            AgentTraceKind::Diff { .. }
        )));
        // A later turn without edits: the card belongs to an older reply and is not shown.
        let later = session_with(&[(1, diff("old.rs", 1, 1, Succeeded))], &[1, 2]);
        assert!(!last_turn_reported(&later, |kind| matches!(
            kind,
            AgentTraceKind::Diff { .. }
        )));
    }

    // SDTEST-3195 / SDUC-595: « Vérifications » shows the tests of the latest
    // turn that ran any, in order, with their own status.
    #[::core::prelude::v1::test]
    fn sdtest_3195_checks_come_from_the_latest_testing_turn() {
        use AgentTraceStatus::*;
        let session = session_with(
            &[
                (1, test("Old", Failed)),
                (2, test("Types", Succeeded)),
                (2, diff("x.rs", 1, 0, Succeeded)),
                (2, test("Tests unitaires", Failed)),
            ],
            &[1, 2],
        );
        let checks = latest_turn_checks(&session);
        assert_eq!(
            checks
                .iter()
                .map(|check| (check.name.as_str(), check.status))
                .collect::<Vec<_>>(),
            vec![("Types", Succeeded), ("Tests unitaires", Failed)]
        );
    }

    // SDTEST-3196 / SDUC-595: reply duration and the sidebar's machine filter.
    #[::core::prelude::v1::test]
    fn sdtest_3196_turn_duration_and_target_filter() {
        assert_eq!(format_turn_duration(126_000), "2 min 06 s");
        assert_eq!(format_turn_duration(44_600), "45 s");
        assert_eq!(format_turn_duration(3_900_000), "1 h 05 min");
        let ssh = AgentTarget::Ssh {
            connection_id: Uuid::nil(),
            connection_label: "infra".into(),
        };
        assert!(NavigatorTargetFilter::All.admits(&ssh));
        assert!(NavigatorTargetFilter::Ssh.admits(&ssh));
        assert!(!NavigatorTargetFilter::Local.admits(&ssh));
        assert!(NavigatorTargetFilter::Local.admits(&AgentTarget::Local));
        assert!(!NavigatorTargetFilter::Ssh.admits(&AgentTarget::Local));
    }
}
