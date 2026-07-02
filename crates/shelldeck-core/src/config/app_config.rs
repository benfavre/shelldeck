use crate::config::cloud_sync::CloudSyncConfig;
use crate::error::{Result, ShellDeckError};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tracing::info;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AppConfig {
    pub theme: ThemePreference,
    pub terminal: TerminalConfig,
    pub general: GeneralConfig,
    /// Cloud Sync (Inklura Manage). `#[serde(default)]` keeps existing
    /// `shelldeck.toml` files without a `[cloud_sync]` section parsing cleanly.
    #[serde(default)]
    pub cloud_sync: CloudSyncConfig,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ThemePreference {
    #[default]
    Dark,
    Light,
    System,
    Dracula,
    Nord,
    TokyoNight,
    GruvboxDark,
    SolarizedDark,
    SolarizedLight,
    CatppuccinMocha,
    OneDark,
    Monokai,
    RosePine,
}

impl ThemePreference {
    /// All selectable themes, in display order.
    pub fn all() -> &'static [ThemePreference] {
        use ThemePreference::*;
        &[
            Dark,
            Light,
            System,
            Dracula,
            Nord,
            TokyoNight,
            GruvboxDark,
            SolarizedDark,
            SolarizedLight,
            CatppuccinMocha,
            OneDark,
            Monokai,
            RosePine,
        ]
    }

    /// Human-friendly display name.
    pub fn display_name(&self) -> &'static str {
        use ThemePreference::*;
        match self {
            Dark => "Dark",
            Light => "Light",
            System => "System",
            Dracula => "Dracula",
            Nord => "Nord",
            TokyoNight => "Tokyo Night",
            GruvboxDark => "Gruvbox Dark",
            SolarizedDark => "Solarized Dark",
            SolarizedLight => "Solarized Light",
            CatppuccinMocha => "Catppuccin Mocha",
            OneDark => "One Dark",
            Monokai => "Monokai",
            RosePine => "Rosé Pine",
        }
    }

    /// Whether this theme uses a dark base (drives the adabraka-ui component
    /// theme and any light/dark-conditional UI). `System` follows dark for now.
    pub fn is_dark(&self) -> bool {
        use ThemePreference::*;
        !matches!(self, Light | SolarizedLight)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct TerminalConfig {
    pub font_family: String,
    pub font_size: f32,
    pub scrollback_lines: usize,
    pub default_shell: Option<String>,
    pub cursor_style: String,
    pub cursor_blink: bool,
    /// Name of the active terminal color theme (matches a `TerminalTheme`
    /// built-in name, e.g. "Dark", "Light", "Pastel Dark", "High Contrast").
    pub theme: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct GeneralConfig {
    pub auto_connect_on_startup: bool,
    pub show_notifications: bool,
    pub confirm_before_close: bool,
    pub sidebar_width: f32,
    pub auto_attach_tmux: bool,
    pub auto_update: bool,
    /// Font family for the application UI (sidebar, dashboard, forms, etc.).
    pub ui_font_family: String,
    /// Base font size in pixels for the application UI.
    pub ui_font_size: f32,
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
            theme: "Dark".to_string(),
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
            auto_update: true,
            ui_font_family: "System Default".to_string(),
            ui_font_size: 14.0,
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
                if let Some(home) = crate::util::home_dir() {
                    home.join(".config").join("shelldeck")
                } else {
                    tracing::warn!("HOME not set and ProjectDirs unavailable; using current dir for config");
                    PathBuf::from(".shelldeck")
                }
            }
        }
    }

    /// Get the config file path.
    pub fn config_path() -> PathBuf {
        Self::config_dir().join("config.toml")
    }

    /// Load config from disk, or create and save defaults.
    pub fn load() -> Result<Self> {
        Self::load_from(&Self::config_path())
    }

    /// Save config to disk.
    pub fn save(&self) -> Result<()> {
        self.save_to(&Self::config_path())
    }

    /// Load config from a specific path, or create and save defaults there.
    pub(crate) fn load_from(path: &Path) -> Result<Self> {
        if path.exists() {
            let content = std::fs::read_to_string(path)?;
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
            config.save_to(path)?;
            info!("Created default config at {}", path.display());
            Ok(config)
        }
    }

    /// Save config to a specific path atomically.
    pub(crate) fn save_to(&self, path: &Path) -> Result<()> {
        if let Some(dir) = path.parent() {
            if !dir.as_os_str().is_empty() && !dir.exists() {
                std::fs::create_dir_all(dir)?;
            }
        }

        let content = toml::to_string_pretty(self).map_err(|e| {
            ShellDeckError::Serialization(format!("Failed to serialize config: {}", e))
        })?;
        crate::util::atomic_write(path, content.as_bytes())?;
        info!("Saved config to {}", path.display());

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

    fn temp_path(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "shelldeck-appconfig-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir.join(name)
    }

    #[test]
    fn round_trip_non_default() {
        let path = temp_path("config.toml");

        let mut config = AppConfig::default();
        config.theme = ThemePreference::Light;
        config.terminal.font_family = "Fira Code".to_string();
        config.terminal.font_size = 17.5;
        config.terminal.scrollback_lines = 42;
        config.terminal.default_shell = Some("/bin/zsh".to_string());
        config.general.sidebar_width = 333.0;
        config.general.auto_connect_on_startup = true;
        config.general.ui_font_family = "Inter".to_string();

        config.save_to(&path).expect("save_to");
        let loaded = AppConfig::load_from(&path).expect("load_from");

        assert_eq!(loaded.theme, ThemePreference::Light);
        assert_eq!(loaded.terminal.font_family, "Fira Code");
        assert_eq!(loaded.terminal.font_size, 17.5);
        assert_eq!(loaded.terminal.scrollback_lines, 42);
        assert_eq!(loaded.terminal.default_shell.as_deref(), Some("/bin/zsh"));
        assert_eq!(loaded.general.sidebar_width, 333.0);
        assert!(loaded.general.auto_connect_on_startup);
        assert_eq!(loaded.general.ui_font_family, "Inter");

        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }

    #[test]
    fn cloud_sync_round_trips() {
        let path = temp_path("config.toml");

        let mut config = AppConfig::default();
        config.cloud_sync.enabled = true;
        config.cloud_sync.token = "sd_secret".to_string();
        config.cloud_sync.base_url = "https://example.test".to_string();
        config.cloud_sync.sync_on_startup = false;

        config.save_to(&path).expect("save_to");
        let loaded = AppConfig::load_from(&path).expect("load_from");

        assert!(loaded.cloud_sync.enabled);
        assert_eq!(loaded.cloud_sync.token, "sd_secret");
        assert_eq!(loaded.cloud_sync.base_url, "https://example.test");
        assert!(!loaded.cloud_sync.sync_on_startup);

        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }

    #[test]
    fn config_without_cloud_sync_section_still_parses() {
        // Simulates an older shelldeck.toml written before Cloud Sync existed.
        let path = temp_path("config.toml");
        let legacy = r#"
theme = "Dark"

[terminal]
font_family = "JetBrains Mono"
font_size = 14.0
scrollback_lines = 10000
cursor_style = "block"
cursor_blink = true
theme = "Dark"

[general]
auto_connect_on_startup = false
show_notifications = true
confirm_before_close = true
sidebar_width = 260.0
auto_attach_tmux = false
auto_update = true
ui_font_family = "System Default"
ui_font_size = 14.0
"#;
        std::fs::write(&path, legacy).expect("seed legacy config");

        let loaded = AppConfig::load_from(&path).expect("legacy config should parse");
        // Cloud Sync falls back to defaults.
        assert!(!loaded.cloud_sync.enabled);
        assert_eq!(loaded.cloud_sync.base_url, "https://manage.inklura.fr");
        assert!(loaded.cloud_sync.token.is_empty());
        assert!(loaded.cloud_sync.sync_on_startup);

        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }

    #[test]
    fn load_from_missing_creates_defaults() {
        let path = temp_path("config.toml");
        assert!(!path.exists());

        let loaded = AppConfig::load_from(&path).expect("load_from");

        // Defaults round-tripped, and the file now exists.
        assert_eq!(loaded.theme, ThemePreference::Dark);
        assert_eq!(loaded.terminal.font_family, "JetBrains Mono");
        assert!(path.exists(), "load_from should create the file");

        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }

    #[test]
    fn load_from_corrupt_returns_err() {
        let path = temp_path("config.toml");
        std::fs::write(&path, b"\xff\xfe not = valid = toml ][[[").expect("seed garbage");

        let result = AppConfig::load_from(&path);
        assert!(result.is_err(), "corrupt config should error, not panic");

        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }
}
