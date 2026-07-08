use crate::scale::px;
use adabraka_ui::prelude::scrollable_vertical;
use gpui::prelude::*;
use gpui::*;

use crate::t;
use shelldeck_core::config::app_config::{AppConfig, ThemePreference, UiLanguage};
use shelldeck_core::config::themes::TerminalTheme;

use crate::theme::{palette_for, ShellDeckColors};
use crate::workspace::CloudSyncNow;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsTab {
    General,
    Terminal,
    Appearance,
    About,
}

/// Events emitted when settings change.
#[derive(Debug, Clone)]
#[allow(clippy::large_enum_variant)]
pub enum SettingsEvent {
    ConfigChanged(AppConfig),
    ThemeChanged(ThemePreference),
}

impl EventEmitter<SettingsEvent> for SettingsView {}

pub struct SettingsView {
    pub config: AppConfig,
    pub active_tab: SettingsTab,
    pub unsaved_changes: bool,
}

impl SettingsView {
    pub fn new(config: AppConfig) -> Self {
        Self {
            config,
            active_tab: SettingsTab::General,
            unsaved_changes: false,
        }
    }

    fn mark_changed(&mut self) {
        self.unsaved_changes = true;
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
                    .child(control),
            )
    }

    fn render_toggle(enabled: bool) -> impl IntoElement {
        let mut el = div()
            .w(px(40.0))
            .h(px(22.0))
            .rounded_full()
            .cursor_pointer()
            .flex()
            .items_center();

        if enabled {
            el = el.bg(ShellDeckColors::primary()).child(
                div()
                    .ml_auto()
                    .mr(px(2.0))
                    .w(px(18.0))
                    .h(px(18.0))
                    .rounded_full()
                    .bg(white()),
            );
        } else {
            el = el.bg(ShellDeckColors::toggle_off_bg()).child(
                div()
                    .ml(px(2.0))
                    .w(px(18.0))
                    .h(px(18.0))
                    .rounded_full()
                    .bg(ShellDeckColors::toggle_off_knob()),
            );
        }

        el
    }

    fn render_general_settings(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let current_lang = self.config.general.ui_language.clone();
        let mut lang_row = div().flex().gap(px(6.0)).flex_wrap();
        for lang in UiLanguage::all() {
            let lang = lang.clone();
            let is_active = current_lang == lang;
            let label = match lang {
                UiLanguage::System => t!("settings.language.system").to_string(),
                UiLanguage::Fr => t!("settings.language.fr").to_string(),
                UiLanguage::En => t!("settings.language.en").to_string(),
            };
            let mut btn = div()
                .id(ElementId::from(SharedString::from(format!(
                    "ui-lang-{lang:?}"
                ))))
                .px(px(10.0))
                .py(px(6.0))
                .rounded(px(6.0))
                .cursor_pointer()
                .text_size(px(12.0))
                .border_1()
                .border_color(ShellDeckColors::border());
            if is_active {
                btn = btn
                    .bg(ShellDeckColors::primary().opacity(0.15))
                    .text_color(ShellDeckColors::primary())
                    .font_weight(FontWeight::MEDIUM);
            } else {
                btn = btn
                    .text_color(ShellDeckColors::text_muted())
                    .hover(|el| el.bg(ShellDeckColors::hover_bg()));
            }
            lang_row = lang_row.child(btn.child(label).on_click(cx.listener(
                move |this, _, _, cx| {
                    this.select_ui_language(lang.clone(), cx);
                },
            )));
        }

        div()
            .flex()
            .flex_col()
            .gap(px(4.0))
            .child(Self::render_setting_row(
                t!("settings.language.label").as_ref(),
                t!("settings.language.description").as_ref(),
                lang_row,
            ))
            .child(Self::render_setting_row(
                t!("settings.general.auto_connect.label").as_ref(),
                t!("settings.general.auto_connect.description").as_ref(),
                div()
                    .id("toggle-auto-connect")
                    .child(Self::render_toggle(
                        self.config.general.auto_connect_on_startup,
                    ))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.config.general.auto_connect_on_startup =
                            !this.config.general.auto_connect_on_startup;
                        this.mark_changed();
                        cx.notify();
                    })),
            ))
            .child(Self::render_setting_row(
                t!("settings.general.notifications.label").as_ref(),
                t!("settings.general.notifications.description").as_ref(),
                div()
                    .id("toggle-notifications")
                    .child(Self::render_toggle(self.config.general.show_notifications))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.config.general.show_notifications =
                            !this.config.general.show_notifications;
                        this.mark_changed();
                        cx.notify();
                    })),
            ))
            .child(Self::render_setting_row(
                t!("settings.general.confirm_close.label").as_ref(),
                t!("settings.general.confirm_close.description").as_ref(),
                div()
                    .id("toggle-confirm-close")
                    .child(Self::render_toggle(
                        self.config.general.confirm_before_close,
                    ))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.config.general.confirm_before_close =
                            !this.config.general.confirm_before_close;
                        this.mark_changed();
                        cx.notify();
                    })),
            ))
            .child(Self::render_setting_row(
                t!("settings.general.tmux.label").as_ref(),
                t!("settings.general.tmux.description").as_ref(),
                div()
                    .id("toggle-tmux")
                    .child(Self::render_toggle(self.config.general.auto_attach_tmux))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.config.general.auto_attach_tmux =
                            !this.config.general.auto_attach_tmux;
                        this.mark_changed();
                        cx.notify();
                    })),
            ))
            .child(Self::render_setting_row(
                t!("settings.general.auto_update.label").as_ref(),
                t!("settings.general.auto_update.description").as_ref(),
                div()
                    .id("toggle-auto-update")
                    .child(Self::render_toggle(self.config.general.auto_update))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.config.general.auto_update = !this.config.general.auto_update;
                        this.mark_changed();
                        cx.notify();
                    })),
            ))
            .child(self.render_cloud_sync_settings(cx))
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
        div()
            .flex()
            .flex_col()
            .gap(px(4.0))
            .child(Self::render_setting_row(
                t!("settings.terminal.font_size.label").as_ref(),
                t!("settings.terminal.font_size.description").as_ref(),
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(
                        div()
                            .id("font-size-down")
                            .text_size(px(16.0))
                            .text_color(ShellDeckColors::text_muted())
                            .cursor_pointer()
                            .hover(|el| el.text_color(ShellDeckColors::text_primary()))
                            .child(svg().path("icons/lucide/minus.svg").size(px(12.0)))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.config.terminal.font_size =
                                    (this.config.terminal.font_size - 1.0).max(8.0);
                                this.mark_changed();
                                cx.notify();
                            })),
                    )
                    .child(
                        div()
                            .text_size(px(13.0))
                            .text_color(ShellDeckColors::text_primary())
                            .child(format!("{}px", self.config.terminal.font_size)),
                    )
                    .child(
                        div()
                            .id("font-size-up")
                            .text_size(px(16.0))
                            .text_color(ShellDeckColors::text_muted())
                            .cursor_pointer()
                            .hover(|el| el.text_color(ShellDeckColors::text_primary()))
                            .child(svg().path("icons/lucide/plus.svg").size(px(12.0)))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.config.terminal.font_size =
                                    (this.config.terminal.font_size + 1.0).min(32.0);
                                this.mark_changed();
                                cx.notify();
                            })),
                    ),
            ))
            .child(Self::render_setting_row(
                t!("settings.terminal.font_family.label").as_ref(),
                t!("settings.terminal.font_family.description").as_ref(),
                {
                    let fonts = [
                        "JetBrains Mono",
                        "Fira Code",
                        "Source Code Pro",
                        "Cascadia Code",
                        "Menlo",
                        "Consolas",
                    ];
                    let current = self.config.terminal.font_family.clone();
                    let mut row = div()
                        .flex()
                        .items_center()
                        .justify_end()
                        .gap(px(4.0))
                        .flex_wrap();
                    for font_name in &fonts {
                        let f = font_name.to_string();
                        let is_active = current == f;
                        let mut btn = div()
                            .id(ElementId::from(SharedString::from(format!("font-{}", f))))
                            .px(px(8.0))
                            .py(px(4.0))
                            .rounded(px(4.0))
                            .text_size(px(11.0))
                            .cursor_pointer();

                        if is_active {
                            btn = btn
                                .bg(ShellDeckColors::primary().opacity(0.2))
                                .text_color(ShellDeckColors::primary());
                        } else {
                            btn = btn
                                .text_color(ShellDeckColors::text_muted())
                                .hover(|el| el.bg(ShellDeckColors::hover_bg()));
                        }

                        let f_clone = f.clone();
                        btn = btn.child(f).on_click(cx.listener(move |this, _, _, cx| {
                            this.config.terminal.font_family = f_clone.clone();
                            this.mark_changed();
                            cx.notify();
                        }));

                        row = row.child(btn);
                    }
                    row
                },
            ))
            .child(Self::render_setting_row(
                t!("settings.terminal.scrollback.label").as_ref(),
                t!("settings.terminal.scrollback.description").as_ref(),
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(
                        div()
                            .id("scrollback-down")
                            .text_size(px(16.0))
                            .text_color(ShellDeckColors::text_muted())
                            .cursor_pointer()
                            .hover(|el| el.text_color(ShellDeckColors::text_primary()))
                            .child(svg().path("icons/lucide/minus.svg").size(px(12.0)))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.config.terminal.scrollback_lines = this
                                    .config
                                    .terminal
                                    .scrollback_lines
                                    .saturating_sub(1000)
                                    .max(1000);
                                this.mark_changed();
                                cx.notify();
                            })),
                    )
                    .child(
                        div()
                            .text_size(px(13.0))
                            .text_color(ShellDeckColors::text_primary())
                            .child(format!("{}", self.config.terminal.scrollback_lines)),
                    )
                    .child(
                        div()
                            .id("scrollback-up")
                            .text_size(px(16.0))
                            .text_color(ShellDeckColors::text_muted())
                            .cursor_pointer()
                            .hover(|el| el.text_color(ShellDeckColors::text_primary()))
                            .child(svg().path("icons/lucide/plus.svg").size(px(12.0)))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.config.terminal.scrollback_lines =
                                    (this.config.terminal.scrollback_lines + 1000).min(100_000);
                                this.mark_changed();
                                cx.notify();
                            })),
                    ),
            ))
            .child(Self::render_setting_row(
                t!("settings.terminal.cursor_style.label").as_ref(),
                t!("settings.terminal.cursor_style.description").as_ref(),
                {
                    let styles = ["block", "underline", "bar"];
                    let current = self.config.terminal.cursor_style.clone();
                    let mut row = div()
                        .flex()
                        .items_center()
                        .justify_end()
                        .gap(px(4.0))
                        .flex_wrap();
                    for style in &styles {
                        let s = style.to_string();
                        let is_active = current == s;
                        let mut btn = div()
                            .id(ElementId::from(SharedString::from(format!("cursor-{}", s))))
                            .px(px(8.0))
                            .py(px(4.0))
                            .rounded(px(4.0))
                            .text_size(px(12.0))
                            .cursor_pointer();

                        if is_active {
                            btn = btn
                                .bg(ShellDeckColors::primary().opacity(0.2))
                                .text_color(ShellDeckColors::primary());
                        } else {
                            btn = btn
                                .text_color(ShellDeckColors::text_muted())
                                .hover(|el| el.bg(ShellDeckColors::hover_bg()));
                        }

                        let s_clone = s.clone();
                        btn = btn.child(s).on_click(cx.listener(move |this, _, _, cx| {
                            this.config.terminal.cursor_style = s_clone.clone();
                            this.mark_changed();
                            cx.notify();
                        }));

                        row = row.child(btn);
                    }
                    row
                },
            ))
            .child(Self::render_setting_row(
                t!("settings.terminal.cursor_blink.label").as_ref(),
                t!("settings.terminal.cursor_blink.description").as_ref(),
                div()
                    .id("toggle-cursor-blink")
                    .child(Self::render_toggle(self.config.terminal.cursor_blink))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.config.terminal.cursor_blink = !this.config.terminal.cursor_blink;
                        this.mark_changed();
                        cx.notify();
                    })),
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
                .child(div().text_size(px(10.0)).text_color(fg).child("Aa Bb 123"));
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
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(
                        div()
                            .id("sidebar-width-down")
                            .text_size(px(16.0))
                            .text_color(ShellDeckColors::text_muted())
                            .cursor_pointer()
                            .hover(|el| el.text_color(ShellDeckColors::text_primary()))
                            .child(svg().path("icons/lucide/minus.svg").size(px(12.0)))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.config.general.sidebar_width =
                                    (this.config.general.sidebar_width - 20.0).max(140.0);
                                this.mark_changed();
                                cx.notify();
                            })),
                    )
                    .child(
                        div()
                            .text_size(px(13.0))
                            .text_color(ShellDeckColors::text_primary())
                            .child(format!("{}px", self.config.general.sidebar_width)),
                    )
                    .child(
                        div()
                            .id("sidebar-width-up")
                            .text_size(px(16.0))
                            .text_color(ShellDeckColors::text_muted())
                            .cursor_pointer()
                            .hover(|el| el.text_color(ShellDeckColors::text_primary()))
                            .child(svg().path("icons/lucide/plus.svg").size(px(12.0)))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.config.general.sidebar_width =
                                    (this.config.general.sidebar_width + 20.0).min(400.0);
                                this.mark_changed();
                                cx.notify();
                            })),
                    ),
            ))
            .child(Self::render_setting_row(
                t!("settings.appearance.ui_font_size.label").as_ref(),
                t!("settings.appearance.ui_font_size.description").as_ref(),
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(
                        div()
                            .id("ui-font-size-down")
                            .text_size(px(16.0))
                            .text_color(ShellDeckColors::text_muted())
                            .cursor_pointer()
                            .hover(|el| el.text_color(ShellDeckColors::text_primary()))
                            .child(svg().path("icons/lucide/minus.svg").size(px(12.0)))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.config.general.ui_font_size =
                                    (this.config.general.ui_font_size - 1.0).max(10.0);
                                this.mark_changed();
                                cx.notify();
                            })),
                    )
                    .child(
                        div()
                            .text_size(px(13.0))
                            .text_color(ShellDeckColors::text_primary())
                            .child(format!("{}px", self.config.general.ui_font_size)),
                    )
                    .child(
                        div()
                            .id("ui-font-size-up")
                            .text_size(px(16.0))
                            .text_color(ShellDeckColors::text_muted())
                            .cursor_pointer()
                            .hover(|el| el.text_color(ShellDeckColors::text_primary()))
                            .child(svg().path("icons/lucide/plus.svg").size(px(12.0)))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.config.general.ui_font_size =
                                    (this.config.general.ui_font_size + 1.0).min(22.0);
                                this.mark_changed();
                                cx.notify();
                            })),
                    ),
            ))
            .child(Self::render_setting_row(
                t!("settings.appearance.ui_font.label").as_ref(),
                t!("settings.appearance.ui_font.description").as_ref(),
                {
                    let fonts = [
                        "System Default",
                        "Inter",
                        "SF Pro Text",
                        "Segoe UI",
                        "Roboto",
                        "Ubuntu",
                        "JetBrains Mono",
                    ];
                    let current = self.config.general.ui_font_family.clone();
                    let mut row = div()
                        .flex()
                        .items_center()
                        .justify_end()
                        .gap(px(4.0))
                        .flex_wrap();
                    for font_name in &fonts {
                        let f = font_name.to_string();
                        let is_active = current == f;
                        let mut btn = div()
                            .id(ElementId::from(SharedString::from(format!(
                                "ui-font-{}",
                                f
                            ))))
                            .px(px(8.0))
                            .py(px(4.0))
                            .rounded(px(4.0))
                            .text_size(px(11.0))
                            .cursor_pointer();

                        if is_active {
                            btn = btn
                                .bg(ShellDeckColors::primary().opacity(0.2))
                                .text_color(ShellDeckColors::primary());
                        } else {
                            btn = btn
                                .text_color(ShellDeckColors::text_muted())
                                .hover(|el| el.bg(ShellDeckColors::hover_bg()));
                        }

                        let f_clone = f.clone();
                        btn = btn.child(f).on_click(cx.listener(move |this, _, _, cx| {
                            this.config.general.ui_font_family = f_clone.clone();
                            this.mark_changed();
                            cx.notify();
                        }));

                        row = row.child(btn);
                    }
                    row
                },
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
                            .child("MIT License"),
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
