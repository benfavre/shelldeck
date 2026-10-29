//! Explicit catalog-only removal. Runtime and filesystem ownership stay untouched.
use super::*;

fn settled_attempt_matches(
    state: Option<&BackgroundWorkspaceCreateState>,
    operation: CreationOperationId,
) -> bool {
    matches!(state,
        Some(BackgroundWorkspaceCreateState::Cancelled { operation: current, .. }
            | BackgroundWorkspaceCreateState::Failed { operation: current, .. }
            | BackgroundWorkspaceCreateState::Conflict { operation: current, .. })
        if *current == operation)
}

impl WorkspaceHubView {
    pub(super) fn can_remove_project(&self, project: CatalogProjectId) -> bool {
        self.load_error.is_none()
            && !self.recovery_pending
            && self.catalog.projects().any(|record| record.id() == project)
            && !self
                .catalog
                .workspaces()
                .any(|workspace| workspace.project_id() == project)
            && self
                .pending_requests
                .values()
                .filter(|request| request.project == project)
                .all(|request| {
                    settled_attempt_matches(
                        self.creation.state(request.workspace),
                        request.operation,
                    )
                })
    }

    pub(super) fn remove_project(&mut self, project: CatalogProjectId, cx: &mut Context<Self>) {
        // Revalidate at dispatch: an older card cannot remove newly attached data.
        if !self.can_remove_project(project) {
            self.error = Some(t!("workspaces.projects.remove_blocked").to_string());
            cx.notify();
            return;
        }
        if let Err(error) = mutate_and_save(&mut self.catalog, |catalog| {
            catalog
                .remove_empty_project(project)
                .map_err(|error| error.to_string())
        }) {
            self.error = Some(error);
            cx.notify();
            return;
        }
        // Only forget settled attempts after the catalog write succeeds. Dismiss
        // retains reducer operation tombstones, rejecting delayed executor events.
        let attempts: Vec<_> = self
            .pending_requests
            .values()
            .filter(|request| request.project == project)
            .map(|request| (request.workspace, request.operation))
            .collect();
        for (workspace, operation) in attempts {
            self.dismiss_create(workspace, operation, cx);
        }
        self.refresh_checkout_select(self.launcher.checkout, cx);
        self.error = None;
        cx.emit(WorkspaceHubEvent::CatalogChanged);
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::{
        mutate_and_persist, settled_attempt_matches, BackgroundWorkspaceCreateState,
        CatalogProjectId, CreationOperationId, ProjectCatalog, ProjectRecord,
    };
    use shelldeck_core::workspace_navigation::WorkspaceCreateProgress;

    // SDTEST-2728 — only exact settled attempts may be forgotten; failed saves roll back metadata.
    #[test]
    fn removal_refuses_live_or_unknown_attempts_and_rolls_back_failed_persistence() {
        let operation = CreationOperationId::new();
        let cancelled = BackgroundWorkspaceCreateState::Cancelled {
            operation,
            catalog_revision: 1,
        };
        assert!(settled_attempt_matches(Some(&cancelled), operation));
        assert!(!settled_attempt_matches(
            Some(&cancelled),
            CreationOperationId::new()
        ));
        assert!(!settled_attempt_matches(None, operation));
        for state in [
            BackgroundWorkspaceCreateState::Running {
                operation,
                catalog_revision: 1,
                progress: WorkspaceCreateProgress::default(),
            },
            BackgroundWorkspaceCreateState::Cancelling {
                operation,
                catalog_revision: 1,
                progress: WorkspaceCreateProgress::default(),
            },
            BackgroundWorkspaceCreateState::Completed {
                operation,
                catalog_revision: 1,
            },
        ] {
            assert!(!settled_attempt_matches(Some(&state), operation));
        }
        let mut catalog = ProjectCatalog::default();
        let project = CatalogProjectId::new();
        catalog
            .insert_project(ProjectRecord::new(project, "Empty project"))
            .unwrap();
        let before = catalog.clone();
        let result = mutate_and_persist(
            &mut catalog,
            |catalog| {
                catalog
                    .remove_empty_project(project)
                    .map_err(|error| error.to_string())
            },
            |_| Err("fixture write refusal".into()),
        );
        assert!(result.is_err());
        assert_eq!(catalog, before);
    }
}
