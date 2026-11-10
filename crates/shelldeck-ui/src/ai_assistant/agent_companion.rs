use super::*;
use crate::agent_console_view::AgentConsoleView;

impl AiAssistantView {
    pub(crate) fn set_shared_agent_console(
        &mut self,
        agents: Option<Entity<AgentConsoleView>>,
        cx: &mut Context<Self>,
    ) {
        if self.shared_agents == agents {
            return;
        }
        self.shared_agent_subscription = None;
        if let Some(previous) = self.shared_agents.take() {
            previous.update(cx, |view, cx| view.set_surface_visible(false, cx));
        }
        self.shared_agents = agents;
        if let Some(agents) = &self.shared_agents {
            agents.update(cx, |view, cx| {
                view.prepare_window_host(cx);
                view.set_surface_visible(self.active_tab == AiActivity::Chat, cx);
            });
            self.shared_agent_subscription = Some(cx.observe(agents, |_, _, cx| cx.notify()));
        }
        self.backend_menu_open = false;
        cx.notify();
    }

    pub(crate) fn has_shared_agents(&self) -> bool {
        self.shared_agents.is_some()
    }
    pub(crate) fn shared_agent_chat_visible(&self) -> bool {
        self.shared_agents.is_some() && self.active_tab == AiActivity::Chat
    }
    pub(crate) fn open_main_window(&self, cx: &mut Context<Self>) {
        cx.emit(AiAssistantEvent::OpenMainWindow);
    }

    pub(super) fn set_companion_activity(&mut self, activity: AiActivity, cx: &mut Context<Self>) {
        self.active_tab = activity;
        if let Some(agents) = &self.shared_agents {
            agents.update(cx, |view, cx| {
                view.set_surface_visible(activity == AiActivity::Chat, cx);
                if activity == AiActivity::Chat {
                    view.request_surface_focus(cx);
                }
            });
        }
        cx.notify();
    }

    pub(super) fn show_agent_conversations(&mut self, cx: &mut Context<Self>) {
        self.set_companion_activity(AiActivity::Chat, cx);
        if let Some(agents) = &self.shared_agents {
            agents.update(cx, |view, cx| {
                if !view.navigator_visible() {
                    view.toggle_navigator(cx);
                }
            });
        }
    }

    pub(super) fn render_agent_companion(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let panel = match self.active_tab {
            AiActivity::Chat => self
                .shared_agents
                .as_ref()
                .unwrap()
                .clone()
                .into_any_element(),
            AiActivity::Clippy => self.render_clippy(window, cx),
            AiActivity::Tasks => {
                let agents = self.shared_agents.as_ref().unwrap();
                let parent = cx.entity();
                let on_open = Rc::new(move |cx: &mut App| {
                    parent.update(cx, |view, cx| {
                        view.set_companion_activity(AiActivity::Chat, cx)
                    });
                });
                let activity = agents
                    .read(cx)
                    .render_companion_activity(agents, cx, on_open);
                let mut tasks = div().flex().flex_col().size_full().min_h_0().child(
                    div()
                        .id("companion-agent-activity")
                        .flex_1()
                        .min_h_0()
                        .p(px(12.0))
                        .overflow_y_scroll()
                        .child(activity),
                );
                if !self.tasks.is_empty() {
                    tasks = tasks.child(
                        div()
                            .flex_1()
                            .min_h_0()
                            .child(self.render_tasks(window, cx)),
                    );
                }
                tasks.into_any_element()
            }
            AiActivity::History => self.render_history(false, window, cx),
            AiActivity::LegacyChat => {
                unreachable!("legacy archive uses the retained helper renderer")
            }
        };
        let active_tasks = self
            .tasks
            .iter()
            .filter(|task| task.status.is_active())
            .count();
        let root = div()
            .flex()
            .size_full()
            .min_w_0()
            .min_h(px(0.0))
            .overflow_hidden()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .min_h(px(0.0))
                    .overflow_hidden()
                    // The retained console contains opaque child panes. Keep them
                    // clear of the native corner instead of relying on ancestor clipping.
                    .when(
                        self.active_tab == AiActivity::Chat && self.dock_corners.0,
                        |body| body.pb(use_theme().tokens.radius_xl),
                    )
                    .child(panel),
            )
            .child(self.render_rail(active_tasks, cx));
        self.render_overlay_panels(root, false, cx)
            .into_any_element()
    }
}
