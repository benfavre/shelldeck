use super::*;

impl Workspace {
    /// Render the custom window titlebar with drag area and window controls.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn render_titlebar(
        is_maximized: bool,
        theme_menu_open: bool,
        account_menu_open: bool,
        account: Option<AccountInfo>,
        account_status: AccountStatus,
        site_menu_open: bool,
        active_site_label: Option<String>,
        sites_loaded: bool,
        mode_switch: Option<(AppMode, &'static [AppMode])>,
        ui_font_size: f32,
        menu_bar_visible: bool,
        ai_configured: bool,
        ai_task_count: usize,
        handle: &WeakEntity<Self>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let titlebar_bg = ShellDeckColors::bg_sidebar();
        let titlebar_border = ShellDeckColors::border();
        let title_color = ShellDeckColors::text_primary();
        let title_dim = ShellDeckColors::text_muted();
        let accent = ShellDeckColors::primary();
        let btn_text = ShellDeckColors::text_muted();
        let btn_hover_bg = ShellDeckColors::hover_bg();

        // Title area — draggable
        let title_area = div()
            .flex_1()
            .h_full()
            .flex()
            .items_center()
            .px(px(10.0))
            .gap(px(8.0))
            .window_control_area(WindowControlArea::Drag)
            .on_mouse_down(MouseButton::Left, |_e, window, _cx| {
                window.start_window_move();
            })
            .child(crate::brand::brand_badge(20.0))
            .child(crate::brand::brand_wordmark(12.0))
            .child(
                // Version pill
                div()
                    .px(px(6.0))
                    .py(px(1.0))
                    .rounded(px(4.0))
                    .bg(ShellDeckColors::badge_bg())
                    .text_color(title_dim)
                    .text_size(px(10.0))
                    .font_weight(FontWeight::MEDIUM)
                    .child(format!("v{}", shelldeck_core::VERSION)),
            );

        // A window-control button with a rounded hover affordance and an SVG
        // glyph. `icon_path` points at an embedded asset (see main.rs Assets).
        //
        // GPUI's `svg()` element paints with its OWN `style.text.color` — it
        // does not inherit from the parent — so we set it explicitly on the
        // SVG and swap it on group hover to whiten the icon over the red
        // close background.
        let control_btn =
            |id: &'static str, icon_path: &'static str, area: WindowControlArea, danger: bool| {
                let hover_bg = if danger {
                    ShellDeckColors::error()
                } else {
                    btn_hover_bg
                };
                let group_name = SharedString::from(format!("ctrl-{id}"));
                div()
                    .id(id)
                    .group(group_name.clone())
                    .flex()
                    .items_center()
                    .justify_center()
                    .size(px(28.0))
                    .rounded(px(6.0))
                    .hover(|s| s.bg(hover_bg))
                    .window_control_area(area)
                    .child(
                        svg()
                            .path(icon_path)
                            .size(px(12.0))
                            .text_color(btn_text)
                            .group_hover(group_name, |s| s.text_color(gpui::white())),
                    )
            };

        let minimize_btn = control_btn(
            "titlebar-minimize",
            "images/minimize.svg",
            WindowControlArea::Min,
            false,
        )
        .on_click(cx.listener(|_this, _event: &ClickEvent, window, _cx| {
            window.minimize_window();
        }));

        let maximize_icon = if is_maximized {
            "images/restore.svg"
        } else {
            "images/maximize.svg"
        };
        let maximize_btn = control_btn(
            "titlebar-maximize",
            maximize_icon,
            WindowControlArea::Max,
            false,
        )
        .on_click(cx.listener(|_this, _event: &ClickEvent, window, _cx| {
            window.zoom_window();
        }));

        let h_quit = handle.clone();
        let close_btn = control_btn(
            "titlebar-close",
            "images/close.svg",
            WindowControlArea::Close,
            true,
        )
        .on_click(
            move |_event: &ClickEvent, window: &mut Window, cx: &mut App| {
                if let Some(ws) = h_quit.upgrade() {
                    if ws.read(cx).should_hide_to_tray() {
                        window.hide_window();
                        return;
                    }
                    let should_close = ws.update(cx, |ws, cx| ws.confirm_window_close(cx));
                    if should_close {
                        ws.update(cx, |ws, cx| ws.shutdown(cx));
                        cx.quit();
                    }
                }
            },
        );

        // Theme switcher — a 2x2 palette swatch that reflects the active theme
        // and toggles the dropdown menu.
        let mut theme_btn = div()
            .id("titlebar-theme")
            .flex()
            .items_center()
            .justify_center()
            .size(px(28.0))
            .rounded(px(6.0))
            .cursor_pointer()
            .hover(|s| s.bg(btn_hover_bg))
            .child(
                div()
                    .size(px(14.0))
                    .rounded(px(4.0))
                    .overflow_hidden()
                    .flex()
                    .flex_wrap()
                    .child(div().size(px(7.0)).bg(ShellDeckColors::primary()))
                    .child(div().size(px(7.0)).bg(ShellDeckColors::success()))
                    .child(div().size(px(7.0)).bg(ShellDeckColors::warning()))
                    .child(div().size(px(7.0)).bg(ShellDeckColors::error())),
            )
            .on_click(cx.listener(|this, _event: &ClickEvent, _window, cx| {
                this.theme_menu_open = !this.theme_menu_open;
                cx.notify();
            }));
        if theme_menu_open {
            theme_btn = theme_btn.bg(ShellDeckColors::hover_bg());
        }

        // Settings is a personal surface shared by User, Support and Dev.
        let settings_btn = account.as_ref().map(|_| {
            let settings_handle = handle.clone();
            IconButton::new("settings")
                .variant(ButtonVariant::Ghost)
                .size(gpui::px(28.0))
                .icon_size(gpui::px(14.0))
                .on_click(move |_, _, cx| {
                    if let Some(ws) = settings_handle.upgrade() {
                        ws.update(cx, |ws, cx| ws.open_settings(cx));
                    }
                })
        });

        // Account chip — "Se connecter" when logged out, otherwise an
        // avatar-initial + name with a health status dot. Toggles the account
        // dropdown.
        let mut account_btn = div()
            .id("titlebar-account")
            .flex()
            .items_center()
            .gap(px(6.0))
            .h(px(28.0))
            .px(px(7.0))
            .rounded(px(6.0))
            .cursor_pointer()
            .hover(|s| s.bg(btn_hover_bg));

        if let Some(acct) = &account {
            let dot = account_status.dot_color();
            account_btn = account_btn
                .child(
                    div()
                        .relative()
                        .child(
                            div()
                                .size(px(18.0))
                                .rounded_full()
                                .bg(accent.opacity(0.20))
                                .flex()
                                .items_center()
                                .justify_center()
                                .text_size(px(10.0))
                                .font_weight(FontWeight::BOLD)
                                .text_color(accent)
                                .child(acct.initial()),
                        )
                        .child(
                            div()
                                .absolute()
                                .bottom(px(-1.0))
                                .right(px(-1.0))
                                .size(px(7.0))
                                .rounded_full()
                                .bg(dot)
                                .border_1()
                                .border_color(titlebar_bg),
                        ),
                )
                .child(
                    div()
                        .max_w(px(96.0))
                        .overflow_hidden()
                        .text_size(px(11.0))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(title_color)
                        .child(acct.display_name()),
                );
        } else {
            account_btn = account_btn
                .child(
                    div()
                        .size(px(18.0))
                        .rounded_full()
                        .bg(ShellDeckColors::badge_bg())
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_size(px(10.0))
                        .text_color(title_dim)
                        .child("\u{25CB}"), // ○ placeholder avatar
                )
                .child(
                    div()
                        .text_size(px(11.0))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(title_dim)
                        .child(crate::t!("account.sign_in").to_string()),
                );
        }

        account_btn = account_btn.on_click(cx.listener(|this, event: &ClickEvent, _window, cx| {
            // Remember where the chip is so the panel can be anchored to it
            // rather than to the window edge.
            this.account_menu_pos = Some(event.position());
            this.account_menu_open = !this.account_menu_open;
            if this.account_menu_open {
                this.theme_menu_open = false;
            }
            cx.notify();
        }));
        if account_menu_open {
            account_btn = account_btn.bg(ShellDeckColors::hover_bg());
        }

        // Mode switcher — only the exact modes granted to this account.
        let mode_switcher = mode_switch.map(|(current, allowed_modes)| {
            let mut seg = div()
                .flex()
                .items_center()
                .gap(px(1.0))
                .p(px(2.0))
                .rounded(px(6.0))
                .bg(ShellDeckColors::badge_bg());
            for &m in allowed_modes {
                let active = m == current;
                let mut btn = div()
                    .id(ElementId::from(SharedString::from(format!(
                        "titlebar-mode-{}",
                        m.label()
                    ))))
                    .px(px(8.0))
                    .py(px(3.0))
                    .rounded(px(5.0))
                    .text_size(px(11.0))
                    .font_weight(FontWeight::MEDIUM)
                    .cursor_pointer()
                    .child(m.label().to_string())
                    .on_click(cx.listener(move |this, _event: &ClickEvent, _window, cx| {
                        this.set_mode(m, cx);
                    }));
                if active {
                    btn = btn
                        .bg(ShellDeckColors::bg_surface())
                        .text_color(ShellDeckColors::text_primary());
                } else {
                    btn = btn
                        .text_color(title_dim)
                        .hover(|s| s.text_color(title_color));
                }
                seg = seg.child(btn);
            }
            seg
        });

        // Site chip — shown only when signed in and the sites directory has
        // loaded. Displays the active site label or "Tous les sites".
        let show_site_chip = account.is_some() && sites_loaded;
        let site_chip = if show_site_chip {
            let label = active_site_label.unwrap_or_else(|| "Tous les sites".to_string());
            let mut chip = div()
                .id("titlebar-site")
                .flex()
                .items_center()
                .gap(px(5.0))
                .h(px(28.0))
                .px(px(8.0))
                .rounded(px(6.0))
                .cursor_pointer()
                .hover(|s| s.bg(btn_hover_bg))
                .child(
                    div()
                        .text_size(px(11.0))
                        .text_color(title_dim)
                        .child("\u{25C9}"), // ◉ site glyph
                )
                .child(
                    div()
                        .max_w(px(120.0))
                        .overflow_hidden()
                        .text_size(px(11.0))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(title_color)
                        .child(label),
                )
                .child(
                    svg()
                        .path("images/chevron-down.svg")
                        .size(px(9.0))
                        .text_color(title_dim),
                )
                .on_click(cx.listener(|this, _event: &ClickEvent, _window, cx| {
                    this.site_menu_open = !this.site_menu_open;
                    if this.site_menu_open {
                        this.theme_menu_open = false;
                        this.account_menu_open = false;
                    }
                    cx.notify();
                }));
            if site_menu_open {
                chip = chip.bg(ShellDeckColors::hover_bg());
            }
            Some(chip)
        } else {
            None
        };

        // UI scale controls — a compact −/value/+ group that adjusts the app
        // font size (which drives proportional UI scaling) live.
        let scale_btn = |id: &'static str, icon_path: &'static str| {
            let group_name = SharedString::from(format!("scale-{id}"));
            div()
                .id(id)
                .group(group_name.clone())
                .flex()
                .items_center()
                .justify_center()
                .size(px(22.0))
                .rounded(px(5.0))
                .cursor_pointer()
                .hover(|s| s.bg(btn_hover_bg))
                .child(
                    svg()
                        .path(icon_path)
                        .size(px(11.0))
                        .text_color(btn_text)
                        .group_hover(group_name, |s| {
                            s.text_color(ShellDeckColors::text_primary())
                        }),
                )
        };
        let dec_btn = scale_btn("titlebar-scale-down", "images/minus.svg").on_click(cx.listener(
            |this, _event: &ClickEvent, _window, cx| {
                this.settings
                    .update(cx, |settings, cx| settings.adjust_ui_font_size(-1.0, cx));
                cx.notify();
            },
        ));
        let inc_btn = scale_btn("titlebar-scale-up", "images/plus.svg").on_click(cx.listener(
            |this, _event: &ClickEvent, _window, cx| {
                this.settings
                    .update(cx, |settings, cx| settings.adjust_ui_font_size(1.0, cx));
                cx.notify();
            },
        ));
        let scale_group = div()
            .flex()
            .items_center()
            .gap(px(1.0))
            .child(dec_btn)
            .child(
                div()
                    .min_w(px(30.0))
                    .flex()
                    .justify_center()
                    .text_size(px(11.0))
                    .text_color(title_dim)
                    .child(format!("{}px", ui_font_size as i32)),
            )
            .child(inc_btn);

        // The menu row contains its own visibility toggle, so hiding it would
        // otherwise remove the only discoverable way to restore it. This
        // compact titlebar affordance exists only while the row is hidden.
        let restore_menu_button = (!menu_bar_visible).then(|| {
            let tooltip: SharedString =
                format!("{} · Ctrl+Shift+M", t!("menu.view.menu_bar")).into();
            let workspace = handle.clone();
            div()
                .id("titlebar-restore-menu")
                .flex()
                .items_center()
                .justify_center()
                .size(px(28.0))
                .rounded(px(6.0))
                .cursor_pointer()
                .hover(|el| el.bg(btn_hover_bg))
                .tooltip(move |_, cx| {
                    cx.new(|_| WorkspaceTooltip {
                        label: tooltip.clone(),
                    })
                    .into()
                })
                .on_click(move |_, _, cx| {
                    if let Some(workspace) = workspace.upgrade() {
                        workspace.update(cx, |this, cx| this.toggle_menu_bar(cx));
                    }
                })
                .child(lucide_icon("list-checks", 14.0, btn_text))
        });

        let ai_button = ai_configured.then(|| {
            let tooltip: SharedString = t!("ai.assistant.open").to_string().into();
            let workspace = handle.clone();
            div()
                .id("titlebar-ai")
                .flex()
                .items_center()
                .justify_center()
                .h(px(28.0))
                .w(if ai_task_count == 0 {
                    px(28.0)
                } else {
                    px(44.0)
                })
                .gap(px(4.0))
                .rounded(px(6.0))
                .border_1()
                .border_color(ShellDeckColors::primary().opacity(0.40))
                .bg(ShellDeckColors::primary().opacity(0.12))
                .cursor_pointer()
                .hover(|el| el.bg(ShellDeckColors::primary().opacity(0.22)))
                .tooltip(move |_, cx| {
                    cx.new(|_| WorkspaceTooltip {
                        label: tooltip.clone(),
                    })
                    .into()
                })
                .on_click(move |_, _, cx| {
                    if let Some(workspace) = workspace.upgrade() {
                        workspace.update(cx, |this, cx| this.open_ai_assistant(cx));
                    }
                })
                .child(
                    svg()
                        .path(lucide_path("sparkles"))
                        .size(px(14.0))
                        .text_color(ShellDeckColors::primary()),
                )
                .when(ai_task_count > 0, |button| {
                    button.child(
                        div()
                            .flex()
                            .items_center()
                            .justify_center()
                            .min_w(px(14.0))
                            .h(px(14.0))
                            .px(px(3.0))
                            .rounded_full()
                            .bg(ShellDeckColors::primary())
                            .text_size(px(9.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(white())
                            .child(if ai_task_count > 99 {
                                "99+".to_string()
                            } else {
                                ai_task_count.to_string()
                            }),
                    )
                })
        });

        // Subtle vertical divider between the chrome control clusters.
        let divider = || div().w(px(1.0)).h(px(16.0)).mx(px(4.0)).bg(titlebar_border);

        let mut titlebar = div()
            .flex()
            .items_center()
            .w_full()
            .flex_shrink_0()
            .h(px(40.0))
            .bg(titlebar_bg);
        // Rounded clipping does not propagate to child backgrounds in GPUI.
        // This element owns the titlebar background, so it must own the
        // floating window's top radius as well.
        if !is_maximized {
            titlebar = titlebar.rounded_t(use_theme().tokens.radius_xl);
        }
        titlebar
            .border_b_1()
            .border_color(titlebar_border)
            .child(title_area)
            .child(
                div()
                    .flex()
                    .items_center()
                    .h_full()
                    .gap(px(4.0))
                    .pr(px(8.0))
                    .children(restore_menu_button)
                    .child(scale_group)
                    .children(ai_button)
                    .child(divider())
                    .child(account_btn)
                    .children(mode_switcher)
                    .children(site_chip)
                    .children(settings_btn)
                    .child(theme_btn)
                    .child(divider())
                    .child(minimize_btn)
                    .child(maximize_btn)
                    .child(close_btn),
            )
    }

    /// Render the titlebar theme-switcher dropdown: a full-window backdrop that
    /// dismisses on click, plus an anchored panel listing every app theme.
    pub(super) fn render_theme_menu(&self, cx: &mut Context<Self>) -> impl IntoElement {
        use shelldeck_core::config::app_config::ThemePreference;

        let current = self.app_config.theme.clone();

        let mut panel = div()
            .id("theme-menu-panel")
            .absolute()
            .top(px(46.0))
            .right(px(12.0))
            .w(px(212.0))
            .max_h(px(440.0))
            .overflow_y_scroll()
            .bg(ShellDeckColors::bg_surface())
            .border_1()
            .border_color(ShellDeckColors::border())
            .rounded(px(10.0))
            .shadow(
                vec![BoxShadow {
                    color: hsla(0.0, 0.0, 0.0, 0.45),
                    // BoxShadow fields are typed `Pixels` — real pixels, not rems.
                    offset: point(gpui::px(0.0), gpui::px(4.0)),
                    blur_radius: gpui::px(20.0),
                    spread_radius: gpui::px(0.0),
                    inset: false,
                }]
                .into(),
            )
            .p(px(4.0))
            .flex()
            .flex_col()
            .gap(px(1.0))
            // Clicks inside the panel must not bubble to the dismiss backdrop.
            .on_mouse_down(MouseButton::Left, |_e, _window, cx: &mut App| {
                cx.stop_propagation();
            });

        for pref in ThemePreference::all() {
            let pref = pref.clone();
            let is_active = current == pref;
            let p = crate::theme::palette_for(&pref);
            let label = pref.display_name().to_string();

            let mut item = div()
                .id(ElementId::from(SharedString::from(format!(
                    "theme-menu-{label}"
                ))))
                .flex()
                .items_center()
                .gap(px(8.0))
                .px(px(8.0))
                .py(px(5.0))
                .rounded(px(6.0))
                .cursor_pointer()
                .hover(|s| s.bg(ShellDeckColors::hover_bg()))
                // A mini swatch showing the theme's background + accent.
                .child(
                    div()
                        .size(px(16.0))
                        .rounded(px(4.0))
                        .bg(p.bg_primary)
                        .border_1()
                        .border_color(p.border)
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(div().size(px(8.0)).rounded(px(2.0)).bg(p.primary)),
                )
                .child(
                    div()
                        .flex_1()
                        .text_size(px(12.0))
                        .text_color(if is_active {
                            ShellDeckColors::primary()
                        } else {
                            ShellDeckColors::text_primary()
                        })
                        .font_weight(if is_active {
                            FontWeight::SEMIBOLD
                        } else {
                            FontWeight::NORMAL
                        })
                        .child(label),
                );

            if is_active {
                item = item.child(
                    div()
                        .text_size(px(12.0))
                        .text_color(ShellDeckColors::primary())
                        .child("\u{2713}"),
                );
            }

            item = item.on_click(cx.listener(move |this, _event: &ClickEvent, _window, cx| {
                let pref = pref.clone();
                this.settings.update(cx, |settings, cx| {
                    settings.select_app_theme(pref, cx);
                });
                this.theme_menu_open = false;
                cx.notify();
            }));

            panel = panel.child(item);
        }

        // Transparent full-window backdrop — a click anywhere outside the panel
        // closes the menu.
        div()
            .id("theme-menu-backdrop")
            .occlude()
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _e, _window, cx| {
                    this.theme_menu_open = false;
                    cx.notify();
                }),
            )
            .child(panel)
    }

    /// Render the titlebar account dropdown: a dismiss backdrop plus an anchored
    /// panel. Logged out shows the sign-in options (password modal + OIDC);
    /// logged in shows the account, sync, and sign-out controls.
    pub(super) fn render_account_menu(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let shadow = vec![BoxShadow {
            color: hsla(0.0, 0.0, 0.0, 0.45),
            // BoxShadow fields are typed `Pixels` — real pixels, not rems.
            offset: point(gpui::px(0.0), gpui::px(4.0)),
            blur_radius: gpui::px(20.0),
            spread_radius: gpui::px(0.0),
            inset: false,
        }];

        let mut panel = div()
            .id("account-menu-panel")
            .w(px(288.0))
            .bg(ShellDeckColors::bg_surface())
            .border_1()
            .border_color(ShellDeckColors::border())
            .rounded(px(10.0))
            .shadow(shadow.into())
            .p(px(12.0))
            .flex()
            .flex_col()
            .gap(px(8.0))
            // Clicks inside must not bubble to the dismiss backdrop.
            .on_mouse_down(MouseButton::Left, |_e, _window, cx: &mut App| {
                cx.stop_propagation();
            });

        // A full-width secondary (outlined) menu button.
        let secondary_btn = |id: &'static str, label: String| {
            div()
                .id(id)
                .w_full()
                .px(px(10.0))
                .py(px(8.0))
                .rounded(px(6.0))
                .border_1()
                .border_color(ShellDeckColors::border())
                .bg(ShellDeckColors::bg_primary())
                .text_size(px(13.0))
                .text_color(ShellDeckColors::text_primary())
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .hover(|s| s.bg(ShellDeckColors::hover_bg()))
                .child(label)
        };

        if let Some(acct) = self.app_config.account.clone() {
            // --- LOGGED IN ---
            panel = panel.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.0))
                    .pb(px(8.0))
                    .border_b_1()
                    .border_color(ShellDeckColors::border())
                    .child(
                        div()
                            .size(px(34.0))
                            .rounded_full()
                            .bg(ShellDeckColors::primary().opacity(0.20))
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_size(px(15.0))
                            .font_weight(FontWeight::BOLD)
                            .text_color(ShellDeckColors::primary())
                            .child(acct.initial()),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .overflow_hidden()
                            .child(
                                div()
                                    .text_size(px(13.0))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(ShellDeckColors::text_primary())
                                    .child(acct.display_name()),
                            )
                            .child(
                                div()
                                    .text_size(px(11.0))
                                    .text_color(ShellDeckColors::text_muted())
                                    .child(acct.email.clone()),
                            ),
                    ),
            );

            let status_label = match self.account_status {
                AccountStatus::Ok => "Connecté",
                AccountStatus::Rejected => "Session expirée — reconnectez-vous",
                AccountStatus::Offline => "Hors ligne",
                AccountStatus::Unknown => "Vérification…",
            };
            let info_row = |label: String, value: String| {
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(px(8.0))
                    .child(
                        div()
                            .text_size(px(11.0))
                            .text_color(ShellDeckColors::text_muted())
                            .child(label),
                    )
                    .child(
                        div()
                            .max_w(px(180.0))
                            .overflow_hidden()
                            .text_size(px(11.0))
                            .text_color(ShellDeckColors::text_primary())
                            .child(value),
                    )
            };
            panel = panel
                .child(info_row("Serveur".to_string(), self.account_base_url()))
                .child(info_row(
                    "Appareil".to_string(),
                    cloud_account::device_name(),
                ))
                .child(info_row(
                    t!("user.sites.active").to_string(),
                    self.app_config
                        .cloud_sync
                        .active_site_label
                        .clone()
                        .unwrap_or_else(|| "Tous les sites".to_string()),
                ))
                .child(info_row(
                    t!("settings.cloud_sync.status.label").to_string(),
                    status_label.to_string(),
                ));

            panel = panel.child(
                secondary_btn("account-sync", t!("user.sync").to_string()).on_click(cx.listener(
                    |this, _: &ClickEvent, _, cx| {
                        this.account_menu_open = false;
                        this.cloud_sync_now(cx);
                    },
                )),
            );
            panel = panel.child(
                secondary_btn("account-logout", t!("user.account.logout").to_string())
                    .text_color(ShellDeckColors::error())
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.logout_account(cx);
                    })),
            );
        } else {
            // --- LOGGED OUT ---
            panel = panel
                .child(
                    div()
                        .text_size(px(13.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(ShellDeckColors::text_primary())
                        .child(t!("user.account.title").to_string()),
                )
                .child(
                    div()
                        .text_size(px(11.0))
                        .text_color(ShellDeckColors::text_muted())
                        .child(t!("user.account.hint").to_string()),
                );

            // Primary: open the password + OIDC login modal.
            panel = panel.child(
                div()
                    .id("account-signin")
                    .w_full()
                    .px(px(10.0))
                    .py(px(9.0))
                    .rounded(px(6.0))
                    .bg(ShellDeckColors::primary())
                    .text_size(px(13.0))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(white())
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .child(crate::t!("account.sign_in").to_string())
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.show_login_form(cx);
                    })),
            );

            // Divider.
            panel = panel.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(div().flex_1().h(px(1.0)).bg(ShellDeckColors::border()))
                    .child(
                        div()
                            .text_size(px(10.0))
                            .text_color(ShellDeckColors::text_muted())
                            .child(t!("user.account.or_one_click").to_string()),
                    )
                    .child(div().flex_1().h(px(1.0)).bg(ShellDeckColors::border())),
            );

            panel = panel
                .child(
                    secondary_btn("account-oidc-sso", t!("login.oidc_sso").to_string()).on_click(
                        cx.listener(|this, _: &ClickEvent, _, cx| {
                            this.start_oidc_login(Some("sso".to_string()), cx);
                        }),
                    ),
                )
                .child(
                    div()
                        .flex()
                        .gap(px(8.0))
                        .child(div().flex_1().child(
                            secondary_btn("account-oidc-google", "Google".to_string()).on_click(
                                cx.listener(|this, _: &ClickEvent, _, cx| {
                                    this.start_oidc_login(Some("google".to_string()), cx);
                                }),
                            ),
                        ))
                        .child(div().flex_1().child(
                            secondary_btn("account-oidc-github", "GitHub".to_string()).on_click(
                                cx.listener(|this, _: &ClickEvent, _, cx| {
                                    this.start_oidc_login(Some("github".to_string()), cx);
                                }),
                            ),
                        )),
                );
        }

        // Anchored to the chip that opened it, not to the window edge — same
        // `deferred(anchored())` treatment as the sidebar kebab, so it also
        // flips back inside the viewport near an edge.
        //
        // Only the X comes from the click. Taking the Y too made the panel hang
        // from wherever inside the chip the pointer happened to be, which put it
        // against the titlebar. The drop is a fixed distance below the titlebar
        // (40px tall, see line 491) expressed in rems, so it follows the UI
        // scale the titlebar itself follows.
        let drop = px(52.0).to_pixels(window.rem_size());
        let anchor_x = self
            .account_menu_pos
            .map(|position| position.x)
            .unwrap_or_else(|| gpui::px(900.0));
        let anchor = point(anchor_x, drop);

        // Dismiss backdrop.
        div()
            .id("account-menu-backdrop")
            .occlude()
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _e, _window, cx| {
                    this.account_menu_open = false;
                    cx.notify();
                }),
            )
            .child(
                deferred(
                    anchored()
                        // Centre the 288px panel under the chip.
                        .position(anchor + point(gpui::px(-144.0), gpui::px(0.0)))
                        .anchor(gpui::Corner::TopLeft)
                        .snap_to_window_with_margin(gpui::px(8.0))
                        .child(panel),
                )
                .with_priority(2),
            )
    }

    /// Render the titlebar site-switcher dropdown: "Tous les sites" + the site
    /// list (active pinned, connection-bearing next, capped) + "Ouvrir dans
    /// Manage" area links for the active site.
    pub(super) fn render_site_menu(&self, cx: &mut Context<Self>) -> impl IntoElement {
        const CAP: usize = 20;
        let payload = self.site_directory.clone().unwrap_or_default();
        let active_id = self.app_config.cloud_sync.active_site_id.clone();

        // Which sites have at least one synced connection.
        let conn_site_ids: std::collections::HashSet<String> = self
            .connections
            .iter()
            .filter_map(|c| c.site_id.map(|id| id.to_string()))
            .collect();

        // Sort: active first, then connection-bearing, then alphabetical.
        let mut sites: Vec<&ManagedSiteInfo> = payload.sites.iter().collect();
        sites.sort_by(|a, b| {
            let a_active = active_id.as_deref() == Some(a.site_id.as_str());
            let b_active = active_id.as_deref() == Some(b.site_id.as_str());
            let a_conn = conn_site_ids.contains(&a.site_id);
            let b_conn = conn_site_ids.contains(&b.site_id);
            b_active.cmp(&a_active).then(b_conn.cmp(&a_conn)).then(
                a.display_label()
                    .to_lowercase()
                    .cmp(&b.display_label().to_lowercase()),
            )
        });
        let total = sites.len();
        let hidden = total.saturating_sub(CAP);

        let row =
            |id: ElementId, label: String, active: bool, badge: Option<String>| -> Stateful<Div> {
                let mut r = div()
                    .id(id)
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .px(px(8.0))
                    .py(px(6.0))
                    .rounded(px(6.0))
                    .cursor_pointer()
                    .hover(|s| s.bg(ShellDeckColors::hover_bg()));
                if active {
                    r = r.bg(ShellDeckColors::selected_bg());
                }
                r = r.child(
                    div()
                        .flex_1()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_size(px(12.0))
                        .text_color(ShellDeckColors::text_primary())
                        .child(label),
                );
                if let Some(b) = badge {
                    r = r.child(
                        div()
                            .flex_shrink_0()
                            .px(px(5.0))
                            .py(px(1.0))
                            .rounded(px(8.0))
                            .bg(ShellDeckColors::badge_bg())
                            .text_size(px(10.0))
                            .text_color(ShellDeckColors::text_muted())
                            .child(b),
                    );
                }
                if active {
                    r = r.child(
                        div()
                            .flex_shrink_0()
                            .text_size(px(12.0))
                            .text_color(ShellDeckColors::primary())
                            .child("\u{2713}"),
                    );
                }
                r
            };

        let shadow = vec![BoxShadow {
            color: hsla(0.0, 0.0, 0.0, 0.45),
            // BoxShadow fields are typed `Pixels` — real pixels, not rems.
            offset: point(gpui::px(0.0), gpui::px(4.0)),
            blur_radius: gpui::px(20.0),
            spread_radius: gpui::px(0.0),
            inset: false,
        }];

        let mut panel = div()
            .id("site-menu-panel")
            .absolute()
            .top(px(46.0))
            .right(px(12.0))
            .w(px(300.0))
            .max_h(px(480.0))
            .overflow_y_scroll()
            .bg(ShellDeckColors::bg_surface())
            .border_1()
            .border_color(ShellDeckColors::border())
            .rounded(px(10.0))
            .shadow(shadow.into())
            .p(px(6.0))
            .flex()
            .flex_col()
            .gap(px(1.0))
            .on_mouse_down(MouseButton::Left, |_e, _window, cx: &mut App| {
                cx.stop_propagation();
            });

        panel = panel.child(Self::render_site_section_header(&format!(
            "SITES ({})",
            total
        )));

        // "Tous les sites" (clear the filter).
        panel = panel.child(
            row(
                ElementId::from(SharedString::from("site-all")),
                "Tous les sites".to_string(),
                active_id.is_none(),
                None,
            )
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.select_site(None, None, cx);
            })),
        );

        for site in sites.iter().take(CAP) {
            let sid = site.site_id.clone();
            let label = site.display_label();
            let is_active = active_id.as_deref() == Some(sid.as_str());
            let badge = if conn_site_ids.contains(&sid) {
                Some("connexions".to_string())
            } else {
                None
            };
            let elem_id = ElementId::from(SharedString::from(format!("site-{}", sid)));
            let sid_for_click = sid.clone();
            let label_for_click = label.clone();
            panel = panel.child(row(elem_id, label, is_active, badge).on_click(cx.listener(
                move |this, _: &ClickEvent, _, cx| {
                    this.select_site(
                        Some(sid_for_click.clone()),
                        Some(label_for_click.clone()),
                        cx,
                    );
                },
            )));
        }

        if hidden > 0 {
            panel = panel.child(
                div()
                    .px(px(8.0))
                    .py(px(6.0))
                    .text_size(px(11.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(format!(
                        "+{} autres sites (les sites avec connexions sont priorisés)",
                        hidden
                    )),
            );
        }

        // "Ouvrir dans Manage" — area links for the active site.
        if let Some(active_site) = self.active_site_info() {
            if !payload.areas.is_empty() {
                panel = panel.child(Self::render_site_section_header(&format!(
                    "OUVRIR DANS MANAGE — {}",
                    active_site.display_label()
                )));
                for area in &payload.areas {
                    let path = area.path.clone();
                    panel = panel.child(
                        div()
                            .id(ElementId::from(SharedString::from(format!(
                                "area-{}",
                                area.key
                            ))))
                            .flex()
                            .items_center()
                            .gap(px(8.0))
                            .px(px(8.0))
                            .py(px(6.0))
                            .rounded(px(6.0))
                            .cursor_pointer()
                            .hover(|s| s.bg(ShellDeckColors::hover_bg()))
                            .child(
                                div()
                                    .flex_1()
                                    .overflow_hidden()
                                    .whitespace_nowrap()
                                    .text_size(px(12.0))
                                    .text_color(ShellDeckColors::text_primary())
                                    .child(area.label.clone()),
                            )
                            .child(
                                div()
                                    .flex_shrink_0()
                                    .text_size(px(11.0))
                                    .text_color(ShellDeckColors::text_muted())
                                    .child("\u{2197}"), // ↗
                            )
                            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                this.open_manage_area(path.clone(), cx);
                            })),
                    );
                }
            }
        }

        // Dismiss backdrop.
        div()
            .id("site-menu-backdrop")
            .occlude()
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _e, _window, cx| {
                    this.site_menu_open = false;
                    cx.notify();
                }),
            )
            .child(panel)
    }

    /// Render the sidebar kebab (⋮) row-action menu: a backdrop that dismisses
    /// on click plus an anchored panel with SSH / Edit / bext / Delete for the
    /// clicked connection. Positioned at the kebab's window-relative click
    /// coordinates.
    pub(super) fn render_sidebar_kebab_menu(
        &self,
        conn_id: Uuid,
        pos: Point<Pixels>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let conn_name = self
            .connections
            .iter()
            .find(|c| c.id == conn_id)
            .map(|c| c.display_name().to_string())
            .unwrap_or_else(|| "Connection".to_string());

        let shadow = vec![BoxShadow {
            color: hsla(0.0, 0.0, 0.0, 0.35),
            // BoxShadow fields are typed `Pixels` — real pixels, not rems.
            offset: point(gpui::px(0.0), gpui::px(4.0)),
            blur_radius: gpui::px(16.0),
            spread_radius: gpui::px(0.0),
            inset: false,
        }];

        // Header (connection name) — reminds the user which row is targeted.
        let header = div()
            .px(px(10.0))
            .py(px(6.0))
            .text_size(px(11.0))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(ShellDeckColors::text_muted())
            .overflow_hidden()
            .whitespace_nowrap()
            .child(conn_name);

        #[allow(clippy::type_complexity)]
        // local closure param; a type alias would need Self, disallowed here
        let item = |id: &'static str,
                    label: &'static str,
                    accent: gpui::Hsla,
                    danger: bool,
                    on_click: Box<dyn Fn(&mut Self, &mut Context<Self>)>|
         -> gpui::Stateful<Div> {
            let hover_bg = if danger {
                ShellDeckColors::error().opacity(0.12)
            } else {
                accent.opacity(0.12)
            };
            let hover_text = if danger {
                ShellDeckColors::error()
            } else {
                accent
            };
            div()
                .id(ElementId::from(SharedString::from(format!(
                    "kebab-item-{id}-{conn_id}"
                ))))
                .flex()
                .items_center()
                .gap(px(8.0))
                .px(px(10.0))
                .py(px(6.0))
                .rounded(px(5.0))
                .text_size(px(12.0))
                .text_color(if danger {
                    ShellDeckColors::error()
                } else {
                    ShellDeckColors::text_primary()
                })
                .cursor_pointer()
                .hover(move |el| el.bg(hover_bg).text_color(hover_text))
                .child(label)
                .on_click(cx.listener(move |this, _event: &ClickEvent, _window, cx| {
                    cx.stop_propagation();
                    this.sidebar_kebab_menu = None;
                    on_click(this, cx);
                }))
        };

        let panel = div()
            .id("sidebar-kebab-panel")
            .occlude()
            .w(px(200.0))
            .bg(ShellDeckColors::bg_surface())
            .border_1()
            .border_color(ShellDeckColors::border())
            .rounded(px(8.0))
            .shadow(shadow.into())
            .p(px(4.0))
            .flex()
            .flex_col()
            .gap(px(1.0))
            // Clicks inside the panel must not bubble to the dismiss backdrop.
            .on_mouse_down(MouseButton::Left, |_e, _window, cx: &mut App| {
                cx.stop_propagation();
            })
            .child(header)
            .child(div().h(px(1.0)).my(px(2.0)).bg(ShellDeckColors::border()))
            .child(item(
                "ssh",
                "Connect (SSH)",
                ShellDeckColors::success(),
                false,
                Box::new(move |this, cx| {
                    if let Some(conn) = this.connections.iter().find(|c| c.id == conn_id) {
                        let conn = conn.clone();
                        this.connect_ssh(conn, cx);
                    }
                    this.active_view = ActiveView::Terminal;
                    cx.notify();
                }),
            ))
            .child(item(
                "edit",
                "Edit…",
                ShellDeckColors::primary(),
                false,
                Box::new(move |this, cx| {
                    if let Some(conn) = this.connections.iter().find(|c| c.id == conn_id) {
                        let conn = conn.clone();
                        this.show_connection_form(Some(conn), cx);
                    }
                }),
            ))
            .child(item(
                "bext",
                "Manage bext…",
                ShellDeckColors::primary(),
                false,
                Box::new(move |this, cx| {
                    this.manage_bext_for_connection(conn_id, cx);
                }),
            ))
            .child(div().h(px(1.0)).my(px(2.0)).bg(ShellDeckColors::border()))
            .child(item(
                "del",
                "Delete",
                ShellDeckColors::error(),
                true,
                Box::new(move |this, cx| {
                    // Reuse the two-step confirm flow from the existing handler.
                    this.handle_sidebar_event(&SidebarEvent::ConnectionDelete(conn_id), cx);
                }),
            ));

        // Transparent full-window backdrop — click anywhere outside dismisses.
        // The panel itself is wrapped in `deferred(anchored())` with
        // `snap_to_window_with_margin` so it flips inside the viewport when
        // the click position would otherwise push the menu off-screen
        // (previously the bottom items got clipped by the status bar).
        div()
            .id("sidebar-kebab-backdrop")
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _e, _window, cx| {
                    this.sidebar_kebab_menu = None;
                    cx.notify();
                }),
            )
            .child(
                deferred(
                    anchored()
                        .position(pos + point(gpui::px(0.0), gpui::px(4.0)))
                        .anchor(gpui::Corner::TopLeft)
                        .snap_to_window_with_margin(gpui::px(8.0))
                        .child(panel),
                )
                .with_priority(2),
            )
    }
}
