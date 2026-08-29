//! Typed presentation and mutation previews for the shared Automonique Platform v2 review contract.
//!
//! This module does not replace [`crate::workspace_review`], which remains the
//! persisted local workflow. Platform review values are authenticated remote
//! observations. Mutation previews retain their exact snapshot coordinates;
//! only the server can resolve them against separately configured authority.

pub use automonique_platform_client::platform_v2_client::ReviewActionConfirmation;
use automonique_protocol::platform::IdempotencyKey;
use automonique_protocol::platform_v2::{ProjectId, WorkContextIdentity, WorkContextTargetKind};
pub use automonique_protocol::platform_v2_review::{
    AttentionOriginKind, AttentionReason, AttentionState, CheckState, CommentAgentState,
    ConflictResolution, ConflictState, DeliveryState, DiffChangeKind, DiffSide, MergeReadiness,
    PreviewKind, PullRequestId, PullRequestState, ReviewAction, ReviewActionReceipt, ReviewAnchor,
    ReviewAuthority, ReviewAuthorityKind, ReviewCheckId, ReviewCommentId, ReviewCommentTarget,
    ReviewDecision, ReviewField, ReviewFile, ReviewFreshness, ReviewFreshnessState,
    ReviewProposalId, ReviewProposalKind, ReviewReceiptOutcome, ReviewReconciliation,
    ReviewSchemaVersion, ReviewSnapshot, ReviewText, WorktreeFileState,
};
pub use automonique_protocol::platform_v2_transport::{
    ReviewAgentDeliveryCapability, ReviewCapabilities, ReviewCheckRerunCapability,
    ReviewConfirmationDigest, ReviewPullRequestCapabilities, ReviewReceiptCorrelationDigest,
};
use automonique_protocol::primitives::Revision;
use std::collections::BTreeSet;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use super::workspace_catalog::PlatformV2Mapping;

mod custody;
mod notes;
pub use custody::{
    PlatformReviewCustodyStore, ReviewCustodyError, ReviewCustodyPresentation,
    ReviewCustodyRecovery,
};
pub use notes::{PlatformReviewNote, PlatformReviewNoteStore, ReviewNoteError};

mod worktree;
pub use worktree::{
    advertised_staging_capability, review_safe_preview, review_safe_text, review_staging_control,
    ReviewPreviewWithheld, ReviewSafeHtml, ReviewSafeImage, ReviewSafePreview, ReviewSafeText,
    ReviewStagingProposal, ReviewStagingWithheld, ReviewWorktreeFile, ReviewWorktreeHunk,
    ReviewWorktreeLane, ReviewWorktreeLaneGroup, ReviewWorktreeProjection, MAX_SAFE_PREVIEW_BYTES,
    MAX_SAFE_PREVIEW_EDGE, MAX_SAFE_PREVIEW_LINES, MAX_SAFE_PREVIEW_LINE_CHARS,
    MAX_SAFE_PREVIEW_PIXELS, SAFE_PREVIEW_BOX_EDGE,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlatformReviewTarget {
    pub project: ProjectId,
    pub workspace: WorkContextIdentity,
}

impl PlatformReviewTarget {
    /// Build a read target only from the catalog's complete exact reconciliation.
    pub fn from_exact_mapping(mapping: &PlatformV2Mapping) -> Result<Self, &'static str> {
        if !mapping.is_exact() {
            return Err("platform mapping is not exactly reconciled");
        }
        let project = ProjectId::new(mapping.project.id.clone())
            .map_err(|_| "platform project identity is invalid")?;
        let workspace = WorkContextIdentity::parse_local(
            WorkContextTargetKind::UserWorkspace,
            &mapping.user_workspace.id,
        )
        .map_err(|_| "platform workspace identity is invalid")?;
        Ok(Self { project, workspace })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PlatformReviewConfirmationCoordinates {
    Comment {
        comment_id: String,
        revision: Revision,
    },
    CommentBatch {
        comments: Vec<(String, Revision)>,
    },
    Proposal {
        kind: ReviewProposalKind,
        proposal_id: String,
    },
    ConflictResolution {
        proposal_id: String,
        file_id: String,
        resolution: ConflictResolution,
    },
    Check {
        check_id: String,
        revision: Revision,
    },
    PullRequestOpen {
        revision: Revision,
        title: String,
    },
    PullRequestUpdate {
        pull_request_id: String,
        revision: Revision,
        title: String,
    },
    PullRequestMerge {
        pull_request_id: String,
        revision: Revision,
        head_revision: String,
    },
}

/// One native review mutation prepared against an exact workspace snapshot.
///
/// Construction is restricted to actions represented by an exact fresh
/// snapshot. The preview never manufactures authority: the host resolves the
/// action against its independently configured review, Git, CI, or PR adapter.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlatformReviewActionPreview {
    target: PlatformReviewTarget,
    expected_revision: Revision,
    action: ReviewAction,
    idempotency_key: IdempotencyKey,
    confirmation: Option<ReviewActionConfirmation>,
}

impl PlatformReviewActionPreview {
    pub fn add_comment(
        target: PlatformReviewTarget,
        review: &PlatformReviewSemantic,
        anchor: &ReviewAnchorSemantic,
        body: &str,
    ) -> Result<Self, &'static str> {
        validate_review_target(&target, review)?;
        validate_anchor(review, anchor)?;
        let nonce = review_nonce();
        let action = ReviewAction::AddComment {
            comment_id: ReviewCommentId::new(format!("shelldeck-comment-{nonce}"))
                .map_err(|_| "review comment identity is invalid")?,
            anchor: ReviewAnchor::new(
                automonique_protocol::platform_v2_review::ReviewFileId::new(anchor.file_id.clone())
                    .map_err(|_| "review file identity is invalid")?,
                automonique_protocol::platform_v2_review::ReviewHunkId::new(anchor.hunk_id.clone())
                    .map_err(|_| "review hunk identity is invalid")?,
                anchor.side,
                anchor.line,
            )
            .map_err(|_| "review anchor is invalid")?,
            body: ReviewText::new(body.trim().to_owned())
                .map_err(|_| "review comment is invalid")?,
        };
        Self::review_action(target, review.revision, action, &nonce, None)
    }

    pub fn approve(
        target: PlatformReviewTarget,
        review: &PlatformReviewSemantic,
    ) -> Result<Self, &'static str> {
        validate_review_target(&target, review)?;
        if !review.approval_is_actionable() {
            return Err("review status is not fresh");
        }
        let nonce = review_nonce();
        Self::review_action(
            target,
            review.revision,
            ReviewAction::ApproveReview {
                expected_review_revision: review.review.freshness.observed_revision,
            },
            &nonce,
            None,
        )
    }

    pub fn batch_send_comments(
        target: PlatformReviewTarget,
        review: &PlatformReviewSemantic,
        comment_ids: &[String],
    ) -> Result<Self, &'static str> {
        validate_review_target(&target, review)?;
        if review.review.freshness.state != ReviewFreshnessState::Fresh || comment_ids.is_empty() {
            return Err("review comments are not actionable");
        }
        let selected = comment_ids.iter().collect::<BTreeSet<_>>();
        if selected.len() != comment_ids.len() {
            return Err("review comment selection is duplicated");
        }
        let comments = comment_ids
            .iter()
            .map(|id| {
                let comment = review
                    .comments
                    .iter()
                    .find(|comment| &comment.id == id)
                    .filter(|comment| review.comment_is_batch_actionable(&comment.id))
                    .ok_or("review comment is not actionable")?;
                Ok(ReviewCommentTarget::new(
                    ReviewCommentId::new(comment.id.clone())
                        .map_err(|_| "review comment identity is invalid")?,
                    comment.revision,
                ))
            })
            .collect::<Result<Vec<_>, &'static str>>()?;
        let nonce = review_nonce();
        Self::review_action(
            target,
            review.revision,
            ReviewAction::BatchSendCommentsToAgent { comments },
            &nonce,
            None,
        )
    }

    pub fn apply_proposal(
        target: PlatformReviewTarget,
        review: &PlatformReviewSemantic,
        proposal_id: &str,
    ) -> Result<Self, &'static str> {
        validate_review_target(&target, review)?;
        let proposal = review
            .proposals
            .iter()
            .find(|proposal| proposal.id == proposal_id)
            .filter(|proposal| review.proposal_is_actionable(&proposal.id))
            .ok_or("review proposal is not actionable")?;
        let proposal_id = ReviewProposalId::new(proposal.id.clone())
            .map_err(|_| "review proposal identity is invalid")?;
        let action = match proposal.kind {
            ReviewProposalKind::Stage => ReviewAction::Stage { proposal_id },
            ReviewProposalKind::Unstage => ReviewAction::Unstage { proposal_id },
            ReviewProposalKind::Commit => ReviewAction::Commit { proposal_id },
            ReviewProposalKind::ResolveConflict => {
                return Err("conflict resolution requires an explicit resolution")
            }
        };
        let nonce = review_nonce();
        Self::review_action(target, review.revision, action, &nonce, None)
    }

    pub fn rerun_check(
        target: PlatformReviewTarget,
        review: &PlatformReviewSemantic,
        capabilities: &ReviewCapabilities,
        check_id: &str,
    ) -> Result<Self, &'static str> {
        validate_review_target(&target, review)?;
        let check = review
            .checks
            .iter()
            .find(|check| check.id == check_id)
            .filter(|check| review.check_is_rerunnable(&check.id))
            .ok_or("review check is not actionable")?;
        if capabilities.project() != &target.project
            || capabilities.workspace() != &target.workspace
            || capabilities.snapshot_revision() != review.revision
        {
            return Err("review capabilities are stale or belong to another workspace");
        }
        let capability = capabilities
            .rerunnable_checks()
            .iter()
            .find(|capability| capability.check_id().as_str() == check.id)
            .filter(|capability| {
                capability.expected_check_revision() == check.freshness.observed_revision
                    && capability.authority().kind() == check.authority.kind
                    && capability.authority().id().as_str() == check.authority.id
            })
            .ok_or("review check capability is unavailable or stale")?;
        let confirmation = ReviewActionConfirmation::new(
            capability.confirmation_digest().clone(),
            capabilities.workspace_revision(),
            capability.receipt_correlation_digest().clone(),
        );
        let nonce = review_nonce();
        Self::review_action(
            target,
            review.revision,
            ReviewAction::RerunCheck {
                check_id: ReviewCheckId::new(check.id.clone())
                    .map_err(|_| "review check identity is invalid")?,
                expected_check_revision: check.freshness.observed_revision,
            },
            &nonce,
            Some(confirmation),
        )
    }

    pub fn merge_pull_request(
        target: PlatformReviewTarget,
        review: &PlatformReviewSemantic,
    ) -> Result<Self, &'static str> {
        validate_review_target(&target, review)?;
        let pull = &review.pull_request;
        if !review.pull_request_is_mergeable() {
            return Err("pull request is not mergeable");
        }
        let id = pull
            .id
            .as_ref()
            .ok_or("pull request identity is unavailable")?;
        let head = pull
            .head_revision
            .as_ref()
            .ok_or("pull request head revision is unavailable")?;
        let nonce = review_nonce();
        Self::review_action(
            target,
            review.revision,
            ReviewAction::MergePullRequest {
                pull_request_id: PullRequestId::new(id.clone())
                    .map_err(|_| "pull request identity is invalid")?,
                expected_pull_request_revision: pull.freshness.observed_revision,
                expected_head_revision: ReviewField::new(head.clone())
                    .map_err(|_| "pull request head revision is invalid")?,
            },
            &nonce,
            None,
        )
    }

    fn review_action(
        target: PlatformReviewTarget,
        expected_revision: Revision,
        action: ReviewAction,
        nonce: &str,
        confirmation: Option<ReviewActionConfirmation>,
    ) -> Result<Self, &'static str> {
        action
            .validate_client_shape()
            .map_err(|_| "review action is invalid")?;
        if matches!(action, ReviewAction::RerunCheck { .. }) != confirmation.is_some() {
            return Err("review action confirmation does not match its action");
        }
        let idempotency_key = IdempotencyKey::new(format!("shelldeck-review-{nonce}"))
            .map_err(|_| "review idempotency key is invalid")?;
        Ok(Self {
            target,
            expected_revision,
            action,
            idempotency_key,
            confirmation,
        })
    }

    #[must_use]
    pub const fn target(&self) -> &PlatformReviewTarget {
        &self.target
    }

    #[must_use]
    pub const fn expected_revision(&self) -> Revision {
        self.expected_revision
    }

    #[must_use]
    pub const fn action(&self) -> &ReviewAction {
        &self.action
    }

    #[must_use]
    pub const fn idempotency_key(&self) -> &IdempotencyKey {
        &self.idempotency_key
    }

    #[must_use]
    pub const fn confirmation(&self) -> Option<&ReviewActionConfirmation> {
        self.confirmation.as_ref()
    }

    /// Revalidate a confirmed rerun against the latest authoritative
    /// capability generation before crossing the durable dispatch fence.
    #[must_use]
    pub fn matches_capabilities(&self, capabilities: &ReviewCapabilities) -> bool {
        let (
            ReviewAction::RerunCheck {
                check_id,
                expected_check_revision,
            },
            Some(confirmation),
        ) = (&self.action, &self.confirmation)
        else {
            return self.confirmation.is_none();
        };
        capabilities.project() == &self.target.project
            && capabilities.workspace() == &self.target.workspace
            && capabilities.snapshot_revision() == self.expected_revision
            && capabilities.workspace_revision() == confirmation.expected_workspace_revision()
            && capabilities.rerunnable_checks().iter().any(|capability| {
                capability.check_id() == check_id
                    && capability.expected_check_revision() == *expected_check_revision
                    && capability.confirmation_digest() == confirmation.confirmation_digest()
                    && capability.receipt_correlation_digest()
                        == confirmation.receipt_correlation_digest()
            })
    }

    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self.action {
            ReviewAction::AddComment { .. } => "add_comment",
            ReviewAction::SendCommentToAgent { .. } => "send_comment_to_agent",
            ReviewAction::BatchSendCommentsToAgent { .. } => "batch_send_comments_to_agent",
            ReviewAction::Stage { .. } => "stage",
            ReviewAction::Unstage { .. } => "unstage",
            ReviewAction::Commit { .. } => "commit",
            ReviewAction::ResolveConflict { .. } => "resolve_conflict",
            ReviewAction::ApproveReview { .. } => "approve_review",
            ReviewAction::RerunCheck { .. } => "rerun_check",
            ReviewAction::OpenPullRequest { .. } => "open_pull_request",
            ReviewAction::UpdatePullRequest { .. } => "update_pull_request",
            ReviewAction::MergePullRequest { .. } => "merge_pull_request",
        }
    }

    /// Exact action coordinates rendered in the confirmation surface.
    /// Local comment and approval actions use their richer localized forms.
    #[must_use]
    pub fn confirmation_coordinates(&self) -> Option<PlatformReviewConfirmationCoordinates> {
        match &self.action {
            ReviewAction::AddComment { .. } | ReviewAction::ApproveReview { .. } => None,
            ReviewAction::SendCommentToAgent {
                comment_id,
                expected_comment_revision,
            } => Some(PlatformReviewConfirmationCoordinates::Comment {
                comment_id: comment_id.as_str().to_owned(),
                revision: *expected_comment_revision,
            }),
            ReviewAction::BatchSendCommentsToAgent { comments } => {
                Some(PlatformReviewConfirmationCoordinates::CommentBatch {
                    comments: comments
                        .iter()
                        .map(|comment| {
                            (
                                comment.comment_id().as_str().to_owned(),
                                comment.expected_revision(),
                            )
                        })
                        .collect(),
                })
            }
            ReviewAction::Stage { proposal_id }
            | ReviewAction::Unstage { proposal_id }
            | ReviewAction::Commit { proposal_id } => {
                Some(PlatformReviewConfirmationCoordinates::Proposal {
                    kind: match &self.action {
                        ReviewAction::Stage { .. } => ReviewProposalKind::Stage,
                        ReviewAction::Unstage { .. } => ReviewProposalKind::Unstage,
                        ReviewAction::Commit { .. } => ReviewProposalKind::Commit,
                        _ => unreachable!(),
                    },
                    proposal_id: proposal_id.as_str().to_owned(),
                })
            }
            ReviewAction::ResolveConflict {
                proposal_id,
                file_id,
                resolution,
            } => Some(PlatformReviewConfirmationCoordinates::ConflictResolution {
                proposal_id: proposal_id.as_str().to_owned(),
                file_id: file_id.as_str().to_owned(),
                resolution: *resolution,
            }),
            ReviewAction::RerunCheck {
                check_id,
                expected_check_revision,
            } => Some(PlatformReviewConfirmationCoordinates::Check {
                check_id: check_id.as_str().to_owned(),
                revision: *expected_check_revision,
            }),
            ReviewAction::OpenPullRequest {
                expected_pull_request_revision,
                title,
            } => Some(PlatformReviewConfirmationCoordinates::PullRequestOpen {
                revision: *expected_pull_request_revision,
                title: title.as_str().to_owned(),
            }),
            ReviewAction::UpdatePullRequest {
                pull_request_id,
                expected_pull_request_revision,
                title,
            } => Some(PlatformReviewConfirmationCoordinates::PullRequestUpdate {
                pull_request_id: pull_request_id.as_str().to_owned(),
                revision: *expected_pull_request_revision,
                title: title.as_str().to_owned(),
            }),
            ReviewAction::MergePullRequest {
                pull_request_id,
                expected_pull_request_revision,
                expected_head_revision,
            } => Some(PlatformReviewConfirmationCoordinates::PullRequestMerge {
                pull_request_id: pull_request_id.as_str().to_owned(),
                revision: *expected_pull_request_revision,
                head_revision: expected_head_revision.as_str().to_owned(),
            }),
        }
    }
}

fn review_nonce() -> String {
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed);
    format!("{nanos}-{sequence}")
}

fn validate_review_target(
    target: &PlatformReviewTarget,
    review: &PlatformReviewSemantic,
) -> Result<(), &'static str> {
    if review.workspace_kind != target.workspace.kind()
        || review.workspace_id != target.workspace.id()
    {
        return Err("review snapshot belongs to another workspace");
    }
    Ok(())
}

fn validate_anchor(
    review: &PlatformReviewSemantic,
    anchor: &ReviewAnchorSemantic,
) -> Result<(), &'static str> {
    let file = review
        .files
        .iter()
        .find(|file| file.id == anchor.file_id)
        .ok_or("review file is not in the exact snapshot")?;
    let hunk = file
        .hunks
        .iter()
        .find(|hunk| hunk.id == anchor.hunk_id)
        .ok_or("review hunk is not in the exact snapshot")?;
    let (start, lines) = match anchor.side {
        DiffSide::Old => (hunk.old_start, hunk.old_lines),
        DiffSide::New => (hunk.new_start, hunk.new_lines),
    };
    if lines == 0 || anchor.line < start || anchor.line >= start.saturating_add(lines) {
        return Err("review line is not in the exact hunk");
    }
    Ok(())
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PlatformReviewLoad {
    Available(Box<PlatformReviewSemantic>),
    Unavailable(PlatformReviewUnavailable),
}

/// Authenticated server-advertised review mutation capabilities for one exact
/// project/workspace snapshot. An unavailable capability load grants nothing.
///
/// The available payload is boxed because `ReviewCapabilities` now carries the
/// agent-delivery list and three pull-request slots, which makes it far larger
/// than a refusal's two strings.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PlatformReviewCapabilitiesLoad {
    Available(Box<ReviewCapabilities>),
    Unavailable(PlatformReviewUnavailable),
}

impl PlatformReviewCapabilitiesLoad {
    #[must_use]
    pub fn available(&self) -> Option<&ReviewCapabilities> {
        match self {
            Self::Available(capabilities) => Some(capabilities),
            Self::Unavailable(_) => None,
        }
    }
}

impl PlatformReviewLoad {
    #[must_use]
    pub fn needs_user_action(&self) -> bool {
        matches!(self, Self::Available(review) if review.attention.needs_user_action)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlatformReviewUnavailable {
    pub category: String,
    pub explanation: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlatformReviewSemantic {
    pub schema: ReviewSchemaVersion,
    pub workspace_kind: WorkContextTargetKind,
    pub workspace_id: String,
    pub revision: Revision,
    pub attention: ReviewAttentionSemantic,
    pub attention_events: Vec<ReviewAttentionEventSemantic>,
    pub review: ReviewStatusSemantic,
    pub checks: Vec<ReviewCheckSemantic>,
    pub pull_request: PullRequestSemantic,
    pub delivery: DeliverySemantic,
    pub files: Vec<ReviewFileSemantic>,
    pub comments: Vec<ReviewCommentSemantic>,
    pub proposals: Vec<ReviewProposalSemantic>,
}

impl PlatformReviewSemantic {
    #[must_use]
    pub fn approval_is_actionable(&self) -> bool {
        self.review.authority.kind == ReviewAuthorityKind::Review
            && self.review.freshness.state == ReviewFreshnessState::Fresh
            && matches!(
                self.review.decision,
                ReviewDecision::Pending | ReviewDecision::ChangesRequested
            )
    }

    #[must_use]
    pub fn comment_is_batch_actionable(&self, comment_id: &str) -> bool {
        self.review.authority.kind == ReviewAuthorityKind::Review
            && self.review.freshness.state == ReviewFreshnessState::Fresh
            && self.comments.iter().any(|comment| {
                comment.id == comment_id
                    && matches!(
                        comment.agent_state,
                        CommentAgentState::NotSent | CommentAgentState::Refused
                    )
            })
    }

    #[must_use]
    pub fn proposal_is_actionable(&self, proposal_id: &str) -> bool {
        self.proposals
            .iter()
            .find(|proposal| proposal.id == proposal_id)
            .is_some_and(|proposal| {
                matches!(
                    proposal.kind,
                    ReviewProposalKind::Stage
                        | ReviewProposalKind::Unstage
                        | ReviewProposalKind::Commit
                ) && proposal
                    .authority
                    .as_ref()
                    .is_some_and(|authority| authority.kind == ReviewAuthorityKind::Git)
                    && !(matches!(
                        proposal.kind,
                        ReviewProposalKind::Stage | ReviewProposalKind::Commit
                    ) && proposal.files.iter().any(|id| {
                        self.files.iter().any(|file| {
                            file.id == *id && file.conflict == ConflictState::Unresolved
                        })
                    }))
            })
    }

    #[must_use]
    pub fn check_is_rerunnable(&self, check_id: &str) -> bool {
        self.checks.iter().any(|check| {
            check.id == check_id
                && check.authority.kind == ReviewAuthorityKind::Ci
                && check.freshness.state == ReviewFreshnessState::Fresh
                && matches!(
                    check.state,
                    CheckState::Passed | CheckState::Failed | CheckState::Cancelled
                )
        })
    }

    #[must_use]
    pub fn pull_request_is_mergeable(&self) -> bool {
        self.pull_request.authority.kind == ReviewAuthorityKind::PullRequest
            && self.pull_request.freshness.state == ReviewFreshnessState::Fresh
            && self.pull_request.state == PullRequestState::Open
            && self.pull_request.readiness == MergeReadiness::Ready
            && self.pull_request.id.is_some()
            && self.pull_request.head_revision.is_some()
    }
}

impl From<&ReviewSnapshot> for PlatformReviewSemantic {
    fn from(snapshot: &ReviewSnapshot) -> Self {
        let attention = snapshot.attention();
        Self {
            schema: snapshot.schema(),
            workspace_kind: snapshot.workspace().kind(),
            workspace_id: snapshot.workspace().id().to_owned(),
            revision: snapshot.revision(),
            attention: ReviewAttentionSemantic {
                state: attention.state(),
                reason: attention.reason(),
                source_revision: attention.source_revision(),
                unread: attention.unread(),
                needs_user_action: attention.state() == AttentionState::NeedsYou
                    && attention.reason().is_some()
                    && attention.source_revision().is_some(),
            },
            attention_events: snapshot
                .attention_events()
                .iter()
                .map(|event| ReviewAttentionEventSemantic {
                    id: event.id().as_str().to_owned(),
                    origin_kind: event.origin().kind(),
                    origin_id: event.origin().id().map(|id| id.as_str().to_owned()),
                    authority: authority(event.origin().authority()),
                    source_revision: event.source_revision(),
                    reason: event.reason(),
                    unread: event.unread(),
                })
                .collect(),
            review: ReviewStatusSemantic {
                decision: snapshot.review().decision(),
                authority: authority(snapshot.review().authority()),
                freshness: freshness(snapshot.review().freshness()),
            },
            checks: snapshot
                .checks()
                .iter()
                .map(|check| ReviewCheckSemantic {
                    id: check.id().as_str().to_owned(),
                    state: check.state(),
                    required: check.required(),
                    authority: authority(check.authority()),
                    freshness: freshness(check.freshness()),
                })
                .collect(),
            pull_request: PullRequestSemantic {
                id: snapshot
                    .pull_request()
                    .id()
                    .map(|id| id.as_str().to_owned()),
                state: snapshot.pull_request().state(),
                readiness: snapshot.pull_request().readiness(),
                head_revision: snapshot
                    .pull_request()
                    .head_revision()
                    .map(|revision| revision.as_str().to_owned()),
                authority: authority(snapshot.pull_request().authority()),
                freshness: freshness(snapshot.pull_request().freshness()),
            },
            delivery: DeliverySemantic {
                id: snapshot.delivery().id().map(|id| id.as_str().to_owned()),
                state: snapshot.delivery().state(),
                authority: authority(snapshot.delivery().authority()),
                freshness: freshness(snapshot.delivery().freshness()),
            },
            files: snapshot.files().iter().map(file).collect(),
            comments: snapshot
                .comments()
                .iter()
                .map(|comment| ReviewCommentSemantic {
                    id: comment.id().as_str().to_owned(),
                    revision: comment.revision(),
                    actor: comment.actor().as_str().to_owned(),
                    body: comment.body().as_str().to_owned(),
                    anchor: ReviewAnchorSemantic {
                        file_id: comment.anchor().file_id().as_str().to_owned(),
                        hunk_id: comment.anchor().hunk_id().as_str().to_owned(),
                        side: comment.anchor().side(),
                        line: comment.anchor().line(),
                    },
                    agent_state: comment.agent_state(),
                    unread: comment.unread(),
                })
                .collect(),
            proposals: snapshot
                .proposals()
                .iter()
                .map(|proposal| ReviewProposalSemantic {
                    id: proposal.id().as_str().to_owned(),
                    kind: proposal.kind(),
                    authority: proposal.authority().map(authority),
                    files: proposal
                        .files()
                        .iter()
                        .map(|file| file.as_str().to_owned())
                        .collect(),
                    subject: proposal
                        .subject()
                        .map(|subject| subject.as_str().to_owned()),
                })
                .collect(),
        }
    }
}

/// Canonical cross-client keys consumed by the read-only review UI.
///
/// The keys deliberately contain no localized copy and no action capability.
/// They are a lossless presentation projection of the typed protocol enums.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlatformReviewRenderSemantic {
    pub source_revision: Revision,
    pub attention: ReviewAttentionRenderSemantic,
    pub review: ReviewFreshRenderSemantic,
    pub checks: Vec<ReviewIdentifiedFreshRenderSemantic>,
    pub pull_request: ReviewFreshRenderSemantic,
    pub delivery: ReviewFreshRenderSemantic,
    pub previews: Vec<ReviewIdentifiedRenderSemantic>,
}

impl From<&PlatformReviewSemantic> for PlatformReviewRenderSemantic {
    fn from(review: &PlatformReviewSemantic) -> Self {
        Self {
            source_revision: review.revision,
            attention: ReviewAttentionRenderSemantic {
                semantic_key: format!("attention.{}", review.attention.state.as_str()),
                reason_key: review
                    .attention
                    .reason
                    .map(|reason| format!("attention_reason.{}", reason.as_str())),
                source_revision: review.attention.source_revision,
            },
            review: fresh_render(
                format!("review.{}", review.review.decision.as_str()),
                review.review.freshness,
            ),
            checks: review
                .checks
                .iter()
                .map(|check| ReviewIdentifiedFreshRenderSemantic {
                    id: check.id.clone(),
                    semantic: fresh_render(
                        format!(
                            "check.{}.{}",
                            check.state.as_str(),
                            if check.required {
                                "required"
                            } else {
                                "optional"
                            }
                        ),
                        check.freshness,
                    ),
                })
                .collect(),
            pull_request: fresh_render(
                format!(
                    "pull_request.{}.{}",
                    review.pull_request.state.as_str(),
                    review.pull_request.readiness.as_str()
                ),
                review.pull_request.freshness,
            ),
            delivery: fresh_render(
                format!("delivery.{}", review.delivery.state.as_str()),
                review.delivery.freshness,
            ),
            previews: review
                .files
                .iter()
                .map(|file| ReviewIdentifiedRenderSemantic {
                    id: file.id.clone(),
                    semantic_key: format!(
                        "preview.{}.{}",
                        file.preview.kind.as_str(),
                        if file.preview.sanitized {
                            "sanitized"
                        } else {
                            "raw"
                        }
                    ),
                    source_revision: review.revision,
                })
                .collect(),
        }
    }
}

fn fresh_render(
    semantic_key: String,
    freshness: ReviewFreshnessSemantic,
) -> ReviewFreshRenderSemantic {
    ReviewFreshRenderSemantic {
        semantic_key,
        freshness_key: format!("freshness.{}", freshness.state.as_str()),
        source_revision: freshness.observed_revision,
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReviewAttentionRenderSemantic {
    pub semantic_key: String,
    pub reason_key: Option<String>,
    pub source_revision: Option<Revision>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReviewFreshRenderSemantic {
    pub semantic_key: String,
    pub freshness_key: String,
    pub source_revision: Revision,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReviewIdentifiedFreshRenderSemantic {
    pub id: String,
    pub semantic: ReviewFreshRenderSemantic,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReviewIdentifiedRenderSemantic {
    pub id: String,
    pub semantic_key: String,
    pub source_revision: Revision,
}

fn file(file: &ReviewFile) -> ReviewFileSemantic {
    ReviewFileSemantic {
        id: file.id().as_str().to_owned(),
        path: file.path().as_str().to_owned(),
        change: file.change(),
        worktree: file.worktree(),
        conflict: file.conflict(),
        preview: ReviewPreviewSemantic {
            kind: file.preview().kind(),
            media_type: file
                .preview()
                .media_type()
                .map(|value| value.as_str().to_owned()),
            byte_size: file.preview().byte_size(),
            width: file.preview().width(),
            height: file.preview().height(),
            sanitized: file.preview().sanitized(),
        },
        hunks: file
            .hunks()
            .iter()
            .map(|hunk| ReviewHunkSemantic {
                id: hunk.id().as_str().to_owned(),
                old_start: hunk.old_start(),
                old_lines: hunk.old_lines(),
                new_start: hunk.new_start(),
                new_lines: hunk.new_lines(),
                preview: hunk.preview().as_str().to_owned(),
            })
            .collect(),
    }
}

fn authority(value: &ReviewAuthority) -> ReviewAuthoritySemantic {
    ReviewAuthoritySemantic {
        kind: value.kind(),
        id: value.id().as_str().to_owned(),
    }
}

fn freshness(value: ReviewFreshness) -> ReviewFreshnessSemantic {
    ReviewFreshnessSemantic {
        state: value.state(),
        observed_revision: value.observed_revision(),
        observed_at_ms: value.observed_at_ms(),
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReviewAttentionSemantic {
    pub state: AttentionState,
    pub reason: Option<AttentionReason>,
    pub source_revision: Option<Revision>,
    pub unread: u32,
    /// A visual prompt to inspect the review, never mutation authority.
    pub needs_user_action: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReviewAttentionEventSemantic {
    pub id: String,
    pub origin_kind: AttentionOriginKind,
    pub origin_id: Option<String>,
    pub authority: ReviewAuthoritySemantic,
    pub source_revision: Revision,
    pub reason: AttentionReason,
    pub unread: u32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReviewAuthoritySemantic {
    pub kind: ReviewAuthorityKind,
    pub id: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReviewFreshnessSemantic {
    pub state: ReviewFreshnessState,
    pub observed_revision: Revision,
    pub observed_at_ms: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReviewStatusSemantic {
    pub decision: ReviewDecision,
    pub authority: ReviewAuthoritySemantic,
    pub freshness: ReviewFreshnessSemantic,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReviewCheckSemantic {
    pub id: String,
    pub state: CheckState,
    pub required: bool,
    pub authority: ReviewAuthoritySemantic,
    pub freshness: ReviewFreshnessSemantic,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PullRequestSemantic {
    pub id: Option<String>,
    pub state: PullRequestState,
    pub readiness: MergeReadiness,
    pub head_revision: Option<String>,
    pub authority: ReviewAuthoritySemantic,
    pub freshness: ReviewFreshnessSemantic,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeliverySemantic {
    pub id: Option<String>,
    pub state: DeliveryState,
    pub authority: ReviewAuthoritySemantic,
    pub freshness: ReviewFreshnessSemantic,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReviewFileSemantic {
    pub id: String,
    pub path: String,
    pub change: DiffChangeKind,
    pub worktree: WorktreeFileState,
    pub conflict: ConflictState,
    pub preview: ReviewPreviewSemantic,
    pub hunks: Vec<ReviewHunkSemantic>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReviewPreviewSemantic {
    pub kind: PreviewKind,
    pub media_type: Option<String>,
    pub byte_size: Option<u64>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub sanitized: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReviewHunkSemantic {
    pub id: String,
    pub old_start: u32,
    pub old_lines: u32,
    pub new_start: u32,
    pub new_lines: u32,
    pub preview: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReviewCommentSemantic {
    pub id: String,
    pub revision: Revision,
    pub actor: String,
    pub body: String,
    pub anchor: ReviewAnchorSemantic,
    pub agent_state: CommentAgentState,
    pub unread: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReviewAnchorSemantic {
    pub file_id: String,
    pub hunk_id: String,
    pub side: DiffSide,
    pub line: u32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReviewProposalSemantic {
    pub id: String,
    pub kind: ReviewProposalKind,
    pub authority: Option<ReviewAuthoritySemantic>,
    pub files: Vec<String>,
    pub subject: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use automonique_protocol::platform_v2_review_api::decode_review_snapshot;
    use serde::Deserialize;
    use std::collections::BTreeSet;

    const CANONICAL_FIXTURE: &[u8] =
        include_bytes!("../../tests/fixtures/platform-v2-review-v2.json");
    const RENDER_CORPUS: &[u8] =
        include_bytes!("../../tests/fixtures/platform-v2-render-conformance-v1.json");

    fn rerun_capabilities(
        target: &PlatformReviewTarget,
        review: &PlatformReviewSemantic,
    ) -> ReviewCapabilities {
        let check = &review.checks[0];
        ReviewCapabilities::new(
            target.project.clone(),
            target.workspace.clone(),
            review.revision,
            Revision::new(91).unwrap(),
            vec![ReviewCheckRerunCapability::new(
                ReviewCheckId::new(check.id.clone()).unwrap(),
                check.freshness.observed_revision,
                ReviewAuthority::new(
                    check.authority.kind,
                    automonique_protocol::platform_v2_review::ReviewAuthorityId::new(
                        check.authority.id.clone(),
                    )
                    .unwrap(),
                ),
                ReviewConfirmationDigest::new("a".repeat(64)).unwrap(),
                ReviewReceiptCorrelationDigest::new("b".repeat(64)).unwrap(),
            )
            .unwrap()],
            Vec::new(),
            ReviewPullRequestCapabilities::default(),
        )
        .unwrap()
    }

    #[derive(Deserialize)]
    struct RenderCorpus {
        schema: String,
        version: String,
        cases: Vec<RenderCase>,
    }

    #[derive(Deserialize)]
    struct RenderCase {
        id: String,
        input: RenderInput,
        expected: RenderExpected,
    }

    #[derive(Deserialize)]
    struct RenderInput {
        revision: String,
        attention: RenderAttentionInput,
        review: RenderReviewInput,
        checks: Vec<RenderCheckInput>,
        pull_request: RenderPullRequestInput,
        delivery: RenderDeliveryInput,
        files: Vec<RenderFileInput>,
    }

    #[derive(Deserialize)]
    struct RenderAttentionInput {
        state: String,
        reason: Option<String>,
        unread: String,
        source_revision: Option<String>,
    }

    #[derive(Deserialize)]
    struct RenderReviewInput {
        decision: String,
        freshness: RenderFreshnessInput,
    }

    #[derive(Deserialize)]
    struct RenderCheckInput {
        id: String,
        state: String,
        required: bool,
        freshness: RenderFreshnessInput,
    }

    #[derive(Deserialize)]
    struct RenderPullRequestInput {
        state: String,
        readiness: String,
        freshness: RenderFreshnessInput,
    }

    #[derive(Deserialize)]
    struct RenderDeliveryInput {
        state: String,
        freshness: RenderFreshnessInput,
    }

    #[derive(Deserialize)]
    struct RenderFileInput {
        id: String,
        preview: RenderPreviewInput,
    }

    #[derive(Deserialize)]
    struct RenderPreviewInput {
        kind: String,
        sanitized: bool,
    }

    #[derive(Deserialize)]
    struct RenderFreshnessInput {
        state: String,
        observed_revision: String,
    }

    #[derive(Deserialize)]
    struct RenderExpected {
        source_revision: String,
        attention: RenderAttentionExpected,
        review: RenderFreshExpected,
        checks: Vec<RenderIdentifiedFreshExpected>,
        pull_request: RenderFreshExpected,
        delivery: RenderFreshExpected,
        previews: Vec<RenderIdentifiedExpected>,
    }

    #[derive(Deserialize)]
    struct RenderAttentionExpected {
        semantic_key: String,
        reason_key: Option<String>,
        source_revision: Option<String>,
    }

    #[derive(Deserialize)]
    struct RenderFreshExpected {
        semantic_key: String,
        freshness_key: String,
        source_revision: String,
    }

    #[derive(Deserialize)]
    struct RenderIdentifiedFreshExpected {
        id: String,
        #[serde(flatten)]
        semantic: RenderFreshExpected,
    }

    #[derive(Deserialize)]
    struct RenderIdentifiedExpected {
        id: String,
        semantic_key: String,
        source_revision: String,
    }

    fn revision(value: &str) -> Revision {
        Revision::new(value.parse().unwrap()).unwrap()
    }

    fn render_freshness(value: &RenderFreshnessInput) -> ReviewFreshnessSemantic {
        ReviewFreshnessSemantic {
            state: ReviewFreshnessState::parse(&value.state).unwrap(),
            observed_revision: revision(&value.observed_revision),
            observed_at_ms: 0,
        }
    }

    fn render_authority(kind: ReviewAuthorityKind) -> ReviewAuthoritySemantic {
        ReviewAuthoritySemantic {
            kind,
            id: "render-corpus".to_owned(),
        }
    }

    fn semantic_from_render_input(input: &RenderInput) -> PlatformReviewSemantic {
        let state = AttentionState::parse(&input.attention.state).unwrap();
        PlatformReviewSemantic {
            schema: ReviewSchemaVersion::V2,
            workspace_kind: WorkContextTargetKind::UserWorkspace,
            workspace_id: "wc_render_corpus".to_owned(),
            revision: revision(&input.revision),
            attention: ReviewAttentionSemantic {
                state,
                reason: input
                    .attention
                    .reason
                    .as_deref()
                    .map(AttentionReason::parse)
                    .transpose()
                    .unwrap(),
                source_revision: input.attention.source_revision.as_deref().map(revision),
                unread: input.attention.unread.parse().unwrap(),
                needs_user_action: state == AttentionState::NeedsYou,
            },
            attention_events: Vec::new(),
            review: ReviewStatusSemantic {
                decision: ReviewDecision::parse(&input.review.decision).unwrap(),
                authority: render_authority(ReviewAuthorityKind::Review),
                freshness: render_freshness(&input.review.freshness),
            },
            checks: input
                .checks
                .iter()
                .map(|check| ReviewCheckSemantic {
                    id: check.id.clone(),
                    state: CheckState::parse(&check.state).unwrap(),
                    required: check.required,
                    authority: render_authority(ReviewAuthorityKind::Ci),
                    freshness: render_freshness(&check.freshness),
                })
                .collect(),
            pull_request: PullRequestSemantic {
                id: None,
                state: PullRequestState::parse(&input.pull_request.state).unwrap(),
                readiness: MergeReadiness::parse(&input.pull_request.readiness).unwrap(),
                head_revision: None,
                authority: render_authority(ReviewAuthorityKind::PullRequest),
                freshness: render_freshness(&input.pull_request.freshness),
            },
            delivery: DeliverySemantic {
                id: None,
                state: DeliveryState::parse(&input.delivery.state).unwrap(),
                authority: render_authority(ReviewAuthorityKind::Delivery),
                freshness: render_freshness(&input.delivery.freshness),
            },
            files: input
                .files
                .iter()
                .map(|file| ReviewFileSemantic {
                    id: file.id.clone(),
                    path: file.id.clone(),
                    change: DiffChangeKind::Modified,
                    worktree: WorktreeFileState::Unstaged,
                    conflict: ConflictState::None,
                    preview: ReviewPreviewSemantic {
                        kind: PreviewKind::parse(&file.preview.kind).unwrap(),
                        media_type: None,
                        byte_size: None,
                        width: None,
                        height: None,
                        sanitized: file.preview.sanitized,
                    },
                    hunks: Vec::new(),
                })
                .collect(),
            comments: Vec::new(),
            proposals: Vec::new(),
        }
    }

    fn assert_fresh_render(actual: &ReviewFreshRenderSemantic, expected: &RenderFreshExpected) {
        assert_eq!(actual.semantic_key, expected.semantic_key);
        assert_eq!(actual.freshness_key, expected.freshness_key);
        assert_eq!(
            actual.source_revision.get().to_string(),
            expected.source_revision
        );
    }

    // SDTEST-1772
    #[test]
    fn review_target_requires_exact_catalog_mapping() {
        use super::super::workspace_catalog::{PlatformContextRef, PlatformMappingReconciliation};

        let mut mapping = PlatformV2Mapping {
            reconciliation_revision: 1,
            project: PlatformContextRef {
                id: "project-1".to_owned(),
                revision: 2,
            },
            checkout: PlatformContextRef {
                id: "checkout-1".to_owned(),
                revision: 3,
            },
            user_workspace: PlatformContextRef {
                id: "wc_user_1".to_owned(),
                revision: 4,
            },
            reconciliation: PlatformMappingReconciliation::Pending,
        };
        assert!(PlatformReviewTarget::from_exact_mapping(&mapping).is_err());

        mapping.reconciliation = PlatformMappingReconciliation::Exact {
            reconciled_at_millis: 5,
        };
        let target = PlatformReviewTarget::from_exact_mapping(&mapping).unwrap();
        assert_eq!(target.project.as_str(), "project-1");
        assert_eq!(target.workspace.id(), "wc_user_1");
    }

    // SDTEST-1773
    #[test]
    fn canonical_fixture_projects_equivalent_review_meaning() {
        let snapshot = decode_review_snapshot(CANONICAL_FIXTURE).unwrap();
        let semantic = PlatformReviewSemantic::from(&snapshot);

        assert_eq!(semantic.schema, ReviewSchemaVersion::V2);
        assert_eq!(
            semantic.workspace_kind,
            WorkContextTargetKind::UserWorkspace
        );
        assert_eq!(semantic.workspace_id, "wc_user_1");
        assert_eq!(semantic.revision.get(), 9);
        assert_eq!(semantic.attention.state, AttentionState::NeedsYou);
        assert_eq!(
            semantic.attention.reason,
            Some(AttentionReason::ReviewRequested)
        );
        assert_eq!(
            semantic.attention.source_revision.map(Revision::get),
            Some(8)
        );
        assert_eq!(semantic.attention.unread, 1);
        assert!(semantic.attention.needs_user_action);

        assert_eq!(semantic.review.decision, ReviewDecision::Pending);
        assert_eq!(semantic.review.authority.kind, ReviewAuthorityKind::Review);
        assert_eq!(semantic.review.freshness.state, ReviewFreshnessState::Fresh);
        assert_eq!(semantic.checks[0].state, CheckState::Passed);
        assert_eq!(semantic.checks[0].authority.kind, ReviewAuthorityKind::Ci);
        assert_eq!(semantic.pull_request.state, PullRequestState::Open);
        assert_eq!(semantic.pull_request.readiness, MergeReadiness::Ready);
        assert_eq!(
            semantic.pull_request.head_revision.as_deref(),
            Some("0123456789abcdef")
        );
        assert_eq!(semantic.delivery.state, DeliveryState::Pending);

        assert_eq!(semantic.files[0].path, "src/review.rs");
        assert_eq!(semantic.files[0].conflict, ConflictState::None);
        assert_eq!(semantic.files[0].preview.kind, PreviewKind::Text);
        assert!(semantic.files[0].preview.sanitized);
        assert_eq!(semantic.files[0].hunks[0].new_lines, 3);
        assert_eq!(
            semantic.files[0].hunks[0].preview,
            "@@ -10,2 +10,3 @@ · sanitized preview"
        );
        assert_eq!(semantic.comments[0].anchor.file_id, "file-1");
        assert_eq!(semantic.comments[0].anchor.hunk_id, "hunk-1");
        assert!(semantic.comments[0].unread);
        assert_eq!(
            semantic.proposals[0].authority.as_ref().unwrap().kind,
            ReviewAuthorityKind::Git
        );
    }

    // SDTEST-1774
    #[test]
    fn stale_unavailable_projection_is_non_actionable() {
        let text = std::str::from_utf8(CANONICAL_FIXTURE).unwrap();
        let text = text
            .replace(
                "\"attention\":{\"reason\":\"review_requested\",\"source_revision\":8,\"state\":\"needs_you\",\"unread\":1}",
                "\"attention\":{\"reason\":null,\"source_revision\":null,\"state\":\"idle\",\"unread\":0}",
            )
            .replace(
                "\"attention_events\":[{\"id\":\"attention-1\",\"origin\":{\"authority\":{\"id\":\"authority-1\",\"kind\":\"review\"},\"id\":null,\"kind\":\"review\",\"revision\":8},\"reason\":\"review_requested\",\"unread\":1}]",
                "\"attention_events\":[]",
            )
            .replace("\"state\":\"passed\"", "\"state\":\"unavailable\"")
            .replace(
                "\"review\":{\"authority\":{\"id\":\"authority-1\",\"kind\":\"review\"},\"decision\":\"pending\",\"freshness\":{\"observed_at_ms\":1800000000000,\"observed_revision\":8,\"state\":\"fresh\"}}",
                "\"review\":{\"authority\":{\"id\":\"authority-1\",\"kind\":\"review\"},\"decision\":\"pending\",\"freshness\":{\"observed_at_ms\":1800000000000,\"observed_revision\":8,\"state\":\"stale\"}}",
            );
        let snapshot = decode_review_snapshot(text.as_bytes()).unwrap();
        let semantic = PlatformReviewSemantic::from(&snapshot);

        assert_eq!(semantic.attention.state, AttentionState::Idle);
        assert!(!semantic.attention.needs_user_action);
        assert_eq!(semantic.review.freshness.state, ReviewFreshnessState::Stale);
        assert_eq!(semantic.checks[0].state, CheckState::Unavailable);

        let unavailable = PlatformReviewLoad::Unavailable(PlatformReviewUnavailable {
            category: "not_available".to_owned(),
            explanation: "review projection is not available".to_owned(),
        });
        assert!(!unavailable.needs_user_action());
    }

    // SDTEST-1791
    #[test]
    fn exact_snapshot_coordinates_prepare_each_supported_external_action_family() {
        let snapshot = decode_review_snapshot(CANONICAL_FIXTURE).unwrap();
        let mut semantic = PlatformReviewSemantic::from(&snapshot);
        semantic.comments[0].agent_state = CommentAgentState::NotSent;
        let target = PlatformReviewTarget {
            project: ProjectId::new("project-1").unwrap(),
            workspace: snapshot.workspace().clone(),
        };
        assert!(semantic.approval_is_actionable());
        let mut non_actionable_approval = semantic.clone();
        non_actionable_approval.review.decision = ReviewDecision::Approved;
        assert!(!non_actionable_approval.approval_is_actionable());
        non_actionable_approval.review.decision = ReviewDecision::Pending;
        non_actionable_approval.review.freshness.state = ReviewFreshnessState::Stale;
        assert!(!non_actionable_approval.approval_is_actionable());

        let batch = PlatformReviewActionPreview::batch_send_comments(
            target.clone(),
            &semantic,
            &[semantic.comments[0].id.clone()],
        )
        .unwrap();
        assert_eq!(batch.target(), &target);
        assert_eq!(batch.expected_revision(), semantic.revision);
        assert_eq!(
            batch.action().required_authority(),
            ReviewAuthorityKind::Review
        );
        let ReviewAction::BatchSendCommentsToAgent { comments } = batch.action() else {
            panic!("batch preview changed action family");
        };
        assert_eq!(comments.len(), 1);
        assert_eq!(comments[0].comment_id().as_str(), semantic.comments[0].id);
        assert_eq!(
            comments[0].expected_revision(),
            semantic.comments[0].revision
        );
        assert_eq!(
            batch.confirmation_coordinates(),
            Some(PlatformReviewConfirmationCoordinates::CommentBatch {
                comments: vec![(
                    semantic.comments[0].id.clone(),
                    semantic.comments[0].revision,
                )],
            })
        );

        let proposal = PlatformReviewActionPreview::apply_proposal(
            target.clone(),
            &semantic,
            &semantic.proposals[0].id,
        )
        .unwrap();
        assert_eq!(proposal.target(), &target);
        assert_eq!(proposal.expected_revision(), semantic.revision);
        assert_eq!(
            proposal.action().required_authority(),
            ReviewAuthorityKind::Git
        );
        let ReviewAction::Commit { proposal_id } = proposal.action() else {
            panic!("proposal preview changed action family");
        };
        assert_eq!(proposal_id.as_str(), semantic.proposals[0].id);
        assert_eq!(
            proposal.confirmation_coordinates(),
            Some(PlatformReviewConfirmationCoordinates::Proposal {
                kind: ReviewProposalKind::Commit,
                proposal_id: semantic.proposals[0].id.clone(),
            })
        );

        let rerun = PlatformReviewActionPreview::rerun_check(
            target.clone(),
            &semantic,
            &rerun_capabilities(&target, &semantic),
            &semantic.checks[0].id,
        )
        .unwrap();
        assert_eq!(rerun.target(), &target);
        assert_eq!(rerun.expected_revision(), semantic.revision);
        assert_eq!(rerun.action().required_authority(), ReviewAuthorityKind::Ci);
        let ReviewAction::RerunCheck {
            check_id,
            expected_check_revision,
        } = rerun.action()
        else {
            panic!("check preview changed action family");
        };
        assert_eq!(check_id.as_str(), semantic.checks[0].id);
        assert_eq!(
            *expected_check_revision,
            semantic.checks[0].freshness.observed_revision
        );
        assert_eq!(
            rerun.confirmation_coordinates(),
            Some(PlatformReviewConfirmationCoordinates::Check {
                check_id: semantic.checks[0].id.clone(),
                revision: semantic.checks[0].freshness.observed_revision,
            })
        );

        let merge =
            PlatformReviewActionPreview::merge_pull_request(target.clone(), &semantic).unwrap();
        assert_eq!(merge.target(), &target);
        assert_eq!(merge.expected_revision(), semantic.revision);
        assert_eq!(
            merge.action().required_authority(),
            ReviewAuthorityKind::PullRequest
        );
        let ReviewAction::MergePullRequest {
            pull_request_id,
            expected_pull_request_revision,
            expected_head_revision,
        } = merge.action()
        else {
            panic!("merge preview changed action family");
        };
        assert_eq!(
            pull_request_id.as_str(),
            semantic.pull_request.id.as_deref().unwrap()
        );
        assert_eq!(
            *expected_pull_request_revision,
            semantic.pull_request.freshness.observed_revision
        );
        assert_eq!(
            expected_head_revision.as_str(),
            semantic.pull_request.head_revision.as_deref().unwrap()
        );
        assert_eq!(
            merge.confirmation_coordinates(),
            Some(PlatformReviewConfirmationCoordinates::PullRequestMerge {
                pull_request_id: semantic.pull_request.id.clone().unwrap(),
                revision: semantic.pull_request.freshness.observed_revision,
                head_revision: semantic.pull_request.head_revision.clone().unwrap(),
            })
        );

        let mut stale = semantic.clone();
        stale.pull_request.freshness.state = ReviewFreshnessState::Stale;
        assert!(PlatformReviewActionPreview::merge_pull_request(target.clone(), &stale).is_err());
        stale = semantic.clone();
        stale.checks[0].freshness.state = ReviewFreshnessState::Stale;
        assert!(PlatformReviewActionPreview::rerun_check(
            target.clone(),
            &stale,
            &rerun_capabilities(&target, &semantic),
            &stale.checks[0].id,
        )
        .is_err());
        let mut conflicted = semantic.clone();
        let proposal_file = conflicted.proposals[0].files[0].clone();
        conflicted
            .files
            .iter_mut()
            .find(|file| file.id == proposal_file)
            .unwrap()
            .conflict = ConflictState::Unresolved;
        assert!(PlatformReviewActionPreview::apply_proposal(
            target.clone(),
            &conflicted,
            &conflicted.proposals[0].id,
        )
        .is_err());
        let mut wrong_authority = semantic.clone();
        wrong_authority.review.authority.kind = ReviewAuthorityKind::Git;
        wrong_authority.proposals[0]
            .authority
            .as_mut()
            .unwrap()
            .kind = ReviewAuthorityKind::Review;
        wrong_authority.checks[0].authority.kind = ReviewAuthorityKind::Review;
        wrong_authority.pull_request.authority.kind = ReviewAuthorityKind::Review;
        assert!(!wrong_authority.approval_is_actionable());
        assert!(PlatformReviewActionPreview::batch_send_comments(
            target.clone(),
            &wrong_authority,
            &[wrong_authority.comments[0].id.clone()],
        )
        .is_err());
        assert!(PlatformReviewActionPreview::apply_proposal(
            target.clone(),
            &wrong_authority,
            &wrong_authority.proposals[0].id,
        )
        .is_err());
        assert!(PlatformReviewActionPreview::rerun_check(
            target.clone(),
            &wrong_authority,
            &rerun_capabilities(&target, &semantic),
            &wrong_authority.checks[0].id,
        )
        .is_err());
        assert!(
            PlatformReviewActionPreview::merge_pull_request(target.clone(), &wrong_authority,)
                .is_err()
        );
        let foreign_target = PlatformReviewTarget {
            project: target.project.clone(),
            workspace: WorkContextIdentity::parse_local(
                WorkContextTargetKind::UserWorkspace,
                "wc_user_foreign",
            )
            .unwrap(),
        };
        assert!(
            PlatformReviewActionPreview::merge_pull_request(foreign_target, &semantic).is_err()
        );
        assert!(PlatformReviewActionPreview::apply_proposal(
            target.clone(),
            &semantic,
            "proposal-missing",
        )
        .is_err());
        assert!(PlatformReviewActionPreview::rerun_check(
            target.clone(),
            &semantic,
            &rerun_capabilities(&target, &semantic),
            "check-missing",
        )
        .is_err());
        assert!(PlatformReviewActionPreview::batch_send_comments(
            target,
            &semantic,
            &[
                semantic.comments[0].id.clone(),
                semantic.comments[0].id.clone()
            ],
        )
        .is_err());
    }

    // SDTEST-1778
    #[test]
    fn canonical_render_corpus_projects_every_cross_client_semantic_key() {
        let canonical = automonique_protocol::wire::parse_canonical(RENDER_CORPUS).unwrap();
        assert_eq!(canonical.to_canonical_bytes(), RENDER_CORPUS);
        let corpus: RenderCorpus = serde_json::from_slice(RENDER_CORPUS).unwrap();
        assert_eq!(corpus.schema, "automonique.render-conformance/v1");
        assert_eq!(corpus.version, "1");

        let mut attention = BTreeSet::new();
        let mut review = BTreeSet::new();
        let mut checks = BTreeSet::new();
        let mut pull_requests = BTreeSet::new();
        let mut readiness = BTreeSet::new();
        let mut deliveries = BTreeSet::new();
        let mut previews = BTreeSet::new();
        let mut freshness = BTreeSet::new();

        for case in corpus.cases {
            assert_eq!(case.id, case.input.attention.state);
            let semantic = semantic_from_render_input(&case.input);
            let actual = PlatformReviewRenderSemantic::from(&semantic);

            assert_eq!(
                actual.source_revision.get().to_string(),
                case.expected.source_revision
            );
            assert_eq!(
                actual.attention.semantic_key,
                case.expected.attention.semantic_key
            );
            assert_eq!(
                actual.attention.reason_key,
                case.expected.attention.reason_key
            );
            assert_eq!(
                actual
                    .attention
                    .source_revision
                    .map(|value| value.get().to_string()),
                case.expected.attention.source_revision
            );
            assert_fresh_render(&actual.review, &case.expected.review);
            assert_eq!(actual.checks.len(), case.expected.checks.len());
            for (actual, expected) in actual.checks.iter().zip(case.expected.checks.iter()) {
                assert_eq!(actual.id, expected.id);
                assert_fresh_render(&actual.semantic, &expected.semantic);
            }
            assert_fresh_render(&actual.pull_request, &case.expected.pull_request);
            assert_fresh_render(&actual.delivery, &case.expected.delivery);
            assert_eq!(actual.previews.len(), case.expected.previews.len());
            for (actual, expected) in actual.previews.iter().zip(case.expected.previews.iter()) {
                assert_eq!(actual.id, expected.id);
                assert_eq!(actual.semantic_key, expected.semantic_key);
                assert_eq!(
                    actual.source_revision.get().to_string(),
                    expected.source_revision
                );
            }

            attention.insert(case.input.attention.state);
            review.insert(case.input.review.decision);
            checks.extend(case.input.checks.iter().map(|value| value.state.clone()));
            pull_requests.insert(case.input.pull_request.state);
            readiness.insert(case.input.pull_request.readiness);
            deliveries.insert(case.input.delivery.state);
            previews.extend(case.input.files.into_iter().map(|value| value.preview.kind));
            freshness.insert(case.input.review.freshness.state);
            freshness.extend(
                case.input
                    .checks
                    .iter()
                    .map(|value| value.freshness.state.clone()),
            );
            freshness.insert(case.input.pull_request.freshness.state);
            freshness.insert(case.input.delivery.freshness.state);
        }

        assert_eq!(
            attention,
            AttentionState::ALL
                .map(AttentionState::as_str)
                .map(str::to_owned)
                .into()
        );
        assert_eq!(
            review,
            ReviewDecision::ALL
                .map(ReviewDecision::as_str)
                .map(str::to_owned)
                .into()
        );
        assert_eq!(
            checks,
            CheckState::ALL
                .map(CheckState::as_str)
                .map(str::to_owned)
                .into()
        );
        assert_eq!(
            pull_requests,
            PullRequestState::ALL
                .map(PullRequestState::as_str)
                .map(str::to_owned)
                .into()
        );
        assert_eq!(
            readiness,
            MergeReadiness::ALL
                .map(MergeReadiness::as_str)
                .map(str::to_owned)
                .into()
        );
        assert_eq!(
            deliveries,
            DeliveryState::ALL
                .map(DeliveryState::as_str)
                .map(str::to_owned)
                .into()
        );
        assert_eq!(
            previews,
            PreviewKind::ALL
                .map(PreviewKind::as_str)
                .map(str::to_owned)
                .into()
        );
        assert_eq!(
            freshness,
            ReviewFreshnessState::ALL
                .map(ReviewFreshnessState::as_str)
                .map(str::to_owned)
                .into()
        );
    }

    // SDTEST-1781
    #[test]
    fn native_review_previews_admit_only_exact_comment_and_approval_authority() {
        let snapshot = decode_review_snapshot(CANONICAL_FIXTURE).unwrap();
        let semantic = PlatformReviewSemantic::from(&snapshot);
        let target = PlatformReviewTarget {
            project: ProjectId::new("project-1").unwrap(),
            workspace: snapshot.workspace().clone(),
        };
        let anchor = ReviewAnchorSemantic {
            file_id: "file-1".to_owned(),
            hunk_id: "hunk-1".to_owned(),
            side: DiffSide::New,
            line: 11,
        };

        let comment = PlatformReviewActionPreview::add_comment(
            target.clone(),
            &semantic,
            &anchor,
            "  Exact line comment.  ",
        )
        .unwrap();
        assert_eq!(comment.target(), &target);
        assert_eq!(comment.expected_revision(), semantic.revision);
        assert_eq!(
            comment.action().required_authority(),
            ReviewAuthorityKind::Review
        );
        assert!(matches!(
            comment.action(),
            ReviewAction::AddComment { anchor, body, .. }
                if anchor.file_id().as_str() == "file-1"
                    && anchor.hunk_id().as_str() == "hunk-1"
                    && anchor.line() == 11
                    && body.as_str() == "Exact line comment."
        ));

        let approval = PlatformReviewActionPreview::approve(target.clone(), &semantic).unwrap();
        assert!(matches!(
            approval.action(),
            ReviewAction::ApproveReview { expected_review_revision }
                if *expected_review_revision == semantic.review.freshness.observed_revision
        ));
        assert_ne!(approval.idempotency_key(), comment.idempotency_key());

        let foreign = PlatformReviewTarget {
            project: target.project.clone(),
            workspace: WorkContextIdentity::parse_local(
                WorkContextTargetKind::UserWorkspace,
                "wc_foreign",
            )
            .unwrap(),
        };
        assert!(
            PlatformReviewActionPreview::add_comment(foreign, &semantic, &anchor, "foreign")
                .is_err()
        );
        let outside = ReviewAnchorSemantic { line: 99, ..anchor };
        assert!(
            PlatformReviewActionPreview::add_comment(target, &semantic, &outside, "outside")
                .is_err()
        );
    }
}
