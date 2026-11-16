use super::*;

/// The menu row shown for one use, like Firefox's menu bar: a tap on Alt or a
/// click on the chevron beside the logo shows it over the content, and it
/// leaves once a menu has been used or dismissed. A pinned row
/// (`general.menu_bar_always_visible`) does not use this state.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(super) struct MenuRowState {
    /// Shown over the content until the next use.
    pub(super) revealed: bool,
    /// Alt went down alone and nothing else has happened since: releasing it
    /// is a tap. Any key or other modifier makes it a shortcut instead.
    alt_armed: bool,
    /// A menu of the row was open at the last notification of the menu bar.
    menu_was_open: bool,
}

impl MenuRowState {
    /// Follows the modifier keys; true when an Alt tap has just completed.
    pub(super) fn on_modifiers(&mut self, modifiers: Modifiers) -> bool {
        let others =
            modifiers.control || modifiers.shift || modifiers.platform || modifiers.function;
        if modifiers.alt && !others {
            self.alt_armed = true;
            false
        } else if !modifiers.alt && !others && self.alt_armed {
            self.alt_armed = false;
            true
        } else {
            self.alt_armed = false;
            false
        }
    }

    /// A key went down: Alt held with it is a shortcut, not a tap.
    pub(super) fn on_key_down(&mut self) {
        self.alt_armed = false;
    }

    /// The menu bar opened or closed a menu; true when a menu has just been
    /// closed, which ends a one-use reveal.
    pub(super) fn on_menu_open(&mut self, open: bool) -> bool {
        let closed = self.menu_was_open && !open;
        self.menu_was_open = open;
        closed
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct MenuBarSnapshot {
    context: crate::menu_bar::MenuBarContext,
    compact: bool,
    locale: &'static str,
}

impl Workspace {
    // --- Application menu bar ---

    /// Rebuild the menu row from current state and hand it to the adabraka
    /// `MenuBar`. Sample every input each render, but rebuild translated items
    /// and callbacks only when their context, responsive layout or locale changes.
    pub(super) fn rebuild_menu_bar(&mut self, logical_width: f32, cx: &mut Context<Self>) {
        use crate::menu_bar::{
            compact_menu_bar, menu_bar_presentation, MenuBarContext, MenuCommand, MenuEntry,
        };

        let mode = self.effective_mode();
        let active_go = if mode == AppMode::Dev {
            match self.active_view {
                ActiveView::Agents => Some(MenuCommand::GoAgents),
                ActiveView::Terminal => Some(MenuCommand::GoTerminal),
                ActiveView::Scripts => Some(MenuCommand::GoScripts),
                ActiveView::Recent => Some(MenuCommand::GoRecent),
                ActiveView::FileEditor => Some(MenuCommand::GoFileEditor),
                ActiveView::MoniqueConsole => Some(MenuCommand::GoMonique),
                ActiveView::Fleet => Some(MenuCommand::GoFleet),
                ActiveView::BextCloud => Some(MenuCommand::GoBextCloud),
                _ => None,
            }
        } else {
            None
        };

        let ctx = MenuBarContext {
            signed_in: self.signed_in(),
            mode,
            dev_capable: self
                .app_config
                .account
                .as_ref()
                .is_some_and(|a| a.is_superadmin),
            sidebar_visible: self.contextual_sidebar_visible(cx),
            menu_bar_always_visible: self.app_config.general.menu_bar_always_visible,
            has_monique: self.has_monique(),
            has_fleet: self.platform_connection().is_some() || self.fleet_snapshot.is_some(),
            ai_configured: self.ai_available_for_current_surface(cx),
            ai_dock: self.can_access_mode(AppMode::Dev)
                || self.ai_available_for_current_surface(cx),
            active_go,
        };

        let snapshot = MenuBarSnapshot {
            context: ctx,
            compact: compact_menu_bar(logical_width),
            locale: crate::i18n::resolve_locale(&self.app_config.general.ui_language),
        };
        if self.menu_bar_snapshot == Some(snapshot) {
            return;
        }
        self.menu_bar_snapshot = Some(snapshot);
        #[cfg(test)]
        {
            self.menu_bar_rebuilds += 1;
        }

        let items = menu_bar_presentation(ctx, snapshot.compact)
            .into_iter()
            .map(|menu| {
                let entries = menu
                    .entries
                    .into_iter()
                    .map(|entry| match entry {
                        MenuEntry::Separator => AdabrakaMenuItem::separator(),
                        MenuEntry::Section { id, label } => {
                            AdabrakaMenuItem::new(id, label).disabled(true)
                        }
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
            bar.set_reveal_active(!compact_menu_bar(logical_width));
            // Preserve the open menu across the rebuild: `set_items` closes
            // it, including a context change while a menu is open.
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
            TogglePaneZoom, ToggleSearch, ZoomIn, ZoomOut, ZoomReset,
        };

        // Using a menu ends a one-use reveal of the row.
        self.menu_row.revealed = false;

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
            Cmd::MaximizePane => window.dispatch_action(Box::new(TogglePaneZoom), cx),
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

            Cmd::NextPresentation => self.execute_palette_action(&CyclePresentation, cx),

            Cmd::GoAgents => self.activate_dev_section(SidebarSection::Agents, cx),
            Cmd::GoTerminal => self.activate_dev_section(SidebarSection::Terminals, cx),
            Cmd::GoScripts => self.activate_dev_section(SidebarSection::Scripts, cx),
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
            Cmd::OpenAiDock => self.open_external_ai_dock(cx),
            Cmd::Documentation => {
                let _ = cloud_account::open_in_browser("https://shelldeck.1clic.pro");
            }
            Cmd::CheckForUpdates => self.check_for_updates_now(cx),
            Cmd::About => {
                self.settings.update(cx, |settings, cx| {
                    settings.set_active_tab(crate::settings::SettingsTab::About, cx);
                });
                self.open_settings(cx);
            }
        }
        cx.notify();
    }

    /// The menu row exists in the full app and on the welcome screen; the
    /// focused views (Agents, Terminal, Editor) have no menus.
    pub(super) fn menu_row_available(&self) -> bool {
        !self.active_presentation().is_focused()
    }

    pub(super) fn menu_row_pinned(&self) -> bool {
        self.app_config.general.menu_bar_always_visible
    }

    /// Whether the row is on screen, pinned or revealed for one use.
    pub(super) fn menu_row_shown(&self) -> bool {
        self.menu_row_available() && (self.menu_row_pinned() || self.menu_row.revealed)
    }

    /// Show or hide the row for one use. Hiding it closes an open menu.
    pub(super) fn set_menu_row_revealed(&mut self, revealed: bool, cx: &mut Context<Self>) {
        if self.menu_row.revealed == revealed {
            return;
        }
        self.menu_row.revealed = revealed;
        if !revealed {
            self.menu_bar.update(cx, |bar, cx| {
                if bar.is_open() {
                    bar.close();
                    cx.notify();
                }
            });
        }
        cx.notify();
    }

    /// The chevron beside the logo: shows the row for one use, or hides it.
    /// On a pinned row it unpins, so the chevron always does what it shows.
    pub(super) fn toggle_menu_row(&mut self, cx: &mut Context<Self>) {
        if self.menu_row_pinned() {
            self.toggle_menu_bar(cx);
            self.set_menu_row_revealed(false, cx);
        } else {
            let revealed = !self.menu_row.revealed;
            self.set_menu_row_revealed(revealed, cx);
        }
    }

    /// Modifier keys changed anywhere in the window: a tap on Alt alone
    /// shows or hides the row, as in Firefox.
    pub(super) fn menu_row_modifiers_changed(
        &mut self,
        modifiers: Modifiers,
        cx: &mut Context<Self>,
    ) {
        if self.menu_row.on_modifiers(modifiers)
            && self.menu_row_available()
            && !self.menu_row_pinned()
        {
            let revealed = !self.menu_row.revealed;
            self.set_menu_row_revealed(revealed, cx);
        }
    }

    pub(super) fn menu_row_key_down(&mut self, key: &str, cx: &mut Context<Self>) {
        self.menu_row.on_key_down();
        if key == "escape" && self.menu_row.revealed && !self.menu_bar.read(cx).is_open() {
            self.set_menu_row_revealed(false, cx);
        }
    }

    /// The menu bar notified: a menu closed without a command (a click on
    /// its title, outside it, Escape) also ends a one-use reveal.
    pub(super) fn menu_bar_open_changed(&mut self, open: bool, cx: &mut Context<Self>) {
        if self.menu_row.on_menu_open(open) && self.menu_row.revealed {
            self.set_menu_row_revealed(false, cx);
        }
    }

    /// Pin / unpin the menu row (Affichage → Toujours afficher la barre de
    /// menus). Only a pinned row takes height from the terminal grid, so the
    /// change is pushed down to the terminal view before it is persisted.
    pub fn toggle_menu_bar(&mut self, cx: &mut Context<Self>) {
        let visible = !self.app_config.general.menu_bar_always_visible;
        self.app_config.general.menu_bar_always_visible = visible;
        self.terminal.update(cx, |terminal, _| {
            terminal.set_menu_bar_always_visible(visible);
        });
        self.settings.update(cx, |settings, cx| {
            settings.set_menu_bar_always_visible(visible, cx);
        });
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::MenuRowState;
    use gpui::Modifiers;

    fn alt() -> Modifiers {
        Modifiers {
            alt: true,
            ..Modifiers::default()
        }
    }

    // SDTEST-3184 / SDUC-591. Alt pressed and released alone is a tap that
    // shows the menu row; Alt held with a key or with another modifier is a
    // shortcut and shows nothing; closing a menu ends a one-use reveal.
    #[test]
    fn sdtest_3184_alt_tap_shows_the_menu_row_and_shortcuts_do_not() {
        let mut row = MenuRowState::default();
        assert!(!row.on_modifiers(alt()));
        assert!(row.on_modifiers(Modifiers::default()), "a tap");

        // Alt+F: a key went down while Alt was held.
        assert!(!row.on_modifiers(alt()));
        row.on_key_down();
        assert!(!row.on_modifiers(Modifiers::default()));

        // Ctrl+Alt: another modifier joined.
        assert!(!row.on_modifiers(alt()));
        assert!(!row.on_modifiers(Modifiers {
            alt: true,
            control: true,
            ..Modifiers::default()
        }));
        assert!(!row.on_modifiers(Modifiers::default()));

        // Shift released alone is not an Alt tap.
        assert!(!row.on_modifiers(Modifiers {
            shift: true,
            ..Modifiers::default()
        }));
        assert!(!row.on_modifiers(Modifiers::default()));

        // A menu opened then closed ends the reveal once; staying closed or
        // moving from one menu to another does not.
        assert!(!row.on_menu_open(false));
        assert!(!row.on_menu_open(true));
        assert!(!row.on_menu_open(true));
        assert!(row.on_menu_open(false));
        assert!(!row.on_menu_open(false));
    }
}
