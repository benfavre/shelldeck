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
use std::path::{Path, PathBuf};
use uuid::Uuid;

use crate::config::workspace_catalog::{
    CatalogCheckoutId, CatalogWorkspaceId, WorkspaceRelativePath,
};
use crate::workspace_navigation::{
    PaneId, PaneNode, WorkspaceFocus, WorkspaceSurfaceState, WorkspaceTabContent,
};

const REVIEW_DRAFT_SCHEMA: u16 = 1;
const MAX_PREVIEW_BYTES: usize = 2 * 1024 * 1024;
const MAX_TEXT_PREVIEW_CHARS: usize = 250_000;
static REVIEW_DRAFT_SAVE_LOCK: parking_lot::Mutex<()> = parking_lot::Mutex::new(());

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
        anchor.review_revision == self.revision
            && self
                .changes
                .iter()
                .filter(|change| change.path == anchor.path)
                .flat_map(|change| &change.hunks)
                .flat_map(|hunk| &hunk.lines)
                .any(|line| match anchor.side {
                    ReviewLineSide::Old => line.old_line == Some(anchor.line),
                    ReviewLineSide::New => line.new_line == Some(anchor.line),
                })
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
        bytes: Vec<u8>,
    },
    Unsupported {
        category: &'static str,
    },
}

/// Classify bounded file content for an inert preview surface.
#[must_use]
pub fn safe_preview(path: &WorkspaceRelativePath, bytes: &[u8]) -> SafePreview {
    if bytes.len() > MAX_PREVIEW_BYTES {
        return SafePreview::Unsupported {
            category: "preview_too_large",
        };
    }
    let lower = path.as_str().to_ascii_lowercase();
    let image_mime = if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        Some("image/jpeg")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("image/gif")
    } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("image/webp")
    } else {
        None
    };
    if let Some(mime) = image_mime {
        return SafePreview::Image {
            mime,
            bytes: bytes.to_vec(),
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
    persisted_revision: u64,
    comments: Vec<ReviewCommentDraft>,
}

#[derive(Debug)]
pub enum ReviewDraftError {
    Io(std::io::Error),
    Json(serde_json::Error),
    UnsupportedSchema(u16),
    WrongWorkspace,
    RevisionConflict { expected: u64, actual: u64 },
    InvalidComment,
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
    #[must_use]
    pub const fn new(workspace: CatalogWorkspaceId) -> Self {
        Self {
            workspace,
            revision: 0,
            persisted_revision: 0,
            comments: Vec::new(),
        }
    }

    #[must_use]
    pub const fn revision(&self) -> u64 {
        self.revision
    }

    pub fn comments(&self) -> impl ExactSizeIterator<Item = &ReviewCommentDraft> {
        self.comments.iter()
    }

    pub fn add(&mut self, comment: ReviewCommentDraft) -> Result<(), ReviewDraftError> {
        if comment.author.trim().is_empty()
            || comment.body.trim().is_empty()
            || comment.anchor.line == 0
            || self.comments.iter().any(|item| item.id == comment.id)
        {
            return Err(if self.comments.iter().any(|item| item.id == comment.id) {
                ReviewDraftError::DuplicateComment(comment.id)
            } else {
                ReviewDraftError::InvalidComment
            });
        }
        self.comments.push(comment);
        self.revision = self.revision.saturating_add(1);
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
            self.revision = self.revision.saturating_add(1);
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

    pub fn save_to(&mut self, path: &Path) -> Result<(), ReviewDraftError> {
        let _process_guard = REVIEW_DRAFT_SAVE_LOCK.lock();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let lock_path = lock_path(path);
        let lock = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(lock_path)?;
        fs2::FileExt::lock_exclusive(&lock)?;
        let actual = read_disk_identity(path)?;
        if actual
            .workspace
            .is_some_and(|workspace| workspace != self.workspace)
        {
            return Err(ReviewDraftError::WrongWorkspace);
        }
        if actual.revision != self.persisted_revision {
            return Err(ReviewDraftError::RevisionConflict {
                expected: self.persisted_revision,
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
        crate::util::atomic_write(path, &serde_json::to_vec_pretty(&disk)?)?;
        self.persisted_revision = next;
        self.revision = self.revision.max(next);
        Ok(())
    }

    pub fn load_from(path: &Path) -> Result<Self, ReviewDraftError> {
        let disk: ReviewDraftDisk = serde_json::from_slice(&std::fs::read(path)?)?;
        if disk.schema_version != REVIEW_DRAFT_SCHEMA {
            return Err(ReviewDraftError::UnsupportedSchema(disk.schema_version));
        }
        validate_loaded_comments(&disk.comments)?;
        Ok(Self {
            workspace: disk.workspace,
            revision: disk.revision,
            persisted_revision: disk.revision,
            comments: disk.comments,
        })
    }
}

fn validate_loaded_comments(comments: &[ReviewCommentDraft]) -> Result<(), ReviewDraftError> {
    let mut ids = BTreeSet::new();
    for comment in comments {
        if !ids.insert(comment.id) {
            return Err(ReviewDraftError::DuplicateComment(comment.id));
        }
        if comment.author.trim().is_empty()
            || comment.body.trim().is_empty()
            || comment.anchor.line == 0
        {
            return Err(ReviewDraftError::InvalidComment);
        }
    }
    Ok(())
}

fn lock_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(".lock");
    PathBuf::from(name)
}

struct ReviewDraftDiskIdentity {
    revision: u64,
    workspace: Option<CatalogWorkspaceId>,
}

fn read_disk_identity(path: &Path) -> Result<ReviewDraftDiskIdentity, ReviewDraftError> {
    match std::fs::read(path) {
        Ok(bytes) => {
            let disk = serde_json::from_slice::<ReviewDraftDisk>(&bytes)?;
            if disk.schema_version != REVIEW_DRAFT_SCHEMA {
                return Err(ReviewDraftError::UnsupportedSchema(disk.schema_version));
            }
            Ok(ReviewDraftDiskIdentity {
                revision: disk.revision,
                workspace: Some(disk.workspace),
            })
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(ReviewDraftDiskIdentity {
            revision: 0,
            workspace: None,
        }),
        Err(error) => Err(error.into()),
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ActorIdentity {
    pub id: String,
    pub display_name: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AuthorityScope {
    Repository {
        checkout: CatalogCheckoutId,
        stage_hunks: bool,
    },
    ProviderSession {
        session_id: String,
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
    pub workspace: CatalogWorkspaceId,
    pub actor: ActorIdentity,
    pub revision: u64,
    pub expires_at_millis: u64,
    pub scope: AuthorityScope,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalDecision {
    Approve,
    Deny,
}

#[derive(Clone, Debug, Eq, PartialEq)]
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReviewMutationPreview {
    pub operation: ReviewMutationId,
    pub idempotency_key: Uuid,
    pub workspace: CatalogWorkspaceId,
    pub expected_review_revision: u64,
    pub authority_revision: u64,
    pub actor: ActorIdentity,
    pub kind: ReviewMutationKind,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReviewMutationReceipt {
    pub operation: ReviewMutationId,
    pub idempotency_key: Uuid,
    pub workspace: CatalogWorkspaceId,
    pub review_revision: u64,
    pub actor_id: String,
    pub outcome: MutationOutcome,
    pub recorded_at_millis: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MutationOutcome {
    Accepted,
    Completed,
    Rejected,
    Conflict,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PendingMutationState {
    Submitting,
    Reconciling { category: String },
    Completed(ReviewMutationReceipt),
    Refused { category: String },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PendingMutation {
    pub preview: ReviewMutationPreview,
    pub state: PendingMutationState,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MutationReceiptLookup {
    pub operation: ReviewMutationId,
    pub idempotency_key: Uuid,
    pub actor_id: String,
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
            }
        )
    }
}

impl std::error::Error for ReviewWorkflowError {}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ReviewWorkflow {
    pending: BTreeMap<ReviewMutationId, PendingMutation>,
}

impl ReviewWorkflow {
    pub fn prepare(
        &self,
        snapshot: &ReviewSnapshot,
        grant: &AuthorityGrant,
        kind: ReviewMutationKind,
        now_millis: u64,
    ) -> Result<ReviewMutationPreview, ReviewWorkflowError> {
        if snapshot.freshness != ObservationFreshness::Fresh {
            return Err(ReviewWorkflowError::StaleReview);
        }
        if grant.workspace != snapshot.workspace {
            return Err(ReviewWorkflowError::WrongWorkspace);
        }
        if grant.expires_at_millis <= now_millis {
            return Err(ReviewWorkflowError::ExpiredGrant);
        }
        if grant.actor.id.trim().is_empty() || grant.actor.display_name.trim().is_empty() {
            return Err(ReviewWorkflowError::InvalidActor);
        }
        validate_authority(snapshot, grant, &kind)?;
        Ok(ReviewMutationPreview {
            operation: ReviewMutationId::new(),
            idempotency_key: Uuid::new_v4(),
            workspace: snapshot.workspace,
            expected_review_revision: snapshot.revision,
            authority_revision: grant.revision,
            actor: grant.actor.clone(),
            kind,
        })
    }

    pub fn submit(&mut self, preview: ReviewMutationPreview) -> Result<(), ReviewWorkflowError> {
        if self.pending.contains_key(&preview.operation) {
            return Err(ReviewWorkflowError::MutationAlreadyPending);
        }
        self.pending.insert(
            preview.operation,
            PendingMutation {
                preview,
                state: PendingMutationState::Submitting,
            },
        );
        Ok(())
    }

    pub fn apply_transport_result(
        &mut self,
        operation: ReviewMutationId,
        result: MutationTransportResult,
    ) -> Result<Option<MutationReceiptLookup>, ReviewWorkflowError> {
        let pending = self
            .pending
            .get_mut(&operation)
            .ok_or(ReviewWorkflowError::UnknownMutation)?;
        if !matches!(pending.state, PendingMutationState::Submitting) {
            return Err(ReviewWorkflowError::MutationAlreadyPending);
        }
        match result {
            MutationTransportResult::Receipt(receipt) => {
                validate_receipt(&pending.preview, &receipt)?;
                pending.state = PendingMutationState::Completed(receipt);
                Ok(None)
            }
            MutationTransportResult::Refused { category } => {
                pending.state = PendingMutationState::Refused { category };
                Ok(None)
            }
            MutationTransportResult::Ambiguous { category } => {
                pending.state = PendingMutationState::Reconciling { category };
                Ok(Some(MutationReceiptLookup {
                    operation,
                    idempotency_key: pending.preview.idempotency_key,
                    actor_id: pending.preview.actor.id.clone(),
                }))
            }
        }
    }

    pub fn apply_reconciled_receipt(
        &mut self,
        receipt: ReviewMutationReceipt,
    ) -> Result<(), ReviewWorkflowError> {
        let pending = self
            .pending
            .get_mut(&receipt.operation)
            .ok_or(ReviewWorkflowError::UnknownMutation)?;
        if !matches!(pending.state, PendingMutationState::Reconciling { .. }) {
            return Err(ReviewWorkflowError::MutationAlreadyPending);
        }
        validate_receipt(&pending.preview, &receipt)?;
        pending.state = PendingMutationState::Completed(receipt);
        Ok(())
    }

    #[must_use]
    pub fn mutation(&self, operation: ReviewMutationId) -> Option<&PendingMutation> {
        self.pending.get(&operation)
    }
}

fn validate_authority(
    snapshot: &ReviewSnapshot,
    grant: &AuthorityGrant,
    kind: &ReviewMutationKind,
) -> Result<(), ReviewWorkflowError> {
    match (kind, &grant.scope) {
        (
            ReviewMutationKind::StageHunks { hunks },
            AuthorityScope::Repository {
                checkout,
                stage_hunks: true,
            },
        ) if *checkout == snapshot.checkout => {
            validate_hunks(snapshot, hunks, ChangeSection::Unstaged)
        }
        (
            ReviewMutationKind::UnstageHunks { hunks },
            AuthorityScope::Repository {
                checkout,
                stage_hunks: true,
            },
        ) if *checkout == snapshot.checkout => {
            validate_hunks(snapshot, hunks, ChangeSection::Staged)
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
        ) if session_id == granted => {
            if comments.is_empty() {
                return Err(ReviewWorkflowError::EmptyMutation);
            }
            if comments.iter().any(|comment| {
                !comment.selected
                    || !snapshot.contains_anchor(&comment.anchor)
                    || comment.body.trim().is_empty()
                    || comment.author != grant.actor.id
            }) {
                return Err(ReviewWorkflowError::InvalidSelection);
            }
            Ok(())
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
        ) if session_id == granted && !approval_id.trim().is_empty() => Ok(()),
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
        ) if provider == granted_provider
            && repository == granted_repository
            && !check_id.trim().is_empty() =>
        {
            Ok(())
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
        ) if provider == granted_provider
            && repository == granted_repository
            && !pull_request.trim().is_empty() =>
        {
            Ok(())
        }
        _ => Err(ReviewWorkflowError::WrongAuthority),
    }
}

fn validate_hunks(
    snapshot: &ReviewSnapshot,
    hunks: &[ReviewHunkId],
    section: ChangeSection,
) -> Result<(), ReviewWorkflowError> {
    if hunks.is_empty() {
        return Err(ReviewWorkflowError::EmptyMutation);
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
        || receipt.review_revision != preview.expected_review_revision
        || receipt.actor_id != preview.actor.id
    {
        return Err(ReviewWorkflowError::ReceiptMismatch);
    }
    Ok(())
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeliveryProjection {
    pub workspace: CatalogWorkspaceId,
    pub revision: u64,
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
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DeliveryBoard {
    projections: BTreeMap<CatalogWorkspaceId, DeliveryProjection>,
}

impl DeliveryBoard {
    pub fn apply(&mut self, projection: DeliveryProjection) -> Result<(), DeliveryProjectionError> {
        if let Some(current) = self.projections.get(&projection.workspace) {
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
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct AttentionBoard {
    items: BTreeMap<AttentionItemId, AttentionItem>,
}

impl AttentionBoard {
    pub fn apply(&mut self, item: AttentionItem) -> Result<bool, AttentionError> {
        if item.title.trim().is_empty()
            || item.agent_path.iter().any(|part| part.trim().is_empty())
            || item
                .target
                .session_id
                .as_ref()
                .is_some_and(|session| session.trim().is_empty())
        {
            return Err(AttentionError::InvalidItem);
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
        self.items.insert(item.id, item);
        Ok(notify)
    }

    pub fn open_target(
        &mut self,
        id: AttentionItemId,
        workspace: CatalogWorkspaceId,
        surface: &WorkspaceSurfaceState,
    ) -> Result<WorkspaceFocus, AttentionError> {
        let item = self.items.get_mut(&id).ok_or(AttentionError::InvalidItem)?;
        if item.target.workspace != workspace {
            return Err(AttentionError::WrongWorkspace);
        }
        let leaf = find_pane(surface.root.as_ref(), item.target.pane)
            .ok_or(AttentionError::UnknownPane)?;
        let tab = match item.target.session_id.as_ref() {
            Some(session_id) => leaf.tabs.iter().find(|tab| {
                matches!(
                    &tab.content,
                    WorkspaceTabContent::ProviderSession(binding)
                        if binding.session_id == *session_id
                )
            }),
            None => leaf
                .active_tab
                .and_then(|active| leaf.tabs.iter().find(|tab| tab.id == active)),
        }
        .ok_or(AttentionError::SessionOutsidePane)?;
        item.unread = false;
        Ok(WorkspaceFocus {
            pane_id: leaf.id,
            tab_id: tab.id,
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
mod tests {
    use super::*;
    use crate::workspace_navigation::{
        PaneLeaf, ProviderSessionBinding, WorkspaceTab, WorkspaceTabId,
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

    fn path(value: &str) -> WorkspaceRelativePath {
        WorkspaceRelativePath::new(value).unwrap()
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

    // SDTEST-1743
    #[test]
    fn sdtest_1743_combined_review_preserves_sections_conflicts_and_inert_previews() {
        let snapshot = snapshot();
        assert_eq!(
            snapshot.combined_sections(),
            BTreeSet::from([
                ChangeSection::Staged,
                ChangeSection::Unstaged,
                ChangeSection::Untracked,
            ])
        );
        assert_eq!(snapshot.changes[1].conflict, ChangeConflict::BothModified);
        assert!(matches!(
            safe_preview(&path("index.html"), b"<script>fetch('/token')</script>"),
            SafePreview::HtmlSource { escaped, .. }
                if escaped == "&lt;script&gt;fetch(&#39;/token&#39;)&lt;/script&gt;"
        ));
        assert!(matches!(
            safe_preview(&path("secret.bin"), &[0xff, 0, 1]),
            SafePreview::Unsupported { category: "binary" }
        ));
    }

    // SDTEST-1744
    #[test]
    fn sdtest_1744_line_comments_persist_and_batch_only_the_selected_exact_revision() {
        let root = std::env::temp_dir().join(format!(
            "shelldeck-review-drafts-{}-{}",
            std::process::id(),
            Uuid::new_v4()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let file = root.join("drafts.json");
        let mut store = ReviewDraftStore::new(workspace(1));
        for (id, revision, selected) in [(10, 9, true), (11, 8, true), (12, 9, false)] {
            store
                .add(ReviewCommentDraft {
                    id: ReviewCommentId::from_uuid(uuid(id)),
                    author: "actor-1".into(),
                    anchor: ReviewLineAnchor {
                        review_revision: revision,
                        path: path("src/main.rs"),
                        side: ReviewLineSide::New,
                        line: id as u32,
                    },
                    body: format!("note {id}"),
                    selected,
                })
                .unwrap();
        }
        store.save_to(&file).unwrap();
        let loaded = ReviewDraftStore::load_from(&file).unwrap();
        assert_eq!(loaded.selected_for_revision(9).len(), 1);
        assert_eq!(loaded.comments().count(), 3);
        std::fs::remove_dir_all(root).ok();
    }

    // SDTEST-1745
    #[test]
    fn sdtest_1745_provider_control_never_grants_repository_or_delivery_mutations() {
        let snapshot = snapshot();
        let workflow = ReviewWorkflow::default();
        let provider = AuthorityGrant {
            workspace: workspace(1),
            actor: actor(),
            revision: 4,
            expires_at_millis: 1_000,
            scope: AuthorityScope::ProviderSession {
                session_id: "session-1".into(),
                send_comments: true,
                decide_approval: true,
            },
        };
        assert_eq!(
            workflow.prepare(
                &snapshot,
                &provider,
                ReviewMutationKind::StageHunks {
                    hunks: vec![ReviewHunkId::from_uuid(uuid(3))],
                },
                10,
            ),
            Err(ReviewWorkflowError::WrongAuthority)
        );
        let comment = ReviewCommentDraft {
            id: ReviewCommentId::from_uuid(uuid(13)),
            author: "actor-1".into(),
            anchor: ReviewLineAnchor {
                review_revision: 9,
                path: path("src/main.rs"),
                side: ReviewLineSide::New,
                line: 1,
            },
            body: "Please rename this.".into(),
            selected: true,
        };
        assert!(workflow
            .prepare(
                &snapshot,
                &provider,
                ReviewMutationKind::SendComments {
                    session_id: "session-1".into(),
                    comments: vec![comment.clone()],
                },
                10,
            )
            .is_ok());
        let mut impersonated = comment;
        impersonated.author = "someone-else".into();
        assert_eq!(
            workflow.prepare(
                &snapshot,
                &provider,
                ReviewMutationKind::SendComments {
                    session_id: "session-1".into(),
                    comments: vec![impersonated],
                },
                10,
            ),
            Err(ReviewWorkflowError::InvalidSelection)
        );
        assert_eq!(
            workflow.prepare(
                &snapshot,
                &provider,
                ReviewMutationKind::MergePullRequest {
                    provider: "github".into(),
                    repository: "owner/repo".into(),
                    pull_request: "42".into(),
                },
                10,
            ),
            Err(ReviewWorkflowError::WrongAuthority)
        );
    }

    // SDTEST-1746
    #[test]
    fn sdtest_1746_ambiguous_mutation_reconciles_the_original_attributed_receipt_once() {
        let snapshot = snapshot();
        let grant = AuthorityGrant {
            workspace: workspace(1),
            actor: actor(),
            revision: 7,
            expires_at_millis: 1_000,
            scope: AuthorityScope::Repository {
                checkout: checkout(2),
                stage_hunks: true,
            },
        };
        let mut workflow = ReviewWorkflow::default();
        let preview = workflow
            .prepare(
                &snapshot,
                &grant,
                ReviewMutationKind::StageHunks {
                    hunks: vec![ReviewHunkId::from_uuid(uuid(3))],
                },
                10,
            )
            .unwrap();
        workflow.submit(preview.clone()).unwrap();
        let lookup = workflow
            .apply_transport_result(
                preview.operation,
                MutationTransportResult::Ambiguous {
                    category: "connection_reset".into(),
                },
            )
            .unwrap()
            .unwrap();
        assert_eq!(lookup.idempotency_key, preview.idempotency_key);
        assert_eq!(lookup.actor_id, "actor-1");
        let receipt = ReviewMutationReceipt {
            operation: preview.operation,
            idempotency_key: preview.idempotency_key,
            workspace: preview.workspace,
            review_revision: preview.expected_review_revision,
            actor_id: preview.actor.id.clone(),
            outcome: MutationOutcome::Completed,
            recorded_at_millis: 20,
        };
        workflow.apply_reconciled_receipt(receipt.clone()).unwrap();
        assert!(matches!(
            &workflow.mutation(preview.operation).unwrap().state,
            PendingMutationState::Completed(found) if found == &receipt
        ));
        assert_eq!(
            workflow.apply_reconciled_receipt(receipt),
            Err(ReviewWorkflowError::MutationAlreadyPending)
        );
    }

    // SDTEST-1747
    #[test]
    fn sdtest_1747_attention_deep_link_opens_only_its_exact_workspace_pane_and_session() {
        let pane = PaneId::from_uuid(uuid(20));
        let tab = WorkspaceTabId::from_uuid(uuid(21));
        let surface = WorkspaceSurfaceState {
            root: Some(PaneNode::Leaf(PaneLeaf {
                id: pane,
                tabs: vec![WorkspaceTab {
                    id: tab,
                    title: "Agent".into(),
                    content: WorkspaceTabContent::ProviderSession(ProviderSessionBinding {
                        platform_user_workspace_id: "platform-workspace".into(),
                        session_id: "session-1".into(),
                        run_id: Some("run-1".into()),
                    }),
                }],
                active_tab: Some(tab),
            })),
            focus: None,
        };
        let id = AttentionItemId::from_uuid(uuid(22));
        let mut board = AttentionBoard::default();
        assert!(board
            .apply(AttentionItem {
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
            })
            .unwrap());
        assert_eq!(
            board.open_target(id, workspace(2), &surface),
            Err(AttentionError::WrongWorkspace)
        );
        assert_eq!(
            board.open_target(id, workspace(1), &surface).unwrap(),
            WorkspaceFocus {
                pane_id: pane,
                tab_id: tab
            }
        );
    }

    // SDTEST-1748
    #[test]
    fn sdtest_1748_delivery_projection_refuses_stale_or_conflicting_authority_state() {
        let projection = DeliveryProjection {
            workspace: workspace(1),
            revision: 3,
            freshness: ObservationFreshness::Fresh,
            authority: DeliveryAuthority {
                provider: "github".into(),
                repository: "owner/repo".into(),
                observed_actor: Some(actor()),
                can_retry_checks: true,
                can_merge: false,
            },
            checks: vec![DeliveryCheck {
                id: "linux".into(),
                name: "Linux".into(),
                state: DeliveryCheckState::Passed,
            }],
            pull_request: Some(PullRequestProjection {
                key: "42".into(),
                review_status: "approved".into(),
                merge_ready: false,
            }),
            state: DeliveryState::ReviewRequired,
        };
        let mut board = DeliveryBoard::default();
        board.apply(projection.clone()).unwrap();
        let mut stale = projection.clone();
        stale.revision = 2;
        assert_eq!(
            board.apply(stale),
            Err(DeliveryProjectionError::StaleObservation)
        );
        let mut conflicting = projection;
        conflicting.state = DeliveryState::Ready;
        assert_eq!(
            board.apply(conflicting),
            Err(DeliveryProjectionError::ConflictingObservation)
        );
    }
}
