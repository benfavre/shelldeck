use super::*;
use crate::workspace_navigation::{PaneLeaf, ProviderSessionBinding, WorkspaceTab, WorkspaceTabId};

fn uuid(value: u128) -> Uuid {
    Uuid::from_u128(value)
}

fn workspace(value: u128) -> CatalogWorkspaceId {
    CatalogWorkspaceId::from_uuid(uuid(value))
}

fn checkout(value: u128) -> CatalogCheckoutId {
    CatalogCheckoutId::from_uuid(uuid(value))
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

fn provider_projection(revision: u64) -> ProviderSessionProjection {
    ProviderSessionProjection {
        workspace: workspace(1),
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
    std::fs::remove_dir_all(root).ok();
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
    let preview = workflow
        .prepare(
            MutationTargetEvidence::LocalReview(&snapshot()),
            &repository_grant(20),
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
            MutationTargetEvidence::LocalReview(&snapshot()),
            11,
        ),
        Err(ReviewWorkflowError::MutationAlreadyPending)
    );
    assert_eq!(
        workflow.submit(
            &preview,
            MutationTargetEvidence::LocalReview(&snapshot()),
            20,
        ),
        Err(ReviewWorkflowError::ExpiredGrant)
    );
    std::fs::remove_dir_all(root).ok();
}

// SDTEST-1754
#[test]
fn sdtest_1754_dispatched_ledger_recovers_only_by_original_receipt() {
    let root = temp_root("ledger");
    let mut workflow = ReviewWorkflow::load_at(root.clone(), workspace(1)).unwrap();
    let preview = workflow
        .prepare(
            MutationTargetEvidence::LocalReview(&snapshot()),
            &repository_grant(1_000),
            ReviewMutationKind::StageHunks {
                hunks: vec![ReviewHunkId::from_uuid(uuid(3))],
            },
            10,
        )
        .unwrap();
    workflow
        .submit(
            &preview,
            MutationTargetEvidence::LocalReview(&snapshot()),
            11,
        )
        .unwrap();
    drop(workflow);
    let mut recovered = ReviewWorkflow::load_at(root.clone(), workspace(1)).unwrap();
    let lookups = recovered.reconciliation_lookups();
    assert_eq!(lookups.len(), 1);
    assert_eq!(lookups[0].idempotency_key, preview.idempotency_key());
    assert_eq!(
        recovered.submit(
            &preview,
            MutationTargetEvidence::LocalReview(&snapshot()),
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
    std::fs::remove_dir_all(root).ok();
}

// SDTEST-1755
#[test]
fn sdtest_1755_attention_read_state_replays_and_duplicate_coordinates_fail_closed() {
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
    assert_eq!(
        board.open_target(id, workspace(2), &surface),
        Err(AttentionError::WrongWorkspace)
    );
    board.open_target(id, workspace(1), &surface).unwrap();
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
    assert_eq!(
        board.open_target(id, workspace(1), &duplicate_surface),
        Err(AttentionError::DuplicateSessionCoordinate)
    );
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
        workflow.submit(&preview, MutationTargetEvidence::Delivery(&advanced), 11),
        Err(ReviewWorkflowError::CurrentTargetMismatch)
    );
    let mut stale = delivery.clone();
    stale.freshness = ObservationFreshness::Stale;
    assert!(matches!(
        workflow.submit(&preview, MutationTargetEvidence::Delivery(&stale), 11),
        Err(ReviewWorkflowError::CurrentTargetMismatch)
    ));
    workflow
        .submit(&preview, MutationTargetEvidence::Delivery(&delivery), 11)
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
            MutationTargetEvidence::ProviderSession(&advanced),
            11,
        ),
        Err(ReviewWorkflowError::InvalidSelection | ReviewWorkflowError::CurrentTargetMismatch)
    ));
    std::fs::remove_dir_all(root).ok();
}
