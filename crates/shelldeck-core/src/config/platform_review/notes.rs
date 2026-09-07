//! Durable line-anchored review notes for the native Platform v2 lane.
//!
//! A note is a local draft, never an observation and never authority. It
//! retains the exact snapshot coordinates it was written against — file,
//! hunk, side, line and the captured snapshot revision — so a later snapshot
//! can never silently re-anchor it. Selection is persisted next to the note
//! so a batch delivery decision survives restart, but the store itself
//! dispatches nothing: only a typed preview may cross the network.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::*;
use crate::config::app_config::AppConfig;
use crate::workspace_review::storage::{
    bounded_descriptor_read, ensure_private_directory_io, lock_path, open_lock_file,
    secure_atomic_write,
};

const REVIEW_NOTE_SCHEMA: u16 = 1;
const MAX_REVIEW_NOTE_TARGETS: usize = 16;
const MAX_REVIEW_NOTES_PER_TARGET: usize = 64;
const MAX_REVIEW_NOTE_BODY_BYTES: usize = 4 * 1024;
const MAX_REVIEW_NOTE_FILE_BYTES: u64 = 1024 * 1024;
static REVIEW_NOTE_PROCESS_LOCK: parking_lot::Mutex<()> = parking_lot::Mutex::new(());

/// One persisted line-anchored draft, bound to the snapshot it was written on.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlatformReviewNote {
    pub id: String,
    pub anchor: ReviewAnchorSemantic,
    pub body: String,
    pub captured_revision: Revision,
    pub selected: bool,
    pub created_at_ms: u64,
}

impl PlatformReviewNote {
    /// A note may be prepared or delivered only while the snapshot it was
    /// anchored against is still the exact active one and its coordinates
    /// still resolve inside that snapshot.
    #[must_use]
    pub fn is_actionable(&self, review: &PlatformReviewSemantic) -> bool {
        self.captured_revision == review.revision && validate_anchor(review, &self.anchor).is_ok()
    }
}

#[derive(Clone, Debug)]
pub struct PlatformReviewNoteStore {
    path: PathBuf,
}

impl PlatformReviewNoteStore {
    pub fn open_default() -> Result<Self, ReviewNoteError> {
        Self::open(
            AppConfig::config_dir()
                .join("platform-review")
                .join("notes")
                .join("notes-v1.json"),
        )
    }

    pub fn open(path: PathBuf) -> Result<Self, ReviewNoteError> {
        let store = Self { path };
        let _guard = REVIEW_NOTE_PROCESS_LOCK.lock();
        store.prepare_storage()?;
        let lock = open_lock_file(&lock_path(&store.path))?;
        fs2::FileExt::lock_exclusive(&lock)?;
        let _ = load_document(&store.path)?;
        Ok(store)
    }

    /// Persist one note anchored inside the exact current snapshot.
    pub fn add(
        &self,
        target: &PlatformReviewTarget,
        review: &PlatformReviewSemantic,
        anchor: &ReviewAnchorSemantic,
        body: &str,
    ) -> Result<PlatformReviewNote, ReviewNoteError> {
        validate_review_target(target, review).map_err(|_| ReviewNoteError::ForeignSnapshot)?;
        validate_anchor(review, anchor).map_err(|_| ReviewNoteError::AnchorInvalid)?;
        let body = body.trim().to_owned();
        if body.is_empty() || body.len() > MAX_REVIEW_NOTE_BODY_BYTES {
            return Err(ReviewNoteError::BodyInvalid);
        }
        let note = PlatformReviewNote {
            id: format!("shelldeck-note-{}", review_nonce()),
            anchor: anchor.clone(),
            body,
            captured_revision: review.revision,
            selected: false,
            created_at_ms: now_ms(),
        };
        let stored = note.clone();
        self.transact(|document| {
            let record = document.target_mut(target)?;
            if record.notes.len() >= MAX_REVIEW_NOTES_PER_TARGET {
                return Err(ReviewNoteError::CapacityExceeded);
            }
            if record.notes.iter().any(|value| value.id == stored.id) {
                return Err(ReviewNoteError::DuplicateNote);
            }
            record.notes.push(ReviewNoteDisk::from_note(&stored));
            Ok(())
        })?;
        Ok(note)
    }

    /// Mark or unmark one note for batch delivery.
    pub fn set_selected(
        &self,
        target: &PlatformReviewTarget,
        id: &str,
        selected: bool,
    ) -> Result<(), ReviewNoteError> {
        self.transact(|document| {
            let record = document.target_mut(target)?;
            let note = record
                .notes
                .iter_mut()
                .find(|note| note.id == id)
                .ok_or(ReviewNoteError::UnknownNote)?;
            note.selected = selected;
            Ok(())
        })
    }

    pub fn remove(&self, target: &PlatformReviewTarget, id: &str) -> Result<(), ReviewNoteError> {
        self.transact(|document| {
            let record = document.target_mut(target)?;
            let before = record.notes.len();
            record.notes.retain(|note| note.id != id);
            if record.notes.len() == before {
                return Err(ReviewNoteError::UnknownNote);
            }
            document.records.retain(|record| !record.notes.is_empty());
            Ok(())
        })
    }

    /// Every retained note for this exact target, oldest first.
    pub fn notes(
        &self,
        target: &PlatformReviewTarget,
    ) -> Result<Vec<PlatformReviewNote>, ReviewNoteError> {
        let document = self.read()?;
        let Some(record) = document
            .records
            .iter()
            .find(|record| record.matches_target(target))
        else {
            return Ok(Vec::new());
        };
        let mut notes = record
            .notes
            .iter()
            .map(ReviewNoteDisk::to_note)
            .collect::<Result<Vec<_>, _>>()?;
        notes.sort_by(|left, right| {
            left.created_at_ms
                .cmp(&right.created_at_ms)
                .then_with(|| left.id.cmp(&right.id))
        });
        Ok(notes)
    }

    /// Selected notes that the exact active snapshot still supports. A note
    /// captured against a superseded revision is retained but never returned
    /// here: it must be re-anchored explicitly, never silently.
    pub fn selected_actionable(
        &self,
        target: &PlatformReviewTarget,
        review: &PlatformReviewSemantic,
    ) -> Result<Vec<PlatformReviewNote>, ReviewNoteError> {
        validate_review_target(target, review).map_err(|_| ReviewNoteError::ForeignSnapshot)?;
        Ok(self
            .notes(target)?
            .into_iter()
            .filter(|note| note.selected && note.is_actionable(review))
            .collect())
    }

    fn read(&self) -> Result<ReviewNoteDocument, ReviewNoteError> {
        let _guard = REVIEW_NOTE_PROCESS_LOCK.lock();
        self.prepare_storage()?;
        let lock = open_lock_file(&lock_path(&self.path))?;
        fs2::FileExt::lock_exclusive(&lock)?;
        load_document(&self.path)
    }

    fn transact<T>(
        &self,
        update: impl FnOnce(&mut ReviewNoteDocument) -> Result<T, ReviewNoteError>,
    ) -> Result<T, ReviewNoteError> {
        let _guard = REVIEW_NOTE_PROCESS_LOCK.lock();
        self.prepare_storage()?;
        let lock = open_lock_file(&lock_path(&self.path))?;
        fs2::FileExt::lock_exclusive(&lock)?;
        let mut document = load_document(&self.path)?;
        let outcome = update(&mut document)?;
        document.revision = document
            .revision
            .checked_add(1)
            .ok_or(ReviewNoteError::DocumentInvalid)?;
        persist_document(&self.path, &document)?;
        Ok(outcome)
    }

    fn prepare_storage(&self) -> Result<(), ReviewNoteError> {
        let parent = self.path.parent().ok_or(ReviewNoteError::PathInvalid)?;
        ensure_private_directory_io(parent)?;
        Ok(())
    }
}

fn now_ms() -> u64 {
    u64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis(),
    )
    .unwrap_or(u64::MAX)
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReviewNoteDocument {
    schema: u16,
    revision: u64,
    records: Vec<ReviewNoteTargetDisk>,
}

impl Default for ReviewNoteDocument {
    fn default() -> Self {
        Self {
            schema: REVIEW_NOTE_SCHEMA,
            revision: 0,
            records: Vec::new(),
        }
    }
}

impl ReviewNoteDocument {
    fn target_mut(
        &mut self,
        target: &PlatformReviewTarget,
    ) -> Result<&mut ReviewNoteTargetDisk, ReviewNoteError> {
        if let Some(index) = self
            .records
            .iter()
            .position(|record| record.matches_target(target))
        {
            return Ok(&mut self.records[index]);
        }
        if self.records.len() >= MAX_REVIEW_NOTE_TARGETS {
            return Err(ReviewNoteError::CapacityExceeded);
        }
        self.records.push(ReviewNoteTargetDisk {
            project: target.project.as_str().to_owned(),
            workspace_kind: target.workspace.kind().as_str().to_owned(),
            workspace: target.workspace.id().to_owned(),
            notes: Vec::new(),
        });
        self.records
            .last_mut()
            .ok_or(ReviewNoteError::DocumentInvalid)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReviewNoteTargetDisk {
    project: String,
    workspace_kind: String,
    workspace: String,
    notes: Vec<ReviewNoteDisk>,
}

impl ReviewNoteTargetDisk {
    fn matches_target(&self, target: &PlatformReviewTarget) -> bool {
        self.project == target.project.as_str()
            && self.workspace_kind == target.workspace.kind().as_str()
            && self.workspace == target.workspace.id()
    }

    fn validate_identity(&self) -> Result<(), ReviewNoteError> {
        ProjectId::new(self.project.clone()).map_err(|_| ReviewNoteError::DocumentInvalid)?;
        let kind = WorkContextTargetKind::parse(&self.workspace_kind)
            .map_err(|_| ReviewNoteError::DocumentInvalid)?;
        if kind != WorkContextTargetKind::UserWorkspace {
            return Err(ReviewNoteError::DocumentInvalid);
        }
        WorkContextIdentity::parse_local(kind, &self.workspace)
            .map_err(|_| ReviewNoteError::DocumentInvalid)?;
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReviewNoteDisk {
    id: String,
    file_id: String,
    hunk_id: String,
    side: String,
    line: u32,
    body: String,
    captured_revision: u64,
    selected: bool,
    created_at_ms: u64,
}

impl ReviewNoteDisk {
    fn from_note(note: &PlatformReviewNote) -> Self {
        Self {
            id: note.id.clone(),
            file_id: note.anchor.file_id.clone(),
            hunk_id: note.anchor.hunk_id.clone(),
            side: note.anchor.side.as_str().to_owned(),
            line: note.anchor.line,
            body: note.body.clone(),
            captured_revision: note.captured_revision.get(),
            selected: note.selected,
            created_at_ms: note.created_at_ms,
        }
    }

    fn to_note(&self) -> Result<PlatformReviewNote, ReviewNoteError> {
        if self.id.is_empty()
            || self.file_id.is_empty()
            || self.hunk_id.is_empty()
            || self.body.is_empty()
            || self.body.len() > MAX_REVIEW_NOTE_BODY_BYTES
        {
            return Err(ReviewNoteError::DocumentInvalid);
        }
        Ok(PlatformReviewNote {
            id: self.id.clone(),
            anchor: ReviewAnchorSemantic {
                file_id: self.file_id.clone(),
                hunk_id: self.hunk_id.clone(),
                side: DiffSide::parse(&self.side).map_err(|_| ReviewNoteError::DocumentInvalid)?,
                line: self.line,
            },
            body: self.body.clone(),
            captured_revision: Revision::new(self.captured_revision)
                .map_err(|_| ReviewNoteError::DocumentInvalid)?,
            selected: self.selected,
            created_at_ms: self.created_at_ms,
        })
    }
}

fn load_document(path: &Path) -> Result<ReviewNoteDocument, ReviewNoteError> {
    let Some(bytes) = bounded_descriptor_read(path, MAX_REVIEW_NOTE_FILE_BYTES)? else {
        return Ok(ReviewNoteDocument::default());
    };
    if bytes.len() as u64 > MAX_REVIEW_NOTE_FILE_BYTES {
        return Err(ReviewNoteError::DocumentInvalid);
    }
    let document: ReviewNoteDocument = serde_json::from_slice(&bytes)?;
    validate_document(&document)?;
    Ok(document)
}

fn validate_document(document: &ReviewNoteDocument) -> Result<(), ReviewNoteError> {
    if document.schema != REVIEW_NOTE_SCHEMA || document.records.len() > MAX_REVIEW_NOTE_TARGETS {
        return Err(ReviewNoteError::DocumentInvalid);
    }
    let mut targets = std::collections::BTreeSet::new();
    for record in &document.records {
        record.validate_identity()?;
        if !targets.insert((
            record.project.as_str(),
            record.workspace_kind.as_str(),
            record.workspace.as_str(),
        )) {
            return Err(ReviewNoteError::DocumentInvalid);
        }
        if record.notes.len() > MAX_REVIEW_NOTES_PER_TARGET {
            return Err(ReviewNoteError::DocumentInvalid);
        }
        let mut ids = std::collections::BTreeSet::new();
        for note in &record.notes {
            let _ = note.to_note()?;
            if !ids.insert(note.id.as_str()) {
                return Err(ReviewNoteError::DocumentInvalid);
            }
        }
    }
    Ok(())
}

fn persist_document(path: &Path, document: &ReviewNoteDocument) -> Result<(), ReviewNoteError> {
    validate_document(document)?;
    let bytes = serde_json::to_vec_pretty(document)?;
    if bytes.len() as u64 > MAX_REVIEW_NOTE_FILE_BYTES {
        return Err(ReviewNoteError::DocumentInvalid);
    }
    secure_atomic_write(path, &bytes)?;
    Ok(())
}

#[derive(Debug, Error)]
pub enum ReviewNoteError {
    #[error("review note storage failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("review note document is not valid JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("review note path is invalid")]
    PathInvalid,
    #[error("review note document is invalid or exceeds its bound")]
    DocumentInvalid,
    #[error("review notes have reached their bounded capacity")]
    CapacityExceeded,
    #[error("review note already exists")]
    DuplicateNote,
    #[error("review note is not retained for this target")]
    UnknownNote,
    #[error("review note body is empty or exceeds its bound")]
    BodyInvalid,
    #[error("review note anchor is not in the exact snapshot")]
    AnchorInvalid,
    #[error("review snapshot belongs to another workspace")]
    ForeignSnapshot,
}

#[cfg(test)]
mod tests {
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

    fn first_anchor(review: &PlatformReviewSemantic) -> ReviewAnchorSemantic {
        let file = review
            .files
            .iter()
            .find(|file| !file.hunks.is_empty())
            .unwrap();
        let hunk = &file.hunks[0];
        let (side, start) = if hunk.new_lines > 0 {
            (DiffSide::New, hunk.new_start)
        } else {
            (DiffSide::Old, hunk.old_start)
        };
        ReviewAnchorSemantic {
            file_id: file.id.clone(),
            hunk_id: hunk.id.clone(),
            side,
            line: start,
        }
    }

    struct TempRoot(PathBuf);

    impl TempRoot {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "shelldeck-review-notes-{}-{}",
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

        fn store_path(&self) -> PathBuf {
            self.0
                .join("platform-review")
                .join("notes")
                .join("notes-v1.json")
        }
    }

    impl Drop for TempRoot {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    // SDTEST-1853 — a line-anchored note survives restart with its exact
    // captured coordinates and its selection decision.
    #[test]
    fn note_and_selection_survive_restart_with_exact_anchor() {
        let directory = TempRoot::new();
        let review = semantic();
        let target = target(&review);
        let anchor = first_anchor(&review);
        let store = PlatformReviewNoteStore::open(directory.store_path()).unwrap();
        let note = store
            .add(&target, &review, &anchor, "  revoir cette borne  ")
            .unwrap();
        store.set_selected(&target, &note.id, true).unwrap();

        let restarted = PlatformReviewNoteStore::open(directory.store_path()).unwrap();
        let notes = restarted.notes(&target).unwrap();
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].anchor, anchor);
        assert_eq!(notes[0].body, "revoir cette borne");
        assert_eq!(notes[0].captured_revision, review.revision);
        assert!(notes[0].selected);
        assert_eq!(
            restarted.selected_actionable(&target, &review).unwrap(),
            notes
        );
    }

    // SDTEST-1854 — a note is never re-anchored onto a newer snapshot and a
    // coordinate outside the exact snapshot is refused before persistence.
    #[test]
    fn superseded_revision_and_foreign_anchor_are_never_actionable() {
        let directory = TempRoot::new();
        let review = semantic();
        let target = target(&review);
        let anchor = first_anchor(&review);
        let store = PlatformReviewNoteStore::open(directory.store_path()).unwrap();
        let note = store.add(&target, &review, &anchor, "borne").unwrap();
        store.set_selected(&target, &note.id, true).unwrap();

        let mut advanced = review.clone();
        advanced.revision = Revision::new(review.revision.get() + 1).unwrap();
        assert!(!note.is_actionable(&advanced));
        assert!(store
            .selected_actionable(&target, &advanced)
            .unwrap()
            .is_empty());
        // The note itself is retained, not silently dropped or moved.
        assert_eq!(store.notes(&target).unwrap().len(), 1);

        let outside = ReviewAnchorSemantic {
            line: anchor.line.saturating_add(9_999),
            ..anchor
        };
        assert!(matches!(
            store.add(&target, &review, &outside, "hors borne"),
            Err(ReviewNoteError::AnchorInvalid)
        ));
        assert!(matches!(
            store.add(&target, &review, &first_anchor(&review), "   "),
            Err(ReviewNoteError::BodyInvalid)
        ));
        assert_eq!(store.notes(&target).unwrap().len(), 1);
    }

    // SDTEST-1855 — an invalid or oversized note document fails closed and
    // never replaces what is already on disk.
    #[test]
    fn oversized_note_document_is_rejected_without_replacement() {
        let directory = TempRoot::new();
        let review = semantic();
        let target = target(&review);
        let path = directory.store_path();
        let store = PlatformReviewNoteStore::open(path.clone()).unwrap();
        store
            .add(&target, &review, &first_anchor(&review), "borne")
            .unwrap();
        let oversized = vec![b'x'; MAX_REVIEW_NOTE_FILE_BYTES as usize + 1];
        std::fs::write(&path, &oversized).unwrap();
        assert!(matches!(
            PlatformReviewNoteStore::open(path.clone()),
            Err(ReviewNoteError::DocumentInvalid)
        ));
        assert_eq!(std::fs::read(path).unwrap(), oversized);
    }
}
