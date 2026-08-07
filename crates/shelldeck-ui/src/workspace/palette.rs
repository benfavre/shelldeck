use super::*;

impl Workspace {
    /// The fixed command-palette entries (everything except the runtime-dependent
    /// manage-area entries, which [`refresh_command_palette`] appends).
    ///
    /// The palette mirrors the current surface: personal actions are global,
    /// while Support and Dev actions only exist when that surface is active.
    /// Mode rows are built from the caller's exact allowed set.
    pub(super) fn base_palette_actions(
        allowed_modes: &'static [AppMode],
        current_mode: AppMode,
        ai_configured: bool,
        clippy_configured: bool,
    ) -> Vec<PaletteAction> {
        let mut actions = vec![
            PaletteAction::new(
                t!("palette.open_settings").to_string(),
                Some("Ctrl+,"),
                "settings",
                Box::new(OpenSettings),
            ),
            PaletteAction::new(
                t!("palette.quit").to_string(),
                Some("Ctrl+Q"),
                "x",
                Box::new(Quit),
            ),
            PaletteAction::new(
                t!("palette.cloud_sync_now").to_string(),
                None,
                "refresh-cw",
                Box::new(CloudSyncNow),
            ),
            PaletteAction::new(
                t!("palette.switch_site").to_string(),
                None,
                "globe",
                Box::new(SwitchSite),
            ),
            PaletteAction::new(
                t!("palette.new_request").to_string(),
                None,
                "plus",
                Box::new(NewRequest),
            ),
        ];

        if current_mode == AppMode::Support && allowed_modes.contains(&AppMode::Support) {
            actions.push(PaletteAction::new(
                t!("palette.support_requests").to_string(),
                None,
                "inbox",
                Box::new(OpenSupportRequests),
            ));
        }

        if current_mode == AppMode::Dev && allowed_modes.contains(&AppMode::Dev) {
            actions.extend([
                PaletteAction::new(
                    t!("palette.new_terminal").to_string(),
                    Some("Ctrl+T"),
                    "terminal",
                    Box::new(NewTerminal),
                ),
                PaletteAction::new(
                    t!("palette.toggle_sidebar").to_string(),
                    Some("Ctrl+B"),
                    "chevron-left",
                    Box::new(ToggleSidebar),
                ),
                PaletteAction::new(
                    t!("palette.close_tab").to_string(),
                    Some("Ctrl+W"),
                    "x",
                    Box::new(CloseTab),
                ),
                PaletteAction::new(
                    t!("palette.next_tab").to_string(),
                    Some("Ctrl+Tab"),
                    "chevron-right",
                    Box::new(NextTab),
                ),
                PaletteAction::new(
                    t!("palette.prev_tab").to_string(),
                    Some("Ctrl+Shift+Tab"),
                    "chevron-left",
                    Box::new(PrevTab),
                ),
                PaletteAction::new(
                    t!("palette.browse_templates").to_string(),
                    None,
                    "scroll-text",
                    Box::new(OpenTemplateBrowser),
                ),
                PaletteAction::new(
                    t!("palette.new_script").to_string(),
                    None,
                    "plus",
                    Box::new(NewScript),
                ),
                PaletteAction::new(
                    t!("palette.open_server_sync").to_string(),
                    None,
                    "refresh-cw",
                    Box::new(OpenServerSync),
                ),
                PaletteAction::new(
                    t!("palette.open_sites").to_string(),
                    None,
                    "globe",
                    Box::new(OpenSites),
                ),
                PaletteAction::new(
                    t!("palette.open_recent").to_string(),
                    None,
                    "activity",
                    Box::new(OpenRecent),
                ),
                PaletteAction::new(
                    t!("palette.open_file_editor").to_string(),
                    Some("Ctrl+E"),
                    "pencil",
                    Box::new(OpenFileEditorView),
                ),
                PaletteAction::new(
                    t!("palette.jean_open").to_string(),
                    None,
                    "cpu",
                    Box::new(OpenJeanConsole),
                ),
                PaletteAction::new(
                    t!("palette.jean_pause").to_string(),
                    None,
                    "clock",
                    Box::new(JeanTogglePause),
                ),
                PaletteAction::new(
                    t!("palette.fleet_open").to_string(),
                    None,
                    "box",
                    Box::new(OpenFleet),
                ),
                PaletteAction::new(
                    t!("palette.fleet_runtime").to_string(),
                    None,
                    "cpu",
                    Box::new(ToggleJeanRuntime),
                ),
                PaletteAction::new(
                    t!("palette.bext_open").to_string(),
                    None,
                    "cloud",
                    Box::new(OpenBextCloud),
                ),
                PaletteAction::new(
                    t!("palette.bext_connect").to_string(),
                    None,
                    "key",
                    Box::new(ConnectBextCloud),
                ),
            ]);
        }

        if ai_configured {
            actions.push(PaletteAction::new(
                t!("palette.open_ai").to_string(),
                Some("Ctrl+Shift+K"),
                "zap",
                Box::new(OpenAiAssistant),
            ));
        }
        // Hidden when the Clippy surface is disallowed — never display a
        // command the caller cannot reach (`.agents/roles.md` spirit).
        if clippy_configured {
            actions.push(PaletteAction::new(
                t!("palette.open_clippy").to_string(),
                None,
                "clipboard-paste",
                Box::new(OpenClippy),
            ));
        }
        if allowed_modes.len() > 1 {
            for &m in allowed_modes {
                actions.push(PaletteAction::new(
                    t!("palette.mode", mode = m.label()).to_string(),
                    None,
                    "shield",
                    Box::new(SetAppMode { mode: m }),
                ));
            }
        }
        for pref in ThemePreference::all() {
            actions.push(PaletteAction::new(
                t!("palette.theme", name = pref.display_name()).to_string(),
                None,
                "settings",
                Box::new(ApplyAppTheme { pref: pref.clone() }),
            ));
        }
        if current_mode == AppMode::Dev && allowed_modes.contains(&AppMode::Dev) {
            for theme in TerminalTheme::builtins() {
                actions.push(PaletteAction::new(
                    t!("palette.terminal_theme", name = theme.name).to_string(),
                    None,
                    "terminal",
                    Box::new(ApplyTerminalTheme { name: theme.name }),
                ));
            }
        }
        actions
    }

    /// Rebuild the palette entries, appending "Site actif : <area>" commands for
    /// the active site's manage areas. Called when the site directory loads or
    /// the active site changes.
    pub(super) fn refresh_command_palette(&mut self, cx: &mut Context<Self>) {
        let mut actions = Self::base_palette_actions(
            self.allowed_modes(),
            self.effective_mode(),
            self.ai_available_for_current_surface(cx),
            self.ai_backend_available() && self.app_config.ai.allows(AiSurface::Clippy),
        );
        if let (Some(site), Some(dir)) = (self.active_site_info(), self.site_directory.as_ref()) {
            let label = site.display_label();
            for area in &dir.areas {
                actions.push(PaletteAction::new(
                    t!(
                        "palette.active_site",
                        site = label,
                        area = area.label.as_str()
                    )
                    .to_string(),
                    None,
                    "external-link",
                    Box::new(OpenManageArea {
                        path: area.path.clone(),
                    }),
                ));
            }
        }
        self.command_palette.update(cx, |palette, _| {
            palette.set_actions(actions.clone());
        });
        self.companion_command_palette.update(cx, |palette, _| {
            palette.set_actions(actions);
        });
    }

    pub(super) fn handle_command_palette_event(
        &mut self,
        event: &CommandPaletteEvent,
        cx: &mut Context<Self>,
    ) {
        match event {
            CommandPaletteEvent::SelectionPreviewed(action) => {
                self.preview_palette_action(action.as_ref(), cx);
            }
            CommandPaletteEvent::ActionSelected(action) => {
                if let Some(theme) = action.as_any().downcast_ref::<ApplyAppTheme>() {
                    self.revert_terminal_theme_preview(cx);
                    self.commit_theme_preview(theme.pref.clone(), cx);
                } else if let Some(theme) = action.as_any().downcast_ref::<ApplyTerminalTheme>() {
                    self.revert_theme_preview(cx);
                    self.terminal_theme_before_preview = None;
                    self.apply_terminal_theme_by_name(&theme.name, cx);
                } else {
                    self.revert_theme_preview(cx);
                    self.revert_terminal_theme_preview(cx);
                    self.execute_palette_action(action.as_ref(), cx);
                }
            }
            CommandPaletteEvent::Dismissed => {
                self.revert_theme_preview(cx);
                self.revert_terminal_theme_preview(cx);
                cx.notify();
            }
        }
    }

    pub(super) fn execute_palette_action(&mut self, action: &dyn Action, cx: &mut Context<Self>) {
        if action.as_any().is::<NewTerminal>() {
            self.open_new_terminal(cx);
        } else if action.as_any().is::<ToggleSidebar>() {
            self.toggle_sidebar(cx);
        } else if action.as_any().is::<OpenSettings>() {
            self.open_settings(cx);
        } else if action.as_any().is::<CloseTab>() {
            self.close_active_tab(cx);
        } else if action.as_any().is::<NextTab>() {
            self.next_tab(cx);
        } else if action.as_any().is::<PrevTab>() {
            self.prev_tab(cx);
        } else if action.as_any().is::<Quit>() {
            self.shutdown(cx);
            cx.quit();
        } else if action.as_any().is::<OpenTemplateBrowser>() {
            self.set_active_view(ActiveView::Scripts);
            self.show_template_browser(cx);
        } else if action.as_any().is::<NewScript>() {
            self.set_active_view(ActiveView::Scripts);
            self.show_script_form(cx);
        } else if action.as_any().is::<OpenServerSync>() {
            self.set_active_view(ActiveView::ServerSync);
            cx.notify();
        } else if action.as_any().is::<OpenSites>() {
            self.set_active_view(ActiveView::Sites);
            cx.notify();
        } else if action.as_any().is::<OpenRecent>() {
            self.activate_dev_section(SidebarSection::Recent, cx);
        } else if action.as_any().is::<OpenFileEditorView>() {
            self.set_active_view(ActiveView::FileEditor);
            cx.notify();
        } else if action.as_any().is::<CloudSyncNow>() {
            self.cloud_sync_now(cx);
        } else if action.as_any().is::<SwitchSite>() {
            self.open_site_switcher(cx);
        } else if let Some(area) = action.as_any().downcast_ref::<OpenManageArea>() {
            self.open_manage_area(area.path.clone(), cx);
        } else if let Some(mode) = action.as_any().downcast_ref::<SetAppMode>() {
            self.set_mode(mode.mode, cx);
        } else if action.as_any().is::<OpenJeanConsole>() {
            self.open_jean_console(cx);
        } else if action.as_any().is::<JeanTogglePause>() {
            self.jean_toggle_pause(cx);
        } else if action.as_any().is::<OpenFleet>() {
            self.open_fleet(cx);
        } else if action.as_any().is::<ToggleJeanRuntime>() {
            self.toggle_jean_runtime(cx);
        } else if action.as_any().is::<NewRequest>() {
            self.open_new_request(cx);
        } else if action.as_any().is::<OpenSupportRequests>() {
            self.open_support_requests(cx);
        } else if action.as_any().is::<OpenBextCloud>() {
            self.open_bext_cloud(cx);
        } else if action.as_any().is::<ConnectBextCloud>() {
            self.connect_bext_cloud_action(cx);
        } else if action.as_any().is::<OpenAiAssistant>() {
            self.open_ai_assistant(cx);
        } else if action.as_any().is::<OpenClippy>() {
            self.open_ai_clippy(cx);
        } else {
            cx.dispatch_action(action);
        }
    }
}
