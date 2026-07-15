mod actions;
mod tray;

use adabraka_ui::prelude::*;
use anyhow::Result;
use gpui::{AssetSource, SharedString, WindowDecorations};
use shelldeck_core::config::app_config::AppConfig;
use shelldeck_core::config::deep_link::DeepLink;
use shelldeck_core::config::single_instance::{self, Acquire};
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
    "activity",
    "arrow-down",
    "arrow-up",
    "arrow-left-right",
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
    "inbox",
    "info",
    "key",
    "keyboard",
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
    "scroll-text",
    "send",
    "server",
    "settings",
    "shield",
    "shield-check",
    "sticky-note",
    "table",
    "tag",
    "terminal",
    "trash-2",
    "triangle-alert",
    "upload",
    "user",
    "user-check",
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
    use shelldeck_ui::TrayNotification;
    let (summary, body) = match n {
        TrayNotification::NewTickets { count } => (
            "ShellDeck — Support".to_string(),
            match count {
                1 => "1 nouveau ticket support".to_string(),
                n => format!("{n} nouveaux tickets support"),
            },
        ),
        TrayNotification::JeanPending { count } => (
            "ShellDeck — Jean".to_string(),
            match count {
                1 => "Un job Jean attend votre validation".to_string(),
                n => format!("{n} jobs Jean attendent votre validation"),
            },
        ),
        TrayNotification::SshDisconnected { count } => (
            "ShellDeck — SSH".to_string(),
            match count {
                1 => "Une connexion SSH s'est terminée".to_string(),
                n => format!("{n} connexions SSH se sont terminées"),
            },
        ),
        TrayNotification::FleetJobDone { success } => (
            "ShellDeck — Fleet".to_string(),
            if success {
                "Job Fleet terminé".to_string()
            } else {
                "Job Fleet échoué".to_string()
            },
        ),
    };
    notify_rust::Notification::new()
        .appname("ShellDeck")
        .summary(&summary)
        .body(&body)
        .icon("shelldeck")
        .show()?;
    Ok(())
}

/// Route a menu-click coming out of the system tray onto the workspace.
/// Runs on the GPUI foreground thread — safe to touch `App` state.
fn dispatch_tray_command(
    cmd: tray::TrayCommand,
    ws: gpui::WeakEntity<Workspace>,
    window: gpui::AnyWindowHandle,
    cx: &mut gpui::App,
) {
    use tray::TrayCommand;
    match cmd {
        TrayCommand::ShowWindow => {
            // Restore + focus the main window (or a no-op if already
            // frontmost). `activate_window` is the portable way to say
            // "give this window the OS focus".
            let _ = window.update(cx, |_, window, _cx| {
                window.activate_window();
            });
        }
        TrayCommand::OpenPalette => {
            if let Some(ws) = ws.upgrade() {
                let _ = window.update(cx, |_, window, cx| {
                    ws.update(cx, |ws, cx| ws.toggle_command_palette(window, cx));
                    window.activate_window();
                });
            }
        }
        TrayCommand::ConnectPinned(id) => {
            if let Some(ws) = ws.upgrade() {
                let _ = window.update(cx, |_, window, cx| {
                    ws.update(cx, |ws, cx| ws.connect_pinned_connection(id, cx));
                    window.activate_window();
                });
            }
        }
        TrayCommand::Quit => {
            if let Some(ws) = ws.upgrade() {
                ws.update(cx, |ws, cx| ws.shutdown(cx));
            }
            cx.quit();
        }
    }
}

/// Parse + route a `shelldeck://…` payload (a bare focus ping arrives as an
/// empty string) onto the workspace, then bring the window to the front so
/// the deep link visibly lands. Runs on the GPUI foreground thread.
fn dispatch_deep_link(
    payload: String,
    ws: gpui::WeakEntity<Workspace>,
    window: gpui::AnyWindowHandle,
    cx: &mut gpui::App,
) {
    let link = DeepLink::parse(&payload);
    let _ = window.update(cx, |_, window, cx| {
        window.activate_window();
        if let (Some(link), Some(ws)) = (link, ws.upgrade()) {
            ws.update(cx, |ws, cx| ws.open_deep_link(link, cx));
        }
    });
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
    // The receiver yields forwarded deep links (and our own launch arg, if
    // any, delivered first). Polled from the GPUI window init below.
    let deep_link_rx = primary.listen(deep_link_arg);

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

            // Route tray menu clicks to workspace actions AND publish
            // workspace counter changes back into the tray menu.
            if let Some((rx, state_tx)) = tray_handles {
                // Publisher for the tray counters. Ships a boxed
                // closure into the workspace so `shelldeck-ui` doesn't
                // need to know about the tray crate.
                workspace.update(cx, |ws, cx| {
                    ws.set_tray_state_publisher(Box::new(move |counters| {
                        let state = tray::TrayState {
                            active_ssh: counters.active_ssh,
                            open_tunnels: counters.open_tunnels,
                            unread_tickets: counters.unread_tickets,
                            jean_pending: counters.jean_pending,
                            pinned_connections: counters
                                .pinned_connections
                                .into_iter()
                                .map(|connection| tray::PinnedConnection {
                                    id: connection.id,
                                    name: connection.name,
                                })
                                .collect(),
                        };
                        // Send failure means the tray thread died —
                        // best-effort, ignore.
                        let _ = state_tx.send(state);
                    }));
                    // OS notifications on deltas. Notify-rust's .show()
                    // is a synchronous D-Bus call on Linux; fire off a
                    // detached thread so a slow notification daemon
                    // never stalls the workspace.
                    ws.set_tray_notifier(Box::new(|n| {
                        std::thread::spawn(move || {
                            if let Err(e) = show_tray_notification(n) {
                                tracing::warn!("OS notification failed: {e}");
                            }
                        });
                    }));
                    // Push a first snapshot so the tray doesn't sit at
                    // "0 / 0 / 0 / 0" until the first mutation. The
                    // first publish also seeds `last_tray_counters`
                    // *without* firing notifications, so pre-existing
                    // unread tickets don't spam the OS on startup.
                    ws.publish_tray_state(cx);
                });
                let ws_handle = workspace.downgrade();
                let window_handle = window.window_handle();
                cx.spawn(async move |cx| {
                    use std::time::Duration;
                    loop {
                        cx.background_executor()
                            .timer(Duration::from_millis(150))
                            .await;
                        while let Ok(cmd) = rx.try_recv() {
                            let ws_handle = ws_handle.clone();
                            let _ = cx.update(|cx| {
                                dispatch_tray_command(cmd, ws_handle, window_handle, cx);
                            });
                        }
                    }
                })
                .detach();
            }

            // Deep-link dispatch loop. Drains URLs forwarded by the
            // single-instance guard (and our own launch arg, delivered
            // first) and routes them onto the workspace. Mirrors the tray
            // loop: a short foreground timer + non-blocking drain.
            {
                let ws_handle = workspace.downgrade();
                let window_handle = window.window_handle();
                cx.spawn(async move |cx| {
                    use std::time::Duration;
                    loop {
                        cx.background_executor()
                            .timer(Duration::from_millis(150))
                            .await;
                        while let Ok(payload) = deep_link_rx.try_recv() {
                            let ws_handle = ws_handle.clone();
                            let _ = cx.update(|cx| {
                                dispatch_deep_link(payload, ws_handle, window_handle, cx);
                            });
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
                let w = workspace.downgrade();
                window.on_window_should_close(cx, move |window, cx| {
                    if let Some(ws) = w.upgrade() {
                        let hide = ws.read(cx).should_hide_to_tray();
                        if hide {
                            window.hide_window();
                            return false;
                        }
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
