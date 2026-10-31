//! Versioned, bounded, private disk storage for agent conversations.
//!
//! Drafts and presentation preferences deliberately live outside AppConfig so
//! saving a conversation can never replace account/session configuration.
use super::*;
use std::collections::{BTreeMap, HashSet};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

pub const AGENT_SESSION_SCHEMA_VERSION: u32 = 2;
pub const AGENT_DRAFT_SCHEMA_VERSION: u32 = 1;
pub const MAX_SAVED_AGENT_FOLLOWUPS: usize = 8;
const MAX_DRAFT_ASSET_STORAGE_BYTES: u64 = 128 * 1024 * 1024;

/// User-approved context snapshots, never executable authority or MCP credentials.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentDraftAttachment {
    pub path: PathBuf,
    pub name: String,
    pub text: Option<String>,
    #[serde(default)]
    pub image_digest: Option<String>,
    /// Only fresh, in-memory UI approvals may copy a source file into managed storage.
    #[serde(skip)]
    pub capture_image: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentQueuedDraft {
    pub context: AgentExecutionContext,
    pub prompt: String,
    pub attachments: Vec<AgentDraftAttachment>,
    pub effort: Option<crate::agent_runtime::AgentReasoningEffort>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentDraftBundle {
    pub version: u32,
    pub context: AgentExecutionContext,
    pub attachments: Vec<AgentDraftAttachment>,
    pub effort: Option<crate::agent_runtime::AgentReasoningEffort>,
    pub followups: Vec<AgentQueuedDraft>,
}

pub const MAX_AGENT_SESSION_FILE_BYTES: usize = 16 * 1024 * 1024;
const MAX_RETAINED_CONTENT_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_COLLAPSED_PROJECT_IDS: usize = 256;
const MAX_COLLAPSED_PROJECT_ID_BYTES: usize = 256;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AgentConsolePreferences {
    pub show_archived: bool,
    pub navigator_visible: bool,
    pub inspector_visible: bool,
    pub context_expanded: bool,
    pub inspector_tab: String,
    pub navigator_width: u16,
    pub inspector_width: u16,
    /// Stable catalog, pinned, conversation-folder and native SSH group keys.
    pub collapsed_project_ids: Vec<String>,
    pub remote_sessions_expanded: bool,
    pub remote_sessions_available_only: bool,
    pub ssh_selected_server: Option<Uuid>,
    pub ssh_sessions_collapsed: bool,
}

impl Default for AgentConsolePreferences {
    fn default() -> Self {
        Self {
            show_archived: false,
            navigator_visible: true,
            inspector_visible: false,
            context_expanded: false,
            inspector_tab: "files".into(),
            navigator_width: 280,
            inspector_width: 280,
            collapsed_project_ids: Vec::new(),
            remote_sessions_expanded: false,
            remote_sessions_available_only: false,
            ssh_selected_server: None,
            ssh_sessions_collapsed: false,
        }
    }
}

impl AgentConsolePreferences {
    pub fn toggle_project_collapsed(&mut self, project_id: &str) -> bool {
        if !valid_collapsed_project_id(project_id) {
            return false;
        }
        if self.collapsed_project_ids.iter().any(|id| id == project_id) {
            self.collapsed_project_ids.retain(|id| id != project_id);
        } else {
            self.collapsed_project_ids.push(project_id.to_owned());
            keep_latest(&mut self.collapsed_project_ids, MAX_COLLAPSED_PROJECT_IDS);
        }
        true
    }

    fn normalize(&mut self) {
        self.navigator_width = self.navigator_width.clamp(180, 480);
        self.inspector_width = self.inspector_width.clamp(240, 720);
        if !matches!(
            self.inspector_tab.as_str(),
            "files" | "changes" | "sources" | "tools"
        ) {
            self.inspector_tab = "files".into();
        }
        let mut seen = HashSet::new();
        self.collapsed_project_ids
            .retain(|id| valid_collapsed_project_id(id) && seen.insert(id.clone()));
        keep_latest(&mut self.collapsed_project_ids, MAX_COLLAPSED_PROJECT_IDS);
    }
}

fn valid_collapsed_project_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= MAX_COLLAPSED_PROJECT_ID_BYTES
        && !id.chars().any(char::is_control)
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AgentSessionSnapshot {
    pub collection: AgentSessionCollection,
    pub drafts: BTreeMap<Uuid, String>,
    pub draft_bundles: BTreeMap<Uuid, AgentDraftBundle>,
    pub preferences: AgentConsolePreferences,
    /// Inform the UI when damaged sessions were skipped, without discarding
    /// other conversations or silently granting an invalid target authority.
    #[serde(skip)]
    pub warnings: Vec<String>,
}

#[derive(Serialize, Deserialize)]
struct Envelope {
    schema_version: u32,
    #[serde(flatten)]
    snapshot: AgentSessionSnapshot,
}

#[derive(Debug, Default)]
pub struct AgentSessionSaveReport {
    pub warnings: Vec<String>,
    pub captured_images: BTreeMap<PathBuf, PathBuf>,
    pub unresolved_images: std::collections::HashSet<PathBuf>,
}

#[derive(Debug, Clone)]
pub struct AgentSessionStore {
    path: PathBuf,
}

impl Default for AgentSessionStore {
    fn default() -> Self {
        Self::new(Self::default_path())
    }
}

impl AgentSessionStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }
    pub fn default_path() -> PathBuf {
        crate::config::AppConfig::config_dir()
            .join("agents")
            .join("sessions.json")
    }
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Missing storage is a new library. Malformed/unsupported storage is an
    /// error so the caller can preserve the file and surface recovery advice.
    pub fn load(&self, now_ms: i64) -> Result<AgentSessionSnapshot> {
        let metadata = match std::fs::symlink_metadata(&self.path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(AgentSessionSnapshot::default());
            }
            Err(error) => return Err(error.into()),
        };
        if !metadata.is_file()
            || metadata.file_type().is_symlink()
            || metadata.len() > MAX_AGENT_SESSION_FILE_BYTES as u64
        {
            return Err(ShellDeckError::Config(
                "agent history is not a bounded regular file".into(),
            ));
        }
        let mut contents = Vec::new();
        std::fs::File::open(&self.path)?
            .take(MAX_AGENT_SESSION_FILE_BYTES as u64 + 1)
            .read_to_end(&mut contents)?;
        if contents.len() > MAX_AGENT_SESSION_FILE_BYTES {
            return Err(ShellDeckError::Config("agent history is too large".into()));
        }
        let value: serde_json::Value =
            serde_json::from_slice(&contents).map_err(serialization_error)?;
        let mut snapshot = if value.get("schema_version").is_some() {
            let envelope: Envelope = serde_json::from_value(value).map_err(serialization_error)?;
            if !(1..=AGENT_SESSION_SCHEMA_VERSION).contains(&envelope.schema_version) {
                return Err(ShellDeckError::Config(
                    "unsupported agent history version; preserve the file for a newer ShellDeck"
                        .into(),
                ));
            }
            envelope.snapshot
        } else if value.get("sessions").is_some() {
            // Older provider-neutral collection files remain readable.
            AgentSessionSnapshot {
                collection: serde_json::from_value(value).map_err(serialization_error)?,
                ..Default::default()
            }
        } else {
            return Err(ShellDeckError::Config(
                "agent history has no supported schema".into(),
            ));
        };
        normalize_snapshot(&mut snapshot);
        self.validate_saved_draft_assets(&mut snapshot);
        snapshot.collection.recover_interrupted(now_ms);
        Ok(snapshot)
    }

    fn asset_root(&self) -> PathBuf {
        self.path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join("draft-assets")
    }

    /// Called only after the UI's explicit attachment-review action.
    pub fn read_draft_image(&self, path: &Path) -> Result<Vec<u8>> {
        use sha2::{Digest, Sha256};
        let root = self.asset_root();
        if !is_managed_asset_path(path, &root)
            || std::fs::symlink_metadata(&root).is_ok_and(|meta| meta.file_type().is_symlink())
        {
            return Err(ShellDeckError::Config(
                "reattach this image: invalid saved asset path".into(),
            ));
        }
        let meta = std::fs::symlink_metadata(path)?;
        if !meta.is_file()
            || meta.file_type().is_symlink()
            || meta.len() > crate::ai::AI_ATTACHMENT_IMAGE_MAX_BYTES as u64
        {
            return Err(ShellDeckError::Config(
                "reattach this image: invalid saved asset".into(),
            ));
        }
        let mut bytes = Vec::new();
        std::fs::File::open(path)?
            .take(crate::ai::AI_ATTACHMENT_IMAGE_MAX_BYTES as u64 + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() > crate::ai::AI_ATTACHMENT_IMAGE_MAX_BYTES
            || path.file_name().and_then(|name| name.to_str())
                != Some(format!("{:x}.image", Sha256::digest(&bytes)).as_str())
        {
            return Err(ShellDeckError::Config(
                "reattach this image: saved content changed".into(),
            ));
        }
        Ok(bytes)
    }

    pub fn owns_draft_image_path(&self, path: &Path) -> bool {
        is_managed_asset_path(path, &self.asset_root())
    }

    fn validate_saved_draft_assets(&self, snapshot: &mut AgentSessionSnapshot) {
        let root = self.asset_root();
        for bundle in snapshot.draft_bundles.values_mut() {
            for attachment in bundle.attachments.iter_mut().chain(
                bundle
                    .followups
                    .iter_mut()
                    .flat_map(|draft| &mut draft.attachments),
            ) {
                if attachment.text.is_none()
                    && (!is_managed_asset_path(&attachment.path, &root)
                        || std::fs::symlink_metadata(&root)
                            .is_ok_and(|meta| meta.file_type().is_symlink()))
                {
                    // Keep a visible unresolved item; never silently send an incomplete draft.
                    attachment.path = PathBuf::new();
                    snapshot.warnings.push(
                        "A saved image reference is invalid. Reattach the image before sending."
                            .into(),
                    );
                }
            }
        }
    }

    fn capture_draft_assets(&self, snapshot: &mut AgentSessionSnapshot) -> AgentSessionSaveReport {
        use sha2::{Digest, Sha256};
        let root = self.asset_root();
        let invalid_root = std::fs::symlink_metadata(&root)
            .is_ok_and(|meta| meta.file_type().is_symlink() || !meta.is_dir());
        let entries = std::fs::read_dir(&root)
            .ok()
            .into_iter()
            .flatten()
            .take(4097)
            .collect::<Vec<_>>();
        let directory_limit_reached = entries.len() > 4096;
        let mut total = entries
            .into_iter()
            .filter_map(|entry| entry.ok())
            .filter_map(|entry| entry.metadata().ok())
            .map(|meta| meta.len())
            .sum::<u64>();
        let mut report = AgentSessionSaveReport::default();
        for bundle in snapshot.draft_bundles.values_mut() {
            for attachment in bundle.attachments.iter_mut().chain(
                bundle
                    .followups
                    .iter_mut()
                    .flat_map(|draft| &mut draft.attachments),
            ) {
                if attachment.text.is_some() || !attachment.capture_image {
                    continue;
                }
                let source = attachment.path.clone();
                let captured = (|| -> Result<PathBuf> {
                    if let Some(path) = report.captured_images.get(&source) {
                        if attachment.image_digest.as_ref().is_some_and(|digest| {
                            path.file_stem().and_then(|stem| stem.to_str()) != Some(digest.as_str())
                        }) {
                            return Err(ShellDeckError::Config(
                                "image changed between attachment previews".into(),
                            ));
                        }
                        return Ok(path.clone());
                    }
                    if invalid_root {
                        return Err(ShellDeckError::Config(
                            "invalid agent draft asset directory".into(),
                        ));
                    }
                    let meta = std::fs::symlink_metadata(&source)?;
                    if !meta.is_file()
                        || meta.file_type().is_symlink()
                        || meta.len() > crate::ai::AI_ATTACHMENT_IMAGE_MAX_BYTES as u64
                    {
                        return Err(ShellDeckError::Config(
                            "draft image must be a bounded regular file".into(),
                        ));
                    }
                    let mut bytes = Vec::new();
                    std::fs::File::open(&source)?
                        .take(crate::ai::AI_ATTACHMENT_IMAGE_MAX_BYTES as u64 + 1)
                        .read_to_end(&mut bytes)?;
                    if bytes.len() > crate::ai::AI_ATTACHMENT_IMAGE_MAX_BYTES {
                        return Err(ShellDeckError::Config(
                            "draft image exceeds its storage limit".into(),
                        ));
                    }
                    let digest = format!("{:x}", Sha256::digest(&bytes));
                    if attachment
                        .image_digest
                        .as_ref()
                        .is_some_and(|expected| *expected != digest)
                    {
                        return Err(ShellDeckError::Config(
                            "image changed since it was attached".into(),
                        ));
                    }
                    let destination = root.join(format!("{digest}.image"));
                    if self
                        .read_draft_image(&destination)
                        .is_ok_and(|saved| saved == bytes)
                    {
                        return Ok(destination);
                    }
                    if !destination.exists() {
                        if directory_limit_reached {
                            return Err(ShellDeckError::Config(
                                "saved draft image directory reached its entry limit".into(),
                            ));
                        }
                        total = total.saturating_add(bytes.len() as u64);
                        if total > MAX_DRAFT_ASSET_STORAGE_BYTES {
                            return Err(ShellDeckError::Config(
                                "saved draft images exceed 128 MiB".into(),
                            ));
                        }
                    }
                    private_atomic_write(&destination, &bytes)?;
                    Ok(destination)
                })();
                attachment.capture_image = false;
                match captured {
                    Ok(path) => {
                        report.captured_images.insert(source, path.clone());
                        attachment.path = path;
                    }
                    Err(error) => {
                        report.unresolved_images.insert(source);
                        // Preserve the user's unresolved item, and never prevent other
                        // conversations/text from being saved because one image vanished.
                        report
                            .warnings
                            .push(format!("Reattach {}: {error}", attachment.name));
                        attachment.path = PathBuf::new();
                    }
                }
            }
        }
        report
    }

    pub fn save(&self, snapshot: &AgentSessionSnapshot) -> Result<()> {
        self.save_with_drafts(snapshot).map(|_| ())
    }

    pub fn save_with_drafts(
        &self,
        snapshot: &AgentSessionSnapshot,
    ) -> Result<AgentSessionSaveReport> {
        let mut snapshot = snapshot.clone();
        normalize_snapshot(&mut snapshot);
        let mut report = self.capture_draft_assets(&mut snapshot);
        report.warnings.extend(snapshot.warnings.clone());
        let contents = serde_json::to_vec(&Envelope {
            schema_version: AGENT_SESSION_SCHEMA_VERSION,
            snapshot,
        })
        .map_err(serialization_error)?;
        if contents.len() > MAX_AGENT_SESSION_FILE_BYTES {
            return Err(ShellDeckError::Config(
                "agent history exceeds its bounded storage budget".into(),
            ));
        }
        private_atomic_write(&self.path, &contents)?;
        Ok(report)
    }
}

pub fn draft_image_digest(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(bytes))
}

fn is_managed_asset_path(path: &Path, root: &Path) -> bool {
    path.parent() == Some(root)
        && path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| {
                name.strip_suffix(".image").is_some_and(|hash| {
                    hash.len() == 64 && hash.bytes().all(|byte| byte.is_ascii_hexdigit())
                })
            })
}

fn serialization_error(error: serde_json::Error) -> ShellDeckError {
    ShellDeckError::Serialization(format!("agent history: {error}"))
}

fn keep_latest<T>(values: &mut Vec<T>, max: usize) {
    if values.len() > max {
        values.drain(..values.len() - max);
    }
}

fn normalize_snapshot(snapshot: &mut AgentSessionSnapshot) {
    snapshot.preferences.normalize();
    snapshot.collection.max_concurrent = snapshot
        .collection
        .max_concurrent
        .clamp(1, MAX_CONCURRENT_AGENT_SESSIONS);
    keep_latest(&mut snapshot.collection.sessions, MAX_AGENT_SESSIONS);
    let mut session_ids = HashSet::new();
    snapshot.collection.sessions.retain_mut(|session| {
        if session.id.is_nil() || !session_ids.insert(session.id) {
            snapshot
                .warnings
                .push("A conversation with a duplicate or invalid identity was skipped.".into());
            return false;
        }
        let Ok(context) = session.context.clone().normalized() else {
            snapshot.warnings.push(format!(
                "A conversation with an invalid execution context was skipped ({})",
                session.id
            ));
            return false;
        };
        session.context = context;
        session.name = truncate_utf8(session.name.trim(), MAX_AGENT_SESSION_NAME_BYTES);
        if session.name.is_empty() {
            session.name = "Recovered conversation".into();
        }
        session.name.retain(|character| !character.is_control());
        session.unread_count = session.unread_count.min(100_000);
        session.current_turn_id = None;
        session.provider_session_id = None;
        session.provider_session_context = None;
        if session.remote_history.as_ref().is_some_and(|reference| !reference.is_valid()) {
            session.remote_history = None;
            snapshot.warnings.push(format!(
                "An invalid remote history reference was removed; the local conversation was preserved ({})",
                session.id
            ));
        }
        session.question = None;
        session.approval = None;
        keep_latest(&mut session.messages, MAX_AGENT_MESSAGES_PER_SESSION);
        keep_latest(&mut session.trace, MAX_AGENT_TRACE_EVENTS_PER_SESSION);
        keep_latest(&mut session.turns, MAX_AGENT_TURNS_PER_SESSION);
        keep_latest(&mut session.boundaries, MAX_AGENT_BOUNDARIES_PER_SESSION);
        let mut turn_ids = HashSet::new();
        session.turns.retain_mut(|turn| {
            let Ok(context) = turn.context.clone().normalized() else {
                return false;
            };
            turn.context = context;
            !turn.id.is_nil() && turn_ids.insert(turn.id)
        });
        let mut entry_ids = HashSet::new();
        for message in &mut session.messages {
            if message.id.is_nil() || !entry_ids.insert(message.id) {
                message.id = Uuid::new_v4();
            }
            if message.turn_id.is_some_and(|id| !turn_ids.contains(&id)) {
                message.turn_id = None;
            }
            message.text = truncate_utf8(&message.text, MAX_AGENT_MESSAGE_BYTES);
        }
        for trace in &mut session.trace {
            if trace.id.is_nil() || !entry_ids.insert(trace.id) {
                trace.id = Uuid::new_v4();
            }
            if trace.turn_id.is_some_and(|id| !turn_ids.contains(&id)) {
                trace.turn_id = None;
            }
            trace.detail = bounded_trace(trace.detail.clone());
            trace.correlation_id = trace
                .correlation_id
                .take()
                .map(|id| truncate_utf8(&id, MAX_AGENT_TRACE_CORRELATION_BYTES));
        }
        session.boundaries.retain_mut(|boundary| {
            let Ok(context) = boundary.context.clone().normalized() else {
                return false;
            };
            boundary.context = context;
            if boundary.id.is_nil() || !entry_ids.insert(boundary.id) {
                boundary.id = Uuid::new_v4();
            }
            true
        });
        repair_sequences(session);
        session.plan = session.plan.take().map(AgentPlanUpdate::bounded);
        session.usage = session.usage.take().map(AgentUsageUpdate::bounded);
        // Older libraries did not record which run supplied metadata and could
        // retain Claude's model after switching to a completely different CLI.
        if session.metadata_turn_id.is_some_and(|id| {
            session
                .turns
                .last()
                .is_some_and(|turn| turn.id == id && turn.context == session.context)
        }) {
            session.metadata = session.metadata.take().map(AgentRunMetadata::bounded);
        } else {
            session.metadata = None;
            session.metadata_turn_id = None;
        }
        keep_latest(&mut session.sources, 128);
        keep_latest(&mut session.artifacts, 128);
        keep_latest(&mut session.subagents, 64);
        session.sources = session
            .sources
            .drain(..)
            .map(AgentSource::bounded)
            .collect();
        session.artifacts = session
            .artifacts
            .drain(..)
            .map(AgentArtifact::bounded)
            .collect();
        session.subagents = session
            .subagents
            .drain(..)
            .map(AgentSubagentUpdate::bounded)
            .collect();
        true
    });
    if !snapshot.collection.selected_id.is_some_and(|id| {
        snapshot
            .collection
            .get(id)
            .is_some_and(|session| !session.archived)
    }) {
        snapshot.collection.selected_id = snapshot
            .collection
            .library_sessions(false)
            .first()
            .map(|session| session.id);
    }
    snapshot.collection.surface_visible = false;
    snapshot.drafts.retain(|id, draft| {
        *draft = truncate_utf8(draft, crate::agent_runtime::MAX_AGENT_PROMPT_BYTES);
        snapshot.collection.get(*id).is_some()
    });
    snapshot.draft_bundles.retain(|id, bundle| {
        if snapshot.collection.get(*id).is_none() {
            return false;
        }
        if bundle.version != AGENT_DRAFT_SCHEMA_VERSION
            || bundle.context.clone().normalized().is_err()
        {
            snapshot.warnings.push(format!(
                "Unsupported or invalid saved draft context ({id}); draft text was preserved."
            ));
            return false;
        }
        bundle
            .attachments
            .truncate(crate::ai::AI_ATTACHMENT_MAX_COUNT);
        bundle.followups.truncate(MAX_SAVED_AGENT_FOLLOWUPS);
        for attachment in bundle.attachments.iter_mut().chain(
            bundle
                .followups
                .iter_mut()
                .flat_map(|draft| &mut draft.attachments),
        ) {
            attachment.name = truncate_utf8(&attachment.name, 256);
            attachment.name.retain(|character| !character.is_control());
            if let Some(text) = &mut attachment.text {
                *text = truncate_utf8(text, crate::ai::AI_ATTACHMENT_TEXT_MAX_BYTES);
            }
            if attachment.path.as_os_str().len() > 8192 {
                attachment.path = PathBuf::new();
                attachment.capture_image = false;
            }
        }
        bundle.followups.retain_mut(|draft| {
            draft
                .attachments
                .truncate(crate::ai::AI_ATTACHMENT_MAX_COUNT);
            draft.prompt =
                truncate_utf8(&draft.prompt, crate::agent_runtime::MAX_AGENT_PROMPT_BYTES);
            draft.context.clone().normalized().is_ok() && !draft.prompt.trim().is_empty()
        });
        true
    });
    // Keep the most recent content within an aggregate budget. Per-field limits
    // alone would allow 64 conversations to produce a multi-gigabyte file.
    let mut retained = 0usize;
    let mut ordered = snapshot.collection.sessions.iter_mut().collect::<Vec<_>>();
    ordered.sort_by_key(|session| std::cmp::Reverse(session.updated_at_ms));
    for session in ordered {
        let extras_size = serde_json::to_vec(&(
            &session.plan,
            &session.metadata,
            &session.sources,
            &session.artifacts,
            &session.subagents,
        ))
        .map_or(MAX_RETAINED_CONTENT_BYTES, |content| content.len());
        if retained.saturating_add(extras_size) <= MAX_RETAINED_CONTENT_BYTES / 2 {
            retained += extras_size;
        } else {
            session.plan = None;
            session.metadata = None;
            session.sources.clear();
            session.artifacts.clear();
            session.subagents.clear();
        }
        for message in session.messages.iter_mut().rev() {
            let remaining = MAX_RETAINED_CONTENT_BYTES.saturating_sub(retained);
            message.text = truncate_utf8(&message.text, remaining.min(MAX_AGENT_MESSAGE_BYTES));
            retained += message.text.len();
        }
        // Empty messages resulting from the global budget carry no history.
        session.messages.retain(|message| !message.text.is_empty());
        for trace in session.trace.iter_mut().rev() {
            let size = serde_json::to_vec(&trace.detail)
                .map_or(MAX_AGENT_TRACE_STRING_BYTES, |value| value.len());
            if retained.saturating_add(size) <= MAX_RETAINED_CONTENT_BYTES {
                retained += size;
            } else {
                trace.detail = AgentTraceKind::Activity {
                    label: String::new(),
                };
            }
        }
        session.trace.retain(|trace| !matches!(&trace.detail, AgentTraceKind::Activity { label } if label.is_empty()));
    }
}

fn repair_sequences(session: &mut AgentSession) {
    let mut entries = session
        .messages
        .iter()
        .enumerate()
        .map(|(index, message)| (message.sequence, message.at_ms, 0, index))
        .chain(
            session
                .trace
                .iter()
                .enumerate()
                .map(|(index, trace)| (trace.sequence, trace.at_ms, 1, index)),
        )
        .chain(
            session
                .boundaries
                .iter()
                .enumerate()
                .map(|(index, boundary)| (boundary.sequence, boundary.at_ms, 2, index)),
        )
        .collect::<Vec<_>>();
    let valid = entries
        .iter()
        .all(|entry| entry.0 > 0 && entry.0 < u64::MAX - 1)
        && entries
            .iter()
            .map(|entry| entry.0)
            .collect::<HashSet<_>>()
            .len()
            == entries.len();
    if valid {
        entries.sort_by_key(|entry| entry.0);
    } else {
        entries.sort_by_key(|entry| (entry.1, entry.0, entry.2, entry.3));
    }
    for (offset, (_, _, kind, index)) in entries.iter().enumerate() {
        let sequence = offset as u64 + 1;
        match kind {
            0 => session.messages[*index].sequence = sequence,
            1 => session.trace[*index].sequence = sequence,
            _ => session.boundaries[*index].sequence = sequence,
        }
    }
    session.messages.sort_by_key(|message| message.sequence);
    session.trace.sort_by_key(|trace| trace.sequence);
    session.boundaries.sort_by_key(|boundary| boundary.sequence);
    session.next_sequence = entries.len() as u64 + 1;
}

fn private_atomic_write(path: &Path, contents: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent)?;
    if std::fs::symlink_metadata(path).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
        return Err(ShellDeckError::Config(
            "refusing to replace symlinked agent history".into(),
        ));
    }
    let temporary = parent.join(format!(".agent-history-{}.tmp", Uuid::new_v4()));
    let result = (|| -> Result<()> {
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temporary)?;
        file.write_all(contents)?;
        file.sync_all()?;
        drop(file);
        replace_file(&temporary, path)?;
        #[cfg(unix)]
        if let Ok(directory) = std::fs::File::open(parent) {
            let _ = directory.sync_all();
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

#[cfg(not(windows))]
fn replace_file(source: &Path, destination: &Path) -> std::io::Result<()> {
    std::fs::rename(source, destination)
}

#[cfg(windows)]
fn replace_file(source: &Path, destination: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    };
    let source = source
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let destination = destination
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    // SAFETY: owned, NUL-terminated UTF-16 buffers remain alive for the call.
    if unsafe {
        MoveFileExW(
            source.as_ptr(),
            destination.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    } == 0
    {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // SDTEST-2836 / SDUC-564 — v1 drafts migrate; complete bundles retain scoped
    // user input and private image snapshots, never runtime resume authority.
    #[test]
    fn durable_draft_bundles_roundtrip_and_capture_private_images() {
        use crate::agent_runtime::AgentReasoningEffort;
        let temporary = TempStore::new();
        let mut snapshot = AgentSessionSnapshot::default();
        let id = snapshot.collection.create("Draft", context(), 1).unwrap();
        snapshot.drafts.insert(id, "Final keystroke".into());
        let original_image = temporary.store.path().with_file_name("clipboard.png");
        let image = b"\x89PNG\r\n\x1a\napproved fixture bytes";
        std::fs::write(&original_image, image).unwrap();
        let descriptor = AgentDraftAttachment {
            path: original_image.clone(),
            name: "Screenshot.png".into(),
            text: None,
            image_digest: None,
            capture_image: true,
        };
        let queued = AgentQueuedDraft {
            context: context(),
            prompt: "Follow up after restart".into(),
            attachments: vec![descriptor.clone()],
            effort: Some(AgentReasoningEffort::High),
        };
        snapshot.draft_bundles.insert(
            id,
            AgentDraftBundle {
                version: AGENT_DRAFT_SCHEMA_VERSION,
                context: context(),
                attachments: vec![
                    descriptor,
                    AgentDraftAttachment {
                        path: PathBuf::from("/gone/notes.txt"),
                        name: "Notes".into(),
                        text: Some("Approved original text".into()),
                        image_digest: None,
                        capture_image: false,
                    },
                ],
                effort: Some(AgentReasoningEffort::High),
                followups: vec![queued],
            },
        );
        temporary.store.save(&snapshot).unwrap();
        std::fs::remove_file(&original_image).unwrap();
        let restored = temporary.store.load(2).unwrap();
        let bundle = &restored.draft_bundles[&id];
        assert_eq!(restored.drafts[&id], "Final keystroke");
        assert_eq!(bundle.context, context());
        assert_eq!(bundle.effort, Some(AgentReasoningEffort::High));
        assert_eq!(bundle.followups[0].prompt, "Follow up after restart");
        assert_eq!(
            bundle.attachments[1].text.as_deref(),
            Some("Approved original text")
        );
        assert!(!bundle.attachments[0].capture_image);
        assert_ne!(bundle.attachments[0].path, original_image);
        assert_eq!(
            temporary
                .store
                .read_draft_image(&bundle.attachments[0].path)
                .unwrap(),
            image
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&bundle.attachments[0].path)
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
        assert_eq!(
            std::fs::read_dir(temporary.store.asset_root())
                .unwrap()
                .count(),
            1
        );
        let mut legacy = serde_json::to_value(Envelope {
            schema_version: 1,
            snapshot: restored,
        })
        .unwrap();
        legacy.as_object_mut().unwrap().remove("draft_bundles");
        std::fs::write(temporary.store.path(), serde_json::to_vec(&legacy).unwrap()).unwrap();
        let migrated = temporary.store.load(3).unwrap();
        assert_eq!(migrated.drafts[&id], "Final keystroke");
        assert!(migrated.draft_bundles.is_empty());
    }

    // SDTEST-2837 / SDUC-564 — untrusted descriptors cannot cause arbitrary file
    // reads on save/reload; explicit review rejects missing/replaced image data.
    #[test]
    fn restored_draft_images_reject_escape_missing_and_changed_content() {
        let temporary = TempStore::new();
        let mut snapshot = AgentSessionSnapshot::default();
        let id = snapshot.collection.create("Draft", context(), 1).unwrap();
        snapshot
            .drafts
            .insert(id, "Keep text despite broken image".into());
        let external = temporary.store.path().with_file_name("external.txt");
        std::fs::write(&external, "must never be copied").unwrap();
        snapshot.draft_bundles.insert(
            id,
            AgentDraftBundle {
                version: AGENT_DRAFT_SCHEMA_VERSION,
                context: context(),
                effort: None,
                followups: vec![],
                attachments: vec![AgentDraftAttachment {
                    path: external.clone(),
                    name: "Untrusted.png".into(),
                    text: None,
                    image_digest: None,
                    capture_image: false,
                }],
            },
        );
        temporary.store.save(&snapshot).unwrap();
        assert!(!temporary.store.asset_root().exists());
        let restored = temporary.store.load(2).unwrap();
        assert_eq!(restored.drafts[&id], "Keep text despite broken image");
        assert!(restored.draft_bundles[&id].attachments[0]
            .path
            .as_os_str()
            .is_empty());
        assert!(!restored.warnings.is_empty());
        assert!(temporary.store.read_draft_image(&external).is_err());
        let fake_asset = temporary
            .store
            .asset_root()
            .join(format!("{}.image", "0".repeat(64)));
        assert!(temporary.store.read_draft_image(&fake_asset).is_err());
        std::fs::create_dir_all(temporary.store.asset_root()).unwrap();
        std::fs::write(&fake_asset, "changed content").unwrap();
        assert!(temporary.store.read_draft_image(&fake_asset).is_err());
        #[cfg(unix)]
        {
            std::fs::remove_file(&fake_asset).unwrap();
            std::os::unix::fs::symlink(&external, &fake_asset).unwrap();
            assert!(temporary.store.read_draft_image(&fake_asset).is_err());
        }
        // Wire data cannot grant capture authority even if someone injects the field.
        let descriptor: AgentDraftAttachment = serde_json::from_value(
            serde_json::json!({"path": &external, "name":"x", "text":null, "capture_image":true}),
        )
        .unwrap();
        assert!(!descriptor.capture_image);
        // A vanished or changed image must not block unrelated text/session saves.
        let attachment = &mut snapshot.draft_bundles.get_mut(&id).unwrap().attachments[0];
        attachment.capture_image = true;
        attachment.image_digest = Some(draft_image_digest(b"original preview"));
        let report = temporary.store.save_with_drafts(&snapshot).unwrap();
        assert!(report.unresolved_images.contains(&external));
        assert!(!report.warnings.is_empty());
        assert_eq!(
            temporary.store.load(4).unwrap().drafts[&id],
            "Keep text despite broken image"
        );
        std::fs::remove_file(&external).unwrap();
        snapshot.drafts.insert(id, "New unrelated keystroke".into());
        let report = temporary.store.save_with_drafts(&snapshot).unwrap();
        assert!(report.unresolved_images.contains(&external));
        assert_eq!(
            temporary.store.load(5).unwrap().drafts[&id],
            "New unrelated keystroke"
        );
    }

    // SDTEST-2817 — SDUC-563: damaged optional remote pointers cannot destroy
    // local history or restore execution authority from a saved library.
    #[test]
    fn sdtest_2817_invalid_remote_references_preserve_local_transcript() {
        let temporary = TempStore::new();
        let mut snapshot = AgentSessionSnapshot::default();
        let remote_context = AgentExecutionContext {
            target: AgentTarget::Ssh {
                connection_id: Uuid::new_v4(),
                connection_label: "server".into(),
            },
            workdir: "/srv/project".into(),
            ..context()
        };
        let id = snapshot
            .collection
            .reconnect_remote("Remote", remote_context, "native-id", 1)
            .unwrap();
        snapshot.collection.get_mut(id).unwrap().apply_stream_event(
            AgentStreamEvent::Text("Keep this reply".into()),
            2,
            true,
        );
        temporary.store.save(&snapshot).unwrap();
        let loaded = temporary.store.load(3).unwrap();
        assert!(loaded.collection.get(id).unwrap().remote_history.is_some());
        assert!(loaded
            .collection
            .get(id)
            .unwrap()
            .run_request("hello")
            .resume_session
            .is_none());
        let original: serde_json::Value =
            serde_json::from_slice(&std::fs::read(temporary.store.path()).unwrap()).unwrap();
        for broken in [
            serde_json::json!({"connection_id": "bad uuid"}),
            serde_json::json!(42),
            {
                let mut reference =
                    serde_json::to_value(&snapshot.collection.get(id).unwrap().remote_history)
                        .unwrap();
                reference["workdir"] = serde_json::json!("/srv/../wrong");
                reference
            },
        ] {
            let mut saved = original.clone();
            saved["collection"]["sessions"][0]["remote_history"] = broken;
            std::fs::write(temporary.store.path(), serde_json::to_vec(&saved).unwrap()).unwrap();
            let loaded = temporary.store.load(4).unwrap();
            let session = loaded.collection.get(id).unwrap();
            assert!(session.remote_history.is_none());
            assert_eq!(session.messages[0].text, "Keep this reply");
            assert!(session.run_request("hello").resume_session.is_none());
            assert!(loaded
                .warnings
                .iter()
                .any(|warning| warning.contains("remote history reference")));
        }
    }

    // SDTEST-2744 — SDUC-550: old libraries keep conservative defaults;
    // navigator choices survive restart and malformed collapse lists are bounded.
    #[test]
    fn navigator_preferences_round_trip_default_and_bound_collapsed_project_ids() {
        let temporary = TempStore::new();
        let mut snapshot = AgentSessionSnapshot::default();
        snapshot
            .collection
            .create("Saved project", context(), 1)
            .unwrap();
        snapshot.preferences.collapsed_project_ids = vec![
            "catalog-project-one".into(),
            "catalog-project-two".into(),
            "pinned-sessions".into(),
            "chat-folder:local:0123456789abcdef".into(),
            "ssh-server:00000000-0000-0000-0000-0000000001a5".into(),
            "ssh-folder:00000000-0000-0000-0000-0000000001a5:0123456789abcdef".into(),
        ];
        snapshot.preferences.remote_sessions_expanded = true;
        snapshot.preferences.remote_sessions_available_only = true;
        snapshot.preferences.ssh_selected_server = Some(Uuid::new_v4());
        snapshot.preferences.ssh_sessions_collapsed = true;
        temporary.store.save(&snapshot).unwrap();
        let loaded = temporary.store.load(2).unwrap();
        assert_eq!(loaded.preferences, snapshot.preferences);

        let mut old: serde_json::Value =
            serde_json::from_slice(&std::fs::read(temporary.store.path()).unwrap()).unwrap();
        let preferences = old["preferences"].as_object_mut().unwrap();
        for name in [
            "collapsed_project_ids",
            "remote_sessions_expanded",
            "remote_sessions_available_only",
            "ssh_selected_server",
            "ssh_sessions_collapsed",
        ] {
            preferences.remove(name);
        }
        std::fs::write(temporary.store.path(), serde_json::to_vec(&old).unwrap()).unwrap();
        let restored = temporary.store.load(3).unwrap();
        assert!(restored.preferences.collapsed_project_ids.is_empty());
        assert!(!restored.preferences.remote_sessions_expanded);
        assert!(!restored.preferences.remote_sessions_available_only);
        assert!(restored.preferences.ssh_selected_server.is_none());
        assert!(!restored.preferences.ssh_sessions_collapsed);
        assert_eq!(restored.collection.sessions().len(), 1);

        snapshot.preferences.collapsed_project_ids =
            (0..300).map(|index| format!("project-{index}")).collect();
        snapshot.preferences.collapsed_project_ids.extend([
            "project-299".into(),
            String::new(),
            "invalid\nidentity".into(),
            "é".repeat(200),
        ]);
        temporary.store.save(&snapshot).unwrap();
        let bounded = temporary.store.load(4).unwrap();
        assert_eq!(
            bounded.preferences.collapsed_project_ids.len(),
            MAX_COLLAPSED_PROJECT_IDS
        );
        assert_eq!(
            bounded.preferences.collapsed_project_ids.first().unwrap(),
            "project-44"
        );
        assert_eq!(
            bounded.preferences.collapsed_project_ids.last().unwrap(),
            "project-299"
        );
        assert!(bounded
            .preferences
            .collapsed_project_ids
            .iter()
            .all(|id| valid_collapsed_project_id(id)));
    }

    struct TempStore {
        root: PathBuf,
        store: AgentSessionStore,
    }
    impl TempStore {
        fn new() -> Self {
            let root =
                std::env::temp_dir().join(format!("shelldeck-agent-history-{}", Uuid::new_v4()));
            std::fs::create_dir_all(&root).unwrap();
            Self {
                store: AgentSessionStore::new(root.join("sessions.json")),
                root,
            }
        }
    }
    impl Drop for TempStore {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }
    fn context() -> AgentExecutionContext {
        AgentExecutionContext {
            provider: AgentProvider::Claude,
            target: AgentTarget::Local,
            access: AgentAccessMode::ReadOnly,
            workdir: std::env::temp_dir().display().to_string(),
            model: None,
        }
    }

    // SDTEST-2800 — metadata must prove its originating turn after persistence.
    #[test]
    fn run_metadata_survives_only_with_matching_turn_and_context() {
        let temporary = TempStore::new();
        let mut snapshot = AgentSessionSnapshot::default();
        let id = snapshot
            .collection
            .create("Metadata", context(), 1)
            .unwrap();
        snapshot.collection.begin_run(id, "hello", 2).unwrap();
        snapshot
            .collection
            .apply_stream_event(
                id,
                AgentStreamEvent::Metadata(AgentRunMetadata {
                    model: Some("claude-test".into()),
                    ..Default::default()
                }),
                3,
            )
            .unwrap();
        snapshot
            .collection
            .finish(id, AgentSessionStatus::Completed, 4)
            .unwrap();
        temporary.store.save(&snapshot).unwrap();
        let loaded = temporary.store.load(5).unwrap();
        assert_eq!(
            loaded
                .collection
                .get(id)
                .unwrap()
                .metadata
                .as_ref()
                .unwrap()
                .model
                .as_deref(),
            Some("claude-test")
        );
        snapshot.collection.get_mut(id).unwrap().metadata_turn_id = None;
        temporary.store.save(&snapshot).unwrap();
        assert!(temporary
            .store
            .load(6)
            .unwrap()
            .collection
            .get(id)
            .unwrap()
            .metadata
            .is_none());
        let mut changed = context();
        changed.provider = AgentProvider::DeepSeek;
        let session = snapshot.collection.get_mut(id).unwrap();
        session.set_context(changed, 7).unwrap();
        assert!(session.metadata.is_none());
        assert!(session.usage.is_none());
    }

    // SDTEST-2200 — SDUC-550
    #[test]
    fn sdtest_2200_private_restart_recovery_preserves_history_draft_and_preferences() {
        let temporary = TempStore::new();
        let mut snapshot = AgentSessionSnapshot::default();
        let id = snapshot
            .collection
            .create("Investigate rendering", context(), 10)
            .unwrap();
        let run = snapshot
            .collection
            .begin_run(id, "Inspect the layout", 11)
            .unwrap();
        snapshot
            .collection
            .apply_stream_event(id, AgentStreamEvent::Ready, 12)
            .unwrap();
        snapshot
            .collection
            .apply_stream_event(
                id,
                AgentStreamEvent::Session("opaque-secret-provider-id".into()),
                13,
            )
            .unwrap();
        snapshot
            .collection
            .apply_stream_event(
                id,
                AgentStreamEvent::Text("Found the sidebar constraint".into()),
                14,
            )
            .unwrap();
        snapshot
            .drafts
            .insert(id, "Continue after checking the screenshot".into());
        snapshot.preferences.show_archived = true;
        snapshot.preferences.inspector_tab = "changes".into();
        temporary.store.save(&snapshot).unwrap();
        let file = std::fs::read_to_string(temporary.store.path()).unwrap();
        assert!(!file.contains("opaque-secret-provider-id"));
        let loaded = temporary.store.load(100).unwrap();
        let recovered = loaded.collection.get(id).unwrap();
        assert_eq!(recovered.status, AgentSessionStatus::Failed);
        assert_eq!(recovered.attention, AgentSessionAttention::NeedsAttention);
        assert_eq!(recovered.messages[0].turn_id, Some(run.id));
        assert!(recovered
            .messages
            .iter()
            .any(|message| message.text == "Found the sidebar constraint"));
        assert!(recovered
            .messages
            .iter()
            .any(|message| message.text.contains("interrupted")));
        assert_eq!(recovered.run_request("continue").resume_session, None);
        assert_eq!(
            recovered.boundaries.last().unwrap().reason,
            AgentHistoryBoundaryReason::ApplicationRestart
        );
        assert_eq!(loaded.drafts[&id], "Continue after checking the screenshot");
        assert!(loaded.preferences.show_archived);
        assert_eq!(loaded.preferences.inspector_tab, "changes");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(temporary.store.path())
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
        // Saving twice exercises replacement of an existing destination.
        temporary.store.save(&loaded).unwrap();
        assert_eq!(std::fs::read_dir(&temporary.root).unwrap().count(), 1);
    }

    // SDTEST-2201 — SDUC-550
    #[test]
    fn sdtest_2201_legacy_history_migrates_and_invalid_context_cannot_become_execution_authority() {
        let temporary = TempStore::new();
        let mut collection = AgentSessionCollection::default();
        let valid = collection.create("Legacy", context(), 1).unwrap();
        collection
            .begin_run(valid, "Find rendering defect", 2)
            .unwrap();
        collection
            .apply_stream_event(valid, AgentStreamEvent::Text("Legacy answer".into()), 3)
            .unwrap();
        collection
            .finish(valid, AgentSessionStatus::Completed, 4)
            .unwrap();
        let invalid = collection.create("Invalid target", context(), 5).unwrap();
        let mut legacy = serde_json::to_value(collection).unwrap();
        legacy["max_concurrent"] = serde_json::json!(9999);
        let sessions = legacy["sessions"].as_array_mut().unwrap();
        sessions[0].as_object_mut().unwrap().remove("turns");
        for message in sessions[0]["messages"].as_array_mut().unwrap() {
            let message = message.as_object_mut().unwrap();
            for field in ["provider", "sequence", "turn_id"] {
                message.remove(field);
            }
        }
        sessions[1]["context"]["workdir"] = serde_json::json!("relative/path");
        std::fs::write(temporary.store.path(), serde_json::to_vec(&legacy).unwrap()).unwrap();
        let loaded = temporary.store.load(50).unwrap();
        assert!(loaded.collection.get(invalid).is_none());
        assert_eq!(loaded.warnings.len(), 1);
        let valid = loaded.collection.get(valid).unwrap();
        assert_eq!(valid.messages.len(), 2);
        assert_eq!(valid.messages[1].text, "Legacy answer");
        assert!(valid
            .messages
            .iter()
            .all(|message| message.turn_id.is_none() && message.provider.is_none()));
        assert!(valid.messages[0].sequence < valid.messages[1].sequence);
        assert_eq!(
            loaded.collection.max_concurrent,
            MAX_CONCURRENT_AGENT_SESSIONS
        );
    }

    // SDTEST-2202 — SDUC-550
    #[test]
    fn sdtest_2202_corrupt_future_and_oversized_files_fail_without_overwriting_history() {
        let temporary = TempStore::new();
        for contents in [
            b"{broken".to_vec(),
            br#"{"schema_version":999,"collection":{}}"#.to_vec(),
        ] {
            std::fs::write(temporary.store.path(), &contents).unwrap();
            assert!(temporary.store.load(10).is_err());
            assert_eq!(std::fs::read(temporary.store.path()).unwrap(), contents);
        }
        let file = std::fs::File::create(temporary.store.path()).unwrap();
        file.set_len(MAX_AGENT_SESSION_FILE_BYTES as u64 + 1)
            .unwrap();
        assert!(temporary.store.load(10).is_err());
    }

    // SDTEST-2203 — SDUC-550
    #[test]
    fn sdtest_2203_loaded_transcripts_and_preferences_are_bounded_before_rendering() {
        let temporary = TempStore::new();
        let mut snapshot = AgentSessionSnapshot::default();
        let id = snapshot.collection.create("Bounds", context(), 1).unwrap();
        snapshot.collection.begin_run(id, "Inspect", 2).unwrap();
        snapshot
            .collection
            .finish(id, AgentSessionStatus::Completed, 3)
            .unwrap();
        let session = snapshot.collection.get_mut(id).unwrap();
        let mut message = session.messages[0].clone();
        message.text = "é".repeat(MAX_AGENT_MESSAGE_BYTES);
        message.sequence = u64::MAX;
        session.messages = vec![message; MAX_AGENT_MESSAGES_PER_SESSION + 10];
        snapshot.drafts.insert(
            id,
            "d".repeat(crate::agent_runtime::MAX_AGENT_PROMPT_BYTES + 1),
        );
        snapshot.preferences.navigator_width = 0;
        snapshot.preferences.inspector_tab = "untrusted".into();
        // The storage boundary enforces all bounds even for caller-built state.
        temporary.store.save(&snapshot).unwrap();
        assert!(
            std::fs::metadata(temporary.store.path()).unwrap().len()
                <= MAX_AGENT_SESSION_FILE_BYTES as u64
        );
        let loaded = temporary.store.load(50).unwrap();
        let loaded_session = loaded.collection.get(id).unwrap();
        assert!(loaded_session.messages.len() <= MAX_AGENT_MESSAGES_PER_SESSION);
        assert!(loaded_session
            .messages
            .iter()
            .all(|message| message.text.len() <= MAX_AGENT_MESSAGE_BYTES));
        assert_eq!(
            loaded_session
                .messages
                .iter()
                .map(|message| message.id)
                .collect::<HashSet<_>>()
                .len(),
            loaded_session.messages.len()
        );
        assert!(loaded.drafts[&id].len() <= crate::agent_runtime::MAX_AGENT_PROMPT_BYTES);
        assert_eq!(loaded.preferences.navigator_width, 180);
        assert_eq!(loaded.preferences.inspector_tab, "files");
    }
}
