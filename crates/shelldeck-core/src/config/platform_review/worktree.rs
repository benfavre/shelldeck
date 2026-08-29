//! Combined worktree lanes and bounded safe previews for one review snapshot.
//!
//! This module is a pure read-side projection of [`PlatformReviewSemantic`].
//! It answers two questions and nothing else: which combined
//! staged/unstaged/untracked/conflicted lane every observed file belongs to,
//! and what — if anything — may be painted for that file's preview.
//!
//! It grants no authority. Staging is a Git mutation, so the projection
//! reports the server's *proposed* transitions as observations and separately
//! reports why no staging control may be offered. See
//! [`advertised_staging_capability`] for the exact contract gap.

use super::{
    ConflictState, DiffChangeKind, DiffSide, PlatformReviewSemantic, PreviewKind,
    ReviewAnchorSemantic, ReviewCapabilities, ReviewFileSemantic, ReviewProposalKind,
    WorktreeFileState,
};
use automonique_protocol::primitives::Revision;

/// Largest declared preview payload ShellDeck will describe at all.
///
/// The declared size is the server's claim, not a measurement: refusing above
/// this ceiling keeps an inflated claim from becoming a rendering budget.
pub const MAX_SAFE_PREVIEW_BYTES: u64 = 512 * 1024;

/// Largest declared raster area ShellDeck will describe.
pub const MAX_SAFE_PREVIEW_PIXELS: u64 = 16 * 1024 * 1024;

/// Largest declared raster edge ShellDeck will describe.
pub const MAX_SAFE_PREVIEW_EDGE: u32 = 16_384;

/// Longest edge, in logical pixels, of the bounded image placeholder box.
pub const SAFE_PREVIEW_BOX_EDGE: u32 = 200;

/// Most preview lines retained for one file or one hunk.
pub const MAX_SAFE_PREVIEW_LINES: usize = 12;

/// Most characters retained per preview line.
pub const MAX_SAFE_PREVIEW_LINE_CHARS: usize = 160;

/// Invisible formatting scalars that can reorder or hide neighbouring text.
///
/// `char::is_control` — the protocol's own bound — does not cover these, so a
/// bidirectional override smuggled into a path or hunk preview would still
/// render as a convincingly different line (the "trojan source" class).
const INVISIBLE_FORMATTING: [char; 13] = [
    '\u{061c}', // ARABIC LETTER MARK
    '\u{200b}', // ZERO WIDTH SPACE
    '\u{200c}', // ZERO WIDTH NON-JOINER
    '\u{200d}', // ZERO WIDTH JOINER
    '\u{200e}', // LEFT-TO-RIGHT MARK
    '\u{200f}', // RIGHT-TO-LEFT MARK
    '\u{202a}', // LEFT-TO-RIGHT EMBEDDING
    '\u{202b}', // RIGHT-TO-LEFT EMBEDDING
    '\u{202d}', // LEFT-TO-RIGHT OVERRIDE
    '\u{202e}', // RIGHT-TO-LEFT OVERRIDE
    '\u{2066}', // LEFT-TO-RIGHT ISOLATE
    '\u{2067}', // RIGHT-TO-LEFT ISOLATE
    '\u{2068}', // FIRST STRONG ISOLATE
];

/// One combined lane of the review's worktree state.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum ReviewWorktreeLane {
    /// Unresolved merge conflicts. Listed first: nothing else can proceed.
    Conflicted,
    Staged,
    Unstaged,
    Untracked,
}

impl ReviewWorktreeLane {
    /// Presentation order, most blocking first.
    pub const ALL: [Self; 4] = [
        Self::Conflicted,
        Self::Staged,
        Self::Unstaged,
        Self::Untracked,
    ];

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Conflicted => "conflicted",
            Self::Staged => "staged",
            Self::Unstaged => "unstaged",
            Self::Untracked => "untracked",
        }
    }

    /// Canonical cross-client presentation key, carrying no localized copy.
    #[must_use]
    pub fn semantic_key(self) -> String {
        format!("worktree_lane.{}", self.as_str())
    }
}

/// Why the projection offers no Git staging control.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReviewStagingWithheld {
    /// The separately fetched capability response advertises no staging
    /// capability for this exact project, workspace, and snapshot revision.
    NoServerCapability,
    /// No durable at-most-once custody store is available in this process.
    NoCustodyLane,
}

impl ReviewStagingWithheld {
    /// Every reason a surface must be able to explain.
    pub const ALL: [Self; 2] = [Self::NoServerCapability, Self::NoCustodyLane];

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NoServerCapability => "no_server_capability",
            Self::NoCustodyLane => "no_custody_lane",
        }
    }

    #[must_use]
    pub fn semantic_key(self) -> String {
        format!("staging_withheld.{}", self.as_str())
    }
}

/// Whether the server advertised an exact Git staging capability.
///
/// [`ReviewCapabilities`] carries `rerunnable_checks` and nothing else. The
/// Platform v2 contract defines no staging capability, no staging confirmation
/// digest, and no staging receipt-correlation digest, and
/// [`super::PlatformReviewActionPreview`] structurally refuses a confirmation
/// on `Stage`, `Unstage`, `Commit`, and `ResolveConflict`. There is therefore
/// nothing a client could examine that would prove staging authority, and a
/// proposal's `git` authority inside the read snapshot is an observation, not
/// a capability. This stays `false` for every capability load until the
/// contract grows the missing lane.
#[must_use]
pub const fn advertised_staging_capability(_capabilities: Option<&ReviewCapabilities>) -> bool {
    false
}

/// Decide whether a Git staging control may be rendered at all.
///
/// Both fences must hold, exactly like the confirmed check rerun: the server
/// must advertise the capability, and the durable custody store must be able
/// to record the at-most-once boundary before dispatch. A missing fence makes
/// the control absent, never optimistically disabled.
///
/// # Errors
///
/// Returns the first unmet fence.
pub const fn review_staging_control(
    server_capability: bool,
    custody_available: bool,
) -> Result<(), ReviewStagingWithheld> {
    if !server_capability {
        Err(ReviewStagingWithheld::NoServerCapability)
    } else if !custody_available {
        Err(ReviewStagingWithheld::NoCustodyLane)
    } else {
        Ok(())
    }
}

/// Bounded, control-free text safe to paint as plain characters.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ReviewSafeText {
    pub lines: Vec<String>,
    /// At least one line or one character was dropped by the bounds.
    pub truncated: bool,
}

impl ReviewSafeText {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }
}

/// Server-sanitized raster metadata plus the bounded box that may be painted.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReviewSafeImage {
    pub media_type: String,
    pub width: u32,
    pub height: u32,
    pub byte_size: u64,
    /// Aspect-preserving placeholder width, never above [`SAFE_PREVIEW_BOX_EDGE`].
    pub box_width: u32,
    /// Aspect-preserving placeholder height, never above [`SAFE_PREVIEW_BOX_EDGE`].
    pub box_height: u32,
}

/// Server-sanitized HTML, described only.
///
/// This deliberately carries no markup and no source text: the review contract
/// ships no HTML body, and ShellDeck would neither interpret one as markup nor
/// re-emit one as source.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReviewSafeHtml {
    pub media_type: String,
    pub byte_size: u64,
}

/// Why a file's preview is not painted.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReviewPreviewWithheld {
    /// The snapshot declares no preview, or the declared text carries none.
    NoContent,
    /// Opaque binary content is never previewed.
    Binary,
    /// The server did not mark the payload sanitized.
    Unsanitized,
    /// The declared payload is missing or above the render budget.
    Oversized,
    /// The declared raster is degenerate or above the decode budget.
    OversizedRaster,
    /// The declared kind, media type, and payload disagree.
    Incoherent,
}

impl ReviewPreviewWithheld {
    /// Every reason a surface must be able to explain.
    pub const ALL: [Self; 6] = [
        Self::NoContent,
        Self::Binary,
        Self::Unsanitized,
        Self::Oversized,
        Self::OversizedRaster,
        Self::Incoherent,
    ];

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NoContent => "no_content",
            Self::Binary => "binary",
            Self::Unsanitized => "unsanitized",
            Self::Oversized => "oversized",
            Self::OversizedRaster => "oversized_raster",
            Self::Incoherent => "incoherent",
        }
    }

    #[must_use]
    pub fn semantic_key(self) -> String {
        format!("preview_withheld.{}", self.as_str())
    }
}

/// What may be painted for one reviewed file.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReviewSafePreview {
    Text(ReviewSafeText),
    Image(ReviewSafeImage),
    Html(ReviewSafeHtml),
    Withheld(ReviewPreviewWithheld),
}

impl ReviewSafePreview {
    #[must_use]
    pub const fn withheld(&self) -> Option<ReviewPreviewWithheld> {
        match self {
            Self::Withheld(reason) => Some(*reason),
            _ => None,
        }
    }
}

/// One server-proposed staging transition covering an exact file.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReviewStagingProposal {
    pub proposal_id: String,
    pub kind: ReviewProposalKind,
    /// The snapshot's own admissibility: a Git authority, a supported kind,
    /// and no unresolved conflict among the proposal's files.
    pub admissible: bool,
}

/// One reviewable hunk with its exact anchor and bounded text.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReviewWorktreeHunk {
    pub id: String,
    pub anchor: ReviewAnchorSemantic,
    pub text: ReviewSafeText,
}

/// One reviewed file as it appears inside a lane.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReviewWorktreeFile {
    pub id: String,
    pub path: ReviewSafeText,
    pub change: DiffChangeKind,
    pub worktree: WorktreeFileState,
    pub conflict: ConflictState,
    /// The file is listed in more than one lane because only part of its
    /// content is staged.
    pub partial: bool,
    pub preview: ReviewSafePreview,
    pub hunks: Vec<ReviewWorktreeHunk>,
    pub staging: Vec<ReviewStagingProposal>,
}

/// One lane of the combined worktree review.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReviewWorktreeLaneGroup {
    pub lane: ReviewWorktreeLane,
    pub semantic_key: String,
    pub files: Vec<ReviewWorktreeFile>,
}

/// The combined staged/unstaged/untracked/conflicted review projection.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReviewWorktreeProjection {
    pub source_revision: Revision,
    pub lanes: Vec<ReviewWorktreeLaneGroup>,
    /// `Ok` only when a real server capability and a durable custody lane both
    /// back a staging mutation. Platform v2 advertises none, so this reports
    /// the exact missing fence instead of a control.
    pub staging: Result<(), ReviewStagingWithheld>,
}

impl ReviewWorktreeProjection {
    /// Project one review snapshot into its combined lanes.
    ///
    /// `custody_available` is the caller's durable at-most-once store; it is
    /// never inferred from the snapshot.
    #[must_use]
    pub fn new(
        review: &PlatformReviewSemantic,
        capabilities: Option<&ReviewCapabilities>,
        custody_available: bool,
    ) -> Self {
        let staging = review_staging_control(
            advertised_staging_capability(capabilities),
            custody_available,
        );
        let lanes = ReviewWorktreeLane::ALL
            .into_iter()
            .map(|lane| ReviewWorktreeLaneGroup {
                lane,
                semantic_key: lane.semantic_key(),
                files: review
                    .files
                    .iter()
                    .filter(|file| file_lanes(file).contains(&lane))
                    .map(|file| worktree_file(review, file))
                    .collect(),
            })
            .collect();
        Self {
            source_revision: review.revision,
            lanes,
            staging,
        }
    }

    /// Files listed in at least one lane, counting a partially staged file
    /// once rather than once per lane.
    #[must_use]
    pub fn distinct_file_count(&self) -> usize {
        let mut ids = self
            .lanes
            .iter()
            .flat_map(|group| group.files.iter().map(|file| file.id.as_str()))
            .collect::<Vec<_>>();
        ids.sort_unstable();
        ids.dedup();
        ids.len()
    }
}

/// The lanes one file belongs to, in presentation order.
///
/// An unresolved conflict owns the file outright: git reports it as unmerged,
/// and staging it is refused, so listing it under staged or unstaged would
/// claim a state the repository does not have. `partially_staged` genuinely
/// has content on both sides and is therefore listed twice.
fn file_lanes(file: &ReviewFileSemantic) -> Vec<ReviewWorktreeLane> {
    if file.conflict == ConflictState::Unresolved {
        return vec![ReviewWorktreeLane::Conflicted];
    }
    match file.worktree {
        WorktreeFileState::Staged => vec![ReviewWorktreeLane::Staged],
        WorktreeFileState::Unstaged => vec![ReviewWorktreeLane::Unstaged],
        WorktreeFileState::Untracked => vec![ReviewWorktreeLane::Untracked],
        WorktreeFileState::PartiallyStaged => {
            vec![ReviewWorktreeLane::Staged, ReviewWorktreeLane::Unstaged]
        }
    }
}

fn worktree_file(review: &PlatformReviewSemantic, file: &ReviewFileSemantic) -> ReviewWorktreeFile {
    ReviewWorktreeFile {
        id: file.id.clone(),
        path: review_safe_text(&file.path),
        change: file.change,
        worktree: file.worktree,
        conflict: file.conflict,
        partial: file.worktree == WorktreeFileState::PartiallyStaged
            && file.conflict != ConflictState::Unresolved,
        preview: review_safe_preview(file),
        hunks: file
            .hunks
            .iter()
            .map(|hunk| ReviewWorktreeHunk {
                id: hunk.id.clone(),
                anchor: ReviewAnchorSemantic {
                    file_id: file.id.clone(),
                    hunk_id: hunk.id.clone(),
                    side: if hunk.new_lines > 0 {
                        DiffSide::New
                    } else {
                        DiffSide::Old
                    },
                    line: if hunk.new_lines > 0 {
                        hunk.new_start
                    } else {
                        hunk.old_start
                    },
                },
                text: review_safe_text(&hunk.preview),
            })
            .collect(),
        staging: review
            .proposals
            .iter()
            .filter(|proposal| proposal.files.iter().any(|id| id == &file.id))
            .map(|proposal| ReviewStagingProposal {
                proposal_id: proposal.id.clone(),
                kind: proposal.kind,
                admissible: review.proposal_is_actionable(&proposal.id),
            })
            .collect(),
    }
}

/// Bound and neutralize one server-supplied string before it is painted.
///
/// The protocol already refuses control characters, but this is the last stop
/// before pixels: line structure, invisible reordering scalars, line length,
/// and line count are all bounded here rather than trusted upstream.
#[must_use]
pub fn review_safe_text(raw: &str) -> ReviewSafeText {
    let mut lines: Vec<String> = Vec::new();
    let mut truncated = false;
    for raw_line in raw.split(['\n', '\r']) {
        if lines.len() == MAX_SAFE_PREVIEW_LINES {
            truncated = true;
            break;
        }
        let mut line = String::new();
        for (characters, character) in raw_line.chars().enumerate() {
            if characters == MAX_SAFE_PREVIEW_LINE_CHARS {
                truncated = true;
                line.push('…');
                break;
            }
            if character.is_control() || INVISIBLE_FORMATTING.contains(&character) {
                line.push('\u{fffd}');
            } else {
                line.push(character);
            }
        }
        lines.push(line);
    }
    while lines.last().is_some_and(String::is_empty) {
        lines.pop();
    }
    ReviewSafeText { lines, truncated }
}

/// Decide what may be painted for one file, from its declared preview only.
///
/// The snapshot carries preview *metadata*, never a payload: text arrives as
/// bounded hunk previews and everything else arrives as a description. Every
/// declared quantity is therefore treated as a claim to bound, not a budget to
/// honour.
#[must_use]
pub fn review_safe_preview(file: &ReviewFileSemantic) -> ReviewSafePreview {
    let preview = &file.preview;
    // The contract admits hunks only for a text preview. A snapshot that
    // disagrees with itself is described, never painted.
    if preview.kind != PreviewKind::Text && !file.hunks.is_empty() {
        return ReviewSafePreview::Withheld(ReviewPreviewWithheld::Incoherent);
    }
    match preview.kind {
        PreviewKind::None => ReviewSafePreview::Withheld(ReviewPreviewWithheld::NoContent),
        PreviewKind::Binary => ReviewSafePreview::Withheld(ReviewPreviewWithheld::Binary),
        PreviewKind::Text => {
            if !preview.sanitized {
                return ReviewSafePreview::Withheld(ReviewPreviewWithheld::Unsanitized);
            }
            if preview
                .byte_size
                .is_some_and(|size| size > MAX_SAFE_PREVIEW_BYTES)
            {
                return ReviewSafePreview::Withheld(ReviewPreviewWithheld::Oversized);
            }
            if preview.width.is_some() || preview.height.is_some() {
                return ReviewSafePreview::Withheld(ReviewPreviewWithheld::Incoherent);
            }
            let text = review_safe_text(
                &file
                    .hunks
                    .iter()
                    .map(|hunk| hunk.preview.as_str())
                    .collect::<Vec<_>>()
                    .join("\n"),
            );
            if text.is_empty() {
                ReviewSafePreview::Withheld(ReviewPreviewWithheld::NoContent)
            } else {
                ReviewSafePreview::Text(text)
            }
        }
        PreviewKind::Image => {
            if !preview.sanitized {
                return ReviewSafePreview::Withheld(ReviewPreviewWithheld::Unsanitized);
            }
            let Some(media_type) = preview
                .media_type
                .as_deref()
                .filter(|value| value.starts_with("image/"))
            else {
                return ReviewSafePreview::Withheld(ReviewPreviewWithheld::Incoherent);
            };
            let Some(byte_size) = preview
                .byte_size
                .filter(|size| *size <= MAX_SAFE_PREVIEW_BYTES)
            else {
                return ReviewSafePreview::Withheld(ReviewPreviewWithheld::Oversized);
            };
            let (Some(width), Some(height)) = (preview.width, preview.height) else {
                return ReviewSafePreview::Withheld(ReviewPreviewWithheld::OversizedRaster);
            };
            if width == 0
                || height == 0
                || width > MAX_SAFE_PREVIEW_EDGE
                || height > MAX_SAFE_PREVIEW_EDGE
                || u64::from(width) * u64::from(height) > MAX_SAFE_PREVIEW_PIXELS
            {
                return ReviewSafePreview::Withheld(ReviewPreviewWithheld::OversizedRaster);
            }
            let (box_width, box_height) = bounded_box(width, height);
            ReviewSafePreview::Image(ReviewSafeImage {
                media_type: media_type.to_owned(),
                width,
                height,
                byte_size,
                box_width,
                box_height,
            })
        }
        PreviewKind::Html => {
            if !preview.sanitized {
                return ReviewSafePreview::Withheld(ReviewPreviewWithheld::Unsanitized);
            }
            if preview.media_type.as_deref() != Some("text/html")
                || preview.width.is_some()
                || preview.height.is_some()
            {
                return ReviewSafePreview::Withheld(ReviewPreviewWithheld::Incoherent);
            }
            let Some(byte_size) = preview
                .byte_size
                .filter(|size| *size <= MAX_SAFE_PREVIEW_BYTES)
            else {
                return ReviewSafePreview::Withheld(ReviewPreviewWithheld::Oversized);
            };
            ReviewSafePreview::Html(ReviewSafeHtml {
                media_type: "text/html".to_owned(),
                byte_size,
            })
        }
    }
}

/// Fit a declared raster into the placeholder box without ever exceeding it.
fn bounded_box(width: u32, height: u32) -> (u32, u32) {
    if width <= SAFE_PREVIEW_BOX_EDGE && height <= SAFE_PREVIEW_BOX_EDGE {
        return (width, height);
    }
    let long_edge = u64::from(width.max(height));
    let scaled = |edge: u32| -> u32 {
        let value = u64::from(edge) * u64::from(SAFE_PREVIEW_BOX_EDGE) / long_edge;
        u32::try_from(value).unwrap_or(SAFE_PREVIEW_BOX_EDGE).max(1)
    };
    (scaled(width), scaled(height))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::platform_review::{
        AttentionState, DeliverySemantic, DeliveryState, MergeReadiness,
        PlatformReviewActionPreview, PlatformReviewTarget, PullRequestSemantic, PullRequestState,
        ReviewAttentionSemantic, ReviewAuthorityKind, ReviewAuthoritySemantic, ReviewDecision,
        ReviewFreshnessSemantic, ReviewFreshnessState, ReviewHunkSemantic, ReviewPreviewSemantic,
        ReviewProposalSemantic, ReviewSchemaVersion, ReviewStatusSemantic,
    };
    use crate::config::workspace_catalog::{
        PlatformContextRef, PlatformMappingReconciliation, PlatformV2Mapping,
    };
    use automonique_protocol::platform_v2::WorkContextTargetKind;
    use automonique_protocol::platform_v2_review_api::decode_review_snapshot;

    const CANONICAL_FIXTURE: &[u8] =
        include_bytes!("../../../tests/fixtures/platform-v2-review-v2.json");

    fn revision(value: u64) -> Revision {
        Revision::new(value).expect("revision")
    }

    fn freshness() -> ReviewFreshnessSemantic {
        ReviewFreshnessSemantic {
            state: ReviewFreshnessState::Fresh,
            observed_revision: revision(9),
            observed_at_ms: 1_800_000_000_000,
        }
    }

    fn authority(kind: ReviewAuthorityKind) -> ReviewAuthoritySemantic {
        ReviewAuthoritySemantic {
            kind,
            id: "authority-1".to_owned(),
        }
    }

    fn preview(kind: PreviewKind) -> ReviewPreviewSemantic {
        ReviewPreviewSemantic {
            kind,
            media_type: None,
            byte_size: None,
            width: None,
            height: None,
            sanitized: kind == PreviewKind::Text,
        }
    }

    fn text_file(
        id: &str,
        worktree: WorktreeFileState,
        conflict: ConflictState,
    ) -> ReviewFileSemantic {
        ReviewFileSemantic {
            id: id.to_owned(),
            path: format!("src/{id}.rs"),
            change: DiffChangeKind::Modified,
            worktree,
            conflict,
            preview: preview(PreviewKind::Text),
            hunks: vec![ReviewHunkSemantic {
                id: format!("{id}-hunk-1"),
                old_start: 10,
                old_lines: 2,
                new_start: 12,
                new_lines: 3,
                preview: "@@ -10,2 +12,3 @@ typed".to_owned(),
            }],
        }
    }

    fn review_with(
        files: Vec<ReviewFileSemantic>,
        proposals: Vec<ReviewProposalSemantic>,
    ) -> PlatformReviewSemantic {
        PlatformReviewSemantic {
            schema: ReviewSchemaVersion::V2,
            workspace_kind: WorkContextTargetKind::UserWorkspace,
            workspace_id: "wc_user_1".to_owned(),
            revision: revision(9),
            attention: ReviewAttentionSemantic {
                state: AttentionState::NeedsYou,
                reason: None,
                source_revision: None,
                unread: 0,
                needs_user_action: false,
            },
            attention_events: Vec::new(),
            review: ReviewStatusSemantic {
                decision: ReviewDecision::Pending,
                authority: authority(ReviewAuthorityKind::Review),
                freshness: freshness(),
            },
            checks: Vec::new(),
            pull_request: PullRequestSemantic {
                id: None,
                state: PullRequestState::Absent,
                readiness: MergeReadiness::Unknown,
                head_revision: None,
                authority: authority(ReviewAuthorityKind::PullRequest),
                freshness: freshness(),
            },
            delivery: DeliverySemantic {
                id: None,
                state: DeliveryState::NotDelivered,
                authority: authority(ReviewAuthorityKind::Delivery),
                freshness: freshness(),
            },
            files,
            comments: Vec::new(),
            proposals,
        }
    }

    fn target() -> PlatformReviewTarget {
        PlatformReviewTarget::from_exact_mapping(&PlatformV2Mapping {
            reconciliation_revision: 1,
            project: PlatformContextRef {
                id: "project-1".to_owned(),
                revision: 1,
            },
            checkout: PlatformContextRef {
                id: "checkout-1".to_owned(),
                revision: 1,
            },
            user_workspace: PlatformContextRef {
                id: "wc_user_1".to_owned(),
                revision: 1,
            },
            reconciliation: PlatformMappingReconciliation::Exact {
                reconciled_at_millis: 1,
            },
        })
        .expect("exact mapping")
    }

    fn lane<'a>(
        projection: &'a ReviewWorktreeProjection,
        lane: ReviewWorktreeLane,
    ) -> &'a [ReviewWorktreeFile] {
        &projection
            .lanes
            .iter()
            .find(|group| group.lane == lane)
            .expect("lane group")
            .files
    }

    // SDTEST-1843 — one snapshot, four lanes. A partially staged file has real
    // content on both sides and must appear twice; an unresolved conflict is
    // unmerged and must not be claimed as staged or unstaged.
    #[test]
    fn sdtest_1843_combined_lanes_place_every_file_where_git_actually_has_it() {
        let review = review_with(
            vec![
                text_file(
                    "file-staged",
                    WorktreeFileState::Staged,
                    ConflictState::None,
                ),
                text_file(
                    "file-partial",
                    WorktreeFileState::PartiallyStaged,
                    ConflictState::None,
                ),
                text_file(
                    "file-untracked",
                    WorktreeFileState::Untracked,
                    ConflictState::None,
                ),
                text_file(
                    "file-conflict",
                    WorktreeFileState::Unstaged,
                    ConflictState::Unresolved,
                ),
                text_file(
                    "file-resolved",
                    WorktreeFileState::Staged,
                    ConflictState::Resolved,
                ),
            ],
            Vec::new(),
        );
        let projection = ReviewWorktreeProjection::new(&review, None, true);

        assert_eq!(
            projection
                .lanes
                .iter()
                .map(|group| group.lane)
                .collect::<Vec<_>>(),
            ReviewWorktreeLane::ALL.to_vec(),
            "lanes are presented most blocking first"
        );
        assert_eq!(
            lane(&projection, ReviewWorktreeLane::Conflicted)
                .iter()
                .map(|file| file.id.as_str())
                .collect::<Vec<_>>(),
            ["file-conflict"]
        );
        assert_eq!(
            lane(&projection, ReviewWorktreeLane::Staged)
                .iter()
                .map(|file| file.id.as_str())
                .collect::<Vec<_>>(),
            ["file-staged", "file-partial", "file-resolved"],
            "a resolved conflict returns to its own worktree lane"
        );
        assert_eq!(
            lane(&projection, ReviewWorktreeLane::Unstaged)
                .iter()
                .map(|file| file.id.as_str())
                .collect::<Vec<_>>(),
            ["file-partial"]
        );
        assert_eq!(
            lane(&projection, ReviewWorktreeLane::Untracked)
                .iter()
                .map(|file| file.id.as_str())
                .collect::<Vec<_>>(),
            ["file-untracked"]
        );
        assert!(lane(&projection, ReviewWorktreeLane::Staged)[1].partial);
        assert!(!lane(&projection, ReviewWorktreeLane::Staged)[0].partial);
        assert_eq!(
            projection.distinct_file_count(),
            5,
            "a file listed twice is still one file"
        );
    }

    // SDTEST-1844 — the lane projection derives each hunk's review anchor. If
    // that derivation drifted, the comment composer would offer a line the
    // snapshot refuses, so the anchors are proved against the real preview
    // constructor rather than against a restatement of the same arithmetic.
    #[test]
    fn sdtest_1844_every_projected_hunk_anchor_is_admitted_by_the_comment_constructor() {
        let mut only_old = text_file("file-old", WorktreeFileState::Staged, ConflictState::None);
        only_old.hunks[0].new_lines = 0;
        only_old.hunks[0].new_start = 0;
        let review = review_with(
            vec![
                text_file("file-new", WorktreeFileState::Staged, ConflictState::None),
                only_old,
            ],
            Vec::new(),
        );
        let projection = ReviewWorktreeProjection::new(&review, None, true);
        let anchors = projection
            .lanes
            .iter()
            .flat_map(|group| group.files.iter())
            .flat_map(|file| file.hunks.iter())
            .map(|hunk| hunk.anchor.clone())
            .collect::<Vec<_>>();

        assert_eq!(anchors.len(), 2);
        assert_eq!(anchors[0].side, DiffSide::New);
        assert_eq!(anchors[0].line, 12);
        assert_eq!(anchors[1].side, DiffSide::Old);
        assert_eq!(anchors[1].line, 10);
        for anchor in &anchors {
            PlatformReviewActionPreview::add_comment(target(), &review, anchor, "typed")
                .unwrap_or_else(|error| panic!("anchor {anchor:?} refused: {error}"));
        }
    }

    // SDTEST-1845 — the protocol bounds hunk previews to 512 control-free
    // bytes, which still admits a right-to-left override that repaints a diff
    // line as something else. This is the last stop before pixels.
    #[test]
    fn sdtest_1845_safe_text_neutralizes_invisible_reordering_and_bounds_the_shape() {
        let bidi = review_safe_text("fn drop(\u{202e}) // safe\u{202d}");
        assert_eq!(bidi.lines, ["fn drop(\u{fffd}) // safe\u{fffd}"]);
        assert!(!bidi.truncated);

        let vertical = review_safe_text("first\nsecond\r\nthird");
        assert_eq!(vertical.lines, ["first", "second", "", "third"]);

        let long = review_safe_text(&"a".repeat(MAX_SAFE_PREVIEW_LINE_CHARS + 40));
        assert!(long.truncated);
        assert_eq!(long.lines.len(), 1);
        assert_eq!(
            long.lines[0].chars().count(),
            MAX_SAFE_PREVIEW_LINE_CHARS + 1,
            "the ellipsis is the only character added past the bound"
        );

        let tall = review_safe_text(&"x\n".repeat(MAX_SAFE_PREVIEW_LINES + 5));
        assert!(tall.truncated);
        assert_eq!(tall.lines.len(), MAX_SAFE_PREVIEW_LINES);

        assert!(review_safe_text("").is_empty());
        assert!(review_safe_text("\n\n").is_empty());
    }

    // SDTEST-1846 — declared raster dimensions are a claim. A sanitized image
    // is described inside a clamped aspect-preserving box; a degenerate or
    // enormous claim is refused instead of becoming a layout or decode budget.
    #[test]
    fn sdtest_1846_image_previews_are_clamped_and_absurd_rasters_are_refused() {
        let image = |width: Option<u32>, height: Option<u32>, bytes: Option<u64>| {
            let mut file = text_file("file-1", WorktreeFileState::Staged, ConflictState::None);
            file.hunks.clear();
            file.preview = ReviewPreviewSemantic {
                kind: PreviewKind::Image,
                media_type: Some("image/png".to_owned()),
                byte_size: bytes,
                width,
                height,
                sanitized: true,
            };
            file
        };

        let ReviewSafePreview::Image(small) =
            review_safe_preview(&image(Some(120), Some(60), Some(4_096)))
        else {
            panic!("a bounded sanitized raster is describable");
        };
        assert_eq!((small.box_width, small.box_height), (120, 60));

        let ReviewSafePreview::Image(large) =
            review_safe_preview(&image(Some(4_000), Some(1_000), Some(4_096)))
        else {
            panic!("a bounded sanitized raster is describable");
        };
        assert_eq!(large.box_width, SAFE_PREVIEW_BOX_EDGE);
        assert_eq!(large.box_height, SAFE_PREVIEW_BOX_EDGE / 4);
        assert_eq!((large.width, large.height), (4_000, 1_000));

        let ReviewSafePreview::Image(sliver) =
            review_safe_preview(&image(Some(4_000), Some(1), Some(4_096)))
        else {
            panic!("a bounded sanitized raster is describable");
        };
        assert_eq!(
            (sliver.box_width, sliver.box_height),
            (SAFE_PREVIEW_BOX_EDGE, 1),
            "an extreme aspect ratio still fits the box"
        );

        for absurd in [
            image(Some(0), Some(10), Some(4_096)),
            image(Some(MAX_SAFE_PREVIEW_EDGE + 1), Some(2), Some(4_096)),
            image(Some(8_192), Some(8_192), Some(4_096)),
            image(Some(10), None, Some(4_096)),
        ] {
            assert_eq!(
                review_safe_preview(&absurd).withheld(),
                Some(ReviewPreviewWithheld::OversizedRaster),
                "{:?}",
                absurd.preview
            );
        }
        assert_eq!(
            review_safe_preview(&image(Some(10), Some(10), None)).withheld(),
            Some(ReviewPreviewWithheld::Oversized)
        );
        assert_eq!(
            review_safe_preview(&image(Some(10), Some(10), Some(MAX_SAFE_PREVIEW_BYTES + 1)))
                .withheld(),
            Some(ReviewPreviewWithheld::Oversized)
        );
    }

    // SDTEST-1847 — HTML is described, never painted as markup and never
    // re-emitted as source; unsanitized, incoherent, binary and empty previews
    // are withheld with the reason the surface must show.
    #[test]
    fn sdtest_1847_html_and_opaque_previews_are_described_but_never_rendered() {
        let mut html = text_file("file-1", WorktreeFileState::Staged, ConflictState::None);
        html.hunks.clear();
        html.preview = ReviewPreviewSemantic {
            kind: PreviewKind::Html,
            media_type: Some("text/html".to_owned()),
            byte_size: Some(2_048),
            width: None,
            height: None,
            sanitized: true,
        };
        let ReviewSafePreview::Html(described) = review_safe_preview(&html) else {
            panic!("a sanitized bounded HTML preview is describable");
        };
        assert_eq!(described.media_type, "text/html");
        assert_eq!(described.byte_size, 2_048);

        let mut unsanitized = html.clone();
        unsanitized.preview.sanitized = false;
        assert_eq!(
            review_safe_preview(&unsanitized).withheld(),
            Some(ReviewPreviewWithheld::Unsanitized)
        );

        let mut wrong_media = html.clone();
        wrong_media.preview.media_type = Some("text/plain".to_owned());
        assert_eq!(
            review_safe_preview(&wrong_media).withheld(),
            Some(ReviewPreviewWithheld::Incoherent)
        );

        let mut with_hunks = html.clone();
        with_hunks.hunks = text_file("x", WorktreeFileState::Staged, ConflictState::None).hunks;
        assert_eq!(
            review_safe_preview(&with_hunks).withheld(),
            Some(ReviewPreviewWithheld::Incoherent),
            "only a text preview may carry hunks"
        );

        let mut binary = html.clone();
        binary.preview = ReviewPreviewSemantic {
            kind: PreviewKind::Binary,
            media_type: Some("application/octet-stream".to_owned()),
            byte_size: Some(64),
            width: None,
            height: None,
            sanitized: false,
        };
        assert_eq!(
            review_safe_preview(&binary).withheld(),
            Some(ReviewPreviewWithheld::Binary)
        );

        let mut none = html.clone();
        none.preview = preview(PreviewKind::None);
        assert_eq!(
            review_safe_preview(&none).withheld(),
            Some(ReviewPreviewWithheld::NoContent)
        );

        let mut unsanitized_text =
            text_file("file-2", WorktreeFileState::Staged, ConflictState::None);
        unsanitized_text.preview.sanitized = false;
        assert_eq!(
            review_safe_preview(&unsanitized_text).withheld(),
            Some(ReviewPreviewWithheld::Unsanitized),
            "an unsanitized payload is never shown as source text"
        );

        let mut empty_text = text_file("file-3", WorktreeFileState::Staged, ConflictState::None);
        empty_text.hunks.clear();
        assert_eq!(
            review_safe_preview(&empty_text).withheld(),
            Some(ReviewPreviewWithheld::NoContent)
        );
    }

    // SDTEST-1848 — a snapshot proposal carrying a `git` authority is an
    // observation. Platform v2 advertises no staging capability, so the
    // projection reports the missing fence and never an offerable control.
    #[test]
    fn sdtest_1848_staging_stays_withheld_for_every_capability_load() {
        let review = review_with(
            vec![
                text_file("file-1", WorktreeFileState::Unstaged, ConflictState::None),
                text_file(
                    "file-2",
                    WorktreeFileState::Unstaged,
                    ConflictState::Unresolved,
                ),
            ],
            vec![
                ReviewProposalSemantic {
                    id: "proposal-stage".to_owned(),
                    kind: ReviewProposalKind::Stage,
                    authority: Some(authority(ReviewAuthorityKind::Git)),
                    files: vec!["file-1".to_owned()],
                    subject: None,
                },
                ReviewProposalSemantic {
                    id: "proposal-blocked".to_owned(),
                    kind: ReviewProposalKind::Stage,
                    authority: Some(authority(ReviewAuthorityKind::Git)),
                    files: vec!["file-2".to_owned()],
                    subject: None,
                },
            ],
        );

        assert!(!advertised_staging_capability(None));
        assert_eq!(
            ReviewWorktreeProjection::new(&review, None, true).staging,
            Err(ReviewStagingWithheld::NoServerCapability)
        );
        assert_eq!(
            ReviewWorktreeProjection::new(&review, None, false).staging,
            Err(ReviewStagingWithheld::NoServerCapability),
            "the absent server capability is reported before the custody lane"
        );
        assert_eq!(
            review_staging_control(true, false),
            Err(ReviewStagingWithheld::NoCustodyLane)
        );
        assert_eq!(review_staging_control(true, true), Ok(()));

        let projection = ReviewWorktreeProjection::new(&review, None, true);
        let unstaged = lane(&projection, ReviewWorktreeLane::Unstaged);
        assert_eq!(unstaged.len(), 1);
        assert_eq!(
            unstaged[0].staging,
            vec![ReviewStagingProposal {
                proposal_id: "proposal-stage".to_owned(),
                kind: ReviewProposalKind::Stage,
                admissible: true,
            }]
        );
        let conflicted = lane(&projection, ReviewWorktreeLane::Conflicted);
        assert_eq!(conflicted.len(), 1);
        assert!(
            !conflicted[0].staging[0].admissible,
            "an unresolved conflict blocks its own staging proposal"
        );
    }

    // SDTEST-1849 — the shipped canonical snapshot, not a hand-built one,
    // reaches the combined lanes, the safe text preview, and the staging
    // observation the surface renders.
    #[test]
    fn sdtest_1849_canonical_snapshot_projects_to_lanes_previews_and_observations() {
        let snapshot = decode_review_snapshot(CANONICAL_FIXTURE).expect("canonical fixture");
        let review = PlatformReviewSemantic::from(&snapshot);
        let projection = ReviewWorktreeProjection::new(&review, None, true);

        assert_eq!(projection.source_revision, review.revision);
        assert_eq!(projection.distinct_file_count(), 1);
        for lane_kind in [ReviewWorktreeLane::Staged, ReviewWorktreeLane::Unstaged] {
            let files = lane(&projection, lane_kind);
            assert_eq!(files.len(), 1, "{lane_kind:?}");
            assert_eq!(files[0].path.lines, ["src/review.rs"]);
            assert!(files[0].partial);
            assert_eq!(files[0].hunks.len(), 1);
            assert_eq!(files[0].hunks[0].anchor.side, DiffSide::New);
            assert_eq!(files[0].hunks[0].anchor.line, 10);
            assert_eq!(
                files[0].preview,
                ReviewSafePreview::Text(ReviewSafeText {
                    lines: vec!["@@ -10,2 +10,3 @@ · sanitized preview".to_owned()],
                    truncated: false,
                })
            );
            assert_eq!(
                files[0].staging,
                vec![ReviewStagingProposal {
                    proposal_id: "proposal-1".to_owned(),
                    kind: ReviewProposalKind::Commit,
                    admissible: true,
                }]
            );
        }
        assert!(lane(&projection, ReviewWorktreeLane::Conflicted).is_empty());
        assert!(lane(&projection, ReviewWorktreeLane::Untracked).is_empty());
        assert_eq!(
            projection.staging,
            Err(ReviewStagingWithheld::NoServerCapability)
        );
    }
}
