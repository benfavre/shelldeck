use super::*;

#[cfg(test)]
mod tests {
    // SDTEST-2822 — SDUC-564: adding an existing location opens its saved
    // project without duplicate catalog entries, metadata changes or a process.
    #[::core::prelude::v1::test]
    fn sdtest_2822_onboarding_reopens_exact_location_without_duplicate_projects() {
        let mut app = TestAppContext::single();
        let (catalog, ..) = fixture_catalog();
        let catalog_store = tempfile::tempdir().unwrap();
        let catalog_path = catalog_store.path().join("project-catalog.json");
        let before = catalog
            .projects()
            .flat_map(|project| project.checkouts())
            .count();
        let first = shelldeck_core::models::Connection::new_manual(
            "Server A".into(),
            "a.invalid".into(),
            "qa".into(),
        );
        let second = shelldeck_core::models::Connection::new_manual(
            "Server B".into(),
            "b.invalid".into(),
            "qa".into(),
        );
        let terminal = app.update(|cx| cx.new(TerminalView::new));
        let hub = app.update(|cx| {
            cx.new(|cx| {
                WorkspaceHubView::new(
                    Ok(catalog),
                    &[first.clone(), second.clone()],
                    terminal.clone(),
                    cx,
                )
            })
        });
        let opened = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let observed = opened.clone();
        let _subscription = app.update(|cx| {
            cx.subscribe(&hub, move |_, event, _| {
                if let super::WorkspaceHubEvent::OpenProjectAgents(checkout) = event {
                    observed.borrow_mut().push(*checkout);
                }
            })
        });
        let local = tempfile::tempdir().unwrap();
        let local_root = local.path().canonicalize().unwrap().display().to_string();
        for (connection, original, equivalent) in [
            (
                Some(first.id),
                "/srv/repeated project ".to_owned(),
                "/srv/repeated project /".to_owned(),
            ),
            (
                None,
                local_root.clone(),
                local.path().join(".").display().to_string(),
            ),
        ] {
            for (path, name) in [
                (original, "Original project"),
                (equivalent, "Do not overwrite"),
            ] {
                hub.update(&mut app, |hub, cx| {
                    hub.show_onboarding(cx);
                    hub.onboarding_ssh = connection.is_some();
                    hub.onboarding_connection = connection;
                    hub.onboarding_root
                        .update(cx, |input, cx| input.replace_content(path, cx));
                    hub.onboarding_project
                        .update(cx, |input, cx| input.replace_content(name.to_owned(), cx));
                    hub.submit_onboarding_with_persist(
                        |catalog| {
                            catalog
                                .save_to(&catalog_path)
                                .map_err(|error| error.to_string())
                        },
                        cx,
                    );
                    assert!(hub.error.is_none(), "{:?}", hub.error);
                    assert!(hub.pending_requests.is_empty());
                    assert_eq!(terminal.read(cx).tab_count(), 0);
                });
            }
            let ids = opened.borrow();
            assert_eq!(ids[ids.len() - 1], ids[ids.len() - 2]);
        }
        hub.update(&mut app, |hub, cx| {
            assert_eq!(
                hub.catalog
                    .projects()
                    .flat_map(|project| project.checkouts())
                    .count(),
                before + 2
            );
            assert!(!hub
                .catalog
                .projects()
                .any(|project| project.name() == "Do not overwrite"));
            hub.show_onboarding(cx);
            hub.onboarding_ssh = true;
            hub.onboarding_connection = Some(second.id);
            hub.onboarding_root.update(cx, |input, cx| {
                input.replace_content("/srv/repeated project ", cx)
            });
            hub.submit_onboarding_with_persist(
                |catalog| {
                    catalog
                        .save_to(&catalog_path)
                        .map_err(|error| error.to_string())
                },
                cx,
            );
            assert!(hub.error.is_none());
            assert_eq!(
                hub.catalog
                    .projects()
                    .flat_map(|project| project.checkouts())
                    .count(),
                before + 3
            );
            assert_eq!(terminal.read(cx).tab_count(), 0);
        });
    }

    // SDTEST-2790 — adding a project from Agents never launches a workspace; Cancel returns once.
    #[test]
    fn agent_project_setup_stays_onboarding_with_existing_checkout_and_returns_once() {
        let mut app = TestAppContext::single();
        let (catalog, ..) = fixture_catalog();
        let terminal = app.update(|cx| cx.new(TerminalView::new));
        let hub = app.update(|cx| {
            cx.new(|cx| WorkspaceHubView::new(Ok(catalog), &[], terminal.clone(), cx))
        });
        let returned = std::rc::Rc::new(std::cell::Cell::new(0));
        let observed = returned.clone();
        let _subscription = app.update(|cx| {
            cx.subscribe(&hub, move |_, event, _| {
                if matches!(event, super::WorkspaceHubEvent::ReturnToAgents) {
                    observed.set(observed.get() + 1);
                }
            })
        });
        hub.update(&mut app, |hub, cx| {
            hub.show_agent_project_setup(cx);
            assert!(hub.onboarding_open && !hub.launcher_open);
            assert!(hub.pending_requests.is_empty());
            assert_eq!(terminal.read(cx).tab_count(), 0);
            hub.cancel_forms(cx);
        });
        assert_eq!(returned.get(), 1);
        hub.update(&mut app, |hub, cx| hub.cancel_forms(cx));
        assert_eq!(returned.get(), 1);
        hub.update(&mut app, |hub, cx| {
            hub.show_agent_project_setup(cx);
            hub.show_launcher(cx);
            assert!(hub.launcher_open && !hub.onboarding_open);
            hub.cancel_forms(cx);
            hub.show_onboarding(cx);
            hub.cancel_forms(cx);
        });
        assert_eq!(
            returned.get(),
            1,
            "ordinary Workspaces navigation stays in Workspaces"
        );
    }

    // SDTEST-2733 — Add project saves its checkout then opens Agents, never a terminal.
    #[test]
    fn onboarding_opens_agents_with_saved_local_or_ssh_checkout_without_launching() {
        let mut app = TestAppContext::single();
        let catalog = ProjectCatalog::load().unwrap();
        let prior_workspaces = catalog.workspaces().count();
        let connection = shelldeck_core::models::Connection::new_manual(
            "QA target".into(),
            "invalid.example".into(),
            "qa".into(),
        );
        let terminal = app.update(|cx| cx.new(TerminalView::new));
        let hub = app.update(|cx| {
            cx.new(|cx| {
                WorkspaceHubView::new(
                    Ok(catalog),
                    std::slice::from_ref(&connection),
                    terminal.clone(),
                    cx,
                )
            })
        });
        let opened = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let observed = opened.clone();
        let _subscription = app.update(|cx| {
            cx.subscribe(&hub, move |hub, event, cx| {
                if let super::WorkspaceHubEvent::OpenProjectAgents(checkout) = event {
                    assert!(hub
                        .read(cx)
                        .catalog
                        .projects()
                        .any(|project| project.checkouts().any(|item| item.id() == *checkout)));
                    observed.borrow_mut().push(*checkout);
                }
            })
        });
        let local = tempfile::tempdir().unwrap();
        for ssh in [false, true] {
            let expected = if ssh {
                "/srv/client folder ".to_owned()
            } else {
                local.path().canonicalize().unwrap().display().to_string()
            };
            hub.update(&mut app, |hub, cx| {
                hub.show_onboarding(cx);
                hub.onboarding_ssh = ssh;
                hub.onboarding_connection = ssh.then_some(connection.id);
                hub.onboarding_root
                    .update(cx, |input, cx| input.replace_content(expected.clone(), cx));
                hub.submit_onboarding(cx);
                assert!(hub.error.is_none(), "{:?}", hub.error);
                assert!(!hub.onboarding_open && !hub.launcher_open);
                assert!(hub.pending_requests.is_empty());
                assert_eq!(hub.catalog.workspaces().count(), prior_workspaces);
                assert_eq!(terminal.read(cx).tab_count(), 0);
            });
            let id = *opened.borrow().last().expect("Agents navigation emitted");
            hub.read_with(&app, |hub, _| {
                let groups = crate::workspace::agents::agent_project_groups(
                    &hub.catalog,
                    std::slice::from_ref(&connection),
                );
                let option = groups
                    .iter()
                    .flat_map(|group| &group.worktrees)
                    .find(|item| item.id == id.to_string())
                    .unwrap();
                assert_eq!(option.path, expected);
                match &option.target {
                    shelldeck_core::agent_runtime::AgentTarget::Local => assert!(!ssh),
                    shelldeck_core::agent_runtime::AgentTarget::Ssh { connection_id, .. } => {
                        assert!(ssh);
                        assert_eq!(*connection_id, connection.id);
                    }
                }
            });
        }
        assert_eq!(opened.borrow().len(), 2);
    }

    // SDTEST-2725 — removing a failed attempt cannot delete data or a newer operation.
    #[test]
    fn dismiss_failed_workspace_preserves_catalog_and_fences_late_events() {
        let mut app = TestAppContext::single();
        let (catalog, existing, _, checkout, ..) = fixture_catalog();
        let terminal = app.update(|cx| cx.new(TerminalView::new));
        let hub = app.update(|cx| {
            cx.new(|cx| WorkspaceHubView::new(Ok(catalog), &[], terminal.clone(), cx))
        });
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("keep.txt"), "keep").unwrap();
        hub.update(&mut app, |hub, cx| {
            let snapshot = hub.catalog.clone();
            let workspace = CatalogWorkspaceId::new();
            let revision = hub.catalog.revision();
            let operation = CreationOperationId::new();
            let newer = CreationOperationId::new();
            let project = hub.catalog.workspace(existing).unwrap().project_id();
            hub.pending_requests.insert(
                workspace,
                WorkspaceExecutionRequest {
                    workspace,
                    project,
                    source_checkout: checkout,
                    checkout,
                    created_checkout: None,
                    operation,
                    catalog_revision: revision,
                    name: "Failed attempt".into(),
                    intake: WorkspaceLaunchIntake::Manual,
                    host: AuthorizedLaunchHost::LocalExisting {
                        authority: crate::terminal_view::AuthorizedLocalRoot::capture(root.path())
                            .unwrap(),
                    },
                    mode: WorkspaceLaunchMode::ExistingFolder,
                },
            );
            hub.creation
                .reduce(
                    revision,
                    WorkspaceCreateEvent::Start {
                        workspace,
                        operation,
                    },
                )
                .unwrap();
            hub.dismiss_create(workspace, operation, cx);
            assert!(hub.pending_requests.contains_key(&workspace));
            let failure = WorkspaceCreateFailure {
                kind: WorkspaceCreateFailureKind::Filesystem,
                message: "fixture".into(),
                retryable: true,
            };
            hub.apply_executor_event(
                workspace,
                WorkspaceCreateEvent::Failed {
                    workspace,
                    operation,
                    failure: failure.clone(),
                },
                cx,
            );
            hub.creation
                .reduce(
                    revision,
                    WorkspaceCreateEvent::Retry {
                        workspace,
                        prior_operation: operation,
                        operation: newer,
                    },
                )
                .unwrap();
            hub.pending_requests.get_mut(&workspace).unwrap().operation = newer;
            hub.dismiss_create(workspace, operation, cx);
            assert!(hub.pending_requests.contains_key(&workspace));
            hub.creation
                .reduce(
                    revision,
                    WorkspaceCreateEvent::RequestCancel {
                        workspace,
                        operation: newer,
                    },
                )
                .unwrap();
            hub.dismiss_create(workspace, newer, cx);
            assert!(hub.pending_requests.contains_key(&workspace));
            hub.apply_executor_event(
                workspace,
                WorkspaceCreateEvent::Cancelled {
                    workspace,
                    operation: newer,
                },
                cx,
            );
            hub.dismiss_create(workspace, newer, cx);
            assert!(!hub.pending_requests.contains_key(&workspace));
            assert!(hub.creation.state(workspace).is_none());
            hub.apply_executor_event(
                workspace,
                WorkspaceCreateEvent::Completed {
                    workspace,
                    operation: newer,
                },
                cx,
            );
            assert!(!hub.pending_requests.contains_key(&workspace));
            assert!(hub
                .creation
                .reduce(
                    revision,
                    WorkspaceCreateEvent::Start {
                        workspace,
                        operation: newer
                    }
                )
                .is_err());
            let final_operation = CreationOperationId::new();
            hub.pending_requests.insert(
                workspace,
                WorkspaceExecutionRequest {
                    workspace,
                    project,
                    source_checkout: checkout,
                    checkout,
                    created_checkout: None,
                    operation: final_operation,
                    catalog_revision: revision,
                    name: "Another failure".into(),
                    intake: WorkspaceLaunchIntake::Manual,
                    host: AuthorizedLaunchHost::LocalExisting {
                        authority: crate::terminal_view::AuthorizedLocalRoot::capture(root.path())
                            .unwrap(),
                    },
                    mode: WorkspaceLaunchMode::ExistingFolder,
                },
            );
            hub.creation
                .reduce(
                    revision,
                    WorkspaceCreateEvent::Start {
                        workspace,
                        operation: final_operation,
                    },
                )
                .unwrap();
            hub.apply_executor_event(
                workspace,
                WorkspaceCreateEvent::Failed {
                    workspace,
                    operation: final_operation,
                    failure,
                },
                cx,
            );
            hub.dismiss_create(workspace, final_operation, cx);
            assert!(!hub.pending_requests.contains_key(&workspace));
            assert!(hub.creation.state(workspace).is_none());
            assert_eq!(snapshot, hub.catalog);
            assert_eq!(terminal.read(cx).tab_count(), 0);
            assert_eq!(
                std::fs::read_to_string(root.path().join("keep.txt")).unwrap(),
                "keep"
            );
        });
    }

    // SDTEST-2719 — stale picker admission and edited connection identity cannot reuse a remote path.
    #[test]
    fn remote_folder_selection_is_scoped_to_current_form_and_connection() {
        let mut app = TestAppContext::single();
        let (catalog, ..) = fixture_catalog();
        let connection = shelldeck_core::models::Connection::new_manual(
            "QA".into(),
            "invalid.example".into(),
            "qa".into(),
        );
        let terminal = app.update(|cx| cx.new(TerminalView::new));
        let hub = app.update(|cx| {
            cx.new(|cx| {
                WorkspaceHubView::new(Ok(catalog), std::slice::from_ref(&connection), terminal, cx)
            })
        });
        hub.update(&mut app, |hub, cx| {
            hub.show_onboarding(cx);
            hub.onboarding_ssh = true;
            hub.onboarding_connection = Some(connection.id);
            hub.request_remote_folder_picker(cx);
            let request = hub.remote_browse_epoch;
            hub.dismiss_forms(cx);
            hub.show_onboarding(cx);
            hub.open_remote_folder_picker(request, connection.clone(), String::new(), cx);
            assert!(hub.remote_folder_picker.is_none());

            hub.onboarding_root
                .update(cx, |input, cx| input.replace_content("/srv/project ", cx));
            assert_eq!(hub.onboarding_path(cx), "/srv/project ");
            hub.onboarding_browse_authority = Some(connection.clone());
            let mut renamed = connection.clone();
            renamed.alias = "A clearer label".into();
            hub.refresh_browse_connections(&[renamed.clone()], cx);
            assert_eq!(hub.onboarding_root.read(cx).content(), "/srv/project ");
            renamed.hostname = "different.invalid".into();
            hub.refresh_browse_connections(&[renamed], cx);
            assert!(hub.onboarding_root.read(cx).content().is_empty());
            assert!(hub.onboarding_browse_authority.is_none());
        });
    }

    // SDTEST-2712 — same folder names cannot establish local/SSH repository mapping.
    #[test]
    fn inferred_folder_names_do_not_pair_unrelated_checkouts() {
        let mut catalog = ProjectCatalog::default();
        let first = CatalogCheckoutId::new();
        let (first_repository, first_match) =
            super::onboarding_identity(&catalog, "app", None, first);
        assert!(first_match.is_none());
        let mut project = ProjectRecord::new(CatalogProjectId::new(), "app");
        project.add_checkout(ProjectCheckout::new(
            first,
            "app",
            CheckoutHost::Local {
                device_label: "Local".into(),
                root: std::env::temp_dir().join("client-a/app"),
            },
            first_repository.clone(),
        ));
        catalog.insert_project(project).unwrap();
        let (second_repository, second_match) =
            super::onboarding_identity(&catalog, "app", None, CatalogCheckoutId::new());
        assert!(second_match.is_none());
        assert_ne!(first_repository, second_repository);

        let shared_id = CatalogProjectId::new();
        let mut explicit = ProjectRecord::new(shared_id, "app");
        explicit.add_checkout(ProjectCheckout::new(
            CatalogCheckoutId::new(),
            "app",
            CheckoutHost::Local {
                device_label: "Local".into(),
                root: std::env::temp_dir().join("shared/app"),
            },
            RepositoryIdentity {
                slug: "team/app".into(),
                canonical_url: None,
            },
        ));
        catalog.insert_project(explicit).unwrap();
        let (_, same) = super::onboarding_identity(
            &catalog,
            "app",
            Some("team/app".into()),
            CatalogCheckoutId::new(),
        );
        assert_eq!(same, Some(shared_id));
        let (_, other) = super::onboarding_identity(
            &catalog,
            "app",
            Some("other/app".into()),
            CatalogCheckoutId::new(),
        );
        assert!(other.is_none());
    }

    use super::{
        mutate_and_persist, reconcile_terminal_surface, terminal_surface,
        workspace_card_presentation,
        workspaces_panes::{
            adjust_split_ratio, remove_agent_session_tabs, resolve_local_tab_path,
            split_leaf_with_active_tab, validated_browser_location,
        },
        AuthorizedLaunchHost, GitWorktreeAdapter, LauncherIntakeKind, NativeLaunchOutcome,
        NativeWorkspaceExecutor, ProviderCardObservation, WorkspaceCardAggregator,
        WorkspaceExecutionRequest, WorkspaceHubView, WorkspaceLaunchExecutor, WorkspaceLaunchMode,
        WorkspaceLauncherDraft, WorkspaceTerminalConfig,
    };
    use crate::terminal_view::TerminalView;
    use gpui::{AppContext, TestAppContext};
    use shelldeck_core::config::platform::{
        ResourceAuthority, ResourceCoordinate, ResourceId, ResourceKind,
    };
    use shelldeck_core::config::themes::TerminalTheme;
    use shelldeck_core::config::workspace_catalog::{
        CatalogCheckoutId, CatalogProjectId, CatalogWorkspaceId, CheckoutHost, ExternalWorkItem,
        ExternalWorkItemKind, PlatformContextRef, PlatformMappingReconciliation, PlatformV2Mapping,
        ProjectCatalog, ProjectCheckout, ProjectRecord, RepositoryIdentity, WorkspaceLaunchIntake,
        WorkspaceLaunchRequest, WorkspaceRelativePath,
    };
    use shelldeck_core::workspace_navigation::{
        AgentSessionBinding, BackgroundWorkspaceCreateState, CreationOperationId, GitDirtyState,
        PaneId, PaneLeaf, PaneNode, ProviderSessionBinding, SplitAxis, TerminalAuthority,
        TerminalBinding, TerminalBindingId, TerminalSurface, TerminalViewport, WorkspaceAgentState,
        WorkspaceCardState, WorkspaceCreateEvent, WorkspaceCreateFailure,
        WorkspaceCreateFailureKind, WorkspaceCreatePhase, WorkspaceCreateProgress, WorkspaceFocus,
        WorkspaceFreshness, WorkspaceNavigationAction, WorkspaceSurfaceState, WorkspaceTab,
        WorkspaceTabContent, WorkspaceTabId,
    };
    use shelldeck_core::workspace_review::{
        AttentionError, AttentionItem, AttentionItemId, AttentionState, AttentionTarget,
    };
    use shelldeck_terminal::session::TerminalSession;
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::sync::Arc;
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

    // SDTEST-2713 — SDUC-490: hidden retained panes must not consume unread attention.
    #[test]
    fn overview_and_forms_hide_retained_attention_without_replacing_entities() {
        let mut app = TestAppContext::single();
        let (catalog, workspace, _, checkout, ..) = fixture_catalog();
        let terminal = app.update(|cx| cx.new(TerminalView::new));
        let hub =
            app.update(|cx| cx.new(|cx| WorkspaceHubView::new(Ok(catalog), &[], terminal, cx)));
        let session_id = Uuid::new_v4();
        hub.update(&mut app, |hub, cx| {
            hub.switch_to(workspace, cx);
            let pane = PaneId::from_uuid(Uuid::new_v4());
            let tab = WorkspaceTabId::from_uuid(Uuid::new_v4());
            let surface = WorkspaceSurfaceState {
                root: Some(PaneNode::Leaf(PaneLeaf {
                    id: pane,
                    tabs: vec![WorkspaceTab {
                        id: tab,
                        title: "Retained agent".into(),
                        content: WorkspaceTabContent::AgentSession(AgentSessionBinding {
                            checkout_id: checkout,
                            session_id,
                        }),
                    }],
                    active_tab: Some(tab),
                })),
                focus: Some(WorkspaceFocus {
                    pane_id: pane,
                    tab_id: tab,
                }),
            };
            hub.navigation
                .reduce(
                    &hub.catalog,
                    WorkspaceNavigationAction::UpdateSurface {
                        id: workspace,
                        surface,
                    },
                )
                .unwrap();
            assert_eq!(hub.active_agent_session_id(), Some(session_id));
        });
        let retained_id = hub.read_with(&app, |hub, _| hub.retained[&workspace].entity_id());
        let visible = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let observed = visible.clone();
        let _subscription = app.update(|cx| {
            cx.subscribe(&hub, move |hub, event, cx| {
                if matches!(event, super::WorkspaceHubEvent::PresentationChanged) {
                    observed
                        .borrow_mut()
                        .push(hub.read(cx).active_agent_session_id());
                }
            })
        });
        hub.update(&mut app, |hub, cx| hub.show_launcher(cx));
        hub.update(&mut app, |hub, cx| hub.show_onboarding(cx));
        hub.update(&mut app, |hub, cx| {
            hub.error = Some("Invalid folder".into());
            hub.dismiss_forms(cx);
            assert!(hub.error.is_none());
        });
        hub.update(&mut app, |hub, cx| {
            hub.toggle_overview(cx);
            assert!(hub.platform_attention_surface_is_visible(workspace));
            assert_eq!(hub.navigation.active(), Some(workspace));
        });
        hub.update(&mut app, |hub, cx| {
            hub.toggle_overview(cx);
            assert!(!hub.platform_attention_surface_is_visible(workspace));
            assert_eq!(hub.active_agent_session_id(), Some(session_id));
            assert_eq!(hub.retained[&workspace].entity_id(), retained_id);
        });
        assert_eq!(
            *visible.borrow(),
            vec![None, None, Some(session_id), None, Some(session_id)]
        );
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
        cx.run_until_parked();
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

    // SDTEST-1885 — SDUC-490/493
    #[test]
    fn terminal_reconciliation_preserves_typed_tabs_splits_and_focus() {
        let checkout = CatalogCheckoutId::from_uuid(Uuid::from_u128(2));
        let agent_pane = PaneId::from_uuid(Uuid::from_u128(80));
        let files_pane = PaneId::from_uuid(Uuid::from_u128(81));
        let agent_tab = WorkspaceTabId::from_uuid(Uuid::from_u128(82));
        let files_tab = WorkspaceTabId::from_uuid(Uuid::from_u128(83));
        let terminal_tab = WorkspaceTabId::from_uuid(Uuid::from_u128(84));
        let retained = WorkspaceSurfaceState {
            root: Some(PaneNode::Split {
                axis: SplitAxis::Horizontal,
                ratio_basis_points: 6_250,
                first: Box::new(PaneNode::Leaf(PaneLeaf {
                    id: agent_pane,
                    tabs: vec![WorkspaceTab {
                        id: agent_tab,
                        title: "Implement cockpit".into(),
                        content: WorkspaceTabContent::AgentSession(AgentSessionBinding {
                            checkout_id: checkout,
                            session_id: Uuid::from_u128(85),
                        }),
                    }],
                    active_tab: Some(agent_tab),
                })),
                second: Box::new(PaneNode::Leaf(PaneLeaf {
                    id: files_pane,
                    tabs: vec![WorkspaceTab {
                        id: files_tab,
                        title: "Files".into(),
                        content: WorkspaceTabContent::Files {
                            checkout_id: checkout,
                            relative_root: WorkspaceRelativePath::new("src").unwrap(),
                        },
                    }],
                    active_tab: Some(files_tab),
                })),
            }),
            focus: Some(WorkspaceFocus {
                pane_id: agent_pane,
                tab_id: agent_tab,
            }),
        };
        let native = WorkspaceSurfaceState {
            root: Some(PaneNode::Leaf(PaneLeaf {
                id: PaneId::from_uuid(Uuid::from_u128(86)),
                tabs: vec![WorkspaceTab {
                    id: terminal_tab,
                    title: "Terminal".into(),
                    content: WorkspaceTabContent::Terminal(TerminalSurface {
                        binding: TerminalBinding {
                            id: TerminalBindingId::from_uuid(Uuid::from_u128(87)),
                            authority: TerminalAuthority::Local {
                                checkout_id: checkout,
                            },
                        },
                        viewport: TerminalViewport::default(),
                        draft: String::new(),
                    }),
                }],
                active_tab: Some(terminal_tab),
            })),
            focus: None,
        };

        let reconciled = reconcile_terminal_surface(&retained, native);
        assert_eq!(reconciled.focus, retained.focus);
        let PaneNode::Split {
            ratio_basis_points,
            first,
            second,
            ..
        } = reconciled.root.unwrap()
        else {
            panic!("typed split was flattened");
        };
        assert_eq!(ratio_basis_points, 6_250);
        let PaneNode::Leaf(first) = *first else {
            panic!("first split leaf missing");
        };
        assert!(first.tabs.iter().any(|tab| tab.id == agent_tab));
        assert!(first.tabs.iter().any(|tab| tab.id == terminal_tab));
        let PaneNode::Leaf(second) = *second else {
            panic!("second split leaf missing");
        };
        assert_eq!(second.tabs[0].id, files_tab);
    }

    // SDTEST-1886 — SDUC-490
    #[test]
    fn closing_agent_session_removes_every_duplicate_and_repairs_focus() {
        let checkout = CatalogCheckoutId::from_uuid(Uuid::from_u128(2));
        let session_id = Uuid::from_u128(1886);
        let first_pane = PaneId::from_uuid(Uuid::from_u128(90));
        let second_pane = PaneId::from_uuid(Uuid::from_u128(91));
        let terminal_tab = WorkspaceTabId::from_uuid(Uuid::from_u128(92));
        let first_agent = WorkspaceTabId::from_uuid(Uuid::from_u128(93));
        let second_agent = WorkspaceTabId::from_uuid(Uuid::from_u128(94));
        let agent = |id| WorkspaceTab {
            id,
            title: "Duplicate agent".into(),
            content: WorkspaceTabContent::AgentSession(AgentSessionBinding {
                checkout_id: checkout,
                session_id,
            }),
        };
        let mut surface = WorkspaceSurfaceState {
            root: Some(PaneNode::Split {
                axis: SplitAxis::Horizontal,
                ratio_basis_points: 5_000,
                first: Box::new(PaneNode::Leaf(PaneLeaf {
                    id: first_pane,
                    tabs: vec![
                        WorkspaceTab {
                            id: terminal_tab,
                            title: "Terminal".into(),
                            content: WorkspaceTabContent::Terminal(TerminalSurface {
                                binding: TerminalBinding {
                                    id: TerminalBindingId::from_uuid(Uuid::from_u128(95)),
                                    authority: TerminalAuthority::Local {
                                        checkout_id: checkout,
                                    },
                                },
                                viewport: TerminalViewport::default(),
                                draft: String::new(),
                            }),
                        },
                        agent(first_agent),
                    ],
                    active_tab: Some(first_agent),
                })),
                second: Box::new(PaneNode::Leaf(PaneLeaf {
                    id: second_pane,
                    tabs: vec![agent(second_agent)],
                    active_tab: Some(second_agent),
                })),
            }),
            focus: Some(WorkspaceFocus {
                pane_id: second_pane,
                tab_id: second_agent,
            }),
        };

        assert!(remove_agent_session_tabs(&mut surface, session_id));
        assert_eq!(
            surface.focus,
            Some(WorkspaceFocus {
                pane_id: first_pane,
                tab_id: terminal_tab,
            })
        );
        assert!(!remove_agent_session_tabs(&mut surface, session_id));
        let PaneNode::Split { first, second, .. } = surface.root.unwrap() else {
            panic!("split disappeared");
        };
        let PaneNode::Leaf(first) = *first else {
            panic!("first leaf disappeared");
        };
        let PaneNode::Leaf(second) = *second else {
            panic!("second leaf disappeared");
        };
        assert_eq!(first.tabs.len(), 1);
        assert_eq!(first.active_tab, Some(terminal_tab));
        assert!(second.tabs.is_empty());
        assert_eq!(second.active_tab, None);
    }

    // SDTEST-1887 — SDUC-490
    #[test]
    fn split_action_moves_the_active_tab_into_a_distinct_native_pane() {
        let checkout = CatalogCheckoutId::from_uuid(Uuid::from_u128(2));
        let pane = PaneId::from_uuid(Uuid::from_u128(100));
        let terminal_tab = WorkspaceTabId::from_uuid(Uuid::from_u128(101));
        let agent_tab = WorkspaceTabId::from_uuid(Uuid::from_u128(102));
        let mut node = PaneNode::Leaf(PaneLeaf {
            id: pane,
            tabs: vec![
                WorkspaceTab {
                    id: terminal_tab,
                    title: "Terminal".into(),
                    content: WorkspaceTabContent::Terminal(TerminalSurface {
                        binding: TerminalBinding {
                            id: TerminalBindingId::from_uuid(Uuid::from_u128(103)),
                            authority: TerminalAuthority::Local {
                                checkout_id: checkout,
                            },
                        },
                        viewport: TerminalViewport::default(),
                        draft: String::new(),
                    }),
                },
                WorkspaceTab {
                    id: agent_tab,
                    title: "Agent".into(),
                    content: WorkspaceTabContent::AgentSession(AgentSessionBinding {
                        checkout_id: checkout,
                        session_id: Uuid::from_u128(104),
                    }),
                },
            ],
            active_tab: Some(agent_tab),
        });

        let focus = split_leaf_with_active_tab(&mut node, pane, SplitAxis::Horizontal)
            .expect("two tabs can be split");
        let PaneNode::Split {
            axis,
            ratio_basis_points,
            first,
            second,
        } = node
        else {
            panic!("split action did not create a split");
        };
        assert_eq!(axis, SplitAxis::Horizontal);
        assert_eq!(ratio_basis_points, 5_000);
        let PaneNode::Leaf(first) = *first else {
            panic!("first leaf missing");
        };
        let PaneNode::Leaf(second) = *second else {
            panic!("second leaf missing");
        };
        assert_eq!(first.tabs[0].id, terminal_tab);
        assert_eq!(first.active_tab, Some(terminal_tab));
        assert_eq!(second.tabs[0].id, agent_tab);
        assert_eq!(second.active_tab, Some(agent_tab));
        assert_eq!(focus.pane_id, second.id);
        assert_eq!(focus.tab_id, agent_tab);
    }

    // SDTEST-1888 — SDUC-490
    #[test]
    fn workspace_editor_path_resolution_requires_an_existing_authorized_entry() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("src")).unwrap();
        std::fs::write(root.path().join("src/lib.rs"), "fn main() {}").unwrap();
        let checkout = ProjectCheckout::new(
            CatalogCheckoutId::from_uuid(Uuid::from_u128(110)),
            "local",
            CheckoutHost::Local {
                device_label: "Local".into(),
                root: std::fs::canonicalize(root.path()).unwrap(),
            },
            RepositoryIdentity {
                slug: "inklura/shelldeck".into(),
                canonical_url: None,
            },
        );
        let valid = WorkspaceRelativePath::new("src/lib.rs").unwrap();
        let missing = WorkspaceRelativePath::new("src/missing.rs").unwrap();

        assert_eq!(
            resolve_local_tab_path(&checkout, &valid).unwrap().as_path(),
            std::fs::canonicalize(root.path().join("src/lib.rs"))
                .unwrap()
                .as_path()
        );
        assert!(resolve_local_tab_path(&checkout, &missing).is_none());
    }

    // SDTEST-1889 — SDUC-490
    #[test]
    fn split_resize_controls_adjust_the_exact_divider_and_stay_bounded() {
        let first = PaneId::from_uuid(Uuid::from_u128(120));
        let second = PaneId::from_uuid(Uuid::from_u128(121));
        let mut node = PaneNode::Split {
            axis: SplitAxis::Horizontal,
            ratio_basis_points: 5_000,
            first: Box::new(PaneNode::Leaf(PaneLeaf {
                id: first,
                tabs: Vec::new(),
                active_tab: None,
            })),
            second: Box::new(PaneNode::Leaf(PaneLeaf {
                id: second,
                tabs: Vec::new(),
                active_tab: None,
            })),
        };

        assert!(adjust_split_ratio(&mut node, first, second, 500));
        let PaneNode::Split {
            ratio_basis_points, ..
        } = &node
        else {
            unreachable!();
        };
        assert_eq!(*ratio_basis_points, 5_500);
        assert!(adjust_split_ratio(&mut node, first, second, 20_000));
        let PaneNode::Split {
            ratio_basis_points, ..
        } = &node
        else {
            unreachable!();
        };
        assert_eq!(*ratio_basis_points, 9_000);
        assert!(!adjust_split_ratio(
            &mut node,
            PaneId::from_uuid(Uuid::from_u128(122)),
            second,
            500,
        ));
    }

    // SDTEST-1890 — SDUC-490
    #[test]
    fn browser_pane_open_action_admits_only_safe_http_locations() {
        assert_eq!(
            validated_browser_location(" https://127.0.0.1:3000/preview "),
            Some("https://127.0.0.1:3000/preview".into())
        );
        assert!(validated_browser_location("javascript:alert(1)").is_none());
        assert!(validated_browser_location("https://user:secret@example.test").is_none());
        assert!(validated_browser_location("https://").is_none());
        assert!(validated_browser_location("https://example.test/\nnext").is_none());
    }

    // SDTEST-1811
    #[test]
    fn sdtest_1811_retained_gpui_attention_opens_exact_workspace_pane_and_tab() {
        let mut cx = TestAppContext::single();
        let (catalog, workspace_a, workspace_b, _checkout, _ssh_checkout, ssh_connection) =
            fixture_catalog();
        let initial_terminal = cx.update(|cx| cx.new(TerminalView::new));
        let hub = cx.update(|cx| {
            cx.new(|cx| WorkspaceHubView::new(Ok(catalog), &[], initial_terminal, cx))
        });
        cx.run_until_parked();

        let (session_a1, _data_a1, _input_a1) =
            TerminalSession::spawn_ssh("A first".into(), 24, 80).unwrap();
        let session_a1_id = session_a1.id;
        let (session_a2, _data_a2, _input_a2) =
            TerminalSession::spawn_ssh("A attention".into(), 24, 80).unwrap();
        let session_a2_id = session_a2.id;
        let (session_b, _data_b, _input_b) =
            TerminalSession::spawn_ssh("B active".into(), 24, 80).unwrap();

        hub.update(&mut cx, |hub, cx| {
            let terminal_a = hub
                .retained
                .get(&workspace_a)
                .unwrap()
                .read(cx)
                .terminal
                .clone();
            terminal_a.update(cx, |terminal, _| {
                terminal.add_session(session_a1);
                terminal.add_session(session_a2);
            });
            let surface_a = terminal_surface(&hub.catalog, workspace_a, terminal_a.read(cx));
            hub.navigation
                .reduce(
                    &hub.catalog,
                    WorkspaceNavigationAction::UpdateSurface {
                        id: workspace_a,
                        surface: surface_a,
                    },
                )
                .unwrap();

            let terminal_b = hub
                .retained
                .get(&workspace_b)
                .unwrap()
                .read(cx)
                .terminal
                .clone();
            terminal_b.update(cx, |terminal, _| {
                terminal.add_session_with_connection(session_b, Some(ssh_connection));
            });
            let surface_b = terminal_surface(&hub.catalog, workspace_b, terminal_b.read(cx));
            hub.navigation
                .reduce(
                    &hub.catalog,
                    WorkspaceNavigationAction::UpdateSurface {
                        id: workspace_b,
                        surface: surface_b,
                    },
                )
                .unwrap();

            let attention_id = AttentionItemId::from_uuid(Uuid::from_u128(70));
            hub.apply_attention_item(
                AttentionItem {
                    id: attention_id,
                    revision: 7,
                    observed_at_millis: 700,
                    target: AttentionTarget {
                        workspace: workspace_a,
                        pane: PaneId::from_uuid(workspace_a.as_uuid()),
                        tab_id: Some(WorkspaceTabId::from_uuid(session_a2_id)),
                        session_id: None,
                    },
                    state: AttentionState::NeedsYou,
                    title: "Review exact tab".into(),
                    unread: true,
                    agent_path: vec!["root".into(), "reviewer".into()],
                },
                cx,
            )
            .unwrap();
            let rows = hub.attention_items(workspace_a);
            assert_eq!(rows.len(), 1);
            assert_eq!(rows[0].state, AttentionState::NeedsYou);
            assert!(rows[0].unread);
            assert_eq!(rows[0].agent_path, ["root", "reviewer"]);

            hub.switch_to(workspace_a, cx);
            hub.switch_to(workspace_b, cx);
            assert_eq!(hub.navigation.active(), Some(workspace_b));

            // The retained pane's mutable active tab changes after the
            // observation. Activation must still use the captured tab ID.
            terminal_a.update(cx, |terminal, _| {
                terminal.select_tab(session_a1_id);
            });
            let moved_surface = terminal_surface(&hub.catalog, workspace_a, terminal_a.read(cx));
            hub.navigation
                .reduce(
                    &hub.catalog,
                    WorkspaceNavigationAction::UpdateSurface {
                        id: workspace_a,
                        surface: moved_surface,
                    },
                )
                .unwrap();
            assert_eq!(
                terminal_a.read(cx).tabs[terminal_a.read(cx).active_tab_index()].id,
                session_a1_id
            );

            let focus = hub
                .open_attention_item(workspace_a, attention_id, 7, cx)
                .unwrap();
            assert_eq!(hub.navigation.active(), Some(workspace_a));
            assert_eq!(focus.pane_id, PaneId::from_uuid(workspace_a.as_uuid()));
            assert_eq!(focus.tab_id.as_uuid(), session_a2_id);
            assert_eq!(
                hub.navigation.workspace(workspace_a).unwrap().surface.focus,
                Some(focus)
            );
            assert_eq!(
                terminal_a.read(cx).tabs[terminal_a.read(cx).active_tab_index()].id,
                session_a2_id
            );
            assert_ne!(session_a1_id, session_a2_id);
            assert!(!hub
                .attention
                .get(&workspace_a)
                .unwrap()
                .is_unread(attention_id));

            hub.apply_attention_item(
                AttentionItem {
                    id: attention_id,
                    revision: 8,
                    observed_at_millis: 800,
                    target: AttentionTarget {
                        workspace: workspace_a,
                        pane: focus.pane_id,
                        tab_id: Some(focus.tab_id),
                        session_id: None,
                    },
                    state: AttentionState::Blocked,
                    title: "Newer exact attention".into(),
                    unread: true,
                    agent_path: vec!["root".into(), "reviewer".into()],
                },
                cx,
            )
            .unwrap();
            assert_eq!(
                hub.open_attention_item(workspace_a, attention_id, 7, cx),
                Err(AttentionError::StaleObservation)
            );
            assert!(hub
                .attention
                .get(&workspace_a)
                .unwrap()
                .is_unread(attention_id));
        });
    }

    // SDTEST-1832
    #[test]
    fn retained_provider_activation_focuses_only_the_exact_native_tab_and_mapping() {
        let mut cx = TestAppContext::single();
        let (mut catalog, workspace_a, workspace_b, _checkout, _ssh_checkout, _ssh_connection) =
            fixture_catalog();
        catalog
            .set_platform_mapping(
                workspace_a,
                None,
                PlatformV2Mapping {
                    reconciliation_revision: 1,
                    project: PlatformContextRef {
                        id: "project-1".into(),
                        revision: 1,
                    },
                    checkout: PlatformContextRef {
                        id: "checkout-1".into(),
                        revision: 1,
                    },
                    user_workspace: PlatformContextRef {
                        id: "workspace-1".into(),
                        revision: 1,
                    },
                    reconciliation: PlatformMappingReconciliation::Exact {
                        reconciled_at_millis: 1,
                    },
                },
            )
            .unwrap();
        let initial_terminal = cx.update(|cx| cx.new(TerminalView::new));
        let hub = cx.update(|cx| {
            cx.new(|cx| WorkspaceHubView::new(Ok(catalog), &[], initial_terminal, cx))
        });
        cx.run_until_parked();

        let (other, _other_data, _other_input) =
            TerminalSession::spawn_ssh("Other".into(), 24, 80).unwrap();
        let other_id = other.id;
        let (provider, _provider_data, _provider_input) =
            TerminalSession::spawn_ssh("Provider".into(), 24, 80).unwrap();
        let provider_id = provider.id;
        let coordinate = ResourceCoordinate::new(
            ResourceAuthority::Automonique,
            ResourceKind::Session,
            ResourceId::new("provider-session-1").unwrap(),
        );

        hub.update(&mut cx, |hub, cx| {
            let terminal = hub
                .retained
                .get(&workspace_a)
                .unwrap()
                .read(cx)
                .terminal
                .clone();
            terminal.update(cx, |terminal, _| {
                terminal.add_session(other);
                terminal.add_session(provider);
                terminal.select_tab(other_id);
            });
            hub.switch_to(workspace_b, cx);
            let focus = WorkspaceFocus {
                pane_id: PaneId::from_uuid(workspace_a.as_uuid()),
                tab_id: WorkspaceTabId::from_uuid(provider_id),
            };
            hub.navigation
                .reduce(
                    &hub.catalog,
                    WorkspaceNavigationAction::UpdateSurface {
                        id: workspace_a,
                        surface: WorkspaceSurfaceState {
                            root: Some(shelldeck_core::workspace_navigation::PaneNode::Leaf(
                                PaneLeaf {
                                    id: focus.pane_id,
                                    tabs: vec![WorkspaceTab {
                                        id: focus.tab_id,
                                        title: "Provider".into(),
                                        content: WorkspaceTabContent::ProviderSession(
                                            ProviderSessionBinding {
                                                platform_user_workspace_id: "workspace-1".into(),
                                                session_id: coordinate.id.as_str().into(),
                                                run_id: None,
                                            },
                                        ),
                                    }],
                                    active_tab: Some(focus.tab_id),
                                },
                            )),
                            focus: Some(focus),
                        },
                    },
                )
                .unwrap();
            assert!(hub.open_retained_provider_pane(workspace_a, &coordinate, focus, cx));
            assert_eq!(hub.navigation.active(), Some(workspace_a));
            assert_eq!(
                terminal.read(cx).tabs[terminal.read(cx).active_tab_index()].id,
                provider_id
            );
            assert_ne!(provider_id, other_id);

            let foreign = ResourceCoordinate::new(
                ResourceAuthority::Automonique,
                ResourceKind::Session,
                ResourceId::new("foreign-session").unwrap(),
            );
            assert!(!hub.open_retained_provider_pane(workspace_a, &foreign, focus, cx));
        });
    }

    #[test]
    fn loading_existing_workspace_with_missing_root_is_fail_closed_and_localized() {
        let root = tempfile::tempdir().unwrap();
        let missing_root = std::fs::canonicalize(root.path()).unwrap();
        root.close().unwrap();
        let project_id = CatalogProjectId::new();
        let checkout_id = CatalogCheckoutId::new();
        let workspace = CatalogWorkspaceId::new();
        let mut project = ProjectRecord::new(project_id, "Missing checkout");
        project.add_checkout(ProjectCheckout::new(
            checkout_id,
            "missing",
            CheckoutHost::Local {
                device_label: "Local".into(),
                root: missing_root.clone(),
            },
            RepositoryIdentity {
                slug: "inklura/missing".into(),
                canonical_url: None,
            },
        ));
        let mut catalog = ProjectCatalog::default();
        catalog.insert_project(project).unwrap();
        catalog
            .create_workspace(WorkspaceLaunchRequest {
                id: workspace,
                project_id,
                checkout_id,
                name: "Missing".into(),
                intake: WorkspaceLaunchIntake::Manual,
            })
            .unwrap();

        let mut app = TestAppContext::single();
        let terminal = app.update(|cx| cx.new(TerminalView::new));
        let _hub = app.update(|cx| {
            cx.new(|cx| WorkspaceHubView::new(Ok(catalog), &[], terminal.clone(), cx))
        });
        terminal.update(&mut app, |terminal, cx| {
            let message = terminal.required_cwd_unavailable_message().unwrap();
            assert!(message.contains(&missing_root.display().to_string()));
            assert!(
                message.contains("Aucun terminal local") || message.contains("No local terminal")
            );
            terminal.spawn_local_terminal(cx);
            assert_eq!(terminal.tab_count(), 0);
            assert!(terminal.active_session().is_none());
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
    fn failed_multi_step_catalog_mutation_rolls_back_before_persistence() {
        let (mut catalog, workspace, ..) = fixture_catalog();
        let before = catalog.clone();
        let result = mutate_and_persist(
            &mut catalog,
            |catalog| {
                catalog
                    .archive_workspace(workspace)
                    .map_err(|error| error.to_string())?;
                Err::<(), _>("second mutation refused".to_string())
            },
            |_| panic!("a failed mutation must not be persisted"),
        );
        assert_eq!(result.unwrap_err(), "second mutation refused");
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

    // SDTEST-1737
    #[tokio::test]
    async fn existing_folder_executor_streams_intermediate_progress() {
        let temp = tempfile::tempdir().unwrap();
        let workspace = CatalogWorkspaceId::new();
        let operation = CreationOperationId::new();
        let request = WorkspaceExecutionRequest {
            workspace,
            project: CatalogProjectId::new(),
            source_checkout: CatalogCheckoutId::new(),
            checkout: CatalogCheckoutId::new(),
            created_checkout: None,
            operation,
            catalog_revision: 7,
            name: "Attach".into(),
            intake: WorkspaceLaunchIntake::Manual,
            host: AuthorizedLaunchHost::LocalExisting {
                authority: crate::terminal_view::AuthorizedLocalRoot::capture(temp.path()).unwrap(),
            },
            mode: WorkspaceLaunchMode::ExistingFolder,
        };
        let (tx, mut rx) = mpsc::unbounded_channel();
        NativeWorkspaceExecutor::default()
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

    #[derive(Default)]
    struct RecordingGitAdapter {
        call: parking_lot::Mutex<Option<(PathBuf, PathBuf, String, String)>>,
    }

    impl GitWorktreeAdapter for RecordingGitAdapter {
        fn prepare(
            &self,
            request: WorkspaceExecutionRequest,
            _cancelled: Arc<super::super::native_lifecycle::LaunchCancellation>,
        ) -> super::super::native_lifecycle::ExecutorFuture<
            Result<NativeLaunchOutcome, WorkspaceCreateFailure>,
        > {
            let AuthorizedLaunchHost::LocalWorktree {
                source_authority,
                target_root,
                branch,
                start_point,
            } = request.host
            else {
                unreachable!()
            };
            *self.call.lock() = Some((
                source_authority.path().to_path_buf(),
                target_root.clone(),
                branch,
                start_point,
            ));
            std::fs::create_dir_all(&target_root).unwrap();
            Box::pin(async move { Ok(NativeLaunchOutcome::test_ready(&target_root)) })
        }
    }

    #[tokio::test]
    async fn native_worktree_executor_uses_exact_typed_arguments_and_completes_all_phases() {
        let temp = tempfile::tempdir().unwrap();
        let source = std::fs::canonicalize(temp.path()).unwrap();
        let target = source.join("owned-worktree");
        let workspace = CatalogWorkspaceId::new();
        let operation = CreationOperationId::new();
        let git = Arc::new(RecordingGitAdapter::default());
        let request = WorkspaceExecutionRequest {
            workspace,
            project: CatalogProjectId::new(),
            source_checkout: CatalogCheckoutId::new(),
            checkout: CatalogCheckoutId::new(),
            created_checkout: None,
            operation,
            catalog_revision: 11,
            name: "Issue 127".into(),
            intake: WorkspaceLaunchIntake::Manual,
            host: AuthorizedLaunchHost::LocalWorktree {
                source_authority: crate::terminal_view::AuthorizedLocalRoot::capture(&source)
                    .unwrap(),
                target_root: target.clone(),
                branch: "fix/issue-127".into(),
                start_point: "origin/main".into(),
            },
            mode: WorkspaceLaunchMode::GitWorktree,
        };
        let (tx, mut rx) = mpsc::unbounded_channel();
        NativeWorkspaceExecutor::with_git(git.clone())
            .launch(request, tx)
            .await
            .unwrap();
        let mut phases = Vec::new();
        let mut completed = false;
        while let Some(event) = rx.recv().await {
            match event {
                WorkspaceCreateEvent::Progress { progress, .. } => phases.push(progress.phase),
                WorkspaceCreateEvent::Completed { workspace: id, .. } => {
                    assert_eq!(id, workspace);
                    completed = true;
                }
                other => panic!("unexpected event: {other:?}"),
            }
        }
        assert_eq!(
            git.call.lock().clone().unwrap(),
            (source, target, "fix/issue-127".into(), "origin/main".into())
        );
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
        assert!(completed);
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
    fn uncorroborated_completion_after_catalog_change_prevents_native_attach() {
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
                    project: hub.catalog.workspace(workspace).unwrap().project_id(),
                    source_checkout: checkout,
                    checkout,
                    created_checkout: None,
                    operation,
                    catalog_revision: starting_revision,
                    name: "Attach".into(),
                    intake: WorkspaceLaunchIntake::Manual,
                    host: AuthorizedLaunchHost::LocalExisting {
                        authority: crate::terminal_view::AuthorizedLocalRoot::capture(root.path())
                            .unwrap(),
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
                Some(BackgroundWorkspaceCreateState::Running { .. })
            ));
            assert_eq!(terminal.read(cx).tab_count(), 0);
            assert!(hub.pending_requests.contains_key(&workspace));
        });
    }

    #[test]
    fn uncorroborated_completion_for_vanished_folder_has_no_side_effect() {
        let mut app = TestAppContext::single();
        let (catalog, workspace, _, checkout, ..) = fixture_catalog();
        let terminal = app.update(|cx| cx.new(TerminalView::new));
        let hub = app.update(|cx| {
            cx.new(|cx| WorkspaceHubView::new(Ok(catalog), &[], terminal.clone(), cx))
        });
        let root = tempfile::tempdir().unwrap();
        let vanished = root.path().to_path_buf();
        let vanished_authority =
            crate::terminal_view::AuthorizedLocalRoot::capture(&vanished).unwrap();
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
                    project: hub.catalog.workspace(workspace).unwrap().project_id(),
                    source_checkout: checkout,
                    checkout,
                    created_checkout: None,
                    operation,
                    catalog_revision: revision,
                    name: "Vanished".into(),
                    intake: WorkspaceLaunchIntake::Manual,
                    host: AuthorizedLaunchHost::LocalExisting {
                        authority: vanished_authority.clone(),
                    },
                    mode: WorkspaceLaunchMode::ExistingFolder,
                },
            );
            terminal.update(cx, |terminal, cx| {
                assert!(terminal.set_default_cwd(&vanished).is_err());
                terminal.spawn_local_terminal(cx);
            });
            // L'action interactive précédant la complétion échoue fermée:
            // aucun PTY n'est créé dans le cwd du processus ou le HOME.
            assert_eq!(terminal.read(cx).tab_count(), 0);
            let message = terminal
                .read(cx)
                .required_cwd_unavailable_message()
                .unwrap();
            assert!(message.contains(&vanished.display().to_string()));
            assert!(
                message.contains("Aucun terminal local") || message.contains("No local terminal")
            );
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
                Some(BackgroundWorkspaceCreateState::Running { .. })
            ));
            assert!(hub.pending_requests.contains_key(&workspace));
        });
    }

    #[test]
    fn uncorroborated_completion_cannot_spawn_a_second_authorized_terminal() {
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
                    project: hub.catalog.workspace(workspace).unwrap().project_id(),
                    source_checkout: checkout,
                    checkout,
                    created_checkout: None,
                    operation,
                    catalog_revision: revision,
                    name: "Authorized".into(),
                    intake: WorkspaceLaunchIntake::Manual,
                    host: AuthorizedLaunchHost::LocalExisting {
                        authority: crate::terminal_view::AuthorizedLocalRoot::capture(
                            &canonical_root,
                        )
                        .unwrap(),
                    },
                    mode: WorkspaceLaunchMode::ExistingFolder,
                },
            );
            let authority =
                crate::terminal_view::AuthorizedLocalRoot::capture(&canonical_root).unwrap();
            terminal.update(cx, |terminal, cx| {
                terminal.install_authorized_default_cwd(&authority);
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
            assert_eq!(terminal.read(cx).tab_count(), 1);
            assert_eq!(
                terminal
                    .read(cx)
                    .active_session()
                    .and_then(|session| session.initial_cwd()),
                Some(canonical_root.as_path())
            );
            assert!(matches!(
                hub.creation.state(workspace),
                Some(BackgroundWorkspaceCreateState::Running { .. })
            ));
            assert!(hub.pending_requests.contains_key(&workspace));
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
                    project: hub.catalog.workspace(workspace).unwrap().project_id(),
                    source_checkout: checkout,
                    checkout,
                    created_checkout: None,
                    operation: operation_b,
                    catalog_revision: revision,
                    name: "Retry".into(),
                    intake: WorkspaceLaunchIntake::Manual,
                    host: AuthorizedLaunchHost::LocalExisting {
                        authority: crate::terminal_view::AuthorizedLocalRoot::capture(root.path())
                            .unwrap(),
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
                    project: hub.catalog.workspace(workspace).unwrap().project_id(),
                    source_checkout: checkout,
                    checkout,
                    created_checkout: None,
                    operation,
                    catalog_revision: revision,
                    name: "Cancel".into(),
                    intake: WorkspaceLaunchIntake::Manual,
                    host: AuthorizedLaunchHost::LocalExisting {
                        authority: crate::terminal_view::AuthorizedLocalRoot::capture(root.path())
                            .unwrap(),
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
                    project: hub.catalog.workspace(workspace).unwrap().project_id(),
                    source_checkout: checkout,
                    checkout,
                    created_checkout: None,
                    operation,
                    catalog_revision: revision,
                    name: "Too soon".into(),
                    intake: WorkspaceLaunchIntake::Manual,
                    host: AuthorizedLaunchHost::LocalExisting {
                        authority: crate::terminal_view::AuthorizedLocalRoot::capture(root.path())
                            .unwrap(),
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
