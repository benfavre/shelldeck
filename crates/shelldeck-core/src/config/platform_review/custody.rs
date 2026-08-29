//! Durable at-most-once custody for native Platform v2 review mutations.
//!
//! A prepared preview is inert. Before any network dispatch it is advanced to
//! `dispatched` under the cross-process lock. Every restart treats dispatched
//! and accepted effects as lookup-only; ShellDeck never reconstructs a POST
//! decision from a provider session, chronology, or other observation.

use std::path::{Path, PathBuf};

use automonique_protocol::platform::ReceiptId;
use automonique_protocol::platform_v2_review::{
    ReviewActionId, ReviewActorId, ReviewFileId, ReviewHunkId, MAX_REVIEW_COMMENTS,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::*;
use crate::config::app_config::AppConfig;
use crate::workspace_review::storage::{
    bounded_descriptor_read, ensure_private_directory_io, lock_path, open_lock_file,
    secure_atomic_write,
};

const REVIEW_CUSTODY_SCHEMA: u16 = 1;
const MAX_REVIEW_CUSTODY_RECORDS: usize = 64;
const MAX_REVIEW_CUSTODY_FILE_BYTES: u64 = 1024 * 1024;
const MAX_REFUSAL_BYTES: usize = 16 * 1024;
static REVIEW_CUSTODY_PROCESS_LOCK: parking_lot::Mutex<()> = parking_lot::Mutex::new(());

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReviewCustodyPresentation {
    pub outcome: String,
    pub detail: String,
    pub actor: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReviewCustodyRecovery {
    NeverStarted(PlatformReviewActionPreview),
    LookupOnly(PlatformReviewActionPreview),
    Terminal(ReviewCustodyPresentation),
}

#[derive(Clone, Debug)]
pub struct PlatformReviewCustodyStore {
    path: PathBuf,
}

impl PlatformReviewCustodyStore {
    pub fn open_default() -> Result<Self, ReviewCustodyError> {
        Self::open(
            AppConfig::config_dir()
                .join("platform-review")
                .join("custody")
                .join("effects-v1.json"),
        )
    }

    pub fn open(path: PathBuf) -> Result<Self, ReviewCustodyError> {
        let store = Self { path };
        let _guard = REVIEW_CUSTODY_PROCESS_LOCK.lock();
        store.prepare_storage()?;
        let lock = open_lock_file(&lock_path(&store.path))?;
        fs2::FileExt::lock_exclusive(&lock)?;
        let _ = load_document(&store.path)?;
        Ok(store)
    }

    /// Persist one inert confirmation preview. A target may own only one
    /// non-terminal effect at a time; terminal presentation is replaced by a
    /// later explicit preview for that same exact target.
    pub fn prepare(&self, preview: &PlatformReviewActionPreview) -> Result<(), ReviewCustodyError> {
        self.transact(|document| {
            let key = preview.idempotency_key().as_str();
            if document.records.iter().any(|record| record.key == key) {
                return Err(ReviewCustodyError::DuplicateEffect);
            }
            if document.records.iter().any(|record| {
                record.matches_target(preview.target()) && !record.state.is_terminal()
            }) {
                return Err(ReviewCustodyError::EffectAlreadyPending);
            }
            document.records.retain(|record| {
                !record.matches_target(preview.target()) || !record.state.is_terminal()
            });
            if document.records.len() >= MAX_REVIEW_CUSTODY_RECORDS {
                return Err(ReviewCustodyError::CapacityExceeded);
            }
            document.records.push(ReviewCustodyRecord::from_preview(
                preview,
                ReviewCustodyState::Prepared,
            )?);
            Ok(())
        })
    }

    /// Establish the at-most-once boundary before the provider call begins.
    pub fn mark_dispatched(
        &self,
        preview: &PlatformReviewActionPreview,
    ) -> Result<(), ReviewCustodyError> {
        self.transact(|document| {
            let record = exact_record_mut(document, preview)?;
            match record.state {
                ReviewCustodyState::Prepared => {
                    record.state = ReviewCustodyState::Dispatched;
                    Ok(())
                }
                ReviewCustodyState::Dispatched | ReviewCustodyState::Receipt { .. } => Ok(()),
                ReviewCustodyState::Refused { .. } => Err(ReviewCustodyError::EffectTerminal),
            }
        })
    }

    pub fn record_receipt(
        &self,
        preview: &PlatformReviewActionPreview,
        receipt: &ReviewActionReceipt,
    ) -> Result<(), ReviewCustodyError> {
        if receipt.idempotency_key() != preview.idempotency_key() {
            return Err(ReviewCustodyError::ReceiptMismatch);
        }
        self.transact(|document| {
            let record = exact_record_mut(document, preview)?;
            if matches!(record.state, ReviewCustodyState::Prepared) {
                return Err(ReviewCustodyError::EffectNeverDispatched);
            }
            record.state = ReviewCustodyState::Receipt {
                receipt: ReviewReceiptDisk::from_receipt(receipt),
            };
            Ok(())
        })
    }

    pub fn record_refusal(
        &self,
        preview: &PlatformReviewActionPreview,
        category: &str,
        explanation: &str,
    ) -> Result<(), ReviewCustodyError> {
        if category.is_empty()
            || category.len() > MAX_REFUSAL_BYTES
            || explanation.len() > MAX_REFUSAL_BYTES
        {
            return Err(ReviewCustodyError::DocumentInvalid);
        }
        self.transact(|document| {
            let record = exact_record_mut(document, preview)?;
            if matches!(record.state, ReviewCustodyState::Prepared) {
                return Err(ReviewCustodyError::EffectNeverDispatched);
            }
            record.state = ReviewCustodyState::Refused {
                category: category.to_owned(),
                explanation: explanation.to_owned(),
            };
            Ok(())
        })
    }

    /// Cancel only a preview that provably never crossed the dispatch fence.
    pub fn cancel_prepared(
        &self,
        preview: &PlatformReviewActionPreview,
    ) -> Result<(), ReviewCustodyError> {
        self.transact(|document| {
            let index = exact_record_index(document, preview)?;
            if !matches!(document.records[index].state, ReviewCustodyState::Prepared) {
                return Err(ReviewCustodyError::EffectAlreadyPending);
            }
            document.records.remove(index);
            Ok(())
        })
    }

    pub fn recovery(
        &self,
        target: &PlatformReviewTarget,
    ) -> Result<Option<ReviewCustodyRecovery>, ReviewCustodyError> {
        let _guard = REVIEW_CUSTODY_PROCESS_LOCK.lock();
        self.prepare_storage()?;
        let lock = open_lock_file(&lock_path(&self.path))?;
        fs2::FileExt::lock_exclusive(&lock)?;
        let document = load_document(&self.path)?;
        let mut matching = document
            .records
            .iter()
            .filter(|record| record.matches_target(target));
        let Some(record) = matching.next() else {
            return Ok(None);
        };
        if matching.next().is_some() {
            return Err(ReviewCustodyError::DocumentInvalid);
        }
        let preview = record.to_preview()?;
        Ok(Some(match &record.state {
            ReviewCustodyState::Prepared => ReviewCustodyRecovery::NeverStarted(preview),
            ReviewCustodyState::Dispatched => ReviewCustodyRecovery::LookupOnly(preview),
            ReviewCustodyState::Receipt { receipt } if receipt.requires_lookup()? => {
                ReviewCustodyRecovery::LookupOnly(preview)
            }
            ReviewCustodyState::Receipt { receipt } => {
                ReviewCustodyRecovery::Terminal(receipt.presentation()?)
            }
            ReviewCustodyState::Refused {
                category,
                explanation,
            } => ReviewCustodyRecovery::Terminal(ReviewCustodyPresentation {
                outcome: category.clone(),
                detail: explanation.clone(),
                actor: None,
            }),
        }))
    }

    fn transact<T>(
        &self,
        update: impl FnOnce(&mut ReviewCustodyDocument) -> Result<T, ReviewCustodyError>,
    ) -> Result<T, ReviewCustodyError> {
        let _guard = REVIEW_CUSTODY_PROCESS_LOCK.lock();
        self.prepare_storage()?;
        let lock = open_lock_file(&lock_path(&self.path))?;
        fs2::FileExt::lock_exclusive(&lock)?;
        let mut document = load_document(&self.path)?;
        let outcome = update(&mut document)?;
        document.revision = document
            .revision
            .checked_add(1)
            .ok_or(ReviewCustodyError::DocumentInvalid)?;
        persist_document(&self.path, &document)?;
        Ok(outcome)
    }

    fn prepare_storage(&self) -> Result<(), ReviewCustodyError> {
        let parent = self.path.parent().ok_or(ReviewCustodyError::PathInvalid)?;
        ensure_private_directory_io(parent)?;
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReviewCustodyDocument {
    schema: u16,
    revision: u64,
    records: Vec<ReviewCustodyRecord>,
}

impl Default for ReviewCustodyDocument {
    fn default() -> Self {
        Self {
            schema: REVIEW_CUSTODY_SCHEMA,
            revision: 0,
            records: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReviewCustodyRecord {
    project: String,
    workspace_kind: String,
    workspace: String,
    expected_revision: u64,
    key: String,
    action: ReviewActionDisk,
    confirmation: Option<ReviewConfirmationDisk>,
    state: ReviewCustodyState,
}

impl ReviewCustodyRecord {
    fn from_preview(
        preview: &PlatformReviewActionPreview,
        state: ReviewCustodyState,
    ) -> Result<Self, ReviewCustodyError> {
        Ok(Self {
            project: preview.target.project.as_str().to_owned(),
            workspace_kind: preview.target.workspace.kind().as_str().to_owned(),
            workspace: preview.target.workspace.id().to_owned(),
            expected_revision: preview.expected_revision.get(),
            key: preview.idempotency_key.as_str().to_owned(),
            action: ReviewActionDisk::from_action(&preview.action)?,
            confirmation: preview
                .confirmation
                .as_ref()
                .map(ReviewConfirmationDisk::from_confirmation),
            state,
        })
    }

    fn matches_target(&self, target: &PlatformReviewTarget) -> bool {
        self.project == target.project.as_str()
            && self.workspace_kind == target.workspace.kind().as_str()
            && self.workspace == target.workspace.id()
    }

    fn to_preview(&self) -> Result<PlatformReviewActionPreview, ReviewCustodyError> {
        let project = ProjectId::new(self.project.clone())
            .map_err(|_| ReviewCustodyError::DocumentInvalid)?;
        let kind = WorkContextTargetKind::parse(&self.workspace_kind)
            .map_err(|_| ReviewCustodyError::DocumentInvalid)?;
        let workspace = WorkContextIdentity::parse_local(kind, &self.workspace)
            .map_err(|_| ReviewCustodyError::DocumentInvalid)?;
        if kind != WorkContextTargetKind::UserWorkspace {
            return Err(ReviewCustodyError::DocumentInvalid);
        }
        let expected_revision = Revision::new(self.expected_revision)
            .map_err(|_| ReviewCustodyError::DocumentInvalid)?;
        let action = self.action.to_action()?;
        let idempotency_key = IdempotencyKey::new(self.key.clone())
            .map_err(|_| ReviewCustodyError::DocumentInvalid)?;
        let confirmation = self
            .confirmation
            .as_ref()
            .map(ReviewConfirmationDisk::to_confirmation)
            .transpose()?;
        if matches!(action, ReviewAction::RerunCheck { .. }) != confirmation.is_some() {
            return Err(ReviewCustodyError::DocumentInvalid);
        }
        action
            .validate_client_shape()
            .map_err(|_| ReviewCustodyError::DocumentInvalid)?;
        Ok(PlatformReviewActionPreview {
            target: PlatformReviewTarget { project, workspace },
            expected_revision,
            action,
            idempotency_key,
            confirmation,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum ReviewActionDisk {
    AddComment {
        comment_id: String,
        file_id: String,
        hunk_id: String,
        side: String,
        line: u32,
        body: String,
    },
    ApproveReview {
        expected_review_revision: u64,
    },
    RerunCheck {
        check_id: String,
        expected_check_revision: u64,
    },
    /// The unconfirmed batch delivery lane.
    ///
    /// It carries no `confirmation` — the server advertises no digest for it —
    /// but it is still an exposed mutation, so it takes the same durable
    /// at-most-once record as every other one. Without it a restart between
    /// the POST and the receipt would leave a delivered batch indistinguishable
    /// from one that never left, and re-preparing would mint a fresh
    /// idempotency key.
    BatchSendCommentsToAgent {
        comments: Vec<ReviewCommentTargetDisk>,
    },
}

/// One comment target inside a persisted batch delivery.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReviewCommentTargetDisk {
    comment_id: String,
    expected_revision: u64,
}

impl ReviewActionDisk {
    fn from_action(action: &ReviewAction) -> Result<Self, ReviewCustodyError> {
        Ok(match action {
            ReviewAction::AddComment {
                comment_id,
                anchor,
                body,
            } => Self::AddComment {
                comment_id: comment_id.as_str().to_owned(),
                file_id: anchor.file_id().as_str().to_owned(),
                hunk_id: anchor.hunk_id().as_str().to_owned(),
                side: anchor.side().as_str().to_owned(),
                line: anchor.line(),
                body: body.as_str().to_owned(),
            },
            ReviewAction::ApproveReview {
                expected_review_revision,
            } => Self::ApproveReview {
                expected_review_revision: expected_review_revision.get(),
            },
            ReviewAction::RerunCheck {
                check_id,
                expected_check_revision,
            } => Self::RerunCheck {
                check_id: check_id.as_str().to_owned(),
                expected_check_revision: expected_check_revision.get(),
            },
            ReviewAction::BatchSendCommentsToAgent { comments } => Self::BatchSendCommentsToAgent {
                comments: comments
                    .iter()
                    .map(|comment| ReviewCommentTargetDisk {
                        comment_id: comment.comment_id().as_str().to_owned(),
                        expected_revision: comment.expected_revision().get(),
                    })
                    .collect(),
            },
            _ => return Err(ReviewCustodyError::UnsupportedAction),
        })
    }

    fn to_action(&self) -> Result<ReviewAction, ReviewCustodyError> {
        Ok(match self {
            Self::AddComment {
                comment_id,
                file_id,
                hunk_id,
                side,
                line,
                body,
            } => ReviewAction::AddComment {
                comment_id: ReviewCommentId::new(comment_id.clone())
                    .map_err(|_| ReviewCustodyError::DocumentInvalid)?,
                anchor: ReviewAnchor::new(
                    ReviewFileId::new(file_id.clone())
                        .map_err(|_| ReviewCustodyError::DocumentInvalid)?,
                    ReviewHunkId::new(hunk_id.clone())
                        .map_err(|_| ReviewCustodyError::DocumentInvalid)?,
                    DiffSide::parse(side).map_err(|_| ReviewCustodyError::DocumentInvalid)?,
                    *line,
                )
                .map_err(|_| ReviewCustodyError::DocumentInvalid)?,
                body: ReviewText::new(body.clone())
                    .map_err(|_| ReviewCustodyError::DocumentInvalid)?,
            },
            Self::ApproveReview {
                expected_review_revision,
            } => ReviewAction::ApproveReview {
                expected_review_revision: Revision::new(*expected_review_revision)
                    .map_err(|_| ReviewCustodyError::DocumentInvalid)?,
            },
            Self::RerunCheck {
                check_id,
                expected_check_revision,
            } => ReviewAction::RerunCheck {
                check_id: ReviewCheckId::new(check_id.clone())
                    .map_err(|_| ReviewCustodyError::DocumentInvalid)?,
                expected_check_revision: Revision::new(*expected_check_revision)
                    .map_err(|_| ReviewCustodyError::DocumentInvalid)?,
            },
            Self::BatchSendCommentsToAgent { comments } => {
                // Bound before allocating. `validate_client_shape` would also
                // reject an oversized batch, but only after building the whole
                // vector out of a document this process has not yet trusted.
                if comments.len() > MAX_REVIEW_COMMENTS {
                    return Err(ReviewCustodyError::DocumentInvalid);
                }
                ReviewAction::BatchSendCommentsToAgent {
                    comments: comments
                        .iter()
                        .map(|comment| {
                            Ok(ReviewCommentTarget::new(
                                ReviewCommentId::new(comment.comment_id.clone())
                                    .map_err(|_| ReviewCustodyError::DocumentInvalid)?,
                                Revision::new(comment.expected_revision)
                                    .map_err(|_| ReviewCustodyError::DocumentInvalid)?,
                            ))
                        })
                        .collect::<Result<Vec<_>, ReviewCustodyError>>()?,
                }
            }
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReviewConfirmationDisk {
    confirmation_digest: String,
    expected_workspace_revision: u64,
    receipt_correlation_digest: String,
}

impl ReviewConfirmationDisk {
    fn from_confirmation(confirmation: &ReviewActionConfirmation) -> Self {
        Self {
            confirmation_digest: confirmation.confirmation_digest().as_str().to_owned(),
            expected_workspace_revision: confirmation.expected_workspace_revision().get(),
            receipt_correlation_digest: confirmation
                .receipt_correlation_digest()
                .as_str()
                .to_owned(),
        }
    }

    fn to_confirmation(&self) -> Result<ReviewActionConfirmation, ReviewCustodyError> {
        Ok(ReviewActionConfirmation::new(
            ReviewConfirmationDigest::new(self.confirmation_digest.clone())
                .map_err(|_| ReviewCustodyError::DocumentInvalid)?,
            Revision::new(self.expected_workspace_revision)
                .map_err(|_| ReviewCustodyError::DocumentInvalid)?,
            ReviewReceiptCorrelationDigest::new(self.receipt_correlation_digest.clone())
                .map_err(|_| ReviewCustodyError::DocumentInvalid)?,
        ))
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
enum ReviewCustodyState {
    Prepared,
    Dispatched,
    Receipt {
        receipt: ReviewReceiptDisk,
    },
    Refused {
        category: String,
        explanation: String,
    },
}

impl ReviewCustodyState {
    fn is_terminal(&self) -> bool {
        match self {
            Self::Prepared | Self::Dispatched => false,
            Self::Receipt { receipt } => receipt.requires_lookup().map_or(true, |value| !value),
            Self::Refused { .. } => true,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReviewReceiptDisk {
    receipt_id: String,
    key: String,
    action_id: String,
    actor: String,
    outcome: String,
    revision: Option<u64>,
    current_revision: Option<u64>,
    reconciliation: String,
}

impl ReviewReceiptDisk {
    fn from_receipt(receipt: &ReviewActionReceipt) -> Self {
        Self {
            receipt_id: receipt.receipt_id().as_str().to_owned(),
            key: receipt.idempotency_key().as_str().to_owned(),
            action_id: receipt.action_id().as_str().to_owned(),
            actor: receipt.actor().as_str().to_owned(),
            outcome: receipt.outcome().as_str().to_owned(),
            revision: receipt.revision().map(Revision::get),
            current_revision: receipt.current_revision().map(Revision::get),
            reconciliation: receipt.reconciliation().as_str().to_owned(),
        }
    }

    fn to_receipt(&self) -> Result<ReviewActionReceipt, ReviewCustodyError> {
        ReviewActionReceipt::new(
            ReceiptId::new(self.receipt_id.clone())
                .map_err(|_| ReviewCustodyError::DocumentInvalid)?,
            IdempotencyKey::new(self.key.clone())
                .map_err(|_| ReviewCustodyError::DocumentInvalid)?,
            ReviewActionId::new(self.action_id.clone())
                .map_err(|_| ReviewCustodyError::DocumentInvalid)?,
            ReviewActorId::new(self.actor.clone())
                .map_err(|_| ReviewCustodyError::DocumentInvalid)?,
            ReviewReceiptOutcome::parse(&self.outcome)
                .map_err(|_| ReviewCustodyError::DocumentInvalid)?,
            self.revision
                .map(Revision::new)
                .transpose()
                .map_err(|_| ReviewCustodyError::DocumentInvalid)?,
            self.current_revision
                .map(Revision::new)
                .transpose()
                .map_err(|_| ReviewCustodyError::DocumentInvalid)?,
            ReviewReconciliation::parse(&self.reconciliation)
                .map_err(|_| ReviewCustodyError::DocumentInvalid)?,
        )
        .map_err(|_| ReviewCustodyError::DocumentInvalid)
    }

    fn requires_lookup(&self) -> Result<bool, ReviewCustodyError> {
        let receipt = self.to_receipt()?;
        Ok(
            receipt.reconciliation() == ReviewReconciliation::PollReceipt
                || matches!(
                    receipt.outcome(),
                    ReviewReceiptOutcome::Accepted | ReviewReceiptOutcome::Unknown
                ),
        )
    }

    fn presentation(&self) -> Result<ReviewCustodyPresentation, ReviewCustodyError> {
        let receipt = self.to_receipt()?;
        Ok(ReviewCustodyPresentation {
            outcome: receipt.outcome().as_str().to_owned(),
            detail: format!(
                "{} · {}",
                receipt.action_id().as_str(),
                receipt.reconciliation().as_str()
            ),
            actor: Some(receipt.actor().as_str().to_owned()),
        })
    }
}

fn exact_record_index(
    document: &ReviewCustodyDocument,
    preview: &PlatformReviewActionPreview,
) -> Result<usize, ReviewCustodyError> {
    let mut matching = document.records.iter().enumerate().filter(|(_, record)| {
        record.key == preview.idempotency_key().as_str() && record.matches_target(preview.target())
    });
    let Some((index, record)) = matching.next() else {
        return Err(ReviewCustodyError::UnknownEffect);
    };
    if matching.next().is_some() || record.to_preview()? != *preview {
        return Err(ReviewCustodyError::DocumentInvalid);
    }
    Ok(index)
}

fn exact_record_mut<'a>(
    document: &'a mut ReviewCustodyDocument,
    preview: &PlatformReviewActionPreview,
) -> Result<&'a mut ReviewCustodyRecord, ReviewCustodyError> {
    let index = exact_record_index(document, preview)?;
    Ok(&mut document.records[index])
}

fn load_document(path: &Path) -> Result<ReviewCustodyDocument, ReviewCustodyError> {
    let Some(bytes) = bounded_descriptor_read(path, MAX_REVIEW_CUSTODY_FILE_BYTES)? else {
        return Ok(ReviewCustodyDocument::default());
    };
    if bytes.len() as u64 > MAX_REVIEW_CUSTODY_FILE_BYTES {
        return Err(ReviewCustodyError::DocumentInvalid);
    }
    let document: ReviewCustodyDocument = serde_json::from_slice(&bytes)?;
    validate_document(&document)?;
    Ok(document)
}

fn validate_document(document: &ReviewCustodyDocument) -> Result<(), ReviewCustodyError> {
    if document.schema != REVIEW_CUSTODY_SCHEMA
        || document.records.len() > MAX_REVIEW_CUSTODY_RECORDS
    {
        return Err(ReviewCustodyError::DocumentInvalid);
    }
    let mut keys = std::collections::BTreeSet::new();
    let mut active_targets = std::collections::BTreeSet::new();
    for record in &document.records {
        let preview = record.to_preview()?;
        if !keys.insert(record.key.as_str()) {
            return Err(ReviewCustodyError::DocumentInvalid);
        }
        if !record.state.is_terminal()
            && !active_targets.insert((
                preview.target.project.as_str().to_owned(),
                preview.target.workspace.kind().as_str().to_owned(),
                preview.target.workspace.id().to_owned(),
            ))
        {
            return Err(ReviewCustodyError::DocumentInvalid);
        }
        if let ReviewCustodyState::Receipt { receipt } = &record.state {
            let value = receipt.to_receipt()?;
            if value.idempotency_key() != preview.idempotency_key() {
                return Err(ReviewCustodyError::DocumentInvalid);
            }
        }
        if let ReviewCustodyState::Refused {
            category,
            explanation,
        } = &record.state
        {
            if category.is_empty()
                || category.len() > MAX_REFUSAL_BYTES
                || explanation.len() > MAX_REFUSAL_BYTES
            {
                return Err(ReviewCustodyError::DocumentInvalid);
            }
        }
    }
    Ok(())
}

fn persist_document(
    path: &Path,
    document: &ReviewCustodyDocument,
) -> Result<(), ReviewCustodyError> {
    validate_document(document)?;
    let bytes = serde_json::to_vec_pretty(document)?;
    if bytes.len() as u64 > MAX_REVIEW_CUSTODY_FILE_BYTES {
        return Err(ReviewCustodyError::DocumentInvalid);
    }
    secure_atomic_write(path, &bytes)?;
    Ok(())
}

#[derive(Debug, Error)]
pub enum ReviewCustodyError {
    #[error("review custody storage failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("review custody document is not valid JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("review custody path is invalid")]
    PathInvalid,
    #[error("review custody document is invalid or exceeds its bound")]
    DocumentInvalid,
    #[error("review custody has reached its bounded capacity")]
    CapacityExceeded,
    #[error("review custody already contains this effect")]
    DuplicateEffect,
    #[error("another review effect is already pending for this workspace")]
    EffectAlreadyPending,
    #[error("review effect is not in custody")]
    UnknownEffect,
    #[error("review effect is already terminal")]
    EffectTerminal,
    #[error("review effect was never dispatched")]
    EffectNeverDispatched,
    #[error("review receipt does not match the exact effect")]
    ReceiptMismatch,
    #[error("review action is outside this custody slice")]
    UnsupportedAction,
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Barrier};

    use super::*;
    use automonique_protocol::platform_v2_review_api::decode_review_snapshot;

    const CANONICAL_FIXTURE: &[u8] =
        include_bytes!("../../../tests/fixtures/platform-v2-review-v2.json");

    fn semantic() -> PlatformReviewSemantic {
        PlatformReviewSemantic::from(&decode_review_snapshot(CANONICAL_FIXTURE).unwrap())
    }

    fn target(review: &PlatformReviewSemantic) -> PlatformReviewTarget {
        PlatformReviewTarget {
            project: ProjectId::new("project-1").unwrap(),
            workspace: WorkContextIdentity::parse_local(
                WorkContextTargetKind::UserWorkspace,
                &review.workspace_id,
            )
            .unwrap(),
        }
    }

    fn approval() -> PlatformReviewActionPreview {
        let review = semantic();
        PlatformReviewActionPreview::approve(target(&review), &review).unwrap()
    }

    fn comment(body: &str) -> PlatformReviewActionPreview {
        let review = semantic();
        let file = review
            .files
            .iter()
            .find(|file| !file.hunks.is_empty())
            .unwrap();
        let hunk = &file.hunks[0];
        let (side, line) = if hunk.new_lines > 0 {
            (DiffSide::New, hunk.new_start)
        } else {
            (DiffSide::Old, hunk.old_start)
        };
        let anchor = ReviewAnchorSemantic {
            file_id: file.id.clone(),
            hunk_id: hunk.id.clone(),
            side,
            line,
        };
        PlatformReviewActionPreview::add_comment(target(&review), &review, &anchor, body).unwrap()
    }

    struct TempRoot(PathBuf);

    impl TempRoot {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "shelldeck-review-custody-{}-{}",
                std::process::id(),
                uuid::Uuid::new_v4()
            ));
            std::fs::create_dir_all(&path).unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
            }
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempRoot {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn store_path(directory: &TempRoot) -> PathBuf {
        directory
            .path()
            .join("platform-review")
            .join("custody")
            .join("effects-v1.json")
    }

    fn receipt(
        preview: &PlatformReviewActionPreview,
        outcome: ReviewReceiptOutcome,
        actor: &str,
    ) -> ReviewActionReceipt {
        let (revision, reconciliation) = match outcome {
            ReviewReceiptOutcome::Accepted | ReviewReceiptOutcome::Unknown => {
                (None, ReviewReconciliation::PollReceipt)
            }
            ReviewReceiptOutcome::Completed => (
                Some(Revision::new(43).unwrap()),
                ReviewReconciliation::Final,
            ),
            ReviewReceiptOutcome::Refused | ReviewReceiptOutcome::Conflict => {
                (None, ReviewReconciliation::Final)
            }
        };
        ReviewActionReceipt::new(
            ReceiptId::new("receipt-1").unwrap(),
            preview.idempotency_key().clone(),
            ReviewActionId::new("action-1").unwrap(),
            ReviewActorId::new(actor).unwrap(),
            outcome,
            revision,
            (outcome == ReviewReceiptOutcome::Conflict).then(|| Revision::new(44).unwrap()),
            reconciliation,
        )
        .unwrap()
    }

    // SDTEST-1822 — an inert preview survives a process restart without ever
    // becoming provider work; only the durable dispatch fence enables lookup.
    #[test]
    fn prepared_restart_is_never_started_and_dispatched_restart_is_lookup_only() {
        let directory = TempRoot::new();
        let preview = approval();
        let store = PlatformReviewCustodyStore::open(store_path(&directory)).unwrap();
        store.prepare(&preview).unwrap();

        let restarted = PlatformReviewCustodyStore::open(store_path(&directory)).unwrap();
        assert_eq!(
            restarted.recovery(preview.target()).unwrap(),
            Some(ReviewCustodyRecovery::NeverStarted(preview.clone()))
        );

        restarted.mark_dispatched(&preview).unwrap();
        let restarted = PlatformReviewCustodyStore::open(store_path(&directory)).unwrap();
        assert_eq!(
            restarted.recovery(preview.target()).unwrap(),
            Some(ReviewCustodyRecovery::LookupOnly(preview))
        );
    }

    // SDTEST-1823 — Accepted remains durable across restart, retains its actor,
    // and advances to a terminal presentation only after the correlated poll.
    #[test]
    fn accepted_restart_then_completed_retains_actor_and_never_reopens_dispatch() {
        let directory = TempRoot::new();
        let preview = approval();
        let store = PlatformReviewCustodyStore::open(store_path(&directory)).unwrap();
        store.prepare(&preview).unwrap();
        store.mark_dispatched(&preview).unwrap();
        store
            .record_receipt(
                &preview,
                &receipt(&preview, ReviewReceiptOutcome::Accepted, "actor-1"),
            )
            .unwrap();

        let restarted = PlatformReviewCustodyStore::open(store_path(&directory)).unwrap();
        assert!(matches!(
            restarted.recovery(preview.target()).unwrap(),
            Some(ReviewCustodyRecovery::LookupOnly(value)) if value == preview
        ));
        restarted
            .record_receipt(
                &preview,
                &receipt(&preview, ReviewReceiptOutcome::Completed, "actor-1"),
            )
            .unwrap();
        assert_eq!(
            restarted.recovery(preview.target()).unwrap(),
            Some(ReviewCustodyRecovery::Terminal(ReviewCustodyPresentation {
                outcome: "completed".to_owned(),
                detail: "action-1 · final".to_owned(),
                actor: Some("actor-1".to_owned()),
            }))
        );
    }

    // SDTEST-1824 — separate store instances serialize through the OS lock, so
    // two processes cannot reserve two effects for the same workspace.
    #[test]
    fn concurrent_stores_reserve_only_one_effect_for_an_exact_target() {
        let directory = TempRoot::new();
        let path = store_path(&directory);
        PlatformReviewCustodyStore::open(path.clone()).unwrap();
        let first = approval();
        let second = approval();
        let barrier = Arc::new(Barrier::new(3));
        let handles = [first, second]
            .into_iter()
            .map(|preview| {
                let path = path.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    let store = PlatformReviewCustodyStore::open(path).unwrap();
                    barrier.wait();
                    store.prepare(&preview)
                })
            })
            .collect::<Vec<_>>();
        barrier.wait();
        let results = handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
        assert_eq!(
            results
                .iter()
                .filter(|result| matches!(result, Err(ReviewCustodyError::EffectAlreadyPending)))
                .count(),
            1
        );
    }

    // SDTEST-1825 — invalid and oversized documents fail closed before any
    // candidate state is admitted or rewritten.
    #[test]
    fn oversized_document_is_rejected_without_replacement() {
        let directory = TempRoot::new();
        let path = store_path(&directory);
        let store = PlatformReviewCustodyStore::open(path.clone()).unwrap();
        store.prepare(&approval()).unwrap();
        let oversized = vec![b'x'; MAX_REVIEW_CUSTODY_FILE_BYTES as usize + 1];
        std::fs::write(&path, &oversized).unwrap();
        assert!(matches!(
            PlatformReviewCustodyStore::open(path.clone()),
            Err(ReviewCustodyError::DocumentInvalid)
        ));
        assert_eq!(std::fs::read(path).unwrap(), oversized);
    }

    // SDTEST-1858 — the unconfirmed comment family crosses the same durable
    // fence as a confirmed rerun: restart never reposts it, its receipt keeps
    // the server actor, and a second comment cannot open a parallel effect.
    #[test]
    fn unconfirmed_comment_is_fenced_restart_safe_and_actor_attributed() {
        let directory = TempRoot::new();
        let preview = comment("borne exacte");
        assert!(preview.confirmation().is_none());
        let store = PlatformReviewCustodyStore::open(store_path(&directory)).unwrap();
        store.prepare(&preview).unwrap();

        // A second comment for the same workspace cannot reserve its own
        // effect while the first is still non-terminal.
        assert!(matches!(
            store.prepare(&comment("seconde borne")),
            Err(ReviewCustodyError::EffectAlreadyPending)
        ));

        store.mark_dispatched(&preview).unwrap();
        let restarted = PlatformReviewCustodyStore::open(store_path(&directory)).unwrap();
        assert_eq!(
            restarted.recovery(preview.target()).unwrap(),
            Some(ReviewCustodyRecovery::LookupOnly(preview.clone()))
        );
        restarted
            .record_receipt(
                &preview,
                &receipt(&preview, ReviewReceiptOutcome::Completed, "reviewer-7"),
            )
            .unwrap();
        assert_eq!(
            PlatformReviewCustodyStore::open(store_path(&directory))
                .unwrap()
                .recovery(preview.target())
                .unwrap(),
            Some(ReviewCustodyRecovery::Terminal(ReviewCustodyPresentation {
                outcome: "completed".to_owned(),
                detail: "action-1 · final".to_owned(),
                actor: Some("reviewer-7".to_owned()),
            }))
        );
    }

    /// The advertisement a coherent server would mint for this snapshot's one
    /// sendable comment, and the preview built verbatim from it.
    fn batch_delivery() -> PlatformReviewActionPreview {
        let mut review = semantic();
        review.comments[0].agent_state = CommentAgentState::NotSent;
        let target = target(&review);
        let capabilities = ReviewCapabilities::new(
            target.project.clone(),
            target.workspace.clone(),
            review.revision,
            Revision::new(91).unwrap(),
            Vec::new(),
            vec![ReviewAgentDeliveryCapability::new(
                ReviewCommentId::new(review.comments[0].id.clone()).unwrap(),
                review.comments[0].revision,
                ReviewAuthority::new(
                    ReviewAuthorityKind::Review,
                    automonique_protocol::platform_v2_review::ReviewAuthorityId::new(
                        review.review.authority.id.clone(),
                    )
                    .unwrap(),
                ),
            )
            .unwrap()],
            ReviewPullRequestCapabilities::default(),
        )
        .unwrap();
        PlatformReviewActionPreview::batch_send_comments(
            target,
            &review,
            &capabilities,
            &[review.comments[0].id.clone()],
        )
        .unwrap()
    }

    // SDTEST-1865 — the unconfirmed batch delivery takes the same durable
    // at-most-once lane as every other exposed mutation, and survives the
    // round trip through disk with its exact comment set intact.
    #[test]
    fn unconfirmed_batch_delivery_is_fenced_and_survives_the_disk_round_trip() {
        let directory = TempRoot::new();
        let preview = batch_delivery();
        // No confirmation digest: this lane is fenced by the advertisement and
        // the domain state machine, never by a digest.
        assert!(preview.confirmation().is_none());
        // It must still be revalidated before dispatch, exactly like a
        // confirmed rerun and unlike a comment or an approval.
        assert!(preview.requires_capability_revalidation());

        // Before this slice the disk format knew only AddComment,
        // ApproveReview and RerunCheck, so a batch delivery could not be
        // recorded at all and would have refused at the fence.
        let store = PlatformReviewCustodyStore::open(store_path(&directory)).unwrap();
        store.prepare(&preview).unwrap();
        assert!(matches!(
            store.prepare(&batch_delivery()),
            Err(ReviewCustodyError::EffectAlreadyPending)
        ));

        // A restart before dispatch reports the preview as never started, and
        // the exact action survives serialization: same comments, same
        // revisions, same order.
        let restarted = PlatformReviewCustodyStore::open(store_path(&directory)).unwrap();
        assert_eq!(
            restarted.recovery(preview.target()).unwrap(),
            Some(ReviewCustodyRecovery::NeverStarted(preview.clone()))
        );

        restarted.mark_dispatched(&preview).unwrap();
        assert_eq!(
            PlatformReviewCustodyStore::open(store_path(&directory))
                .unwrap()
                .recovery(preview.target())
                .unwrap(),
            Some(ReviewCustodyRecovery::LookupOnly(preview.clone())),
            "a restart mid-delivery must look the receipt up, never re-post"
        );

        let reopened = PlatformReviewCustodyStore::open(store_path(&directory)).unwrap();
        reopened
            .record_receipt(
                &preview,
                &receipt(&preview, ReviewReceiptOutcome::Completed, "reviewer-9"),
            )
            .unwrap();
        assert_eq!(
            PlatformReviewCustodyStore::open(store_path(&directory))
                .unwrap()
                .recovery(preview.target())
                .unwrap(),
            Some(ReviewCustodyRecovery::Terminal(ReviewCustodyPresentation {
                outcome: "completed".to_owned(),
                detail: "action-1 · final".to_owned(),
                actor: Some("reviewer-9".to_owned()),
            }))
        );
    }

    #[cfg(unix)]
    // SDTEST-1826 — linked/replaced custody paths and lock files are never
    // followed outside ShellDeck's private directory boundary.
    #[test]
    fn linked_custody_path_is_rejected() {
        use std::os::unix::fs::symlink;

        let directory = TempRoot::new();
        let path = store_path(&directory);
        let parent = path.parent().unwrap();
        std::fs::create_dir_all(parent).unwrap();
        let outside = directory.path().join("outside.json");
        std::fs::write(&outside, b"outside").unwrap();
        symlink(&outside, &path).unwrap();
        assert!(PlatformReviewCustodyStore::open(path).is_err());
        assert_eq!(std::fs::read(outside).unwrap(), b"outside");
    }
}
