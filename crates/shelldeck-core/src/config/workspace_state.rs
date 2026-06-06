use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use uuid::Uuid;

use crate::error::{Result, ShellDeckError};

/// Serialized workspace state for session persistence.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceState {
    pub tabs: Vec<TabState>,
    pub active_tab: usize,
    pub sidebar_visible: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TabState {
    pub id: String,
    pub title: String,
    #[serde(rename = "type")]
    pub tab_type: TabType,
    /// For SSH tabs: the connection ID to reconnect.
    pub connection_id: Option<Uuid>,
    /// For local tabs: the shell to use.
    pub shell: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TabType {
    #[serde(rename = "local")]
    Local,
    #[serde(rename = "ssh")]
    Ssh,
}

impl Default for WorkspaceState {
    fn default() -> Self {
        Self {
            tabs: Vec::new(),
            active_tab: 0,
            sidebar_visible: true,
        }
    }
}

impl WorkspaceState {
    /// Get the state file path.
    fn state_path() -> PathBuf {
        super::app_config::AppConfig::config_dir().join("workspace.json")
    }

    /// Save workspace state to disk.
    pub fn save(&self) -> Result<()> {
        self.save_to(&Self::state_path())
    }

    /// Load workspace state from disk.
    pub fn load() -> Result<Self> {
        Self::load_from(&Self::state_path())
    }

    /// Delete the saved state (used after successful restore).
    pub fn clear() -> Result<()> {
        Self::clear_at(&Self::state_path())
    }

    /// Save workspace state to a specific path atomically.
    pub(crate) fn save_to(&self, path: &Path) -> Result<()> {
        if let Some(dir) = path.parent() {
            if !dir.as_os_str().is_empty() && !dir.exists() {
                std::fs::create_dir_all(dir)?;
            }
        }
        let content = serde_json::to_string_pretty(self).map_err(|e| {
            ShellDeckError::Serialization(format!("Failed to serialize workspace state: {}", e))
        })?;
        crate::util::atomic_write(path, content.as_bytes())?;
        Ok(())
    }

    /// Load workspace state from a specific path, returning defaults if missing.
    pub(crate) fn load_from(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let content = std::fs::read_to_string(path)?;
        let state: Self = serde_json::from_str(&content).map_err(|e| {
            ShellDeckError::Serialization(format!("Failed to parse workspace state: {}", e))
        })?;
        Ok(state)
    }

    /// Delete the saved state at a specific path.
    pub(crate) fn clear_at(path: &Path) -> Result<()> {
        if path.exists() {
            std::fs::remove_file(path)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

    fn temp_path(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "shelldeck-wsstate-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir.join(name)
    }

    #[test]
    fn round_trip_with_tabs() {
        let path = temp_path("workspace.json");

        let conn_id = Uuid::new_v4();
        let state = WorkspaceState {
            tabs: vec![
                TabState {
                    id: "tab-1".to_string(),
                    title: "Local Shell".to_string(),
                    tab_type: TabType::Local,
                    connection_id: None,
                    shell: Some("/bin/bash".to_string()),
                },
                TabState {
                    id: "tab-2".to_string(),
                    title: "prod ssh".to_string(),
                    tab_type: TabType::Ssh,
                    connection_id: Some(conn_id),
                    shell: None,
                },
            ],
            active_tab: 1,
            sidebar_visible: false,
        };

        state.save_to(&path).expect("save_to");
        let loaded = WorkspaceState::load_from(&path).expect("load_from");

        assert_eq!(loaded.tabs.len(), 2);
        assert_eq!(loaded.active_tab, 1);
        assert!(!loaded.sidebar_visible);
        assert_eq!(loaded.tabs[0].id, "tab-1");
        assert!(matches!(loaded.tabs[0].tab_type, TabType::Local));
        assert_eq!(loaded.tabs[0].shell.as_deref(), Some("/bin/bash"));
        assert!(matches!(loaded.tabs[1].tab_type, TabType::Ssh));
        assert_eq!(loaded.tabs[1].connection_id, Some(conn_id));

        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }

    #[test]
    fn load_from_missing_returns_default() {
        let path = temp_path("workspace.json");
        assert!(!path.exists());

        let loaded = WorkspaceState::load_from(&path).expect("load_from");

        assert!(loaded.tabs.is_empty());
        assert_eq!(loaded.active_tab, 0);
        assert!(loaded.sidebar_visible);
        // Unlike config/store, missing workspace state does NOT create a file.
        assert!(!path.exists());

        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }

    #[test]
    fn clear_at_removes_file() {
        let path = temp_path("workspace.json");
        WorkspaceState::default().save_to(&path).expect("save_to");
        assert!(path.exists());

        WorkspaceState::clear_at(&path).expect("clear_at");
        assert!(!path.exists());
        // Clearing a missing file is a no-op, not an error.
        WorkspaceState::clear_at(&path).expect("clear_at idempotent");

        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }

    #[test]
    fn load_from_corrupt_returns_err() {
        let path = temp_path("workspace.json");
        std::fs::write(&path, b"not json at all <<<>>>").expect("seed garbage");

        let result = WorkspaceState::load_from(&path);
        assert!(result.is_err(), "corrupt state should error, not panic");

        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }
}
