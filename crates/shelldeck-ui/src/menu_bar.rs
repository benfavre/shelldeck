//! The application menu bar (Fichier / Édition / Affichage / Aller / …).
//!
//! Rendered as a dedicated row under the titlebar in **every** app mode —
//! User, Support and Dev — plus the pre-login welcome screen, where it is
//! reduced to the handful of commands that work without a session.
//!
//! The row itself is adabraka's [`MenuBar`](adabraka_ui::prelude::MenuBar)
//! (see SDPATCH-025, which taught it to actually render its dropdown). This
//! module owns only the *contents*: [`menu_bar_spec`] is a pure function from
//! application state to a menu description, and the workspace turns that
//! description into live adabraka items with real click handlers.
//!
//! Splitting it this way keeps the interesting part — *which commands appear
//! for which account in which mode* — testable without a GPUI context, per
//! `.agents/testing.md`.

use shelldeck_core::config::cloud_account::AppMode;

/// Height of the menu row, in logical px before UI scaling.
///
/// The terminal grid sits below this row, so `TerminalView::content_area`
/// subtracts it when converting the viewport into rows/cols. Keep the two in
/// sync — see `terminal_view::chrome_top_offset`.
pub const MENU_BAR_HEIGHT: f32 = 28.0;

/// A command a menu entry can fire.
///
/// Most variants map 1:1 onto an existing `actions!` entry and are routed
/// through `Workspace::execute_palette_action`, so the menu bar, the command
/// palette and the keyboard shortcuts all go through one code path. The
/// remainder call a workspace method directly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuCommand {
    // — Fichier —
    QuickConnect,
    NewTerminal,
    NewScript,
    NewRequest,
    SyncNow,
    OpenCompanionSettings,
    OpenSettings,
    Quit,
    // — Édition —
    Copy,
    Paste,
    Find,
    CommandPalette,
    // — Affichage —
    ToggleSidebar,
    ToggleMenuBar,
    UiZoomIn,
    UiZoomOut,
    UiZoomReset,
    TerminalZoomIn,
    TerminalZoomOut,
    TerminalZoomReset,
    // — Aller —
    GoDashboard,
    GoTerminal,
    GoScripts,
    GoPortForwards,
    GoServerSync,
    GoSites,
    GoRecent,
    GoFileEditor,
    GoJean,
    GoFleet,
    GoBextCloud,
    GoSupportRequests,
    // — Terminal —
    CloseTab,
    NextTab,
    PrevTab,
    ClearTerminal,
    SplitHorizontal,
    SplitVertical,
    // — Compte / Aide —
    SignIn,
    SignOut,
    OpenAiAssistant,
    Documentation,
    About,
}

/// One row inside a dropdown.
#[derive(Debug, Clone, PartialEq)]
pub enum MenuEntry {
    Separator,
    Command {
        id: &'static str,
        label: String,
        command: MenuCommand,
        /// Rendered right-aligned in muted text. Already platform-resolved.
        shortcut: Option<String>,
        /// Lucide slug, must exist in the bundled subset (see `.agents/icons.md`).
        icon: Option<&'static str>,
        /// Renders a checkmark column. `None` = not a toggle.
        checked: Option<bool>,
    },
}

impl MenuEntry {
    fn command(id: &'static str, label: impl Into<String>, command: MenuCommand) -> Self {
        MenuEntry::Command {
            id,
            label: label.into(),
            command,
            shortcut: None,
            icon: None,
            checked: None,
        }
    }

    fn shortcut(mut self, keys: &str) -> Self {
        if let MenuEntry::Command { shortcut, .. } = &mut self {
            *shortcut = Some(accel(keys));
        }
        self
    }

    fn icon(mut self, slug: &'static str) -> Self {
        if let MenuEntry::Command { icon, .. } = &mut self {
            *icon = Some(slug);
        }
        self
    }

    fn checked(mut self, value: bool) -> Self {
        if let MenuEntry::Command { checked, .. } = &mut self {
            *checked = Some(value);
        }
        self
    }
}

/// One top-level title plus its dropdown.
#[derive(Debug, Clone, PartialEq)]
pub struct MenuSpec {
    pub id: &'static str,
    pub label: String,
    pub entries: Vec<MenuEntry>,
}

/// Everything [`menu_bar_spec`] needs to decide what to show.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MenuBarContext {
    pub signed_in: bool,
    /// The mode actually being rendered (already clamped by capability).
    pub mode: AppMode,
    /// Whether this account may reach Dev mode at all. Gates the Dev-only
    /// menus even while the user is temporarily in User mode.
    pub dev_capable: bool,
    pub sidebar_visible: bool,
    pub menu_bar_visible: bool,
    pub has_jean: bool,
    pub has_fleet: bool,
    pub ai_configured: bool,
}

/// Render `secondary-k` style binding descriptions the way the host platform
/// spells them. `secondary` is Cmd on macOS and Ctrl elsewhere, matching
/// `crates/shelldeck/src/actions.rs`.
fn accel(keys: &str) -> String {
    #[cfg(target_os = "macos")]
    const SECONDARY: &str = "Cmd";
    #[cfg(not(target_os = "macos"))]
    const SECONDARY: &str = "Ctrl";

    keys.split('-')
        .map(|part| match part {
            "secondary" => SECONDARY.to_string(),
            "ctrl" => "Ctrl".to_string(),
            "alt" => "Alt".to_string(),
            "shift" => "Shift".to_string(),
            "tab" => "Tab".to_string(),
            other => {
                let mut chars = other.chars();
                match chars.next() {
                    Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                    None => String::new(),
                }
            }
        })
        .collect::<Vec<_>>()
        .join("+")
}

/// Build the menu bar for the given application state.
///
/// Pure: no GPUI, no I/O. The workspace attaches behaviour afterwards.
pub fn menu_bar_spec(ctx: MenuBarContext) -> Vec<MenuSpec> {
    use crate::t;

    let dev = ctx.mode == AppMode::Dev;

    // ── Fichier ────────────────────────────────────────────────────────
    let mut file = Vec::new();
    if ctx.signed_in {
        if dev {
            file.push(
                MenuEntry::command(
                    "file-quick-connect",
                    t!("menu.file.quick_connect").to_string(),
                    MenuCommand::QuickConnect,
                )
                .shortcut("secondary-k")
                .icon("zap"),
            );
            file.push(
                MenuEntry::command(
                    "file-new-terminal",
                    t!("menu.file.new_terminal").to_string(),
                    MenuCommand::NewTerminal,
                )
                .shortcut("secondary-t")
                .icon("terminal"),
            );
            file.push(
                MenuEntry::command(
                    "file-new-script",
                    t!("menu.file.new_script").to_string(),
                    MenuCommand::NewScript,
                )
                .icon("scroll-text"),
            );
            file.push(MenuEntry::Separator);
        }
        file.push(
            MenuEntry::command(
                "file-new-request",
                t!("menu.file.new_request").to_string(),
                MenuCommand::NewRequest,
            )
            .icon("plus"),
        );
        file.push(
            MenuEntry::command(
                "file-sync",
                t!("menu.file.sync_now").to_string(),
                MenuCommand::SyncNow,
            )
            .icon("refresh-cw"),
        );
        file.push(MenuEntry::Separator);
        file.push(
            MenuEntry::command(
                "file-characters",
                t!("menu.file.characters").to_string(),
                MenuCommand::OpenCompanionSettings,
            )
            .icon("bot"),
        );
        file.push(
            MenuEntry::command(
                "file-settings",
                t!("menu.file.settings").to_string(),
                MenuCommand::OpenSettings,
            )
            .shortcut("secondary-,")
            .icon("settings"),
        );
        file.push(MenuEntry::Separator);
    } else {
        file.push(
            MenuEntry::command(
                "file-sign-in",
                t!("menu.account.sign_in").to_string(),
                MenuCommand::SignIn,
            )
            .icon("user-check"),
        );
        file.push(MenuEntry::Separator);
    }
    file.push(
        MenuEntry::command(
            "file-quit",
            t!("menu.file.quit").to_string(),
            MenuCommand::Quit,
        )
        .shortcut("secondary-q")
        .icon("x"),
    );

    // ── Édition ────────────────────────────────────────────────────────
    let mut edit = Vec::new();
    if ctx.signed_in {
        edit.extend([
            MenuEntry::command(
                "edit-copy",
                t!("menu.edit.copy").to_string(),
                MenuCommand::Copy,
            )
            .shortcut(if cfg!(target_os = "macos") {
                "secondary-c"
            } else {
                "ctrl-shift-c"
            })
            .icon("copy"),
            MenuEntry::command(
                "edit-paste",
                t!("menu.edit.paste").to_string(),
                MenuCommand::Paste,
            )
            .shortcut(if cfg!(target_os = "macos") {
                "secondary-v"
            } else {
                "ctrl-shift-v"
            })
            .icon("clipboard-paste"),
            MenuEntry::Separator,
        ]);
    }
    if dev {
        edit.push(
            MenuEntry::command(
                "edit-find",
                t!("menu.edit.find").to_string(),
                MenuCommand::Find,
            )
            .shortcut("secondary-f")
            .icon("search"),
        );
    }
    edit.push(
        MenuEntry::command(
            "edit-palette",
            t!("menu.edit.command_palette").to_string(),
            MenuCommand::CommandPalette,
        )
        .shortcut("secondary-shift-p")
        .icon("keyboard"),
    );

    // ── Affichage ──────────────────────────────────────────────────────
    let mut view = Vec::new();
    if dev {
        view.push(
            MenuEntry::command(
                "view-sidebar",
                t!("menu.view.sidebar").to_string(),
                MenuCommand::ToggleSidebar,
            )
            .shortcut("secondary-b")
            .checked(ctx.sidebar_visible),
        );
    }
    view.push(
        MenuEntry::command(
            "view-menu-bar",
            t!("menu.view.menu_bar").to_string(),
            MenuCommand::ToggleMenuBar,
        )
        .shortcut("secondary-shift-m")
        .checked(ctx.menu_bar_visible),
    );
    view.push(MenuEntry::Separator);
    view.push(
        MenuEntry::command(
            "view-ui-zoom-in",
            t!("menu.view.ui_zoom_in").to_string(),
            MenuCommand::UiZoomIn,
        )
        .icon("maximize-2"),
    );
    view.push(
        MenuEntry::command(
            "view-ui-zoom-out",
            t!("menu.view.ui_zoom_out").to_string(),
            MenuCommand::UiZoomOut,
        )
        .icon("minimize-2"),
    );
    view.push(MenuEntry::command(
        "view-ui-zoom-reset",
        t!("menu.view.ui_zoom_reset").to_string(),
        MenuCommand::UiZoomReset,
    ));
    if dev {
        view.push(MenuEntry::Separator);
        view.push(
            MenuEntry::command(
                "view-term-zoom-in",
                t!("menu.view.terminal_zoom_in").to_string(),
                MenuCommand::TerminalZoomIn,
            )
            .shortcut("secondary-="),
        );
        view.push(
            MenuEntry::command(
                "view-term-zoom-out",
                t!("menu.view.terminal_zoom_out").to_string(),
                MenuCommand::TerminalZoomOut,
            )
            .shortcut("secondary--"),
        );
        view.push(
            MenuEntry::command(
                "view-term-zoom-reset",
                t!("menu.view.terminal_zoom_reset").to_string(),
                MenuCommand::TerminalZoomReset,
            )
            .shortcut("secondary-0"),
        );
    }

    let mut menus = vec![
        MenuSpec {
            id: "menu-file",
            label: t!("menu.title.file").to_string(),
            entries: file,
        },
        MenuSpec {
            id: "menu-edit",
            label: t!("menu.title.edit").to_string(),
            entries: edit,
        },
        MenuSpec {
            id: "menu-view",
            label: t!("menu.title.view").to_string(),
            entries: view,
        },
    ];

    // ── Aller ──────────────────────────────────────────────────────────
    // Only meaningful once there is a session to navigate.
    if ctx.signed_in {
        let mut go = Vec::new();
        if dev {
            go.push(
                MenuEntry::command(
                    "go-dashboard",
                    t!("menu.go.dashboard").to_string(),
                    MenuCommand::GoDashboard,
                )
                .icon("grid-2x2"),
            );
            go.push(
                MenuEntry::command(
                    "go-terminal",
                    t!("menu.go.terminal").to_string(),
                    MenuCommand::GoTerminal,
                )
                .icon("terminal"),
            );
            go.push(
                MenuEntry::command(
                    "go-scripts",
                    t!("menu.go.scripts").to_string(),
                    MenuCommand::GoScripts,
                )
                .icon("scroll-text"),
            );
            go.push(
                MenuEntry::command(
                    "go-forwards",
                    t!("menu.go.port_forwards").to_string(),
                    MenuCommand::GoPortForwards,
                )
                .icon("arrow-left-right"),
            );
            go.push(
                MenuEntry::command(
                    "go-server-sync",
                    t!("menu.go.server_sync").to_string(),
                    MenuCommand::GoServerSync,
                )
                .icon("refresh-cw"),
            );
            go.push(
                MenuEntry::command(
                    "go-sites",
                    t!("menu.go.sites").to_string(),
                    MenuCommand::GoSites,
                )
                .icon("globe"),
            );
            go.push(
                MenuEntry::command(
                    "go-recent",
                    t!("menu.go.recent").to_string(),
                    MenuCommand::GoRecent,
                )
                .icon("clock"),
            );
            go.push(
                MenuEntry::command(
                    "go-editor",
                    t!("menu.go.file_editor").to_string(),
                    MenuCommand::GoFileEditor,
                )
                .shortcut("secondary-e")
                .icon("pencil"),
            );
        }
        go.push(
            MenuEntry::command(
                "go-requests",
                t!("menu.go.requests").to_string(),
                MenuCommand::GoSupportRequests,
            )
            .icon("inbox"),
        );
        // Staff-only consoles. Gated on capability, not on the current mode,
        // so a super-admin sitting in User mode still sees them — but a
        // regular account never does, per `.agents/roles.md`.
        if ctx.dev_capable && dev {
            if ctx.has_jean {
                go.push(
                    MenuEntry::command(
                        "go-jean",
                        t!("menu.go.jean").to_string(),
                        MenuCommand::GoJean,
                    )
                    .icon("bot"),
                );
            }
            if ctx.has_fleet {
                go.push(
                    MenuEntry::command(
                        "go-fleet",
                        t!("menu.go.fleet").to_string(),
                        MenuCommand::GoFleet,
                    )
                    .icon("server"),
                );
            }
            go.push(
                MenuEntry::command(
                    "go-bext",
                    t!("menu.go.bext_cloud").to_string(),
                    MenuCommand::GoBextCloud,
                )
                .icon("cloud"),
            );
        }
        menus.push(MenuSpec {
            id: "menu-go",
            label: t!("menu.title.go").to_string(),
            entries: go,
        });
    }

    // ── Terminal (Dev only) ────────────────────────────────────────────
    if dev {
        menus.push(MenuSpec {
            id: "menu-terminal",
            label: t!("menu.title.terminal").to_string(),
            entries: vec![
                MenuEntry::command(
                    "term-new",
                    t!("menu.file.new_terminal").to_string(),
                    MenuCommand::NewTerminal,
                )
                .shortcut("secondary-t"),
                MenuEntry::command(
                    "term-close",
                    t!("menu.terminal.close_tab").to_string(),
                    MenuCommand::CloseTab,
                )
                .shortcut("secondary-w"),
                MenuEntry::Separator,
                MenuEntry::command(
                    "term-next",
                    t!("menu.terminal.next_tab").to_string(),
                    MenuCommand::NextTab,
                )
                .shortcut("ctrl-tab"),
                MenuEntry::command(
                    "term-prev",
                    t!("menu.terminal.prev_tab").to_string(),
                    MenuCommand::PrevTab,
                )
                .shortcut("ctrl-shift-tab"),
                MenuEntry::Separator,
                MenuEntry::command(
                    "term-split-h",
                    t!("menu.terminal.split_horizontal").to_string(),
                    MenuCommand::SplitHorizontal,
                ),
                MenuEntry::command(
                    "term-split-v",
                    t!("menu.terminal.split_vertical").to_string(),
                    MenuCommand::SplitVertical,
                ),
                MenuEntry::Separator,
                MenuEntry::command(
                    "term-clear",
                    t!("menu.terminal.clear").to_string(),
                    MenuCommand::ClearTerminal,
                )
                .shortcut("secondary-l"),
            ],
        });
    }

    // ── Aide ───────────────────────────────────────────────────────────
    let mut help = Vec::new();
    if ctx.ai_configured && ctx.signed_in {
        help.push(
            MenuEntry::command(
                "help-ai",
                t!("menu.help.ai_assistant").to_string(),
                MenuCommand::OpenAiAssistant,
            )
            .shortcut("secondary-shift-k")
            .icon("sparkles"),
        );
        help.push(MenuEntry::Separator);
    }
    help.push(
        MenuEntry::command(
            "help-docs",
            t!("menu.help.documentation").to_string(),
            MenuCommand::Documentation,
        )
        .icon("circle-help"),
    );
    if ctx.signed_in {
        help.push(
            MenuEntry::command(
                "help-about",
                t!("menu.help.about").to_string(),
                MenuCommand::About,
            )
            .icon("info"),
        );
    }
    if ctx.signed_in {
        help.push(MenuEntry::Separator);
        help.push(
            MenuEntry::command(
                "help-sign-out",
                t!("menu.account.sign_out").to_string(),
                MenuCommand::SignOut,
            )
            .icon("user-x"),
        );
    }
    menus.push(MenuSpec {
        id: "menu-help",
        label: t!("menu.title.help").to_string(),
        entries: help,
    });

    menus
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(mode: AppMode, signed_in: bool) -> MenuBarContext {
        MenuBarContext {
            signed_in,
            mode,
            dev_capable: mode == AppMode::Dev,
            sidebar_visible: true,
            menu_bar_visible: true,
            has_jean: false,
            has_fleet: false,
            ai_configured: false,
        }
    }

    fn commands(menus: &[MenuSpec]) -> Vec<MenuCommand> {
        menus
            .iter()
            .flat_map(|m| &m.entries)
            .filter_map(|e| match e {
                MenuEntry::Command { command, .. } => Some(*command),
                MenuEntry::Separator => None,
            })
            .collect()
    }

    fn titles(menus: &[MenuSpec]) -> Vec<&'static str> {
        menus.iter().map(|m| m.id).collect()
    }

    // SDTEST-1200 — logged out, the bar must not offer any command that needs
    // a session. This is the welcome-screen contract from `.agents/roles.md`:
    // no guest mode, so nothing beyond sign-in / quit / zoom, palette,
    // documentation and menu recovery.
    #[test]
    fn logged_out_bar_exposes_no_session_commands() {
        let menus = menu_bar_spec(ctx(AppMode::User, false));
        let cmds = commands(&menus);

        assert!(cmds.contains(&MenuCommand::SignIn));
        assert!(cmds.contains(&MenuCommand::Quit));
        // Nothing that would hit the network or a connection store.
        for forbidden in [
            MenuCommand::QuickConnect,
            MenuCommand::NewTerminal,
            MenuCommand::NewRequest,
            MenuCommand::SyncNow,
            MenuCommand::OpenSettings,
            MenuCommand::Copy,
            MenuCommand::Paste,
            MenuCommand::About,
            MenuCommand::SignOut,
            MenuCommand::GoSupportRequests,
        ] {
            assert!(
                !cmds.contains(&forbidden),
                "{forbidden:?} must not appear while logged out"
            );
        }
        // "Aller" is a navigation menu with nothing to navigate to.
        assert!(!titles(&menus).contains(&"menu-go"));
    }

    // SDTEST-1201 — User mode is the customer surface: no terminal, no SSH,
    // no staff consoles, even though the same function serves Dev.
    #[test]
    fn user_mode_hides_every_dev_only_command() {
        let menus = menu_bar_spec(ctx(AppMode::User, true));
        let cmds = commands(&menus);

        assert!(cmds.contains(&MenuCommand::NewRequest));
        assert!(cmds.contains(&MenuCommand::GoSupportRequests));
        for forbidden in [
            MenuCommand::QuickConnect,
            MenuCommand::NewTerminal,
            MenuCommand::NewScript,
            MenuCommand::GoTerminal,
            MenuCommand::GoFileEditor,
            MenuCommand::ToggleSidebar,
            MenuCommand::SplitHorizontal,
            MenuCommand::TerminalZoomIn,
        ] {
            assert!(
                !cmds.contains(&forbidden),
                "{forbidden:?} must not appear in User mode"
            );
        }
        assert!(!titles(&menus).contains(&"menu-terminal"));
    }

    // SDTEST-1202 — the staff consoles are gated on *availability*, not just
    // on Dev mode: a super-admin with no Jean config must not get a dead
    // "JeanClaude" entry ("never display a mode the caller can't reach").
    #[test]
    fn staff_consoles_follow_availability_flags() {
        let mut c = ctx(AppMode::Dev, true);
        c.dev_capable = true;
        c.has_jean = false;
        c.has_fleet = false;
        let cmds = commands(&menu_bar_spec(c));
        assert!(!cmds.contains(&MenuCommand::GoJean));
        assert!(!cmds.contains(&MenuCommand::GoFleet));
        // bext Cloud has no availability flag — it is Dev-capable-gated only.
        assert!(cmds.contains(&MenuCommand::GoBextCloud));

        c.has_jean = true;
        c.has_fleet = true;
        let cmds = commands(&menu_bar_spec(c));
        assert!(cmds.contains(&MenuCommand::GoJean));
        assert!(cmds.contains(&MenuCommand::GoFleet));
    }

    // SDTEST-1203 — the two view toggles must report live state, otherwise the
    // checkmark lies about what is on screen.
    #[test]
    fn view_toggles_reflect_current_state() {
        let mut c = ctx(AppMode::Dev, true);
        c.dev_capable = true;
        c.sidebar_visible = false;
        c.menu_bar_visible = true;

        let menus = menu_bar_spec(c);
        let view = menus.iter().find(|m| m.id == "menu-view").unwrap();

        let checked_of = |cmd: MenuCommand| {
            view.entries.iter().find_map(|e| match e {
                MenuEntry::Command {
                    command, checked, ..
                } if *command == cmd => Some(*checked),
                _ => None,
            })
        };

        assert_eq!(checked_of(MenuCommand::ToggleSidebar), Some(Some(false)));
        assert_eq!(checked_of(MenuCommand::ToggleMenuBar), Some(Some(true)));
    }

    // SDTEST-1204 — entry ids are used as GPUI ElementIds; a duplicate makes
    // two rows share hover/click state. Guard the whole bar, in every mode.
    #[test]
    fn entry_ids_are_unique_across_the_whole_bar() {
        for (mode, signed_in) in [
            (AppMode::User, false),
            (AppMode::User, true),
            (AppMode::Support, true),
            (AppMode::Dev, true),
        ] {
            let mut c = ctx(mode, signed_in);
            c.dev_capable = true;
            c.has_jean = true;
            c.has_fleet = true;
            c.ai_configured = true;

            let menus = menu_bar_spec(c);
            let mut seen = std::collections::HashSet::new();
            for menu in &menus {
                assert!(seen.insert(menu.id), "duplicate menu id {}", menu.id);
                for entry in &menu.entries {
                    if let MenuEntry::Command { id, .. } = entry {
                        assert!(
                            seen.insert(id),
                            "duplicate entry id {id} in {mode:?}/{signed_in}"
                        );
                    }
                }
            }
        }
    }

    // SDTEST-1205 — `accel` renders bindings the way the platform spells
    // them, from the same `secondary-…` vocabulary `actions.rs` binds with.
    #[test]
    fn accel_renders_platform_modifiers() {
        let secondary = if cfg!(target_os = "macos") {
            "Cmd"
        } else {
            "Ctrl"
        };
        assert_eq!(accel("secondary-k"), format!("{secondary}+K"));
        assert_eq!(accel("secondary-shift-p"), format!("{secondary}+Shift+P"));
        assert_eq!(accel("ctrl-shift-tab"), "Ctrl+Shift+Tab");
    }
}
