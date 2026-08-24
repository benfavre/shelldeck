use super::*;
use crate::monolith::{animated_monolith, MonolithMotion};

/// Courbe du voile de transition, rapportée à la durée *de cette*
/// transition : le palier n'est plus constant depuis que le retour dans un
/// mode déjà ouvert est court.
fn mode_transition_overlay_opacity(delta: f32, loading_ms: u64) -> f32 {
    let total_ms = (MODE_TRANSITION_OUT_MS + loading_ms + MODE_TRANSITION_IN_MS) as f32;
    let delta = delta.clamp(0.0, 1.0);
    let fade_out_end = MODE_TRANSITION_OUT_MS as f32 / total_ms;
    let fade_in_start = (MODE_TRANSITION_OUT_MS + loading_ms) as f32 / total_ms;

    if delta < fade_out_end {
        ease_in_out(delta / fade_out_end)
    } else if delta < fade_in_start {
        1.0
    } else {
        1.0 - ease_in_out((delta - fade_in_start) / (1.0 - fade_in_start))
    }
}

fn workspace_status_bar_visible(show_welcome: bool, mode: AppMode) -> bool {
    !show_welcome && mode == AppMode::Dev
}

impl Workspace {
    fn render_mode_transition_loader(
        &self,
        transition: ModeTransition,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let mode_label = match transition.target {
            AppMode::User => t!("mode.transition.user").to_string(),
            AppMode::Support => t!("mode.transition.support").to_string(),
            AppMode::Dev => t!("mode.transition.dev").to_string(),
        };
        let (mascot_motion, subtitle) = match transition.target {
            AppMode::User => (
                MonolithMotion::UserCompanion,
                t!("mode.transition.subtitle.user").to_string(),
            ),
            AppMode::Support => (
                MonolithMotion::SupportScan,
                t!("mode.transition.subtitle.support").to_string(),
            ),
            AppMode::Dev => (
                MonolithMotion::DevTyping,
                t!("mode.transition.subtitle.dev").to_string(),
            ),
        };
        div()
            .id("mode-transition-loader")
            .occlude()
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .bg(ShellDeckColors::bg_primary())
            .on_mouse_down(MouseButton::Left, |_event, _window, cx| {
                cx.stop_propagation();
            })
            .child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(14.0))
                    .child(animated_monolith(
                        "mode-transition-mascot",
                        166.0,
                        mascot_motion,
                        cx,
                    ))
                    .child(
                        div()
                            .text_size(px(18.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(ShellDeckColors::text_primary())
                            .child(
                                t!("mode.transition.title", mode = mode_label.as_str()).to_string(),
                            ),
                    )
                    .child(
                        div()
                            .text_size(px(12.0))
                            .text_color(ShellDeckColors::text_muted())
                            .child(subtitle),
                    ),
            )
            .with_animation(
                SharedString::from(format!("mode-loader-{:?}", transition.target)),
                Animation::new(std::time::Duration::from_millis(transition.total_ms())),
                move |element, delta| {
                    element.opacity(mode_transition_overlay_opacity(
                        delta,
                        transition.loading_ms,
                    ))
                },
            )
    }
}

impl Render for Workspace {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        self.window_active = _window.is_window_active();
        // Window chrome geometry is in real device-independent pixels.
        _window.set_client_inset(gpui::px(5.0));

        // Drive proportional UI scaling from the App Font Size setting. Every
        // view that styles via `crate::scale::px` (i.e. rems) tracks this rem
        // size; the terminal grid and window chrome use absolute pixels and are
        // intentionally unaffected.
        {
            use crate::scale::{rem_size_for_scale, scale_for_font_size};
            let scale = scale_for_font_size(self.ui_font_size);
            // The rem size itself is necessarily absolute — it is the unit
            // every `crate::scale::px` above resolves against.
            _window.set_rem_size(gpui::px(rem_size_for_scale(scale)));
        }

        // The menu row reads a dozen pieces of state (mode, sign-in, sidebar,
        // Monique/Fleet availability, AI); rebuilding it here keeps it honest
        // without a subscription per input.
        self.rebuild_menu_bar(_cx);
        // Same reasoning: the contextual panel reads several live entities
        // with no shared change signal.
        if self.effective_mode() == AppMode::Dev {
            self.refresh_sidebar_panels(_cx);
        }

        // Check if script editor wants to open the template browser
        if self.scripts.read(_cx).template_browser_open && self.template_browser.is_none() {
            self.scripts.update(_cx, |editor, _| {
                editor.template_browser_open = false;
            });
            self.show_template_browser(_cx);
        }

        let handle = _cx.entity().downgrade();
        let is_maximized = _window.is_maximized();

        let sidebar_resizing = self.sidebar_visible && self.sidebar.read(_cx).is_resizing();

        let output_resizing = (self.active_view == ActiveView::Scripts
            && self.scripts.read(_cx).is_output_resizing())
            || (self.active_view == ActiveView::ServerSync
                && (self.server_sync.read(_cx).is_panel_dragging()
                    || self.server_sync.read(_cx).is_log_resizing()
                    || self.server_sync.read(_cx).is_discovery_resizing()));

        // Build main content area — flex_grow fills between titlebar and status bar
        let mut main_area = div().flex().flex_grow().min_h(px(0.0)).overflow_hidden();

        // Pre-login landing: intercepts before `effective_mode()` gets a say.
        // There is no guest/local bypass; logout returns here as well.
        if self.show_welcome() {
            main_area = main_area.child(self.render_welcome_screen(is_maximized, _cx));
            // Fall through to render titlebar + status bar chrome around
            // the welcome — no sidebar, no mode-specific children.
        } else if self.settings_open {
            main_area = main_area.child(self.settings.clone());
        } else {
            // The app mode selects the whole surface. User/Support are full-pane
            // manage surfaces (no sidebar); Dev is the classic terminal workspace.
            // Dev views (terminal sessions etc.) are hidden, never destroyed.
            match self.effective_mode() {
                AppMode::Support => {
                    main_area = main_area.child(self.support.clone());
                }
                AppMode::User => {
                    main_area = main_area.child(self.render_user_home(is_maximized, _cx));
                }
                AppMode::Dev => {
                    // Always rendered: the activity rail stays on screen even
                    // when the panel is collapsed (VS Code layout). The
                    // sidebar itself decides what to draw from its own
                    // collapsed / nav-collapsed state.
                    main_area = main_area.child(self.sidebar.clone());

                    let mut content = div().flex_grow().w_full().min_h(px(0.0)).overflow_hidden();
                    if !output_resizing && !sidebar_resizing {
                        content = content.block_mouse_except_scroll();
                    }

                    match self.active_view {
                        ActiveView::Dashboard => content = content.child(self.dashboard.clone()),
                        ActiveView::Terminal => content = content.child(self.terminal.clone()),
                        ActiveView::Agents => content = content.child(self.agent_console.clone()),
                        ActiveView::Scripts => content = content.child(self.scripts.clone()),
                        ActiveView::PortForwards => {
                            content = content.child(self.port_forwards.clone())
                        }
                        ActiveView::ServerSync => content = content.child(self.server_sync.clone()),
                        ActiveView::Sites => content = content.child(self.sites.clone()),
                        ActiveView::Recent => content = content.child(self.recent.clone()),
                        ActiveView::FileEditor => content = content.child(self.file_editor.clone()),
                        ActiveView::MoniqueConsole => {
                            content = content.child(self.monique_view.clone())
                        }
                        ActiveView::Fleet => content = content.child(self.fleet_view.clone()),
                        ActiveView::BextCloud => content = content.child(self.bext_view.clone()),
                        ActiveView::Settings => content = content.child(self.settings.clone()),
                    }

                    main_area = main_area.child(content);
                }
            }
        } // end of `else` (not-welcome branch)

        let h1 = handle.clone();
        let h2 = handle.clone();
        let h3 = handle.clone();
        let h4 = handle.clone();
        let h5 = handle.clone();
        let h6 = handle.clone();
        let h7 = handle.clone();
        let h8 = handle.clone();
        let h9 = handle.clone();
        let h10 = handle.clone();
        let h11 = handle.clone();
        let h12 = handle.clone();
        let h13 = handle.clone();
        let h14 = handle.clone();
        let h15 = handle.clone();
        let h16 = handle.clone();
        let h17 = handle.clone();
        let h18 = handle.clone();
        let h19 = handle.clone();

        let mut root = div()
            .relative()
            .flex()
            .flex_col()
            .size_full()
            .bg(ShellDeckColors::bg_primary())
            .id("workspace-root")
            .track_focus(&self.focus_handle)
            .on_action(move |_: &NewTerminal, _window, cx| {
                if let Some(ws) = h1.upgrade() {
                    ws.update(cx, |ws, cx| {
                        ws.open_new_terminal(cx);
                        cx.notify();
                    });
                }
            })
            .on_action(move |_: &ToggleSidebar, _window, cx| {
                if let Some(ws) = h2.upgrade() {
                    ws.update(cx, |ws, cx| {
                        ws.toggle_sidebar(cx);
                        cx.notify();
                    });
                }
            })
            .on_action(move |_: &OpenSettings, _window, cx| {
                if let Some(ws) = h3.upgrade() {
                    ws.update(cx, |ws, cx| {
                        ws.open_settings(cx);
                    });
                }
            })
            .on_action(move |_: &Quit, _window, cx| {
                if let Some(ws) = h4.upgrade() {
                    ws.update(cx, |ws, cx| {
                        ws.shutdown(cx);
                        cx.quit();
                    });
                }
            })
            .on_action(move |_: &ToggleCommandPalette, window, cx| {
                if let Some(ws) = h5.upgrade() {
                    ws.update(cx, |ws, cx| {
                        ws.command_palette.update(cx, |palette, cx| {
                            palette.toggle(window, cx);
                            cx.notify();
                        });
                        cx.notify();
                    });
                }
            })
            .on_action(move |_: &ToggleMenuBar, _window, cx| {
                if let Some(ws) = h18.upgrade() {
                    ws.update(cx, |ws, cx| ws.toggle_menu_bar(cx));
                }
            })
            .on_action(move |_: &OpenQuickConnect, _window, cx| {
                if let Some(ws) = h6.upgrade() {
                    ws.update(cx, |ws, cx| {
                        ws.show_connection_form(None, cx);
                    });
                }
            })
            .on_action(move |_: &NextTab, _window, cx| {
                if let Some(ws) = h7.upgrade() {
                    ws.update(cx, |ws, cx| ws.next_tab(cx));
                }
            })
            .on_action(move |_: &PrevTab, _window, cx| {
                if let Some(ws) = h8.upgrade() {
                    ws.update(cx, |ws, cx| ws.prev_tab(cx));
                }
            })
            .on_action(move |_: &CloseTab, _window, cx| {
                if let Some(ws) = h9.upgrade() {
                    ws.update(cx, |ws, cx| ws.close_active_tab(cx));
                }
            })
            .on_action(move |_: &OpenTemplateBrowser, _window, cx| {
                if let Some(ws) = h10.upgrade() {
                    ws.update(cx, |ws, cx| {
                        if !ws.enter_dev_mode(cx) {
                            return;
                        }
                        ws.set_active_view(ActiveView::Scripts);
                        ws.show_template_browser(cx);
                        cx.notify();
                    });
                }
            })
            .on_action(move |_: &NewScript, _window, cx| {
                if let Some(ws) = h11.upgrade() {
                    ws.update(cx, |ws, cx| {
                        if !ws.enter_dev_mode(cx) {
                            return;
                        }
                        ws.set_active_view(ActiveView::Scripts);
                        ws.show_script_form(cx);
                        cx.notify();
                    });
                }
            })
            .on_action(move |_: &OpenServerSync, _window, cx| {
                if let Some(ws) = h12.upgrade() {
                    ws.update(cx, |ws, cx| {
                        ws.activate_dev_section(SidebarSection::ServerSync, cx)
                    });
                }
            })
            .on_action(move |_: &OpenSites, _window, cx| {
                if let Some(ws) = h13.upgrade() {
                    ws.update(cx, |ws, cx| {
                        ws.activate_dev_section(SidebarSection::Sites, cx)
                    });
                }
            })
            .on_action(move |_: &OpenFileEditorView, _window, cx| {
                if let Some(ws) = h14.upgrade() {
                    ws.update(cx, |ws, cx| {
                        ws.activate_dev_section(SidebarSection::FileEditor, cx)
                    });
                }
            })
            .on_action(move |action: &ApplyTerminalTheme, _window, cx| {
                if let Some(ws) = h15.upgrade() {
                    let name = action.name.clone();
                    ws.update(cx, |ws, cx| {
                        if ws.enter_dev_mode(cx) {
                            ws.apply_terminal_theme_by_name(&name, cx);
                        }
                    });
                }
            })
            .on_action(move |_: &OpenRecent, _window, cx| {
                if let Some(ws) = h16.upgrade() {
                    ws.update(cx, |ws, cx| {
                        ws.activate_dev_section(SidebarSection::Recent, cx);
                    });
                }
            })
            .on_action(move |_: &OpenAgents, _window, cx| {
                if let Some(ws) = h19.upgrade() {
                    ws.update(cx, |ws, cx| {
                        ws.activate_dev_section(SidebarSection::Agents, cx)
                    });
                }
            })
            .on_action(move |_: &OpenAiAssistant, _window, cx| {
                if let Some(ws) = h17.upgrade() {
                    ws.update(cx, |ws, cx| ws.open_ai_assistant(cx));
                }
            });

        // Apply the application UI font family on the root so it cascades to
        // every child view. Unconditional on purpose: the family is always
        // resolvable, and skipping it is what used to leave hand-rolled
        // elements on GPUI's monospace default while adabraka widgets rendered
        // in Inter. (UI scale is driven by the rem size set at the top of
        // render.)
        root = root.font_family(self.resolved_ui_font_family.clone());

        // Window chrome: clip children to the root so the custom titlebar and
        // status bar follow the window's rounded corners. When floating (not
        // maximized) draw a 1px frame inside the 5px client inset; when
        // maximized the window is edge-to-edge with square corners and no
        // frame. The floating radius follows the 12px outer-chrome radius in
        // the assistant prototype, not the smaller standard Card radius.
        root = root.overflow_hidden();
        if is_maximized {
            root = root.rounded(px(0.0));
        } else {
            root = root
                .rounded(use_theme().tokens.radius_xl)
                .border_1()
                .border_color(ShellDeckColors::border());
        }

        // Sidebar resize drag
        if sidebar_resizing {
            let h_move = handle.clone();
            let h_up = handle.clone();
            root = root
                .cursor_col_resize()
                .on_mouse_move(
                    move |event: &MouseMoveEvent, _window: &mut Window, cx: &mut App| {
                        if let Some(ws) = h_move.upgrade() {
                            ws.update(cx, |ws, cx| {
                                // The pointer is in window space; the panel
                                // starts to the right of the activity rail.
                                let rail = ws.sidebar.read(cx).rail_offset();
                                let new_width = event.position.x.to_f64() as f32 - rail;
                                let clamped = new_width.clamp(180.0, 400.0);
                                ws.sidebar_width = clamped;
                                let total = ws.sidebar.update(cx, |sidebar, _| {
                                    sidebar.set_width(clamped);
                                    sidebar.total_width()
                                });
                                ws.terminal.update(cx, |terminal, _| {
                                    terminal.set_sidebar_width(total);
                                });
                                cx.notify();
                            });
                        }
                    },
                )
                .on_mouse_up(
                    MouseButton::Left,
                    move |_event: &MouseUpEvent, _window: &mut Window, cx: &mut App| {
                        if let Some(ws) = h_up.upgrade() {
                            ws.update(cx, |ws, cx| {
                                ws.sidebar.update(cx, |sidebar, _| {
                                    sidebar.stop_resizing();
                                });
                                cx.notify();
                            });
                        }
                    },
                );
        }

        // Output panel resize drag (scripts or server sync)
        if output_resizing {
            let h_move = handle.clone();
            let h_up = handle.clone();
            let is_sync_panel_drag = self.active_view == ActiveView::ServerSync
                && self.server_sync.read(_cx).is_panel_dragging();
            let is_sync_log_resize = self.active_view == ActiveView::ServerSync
                && self.server_sync.read(_cx).is_log_resizing();
            let is_sync_discovery_resize = self.active_view == ActiveView::ServerSync
                && self.server_sync.read(_cx).is_discovery_resizing();

            if is_sync_panel_drag {
                root = root.cursor_col_resize();
            } else {
                root = root.cursor_row_resize();
            }

            root = root
                .on_mouse_move(
                    move |event: &MouseMoveEvent, window: &mut Window, cx: &mut App| {
                        if let Some(ws) = h_move.upgrade() {
                            ws.update(cx, |ws, cx| {
                                if is_sync_panel_drag {
                                    let window_width = window.viewport_size().width.to_f64() as f32;
                                    let mouse_x = event.position.x.to_f64() as f32;
                                    // Rail + panel: the content area starts to
                                    // the right of both.
                                    let sidebar_w = ws.sidebar.read(cx).total_width();
                                    let content_w = window_width - sidebar_w;
                                    if content_w > 0.0 {
                                        let ratio =
                                            ((mouse_x - sidebar_w) / content_w).clamp(0.2, 0.8);
                                        ws.server_sync.update(cx, |view, _| {
                                            view.panel_ratio = ratio;
                                        });
                                    }
                                } else if is_sync_log_resize {
                                    let window_height =
                                        window.viewport_size().height.to_f64() as f32;
                                    let mouse_y = event.position.y.to_f64() as f32;
                                    let new_height =
                                        (window_height - 28.0 - mouse_y).clamp(60.0, 600.0);
                                    ws.server_sync.update(cx, |view, _| {
                                        view.log_panel_height = new_height;
                                    });
                                } else if is_sync_discovery_resize {
                                    let window_height =
                                        window.viewport_size().height.to_f64() as f32;
                                    let mouse_y = event.position.y.to_f64() as f32;
                                    // Discovery panel grows upward from the bottom of the server panel
                                    let new_height =
                                        (window_height - 28.0 - mouse_y).clamp(60.0, 400.0);
                                    ws.server_sync.update(cx, |view, _| {
                                        if view.source_panel.discovery_resizing {
                                            view.source_panel.discovery_panel_height = new_height;
                                        }
                                        if view.dest_panel.discovery_resizing {
                                            view.dest_panel.discovery_panel_height = new_height;
                                        }
                                    });
                                } else {
                                    let window_height =
                                        window.viewport_size().height.to_f64() as f32;
                                    let mouse_y = event.position.y.to_f64() as f32;
                                    let new_height = window_height - 28.0 - mouse_y;
                                    ws.scripts.update(cx, |editor, _| {
                                        editor.set_output_height(new_height);
                                    });
                                }
                                cx.notify();
                            });
                        }
                    },
                )
                .on_mouse_up(
                    MouseButton::Left,
                    move |_event: &MouseUpEvent, _window: &mut Window, cx: &mut App| {
                        if let Some(ws) = h_up.upgrade() {
                            ws.update(cx, |ws, cx| {
                                ws.scripts.update(cx, |editor, _| {
                                    editor.stop_output_resizing();
                                });
                                ws.server_sync.update(cx, |view, _| {
                                    view.panel_dragging = false;
                                    view.log_panel_resizing = false;
                                    view.stop_discovery_resizing();
                                });
                                cx.notify();
                            });
                        }
                    },
                );
        }

        // Edge resize handling (when not maximized and not already resizing)
        if !is_maximized && !sidebar_resizing && !output_resizing {
            // Window-edge resize hit-testing works in real screen pixels.
            let border = gpui::px(5.0);
            root = root
                .child(
                    canvas(
                        |_bounds, window, _cx| {
                            window.insert_hitbox(
                                Bounds::new(
                                    point(gpui::px(0.0), gpui::px(0.0)),
                                    window.window_bounds().get_bounds().size,
                                ),
                                HitboxBehavior::Normal,
                            )
                        },
                        move |_bounds, hitbox, window, _cx| {
                            let mouse = window.mouse_position();
                            let size = window.window_bounds().get_bounds().size;
                            let Some(edge) = resize_edge(mouse, border, size) else {
                                return;
                            };
                            window.set_cursor_style(
                                match edge {
                                    ResizeEdge::Top | ResizeEdge::Bottom => {
                                        CursorStyle::ResizeUpDown
                                    }
                                    ResizeEdge::Left | ResizeEdge::Right => {
                                        CursorStyle::ResizeLeftRight
                                    }
                                    ResizeEdge::TopLeft | ResizeEdge::BottomRight => {
                                        CursorStyle::ResizeUpLeftDownRight
                                    }
                                    ResizeEdge::TopRight | ResizeEdge::BottomLeft => {
                                        CursorStyle::ResizeUpRightDownLeft
                                    }
                                },
                                &hitbox,
                            );
                        },
                    )
                    .size_full()
                    .absolute(),
                )
                .on_mouse_move(|_e, window, _cx| {
                    window.refresh();
                })
                .on_mouse_down(MouseButton::Left, move |e, window, _cx| {
                    let size = window.window_bounds().get_bounds().size;
                    if let Some(edge) = resize_edge(e.position, gpui::px(5.0), size) {
                        window.start_window_resize(edge);
                    }
                });
        }

        // Custom titlebar with drag area + window controls
        let compact_titlebar = _window.window_bounds().get_bounds().size.width < gpui::px(760.0);
        let titlebar = Self::render_titlebar(
            is_maximized,
            compact_titlebar,
            self.account_menu_open,
            self.mode_menu_open,
            self.app_config.account.clone(),
            self.account_status,
            self.site_menu_open,
            self.app_config.cloud_sync.active_site_label.clone(),
            self.site_directory.is_some(),
            if self.can_switch_mode() {
                Some((self.effective_mode(), self.allowed_modes()))
            } else {
                None
            },
            self.app_config.general.menu_bar_visible,
            self.ai_available_for_current_surface(_cx),
            self.ai_dock_open,
            self.ai_tasks
                .iter()
                .filter(|task| {
                    task.status.is_active()
                        || matches!(task.status, AiTaskStatus::Ready | AiTaskStatus::Pending)
                })
                .count(),
            &handle,
            _cx,
        );

        // The application menu row sits between the titlebar and the content
        // in every mode, including the pre-login welcome screen (where
        // `menu_bar_spec` reduces it to sign-in / quit / zoom / about).
        root = root.child(titlebar);
        if self.app_config.general.menu_bar_visible {
            root = root.child(self.menu_bar.clone());
        }
        let show_status_bar =
            workspace_status_bar_visible(self.show_welcome(), self.effective_mode());
        root = root.child(main_area);
        if show_status_bar {
            root = root.child(self.status_bar.clone());
        }

        // Titlebar account dropdown overlay
        if self.account_menu_open {
            root = root.child(self.render_account_menu(_window, _cx));
        }

        // Compact titlebar mode dropdown.
        if self.mode_menu_open {
            root = root.child(self.render_mode_menu(_window, _cx));
        }

        // Titlebar site-switcher dropdown overlay
        if self.site_menu_open {
            root = root.child(self.render_site_menu(_cx));
        }

        // Sidebar kebab (⋮) row-action menu
        if let Some((conn_id, pos)) = self.sidebar_kebab_menu {
            root = root.child(self.render_sidebar_kebab_menu(conn_id, pos, _cx));
        }

        // User-mode "Mes demandes" sheets: composer + selected-request detail.
        // Both live at workspace root so they slide over the list without
        // pushing it down (their inline predecessors did the pushing).
        if !self.settings_open && matches!(self.effective_mode(), AppMode::User) {
            let sheet = if self.user_new_request_sheet_open {
                Some(
                    self.render_user_new_request_sheet(is_maximized, _cx)
                        .into_any_element(),
                )
            } else if let Some(iss) = self.issue_detail.clone() {
                if self.issue_selected.as_deref() == Some(iss.id.as_str()) {
                    Some(
                        self.render_user_issue_detail_sheet(iss, is_maximized, _window, _cx)
                            .into_any_element(),
                    )
                } else {
                    None
                }
            } else {
                None
            };

            if let Some(sheet) = sheet {
                root = root.child(sheet);
            }
        }

        // Command palette overlay
        root = root.child(self.command_palette.clone());

        if let Some(action) = self.issue_thread_link_action.clone() {
            let workspace = _cx.entity();
            root = root.child(crate::support_view::thread::thread_link_popover(
                action,
                move |cx| {
                    workspace.update(cx, |this, cx| {
                        this.issue_thread_link_action = None;
                        cx.notify();
                    });
                },
            ));
        }

        // Toast notification overlay
        root = root.child(self.toasts.clone());

        // Modal form overlays — render an occluding backdrop at the workspace
        // level so hover/click on elements behind is properly blocked.
        let has_modal = self.connection_form.is_some()
            || self.login_form.is_some()
            || self.onboarding.is_some()
            || self.port_forward_form.is_some()
            || self.script_form.is_some()
            || self.template_browser.is_some()
            || self.variable_prompt.is_some();

        if has_modal {
            let mut modal_layer = div()
                .id("modal-backdrop")
                .occlude()
                .absolute()
                .top_0()
                .left_0()
                .size_full();

            if let Some(ref form) = self.connection_form {
                modal_layer = modal_layer.child(form.clone());
            }
            if let Some(ref form) = self.login_form {
                modal_layer = modal_layer.child(form.clone());
            }
            if let Some(ref form) = self.onboarding {
                modal_layer = modal_layer.child(form.clone());
            }
            if let Some(ref form) = self.port_forward_form {
                modal_layer = modal_layer.child(form.clone());
            }
            if let Some(ref form) = self.script_form {
                modal_layer = modal_layer.child(form.clone());
            }
            if let Some(ref browser) = self.template_browser {
                modal_layer = modal_layer.child(browser.clone());
            }
            if let Some(ref prompt) = self.variable_prompt {
                modal_layer = modal_layer.child(prompt.clone());
            }

            root = root.child(modal_layer);
        }

        // The AI sheets paint *after* the modal layer on purpose.
        //
        // `modal_layer` is a full-window `occlude()` backdrop, so anything
        // added before it is visible but inert. The Script and Tunnel naming
        // sheets are opened from a button *inside* that very modal, so putting
        // them underneath made them unusable: no click reached Accept, Send, or
        // even their close button, and the request was never sent.
        //
        // On top is also the right semantics: the sheet is a child surface of
        // the modal that opened it, so while it is up the modal must wait.
        if let Some(sheet) = &self.ai_sheet {
            // The Sheet backdrop and panel own their native corners directly,
            // following the same proven structure as `render_user_sheet`.
            root = root.child(sheet.clone());
        }
        if let Some(sheet) = &self.ai_workflow_sheet {
            root = root.child(sheet.clone());
        }

        // User-mode delete-issue confirm modal (surfaces outside modal_backdrop
        // since UiDialog provides its own backdrop + occlude).
        if let Some(id) = self.confirm_issue_delete.clone() {
            root = root.child(self.render_delete_issue_modal(id, _cx));
        }
        if let Some((issue_id, attachment_id)) = self.confirm_attachment_delete.clone() {
            root = root.child(self.render_delete_attachment_modal(issue_id, attachment_id, _cx));
        }

        if let Some(lightbox) = &self.issue_attachment_lightbox {
            root = root.child(lightbox.clone());
        }

        if let Some(annotator) = &self.issue_capture_annotator {
            root = root.child(annotator.clone());
        }

        if let Some(plan) = self.ai_action_confirmation.clone() {
            let workspace = _cx.entity().downgrade();
            let close_workspace = workspace.clone();
            root = root.child(render_ai_action_dialog(
                plan,
                move |cx| {
                    if let Some(workspace) = close_workspace.upgrade() {
                        workspace.update(cx, |workspace, cx| {
                            workspace.cancel_ai_action_confirmation(cx);
                        });
                    }
                },
                move |cx| {
                    if let Some(workspace) = workspace.upgrade() {
                        workspace.update(cx, |workspace, cx| workspace.confirm_ai_action(cx));
                    }
                },
            ));
        }

        if let Some(transition) = self.mode_transition {
            root = root.child(self.render_mode_transition_loader(transition, _cx));
        }

        // The post-login transition is intentionally last: it covers window
        // chrome, toasts and first-run onboarding until the initial sync has
        // produced a coherent signed-in workspace.
        if let Some(splash) = &self.post_login_splash {
            root = root.child(self.render_post_login_splash(splash, _cx));
        }

        root
    }
}

#[cfg(test)]
mod tests {
    use super::{
        mode_transition_overlay_opacity, workspace_status_bar_visible, AppMode,
        MODE_TRANSITION_IN_MS, MODE_TRANSITION_LOADING_MS, MODE_TRANSITION_LOADING_REPEAT_MS,
        MODE_TRANSITION_OUT_MS, MODE_TRANSITION_TOTAL_MS,
    };

    /// SDTEST-1703 — la courbe suit la durée réelle de la transition.
    ///
    /// Le palier était constant à 2,54 s, soit trois secondes pleines à chaque
    /// aller-retour Support ↔ Dev alors que rien ne charge : les entités Dev
    /// sont masquées, pas détruites. Il est désormais long à la première
    /// entrée dans un mode et court ensuite — la courbe doit donc rester
    /// correcte pour les deux, sinon le voile disparaît avant la fin ou reste
    /// après.
    #[test]
    fn mode_transition_loader_fades_in_holds_then_fades_out() {
        for loading_ms in [
            MODE_TRANSITION_LOADING_MS,
            MODE_TRANSITION_LOADING_REPEAT_MS,
        ] {
            let total = (MODE_TRANSITION_OUT_MS + loading_ms + MODE_TRANSITION_IN_MS) as f32;
            let hold_delta = (MODE_TRANSITION_OUT_MS + loading_ms / 2) as f32 / total;

            assert_eq!(mode_transition_overlay_opacity(0.0, loading_ms), 0.0);
            assert_eq!(mode_transition_overlay_opacity(hold_delta, loading_ms), 1.0);
            assert_eq!(mode_transition_overlay_opacity(1.0, loading_ms), 0.0);
        }
        assert_eq!(MODE_TRANSITION_TOTAL_MS, 3_000);
        assert!(
            MODE_TRANSITION_LOADING_REPEAT_MS < MODE_TRANSITION_LOADING_MS,
            "un retour dans un mode déjà ouvert ne doit pas coûter plus cher \
             que la première entrée"
        );
    }

    // SDTEST-1616
    #[test]
    fn workspace_status_bar_is_exclusive_to_authenticated_dev_mode() {
        assert!(!workspace_status_bar_visible(false, AppMode::User));
        assert!(!workspace_status_bar_visible(false, AppMode::Support));
        assert!(workspace_status_bar_visible(false, AppMode::Dev));
        assert!(!workspace_status_bar_visible(true, AppMode::Dev));
    }
}
