use super::*;

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
        // Jean/Fleet availability, AI); rebuilding it here keeps it honest
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
            main_area = main_area.child(self.render_welcome_screen(_cx));
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
                    main_area = main_area.child(self.render_user_home(_cx));
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
                        ActiveView::Scripts => content = content.child(self.scripts.clone()),
                        ActiveView::PortForwards => {
                            content = content.child(self.port_forwards.clone())
                        }
                        ActiveView::ServerSync => content = content.child(self.server_sync.clone()),
                        ActiveView::Sites => content = content.child(self.sites.clone()),
                        ActiveView::Recent => content = content.child(self.recent.clone()),
                        ActiveView::FileEditor => content = content.child(self.file_editor.clone()),
                        ActiveView::JeanConsole => content = content.child(self.jean_view.clone()),
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
                        ws.set_active_view(ActiveView::Scripts);
                        ws.show_template_browser(cx);
                        cx.notify();
                    });
                }
            })
            .on_action(move |_: &NewScript, _window, cx| {
                if let Some(ws) = h11.upgrade() {
                    ws.update(cx, |ws, cx| {
                        ws.set_active_view(ActiveView::Scripts);
                        ws.show_script_form(cx);
                        cx.notify();
                    });
                }
            })
            .on_action(move |_: &OpenServerSync, _window, cx| {
                if let Some(ws) = h12.upgrade() {
                    ws.update(cx, |ws, cx| {
                        ws.set_active_view(ActiveView::ServerSync);
                        cx.notify();
                    });
                }
            })
            .on_action(move |_: &OpenSites, _window, cx| {
                if let Some(ws) = h13.upgrade() {
                    ws.update(cx, |ws, cx| {
                        ws.set_active_view(ActiveView::Sites);
                        cx.notify();
                    });
                }
            })
            .on_action(move |_: &OpenFileEditorView, _window, cx| {
                if let Some(ws) = h14.upgrade() {
                    ws.update(cx, |ws, cx| {
                        ws.set_active_view(ActiveView::FileEditor);
                        cx.notify();
                    });
                }
            })
            .on_action(move |action: &ApplyTerminalTheme, _window, cx| {
                if let Some(ws) = h15.upgrade() {
                    let name = action.name.clone();
                    ws.update(cx, |ws, cx| {
                        ws.apply_terminal_theme_by_name(&name, cx);
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
            .on_action(move |_: &OpenAiAssistant, _window, cx| {
                if let Some(ws) = h17.upgrade() {
                    ws.update(cx, |ws, cx| ws.open_ai_assistant(cx));
                }
            });

        // Apply the configured application UI font family on the root so it
        // cascades to every child view; "System Default" leaves GPUI's
        // default font untouched. (UI scale is driven by the rem size set at
        // the top of render.)
        if self.ui_font_family != "System Default" {
            root = root.font_family(self.ui_font_family.clone());
        }

        // Window chrome: clip children to the root so the custom titlebar and
        // status bar follow the window's rounded corners. When floating (not
        // maximized) draw a 1px frame inside the 5px client inset; when
        // maximized the window is edge-to-edge with square corners and no
        // frame. The floating radius matches the standard Card component.
        root = root.overflow_hidden();
        if is_maximized {
            root = root.rounded(px(0.0));
        } else {
            root = root
                .rounded(use_theme().tokens.radius_lg)
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
        let titlebar = Self::render_titlebar(
            is_maximized,
            self.theme_menu_open,
            self.account_menu_open,
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
            self.ui_font_size,
            self.ai_available_for_current_surface(_cx),
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
        root = root.child(main_area).child(self.status_bar.clone());

        // Titlebar theme-switcher dropdown overlay
        if self.theme_menu_open {
            root = root.child(self.render_theme_menu(_cx));
        }

        // Titlebar account dropdown overlay
        if self.account_menu_open {
            root = root.child(self.render_account_menu(_cx));
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
            if self.user_new_request_sheet_open {
                root = root.child(self.render_user_new_request_sheet(_cx));
            } else if let Some(iss) = self.issue_detail.clone() {
                if self.issue_selected.as_deref() == Some(iss.id.as_str()) {
                    root = root.child(self.render_user_issue_detail_sheet(iss, _cx));
                }
            }
        }

        // Command palette overlay
        root = root.child(self.command_palette.clone());

        if let Some(sheet) = &self.ai_sheet {
            root = root.child(sheet.clone());
        }
        if let Some(sheet) = &self.ai_workflow_sheet {
            root = root.child(sheet.clone());
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

        // The post-login transition is intentionally last: it covers window
        // chrome, toasts and first-run onboarding until the initial sync has
        // produced a coherent signed-in workspace.
        if let Some(splash) = &self.post_login_splash {
            root = root.child(self.render_post_login_splash(splash));
        }

        root
    }
}
