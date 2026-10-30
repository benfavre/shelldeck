//! Explicit project-host repair stages idle conversation changes before catalog persistence.
use super::*;

pub(crate) struct AgentConnectionRepair {
    sessions: Vec<AgentSession>,
}

fn matches_checkout(context: &AgentExecutionContext, connection: Uuid, root: &str) -> bool {
    context.workdir == root
        && matches!(context.target, AgentTarget::Ssh { connection_id, .. } if connection_id == connection)
}

impl AgentConsoleView {
    pub(crate) fn prepare_ssh_project_rebind(
        &self,
        previous: Uuid,
        root: &str,
        replacement: AgentTarget,
        cx: &App,
    ) -> Result<AgentConnectionRepair, String> {
        if !self.connection_repair_storage_ready() {
            return Err(t!("agents.project.reconnect_blocked").to_string());
        }
        validate_agent_target(&replacement, &self.connections)?;
        if !matches!(replacement, AgentTarget::Ssh { .. }) {
            return Err(t!("agents.project.reconnect_blocked").to_string());
        }
        let mut repaired = Vec::new();
        for session in self
            .sessions
            .sessions()
            .iter()
            .filter(|session| matches_checkout(&session.context, previous, root))
        {
            if session.status.is_active()
                || self.context_locks.contains(&session.id)
                || self
                    .session_ui
                    .get(&session.id)
                    .is_some_and(|ui| ui.run_id.is_some())
                || self
                    .pending_confirm
                    .as_ref()
                    .is_some_and(|pending| pending.session_id == session.id)
            {
                return Err(t!("agents.project.reconnect_blocked").to_string());
            }
            let mut next = session.clone();
            let mut context = if self.sessions.selected_id() == Some(session.id) {
                let displayed = self.displayed_context(cx);
                // Do not discard unsent edits that moved the selected draft to
                // another target/folder since the project menu was rendered.
                if !matches_checkout(&displayed, previous, root) {
                    return Err(t!("agents.project.reconnect_blocked").to_string());
                }
                displayed
            } else {
                session.context.clone()
            };
            context.target = replacement.clone();
            self.validate_native_context_transition(session.id, &context)?;
            next.set_context(context, now_ms())
                .map_err(|error| error.to_string())?;
            repaired.push(next);
        }
        Ok(AgentConnectionRepair { sessions: repaired })
    }

    /// The caller applies this synchronously immediately after saving the catalog.
    pub(crate) fn apply_ssh_project_rebind(
        &mut self,
        plan: AgentConnectionRepair,
        cx: &mut Context<Self>,
    ) {
        let mut selected = false;
        for repaired in plan.sessions {
            selected |= self.sessions.selected_id() == Some(repaired.id);
            if let Some(session) = self.sessions.get_mut(repaired.id) {
                *session = repaired;
            }
        }
        if selected {
            self.model_discovery = model::AgentModelDiscoveryUi::new();
            self.inspector_remote_request = Uuid::new_v4();
            self.apply_selected_context(cx);
        }
        // Prompt InputState entities and composer attachments are deliberately retained.
        self.schedule_library_save(cx);
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::{
        AgentConnectionOption, AgentConsoleView, AgentSessionStatus, AgentStreamEvent, AgentTarget,
    };
    use gpui::{AppContext, TestAppContext};
    use uuid::Uuid;

    // SDTEST-2737 — explicit repair preserves transcript/draft, resets resume and refuses live/bound sessions.
    #[test]
    fn project_connection_repair_preserves_conversation_and_refuses_active_authority() {
        let mut app = TestAppContext::single();
        let view = app.update(|cx| cx.new(AgentConsoleView::new));
        view.update(&mut app, |view, cx| {
            let old = Uuid::new_v4();
            let new = Uuid::new_v4();
            let replacement = AgentTarget::Ssh { connection_id: new, connection_label: "Selected host".into() };
            view.set_connections(vec![AgentConnectionOption { id: new, label: "Selected host".into(), host: "fixture.invalid".into() }], cx);
            assert!(!view.connection_repair_storage_ready());
            assert!(view.prepare_ssh_project_rebind(old, "/fixture/project", replacement.clone(), cx).is_err());
            // Supply an already-loaded empty fixture library, dropping the
            // constructor's pending disk restore. The production guard remains.
            view.library = super::AgentLibraryUi::new(cx);
            assert!(view.connection_repair_storage_ready());
            let id = view.sessions.selected_id().unwrap();
            let session = view.sessions.get_mut(id).unwrap();
            let mut context = session.context.clone();
            context.target = AgentTarget::Ssh { connection_id: old, connection_label: "Missing host".into() };
            context.workdir = "/fixture/project".into();
            session.set_context(context, 1).unwrap();
            session.apply_stream_event(AgentStreamEvent::Text("Keep transcript".into()), 2, true);
            session.apply_stream_event(AgentStreamEvent::Session("old-provider-resume".into()), 3, true);
            assert!(session.run_request("next").resume_session.is_some());
            view.apply_selected_context(cx);
            view.session_ui.get(&id).unwrap().prompt.update(cx, |input, cx| input.replace_content("Keep unsent draft", cx));
            view.context_locks.insert(id);
            assert!(view.prepare_ssh_project_rebind(old, "/fixture/project", replacement.clone(), cx).is_err());
            view.context_locks.remove(&id);
            view.sessions.get_mut(id).unwrap().status = AgentSessionStatus::Running;
            assert!(view.prepare_ssh_project_rebind(old, "/fixture/project", replacement.clone(), cx).is_err());
            view.sessions.get_mut(id).unwrap().status = AgentSessionStatus::Idle;
            let plan = view.prepare_ssh_project_rebind(old, "/fixture/project", replacement.clone(), cx).unwrap();
            assert!(matches!(view.sessions.get(id).unwrap().context.target, AgentTarget::Ssh { connection_id, .. } if connection_id == old));
            view.apply_ssh_project_rebind(plan, cx);
            let session = view.sessions.get(id).unwrap();
            assert_eq!(session.context.target, replacement);
            assert_eq!(session.context.workdir, "/fixture/project");
            assert!(session.messages.iter().any(|message| message.text == "Keep transcript"));
            assert!(session.run_request("next").resume_session.is_none());
            assert_eq!(view.session_ui.get(&id).unwrap().prompt.read(cx).content(), "Keep unsent draft");
            assert_eq!(view.selected_connection, Some(new));
        });
    }
}
