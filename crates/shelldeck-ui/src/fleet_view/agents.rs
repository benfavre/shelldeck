//! Reuse the retained Platform cockpit inside Agents without mounting Fleet's navigation.
use super::*;

impl FleetView {
    pub(crate) fn agent_sessions(&self) -> &[SessionRecord] {
        self.snapshot
            .as_ref()
            .map_or(&[], |snapshot| snapshot.sessions.as_slice())
    }

    pub(crate) fn agent_sessions_loading(&self) -> bool {
        self.loading
    }

    pub(crate) fn agent_sessions_error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    pub(crate) fn select_agent_session(
        &mut self,
        coordinate: &ResourceCoordinate,
        cx: &mut Context<Self>,
    ) -> bool {
        // Use the full authority/kind/id, including for observation-only sessions.
        if self
            .agent_sessions()
            .iter()
            .filter(|record| &record.session.resource == coordinate)
            .count()
            != 1
        {
            return false;
        }
        self.select_session(coordinate, cx);
        cx.notify();
        true
    }

    pub(crate) fn render_agent_session(&self, cx: &mut Context<Self>) -> AnyElement {
        if self.tasks.open {
            return div()
                .relative()
                .flex_1()
                .min_w(px(0.0))
                .min_h(px(0.0))
                .flex()
                .flex_col()
                .child(self.render_task_toolbar(cx))
                .child(self.render_tasks(cx))
                .children(self.task_confirmation())
                .into_any_element();
        }
        let mut content = div()
            .flex_1()
            .min_w(px(0.0))
            .min_h(px(0.0))
            .overflow_hidden()
            .flex()
            .flex_col();
        if let Some(error) = &self.error {
            content = content.child(
                Alert::error()
                    .title(t!("fleet.error.title").to_string())
                    .description(error.clone()),
            );
        }
        if let Some((outcome, explanation)) = &self.refusal {
            content = content.child(
                Alert::error()
                    .title(t!("fleet.action.refused", outcome = outcome).to_string())
                    .description(explanation.clone()),
            );
        }
        let selected = self.agent_sessions().iter().find(|record| {
            self.selected_session.as_deref() == Some(record.session.resource.id.as_str())
        });
        if let Some(record) = selected {
            content = content.child(div().flex_shrink_0().child(self.render_session(record, cx)));
            if self.cockpit.pane(&record.session.resource).is_some() {
                content = content.child(self.render_selected_pane(cx));
            } else {
                content = content.child(
                    div()
                        .flex_1()
                        .min_h(px(0.0))
                        .flex()
                        .items_center()
                        .justify_center()
                        .p(px(24.0))
                        .text_size(px(12.0))
                        .text_color(ShellDeckColors::text_muted())
                        .child(if record.attachable {
                            t!("fleet.pane.attach_to_read").to_string()
                        } else {
                            t!("agents.remote.cannot_observe").to_string()
                        }),
                );
            }
        } else {
            content = content.child(
                div()
                    .p(px(24.0))
                    .text_size(px(12.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(t!("agents.remote.unavailable").to_string()),
            );
        }
        div()
            .flex_1()
            .min_w(px(0.0))
            .min_h(px(0.0))
            .flex()
            .flex_col()
            .child(self.render_task_toolbar(cx))
            .child(self.render_action_preview(true, cx))
            .child(content)
            .into_any_element()
    }
}
