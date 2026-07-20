use crate::config::cloud_account::AccountInfo;
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
    /// `[ai]` — opt-in contextual assistant backend and per-surface controls.
    /// API credentials are stored separately in the OS keychain.
    #[serde(default)]
    pub ai: crate::ai::AiConfig,
    /// `[editor]` — code editor preferences (font, indent, wrap, gutter…).
    /// `#[serde(default)]` keeps existing `shelldeck.toml` files without an
    /// `[editor]` section parsing cleanly.
    #[serde(default)]
    pub editor: EditorConfig,
    /// Cloud Sync (Inklura Manage). `#[serde(default)]` keeps existing
    /// `shelldeck.toml` files without a `[cloud_sync]` section parsing cleanly.
    #[serde(default)]
    pub cloud_sync: CloudSyncConfig,
    /// Signed-in Inklura Manage account. `None`/absent = logged out. Written
    /// on login, cleared on logout. `skip_serializing_if` keeps the `[account]`
    /// table out of the file entirely when logged out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<AccountInfo>,
    /// Local `[jeanclaude]` override (url/user/pass). When set, takes precedence
    /// over the server-delivered config (e.g. to point at an SSH tunnel on
    /// 127.0.0.1). Absent → use the server-delivered config.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jeanclaude: Option<crate::config::jeanclaude::JeanConfig>,
    /// `[jean_runtime]` — whether this machine hosts a Jean fleet runtime.
    /// `#[serde(default)]` keeps older configs parsing; `enabled` defaults false.
    #[serde(default)]
    pub jean_runtime: crate::config::jean_fleet::JeanRuntimeConfig,
    /// `[bext_cloud]` — connection to the cloud.bext.dev control plane. Empty
    /// token = not connected; `#[serde(default)]` keeps older configs parsing.
    #[serde(default)]
    pub bext_cloud: crate::config::bext_cloud::BextCloudConfig,
    /// `[tray]` — system-tray preferences (close-to-tray + per-category
    /// notification opt-in). `#[serde(default)]` keeps older configs
    /// without a `[tray]` section parsing cleanly.
    #[serde(default)]
    pub tray: TrayConfig,
    /// Connection ids shown in the sidebar and system-tray quick-access
    /// sections. Order is user-defined and preserved across sessions.
    #[serde(default)]
    pub pinned_connections: Vec<uuid::Uuid>,
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

    /// Filesystem slug used by the brand kit (`brand/svg/themes/{slug}-…`,
    /// `brand/png/themes/monolith-{slug}-…`). Kept in sync with
    /// `scripts/export-monolith-brand.py`. `System` follows the dark asset.
    pub fn brand_slug(&self) -> &'static str {
        use ThemePreference::*;
        match self {
            Dark | System => "dark",
            Light => "light",
            Dracula => "dracula",
            Nord => "nord",
            TokyoNight => "tokyo-night",
            GruvboxDark => "gruvbox-dark",
            SolarizedDark => "solarized-dark",
            SolarizedLight => "solarized-light",
            CatppuccinMocha => "catppuccin-mocha",
            OneDark => "one-dark",
            Monokai => "monokai",
            RosePine => "rose-pine",
        }
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

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UiLanguage {
    /// Follow the OS locale (`fr*` → French, otherwise English). Unknown → French.
    #[default]
    System,
    Fr,
    En,
}

impl UiLanguage {
    pub fn all() -> &'static [UiLanguage] {
        use UiLanguage::*;
        &[System, Fr, En]
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct GeneralConfig {
    pub auto_connect_on_startup: bool,
    pub show_notifications: bool,
    pub confirm_before_close: bool,
    pub sidebar_width: f32,
    /// Whether the sidebar's top navigation section (Connections / Terminals
    /// / Scripts / … / Settings) is collapsed. Persisted so the layout the
    /// user picks sticks across sessions.
    #[serde(default)]
    pub sidebar_nav_collapsed: bool,
    pub auto_attach_tmux: bool,
    pub auto_update: bool,
    /// Interface language. `system` follows the OS locale (French default).
    #[serde(default)]
    pub ui_language: UiLanguage,
    /// Font family for the application UI (sidebar, dashboard, forms, etc.).
    pub ui_font_family: String,
    /// Base font size in pixels for the application UI.
    pub ui_font_size: f32,
    /// Launch ShellDeck automatically at OS login. Applied via
    /// `shelldeck_core::config::autostart` (cross-platform: XDG desktop
    /// entry on Linux, `launchd` login item on macOS, HKCU Run key on
    /// Windows). `false` by default — opt-in.
    #[serde(default)]
    pub autostart: bool,
    /// Whether the post-login onboarding tour has been completed (or
    /// skipped). `false` by default — first successful login shows the
    /// tour; replayable from Settings → Général.
    #[serde(default)]
    pub onboarding_completed: bool,
}

/// System-tray preferences. Per-category opt-in on the OS notifications
/// fired from workspace state deltas + `close_to_tray` for
/// close-button-hides-to-tray behaviour. Every field defaults to a
/// safe/opt-in setting so a fresh install doesn't burst notifications
/// but the tray + close-quit still work.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct TrayConfig {
    /// When true and the tray is up, clicking the window close button
    /// hides the window instead of quitting. The user can still quit
    /// from the tray "Quitter" item or Cmd/Ctrl+Q. Defaults to
    /// **false** — traditional close-quits behaviour — because a
    /// silent minimize-to-tray surprises users who don't know the tray
    /// icon is there. Users opt in via Settings.
    pub close_to_tray: bool,
    /// Show an OS notification when new unread support tickets arrive.
    pub notify_new_tickets: bool,
    /// Show an OS notification when Jean fleet jobs need user
    /// confirmation.
    pub notify_jean_pending: bool,
    /// Show an OS notification when previously-active SSH sessions
    /// drop.
    pub notify_ssh_disconnect: bool,
    /// Show an OS notification when a Fleet job finishes (either
    /// success or failure). Especially useful in `auto` runtime mode.
    pub notify_fleet_done: bool,
    /// Show an OS notification when an AI task completes while the
    /// ShellDeck window is not active.
    pub notify_ai_tasks: bool,
}

impl Default for TrayConfig {
    fn default() -> Self {
        Self {
            close_to_tray: false,
            // Notifications on by default — the whole point of Phase C
            // is to catch state changes while the window is hidden.
            // Users can mute any category from Settings → Général.
            notify_new_tickets: true,
            notify_jean_pending: true,
            notify_ssh_disconnect: true,
            notify_fleet_done: true,
            notify_ai_tasks: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct EditorConfig {
    /// Editor font family (must be monospace — the paint loop assumes fixed
    /// cell width). "System Default" falls back to the app's UI font family.
    pub font_family: String,
    /// Editor base font size in pixels. Per-tab zoom stacks on top and is
    /// not persisted here.
    pub font_size: f32,
    /// Line-height multiplier applied to the font size (1.4 ≈ VS Code default).
    pub line_height: f32,
    /// Visible width of a tab, in columns.
    pub tab_size: usize,
    /// When true, pressing Tab inserts spaces; when false, a real `\t`.
    pub insert_spaces: bool,
    /// Show the gutter with line numbers.
    pub show_line_numbers: bool,
    /// Render whitespace glyphs (space dots, tab arrows).
    pub show_whitespace: bool,
    /// Soft-wrap long lines at `word_wrap_column`.
    pub word_wrap: bool,
    /// Wrap column when `word_wrap` is on.
    pub word_wrap_column: usize,
    /// Blink the primary cursor.
    pub cursor_blink: bool,
}

impl Default for EditorConfig {
    fn default() -> Self {
        Self {
            font_family: "JetBrains Mono".to_string(),
            font_size: 14.0,
            // VS Code / Zed default — feels aired-out, matches the reference
            // look the maintainer targets. Tighter values (1.2..1.4) work but
            // start to feel cramped past 15px.
            line_height: 1.5,
            tab_size: 4,
            insert_spaces: true,
            show_line_numbers: true,
            show_whitespace: false,
            word_wrap: false,
            word_wrap_column: 120,
            cursor_blink: true,
        }
    }
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
            sidebar_nav_collapsed: false,
            auto_attach_tmux: false,
            auto_update: true,
            ui_language: UiLanguage::default(),
            ui_font_family: "System Default".to_string(),
            ui_font_size: 14.0,
            autostart: false,
            onboarding_completed: false,
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
                    tracing::warn!(
                        "HOME not set and ProjectDirs unavailable; using current dir for config"
                    );
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
        let pinned_id = uuid::Uuid::new_v4();
        config.pinned_connections.push(pinned_id);

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
        assert_eq!(loaded.pinned_connections, vec![pinned_id]);

        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }

    #[test]
    fn older_config_defaults_pinned_connections_to_empty() {
        let config: AppConfig = toml::from_str(
            r#"
theme = "Dark"

[terminal]

[general]
"#,
        )
        .expect("parse config without pinned_connections");

        assert!(config.pinned_connections.is_empty());
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
    fn account_round_trips_and_omits_when_logged_out() {
        let path = temp_path("config.toml");

        // Logged out: no [account] table should be written, and it reloads None.
        let logged_out = AppConfig::default();
        logged_out.save_to(&path).expect("save_to");
        let serialized = std::fs::read_to_string(&path).unwrap();
        assert!(
            !serialized.contains("[account]"),
            "logged-out config must not emit [account]"
        );
        let loaded = AppConfig::load_from(&path).expect("load_from");
        assert!(loaded.account.is_none());

        // Logged in: round-trips the identity.
        let mut logged_in = AppConfig::default();
        logged_in.account = Some(AccountInfo {
            email: "ben@webdesign29.net".to_string(),
            name: "Ben Favre".to_string(),
            is_superadmin: true,
            is_admin: true,
            is_inklura_support: true,
            roles: vec!["superadmin".to_string()],
        });
        logged_in.save_to(&path).expect("save_to");
        let loaded = AppConfig::load_from(&path).expect("load_from");
        let acct = loaded.account.expect("account present");
        assert_eq!(acct.email, "ben@webdesign29.net");
        assert_eq!(acct.name, "Ben Favre");
        assert!(acct.is_superadmin, "is_superadmin should round-trip");

        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }

    #[test]
    fn jeanclaude_override_round_trips_and_omits_when_unset() {
        use crate::config::jeanclaude::JeanConfig;
        let path = temp_path("config.toml");

        // Unset: no [jeanclaude] table emitted, reloads None.
        AppConfig::default().save_to(&path).expect("save");
        let serialized = std::fs::read_to_string(&path).unwrap();
        assert!(!serialized.contains("[jeanclaude]"));
        assert!(AppConfig::load_from(&path).unwrap().jeanclaude.is_none());

        // Set: round-trips the local override.
        let mut cfg = AppConfig::default();
        cfg.jeanclaude = Some(JeanConfig {
            url: "http://127.0.0.1:3100".into(),
            user: "jean".into(),
            pass: "x".into(),
        });
        cfg.save_to(&path).expect("save");
        let loaded = AppConfig::load_from(&path)
            .unwrap()
            .jeanclaude
            .expect("present");
        assert_eq!(loaded.url, "http://127.0.0.1:3100");
        assert_eq!(loaded.user, "jean");

        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }

    #[test]
    fn jean_runtime_round_trips_and_defaults_off() {
        let path = temp_path("config.toml");

        // Default: enabled=false, no instance id.
        AppConfig::default().save_to(&path).expect("save");
        let loaded = AppConfig::load_from(&path).expect("load");
        assert!(!loaded.jean_runtime.enabled);
        assert!(loaded.jean_runtime.instance_id.is_none());

        // Round-trip a registered runtime.
        let mut cfg = AppConfig::default();
        cfg.jean_runtime.enabled = true;
        cfg.jean_runtime.instance_id = Some("4365eee9".to_string());
        cfg.jean_runtime.workdir = Some("/home/x/infra".to_string());
        cfg.save_to(&path).expect("save");
        let loaded = AppConfig::load_from(&path).expect("load");
        assert!(loaded.jean_runtime.enabled);
        assert_eq!(loaded.jean_runtime.instance_id.as_deref(), Some("4365eee9"));
        assert_eq!(
            loaded.jean_runtime.workdir.as_deref(),
            Some("/home/x/infra")
        );

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
        // Cloud Sync + account fall back to defaults (logged out).
        assert!(!loaded.cloud_sync.enabled);
        assert_eq!(loaded.cloud_sync.base_url, "https://manage.inklura.fr");
        assert!(loaded.cloud_sync.token.is_empty());
        assert!(loaded.cloud_sync.sync_on_startup);
        assert!(loaded.account.is_none());
        assert!(!loaded.ai.enabled);
        assert_eq!(loaded.ai.backend, crate::ai::AiBackend::Disabled);

        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }

    // SDTEST-1342
    #[test]
    fn ai_config_round_trips_without_any_credential_field() {
        let mut config = AppConfig::default();
        config.ai.enabled = true;
        config.ai.backend = crate::ai::AiBackend::OpenAi;
        config.ai.model = "gpt-test".to_string();
        config.ai.surfaces.terminal = false;

        let serialized = toml::to_string_pretty(&config).expect("serialize AI config");
        assert!(serialized.contains("[ai]"));
        assert!(serialized.contains("backend = \"open_ai\""));
        assert!(!serialized.contains("api_key"));
        assert!(!serialized.contains("password"));
        assert!(!serialized.contains("secret"));

        let loaded: AppConfig = toml::from_str(&serialized).expect("reload AI config");
        assert!(loaded.ai.enabled);
        assert_eq!(loaded.ai.backend, crate::ai::AiBackend::OpenAi);
        assert_eq!(loaded.ai.model, "gpt-test");
        assert!(!loaded.ai.surfaces.terminal);
    }

    #[test]
    fn editor_config_round_trips_and_defaults_apply() {
        // Defaults are applied when the section is absent, and a round-trip
        // preserves every field. Keeps the wire format stable for older
        // shelldeck.toml files that predate `[editor]`.
        let path = temp_path("config.toml");
        AppConfig::default().save_to(&path).expect("save");
        let loaded = AppConfig::load_from(&path).expect("load");
        assert_eq!(loaded.editor.font_family, "JetBrains Mono");
        assert!((loaded.editor.font_size - 14.0).abs() < f32::EPSILON);
        assert_eq!(loaded.editor.tab_size, 4);
        assert!(loaded.editor.insert_spaces);
        assert!(loaded.editor.show_line_numbers);
        assert!(!loaded.editor.word_wrap);

        // Round-trip a customised editor block.
        let mut cfg = AppConfig::default();
        cfg.editor.font_family = "Fira Code".to_string();
        cfg.editor.font_size = 16.0;
        cfg.editor.tab_size = 2;
        cfg.editor.insert_spaces = false;
        cfg.editor.word_wrap = true;
        cfg.editor.word_wrap_column = 80;
        cfg.save_to(&path).expect("save");
        let loaded = AppConfig::load_from(&path).expect("load");
        assert_eq!(loaded.editor.font_family, "Fira Code");
        assert_eq!(loaded.editor.tab_size, 2);
        assert!(!loaded.editor.insert_spaces);
        assert!(loaded.editor.word_wrap);
        assert_eq!(loaded.editor.word_wrap_column, 80);

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

    // SDTEST-069 — pin the documented first-run values. A fresh
    // ShellDeck install shows this exact terminal + general layout;
    // silent drift on any of these fields would change what every
    // new user sees on day one. Cheap invariant test.
    #[test]
    fn default_matches_documented_first_run_values() {
        let cfg = AppConfig::default();

        // Theme
        assert_eq!(cfg.theme, ThemePreference::Dark);

        // Terminal
        assert_eq!(cfg.terminal.font_family, "JetBrains Mono");
        assert_eq!(cfg.terminal.font_size, 14.0);
        assert_eq!(cfg.terminal.scrollback_lines, 10_000);
        assert!(cfg.terminal.default_shell.is_none());
        assert_eq!(cfg.terminal.cursor_style, "block");
        assert!(cfg.terminal.cursor_blink);
        assert_eq!(cfg.terminal.theme, "Dark");

        // General
        assert!(!cfg.general.auto_connect_on_startup);
        assert!(cfg.general.show_notifications);
        assert!(cfg.general.confirm_before_close);
        assert_eq!(cfg.general.sidebar_width, 260.0);
        assert!(!cfg.general.sidebar_nav_collapsed);
        assert!(!cfg.general.auto_attach_tmux);
        assert!(cfg.general.auto_update);
        assert_eq!(cfg.general.ui_language, UiLanguage::System);
        assert_eq!(cfg.general.ui_font_family, "System Default");
        assert_eq!(cfg.general.ui_font_size, 14.0);

        // Session state that must be OFF on first run
        assert!(cfg.account.is_none());
        assert!(cfg.jeanclaude.is_none());
        assert!(!cfg.cloud_sync.enabled);
        assert!(!cfg.jean_runtime.enabled);
        assert!(!cfg.bext_cloud.is_connected());
    }
}
