use super::*;

#[cfg(test)]
mod tests {
    use super::{
        mutate_and_persist, workspace_card_presentation, AuthorizedLaunchHost,
        ExistingFolderWorkspaceExecutor, LauncherIntakeKind, ProviderCardObservation,
        WorkspaceCardAggregator, WorkspaceExecutionRequest, WorkspaceHubView,
        WorkspaceLaunchExecutor, WorkspaceLaunchMode, WorkspaceLauncherDraft,
        WorkspaceTerminalConfig,
    };
    use crate::t;
    use crate::terminal_view::TerminalView;
    use gpui::{AppContext, TestAppContext};
    use shelldeck_core::config::themes::TerminalTheme;
    use shelldeck_core::config::workspace_catalog::{
        CatalogCheckoutId, CatalogProjectId, CatalogWorkspaceId, CheckoutHost, ExternalWorkItem,
        ExternalWorkItemKind, PlatformContextRef, PlatformMappingReconciliation, PlatformV2Mapping,
        ProjectCatalog, ProjectCheckout, ProjectRecord, RepositoryIdentity, WorkspaceLaunchIntake,
        WorkspaceLaunchRequest,
    };
    use shelldeck_core::workspace_navigation::{
        BackgroundWorkspaceCreateState, CreationOperationId, GitDirtyState, WorkspaceAgentState,
        WorkspaceCardState, WorkspaceCreateConflict, WorkspaceCreateEvent, WorkspaceCreateFailure,
        WorkspaceCreateFailureKind, WorkspaceCreatePhase, WorkspaceCreateProgress,
        WorkspaceFreshness,
    };
    use std::collections::HashMap;
    use tokio::sync::mpsc;
    use uuid::Uuid;

    fn fixture_catalog() -> (
        ProjectCatalog,
        CatalogWorkspaceId,
        CatalogWorkspaceId,
        CatalogCheckoutId,
        CatalogCheckoutId,
        Uuid,
    ) {
        let project_id = CatalogProjectId::from_uuid(Uuid::from_u128(1));
        let checkout_id = CatalogCheckoutId::from_uuid(Uuid::from_u128(2));
        let ssh_checkout_id = CatalogCheckoutId::from_uuid(Uuid::from_u128(5));
        let ssh_connection = Uuid::from_u128(50);
        let workspace_a = CatalogWorkspaceId::from_uuid(Uuid::from_u128(3));
        let workspace_b = CatalogWorkspaceId::from_uuid(Uuid::from_u128(4));
        let mut project = ProjectRecord::new(project_id, "ShellDeck");
        project.add_checkout(ProjectCheckout::new(
            checkout_id,
            "principal",
            CheckoutHost::Local {
                device_label: "Machine locale".into(),
                root: std::env::current_dir().unwrap().join("fixture-repo"),
            },
            RepositoryIdentity {
                slug: "inklura/shelldeck".into(),
                canonical_url: None,
            },
        ));
        project.add_checkout(ProjectCheckout::new(
            ssh_checkout_id,
            "distant",
            CheckoutHost::Ssh {
                connection_id: ssh_connection,
                root: shelldeck_core::config::workspace_catalog::RemotePosixPath::new(
                    "/srv/shelldeck",
                )
                .unwrap(),
            },
            RepositoryIdentity {
                slug: "inklura/shelldeck".into(),
                canonical_url: None,
            },
        ));
        let mut catalog = ProjectCatalog::default();
        catalog.insert_project(project).unwrap();
        for (id, name, checkout_id) in [
            (workspace_a, "A", checkout_id),
            (workspace_b, "B", ssh_checkout_id),
        ] {
            catalog
                .create_workspace(WorkspaceLaunchRequest {
                    id,
                    project_id,
                    checkout_id,
                    name: name.into(),
                    intake: WorkspaceLaunchIntake::Manual,
                })
                .unwrap();
        }
        (
            catalog,
            workspace_a,
            workspace_b,
            checkout_id,
            ssh_checkout_id,
            ssh_connection,
        )
    }

    fn advance_creation_to_binding(
        hub: &mut WorkspaceHubView,
        revision: u64,
        workspace: CatalogWorkspaceId,
        operation: CreationOperationId,
    ) {
        for phase in [
            WorkspaceCreatePhase::Queued,
            WorkspaceCreatePhase::ResolvingHost,
            WorkspaceCreatePhase::PreparingCheckout,
            WorkspaceCreatePhase::CreatingWorkspace,
            WorkspaceCreatePhase::BindingRuntime,
        ] {
            hub.creation
                .reduce(
                    revision,
                    WorkspaceCreateEvent::Progress {
                        workspace,
                        operation,
                        progress: WorkspaceCreateProgress {
                            phase,
                            completed_steps: 1,
                            total_steps: 1,
                            detail: "Attach".into(),
                        },
                    },
                )
                .unwrap();
        }
    }

    // SDTEST-1736 — SDUC-490 — YELLOW: native terminal identity is proven;
    // editor/files/browser ownership remains explicitly unsupported.
    #[test]
    fn keyed_gpui_workspace_entity_retention_preserves_hidden_terminal_state() {
        let mut cx = TestAppContext::single();
        let (catalog, workspace_a, workspace_b, _checkout, _ssh_checkout, _ssh_connection) =
            fixture_catalog();
        let initial_terminal = cx.update(|cx| cx.new(TerminalView::new));
        let native_terminal_before = initial_terminal.entity_id();
        let hub = cx.update(|cx| {
            cx.new(|cx| WorkspaceHubView::new(Ok(catalog), &[], initial_terminal.clone(), cx))
        });
        let workspace_entity_before = hub.read_with(&cx, |hub, _| {
            hub.retained.get(&workspace_a).unwrap().entity_id()
        });

        hub.update(&mut cx, |hub, cx| {
            hub.switch_to(workspace_a, cx);
            hub.switch_to(workspace_b, cx);
            hub.switch_to(workspace_a, cx);
        });

        hub.read_with(&cx, |hub, cx| {
            let surface = hub.retained.get(&workspace_a).unwrap();
            assert_eq!(surface.entity_id(), workspace_entity_before);
            assert_eq!(
                surface.read(cx).terminal.entity_id(),
                native_terminal_before
            );
            assert!(surface.read(cx).native_snapshot.is_some());
        });
    }

    #[test]
    fn retained_terminal_activation_keeps_the_complete_runtime_config() {
        let mut cx = TestAppContext::single();
        let (catalog, workspace_a, workspace_b, ..) = fixture_catalog();
        let initial_terminal = cx.update(|cx| cx.new(TerminalView::new));
        let hub = cx.update(|cx| {
            cx.new(|cx| WorkspaceHubView::new(Ok(catalog), &[], initial_terminal, cx))
        });
        hub.update(&mut cx, |hub, cx| {
            hub.configure_terminals(
                WorkspaceTerminalConfig {
                    theme: TerminalTheme::default(),
                    font_size: 17.0,
                    font_family: "Runtime Font".into(),
                    default_shell: Some("/bin/sh".into()),
                    cursor_style: "bar".into(),
                    cursor_blink: false,
                    scrollback_lines: 4321,
                    // Valeur calculée après repli du panneau latéral.
                    sidebar_width: crate::sidebar::RAIL_WIDTH,
                    menu_bar_visible: false,
                },
                cx,
            );
            hub.switch_to(workspace_a, cx);
            hub.switch_to(workspace_b, cx);
            hub.switch_to(workspace_a, cx);

            let terminal = hub
                .retained
                .get(&workspace_a)
                .unwrap()
                .read(cx)
                .terminal
                .clone();
            assert_eq!(
                terminal.read(cx).runtime_config_probe(),
                (
                    crate::sidebar::RAIL_WIDTH,
                    false,
                    17.0,
                    "Runtime Font",
                    Some("/bin/sh"),
                    false,
                    4321,
                )
            );
        });
    }

    // SDTEST-1738 — SDUC-489, SDUC-490 — YELLOW: local git/terminal
    // observations use UpdateCard; a live provider observation feed is pending.
    #[test]
    fn workspace_card_keeps_external_and_provider_authorities_distinct() {
        let (mut catalog, workspace_a, _, checkout, _, _) = fixture_catalog();
        let project = catalog.workspace(workspace_a).unwrap().project_id();
        let exact_mapping = PlatformV2Mapping {
            reconciliation_revision: 1,
            project: PlatformContextRef {
                id: "platform-project".into(),
                revision: 1,
            },
            checkout: PlatformContextRef {
                id: "platform-checkout".into(),
                revision: 1,
            },
            user_workspace: PlatformContextRef {
                id: "platform-workspace-a".into(),
                revision: 1,
            },
            reconciliation: PlatformMappingReconciliation::Exact {
                reconciled_at_millis: 1,
            },
        };
        catalog
            .set_platform_mapping(workspace_a, None, exact_mapping)
            .unwrap();
        catalog
            .bind_orchestration_run(
                workspace_a,
                Some(
                    shelldeck_core::config::workspace_catalog::OrchestrationRunRef {
                        runtime: "Automonique".into(),
                        run_id: "run-1".into(),
                        session_id: Some("session-1".into()),
                        platform_user_workspace_id: "platform-workspace-a".into(),
                    },
                ),
            )
            .unwrap();
        let mut task_catalog = ProjectCatalog::default();
        let source_project = catalog
            .projects()
            .find(|item| item.id() == project)
            .unwrap()
            .clone();
        task_catalog.insert_project(source_project).unwrap();
        let task_workspace = CatalogWorkspaceId::from_uuid(Uuid::from_u128(30));
        task_catalog
            .create_workspace(WorkspaceLaunchRequest {
                id: task_workspace,
                project_id: project,
                checkout_id: checkout,
                name: "Issue 127".into(),
                intake: WorkspaceLaunchIntake::Prefilled(ExternalWorkItem {
                    provider: "GitHub".into(),
                    repository: "inklura/shelldeck".into(),
                    kind: ExternalWorkItemKind::Issue,
                    key: "#127".into(),
                    title: Some("Workspace navigation".into()),
                    url: None,
                }),
            })
            .unwrap();

        let card = WorkspaceCardState {
            branch: Some("fix/workspace-navigation-ui-127".into()),
            dirty: GitDirtyState {
                staged: 1,
                modified: 2,
                untracked: 3,
                conflicted: 0,
            },
            agent: WorkspaceAgentState::WaitingForInput,
            unread: 4,
            attention: 2,
            freshness: WorkspaceFreshness::Aging,
            source_revision: 9,
            observed_at_millis: 100,
        };
        let external = workspace_card_presentation(
            &task_catalog,
            task_catalog.workspace(task_workspace).unwrap(),
            &card,
            &HashMap::new(),
            None,
        )
        .unwrap();
        assert_eq!(
            external.external.as_deref(),
            Some("Issue #127 · inklura/shelldeck")
        );
        assert_eq!(external.orchestration, None);
        assert!(!external.provider_bound);
        assert_eq!(external.branch, None);
        assert_eq!(external.dirty.modified, 2);
        assert_eq!(external.agent, WorkspaceAgentState::WaitingForInput);
        assert_eq!((external.unread, external.attention), (4, 2));

        let provider = workspace_card_presentation(
            &catalog,
            catalog.workspace(workspace_a).unwrap(),
            &card,
            &HashMap::new(),
            None,
        )
        .unwrap();
        assert_eq!(provider.external, None);
        assert_eq!(
            provider.orchestration.as_deref(),
            Some("Automonique · run-1")
        );
        assert!(provider.provider_bound);
    }

    #[test]
    fn launcher_prefills_all_external_kinds_through_one_validated_model() {
        for (intake, expected) in [
            (LauncherIntakeKind::Issue, ExternalWorkItemKind::Issue),
            (
                LauncherIntakeKind::PullRequest,
                ExternalWorkItemKind::PullRequest,
            ),
            (LauncherIntakeKind::Task, ExternalWorkItemKind::Task),
        ] {
            let draft = WorkspaceLauncherDraft {
                intake,
                provider: "GitHub".into(),
                repository: "inklura/shelldeck".into(),
                key: "#127".into(),
                title: "Navigation".into(),
                ..WorkspaceLauncherDraft::default()
            };
            let WorkspaceLaunchIntake::Prefilled(item) = draft.launch_intake().unwrap() else {
                panic!("external intake must stay prefilled");
            };
            assert_eq!(item.kind, expected);
            assert_eq!(item.key, "#127");
        }
    }

    #[test]
    fn catalog_save_failure_rolls_back_the_real_mutation() {
        let (mut catalog, workspace, ..) = fixture_catalog();
        let before = catalog.clone();
        let temp = tempfile::tempdir().unwrap();
        let parent_file = temp.path().join("not-a-directory");
        std::fs::write(&parent_file, b"occupied").unwrap();
        let invalid_target = parent_file.join("catalog.json");
        let result = mutate_and_persist(
            &mut catalog,
            |catalog| {
                catalog
                    .archive_workspace(workspace)
                    .map_err(|error| error.to_string())
            },
            |catalog| {
                catalog
                    .save_to(&invalid_target)
                    .map_err(|error| error.to_string())
            },
        );
        assert!(result.is_err());
        assert_eq!(catalog, before);
    }

    #[test]
    fn explicit_catalog_store_round_trips_an_existing_folder_checkout() {
        let temp = tempfile::tempdir().unwrap();
        let catalog_path = temp.path().join("catalog.json");
        let mut catalog = ProjectCatalog::default();
        let project_id = CatalogProjectId::new();
        let checkout_id = CatalogCheckoutId::new();
        catalog
            .insert_project(ProjectRecord::new(project_id, "ShellDeck"))
            .unwrap();
        catalog
            .add_checkout(
                project_id,
                ProjectCheckout::new(
                    checkout_id,
                    "Issue 127",
                    CheckoutHost::Local {
                        device_label: "Test".into(),
                        root: temp.path().to_path_buf(),
                    },
                    RepositoryIdentity {
                        slug: "inklura/shelldeck".into(),
                        canonical_url: None,
                    },
                ),
            )
            .unwrap();
        catalog.save_to(&catalog_path).unwrap();
        let loaded = ProjectCatalog::load_from(&catalog_path).unwrap();
        assert_eq!(loaded, catalog);
        assert!(loaded.checkout_in_project(project_id, checkout_id).is_ok());
    }

    #[tokio::test]
    async fn existing_folder_executor_streams_intermediate_progress() {
        let temp = tempfile::tempdir().unwrap();
        let workspace = CatalogWorkspaceId::new();
        let operation = CreationOperationId::new();
        let request = WorkspaceExecutionRequest {
            workspace,
            checkout: CatalogCheckoutId::new(),
            operation,
            catalog_revision: 7,
            name: "Attach".into(),
            intake: WorkspaceLaunchIntake::Manual,
            host: AuthorizedLaunchHost::Local {
                canonical_root: temp.path().to_path_buf(),
            },
            mode: WorkspaceLaunchMode::ExistingFolder,
        };
        let (tx, mut rx) = mpsc::unbounded_channel();
        ExistingFolderWorkspaceExecutor
            .launch(request, tx)
            .await
            .unwrap();
        let mut phases = Vec::new();
        while let Some(event) = rx.recv().await {
            match event {
                WorkspaceCreateEvent::Progress { progress, .. } => phases.push(progress.phase),
                WorkspaceCreateEvent::Completed { workspace: id, .. } => {
                    assert_eq!(id, workspace);
                }
                other => panic!("unexpected event: {other:?}"),
            }
        }
        assert_eq!(
            phases,
            vec![
                WorkspaceCreatePhase::Queued,
                WorkspaceCreatePhase::ResolvingHost,
                WorkspaceCreatePhase::PreparingCheckout,
                WorkspaceCreatePhase::CreatingWorkspace,
                WorkspaceCreatePhase::BindingRuntime,
            ]
        );
    }

    #[test]
    fn git_source_never_erases_provider_or_conflict_evidence() {
        let workspace = CatalogWorkspaceId::new();
        let mut owner = WorkspaceCardAggregator::default();
        owner.observe_provider(
            workspace,
            ProviderCardObservation {
                agent: WorkspaceAgentState::WaitingForInput,
                unread: 8,
                attention: 3,
                freshness: WorkspaceFreshness::Fresh,
                observed_at: 20,
            },
        );
        owner.observe_git(
            workspace,
            Some(shelldeck_core::git::GitStatus {
                branch: Some("main".into()),
                modified: 2,
                staged: 1,
                untracked: 4,
            }),
            21,
        );
        let previous = WorkspaceCardState {
            dirty: GitDirtyState {
                conflicted: 5,
                ..GitDirtyState::default()
            },
            ..WorkspaceCardState::default()
        };
        let card = owner.aggregate(workspace, &previous);
        assert_eq!(card.agent, WorkspaceAgentState::WaitingForInput);
        assert_eq!((card.unread, card.attention), (8, 3));
        assert_eq!(card.dirty.conflicted, 5);
        assert_eq!(card.dirty.modified, 2);

        owner.observe_git(workspace, None, 22);
        let unavailable = owner.aggregate(workspace, &card);
        assert_eq!((unavailable.unread, unavailable.attention), (8, 3));
        assert_eq!(unavailable.dirty.conflicted, 5);
        assert_eq!(unavailable.observed_at_millis, 0);
        assert_eq!(unavailable.freshness, WorkspaceFreshness::Fresh);

        let (catalog, workspace_a, ..) = fixture_catalog();
        let presentation = workspace_card_presentation(
            &catalog,
            catalog.workspace(workspace_a).unwrap(),
            &unavailable,
            &HashMap::new(),
            owner.sources.get(&workspace),
        )
        .unwrap();
        assert!(!presentation.git_observed);
        assert!(presentation.git_unavailable);
        assert_eq!(presentation.git_freshness, Some(WorkspaceFreshness::Stale));
        assert_eq!(presentation.branch, None);
        assert!(presentation.provider_observed);
        assert_eq!(
            presentation.provider_freshness,
            Some(WorkspaceFreshness::Fresh)
        );
    }

    #[test]
    fn provider_only_observation_never_presents_retained_git_as_current() {
        let (catalog, workspace, ..) = fixture_catalog();
        let mut owner = WorkspaceCardAggregator::default();
        owner.observe_provider(
            workspace,
            ProviderCardObservation {
                agent: WorkspaceAgentState::Running,
                unread: 2,
                attention: 1,
                freshness: WorkspaceFreshness::Fresh,
                observed_at: 50,
            },
        );
        let retained = WorkspaceCardState {
            branch: Some("retained".into()),
            dirty: GitDirtyState {
                modified: 9,
                ..GitDirtyState::default()
            },
            ..WorkspaceCardState::default()
        };
        let card = owner.aggregate(workspace, &retained);
        let presentation = workspace_card_presentation(
            &catalog,
            catalog.workspace(workspace).unwrap(),
            &card,
            &HashMap::new(),
            owner.sources.get(&workspace),
        )
        .unwrap();
        assert!(!presentation.git_observed);
        assert!(!presentation.git_unavailable);
        assert_eq!(presentation.git_freshness, None);
        assert_eq!(presentation.branch, None);
        assert!(presentation.provider_observed);
        assert_eq!(presentation.agent, WorkspaceAgentState::Running);
        assert_eq!((presentation.unread, presentation.attention), (2, 1));
        assert_eq!(
            presentation.provider_freshness,
            Some(WorkspaceFreshness::Fresh)
        );
    }

    #[test]
    fn catalog_change_before_completion_prevents_native_attach() {
        let mut app = TestAppContext::single();
        let (catalog, workspace, other, checkout, ..) = fixture_catalog();
        let terminal = app.update(|cx| cx.new(TerminalView::new));
        let hub = app.update(|cx| {
            cx.new(|cx| WorkspaceHubView::new(Ok(catalog), &[], terminal.clone(), cx))
        });
        let root = tempfile::tempdir().unwrap();
        hub.update(&mut app, |hub, cx| {
            let operation = CreationOperationId::new();
            let starting_revision = hub.catalog.revision();
            hub.creation
                .reduce(
                    starting_revision,
                    WorkspaceCreateEvent::Start {
                        workspace,
                        operation,
                    },
                )
                .unwrap();
            advance_creation_to_binding(hub, starting_revision, workspace, operation);
            hub.pending_requests.insert(
                workspace,
                WorkspaceExecutionRequest {
                    workspace,
                    checkout,
                    operation,
                    catalog_revision: starting_revision,
                    name: "Attach".into(),
                    intake: WorkspaceLaunchIntake::Manual,
                    host: AuthorizedLaunchHost::Local {
                        canonical_root: root.path().to_path_buf(),
                    },
                    mode: WorkspaceLaunchMode::ExistingFolder,
                },
            );
            hub.catalog.archive_workspace(other).unwrap();
            hub.apply_executor_event(
                workspace,
                WorkspaceCreateEvent::Completed {
                    workspace,
                    operation,
                },
                cx,
            );
            assert!(matches!(
                hub.creation.state(workspace),
                Some(BackgroundWorkspaceCreateState::Conflict {
                    conflict: WorkspaceCreateConflict::CatalogRevisionChanged { .. },
                    ..
                })
            ));
            assert_eq!(terminal.read(cx).tab_count(), 0);
            assert!(hub.pending_requests.contains_key(&workspace));
        });
    }

    #[test]
    fn folder_disappearing_before_attach_reports_only_localized_unavailability() {
        let mut app = TestAppContext::single();
        let (catalog, workspace, _, checkout, ..) = fixture_catalog();
        let terminal = app.update(|cx| cx.new(TerminalView::new));
        let hub = app.update(|cx| {
            cx.new(|cx| WorkspaceHubView::new(Ok(catalog), &[], terminal.clone(), cx))
        });
        let root = tempfile::tempdir().unwrap();
        let vanished = root.path().to_path_buf();
        root.close().unwrap();
        hub.update(&mut app, |hub, cx| {
            let operation = CreationOperationId::new();
            let revision = hub.catalog.revision();
            hub.creation
                .reduce(
                    revision,
                    WorkspaceCreateEvent::Start {
                        workspace,
                        operation,
                    },
                )
                .unwrap();
            hub.pending_requests.insert(
                workspace,
                WorkspaceExecutionRequest {
                    workspace,
                    checkout,
                    operation,
                    catalog_revision: revision,
                    name: "Vanished".into(),
                    intake: WorkspaceLaunchIntake::Manual,
                    host: AuthorizedLaunchHost::Local {
                        canonical_root: vanished.clone(),
                    },
                    mode: WorkspaceLaunchMode::ExistingFolder,
                },
            );
            terminal.update(cx, |terminal, cx| {
                terminal.install_authorized_default_cwd(&vanished);
                terminal.spawn_local_terminal(cx);
            });
            // L'action interactive précédant la complétion échoue fermée:
            // aucun PTY n'est créé dans le cwd du processus ou le HOME.
            assert_eq!(terminal.read(cx).tab_count(), 0);
            advance_creation_to_binding(hub, revision, workspace, operation);
            hub.apply_executor_event(
                workspace,
                WorkspaceCreateEvent::Completed {
                    workspace,
                    operation,
                },
                cx,
            );
            assert_eq!(terminal.read(cx).tab_count(), 0);
            assert!(matches!(
                hub.creation.state(workspace),
                Some(BackgroundWorkspaceCreateState::Failed {
                    failure: WorkspaceCreateFailure {
                        message,
                        retryable: true,
                        ..
                    },
                    ..
                }) if message == &t!("workspaces.launcher.folder_unavailable").to_string()
            ));
            assert!(hub.pending_requests.contains_key(&workspace));
        });
    }

    #[test]
    fn pending_interaction_and_completion_use_only_the_authorized_checkout_cwd() {
        let mut app = TestAppContext::single();
        let (catalog, workspace, _, checkout, ..) = fixture_catalog();
        let terminal = app.update(|cx| cx.new(TerminalView::new));
        let hub = app.update(|cx| {
            cx.new(|cx| WorkspaceHubView::new(Ok(catalog), &[], terminal.clone(), cx))
        });
        let root = tempfile::tempdir().unwrap();
        let canonical_root = std::fs::canonicalize(root.path()).unwrap();
        hub.update(&mut app, |hub, cx| {
            let operation = CreationOperationId::new();
            let revision = hub.catalog.revision();
            hub.creation
                .reduce(
                    revision,
                    WorkspaceCreateEvent::Start {
                        workspace,
                        operation,
                    },
                )
                .unwrap();
            hub.pending_requests.insert(
                workspace,
                WorkspaceExecutionRequest {
                    workspace,
                    checkout,
                    operation,
                    catalog_revision: revision,
                    name: "Authorized".into(),
                    intake: WorkspaceLaunchIntake::Manual,
                    host: AuthorizedLaunchHost::Local {
                        canonical_root: canonical_root.clone(),
                    },
                    mode: WorkspaceLaunchMode::ExistingFolder,
                },
            );
            terminal.update(cx, |terminal, cx| {
                terminal.install_authorized_default_cwd(&canonical_root);
                terminal.spawn_local_terminal(cx);
            });
            assert_eq!(terminal.read(cx).tab_count(), 1);
            assert_eq!(
                terminal
                    .read(cx)
                    .active_session()
                    .and_then(|session| session.initial_cwd()),
                Some(canonical_root.as_path())
            );
            assert!(hub.pending_requests.contains_key(&workspace));
            assert!(matches!(
                hub.creation.state(workspace),
                Some(BackgroundWorkspaceCreateState::Running { .. })
            ));

            advance_creation_to_binding(hub, revision, workspace, operation);
            hub.apply_executor_event(
                workspace,
                WorkspaceCreateEvent::Completed {
                    workspace,
                    operation,
                },
                cx,
            );
            assert_eq!(terminal.read(cx).tab_count(), 2);
            assert_eq!(
                terminal
                    .read(cx)
                    .active_session()
                    .and_then(|session| session.initial_cwd()),
                Some(canonical_root.as_path())
            );
            assert!(matches!(
                hub.creation.state(workspace),
                Some(BackgroundWorkspaceCreateState::Completed { .. })
            ));
            assert!(!hub.pending_requests.contains_key(&workspace));
        });
    }

    #[test]
    fn late_completion_from_prior_retry_has_no_terminal_side_effect() {
        let mut app = TestAppContext::single();
        let (catalog, workspace, _, checkout, ..) = fixture_catalog();
        let terminal = app.update(|cx| cx.new(TerminalView::new));
        let hub = app.update(|cx| {
            cx.new(|cx| WorkspaceHubView::new(Ok(catalog), &[], terminal.clone(), cx))
        });
        let root = tempfile::tempdir().unwrap();
        hub.update(&mut app, |hub, cx| {
            let operation_a = CreationOperationId::new();
            let operation_b = CreationOperationId::new();
            let revision = hub.catalog.revision();
            hub.creation
                .reduce(
                    revision,
                    WorkspaceCreateEvent::Start {
                        workspace,
                        operation: operation_a,
                    },
                )
                .unwrap();
            hub.creation
                .reduce(
                    revision,
                    WorkspaceCreateEvent::Failed {
                        workspace,
                        operation: operation_a,
                        failure: WorkspaceCreateFailure {
                            kind: WorkspaceCreateFailureKind::Filesystem,
                            message: "fixture".into(),
                            retryable: true,
                        },
                    },
                )
                .unwrap();
            hub.creation
                .reduce(
                    revision,
                    WorkspaceCreateEvent::Retry {
                        workspace,
                        prior_operation: operation_a,
                        operation: operation_b,
                    },
                )
                .unwrap();
            hub.pending_requests.insert(
                workspace,
                WorkspaceExecutionRequest {
                    workspace,
                    checkout,
                    operation: operation_b,
                    catalog_revision: revision,
                    name: "Retry".into(),
                    intake: WorkspaceLaunchIntake::Manual,
                    host: AuthorizedLaunchHost::Local {
                        canonical_root: root.path().to_path_buf(),
                    },
                    mode: WorkspaceLaunchMode::ExistingFolder,
                },
            );

            hub.apply_executor_event(
                workspace,
                WorkspaceCreateEvent::Completed {
                    workspace,
                    operation: operation_a,
                },
                cx,
            );

            assert_eq!(terminal.read(cx).tab_count(), 0);
            assert!(matches!(
                hub.creation.state(workspace),
                Some(BackgroundWorkspaceCreateState::Running { operation, .. })
                    if *operation == operation_b
            ));
            assert_eq!(hub.pending_requests[&workspace].operation, operation_b);
        });
    }

    #[test]
    fn completion_during_cancellation_has_no_terminal_side_effect() {
        let mut app = TestAppContext::single();
        let (catalog, workspace, _, checkout, ..) = fixture_catalog();
        let terminal = app.update(|cx| cx.new(TerminalView::new));
        let hub = app.update(|cx| {
            cx.new(|cx| WorkspaceHubView::new(Ok(catalog), &[], terminal.clone(), cx))
        });
        let root = tempfile::tempdir().unwrap();
        hub.update(&mut app, |hub, cx| {
            let operation = CreationOperationId::new();
            let revision = hub.catalog.revision();
            hub.creation
                .reduce(
                    revision,
                    WorkspaceCreateEvent::Start {
                        workspace,
                        operation,
                    },
                )
                .unwrap();
            hub.creation
                .reduce(
                    revision,
                    WorkspaceCreateEvent::RequestCancel {
                        workspace,
                        operation,
                    },
                )
                .unwrap();
            hub.pending_requests.insert(
                workspace,
                WorkspaceExecutionRequest {
                    workspace,
                    checkout,
                    operation,
                    catalog_revision: revision,
                    name: "Cancel".into(),
                    intake: WorkspaceLaunchIntake::Manual,
                    host: AuthorizedLaunchHost::Local {
                        canonical_root: root.path().to_path_buf(),
                    },
                    mode: WorkspaceLaunchMode::ExistingFolder,
                },
            );
            hub.apply_executor_event(
                workspace,
                WorkspaceCreateEvent::Completed {
                    workspace,
                    operation,
                },
                cx,
            );
            assert_eq!(terminal.read(cx).tab_count(), 0);
            assert!(matches!(
                hub.creation.state(workspace),
                Some(BackgroundWorkspaceCreateState::Cancelling { .. })
            ));
            assert!(hub.pending_requests.contains_key(&workspace));
        });
    }

    #[test]
    fn premature_completion_has_no_terminal_side_effect() {
        let mut app = TestAppContext::single();
        let (catalog, workspace, _, checkout, ..) = fixture_catalog();
        let terminal = app.update(|cx| cx.new(TerminalView::new));
        let hub = app.update(|cx| {
            cx.new(|cx| WorkspaceHubView::new(Ok(catalog), &[], terminal.clone(), cx))
        });
        let root = tempfile::tempdir().unwrap();
        hub.update(&mut app, |hub, cx| {
            let operation = CreationOperationId::new();
            let revision = hub.catalog.revision();
            hub.creation
                .reduce(
                    revision,
                    WorkspaceCreateEvent::Start {
                        workspace,
                        operation,
                    },
                )
                .unwrap();
            hub.pending_requests.insert(
                workspace,
                WorkspaceExecutionRequest {
                    workspace,
                    checkout,
                    operation,
                    catalog_revision: revision,
                    name: "Too soon".into(),
                    intake: WorkspaceLaunchIntake::Manual,
                    host: AuthorizedLaunchHost::Local {
                        canonical_root: root.path().to_path_buf(),
                    },
                    mode: WorkspaceLaunchMode::ExistingFolder,
                },
            );
            hub.apply_executor_event(
                workspace,
                WorkspaceCreateEvent::Completed {
                    workspace,
                    operation,
                },
                cx,
            );
            assert_eq!(terminal.read(cx).tab_count(), 0);
            assert!(matches!(
                hub.creation.state(workspace),
                Some(BackgroundWorkspaceCreateState::Running { progress, .. })
                    if progress.phase == WorkspaceCreatePhase::Queued
                        && progress.completed_steps == 0
            ));
            assert!(hub.pending_requests.contains_key(&workspace));
        });
    }
}
