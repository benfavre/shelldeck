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
        let signed_in = !allowed_modes.is_empty();
        let mut actions = vec![
            PaletteAction::new(
                t!("palette.quit").to_string(),
                Some("Ctrl+Q"),
                "x",
                Box::new(Quit),
            ),
            // Recovery path must exist in every mode, including logged out:
            // the menu row cannot be the only place that can restore itself.
            PaletteAction::new(
                t!("menu.view.menu_bar").to_string(),
                Some("Ctrl+Shift+M"),
                "menu",
                Box::new(ToggleMenuBar),
            ),
        ];

        if signed_in {
            actions.extend([
                PaletteAction::new(
                    t!("palette.open_settings").to_string(),
                    Some("Ctrl+,"),
                    "settings",
                    Box::new(OpenSettings),
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
            ]);
        } else {
            actions.push(PaletteAction::new(
                t!("menu.account.sign_in").to_string(),
                None,
                "user-check",
                Box::new(OpenLogin),
            ));
        }

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
                    t!("palette.open_agents").to_string(),
                    None,
                    "bot",
                    Box::new(OpenAgents),
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
                    t!("palette.monique_open").to_string(),
                    None,
                    "cpu",
                    Box::new(OpenMoniqueConsole),
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
                    Box::new(ToggleMoniqueRuntime),
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

        if signed_in && ai_configured {
            actions.push(PaletteAction::new(
                t!("palette.open_ai").to_string(),
                Some("Ctrl+Shift+K"),
                "zap",
                Box::new(OpenAiAssistant),
            ));
        }
        // Hidden when the Clippy surface is disallowed — never display a
        // command the caller cannot reach (`.agents/roles.md` spirit).
        if signed_in && clippy_configured {
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
                    if self.enter_dev_mode(cx) {
                        self.apply_terminal_theme_by_name(&theme.name, cx);
                    }
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
        // The standalone palette and global shortcuts can dispatch actions
        // without the main surface being rendered. Keep the execution gate
        // here; filtering the visible action list is only presentation.
        if !self.signed_in()
            && !action.as_any().is::<Quit>()
            && !action.as_any().is::<ToggleMenuBar>()
            && !action.as_any().is::<OpenLogin>()
            && !action.as_any().is::<ApplyAppTheme>()
        {
            return;
        }
        if action.as_any().is::<NewTerminal>() {
            self.open_new_terminal(cx);
        } else if action.as_any().is::<ToggleSidebar>() {
            self.toggle_sidebar(cx);
        } else if action.as_any().is::<ToggleMenuBar>() {
            self.toggle_menu_bar(cx);
        } else if action.as_any().is::<OpenLogin>() {
            self.show_login_form(cx);
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
            if !self.enter_dev_mode(cx) {
                return;
            }
            self.set_active_view(ActiveView::Scripts);
            self.show_template_browser(cx);
        } else if action.as_any().is::<NewScript>() {
            if !self.enter_dev_mode(cx) {
                return;
            }
            self.set_active_view(ActiveView::Scripts);
            self.show_script_form(cx);
        } else if action.as_any().is::<OpenServerSync>() {
            self.activate_dev_section(SidebarSection::ServerSync, cx);
        } else if action.as_any().is::<OpenAgents>() {
            self.activate_dev_section(SidebarSection::Agents, cx);
        } else if action.as_any().is::<OpenSites>() {
            self.activate_dev_section(SidebarSection::Sites, cx);
        } else if action.as_any().is::<OpenRecent>() {
            self.activate_dev_section(SidebarSection::Recent, cx);
        } else if action.as_any().is::<OpenFileEditorView>() {
            self.activate_dev_section(SidebarSection::FileEditor, cx);
        } else if action.as_any().is::<CloudSyncNow>() {
            self.cloud_sync_now(cx);
        } else if action.as_any().is::<SwitchSite>() {
            self.open_site_switcher(cx);
        } else if let Some(area) = action.as_any().downcast_ref::<OpenManageArea>() {
            self.open_manage_area(area.path.clone(), cx);
        } else if let Some(mode) = action.as_any().downcast_ref::<SetAppMode>() {
            self.set_mode(mode.mode, cx);
        } else if action.as_any().is::<OpenMoniqueConsole>() {
            self.open_monique_console(cx);
        } else if action.as_any().is::<OpenFleet>() {
            self.open_fleet(cx);
        } else if action.as_any().is::<ToggleMoniqueRuntime>() {
            self.toggle_monique_runtime(cx);
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

#[cfg(test)]
mod tests {
    use super::{
        AppMode, ApplyTerminalTheme, CloudSyncNow, NewRequest, NewScript, NewTerminal, OpenAgents,
        OpenAiAssistant, OpenBextCloud, OpenClippy, OpenFileEditorView, OpenFleet, OpenLogin,
        OpenMoniqueConsole, OpenRecent, OpenServerSync, OpenSettings, OpenSites,
        OpenSupportRequests, OpenTemplateBrowser, PaletteAction, Quit, SetAppMode, SwitchSite,
        ToggleMenuBar, ToggleMoniqueRuntime, Workspace,
    };
    use gpui::Action;

    fn contains_action<T: Action + 'static>(actions: &[PaletteAction]) -> bool {
        actions
            .iter()
            .any(|candidate| candidate.action.as_any().is::<T>())
    }

    /// The Dev block of the palette. Every one of these routes into a
    /// `enter_dev_mode` gate at execution time; none may be *offered* outside
    /// Dev.
    fn assert_no_dev_actions(actions: &[PaletteAction], context: &str) {
        assert!(!contains_action::<NewTerminal>(actions), "{context}");
        assert!(!contains_action::<OpenFileEditorView>(actions), "{context}");
        assert!(!contains_action::<OpenServerSync>(actions), "{context}");
        assert!(!contains_action::<OpenAgents>(actions), "{context}");
        assert!(!contains_action::<OpenSites>(actions), "{context}");
        assert!(!contains_action::<OpenRecent>(actions), "{context}");
        assert!(!contains_action::<NewScript>(actions), "{context}");
        assert!(
            !contains_action::<OpenTemplateBrowser>(actions),
            "{context}"
        );
        assert!(!contains_action::<OpenMoniqueConsole>(actions), "{context}");
        assert!(!contains_action::<OpenFleet>(actions), "{context}");
        assert!(!contains_action::<OpenBextCloud>(actions), "{context}");
        assert!(
            !contains_action::<ToggleMoniqueRuntime>(actions),
            "{context}"
        );
    }

    // SDTEST-1377 — a regular account is offered nothing beyond User.
    //
    // The tier table itself is pinned by SDTEST-184 in `shelldeck-core`; what
    // this asserts is the consequence the user actually sees, which is the
    // rule from `.agents/roles.md`: never display a command the caller cannot
    // reach.
    #[test]
    fn a_regular_account_is_offered_no_support_or_dev_command() {
        let actions = Workspace::base_palette_actions(&[AppMode::User], AppMode::User, true, true);

        assert!(contains_action::<OpenSettings>(&actions));
        assert!(contains_action::<NewRequest>(&actions));
        assert!(!contains_action::<OpenSupportRequests>(&actions));
        assert_no_dev_actions(&actions, "regular account");

        // A single allowed mode means there is nothing to switch between, so
        // the mode rows must not appear either.
        assert!(!contains_action::<SetAppMode>(&actions));
    }

    // SDTEST-1377 — Inklura support reaches triage, never Dev.
    #[test]
    fn inklura_support_reaches_triage_but_never_dev() {
        let allowed: &'static [AppMode] = &[AppMode::User, AppMode::Support];

        let in_support = Workspace::base_palette_actions(allowed, AppMode::Support, true, true);
        assert!(contains_action::<OpenSupportRequests>(&in_support));
        assert!(contains_action::<SetAppMode>(&in_support));
        assert_no_dev_actions(&in_support, "support account in Support mode");

        // Triage commands belong to the Support surface, not to the account.
        let in_user = Workspace::base_palette_actions(allowed, AppMode::User, true, true);
        assert!(!contains_action::<OpenSupportRequests>(&in_user));
        assert_no_dev_actions(&in_user, "support account in User mode");
    }

    // SDTEST-1377 — Dev commands follow the *surface*, not the privilege.
    //
    // This is the half that is easy to get wrong: a super-admin standing in
    // User mode must not be offered terminals and Monique. Gating those on
    // `allowed_modes.contains(Dev)` alone would leak the whole Dev block into
    // the customer-facing surface.
    #[test]
    fn a_super_admin_gets_dev_commands_only_while_in_dev_mode() {
        let allowed: &'static [AppMode] = &[AppMode::User, AppMode::Support, AppMode::Dev];

        let in_dev = Workspace::base_palette_actions(allowed, AppMode::Dev, true, true);
        assert!(contains_action::<NewTerminal>(&in_dev));
        assert!(contains_action::<OpenAgents>(&in_dev));
        assert!(contains_action::<OpenMoniqueConsole>(&in_dev));
        assert!(contains_action::<OpenBextCloud>(&in_dev));
        assert!(contains_action::<ApplyTerminalTheme>(&in_dev));

        assert_no_dev_actions(
            &Workspace::base_palette_actions(allowed, AppMode::User, true, true),
            "super-admin in User mode",
        );
        assert_no_dev_actions(
            &Workspace::base_palette_actions(allowed, AppMode::Support, true, true),
            "super-admin in Support mode",
        );
    }

    // SDTEST-1377 — an unusable AI command is worse than an absent one.
    #[test]
    fn ai_commands_appear_only_when_their_backend_is_configured() {
        let allowed: &'static [AppMode] = &[AppMode::User];

        let configured = Workspace::base_palette_actions(allowed, AppMode::User, true, true);
        assert!(contains_action::<OpenAiAssistant>(&configured));
        assert!(contains_action::<OpenClippy>(&configured));

        let unconfigured = Workspace::base_palette_actions(allowed, AppMode::User, false, false);
        assert!(!contains_action::<OpenAiAssistant>(&unconfigured));
        assert!(!contains_action::<OpenClippy>(&unconfigured));
    }

    // SDTEST-1602 — the standalone palette is available before login, but it
    // must be a login/recovery surface rather than a guest-mode back door.
    #[test]
    fn logged_out_palette_contains_only_public_or_recovery_actions() {
        let actions = Workspace::base_palette_actions(&[], AppMode::User, true, true);

        assert!(contains_action::<OpenLogin>(&actions));
        assert!(contains_action::<ToggleMenuBar>(&actions));
        assert!(contains_action::<Quit>(&actions));
        assert!(!contains_action::<OpenSettings>(&actions));
        assert!(!contains_action::<CloudSyncNow>(&actions));
        assert!(!contains_action::<SwitchSite>(&actions));
        assert!(!contains_action::<NewRequest>(&actions));
        assert!(!contains_action::<OpenAiAssistant>(&actions));
        assert!(!contains_action::<OpenClippy>(&actions));
        assert!(!contains_action::<NewTerminal>(&actions));
        assert!(!contains_action::<OpenFileEditorView>(&actions));
    }
}
