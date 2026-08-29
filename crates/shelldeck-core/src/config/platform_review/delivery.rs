//! Server-advertised batch delivery of review comments to the authorized session.
//!
//! This module is a read-side projection, exactly like [`super::worktree`]. It
//! grants nothing: it reports which comments the server has *proven* it can
//! deliver to the retained agent session bound to this review coordinate, and
//! otherwise reports which fence is missing so the surface can say so instead
//! of offering a control that would refuse.
//!
//! # Why this lane carries no confirmation digest
//!
//! A confirmed check rerun needs one because the client names the target and
//! the effect fires in a system the daemon does not own, so only a digest can
//! bind the executed write to the preflighted plan. Delivery to a retained
//! session differs on every axis that motivated the digest:
//!
//! * The target is never on the wire. Provider and session identity come from
//!   the operator-owned registry, keyed by the review coordinate, so a client
//!   can neither name nor redirect the session it reaches.
//! * The snapshot revision is fenced twice server-side, by the store's
//!   current-revision check and again by `resolve_action`.
//! * The comment set is fenced by the batch arm of `resolve_action`, which
//!   requires every comment to exist at exactly the advertised revision and to
//!   still be in `not_sent` or `refused`.
//!
//! Exactly-once therefore falls out of the domain state machine rather than a
//! digest: settling a delivery moves the comment out of those two states and
//! bumps both the snapshot revision and the comment's own revision, so the
//! advertisement that authorized a send cannot construct a second one.
//!
//! Minting a receipt correlation here would be worse than redundant: the
//! host's receipt lookup *skips* a retained action whenever the request
//! carries a correlation digest, so advertising one would hand this client a
//! token that makes its own receipt unfindable. A fence that breaks recovery
//! is not a fence.
//!
//! The client consequence is the one rule this module exists to enforce: the
//! advertisement is the fence, so it must be re-read after every settlement
//! rather than reused. See [`super::PlatformReviewActionPreview::matches_capabilities`].

use super::{
    PlatformReviewSemantic, PlatformReviewTarget, ReviewAuthorityKind, ReviewCapabilities,
};
use automonique_protocol::primitives::Revision;

/// Why the projection offers no batch send-to-agent control.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReviewDeliveryWithheld {
    /// No capability response is attributed to this exact project, workspace
    /// and snapshot revision, so nothing has been advertised at all.
    NoServerCapability,
    /// The capability response is exact and its advertised list is empty.
    /// That is the server's honest fail-closed answer — no registry binding,
    /// no reachable delivery adapter, a stale review, or no comment currently
    /// in a sendable state — and it must produce no control.
    NoDeliverableComment,
    /// No durable at-most-once custody store is available in this process.
    NoCustodyLane,
}

impl ReviewDeliveryWithheld {
    /// Every reason a surface must be able to explain.
    pub const ALL: [Self; 3] = [
        Self::NoServerCapability,
        Self::NoDeliverableComment,
        Self::NoCustodyLane,
    ];

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NoServerCapability => "no_server_capability",
            Self::NoDeliverableComment => "no_deliverable_comment",
            Self::NoCustodyLane => "no_custody_lane",
        }
    }

    /// Canonical cross-client presentation key, carrying no localized copy.
    #[must_use]
    pub fn semantic_key(self) -> String {
        format!("delivery_withheld.{}", self.as_str())
    }
}

/// One comment the server advertised as deliverable, carried verbatim.
///
/// `expected_comment_revision` is the server's value, never the snapshot's.
/// The two agree on a coherent read — that is what [`advertised_agent_deliveries`]
/// checks — but the value that crosses the network is this one, because it is
/// the one the server committed to accepting.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AdvertisedReviewDelivery {
    pub comment_id: String,
    pub expected_comment_revision: Revision,
    pub authority_id: String,
}

/// The exact advertised delivery set for one active snapshot.
///
/// Every entry survives three checks: the capability response is attributed to
/// this exact project, workspace and snapshot revision; its authority is the
/// review authority the snapshot itself names; and the comment is present in
/// that same snapshot, at the advertised revision, in a sendable state.
///
/// The third check is a torn-read guard, not a second source of authority. The
/// capability list and the snapshot are two separate reads, and a coherent
/// server mints them from one projection, so a disagreement means they
/// straddled a change. Refusing the entry is the fail-closed answer; it can
/// never *add* an entry the server did not advertise.
#[must_use]
pub fn advertised_agent_deliveries(
    capabilities: Option<&ReviewCapabilities>,
    target: &PlatformReviewTarget,
    review: &PlatformReviewSemantic,
) -> Vec<AdvertisedReviewDelivery> {
    let Some(capabilities) = capabilities else {
        return Vec::new();
    };
    if capabilities.project() != &target.project
        || capabilities.workspace() != &target.workspace
        || capabilities.snapshot_revision() != review.revision
        || review.workspace_kind != target.workspace.kind()
        || review.workspace_id != target.workspace.id()
    {
        return Vec::new();
    }
    capabilities
        .agent_deliverable_comments()
        .iter()
        .filter(|capability| {
            capability.authority().kind() == ReviewAuthorityKind::Review
                && capability.authority().id().as_str() == review.review.authority.id
                && review.review.authority.kind == ReviewAuthorityKind::Review
        })
        .filter(|capability| {
            review.comments.iter().any(|comment| {
                comment.id == capability.comment_id().as_str()
                    && comment.revision == capability.expected_comment_revision()
            }) && review.comment_is_batch_actionable(capability.comment_id().as_str())
        })
        .map(|capability| AdvertisedReviewDelivery {
            comment_id: capability.comment_id().as_str().to_owned(),
            expected_comment_revision: capability.expected_comment_revision(),
            authority_id: capability.authority().id().as_str().to_owned(),
        })
        .collect()
}

/// Decide whether a batch send-to-agent control may be rendered at all.
///
/// Both fences must hold, exactly like the confirmed check rerun: the server
/// must have advertised at least one deliverable comment for this exact
/// snapshot, and the durable custody store must be able to record the
/// at-most-once boundary before dispatch. This lane needs no *confirmation
/// digest*, but it still needs the durable record every other exposed mutation
/// gets: a restart between the POST and the receipt must not make a delivered
/// batch indistinguishable from one that never left.
///
/// A missing fence makes the control absent, never optimistically disabled.
///
/// # Errors
///
/// Returns the first unmet fence.
pub const fn review_delivery_control(
    exact_capability: bool,
    advertised: bool,
    custody_available: bool,
) -> Result<(), ReviewDeliveryWithheld> {
    if !exact_capability {
        Err(ReviewDeliveryWithheld::NoServerCapability)
    } else if !advertised {
        Err(ReviewDeliveryWithheld::NoDeliverableComment)
    } else if !custody_available {
        Err(ReviewDeliveryWithheld::NoCustodyLane)
    } else {
        Ok(())
    }
}

/// Everything a surface needs to render the delivery lane for one snapshot.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReviewAgentDeliveryProjection {
    advertised: Vec<AdvertisedReviewDelivery>,
    /// Why no control may be offered, when none may.
    pub control: Result<(), ReviewDeliveryWithheld>,
}

impl ReviewAgentDeliveryProjection {
    #[must_use]
    pub fn new(
        review: &PlatformReviewSemantic,
        target: &PlatformReviewTarget,
        capabilities: Option<&ReviewCapabilities>,
        custody_available: bool,
    ) -> Self {
        // "Exact" is about attribution, not content: a capability response for
        // this project, workspace and snapshot revision was received. An empty
        // advertised list inside it is a different, more specific answer, and
        // the surface must be able to say which one it got.
        let exact_capability = capabilities.is_some_and(|capabilities| {
            capabilities.project() == &target.project
                && capabilities.workspace() == &target.workspace
                && capabilities.snapshot_revision() == review.revision
        });
        let advertised = advertised_agent_deliveries(capabilities, target, review);
        let control =
            review_delivery_control(exact_capability, !advertised.is_empty(), custody_available);
        Self {
            advertised,
            control,
        }
    }

    /// The projection for a surface that has no exactly attributed target.
    ///
    /// Nothing was advertised because nothing could have been, so this is the
    /// same fail-closed answer as an absent capability response.
    #[must_use]
    pub const fn unavailable() -> Self {
        Self {
            advertised: Vec::new(),
            control: Err(ReviewDeliveryWithheld::NoServerCapability),
        }
    }

    /// The advertised set, in the capability's own order.
    ///
    /// The server sorts it by comment id, and the protocol requires a batch to
    /// be strictly ordered by that same id, so a selection filtered out of
    /// this slice is already in wire order.
    #[must_use]
    pub fn advertised(&self) -> &[AdvertisedReviewDelivery] {
        &self.advertised
    }

    /// Whether this exact comment may carry a send control.
    #[must_use]
    pub fn advertises(&self, comment_id: &str) -> bool {
        self.advertised
            .iter()
            .any(|entry| entry.comment_id == comment_id)
    }
}

#[cfg(test)]
mod tests {
    use super::super::{
        CommentAgentState, PlatformReviewSemantic, ReviewAgentDeliveryCapability, ReviewAuthority,
        ReviewCommentId, ReviewFreshnessState, ReviewPullRequestCapabilities,
    };
    use super::*;
    use automonique_protocol::platform_v2::{ProjectId, WorkContextIdentity};
    use automonique_protocol::platform_v2_review::ReviewAuthorityId;
    use automonique_protocol::platform_v2_review_api::decode_review_snapshot;

    const CANONICAL_FIXTURE: &[u8] =
        include_bytes!("../../../tests/fixtures/platform-v2-review-v2.json");

    fn fixture() -> (PlatformReviewTarget, PlatformReviewSemantic) {
        let snapshot = decode_review_snapshot(CANONICAL_FIXTURE).unwrap();
        let mut review = PlatformReviewSemantic::from(&snapshot);
        review.comments[0].agent_state = CommentAgentState::NotSent;
        let target = PlatformReviewTarget {
            project: ProjectId::new("project-1").unwrap(),
            workspace: snapshot.workspace().clone(),
        };
        (target, review)
    }

    /// Build a capability response with arbitrary coordinates, so each fence
    /// can be broken one at a time.
    fn capabilities(
        project: &ProjectId,
        workspace: &WorkContextIdentity,
        snapshot_revision: Revision,
        entries: Vec<(&str, Revision, &str)>,
    ) -> ReviewCapabilities {
        let advertised = entries
            .into_iter()
            .map(|(id, revision, authority)| {
                ReviewAgentDeliveryCapability::new(
                    ReviewCommentId::new(id.to_owned()).unwrap(),
                    revision,
                    ReviewAuthority::new(
                        ReviewAuthorityKind::Review,
                        ReviewAuthorityId::new(authority.to_owned()).unwrap(),
                    ),
                )
                .unwrap()
            })
            .collect();
        ReviewCapabilities::new(
            project.clone(),
            workspace.clone(),
            snapshot_revision,
            Revision::new(91).unwrap(),
            Vec::new(),
            advertised,
            ReviewPullRequestCapabilities::default(),
        )
        .unwrap()
    }

    fn exact(target: &PlatformReviewTarget, review: &PlatformReviewSemantic) -> ReviewCapabilities {
        let comment = &review.comments[0];
        capabilities(
            &target.project,
            &target.workspace,
            review.revision,
            vec![(
                comment.id.as_str(),
                comment.revision,
                review.review.authority.id.as_str(),
            )],
        )
    }

    // SDTEST-1864
    #[test]
    fn every_delivery_fence_fails_closed_with_its_own_distinct_reason() {
        let (target, review) = fixture();
        let exact_capabilities = exact(&target, &review);

        let granted =
            ReviewAgentDeliveryProjection::new(&review, &target, Some(&exact_capabilities), true);
        assert_eq!(granted.control, Ok(()));
        assert!(granted.advertises(&review.comments[0].id));
        assert_eq!(
            granted.advertised(),
            &[AdvertisedReviewDelivery {
                comment_id: review.comments[0].id.clone(),
                expected_comment_revision: review.comments[0].revision,
                authority_id: review.review.authority.id.clone(),
            }]
        );

        // No capability response at all.
        assert_eq!(
            ReviewAgentDeliveryProjection::new(&review, &target, None, true).control,
            Err(ReviewDeliveryWithheld::NoServerCapability)
        );
        assert_eq!(
            ReviewAgentDeliveryProjection::unavailable().control,
            Err(ReviewDeliveryWithheld::NoServerCapability)
        );

        // A response for another project, another workspace, or another
        // snapshot revision is not this snapshot's answer.
        let foreign_project = capabilities(
            &ProjectId::new("project-2").unwrap(),
            &target.workspace,
            review.revision,
            vec![(
                review.comments[0].id.as_str(),
                review.comments[0].revision,
                review.review.authority.id.as_str(),
            )],
        );
        assert_eq!(
            ReviewAgentDeliveryProjection::new(&review, &target, Some(&foreign_project), true)
                .control,
            Err(ReviewDeliveryWithheld::NoServerCapability)
        );
        let stale_snapshot = capabilities(
            &target.project,
            &target.workspace,
            Revision::new(review.revision.get() + 1).unwrap(),
            vec![(
                review.comments[0].id.as_str(),
                review.comments[0].revision,
                review.review.authority.id.as_str(),
            )],
        );
        assert_eq!(
            ReviewAgentDeliveryProjection::new(&review, &target, Some(&stale_snapshot), true)
                .control,
            Err(ReviewDeliveryWithheld::NoServerCapability)
        );

        // `advertised_agent_deliveries` is asserted directly, not only through
        // the projection: the action preview calls it on its own, so a fence
        // that lived only in `ReviewAgentDeliveryProjection::new` would let a
        // preview pin itself to a foreign or superseded snapshot revision.
        assert!(
            advertised_agent_deliveries(Some(&exact_capabilities), &target, &review).len() == 1
        );
        for hole in [&foreign_project, &stale_snapshot] {
            assert!(
                advertised_agent_deliveries(Some(hole), &target, &review).is_empty(),
                "a capability response for another coordinate advertises nothing"
            );
        }
        assert!(advertised_agent_deliveries(None, &target, &review).is_empty());
        let foreign_workspace = PlatformReviewTarget {
            project: target.project.clone(),
            workspace: WorkContextIdentity::parse_local(target.workspace.kind(), "wc_user_foreign")
                .unwrap(),
        };
        assert!(advertised_agent_deliveries(
            Some(&exact_capabilities),
            &foreign_workspace,
            &review
        )
        .is_empty());

        // An exact response advertising nothing is the server's honest
        // fail-closed answer, and must read differently from an absent one.
        let empty = capabilities(
            &target.project,
            &target.workspace,
            review.revision,
            Vec::new(),
        );
        assert_eq!(
            ReviewAgentDeliveryProjection::new(&review, &target, Some(&empty), true).control,
            Err(ReviewDeliveryWithheld::NoDeliverableComment)
        );

        // Custody is still required. This lane needs no confirmation digest,
        // but it is an exposed mutation and takes the same durable record.
        assert_eq!(
            ReviewAgentDeliveryProjection::new(&review, &target, Some(&exact_capabilities), false)
                .control,
            Err(ReviewDeliveryWithheld::NoCustodyLane)
        );

        // Torn reads. The capability list and the snapshot are two separate
        // reads; a coherent server mints them from one projection, so any
        // disagreement means they straddled a change and the entry is dropped.
        let wrong_comment_revision = capabilities(
            &target.project,
            &target.workspace,
            review.revision,
            vec![(
                review.comments[0].id.as_str(),
                Revision::new(review.comments[0].revision.get() + 1).unwrap(),
                review.review.authority.id.as_str(),
            )],
        );
        assert_eq!(
            ReviewAgentDeliveryProjection::new(
                &review,
                &target,
                Some(&wrong_comment_revision),
                true
            )
            .control,
            Err(ReviewDeliveryWithheld::NoDeliverableComment)
        );
        let unknown_comment = capabilities(
            &target.project,
            &target.workspace,
            review.revision,
            vec![(
                "comment-absent",
                review.comments[0].revision,
                review.review.authority.id.as_str(),
            )],
        );
        assert_eq!(
            ReviewAgentDeliveryProjection::new(&review, &target, Some(&unknown_comment), true)
                .control,
            Err(ReviewDeliveryWithheld::NoDeliverableComment)
        );
        let foreign_authority = capabilities(
            &target.project,
            &target.workspace,
            review.revision,
            vec![(
                review.comments[0].id.as_str(),
                review.comments[0].revision,
                "authority-other",
            )],
        );
        assert_eq!(
            ReviewAgentDeliveryProjection::new(&review, &target, Some(&foreign_authority), true)
                .control,
            Err(ReviewDeliveryWithheld::NoDeliverableComment)
        );

        // A comment already delivered, and a review that is no longer fresh,
        // are both unsendable however the advertisement reads.
        let mut sent = review.clone();
        sent.comments[0].agent_state = CommentAgentState::Sent;
        assert_eq!(
            ReviewAgentDeliveryProjection::new(&sent, &target, Some(&exact_capabilities), true)
                .control,
            Err(ReviewDeliveryWithheld::NoDeliverableComment)
        );
        let mut stale = review.clone();
        stale.review.freshness.state = ReviewFreshnessState::Stale;
        assert_eq!(
            ReviewAgentDeliveryProjection::new(&stale, &target, Some(&exact_capabilities), true)
                .control,
            Err(ReviewDeliveryWithheld::NoDeliverableComment)
        );

        // Every reason a surface must explain carries its own key.
        let keys = ReviewDeliveryWithheld::ALL
            .iter()
            .map(|reason| reason.semantic_key())
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(keys.len(), ReviewDeliveryWithheld::ALL.len());
    }
}
