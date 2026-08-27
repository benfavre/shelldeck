use super::*;

pub(super) fn count_provider_session(
    root: Option<&PaneNode>,
    platform_workspace: &str,
    session_id: &str,
) -> usize {
    match root {
        None => 0,
        Some(PaneNode::Leaf(leaf)) => leaf
            .tabs
            .iter()
            .filter(|tab| {
                matches!(
                    &tab.content,
                    WorkspaceTabContent::ProviderSession(binding)
                        if binding.platform_user_workspace_id == platform_workspace
                            && binding.session_id == session_id
                )
            })
            .count(),
        Some(PaneNode::Split { first, second, .. }) => {
            count_provider_session(Some(first), platform_workspace, session_id)
                + count_provider_session(Some(second), platform_workspace, session_id)
        }
    }
}

fn authority_scope_admits_preview(preview: &ReviewMutationPreview) -> bool {
    match (&preview.kind, &preview.target, &preview.authority_scope) {
        (
            ReviewMutationKind::StageHunks { .. } | ReviewMutationKind::UnstageHunks { .. },
            MutationTargetFence::LocalReview { checkout, .. },
            AuthorityScope::Repository {
                checkout: granted_checkout,
                stage_hunks: true,
            },
        ) => checkout == granted_checkout,
        (
            ReviewMutationKind::SendComments { session_id, .. },
            MutationTargetFence::ReviewAndProviderSession {
                platform_user_workspace_id,
                mapping_reconciliation_revision,
                ..
            },
            AuthorityScope::ProviderSession {
                session_id: granted_session,
                platform_user_workspace_id: granted_platform_workspace,
                mapping_reconciliation_revision: granted_mapping_revision,
                send_comments: true,
                ..
            },
        ) => {
            session_id == granted_session
                && platform_user_workspace_id == granted_platform_workspace
                && mapping_reconciliation_revision == granted_mapping_revision
        }
        (
            ReviewMutationKind::DecideApproval { session_id, .. },
            MutationTargetFence::ProviderApproval {
                platform_user_workspace_id,
                mapping_reconciliation_revision,
                ..
            },
            AuthorityScope::ProviderSession {
                session_id: granted_session,
                platform_user_workspace_id: granted_platform_workspace,
                mapping_reconciliation_revision: granted_mapping_revision,
                decide_approval: true,
                ..
            },
        ) => {
            session_id == granted_session
                && platform_user_workspace_id == granted_platform_workspace
                && mapping_reconciliation_revision == granted_mapping_revision
        }
        (
            ReviewMutationKind::RetryCheck {
                provider,
                repository,
                ..
            },
            MutationTargetFence::DeliveryCheck { .. },
            AuthorityScope::Delivery {
                provider: granted_provider,
                repository: granted_repository,
                retry_checks: true,
                ..
            },
        ) => provider == granted_provider && repository == granted_repository,
        (
            ReviewMutationKind::MergePullRequest {
                provider,
                repository,
                ..
            },
            MutationTargetFence::DeliveryPullRequest { .. },
            AuthorityScope::Delivery {
                provider: granted_provider,
                repository: granted_repository,
                merge_pull_request: true,
                ..
            },
        ) => provider == granted_provider && repository == granted_repository,
        _ => false,
    }
}

pub(super) fn validate_fresh_review(snapshot: &ReviewSnapshot) -> Result<(), ReviewWorkflowError> {
    if snapshot.freshness != ObservationFreshness::Fresh {
        return Err(ReviewWorkflowError::StaleReview);
    }
    let mut total_hunks = 0_usize;
    let mut total_lines = 0_usize;
    let mut hunk_ids = BTreeSet::new();
    if snapshot.changes.len() > MAX_REVIEW_CHANGES {
        return Err(ReviewWorkflowError::BoundsExceeded(
            "review projection exceeds its bound",
        ));
    }
    for change in &snapshot.changes {
        if change.path.as_str().len() > MAX_PATH_BYTES || change.hunks.len() > MAX_HUNKS_PER_FILE {
            return Err(ReviewWorkflowError::BoundsExceeded(
                "review projection exceeds its bound",
            ));
        }
        total_hunks = total_hunks.saturating_add(change.hunks.len());
        for hunk in &change.hunks {
            if hunk.id.as_uuid().is_nil()
                || !hunk_ids.insert(hunk.id)
                || !bounded_nonempty(&hunk.header, MAX_TITLE_BYTES)
                || hunk.lines.len() > MAX_LINES_PER_HUNK
            {
                return Err(ReviewWorkflowError::BoundsExceeded(
                    "review projection exceeds its bound",
                ));
            }
            total_lines = total_lines.saturating_add(hunk.lines.len());
            let mut old_lines = BTreeSet::new();
            let mut new_lines = BTreeSet::new();
            if hunk.lines.iter().any(|line| {
                line.text.len() > MAX_COMMENT_BYTES
                    || line
                        .old_line
                        .is_some_and(|line| line == 0 || !old_lines.insert(line))
                    || line
                        .new_line
                        .is_some_and(|line| line == 0 || !new_lines.insert(line))
            }) {
                return Err(ReviewWorkflowError::BoundsExceeded(
                    "review projection exceeds its bound",
                ));
            }
        }
    }
    if total_hunks > MAX_REVIEW_CHANGES || total_lines > MAX_LINES_PER_HUNK {
        return Err(ReviewWorkflowError::BoundsExceeded(
            "review projection exceeds its bound",
        ));
    }
    Ok(())
}

pub(super) fn validate_provider_evidence(
    session: &ProviderSessionProjection,
    grant: &AuthorityGrant,
    sending_comments: bool,
    approval: Option<&String>,
) -> Result<(), ReviewWorkflowError> {
    let AuthorityScope::ProviderSession {
        platform_user_workspace_id,
        mapping_reconciliation_revision,
        ..
    } = &grant.scope
    else {
        return Err(ReviewWorkflowError::WrongAuthority);
    };
    if session.workspace != grant.workspace
        || session.platform_user_workspace_id != *platform_user_workspace_id
        || session.mapping_reconciliation_revision != *mapping_reconciliation_revision
        || !bounded_nonempty(&session.platform_user_workspace_id, MAX_ID_BYTES)
        || session.mapping_reconciliation_revision == 0
        || session.freshness != ObservationFreshness::Fresh
        || session.authority_revision != grant.revision
        || session.observed_actor.as_ref() != Some(&grant.actor)
        || !bounded_nonempty(&session.session_id, MAX_ID_BYTES)
        || session.pending_approval_ids.len() > MAX_MUTATION_ITEMS
        || session
            .pending_approval_ids
            .iter()
            .any(|id| !bounded_nonempty(id, MAX_ID_BYTES))
    {
        return Err(ReviewWorkflowError::CurrentTargetMismatch);
    }
    if sending_comments && !session.can_send_comments {
        return Err(ReviewWorkflowError::WrongAuthority);
    }
    if let Some(approval) = approval {
        if !session.can_decide_approval
            || !bounded_nonempty(approval, MAX_ID_BYTES)
            || !session.pending_approval_ids.contains(approval)
        {
            return Err(ReviewWorkflowError::InvalidSelection);
        }
    }
    Ok(())
}

pub(super) fn validate_delivery_evidence(
    delivery: &DeliveryProjection,
    grant: &AuthorityGrant,
) -> Result<(), ReviewWorkflowError> {
    if delivery.workspace != grant.workspace
        || delivery.freshness != ObservationFreshness::Fresh
        || delivery.authority_revision != grant.revision
        || delivery.authority.observed_actor.as_ref() != Some(&grant.actor)
        || !bounded_nonempty(&delivery.authority.provider, MAX_ID_BYTES)
        || !bounded_nonempty(&delivery.authority.repository, MAX_PATH_BYTES)
        || delivery.checks.len() > MAX_MUTATION_ITEMS
        || delivery.checks.iter().any(|check| {
            !bounded_nonempty(&check.name, MAX_TITLE_BYTES)
                || !bounded_nonempty(&check.id, MAX_ID_BYTES)
        })
        || delivery.pull_request.as_ref().is_some_and(|pull| {
            !bounded_nonempty(&pull.key, MAX_ID_BYTES)
                || !bounded_nonempty(&pull.review_status, MAX_TITLE_BYTES)
        })
    {
        return Err(ReviewWorkflowError::CurrentTargetMismatch);
    }
    let mut ids = BTreeSet::new();
    if delivery
        .checks
        .iter()
        .any(|check| !bounded_nonempty(&check.id, MAX_ID_BYTES) || !ids.insert(&check.id))
    {
        return Err(ReviewWorkflowError::InvalidSelection);
    }
    Ok(())
}

pub(super) fn validate_preview_bounds(
    preview: &ReviewMutationPreview,
) -> Result<(), ReviewWorkflowError> {
    if preview.operation.as_uuid().is_nil()
        || preview.idempotency_key.is_nil()
        || !bounded_nonempty(&preview.actor.id, MAX_ID_BYTES)
        || !bounded_nonempty(&preview.actor.display_name, MAX_TITLE_BYTES)
    {
        return Err(ReviewWorkflowError::BoundsExceeded(
            "mutation identity exceeds its bound",
        ));
    }
    let valid = match (&preview.kind, &preview.target) {
        (
            ReviewMutationKind::StageHunks { hunks } | ReviewMutationKind::UnstageHunks { hunks },
            MutationTargetFence::LocalReview { .. },
        ) => !hunks.is_empty() && hunks.len() <= MAX_MUTATION_ITEMS,
        (
            ReviewMutationKind::SendComments {
                session_id,
                comments,
            },
            MutationTargetFence::ReviewAndProviderSession {
                platform_user_workspace_id,
                mapping_reconciliation_revision,
                session_id: target_session,
                ..
            },
        ) => {
            let mut ids = BTreeSet::new();
            bounded_nonempty(session_id, MAX_ID_BYTES)
                && bounded_nonempty(platform_user_workspace_id, MAX_ID_BYTES)
                && *mapping_reconciliation_revision != 0
                && session_id == target_session
                && !comments.is_empty()
                && comments.len() <= MAX_MUTATION_ITEMS
                && comments
                    .iter()
                    .all(|comment| ids.insert(comment.id) && validate_comment(comment).is_ok())
        }
        (
            ReviewMutationKind::DecideApproval {
                session_id,
                approval_id,
                ..
            },
            MutationTargetFence::ProviderApproval {
                platform_user_workspace_id,
                mapping_reconciliation_revision,
                session_id: target_session,
                approval_id: target_approval,
                ..
            },
        ) => {
            bounded_nonempty(session_id, MAX_ID_BYTES)
                && bounded_nonempty(platform_user_workspace_id, MAX_ID_BYTES)
                && *mapping_reconciliation_revision != 0
                && bounded_nonempty(approval_id, MAX_ID_BYTES)
                && session_id == target_session
                && approval_id == target_approval
        }
        (
            ReviewMutationKind::RetryCheck {
                provider,
                repository,
                check_id,
            },
            MutationTargetFence::DeliveryCheck {
                provider: target_provider,
                repository: target_repository,
                check_id: target_check,
                ..
            },
        ) => {
            bounded_nonempty(provider, MAX_ID_BYTES)
                && bounded_nonempty(repository, MAX_PATH_BYTES)
                && bounded_nonempty(check_id, MAX_ID_BYTES)
                && provider == target_provider
                && repository == target_repository
                && check_id == target_check
        }
        (
            ReviewMutationKind::MergePullRequest {
                provider,
                repository,
                pull_request,
            },
            MutationTargetFence::DeliveryPullRequest {
                provider: target_provider,
                repository: target_repository,
                pull_request: target_pull,
                ..
            },
        ) => {
            bounded_nonempty(provider, MAX_ID_BYTES)
                && bounded_nonempty(repository, MAX_PATH_BYTES)
                && bounded_nonempty(pull_request, MAX_ID_BYTES)
                && provider == target_provider
                && repository == target_repository
                && pull_request == target_pull
        }
        _ => false,
    };
    if !valid || !authority_scope_admits_preview(preview) {
        return Err(ReviewWorkflowError::BoundsExceeded(
            "mutation kind or target exceeds its bound",
        ));
    }
    Ok(())
}

pub(super) fn validate_pending_record(
    pending: &PendingMutation,
) -> Result<(), ReviewWorkflowError> {
    validate_preview_bounds(&pending.preview)?;
    match &pending.state {
        PendingMutationState::Reconciling { category }
        | PendingMutationState::Refused { category } => {
            if !bounded_nonempty(category, MAX_TITLE_BYTES) {
                return Err(ReviewWorkflowError::BoundsExceeded(
                    "mutation state category exceeds its bound",
                ));
            }
        }
        PendingMutationState::Completed(receipt) => validate_receipt(&pending.preview, receipt)?,
        PendingMutationState::Prepared | PendingMutationState::Submitting => {}
    }
    Ok(())
}
