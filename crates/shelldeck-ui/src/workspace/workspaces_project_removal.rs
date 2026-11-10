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
    pub(in crate::workspace) fn rename_project_from_navigator(
        &mut self,
        id: &str,
        name: &str,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        if self.load_error.is_some() || self.recovery_pending {
            return Err(t!("agents.project.unavailable").to_string());
        }
        let project = self
            .catalog
            .projects()
            .find(|project| project.id().to_string() == id)
            .map(|project| project.id())
            .ok_or_else(|| t!("agents.project.unavailable").to_string())?;
        mutate_and_save(&mut self.catalog, |catalog| {
            catalog
                .rename_project(project, name)
                .map_err(|error| error.to_string())
        })?;
        cx.emit(WorkspaceHubEvent::CatalogChanged);
        cx.notify();
        Ok(())
    }

    pub(in crate::workspace) fn remove_project_from_navigator(
        &mut self,
        project: CatalogProjectId,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        self.remove_project_from_navigator_with_persist(
            project,
            |catalog| catalog.save().map_err(|error| error.to_string()),
            cx,
        )
    }

    pub(super) fn remove_project_from_navigator_with_persist(
        &mut self,
        project: CatalogProjectId,
        persist: impl FnOnce(&mut ProjectCatalog) -> Result<(), String>,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        if self.load_error.is_some() || self.recovery_pending {
            return Err(t!("agents.project.unavailable").to_string());
        }
        // Retained workspaces do not prevent sidebar removal. Only an unsettled
        // creation still needs its visible project until the executor finishes.
        if self
            .pending_requests
            .values()
            .filter(|request| request.project == project)
            .any(|request| {
                !settled_attempt_matches(self.creation.state(request.workspace), request.operation)
                    && !matches!(self.creation.state(request.workspace),
                        Some(BackgroundWorkspaceCreateState::Completed { operation, .. })
                        if *operation == request.operation
                            && self.catalog.workspace(request.workspace).is_ok())
            })
        {
            return Err(t!("agents.project.creating").to_string());
        }
        mutate_and_persist(
            &mut self.catalog,
            |catalog| {
                catalog
                    .set_project_navigator_hidden(project, true)
                    .map_err(|error| error.to_string())
            },
            persist,
        )?;
        self.error = None;
        cx.emit(WorkspaceHubEvent::CatalogChanged);
        cx.notify();
        Ok(())
    }

    pub(in crate::workspace) fn restore_navigator_project(
        &mut self,
        checkout: CatalogCheckoutId,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        let project = self
            .catalog
            .projects()
            .find(|project| {
                project.hidden_from_navigator()
                    && project.checkouts().any(|record| record.id() == checkout)
            })
            .map(ProjectRecord::id);
        if let Some(project) = project {
            if self.load_error.is_some() || self.recovery_pending {
                return Err(t!("agents.project.unavailable").to_string());
            }
            if let Err(error) = mutate_and_save(&mut self.catalog, |catalog| {
                catalog
                    .set_project_navigator_hidden(project, false)
                    .map_err(|error| error.to_string())
            }) {
                self.error = Some(error.clone());
                cx.notify();
                return Err(error);
            }
            self.error = None;
            cx.emit(WorkspaceHubEvent::CatalogChanged);
            cx.notify();
        }
        Ok(())
    }

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
        let result = mutate_and_persist(
            &mut catalog,
            |catalog| {
                catalog
                    .rename_project(project, "Renamed project")
                    .map_err(|error| error.to_string())
            },
            |_| Err("fixture write refusal".into()),
        );
        assert!(result.is_err());
        assert_eq!(
            catalog, before,
            "a failed rename write must retain the original name and revision"
        );
    }
}
