use super::*;

impl Workspace {
    // --- Application menu bar ---

    /// Rebuild the menu row from current state and hand it to the adabraka
    /// `MenuBar`. Called every render: the spec is a cheap pure function and
    /// the alternative — invalidating on each of the a dozen inputs it reads
    /// (mode, sign-in, sidebar, Monique/Fleet availability, AI config) — is a
    /// standing source of stale-menu bugs.
    pub(super) fn rebuild_menu_bar(&mut self, cx: &mut Context<Self>) {
        use crate::menu_bar::{menu_bar_spec, MenuBarContext, MenuEntry};

        let ctx = MenuBarContext {
            signed_in: self.signed_in(),
            mode: self.effective_mode(),
            dev_capable: self
                .app_config
                .account
                .as_ref()
                .is_some_and(|a| a.is_superadmin),
            sidebar_visible: self.sidebar_visible,
            menu_bar_visible: self.app_config.general.menu_bar_visible,
            has_monique: self.has_monique(),
            has_fleet: self.platform_connection().is_some() || self.fleet_snapshot.is_some(),
            ai_configured: self.ai_available_for_current_surface(cx),
        };

        let items = menu_bar_spec(ctx)
            .into_iter()
            .map(|menu| {
                let entries = menu
                    .entries
                    .into_iter()
                    .map(|entry| match entry {
                        MenuEntry::Separator => AdabrakaMenuItem::separator(),
                        MenuEntry::Command {
                            id,
                            label,
                            command,
                            shortcut,
                            icon,
                            checked,
                        } => {
                            let mut item = match checked {
                                // A toggle renders adabraka's check column; a
                                // plain command leaves it blank but still
                                // reserves the width, so labels stay aligned.
                                Some(value) => AdabrakaMenuItem::checkbox(id, label, value),
                                None => AdabrakaMenuItem::new(id, label),
                            };
                            if let Some(slug) = icon {
                                item = item.with_icon(IconSource::from(slug));
                            }
                            if let Some(keys) = shortcut {
                                item = item.with_shortcut(keys);
                            }
                            let handle = cx.entity().downgrade();
                            item.on_click(move |window, cx| {
                                if let Some(ws) = handle.upgrade() {
                                    ws.update(cx, |ws, cx| {
                                        ws.execute_menu_command(command, window, cx);
                                    });
                                }
                            })
                        }
                    })
                    .collect::<Vec<_>>();
                MenuBarItem::new(menu.id, menu.label).with_items(entries)
            })
            .collect::<Vec<_>>();

        self.menu_bar.update(cx, |bar, _| {
            // Preserve the open menu across the rebuild: `set_items` closes
            // it, and this runs on every render — including the render the
            // click that *opened* a menu triggered.
            let open = bar.open_index();
            bar.set_items(items);
            bar.set_open_index(open);
        });
    }

    /// Run one menu command. Anything with an existing `actions!` entry goes
    /// through `execute_palette_action` so the menu bar, the palette and the
    /// keybindings stay one code path.
    pub(super) fn execute_menu_command(
        &mut self,
        command: crate::menu_bar::MenuCommand,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use crate::menu_bar::MenuCommand as Cmd;
        use crate::terminal_view::{
            ClearTerminal, CopySelection, PasteClipboard, SplitHorizontal, SplitVertical,
            ToggleSearch, ZoomIn, ZoomOut, ZoomReset,
        };

        // A stale menu click must not outlive logout. The menu specification
        // hides authenticated commands, but authorization belongs here too.
        if !self.signed_in()
            && !matches!(
                command,
                Cmd::SignIn
                    | Cmd::Quit
                    | Cmd::CommandPalette
                    | Cmd::ToggleMenuBar
                    | Cmd::UiZoomIn
                    | Cmd::UiZoomOut
                    | Cmd::UiZoomReset
                    | Cmd::Documentation
            )
        {
            return;
        }

        match command {
            Cmd::QuickConnect => self.execute_palette_action(&OpenQuickConnect, cx),
            Cmd::NewTerminal => self.execute_palette_action(&NewTerminal, cx),
            Cmd::NewScript => self.execute_palette_action(&NewScript, cx),
            Cmd::NewRequest => self.execute_palette_action(&NewRequest, cx),
            Cmd::SyncNow => self.execute_palette_action(&CloudSyncNow, cx),
            Cmd::OpenCompanionSettings => self.open_companion_settings(cx),
            Cmd::OpenSettings => self.open_settings(cx),
            Cmd::Quit => {
                if self.confirm_window_close(cx) {
                    self.shutdown(cx);
                    cx.quit();
                }
            }

            // Terminal-owned actions are dispatched into the focus path
            // rather than handled here: only the focused pane knows what the
            // selection or the search state is.
            Cmd::Copy => window.dispatch_action(Box::new(CopySelection), cx),
            Cmd::Paste => window.dispatch_action(Box::new(PasteClipboard), cx),
            Cmd::Find => window.dispatch_action(Box::new(ToggleSearch), cx),
            Cmd::ClearTerminal => window.dispatch_action(Box::new(ClearTerminal), cx),
            Cmd::SplitHorizontal => window.dispatch_action(Box::new(SplitHorizontal), cx),
            Cmd::SplitVertical => window.dispatch_action(Box::new(SplitVertical), cx),
            Cmd::TerminalZoomIn => window.dispatch_action(Box::new(ZoomIn), cx),
            Cmd::TerminalZoomOut => window.dispatch_action(Box::new(ZoomOut), cx),
            Cmd::TerminalZoomReset => window.dispatch_action(Box::new(ZoomReset), cx),

            Cmd::CommandPalette => self.toggle_command_palette(window, cx),

            Cmd::ToggleSidebar => self.toggle_sidebar(cx),
            Cmd::ToggleMenuBar => self.toggle_menu_bar(cx),
            Cmd::UiZoomIn => self
                .settings
                .update(cx, |settings, cx| settings.adjust_ui_font_size(1.0, cx)),
            Cmd::UiZoomOut => self
                .settings
                .update(cx, |settings, cx| settings.adjust_ui_font_size(-1.0, cx)),
            Cmd::UiZoomReset => {
                let delta = crate::scale::BASELINE_FONT_SIZE - self.ui_font_size;
                self.settings
                    .update(cx, |settings, cx| settings.adjust_ui_font_size(delta, cx));
            }

            Cmd::GoDashboard => self.activate_dev_section(SidebarSection::Connections, cx),
            Cmd::GoTerminal => self.activate_dev_section(SidebarSection::Terminals, cx),
            Cmd::GoScripts => self.activate_dev_section(SidebarSection::Scripts, cx),
            Cmd::GoPortForwards => self.activate_dev_section(SidebarSection::PortForwards, cx),
            Cmd::GoServerSync => self.execute_palette_action(&OpenServerSync, cx),
            Cmd::GoSites => self.execute_palette_action(&OpenSites, cx),
            Cmd::GoRecent => self.execute_palette_action(&OpenRecent, cx),
            Cmd::GoFileEditor => self.execute_palette_action(&OpenFileEditorView, cx),
            Cmd::GoMonique => self.execute_palette_action(&OpenMoniqueConsole, cx),
            Cmd::GoFleet => self.execute_palette_action(&OpenFleet, cx),
            Cmd::GoBextCloud => self.execute_palette_action(&OpenBextCloud, cx),
            Cmd::GoSupportRequests => self.execute_palette_action(&OpenSupportRequests, cx),

            Cmd::CloseTab => self.execute_palette_action(&CloseTab, cx),
            Cmd::NextTab => self.next_tab(cx),
            Cmd::PrevTab => self.prev_tab(cx),

            Cmd::SignIn => self.show_login_form(cx),
            Cmd::SignOut => self.logout_account(cx),
            Cmd::OpenAiAssistant => self.open_ai_assistant(cx),
            Cmd::Documentation => {
                let _ = cloud_account::open_in_browser("https://shelldeck.1clic.pro");
            }
            Cmd::About => self.open_settings(cx),
        }
        cx.notify();
    }

    /// Show / hide the application menu row. The terminal grid's usable height
    /// depends on it, so the change is pushed down to the terminal view before
    /// it is persisted.
    pub fn toggle_menu_bar(&mut self, cx: &mut Context<Self>) {
        let visible = !self.app_config.general.menu_bar_visible;
        self.app_config.general.menu_bar_visible = visible;
        self.terminal.update(cx, |terminal, _| {
            terminal.set_menu_bar_visible(visible);
        });
        self.settings.update(cx, |settings, cx| {
            settings.set_menu_bar_visible(visible, cx);
        });
        cx.notify();
    }
}
