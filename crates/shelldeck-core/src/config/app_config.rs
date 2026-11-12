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
    /// `[clippy]` — portable desktop assistant preferences. AI/backend selection
    /// remains in `[ai]`; this section only stores opt-in collection and companion
    /// presentation settings. Defaults keep older configs parsing and Clippy disabled.
    #[serde(default)]
    pub clippy: crate::ai::ClippyConfig,
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
    /// Local `[monique]` dashboard override. When absent, signed-in
    /// super-admins use the configuration delivered by Inklura Manage.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub monique: Option<crate::config::monique::MoniqueConfig>,
    /// `[bext_cloud]` — connection to the cloud.bext.dev control plane. Empty
    /// token = not connected; `#[serde(default)]` keeps older configs parsing.
    #[serde(default)]
    pub bext_cloud: crate::config::bext_cloud::BextCloudConfig,
    /// `[tray]` — system-tray preferences (close-to-tray + per-category
    /// notification opt-in). `#[serde(default)]` keeps older configs
    /// without a `[tray]` section parsing cleanly.
    #[serde(default)]
    pub tray: TrayConfig,
    /// `[companion]` — process/window lifecycle for the tray companion.
    /// Kept separate from notification preferences and defaulted so configs
    /// written before the AI Dock continue to parse unchanged.
    #[serde(default)]
    pub companion: CompanionConfig,
    /// `[voice]`: speaking to agents with the user's own ElevenLabs key
    /// (dictation in the composer, replies read aloud). The key itself is
    /// in the OS keychain, never here.
    #[serde(default)]
    pub voice: crate::config::elevenlabs::VoiceConfig,
    /// Connection ids shown in the sidebar and system-tray quick-access
    /// sections. Order is user-defined and preserved across sessions.
    #[serde(default)]
    pub pinned_connections: Vec<uuid::Uuid>,
    /// `[keybindings]`: the user's changes to the application shortcuts, by
    /// action id (`next_view = "secondary-alt-v"`; an empty value removes the
    /// shortcut). Read at startup. The action ids and the validation live in
    /// `shelldeck-ui::shortcut_reference`; an entry that cannot be applied is
    /// reported there and the built-in keys stay.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub keybindings: std::collections::BTreeMap<String, String>,
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
    /// Show the terminals as a list grouped by machine in place of the tab
    /// strip. Toggled with the sidebar shortcut in the Terminal view.
    #[serde(default)]
    pub session_list: bool,
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

/// `#[serde(default)]` for `bool` fields that must default to `true` when the
/// key is absent from an older `shelldeck.toml`.
fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct GeneralConfig {
    pub auto_connect_on_startup: bool,
    pub show_notifications: bool,
    pub confirm_before_close: bool,
    pub sidebar_width: f32,
    /// Retired in v0.6.3. Used to hide the sidebar's in-panel navigation
    /// list, which the always-on activity rail replaced; there is no longer a
    /// state in which the Dev sidebar has no navigation surface. Kept as a
    /// parsed-and-ignored field so a `shelldeck.toml` written by an older
    /// build still loads.
    #[serde(default)]
    pub sidebar_nav_collapsed: bool,
    /// Whether the application menu row (Fichier / Édition / …) is shown
    /// under the titlebar. Visible by default; hidden from Affichage → Barre
    /// de menus. The terminal grid's usable height depends on this, so the
    /// workspace pushes changes down to `TerminalView::set_menu_bar_visible`.
    #[serde(default = "default_true")]
    pub menu_bar_visible: bool,
    pub auto_attach_tmux: bool,
    /// Opt-in prototype (`docs/design/session-daemon.md`): new local
    /// terminal tabs and splits run in panes owned by the per-user
    /// `shelldeck-sessiond` daemon, so they survive quitting or updating
    /// the app and come back in their saved layout at the next start.
    /// Unix only; off by default; toggle in Settings, General. Read when a
    /// terminal is created: changing it applies to new terminals only.
    pub session_daemon: bool,
    pub auto_update: bool,
    /// Interface language. `system` follows the OS locale (French default).
    #[serde(default)]
    pub ui_language: UiLanguage,
    /// Font family for the application UI (sidebar, dashboard, forms, etc.).
    ///
    /// Defaults to `Inter`, which ShellDeck embeds and registers at startup, so
    /// the application always has one resolvable UI family on every platform.
    /// The legacy `"System Default"` sentinel still parses and resolves to
    /// Inter — it never named a real family, and leaving it unresolved is what
    /// let GPUI's built-in (monospace) default leak into every hand-rolled
    /// element while adabraka components rendered in Inter.
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
    /// `[general.session_daemon_hosts]`: for each SSH connection (key: its
    /// id), whether the app may upload the `shelldeck-sessiond` binary to
    /// `~/.shelldeck/bin` on that host when it is missing there. A host
    /// without an entry is asked about once per run; nothing is ever
    /// uploaded without an `allow` entry. Only read with
    /// `session_daemon` on.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub session_daemon_hosts: std::collections::BTreeMap<String, RemoteDaemonInstall>,
    /// `[general.session_daemon_host_targets]`: for each answer above, the
    /// server it was given for (`Connection::dial_identity`: user, host,
    /// port and jump host). An answer only counts while the connection
    /// still dials that server: after the entry is edited to point at
    /// another machine or account, the user is asked again instead of
    /// the old answer being applied to a server it was never given for.
    /// An answer without a recorded server reads as "not asked".
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub session_daemon_host_targets: std::collections::BTreeMap<String, String>,
}

/// The user's recorded answer to "install the session daemon on this
/// host?" (`[general.session_daemon_hosts]`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RemoteDaemonInstall {
    /// Upload the binary when it is missing.
    Allow,
    /// Never upload and never ask again for this host.
    Never,
}

impl GeneralConfig {
    /// The recorded answer for the SSH connection `connection_id`, which
    /// now dials `target` (`Connection::dial_identity`). `None` when the
    /// user was never asked, did not decide, or answered for another
    /// server: the connection entry was edited since (other host, user,
    /// port or jump host), and consent for one server is not consent for
    /// another.
    pub fn remote_daemon_install(
        &self,
        connection_id: uuid::Uuid,
        target: &str,
    ) -> Option<RemoteDaemonInstall> {
        let key = connection_id.to_string();
        let answer = self.session_daemon_hosts.get(&key).copied()?;
        let answered_for = self.session_daemon_host_targets.get(&key)?;
        (answered_for == target).then_some(answer)
    }

    /// Record the answer for `connection_id`, given while it dials
    /// `target`.
    pub fn set_remote_daemon_install(
        &mut self,
        connection_id: uuid::Uuid,
        target: &str,
        answer: RemoteDaemonInstall,
    ) {
        let key = connection_id.to_string();
        self.session_daemon_host_targets
            .insert(key.clone(), target.to_string());
        self.session_daemon_hosts.insert(key, answer);
    }

    /// How many recorded answers allow the install, and how many refuse
    /// it (for the Settings row that offers to forget them).
    pub fn remote_daemon_install_counts(&self) -> (usize, usize) {
        let allowed = self
            .session_daemon_hosts
            .values()
            .filter(|answer| **answer == RemoteDaemonInstall::Allow)
            .count();
        (allowed, self.session_daemon_hosts.len() - allowed)
    }

    /// Forget every recorded answer: each server is asked about again at
    /// its next connection. Nothing is removed from any server.
    pub fn clear_remote_daemon_installs(&mut self) {
        self.session_daemon_hosts.clear();
        self.session_daemon_host_targets.clear();
    }

    /// Every recorded answer, for the Settings rows that show and change
    /// them one server at a time, ordered by server.
    pub fn remote_daemon_install_entries(&self) -> Vec<RemoteDaemonAnswer> {
        let mut entries: Vec<RemoteDaemonAnswer> = self
            .session_daemon_hosts
            .iter()
            .map(|(key, answer)| RemoteDaemonAnswer {
                key: key.clone(),
                target: self.session_daemon_host_targets.get(key).cloned(),
                answer: *answer,
            })
            .collect();
        entries.sort_by(|a, b| a.target.cmp(&b.target).then_with(|| a.key.cmp(&b.key)));
        entries
    }

    /// Forget the answer recorded under `key` (a connection id): that
    /// server is asked again at its next connection. Nothing is removed
    /// from the server.
    pub fn forget_remote_daemon_install(&mut self, key: &str) {
        self.session_daemon_hosts.remove(key);
        self.session_daemon_host_targets.remove(key);
    }

    /// Change the answer recorded under `key`, for the server it was given
    /// for. A key without a recorded answer stays unanswered: consent is
    /// only recorded together with the server it applies to.
    pub fn change_remote_daemon_install(&mut self, key: &str, answer: RemoteDaemonInstall) {
        if let Some(recorded) = self.session_daemon_hosts.get_mut(key) {
            *recorded = answer;
        }
    }
}

/// One recorded answer to "install the session daemon on this server?".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteDaemonAnswer {
    /// Key in `[general.session_daemon_hosts]`: the SSH connection id.
    pub key: String,
    /// The server the answer was given for (`Connection::dial_identity`),
    /// `None` for an answer from before targets were recorded (it reads
    /// as "not asked").
    pub target: Option<String>,
    pub answer: RemoteDaemonInstall,
}

impl RemoteDaemonAnswer {
    /// The server as people read it: `user@host:port`, followed by the
    /// jump host when there is one.
    pub fn server_label(&self) -> Option<String> {
        let target = self.target.as_deref()?;
        Some(match target.split_once('|') {
            Some((server, "")) => server.to_string(),
            Some((server, jump)) => format!("{server} \u{2192} {jump}"),
            None => target.to_string(),
        })
    }
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
    /// Show an OS notification when Monique fleet jobs need user
    /// confirmation.
    pub notify_monique_pending: bool,
    /// Show an OS notification when previously-active SSH sessions
    /// drop.
    pub notify_ssh_disconnect: bool,
    /// Show an OS notification when a Fleet job finishes (either
    /// success or failure). Especially useful in `auto` runtime mode.
    pub notify_fleet_done: bool,
    /// Show an OS notification when an AI task completes while the
    /// ShellDeck window is not active.
    pub notify_ai_tasks: bool,
    /// Show an OS notification when new Inklura Manage bell items arrive
    /// (exports, automations, agent alerts) in any of the account's spaces.
    pub notify_inklura: bool,
}

impl Default for TrayConfig {
    fn default() -> Self {
        Self {
            close_to_tray: false,
            // Notifications on by default — the whole point of Phase C
            // is to catch state changes while the window is hidden.
            // Users can mute any category from Settings → Général.
            notify_new_tickets: true,
            notify_monique_pending: true,
            notify_ssh_disconnect: true,
            notify_fleet_done: true,
            notify_ai_tasks: true,
            notify_inklura: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct CompanionConfig {
    /// Start with the main window hidden when the tray backend is available.
    /// Defaults off so upgrades preserve ShellDeck's traditional visible start.
    pub start_hidden: bool,
    /// Register the system-wide shortcut that toggles the Assistant Dock.
    pub global_shortcut_enabled: bool,
    /// Register the system-wide shortcut that toggles the command palette.
    pub global_palette_shortcut_enabled: bool,
    /// GPUI keystroke syntax for the Assistant Dock global shortcut.
    pub global_shortcut: String,
    /// GPUI keystroke syntax for the command-palette global shortcut.
    pub global_palette_shortcut: String,
    /// Register the system-wide "Capture & share" shortcut (region capture
    /// into Share). Off by default so upgrades never claim a new key.
    pub global_capture_shortcut_enabled: bool,
    /// GPUI keystroke syntax for the "Capture & share" global shortcut.
    pub global_capture_shortcut: String,
}

impl Default for CompanionConfig {
    fn default() -> Self {
        Self {
            start_hidden: false,
            global_shortcut_enabled: true,
            global_palette_shortcut_enabled: true,
            global_shortcut: Self::default_global_shortcut().to_string(),
            global_palette_shortcut: Self::default_global_palette_shortcut().to_string(),
            global_capture_shortcut_enabled: false,
            global_capture_shortcut: Self::default_global_capture_shortcut().to_string(),
        }
    }
}

impl CompanionConfig {
    pub fn default_global_shortcut() -> &'static str {
        if cfg!(target_os = "macos") {
            "cmd-shift-space"
        } else {
            "ctrl-shift-space"
        }
    }

    pub fn default_global_capture_shortcut() -> &'static str {
        if cfg!(target_os = "macos") {
            "cmd-alt-s"
        } else {
            "ctrl-alt-s"
        }
    }

    pub fn default_global_palette_shortcut() -> &'static str {
        if cfg!(target_os = "macos") {
            "cmd-alt-space"
        } else {
            "ctrl-alt-space"
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
            session_list: false,
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
            menu_bar_visible: true,
            auto_attach_tmux: false,
            session_daemon: false,
            auto_update: true,
            ui_language: UiLanguage::default(),
            ui_font_family: "Inter".to_string(),
            ui_font_size: 14.0,
            autostart: false,
            onboarding_completed: false,
            session_daemon_hosts: std::collections::BTreeMap::new(),
            session_daemon_host_targets: std::collections::BTreeMap::new(),
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

    // SDTEST-3014 (SDUC-574): the per-host answer to "install the session
    // daemon there?" is a file contract: it decides whether a binary may
    // be uploaded to a server.
    // SDTEST-3070 (SDUC-574): recorded install answers are listed one
    // server at a time, and one of them can be changed or forgotten
    // without touching the others. A changed answer keeps the server it
    // was given for; an unrecorded key cannot be turned into consent.
    #[test]
    fn sdtest_3070_install_answers_are_changed_one_server_at_a_time() {
        let mut general = AppConfig::default().general;
        let (one, two, three) = (
            uuid::Uuid::from_u128(1),
            uuid::Uuid::from_u128(2),
            uuid::Uuid::from_u128(3),
        );
        general.set_remote_daemon_install(one, "deploy@b.example:22|", RemoteDaemonInstall::Allow);
        general.set_remote_daemon_install(
            two,
            "root@a.example:2222|bastion",
            RemoteDaemonInstall::Never,
        );
        // An answer from before targets were recorded.
        general
            .session_daemon_hosts
            .insert(three.to_string(), RemoteDaemonInstall::Allow);

        let entries = general.remote_daemon_install_entries();
        let labels: Vec<_> = entries.iter().map(|e| e.server_label()).collect();
        assert_eq!(
            labels,
            vec![
                None,
                Some("deploy@b.example:22".to_string()),
                Some("root@a.example:2222 \u{2192} bastion".to_string()),
            ]
        );
        assert_eq!(entries[1].key, one.to_string());
        assert_eq!(entries[2].answer, RemoteDaemonInstall::Never);

        general.change_remote_daemon_install(&two.to_string(), RemoteDaemonInstall::Allow);
        assert_eq!(
            general.remote_daemon_install(two, "root@a.example:2222|bastion"),
            Some(RemoteDaemonInstall::Allow)
        );
        // Still not for another server.
        assert_eq!(
            general.remote_daemon_install(two, "root@c.example:22|"),
            None
        );
        // A key nobody answered for stays unanswered.
        let stranger = uuid::Uuid::from_u128(9);
        general.change_remote_daemon_install(&stranger.to_string(), RemoteDaemonInstall::Allow);
        assert!(!general
            .session_daemon_hosts
            .contains_key(&stranger.to_string()));

        general.forget_remote_daemon_install(&one.to_string());
        assert_eq!(
            general.remote_daemon_install(one, "deploy@b.example:22|"),
            None
        );
        assert!(!general
            .session_daemon_host_targets
            .contains_key(&one.to_string()));
        assert_eq!(general.remote_daemon_install_counts(), (2, 0));
    }

    #[test]
    fn sdtest_3014_remote_daemon_install_answers_round_trip_and_default_to_unasked() {
        let path = temp_path("config.toml");
        let allowed = uuid::Uuid::new_v4();
        let refused = uuid::Uuid::new_v4();
        let unasked = uuid::Uuid::new_v4();
        let (prod_a, prod_b) = ("deploy@prod-a:22|", "deploy@prod-b:22|");

        let mut config = AppConfig::default();
        assert_eq!(config.general.remote_daemon_install(allowed, prod_a), None);
        // No entry, no table in the file.
        config.save_to(&path).expect("save_to");
        let text = std::fs::read_to_string(&path).expect("read");
        assert!(!text.contains("session_daemon_host"), "{text}");

        config
            .general
            .set_remote_daemon_install(allowed, prod_a, RemoteDaemonInstall::Allow);
        config
            .general
            .set_remote_daemon_install(refused, prod_b, RemoteDaemonInstall::Never);
        config.save_to(&path).expect("save_to");
        let text = std::fs::read_to_string(&path).expect("read");
        assert!(text.contains("[general.session_daemon_hosts]"), "{text}");
        assert!(text.contains(&format!("{allowed} = \"allow\"")), "{text}");
        assert!(text.contains(&format!("{refused} = \"never\"")), "{text}");
        assert!(
            text.contains("[general.session_daemon_host_targets]"),
            "{text}"
        );
        assert!(
            text.contains(&format!("{allowed} = \"{prod_a}\"")),
            "{text}"
        );

        let loaded = AppConfig::load_from(&path).expect("load_from");
        assert_eq!(
            loaded.general.remote_daemon_install(allowed, prod_a),
            Some(RemoteDaemonInstall::Allow)
        );
        assert_eq!(
            loaded.general.remote_daemon_install(refused, prod_b),
            Some(RemoteDaemonInstall::Never)
        );
        assert_eq!(loaded.general.remote_daemon_install(unasked, prod_a), None);

        // SDTEST-3043: an answer counts only for the server it was given
        // for. The same connection entry edited to dial another host,
        // user, port or jump host is "not asked": nothing is uploaded
        // there without a new answer, and a "never" does not silence the
        // question for the new server either.
        for other in [
            prod_b,
            "root@prod-a:22|",
            "deploy@prod-a:2222|",
            "deploy@prod-a:22|bastion",
            "",
        ] {
            assert_eq!(
                loaded.general.remote_daemon_install(allowed, other),
                None,
                "{other:?}"
            );
        }
        assert_eq!(loaded.general.remote_daemon_install(refused, prod_a), None);
        // Answering again for the new server replaces the old answer.
        let mut edited = loaded.clone();
        edited
            .general
            .set_remote_daemon_install(allowed, prod_b, RemoteDaemonInstall::Never);
        assert_eq!(edited.general.remote_daemon_install(allowed, prod_a), None);
        assert_eq!(
            loaded.general.remote_daemon_install_counts(),
            (1, 1),
            "one allowed, one refused"
        );
        assert_eq!(
            edited.general.remote_daemon_install(allowed, prod_b),
            Some(RemoteDaemonInstall::Never)
        );
        // An answer without a recorded server (written by hand) is not
        // trusted for any server.
        let mut by_hand = loaded.clone();
        by_hand.general.session_daemon_host_targets.clear();
        by_hand.save_to(&path).expect("save_to");
        let by_hand = AppConfig::load_from(&path).expect("load_from");
        assert_eq!(
            by_hand.general.session_daemon_hosts.len(),
            2,
            "the answers are still in the file"
        );
        assert_eq!(by_hand.general.remote_daemon_install(allowed, prod_a), None);
        assert_eq!(by_hand.general.remote_daemon_install(refused, prod_b), None);

        // Settings can forget every answer: the tables leave the file and
        // each server reads as not asked.
        let mut forgotten = loaded.clone();
        forgotten.general.clear_remote_daemon_installs();
        assert_eq!(forgotten.general.remote_daemon_install_counts(), (0, 0));
        forgotten.save_to(&path).expect("save_to");
        let text = std::fs::read_to_string(&path).expect("read");
        assert!(!text.contains("session_daemon_host"), "{text}");
        let forgotten = AppConfig::load_from(&path).expect("load_from");
        assert_eq!(
            forgotten.general.remote_daemon_install(allowed, prod_a),
            None
        );
        assert_eq!(
            forgotten.general.remote_daemon_install(refused, prod_b),
            None
        );

        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }

    #[test]
    fn round_trip_non_default() {
        let path = temp_path("config.toml");

        let mut config = AppConfig {
            theme: ThemePreference::Light,
            ..Default::default()
        };
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

    // SDTEST-070
    #[test]
    fn save_to_replaces_config_file_atomically() {
        let path = temp_path("config.toml");
        let prior_version = path.with_extension("prior.toml");

        AppConfig::default()
            .save_to(&path)
            .expect("save initial config");
        std::fs::hard_link(&path, &prior_version).expect("link initial config");

        let updated = AppConfig {
            theme: ThemePreference::Light,
            ..Default::default()
        };
        updated.save_to(&path).expect("replace config atomically");

        let current = AppConfig::load_from(&path).expect("load current config");
        let prior = AppConfig::load_from(&prior_version).expect("load linked prior config");
        assert_eq!(current.theme, ThemePreference::Light);
        assert_eq!(prior.theme, ThemePreference::Dark);

        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }

    // SDTEST-3103 / SDUC-577. `[keybindings]` is a file the user edits by
    // hand: an older file has none, the application writes none until one
    // exists, and saving the configuration must not lose the user's table.
    #[test]
    fn sdtest_3103_keybindings_table_is_optional_and_survives_a_save() {
        let older: AppConfig = toml::from_str("theme = \"Dark\"\n\n[terminal]\n\n[general]\n")
            .expect("parse config without keybindings");
        assert!(older.keybindings.is_empty());
        let written = toml::to_string_pretty(&older).expect("serialize");
        assert!(!written.contains("keybindings"), "{written}");

        let edited = format!(
            "{written}\n[keybindings]\nnext_view = \"secondary-alt-v\"\ntoggle_sidebar = \"\"\n"
        );
        let config: AppConfig = toml::from_str(&edited).expect("parse edited config");
        assert_eq!(
            config.keybindings.get("next_view").map(String::as_str),
            Some("secondary-alt-v")
        );
        assert_eq!(
            config.keybindings.get("toggle_sidebar").map(String::as_str),
            Some("")
        );
        let saved = toml::to_string_pretty(&config).expect("serialize");
        let reloaded: AppConfig = toml::from_str(&saved).expect("parse saved config");
        assert_eq!(reloaded.keybindings, config.keybindings, "{saved}");
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

    // SDTEST-1382
    #[test]
    fn config_without_companion_section_defaults_to_visible_start() {
        let config: AppConfig = toml::from_str(
            r#"
theme = "Dark"

[terminal]

[general]
"#,
        )
        .expect("parse config without companion section");

        assert!(!config.companion.start_hidden);
        assert!(config.companion.global_shortcut_enabled);
        assert!(config.companion.global_palette_shortcut_enabled);

        let configured: AppConfig = toml::from_str(
            r#"
theme = "Dark"

[terminal]

[general]

[companion]
start_hidden = true
"#,
        )
        .expect("parse config with companion section");
        assert!(configured.companion.start_hidden);
        let serialized = toml::to_string(&configured).expect("serialize companion config");
        let reloaded: AppConfig = toml::from_str(&serialized).expect("reload companion config");
        assert!(reloaded.companion.start_hidden);
    }

    // SDTEST-1400
    #[test]
    fn companion_shortcuts_default_for_old_configs_and_round_trip_custom_values() {
        let mut config: AppConfig = toml::from_str(
            r#"
theme = "Dark"

[terminal]

[general]

[companion]
global_shortcut_enabled = true
global_palette_shortcut_enabled = true
"#,
        )
        .expect("parse old companion config");
        assert_eq!(
            config.companion.global_shortcut,
            CompanionConfig::default_global_shortcut()
        );
        assert_eq!(
            config.companion.global_palette_shortcut,
            CompanionConfig::default_global_palette_shortcut()
        );

        config.companion.global_shortcut = "ctrl-alt-k".to_string();
        config.companion.global_palette_shortcut = "ctrl-alt-p".to_string();
        let serialized = toml::to_string(&config).expect("serialize custom shortcuts");
        let reloaded: AppConfig = toml::from_str(&serialized).expect("reload custom shortcuts");
        assert_eq!(reloaded.companion.global_shortcut, "ctrl-alt-k");
        assert_eq!(reloaded.companion.global_palette_shortcut, "ctrl-alt-p");
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
        let serialized = std::fs::read_to_string(&path).expect("saved config");
        assert!(!serialized.contains("sd_secret"));
        let loaded = AppConfig::load_from(&path).expect("load_from");

        assert!(loaded.cloud_sync.enabled);
        assert!(loaded.cloud_sync.token.is_empty());
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
        let logged_in = AppConfig {
            account: Some(AccountInfo {
                email: "ben@webdesign29.net".to_string(),
                name: "Ben Favre".to_string(),
                is_superadmin: true,
                is_admin: true,
                is_inklura_support: true,
                roles: vec!["superadmin".to_string()],
            }),
            ..Default::default()
        };
        logged_in.save_to(&path).expect("save_to");
        let loaded = AppConfig::load_from(&path).expect("load_from");
        let acct = loaded.account.expect("account present");
        assert_eq!(acct.email, "ben@webdesign29.net");
        assert_eq!(acct.name, "Ben Favre");
        assert!(acct.is_superadmin, "is_superadmin should round-trip");

        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }

    #[test]
    // SDTEST-1666
    fn monique_override_round_trips_and_omits_when_unset() {
        use crate::config::monique::MoniqueConfig;

        let path = temp_path("shelldeck.toml");
        let mut cfg = AppConfig::default();
        cfg.save_to(&path).unwrap();
        let serialized = std::fs::read_to_string(&path).unwrap();
        assert!(!serialized.contains("[monique]"));

        cfg.monique = Some(MoniqueConfig {
            url: "https://monique.example".into(),
            user: "ops".into(),
            pass: "secret".into(),
        });
        cfg.save_to(&path).unwrap();
        let loaded = AppConfig::load_from(&path).unwrap().monique.unwrap();
        assert_eq!(loaded.url, "https://monique.example");
        assert_eq!(loaded.user, "ops");
        assert_eq!(loaded.pass, "secret");

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
        assert!(!loaded.clippy.auto_import_clipboard_on_shortcut);
        assert_eq!(
            loaded.clippy.appearance.character_id(),
            crate::ai::CompanionCharacterId::Clippy
        );

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

    // SDTEST-1477
    #[test]
    fn clippy_config_defaults_and_surface_opt_in_round_trip() {
        let mut config = AppConfig::default();
        assert!(!config.ai.surfaces.clippy);
        assert!(!config.clippy.auto_import_clipboard_on_shortcut);
        assert!(!config.clippy.appearance.desktop.enabled);

        config.ai.enabled = true;
        config.ai.backend = crate::ai::AiBackend::OpenAi;
        config.ai.surfaces.clippy = true;
        config.clippy.auto_import_clipboard_on_shortcut = true;
        config.clippy.appearance.character = "shelly".to_string();
        config.clippy.appearance.desktop.enabled = true;

        let serialized = toml::to_string_pretty(&config).expect("serialize clippy config");
        assert!(serialized.contains("[clippy]"));
        assert!(serialized.contains("clippy = true"));
        let loaded: AppConfig = toml::from_str(&serialized).expect("reload clippy config");
        assert!(loaded.ai.surfaces.clippy);
        assert!(loaded.clippy.auto_import_clipboard_on_shortcut);
        assert_eq!(
            loaded.clippy.appearance.character_id(),
            crate::ai::CompanionCharacterId::Shelly
        );
        assert!(loaded.clippy.appearance.desktop.enabled);
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
        assert!(!cfg.general.session_daemon);
        assert!(cfg.general.auto_update);
        assert_eq!(cfg.general.ui_language, UiLanguage::System);
        assert_eq!(cfg.general.ui_font_family, "Inter");
        assert_eq!(cfg.general.ui_font_size, 14.0);

        // Session state that must be OFF on first run
        assert!(cfg.account.is_none());
        assert!(cfg.monique.is_none());
        assert!(!cfg.cloud_sync.enabled);
        assert!(!cfg.bext_cloud.is_connected());
        assert!(!cfg.companion.start_hidden);
    }
}
