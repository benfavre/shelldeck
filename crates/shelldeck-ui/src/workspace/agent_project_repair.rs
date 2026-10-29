//! User-selected SSH project repair. This performs no remote operation; only local catalog and conversation metadata change.
use super::*;
use shelldeck_core::agent_runtime::AgentTarget;
use shelldeck_core::config::workspace_catalog::CatalogCheckoutId;

impl Workspace {
    pub(super) fn reconnect_agent_project(
        &mut self,
        checkout: CatalogCheckoutId,
        expected: Uuid,
        selected: Uuid,
        cx: &mut Context<Self>,
    ) {
        if !self.can_access_mode(AppMode::Dev) || self.effective_mode() != AppMode::Dev {
            return;
        }
        let result = (|| -> Result<(), String> {
            let connection = self
                .connections
                .iter()
                .find(|connection| connection.id == selected)
                .ok_or_else(|| t!("agents.project.reconnect_blocked").to_string())?;
            let replacement = AgentTarget::Ssh {
                connection_id: selected,
                connection_label: connection.display_name().to_owned(),
            };
            let (_, root) = self
                .workspace_hub
                .read(cx)
                .ssh_checkout_repair_scope(checkout, expected)?;
            let plan = self.agent_console.read(cx).prepare_ssh_project_rebind(
                expected,
                &root,
                replacement,
                cx,
            )?;
            // No await or event-loop yield between staging, catalog CAS save and
            // applying session contexts. Failed persistence leaves conversations untouched.
            self.workspace_hub.update(cx, |hub, cx| {
                hub.rebind_ssh_checkout(checkout, expected, selected, cx)
            })?;
            self.agent_console
                .update(cx, |view, cx| view.apply_ssh_project_rebind(plan, cx));
            self.refresh_agent_projects(cx);
            Ok(())
        })();
        match result {
            Ok(()) => self.show_toast(
                t!("agents.project.reconnected").to_string(),
                ToastLevel::Success,
                cx,
            ),
            Err(error) => self.show_toast(error, ToastLevel::Warning, cx),
        }
    }
}
