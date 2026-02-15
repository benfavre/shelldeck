use gpui::prelude::*;
use gpui::*;

use shelldeck_core::config::app_config::{AppConfig, ThemePreference};
use shelldeck_core::config::themes::TerminalTheme;

use crate::theme::ShellDeckColors;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsTab {
    General,
    Terminal,
    Appearance,
    About,
}

/// Events emitted when settings change.
#[derive(Debug, Clone)]
pub enum SettingsEvent {
    ConfigChanged(AppConfig),
    ThemeChanged(ThemePreference),
    TerminalThemeChanged(TerminalTheme),
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

    fn save_config(&mut self, cx: &mut Context<Self>) {
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
            .id(ElementId::from(SharedString::from(format!("settings-tab-{tab:?}"))))
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
            .child(control)
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
            el = el
                .bg(ShellDeckColors::primary())
                .child(
                    div()
                        .ml_auto()
                        .mr(px(2.0))
                        .w(px(18.0))
                        .h(px(18.0))
                        .rounded_full()
                        .bg(white()),
                );
        } else {
            el = el
                .bg(ShellDeckColors::toggle_off_bg())
                .child(
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
        div()
            .flex()
            .flex_col()
            .gap(px(4.0))
            .child(Self::render_setting_row(
                "Auto-connect on startup",
                "Reconnect to previously active sessions when app starts",
                div()
                    .id("toggle-auto-connect")
                    .child(Self::render_toggle(self.config.general.auto_connect_on_startup))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.config.general.auto_connect_on_startup = !this.config.general.auto_connect_on_startup;
                        this.mark_changed();
                        cx.notify();
                    })),
            ))
            .child(Self::render_setting_row(
                "Show notifications",
                "Display toast notifications for connection events",
                div()
                    .id("toggle-notifications")
                    .child(Self::render_toggle(self.config.general.show_notifications))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.config.general.show_notifications = !this.config.general.show_notifications;
                        this.mark_changed();
                        cx.notify();
                    })),
            ))
            .child(Self::render_setting_row(
                "Confirm before close",
                "Ask for confirmation when closing with active sessions",
                div()
                    .id("toggle-confirm-close")
                    .child(Self::render_toggle(self.config.general.confirm_before_close))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.config.general.confirm_before_close = !this.config.general.confirm_before_close;
                        this.mark_changed();
                        cx.notify();
                    })),
            ))
            .child(Self::render_setting_row(
                "Auto-attach tmux",
                "Automatically attach to tmux sessions on remote hosts",
                div()
                    .id("toggle-tmux")
                    .child(Self::render_toggle(self.config.general.auto_attach_tmux))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.config.general.auto_attach_tmux = !this.config.general.auto_attach_tmux;
                        this.mark_changed();
                        cx.notify();
                    })),
            ))
    }

    fn render_terminal_settings(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .gap(px(4.0))
            .child(Self::render_setting_row(
                "Font Size",
                "Terminal font size in pixels",
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
                            .child("-")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.config.terminal.font_size = (this.config.terminal.font_size - 1.0).max(8.0);
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
                            .child("+")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.config.terminal.font_size = (this.config.terminal.font_size + 1.0).min(32.0);
                                this.mark_changed();
                                cx.notify();
                            })),
                    ),
            ))
            .child(Self::render_setting_row(
                "Font Family",
                "Monospace font for terminal rendering",
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
                    let mut row = div().flex().items_center().gap(px(4.0));
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
                        btn = btn
                            .child(f)
                            .on_click(cx.listener(move |this, _, _, cx| {
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
                "Scrollback Lines",
                "Maximum number of lines kept in scrollback buffer",
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
                            .child("-")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.config.terminal.scrollback_lines = this.config.terminal.scrollback_lines.saturating_sub(1000).max(1000);
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
                            .child("+")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.config.terminal.scrollback_lines = (this.config.terminal.scrollback_lines + 1000).min(100_000);
                                this.mark_changed();
                                cx.notify();
                            })),
                    ),
            ))
            .child(Self::render_setting_row(
                "Cursor Style",
                "Terminal cursor shape (block, underline, bar)",
                {
                    let styles = ["block", "underline", "bar"];
                    let current = self.config.terminal.cursor_style.clone();
                    let mut row = div().flex().items_center().gap(px(4.0));
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
                        btn = btn
                            .child(s)
                            .on_click(cx.listener(move |this, _, _, cx| {
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
                "Cursor Blink",
                "Enable cursor blinking in terminal",
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
        // Theme preference buttons (Dark / Light / System)
        let current_theme = self.config.theme.clone();
        let theme_options = [
            ("Dark", ThemePreference::Dark),
            ("Light", ThemePreference::Light),
            ("System", ThemePreference::System),
        ];

        let mut theme_buttons = div().flex().items_center().gap(px(4.0));
        for (label, pref) in &theme_options {
            let is_active = current_theme == *pref;
            let pref_clone = pref.clone();
            let mut btn = div()
                .id(ElementId::from(SharedString::from(format!("theme-pref-{}", label))))
                .px(px(10.0))
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

            btn = btn
                .child(label.to_string())
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.config.theme = pref_clone.clone();
                    this.mark_changed();
                    cx.emit(SettingsEvent::ThemeChanged(pref_clone.clone()));
                    cx.notify();
                }));

            theme_buttons = theme_buttons.child(btn);
        }

        // Terminal theme picker (built-in themes)
        let mut theme_cards = div()
            .flex()
            .gap(px(8.0))
            .flex_wrap();

        for terminal_theme in TerminalTheme::builtins() {
            let name = terminal_theme.name.clone();
            let bg_hex = terminal_theme.background.clone();
            let fg_hex = terminal_theme.foreground.clone();
            let theme_name = name.clone();

            theme_cards = theme_cards.child(
                div()
                    .id(ElementId::from(SharedString::from(format!("theme-{}", name))))
                    .w(px(120.0))
                    .h(px(70.0))
                    .rounded(px(6.0))
                    .border_1()
                    .border_color(ShellDeckColors::border())
                    .cursor_pointer()
                    .hover(|el| el.border_color(ShellDeckColors::primary()))
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap(px(4.0))
                    .child(
                        div()
                            .text_size(px(11.0))
                            .text_color(ShellDeckColors::text_primary())
                            .child(name.clone()),
                    )
                    .child(
                        div()
                            .text_size(px(9.0))
                            .text_color(ShellDeckColors::text_muted())
                            .child(format!("{} / {}", bg_hex, fg_hex)),
                    )
                    .on_click(cx.listener(move |_this, _, _, cx| {
                        tracing::info!("Terminal theme selected: {}", theme_name);
                        // Find the matching built-in theme and emit it
                        if let Some(theme) = TerminalTheme::builtins()
                            .into_iter()
                            .find(|t| t.name == theme_name)
                        {
                            cx.emit(SettingsEvent::TerminalThemeChanged(theme));
                        }
                        cx.notify();
                    })),
            );
        }

        div()
            .flex()
            .flex_col()
            .gap(px(4.0))
            .child(Self::render_setting_row(
                "Theme",
                "Application color theme",
                theme_buttons,
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
                            .child("Terminal Themes"),
                    )
                    .child(theme_cards),
            )
            .child(Self::render_setting_row(
                "Sidebar Width",
                "Width of the sidebar panel in pixels",
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
                            .child("-")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.config.general.sidebar_width = (this.config.general.sidebar_width - 20.0).max(140.0);
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
                            .child("+")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.config.general.sidebar_width = (this.config.general.sidebar_width + 20.0).min(400.0);
                                this.mark_changed();
                                cx.notify();
                            })),
                    ),
            ))
    }

    fn render_about() -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .items_center()
            .gap(px(16.0))
            .py(px(32.0))
            .child(
                div()
                    .text_size(px(24.0))
                    .font_weight(FontWeight::BOLD)
                    .text_color(ShellDeckColors::primary())
                    .child("ShellDeck"),
            )
            .child(
                div()
                    .text_size(px(14.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child("Desktop SSH & Terminal Companion"),
            )
            .child(
                div()
                    .text_size(px(13.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child("Version 0.1.0"),
            )
            .child(
                div()
                    .text_size(px(12.0))
                    .text_color(ShellDeckColors::text_muted())
                    .mt(px(16.0))
                    .child("Built with Rust, GPUI, and adabraka-ui"),
            )
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
                    .child("Settings"),
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
                    .child("Save")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.save_config(cx);
                    })),
            );
        }

        // Tab content
        let mut tab_content = div()
            .flex_grow()
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
            // Content
            .child(
                div()
                    .flex()
                    .flex_grow()
                    .id("settings-scroll")
                    .overflow_y_scroll()
                    // Tab sidebar
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(2.0))
                            .w(px(180.0))
                            .p(px(12.0))
                            .border_r_1()
                            .border_color(ShellDeckColors::border())
                            .child(self.render_tab_button(SettingsTab::General, "General", cx))
                            .child(self.render_tab_button(SettingsTab::Terminal, "Terminal", cx))
                            .child(self.render_tab_button(SettingsTab::Appearance, "Appearance", cx))
                            .child(self.render_tab_button(SettingsTab::About, "About", cx)),
                    )
                    // Tab content
                    .child(tab_content),
            )
    }
}
