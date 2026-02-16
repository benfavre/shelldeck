use crate::error::{Result, ShellDeckError};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tracing::info;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AppConfig {
    pub theme: ThemePreference,
    pub terminal: TerminalConfig,
    pub general: GeneralConfig,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ThemePreference {
    #[default]
    Dark,
    Light,
    System,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TerminalConfig {
    pub font_family: String,
    pub font_size: f32,
    pub scrollback_lines: usize,
    pub default_shell: Option<String>,
    pub cursor_style: String,
    pub cursor_blink: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeneralConfig {
    pub auto_connect_on_startup: bool,
    pub show_notifications: bool,
    pub confirm_before_close: bool,
    pub sidebar_width: f32,
    pub auto_attach_tmux: bool,
}

impl Default for TerminalConfig {
    fn default() -> Self {
        Self {
            font_family: "JetBrains Mono".to_string(),
            font_size: 14.0,
            scrollback_lines: 10000,
            default_shell: None,
            cursor_style: "block".to_string(),
            cursor_blink: true,
        }
    }
}

impl Default for GeneralConfig {
    fn default() -> Self {
        Self {
            auto_connect_on_startup: false,
            show_notifications: true,
            confirm_before_close: true,
            sidebar_width: 260.0,
            auto_attach_tmux: false,
        }
    }
}

impl AppConfig {
    /// Get the project directories for ShellDeck.
    fn project_dirs() -> Result<ProjectDirs> {
        ProjectDirs::from("com", "shelldeck", "ShellDeck").ok_or_else(|| {
            ShellDeckError::Config("Could not determine config directory".to_string())
        })
    }

    /// Get the config directory path.
    pub fn config_dir() -> PathBuf {
        match Self::project_dirs() {
            Ok(dirs) => dirs.config_dir().to_path_buf(),
            Err(_) => {
                let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
                PathBuf::from(home).join(".config").join("shelldeck")
            }
        }
    }

    /// Get the config file path.
    pub fn config_path() -> PathBuf {
        Self::config_dir().join("config.toml")
    }

    /// Load config from disk, or create and save defaults.
    pub fn load() -> Result<Self> {
        let path = Self::config_path();

        if path.exists() {
            let content = std::fs::read_to_string(&path)?;
            let config: Self = toml::from_str(&content).map_err(|e| {
                ShellDeckError::Config(format!(
                    "Failed to parse config at {}: {}",
                    path.display(),
                    e
                ))
            })?;
            info!("Loaded config from {}", path.display());
            Ok(config)
        } else {
            let config = Self::default();
            config.save()?;
            info!("Created default config at {}", path.display());
            Ok(config)
        }
    }

    /// Save config to disk.
    pub fn save(&self) -> Result<()> {
        let path = Self::config_path();
        let dir = Self::config_dir();

        if !dir.exists() {
            std::fs::create_dir_all(&dir)?;
        }

        let content = toml::to_string_pretty(self).map_err(|e| {
            ShellDeckError::Serialization(format!("Failed to serialize config: {}", e))
        })?;
        std::fs::write(&path, content)?;
        info!("Saved config to {}", path.display());

        Ok(())
    }
}
