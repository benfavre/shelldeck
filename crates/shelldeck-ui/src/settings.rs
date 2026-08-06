use crate::scale::px;
use adabraka_ui::components::icon_button::IconButton;
use adabraka_ui::components::icon_source::IconSource;
use adabraka_ui::components::input::{Input, InputSize};
use adabraka_ui::components::input_state::InputState;
use adabraka_ui::components::select::{Select, SelectOption};
use adabraka_ui::components::toggle::Toggle;
use adabraka_ui::prelude::{
    scrollable_vertical, Badge, BadgeVariant, Button, ButtonSize, ButtonVariant, Spinner,
    SpinnerSize, SpinnerVariant,
};
use gpui::prelude::*;
use gpui::*;

use crate::t;
use shelldeck_core::ai::{
    configured_cli_available, AiAutonomyLevel, AiBackend, ClippyAppearanceConfig,
    CompanionMotionPreference, CompanionScale, DesktopCompanionMovement,
};
use shelldeck_core::config::app_config::{AppConfig, CompanionConfig, ThemePreference, UiLanguage};
use shelldeck_core::config::themes::TerminalTheme;

use crate::theme::{palette_for, ShellDeckColors};
use crate::workspace::CloudSyncNow;

/// Fixed shortlist of monospace families offered for the editor + terminal.
/// Kept in sync between the two settings tabs; extend here to surface new
/// picks everywhere.
const MONOSPACE_FONTS: &[&str] = &[
    "JetBrains Mono",
    "Fira Code",
    "Source Code Pro",
    "Cascadia Code",
    "Menlo",
    "Consolas",
];

const EDITOR_TAB_SIZES: &[usize] = &[2, 4, 8];

fn apply_character_choice(appearance: &mut ClippyAppearanceConfig, id: &str) {
    appearance.character = id.to_string();
    appearance.desktop.enabled = id != "none";
}

fn compositor_companion_limited(compositor: &str) -> bool {
    compositor.eq_ignore_ascii_case("wayland")
}

#[cfg(target_os = "linux")]
fn desktop_companion_platform_limited() -> bool {
    compositor_companion_limited(gpui::guess_compositor())
}

#[cfg(not(target_os = "linux"))]
fn desktop_companion_platform_limited() -> bool {
    false
}

fn display_shortcut(shortcut: &str) -> String {
    let Ok(keystroke) = Keystroke::parse(shortcut) else {
        return shortcut.to_string();
    };
    let mut parts = Vec::with_capacity(5);
    if keystroke.modifiers.control {
        parts.push("Ctrl".to_string());
    }
    if keystroke.modifiers.alt {
        parts.push("Alt".to_string());
    }
    if keystroke.modifiers.platform {
        parts.push(if cfg!(target_os = "macos") {
            "Cmd".to_string()
        } else {
            "Super".to_string()
        });
    }
    if keystroke.modifiers.shift {
        parts.push("Shift".to_string());
    }
    let mut chars = keystroke.key.chars();
    parts.push(match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    });
    parts.join("+")
}

/// Registration errors reach the UI as raw platform strings, so a French
/// session ends up reading a D-Bus sentence in English. Only one of them is
/// both frequent and environmental — a Wayland session whose portal stack
/// predates `org.freedesktop.portal.GlobalShortcuts` (no portal frontend
/// implements it, so no grab can ever succeed) — so that one gets a
/// translated explanation and everything else still passes through verbatim
/// rather than being flattened into a useless generic message.
pub fn shortcut_error_is_portal_missing(error: &str) -> bool {
    error.contains("GlobalShortcuts")
        && (error.contains("not found") || error.contains("ServiceUnknown"))
}

#[derive(Debug, PartialEq, Eq)]
enum ShortcutCaptureValidation {
    Accepted(String),
    ModifierRequired,
    Conflict,
}

fn validate_shortcut_capture(
    keystroke: &Keystroke,
    other_shortcut: &str,
) -> ShortcutCaptureValidation {
    let modifiers = keystroke.modifiers;
    let modifier_key = matches!(
        keystroke.key.as_str(),
        "shift" | "control" | "ctrl" | "alt" | "platform" | "cmd" | "super" | "win" | "fn"
    );
    if modifier_key || !(modifiers.control || modifiers.alt || modifiers.platform) {
        return ShortcutCaptureValidation::ModifierRequired;
    }
    let shortcut = keystroke.unparse();
    if shortcut == other_shortcut {
        ShortcutCaptureValidation::Conflict
    } else {
        ShortcutCaptureValidation::Accepted(shortcut)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsTab {
    General,
    Terminal,
    Editor,
    Ai,
    Appearance,
    About,
}

/// Events emitted when settings change.
#[derive(Debug, Clone)]
#[allow(clippy::large_enum_variant)]
pub enum SettingsEvent {
    ConfigChanged(AppConfig),
    ThemeChanged(ThemePreference),
    /// Leave the global Settings surface and return to the current app mode.
    CloseRequested,
    /// User flipped the "Launch at login" toggle to `desired`. Workspace
    /// applies the OS-level change on a background thread, then either
    /// commits the config field (on success) or toasts + leaves the
    /// toggle unchanged (on OS failure — Flatpak sandbox, permissions,
    /// missing HOME, …). See `Workspace::apply_autostart_request`.
    AutostartRequested(bool),
    /// Replay the post-login onboarding tour (`Settings → Général`).
    ShowOnboarding,
    /// Store or remove an API credential in the OS keychain. The value never
    /// enters `AppConfig` or `shelldeck.toml`.
    AiApiKeyStored {
        backend: AiBackend,
        value: String,
    },
    AiApiKeyDeleted {
        backend: AiBackend,
    },
    /// Run a real minimal completion through the selected backend.
    AiTestRequested(AppConfig),
}

impl EventEmitter<SettingsEvent> for SettingsView {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CompanionShortcutKind {
    AiDock,
    CommandPalette,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShortcutRegistrationStatus {
    Disabled,
    Applying,
    Registered,
    PendingPortal,
    Conflict,
    Error(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompanionShortcutStatuses {
    pub ai_dock: ShortcutRegistrationStatus,
    pub command_palette: ShortcutRegistrationStatus,
}

impl Default for CompanionShortcutStatuses {
    fn default() -> Self {
        Self {
            ai_dock: ShortcutRegistrationStatus::Disabled,
            command_palette: ShortcutRegistrationStatus::Disabled,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub enum AiConnectionState {
    #[default]
    NotTested,
    Testing,
    Connected,
    Failed(String),
}

pub struct SettingsView {
    pub config: AppConfig,
    pub active_tab: SettingsTab,
    dev_tabs_enabled: bool,
    pub unsaved_changes: bool,
    /// Adabraka `Select` entities. Each keeps its own open/highlighted state
    /// and is rebuilt in `sync_selects` whenever the underlying config
    /// changes externally so the shown selection stays true.
    editor_font_family_select: Entity<Select<SharedString>>,
    editor_tab_size_select: Entity<Select<usize>>,
    terminal_font_family_select: Entity<Select<SharedString>>,
    terminal_cursor_style_select: Entity<Select<SharedString>>,
    general_language_select: Entity<Select<UiLanguage>>,
    ui_font_family_select: Entity<Select<SharedString>>,
    ai_backend_select: Entity<Select<AiBackend>>,
    ai_model_state: Entity<InputState>,
    ai_api_key_state: Entity<InputState>,
    ai_connection_state: AiConnectionState,
    shortcut_capture: Option<CompanionShortcutKind>,
    shortcut_status_before_capture: Option<(CompanionShortcutKind, ShortcutRegistrationStatus)>,
    shortcut_capture_focus: FocusHandle,
    companion_shortcut_statuses: CompanionShortcutStatuses,
}

impl SettingsView {
    pub fn new(config: AppConfig, cx: &mut Context<Self>) -> Self {
        let editor_font_family_select = build_editor_font_family_select(&config, cx);
        let editor_tab_size_select = build_editor_tab_size_select(&config, cx);
        let terminal_font_family_select = build_terminal_font_family_select(&config, cx);
        let terminal_cursor_style_select = build_terminal_cursor_style_select(&config, cx);
        let general_language_select = build_general_language_select(&config, cx);
        let ui_font_family_select = build_ui_font_family_select(&config, cx);
        let ai_backend_select = build_ai_backend_select(&config, cx);
        let ai_model = config.ai.model.clone();
        let shortcut_capture_focus = cx.focus_handle();
        Self {
            config,
            active_tab: SettingsTab::General,
            dev_tabs_enabled: false,
            unsaved_changes: false,
            editor_font_family_select,
            editor_tab_size_select,
            terminal_font_family_select,
            terminal_cursor_style_select,
            general_language_select,
            ui_font_family_select,
            ai_backend_select,
            ai_model_state: cx.new(|cx| {
                let mut state = InputState::new(cx);
                state.content = ai_model.into();
                state
            }),
            ai_api_key_state: cx.new(InputState::new),
            ai_connection_state: AiConnectionState::NotTested,
            shortcut_capture: None,
            shortcut_status_before_capture: None,
            shortcut_capture_focus,
            companion_shortcut_statuses: CompanionShortcutStatuses::default(),
        }
    }

    /// Rebuild only the `Select` entities whose backing config slice differs
    /// from `old`. Called from `Workspace::sync_settings_config` — a mode
    /// switch or `cloud_sync` toggle no longer nukes the 6 dropdown
    /// popovers just to refresh their `selected_index` (which fixed a UX
    /// bug where opening a Select then triggering any workspace event
    /// would close the popover mid-pick).
    pub fn sync_selects_if_changed(&mut self, old: &AppConfig, cx: &mut Context<Self>) {
        if self.config.editor.font_family != old.editor.font_family {
            self.editor_font_family_select = build_editor_font_family_select(&self.config, cx);
        }
        if self.config.editor.tab_size != old.editor.tab_size {
            self.editor_tab_size_select = build_editor_tab_size_select(&self.config, cx);
        }
        if self.config.terminal.font_family != old.terminal.font_family {
            self.terminal_font_family_select = build_terminal_font_family_select(&self.config, cx);
        }
        if self.config.terminal.cursor_style != old.terminal.cursor_style {
            self.terminal_cursor_style_select =
                build_terminal_cursor_style_select(&self.config, cx);
        }
        if self.config.general.ui_language != old.general.ui_language {
            self.general_language_select = build_general_language_select(&self.config, cx);
        }
        if self.config.general.ui_font_family != old.general.ui_font_family {
            self.ui_font_family_select = build_ui_font_family_select(&self.config, cx);
        }
        if self.config.ai.backend != old.ai.backend {
            self.ai_backend_select = build_ai_backend_select(&self.config, cx);
        }
        if self.config.ai.model != old.ai.model {
            let model = self.config.ai.model.clone();
            self.ai_model_state.update(cx, |state, cx| {
                state.content = model.into();
                cx.notify();
            });
        }
    }

    /// Select a terminal color theme by name and persist it immediately.
    /// Emits `ConfigChanged` so the live terminal repaints with the new theme.
    pub fn select_terminal_theme(&mut self, name: &str, cx: &mut Context<Self>) {
        if self.config.terminal.theme == name {
            return;
        }
        self.config.terminal.theme = name.to_string();
        self.save_config(cx);
    }

    /// The name of the currently selected terminal theme.
    pub fn terminal_theme_name(&self) -> &str {
        &self.config.terminal.theme
    }

    /// Select an application theme and persist it immediately. Emits
    /// `ThemeChanged` so the workspace swaps the live palette. Shared by the
    /// Appearance settings cards and the titlebar theme switcher.
    pub fn select_app_theme(&mut self, pref: ThemePreference, cx: &mut Context<Self>) {
        if self.config.theme == pref {
            return;
        }
        self.config.theme = pref.clone();
        if let Err(e) = self.config.save() {
            tracing::error!("Failed to save config: {}", e);
        }
        self.unsaved_changes = false;
        cx.emit(SettingsEvent::ThemeChanged(pref));
        cx.notify();
    }

    /// Select interface language, persist, and emit `ConfigChanged` so the
    /// workspace applies `rust_i18n::set_locale` and repaints.
    pub fn select_ui_language(&mut self, lang: UiLanguage, cx: &mut Context<Self>) {
        if self.config.general.ui_language == lang {
            return;
        }
        self.config.general.ui_language = lang;
        self.save_config(cx);
    }

    /// The currently selected application theme.
    pub fn app_theme(&self) -> ThemePreference {
        self.config.theme.clone()
    }

    pub fn set_companion_shortcut_statuses(
        &mut self,
        statuses: CompanionShortcutStatuses,
        cx: &mut Context<Self>,
    ) {
        if self.companion_shortcut_statuses == statuses {
            return;
        }
        if let Some(kind) = self.shortcut_capture {
            let status = match kind {
                CompanionShortcutKind::AiDock => statuses.ai_dock.clone(),
                CompanionShortcutKind::CommandPalette => statuses.command_palette.clone(),
            };
            self.shortcut_status_before_capture = Some((kind, status));
        }
        self.companion_shortcut_statuses = statuses;
        cx.notify();
    }

    fn shortcut_status(&self, kind: CompanionShortcutKind) -> &ShortcutRegistrationStatus {
        match kind {
            CompanionShortcutKind::AiDock => &self.companion_shortcut_statuses.ai_dock,
            CompanionShortcutKind::CommandPalette => {
                &self.companion_shortcut_statuses.command_palette
            }
        }
    }

    fn set_shortcut_status(
        &mut self,
        kind: CompanionShortcutKind,
        status: ShortcutRegistrationStatus,
    ) {
        match kind {
            CompanionShortcutKind::AiDock => {
                self.companion_shortcut_statuses.ai_dock = status;
            }
            CompanionShortcutKind::CommandPalette => {
                self.companion_shortcut_statuses.command_palette = status;
            }
        }
    }

    fn handle_shortcut_capture(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) {
        let Some(kind) = self.shortcut_capture else {
            return;
        };
        cx.stop_propagation();
        if event.keystroke.key.eq_ignore_ascii_case("escape") {
            self.cancel_shortcut_capture();
            cx.notify();
            return;
        }

        let other = match kind {
            CompanionShortcutKind::AiDock => &self.config.companion.global_palette_shortcut,
            CompanionShortcutKind::CommandPalette => &self.config.companion.global_shortcut,
        };
        let shortcut = match validate_shortcut_capture(&event.keystroke, other) {
            ShortcutCaptureValidation::Accepted(shortcut) => shortcut,
            ShortcutCaptureValidation::ModifierRequired => {
                self.set_shortcut_status(
                    kind,
                    ShortcutRegistrationStatus::Error(
                        t!("settings.companion.shortcut.error.modifier_required").to_string(),
                    ),
                );
                cx.notify();
                return;
            }
            ShortcutCaptureValidation::Conflict => {
                self.set_shortcut_status(kind, ShortcutRegistrationStatus::Conflict);
                cx.notify();
                return;
            }
        };
        let current = match kind {
            CompanionShortcutKind::AiDock => &self.config.companion.global_shortcut,
            CompanionShortcutKind::CommandPalette => &self.config.companion.global_palette_shortcut,
        };
        if shortcut == *current {
            self.cancel_shortcut_capture();
            cx.notify();
            return;
        }

        match kind {
            CompanionShortcutKind::AiDock => {
                self.config.companion.global_shortcut = shortcut;
            }
            CompanionShortcutKind::CommandPalette => {
                self.config.companion.global_palette_shortcut = shortcut;
            }
        }
        self.shortcut_capture = None;
        self.shortcut_status_before_capture = None;
        self.set_shortcut_status(kind, ShortcutRegistrationStatus::Applying);
        self.save_config(cx);
    }

    fn reset_shortcut(&mut self, kind: CompanionShortcutKind, cx: &mut Context<Self>) {
        let target = match kind {
            CompanionShortcutKind::AiDock => CompanionConfig::default_global_shortcut(),
            CompanionShortcutKind::CommandPalette => {
                CompanionConfig::default_global_palette_shortcut()
            }
        };
        let current = match kind {
            CompanionShortcutKind::AiDock => &self.config.companion.global_shortcut,
            CompanionShortcutKind::CommandPalette => &self.config.companion.global_palette_shortcut,
        };
        if current == target {
            self.cancel_shortcut_capture();
            cx.notify();
            return;
        }
        match kind {
            CompanionShortcutKind::AiDock => {
                self.config.companion.global_shortcut = target.to_string();
            }
            CompanionShortcutKind::CommandPalette => {
                self.config.companion.global_palette_shortcut = target.to_string();
            }
        }
        self.shortcut_capture = None;
        self.shortcut_status_before_capture = None;
        self.set_shortcut_status(kind, ShortcutRegistrationStatus::Applying);
        self.save_config(cx);
    }

    fn cancel_shortcut_capture(&mut self) {
        if let Some((kind, status)) = self.shortcut_status_before_capture.take() {
            self.set_shortcut_status(kind, status);
        }
        self.shortcut_capture = None;
    }

    /// Update the persisted "application menu row visible" state. Called by
    /// the workspace from Affichage → Barre de menus.
    pub fn set_menu_bar_visible(&mut self, visible: bool, cx: &mut Context<Self>) {
        if self.config.general.menu_bar_visible == visible {
            return;
        }
        self.config.general.menu_bar_visible = visible;
        self.save_config(cx);
    }

    /// Nudge the UI scale (app font size) by `delta`, clamped to [10, 22], and
    /// persist immediately. Emits `ConfigChanged` so the workspace re-applies
    /// the rem size live. Shared by the Appearance settings and the titlebar
    /// scale controls.
    pub fn adjust_ui_font_size(&mut self, delta: f32, cx: &mut Context<Self>) {
        let new = (self.config.general.ui_font_size + delta).clamp(10.0, 22.0);
        if (new - self.config.general.ui_font_size).abs() < f32::EPSILON {
            return;
        }
        self.config.general.ui_font_size = new;
        self.save_config(cx);
    }

    /// The current UI scale (app font size in px).
    pub fn ui_font_size(&self) -> f32 {
        self.config.general.ui_font_size
    }

    /// Change the AI backend from outside the Settings surface (the assistant
    /// composer picker). Routed through here on purpose: `ai.*` is owned by
    /// Settings, so writing `Workspace::app_config` directly would leave this
    /// snapshot stale — see `.agents/session-state.md`.
    pub fn set_ai_backend(&mut self, backend: AiBackend, cx: &mut Context<Self>) {
        if self.config.ai.backend == backend {
            return;
        }
        self.config.ai.backend = backend;
        // The model string belongs to the previous provider; clearing it makes
        // the new backend fall back to its own default.
        self.config.ai.model.clear();
        self.ai_backend_select = build_ai_backend_select(&self.config, cx);
        self.save_config(cx);
    }

    fn save_config(&mut self, cx: &mut Context<Self>) {
        // Emits the full snapshot — workspace must merge slices only and keep
        // this copy fresh after login/logout (see `.agents/session-state.md`).
        if let Err(e) = self.config.save() {
            tracing::error!("Failed to save config: {}", e);
        }
        self.unsaved_changes = false;
        cx.emit(SettingsEvent::ConfigChanged(self.config.clone()));
        cx.notify();
    }

    pub fn set_dev_tabs_enabled(&mut self, enabled: bool, cx: &mut Context<Self>) {
        self.dev_tabs_enabled = enabled;
        if !enabled && matches!(self.active_tab, SettingsTab::Terminal | SettingsTab::Editor) {
            self.active_tab = SettingsTab::General;
        }
        cx.notify();
    }

    /// Navigate directly to a personal Settings tab. Used by command-palette,
    /// menu-bar, and tray entry points that should land on a specific control.
    pub fn set_active_tab(&mut self, tab: SettingsTab, cx: &mut Context<Self>) {
        if self.active_tab == tab {
            return;
        }
        self.active_tab = tab;
        cx.notify();
    }

    fn render_tab_button(
        &self,
        tab: SettingsTab,
        label: &str,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let is_active = self.active_tab == tab;

        let mut el = div()
            .id(ElementId::from(SharedString::from(format!(
                "settings-tab-{tab:?}"
            ))))
            .px(px(16.0))
            .py(px(8.0))
            .cursor_pointer()
            .rounded(px(6.0))
            .text_size(px(13.0));

        if is_active {
            el = el
                .bg(ShellDeckColors::primary().opacity(0.15))
                .text_color(ShellDeckColors::primary())
                .font_weight(FontWeight::MEDIUM);
        } else {
            el = el
                .text_color(ShellDeckColors::text_muted())
                .hover(|el| el.bg(ShellDeckColors::hover_bg()));
        }

        el.on_click(cx.listener(move |this, _event: &ClickEvent, _window, cx| {
            this.active_tab = tab;
            cx.notify();
        }))
        .child(label.to_string())
    }

    fn render_setting_row(
        label: &str,
        description: &str,
        control: impl IntoElement,
    ) -> impl IntoElement {
        div()
            .flex()
            .items_center()
            .justify_between()
            .gap(px(16.0))
            .w_full()
            .min_w(px(0.0))
            .overflow_hidden()
            .py(px(12.0))
            .border_b_1()
            .border_color(ShellDeckColors::border())
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(2.0))
                    .flex_1()
                    .min_w(px(0.0))
                    .overflow_hidden()
                    .child(
                        div()
                            .min_w(px(0.0))
                            .line_clamp(1)
                            .text_size(px(13.0))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(ShellDeckColors::text_primary())
                            .child(label.to_string()),
                    )
                    .child(
                        div()
                            .min_w(px(0.0))
                            .line_clamp(2)
                            .text_size(px(12.0))
                            .text_color(ShellDeckColors::text_muted())
                            .child(description.to_string()),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_shrink_0()
                    .justify_end()
                    // Right padding so the rounded end of a Toggle or the
                    // caret of a Select never sits under the vertical
                    // scrollbar overlay (`scrollable_vertical`) — used to
                    // clip the last ~4-6px and printed a hard vertical
                    // seam. See `.agents/spacing.md`.
                    .pr(px(4.0))
                    .child(control),
            )
    }

    fn render_shortcut_control(
        &self,
        kind: CompanionShortcutKind,
        enabled: bool,
        shortcut: &str,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let entity = cx.entity();
        let capturing = self.shortcut_capture == Some(kind);
        let button_label = if capturing {
            t!("settings.companion.shortcut.capture").to_string()
        } else {
            display_shortcut(shortcut)
        };
        let (status_label, status_variant) = if capturing {
            (
                t!("settings.companion.shortcut.status.capturing").to_string(),
                BadgeVariant::Warning,
            )
        } else {
            match self.shortcut_status(kind) {
                ShortcutRegistrationStatus::Disabled => (
                    t!("settings.companion.shortcut.status.disabled").to_string(),
                    BadgeVariant::Secondary,
                ),
                ShortcutRegistrationStatus::Applying => (
                    t!("settings.companion.shortcut.status.applying").to_string(),
                    BadgeVariant::Warning,
                ),
                ShortcutRegistrationStatus::Registered => (
                    t!("settings.companion.shortcut.status.registered").to_string(),
                    BadgeVariant::Default,
                ),
                ShortcutRegistrationStatus::PendingPortal => (
                    t!("settings.companion.shortcut.status.portal_pending").to_string(),
                    BadgeVariant::Warning,
                ),
                ShortcutRegistrationStatus::Conflict => (
                    t!("settings.companion.shortcut.status.conflict").to_string(),
                    BadgeVariant::Destructive,
                ),
                ShortcutRegistrationStatus::Error(error) => {
                    if shortcut_error_is_portal_missing(error) {
                        (
                            t!("settings.companion.shortcut.status.portal_missing").to_string(),
                            BadgeVariant::Destructive,
                        )
                    } else {
                        (error.clone(), BadgeVariant::Destructive)
                    }
                }
            }
        };

        let toggle_entity = entity.clone();
        let capture_entity = entity.clone();
        let reset_entity = entity;
        div()
            .flex()
            .flex_col()
            .items_end()
            .gap(px(6.0))
            .max_w(px(310.0))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .child(
                        Toggle::new(match kind {
                            CompanionShortcutKind::AiDock => "companion-global-shortcut",
                            CompanionShortcutKind::CommandPalette => {
                                "companion-global-palette-shortcut"
                            }
                        })
                        .checked(enabled)
                        .on_click(move |value, _window, cx| {
                            let value = *value;
                            toggle_entity.update(cx, |this, cx| {
                                match kind {
                                    CompanionShortcutKind::AiDock => {
                                        this.config.companion.global_shortcut_enabled = value;
                                    }
                                    CompanionShortcutKind::CommandPalette => {
                                        this.config.companion.global_palette_shortcut_enabled =
                                            value;
                                    }
                                }
                                this.set_shortcut_status(
                                    kind,
                                    if value {
                                        ShortcutRegistrationStatus::Applying
                                    } else {
                                        ShortcutRegistrationStatus::Disabled
                                    },
                                );
                                this.save_config(cx);
                            });
                        }),
                    )
                    .child(
                        Button::new(
                            match kind {
                                CompanionShortcutKind::AiDock => "capture-ai-dock-shortcut",
                                CompanionShortcutKind::CommandPalette => {
                                    "capture-command-palette-shortcut"
                                }
                            },
                            button_label,
                        )
                        .variant(ButtonVariant::Outline)
                        .size(ButtonSize::Sm)
                        .selected(capturing)
                        .on_click(move |_, window, cx| {
                            capture_entity.update(cx, |this, cx| {
                                this.cancel_shortcut_capture();
                                this.shortcut_capture = Some(kind);
                                this.shortcut_status_before_capture =
                                    Some((kind, this.shortcut_status(kind).clone()));
                                this.shortcut_capture_focus.focus(window);
                                cx.notify();
                            });
                        }),
                    )
                    .child(
                        Button::new(
                            match kind {
                                CompanionShortcutKind::AiDock => "reset-ai-dock-shortcut",
                                CompanionShortcutKind::CommandPalette => {
                                    "reset-command-palette-shortcut"
                                }
                            },
                            t!("settings.companion.shortcut.reset").to_string(),
                        )
                        .variant(ButtonVariant::Ghost)
                        .size(ButtonSize::Sm)
                        .on_click(move |_, _window, cx| {
                            reset_entity.update(cx, |this, cx| {
                                this.reset_shortcut(kind, cx);
                            });
                        }),
                    ),
            )
            .child(
                Badge::new(status_label)
                    .variant(status_variant)
                    .max_w(px(310.0)),
            )
    }

    fn render_general_settings(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let entity = cx.entity();
        div()
            .flex()
            .flex_col()
            .gap(px(4.0))
            .child(Self::render_setting_row(
                t!("settings.language.label").as_ref(),
                t!("settings.language.description").as_ref(),
                div()
                    .w(px(180.0))
                    .child(self.general_language_select.clone()),
            ))
            .child(Self::render_setting_row(
                t!("settings.general.auto_connect.label").as_ref(),
                t!("settings.general.auto_connect.description").as_ref(),
                Self::bind_toggle(
                    "general-auto-connect",
                    self.config.general.auto_connect_on_startup,
                    &entity,
                    |this, value| {
                        this.config.general.auto_connect_on_startup = value;
                    },
                ),
            ))
            .child(Self::render_setting_row(
                t!("settings.general.notifications.label").as_ref(),
                t!("settings.general.notifications.description").as_ref(),
                Self::bind_toggle(
                    "general-notifications",
                    self.config.general.show_notifications,
                    &entity,
                    |this, value| {
                        this.config.general.show_notifications = value;
                    },
                ),
            ))
            .child(Self::render_setting_row(
                t!("settings.general.confirm_close.label").as_ref(),
                t!("settings.general.confirm_close.description").as_ref(),
                Self::bind_toggle(
                    "general-confirm-close",
                    self.config.general.confirm_before_close,
                    &entity,
                    |this, value| {
                        this.config.general.confirm_before_close = value;
                    },
                ),
            ))
            .child(Self::render_setting_row(
                t!("settings.general.tmux.label").as_ref(),
                t!("settings.general.tmux.description").as_ref(),
                Self::bind_toggle(
                    "general-tmux",
                    self.config.general.auto_attach_tmux,
                    &entity,
                    |this, value| {
                        this.config.general.auto_attach_tmux = value;
                    },
                ),
            ))
            .child(Self::render_setting_row(
                t!("settings.general.auto_update.label").as_ref(),
                t!("settings.general.auto_update.description").as_ref(),
                Self::bind_toggle(
                    "general-auto-update",
                    self.config.general.auto_update,
                    &entity,
                    |this, value| {
                        this.config.general.auto_update = value;
                    },
                ),
            ))
            .child(Self::render_setting_row(
                t!("settings.general.autostart.label").as_ref(),
                t!("settings.general.autostart.description").as_ref(),
                // Deliberately NOT `bind_toggle`: autostart writes to the OS
                // (XDG autostart / launchd / registry) and may fail; the
                // toggle only "sticks" once the workspace confirms the OS
                // accepted the change. See `Workspace::apply_autostart_request`.
                Self::bind_autostart_toggle(
                    "general-autostart",
                    self.config.general.autostart,
                    &entity,
                ),
            ))
            .child(Self::render_setting_row(
                t!("settings.companion.start_hidden.label").as_ref(),
                t!("settings.companion.start_hidden.description").as_ref(),
                Self::bind_toggle(
                    "companion-start-hidden",
                    self.config.companion.start_hidden,
                    &entity,
                    |this, value| {
                        this.config.companion.start_hidden = value;
                    },
                ),
            ))
            .child(Self::render_setting_row(
                t!("settings.companion.global_shortcut.label").as_ref(),
                t!("settings.companion.global_shortcut.description").as_ref(),
                self.render_shortcut_control(
                    CompanionShortcutKind::AiDock,
                    self.config.companion.global_shortcut_enabled,
                    &self.config.companion.global_shortcut,
                    cx,
                ),
            ))
            .child(Self::render_setting_row(
                t!("settings.companion.global_palette_shortcut.label").as_ref(),
                t!("settings.companion.global_palette_shortcut.description").as_ref(),
                self.render_shortcut_control(
                    CompanionShortcutKind::CommandPalette,
                    self.config.companion.global_palette_shortcut_enabled,
                    &self.config.companion.global_palette_shortcut,
                    cx,
                ),
            ))
            .child(Self::render_setting_row(
                t!("settings.general.onboarding_replay.label").as_ref(),
                t!("settings.general.onboarding_replay.description").as_ref(),
                Button::new(
                    "onboarding-replay",
                    t!("settings.general.onboarding_replay.button").to_string(),
                )
                .variant(ButtonVariant::Outline)
                .on_click(cx.listener(|_this, _, _window, cx| {
                    cx.emit(SettingsEvent::ShowOnboarding);
                })),
            ))
            // System-tray preferences — grouped at the bottom of the
            // Général tab because they're companion-mode polish (opt-in
            // per notification category + close-button minimizes to
            // tray). All persisted via `AppConfig.tray`.
            .child(Self::render_setting_row(
                t!("settings.tray.close_to_tray.label").as_ref(),
                t!("settings.tray.close_to_tray.description").as_ref(),
                Self::bind_toggle(
                    "tray-close-to-tray",
                    self.config.tray.close_to_tray,
                    &entity,
                    |this, value| {
                        this.config.tray.close_to_tray = value;
                    },
                ),
            ))
            .child(Self::render_setting_row(
                t!("settings.tray.notify_new_tickets.label").as_ref(),
                t!("settings.tray.notify_new_tickets.description").as_ref(),
                Self::bind_toggle(
                    "tray-notify-new-tickets",
                    self.config.tray.notify_new_tickets,
                    &entity,
                    |this, value| {
                        this.config.tray.notify_new_tickets = value;
                    },
                ),
            ))
            .child(Self::render_setting_row(
                t!("settings.tray.notify_jean_pending.label").as_ref(),
                t!("settings.tray.notify_jean_pending.description").as_ref(),
                Self::bind_toggle(
                    "tray-notify-jean-pending",
                    self.config.tray.notify_jean_pending,
                    &entity,
                    |this, value| {
                        this.config.tray.notify_jean_pending = value;
                    },
                ),
            ))
            .child(Self::render_setting_row(
                t!("settings.tray.notify_ssh_disconnect.label").as_ref(),
                t!("settings.tray.notify_ssh_disconnect.description").as_ref(),
                Self::bind_toggle(
                    "tray-notify-ssh-disconnect",
                    self.config.tray.notify_ssh_disconnect,
                    &entity,
                    |this, value| {
                        this.config.tray.notify_ssh_disconnect = value;
                    },
                ),
            ))
            .child(Self::render_setting_row(
                t!("settings.tray.notify_fleet_done.label").as_ref(),
                t!("settings.tray.notify_fleet_done.description").as_ref(),
                Self::bind_toggle(
                    "tray-notify-fleet-done",
                    self.config.tray.notify_fleet_done,
                    &entity,
                    |this, value| {
                        this.config.tray.notify_fleet_done = value;
                    },
                ),
            ))
            .child(Self::render_setting_row(
                t!("settings.tray.notify_ai_tasks.label").as_ref(),
                t!("settings.tray.notify_ai_tasks.description").as_ref(),
                Self::bind_toggle(
                    "tray-notify-ai-tasks",
                    self.config.tray.notify_ai_tasks,
                    &entity,
                    |this, value| {
                        this.config.tray.notify_ai_tasks = value;
                    },
                ),
            ))
            .child(self.render_cloud_sync_settings(cx))
    }

    /// Shared adabraka `Toggle` bound to a single `bool` config field. The
    /// `set` callback mutates the field on `SettingsView`; `save_config` is
    /// invoked automatically so every toggle in the Settings screen persists
    /// on the fly (no “Save” button required).
    fn bind_toggle(
        id: &'static str,
        checked: bool,
        entity: &Entity<SettingsView>,
        set: impl Fn(&mut SettingsView, bool) + 'static,
    ) -> Toggle {
        let entity = entity.clone();
        Toggle::new(id)
            .checked(checked)
            .on_click(move |value, _window, cx| {
                let value = *value;
                entity.update(cx, |this, cx| {
                    set(this, value);
                    this.save_config(cx);
                });
            })
    }

    /// Autostart toggle. Emits `SettingsEvent::AutostartRequested(desired)`
    /// instead of updating the config: the workspace attempts the
    /// OS-level change asynchronously, then commits the field (via
    /// `set_autostart` + `save_config`) only if the OS accepted it. If
    /// the OS refuses the toggle stays where it was — no disk write, no
    /// visual bounce.
    fn bind_autostart_toggle(
        id: &'static str,
        checked: bool,
        entity: &Entity<SettingsView>,
    ) -> impl IntoElement {
        let entity = entity.clone();
        Toggle::new(id)
            .checked(checked)
            .on_click(move |value, _window, cx| {
                let value = *value;
                entity.update(cx, |_, cx| {
                    cx.emit(SettingsEvent::AutostartRequested(value));
                });
            })
    }

    /// Commit an autostart change once the workspace confirmed the OS
    /// accepted it. Bypasses the toggle path so the workspace doesn't
    /// bounce a fresh `AutostartRequested` back at itself.
    pub fn set_autostart(&mut self, value: bool, cx: &mut Context<Self>) {
        self.config.general.autostart = value;
        self.save_config(cx);
    }

    /// Mask a Cloud Sync token for display: never show the full secret, just a
    /// hint of its tail (e.g. `sd_…9f2a`), or a placeholder when unset.
    fn mask_token(token: &str) -> String {
        if token.is_empty() {
            return t!("settings.cloud_sync.not_configured").to_string();
        }
        let last4: String = {
            let chars: Vec<char> = token.chars().collect();
            let start = chars.len().saturating_sub(4);
            chars[start..].iter().collect()
        };
        format!("sd_…{}", last4)
    }

    /// Read-only Cloud Sync status block for the General tab, plus a "Sync now"
    /// button that dispatches [`CloudSyncNow`]. Editing happens in
    /// `shelldeck.toml`; this surface is intentionally view-only.
    fn render_cloud_sync_settings(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let cfg = &self.config.cloud_sync;
        let status_text = if cfg.enabled {
            t!("settings.cloud_sync.enabled").to_string()
        } else {
            t!("settings.cloud_sync.disabled").to_string()
        };
        let token_display = Self::mask_token(&cfg.token);

        let value_text = |s: String| {
            div()
                .text_size(px(13.0))
                .font_weight(FontWeight::MEDIUM)
                .text_color(ShellDeckColors::text_primary())
                .child(s)
        };

        let account_text = match &self.config.account {
            Some(a) if !a.email.is_empty() => format!("{} ({})", a.display_name(), a.email),
            Some(a) => a.display_name(),
            None => t!("settings.cloud_sync.not_signed_in").to_string(),
        };

        div()
            .flex()
            .flex_col()
            .child(Self::render_about_section(
                t!("settings.cloud_sync.section").as_ref(),
            ))
            .child(Self::render_setting_row(
                t!("settings.cloud_sync.account.label").as_ref(),
                t!("settings.cloud_sync.account.description").as_ref(),
                value_text(account_text),
            ))
            .child(Self::render_setting_row(
                t!("settings.cloud_sync.status.label").as_ref(),
                t!("settings.cloud_sync.status.description").as_ref(),
                value_text(status_text),
            ))
            .child(Self::render_setting_row(
                t!("settings.cloud_sync.server.label").as_ref(),
                t!("settings.cloud_sync.server.description").as_ref(),
                value_text(cfg.base_url.clone()),
            ))
            .child(Self::render_setting_row(
                t!("settings.cloud_sync.token.label").as_ref(),
                t!("settings.cloud_sync.token.description").as_ref(),
                value_text(token_display),
            ))
            .child(
                div()
                    .mt(px(10.0))
                    .text_size(px(12.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(
                        "Edit [cloud_sync] in shelldeck.toml — get a token at \
                         https://manage.inklura.fr/manage/shelldeck",
                    ),
            )
            .child(
                div()
                    .id("cloud-sync-now")
                    .mt(px(12.0))
                    .w(px(120.0))
                    .px(px(14.0))
                    .py(px(8.0))
                    .rounded(px(6.0))
                    .bg(ShellDeckColors::primary())
                    .text_size(px(13.0))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(white())
                    .cursor_pointer()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(t!("settings.cloud_sync.sync_now").to_string())
                    .on_click(cx.listener(|_this, _, _window, cx| {
                        cx.dispatch_action(&CloudSyncNow);
                    })),
            )
    }

    fn render_terminal_settings(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let entity = cx.entity();
        div()
            .flex()
            .flex_col()
            .gap(px(4.0))
            .child(Self::render_setting_row(
                t!("settings.terminal.font_size.label").as_ref(),
                t!("settings.terminal.font_size.description").as_ref(),
                Self::render_number_stepper(
                    "terminal-font-size",
                    format!("{}px", self.config.terminal.font_size),
                    cx.listener(|this, _, _, cx| {
                        let new = (this.config.terminal.font_size - 1.0).max(8.0);
                        if (new - this.config.terminal.font_size).abs() < f32::EPSILON {
                            return;
                        }
                        this.config.terminal.font_size = new;
                        this.save_config(cx);
                    }),
                    cx.listener(|this, _, _, cx| {
                        let new = (this.config.terminal.font_size + 1.0).min(32.0);
                        if (new - this.config.terminal.font_size).abs() < f32::EPSILON {
                            return;
                        }
                        this.config.terminal.font_size = new;
                        this.save_config(cx);
                    }),
                ),
            ))
            .child(Self::render_setting_row(
                t!("settings.terminal.font_family.label").as_ref(),
                t!("settings.terminal.font_family.description").as_ref(),
                div()
                    .w(px(200.0))
                    .child(self.terminal_font_family_select.clone()),
            ))
            .child(Self::render_setting_row(
                t!("settings.terminal.scrollback.label").as_ref(),
                t!("settings.terminal.scrollback.description").as_ref(),
                Self::render_number_stepper(
                    "terminal-scrollback",
                    format!("{}", self.config.terminal.scrollback_lines),
                    cx.listener(|this, _, _, cx| {
                        let new = this
                            .config
                            .terminal
                            .scrollback_lines
                            .saturating_sub(1000)
                            .max(1000);
                        if new == this.config.terminal.scrollback_lines {
                            return;
                        }
                        this.config.terminal.scrollback_lines = new;
                        this.save_config(cx);
                    }),
                    cx.listener(|this, _, _, cx| {
                        let new = (this.config.terminal.scrollback_lines + 1000).min(100_000);
                        if new == this.config.terminal.scrollback_lines {
                            return;
                        }
                        this.config.terminal.scrollback_lines = new;
                        this.save_config(cx);
                    }),
                ),
            ))
            .child(Self::render_setting_row(
                t!("settings.terminal.cursor_style.label").as_ref(),
                t!("settings.terminal.cursor_style.description").as_ref(),
                div()
                    .w(px(140.0))
                    .child(self.terminal_cursor_style_select.clone()),
            ))
            .child(Self::render_setting_row(
                t!("settings.terminal.cursor_blink.label").as_ref(),
                t!("settings.terminal.cursor_blink.description").as_ref(),
                Self::bind_toggle(
                    "terminal-cursor-blink",
                    self.config.terminal.cursor_blink,
                    &entity,
                    |this, value| {
                        this.config.terminal.cursor_blink = value;
                    },
                ),
            ))
    }

    fn render_ai_settings(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let entity = cx.entity();
        let backend = self.config.ai.backend;
        let cli_missing = backend.is_cli() && !configured_cli_available(&self.config.ai);
        let (status, status_color) = match &self.ai_connection_state {
            AiConnectionState::Testing => (
                t!("settings.ai.status.testing").to_string(),
                ShellDeckColors::warning(),
            ),
            AiConnectionState::Connected => (
                t!("settings.ai.status.connected").to_string(),
                ShellDeckColors::success(),
            ),
            AiConnectionState::Failed(error) => (error.clone(), ShellDeckColors::error()),
            AiConnectionState::NotTested => match backend {
                AiBackend::Disabled => (
                    t!("settings.ai.status.disabled").to_string(),
                    ShellDeckColors::text_muted(),
                ),
                AiBackend::ClaudeCli | AiBackend::CodexCli | AiBackend::AiderCli => (
                    local_backend_status(backend.cli_command().expect("CLI backend"), !cli_missing),
                    if cli_missing {
                        ShellDeckColors::error()
                    } else {
                        ShellDeckColors::text_muted()
                    },
                ),
                AiBackend::OpenAi | AiBackend::Anthropic => (
                    t!("settings.ai.status.not_tested").to_string(),
                    ShellDeckColors::text_muted(),
                ),
            },
        };
        let testing = matches!(self.ai_connection_state, AiConnectionState::Testing);

        let model_parent = entity.clone();
        let model_input = Input::new(&self.ai_model_state)
            .size(InputSize::Sm)
            .placeholder(if self.config.ai.model.is_empty() {
                backend.default_model().to_string()
            } else {
                self.config.ai.model.clone()
            })
            .on_blur(move |value, cx| {
                model_parent.update(cx, |this, cx| {
                    let value = value.trim().to_string();
                    if this.config.ai.model != value {
                        this.config.ai.model = value;
                        this.ai_connection_state = AiConnectionState::NotTested;
                        this.save_config(cx);
                    }
                });
            });

        let mut root = div()
            .flex()
            .flex_col()
            .gap(px(4.0))
            .child(Self::render_setting_row(
                t!("settings.ai.enabled.label").as_ref(),
                t!("settings.ai.enabled.description").as_ref(),
                Self::bind_toggle(
                    "ai-enabled",
                    self.config.ai.enabled,
                    &entity,
                    |this, value| {
                        this.config.ai.enabled = value;
                        this.ai_connection_state = AiConnectionState::NotTested;
                    },
                ),
            ))
            .child(Self::render_setting_row(
                t!("settings.ai.backend.label").as_ref(),
                t!("settings.ai.backend.description").as_ref(),
                div().w(px(220.0)).child(self.ai_backend_select.clone()),
            ))
            .child(Self::render_setting_row(
                t!("settings.ai.status.label").as_ref(),
                t!("settings.ai.status.description").as_ref(),
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .text_size(px(13.0))
                    .text_color(status_color)
                    .when(testing, |row| {
                        row.child(
                            Spinner::new()
                                .size(SpinnerSize::Xs)
                                .variant(SpinnerVariant::Primary),
                        )
                    })
                    .child(status),
            ))
            .child(Self::render_setting_row(
                t!("settings.ai.model.label").as_ref(),
                t!("settings.ai.model.description").as_ref(),
                div().w(px(220.0)).child(model_input),
            ));

        if let Some(provider) = backend.provider_key() {
            let key_state = self.ai_api_key_state.clone();
            let provider = provider.to_string();
            root = root.child(Self::render_setting_row(
                t!("settings.ai.api_key.label").as_ref(),
                t!("settings.ai.api_key.description").as_ref(),
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .w(px(300.0))
                    .child(
                        div().flex_grow().child(
                            Input::new(&self.ai_api_key_state)
                                .size(InputSize::Sm)
                                .password(true)
                                .placeholder(t!("settings.ai.api_key.placeholder").to_string()),
                        ),
                    )
                    .child(
                        Button::new(
                            "ai-api-key-save",
                            t!("settings.ai.api_key.save").to_string(),
                        )
                        .variant(ButtonVariant::Outline)
                        .on_click(cx.listener(
                            move |this, _, _window, cx| {
                                let value = key_state.read(cx).content().trim().to_string();
                                if value.is_empty() {
                                    this.ai_connection_state = AiConnectionState::Failed(
                                        t!("settings.ai.api_key.required").to_string(),
                                    );
                                    cx.notify();
                                    return;
                                }
                                key_state.update(cx, |state, cx| {
                                    state.reset(cx);
                                });
                                cx.emit(SettingsEvent::AiApiKeyStored { backend, value });
                            },
                        )),
                    )
                    .child(
                        Button::new(
                            "ai-api-key-delete",
                            t!("settings.ai.api_key.delete").to_string(),
                        )
                        .variant(ButtonVariant::Ghost)
                        .on_click(cx.listener(
                            move |_this, _, _window, cx| {
                                cx.emit(SettingsEvent::AiApiKeyDeleted {
                                    backend: if provider == "openai" {
                                        AiBackend::OpenAi
                                    } else {
                                        AiBackend::Anthropic
                                    },
                                });
                            },
                        )),
                    ),
            ));
        }

        root = root.child(Self::render_setting_row(
            t!("settings.ai.test.label").as_ref(),
            t!("settings.ai.test.description").as_ref(),
            Button::new(
                "ai-test-connection",
                if testing {
                    t!("settings.ai.test.testing").to_string()
                } else {
                    t!("settings.ai.test.button").to_string()
                },
            )
            .variant(ButtonVariant::Outline)
            .icon(IconSource::Named("refresh-cw".into()))
            .disabled(
                testing || backend == AiBackend::Disabled || cli_missing || !self.config.ai.enabled,
            )
            .on_click(cx.listener(|this, _, _window, cx| {
                this.ai_connection_state = AiConnectionState::Testing;
                cx.emit(SettingsEvent::AiTestRequested(this.config.clone()));
                cx.notify();
            })),
        ));

        root.child(Self::render_about_section(
            t!("settings.ai.clippy.section").as_ref(),
        ))
        .child(Self::render_setting_row(
            t!("settings.ai.clippy.auto_clipboard.label").as_ref(),
            t!("settings.ai.clippy.auto_clipboard.description").as_ref(),
            Self::bind_toggle(
                "ai-clippy-auto-clipboard",
                self.config.clippy.auto_import_clipboard_on_shortcut,
                &entity,
                |this, value| this.config.clippy.auto_import_clipboard_on_shortcut = value,
            ),
        ))
        .child(Self::render_about_section(
            t!("settings.ai.surfaces.section").as_ref(),
        ))
        .child(ai_surface_row(
            "ai-surface-support",
            "support",
            self.config.ai.surfaces.support,
            &entity,
            |this, value| this.config.ai.surfaces.support = value,
        ))
        .child(ai_surface_row(
            "ai-surface-issues",
            "issues",
            self.config.ai.surfaces.issues,
            &entity,
            |this, value| this.config.ai.surfaces.issues = value,
        ))
        .child(ai_surface_row(
            "ai-surface-scripts",
            "scripts",
            self.config.ai.surfaces.scripts,
            &entity,
            |this, value| this.config.ai.surfaces.scripts = value,
        ))
        .child(ai_surface_row(
            "ai-surface-terminal",
            "terminal",
            self.config.ai.surfaces.terminal,
            &entity,
            |this, value| this.config.ai.surfaces.terminal = value,
        ))
        .child(ai_surface_row(
            "ai-surface-jean",
            "jean",
            self.config.ai.surfaces.jean,
            &entity,
            |this, value| this.config.ai.surfaces.jean = value,
        ))
        .child(ai_surface_row(
            "ai-surface-naming",
            "naming",
            self.config.ai.surfaces.naming,
            &entity,
            |this, value| this.config.ai.surfaces.naming = value,
        ))
        .child(ai_surface_row(
            "ai-surface-recent",
            "recent",
            self.config.ai.surfaces.recent,
            &entity,
            |this, value| this.config.ai.surfaces.recent = value,
        ))
        .child(ai_surface_row(
            "ai-surface-clippy",
            "clippy",
            self.config.ai.surfaces.clippy,
            &entity,
            |this, value| this.config.ai.surfaces.clippy = value,
        ))
        .child(Self::render_about_section(
            t!("settings.ai.policies.section").as_ref(),
        ))
        .child(ai_policy_row(
            "ai-policy-support-send",
            "support_send",
            self.config.ai.policies.support_send,
            &entity,
            |this, value| this.config.ai.policies.support_send = value,
        ))
        .child(ai_policy_row(
            "ai-policy-support-triage",
            "support_triage",
            self.config.ai.policies.support_triage,
            &entity,
            |this, value| this.config.ai.policies.support_triage = value,
        ))
        .child(ai_policy_row(
            "ai-policy-terminal-execute",
            "terminal_execute",
            self.config.ai.policies.terminal_execute,
            &entity,
            |this, value| this.config.ai.policies.terminal_execute = value,
        ))
        .child(ai_policy_row(
            "ai-policy-script-execute",
            "script_execute",
            self.config.ai.policies.script_execute,
            &entity,
            |this, value| this.config.ai.policies.script_execute = value,
        ))
        .child(ai_policy_row(
            "ai-policy-jean-dispatch",
            "jean_dispatch",
            self.config.ai.policies.jean_dispatch,
            &entity,
            |this, value| this.config.ai.policies.jean_dispatch = value,
        ))
        .child(ai_policy_row(
            "ai-policy-fleet-dispatch",
            "fleet_dispatch",
            self.config.ai.policies.fleet_dispatch,
            &entity,
            |this, value| this.config.ai.policies.fleet_dispatch = value,
        ))
    }

    pub fn set_ai_connection_result(&mut self, result: Result<(), String>, cx: &mut Context<Self>) {
        self.ai_connection_state = match result {
            Ok(()) => AiConnectionState::Connected,
            Err(error) => AiConnectionState::Failed(error),
        };
        cx.notify();
    }

    pub fn reset_ai_connection_state(&mut self, cx: &mut Context<Self>) {
        self.ai_connection_state = AiConnectionState::NotTested;
        cx.notify();
    }

    /// Shared `[- value +]` stepper — used by every numeric setting that
    /// doesn't have a natural adabraka NumberInput fit (font size, scrollback,
    /// sidebar width, UI font size). The `-`/`+` buttons are adabraka
    /// `IconButton` so clicks land reliably through the icon (the previous
    /// hand-rolled `div + svg` swallowed events on some builds).
    ///
    /// The `on_*` closures use the raw GPUI listener signature (`(&ClickEvent,
    /// &mut Window, &mut App)`) so callers can pass `cx.listener(...)`
    /// directly — the same shape adabraka's own `on_click` expects.
    fn render_number_stepper(
        _id: &str,
        value: String,
        on_minus: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
        on_plus: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> impl IntoElement {
        div()
            .flex()
            .items_center()
            .gap(px(8.0))
            .child(
                IconButton::new(IconSource::Named("minus".into()))
                    .size(gpui::px(28.0))
                    .icon_size(gpui::px(14.0))
                    .no_background(true)
                    .on_click(on_minus),
            )
            .child(
                div()
                    .min_w(px(64.0))
                    .flex()
                    .justify_center()
                    .text_size(px(13.0))
                    .text_color(ShellDeckColors::text_primary())
                    .child(value),
            )
            .child(
                IconButton::new(IconSource::Named("plus".into()))
                    .size(gpui::px(28.0))
                    .icon_size(gpui::px(14.0))
                    .no_background(true)
                    .on_click(on_plus),
            )
    }

    fn render_editor_settings(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let entity = cx.entity();
        div()
            .flex()
            .flex_col()
            .gap(px(4.0))
            .child(Self::render_setting_row(
                t!("settings.editor.font_size.label").as_ref(),
                t!("settings.editor.font_size.description").as_ref(),
                Self::render_number_stepper(
                    "editor-font-size",
                    format!("{}px", self.config.editor.font_size),
                    cx.listener(|this, _, _, cx| {
                        let new = (this.config.editor.font_size - 1.0).max(8.0);
                        if (new - this.config.editor.font_size).abs() < f32::EPSILON {
                            return;
                        }
                        this.config.editor.font_size = new;
                        this.save_config(cx);
                    }),
                    cx.listener(|this, _, _, cx| {
                        let new = (this.config.editor.font_size + 1.0).min(40.0);
                        if (new - this.config.editor.font_size).abs() < f32::EPSILON {
                            return;
                        }
                        this.config.editor.font_size = new;
                        this.save_config(cx);
                    }),
                ),
            ))
            // Font family — adabraka Select (searchable dropdown).
            .child(Self::render_setting_row(
                t!("settings.editor.font_family.label").as_ref(),
                t!("settings.editor.font_family.description").as_ref(),
                div()
                    .w(px(200.0))
                    .child(self.editor_font_family_select.clone()),
            ))
            // Tab size — adabraka Select (2 / 4 / 8).
            .child(Self::render_setting_row(
                t!("settings.editor.tab_size.label").as_ref(),
                t!("settings.editor.tab_size.description").as_ref(),
                div()
                    .w(px(100.0))
                    .child(self.editor_tab_size_select.clone()),
            ))
            // Toggles — all through adabraka Toggle, so the OFF state renders
            // with theme-aware muted/background tokens (fixes the visible
            // "seam" we had in Solarized Light with the hand-rolled toggle).
            .child(Self::render_setting_row(
                t!("settings.editor.insert_spaces.label").as_ref(),
                t!("settings.editor.insert_spaces.description").as_ref(),
                Self::bind_toggle(
                    "editor-insert-spaces",
                    self.config.editor.insert_spaces,
                    &entity,
                    |this, value| {
                        this.config.editor.insert_spaces = value;
                    },
                ),
            ))
            .child(Self::render_setting_row(
                t!("settings.editor.show_line_numbers.label").as_ref(),
                t!("settings.editor.show_line_numbers.description").as_ref(),
                Self::bind_toggle(
                    "editor-line-numbers",
                    self.config.editor.show_line_numbers,
                    &entity,
                    |this, value| {
                        this.config.editor.show_line_numbers = value;
                    },
                ),
            ))
            .child(Self::render_setting_row(
                t!("settings.editor.show_whitespace.label").as_ref(),
                t!("settings.editor.show_whitespace.description").as_ref(),
                Self::bind_toggle(
                    "editor-whitespace",
                    self.config.editor.show_whitespace,
                    &entity,
                    |this, value| {
                        this.config.editor.show_whitespace = value;
                    },
                ),
            ))
            .child(Self::render_setting_row(
                t!("settings.editor.word_wrap.label").as_ref(),
                t!("settings.editor.word_wrap.description").as_ref(),
                Self::bind_toggle(
                    "editor-word-wrap",
                    self.config.editor.word_wrap,
                    &entity,
                    |this, value| {
                        this.config.editor.word_wrap = value;
                    },
                ),
            ))
            .child(Self::render_setting_row(
                t!("settings.editor.cursor_blink.label").as_ref(),
                t!("settings.editor.cursor_blink.description").as_ref(),
                Self::bind_toggle(
                    "editor-cursor-blink",
                    self.config.editor.cursor_blink,
                    &entity,
                    |this, value| {
                        this.config.editor.cursor_blink = value;
                    },
                ),
            ))
    }

    fn render_appearance_settings(&self, cx: &mut Context<Self>) -> impl IntoElement {
        // App theme picker — a live-preview card per built-in theme.
        let current_theme = self.config.theme.clone();
        let mut app_theme_cards = div().flex().gap(px(8.0)).flex_wrap();

        for pref in ThemePreference::all() {
            let pref = pref.clone();
            let is_active = current_theme == pref;
            let label = pref.display_name().to_string();
            let p = palette_for(&pref);

            // Mini app mock-up: sidebar stripe + content with an accent bar and
            // a couple of "text" lines, all rendered in the theme's own colors.
            let preview = div()
                .w_full()
                .flex_grow()
                .rounded(px(4.0))
                .overflow_hidden()
                .flex()
                .child(
                    // Sidebar
                    div()
                        .w(px(20.0))
                        .h_full()
                        .bg(p.bg_sidebar)
                        .border_r_1()
                        .border_color(p.border)
                        .flex()
                        .flex_col()
                        .items_center()
                        .justify_center()
                        .gap(px(3.0))
                        .child(div().w(px(8.0)).h(px(3.0)).rounded(px(1.0)).bg(p.primary))
                        .child(
                            div()
                                .w(px(8.0))
                                .h(px(3.0))
                                .rounded(px(1.0))
                                .bg(p.text_muted),
                        ),
                )
                .child(
                    // Content
                    div()
                        .flex_grow()
                        .h_full()
                        .bg(p.bg_primary)
                        .p(px(6.0))
                        .flex()
                        .flex_col()
                        .gap(px(4.0))
                        .child(div().w(px(30.0)).h(px(4.0)).rounded(px(2.0)).bg(p.primary))
                        .child(
                            div()
                                .w(px(46.0))
                                .h(px(3.0))
                                .rounded(px(1.0))
                                .bg(p.text_primary),
                        )
                        .child(
                            div()
                                .w(px(38.0))
                                .h(px(3.0))
                                .rounded(px(1.0))
                                .bg(p.text_muted),
                        )
                        .child(
                            div()
                                .flex()
                                .gap(px(3.0))
                                .child(div().w(px(8.0)).h(px(8.0)).rounded(px(2.0)).bg(p.success))
                                .child(div().w(px(8.0)).h(px(8.0)).rounded(px(2.0)).bg(p.warning))
                                .child(div().w(px(8.0)).h(px(8.0)).rounded(px(2.0)).bg(p.error)),
                        ),
                );

            let mut card = div()
                .id(ElementId::from(SharedString::from(format!(
                    "app-theme-{}",
                    label
                ))))
                .w(px(132.0))
                .h(px(92.0))
                .rounded(px(6.0))
                .border_1()
                .cursor_pointer()
                .flex()
                .flex_col()
                .gap(px(4.0))
                .p(px(4.0));

            if is_active {
                card = card.border_color(ShellDeckColors::primary());
            } else {
                card = card
                    .border_color(ShellDeckColors::border())
                    .hover(|el| el.border_color(ShellDeckColors::primary()));
            }

            card = card
                .child(preview)
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .px(px(2.0))
                        .child(
                            div()
                                .text_size(px(10.0))
                                .font_weight(if is_active {
                                    FontWeight::SEMIBOLD
                                } else {
                                    FontWeight::NORMAL
                                })
                                .text_color(if is_active {
                                    ShellDeckColors::primary()
                                } else {
                                    ShellDeckColors::text_primary()
                                })
                                .child(label.clone()),
                        )
                        .child(
                            div()
                                .text_size(px(10.0))
                                .text_color(ShellDeckColors::primary())
                                .child(if is_active { "\u{2713}" } else { "" }),
                        ),
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.select_app_theme(pref.clone(), cx);
                }));

            app_theme_cards = app_theme_cards.child(card);
        }

        // Terminal theme picker (built-in themes)
        let mut theme_cards = div().flex().gap(px(8.0)).flex_wrap();

        // Parse a "#rrggbb" string into a gpui color (falls back to black).
        let hex_color = |hex: &str| -> Rgba {
            let h = hex.trim_start_matches('#');
            let v = u32::from_str_radix(h.get(0..6).unwrap_or("000000"), 16).unwrap_or(0);
            rgb(v)
        };

        let active_theme = self.config.terminal.theme.clone();

        for terminal_theme in TerminalTheme::builtins() {
            let name = terminal_theme.name.clone();
            let is_active = name == active_theme;
            let theme_name = name.clone();

            let bg = hex_color(&terminal_theme.background);
            let fg = hex_color(&terminal_theme.foreground);
            // A few representative ANSI swatches (red, green, blue, magenta).
            let swatches = [
                hex_color(&terminal_theme.ansi_colors[1]),
                hex_color(&terminal_theme.ansi_colors[2]),
                hex_color(&terminal_theme.ansi_colors[4]),
                hex_color(&terminal_theme.ansi_colors[5]),
            ];

            // Live preview: a mini "terminal" rendered in the theme's own colors.
            let mut preview = div()
                .w_full()
                .flex_grow()
                .rounded(px(4.0))
                .bg(bg)
                .p(px(6.0))
                .flex()
                .flex_col()
                .justify_between()
                .child(
                    div()
                        .text_size(px(10.0))
                        .text_color(fg)
                        .child(t!("settings.theme.preview_sample").to_string()),
                );
            let mut dots = div().flex().gap(px(3.0));
            for s in swatches {
                dots = dots.child(div().w(px(8.0)).h(px(8.0)).rounded(px(2.0)).bg(s));
            }
            preview = preview.child(dots);

            let mut card = div()
                .id(ElementId::from(SharedString::from(format!(
                    "theme-{}",
                    name
                ))))
                .w(px(124.0))
                .h(px(82.0))
                .rounded(px(6.0))
                .border_1()
                .cursor_pointer()
                .flex()
                .flex_col()
                .gap(px(4.0))
                .p(px(4.0));

            if is_active {
                card = card.border_color(ShellDeckColors::primary());
            } else {
                card = card
                    .border_color(ShellDeckColors::border())
                    .hover(|el| el.border_color(ShellDeckColors::primary()));
            }

            card = card
                .child(preview)
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .px(px(2.0))
                        .child(
                            div()
                                .text_size(px(10.0))
                                .font_weight(if is_active {
                                    FontWeight::SEMIBOLD
                                } else {
                                    FontWeight::NORMAL
                                })
                                .text_color(if is_active {
                                    ShellDeckColors::primary()
                                } else {
                                    ShellDeckColors::text_primary()
                                })
                                .child(name.clone()),
                        )
                        .child(
                            div()
                                .text_size(px(10.0))
                                .text_color(ShellDeckColors::primary())
                                .child(if is_active { "\u{2713}" } else { "" }),
                        ),
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.select_terminal_theme(&theme_name, cx);
                    cx.notify();
                }));

            theme_cards = theme_cards.child(card);
        }

        let mut character_cards = div().flex().gap(px(8.0)).flex_wrap();
        for (id, label, accent) in [
            (
                "none",
                t!("settings.companion.characters.none").to_string(),
                ShellDeckColors::text_muted(),
            ),
            (
                "clippy",
                t!("settings.companion.characters.clippy").to_string(),
                ShellDeckColors::primary(),
            ),
            (
                "shelly",
                t!("settings.companion.characters.shelly").to_string(),
                ShellDeckColors::success(),
            ),
            (
                "spark",
                t!("settings.companion.characters.spark").to_string(),
                ShellDeckColors::warning(),
            ),
            (
                "byte",
                t!("settings.companion.characters.byte").to_string(),
                ShellDeckColors::error(),
            ),
            (
                "orbit",
                t!("settings.companion.characters.orbit").to_string(),
                ShellDeckColors::primary().opacity(0.75),
            ),
            (
                "nox",
                t!("settings.companion.characters.nox").to_string(),
                ShellDeckColors::text_primary(),
            ),
        ] {
            let active = self.config.clippy.appearance.character_id().as_str() == id;
            let preview = if id == "none" {
                div()
                    .w(px(54.0))
                    .h(px(44.0))
                    .rounded(px(8.0))
                    .bg(accent.opacity(0.12))
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_size(px(18.0))
                    .text_color(accent)
                    .child("—")
                    .into_any_element()
            } else {
                img(SharedString::from(format!("characters/{id}/idle.png")))
                    .w(px(54.0))
                    .h(px(44.0))
                    .object_fit(ObjectFit::Contain)
                    .into_any_element()
            };
            character_cards = character_cards.child(
                div()
                    .id(ElementId::from(SharedString::from(format!(
                        "companion-character-{id}"
                    ))))
                    .w(px(112.0))
                    .h(px(96.0))
                    .rounded(px(7.0))
                    .border_1()
                    .border_color(if active {
                        ShellDeckColors::primary()
                    } else {
                        ShellDeckColors::border()
                    })
                    .cursor_pointer()
                    .p(px(8.0))
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_between()
                    .child(preview)
                    .child(
                        div()
                            .text_size(px(11.0))
                            .font_weight(if active {
                                FontWeight::SEMIBOLD
                            } else {
                                FontWeight::NORMAL
                            })
                            .text_color(if active {
                                ShellDeckColors::primary()
                            } else {
                                ShellDeckColors::text_primary()
                            })
                            .child(label),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        apply_character_choice(&mut this.config.clippy.appearance, id);
                        this.save_config(cx);
                    })),
            );
        }

        let mut motion_buttons = div().flex().gap(px(8.0)).flex_wrap();
        for (id, preference, label) in [
            (
                "system",
                CompanionMotionPreference::System,
                t!("settings.companion.characters.motion.system").to_string(),
            ),
            (
                "full",
                CompanionMotionPreference::Full,
                t!("settings.companion.characters.motion.full").to_string(),
            ),
            (
                "reduced",
                CompanionMotionPreference::Reduced,
                t!("settings.companion.characters.motion.reduced").to_string(),
            ),
            (
                "off",
                CompanionMotionPreference::Off,
                t!("settings.companion.characters.motion.off").to_string(),
            ),
        ] {
            motion_buttons = motion_buttons.child(
                Button::new(SharedString::from(format!("companion-motion-{id}")), label)
                    .variant(if self.config.clippy.appearance.motion == preference {
                        ButtonVariant::Secondary
                    } else {
                        ButtonVariant::Outline
                    })
                    .size(ButtonSize::Sm)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.config.clippy.appearance.motion = preference;
                        this.save_config(cx);
                    })),
            );
        }

        let mut scale_buttons = div().flex().gap(px(8.0));
        for (id, scale, label) in [
            (
                "small",
                CompanionScale::Small,
                t!("settings.companion.characters.scale.small").to_string(),
            ),
            (
                "medium",
                CompanionScale::Medium,
                t!("settings.companion.characters.scale.medium").to_string(),
            ),
            (
                "large",
                CompanionScale::Large,
                t!("settings.companion.characters.scale.large").to_string(),
            ),
        ] {
            scale_buttons = scale_buttons.child(
                Button::new(SharedString::from(format!("companion-scale-{id}")), label)
                    .variant(if self.config.clippy.appearance.scale == scale {
                        ButtonVariant::Secondary
                    } else {
                        ButtonVariant::Outline
                    })
                    .size(ButtonSize::Sm)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.config.clippy.appearance.scale = scale;
                        this.save_config(cx);
                    })),
            );
        }

        let mut roaming_buttons = div().flex().gap(px(8.0));
        for (id, movement, label) in [
            (
                "still",
                DesktopCompanionMovement::Still,
                t!("settings.companion.characters.roaming.still").to_string(),
            ),
            (
                "occasional",
                DesktopCompanionMovement::Occasional,
                t!("settings.companion.characters.roaming.occasional").to_string(),
            ),
            (
                "playful",
                DesktopCompanionMovement::Playful,
                t!("settings.companion.characters.roaming.playful").to_string(),
            ),
        ] {
            roaming_buttons = roaming_buttons.child(
                Button::new(SharedString::from(format!("companion-roaming-{id}")), label)
                    .variant(
                        if self.config.clippy.appearance.desktop.movement == movement {
                            ButtonVariant::Secondary
                        } else {
                            ButtonVariant::Outline
                        },
                    )
                    .size(ButtonSize::Sm)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.config.clippy.appearance.desktop.movement = movement;
                        this.save_config(cx);
                    })),
            );
        }

        let entity = cx.entity();
        let platform_warning = if desktop_companion_platform_limited() {
            div()
                .rounded(px(7.0))
                .border_1()
                .border_color(ShellDeckColors::warning().opacity(0.45))
                .bg(ShellDeckColors::warning().opacity(0.08))
                .px(px(10.0))
                .py(px(8.0))
                .text_size(px(11.0))
                .text_color(ShellDeckColors::text_primary())
                .child(t!("settings.companion.characters.platform_warning").to_string())
        } else {
            div()
        };

        div()
            .flex()
            .flex_col()
            .gap(px(4.0))
            .child(
                div()
                    .py(px(12.0))
                    .flex()
                    .flex_col()
                    .gap(px(8.0))
                    .child(Self::render_about_section(
                        t!("settings.companion.characters.section").as_ref(),
                    ))
                    .child(
                        div()
                            .text_size(px(13.0))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(ShellDeckColors::text_primary())
                            .child(t!("settings.companion.characters.label").to_string()),
                    )
                    .child(
                        div()
                            .text_size(px(11.0))
                            .text_color(ShellDeckColors::text_muted())
                            .child(t!("settings.companion.characters.description").to_string()),
                    )
                    .child(platform_warning)
                    .child(character_cards),
            )
            .child(Self::render_setting_row(
                t!("settings.companion.characters.motion.label").as_ref(),
                t!("settings.companion.characters.motion.description").as_ref(),
                motion_buttons,
            ))
            .child(Self::render_setting_row(
                t!("settings.companion.characters.scale.label").as_ref(),
                t!("settings.companion.characters.scale.description").as_ref(),
                scale_buttons,
            ))
            .child(Self::render_setting_row(
                t!("settings.companion.characters.desktop.label").as_ref(),
                t!("settings.companion.characters.desktop.description").as_ref(),
                Self::bind_toggle(
                    "companion-desktop-character",
                    self.config.clippy.appearance.desktop.enabled,
                    &entity,
                    |this, value| this.config.clippy.appearance.desktop.enabled = value,
                )
                .disabled(self.config.clippy.appearance.character_id().as_str() == "none"),
            ))
            .child(Self::render_setting_row(
                t!("settings.companion.characters.roaming.label").as_ref(),
                t!("settings.companion.characters.roaming.description").as_ref(),
                roaming_buttons,
            ))
            .child(Self::render_setting_row(
                t!("settings.companion.characters.window_climbing.label").as_ref(),
                t!("settings.companion.characters.window_climbing.description").as_ref(),
                Self::bind_toggle(
                    "companion-window-climbing",
                    self.config.clippy.appearance.desktop.allow_window_climbing,
                    &entity,
                    |this, value| {
                        this.config.clippy.appearance.desktop.allow_window_climbing = value
                    },
                ),
            ))
            .child(Self::render_setting_row(
                t!("settings.companion.characters.multi_monitor.label").as_ref(),
                t!("settings.companion.characters.multi_monitor.description").as_ref(),
                Self::bind_toggle(
                    "companion-multi-monitor",
                    self.config.clippy.appearance.desktop.allow_multi_monitor,
                    &entity,
                    |this, value| this.config.clippy.appearance.desktop.allow_multi_monitor = value,
                ),
            ))
            .child(
                div()
                    .py(px(12.0))
                    .flex()
                    .flex_col()
                    .gap(px(8.0))
                    .child(
                        div()
                            .text_size(px(13.0))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(ShellDeckColors::text_primary())
                            .child(t!("settings.appearance.app_theme.title").to_string()),
                    )
                    .child(
                        div()
                            .text_size(px(11.0))
                            .text_color(ShellDeckColors::text_muted())
                            .child(t!("settings.appearance.app_theme.description").to_string()),
                    )
                    .child(app_theme_cards),
            )
            .child(
                div()
                    .py(px(12.0))
                    .flex()
                    .flex_col()
                    .gap(px(8.0))
                    .child(
                        div()
                            .text_size(px(13.0))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(ShellDeckColors::text_primary())
                            .child(t!("settings.appearance.terminal_themes.title").to_string()),
                    )
                    .child(theme_cards),
            )
            .child(Self::render_setting_row(
                t!("settings.appearance.sidebar_width.label").as_ref(),
                t!("settings.appearance.sidebar_width.description").as_ref(),
                Self::render_number_stepper(
                    "sidebar-width",
                    format!("{}px", self.config.general.sidebar_width),
                    cx.listener(|this, _, _, cx| {
                        let new = (this.config.general.sidebar_width - 20.0).max(140.0);
                        if (new - this.config.general.sidebar_width).abs() < f32::EPSILON {
                            return;
                        }
                        this.config.general.sidebar_width = new;
                        this.save_config(cx);
                    }),
                    cx.listener(|this, _, _, cx| {
                        let new = (this.config.general.sidebar_width + 20.0).min(400.0);
                        if (new - this.config.general.sidebar_width).abs() < f32::EPSILON {
                            return;
                        }
                        this.config.general.sidebar_width = new;
                        this.save_config(cx);
                    }),
                ),
            ))
            .child(Self::render_setting_row(
                t!("settings.appearance.ui_font_size.label").as_ref(),
                t!("settings.appearance.ui_font_size.description").as_ref(),
                Self::render_number_stepper(
                    "ui-font-size",
                    format!("{}px", self.config.general.ui_font_size),
                    cx.listener(|this, _, _, cx| {
                        let new = (this.config.general.ui_font_size - 1.0).max(10.0);
                        if (new - this.config.general.ui_font_size).abs() < f32::EPSILON {
                            return;
                        }
                        this.config.general.ui_font_size = new;
                        this.save_config(cx);
                    }),
                    cx.listener(|this, _, _, cx| {
                        let new = (this.config.general.ui_font_size + 1.0).min(22.0);
                        if (new - this.config.general.ui_font_size).abs() < f32::EPSILON {
                            return;
                        }
                        this.config.general.ui_font_size = new;
                        this.save_config(cx);
                    }),
                ),
            ))
            .child(Self::render_setting_row(
                t!("settings.appearance.ui_font.label").as_ref(),
                t!("settings.appearance.ui_font.description").as_ref(),
                div().w(px(200.0)).child(self.ui_font_family_select.clone()),
            ))
    }

    fn render_about_section(title: &str) -> Div {
        div()
            .text_size(px(11.0))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(ShellDeckColors::text_muted())
            .mt(px(20.0))
            .mb(px(6.0))
            .child(title.to_string())
    }

    fn render_about_row(label: &str, value: &str) -> impl IntoElement {
        div()
            .flex()
            .justify_between()
            .w_full()
            .py(px(4.0))
            .child(
                div()
                    .text_size(px(12.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(label.to_string()),
            )
            .child(
                div()
                    .text_size(px(12.0))
                    .text_color(ShellDeckColors::text_primary())
                    .child(value.to_string()),
            )
    }

    fn render_about() -> impl IntoElement {
        let tech_stack = [
            (
                t!("settings.about.tech.ui").to_string(),
                t!("settings.about.tech.ui_value").to_string(),
            ),
            (
                t!("settings.about.tech.components").to_string(),
                t!("settings.about.tech.components_value").to_string(),
            ),
            (
                t!("settings.about.tech.terminal").to_string(),
                t!("settings.about.tech.terminal_value").to_string(),
            ),
            (
                t!("settings.about.tech.ssh").to_string(),
                t!("settings.about.tech.ssh_value").to_string(),
            ),
            (
                t!("settings.about.tech.language").to_string(),
                t!("settings.about.tech.language_value").to_string(),
            ),
        ];

        let shortcuts = [
            (
                t!("settings.about.shortcut.new_terminal").to_string(),
                "Ctrl+T",
            ),
            (
                t!("settings.about.shortcut.close_tab").to_string(),
                "Ctrl+W",
            ),
            (
                t!("settings.about.shortcut.toggle_sidebar").to_string(),
                "Ctrl+B",
            ),
            (
                t!("settings.about.shortcut.command_palette").to_string(),
                "Ctrl+Shift+P",
            ),
            (t!("settings.about.shortcut.settings").to_string(), "Ctrl+,"),
            (t!("settings.about.shortcut.search").to_string(), "Ctrl+F"),
            (
                t!("settings.about.shortcut.zoom").to_string(),
                "Ctrl++ / Ctrl+-",
            ),
            (t!("settings.about.shortcut.quit").to_string(), "Ctrl+Q"),
        ];

        let mut root = div()
            .flex()
            .flex_col()
            .items_center()
            .w_full()
            .py(px(24.0))
            .gap(px(4.0));

        // Header: brand icon + wordmark + tagline
        root = root
            .child(div().mb(px(8.0)).child(crate::brand::brand_badge(56.0)))
            .child(crate::brand::brand_wordmark(28.0))
            .child(
                div()
                    .text_size(px(13.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(t!("settings.about.tagline").to_string()),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .mt(px(4.0))
                    .child(
                        div()
                            .px(px(8.0))
                            .py(px(2.0))
                            .rounded(px(4.0))
                            .bg(ShellDeckColors::primary().opacity(0.15))
                            .text_size(px(12.0))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(ShellDeckColors::primary())
                            .child(format!("v{}", shelldeck_core::VERSION)),
                    )
                    .child(
                        div()
                            .text_size(px(11.0))
                            .text_color(ShellDeckColors::text_muted())
                            .child(t!("settings.about.license").to_string()),
                    ),
            );

        // Content card
        let mut card = div()
            .w(px(420.0))
            .mt(px(16.0))
            .p(px(20.0))
            .rounded(px(8.0))
            .border_1()
            .border_color(ShellDeckColors::border())
            .bg(ShellDeckColors::bg_sidebar())
            .flex()
            .flex_col();

        // Tech stack section
        card = card.child(Self::render_about_section(
            t!("settings.about.tech_stack").as_ref(),
        ));
        for (label, value) in &tech_stack {
            card = card.child(Self::render_about_row(label, value));
        }

        // Keyboard shortcuts section
        card = card.child(Self::render_about_section(
            t!("settings.about.shortcuts").as_ref(),
        ));
        for (label, key) in &shortcuts {
            card = card.child(
                div()
                    .flex()
                    .justify_between()
                    .w_full()
                    .py(px(3.0))
                    .child(
                        div()
                            .text_size(px(12.0))
                            .text_color(ShellDeckColors::text_muted())
                            .child(label.to_string()),
                    )
                    .child(
                        div()
                            .px(px(6.0))
                            .py(px(1.0))
                            .rounded(px(3.0))
                            .bg(ShellDeckColors::hint_bg())
                            .text_size(px(11.0))
                            .text_color(ShellDeckColors::text_primary())
                            .child(key.to_string()),
                    ),
            );
        }

        // Links section
        card = card.child(Self::render_about_section(
            t!("settings.about.links").as_ref(),
        ));
        card = card
            .child(Self::render_about_row(
                t!("settings.about.link.github").as_ref(),
                "github.com/benfavre/shelldeck",
            ))
            .child(Self::render_about_row(
                t!("settings.about.link.website").as_ref(),
                "shelldeck.1clic.pro",
            ));

        root = root.child(card);

        // Footer: "Made by" + Webdesign29 logo — row height locked so text
        // and SVG share the same vertical center (logo viewBox has top padding).
        const LOGO_H: f32 = 22.0;
        root = root.child(
            div()
                .mt(px(16.0))
                .flex()
                .items_center()
                .justify_center()
                .gap(px(8.0))
                .text_color(ShellDeckColors::text_muted())
                .child(
                    div()
                        .flex()
                        .items_center()
                        .h(px(LOGO_H))
                        .text_size(px(11.0))
                        .line_height(px(LOGO_H))
                        .child(t!("settings.about.made_by").to_string()),
                )
                .child(
                    div().flex().items_center().h(px(LOGO_H)).child(
                        svg()
                            .path("images/wd29-logo.svg")
                            .w(px(62.0))
                            .h(px(LOGO_H))
                            .flex_shrink_0()
                            .text_color(ShellDeckColors::text_muted()),
                    ),
                ),
        );

        root
    }
}

impl Render for SettingsView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let close_button = IconButton::new("x")
            .variant(ButtonVariant::Ghost)
            .size(gpui::px(30.0))
            .icon_size(gpui::px(14.0))
            .on_click(cx.listener(|_this, _, _, cx| {
                cx.emit(SettingsEvent::CloseRequested);
            }));
        let mut header = div()
            .flex()
            .items_center()
            .justify_between()
            .px(px(24.0))
            .py(px(16.0))
            .border_b_1()
            .border_color(ShellDeckColors::border())
            .child(
                div()
                    .text_size(px(18.0))
                    .font_weight(FontWeight::BOLD)
                    .text_color(ShellDeckColors::text_primary())
                    .child(t!("settings.title").to_string()),
            );

        if self.unsaved_changes {
            header = header.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(
                        div()
                            .id("save-settings-btn")
                            .px(px(16.0))
                            .py(px(6.0))
                            .rounded(px(4.0))
                            .bg(ShellDeckColors::primary())
                            .text_color(white())
                            .text_size(px(13.0))
                            .cursor_pointer()
                            .hover(|el| el.bg(ShellDeckColors::primary().opacity(0.8)))
                            .child(t!("settings.save").to_string())
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.save_config(cx);
                            })),
                    )
                    .child(close_button),
            );
        } else {
            header = header.child(close_button);
        }

        // Tab content — scrolls vertically inside its own column.
        let mut tab_content = div()
            .id("settings-tab-content")
            .flex()
            .flex_col()
            .p(px(24.0))
            .max_w(px(600.0));

        match self.active_tab {
            SettingsTab::General => {
                tab_content = tab_content.child(self.render_general_settings(cx));
            }
            SettingsTab::Terminal => {
                tab_content = tab_content.child(self.render_terminal_settings(cx));
            }
            SettingsTab::Editor => {
                tab_content = tab_content.child(self.render_editor_settings(cx));
            }
            SettingsTab::Ai => {
                tab_content = tab_content.child(self.render_ai_settings(cx));
            }
            SettingsTab::Appearance => {
                tab_content = tab_content.child(self.render_appearance_settings(cx));
            }
            SettingsTab::About => {
                tab_content = tab_content.child(Self::render_about());
            }
        }

        div()
            .flex()
            .flex_col()
            .size_full()
            .track_focus(&self.shortcut_capture_focus)
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _window, cx| {
                this.handle_shortcut_capture(event, cx);
            }))
            .bg(ShellDeckColors::bg_primary())
            // Header
            .child(header)
            // Content: horizontal row with fixed tab sidebar + scrollable tab content
            .child(
                div()
                    .flex()
                    .flex_grow()
                    .min_h(px(0.0))
                    .id("settings-body")
                    .overflow_hidden()
                    // Tab sidebar
                    .child({
                        let mut tabs = div()
                            .flex()
                            .flex_col()
                            .flex_shrink_0()
                            .gap(px(2.0))
                            .w(px(180.0))
                            .p(px(12.0))
                            .border_r_1()
                            .border_color(ShellDeckColors::border())
                            .child(self.render_tab_button(
                                SettingsTab::General,
                                t!("settings.tab.general").as_ref(),
                                cx,
                            ));
                        if self.dev_tabs_enabled {
                            tabs = tabs
                                .child(self.render_tab_button(
                                    SettingsTab::Terminal,
                                    t!("settings.tab.terminal").as_ref(),
                                    cx,
                                ))
                                .child(self.render_tab_button(
                                    SettingsTab::Editor,
                                    t!("settings.tab.editor").as_ref(),
                                    cx,
                                ));
                        }
                        tabs.child(self.render_tab_button(
                            SettingsTab::Ai,
                            t!("settings.tab.ai").as_ref(),
                            cx,
                        ))
                        .child(self.render_tab_button(
                            SettingsTab::Appearance,
                            t!("settings.tab.appearance").as_ref(),
                            cx,
                        ))
                        .child(self.render_tab_button(
                            SettingsTab::About,
                            t!("settings.tab.about").as_ref(),
                            cx,
                        ))
                    })
                    // Tab content — scrolls independently
                    .child(scrollable_vertical(tab_content)),
            )
    }
}

/// Build a `Select<SharedString>` bound to a `String` field of
/// `AppConfig`, persisting via `save_config` on change. `field_get`
/// reads the current value (used for the equality guard so a no-op
/// pick doesn't rewrite the config file); `field_set` writes the
/// new value.
///
/// Options are passed as `(value, label)` pairs — same convention
/// `SelectOption::new` uses, so callers with i18n'd labels or a
/// sentinel-value shortlist (e.g. `"System Default"`) can keep the
/// value stable while translating the display.
fn build_string_field_select<G, S>(
    entries: Vec<(SharedString, SharedString)>,
    current: &str,
    placeholder: Option<SharedString>,
    searchable: bool,
    cx: &mut Context<SettingsView>,
    field_get: G,
    field_set: S,
) -> Entity<Select<SharedString>>
where
    G: Fn(&SettingsView) -> &str + Send + Sync + 'static,
    S: Fn(&mut SettingsView, String) + Send + Sync + 'static,
{
    let options: Vec<SelectOption<SharedString>> = entries
        .iter()
        .map(|(value, label)| SelectOption::new(value.clone(), label.clone()))
        .collect();
    let selected = entries.iter().position(|(v, _)| v.as_ref() == current);
    let parent = cx.entity();
    cx.new(move |select_cx| {
        let mut sel = Select::new(select_cx)
            .options(options)
            .selected_index(selected);
        if let Some(p) = placeholder {
            sel = sel.placeholder(p);
        }
        if searchable {
            sel = sel.searchable(true);
        }
        sel.on_change(move |value, _window, cx| {
            let picked = value.to_string();
            parent.update(cx, |this, cx| {
                if field_get(this) == picked {
                    return;
                }
                field_set(this, picked);
                this.save_config(cx);
            });
        })
    })
}

fn build_editor_font_family_select(
    config: &AppConfig,
    cx: &mut Context<SettingsView>,
) -> Entity<Select<SharedString>> {
    let entries: Vec<(SharedString, SharedString)> = MONOSPACE_FONTS
        .iter()
        .map(|name| (SharedString::from(*name), SharedString::from(*name)))
        .collect();
    build_string_field_select(
        entries,
        &config.editor.font_family,
        Some(SharedString::from("JetBrains Mono")),
        true,
        cx,
        |this| this.config.editor.font_family.as_str(),
        |this, v| this.config.editor.font_family = v,
    )
}

/// Fresh `Select<usize>` for the editor tab size (2/4/8). Same wiring pattern
/// as the font-family select.
fn build_editor_tab_size_select(
    config: &AppConfig,
    cx: &mut Context<SettingsView>,
) -> Entity<Select<usize>> {
    let options: Vec<SelectOption<usize>> = EDITOR_TAB_SIZES
        .iter()
        .map(|size| SelectOption::new(*size, format!("{}", size)))
        .collect();
    let selected = EDITOR_TAB_SIZES
        .iter()
        .position(|s| *s == config.editor.tab_size);
    let parent = cx.entity();
    cx.new(move |select_cx| {
        Select::new(select_cx)
            .options(options)
            .selected_index(selected)
            .on_change(move |value, _window, cx| {
                let picked = *value;
                parent.update(cx, |this, cx| {
                    if this.config.editor.tab_size == picked {
                        return;
                    }
                    this.config.editor.tab_size = picked;
                    this.save_config(cx);
                });
            })
    })
}

/// Fresh `Select<SharedString>` for the terminal font family. Same shortlist
/// as the editor (both need monospace metrics).
fn build_terminal_font_family_select(
    config: &AppConfig,
    cx: &mut Context<SettingsView>,
) -> Entity<Select<SharedString>> {
    let entries: Vec<(SharedString, SharedString)> = MONOSPACE_FONTS
        .iter()
        .map(|name| (SharedString::from(*name), SharedString::from(*name)))
        .collect();
    build_string_field_select(
        entries,
        &config.terminal.font_family,
        Some(SharedString::from("JetBrains Mono")),
        true,
        cx,
        |this| this.config.terminal.font_family.as_str(),
        |this, v| this.config.terminal.font_family = v,
    )
}

/// Fresh `Select<SharedString>` for the terminal cursor style (block /
/// underline / bar). Snake_case values match the runtime `set_cursor_style`
/// API, so the picker persists exactly what the terminal expects.
fn build_terminal_cursor_style_select(
    config: &AppConfig,
    cx: &mut Context<SettingsView>,
) -> Entity<Select<SharedString>> {
    let entries: Vec<(SharedString, SharedString)> = vec![
        (
            "block".into(),
            t!("settings.terminal.cursor_style.block")
                .to_string()
                .into(),
        ),
        (
            "underline".into(),
            t!("settings.terminal.cursor_style.underline")
                .to_string()
                .into(),
        ),
        (
            "bar".into(),
            t!("settings.terminal.cursor_style.bar").to_string().into(),
        ),
    ];
    build_string_field_select(
        entries,
        &config.terminal.cursor_style,
        None,
        false,
        cx,
        |this| this.config.terminal.cursor_style.as_str(),
        |this, v| this.config.terminal.cursor_style = v,
    )
}

/// Fresh `Select<UiLanguage>` for the interface language (System / Français /
/// English). Persists via `select_ui_language` so the workspace re-applies
/// `rust_i18n::set_locale` and every view repaints.
fn build_general_language_select(
    config: &AppConfig,
    cx: &mut Context<SettingsView>,
) -> Entity<Select<UiLanguage>> {
    let entries: &[(UiLanguage, &str)] = &[
        (UiLanguage::System, "settings.language.system"),
        (UiLanguage::Fr, "settings.language.fr"),
        (UiLanguage::En, "settings.language.en"),
    ];
    let options: Vec<SelectOption<UiLanguage>> = entries
        .iter()
        .map(|(lang, key)| SelectOption::new(lang.clone(), t!(*key).to_string()))
        .collect();
    let selected = entries
        .iter()
        .position(|(lang, _)| *lang == config.general.ui_language);
    let parent = cx.entity();
    cx.new(move |select_cx| {
        Select::new(select_cx)
            .options(options)
            .selected_index(selected)
            .on_change(move |value, _window, cx| {
                let picked = value.clone();
                parent.update(cx, |this, cx| {
                    this.select_ui_language(picked, cx);
                });
            })
    })
}

/// Fresh `Select<SharedString>` for the app UI font. Mirrors the terminal
/// shortlist with a “System Default” option on top — that value falls back
/// to the platform's default sans-serif family.
fn build_ui_font_family_select(
    config: &AppConfig,
    cx: &mut Context<SettingsView>,
) -> Entity<Select<SharedString>> {
    // "System Default" is a stable sentinel value persisted in config
    // (see `AppConfig::default().general.ui_font_family`); only the display
    // label is translated.
    let system_default_label: SharedString = t!("settings.general.font.system_default")
        .to_string()
        .into();
    let fonts: &[&str] = &[
        "System Default",
        "Inter",
        "SF Pro Text",
        "Segoe UI",
        "Ubuntu",
        "Roboto",
        "JetBrains Mono",
        "Fira Code",
    ];
    let entries: Vec<(SharedString, SharedString)> = fonts
        .iter()
        .map(|name| {
            let label: SharedString = if *name == "System Default" {
                system_default_label.clone()
            } else {
                SharedString::from(*name)
            };
            (SharedString::from(*name), label)
        })
        .collect();
    build_string_field_select(
        entries,
        &config.general.ui_font_family,
        Some(system_default_label),
        true,
        cx,
        |this| this.config.general.ui_font_family.as_str(),
        |this, v| this.config.general.ui_font_family = v,
    )
}

fn build_ai_backend_select(
    config: &AppConfig,
    cx: &mut Context<SettingsView>,
) -> Entity<Select<AiBackend>> {
    let entries = [
        (
            AiBackend::Disabled,
            t!("settings.ai.backend.disabled").to_string(),
        ),
        (AiBackend::ClaudeCli, "Claude Code CLI".to_string()),
        (AiBackend::CodexCli, "Codex CLI".to_string()),
        (AiBackend::AiderCli, "Aider CLI".to_string()),
        (AiBackend::OpenAi, "OpenAI API".to_string()),
        (AiBackend::Anthropic, "Anthropic API".to_string()),
    ];
    let options = entries
        .iter()
        .map(|(backend, label)| {
            let icon = match backend {
                AiBackend::Disabled => IconSource::Named("x".into()),
                AiBackend::ClaudeCli => IconSource::from("icons/simple/claudecode.svg"),
                AiBackend::CodexCli | AiBackend::OpenAi => {
                    IconSource::from("icons/simple/openai.svg")
                }
                AiBackend::AiderCli => IconSource::Named("terminal".into()),
                AiBackend::Anthropic => IconSource::from("icons/simple/anthropic.svg"),
            };
            SelectOption::new(*backend, label.clone()).with_icon(icon)
        })
        .collect();
    let selected = entries
        .iter()
        .position(|(backend, _)| *backend == config.ai.backend);
    let parent = cx.entity();
    cx.new(move |select_cx| {
        Select::new(select_cx)
            .options(options)
            .selected_index(selected)
            .on_change(move |backend, _window, cx| {
                let backend = *backend;
                parent.update(cx, |this, cx| {
                    if this.config.ai.backend == backend {
                        return;
                    }
                    this.config.ai.backend = backend;
                    this.config.ai.enabled = backend != AiBackend::Disabled;
                    this.config.ai.model.clear();
                    this.ai_connection_state = AiConnectionState::NotTested;
                    this.ai_model_state.update(cx, |state, cx| {
                        state.reset(cx);
                    });
                    this.save_config(cx);
                });
            })
    })
}

fn local_backend_status(command: &str, available: bool) -> String {
    if available {
        t!("settings.ai.status.available", command = command).to_string()
    } else {
        t!("settings.ai.status.missing", command = command).to_string()
    }
}

fn ai_surface_row(
    id: &'static str,
    name: &'static str,
    checked: bool,
    entity: &Entity<SettingsView>,
    set: impl Fn(&mut SettingsView, bool) + 'static,
) -> impl IntoElement {
    let label = match name {
        "support" => t!("settings.ai.surfaces.support").to_string(),
        "issues" => t!("settings.ai.surfaces.issues").to_string(),
        "scripts" => t!("settings.ai.surfaces.scripts").to_string(),
        "terminal" => t!("settings.ai.surfaces.terminal").to_string(),
        "jean" => t!("settings.ai.surfaces.jean").to_string(),
        "naming" => t!("settings.ai.surfaces.naming").to_string(),
        "recent" => t!("settings.ai.surfaces.recent").to_string(),
        "clippy" => t!("settings.ai.surfaces.clippy").to_string(),
        _ => name.to_string(),
    };
    SettingsView::render_setting_row(
        &label,
        t!("settings.ai.surfaces.description").as_ref(),
        SettingsView::bind_toggle(id, checked, entity, set),
    )
}

fn ai_policy_row(
    id: &'static str,
    name: &'static str,
    current: AiAutonomyLevel,
    entity: &Entity<SettingsView>,
    set: fn(&mut SettingsView, AiAutonomyLevel),
) -> impl IntoElement {
    let label = match name {
        "support_send" => t!("settings.ai.policies.support_send").to_string(),
        "support_triage" => t!("settings.ai.policies.support_triage").to_string(),
        "terminal_execute" => t!("settings.ai.policies.terminal_execute").to_string(),
        "script_execute" => t!("settings.ai.policies.script_execute").to_string(),
        "jean_dispatch" => t!("settings.ai.policies.jean_dispatch").to_string(),
        "fleet_dispatch" => t!("settings.ai.policies.fleet_dispatch").to_string(),
        _ => name.to_string(),
    };
    let mut controls = div().flex().items_center().gap(px(6.0));
    for (index, (level, key)) in [
        (
            AiAutonomyLevel::Preparation,
            "settings.ai.policies.preparation",
        ),
        (
            AiAutonomyLevel::Confirmation,
            "settings.ai.policies.confirmation",
        ),
        (AiAutonomyLevel::Automatic, "settings.ai.policies.automatic"),
    ]
    .into_iter()
    .enumerate()
    {
        let entity = entity.clone();
        controls = controls.child(
            Button::new((id, index), t!(key).to_string())
                .variant(if current == level {
                    ButtonVariant::Secondary
                } else {
                    ButtonVariant::Ghost
                })
                .size(ButtonSize::Sm)
                .on_click(move |_, _, cx| {
                    entity.update(cx, |this, cx| {
                        set(this, level);
                        this.save_config(cx);
                    });
                }),
        );
    }
    SettingsView::render_setting_row(
        &label,
        t!("settings.ai.policies.description").as_ref(),
        controls,
    )
}

#[cfg(test)]
mod tests {
    use super::{
        apply_character_choice, compositor_companion_limited, display_shortcut,
        shortcut_error_is_portal_missing, validate_shortcut_capture, ClippyAppearanceConfig,
        ShortcutCaptureValidation,
    };
    use gpui::Keystroke;

    // SDTEST-1419 — the portal-missing classifier decides whether a user reads
    // an explanation or a D-Bus sentence. It has to catch both shapes ashpd
    // produces (resolved name, raw `ServiceUnknown`) without swallowing the
    // grab errors that name a specific key or a specific conflict.
    #[test]
    fn portal_missing_matches_ashpd_shapes_only() {
        assert!(shortcut_error_is_portal_missing(
            "A portal frontend implementing `org.freedesktop.portal.GlobalShortcuts` was not found"
        ));
        assert!(shortcut_error_is_portal_missing(
            "org.freedesktop.DBus.Error.ServiceUnknown: \
             org.freedesktop.portal.GlobalShortcuts is not provided"
        ));

        for unrelated in [
            "Could not resolve keycode for key: nosuchkey",
            "BadAccess: another client already grabbed this combination",
            "Wayland Global Shortcuts portal did not accept this shortcut",
        ] {
            assert!(
                !shortcut_error_is_portal_missing(unrelated),
                "{unrelated} must reach the user verbatim"
            );
        }
    }

    // SDTEST-1401
    #[test]
    fn shortcut_capture_requires_modifier_and_rejects_duplicate() {
        let bare = Keystroke::parse("space").unwrap();
        assert_eq!(
            validate_shortcut_capture(&bare, "ctrl-alt-space"),
            ShortcutCaptureValidation::ModifierRequired
        );

        let duplicate = Keystroke::parse("ctrl-alt-space").unwrap();
        assert_eq!(
            validate_shortcut_capture(&duplicate, "ctrl-alt-space"),
            ShortcutCaptureValidation::Conflict
        );

        let custom = Keystroke::parse("ctrl-shift-k").unwrap();
        assert_eq!(
            validate_shortcut_capture(&custom, "ctrl-alt-space"),
            ShortcutCaptureValidation::Accepted("ctrl-shift-k".to_string())
        );
        assert_eq!(display_shortcut("ctrl-shift-k"), "Ctrl+Shift+K");
    }

    // SDTEST-1489
    #[test]
    fn choosing_a_visible_character_enables_it_and_none_disables_it() {
        let mut appearance = ClippyAppearanceConfig::default();

        apply_character_choice(&mut appearance, "nox");
        assert_eq!(appearance.character, "nox");
        assert!(appearance.desktop.enabled);

        apply_character_choice(&mut appearance, "none");
        assert_eq!(appearance.character, "none");
        assert!(!appearance.desktop.enabled);
    }

    // SDTEST-1570
    #[test]
    fn native_wayland_companion_limitation_is_reported_without_misclassifying_x11() {
        assert!(compositor_companion_limited("Wayland"));
        assert!(compositor_companion_limited("wayland"));
        assert!(!compositor_companion_limited("X11"));
        assert!(!compositor_companion_limited("unknown"));
    }
}
