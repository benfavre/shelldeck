//! Versioned, bounded, private disk storage for agent conversations.
//!
//! Drafts and presentation preferences deliberately live outside AppConfig so
//! saving a conversation can never replace account/session configuration.
use super::*;
use std::collections::{BTreeMap, HashSet};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

pub const AGENT_SESSION_SCHEMA_VERSION: u32 = 1;
pub const MAX_AGENT_SESSION_FILE_BYTES: usize = 16 * 1024 * 1024;
const MAX_RETAINED_CONTENT_BYTES: usize = 8 * 1024 * 1024;

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
}

impl Default for AgentConsolePreferences {
    fn default() -> Self {
        Self {
            show_archived: false,
            navigator_visible: true,
            inspector_visible: false,
            context_expanded: false,
            inspector_tab: "files".into(),
            navigator_width: 240,
            inspector_width: 280,
        }
    }
}

impl AgentConsolePreferences {
    fn normalize(&mut self) {
        self.navigator_width = self.navigator_width.clamp(180, 480);
        self.inspector_width = self.inspector_width.clamp(240, 720);
        if !matches!(
            self.inspector_tab.as_str(),
            "files" | "changes" | "sources" | "tools"
        ) {
            self.inspector_tab = "files".into();
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AgentSessionSnapshot {
    pub collection: AgentSessionCollection,
    pub drafts: BTreeMap<Uuid, String>,
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
            if envelope.schema_version != AGENT_SESSION_SCHEMA_VERSION {
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
        snapshot.collection.recover_interrupted(now_ms);
        Ok(snapshot)
    }

    pub fn save(&self, snapshot: &AgentSessionSnapshot) -> Result<()> {
        let mut snapshot = snapshot.clone();
        normalize_snapshot(&mut snapshot);
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
        private_atomic_write(&self.path, &contents)
    }
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
        session.metadata = session.metadata.take().map(AgentRunMetadata::bounded);
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
