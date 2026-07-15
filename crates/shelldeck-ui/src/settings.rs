use crate::scale::px;
use adabraka_ui::components::icon_button::IconButton;
use adabraka_ui::components::icon_source::IconSource;
use adabraka_ui::components::select::{Select, SelectOption};
use adabraka_ui::components::toggle::Toggle;
use adabraka_ui::prelude::scrollable_vertical;
use gpui::prelude::*;
use gpui::*;

use crate::t;
use shelldeck_core::config::app_config::{AppConfig, ThemePreference, UiLanguage};
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsTab {
    General,
    Terminal,
    Editor,
    Appearance,
    About,
}

/// Events emitted when settings change.
#[derive(Debug, Clone)]
#[allow(clippy::large_enum_variant)]
pub enum SettingsEvent {
    ConfigChanged(AppConfig),
    ThemeChanged(ThemePreference),
    /// User flipped the "Launch at login" toggle to `desired`. Workspace
    /// applies the OS-level change on a background thread, then either
    /// commits the config field (on success) or toasts + leaves the
    /// toggle unchanged (on OS failure — Flatpak sandbox, permissions,
    /// missing HOME, …). See `Workspace::apply_autostart_request`.
    AutostartRequested(bool),
}

impl EventEmitter<SettingsEvent> for SettingsView {}

pub struct SettingsView {
    pub config: AppConfig,
    pub active_tab: SettingsTab,
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
}

impl SettingsView {
    pub fn new(config: AppConfig, cx: &mut Context<Self>) -> Self {
        let editor_font_family_select = build_editor_font_family_select(&config, cx);
        let editor_tab_size_select = build_editor_tab_size_select(&config, cx);
        let terminal_font_family_select = build_terminal_font_family_select(&config, cx);
        let terminal_cursor_style_select = build_terminal_cursor_style_select(&config, cx);
        let general_language_select = build_general_language_select(&config, cx);
        let ui_font_family_select = build_ui_font_family_select(&config, cx);
        Self {
            config,
            active_tab: SettingsTab::General,
            unsaved_changes: false,
            editor_font_family_select,
            editor_tab_size_select,
            terminal_font_family_select,
            terminal_cursor_style_select,
            general_language_select,
            ui_font_family_select,
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

    /// Update the persisted "sidebar top-nav collapsed" state. Called by the
    /// workspace when the user clicks the sidebar's collapse chevron.
    pub fn set_sidebar_nav_collapsed(&mut self, collapsed: bool, cx: &mut Context<Self>) {
        if self.config.general.sidebar_nav_collapsed == collapsed {
            return;
        }
        self.config.general.sidebar_nav_collapsed = collapsed;
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
            .w_full()
            .py(px(12.0))
            .border_b_1()
            .border_color(ShellDeckColors::border())
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(2.0))
                    .flex_shrink_0()
                    .child(
                        div()
                            .text_size(px(13.0))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(ShellDeckColors::text_primary())
                            .child(label.to_string()),
                    )
                    .child(
                        div()
                            .text_size(px(12.0))
                            .text_color(ShellDeckColors::text_muted())
                            .child(description.to_string()),
                    ),
            )
            .child(
                div()
                    .flex_grow()
                    .min_w(px(0.0))
                    .overflow_hidden()
                    .flex()
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
    ) -> impl IntoElement {
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
            );
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
                    .child(
                        div()
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
                            ))
                            .child(self.render_tab_button(
                                SettingsTab::Terminal,
                                t!("settings.tab.terminal").as_ref(),
                                cx,
                            ))
                            .child(self.render_tab_button(
                                SettingsTab::Editor,
                                t!("settings.tab.editor").as_ref(),
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
                            )),
                    )
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
