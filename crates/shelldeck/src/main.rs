mod actions;

use adabraka_ui::prelude::*;
use anyhow::Result;
use gpui::{AssetSource, SharedString, WindowDecorations};
use shelldeck_core::config::app_config::AppConfig;
use shelldeck_core::config::ssh_config::parse_ssh_config;
use shelldeck_core::config::store::ConnectionStore;
use shelldeck_ui::theme::ShellDeckColors;
use shelldeck_ui::Workspace;
use std::borrow::Cow;
use tracing_subscriber::EnvFilter;

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
    "arrow-down",
    "arrow-up",
    "calendar",
    "check",
    "check-check",
    "chevron-down",
    "chevron-left",
    "chevron-right",
    "chevron-up",
    "circle-alert",
    "circle-check",
    "circle-help",
    "clock",
    "copy",
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
    "inbox",
    "info",
    "key",
    "lock",
    "mail",
    "maximize-2",
    "minimize-2",
    "minus",
    "pencil",
    "pin",
    "plus",
    "refresh-cw",
    "reply",
    "search",
    "send",
    "server",
    "settings",
    "shield",
    "sticky-note",
    "tag",
    "terminal",
    "trash-2",
    "triangle-alert",
    "upload",
    "user",
    "user-check",
    "users",
    "x",
);

/// In-process asset source that ships a small set of images embedded in the
/// binary (see `assets/images/`). GPUI's `svg()` element requires an
/// `AssetSource` to resolve `.path("images/…")`.
struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        let path = path.strip_prefix('/').unwrap_or(path);
        let bytes: &'static [u8] = match path {
            "images/wd29-logo.svg" => include_bytes!("../assets/images/wd29-logo.svg"),
            // Filled brand icon: `currentColor` fills the rounded square, prompt
            // stays white. Use with `.text_color(ShellDeckColors::primary())`.
            "images/shelldeck-icon.svg" => include_bytes!("../assets/images/shelldeck-icon.svg"),
            // Outline monochrome mark, everything `currentColor`. Use when you
            // want the whole logo tinted with one color (e.g. muted footer).
            "images/shelldeck-mark.svg" => include_bytes!("../assets/images/shelldeck-mark.svg"),
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
            SharedString::from("images/shelldeck-mark.svg"),
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
        paths.extend(lucide_asset_paths());
        Ok(paths)
    }
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

    // Load configuration
    let config = AppConfig::load().unwrap_or_else(|e| {
        tracing::warn!("Failed to load config, using defaults: {}", e);
        AppConfig::default()
    });

    // Parse SSH config
    let ssh_connections = parse_ssh_config().unwrap_or_else(|e| {
        tracing::warn!("Failed to parse SSH config: {}", e);
        Vec::new()
    });

    tracing::info!(
        "Loaded {} connections from SSH config",
        ssh_connections.len()
    );

    // Load connection store (manual connections, scripts, forwards)
    let mut store = ConnectionStore::load().unwrap_or_else(|e| {
        tracing::warn!("Failed to load connection store: {}", e);
        ConnectionStore::default()
    });

    tracing::info!(
        "Loaded {} manual connections, {} scripts, {} port forwards",
        store.connections.len(),
        store.scripts.len(),
        store.port_forwards.len()
    );

    // Cloud Sync: pull remote SSH profiles at startup (best-effort). Network
    // failure never blocks launch — the fetch is bounded by 4s connect / 10s
    // total timeouts. On a successful merge we reload the store so the freshly
    // synced connections feed the workspace.
    if config.cloud_sync.is_configured() && config.cloud_sync.sync_on_startup {
        match shelldeck_core::config::cloud_sync::sync_now(
            &config.cloud_sync,
            shelldeck_core::VERSION,
        ) {
            Ok(stats) => {
                tracing::info!(
                    "Cloud sync: {} added, {} updated, {} removed",
                    stats.added,
                    stats.updated,
                    stats.removed
                );
                if stats.changed() {
                    match ConnectionStore::load() {
                        Ok(s) => store = s,
                        Err(e) => {
                            tracing::warn!("Failed to reload store after cloud sync: {}", e)
                        }
                    }
                }
            }
            Err(e) => tracing::warn!("Cloud sync failed: {}", e),
        }
    }

    // Keep store for passing to workspace
    let store_for_workspace = store.clone();

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

        // Combine SSH config connections with manual ones
        let all_connections = {
            let mut conns = ssh_connections;
            for manual_conn in &store.connections {
                if !conns.iter().any(|c| c.alias == manual_conn.alias) {
                    conns.push(manual_conn.clone());
                }
            }
            conns
        };

        // Open main window
        let window_options = WindowOptions {
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
            app_id: Some("com.shelldeck.desktop".to_string()),
            ..Default::default()
        };

        match cx.open_window(window_options, |window, cx| {
            let workspace = cx.new(|cx| {
                Workspace::new(
                    cx,
                    config.clone(),
                    all_connections,
                    store_for_workspace.clone(),
                )
            });
            // Focus the workspace root so keyboard shortcuts dispatch correctly
            workspace.read(cx).focus_handle.focus(window);

            // Restore the previous session's tabs when auto-connect-on-startup
            // is enabled. No-op when the setting is off, keeping default startup
            // (empty terminal view) unchanged.
            workspace.update(cx, |ws, cx| ws.restore_session(cx));

            // Background whoami to light up the titlebar account status dot and
            // refresh the account name (or flag a revoked token).
            workspace.update(cx, |ws, cx| ws.check_account_on_startup(cx));
            // Activate the persisted app mode (loads Support data + poll if the
            // last session was in Support mode).
            workspace.update(cx, |ws, cx| ws.activate_current_mode(cx));

            // Intercept window close to honor the `confirm_before_close` setting.
            {
                let w = workspace.downgrade();
                window.on_window_should_close(cx, move |_window, cx| {
                    if let Some(ws) = w.upgrade() {
                        ws.update(cx, |ws, cx| ws.confirm_window_close(cx))
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
                use shelldeck_ui::workspace::ActiveView;

                let w = workspace.downgrade();
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
                                ws.set_active_view(ActiveView::Settings);
                                cx.notify();
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

            workspace
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
