//! Review, approval, delivery, and agent-attention state for one user workspace.
//!
//! This module is deliberately a reducer/model boundary. Git, CI, pull-request,
//! and provider-session adapters must present explicit grants; observing or
//! controlling a provider session never creates repository or delivery
//! authority. Every mutation is tied to the review revision that was previewed
//! and ambiguous writes are reconciled by their original idempotency key.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::PathBuf;
use uuid::Uuid;

use crate::config::workspace_catalog::{
    CatalogCheckoutId, CatalogWorkspaceId, ProjectCatalog, WorkspaceRelativePath,
};
use crate::workspace_navigation::{
    PaneId, PaneNode, WorkspaceFocus, WorkspaceNavigationState, WorkspaceSurfaceState,
    WorkspaceTabContent,
};

#[path = "workspace_review_preview.rs"]
mod preview_image;
use preview_image::{looks_like_image, validated_image_metadata};
#[path = "workspace_review_validation.rs"]
mod validation;
use validation::{
    count_provider_session, validate_delivery_evidence, validate_fresh_review,
    validate_pending_record, validate_preview_bounds, validate_provider_evidence,
};
#[path = "workspace_review_storage.rs"]
mod storage;
use storage::{
    bounded_read, ensure_private_directory, lock_path, open_lock_file, read_disk_identity,
    secure_atomic_write, workflow_bounded_read, workflow_disk_revision, workspace_review_root,
    workspace_state_path,
};

const REVIEW_DRAFT_SCHEMA: u16 = 2;
const REVIEW_WORKFLOW_SCHEMA: u16 = 3;
const MAX_PREVIEW_BYTES: usize = 2 * 1024 * 1024;
const MAX_TEXT_PREVIEW_CHARS: usize = 250_000;
const MAX_DRAFT_FILE_BYTES: u64 = 8 * 1024 * 1024;
const MAX_WORKFLOW_FILE_BYTES: u64 = 8 * 1024 * 1024;
const MAX_DRAFT_COMMENTS: usize = 1_024;
const MAX_PENDING_MUTATIONS: usize = 1_024;
const MAX_ID_BYTES: usize = 256;
const MAX_PATH_BYTES: usize = 4_096;
const MAX_COMMENT_BYTES: usize = 64 * 1024;
const MAX_TITLE_BYTES: usize = 1_024;
const MAX_AGENT_PATH_PARTS: usize = 64;
const MAX_REVIEW_CHANGES: usize = 20_000;
const MAX_HUNKS_PER_FILE: usize = 10_000;
const MAX_LINES_PER_HUNK: usize = 100_000;
const MAX_MUTATION_ITEMS: usize = 1_024;
static REVIEW_DRAFT_SAVE_LOCK: parking_lot::Mutex<()> = parking_lot::Mutex::new(());
static REVIEW_WORKFLOW_SAVE_LOCK: parking_lot::Mutex<()> = parking_lot::Mutex::new(());

macro_rules! uuid_id {
    ($name:ident) => {
        #[derive(
            Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize,
        )]
        #[serde(transparent)]
        pub struct $name(Uuid);
        impl $name {
            #[must_use]
            pub fn new() -> Self {
                Self(Uuid::new_v4())
            }
            #[must_use]
            pub const fn from_uuid(value: Uuid) -> Self {
                Self(value)
            }
            #[must_use]
            pub const fn as_uuid(self) -> Uuid {
                self.0
            }
        }
        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }
        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(formatter)
            }
        }
    };
}

uuid_id!(ReviewHunkId);
uuid_id!(ReviewCommentId);
uuid_id!(ReviewMutationId);
uuid_id!(AttentionItemId);

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObservationFreshness {
    Fresh,
    Stale,
    Unknown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeConflict {
    None,
    BothAdded,
    BothModified,
    BothDeleted,
    AddedByUs,
    AddedByThem,
    DeletedByUs,
    DeletedByThem,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeSection {
    Staged,
    Unstaged,
    Untracked,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiffLineKind {
    Context,
    Added,
    Removed,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DiffLine {
    pub kind: DiffLineKind,
    pub old_line: Option<u32>,
    pub new_line: Option<u32>,
    pub text: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ReviewHunk {
    pub id: ReviewHunkId,
    pub header: String,
    #[serde(default)]
    pub lines: Vec<DiffLine>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ReviewFileChange {
    pub path: WorkspaceRelativePath,
    pub section: ChangeSection,
    pub conflict: ChangeConflict,
    #[serde(default)]
    pub hunks: Vec<ReviewHunk>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ReviewSnapshot {
    pub workspace: CatalogWorkspaceId,
    pub checkout: CatalogCheckoutId,
    pub revision: u64,
    pub observed_at_millis: u64,
    pub freshness: ObservationFreshness,
    #[serde(default)]
    pub changes: Vec<ReviewFileChange>,
}

impl ReviewSnapshot {
    #[must_use]
    pub fn combined_sections(&self) -> BTreeSet<ChangeSection> {
        self.changes.iter().map(|change| change.section).collect()
    }

    #[must_use]
    pub fn contains_hunk(&self, hunk: ReviewHunkId, section: ChangeSection) -> bool {
        self.changes
            .iter()
            .filter(|change| change.section == section)
            .any(|change| change.hunks.iter().any(|candidate| candidate.id == hunk))
    }

    #[must_use]
    pub fn contains_anchor(&self, anchor: &ReviewLineAnchor) -> bool {
        if anchor.review_revision != self.revision {
            return false;
        }
        self.changes
            .iter()
            .filter(|change| change.path == anchor.path && change.section == anchor.section)
            .flat_map(|change| &change.hunks)
            .filter(|hunk| hunk.id == anchor.hunk)
            .flat_map(|hunk| &hunk.lines)
            .filter(|line| match anchor.side {
                ReviewLineSide::Old => line.old_line == Some(anchor.line),
                ReviewLineSide::New => line.new_line == Some(anchor.line),
            })
            .take(2)
            .count()
            == 1
    }
}

impl Ord for ChangeSection {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        (*self as u8).cmp(&(*other as u8))
    }
}

impl PartialOrd for ChangeSection {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SafePreview {
    Text {
        text: String,
        truncated: bool,
    },
    /// HTML is displayed as inert escaped source. It is never loaded in a web
    /// view and therefore cannot run script, fetch remote content, or navigate.
    HtmlSource {
        escaped: String,
        truncated: bool,
    },
    Image {
        mime: &'static str,
        width: u32,
        height: u32,
        byte_len: usize,
    },
    Unsupported {
        category: &'static str,
    },
}

/// Classify bounded file content for an inert preview surface.
#[must_use]
pub fn safe_preview(path: &WorkspaceRelativePath, bytes: &[u8]) -> SafePreview {
    if path.as_str().len() > MAX_PATH_BYTES {
        return SafePreview::Unsupported {
            category: "preview_path_too_large",
        };
    }
    if bytes.len() > MAX_PREVIEW_BYTES {
        return SafePreview::Unsupported {
            category: "preview_too_large",
        };
    }
    let lower = path.as_str().to_ascii_lowercase();
    if let Some((mime, width, height)) = validated_image_metadata(bytes) {
        return SafePreview::Image {
            mime,
            width,
            height,
            byte_len: bytes.len(),
        };
    }
    if looks_like_image(bytes) {
        return SafePreview::Unsupported {
            category: "invalid_or_unsupported_image",
        };
    }
    let Ok(text) = std::str::from_utf8(bytes) else {
        return SafePreview::Unsupported { category: "binary" };
    };
    let (text, truncated) = truncate_chars(text, MAX_TEXT_PREVIEW_CHARS);
    if lower.ends_with(".html") || lower.ends_with(".htm") {
        SafePreview::HtmlSource {
            escaped: escape_html(text),
            truncated,
        }
    } else {
        SafePreview::Text {
            text: text.to_string(),
            truncated,
        }
    }
}

fn truncate_chars(value: &str, limit: usize) -> (&str, bool) {
    let Some((offset, _)) = value.char_indices().nth(limit) else {
        return (value, false);
    };
    (&value[..offset], true)
}

fn escape_html(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&#39;"),
            _ => escaped.push(character),
        }
    }
    escaped
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewLineSide {
    Old,
    New,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ReviewLineAnchor {
    pub review_revision: u64,
    pub path: WorkspaceRelativePath,
    pub section: ChangeSection,
    pub hunk: ReviewHunkId,
    pub side: ReviewLineSide,
    pub line: u32,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ReviewCommentDraft {
    pub id: ReviewCommentId,
    pub author: String,
    pub anchor: ReviewLineAnchor,
    pub body: String,
    #[serde(default)]
    pub selected: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
struct ReviewDraftDisk {
    schema_version: u16,
    revision: u64,
    workspace: CatalogWorkspaceId,
    comments: Vec<ReviewCommentDraft>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReviewDraftStore {
    workspace: CatalogWorkspaceId,
    revision: u64,
    comments: Vec<ReviewCommentDraft>,
    path: PathBuf,
    dirty: bool,
}

#[derive(Debug)]
pub enum ReviewDraftError {
    Io(std::io::Error),
    Json(serde_json::Error),
    UnsupportedSchema(u16),
    WrongWorkspace,
    RevisionConflict { expected: u64, actual: u64 },
    InvalidComment,
    BoundsExceeded(&'static str),
    DuplicateComment(ReviewCommentId),
    UnknownComment(ReviewCommentId),
}

impl fmt::Display for ReviewDraftError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => error.fmt(formatter),
            Self::Json(error) => error.fmt(formatter),
            Self::UnsupportedSchema(version) => {
                write!(formatter, "unsupported review draft schema {version}")
            }
            Self::WrongWorkspace => {
                formatter.write_str("review drafts belong to another workspace")
            }
            Self::RevisionConflict { expected, actual } => write!(
                formatter,
                "review draft revision conflict: expected {expected}, found {actual}"
            ),
            Self::InvalidComment => formatter.write_str("invalid review comment"),
            Self::BoundsExceeded(category) => {
                write!(formatter, "review draft exceeds {category} bound")
            }
            Self::DuplicateComment(id) => write!(formatter, "duplicate review comment {id}"),
            Self::UnknownComment(id) => write!(formatter, "unknown review comment {id}"),
        }
    }
}

impl std::error::Error for ReviewDraftError {}

impl From<std::io::Error> for ReviewDraftError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<serde_json::Error> for ReviewDraftError {
    fn from(value: serde_json::Error) -> Self {
        Self::Json(value)
    }
}

impl ReviewDraftStore {
    /// Load this workspace's drafts from ShellDeck's private state root.
    pub fn load(workspace: CatalogWorkspaceId) -> Result<Self, ReviewDraftError> {
        Self::load_at(workspace_review_root(), workspace)
    }

    #[must_use]
    pub const fn revision(&self) -> u64 {
        self.revision
    }

    #[must_use]
    pub const fn workspace(&self) -> CatalogWorkspaceId {
        self.workspace
    }

    #[must_use]
    pub const fn is_dirty(&self) -> bool {
        self.dirty
    }

    pub fn comments(&self) -> impl ExactSizeIterator<Item = &ReviewCommentDraft> {
        self.comments.iter()
    }

    pub fn add(&mut self, comment: ReviewCommentDraft) -> Result<(), ReviewDraftError> {
        validate_comment(&comment)?;
        if self.comments.len() >= MAX_DRAFT_COMMENTS {
            return Err(ReviewDraftError::BoundsExceeded("comment count"));
        }
        if self.comments.iter().any(|item| item.id == comment.id) {
            return Err(ReviewDraftError::DuplicateComment(comment.id));
        }
        self.comments.push(comment);
        self.dirty = true;
        Ok(())
    }

    pub fn select(&mut self, id: ReviewCommentId, selected: bool) -> Result<(), ReviewDraftError> {
        let comment = self
            .comments
            .iter_mut()
            .find(|comment| comment.id == id)
            .ok_or(ReviewDraftError::UnknownComment(id))?;
        if comment.selected != selected {
            comment.selected = selected;
            self.dirty = true;
        }
        Ok(())
    }

    pub fn selected_for_revision(&self, review_revision: u64) -> Vec<ReviewCommentDraft> {
        self.comments
            .iter()
            .filter(|comment| comment.selected && comment.anchor.review_revision == review_revision)
            .cloned()
            .collect()
    }

    pub fn save(&mut self) -> Result<(), ReviewDraftError> {
        if !self.dirty {
            return Ok(());
        }
        validate_loaded_comments(&self.comments)?;
        let _process_guard = REVIEW_DRAFT_SAVE_LOCK.lock();
        let parent = self.path.parent().ok_or_else(|| {
            ReviewDraftError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "review draft path has no parent",
            ))
        })?;
        ensure_private_directory(parent)?;
        let lock_path = lock_path(&self.path);
        let lock = open_lock_file(&lock_path)?;
        fs2::FileExt::lock_exclusive(&lock)?;
        let actual = read_disk_identity(&self.path)?;
        if actual
            .workspace
            .is_some_and(|workspace| workspace != self.workspace)
        {
            return Err(ReviewDraftError::WrongWorkspace);
        }
        if actual.revision != self.revision {
            return Err(ReviewDraftError::RevisionConflict {
                expected: self.revision,
                actual: actual.revision,
            });
        }
        let next = actual
            .revision
            .checked_add(1)
            .ok_or(ReviewDraftError::RevisionConflict {
                expected: actual.revision,
                actual: actual.revision,
            })?;
        let disk = ReviewDraftDisk {
            schema_version: REVIEW_DRAFT_SCHEMA,
            revision: next,
            workspace: self.workspace,
            comments: self.comments.clone(),
        };
        let payload = serde_json::to_vec_pretty(&disk)?;
        if payload.len() as u64 > MAX_DRAFT_FILE_BYTES {
            return Err(ReviewDraftError::BoundsExceeded("file size"));
        }
        secure_atomic_write(&self.path, &payload)?;
        self.revision = next;
        self.dirty = false;
        Ok(())
    }

    fn load_at(root: PathBuf, workspace: CatalogWorkspaceId) -> Result<Self, ReviewDraftError> {
        let path = workspace_state_path(&root, workspace, "drafts.json");
        let bytes = match bounded_read(&path, MAX_DRAFT_FILE_BYTES)? {
            Some(bytes) => bytes,
            None => {
                return Ok(Self {
                    workspace,
                    revision: 0,
                    comments: Vec::new(),
                    path,
                    dirty: false,
                })
            }
        };
        let disk: ReviewDraftDisk = serde_json::from_slice(&bytes)?;
        if disk.schema_version != REVIEW_DRAFT_SCHEMA {
            return Err(ReviewDraftError::UnsupportedSchema(disk.schema_version));
        }
        if disk.workspace != workspace {
            return Err(ReviewDraftError::WrongWorkspace);
        }
        validate_loaded_comments(&disk.comments)?;
        Ok(Self {
            workspace,
            revision: disk.revision,
            comments: disk.comments,
            path,
            dirty: false,
        })
    }
}

fn validate_loaded_comments(comments: &[ReviewCommentDraft]) -> Result<(), ReviewDraftError> {
    if comments.len() > MAX_DRAFT_COMMENTS {
        return Err(ReviewDraftError::BoundsExceeded("comment count"));
    }
    let mut ids = BTreeSet::new();
    for comment in comments {
        if !ids.insert(comment.id) {
            return Err(ReviewDraftError::DuplicateComment(comment.id));
        }
        validate_comment(comment)?;
    }
    Ok(())
}

fn validate_comment(comment: &ReviewCommentDraft) -> Result<(), ReviewDraftError> {
    if comment.id.as_uuid().is_nil()
        || !bounded_nonempty(&comment.author, MAX_ID_BYTES)
        || !bounded_nonempty(&comment.body, MAX_COMMENT_BYTES)
        || comment.anchor.hunk.as_uuid().is_nil()
        || comment.anchor.line == 0
        || comment.anchor.path.as_str().len() > MAX_PATH_BYTES
    {
        return Err(ReviewDraftError::InvalidComment);
    }
    Ok(())
}

fn bounded_nonempty(value: &str, maximum: usize) -> bool {
    !value.trim().is_empty() && value.len() <= maximum
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ActorIdentity {
    pub id: String,
    pub display_name: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
enum AuthorityScope {
    Repository {
        checkout: CatalogCheckoutId,
        stage_hunks: bool,
    },
    ProviderSession {
        session_id: String,
        platform_user_workspace_id: String,
        mapping_reconciliation_revision: u64,
        send_comments: bool,
        decide_approval: bool,
    },
    Delivery {
        provider: String,
        repository: String,
        retry_checks: bool,
        merge_pull_request: bool,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuthorityGrant {
    workspace: CatalogWorkspaceId,
    actor: ActorIdentity,
    revision: u64,
    expires_at_millis: u64,
    scope: AuthorityScope,
}

// These are intentionally crate-private minting seams for authenticated
// adapters. The UI crate cannot manufacture a grant; adapter wiring is a
// later integration milestone.
#[allow(dead_code, clippy::too_many_arguments)]
impl AuthorityGrant {
    pub(crate) fn repository(
        workspace: CatalogWorkspaceId,
        actor: ActorIdentity,
        revision: u64,
        expires_at_millis: u64,
        checkout: CatalogCheckoutId,
        stage_hunks: bool,
    ) -> Self {
        Self {
            workspace,
            actor,
            revision,
            expires_at_millis,
            scope: AuthorityScope::Repository {
                checkout,
                stage_hunks,
            },
        }
    }

    pub(crate) fn provider_session(
        workspace: CatalogWorkspaceId,
        actor: ActorIdentity,
        revision: u64,
        expires_at_millis: u64,
        session_id: String,
        platform_user_workspace_id: String,
        mapping_reconciliation_revision: u64,
        send_comments: bool,
        decide_approval: bool,
    ) -> Self {
        Self {
            workspace,
            actor,
            revision,
            expires_at_millis,
            scope: AuthorityScope::ProviderSession {
                session_id,
                platform_user_workspace_id,
                mapping_reconciliation_revision,
                send_comments,
                decide_approval,
            },
        }
    }

    pub(crate) fn delivery(
        workspace: CatalogWorkspaceId,
        actor: ActorIdentity,
        revision: u64,
        expires_at_millis: u64,
        provider: String,
        repository: String,
        retry_checks: bool,
        merge_pull_request: bool,
    ) -> Self {
        Self {
            workspace,
            actor,
            revision,
            expires_at_millis,
            scope: AuthorityScope::Delivery {
                provider,
                repository,
                retry_checks,
                merge_pull_request,
            },
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalDecision {
    Approve,
    Deny,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum ReviewMutationKind {
    StageHunks {
        hunks: Vec<ReviewHunkId>,
    },
    UnstageHunks {
        hunks: Vec<ReviewHunkId>,
    },
    SendComments {
        session_id: String,
        comments: Vec<ReviewCommentDraft>,
    },
    DecideApproval {
        session_id: String,
        approval_id: String,
        decision: ApprovalDecision,
    },
    RetryCheck {
        provider: String,
        repository: String,
        check_id: String,
    },
    MergePullRequest {
        provider: String,
        repository: String,
        pull_request: String,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MutationTargetFence {
    LocalReview {
        checkout: CatalogCheckoutId,
        review_revision: u64,
    },
    ReviewAndProviderSession {
        checkout: CatalogCheckoutId,
        review_revision: u64,
        platform_user_workspace_id: String,
        mapping_reconciliation_revision: u64,
        session_id: String,
        session_revision: u64,
    },
    ProviderApproval {
        platform_user_workspace_id: String,
        mapping_reconciliation_revision: u64,
        session_id: String,
        session_revision: u64,
        approval_id: String,
    },
    DeliveryCheck {
        provider: String,
        repository: String,
        delivery_revision: u64,
        check_id: String,
    },
    DeliveryPullRequest {
        provider: String,
        repository: String,
        delivery_revision: u64,
        pull_request: String,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ReviewMutationPreview {
    operation: ReviewMutationId,
    idempotency_key: Uuid,
    workspace: CatalogWorkspaceId,
    target: MutationTargetFence,
    authority_revision: u64,
    authority_expires_at_millis: u64,
    authority_scope: AuthorityScope,
    actor: ActorIdentity,
    kind: ReviewMutationKind,
}

impl ReviewMutationPreview {
    #[must_use]
    pub const fn operation(&self) -> ReviewMutationId {
        self.operation
    }
    #[must_use]
    pub const fn idempotency_key(&self) -> Uuid {
        self.idempotency_key
    }
    #[must_use]
    pub const fn workspace(&self) -> CatalogWorkspaceId {
        self.workspace
    }
    #[must_use]
    pub const fn target(&self) -> &MutationTargetFence {
        &self.target
    }
    #[must_use]
    pub const fn authority_revision(&self) -> u64 {
        self.authority_revision
    }
    #[must_use]
    pub const fn actor(&self) -> &ActorIdentity {
        &self.actor
    }
    #[must_use]
    pub const fn kind(&self) -> &ReviewMutationKind {
        &self.kind
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ReviewMutationReceipt {
    pub operation: ReviewMutationId,
    pub idempotency_key: Uuid,
    pub workspace: CatalogWorkspaceId,
    pub target: MutationTargetFence,
    pub authority_revision: u64,
    pub actor_id: String,
    pub outcome: MutationOutcome,
    pub recorded_at_millis: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum MutationOutcome {
    Accepted,
    Completed,
    Rejected,
    Conflict,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum PendingMutationState {
    Prepared,
    Submitting,
    Reconciling { category: String },
    Completed(ReviewMutationReceipt),
    Refused { category: String },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PendingMutation {
    preview: ReviewMutationPreview,
    state: PendingMutationState,
}

impl PendingMutation {
    #[must_use]
    pub const fn preview(&self) -> &ReviewMutationPreview {
        &self.preview
    }
    #[must_use]
    pub const fn state(&self) -> &PendingMutationState {
        &self.state
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MutationReceiptLookup {
    pub operation: ReviewMutationId,
    pub idempotency_key: Uuid,
    pub actor_id: String,
    pub workspace: CatalogWorkspaceId,
    pub target: MutationTargetFence,
    pub authority_revision: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MutationTransportResult {
    Receipt(ReviewMutationReceipt),
    Refused { category: String },
    Ambiguous { category: String },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReviewWorkflowError {
    StaleReview,
    WrongWorkspace,
    ExpiredGrant,
    InvalidActor,
    WrongAuthority,
    InvalidSelection,
    EmptyMutation,
    UnknownMutation,
    MutationAlreadyPending,
    ReceiptMismatch,
    CurrentTargetMismatch,
    BoundsExceeded(&'static str),
    Storage(String),
}

impl fmt::Display for ReviewWorkflowError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{}",
            match self {
                Self::StaleReview => "review snapshot is not fresh",
                Self::WrongWorkspace => "authority belongs to a different workspace",
                Self::ExpiredGrant => "authority grant expired",
                Self::InvalidActor => "authority actor is invalid",
                Self::WrongAuthority => "authority does not admit this mutation",
                Self::InvalidSelection => "mutation selection is outside the reviewed snapshot",
                Self::EmptyMutation => "mutation has no selected work",
                Self::UnknownMutation => "mutation is unknown",
                Self::MutationAlreadyPending => "mutation is already pending",
                Self::ReceiptMismatch => "mutation receipt does not match its preview",
                Self::CurrentTargetMismatch => "mutation target evidence changed after preview",
                Self::BoundsExceeded(category) => category,
                Self::Storage(category) => category,
            }
        )
    }
}

impl std::error::Error for ReviewWorkflowError {}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
struct ReviewWorkflowDisk {
    schema_version: u16,
    revision: u64,
    workspace: CatalogWorkspaceId,
    pending: Vec<PendingMutation>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReviewWorkflow {
    workspace: CatalogWorkspaceId,
    revision: u64,
    pending: BTreeMap<ReviewMutationId, PendingMutation>,
    path: PathBuf,
}

pub enum MutationTargetEvidence<'a> {
    LocalReview(&'a ReviewSnapshot),
    ReviewAndProviderSession {
        review: &'a ReviewSnapshot,
        session: &'a ProviderSessionProjection,
        catalog: &'a ProjectCatalog,
        surface: &'a WorkspaceSurfaceState,
    },
    ProviderSession(&'a ProviderSessionProjection),
    Delivery(&'a DeliveryProjection),
}

pub struct CurrentMutationEvidence<'a> {
    pub target: MutationTargetEvidence<'a>,
    pub grant: &'a AuthorityGrant,
}

impl ReviewWorkflow {
    /// Load the durable per-workspace mutation ledger. Any dispatch that was
    /// in flight at process exit is recovered as reconciliation-only work.
    pub fn load(workspace: CatalogWorkspaceId) -> Result<Self, ReviewWorkflowError> {
        Self::load_at(workspace_review_root(), workspace)
    }

    fn load_at(root: PathBuf, workspace: CatalogWorkspaceId) -> Result<Self, ReviewWorkflowError> {
        let path = workspace_state_path(&root, workspace, "mutation-ledger.json");
        let Some(bytes) = workflow_bounded_read(&path)? else {
            return Ok(Self {
                workspace,
                revision: 0,
                pending: BTreeMap::new(),
                path,
            });
        };
        let disk: ReviewWorkflowDisk = serde_json::from_slice(&bytes)
            .map_err(|error| ReviewWorkflowError::Storage(error.to_string()))?;
        if disk.schema_version != REVIEW_WORKFLOW_SCHEMA || disk.workspace != workspace {
            return Err(ReviewWorkflowError::Storage(
                "invalid mutation ledger identity or schema".into(),
            ));
        }
        if disk.pending.len() > MAX_PENDING_MUTATIONS {
            return Err(ReviewWorkflowError::BoundsExceeded(
                "mutation ledger item count exceeds its bound",
            ));
        }
        let mut pending = BTreeMap::new();
        let mut keys = BTreeSet::new();
        let mut recovered_dispatch = false;
        for mut mutation in disk.pending {
            validate_pending_record(&mutation)?;
            if !keys.insert(mutation.preview.idempotency_key)
                || pending
                    .insert(mutation.preview.operation, mutation.clone())
                    .is_some()
            {
                return Err(ReviewWorkflowError::Storage(
                    "duplicate mutation identity in ledger".into(),
                ));
            }
            if matches!(mutation.state, PendingMutationState::Submitting) {
                recovered_dispatch = true;
                mutation.state = PendingMutationState::Reconciling {
                    category: "process_restarted_after_dispatch".into(),
                };
                pending.insert(mutation.preview.operation, mutation);
            }
        }
        let mut workflow = Self {
            workspace,
            revision: disk.revision,
            pending,
            path,
        };
        if recovered_dispatch {
            workflow.persist()?;
        }
        Ok(workflow)
    }

    #[must_use]
    pub const fn revision(&self) -> u64 {
        self.revision
    }

    pub fn prepare(
        &mut self,
        evidence: MutationTargetEvidence<'_>,
        grant: &AuthorityGrant,
        kind: ReviewMutationKind,
        now_millis: u64,
    ) -> Result<ReviewMutationPreview, ReviewWorkflowError> {
        if grant.workspace != evidence_workspace(&evidence) || grant.workspace != self.workspace {
            return Err(ReviewWorkflowError::WrongWorkspace);
        }
        if grant.expires_at_millis <= now_millis {
            return Err(ReviewWorkflowError::ExpiredGrant);
        }
        if !bounded_nonempty(&grant.actor.id, MAX_ID_BYTES)
            || !bounded_nonempty(&grant.actor.display_name, MAX_TITLE_BYTES)
        {
            return Err(ReviewWorkflowError::InvalidActor);
        }
        let target = prepare_target(&evidence, grant, &kind)?;
        let preview = ReviewMutationPreview {
            operation: ReviewMutationId::new(),
            idempotency_key: Uuid::new_v4(),
            workspace: self.workspace,
            target,
            authority_revision: grant.revision,
            authority_expires_at_millis: grant.expires_at_millis,
            authority_scope: grant.scope.clone(),
            actor: grant.actor.clone(),
            kind,
        };
        validate_preview_bounds(&preview)?;
        if self.pending.len() >= MAX_PENDING_MUTATIONS {
            return Err(ReviewWorkflowError::BoundsExceeded(
                "mutation ledger item count exceeds its bound",
            ));
        }
        self.transact(|pending| {
            pending.insert(
                preview.operation,
                PendingMutation {
                    preview: preview.clone(),
                    state: PendingMutationState::Prepared,
                },
            );
            Ok(())
        })?;
        Ok(preview)
    }

    /// Durably marks an internally prepared operation as dispatched. Callers
    /// must not perform transport I/O unless this transition succeeds.
    pub fn submit(
        &mut self,
        preview: &ReviewMutationPreview,
        current: CurrentMutationEvidence<'_>,
        now_millis: u64,
    ) -> Result<(), ReviewWorkflowError> {
        let stored = self
            .pending
            .get(&preview.operation)
            .ok_or(ReviewWorkflowError::UnknownMutation)?;
        if stored.preview != *preview || !matches!(stored.state, PendingMutationState::Prepared) {
            return Err(ReviewWorkflowError::MutationAlreadyPending);
        }
        if preview.authority_expires_at_millis <= now_millis {
            return Err(ReviewWorkflowError::ExpiredGrant);
        }
        validate_current_target(preview, &current, now_millis)?;
        let operation = preview.operation;
        self.transact(|pending| {
            pending
                .get_mut(&operation)
                .ok_or(ReviewWorkflowError::UnknownMutation)?
                .state = PendingMutationState::Submitting;
            Ok(())
        })
    }

    pub fn apply_transport_result(
        &mut self,
        operation: ReviewMutationId,
        result: MutationTransportResult,
    ) -> Result<Option<MutationReceiptLookup>, ReviewWorkflowError> {
        let pending = self
            .pending
            .get(&operation)
            .ok_or(ReviewWorkflowError::UnknownMutation)?;
        if !matches!(pending.state, PendingMutationState::Submitting) {
            return Err(ReviewWorkflowError::MutationAlreadyPending);
        }
        let lookup = match &result {
            MutationTransportResult::Ambiguous { category }
                if bounded_nonempty(category, MAX_TITLE_BYTES) =>
            {
                Some(MutationReceiptLookup {
                    operation,
                    idempotency_key: pending.preview.idempotency_key,
                    actor_id: pending.preview.actor.id.clone(),
                    workspace: pending.preview.workspace,
                    target: pending.preview.target.clone(),
                    authority_revision: pending.preview.authority_revision,
                })
            }
            MutationTransportResult::Ambiguous { category }
            | MutationTransportResult::Refused { category }
                if !bounded_nonempty(category, MAX_TITLE_BYTES) =>
            {
                return Err(ReviewWorkflowError::BoundsExceeded(
                    "transport category exceeds its bound",
                ))
            }
            _ => None,
        };
        if let MutationTransportResult::Receipt(receipt) = &result {
            validate_receipt(&pending.preview, receipt)?;
        }
        self.transact(|pending| {
            let pending = pending
                .get_mut(&operation)
                .ok_or(ReviewWorkflowError::UnknownMutation)?;
            match result {
                MutationTransportResult::Receipt(receipt) => {
                    pending.state = PendingMutationState::Completed(receipt);
                }
                MutationTransportResult::Refused { category } => {
                    pending.state = PendingMutationState::Refused { category };
                }
                MutationTransportResult::Ambiguous { category } => {
                    pending.state = PendingMutationState::Reconciling { category };
                }
            }
            Ok(())
        })?;
        Ok(lookup)
    }

    pub fn apply_reconciled_receipt(
        &mut self,
        receipt: ReviewMutationReceipt,
    ) -> Result<(), ReviewWorkflowError> {
        let pending = self
            .pending
            .get(&receipt.operation)
            .ok_or(ReviewWorkflowError::UnknownMutation)?;
        if !matches!(pending.state, PendingMutationState::Reconciling { .. }) {
            return Err(ReviewWorkflowError::MutationAlreadyPending);
        }
        validate_receipt(&pending.preview, &receipt)?;
        let operation = receipt.operation;
        self.transact(|pending| {
            pending
                .get_mut(&operation)
                .ok_or(ReviewWorkflowError::UnknownMutation)?
                .state = PendingMutationState::Completed(receipt);
            Ok(())
        })
    }

    /// Record an authoritative receipt-lookup refusal/not-found result. This
    /// terminates reconciliation without ever dispatching the mutation again.
    pub fn apply_reconciliation_refusal(
        &mut self,
        operation: ReviewMutationId,
        category: String,
    ) -> Result<(), ReviewWorkflowError> {
        if !bounded_nonempty(&category, MAX_TITLE_BYTES) {
            return Err(ReviewWorkflowError::BoundsExceeded(
                "reconciliation category exceeds its bound",
            ));
        }
        let pending = self
            .pending
            .get(&operation)
            .ok_or(ReviewWorkflowError::UnknownMutation)?;
        if !matches!(pending.state, PendingMutationState::Reconciling { .. }) {
            return Err(ReviewWorkflowError::MutationAlreadyPending);
        }
        self.transact(|pending| {
            pending
                .get_mut(&operation)
                .ok_or(ReviewWorkflowError::UnknownMutation)?
                .state = PendingMutationState::Refused { category };
            Ok(())
        })
    }

    #[must_use]
    pub fn mutation(&self, operation: ReviewMutationId) -> Option<&PendingMutation> {
        self.pending.get(&operation)
    }

    /// Enumerate every durable operation after reload so a lost in-memory
    /// operation id cannot make prepared or terminal records unreachable.
    /// Records remain bounded by the ledger limit and are never auto-deleted.
    pub fn recoverable_mutations(&self) -> Vec<PendingMutation> {
        self.pending.values().cloned().collect()
    }

    /// Explicitly terminalize a prepared operation that was never dispatched.
    /// The resulting refusal still requires [`Self::acknowledge_terminal`]
    /// before its bounded ledger slot is released.
    pub fn abandon_prepared(
        &mut self,
        operation: ReviewMutationId,
    ) -> Result<(), ReviewWorkflowError> {
        let pending = self
            .pending
            .get(&operation)
            .ok_or(ReviewWorkflowError::UnknownMutation)?;
        if !matches!(pending.state, PendingMutationState::Prepared) {
            return Err(ReviewWorkflowError::MutationAlreadyPending);
        }
        self.transact(|pending| {
            pending
                .get_mut(&operation)
                .ok_or(ReviewWorkflowError::UnknownMutation)?
                .state = PendingMutationState::Refused {
                category: "abandoned_before_dispatch".into(),
            };
            Ok(())
        })
    }

    #[must_use]
    pub fn reconciliation_lookups(&self) -> Vec<MutationReceiptLookup> {
        self.pending
            .values()
            .filter(|pending| matches!(pending.state, PendingMutationState::Reconciling { .. }))
            .map(|pending| MutationReceiptLookup {
                operation: pending.preview.operation,
                idempotency_key: pending.preview.idempotency_key,
                actor_id: pending.preview.actor.id.clone(),
                workspace: pending.preview.workspace,
                target: pending.preview.target.clone(),
                authority_revision: pending.preview.authority_revision,
            })
            .collect()
    }

    /// Remove a terminal ledger entry after the UI has durably consumed its
    /// receipt/refusal. In-flight and prepared operations cannot be discarded.
    pub fn acknowledge_terminal(
        &mut self,
        operation: ReviewMutationId,
    ) -> Result<(), ReviewWorkflowError> {
        let pending = self
            .pending
            .get(&operation)
            .ok_or(ReviewWorkflowError::UnknownMutation)?;
        if !matches!(
            pending.state,
            PendingMutationState::Completed(_) | PendingMutationState::Refused { .. }
        ) {
            return Err(ReviewWorkflowError::MutationAlreadyPending);
        }
        self.transact(|pending| {
            pending.remove(&operation);
            Ok(())
        })
    }

    fn transact(
        &mut self,
        mutation: impl FnOnce(
            &mut BTreeMap<ReviewMutationId, PendingMutation>,
        ) -> Result<(), ReviewWorkflowError>,
    ) -> Result<(), ReviewWorkflowError> {
        let previous = self.pending.clone();
        mutation(&mut self.pending)?;
        if let Err(error) = self.persist() {
            self.pending = previous;
            return Err(error);
        }
        Ok(())
    }

    fn persist(&mut self) -> Result<(), ReviewWorkflowError> {
        let _process_guard = REVIEW_WORKFLOW_SAVE_LOCK.lock();
        let parent = self.path.parent().ok_or_else(|| {
            ReviewWorkflowError::Storage("mutation ledger path has no parent".into())
        })?;
        ensure_private_directory(parent)
            .map_err(|error| ReviewWorkflowError::Storage(error.to_string()))?;
        let lock = open_lock_file(&lock_path(&self.path))
            .map_err(|error| ReviewWorkflowError::Storage(error.to_string()))?;
        fs2::FileExt::lock_exclusive(&lock)
            .map_err(|error| ReviewWorkflowError::Storage(error.to_string()))?;
        let actual = workflow_disk_revision(&self.path, self.workspace)?;
        if actual != self.revision {
            return Err(ReviewWorkflowError::Storage(format!(
                "mutation ledger revision conflict: expected {}, found {actual}",
                self.revision
            )));
        }
        let next = actual.checked_add(1).ok_or_else(|| {
            ReviewWorkflowError::Storage("mutation ledger revision overflow".into())
        })?;
        let disk = ReviewWorkflowDisk {
            schema_version: REVIEW_WORKFLOW_SCHEMA,
            revision: next,
            workspace: self.workspace,
            pending: self.pending.values().cloned().collect(),
        };
        let payload = serde_json::to_vec_pretty(&disk)
            .map_err(|error| ReviewWorkflowError::Storage(error.to_string()))?;
        if payload.len() as u64 > MAX_WORKFLOW_FILE_BYTES {
            return Err(ReviewWorkflowError::BoundsExceeded(
                "mutation ledger file exceeds its bound",
            ));
        }
        secure_atomic_write(&self.path, &payload)
            .map_err(|error| ReviewWorkflowError::Storage(error.to_string()))?;
        self.revision = next;
        Ok(())
    }
}

fn evidence_workspace(evidence: &MutationTargetEvidence<'_>) -> CatalogWorkspaceId {
    match evidence {
        MutationTargetEvidence::LocalReview(review) => review.workspace,
        MutationTargetEvidence::ReviewAndProviderSession { review, .. } => review.workspace,
        MutationTargetEvidence::ProviderSession(session) => session.workspace,
        MutationTargetEvidence::Delivery(delivery) => delivery.workspace,
    }
}

fn prepare_target(
    evidence: &MutationTargetEvidence<'_>,
    grant: &AuthorityGrant,
    kind: &ReviewMutationKind,
) -> Result<MutationTargetFence, ReviewWorkflowError> {
    match (kind, &grant.scope, evidence) {
        (
            ReviewMutationKind::StageHunks { hunks },
            AuthorityScope::Repository {
                checkout,
                stage_hunks: true,
            },
            MutationTargetEvidence::LocalReview(snapshot),
        ) if *checkout == snapshot.checkout => {
            validate_fresh_review(snapshot)?;
            validate_hunks(snapshot, hunks, ChangeSection::Unstaged)?;
            Ok(MutationTargetFence::LocalReview {
                checkout: *checkout,
                review_revision: snapshot.revision,
            })
        }
        (
            ReviewMutationKind::UnstageHunks { hunks },
            AuthorityScope::Repository {
                checkout,
                stage_hunks: true,
            },
            MutationTargetEvidence::LocalReview(snapshot),
        ) if *checkout == snapshot.checkout => {
            validate_fresh_review(snapshot)?;
            validate_hunks(snapshot, hunks, ChangeSection::Staged)?;
            Ok(MutationTargetFence::LocalReview {
                checkout: *checkout,
                review_revision: snapshot.revision,
            })
        }
        (
            ReviewMutationKind::SendComments {
                session_id,
                comments,
            },
            AuthorityScope::ProviderSession {
                session_id: granted,
                send_comments: true,
                ..
            },
            MutationTargetEvidence::ReviewAndProviderSession {
                review,
                session,
                catalog,
                surface,
            },
        ) if session_id == granted && session_id == &session.session_id => {
            validate_fresh_review(review)?;
            validate_provider_evidence(session, grant, true, None)?;
            surface
                .validate_for(catalog, review.workspace)
                .map_err(|_| ReviewWorkflowError::CurrentTargetMismatch)?;
            let mapping = catalog
                .workspace(review.workspace)
                .ok()
                .and_then(|workspace| workspace.platform_mapping())
                .filter(|mapping| mapping.is_exact())
                .ok_or(ReviewWorkflowError::CurrentTargetMismatch)?;
            let platform_workspace = mapping.user_workspace.id.as_str();
            if session.platform_user_workspace_id != platform_workspace
                || session.mapping_reconciliation_revision != mapping.reconciliation_revision
            {
                return Err(ReviewWorkflowError::CurrentTargetMismatch);
            }
            if count_provider_session(
                surface.root.as_ref(),
                platform_workspace,
                &session.session_id,
            ) != 1
            {
                return Err(ReviewWorkflowError::CurrentTargetMismatch);
            }
            if comments.is_empty() {
                return Err(ReviewWorkflowError::EmptyMutation);
            }
            let mut comment_ids = BTreeSet::new();
            if comments.iter().any(|comment| {
                !comment_ids.insert(comment.id)
                    || !comment.selected
                    || !review.contains_anchor(&comment.anchor)
                    || comment.body.trim().is_empty()
                    || comment.author != grant.actor.id
            }) {
                return Err(ReviewWorkflowError::InvalidSelection);
            }
            if comments.len() > MAX_MUTATION_ITEMS {
                return Err(ReviewWorkflowError::BoundsExceeded(
                    "selected comment count exceeds its bound",
                ));
            }
            Ok(MutationTargetFence::ReviewAndProviderSession {
                checkout: review.checkout,
                review_revision: review.revision,
                platform_user_workspace_id: session.platform_user_workspace_id.clone(),
                mapping_reconciliation_revision: session.mapping_reconciliation_revision,
                session_id: session.session_id.clone(),
                session_revision: session.revision,
            })
        }
        (
            ReviewMutationKind::DecideApproval {
                session_id,
                approval_id,
                ..
            },
            AuthorityScope::ProviderSession {
                session_id: granted,
                decide_approval: true,
                ..
            },
            MutationTargetEvidence::ProviderSession(session),
        ) if session_id == granted && session_id == &session.session_id => {
            validate_provider_evidence(session, grant, false, Some(approval_id))?;
            Ok(MutationTargetFence::ProviderApproval {
                platform_user_workspace_id: session.platform_user_workspace_id.clone(),
                mapping_reconciliation_revision: session.mapping_reconciliation_revision,
                session_id: session.session_id.clone(),
                session_revision: session.revision,
                approval_id: approval_id.clone(),
            })
        }
        (
            ReviewMutationKind::RetryCheck {
                provider,
                repository,
                check_id,
            },
            AuthorityScope::Delivery {
                provider: granted_provider,
                repository: granted_repository,
                retry_checks: true,
                ..
            },
            MutationTargetEvidence::Delivery(delivery),
        ) if provider == granted_provider
            && repository == granted_repository
            && provider == &delivery.authority.provider
            && repository == &delivery.authority.repository =>
        {
            validate_delivery_evidence(delivery, grant)?;
            if !delivery.authority.can_retry_checks {
                return Err(ReviewWorkflowError::WrongAuthority);
            }
            let check = delivery
                .checks
                .iter()
                .find(|check| check.id == *check_id)
                .ok_or(ReviewWorkflowError::InvalidSelection)?;
            if !matches!(
                check.state,
                DeliveryCheckState::Failed | DeliveryCheckState::Cancelled
            ) {
                return Err(ReviewWorkflowError::InvalidSelection);
            }
            Ok(MutationTargetFence::DeliveryCheck {
                provider: provider.clone(),
                repository: repository.clone(),
                delivery_revision: delivery.revision,
                check_id: check_id.clone(),
            })
        }
        (
            ReviewMutationKind::MergePullRequest {
                provider,
                repository,
                pull_request,
            },
            AuthorityScope::Delivery {
                provider: granted_provider,
                repository: granted_repository,
                merge_pull_request: true,
                ..
            },
            MutationTargetEvidence::Delivery(delivery),
        ) if provider == granted_provider
            && repository == granted_repository
            && provider == &delivery.authority.provider
            && repository == &delivery.authority.repository =>
        {
            validate_delivery_evidence(delivery, grant)?;
            if !delivery.authority.can_merge {
                return Err(ReviewWorkflowError::WrongAuthority);
            }
            let pull = delivery
                .pull_request
                .as_ref()
                .filter(|pull| pull.key == *pull_request && pull.merge_ready)
                .ok_or(ReviewWorkflowError::InvalidSelection)?;
            Ok(MutationTargetFence::DeliveryPullRequest {
                provider: provider.clone(),
                repository: repository.clone(),
                delivery_revision: delivery.revision,
                pull_request: pull.key.clone(),
            })
        }
        _ => Err(ReviewWorkflowError::WrongAuthority),
    }
}

fn validate_current_target(
    preview: &ReviewMutationPreview,
    current: &CurrentMutationEvidence<'_>,
    now_millis: u64,
) -> Result<(), ReviewWorkflowError> {
    if evidence_workspace(&current.target) != preview.workspace
        || current.grant.workspace != preview.workspace
    {
        return Err(ReviewWorkflowError::WrongWorkspace);
    }
    if current.grant.expires_at_millis <= now_millis {
        return Err(ReviewWorkflowError::ExpiredGrant);
    }
    if current.grant.actor != preview.actor
        || current.grant.revision != preview.authority_revision
        || current.grant.expires_at_millis != preview.authority_expires_at_millis
        || current.grant.scope != preview.authority_scope
    {
        return Err(ReviewWorkflowError::CurrentTargetMismatch);
    }
    let current_target = prepare_target(&current.target, current.grant, &preview.kind)?;
    if current_target != preview.target {
        return Err(ReviewWorkflowError::CurrentTargetMismatch);
    }
    Ok(())
}

fn validate_hunks(
    snapshot: &ReviewSnapshot,
    hunks: &[ReviewHunkId],
    section: ChangeSection,
) -> Result<(), ReviewWorkflowError> {
    if hunks.is_empty() {
        return Err(ReviewWorkflowError::EmptyMutation);
    }
    if hunks.len() > MAX_MUTATION_ITEMS {
        return Err(ReviewWorkflowError::BoundsExceeded(
            "selected hunk count exceeds its bound",
        ));
    }
    let unique = hunks.iter().copied().collect::<BTreeSet<_>>();
    if unique.len() != hunks.len()
        || hunks
            .iter()
            .any(|hunk| !snapshot.contains_hunk(*hunk, section))
    {
        return Err(ReviewWorkflowError::InvalidSelection);
    }
    Ok(())
}

fn validate_receipt(
    preview: &ReviewMutationPreview,
    receipt: &ReviewMutationReceipt,
) -> Result<(), ReviewWorkflowError> {
    if receipt.operation != preview.operation
        || receipt.idempotency_key != preview.idempotency_key
        || receipt.workspace != preview.workspace
        || receipt.target != preview.target
        || receipt.authority_revision != preview.authority_revision
        || receipt.actor_id != preview.actor.id
    {
        return Err(ReviewWorkflowError::ReceiptMismatch);
    }
    Ok(())
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProviderSessionProjection {
    pub workspace: CatalogWorkspaceId,
    pub platform_user_workspace_id: String,
    pub mapping_reconciliation_revision: u64,
    pub session_id: String,
    pub revision: u64,
    pub authority_revision: u64,
    pub freshness: ObservationFreshness,
    pub observed_actor: Option<ActorIdentity>,
    pub can_send_comments: bool,
    pub can_decide_approval: bool,
    pub pending_approval_ids: BTreeSet<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeliveryProjection {
    pub workspace: CatalogWorkspaceId,
    pub revision: u64,
    pub authority_revision: u64,
    pub freshness: ObservationFreshness,
    pub authority: DeliveryAuthority,
    pub checks: Vec<DeliveryCheck>,
    pub pull_request: Option<PullRequestProjection>,
    pub state: DeliveryState,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeliveryAuthority {
    pub provider: String,
    pub repository: String,
    pub observed_actor: Option<ActorIdentity>,
    pub can_retry_checks: bool,
    pub can_merge: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeliveryCheckState {
    Queued,
    Running,
    Passed,
    Failed,
    Cancelled,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeliveryCheck {
    pub id: String,
    pub name: String,
    pub state: DeliveryCheckState,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PullRequestProjection {
    pub key: String,
    pub review_status: String,
    pub merge_ready: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeliveryState {
    LocalOnly,
    ChecksPending,
    ReviewRequired,
    Ready,
    Delivered,
    Blocked,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DeliveryProjectionError {
    WrongWorkspace,
    StaleObservation,
    ConflictingObservation,
    InvalidObservation,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DeliveryBoard {
    projections: BTreeMap<CatalogWorkspaceId, DeliveryProjection>,
}

impl DeliveryBoard {
    pub fn apply(&mut self, projection: DeliveryProjection) -> Result<(), DeliveryProjectionError> {
        if self.projections.len() >= MAX_PENDING_MUTATIONS
            && !self.projections.contains_key(&projection.workspace)
        {
            return Err(DeliveryProjectionError::InvalidObservation);
        }
        let mut check_ids = BTreeSet::new();
        if !bounded_nonempty(&projection.authority.provider, MAX_ID_BYTES)
            || !bounded_nonempty(&projection.authority.repository, MAX_PATH_BYTES)
            || projection
                .authority
                .observed_actor
                .as_ref()
                .is_some_and(|actor| {
                    !bounded_nonempty(&actor.id, MAX_ID_BYTES)
                        || !bounded_nonempty(&actor.display_name, MAX_TITLE_BYTES)
                })
            || projection.checks.len() > MAX_MUTATION_ITEMS
            || projection.checks.iter().any(|check| {
                !bounded_nonempty(&check.id, MAX_ID_BYTES)
                    || !bounded_nonempty(&check.name, MAX_TITLE_BYTES)
                    || !check_ids.insert(&check.id)
            })
            || projection.pull_request.as_ref().is_some_and(|pull| {
                !bounded_nonempty(&pull.key, MAX_ID_BYTES)
                    || !bounded_nonempty(&pull.review_status, MAX_TITLE_BYTES)
            })
        {
            return Err(DeliveryProjectionError::InvalidObservation);
        }
        if let Some(current) = self.projections.get(&projection.workspace) {
            if current.freshness == ObservationFreshness::Fresh
                && projection.freshness != ObservationFreshness::Fresh
            {
                return Err(DeliveryProjectionError::StaleObservation);
            }
            if projection.revision < current.revision {
                return Err(DeliveryProjectionError::StaleObservation);
            }
            if projection.revision == current.revision && projection != *current {
                return Err(DeliveryProjectionError::ConflictingObservation);
            }
        }
        self.projections.insert(projection.workspace, projection);
        Ok(())
    }

    #[must_use]
    pub fn get(&self, workspace: CatalogWorkspaceId) -> Option<&DeliveryProjection> {
        self.projections.get(&workspace)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttentionState {
    NeedsYou,
    Working,
    Blocked,
    Done,
    Idle,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AttentionTarget {
    pub workspace: CatalogWorkspaceId,
    pub pane: PaneId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AttentionItem {
    pub id: AttentionItemId,
    pub revision: u64,
    pub observed_at_millis: u64,
    pub target: AttentionTarget,
    pub state: AttentionState,
    pub title: String,
    #[serde(default)]
    pub unread: bool,
    #[serde(default)]
    pub agent_path: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AttentionError {
    InvalidItem,
    StaleObservation,
    ConflictingObservation,
    WrongWorkspace,
    UnknownPane,
    SessionOutsidePane,
    InvalidSurface,
    DuplicateSessionCoordinate,
    BoundsExceeded,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AttentionBoard {
    workspace: CatalogWorkspaceId,
    items: BTreeMap<AttentionItemId, AttentionItem>,
    locally_read: BTreeSet<(AttentionItemId, u64)>,
}

impl AttentionBoard {
    #[must_use]
    pub const fn new(workspace: CatalogWorkspaceId) -> Self {
        Self {
            workspace,
            items: BTreeMap::new(),
            locally_read: BTreeSet::new(),
        }
    }

    pub fn apply(&mut self, item: AttentionItem) -> Result<bool, AttentionError> {
        if item.target.workspace != self.workspace {
            return Err(AttentionError::WrongWorkspace);
        }
        if item.id.as_uuid().is_nil()
            || (self.items.len() >= MAX_PENDING_MUTATIONS && !self.items.contains_key(&item.id))
            || !bounded_nonempty(&item.title, MAX_TITLE_BYTES)
            || item.agent_path.len() > MAX_AGENT_PATH_PARTS
            || item
                .agent_path
                .iter()
                .any(|part| !bounded_nonempty(part, MAX_ID_BYTES))
            || item
                .target
                .session_id
                .as_ref()
                .is_some_and(|session| !bounded_nonempty(session, MAX_ID_BYTES))
        {
            return Err(AttentionError::BoundsExceeded);
        }
        let notify = match self.items.get(&item.id) {
            Some(current) if item.revision < current.revision => {
                return Err(AttentionError::StaleObservation)
            }
            Some(current) if item.revision == current.revision && item != *current => {
                return Err(AttentionError::ConflictingObservation)
            }
            Some(current) => {
                item.unread
                    && item.state != current.state
                    && matches!(
                        item.state,
                        AttentionState::NeedsYou | AttentionState::Blocked | AttentionState::Done
                    )
            }
            None => {
                item.unread
                    && matches!(
                        item.state,
                        AttentionState::NeedsYou | AttentionState::Blocked | AttentionState::Done
                    )
            }
        };
        if self
            .items
            .get(&item.id)
            .is_some_and(|current| current.revision < item.revision)
        {
            self.locally_read.retain(|(id, _)| *id != item.id);
        }
        self.items.insert(item.id, item);
        Ok(notify)
    }

    pub fn open_target(
        &mut self,
        id: AttentionItemId,
        workspace: CatalogWorkspaceId,
        catalog: &ProjectCatalog,
        navigation: &WorkspaceNavigationState,
    ) -> Result<WorkspaceFocus, AttentionError> {
        let item = self.items.get_mut(&id).ok_or(AttentionError::InvalidItem)?;
        if self.workspace != workspace || item.target.workspace != workspace {
            return Err(AttentionError::WrongWorkspace);
        }
        let surface = &navigation
            .workspace(workspace)
            .ok_or(AttentionError::InvalidSurface)?
            .surface;
        surface
            .validate_for(catalog, workspace)
            .map_err(|_| AttentionError::InvalidSurface)?;
        let leaf = find_pane(surface.root.as_ref(), item.target.pane)
            .ok_or(AttentionError::UnknownPane)?;
        let tab = match item.target.session_id.as_ref() {
            Some(session_id) => {
                let mut matches = leaf.tabs.iter().filter(|tab| {
                    matches!(
                        &tab.content,
                        WorkspaceTabContent::ProviderSession(binding)
                            if binding.session_id == *session_id
                    )
                });
                let tab = matches.next();
                if matches.next().is_some() {
                    return Err(AttentionError::DuplicateSessionCoordinate);
                }
                tab
            }
            None => leaf
                .active_tab
                .and_then(|active| leaf.tabs.iter().find(|tab| tab.id == active)),
        }
        .ok_or(AttentionError::SessionOutsidePane)?;
        self.locally_read.insert((id, item.revision));
        Ok(WorkspaceFocus {
            pane_id: leaf.id,
            tab_id: tab.id,
        })
    }

    #[must_use]
    pub fn is_unread(&self, id: AttentionItemId) -> bool {
        self.items.get(&id).is_some_and(|item| {
            item.unread && !self.locally_read.contains(&(item.id, item.revision))
        })
    }

    pub fn ordered(&self) -> Vec<&AttentionItem> {
        let mut items = self.items.values().collect::<Vec<_>>();
        items.sort_by_key(|item| (item.observed_at_millis, item.revision, item.id));
        items
    }
}

fn find_pane(
    root: Option<&PaneNode>,
    pane: PaneId,
) -> Option<&crate::workspace_navigation::PaneLeaf> {
    match root? {
        PaneNode::Leaf(leaf) => (leaf.id == pane).then_some(leaf),
        PaneNode::Split { first, second, .. } => {
            find_pane(Some(first), pane).or_else(|| find_pane(Some(second), pane))
        }
    }
}

#[cfg(test)]
#[path = "workspace_review_tests.rs"]
mod tests;
