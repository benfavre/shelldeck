//! Catalog mutation boundary for explicit saved-connection repair.
use super::*;

impl WorkspaceHubView {
    pub(in crate::workspace) fn ssh_checkout_repair_scope(
        &self,
        checkout: CatalogCheckoutId,
        expected: Uuid,
    ) -> Result<(CatalogProjectId, String), String> {
        let blocked = || t!("agents.project.reconnect_blocked").to_string();
        if self.load_error.is_some() || self.recovery_pending {
            return Err(blocked());
        }
        let (project, record) = self
            .catalog
            .projects()
            .find_map(|project| {
                project
                    .checkouts()
                    .find(|item| item.id() == checkout)
                    .map(|record| (project.id(), record))
            })
            .ok_or_else(blocked)?;
        let CheckoutHost::Ssh {
            connection_id,
            root,
        } = record.host()
        else {
            return Err(blocked());
        };
        if *connection_id != expected
            || self.catalog.workspaces().any(|workspace| workspace.checkout_id() == checkout)
            || self.pending_requests.values().filter(|request| request.source_checkout == checkout || request.checkout == checkout).any(|request| {
                !matches!(self.creation.state(request.workspace),
                    Some(BackgroundWorkspaceCreateState::Cancelled { operation, .. }
                        | BackgroundWorkspaceCreateState::Conflict { operation, .. }
                        | BackgroundWorkspaceCreateState::Failed { operation, .. }) if *operation == request.operation)
            }) {
            return Err(blocked());
        }
        Ok((project, root.as_str().to_owned()))
    }

    pub(in crate::workspace) fn rebind_ssh_checkout(
        &mut self,
        checkout: CatalogCheckoutId,
        expected: Uuid,
        selected: Uuid,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        let (project, _) = self.ssh_checkout_repair_scope(checkout, expected)?;
        if !self.connections.contains_key(&selected) {
            return Err(t!("agents.project.reconnect_blocked").to_string());
        }
        mutate_and_save(&mut self.catalog, |catalog| {
            catalog
                .rebind_ssh_checkout(project, checkout, expected, selected)
                .map_err(|error| error.to_string())
        })?;
        let settled: Vec<_> = self
            .pending_requests
            .values()
            .filter(|request| request.source_checkout == checkout || request.checkout == checkout)
            .map(|request| (request.workspace, request.operation))
            .collect();
        for (workspace, operation) in settled {
            self.dismiss_create(workspace, operation, cx);
        }
        self.refresh_checkout_select(self.launcher.checkout, cx);
        self.error = None;
        cx.emit(WorkspaceHubEvent::CatalogChanged);
        cx.notify();
        Ok(())
    }
}
