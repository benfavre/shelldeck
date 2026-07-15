use crate::config::app_config::AppConfig;
use crate::error::{Result, ShellDeckError};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};
use uuid::Uuid;

const MAX_ACTIVITY_ENTRIES: usize = 500;
const MAX_ACTIVITY_BYTES: u64 = 2 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActivityKind {
    Terminal,
    Connection,
    Forward,
    Script,
    Support,
    Issue,
    Jean,
    Fleet,
    Site,
    Bext,
    Error,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActivityAction {
    #[default]
    None,
    OpenTerminal,
    OpenConnection,
    ConnectConnection,
    OpenForward,
    OpenScript,
    OpenSupport,
    OpenTicket,
    OpenIssue,
    OpenSite,
    OpenJean,
    OpenFleet,
    OpenBext,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActivityEntry {
    pub id: Uuid,
    pub at: DateTime<Utc>,
    pub kind: ActivityKind,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_label: Option<String>,
    #[serde(default)]
    pub action: ActivityAction,
}

impl ActivityEntry {
    pub fn new(kind: ActivityKind, message: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            at: Utc::now(),
            kind,
            message: message.into(),
            detail: None,
            target_id: None,
            target_label: None,
            action: ActivityAction::None,
        }
    }

    pub fn with_detail(mut self, detail: impl Into<String>) -> Self {
        let detail = detail.into();
        if !detail.trim().is_empty() {
            self.detail = Some(detail);
        }
        self
    }

    pub fn with_target(mut self, id: impl Into<String>, label: impl Into<String>) -> Self {
        self.target_id = Some(id.into());
        self.target_label = Some(label.into());
        self
    }

    pub fn with_action(mut self, action: ActivityAction) -> Self {
        self.action = action;
        self
    }
}

pub struct ActivityStore;

impl ActivityStore {
    pub fn activity_path() -> PathBuf {
        AppConfig::config_dir().join("activity.jsonl")
    }

    pub fn load_recent(limit: usize) -> Result<Vec<ActivityEntry>> {
        Self::load_recent_from(&Self::activity_path(), limit)
    }

    pub fn append(entry: &ActivityEntry) -> Result<()> {
        Self::append_to(&Self::activity_path(), entry)
    }

    pub(crate) fn load_recent_from(path: &Path, limit: usize) -> Result<Vec<ActivityEntry>> {
        if limit == 0 || !path.exists() {
            return Ok(Vec::new());
        }

        let content = std::fs::read_to_string(path)?;
        let mut entries = Vec::new();
        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            match serde_json::from_str::<ActivityEntry>(line) {
                Ok(entry) => entries.push(entry),
                Err(e) => {
                    tracing::warn!(
                        "Skipping malformed activity entry in {}: {}",
                        path.display(),
                        e
                    );
                }
            }
        }

        if entries.len() > limit {
            entries.drain(0..entries.len() - limit);
        }
        entries.reverse();
        Ok(entries)
    }

    pub(crate) fn append_to(path: &Path, entry: &ActivityEntry) -> Result<()> {
        if let Some(dir) = path.parent() {
            if !dir.as_os_str().is_empty() && !dir.exists() {
                std::fs::create_dir_all(dir)?;
            }
        }

        let line = serde_json::to_string(entry)
            .map_err(|e| ShellDeckError::Serialization(format!("activity serialize: {}", e)))?;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?;
        writeln!(file, "{}", line)?;

        if file.metadata()?.len() > MAX_ACTIVITY_BYTES {
            Self::compact_to(path, MAX_ACTIVITY_ENTRIES)?;
        }
        Ok(())
    }

    fn compact_to(path: &Path, limit: usize) -> Result<()> {
        let mut newest_first = Self::load_recent_from(path, limit)?;
        newest_first.reverse();
        let mut buf = Vec::new();
        for entry in newest_first {
            let line = serde_json::to_string(&entry)
                .map_err(|e| ShellDeckError::Serialization(format!("activity serialize: {}", e)))?;
            writeln!(&mut buf, "{}", line)?;
        }
        crate::util::atomic_write(path, &buf)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

    fn temp_path(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "shelldeck-activity-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir.join(name)
    }

    fn entry(n: i64) -> ActivityEntry {
        let mut e = ActivityEntry::new(ActivityKind::Terminal, format!("event {n}"))
            .with_action(ActivityAction::OpenTerminal);
        e.at = Utc.timestamp_millis_opt(n).single().unwrap();
        e
    }

    // SDTEST-1330
    #[test]
    fn append_to_and_load_recent_return_newest_first_with_limit() {
        let path = temp_path("activity.jsonl");
        let a = entry(1000);
        let b = entry(2000);
        let c = entry(3000);

        ActivityStore::append_to(&path, &a).expect("append a");
        ActivityStore::append_to(&path, &b).expect("append b");
        ActivityStore::append_to(&path, &c).expect("append c");

        let loaded = ActivityStore::load_recent_from(&path, 2).expect("load");
        assert_eq!(
            loaded
                .iter()
                .map(|e| e.message.as_str())
                .collect::<Vec<_>>(),
            vec!["event 3000", "event 2000"]
        );

        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }

    // SDTEST-1331
    #[test]
    fn load_recent_ignores_blank_and_malformed_lines() {
        let path = temp_path("activity.jsonl");
        let good = serde_json::to_string(&entry(42)).unwrap();
        std::fs::write(&path, format!("\nnot-json\n{}\n", good)).unwrap();

        let loaded = ActivityStore::load_recent_from(&path, 10).expect("load");
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].message, "event 42");

        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }

    // SDTEST-1332
    #[test]
    fn old_entries_without_action_default_to_none() {
        let raw = r#"{
            "id":"00000000-0000-0000-0000-000000000001",
            "at":"2026-07-15T10:00:00Z",
            "kind":"site",
            "message":"Site activé"
        }"#;

        let parsed: ActivityEntry = serde_json::from_str(raw).expect("parse old entry");
        assert_eq!(parsed.action, ActivityAction::None);
        assert_eq!(parsed.kind, ActivityKind::Site);
    }
}
