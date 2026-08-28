use super::*;
use crate::config::workspace_catalog::{
    CatalogProjectId, CheckoutHost, PlatformContextRef, PlatformMappingReconciliation,
    PlatformV2Mapping, ProjectCheckout, ProjectRecord, RepositoryIdentity, WorkspaceLaunchIntake,
    WorkspaceLaunchRequest,
};
use crate::workspace_navigation::{
    PaneLeaf, ProviderSessionBinding, WorkspaceCardAggregate, WorkspaceNavigationAction,
    WorkspaceNavigationState, WorkspaceTab, WorkspaceTabId,
};

fn uuid(value: u128) -> Uuid {
    Uuid::from_u128(value)
}

fn workspace(value: u128) -> CatalogWorkspaceId {
    CatalogWorkspaceId::from_uuid(uuid(value))
}

fn checkout(value: u128) -> CatalogCheckoutId {
    CatalogCheckoutId::from_uuid(uuid(value))
}

fn project(value: u128) -> CatalogProjectId {
    CatalogProjectId::from_uuid(uuid(value))
}

fn path(value: &str) -> WorkspaceRelativePath {
    WorkspaceRelativePath::new(value).unwrap()
}

fn temp_root(label: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "shelldeck-review-{label}-{}-{}",
        std::process::id(),
        Uuid::new_v4()
    ));
    std::fs::create_dir_all(&root).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    root
}

fn snapshot() -> ReviewSnapshot {
    ReviewSnapshot {
        workspace: workspace(1),
        checkout: checkout(2),
        revision: 9,
        observed_at_millis: 100,
        freshness: ObservationFreshness::Fresh,
        changes: vec![
            ReviewFileChange {
                path: path("src/main.rs"),
                section: ChangeSection::Unstaged,
                conflict: ChangeConflict::None,
                hunks: vec![ReviewHunk {
                    id: ReviewHunkId::from_uuid(uuid(3)),
                    header: "@@ -1 +1 @@".into(),
                    lines: vec![DiffLine {
                        kind: DiffLineKind::Added,
                        old_line: None,
                        new_line: Some(1),
                        text: "fn main() {}".into(),
                    }],
                }],
            },
            ReviewFileChange {
                path: path("Cargo.toml"),
                section: ChangeSection::Staged,
                conflict: ChangeConflict::BothModified,
                hunks: vec![ReviewHunk {
                    id: ReviewHunkId::from_uuid(uuid(4)),
                    header: "@@ -2 +2 @@".into(),
                    lines: vec![],
                }],
            },
            ReviewFileChange {
                path: path("notes.txt"),
                section: ChangeSection::Untracked,
                conflict: ChangeConflict::None,
                hunks: vec![],
            },
        ],
    }
}

fn actor() -> ActorIdentity {
    ActorIdentity {
        id: "actor-1".into(),
        display_name: "Reviewer".into(),
    }
}

fn repository_grant(expires: u64) -> AuthorityGrant {
    AuthorityGrant::repository(workspace(1), actor(), 7, expires, checkout(2), true)
}

fn current<'a>(
    target: MutationTargetEvidence<'a>,
    grant: &'a AuthorityGrant,
) -> CurrentMutationEvidence<'a> {
    CurrentMutationEvidence { target, grant }
}

fn catalog_for(workspace_id: CatalogWorkspaceId, platform_workspace: &str) -> ProjectCatalog {
    let mut catalog = ProjectCatalog::default();
    catalog
        .insert_project(ProjectRecord::new(project(30), "Project"))
        .unwrap();
    catalog
        .add_checkout(
            project(30),
            ProjectCheckout::new(
                checkout(2),
                "Checkout",
                CheckoutHost::Local {
                    device_label: "Local".into(),
                    root: std::env::temp_dir().join("shelldeck-review-checkout"),
                },
                RepositoryIdentity {
                    slug: "owner/repo".into(),
                    canonical_url: None,
                },
            ),
        )
        .unwrap();
    catalog
        .create_workspace(WorkspaceLaunchRequest {
            id: workspace_id,
            project_id: project(30),
            checkout_id: checkout(2),
            name: "Review".into(),
            intake: WorkspaceLaunchIntake::Manual,
        })
        .unwrap();
    catalog
        .set_platform_mapping(
            workspace_id,
            None,
            PlatformV2Mapping {
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
                    id: platform_workspace.into(),
                    revision: 1,
                },
                reconciliation: PlatformMappingReconciliation::Exact {
                    reconciled_at_millis: 1,
                },
            },
        )
        .unwrap();
    catalog
}

fn add_workspace(
    catalog: &mut ProjectCatalog,
    workspace_id: CatalogWorkspaceId,
    platform_workspace: &str,
) {
    catalog
        .create_workspace(WorkspaceLaunchRequest {
            id: workspace_id,
            project_id: project(30),
            checkout_id: checkout(2),
            name: "Review".into(),
            intake: WorkspaceLaunchIntake::Manual,
        })
        .unwrap();
    catalog
        .set_platform_mapping(
            workspace_id,
            None,
            PlatformV2Mapping {
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
                    id: platform_workspace.into(),
                    revision: 1,
                },
                reconciliation: PlatformMappingReconciliation::Exact {
                    reconciled_at_millis: 1,
                },
            },
        )
        .unwrap();
}

fn provider_projection(revision: u64) -> ProviderSessionProjection {
    ProviderSessionProjection {
        workspace: workspace(1),
        platform_user_workspace_id: "platform-workspace".into(),
        mapping_reconciliation_revision: 1,
        session_id: "session-1".into(),
        revision,
        authority_revision: 8,
        freshness: ObservationFreshness::Fresh,
        observed_actor: Some(actor()),
        can_send_comments: true,
        can_decide_approval: true,
        pending_approval_ids: BTreeSet::from(["approval-1".into()]),
    }
}

fn delivery_projection(revision: u64) -> DeliveryProjection {
    DeliveryProjection {
        workspace: workspace(1),
        revision,
        authority_revision: 12,
        freshness: ObservationFreshness::Fresh,
        authority: DeliveryAuthority {
            provider: "github".into(),
            repository: "owner/repo".into(),
            observed_actor: Some(actor()),
            can_retry_checks: true,
            can_merge: true,
        },
        checks: vec![DeliveryCheck {
            id: "linux".into(),
            name: "Linux".into(),
            state: DeliveryCheckState::Failed,
        }],
        pull_request: Some(PullRequestProjection {
            key: "42".into(),
            review_status: "approved".into(),
            merge_ready: true,
        }),
        state: DeliveryState::Ready,
    }
}

fn receipt(preview: &ReviewMutationPreview) -> ReviewMutationReceipt {
    ReviewMutationReceipt {
        operation: preview.operation(),
        idempotency_key: preview.idempotency_key(),
        workspace: preview.workspace(),
        target: preview.target().clone(),
        authority_revision: preview.authority_revision(),
        actor_id: preview.actor().id.clone(),
        outcome: MutationOutcome::Completed,
        recorded_at_millis: 20,
    }
}

fn review_comment(id: u128) -> ReviewCommentDraft {
    ReviewCommentDraft {
        id: ReviewCommentId::from_uuid(uuid(id)),
        author: actor().id,
        anchor: ReviewLineAnchor {
            review_revision: 9,
            path: path("src/main.rs"),
            section: ChangeSection::Unstaged,
            hunk: ReviewHunkId::from_uuid(uuid(3)),
            side: ReviewLineSide::New,
            line: 1,
        },
        body: "Please keep this exact behavior.".into(),
        selected: true,
    }
}

fn provider_surface(platform_workspace: &str) -> WorkspaceSurfaceState {
    WorkspaceSurfaceState {
        root: Some(PaneNode::Leaf(PaneLeaf {
            id: PaneId::from_uuid(uuid(40)),
            tabs: vec![WorkspaceTab {
                id: WorkspaceTabId::from_uuid(uuid(41)),
                title: "Agent".into(),
                content: WorkspaceTabContent::ProviderSession(ProviderSessionBinding {
                    platform_user_workspace_id: platform_workspace.into(),
                    session_id: "session-1".into(),
                    run_id: None,
                }),
            }],
            active_tab: Some(WorkspaceTabId::from_uuid(uuid(41))),
        })),
        focus: None,
    }
}

// SDTEST-1751
#[test]
fn sdtest_1751_combined_review_preserves_sections_and_validates_inert_previews() {
    assert_eq!(
        snapshot().combined_sections(),
        BTreeSet::from([
            ChangeSection::Staged,
            ChangeSection::Unstaged,
            ChangeSection::Untracked,
        ])
    );
    assert!(matches!(
        safe_preview(&path("index.html"), b"<script>fetch('/token')</script>"),
        SafePreview::HtmlSource { escaped, .. }
            if escaped == "&lt;script&gt;fetch(&#39;/token&#39;)&lt;/script&gt;"
    ));
    let mut png = vec![0_u8; 45];
    png[..8].copy_from_slice(b"\x89PNG\r\n\x1a\n");
    png[8..12].copy_from_slice(&13_u32.to_be_bytes());
    png[12..16].copy_from_slice(b"IHDR");
    png[16..20].copy_from_slice(&640_u32.to_be_bytes());
    png[20..24].copy_from_slice(&480_u32.to_be_bytes());
    png[37..41].copy_from_slice(b"IEND");
    assert!(matches!(
        safe_preview(&path("image.png"), &png),
        SafePreview::Image {
            width: 640,
            height: 480,
            byte_len: 45,
            ..
        }
    ));
    png[16..20].copy_from_slice(&20_000_u32.to_be_bytes());
    assert!(matches!(
        safe_preview(&path("bomb.png"), &png),
        SafePreview::Unsupported {
            category: "invalid_or_unsupported_image"
        }
    ));
    assert!(matches!(
        safe_preview(&path("broken.png"), b"\x89PNG\r\n\x1a\n"),
        SafePreview::Unsupported {
            category: "invalid_or_unsupported_image"
        }
    ));
}

// SDTEST-1752
#[test]
fn sdtest_1752_drafts_use_workspace_paths_monotone_cas_and_expected_identity() {
    let root = temp_root("drafts");
    let mut store = ReviewDraftStore::load_at(root.clone(), workspace(1)).unwrap();
    store
        .add(ReviewCommentDraft {
            id: ReviewCommentId::from_uuid(uuid(10)),
            author: "actor-1".into(),
            anchor: ReviewLineAnchor {
                review_revision: 9,
                path: path("src/main.rs"),
                section: ChangeSection::Unstaged,
                hunk: ReviewHunkId::from_uuid(uuid(3)),
                side: ReviewLineSide::New,
                line: 1,
            },
            body: "note".into(),
            selected: true,
        })
        .unwrap();
    assert_eq!(store.revision(), 0);
    store.save().unwrap();
    assert_eq!(store.revision(), 1);
    let mut first = ReviewDraftStore::load_at(root.clone(), workspace(1)).unwrap();
    let mut stale = ReviewDraftStore::load_at(root.clone(), workspace(1)).unwrap();
    first
        .select(ReviewCommentId::from_uuid(uuid(10)), false)
        .unwrap();
    first.save().unwrap();
    assert_eq!(first.revision(), 2);
    stale
        .select(ReviewCommentId::from_uuid(uuid(10)), false)
        .unwrap();
    assert!(matches!(
        stale.save(),
        Err(ReviewDraftError::RevisionConflict {
            expected: 1,
            actual: 2
        })
    ));
    let loaded = ReviewDraftStore::load_at(root.clone(), workspace(1)).unwrap();
    assert_eq!(loaded.revision(), 2);
    assert_eq!(loaded.workspace(), workspace(1));
    let wrong_path = workspace_state_path(&root, workspace(2), "drafts.json");
    ensure_private_directory(wrong_path.parent().unwrap()).unwrap();
    let wrong = ReviewDraftDisk {
        schema_version: REVIEW_DRAFT_SCHEMA,
        revision: 1,
        workspace: workspace(1),
        comments: vec![],
    };
    crate::util::atomic_write(&wrong_path, &serde_json::to_vec(&wrong).unwrap()).unwrap();
    assert!(matches!(
        ReviewDraftStore::load_at(root.clone(), workspace(2)),
        Err(ReviewDraftError::WrongWorkspace)
    ));
    let legacy_path = workspace_state_path(&root, workspace(3), "drafts.json");
    ensure_private_directory(legacy_path.parent().unwrap()).unwrap();
    let legacy = serde_json::json!({
        "schema_version": 1,
        "revision": 1,
        "workspace": workspace(3),
        "comments": []
    });
    crate::util::atomic_write(&legacy_path, &serde_json::to_vec(&legacy).unwrap()).unwrap();
    assert!(matches!(
        ReviewDraftStore::load_at(root.clone(), workspace(3)),
        Err(ReviewDraftError::UnsupportedSchema(1))
    ));
    std::fs::remove_dir_all(root).ok();
}

#[cfg(unix)]
// SDTEST-1759
#[test]
fn sdtest_1759_draft_persistence_refuses_links_and_reads_one_bounded_descriptor() {
    use std::os::unix::fs::symlink;

    let root = temp_root("draft-path-safety");
    let outside = temp_root("draft-path-outside");
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o755)).unwrap();
    ReviewDraftStore::load_at(root.clone(), workspace(1)).unwrap();
    assert_eq!(
        std::fs::metadata(&root).unwrap().permissions().mode() & 0o777,
        0o700
    );
    let linked_workspace = root.join(workspace(1).to_string());
    symlink(&outside, &linked_workspace).unwrap();
    assert!(ReviewDraftStore::load_at(root.clone(), workspace(1)).is_err());
    std::fs::remove_file(&linked_workspace).unwrap();

    let draft_path = workspace_state_path(&root, workspace(1), "drafts.json");
    ensure_private_directory(draft_path.parent().unwrap()).unwrap();
    let outside_file = outside.join("outside.json");
    std::fs::write(&outside_file, b"outside").unwrap();
    symlink(&outside_file, &draft_path).unwrap();
    assert!(ReviewDraftStore::load_at(root.clone(), workspace(1)).is_err());
    std::fs::remove_file(&draft_path).unwrap();

    std::fs::write(&draft_path, b"original").unwrap();
    let replacement = draft_path.with_extension("replacement");
    let bytes = storage::bounded_read_after_open_for_test(&draft_path, 8, || {
        std::fs::rename(&draft_path, &replacement).unwrap();
        std::fs::write(&draft_path, b"replacement-is-too-large").unwrap();
    })
    .unwrap()
    .unwrap();
    assert_eq!(bytes, b"original");
    std::fs::remove_file(&draft_path).unwrap();
    std::fs::rename(&replacement, &draft_path).unwrap();
    assert_eq!(
        storage::bounded_read_after_open_for_test(&draft_path, 8, || {
            use std::io::Write;
            let mut writer = std::fs::OpenOptions::new()
                .append(true)
                .open(&draft_path)
                .unwrap();
            writer.write_all(b"-growth").unwrap();
        })
        .unwrap()
        .unwrap()
        .len(),
        9
    );

    std::fs::remove_file(&draft_path).unwrap();
    symlink(&outside_file, lock_path(&draft_path)).unwrap();
    let mut store = ReviewDraftStore::load_at(root.clone(), workspace(1)).unwrap();
    store.add(review_comment(99)).unwrap();
    assert!(store.save().is_err());
    std::fs::remove_dir_all(root).ok();
    std::fs::remove_dir_all(outside).ok();
}

// SDTEST-1753
#[test]
fn sdtest_1753_only_registered_unexpired_previews_can_submit() {
    let root = temp_root("capability");
    let mut workflow = ReviewWorkflow::load_at(root.clone(), workspace(1)).unwrap();
    let provider = AuthorityGrant::provider_session(
        workspace(1),
        actor(),
        8,
        100,
        "session-1".into(),
        "platform-workspace".into(),
        1,
        true,
        true,
    );
    assert_eq!(
        workflow.prepare(
            MutationTargetEvidence::LocalReview(&snapshot()),
            &provider,
            ReviewMutationKind::StageHunks {
                hunks: vec![ReviewHunkId::from_uuid(uuid(3))],
            },
            10,
        ),
        Err(ReviewWorkflowError::WrongAuthority)
    );
    let repository = repository_grant(20);
    let preview = workflow
        .prepare(
            MutationTargetEvidence::LocalReview(&snapshot()),
            &repository,
            ReviewMutationKind::StageHunks {
                hunks: vec![ReviewHunkId::from_uuid(uuid(3))],
            },
            10,
        )
        .unwrap();
    let mut forged_json = serde_json::to_value(&preview).unwrap();
    forged_json["actor"]["id"] = "attacker".into();
    let forged: ReviewMutationPreview = serde_json::from_value(forged_json).unwrap();
    assert_eq!(
        workflow.submit(
            &forged,
            current(
                MutationTargetEvidence::LocalReview(&snapshot()),
                &repository
            ),
            11,
        ),
        Err(ReviewWorkflowError::MutationAlreadyPending)
    );
    assert_eq!(
        workflow.submit(
            &preview,
            current(
                MutationTargetEvidence::LocalReview(&snapshot()),
                &repository
            ),
            20,
        ),
        Err(ReviewWorkflowError::ExpiredGrant)
    );

    let current_repository = repository_grant(1_000);
    let current_preview = workflow
        .prepare(
            MutationTargetEvidence::LocalReview(&snapshot()),
            &current_repository,
            ReviewMutationKind::StageHunks {
                hunks: vec![ReviewHunkId::from_uuid(uuid(3))],
            },
            30,
        )
        .unwrap();
    let revoked = AuthorityGrant::repository(
        workspace(1),
        actor(),
        current_repository.revision,
        current_repository.expires_at_millis,
        checkout(2),
        false,
    );
    assert_eq!(
        workflow.submit(
            &current_preview,
            current(MutationTargetEvidence::LocalReview(&snapshot()), &revoked),
            31,
        ),
        Err(ReviewWorkflowError::CurrentTargetMismatch)
    );
    let superseded = AuthorityGrant::repository(
        workspace(1),
        actor(),
        current_repository.revision + 1,
        current_repository.expires_at_millis,
        checkout(2),
        true,
    );
    assert_eq!(
        workflow.submit(
            &current_preview,
            current(
                MutationTargetEvidence::LocalReview(&snapshot()),
                &superseded,
            ),
            31,
        ),
        Err(ReviewWorkflowError::CurrentTargetMismatch)
    );
    workflow
        .submit(
            &current_preview,
            current(
                MutationTargetEvidence::LocalReview(&snapshot()),
                &current_repository,
            ),
            31,
        )
        .unwrap();
    std::fs::remove_dir_all(root).ok();
}

// SDTEST-1760
#[test]
fn sdtest_1760_comment_batches_require_exact_workspace_surface_and_unique_anchors() {
    let root = temp_root("comment-workspace");
    let review = snapshot();
    let mut catalog = catalog_for(workspace(1), "platform-workspace");
    let surface = provider_surface("platform-workspace");
    let grant = AuthorityGrant::provider_session(
        workspace(1),
        actor(),
        8,
        1_000,
        "session-1".into(),
        "platform-workspace".into(),
        1,
        true,
        false,
    );
    let mut workflow = ReviewWorkflow::load_at(root.clone(), workspace(1)).unwrap();
    let kind = ReviewMutationKind::SendComments {
        session_id: "session-1".into(),
        comments: vec![review_comment(50)],
    };
    let preview = workflow
        .prepare(
            MutationTargetEvidence::ReviewAndProviderSession {
                review: &review,
                session: &provider_projection(5),
                catalog: &catalog,
                surface: &surface,
            },
            &grant,
            kind,
            10,
        )
        .unwrap();
    let receipt_preview = workflow
        .prepare(
            MutationTargetEvidence::ReviewAndProviderSession {
                review: &review,
                session: &provider_projection(5),
                catalog: &catalog,
                surface: &surface,
            },
            &grant,
            ReviewMutationKind::SendComments {
                session_id: "session-1".into(),
                comments: vec![review_comment(55)],
            },
            10,
        )
        .unwrap();
    workflow
        .submit(
            &receipt_preview,
            current(
                MutationTargetEvidence::ReviewAndProviderSession {
                    review: &review,
                    session: &provider_projection(5),
                    catalog: &catalog,
                    surface: &surface,
                },
                &grant,
            ),
            11,
        )
        .unwrap();
    let mut wrong_platform_receipt = receipt(&receipt_preview);
    if let MutationTargetFence::ReviewAndProviderSession {
        platform_user_workspace_id,
        ..
    } = &mut wrong_platform_receipt.target
    {
        *platform_user_workspace_id = "other-platform-workspace".into();
    }
    assert_eq!(
        workflow.apply_transport_result(
            receipt_preview.operation(),
            MutationTransportResult::Receipt(wrong_platform_receipt),
        ),
        Err(ReviewWorkflowError::ReceiptMismatch)
    );

    let mut foreign_session = provider_projection(5);
    foreign_session.workspace = workspace(2);
    assert_eq!(
        workflow.submit(
            &preview,
            current(
                MutationTargetEvidence::ReviewAndProviderSession {
                    review: &review,
                    session: &foreign_session,
                    catalog: &catalog,
                    surface: &surface,
                },
                &grant,
            ),
            11,
        ),
        Err(ReviewWorkflowError::CurrentTargetMismatch)
    );
    let foreign_grant = AuthorityGrant::provider_session(
        workspace(2),
        actor(),
        8,
        1_000,
        "session-1".into(),
        "platform-workspace".into(),
        1,
        true,
        false,
    );
    assert_eq!(
        workflow.submit(
            &preview,
            current(
                MutationTargetEvidence::ReviewAndProviderSession {
                    review: &review,
                    session: &provider_projection(5),
                    catalog: &catalog,
                    surface: &surface,
                },
                &foreign_grant,
            ),
            11,
        ),
        Err(ReviewWorkflowError::WrongWorkspace)
    );
    let mut foreign_review = review.clone();
    foreign_review.workspace = workspace(2);
    assert_eq!(
        workflow.submit(
            &preview,
            current(
                MutationTargetEvidence::ReviewAndProviderSession {
                    review: &foreign_review,
                    session: &provider_projection(5),
                    catalog: &catalog,
                    surface: &surface,
                },
                &grant,
            ),
            11,
        ),
        Err(ReviewWorkflowError::WrongWorkspace)
    );
    let foreign_surface = provider_surface("foreign-platform-workspace");
    assert_eq!(
        workflow.submit(
            &preview,
            current(
                MutationTargetEvidence::ReviewAndProviderSession {
                    review: &review,
                    session: &provider_projection(5),
                    catalog: &catalog,
                    surface: &foreign_surface,
                },
                &grant,
            ),
            11,
        ),
        Err(ReviewWorkflowError::CurrentTargetMismatch)
    );
    assert!(matches!(
        preview.target(),
        MutationTargetFence::ReviewAndProviderSession {
            platform_user_workspace_id,
            mapping_reconciliation_revision: 1,
            ..
        } if platform_user_workspace_id == "platform-workspace"
    ));
    catalog
        .set_platform_mapping(
            workspace(1),
            Some(1),
            PlatformV2Mapping {
                reconciliation_revision: 2,
                project: PlatformContextRef {
                    id: "platform-project".into(),
                    revision: 1,
                },
                checkout: PlatformContextRef {
                    id: "platform-checkout".into(),
                    revision: 1,
                },
                user_workspace: PlatformContextRef {
                    id: "platform-workspace".into(),
                    revision: 1,
                },
                reconciliation: PlatformMappingReconciliation::Diverged {
                    observed_at_millis: 2,
                },
            },
        )
        .unwrap();
    catalog
        .set_platform_mapping(
            workspace(1),
            Some(2),
            PlatformV2Mapping {
                reconciliation_revision: 3,
                project: PlatformContextRef {
                    id: "platform-project".into(),
                    revision: 1,
                },
                checkout: PlatformContextRef {
                    id: "platform-checkout".into(),
                    revision: 1,
                },
                user_workspace: PlatformContextRef {
                    id: "remapped-platform-workspace".into(),
                    revision: 2,
                },
                reconciliation: PlatformMappingReconciliation::Exact {
                    reconciled_at_millis: 3,
                },
            },
        )
        .unwrap();
    let remapped_surface = provider_surface("remapped-platform-workspace");
    let mut remapped_session = provider_projection(5);
    remapped_session.platform_user_workspace_id = "remapped-platform-workspace".into();
    remapped_session.mapping_reconciliation_revision = 3;
    let remapped_grant = AuthorityGrant::provider_session(
        workspace(1),
        actor(),
        8,
        1_000,
        "session-1".into(),
        "remapped-platform-workspace".into(),
        3,
        true,
        false,
    );
    assert_eq!(
        workflow.submit(
            &preview,
            current(
                MutationTargetEvidence::ReviewAndProviderSession {
                    review: &review,
                    session: &remapped_session,
                    catalog: &catalog,
                    surface: &remapped_surface,
                },
                &remapped_grant,
            ),
            11,
        ),
        Err(ReviewWorkflowError::CurrentTargetMismatch)
    );

    let duplicate = review_comment(51);
    assert_eq!(
        workflow.prepare(
            MutationTargetEvidence::ReviewAndProviderSession {
                review: &review,
                session: &remapped_session,
                catalog: &catalog,
                surface: &remapped_surface,
            },
            &remapped_grant,
            ReviewMutationKind::SendComments {
                session_id: "session-1".into(),
                comments: vec![duplicate.clone(), duplicate],
            },
            11,
        ),
        Err(ReviewWorkflowError::InvalidSelection)
    );

    let mut aliased_review = review.clone();
    aliased_review.changes.push(ReviewFileChange {
        path: path("src/main.rs"),
        section: ChangeSection::Staged,
        conflict: ChangeConflict::None,
        hunks: vec![ReviewHunk {
            id: ReviewHunkId::from_uuid(uuid(52)),
            header: "@@ -1 +1 @@".into(),
            lines: vec![DiffLine {
                kind: DiffLineKind::Added,
                old_line: None,
                new_line: Some(1),
                text: "fn main() {}".into(),
            }],
        }],
    });
    assert!(aliased_review.contains_anchor(&review_comment(53).anchor));
    let mut wrong_hunk = review_comment(54);
    wrong_hunk.anchor.section = ChangeSection::Staged;
    wrong_hunk.anchor.hunk = ReviewHunkId::from_uuid(uuid(3));
    assert!(!aliased_review.contains_anchor(&wrong_hunk.anchor));

    let mut duplicate_lines = review.clone();
    let duplicate_line = duplicate_lines.changes[0].hunks[0].lines[0].clone();
    duplicate_lines.changes[0].hunks[0]
        .lines
        .push(duplicate_line);
    assert!(matches!(
        validate_fresh_review(&duplicate_lines),
        Err(ReviewWorkflowError::BoundsExceeded(_))
    ));
    std::fs::remove_dir_all(root).ok();
}

// SDTEST-1754
#[test]
fn sdtest_1754_dispatched_ledger_recovers_only_by_original_receipt() {
    let root = temp_root("ledger");
    let mut workflow = ReviewWorkflow::load_at(root.clone(), workspace(1)).unwrap();
    let repository = repository_grant(1_000);
    let preview = workflow
        .prepare(
            MutationTargetEvidence::LocalReview(&snapshot()),
            &repository,
            ReviewMutationKind::StageHunks {
                hunks: vec![ReviewHunkId::from_uuid(uuid(3))],
            },
            10,
        )
        .unwrap();
    workflow
        .submit(
            &preview,
            current(
                MutationTargetEvidence::LocalReview(&snapshot()),
                &repository,
            ),
            11,
        )
        .unwrap();
    drop(workflow);
    let recovered = ReviewWorkflow::load_at(root.clone(), workspace(1)).unwrap();
    let recovered_revision = recovered.revision();
    let lookups = recovered.reconciliation_lookups();
    assert_eq!(lookups.len(), 1);
    assert_eq!(lookups[0].idempotency_key, preview.idempotency_key());
    drop(recovered);
    let mut recovered = ReviewWorkflow::load_at(root.clone(), workspace(1)).unwrap();
    assert_eq!(recovered.revision(), recovered_revision);
    assert_eq!(
        recovered.submit(
            &preview,
            current(
                MutationTargetEvidence::LocalReview(&snapshot()),
                &repository
            ),
            12,
        ),
        Err(ReviewWorkflowError::MutationAlreadyPending)
    );
    let mut mismatched = receipt(&preview);
    mismatched.authority_revision += 1;
    assert_eq!(
        recovered.apply_reconciled_receipt(mismatched),
        Err(ReviewWorkflowError::ReceiptMismatch)
    );
    recovered
        .apply_reconciled_receipt(receipt(&preview))
        .unwrap();
    drop(recovered);
    let completed = ReviewWorkflow::load_at(root.clone(), workspace(1)).unwrap();
    assert!(matches!(
        completed.mutation(preview.operation()).unwrap().state(),
        PendingMutationState::Completed(_)
    ));
    assert_eq!(completed.recoverable_mutations().len(), 1);
    drop(completed);
    let mut completed = ReviewWorkflow::load_at(root.clone(), workspace(1)).unwrap();
    completed.acknowledge_terminal(preview.operation()).unwrap();
    drop(completed);
    assert!(ReviewWorkflow::load_at(root.clone(), workspace(1))
        .unwrap()
        .recoverable_mutations()
        .is_empty());
    std::fs::remove_dir_all(root).ok();
}

// SDTEST-1761
#[test]
fn sdtest_1761_prepared_and_refused_records_remain_until_explicit_ack() {
    let root = temp_root("ledger-terminal-ack");
    let repository = repository_grant(1_000);
    let mut workflow = ReviewWorkflow::load_at(root.clone(), workspace(1)).unwrap();
    let preview = workflow
        .prepare(
            MutationTargetEvidence::LocalReview(&snapshot()),
            &repository,
            ReviewMutationKind::StageHunks {
                hunks: vec![ReviewHunkId::from_uuid(uuid(3))],
            },
            10,
        )
        .unwrap();
    drop(workflow);
    let mut recovered = ReviewWorkflow::load_at(root.clone(), workspace(1)).unwrap();
    let records = recovered.recoverable_mutations();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].preview().operation(), preview.operation());
    assert!(matches!(records[0].state(), PendingMutationState::Prepared));
    assert_eq!(
        recovered.acknowledge_terminal(preview.operation()),
        Err(ReviewWorkflowError::MutationAlreadyPending)
    );
    recovered.abandon_prepared(preview.operation()).unwrap();
    drop(recovered);
    let mut recovered = ReviewWorkflow::load_at(root.clone(), workspace(1)).unwrap();
    assert!(matches!(
        recovered
            .recoverable_mutations()
            .first()
            .unwrap()
            .state(),
        PendingMutationState::Refused { category }
            if category == "abandoned_before_dispatch"
    ));
    recovered.acknowledge_terminal(preview.operation()).unwrap();
    drop(recovered);
    assert!(ReviewWorkflow::load_at(root.clone(), workspace(1))
        .unwrap()
        .recoverable_mutations()
        .is_empty());
    std::fs::remove_dir_all(root).ok();
}

// SDTEST-1762
#[test]
fn sdtest_1762_reconciliation_not_found_terminates_without_redispatch() {
    let root = temp_root("ledger-not-found");
    let mut workflow = ReviewWorkflow::load_at(root.clone(), workspace(1)).unwrap();
    let repository = repository_grant(1_000);
    let preview = workflow
        .prepare(
            MutationTargetEvidence::LocalReview(&snapshot()),
            &repository,
            ReviewMutationKind::StageHunks {
                hunks: vec![ReviewHunkId::from_uuid(uuid(3))],
            },
            10,
        )
        .unwrap();
    workflow
        .submit(
            &preview,
            current(
                MutationTargetEvidence::LocalReview(&snapshot()),
                &repository,
            ),
            11,
        )
        .unwrap();
    drop(workflow);
    let mut recovered = ReviewWorkflow::load_at(root.clone(), workspace(1)).unwrap();
    recovered
        .apply_reconciliation_refusal(preview.operation(), "receipt_not_found".into())
        .unwrap();
    assert!(matches!(
        recovered.mutation(preview.operation()).unwrap().state(),
        PendingMutationState::Refused { category } if category == "receipt_not_found"
    ));
    assert_eq!(
        recovered.submit(
            &preview,
            current(
                MutationTargetEvidence::LocalReview(&snapshot()),
                &repository
            ),
            12,
        ),
        Err(ReviewWorkflowError::MutationAlreadyPending)
    );
    drop(recovered);
    let mut recovered = ReviewWorkflow::load_at(root.clone(), workspace(1)).unwrap();
    assert!(matches!(
        recovered
            .recoverable_mutations()
            .first()
            .unwrap()
            .state(),
        PendingMutationState::Refused { category } if category == "receipt_not_found"
    ));
    recovered.acknowledge_terminal(preview.operation()).unwrap();
    std::fs::remove_dir_all(root).ok();
}

// SDTEST-1755
#[test]
fn sdtest_1755_attention_read_state_replays_and_duplicate_coordinates_fail_closed() {
    let mut catalog = catalog_for(workspace(1), "platform-workspace");
    let pane = PaneId::from_uuid(uuid(20));
    let tab = WorkspaceTabId::from_uuid(uuid(21));
    let provider_tab = |id, session: &str| WorkspaceTab {
        id: WorkspaceTabId::from_uuid(uuid(id)),
        title: "Agent".into(),
        content: WorkspaceTabContent::ProviderSession(ProviderSessionBinding {
            platform_user_workspace_id: "platform-workspace".into(),
            session_id: session.into(),
            run_id: None,
        }),
    };
    let surface = WorkspaceSurfaceState {
        root: Some(PaneNode::Leaf(PaneLeaf {
            id: pane,
            tabs: vec![provider_tab(21, "session-1")],
            active_tab: Some(tab),
        })),
        focus: None,
    };
    let id = AttentionItemId::from_uuid(uuid(22));
    let item = AttentionItem {
        id,
        revision: 1,
        observed_at_millis: 50,
        target: AttentionTarget {
            workspace: workspace(1),
            pane,
            session_id: Some("session-1".into()),
        },
        state: AttentionState::NeedsYou,
        title: "Approval requested".into(),
        unread: true,
        agent_path: vec!["root".into(), "reviewer".into()],
    };
    let mut board = AttentionBoard::new(workspace(1));
    board.apply(item.clone()).unwrap();
    let mut navigation = WorkspaceNavigationState::default();
    navigation
        .reduce(
            &catalog,
            WorkspaceNavigationAction::Retain {
                id: workspace(1),
                surface,
                card: WorkspaceCardAggregate::default(),
            },
        )
        .unwrap();
    assert_eq!(
        board.open_target(id, workspace(2), &catalog, &navigation),
        Err(AttentionError::WrongWorkspace)
    );
    board
        .open_target(id, workspace(1), &catalog, &navigation)
        .unwrap();
    assert!(!board.is_unread(id));
    assert!(!board.apply(item).unwrap());
    assert!(!board.is_unread(id));
    let duplicate_surface = WorkspaceSurfaceState {
        root: Some(PaneNode::Leaf(PaneLeaf {
            id: pane,
            tabs: vec![provider_tab(23, "session-1"), provider_tab(24, "session-1")],
            active_tab: Some(WorkspaceTabId::from_uuid(uuid(23))),
        })),
        focus: None,
    };
    navigation
        .reduce(
            &catalog,
            WorkspaceNavigationAction::UpdateSurface {
                id: workspace(1),
                surface: duplicate_surface,
            },
        )
        .unwrap();
    assert_eq!(
        board.open_target(id, workspace(1), &catalog, &navigation),
        Err(AttentionError::DuplicateSessionCoordinate)
    );

    add_workspace(&mut catalog, workspace(2), "foreign-platform-workspace");
    let browser_pane = PaneId::from_uuid(uuid(25));
    let local_browser_tab = WorkspaceTabId::from_uuid(uuid(26));
    let foreign_browser_tab = WorkspaceTabId::from_uuid(uuid(27));
    let browser_surface = |tab_id| WorkspaceSurfaceState {
        root: Some(PaneNode::Leaf(PaneLeaf {
            id: browser_pane,
            tabs: vec![WorkspaceTab {
                id: tab_id,
                title: "Browser".into(),
                content: WorkspaceTabContent::Browser {
                    location: "https://example.invalid".into(),
                },
            }],
            active_tab: Some(tab_id),
        })),
        focus: None,
    };
    let mut keyed_navigation = WorkspaceNavigationState::default();
    keyed_navigation
        .reduce(
            &catalog,
            WorkspaceNavigationAction::Retain {
                id: workspace(2),
                surface: browser_surface(foreign_browser_tab),
                card: WorkspaceCardAggregate::default(),
            },
        )
        .unwrap();
    let browser_id = AttentionItemId::from_uuid(uuid(28));
    board
        .apply(AttentionItem {
            id: browser_id,
            revision: 1,
            observed_at_millis: 60,
            target: AttentionTarget {
                workspace: workspace(1),
                pane: browser_pane,
                session_id: None,
            },
            state: AttentionState::NeedsYou,
            title: "Review browser result".into(),
            unread: true,
            agent_path: vec!["root".into()],
        })
        .unwrap();
    assert_eq!(
        board.open_target(browser_id, workspace(1), &catalog, &keyed_navigation),
        Err(AttentionError::InvalidSurface)
    );
    assert!(board.is_unread(browser_id));
    keyed_navigation
        .reduce(
            &catalog,
            WorkspaceNavigationAction::Retain {
                id: workspace(1),
                surface: browser_surface(local_browser_tab),
                card: WorkspaceCardAggregate::default(),
            },
        )
        .unwrap();
    assert_eq!(
        board
            .open_target(browser_id, workspace(1), &catalog, &keyed_navigation)
            .unwrap(),
        WorkspaceFocus {
            pane_id: browser_pane,
            tab_id: local_browser_tab,
        }
    );
    assert!(!board.is_unread(browser_id));
}

// SDTEST-1756
#[test]
fn sdtest_1756_delivery_mutations_require_exact_fresh_projection_revision() {
    let root = temp_root("delivery");
    let delivery = delivery_projection(3);
    let grant = AuthorityGrant::delivery(
        workspace(1),
        actor(),
        12,
        1_000,
        "github".into(),
        "owner/repo".into(),
        true,
        true,
    );
    let mut workflow = ReviewWorkflow::load_at(root.clone(), workspace(1)).unwrap();
    let preview = workflow
        .prepare(
            MutationTargetEvidence::Delivery(&delivery),
            &grant,
            ReviewMutationKind::MergePullRequest {
                provider: "github".into(),
                repository: "owner/repo".into(),
                pull_request: "42".into(),
            },
            10,
        )
        .unwrap();
    let mut advanced = delivery.clone();
    advanced.revision = 4;
    assert_eq!(
        workflow.submit(
            &preview,
            current(MutationTargetEvidence::Delivery(&advanced), &grant),
            11,
        ),
        Err(ReviewWorkflowError::CurrentTargetMismatch)
    );
    let mut stale = delivery.clone();
    stale.freshness = ObservationFreshness::Stale;
    assert!(matches!(
        workflow.submit(
            &preview,
            current(MutationTargetEvidence::Delivery(&stale), &grant),
            11,
        ),
        Err(ReviewWorkflowError::CurrentTargetMismatch)
    ));
    workflow
        .submit(
            &preview,
            current(MutationTargetEvidence::Delivery(&delivery), &grant),
            11,
        )
        .unwrap();
    let mut wrong_target_receipt = receipt(&preview);
    if let MutationTargetFence::DeliveryPullRequest {
        delivery_revision, ..
    } = &mut wrong_target_receipt.target
    {
        *delivery_revision += 1;
    }
    assert_eq!(
        workflow.apply_transport_result(
            preview.operation(),
            MutationTransportResult::Receipt(wrong_target_receipt),
        ),
        Err(ReviewWorkflowError::ReceiptMismatch)
    );

    let mut board = DeliveryBoard::default();
    board.apply(delivery.clone()).unwrap();
    let mut higher_stale = delivery.clone();
    higher_stale.revision += 1;
    higher_stale.freshness = ObservationFreshness::Stale;
    assert_eq!(
        board.apply(higher_stale),
        Err(DeliveryProjectionError::StaleObservation)
    );
    let mut higher_unknown = delivery.clone();
    higher_unknown.revision += 2;
    higher_unknown.freshness = ObservationFreshness::Unknown;
    assert_eq!(
        board.apply(higher_unknown),
        Err(DeliveryProjectionError::StaleObservation)
    );
    assert_eq!(board.get(workspace(1)).unwrap(), &delivery);
    let mut higher_fresh = delivery.clone();
    higher_fresh.revision += 1;
    board.apply(higher_fresh.clone()).unwrap();
    assert_eq!(board.get(workspace(1)).unwrap(), &higher_fresh);
    std::fs::remove_dir_all(root).ok();
}

// SDTEST-1757
#[test]
fn sdtest_1757_approval_is_bound_to_pending_id_and_session_revision() {
    let root = temp_root("approval");
    let session = provider_projection(5);
    let grant = AuthorityGrant::provider_session(
        workspace(1),
        actor(),
        8,
        1_000,
        "session-1".into(),
        "platform-workspace".into(),
        1,
        true,
        true,
    );
    let mut workflow = ReviewWorkflow::load_at(root.clone(), workspace(1)).unwrap();
    let preview = workflow
        .prepare(
            MutationTargetEvidence::ProviderSession(&session),
            &grant,
            ReviewMutationKind::DecideApproval {
                session_id: "session-1".into(),
                approval_id: "approval-1".into(),
                decision: ApprovalDecision::Approve,
            },
            10,
        )
        .unwrap();
    let mut advanced = session.clone();
    advanced.revision += 1;
    advanced.pending_approval_ids.clear();
    assert!(matches!(
        workflow.submit(
            &preview,
            current(MutationTargetEvidence::ProviderSession(&advanced), &grant),
            11,
        ),
        Err(ReviewWorkflowError::InvalidSelection | ReviewWorkflowError::CurrentTargetMismatch)
    ));
    std::fs::remove_dir_all(root).ok();
}
