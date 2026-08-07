mod actions;
mod companion_desktop;
mod tray;

use adabraka_ui::prelude::*;
use anyhow::Result;
use gpui::{AssetSource, SharedString, WindowDecorations};
use shelldeck_core::ai::ClippyConfig;
use shelldeck_core::config::app_config::{AppConfig, CompanionConfig};
use shelldeck_core::config::deep_link::DeepLink;
use shelldeck_core::config::single_instance::{self, Acquire};
use shelldeck_core::config::ssh_config::parse_ssh_config;
use shelldeck_core::config::store::ConnectionStore;
use shelldeck_core::models::connection::Connection;
use shelldeck_ui::theme::ShellDeckColors;
use shelldeck_ui::{
    settings::{CompanionShortcutStatuses, ShortcutRegistrationStatus},
    AiCompanionController, AiCompanionEvent, AiDockView, CommandPaletteWindowView, Workspace,
    WorkspaceAiBindings,
};
use std::{borrow::Cow, cell::RefCell, rc::Rc};
use tracing_subscriber::EnvFilter;

use crate::companion_desktop::{is_x11_session, CompanionRuntimeCommand, DesktopCharacterRuntime};

/// Embed Lucide SVGs at `icons/lucide/{name}.svg`. Add new slugs here when
/// copying icons into `assets/icons/lucide/` (see that folder's README).
macro_rules! lucide_assets {
    ($($name:literal),* $(,)?) => {
        fn lucide_bytes(path: &str) -> Option<&'static [u8]> {
            match path {
                $(
                    concat!("icons/lucide/", $name, ".svg") => Some(include_bytes!(concat!(
                        "../assets/icons/lucide/",
                        $name,
                        ".svg"
                    ))),
                )*
                _ => None,
            }
        }

        fn lucide_asset_paths() -> Vec<SharedString> {
            vec![$(SharedString::from(concat!("icons/lucide/", $name, ".svg")),)*]
        }
    };
}

lucide_assets!(
    "activity",
    "archive",
    "archive-restore",
    "arrow-down",
    "arrow-up",
    "at-sign",
    "git-branch",
    "arrow-left-right",
    "arrow-right",
    "bot",
    "box",
    "calendar",
    "check",
    "check-check",
    "chevron-down",
    "chevron-left",
    "chevron-right",
    "chevron-up",
    "circle-alert",
    "cloud",
    "circle-check",
    "circle-help",
    "clock",
    "clipboard-paste",
    "copy",
    "cpu",
    "database",
    "download",
    "ellipsis",
    "ellipsis-vertical",
    "external-link",
    "eye",
    "eye-off",
    "filter",
    "flag",
    "globe",
    "grid-2x2",
    "house",
    "inbox",
    "info",
    "key",
    "keyboard",
    "list-checks",
    "lock",
    "mail",
    "maximize-2",
    "messages-square",
    "minimize-2",
    "minus",
    "pencil",
    "pin",
    "play",
    "plus",
    "refresh-cw",
    "reply",
    "route",
    "rotate-ccw",
    "scan",
    "search",
    "scroll-text",
    "send",
    "server",
    "settings",
    "shield",
    "shield-check",
    "sparkles",
    "square",
    "sticky-note",
    "table",
    "tag",
    "terminal",
    "trash-2",
    "triangle-alert",
    "upload",
    "user",
    "user-check",
    "user-x",
    "users",
    "x",
    "zap",
);

/// Embed Simple Icons SVGs at `icons/simple/{name}.svg` (brand / tech marks).
/// Sourced from https://github.com/LitoMore/simple-icons-cdn — GPUI tints via
/// `text_color` like Lucide.
macro_rules! simple_assets {
    ($($name:literal),* $(,)?) => {
        fn simple_bytes(path: &str) -> Option<&'static [u8]> {
            match path {
                $(
                    concat!("icons/simple/", $name, ".svg") => Some(include_bytes!(concat!(
                        "../assets/icons/simple/",
                        $name,
                        ".svg"
                    ))),
                )*
                _ => None,
            }
        }

        fn simple_asset_paths() -> Vec<SharedString> {
            vec![$(SharedString::from(concat!("icons/simple/", $name, ".svg")),)*]
        }
    };
}

simple_assets!(
    "anthropic",
    "bun",
    "claudecode",
    "docker",
    "dockercompose",
    "gnubash",
    "linux",
    "mysql",
    "nginx",
    "nodedotjs",
    "openai",
    "php",
    "postgresql",
    "python",
    "systemd",
);

macro_rules! character_assets {
    ($($character:literal),+ $(,)?) => {
        fn character_bytes(path: &str) -> Option<&'static [u8]> {
            match path {
                $(
                    concat!("characters/", $character, "/idle.png") => Some(include_bytes!(concat!("../assets/characters/", $character, "/idle.png"))),
                    concat!("characters/", $character, "/idle.svg") => Some(include_bytes!(concat!("../assets/characters/", $character, "/idle.svg"))),
                    concat!("characters/", $character, "/listening.svg") => Some(include_bytes!(concat!("../assets/characters/", $character, "/listening.svg"))),
                    concat!("characters/", $character, "/thinking.svg") => Some(include_bytes!(concat!("../assets/characters/", $character, "/thinking.svg"))),
                    concat!("characters/", $character, "/success.svg") => Some(include_bytes!(concat!("../assets/characters/", $character, "/success.svg"))),
                    concat!("characters/", $character, "/warning.svg") => Some(include_bytes!(concat!("../assets/characters/", $character, "/warning.svg"))),
                    concat!("characters/", $character, "/error.svg") => Some(include_bytes!(concat!("../assets/characters/", $character, "/error.svg"))),
                    concat!("characters/", $character, "/sleeping.svg") => Some(include_bytes!(concat!("../assets/characters/", $character, "/sleeping.svg"))),
                )+
                _ => None,
            }
        }

        fn character_asset_paths() -> Vec<SharedString> {
            vec![
                $(
                    SharedString::from(concat!("characters/", $character, "/idle.png")),
                    SharedString::from(concat!("characters/", $character, "/idle.svg")),
                    SharedString::from(concat!("characters/", $character, "/listening.svg")),
                    SharedString::from(concat!("characters/", $character, "/thinking.svg")),
                    SharedString::from(concat!("characters/", $character, "/success.svg")),
                    SharedString::from(concat!("characters/", $character, "/warning.svg")),
                    SharedString::from(concat!("characters/", $character, "/error.svg")),
                    SharedString::from(concat!("characters/", $character, "/sleeping.svg")),
                )+
            ]
        }
    };
}

character_assets!("clippy", "shelly", "spark", "byte", "orbit", "nox");

/// In-process asset source that ships a small set of images embedded in the
/// binary (see `assets/images/`). GPUI's `svg()` element requires an
/// `AssetSource` to resolve `.path("images/…")`.
struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        let path = path.strip_prefix('/').unwrap_or(path);
        let bytes: &'static [u8] = match path {
            "images/wd29-logo.svg" => include_bytes!("../assets/images/wd29-logo.svg"),
            // Monolith app icon (Dark, coins arrondis) — SVG for reference, PNG for UI paint.
            "images/shelldeck-icon.svg" => include_bytes!("../assets/images/shelldeck-icon.svg"),
            "images/shelldeck-icon.png" => include_bytes!("../assets/images/shelldeck-icon.png"),
            // Monochrome mark — cadre evenodd + visage, `currentColor`.
            "images/shelldeck-mark.svg" => include_bytes!("../assets/images/shelldeck-mark.svg"),
            // Monolith expression sources (brand kit — future animation swaps).
            "images/brand/svg/expressions/dark-default-logo.svg" => {
                include_bytes!("../assets/images/brand/svg/expressions/dark-default-logo.svg")
            }
            "images/brand/svg/expressions/dark-neutral-logo.svg" => {
                include_bytes!("../assets/images/brand/svg/expressions/dark-neutral-logo.svg")
            }
            "images/brand/svg/expressions/dark-wink-logo.svg" => {
                include_bytes!("../assets/images/brand/svg/expressions/dark-wink-logo.svg")
            }
            // Full-expression animated Monolith personalities used by the
            // User, Support, and Dev mode transition loaders.
            "images/brand/webp/modes/monolith-user.webp" => {
                include_bytes!("../assets/images/brand/webp/modes/monolith-user.webp")
            }
            "images/brand/webp/modes/monolith-support.webp" => {
                include_bytes!("../assets/images/brand/webp/modes/monolith-support.webp")
            }
            "images/brand/webp/modes/monolith-dev.webp" => {
                include_bytes!("../assets/images/brand/webp/modes/monolith-dev.webp")
            }
            // Compact Monolith motions used by contextual loading and empty
            // states throughout the application.
            "images/brand/webp/studies/monolith-thinking.webp" => {
                include_bytes!("../assets/images/brand/webp/studies/monolith-thinking.webp")
            }
            "images/brand/webp/studies/monolith-scan.webp" => {
                include_bytes!("../assets/images/brand/webp/studies/monolith-scan.webp")
            }
            "images/brand/webp/studies/monolith-terminal-typing.webp" => {
                include_bytes!("../assets/images/brand/webp/studies/monolith-terminal-typing.webp")
            }
            // Per-theme in-app badge PNGs — `brand_badge()` swaps to match the
            // active palette. Kept as PNG because GPUI `svg()` is monochrome.
            "images/brand/png/themes/monolith-dark-128.png" => {
                include_bytes!("../assets/images/brand/png/themes/monolith-dark-128.png")
            }
            "images/brand/png/themes/monolith-light-128.png" => {
                include_bytes!("../assets/images/brand/png/themes/monolith-light-128.png")
            }
            "images/brand/png/themes/monolith-dracula-128.png" => {
                include_bytes!("../assets/images/brand/png/themes/monolith-dracula-128.png")
            }
            "images/brand/png/themes/monolith-nord-128.png" => {
                include_bytes!("../assets/images/brand/png/themes/monolith-nord-128.png")
            }
            "images/brand/png/themes/monolith-tokyo-night-128.png" => {
                include_bytes!("../assets/images/brand/png/themes/monolith-tokyo-night-128.png")
            }
            "images/brand/png/themes/monolith-gruvbox-dark-128.png" => {
                include_bytes!("../assets/images/brand/png/themes/monolith-gruvbox-dark-128.png")
            }
            "images/brand/png/themes/monolith-solarized-dark-128.png" => {
                include_bytes!("../assets/images/brand/png/themes/monolith-solarized-dark-128.png")
            }
            "images/brand/png/themes/monolith-solarized-light-128.png" => {
                include_bytes!("../assets/images/brand/png/themes/monolith-solarized-light-128.png")
            }
            "images/brand/png/themes/monolith-catppuccin-mocha-128.png" => {
                include_bytes!(
                    "../assets/images/brand/png/themes/monolith-catppuccin-mocha-128.png"
                )
            }
            "images/brand/png/themes/monolith-one-dark-128.png" => {
                include_bytes!("../assets/images/brand/png/themes/monolith-one-dark-128.png")
            }
            "images/brand/png/themes/monolith-monokai-128.png" => {
                include_bytes!("../assets/images/brand/png/themes/monolith-monokai-128.png")
            }
            "images/brand/png/themes/monolith-rose-pine-128.png" => {
                include_bytes!("../assets/images/brand/png/themes/monolith-rose-pine-128.png")
            }
            // First-run onboarding artwork — exported at 2× from the editable
            // HTML/CSS brand study under docs/design/.
            "images/onboarding/welcome.png" => {
                include_bytes!("../assets/images/onboarding/welcome.png")
            }
            "images/onboarding/modes.png" => {
                include_bytes!("../assets/images/onboarding/modes.png")
            }
            "images/onboarding/surfaces.png" => {
                include_bytes!("../assets/images/onboarding/surfaces.png")
            }
            "images/onboarding/shortcuts.png" => {
                include_bytes!("../assets/images/onboarding/shortcuts.png")
            }
            // Dashboard-specific artwork. It deliberately leaves the right
            // half quiet so localized copy remains readable over the image.
            "images/home/user-dashboard-colorful-watermark-v2.webp" => {
                include_bytes!("../assets/images/home/user-dashboard-colorful-watermark-v2.webp")
            }
            // Magnifying-glass icon used by search inputs (sidebar filter, …).
            "images/search.svg" => include_bytes!("../assets/images/search.svg"),
            // Vertical three-dot "kebab" menu handle used by list row actions.
            "images/kebab.svg" => include_bytes!("../assets/images/kebab.svg"),
            // Common UI glyphs (mono, currentColor).
            "images/close.svg" => include_bytes!("../assets/images/close.svg"),
            "images/plus.svg" => include_bytes!("../assets/images/plus.svg"),
            "images/minus.svg" => include_bytes!("../assets/images/minus.svg"),
            "images/minimize.svg" => include_bytes!("../assets/images/minimize.svg"),
            "images/maximize.svg" => include_bytes!("../assets/images/maximize.svg"),
            "images/restore.svg" => include_bytes!("../assets/images/restore.svg"),
            "images/chevron-down.svg" => include_bytes!("../assets/images/chevron-down.svg"),
            "images/refresh.svg" => include_bytes!("../assets/images/refresh.svg"),
            "images/pin.svg" => include_bytes!("../assets/images/pin.svg"),
            "images/pin-outline.svg" => include_bytes!("../assets/images/pin-outline.svg"),
            "images/external-link.svg" => include_bytes!("../assets/images/external-link.svg"),
            // Login OIDC provider logos. Simple-icons GitHub/Google + Inklura
            // (multi-color source, GPUI paints mono via text_color) and a
            // cloud-glyph placeholder for 1clic.pro until we get the brand mark.
            "images/logo-inklura.svg" => include_bytes!("../assets/images/logo-inklura.svg"),
            "images/logo-github.svg" => include_bytes!("../assets/images/logo-github.svg"),
            "images/logo-google.svg" => include_bytes!("../assets/images/logo-google.svg"),
            "images/logo-1clicpro.svg" => include_bytes!("../assets/images/logo-1clicpro.svg"),
            _ => {
                if let Some(bytes) = character_bytes(path) {
                    return Ok(Some(Cow::Borrowed(bytes)));
                }
                if let Some(bytes) = simple_bytes(path) {
                    return Ok(Some(Cow::Borrowed(bytes)));
                }
                if let Some(bytes) = lucide_bytes(path) {
                    return Ok(Some(Cow::Borrowed(bytes)));
                }
                return Ok(None);
            }
        };
        Ok(Some(Cow::Borrowed(bytes)))
    }

    fn list(&self, _path: &str) -> Result<Vec<SharedString>> {
        let mut paths = vec![
            SharedString::from("images/wd29-logo.svg"),
            SharedString::from("images/shelldeck-icon.svg"),
            SharedString::from("images/shelldeck-icon.png"),
            SharedString::from("images/shelldeck-mark.svg"),
            SharedString::from("images/brand/svg/expressions/dark-default-logo.svg"),
            SharedString::from("images/brand/svg/expressions/dark-neutral-logo.svg"),
            SharedString::from("images/brand/svg/expressions/dark-wink-logo.svg"),
            SharedString::from("images/brand/webp/modes/monolith-user.webp"),
            SharedString::from("images/brand/webp/modes/monolith-support.webp"),
            SharedString::from("images/brand/webp/modes/monolith-dev.webp"),
            SharedString::from("images/brand/webp/studies/monolith-thinking.webp"),
            SharedString::from("images/brand/webp/studies/monolith-scan.webp"),
            SharedString::from("images/brand/webp/studies/monolith-terminal-typing.webp"),
            SharedString::from("images/brand/png/themes/monolith-dark-128.png"),
            SharedString::from("images/brand/png/themes/monolith-light-128.png"),
            SharedString::from("images/brand/png/themes/monolith-dracula-128.png"),
            SharedString::from("images/brand/png/themes/monolith-nord-128.png"),
            SharedString::from("images/brand/png/themes/monolith-tokyo-night-128.png"),
            SharedString::from("images/brand/png/themes/monolith-gruvbox-dark-128.png"),
            SharedString::from("images/brand/png/themes/monolith-solarized-dark-128.png"),
            SharedString::from("images/brand/png/themes/monolith-solarized-light-128.png"),
            SharedString::from("images/brand/png/themes/monolith-catppuccin-mocha-128.png"),
            SharedString::from("images/brand/png/themes/monolith-one-dark-128.png"),
            SharedString::from("images/brand/png/themes/monolith-monokai-128.png"),
            SharedString::from("images/brand/png/themes/monolith-rose-pine-128.png"),
            SharedString::from("images/onboarding/welcome.png"),
            SharedString::from("images/onboarding/modes.png"),
            SharedString::from("images/onboarding/surfaces.png"),
            SharedString::from("images/onboarding/shortcuts.png"),
            SharedString::from("images/home/user-dashboard-colorful-watermark-v2.webp"),
            SharedString::from("images/search.svg"),
            SharedString::from("images/kebab.svg"),
            SharedString::from("images/close.svg"),
            SharedString::from("images/plus.svg"),
            SharedString::from("images/minus.svg"),
            SharedString::from("images/minimize.svg"),
            SharedString::from("images/maximize.svg"),
            SharedString::from("images/restore.svg"),
            SharedString::from("images/chevron-down.svg"),
            SharedString::from("images/refresh.svg"),
            SharedString::from("images/pin.svg"),
            SharedString::from("images/pin-outline.svg"),
            SharedString::from("images/external-link.svg"),
            SharedString::from("images/logo-inklura.svg"),
            SharedString::from("images/logo-github.svg"),
            SharedString::from("images/logo-google.svg"),
            SharedString::from("images/logo-1clicpro.svg"),
        ];
        paths.extend(character_asset_paths());
        paths.extend(simple_asset_paths());
        paths.extend(lucide_asset_paths());
        Ok(paths)
    }
}

/// Format + fire an OS notification for a workspace-side delta.
/// Called from a detached thread so a slow notification daemon (D-Bus
/// on Linux, `NSUserNotificationCenter` on macOS, WinRT toasts on
/// Windows) never blocks the workspace or the tray.
///
/// Icon strategy: pass the freedesktop-compatible name `shelldeck` —
/// the packaging install ships an entry under `/usr/share/icons/…` so
/// notification daemons pick it up. On macOS/Windows `notify-rust`
/// falls back gracefully when the icon can't be resolved.
fn show_tray_notification(n: shelldeck_ui::TrayNotification) -> anyhow::Result<()> {
    let (summary, body) = n.localized_text();
    notify_rust::Notification::new()
        .appname("ShellDeck")
        .summary(&summary)
        .body(&body)
        .icon("shelldeck")
        .show()?;
    Ok(())
}

#[derive(Clone, Default)]
struct WorkspaceSlot(Rc<RefCell<Option<gpui::WeakEntity<Workspace>>>>);

impl WorkspaceSlot {
    fn set(&self, workspace: &gpui::Entity<Workspace>) {
        *self.0.borrow_mut() = Some(workspace.downgrade());
    }

    fn upgrade(&self) -> Option<gpui::Entity<Workspace>> {
        self.0.borrow().as_ref().and_then(gpui::WeakEntity::upgrade)
    }
}

/// Application-level owner for companion services and auxiliary windows.
///
/// This deliberately contains no `Workspace`: tray state, global-shortcut
/// routing, the AI controller, and Dock/palette window handles remain usable
/// while the main application surface is absent.
struct CompanionRuntime {
    main_window: gpui::AnyWindowHandle,
    ai_companion: gpui::Entity<AiCompanionController>,
    desktop_character: gpui::Entity<DesktopCharacterRuntime>,
    tray_state_tx: Option<tokio::sync::mpsc::UnboundedSender<tray::TrayState>>,
    companion_config_tx: tokio::sync::mpsc::UnboundedSender<(CompanionConfig, ClippyConfig)>,
    ai_dock_window: Option<gpui::WindowHandle<AiDockView>>,
    command_palette_window: Option<gpui::WindowHandle<CommandPaletteWindowView>>,
}

impl CompanionRuntime {
    fn command_for_hotkey(id: u32) -> Option<CompanionCommand> {
        match id {
            AI_DOCK_GLOBAL_HOTKEY_ID => Some(CompanionCommand::ToggleAiDock),
            COMMAND_PALETTE_GLOBAL_HOTKEY_ID => Some(CompanionCommand::ToggleCommandPalette),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CompanionCommand {
    ToggleAiDock,
    ToggleCommandPalette,
}

/// Lightweight root kept in the hidden main window until a surface actually
/// needs the full application. It lets the tray and global shortcuts stay
/// alive without constructing every Workspace view and network poller.
struct CompanionRoot {
    config: Option<AppConfig>,
    runtime: CompanionRuntime,
    workspace: Option<gpui::Entity<Workspace>>,
    workspace_slot: WorkspaceSlot,
    /// Shared with the registration runtime rather than snapshotted at boot.
    /// A Wayland portal answers asynchronously, and in tray mode (`start_hidden`)
    /// no Workspace exists yet to receive that answer — the result lands here
    /// and would be lost if the Workspace were later seeded from a snapshot
    /// taken before the portal replied.
    shortcut_state: Rc<RefCell<GlobalShortcutRegistrationState>>,
    _ai_companion_sub: gpui::Subscription,
}

impl CompanionRoot {
    fn new(
        config: AppConfig,
        workspace_slot: WorkspaceSlot,
        tray_state_tx: Option<tokio::sync::mpsc::UnboundedSender<tray::TrayState>>,
        companion_config_tx: tokio::sync::mpsc::UnboundedSender<(CompanionConfig, ClippyConfig)>,
        shortcut_state: Rc<RefCell<GlobalShortcutRegistrationState>>,
        main_window: gpui::AnyWindowHandle,
        cx: &mut gpui::Context<Self>,
    ) -> Self {
        let ai_companion =
            cx.new(|cx| AiCompanionController::new(config.ai.clone(), config.clippy.clone(), cx));
        let desktop_character = cx.new(|_cx| DesktopCharacterRuntime::new(config.clippy.clone()));
        let ai_companion_sub = cx.subscribe(
            &ai_companion,
            |this, _controller, event: &AiCompanionEvent, cx| {
                this.route_ai_companion_event(event.clone(), cx);
            },
        );
        Self {
            config: Some(config),
            runtime: CompanionRuntime {
                main_window,
                ai_companion,
                desktop_character,
                tray_state_tx,
                companion_config_tx,
                ai_dock_window: None,
                command_palette_window: None,
            },
            workspace: None,
            workspace_slot,
            shortcut_state,
            _ai_companion_sub: ai_companion_sub,
        }
    }

    fn route_ai_companion_event(&mut self, event: AiCompanionEvent, cx: &mut gpui::Context<Self>) {
        let main_window = self.runtime.main_window;
        // The palette is a window of its own, so it must not drag the whole
        // workspace forward with it.
        //
        // Deferred through `spawn` on purpose: this handler already runs inside
        // a `CompanionRoot` update, and `toggle_companion_command_palette`
        // re-enters the same entity to read its window handle. Calling it
        // synchronously panics with "cannot update CompanionRoot while it is
        // already being updated".
        if matches!(event, AiCompanionEvent::OpenPalette) {
            let root = cx.entity().downgrade();
            cx.spawn(async move |_this, cx: &mut gpui::AsyncApp| {
                let _ = cx.update(|cx| {
                    toggle_companion_command_palette(root, main_window, cx);
                });
            })
            .detach();
            return;
        }
        let opens_workspace = matches!(
            event,
            AiCompanionEvent::ApplyAction(_)
                | AiCompanionEvent::OpenSettings
                | AiCompanionEvent::OpenMainWindow
        );
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let _ = main_window.update(cx, |_, window, cx| {
                if opens_workspace {
                    window.show_window();
                    window.activate_window();
                }
                let _ = this.update(cx, |root, cx| {
                    if opens_workspace {
                        if let Some(dock) = root.runtime.ai_dock_window.take() {
                            let _ = dock.update(cx, |_, window, _| window.remove_window());
                        }
                    }
                    let workspace = root.ensure_workspace(window, cx);
                    workspace.update(cx, |workspace, cx| {
                        workspace.handle_ai_companion_event(event, cx);
                    });
                });
            });
        })
        .detach();
    }

    fn ensure_workspace(
        &mut self,
        window: &mut gpui::Window,
        cx: &mut gpui::Context<Self>,
    ) -> gpui::Entity<Workspace> {
        if let Some(workspace) = &self.workspace {
            return workspace.clone();
        }

        tracing::info!("initializing full Workspace on first main-surface demand");
        let config = self
            .config
            .take()
            .expect("CompanionRoot config consumed before Workspace creation");
        let sync_on_startup =
            config.cloud_sync.is_configured() && config.cloud_sync.sync_on_startup;
        let (connections, store) = load_workspace_data();
        let controller = self.runtime.ai_companion.read(cx);
        let ai_dock_assistant = controller.assistant();
        let ai_tasks = controller.tasks();
        let ai_companion_config = controller.shared_config();
        let clippy_companion_config = controller.shared_clippy_config();
        let workspace = cx.new(|cx| {
            Workspace::new(
                cx,
                config,
                connections,
                store,
                WorkspaceAiBindings {
                    assistant: ai_dock_assistant,
                    tasks: ai_tasks,
                    config: ai_companion_config,
                    clippy_config: clippy_companion_config,
                },
            )
        });

        workspace.update(cx, |ws, cx| {
            ws.start_git_polling(window.window_handle(), cx);
        });
        workspace.read(cx).focus_handle.focus(window);
        workspace.update(cx, |ws, cx| ws.restore_session(cx));
        workspace.update(cx, |ws, cx| ws.check_account_on_startup(cx));
        workspace.update(cx, |ws, cx| ws.activate_current_mode(cx));
        if sync_on_startup {
            workspace.update(cx, |ws, cx| ws.cloud_sync_on_startup(cx));
        }

        let companion_config_tx = self.runtime.companion_config_tx.clone();
        workspace.update(cx, |ws, _cx| {
            ws.set_companion_config_publisher(Box::new(move |companion, clippy| {
                if companion_config_tx.send((companion, clippy)).is_err() {
                    tracing::debug!("companion config dropped during shutdown");
                }
            }));
        });
        let shortcut_statuses = self.shortcut_state.borrow().statuses();
        workspace.update(cx, |ws, cx| {
            ws.set_companion_shortcut_statuses(shortcut_statuses, cx);
        });

        if let Some(state_tx) = self.runtime.tray_state_tx.clone() {
            workspace.update(cx, |ws, cx| {
                ws.set_tray_state_publisher(Box::new(move |counters| {
                    let state = tray::TrayState {
                        active_ssh: counters.active_ssh,
                        open_tunnels: counters.open_tunnels,
                        unread_tickets: counters.unread_tickets,
                        jean_pending: counters.jean_pending,
                        ai_tasks_running: counters.ai_tasks_running,
                        pinned_connections: counters
                            .pinned_connections
                            .into_iter()
                            .map(|connection| tray::PinnedConnection {
                                id: connection.id,
                                name: connection.name,
                            })
                            .collect(),
                        labels: tray::TrayLabels::localized(),
                    };
                    if let Err(error) = state_tx.send(state) {
                        tracing::debug!(
                            error = %error,
                            "tray state dropped because its worker stopped"
                        );
                    }
                }));
                ws.set_tray_notifier(Box::new(|notification| {
                    std::thread::spawn(move || {
                        if let Err(error) = show_tray_notification(notification) {
                            tracing::warn!("OS notification failed: {error}");
                        }
                    });
                }));
                ws.publish_tray_state(cx);
            });
        }

        self.workspace_slot.set(&workspace);
        self.workspace = Some(workspace.clone());
        cx.notify();
        workspace
    }

    fn companion_ui_font_family(&self, cx: &gpui::App) -> Option<String> {
        if let Some(workspace) = &self.workspace {
            return workspace.read(cx).companion_ui_font_family();
        }
        self.config.as_ref().and_then(|config| {
            (config.general.ui_font_family != "System Default")
                .then(|| config.general.ui_font_family.clone())
        })
    }
}

impl gpui::Render for CompanionRoot {
    fn render(
        &mut self,
        _window: &mut gpui::Window,
        _cx: &mut gpui::Context<Self>,
    ) -> impl gpui::IntoElement {
        let mut root = div().size_full();
        if let Some(workspace) = &self.workspace {
            root = root.child(workspace.clone());
        }
        root
    }
}

/// Route a menu-click coming out of the system tray onto the workspace.
/// Runs on the GPUI foreground thread — safe to touch `App` state.
fn dispatch_tray_command(
    cmd: tray::TrayCommand,
    root: gpui::WeakEntity<CompanionRoot>,
    window: gpui::AnyWindowHandle,
    cx: &mut gpui::App,
) {
    use tray::TrayCommand;
    match cmd {
        TrayCommand::ShowWindow => {
            // Restore + focus the main window (or a no-op if already
            // frontmost). `activate_window` is the portable way to say
            // "give this window the OS focus".
            if let Err(error) = window.update(cx, |_, window, cx| {
                if let Some(root) = root.upgrade() {
                    root.update(cx, |root, cx| {
                        root.ensure_workspace(window, cx);
                    });
                }
                window.show_window();
                window.activate_window();
            }) {
                tracing::warn!(error = %error, "tray could not show the main window");
            }
        }
        TrayCommand::ToggleAiDock => toggle_ai_dock(root, window, cx),
        TrayCommand::OpenAiTasks => show_ai_task_center(root, window, cx),
        TrayCommand::OpenClippy => show_clippy(root, window, cx),
        TrayCommand::OpenPalette => toggle_companion_command_palette(root, window, cx),
        TrayCommand::ChooseCharacter => {
            if let Err(error) = window.update(cx, |_, window, cx| {
                if let Some(root) = root.upgrade() {
                    let workspace = root.update(cx, |root, cx| root.ensure_workspace(window, cx));
                    workspace.update(cx, |workspace, cx| workspace.open_companion_settings(cx));
                    window.show_window();
                    window.activate_window();
                }
            }) {
                tracing::warn!(error = %error, "tray could not open character settings");
            }
        }
        TrayCommand::PauseCharacter => {
            route_desktop_character_command(root, window, CompanionRuntimeCommand::Pause, cx)
        }
        TrayCommand::ReturnCharacterToCorner => route_desktop_character_command(
            root,
            window,
            CompanionRuntimeCommand::ReturnToCorner,
            cx,
        ),
        TrayCommand::ConnectPinned(id) => {
            if let Err(error) = window.update(cx, |_, window, cx| {
                if let Some(root) = root.upgrade() {
                    let ws = root.update(cx, |root, cx| root.ensure_workspace(window, cx));
                    ws.update(cx, |ws, cx| ws.connect_pinned_connection(id, cx));
                    window.show_window();
                    window.activate_window();
                }
            }) {
                tracing::warn!(error = %error, "tray could not open a pinned connection");
            }
        }
        TrayCommand::Quit => {
            if let Some(root) = root.upgrade() {
                if let Some(ws) = root.read(cx).workspace.clone() {
                    ws.update(cx, |ws, cx| ws.shutdown(cx));
                }
            }
            cx.quit();
        }
    }
}

fn route_desktop_character_command(
    root: gpui::WeakEntity<CompanionRoot>,
    main_window: gpui::AnyWindowHandle,
    command: CompanionRuntimeCommand,
    cx: &mut gpui::App,
) {
    let Some(root) = root.upgrade() else {
        return;
    };
    let desktop_character = root.read(cx).runtime.desktop_character.clone();
    let runtime_entity = desktop_character.clone();
    desktop_character.update(cx, |runtime, cx| {
        runtime.handle_command(runtime_entity.clone(), command, main_window, cx);
        let diagnostics = runtime.diagnostics();
        if let Some(reason) = &diagnostics.reason {
            tracing::info!(?diagnostics.tier, %reason, "desktop character command handled with limitation");
        }
    });
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AiDockRequest {
    Toggle,
    Show,
    ShowTasks,
    ShowClippy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AiDockWindowAction {
    Create,
    Show,
    Hide,
}

fn ai_dock_window_action(visible: Option<bool>, request: AiDockRequest) -> AiDockWindowAction {
    match (visible, request) {
        (None, _) => AiDockWindowAction::Create,
        (Some(false), _)
        | (
            Some(true),
            AiDockRequest::Show | AiDockRequest::ShowTasks | AiDockRequest::ShowClippy,
        ) => AiDockWindowAction::Show,
        (Some(true), AiDockRequest::Toggle) => AiDockWindowAction::Hide,
    }
}

fn companion_main_window_visible(start_hidden: bool, tray_available: bool) -> bool {
    !start_hidden || !tray_available
}

fn workspace_created_at_boot(main_window_visible: bool) -> bool {
    main_window_visible
}

const AI_DOCK_GLOBAL_HOTKEY_ID: u32 = 1;
const COMMAND_PALETTE_GLOBAL_HOTKEY_ID: u32 = 2;

#[cfg(test)]
fn ai_dock_global_shortcut() -> &'static str {
    CompanionConfig::default_global_shortcut()
}

#[cfg(test)]
fn command_palette_global_shortcut() -> &'static str {
    CompanionConfig::default_global_palette_shortcut()
}

trait GlobalHotkeyRegistry {
    fn compositor_name(&self) -> &'static str;
    fn register(&self, id: u32, keystroke: &gpui::Keystroke) -> Result<()>;
    fn unregister(&self, id: u32);
}

impl GlobalHotkeyRegistry for gpui::App {
    fn compositor_name(&self) -> &'static str {
        gpui::App::compositor_name(self)
    }

    fn register(&self, id: u32, keystroke: &gpui::Keystroke) -> Result<()> {
        gpui::App::register_global_hotkey(self, id, keystroke)
    }

    fn unregister(&self, id: u32) {
        gpui::App::unregister_global_hotkey(self, id);
    }
}

#[derive(Debug, PartialEq, Eq)]
struct RegisteredGlobalShortcut {
    shortcut: Option<String>,
    status: ShortcutRegistrationStatus,
}

impl Default for RegisteredGlobalShortcut {
    fn default() -> Self {
        Self {
            shortcut: None,
            status: ShortcutRegistrationStatus::Disabled,
        }
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
struct GlobalShortcutRegistrationState {
    ai_dock: RegisteredGlobalShortcut,
    command_palette: RegisteredGlobalShortcut,
}

impl GlobalShortcutRegistrationState {
    fn sync(&mut self, config: &CompanionConfig, registry: &impl GlobalHotkeyRegistry) {
        let conflict = config.global_shortcut_enabled
            && config.global_palette_shortcut_enabled
            && config.global_shortcut == config.global_palette_shortcut;
        if conflict {
            sync_global_shortcut(
                &mut self.command_palette,
                false,
                COMMAND_PALETTE_GLOBAL_HOTKEY_ID,
                &config.global_palette_shortcut,
                "command palette",
                registry,
            );
            self.command_palette.status = ShortcutRegistrationStatus::Conflict;
        }
        sync_global_shortcut(
            &mut self.ai_dock,
            config.global_shortcut_enabled,
            AI_DOCK_GLOBAL_HOTKEY_ID,
            &config.global_shortcut,
            "AI Dock",
            registry,
        );
        if !conflict {
            sync_global_shortcut(
                &mut self.command_palette,
                config.global_palette_shortcut_enabled,
                COMMAND_PALETTE_GLOBAL_HOTKEY_ID,
                &config.global_palette_shortcut,
                "command palette",
                registry,
            );
        }
    }

    fn statuses(&self) -> CompanionShortcutStatuses {
        CompanionShortcutStatuses {
            ai_dock: self.ai_dock.status.clone(),
            command_palette: self.command_palette.status.clone(),
        }
    }

    fn apply_portal_result(&mut self, event: gpui::GlobalHotkeyRegistrationEvent) {
        let (id, shortcut, status) = match event {
            gpui::GlobalHotkeyRegistrationEvent::Registered { id, shortcut } => {
                (id, shortcut, ShortcutRegistrationStatus::Registered)
            }
            gpui::GlobalHotkeyRegistrationEvent::Failed {
                id,
                shortcut,
                error,
            } => (id, shortcut, ShortcutRegistrationStatus::Error(error)),
        };
        let registration = match id {
            AI_DOCK_GLOBAL_HOTKEY_ID => &mut self.ai_dock,
            COMMAND_PALETTE_GLOBAL_HOTKEY_ID => &mut self.command_palette,
            _ => return,
        };
        if registration.shortcut.as_deref() == Some(shortcut.as_str()) {
            registration.status = status;
        }
    }
}

fn sync_global_shortcut(
    registration: &mut RegisteredGlobalShortcut,
    enabled: bool,
    id: u32,
    shortcut: &str,
    label: &str,
    registry: &impl GlobalHotkeyRegistry,
) {
    if !enabled {
        if registration.shortcut.take().is_some() {
            registry.unregister(id);
            tracing::info!(shortcut, "{label} global shortcut unregistered");
        }
        registration.status = ShortcutRegistrationStatus::Disabled;
        return;
    }
    if registration.shortcut.as_deref() == Some(shortcut) {
        return;
    }
    if registration.shortcut.take().is_some() {
        registry.unregister(id);
    }

    let keystroke = match gpui::Keystroke::parse(shortcut) {
        Ok(keystroke) => keystroke,
        Err(error) => {
            tracing::error!("invalid {label} global shortcut: {error}");
            registration.status = ShortcutRegistrationStatus::Error(error.to_string());
            return;
        }
    };
    match registry.register(id, &keystroke) {
        Ok(()) => {
            registration.shortcut = Some(shortcut.to_string());
            if registry.compositor_name() == "Wayland" {
                registration.status = ShortcutRegistrationStatus::PendingPortal;
                tracing::info!(
                    shortcut,
                    "{label} global shortcut requested through Wayland portal"
                );
            } else {
                registration.status = ShortcutRegistrationStatus::Registered;
                tracing::info!(shortcut, "{label} global shortcut registered");
            }
        }
        Err(error) => {
            registration.status = ShortcutRegistrationStatus::Error(error.to_string());
            tracing::warn!(shortcut, "{label} global shortcut unavailable: {error:#}");
        }
    }
}

fn companion_pointer(
    window_bounds: gpui::Bounds<gpui::Pixels>,
    mouse_position: gpui::Point<gpui::Pixels>,
    mouse_is_global: bool,
) -> gpui::Point<gpui::Pixels> {
    if mouse_is_global {
        mouse_position
    } else {
        window_bounds.origin + mouse_position
    }
}

#[derive(Clone)]
struct CompanionDisplay {
    bounds: gpui::Bounds<gpui::Pixels>,
    id: gpui::DisplayId,
}

#[cfg(target_os = "linux")]
fn parse_xrandr_monitor_geometry(
    geometry: &str,
    scale_factor: f32,
) -> Option<gpui::Bounds<gpui::Pixels>> {
    let (width, after_width_mm) = geometry.split_once('/')?;
    let (_, after_x) = after_width_mm.split_once('x')?;
    let (height, after_height_mm) = after_x.split_once('/')?;
    let first_sign = after_height_mm.find(['+', '-'])?;
    let origins = &after_height_mm[first_sign..];
    let second_sign = origins[1..].find(['+', '-'])? + 1;
    let (origin_x, origin_y) = origins.split_at(second_sign);
    let scale_factor = scale_factor.max(1.0);
    Some(gpui::Bounds {
        origin: gpui::point(
            gpui::px(origin_x.parse::<f32>().ok()? / scale_factor),
            gpui::px(origin_y.parse::<f32>().ok()? / scale_factor),
        ),
        size: gpui::size(
            gpui::px(width.parse::<f32>().ok()? / scale_factor),
            gpui::px(height.parse::<f32>().ok()? / scale_factor),
        ),
    })
}

#[cfg(target_os = "linux")]
fn x11_monitor_bounds(scale_factor: f32) -> Vec<gpui::Bounds<gpui::Pixels>> {
    let output = match std::process::Command::new("xrandr")
        .arg("--listactivemonitors")
        .output()
    {
        Ok(output) => output,
        Err(error) => {
            tracing::warn!(error = %error, "could not query X11 monitors with xrandr");
            return Vec::new();
        }
    };
    if !output.status.success() {
        tracing::warn!(status = %output.status, "xrandr monitor query failed");
        return Vec::new();
    }
    let monitors = String::from_utf8_lossy(&output.stdout)
        .lines()
        .skip(1)
        .filter_map(|line| {
            line.split_whitespace()
                .find_map(|token| parse_xrandr_monitor_geometry(token, scale_factor))
        })
        .collect::<Vec<_>>();
    if monitors.is_empty() {
        tracing::warn!("xrandr returned no parseable active monitor");
    }
    monitors
}

#[cfg(target_os = "linux")]
fn parse_x11_workarea(properties: &str, scale_factor: f32) -> Option<gpui::Bounds<gpui::Pixels>> {
    let desktop = properties
        .lines()
        .find(|line| line.starts_with("_NET_CURRENT_DESKTOP"))
        .and_then(|line| line.split_once('='))
        .and_then(|(_, value)| value.trim().parse::<usize>().ok())
        .unwrap_or(0);
    let values = properties
        .lines()
        .find(|line| line.starts_with("_NET_WORKAREA"))?
        .split_once('=')?
        .1
        .split(',')
        .filter_map(|value| value.trim().parse::<f32>().ok())
        .collect::<Vec<_>>();
    let offset = desktop.checked_mul(4)?;
    let values = values.get(offset..offset + 4)?;
    let scale_factor = scale_factor.max(1.0);
    Some(gpui::Bounds {
        origin: gpui::point(
            gpui::px(values[0] / scale_factor),
            gpui::px(values[1] / scale_factor),
        ),
        size: gpui::size(
            gpui::px(values[2] / scale_factor),
            gpui::px(values[3] / scale_factor),
        ),
    })
}

#[cfg(target_os = "linux")]
fn x11_workarea(scale_factor: f32) -> Option<gpui::Bounds<gpui::Pixels>> {
    let output = match std::process::Command::new("xprop")
        .args(["-root", "_NET_CURRENT_DESKTOP", "_NET_WORKAREA"])
        .output()
    {
        Ok(output) => output,
        Err(error) => {
            tracing::warn!(error = %error, "could not query the X11 work area with xprop");
            return None;
        }
    };
    if !output.status.success() {
        tracing::warn!(status = %output.status, "xprop work-area query failed");
        return None;
    }
    let workarea = parse_x11_workarea(&String::from_utf8_lossy(&output.stdout), scale_factor);
    if workarea.is_none() {
        tracing::warn!("xprop returned an unparseable X11 work area");
    }
    workarea
}

fn companion_display(
    main_window: gpui::AnyWindowHandle,
    cx: &mut gpui::App,
) -> Option<CompanionDisplay> {
    let window_geometry = match main_window.update(cx, |_, window, _| {
        let bounds = window.window_bounds().get_bounds();
        let mouse = window.mouse_position();
        // X11 reports the pointer in root-window coordinates. Wayland,
        // macOS and Windows report it relative to the GPUI window.
        let mouse_is_global = is_x11_session();
        let scale_factor = window.scale_factor();
        let mut pointer = companion_pointer(bounds, mouse, mouse_is_global);
        if mouse_is_global {
            pointer = gpui::point(
                pointer.x * (1.0 / scale_factor),
                pointer.y * (1.0 / scale_factor),
            );
        }
        (pointer, bounds.center(), scale_factor)
    }) {
        Ok(geometry) => Some(geometry),
        Err(error) => {
            tracing::warn!(error = %error, "could not read the main window geometry for companion placement");
            None
        }
    };
    let displays = cx.displays();
    let platform_display = cx.primary_display().or_else(|| displays.first().cloned())?;

    #[cfg(target_os = "linux")]
    if is_x11_session() {
        if let Some((pointer, _, scale_factor)) = window_geometry {
            if let Some(monitor_bounds) = x11_monitor_bounds(scale_factor)
                .into_iter()
                .find(|bounds| bounds.contains(&pointer))
            {
                let bounds = x11_workarea(scale_factor)
                    .filter(|workarea| workarea.intersects(&monitor_bounds))
                    .map(|workarea| monitor_bounds.intersect(&workarea))
                    .unwrap_or(monitor_bounds);
                return Some(CompanionDisplay {
                    bounds,
                    id: platform_display.id(),
                });
            }
            tracing::warn!(?pointer, "no active XRandR monitor contains the pointer");
        }
    }

    window_geometry
        .and_then(|(pointer, main_center, _)| {
            displays
                .iter()
                .find(|display| display.bounds().contains(&pointer))
                .or_else(|| {
                    displays
                        .iter()
                        .find(|display| display.bounds().contains(&main_center))
                })
        })
        .map(|display| CompanionDisplay {
            bounds: display.bounds(),
            id: display.id(),
        })
        .or(Some(CompanionDisplay {
            bounds: platform_display.bounds(),
            id: platform_display.id(),
        }))
}

fn ai_dock_bounds(display_bounds: gpui::Bounds<gpui::Pixels>) -> gpui::Bounds<gpui::Pixels> {
    let width = gpui::px(480.0).min(display_bounds.size.width);
    gpui::Bounds {
        origin: gpui::point(display_bounds.right() - width, display_bounds.origin.y),
        size: gpui::size(width, display_bounds.size.height),
    }
}

fn command_palette_bounds(
    display_bounds: gpui::Bounds<gpui::Pixels>,
) -> gpui::Bounds<gpui::Pixels> {
    let width = gpui::px(620.0).min(display_bounds.size.width);
    let height = gpui::px(480.0).min(display_bounds.size.height);
    gpui::Bounds {
        origin: gpui::point(
            display_bounds.origin.x + (display_bounds.size.width - width) * 0.5,
            display_bounds.origin.y + (display_bounds.size.height - height) * 0.5,
        ),
        size: gpui::size(width, height),
    }
}

/// Toggle the single compact Assistant window owned by this process.
/// The application-level companion runtime retains the window handle so
/// repeated invocations reuse the same Dock.
fn toggle_ai_dock(
    root: gpui::WeakEntity<CompanionRoot>,
    main_window: gpui::AnyWindowHandle,
    cx: &mut gpui::App,
) {
    open_ai_dock(root, main_window, AiDockRequest::Toggle, cx);
}

/// Show and focus the Assistant Dock without toggling an already visible
/// window off. This is the idempotent entry point used by deep links.
fn show_ai_dock(
    root: gpui::WeakEntity<CompanionRoot>,
    main_window: gpui::AnyWindowHandle,
    cx: &mut gpui::App,
) {
    open_ai_dock(root, main_window, AiDockRequest::Show, cx);
}

fn show_ai_task_center(
    root: gpui::WeakEntity<CompanionRoot>,
    main_window: gpui::AnyWindowHandle,
    cx: &mut gpui::App,
) {
    open_ai_dock(root, main_window, AiDockRequest::ShowTasks, cx);
}

fn show_clippy(
    root: gpui::WeakEntity<CompanionRoot>,
    main_window: gpui::AnyWindowHandle,
    cx: &mut gpui::App,
) {
    open_ai_dock(root, main_window, AiDockRequest::ShowClippy, cx);
}

fn open_ai_dock(
    root: gpui::WeakEntity<CompanionRoot>,
    main_window: gpui::AnyWindowHandle,
    request: AiDockRequest,
    cx: &mut gpui::App,
) {
    use gpui::{WindowBounds, WindowDecorations, WindowKind, WindowOptions};

    let Some(root) = root.upgrade() else {
        return;
    };
    let (ai_companion, font_family, existing) = root.update(cx, |root, cx| {
        if let Some(workspace) = &root.workspace {
            workspace.update(cx, |workspace, cx| {
                workspace.close_ai_sheet_for_companion(cx);
            });
        }
        (
            root.runtime.ai_companion.clone(),
            root.companion_ui_font_family(cx),
            root.runtime.ai_dock_window,
        )
    });
    let Some(display) = companion_display(main_window, cx) else {
        tracing::error!("failed to open AI Dock: no display available");
        return;
    };
    if let Some(handle) = existing {
        let (recreate, clear_handle) = match handle.update(cx, |dock, window, cx| {
            match ai_dock_window_action(Some(window.is_window_visible()), request) {
                AiDockWindowAction::Show => {
                    if !display
                        .bounds
                        .contains(&window.window_bounds().get_bounds().center())
                    {
                        window.remove_window();
                        return (true, true);
                    }
                    ai_companion.update(cx, |controller, cx| match request {
                        AiDockRequest::ShowTasks => {
                            controller.prepare_task_center(cx);
                        }
                        AiDockRequest::ShowClippy => {
                            controller.prepare_clippy(cx);
                        }
                        AiDockRequest::Toggle | AiDockRequest::Show => {
                            controller.refresh(cx);
                        }
                    });
                    window.show_window();
                    window.activate_window();
                    dock.focus_composer(window, cx);
                    (false, false)
                }
                AiDockWindowAction::Hide => {
                    window.remove_window();
                    (false, true)
                }
                AiDockWindowAction::Create => unreachable!(),
            }
        }) {
            Ok(result) => result,
            Err(error) => {
                tracing::warn!(error = %error, "failed to update the existing AI Dock window");
                (true, true)
            }
        };
        if clear_handle {
            root.update(cx, |root, _| {
                root.runtime.ai_dock_window = None;
            });
        }
        if !recreate {
            return;
        }
    }

    let assistant = ai_companion.update(cx, |controller, cx| match request {
        AiDockRequest::ShowTasks => controller.prepare_task_center(cx),
        AiDockRequest::ShowClippy => controller.prepare_clippy(cx),
        AiDockRequest::Toggle | AiDockRequest::Show => controller.prepare(cx),
    });
    let bounds = ai_dock_bounds(display.bounds);
    let options = WindowOptions {
        titlebar: None,
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        kind: WindowKind::Overlay,
        is_movable: false,
        is_resizable: false,
        is_minimizable: false,
        display_id: Some(display.id),
        window_decorations: Some(WindowDecorations::Client),
        app_id: Some("shelldeck".to_string()),
        ..Default::default()
    };

    match cx.open_window(options, move |window, cx| {
        cx.new(|cx| AiDockView::new(assistant, main_window, font_family, window, cx))
    }) {
        Ok(handle) => {
            root.update(cx, |root, _| {
                root.runtime.ai_dock_window = Some(handle);
            });
            cx.activate(true);
            if let Err(error) = handle.update(cx, |dock, window, cx| {
                window.activate_window();
                dock.focus_composer(window, cx);
            }) {
                tracing::warn!(error = %error, "failed to activate the new AI Dock window");
            }
        }
        Err(error) => tracing::error!("failed to open AI Dock window: {error:#}"),
    }
}

fn toggle_companion_command_palette(
    root: gpui::WeakEntity<CompanionRoot>,
    main_window: gpui::AnyWindowHandle,
    cx: &mut gpui::App,
) {
    use gpui::{WindowBounds, WindowDecorations, WindowKind, WindowOptions};

    let Some(root) = root.upgrade() else {
        return;
    };
    let workspace = match main_window.update(cx, |_, window, cx| {
        root.update(cx, |root, cx| root.ensure_workspace(window, cx))
    }) {
        Ok(workspace) => workspace,
        Err(error) => {
            tracing::warn!(error = %error, "failed to initialize Workspace for command palette");
            return;
        }
    };
    let Some(display) = companion_display(main_window, cx) else {
        tracing::error!("failed to open command palette: no display available");
        return;
    };
    let existing = root.read(cx).runtime.command_palette_window;
    if let Some(handle) = existing {
        let (recreate, clear_handle) = match handle.update(cx, |palette, window, cx| {
            if window.is_window_visible() {
                window.remove_window();
                (false, true)
            } else if !display
                .bounds
                .contains(&window.window_bounds().get_bounds().center())
            {
                window.remove_window();
                (true, true)
            } else {
                workspace.update(cx, |workspace, cx| {
                    workspace.prepare_companion_command_palette(cx);
                });
                window.show_window();
                window.activate_window();
                palette.show(window, cx);
                (false, false)
            }
        }) {
            Ok(result) => result,
            Err(error) => {
                tracing::warn!(
                    error = %error,
                    "failed to update the existing command palette window"
                );
                (true, true)
            }
        };
        if clear_handle {
            root.update(cx, |root, _| {
                root.runtime.command_palette_window = None;
            });
        }
        if !recreate {
            return;
        }
    }

    let (palette, font_family) = workspace.update(cx, |workspace, cx| {
        (
            workspace.prepare_companion_command_palette(cx),
            workspace.companion_ui_font_family(),
        )
    });
    let bounds = command_palette_bounds(display.bounds);
    let options = WindowOptions {
        titlebar: None,
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        kind: WindowKind::Overlay,
        is_movable: false,
        is_resizable: false,
        is_minimizable: false,
        display_id: Some(display.id),
        window_decorations: Some(WindowDecorations::Client),
        app_id: Some("shelldeck".to_string()),
        ..Default::default()
    };

    match cx.open_window(options, move |window, cx| {
        let palette_window = window.window_handle();
        let view = cx.new(|cx| {
            CommandPaletteWindowView::new(
                palette,
                main_window,
                palette_window,
                font_family,
                window,
                cx,
            )
        });
        view.update(cx, |view, cx| view.show(window, cx));
        view
    }) {
        Ok(handle) => {
            root.update(cx, |root, _| {
                root.runtime.command_palette_window = Some(handle);
            });
            cx.activate(true);
            if let Err(error) = handle.update(cx, |_, window, _| window.activate_window()) {
                tracing::warn!(error = %error, "failed to activate the new command palette");
            }
        }
        Err(error) => tracing::error!("failed to open command palette window: {error:#}"),
    }
}

/// Parse + route a `shelldeck://…` payload (a bare focus ping arrives as an
/// empty string). The Assistant target stays entirely inside the lightweight
/// companion runtime; other targets bring the main window forward and route
/// through the Workspace. Runs on the GPUI foreground thread.
fn dispatch_deep_link(
    payload: String,
    root: gpui::WeakEntity<CompanionRoot>,
    window: gpui::AnyWindowHandle,
    cx: &mut gpui::App,
) {
    let link = DeepLink::parse(&payload);
    if matches!(link, Some(DeepLink::Assistant)) {
        show_ai_dock(root, window, cx);
        return;
    }
    if let Err(error) = window.update(cx, |_, window, cx| {
        window.show_window();
        window.activate_window();
        if let (Some(link), Some(root)) = (link, root.upgrade()) {
            let ws = root.update(cx, |root, cx| root.ensure_workspace(window, cx));
            ws.update(cx, |ws, cx| ws.open_deep_link(link, cx));
        }
    }) {
        tracing::warn!(error = %error, "deep link could not activate the main window");
    }
}

fn merge_workspace_connections(
    mut ssh_connections: Vec<Connection>,
    manual_connections: &[Connection],
) -> Vec<Connection> {
    for connection in manual_connections {
        if !ssh_connections
            .iter()
            .any(|existing| existing.alias == connection.alias)
        {
            ssh_connections.push(connection.clone());
        }
    }
    ssh_connections
}

/// Load data used only by the full application surface.
///
/// Keeping this behind `CompanionRoot::ensure_workspace` prevents a hidden
/// companion start from parsing SSH config or loading scripts, forwards and
/// managed connections that the Dock/tray runtime does not consume.
fn load_workspace_data() -> (Vec<Connection>, ConnectionStore) {
    tracing::info!("loading deferred Workspace connection data");
    let ssh_connections = parse_ssh_config().unwrap_or_else(|error| {
        tracing::warn!("Failed to parse SSH config: {error}");
        Vec::new()
    });
    tracing::info!(
        "Loaded {} connections from SSH config",
        ssh_connections.len()
    );

    let store = ConnectionStore::load().unwrap_or_else(|error| {
        tracing::warn!("Failed to load connection store: {error}");
        ConnectionStore::default()
    });
    tracing::info!(
        "Loaded {} manual connections, {} scripts, {} port forwards",
        store.connections.len(),
        store.scripts.len(),
        store.port_forwards.len()
    );
    let connections = merge_workspace_connections(ssh_connections, &store.connections);
    (connections, store)
}

fn main() -> Result<()> {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("shelldeck=info,warn")),
        )
        .init();

    tracing::info!("Starting ShellDeck v{}", shelldeck_core::VERSION);

    // Single-instance guard + deep-link hand-off. If the OS launched us to
    // follow a `shelldeck://…` link (or just to focus an existing window),
    // and another instance is already running, forward the URL to it and
    // exit — never spawn a duplicate window. Otherwise we become the
    // primary and hold the listener for the app lifetime.
    let deep_link_arg = std::env::args().skip(1).find(|a| DeepLink::looks_like(a));
    let primary = match single_instance::acquire(deep_link_arg.as_deref()) {
        Acquire::AlreadyRunning => {
            tracing::info!("another ShellDeck instance is running; forwarded request and exiting");
            return Ok(());
        }
        Acquire::Primary(p) => p,
    };
    // Bridge forwarded deep links directly into an async receiver. The IPC
    // thread blocks in accept() and wakes GPUI only when a link actually
    // arrives.
    let (deep_link_tx, deep_link_rx) = tokio::sync::mpsc::unbounded_channel();
    primary.listen_with(deep_link_arg, move |payload| {
        deep_link_tx.send(payload).is_ok()
    });

    // Load configuration
    let config = AppConfig::load().unwrap_or_else(|e| {
        tracing::warn!("Failed to load config, using defaults: {}", e);
        AppConfig::default()
    });

    shelldeck_ui::i18n::apply_ui_language(&config.general.ui_language);

    // Reconcile OS-level autostart with the persisted preference. The
    // Settings toggle is the source of truth; if the user manually
    // removed the .desktop entry (Linux), disabled the launchd item
    // (macOS), or the registry Run key was scrubbed, we re-enable so
    // the behaviour matches what they saw last. Best-effort: sandboxed
    // environments (Flatpak, Snap) may refuse — that's a warning, not
    // a fatal error.
    match shelldeck_core::config::autostart::apply(config.general.autostart) {
        Ok(actual) => tracing::info!("autostart reconciled: {actual}"),
        Err(e) => tracing::warn!("autostart reconcile skipped: {e}"),
    }

    // Start GPUI application
    Application::new().with_assets(Assets).run(move |cx| {
        // Initialize adabraka-ui
        adabraka_ui::init(cx);
        // Lucide subset — see crates/shelldeck/assets/icons/lucide/README.md
        adabraka_ui::set_icon_base_path("icons/lucide");
        // Real text-input widget from adabraka: registers keybindings (Backspace,
        // arrows, Home/End, Ctrl/Cmd-A/C/V/X, …) inside the "Input" context so
        // that focused `Input::new(...)` widgets get proper cursor + editing.
        adabraka_ui::components::input::init(cx);

        // Install theme — resolve the configured preference into a full palette,
        // then hand a matching adabraka Theme (with tokens overridden by the
        // ShellDeck palette) to the component library.
        ShellDeckColors::set_theme(&config.theme);
        install_theme(cx, shelldeck_ui::theme::adabraka_theme_from_palette());

        // Register keyboard shortcuts
        actions::register_keybindings(cx);

        // Platform callbacks intentionally only enqueue a small ID. Routing
        // back into GPUI happens from the foreground loop after the Workspace
        // and its window handles exist.
        let (global_hotkey_tx, global_hotkey_rx) = tokio::sync::mpsc::unbounded_channel();
        cx.on_global_hotkey(move |id| {
            tracing::debug!(id, "global hotkey callback received");
            if let Err(error) = global_hotkey_tx.send(id) {
                tracing::error!(id, error = %error, "global hotkey dispatch channel closed");
            }
        });
        let (global_hotkey_registration_tx, global_hotkey_registration_rx) =
            tokio::sync::mpsc::unbounded_channel();
        cx.on_global_hotkey_registration(move |event| {
            if global_hotkey_registration_tx.send(event).is_err() {
                tracing::debug!("global hotkey registration result dropped during shutdown");
            }
        });
        let mut initial_global_shortcut_state = GlobalShortcutRegistrationState::default();
        initial_global_shortcut_state.sync(&config.companion, cx);
        let global_shortcut_state = Rc::new(RefCell::new(initial_global_shortcut_state));
        let (companion_config_tx, companion_config_rx) =
            tokio::sync::mpsc::unbounded_channel::<(CompanionConfig, ClippyConfig)>();

        // Open main window
        let mut window_options = WindowOptions {
            titlebar: Some(TitlebarOptions {
                title: Some("ShellDeck".into()),
                appears_transparent: true,
                traffic_light_position: None,
            }),
            window_bounds: Some(WindowBounds::Windowed(Bounds {
                origin: Point::default(),
                size: size(px(1200.0), px(800.0)),
            })),
            is_resizable: true,
            window_min_size: Some(size(px(600.0), px(400.0))),
            window_decorations: Some(WindowDecorations::Client),
            // Must match packaging/linux/shelldeck.desktop (basename + StartupWMClass)
            // so GNOME/Wayland can pair the window with the .desktop and pick up the icon.
            app_id: Some("shelldeck".to_string()),
            ..Default::default()
        };

        // System tray. Ships the app's "companion" presence: menu with
        // Show / Palette / Quit, live counters (phase B), OS notifs
        // (phase C). Best-effort — if the tray backend refuses (Flatpak
        // sandbox, headless container, minimal WM) the app still runs.
        // Must be constructed on the main thread (GTK requirement on
        // Linux); running inside the GPUI closure satisfies that.
        // (cmd_rx, state_tx) — Some when the tray came up, None when
        // the backend refused. state_tx feeds the live-counter row
        // updates from the workspace side.
        let tray_handles = match tray::TrayService::new() {
            Ok(mut svc) => {
                svc.start_state_updates(cx);
                let cmd_rx = svc.take_receiver();
                let state_tx = svc.take_state_sender();
                drop(svc);
                Some((cmd_rx, state_tx))
            }
            Err(e) => {
                tracing::warn!("system tray unavailable: {e:#}");
                None
            }
        };
        let tray_available = tray_handles.is_some();
        window_options.show =
            companion_main_window_visible(config.companion.start_hidden, tray_available);
        let start_hidden = !window_options.show;
        if start_hidden {
            tracing::info!("companion start: main window hidden in system tray");
        } else if config.companion.start_hidden {
            tracing::warn!(
                "companion start requested but tray is unavailable; showing main window"
            );
        }

        let create_workspace_immediately = workspace_created_at_boot(window_options.show);
        let (tray_rx, tray_state_tx) = match tray_handles {
            Some((rx, state_tx)) => (Some(rx), Some(state_tx)),
            None => (None, None),
        };
        let workspace_slot = WorkspaceSlot::default();

        match cx.open_window(window_options, |window, cx| {
            let main_window = window.window_handle();
            let root = cx.new(|cx| {
                CompanionRoot::new(
                    config.clone(),
                    workspace_slot.clone(),
                    tray_state_tx,
                    companion_config_tx,
                    global_shortcut_state.clone(),
                    main_window,
                    cx,
                )
            });
            if create_workspace_immediately {
                root.update(cx, |root, cx| {
                    root.ensure_workspace(window, cx);
                });
            }
            root.update(cx, |root, cx| {
                let desktop_character = root.runtime.desktop_character.clone();
                let runtime_entity = desktop_character.clone();
                desktop_character.update(cx, |runtime, cx| {
                    runtime.apply_config(
                        runtime_entity.clone(),
                        config.clippy.clone(),
                        main_window,
                        cx,
                    );
                    if let Some(reason) = &runtime.diagnostics().reason {
                        tracing::info!(
                            tier = ?runtime.diagnostics().tier,
                            %reason,
                            "desktop character startup capability limitation"
                        );
                    }
                });
            });

            // Route tray menu clicks through the lightweight root. Commands
            // that need application state initialize the Workspace once.
            if let Some(rx) = tray_rx {
                let mut rx = rx;
                let root_handle = root.downgrade();
                let window_handle = window.window_handle();
                cx.spawn(async move |cx| {
                    while let Some(cmd) = rx.recv().await {
                        let root_handle = root_handle.clone();
                        if let Err(error) = cx.update(|cx| {
                            dispatch_tray_command(cmd, root_handle, window_handle, cx);
                        }) {
                            tracing::debug!(error = %error, "tray command dropped during shutdown");
                            break;
                        }
                    }
                })
                .detach();
            }

            // Deep-link dispatch waits until the single-instance listener
            // forwards a URL, then routes it onto the workspace.
            {
                let mut deep_link_rx = deep_link_rx;
                let root_handle = root.downgrade();
                let window_handle = window.window_handle();
                cx.spawn(async move |cx| {
                    while let Some(payload) = deep_link_rx.recv().await {
                        let root_handle = root_handle.clone();
                        if let Err(error) = cx.update(|cx| {
                            dispatch_deep_link(payload, root_handle, window_handle, cx);
                        }) {
                            tracing::debug!(error = %error, "deep link dropped during shutdown");
                            break;
                        }
                    }
                })
                .detach();
            }

            // Global shortcut dispatch. The platform callback above may run
            // outside GPUI's entity update context, hence this foreground
            // receiver mirrors the tray/deep-link routing loops.
            {
                let mut global_hotkey_rx = global_hotkey_rx;
                let root_handle = root.downgrade();
                let window_handle = window.window_handle();
                cx.spawn(async move |cx| {
                    while let Some(id) = global_hotkey_rx.recv().await {
                        let root_handle = root_handle.clone();
                        if let Err(error) =
                            cx.update(|cx| match CompanionRuntime::command_for_hotkey(id) {
                                Some(CompanionCommand::ToggleAiDock) => {
                                    toggle_ai_dock(root_handle, window_handle, cx);
                                }
                                Some(CompanionCommand::ToggleCommandPalette) => {
                                    toggle_companion_command_palette(
                                        root_handle,
                                        window_handle,
                                        cx,
                                    );
                                }
                                None => {}
                            })
                        {
                            tracing::debug!(
                                id,
                                error = %error,
                                "global hotkey dropped during shutdown"
                            );
                            break;
                        }
                    }
                })
                .detach();
            }

            // Settings persists the companion toggles inside the Workspace,
            // then publishes the changed slice here so the application-level
            // runtime can update native registrations immediately.
            {
                let mut companion_config_rx = companion_config_rx;
                let shortcut_workspace_slot = workspace_slot.clone();
                let global_shortcut_state = global_shortcut_state.clone();
                let root_handle = root.downgrade();
                let window_handle = window.window_handle();
                cx.spawn(async move |cx| {
                    while let Some((companion, clippy)) = companion_config_rx.recv().await {
                        let root_handle = root_handle.clone();
                        if let Err(error) = cx.update(|cx| {
                            global_shortcut_state.borrow_mut().sync(&companion, cx);
                            if let Some(root) = root_handle.upgrade() {
                                let desktop_character =
                                    root.read(cx).runtime.desktop_character.clone();
                                let runtime_entity = desktop_character.clone();
                                desktop_character.update(cx, |runtime, cx| {
                                    runtime.apply_config(
                                        runtime_entity.clone(),
                                        clippy.clone(),
                                        window_handle,
                                        cx,
                                    );
                                    if let Some(reason) = &runtime.diagnostics().reason {
                                        tracing::info!(
                                            tier = ?runtime.diagnostics().tier,
                                            %reason,
                                            "desktop character capability limitation"
                                        );
                                    }
                                });
                            }
                            if let Some(workspace) = shortcut_workspace_slot.upgrade() {
                                let statuses = global_shortcut_state.borrow().statuses();
                                workspace.update(cx, |workspace, cx| {
                                    workspace.set_companion_shortcut_statuses(statuses, cx);
                                });
                            }
                        }) {
                            tracing::debug!(
                                error = %error,
                                "companion config dropped during shutdown"
                            );
                            break;
                        }
                    }
                })
                .detach();
            }

            // Wayland portal acceptance/refusal arrives asynchronously after
            // the compositor permission dialog completes.
            {
                let mut global_hotkey_registration_rx = global_hotkey_registration_rx;
                let shortcut_workspace_slot = workspace_slot.clone();
                let global_shortcut_state = global_shortcut_state.clone();
                cx.spawn(async move |cx| {
                    while let Some(event) = global_hotkey_registration_rx.recv().await {
                        if let Err(error) = cx.update(|cx| {
                            global_shortcut_state
                                .borrow_mut()
                                .apply_portal_result(event);
                            if let Some(workspace) = shortcut_workspace_slot.upgrade() {
                                let statuses = global_shortcut_state.borrow().statuses();
                                workspace.update(cx, |workspace, cx| {
                                    workspace.set_companion_shortcut_statuses(statuses, cx);
                                });
                            }
                        }) {
                            tracing::debug!(
                                error = %error,
                                "global hotkey registration result dropped during shutdown"
                            );
                            break;
                        }
                    }
                })
                .detach();
            }

            // Intercept window close to honor the `confirm_before_close`
            // and `close_to_tray` settings. `close_to_tray` wins when
            // enabled + tray up: hide the window and return false so
            // the app stays alive in the tray.
            {
                let w = workspace_slot.clone();
                window.on_window_should_close(cx, move |window, cx| {
                    if let Some(ws) = w.upgrade() {
                        let hide = ws.read(cx).should_hide_to_tray();
                        if hide {
                            window.hide_window();
                            return false;
                        }
                        let should_close = ws.update(cx, |ws, cx| ws.confirm_window_close(cx));
                        if should_close {
                            ws.update(cx, |ws, cx| ws.shutdown(cx));
                            cx.quit();
                        }
                        should_close
                    } else {
                        true
                    }
                });
            }

            // Register global action handlers as a fallback in case the
            // element-level dispatch tree doesn't route actions properly
            // (e.g. nothing focused, focus on wrong element, etc.).
            {
                use actions::*;
                let w = workspace_slot.clone();
                cx.on_action({
                    let w = w.clone();
                    move |_: &NewTerminal, cx| {
                        if let Some(ws) = w.upgrade() {
                            ws.update(cx, |ws, cx| ws.open_new_terminal(cx));
                        }
                    }
                });
                cx.on_action({
                    let w = w.clone();
                    move |_: &ToggleSidebar, cx| {
                        if let Some(ws) = w.upgrade() {
                            ws.update(cx, |ws, cx| ws.toggle_sidebar(cx));
                        }
                    }
                });
                cx.on_action({
                    let w = w.clone();
                    move |_: &OpenSettings, cx| {
                        if let Some(ws) = w.upgrade() {
                            ws.update(cx, |ws, cx| {
                                ws.open_settings(cx);
                            });
                        }
                    }
                });
                cx.on_action({
                    let w = w.clone();
                    move |_: &OpenQuickConnect, cx| {
                        if let Some(ws) = w.upgrade() {
                            ws.update(cx, |ws, cx| {
                                ws.show_connection_form(None, cx);
                                cx.notify();
                            });
                        }
                    }
                });
                cx.on_action({
                    let w = w.clone();
                    move |_: &Quit, cx| {
                        if let Some(ws) = w.upgrade() {
                            ws.update(cx, |ws, cx| ws.shutdown(cx));
                        }
                        cx.quit();
                    }
                });
                cx.on_action({
                    let w = w.clone();
                    move |_: &NextTab, cx| {
                        if let Some(ws) = w.upgrade() {
                            ws.update(cx, |ws, cx| ws.next_tab(cx));
                        }
                    }
                });
                cx.on_action({
                    let w = w.clone();
                    move |_: &PrevTab, cx| {
                        if let Some(ws) = w.upgrade() {
                            ws.update(cx, |ws, cx| ws.prev_tab(cx));
                        }
                    }
                });
                cx.on_action({
                    let w = w.clone();
                    move |_: &CloseTab, cx| {
                        if let Some(ws) = w.upgrade() {
                            ws.update(cx, |ws, cx| ws.close_active_tab(cx));
                        }
                    }
                });
                cx.on_action({
                    let w = w.clone();
                    move |action: &ApplyTerminalTheme, cx| {
                        if let Some(ws) = w.upgrade() {
                            let name = action.name.clone();
                            ws.update(cx, |ws, cx| ws.apply_terminal_theme_by_name(&name, cx));
                        }
                    }
                });
                cx.on_action({
                    let w = w.clone();
                    move |_: &CloudSyncNow, cx| {
                        if let Some(ws) = w.upgrade() {
                            ws.update(cx, |ws, cx| ws.cloud_sync_now(cx));
                        }
                    }
                });
                cx.on_action({
                    let w = w.clone();
                    move |_: &SwitchSite, cx| {
                        if let Some(ws) = w.upgrade() {
                            ws.update(cx, |ws, cx| ws.open_site_switcher(cx));
                        }
                    }
                });
                cx.on_action({
                    let w = w.clone();
                    move |action: &OpenManageArea, cx| {
                        if let Some(ws) = w.upgrade() {
                            let path = action.path.clone();
                            ws.update(cx, |ws, cx| ws.open_manage_area(path, cx));
                        }
                    }
                });
                cx.on_action({
                    let w = w.clone();
                    move |action: &SetAppMode, cx| {
                        if let Some(ws) = w.upgrade() {
                            let mode = action.mode;
                            ws.update(cx, |ws, cx| ws.set_mode(mode, cx));
                        }
                    }
                });
                cx.on_action({
                    let w = w.clone();
                    move |_: &OpenJeanConsole, cx| {
                        if let Some(ws) = w.upgrade() {
                            ws.update(cx, |ws, cx| ws.open_jean_console(cx));
                        }
                    }
                });
                cx.on_action({
                    let w = w.clone();
                    move |_: &JeanTogglePause, cx| {
                        if let Some(ws) = w.upgrade() {
                            ws.update(cx, |ws, cx| ws.jean_toggle_pause(cx));
                        }
                    }
                });
                cx.on_action({
                    let w = w.clone();
                    move |_: &OpenFleet, cx| {
                        if let Some(ws) = w.upgrade() {
                            ws.update(cx, |ws, cx| ws.open_fleet(cx));
                        }
                    }
                });
                cx.on_action({
                    let w = w.clone();
                    move |_: &ToggleJeanRuntime, cx| {
                        if let Some(ws) = w.upgrade() {
                            ws.update(cx, |ws, cx| ws.toggle_jean_runtime(cx));
                        }
                    }
                });
                cx.on_action({
                    let w = w.clone();
                    move |_: &NewRequest, cx| {
                        if let Some(ws) = w.upgrade() {
                            ws.update(cx, |ws, cx| ws.open_new_request(cx));
                        }
                    }
                });
                cx.on_action({
                    let w = w.clone();
                    move |_: &OpenSupportRequests, cx| {
                        if let Some(ws) = w.upgrade() {
                            ws.update(cx, |ws, cx| ws.open_support_requests(cx));
                        }
                    }
                });
                cx.on_action({
                    let w = w.clone();
                    move |_: &OpenBextCloud, cx| {
                        if let Some(ws) = w.upgrade() {
                            ws.update(cx, |ws, cx| ws.open_bext_cloud(cx));
                        }
                    }
                });
                cx.on_action({
                    let w = w.clone();
                    move |_: &ConnectBextCloud, cx| {
                        if let Some(ws) = w.upgrade() {
                            ws.update(cx, |ws, cx| ws.connect_bext_cloud_action(cx));
                        }
                    }
                });
            }

            root
        }) {
            Ok(_) => {}
            Err(e) => {
                tracing::error!("Failed to open main window: {}", e);
                cx.quit();
            }
        }

        tracing::info!("ShellDeck window opened");
    });

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        ai_dock_bounds, ai_dock_global_shortcut, ai_dock_window_action, character_asset_paths,
        command_palette_bounds, command_palette_global_shortcut, companion_main_window_visible,
        companion_pointer, merge_workspace_connections, workspace_created_at_boot, AiDockRequest,
        AiDockWindowAction, Assets, CompanionCommand, CompanionRuntime, GlobalHotkeyRegistry,
        GlobalShortcutRegistrationState, ShortcutRegistrationStatus, AI_DOCK_GLOBAL_HOTKEY_ID,
        COMMAND_PALETTE_GLOBAL_HOTKEY_ID,
    };
    #[cfg(target_os = "linux")]
    use super::{parse_x11_workarea, parse_xrandr_monitor_geometry};
    use gpui::AssetSource;
    use shelldeck_core::config::app_config::CompanionConfig;
    use shelldeck_core::models::connection::Connection;
    use std::cell::{Cell, RefCell};

    #[derive(Default)]
    struct FakeGlobalHotkeyRegistry {
        calls: RefCell<Vec<(&'static str, u32)>>,
        fail_next_registration: Cell<bool>,
        wayland: bool,
    }

    impl GlobalHotkeyRegistry for FakeGlobalHotkeyRegistry {
        fn compositor_name(&self) -> &'static str {
            if self.wayland {
                "Wayland"
            } else {
                "Test"
            }
        }

        fn register(&self, id: u32, _keystroke: &gpui::Keystroke) -> anyhow::Result<()> {
            self.calls.borrow_mut().push(("register", id));
            if self.fail_next_registration.replace(false) {
                anyhow::bail!("simulated registration failure");
            }
            Ok(())
        }

        fn unregister(&self, id: u32) {
            self.calls.borrow_mut().push(("unregister", id));
        }
    }

    // SDTEST-1381
    #[test]
    fn ai_dock_toggle_reuses_the_existing_window() {
        assert_eq!(
            ai_dock_window_action(None, AiDockRequest::Toggle),
            AiDockWindowAction::Create
        );
        assert_eq!(
            ai_dock_window_action(Some(false), AiDockRequest::Toggle),
            AiDockWindowAction::Show
        );
        assert_eq!(
            ai_dock_window_action(Some(true), AiDockRequest::Toggle),
            AiDockWindowAction::Hide
        );
    }

    // SDTEST-1405
    #[test]
    fn assistant_deep_link_show_is_idempotent() {
        assert_eq!(
            ai_dock_window_action(None, AiDockRequest::Show),
            AiDockWindowAction::Create
        );
        assert_eq!(
            ai_dock_window_action(Some(false), AiDockRequest::Show),
            AiDockWindowAction::Show
        );
        assert_eq!(
            ai_dock_window_action(Some(true), AiDockRequest::Show),
            AiDockWindowAction::Show
        );
    }

    // SDTEST-1409
    #[test]
    fn task_center_request_always_shows_the_existing_dock() {
        assert_eq!(
            ai_dock_window_action(None, AiDockRequest::ShowTasks),
            AiDockWindowAction::Create
        );
        assert_eq!(
            ai_dock_window_action(Some(false), AiDockRequest::ShowTasks),
            AiDockWindowAction::Show
        );
        assert_eq!(
            ai_dock_window_action(Some(true), AiDockRequest::ShowTasks),
            AiDockWindowAction::Show
        );
    }

    // SDTEST-1383
    #[test]
    fn companion_hidden_start_requires_an_available_tray() {
        assert!(companion_main_window_visible(false, false));
        assert!(companion_main_window_visible(false, true));
        assert!(companion_main_window_visible(true, false));
        assert!(!companion_main_window_visible(true, true));
    }

    // SDTEST-1493
    #[test]
    fn every_authored_character_state_is_embedded_in_the_binary_asset_source() {
        let assets = Assets;
        let paths = character_asset_paths();
        let listed = assets.list("").expect("character asset list");

        assert_eq!(paths.len(), 48);
        for path in paths {
            assert!(listed.contains(&path), "asset list omitted {path}");
            assert!(
                assets
                    .load(path.as_ref())
                    .expect("embedded character asset load")
                    .is_some(),
                "asset bytes omitted {path}"
            );
        }
    }

    // SDTEST-1391
    #[test]
    fn hidden_companion_start_defers_workspace_creation() {
        assert!(workspace_created_at_boot(true));
        assert!(!workspace_created_at_boot(false));
    }

    // SDTEST-1393
    #[test]
    fn companion_runtime_owns_global_shortcut_routing() {
        assert_eq!(
            CompanionRuntime::command_for_hotkey(AI_DOCK_GLOBAL_HOTKEY_ID),
            Some(CompanionCommand::ToggleAiDock)
        );
        assert_eq!(
            CompanionRuntime::command_for_hotkey(COMMAND_PALETTE_GLOBAL_HOTKEY_ID),
            Some(CompanionCommand::ToggleCommandPalette)
        );
        assert_eq!(CompanionRuntime::command_for_hotkey(u32::MAX), None);
    }

    // SDTEST-1398
    #[test]
    fn companion_shortcuts_register_and_unregister_without_restart() {
        let registry = FakeGlobalHotkeyRegistry::default();
        let mut state = GlobalShortcutRegistrationState::default();
        let mut config = CompanionConfig::default();

        state.sync(&config, &registry);
        state.sync(&config, &registry);
        assert_eq!(
            registry.calls.borrow().as_slice(),
            [
                ("register", AI_DOCK_GLOBAL_HOTKEY_ID),
                ("register", COMMAND_PALETTE_GLOBAL_HOTKEY_ID),
            ]
        );

        config.global_shortcut_enabled = false;
        state.sync(&config, &registry);
        assert_eq!(
            registry.calls.borrow().last(),
            Some(&("unregister", AI_DOCK_GLOBAL_HOTKEY_ID))
        );
        assert!(state.ai_dock.shortcut.is_none());
        assert_eq!(state.ai_dock.status, ShortcutRegistrationStatus::Disabled);
        assert!(state.command_palette.shortcut.is_some());

        registry.fail_next_registration.set(true);
        config.global_shortcut_enabled = true;
        state.sync(&config, &registry);
        assert!(state.ai_dock.shortcut.is_none());
        assert!(matches!(
            state.ai_dock.status,
            ShortcutRegistrationStatus::Error(_)
        ));
        state.sync(&config, &registry);
        assert_eq!(
            state.ai_dock.shortcut.as_deref(),
            Some(config.global_shortcut.as_str())
        );
        assert_eq!(state.ai_dock.status, ShortcutRegistrationStatus::Registered);
        assert_eq!(
            registry
                .calls
                .borrow()
                .iter()
                .filter(|call| **call == ("register", AI_DOCK_GLOBAL_HOTKEY_ID))
                .count(),
            3
        );
    }

    // SDTEST-1402
    #[test]
    fn custom_shortcuts_replace_native_registration_and_surface_conflicts() {
        let registry = FakeGlobalHotkeyRegistry::default();
        let mut state = GlobalShortcutRegistrationState::default();
        let mut config = CompanionConfig::default();
        state.sync(&config, &registry);

        config.global_palette_shortcut = "ctrl-alt-p".to_string();
        state.sync(&config, &registry);
        assert_eq!(
            registry.calls.borrow().as_slice(),
            [
                ("register", AI_DOCK_GLOBAL_HOTKEY_ID),
                ("register", COMMAND_PALETTE_GLOBAL_HOTKEY_ID),
                ("unregister", COMMAND_PALETTE_GLOBAL_HOTKEY_ID),
                ("register", COMMAND_PALETTE_GLOBAL_HOTKEY_ID),
            ]
        );
        assert_eq!(
            state.command_palette.shortcut.as_deref(),
            Some("ctrl-alt-p")
        );

        config.global_palette_shortcut = config.global_shortcut.clone();
        state.sync(&config, &registry);
        assert!(state.command_palette.shortcut.is_none());
        assert_eq!(
            state.command_palette.status,
            ShortcutRegistrationStatus::Conflict
        );
    }

    // SDTEST-1403
    #[test]
    fn wayland_portal_result_replaces_pending_status() {
        let registry = FakeGlobalHotkeyRegistry {
            wayland: true,
            ..Default::default()
        };
        let mut state = GlobalShortcutRegistrationState::default();
        state.sync(&CompanionConfig::default(), &registry);
        assert_eq!(
            state.ai_dock.status,
            ShortcutRegistrationStatus::PendingPortal
        );

        state.apply_portal_result(gpui::GlobalHotkeyRegistrationEvent::Registered {
            id: AI_DOCK_GLOBAL_HOTKEY_ID,
            shortcut: CompanionConfig::default_global_shortcut().to_string(),
        });
        assert_eq!(state.ai_dock.status, ShortcutRegistrationStatus::Registered);
        state.apply_portal_result(gpui::GlobalHotkeyRegistrationEvent::Failed {
            id: COMMAND_PALETTE_GLOBAL_HOTKEY_ID,
            shortcut: CompanionConfig::default_global_palette_shortcut().to_string(),
            error: "permission denied".to_string(),
        });
        assert_eq!(
            state.command_palette.status,
            ShortcutRegistrationStatus::Error("permission denied".to_string())
        );
    }

    // SDTEST-1394
    #[test]
    fn deferred_workspace_data_merge_preserves_ssh_alias_precedence() {
        let ssh = Connection::new_manual(
            "shared".to_string(),
            "ssh.example.test".to_string(),
            "ssh-user".to_string(),
        );
        let duplicate = Connection::new_manual(
            "shared".to_string(),
            "manual.example.test".to_string(),
            "manual-user".to_string(),
        );
        let manual = Connection::new_manual(
            "manual".to_string(),
            "manual-only.example.test".to_string(),
            "manual-user".to_string(),
        );

        let merged = merge_workspace_connections(vec![ssh], &[duplicate, manual]);

        assert_eq!(merged.len(), 2);
        assert_eq!(merged[0].hostname, "ssh.example.test");
        assert_eq!(merged[1].alias, "manual");
    }

    #[test]
    fn ai_dock_is_anchored_to_the_display_right_edge() {
        let display = gpui::Bounds {
            origin: gpui::point(gpui::px(100.0), gpui::px(30.0)),
            size: gpui::size(gpui::px(1920.0), gpui::px(1050.0)),
        };

        let dock = ai_dock_bounds(display);

        assert_eq!(dock.origin, gpui::point(gpui::px(1540.0), gpui::px(30.0)));
        assert_eq!(dock.size, gpui::size(gpui::px(480.0), gpui::px(1050.0)));
        assert_eq!(dock.right(), display.right());
    }

    #[test]
    fn x11_pointer_coordinates_are_not_offset_twice() {
        let window = gpui::Bounds {
            origin: gpui::point(gpui::px(1920.0), gpui::px(0.0)),
            size: gpui::size(gpui::px(1200.0), gpui::px(800.0)),
        };
        let pointer = gpui::point(gpui::px(2500.0), gpui::px(400.0));

        assert_eq!(companion_pointer(window, pointer, true), pointer);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn xrandr_geometry_preserves_each_monitor_origin() {
        let primary = parse_xrandr_monitor_geometry("1920/598x1080/336+0+0", 1.0)
            .expect("primary monitor geometry");
        let secondary = parse_xrandr_monitor_geometry("1920/598x1080/336+1920+0", 1.0)
            .expect("secondary monitor geometry");

        assert_eq!(primary.origin, gpui::point(gpui::px(0.0), gpui::px(0.0)));
        assert_eq!(
            secondary.origin,
            gpui::point(gpui::px(1920.0), gpui::px(0.0))
        );
        assert_eq!(primary.size, secondary.size);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn x11_workarea_excludes_the_system_toolbar() {
        let properties = "_NET_CURRENT_DESKTOP(CARDINAL) = 0\n\
                          _NET_WORKAREA(CARDINAL) = 0, 32, 3840, 1048, 0, 32, 3840, 1048\n";
        let workarea = parse_x11_workarea(properties, 1.0).expect("desktop workarea");
        let right_monitor = gpui::Bounds {
            origin: gpui::point(gpui::px(1920.0), gpui::px(0.0)),
            size: gpui::size(gpui::px(1920.0), gpui::px(1080.0)),
        };

        let usable = right_monitor.intersect(&workarea);

        assert_eq!(usable.origin, gpui::point(gpui::px(1920.0), gpui::px(32.0)));
        assert_eq!(usable.size, gpui::size(gpui::px(1920.0), gpui::px(1048.0)));

        let missing_current_desktop =
            "_NET_CURRENT_DESKTOP:  not found.\n_NET_WORKAREA(CARDINAL) = 0, 32, 3840, 1048\n";
        assert_eq!(
            parse_x11_workarea(missing_current_desktop, 1.0),
            Some(workarea)
        );
    }

    #[test]
    fn command_palette_is_centered_inside_the_selected_display() {
        let display = gpui::Bounds {
            origin: gpui::point(gpui::px(1920.0), gpui::px(0.0)),
            size: gpui::size(gpui::px(1920.0), gpui::px(1080.0)),
        };

        let palette = command_palette_bounds(display);

        assert_eq!(
            palette.origin,
            gpui::point(gpui::px(2570.0), gpui::px(300.0))
        );
        assert_eq!(palette.size, gpui::size(gpui::px(620.0), gpui::px(480.0)));
    }

    #[test]
    fn ai_dock_global_shortcut_is_parseable() {
        gpui::Keystroke::parse(ai_dock_global_shortcut()).expect("valid global shortcut");
    }

    #[test]
    fn command_palette_global_shortcut_is_parseable() {
        gpui::Keystroke::parse(command_palette_global_shortcut())
            .expect("valid command palette global shortcut");
    }

    // SDTEST-1388
    #[test]
    fn reachable_dynamic_icons_are_embedded() {
        for name in [
            "arrow-right",
            "bot",
            "circle-alert",
            "circle-check",
            "play",
            "route",
            "triangle-alert",
        ] {
            let path = format!("icons/lucide/{name}.svg");
            assert!(
                super::lucide_bytes(&path).is_some(),
                "reachable icon is not embedded: {path}"
            );
        }
    }

    // SDTEST-1424
    #[test]
    fn contextual_monolith_animations_are_embedded_webp_assets() {
        for path in [
            "images/brand/webp/studies/monolith-thinking.webp",
            "images/brand/webp/studies/monolith-scan.webp",
            "images/brand/webp/studies/monolith-terminal-typing.webp",
        ] {
            let bytes = Assets
                .load(path)
                .expect("embedded asset lookup succeeds")
                .unwrap_or_else(|| panic!("contextual Monolith motion is not embedded: {path}"));

            assert!(
                bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP"),
                "contextual Monolith motion is not a WebP asset: {path}"
            );
        }
    }
}
