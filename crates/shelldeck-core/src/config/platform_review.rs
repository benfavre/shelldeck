//! Read-only presentation of the shared Automonique Platform v2 review contract.
//!
//! This module does not replace [`crate::workspace_review`], which remains the
//! persisted local workflow. Platform review values are authenticated remote
//! observations for display only; they never authorize filesystem, git, CI,
//! pull-request, review, or delivery mutations in ShellDeck.

use automonique_protocol::platform_v2::{ProjectId, WorkContextIdentity, WorkContextTargetKind};
pub use automonique_protocol::platform_v2_review::{
    AttentionOriginKind, AttentionReason, AttentionState, CheckState, CommentAgentState,
    ConflictState, DeliveryState, DiffChangeKind, DiffSide, MergeReadiness, PreviewKind,
    PullRequestState, ReviewAuthority, ReviewAuthorityKind, ReviewDecision, ReviewFile,
    ReviewFreshness, ReviewFreshnessState, ReviewProposalKind, ReviewSchemaVersion, ReviewSnapshot,
    WorktreeFileState,
};
use automonique_protocol::primitives::Revision;

use super::workspace_catalog::PlatformV2Mapping;

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
pub enum PlatformReviewLoad {
    Available(Box<PlatformReviewSemantic>),
    Unavailable(PlatformReviewUnavailable),
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
}
