use serde::{Deserialize, Serialize};
use std::path::PathBuf;
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
        let path = Self::state_path();
        let dir = path.parent().expect("state_path always has a parent directory");
        if !dir.exists() {
            std::fs::create_dir_all(dir)?;
        }
        let content = serde_json::to_string_pretty(self).map_err(|e| {
            ShellDeckError::Serialization(format!("Failed to serialize workspace state: {}", e))
        })?;
        std::fs::write(&path, content)?;
        Ok(())
    }

    /// Load workspace state from disk.
    pub fn load() -> Result<Self> {
        let path = Self::state_path();
        if !path.exists() {
            return Ok(Self::default());
        }
        let content = std::fs::read_to_string(&path)?;
        let state: Self = serde_json::from_str(&content).map_err(|e| {
            ShellDeckError::Serialization(format!("Failed to parse workspace state: {}", e))
        })?;
        Ok(state)
    }

    /// Delete the saved state (used after successful restore).
    pub fn clear() -> Result<()> {
        let path = Self::state_path();
        if path.exists() {
            std::fs::remove_file(&path)?;
        }
        Ok(())
    }
}
