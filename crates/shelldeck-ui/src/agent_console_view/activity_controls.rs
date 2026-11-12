//! Live activity and admission controls share the session collection's capacity.
use super::*;
use std::rc::Rc;

type CompanionOpenHandler = Rc<dyn Fn(&mut App)>;

impl AgentConsoleView {
    pub fn max_concurrent_runs(&self) -> usize {
        self.sessions.max_concurrent()
    }

    fn change_run_limit(&mut self, limit: usize, cx: &mut Context<Self>) {
        if self.connection_repair_storage_ready() && self.sessions.set_max_concurrent(limit).is_ok()
        {
            self.schedule_library_save(cx);
            cx.emit(AgentConsoleEvent::NavigatorChanged);
            cx.notify();
        }
    }

    pub(super) fn render_activity_footer(&self, cx: &Context<Self>) -> AnyElement {
        let parent = cx.entity();
        let unread = self
            .sessions
            .sessions()
            .iter()
            .map(|s| u64::from(s.unread_count))
            .sum::<u64>();
        let label = t!(
            "agents.activity_controls.summary",
            running = self.sessions.active_count(),
            limit = self.max_concurrent_runs()
        )
        .to_string();
        div()
            .h(px(36.0))
            .flex_shrink_0()
            .flex()
            .items_center()
            .gap(px(4.0))
            .px(px(6.0))
            .border_t_1()
            .border_color(ShellDeckColors::border())
            .child(
                div().flex_1().min_w(px(0.0)).child(
                    Popover::new("agent-activity-menu")
                        .anchor(Corner::BottomLeft)
                        .trigger(
                            Button::new("agent-activity-trigger", label)
                                .debug_selector(|| "agent-activity-trigger".into())
                                .variant(ButtonVariant::Ghost)
                                .size(ButtonSize::Sm)
                                .icon("activity")
                                .w_full()
                                .tooltip(t!("agents.activity_controls.title").to_string()),
                        )
                        .content(move |window, cx| {
                            let parent = parent.clone();
                            cx.new(move |cx| {
                                cx.observe(&parent, |_, _, cx| cx.notify()).detach();
                                PopoverContent::new(window, cx, move |_, cx| {
                                    parent.read(cx).render_activity_controls(&parent, cx, None)
                                })
                            })
                        }),
                ),
            )
            .when(unread > 0, |row| {
                row.child(Badge::new(unread.to_string()).variant(BadgeVariant::Secondary))
            })
            .child(
                Button::new("agent-footer-settings", "")
                    .debug_selector(|| "agent-footer-settings".into())
                    .variant(ButtonVariant::Ghost)
                    .size(ButtonSize::Icon)
                    .icon("settings")
                    .w(px(28.0))
                    .h(px(28.0))
                    .tooltip(t!("settings.title").to_string())
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(AgentConsoleEvent::OpenSettings))),
            )
            .into_any_element()
    }

    /// What the conversations want from a user who is looking elsewhere:
    /// one waits on them, or one has replies they have not read.
    pub fn attention(&self) -> crate::view_attention::ViewAttention {
        use crate::view_attention::ViewAttention;
        use shelldeck_core::agent_session::AgentSessionAttention;
        self.sessions
            .sessions()
            .iter()
            .map(|session| match session.attention {
                AgentSessionAttention::NeedsAttention => ViewAttention::NeedsYou,
                AgentSessionAttention::Unread => ViewAttention::Finished,
                AgentSessionAttention::None if session.unread_count > 0 => ViewAttention::Finished,
                AgentSessionAttention::None => ViewAttention::None,
            })
            .max()
            .unwrap_or_default()
    }

    pub(crate) fn active_run_count(&self) -> usize {
        self.sessions.active_count()
    }

    pub(crate) fn render_companion_activity(
        &self,
        parent: &Entity<Self>,
        cx: &App,
        on_open: CompanionOpenHandler,
    ) -> AnyElement {
        self.render_activity_controls(parent, cx, Some(on_open))
    }

    fn render_activity_controls(
        &self,
        parent: &Entity<Self>,
        _cx: &App,
        on_open: Option<CompanionOpenHandler>,
    ) -> AnyElement {
        let embedded = on_open.is_some();
        let limit = self.max_concurrent_runs();
        let decrease = parent.clone();
        let increase = parent.clone();
        let mut rows = div()
            .id("agent-activity-rows")
            .max_h(px(260.0))
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap(px(4.0));
        let mut count = 0;
        for session in self
            .sessions
            .sessions()
            .iter()
            .filter(|s| s.status.is_active() || s.unread_count > 0)
        {
            count += 1;
            let id = session.id;
            let open = parent.clone();
            let selected = on_open.clone();
            let mut row = div().flex().items_center().gap(px(4.0)).child(
                div().flex_1().min_w(px(0.0)).child(
                    Button::new(
                        SharedString::from(format!("agent-activity-open-{id}")),
                        session.name.clone(),
                    )
                    .debug_selector(move || format!("agent-activity-open-{id}"))
                    .variant(ButtonVariant::Ghost)
                    .size(ButtonSize::Sm)
                    .icon(if session.status.is_active() {
                        "activity"
                    } else {
                        "messages-square"
                    })
                    .w_full()
                    .tooltip(session.name.clone())
                    .on_click(move |_, _, cx| {
                        open.update(cx, |view, cx| {
                            view.select_session(id, cx);
                        });
                        if let Some(selected) = &selected {
                            selected(cx);
                        }
                    }),
                ),
            );
            if session.unread_count > 0 {
                row = row.child(
                    Badge::new(session.unread_count.to_string()).variant(BadgeVariant::Secondary),
                );
            }
            if let Some(run_id) = self
                .run_sessions
                .iter()
                .find_map(|(run, owner)| (*owner == id).then_some(*run))
            {
                let stop = parent.clone();
                row = row.child(
                    Button::new(SharedString::from(format!("agent-activity-stop-{id}")), "")
                        .debug_selector(move || format!("agent-activity-stop-{id}"))
                        .variant(ButtonVariant::Ghost)
                        .size(ButtonSize::Icon)
                        .icon("square")
                        .w(px(26.0))
                        .h(px(26.0))
                        .disabled(session.status == AgentSessionStatus::Stopping)
                        .tooltip(t!("agents.stop").to_string())
                        .on_click(move |_, _, cx| {
                            stop.update(cx, |view, cx| view.request_stop(run_id, cx))
                        }),
                );
            }
            rows = rows.child(row);
        }
        if count == 0 {
            rows = rows.child(
                div()
                    .py(px(10.0))
                    .text_size(px(12.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(t!("agents.activity_controls.empty").to_string()),
            );
        }
        div()
            .w(px(290.0))
            .when(embedded, |panel| panel.w_full())
            .max_w_full()
            .flex()
            .flex_col()
            .gap(px(10.0))
            .child(
                div()
                    .text_size(px(13.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(t!("agents.activity_controls.title").to_string()),
            )
            .child(rows)
            .child(
                div()
                    .border_t_1()
                    .border_color(ShellDeckColors::border())
                    .pt(px(10.0))
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(
                        div()
                            .flex_1()
                            .text_size(px(12.0))
                            .child(t!("agents.activity_controls.limit").to_string()),
                    )
                    .child(
                        Button::new("agent-limit-decrease", "")
                            .debug_selector(|| "agent-limit-decrease".into())
                            .variant(ButtonVariant::Ghost)
                            .size(ButtonSize::Icon)
                            .icon("minus")
                            .w(px(26.0))
                            .h(px(26.0))
                            .disabled(limit <= 1 || !self.connection_repair_storage_ready())
                            .tooltip(t!("agents.activity_controls.decrease").to_string())
                            .on_click(move |_, _, cx| {
                                decrease.update(cx, |view, cx| {
                                    view.change_run_limit(
                                        view.max_concurrent_runs().saturating_sub(1),
                                        cx,
                                    )
                                })
                            }),
                    )
                    .child(div().text_size(px(13.0)).child(limit.to_string()))
                    .child(
                        Button::new("agent-limit-increase", "")
                            .debug_selector(|| "agent-limit-increase".into())
                            .variant(ButtonVariant::Ghost)
                            .size(ButtonSize::Icon)
                            .icon("plus")
                            .w(px(26.0))
                            .h(px(26.0))
                            .disabled(
                                limit >= MAX_CONCURRENT_AGENT_SESSIONS
                                    || !self.connection_repair_storage_ready(),
                            )
                            .tooltip(t!("agents.activity_controls.increase").to_string())
                            .on_click(move |_, _, cx| {
                                increase.update(cx, |view, cx| {
                                    view.change_run_limit(view.max_concurrent_runs() + 1, cx)
                                })
                            }),
                    ),
            )
            .child(
                div()
                    .text_size(px(11.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(t!("agents.activity_controls.limit_hint").to_string()),
            )
            .into_any_element()
    }
}
