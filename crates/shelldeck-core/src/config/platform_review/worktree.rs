//! Combined worktree lanes and bounded safe previews for one review snapshot.
//!
//! This module is a pure read-side projection of [`PlatformReviewSemantic`].
//! It answers two questions and nothing else: which combined
//! staged/unstaged/untracked/conflicted lane every observed file belongs to,
//! and what — if anything — may be painted for that file's preview.
//!
//! It grants no authority of its own. Staging is a Git mutation, so a control
//! exists only where the server advertised a slot for that exact proposal, and
//! the projection otherwise reports which fence is missing so the surface can
//! say so instead of offering a control that would refuse.
//!
//! # What the server has to prove before a control exists
//!
//! A worktree is shared substrate: any other process running as the daemon uid
//! can move `HEAD` and rewrite the index between an advertisement and the
//! action arriving, and the snapshot revision does not help — it tracks what
//! the projection observed, not what the repository now is. So every entry in
//! `staging` and `conflict_resolutions` names both mutable things the write
//! depends on, and its confirmation digest commits to them. This module
//! carries that observation verbatim ([`ReviewWorktreeObservation`]) so a
//! reader can see what the server read rather than having to trust that it
//! read anything.
//!
//! # Absence is the withholding
//!
//! The three staging kinds share one capability type but not one grant: an
//! operator withholds committing separately from index writes, and conflict
//! resolution separately again. A deployment that grants stage and unstage but
//! not commit therefore produces a `staging` list with `Stage` and `Unstage`
//! entries and no `Commit` entry — and the commit control must simply not
//! exist. A disabled button with a tooltip would claim the grant is present
//! and merely unavailable, which is the opposite of what the list says.
//!
//! # No hunk granularity
//!
//! [`super::ReviewProposalId`] names a proposal and a proposal lists file ids;
//! `ReviewAnchor`'s hunk id exists for comments only. File-level staging is
//! all the contract can express, so it is all this module offers. A hunk
//! control here would be advertising something no action can name.

use super::{
    ConflictResolution, ConflictState, DiffChangeKind, DiffSide, PlatformReviewSemantic,
    PlatformReviewTarget, PreviewKind, ReviewAnchorSemantic, ReviewAuthorityKind,
    ReviewCapabilities, ReviewFileSemantic, ReviewProposalKind, WorktreeFileState,
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
    /// No capability response is attributed to this exact project, workspace
    /// and snapshot revision, so nothing has been advertised at all.
    NoServerCapability,
    /// The capability response is exact and both its git lists are empty.
    /// That is the server's honest fail-closed answer — no registry binding
    /// granting index writes, a repository the preflight could not read, an
    /// unborn `HEAD`, or a worktree in a state no staging write can be fenced
    /// against — and it must produce no control.
    NoStageableProposal,
    /// No durable at-most-once custody store is available in this process.
    NoCustodyLane,
}

impl ReviewStagingWithheld {
    /// Every reason a surface must be able to explain.
    pub const ALL: [Self; 3] = [
        Self::NoServerCapability,
        Self::NoStageableProposal,
        Self::NoCustodyLane,
    ];

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NoServerCapability => "no_server_capability",
            Self::NoStageableProposal => "no_stageable_proposal",
            Self::NoCustodyLane => "no_custody_lane",
        }
    }

    #[must_use]
    pub fn semantic_key(self) -> String {
        format!("staging_withheld.{}", self.as_str())
    }
}

/// The one repository read every git capability in a response was minted from.
///
/// `ReviewCapabilities::new` refuses a response whose git entries disagree on
/// either field, so this is a property of the whole document rather than of an
/// entry. It is carried here for the reason the server carries it at all: a
/// client must be able to see the `HEAD` and index the preflight observed, and
/// decline to offer a control once it holds a document that read a different
/// worktree.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReviewWorktreeObservation {
    /// The commit `HEAD` resolved to when the server read the repository.
    pub head_revision: String,
    /// A digest over the whole index as the preflight read it.
    pub index_digest: String,
}

/// One staging transition the server proved it can perform, carried verbatim.
///
/// The two confirmation digests are deliberately absent. They are minted per
/// entry and must cross the wire exactly as the server spelled them, so
/// [`super::PlatformReviewActionPreview`] reads them straight off the
/// capability rather than through a render-side copy that could drift.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AdvertisedReviewStaging {
    pub proposal_id: String,
    pub kind: ReviewProposalKind,
    pub expected_head_revision: String,
    pub expected_index_digest: String,
    pub authority_id: String,
}

/// One conflicted file the server proved it can collapse to one recorded side.
///
/// The server lists **one entry per admissible side**, not one per file with
/// the side left to the client: the side decides which bytes land, so it is
/// inside the confirmation digest, and a digest cannot commit to a choice not
/// yet made. A file with both stage 2 and stage 3 recorded therefore yields
/// two entries here, and the surface renders two controls. A delete/modify
/// conflict has one side recorded and yields one.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AdvertisedReviewConflictResolution {
    pub proposal_id: String,
    pub file_id: String,
    pub resolution: ConflictResolution,
    pub expected_head_revision: String,
    pub expected_index_digest: String,
    pub authority_id: String,
}

/// Whether a capability response is attributed to this exact review read.
fn exact_capability_attribution(
    capabilities: &ReviewCapabilities,
    target: &PlatformReviewTarget,
    review: &PlatformReviewSemantic,
) -> bool {
    capabilities.project() == &target.project
        && capabilities.workspace() == &target.workspace
        && capabilities.snapshot_revision() == review.revision
        && review.workspace_kind == target.workspace.kind()
        && review.workspace_id == target.workspace.id()
}

/// The exact advertised staging set for one active snapshot.
///
/// Every entry survives three checks, exactly like the delivery lane: the
/// capability response is attributed to this project, workspace and snapshot
/// revision; its authority is a Git authority the proposal itself names; and
/// the proposal is present in that same snapshot, at the same kind, and
/// locally actionable.
///
/// The third check is a torn-read guard, not a second source of authority. The
/// capability list and the snapshot are two separate reads; a disagreement
/// means they straddled a change, and refusing the entry is the fail-closed
/// answer. It can never *add* an entry the server did not advertise, which is
/// what makes a withheld `commit` grant render as absence.
#[must_use]
pub fn advertised_staging_proposals(
    capabilities: Option<&ReviewCapabilities>,
    target: &PlatformReviewTarget,
    review: &PlatformReviewSemantic,
) -> Vec<AdvertisedReviewStaging> {
    let Some(capabilities) = capabilities else {
        return Vec::new();
    };
    if !exact_capability_attribution(capabilities, target, review) {
        return Vec::new();
    }
    capabilities
        .staging()
        .iter()
        .filter(|capability| capability.authority().kind() == ReviewAuthorityKind::Git)
        .filter(|capability| {
            review.proposals.iter().any(|proposal| {
                proposal.id == capability.proposal_id().as_str()
                    && proposal.kind == capability.kind()
                    && proposal.authority.as_ref().is_some_and(|authority| {
                        authority.kind == ReviewAuthorityKind::Git
                            && authority.id == capability.authority().id().as_str()
                    })
            }) && review.proposal_is_actionable(capability.proposal_id().as_str())
        })
        .map(|capability| AdvertisedReviewStaging {
            proposal_id: capability.proposal_id().as_str().to_owned(),
            kind: capability.kind(),
            expected_head_revision: capability.expected_head_revision().as_str().to_owned(),
            expected_index_digest: capability.expected_index_digest().as_str().to_owned(),
            authority_id: capability.authority().id().as_str().to_owned(),
        })
        .collect()
}

/// The exact advertised conflict-resolution set for one active snapshot.
///
/// The snapshot-side guard differs from the staging one because the domain
/// does: the proposal must be a `ResolveConflict` on a Git authority, the file
/// must be one the proposal names, and that file must still be unmerged in
/// this snapshot. A file git no longer reports as conflicted cannot be
/// collapsed to a side, whatever an older capability read said.
#[must_use]
pub fn advertised_conflict_resolutions(
    capabilities: Option<&ReviewCapabilities>,
    target: &PlatformReviewTarget,
    review: &PlatformReviewSemantic,
) -> Vec<AdvertisedReviewConflictResolution> {
    let Some(capabilities) = capabilities else {
        return Vec::new();
    };
    if !exact_capability_attribution(capabilities, target, review) {
        return Vec::new();
    }
    capabilities
        .conflict_resolutions()
        .iter()
        .filter(|capability| capability.authority().kind() == ReviewAuthorityKind::Git)
        .filter(|capability| {
            review.conflict_resolution_is_actionable(
                capability.proposal_id().as_str(),
                capability.file_id().as_str(),
            ) && review.proposals.iter().any(|proposal| {
                proposal.id == capability.proposal_id().as_str()
                    && proposal.authority.as_ref().is_some_and(|authority| {
                        authority.id == capability.authority().id().as_str()
                    })
            })
        })
        .map(|capability| AdvertisedReviewConflictResolution {
            proposal_id: capability.proposal_id().as_str().to_owned(),
            file_id: capability.file_id().as_str().to_owned(),
            resolution: capability.resolution(),
            expected_head_revision: capability.expected_head_revision().as_str().to_owned(),
            expected_index_digest: capability.expected_index_digest().as_str().to_owned(),
            authority_id: capability.authority().id().as_str().to_owned(),
        })
        .collect()
}

/// Decide whether any Git staging control may be rendered at all.
///
/// Three fences must hold, exactly like the batch delivery lane: a capability
/// response attributed to this exact snapshot, at least one entry inside it,
/// and a durable custody store able to record the at-most-once boundary before
/// dispatch. A missing fence makes the control absent, never optimistically
/// disabled.
///
/// This is the lane-level answer. It says a control *may* exist, never that
/// any particular one does: each proposal still needs its own advertised slot,
/// which is how a deployment withholding `commit` while granting `stage` and
/// `unstage` produces two controls and not three.
///
/// # Errors
///
/// Returns the first unmet fence.
pub const fn review_staging_control(
    exact_capability: bool,
    advertised: bool,
    custody_available: bool,
) -> Result<(), ReviewStagingWithheld> {
    if !exact_capability {
        Err(ReviewStagingWithheld::NoServerCapability)
    } else if !advertised {
        Err(ReviewStagingWithheld::NoStageableProposal)
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
    /// `Ok` only when an exact server capability, at least one advertised
    /// entry, and a durable custody lane all back a staging mutation.
    /// Otherwise it names the exact missing fence instead of a control.
    pub staging: Result<(), ReviewStagingWithheld>,
    advertised_staging: Vec<AdvertisedReviewStaging>,
    advertised_conflict_resolutions: Vec<AdvertisedReviewConflictResolution>,
    observation: Option<ReviewWorktreeObservation>,
}

impl ReviewWorktreeProjection {
    /// Project one review snapshot into its combined lanes.
    ///
    /// `custody_available` is the caller's durable at-most-once store; it is
    /// never inferred from the snapshot.
    /// `target` is optional because the lanes are not: a surface with no
    /// exactly reconciled target still renders the combined read, it simply
    /// has nothing a capability response could be attributed to, so every
    /// control is withheld for want of a server capability.
    #[must_use]
    pub fn new(
        review: &PlatformReviewSemantic,
        target: Option<&PlatformReviewTarget>,
        capabilities: Option<&ReviewCapabilities>,
        custody_available: bool,
    ) -> Self {
        // "Exact" is about attribution, not content: a capability response for
        // this project, workspace and snapshot revision was received. Two
        // empty git lists inside it is a different, more specific answer, and
        // the surface must be able to say which one it got.
        let exact_capability = target
            .zip(capabilities)
            .is_some_and(|(target, capabilities)| {
                exact_capability_attribution(capabilities, target, review)
            });
        let advertised_staging = target.map_or_else(Vec::new, |target| {
            advertised_staging_proposals(capabilities, target, review)
        });
        let advertised_conflict_resolutions = target.map_or_else(Vec::new, |target| {
            advertised_conflict_resolutions(capabilities, target, review)
        });
        let staging = review_staging_control(
            exact_capability,
            !advertised_staging.is_empty() || !advertised_conflict_resolutions.is_empty(),
            custody_available,
        );
        // Every git entry in one response shares one repository read, so the
        // first admitted entry carries the whole document's observation.
        let observation = advertised_staging
            .first()
            .map(|entry| ReviewWorktreeObservation {
                head_revision: entry.expected_head_revision.clone(),
                index_digest: entry.expected_index_digest.clone(),
            })
            .or_else(|| {
                advertised_conflict_resolutions
                    .first()
                    .map(|entry| ReviewWorktreeObservation {
                        head_revision: entry.expected_head_revision.clone(),
                        index_digest: entry.expected_index_digest.clone(),
                    })
            });
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
            advertised_staging,
            advertised_conflict_resolutions,
            observation,
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

    /// The advertised staging set, in the capability's own order.
    #[must_use]
    pub fn advertised_staging(&self) -> &[AdvertisedReviewStaging] {
        &self.advertised_staging
    }

    /// The advertised conflict-resolution set, in the capability's own order.
    #[must_use]
    pub fn advertised_conflict_resolutions(&self) -> &[AdvertisedReviewConflictResolution] {
        &self.advertised_conflict_resolutions
    }

    /// The `HEAD` and index the server read, when it advertised anything.
    #[must_use]
    pub const fn observation(&self) -> Option<&ReviewWorktreeObservation> {
        self.observation.as_ref()
    }

    /// Whether this exact proposal may carry a staging control.
    ///
    /// This is the whole of the withholding rule at the render site: a
    /// `Commit` proposal the server did not advertise answers `false` here and
    /// simply grows no button, while its `Stage` and `Unstage` siblings do.
    #[must_use]
    pub fn advertises_staging(&self, proposal_id: &str) -> bool {
        self.advertised_staging
            .iter()
            .any(|entry| entry.proposal_id == proposal_id)
    }

    /// Every admissible side for one conflicted file, in advertisement order.
    ///
    /// The server lists one entry per side it actually recorded, so this
    /// returns two for a file holding both and one for a delete/modify pair.
    #[must_use]
    pub fn advertised_sides(&self, proposal_id: &str, file_id: &str) -> Vec<ConflictResolution> {
        self.advertised_conflict_resolutions
            .iter()
            .filter(|entry| entry.proposal_id == proposal_id && entry.file_id == file_id)
            .map(|entry| entry.resolution)
            .collect()
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
        ReviewAction, ReviewAttentionSemantic, ReviewAuthority, ReviewAuthorityKind,
        ReviewAuthoritySemantic, ReviewConfirmationDigest, ReviewConflictResolutionCapability,
        ReviewDecision, ReviewField, ReviewFreshnessSemantic, ReviewFreshnessState,
        ReviewGitStagingCapabilities, ReviewHunkSemantic, ReviewIndexDigest, ReviewPreviewSemantic,
        ReviewProposalId, ReviewProposalSemantic, ReviewPullRequestCapabilities,
        ReviewReceiptCorrelationDigest, ReviewSchemaVersion, ReviewStagingCapability,
        ReviewStatusSemantic,
    };
    use crate::config::workspace_catalog::{
        PlatformContextRef, PlatformMappingReconciliation, PlatformV2Mapping,
    };
    use automonique_protocol::platform_v2::WorkContextTargetKind;
    use automonique_protocol::platform_v2_review::{ReviewAuthorityId, ReviewFileId};
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
        let projection = ReviewWorktreeProjection::new(&review, None, None, true);

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
        let projection = ReviewWorktreeProjection::new(&review, None, None, true);
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

    const OBSERVED_HEAD: &str = "1f0e4b8c2a6d9e3f7051c8b4a2d6e9f30517b2c4";
    const OTHER_HEAD: &str = "9a8b7c6d5e4f30219a8b7c6d5e4f30219a8b7c6d";

    fn index_digest(seed: char) -> ReviewIndexDigest {
        ReviewIndexDigest::new(seed.to_string().repeat(64)).expect("index digest")
    }

    /// Two distinct lowercase-hex digests per seed, so a test can assert that
    /// the confirmation and the correlation are carried separately rather than
    /// one being reused for both.
    fn digests(seed: char) -> (ReviewConfirmationDigest, ReviewReceiptCorrelationDigest) {
        let confirmation = format!("{}{}", seed.to_string().repeat(63), '0');
        let correlation = format!("{}{}", seed.to_string().repeat(63), '1');
        (
            ReviewConfirmationDigest::new(confirmation).expect("confirmation"),
            ReviewReceiptCorrelationDigest::new(correlation).expect("correlation"),
        )
    }

    fn git_authority() -> ReviewAuthority {
        ReviewAuthority::new(
            ReviewAuthorityKind::Git,
            ReviewAuthorityId::new("authority-1".to_owned()).expect("authority id"),
        )
    }

    fn staging_capability(
        proposal_id: &str,
        kind: ReviewProposalKind,
        head: &str,
        index: char,
        seed: char,
    ) -> ReviewStagingCapability {
        let (confirmation, correlation) = digests(seed);
        ReviewStagingCapability::new(
            ReviewProposalId::new(proposal_id.to_owned()).expect("proposal id"),
            kind,
            ReviewField::new(head.to_owned()).expect("head revision"),
            index_digest(index),
            git_authority(),
            confirmation,
            correlation,
        )
        .expect("staging capability")
    }

    fn conflict_capability(
        proposal_id: &str,
        file_id: &str,
        resolution: ConflictResolution,
        index: char,
        seed: char,
    ) -> ReviewConflictResolutionCapability {
        let (confirmation, correlation) = digests(seed);
        ReviewConflictResolutionCapability::new(
            ReviewProposalId::new(proposal_id.to_owned()).expect("proposal id"),
            ReviewFileId::new(file_id.to_owned()).expect("file id"),
            resolution,
            ReviewField::new(OBSERVED_HEAD.to_owned()).expect("head revision"),
            index_digest(index),
            git_authority(),
            confirmation,
            correlation,
        )
        .expect("conflict capability")
    }

    fn capabilities(
        target: &PlatformReviewTarget,
        review: &PlatformReviewSemantic,
        git: ReviewGitStagingCapabilities,
    ) -> ReviewCapabilities {
        ReviewCapabilities::new(
            target.project.clone(),
            target.workspace.clone(),
            review.revision,
            revision(91),
            Vec::new(),
            Vec::new(),
            ReviewPullRequestCapabilities::default(),
            git,
        )
        .expect("capability response")
    }

    /// One unstaged file with a `Stage` proposal, one with an `Unstage`, one
    /// with a `Commit`, and one unmerged file carrying a `ResolveConflict`
    /// proposal over both sides.
    fn staging_review() -> PlatformReviewSemantic {
        review_with(
            vec![
                text_file("file-1", WorktreeFileState::Unstaged, ConflictState::None),
                text_file("file-2", WorktreeFileState::Staged, ConflictState::None),
                text_file("file-3", WorktreeFileState::Staged, ConflictState::None),
                text_file(
                    "file-conflict",
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
                    id: "proposal-unstage".to_owned(),
                    kind: ReviewProposalKind::Unstage,
                    authority: Some(authority(ReviewAuthorityKind::Git)),
                    files: vec!["file-2".to_owned()],
                    subject: None,
                },
                ReviewProposalSemantic {
                    id: "proposal-commit".to_owned(),
                    kind: ReviewProposalKind::Commit,
                    authority: Some(authority(ReviewAuthorityKind::Git)),
                    files: vec!["file-3".to_owned()],
                    subject: Some("record the reviewed change".to_owned()),
                },
                ReviewProposalSemantic {
                    id: "proposal-conflict".to_owned(),
                    kind: ReviewProposalKind::ResolveConflict,
                    authority: Some(authority(ReviewAuthorityKind::Git)),
                    files: vec!["file-conflict".to_owned()],
                    subject: None,
                },
            ],
        )
    }

    // SDTEST-1867 — the withholding demonstration. A deployment that installs
    // `index_write` but not `commit` advertises `Stage` and `Unstage` and no
    // `Commit`. The commit control must be *absent*, not disabled: the three
    // kinds share one capability type but not one grant, so the only honest
    // rendering of a missing grant is a missing control. The two granted kinds
    // must go all the way through to a dispatchable confirmed preview in the
    // same run, or the test would prove nothing about the ones that are there.
    #[test]
    fn sdtest_1867_withheld_commit_grant_renders_no_control_while_its_siblings_go_end_to_end() {
        let review = staging_review();
        let target = target();
        let granted = capabilities(
            &target,
            &review,
            ReviewGitStagingCapabilities {
                staging: vec![
                    staging_capability(
                        "proposal-stage",
                        ReviewProposalKind::Stage,
                        OBSERVED_HEAD,
                        'a',
                        'c',
                    ),
                    staging_capability(
                        "proposal-unstage",
                        ReviewProposalKind::Unstage,
                        OBSERVED_HEAD,
                        'a',
                        'd',
                    ),
                ],
                conflict_resolutions: Vec::new(),
            },
        );

        let projection =
            ReviewWorktreeProjection::new(&review, Some(&target), Some(&granted), true);
        assert_eq!(projection.staging, Ok(()));
        assert_eq!(
            projection
                .advertised_staging()
                .iter()
                .map(|entry| (entry.proposal_id.as_str(), entry.kind))
                .collect::<Vec<_>>(),
            [
                ("proposal-stage", ReviewProposalKind::Stage),
                ("proposal-unstage", ReviewProposalKind::Unstage),
            ]
        );
        assert!(
            !projection.advertises_staging("proposal-commit"),
            "a grant the server withheld must produce no control at all"
        );
        // The commit proposal is still *observed* — the read side never lost
        // it — which is exactly what distinguishes absence-of-control from
        // absence-of-proposal.
        let staged = lane(&projection, ReviewWorktreeLane::Staged);
        assert!(staged.iter().any(|file| file
            .staging
            .iter()
            .any(|proposal| proposal.proposal_id == "proposal-commit" && proposal.admissible)));

        // The withheld kind refuses at the preview constructor too, so nothing
        // downstream can reconstruct it from the snapshot's own proposal.
        assert!(PlatformReviewActionPreview::stage_proposal(
            target.clone(),
            &review,
            &granted,
            "proposal-commit",
        )
        .is_err());

        // The two granted kinds go end to end: a confirmed preview carrying the
        // server's own digests verbatim, which the fresh capability document
        // still admits.
        for (proposal_id, seed) in [("proposal-stage", 'c'), ("proposal-unstage", 'd')] {
            let preview = PlatformReviewActionPreview::stage_proposal(
                target.clone(),
                &review,
                &granted,
                proposal_id,
            )
            .expect("granted staging proposal is dispatchable");
            let (confirmation, correlation) = digests(seed);
            let carried = preview.confirmation().expect("staging is a confirmed lane");
            assert_eq!(carried.confirmation_digest(), &confirmation);
            assert_eq!(carried.receipt_correlation_digest(), &correlation);
            assert_eq!(carried.expected_workspace_revision(), revision(91));
            assert!(preview.requires_capability_revalidation());
            assert!(preview.matches_capabilities(&granted));
        }

        // Grant the commit too and the third control appears, from the same
        // projection code and with no other change.
        let full = capabilities(
            &target,
            &review,
            ReviewGitStagingCapabilities {
                staging: vec![
                    staging_capability(
                        "proposal-stage",
                        ReviewProposalKind::Stage,
                        OBSERVED_HEAD,
                        'a',
                        'c',
                    ),
                    staging_capability(
                        "proposal-unstage",
                        ReviewProposalKind::Unstage,
                        OBSERVED_HEAD,
                        'a',
                        'd',
                    ),
                    staging_capability(
                        "proposal-commit",
                        ReviewProposalKind::Commit,
                        OBSERVED_HEAD,
                        'a',
                        'e',
                    ),
                ],
                conflict_resolutions: Vec::new(),
            },
        );
        let projection = ReviewWorktreeProjection::new(&review, Some(&target), Some(&full), true);
        assert!(projection.advertises_staging("proposal-commit"));
        assert!(PlatformReviewActionPreview::stage_proposal(
            target.clone(),
            &review,
            &full,
            "proposal-commit",
        )
        .is_ok());
    }

    // SDTEST-1868 — every other way the lane can be withheld, and the fence
    // that a worktree moving under the daemon has to trip.
    #[test]
    fn sdtest_1868_staging_lane_names_its_missing_fence_and_refuses_a_moved_worktree() {
        let review = staging_review();
        let target = target();
        let granted = capabilities(
            &target,
            &review,
            ReviewGitStagingCapabilities {
                staging: vec![staging_capability(
                    "proposal-stage",
                    ReviewProposalKind::Stage,
                    OBSERVED_HEAD,
                    'a',
                    'c',
                )],
                conflict_resolutions: Vec::new(),
            },
        );

        assert_eq!(
            ReviewWorktreeProjection::new(&review, Some(&target), None, true).staging,
            Err(ReviewStagingWithheld::NoServerCapability)
        );
        assert_eq!(
            ReviewWorktreeProjection::new(&review, None, Some(&granted), true).staging,
            Err(ReviewStagingWithheld::NoServerCapability),
            "no exactly reconciled target means nothing can be attributed"
        );
        // An exact response with two empty git lists is a different, more
        // specific answer than no response at all, and the surface must be
        // able to say which one it got.
        let empty = capabilities(&target, &review, ReviewGitStagingCapabilities::default());
        assert_eq!(
            ReviewWorktreeProjection::new(&review, Some(&target), Some(&empty), true).staging,
            Err(ReviewStagingWithheld::NoStageableProposal)
        );
        assert_eq!(
            ReviewWorktreeProjection::new(&review, Some(&target), Some(&granted), false).staging,
            Err(ReviewStagingWithheld::NoCustodyLane),
            "the durable lane is reported last, once the server has proved a slot"
        );
        // The lanes render in every one of those states; only the control is
        // withheld.
        for projection in [
            ReviewWorktreeProjection::new(&review, Some(&target), None, true),
            ReviewWorktreeProjection::new(&review, Some(&target), Some(&empty), true),
        ] {
            assert_eq!(projection.distinct_file_count(), 4);
            assert!(projection.advertised_staging().is_empty());
            assert!(projection.observation().is_none());
        }

        // The observation is exposed so a reader can see the fence.
        let projection =
            ReviewWorktreeProjection::new(&review, Some(&target), Some(&granted), true);
        let observation = projection
            .observation()
            .expect("granted lane names its read");
        assert_eq!(observation.head_revision, OBSERVED_HEAD);
        assert_eq!(observation.index_digest, "a".repeat(64));

        // A preview minted against one worktree read must stop matching once a
        // fresher document reports a different `HEAD` or a different index.
        // The server mints the digest over both, so the comparison is the
        // digest — and the refusal is local, before anything reaches the
        // daemon.
        let preview = PlatformReviewActionPreview::stage_proposal(
            target.clone(),
            &review,
            &granted,
            "proposal-stage",
        )
        .expect("advertised proposal");
        for moved in [
            staging_capability(
                "proposal-stage",
                ReviewProposalKind::Stage,
                OTHER_HEAD,
                'a',
                'f',
            ),
            staging_capability(
                "proposal-stage",
                ReviewProposalKind::Stage,
                OBSERVED_HEAD,
                'b',
                'f',
            ),
        ] {
            let refreshed = capabilities(
                &target,
                &review,
                ReviewGitStagingCapabilities {
                    staging: vec![moved],
                    conflict_resolutions: Vec::new(),
                },
            );
            assert!(
                !preview.matches_capabilities(&refreshed),
                "a worktree that moved must withdraw the control locally"
            );
        }

        // A capability whose kind disagrees with the snapshot's proposal is a
        // torn read between two server-side reads, and is refused rather than
        // letting the client pick which one to believe.
        let torn = capabilities(
            &target,
            &review,
            ReviewGitStagingCapabilities {
                staging: vec![staging_capability(
                    "proposal-stage",
                    ReviewProposalKind::Commit,
                    OBSERVED_HEAD,
                    'a',
                    'c',
                )],
                conflict_resolutions: Vec::new(),
            },
        );
        assert!(
            ReviewWorktreeProjection::new(&review, Some(&target), Some(&torn), true)
                .advertised_staging()
                .is_empty()
        );
    }

    // SDTEST-1869 — `conflict_resolutions` lists one entry per admissible
    // side, so a file with both sides recorded yields two controls and a
    // delete/modify pair yields one. The side is part of the identity, not a
    // parameter chosen after the fact: each entry carries the digest minted
    // over the blob that side would write.
    #[test]
    fn sdtest_1869_both_recorded_sides_render_two_controls_and_a_single_side_renders_one() {
        let review = staging_review();
        let target = target();
        let both = capabilities(
            &target,
            &review,
            ReviewGitStagingCapabilities {
                staging: Vec::new(),
                conflict_resolutions: vec![
                    conflict_capability(
                        "proposal-conflict",
                        "file-conflict",
                        ConflictResolution::KeepCurrent,
                        'a',
                        'c',
                    ),
                    conflict_capability(
                        "proposal-conflict",
                        "file-conflict",
                        ConflictResolution::KeepIncoming,
                        'a',
                        'd',
                    ),
                ],
            },
        );
        let projection = ReviewWorktreeProjection::new(&review, Some(&target), Some(&both), true);
        assert_eq!(projection.staging, Ok(()));
        assert_eq!(
            projection.advertised_sides("proposal-conflict", "file-conflict"),
            [
                ConflictResolution::KeepCurrent,
                ConflictResolution::KeepIncoming
            ],
            "one entry per admissible side means two buttons, not one with a choice"
        );

        for (resolution, seed) in [
            (ConflictResolution::KeepCurrent, 'c'),
            (ConflictResolution::KeepIncoming, 'd'),
        ] {
            let preview = PlatformReviewActionPreview::resolve_conflict(
                target.clone(),
                &review,
                &both,
                "proposal-conflict",
                "file-conflict",
                resolution,
            )
            .expect("advertised side");
            let (confirmation, correlation) = digests(seed);
            let carried = preview.confirmation().expect("confirmed lane");
            assert_eq!(
                carried.confirmation_digest(),
                &confirmation,
                "each side carries its own digest, so they can never be swapped"
            );
            assert_eq!(carried.receipt_correlation_digest(), &correlation);
            assert!(matches!(
                preview.action(),
                ReviewAction::ResolveConflict { resolution: sent, .. } if *sent == resolution
            ));
            assert!(preview.matches_capabilities(&both));
        }

        // A delete/modify conflict has one side recorded, so only that side is
        // ever offered and the other refuses locally.
        let one_side = capabilities(
            &target,
            &review,
            ReviewGitStagingCapabilities {
                staging: Vec::new(),
                conflict_resolutions: vec![conflict_capability(
                    "proposal-conflict",
                    "file-conflict",
                    ConflictResolution::KeepCurrent,
                    'a',
                    'c',
                )],
            },
        );
        let projection =
            ReviewWorktreeProjection::new(&review, Some(&target), Some(&one_side), true);
        assert_eq!(
            projection.advertised_sides("proposal-conflict", "file-conflict"),
            [ConflictResolution::KeepCurrent]
        );
        assert!(PlatformReviewActionPreview::resolve_conflict(
            target.clone(),
            &review,
            &one_side,
            "proposal-conflict",
            "file-conflict",
            ConflictResolution::KeepIncoming,
        )
        .is_err());

        // A file the snapshot no longer reports as unmerged cannot be
        // collapsed, whatever an older capability read said.
        let mut resolved = review.clone();
        resolved
            .files
            .iter_mut()
            .find(|file| file.id == "file-conflict")
            .expect("conflicted file")
            .conflict = ConflictState::Resolved;
        assert!(
            ReviewWorktreeProjection::new(&resolved, Some(&target), Some(&both), true)
                .advertised_conflict_resolutions()
                .is_empty()
        );
    }

    // SDTEST-1849 — the shipped canonical snapshot, not a hand-built one,
    // reaches the combined lanes, the safe text preview, and the staging
    // observation the surface renders.
    #[test]
    fn sdtest_1849_canonical_snapshot_projects_to_lanes_previews_and_observations() {
        let snapshot = decode_review_snapshot(CANONICAL_FIXTURE).expect("canonical fixture");
        let review = PlatformReviewSemantic::from(&snapshot);
        let projection = ReviewWorktreeProjection::new(&review, None, None, true);

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
