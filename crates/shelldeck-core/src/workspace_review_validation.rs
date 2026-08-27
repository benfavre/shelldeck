use super::*;
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
            if hunk
                .lines
                .iter()
                .any(|line| line.text.len() > MAX_COMMENT_BYTES)
            {
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
    if session.freshness != ObservationFreshness::Fresh
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
    if delivery.freshness != ObservationFreshness::Fresh
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
                session_id: target_session,
                ..
            },
        ) => {
            bounded_nonempty(session_id, MAX_ID_BYTES)
                && session_id == target_session
                && !comments.is_empty()
                && comments.len() <= MAX_MUTATION_ITEMS
                && comments
                    .iter()
                    .all(|comment| validate_comment(comment).is_ok())
        }
        (
            ReviewMutationKind::DecideApproval {
                session_id,
                approval_id,
                ..
            },
            MutationTargetFence::ProviderApproval {
                session_id: target_session,
                approval_id: target_approval,
                ..
            },
        ) => {
            bounded_nonempty(session_id, MAX_ID_BYTES)
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
    if !valid {
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
