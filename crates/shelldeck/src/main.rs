mod actions;

use adabraka_ui::prelude::*;
use anyhow::Result;
use gpui::WindowDecorations;
use shelldeck_core::config::app_config::AppConfig;
use shelldeck_core::config::ssh_config::parse_ssh_config;
use shelldeck_core::config::store::ConnectionStore;
use shelldeck_ui::theme::ShellDeckColors;
use shelldeck_ui::Workspace;
use tracing_subscriber::EnvFilter;

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
    Application::new().run(move |cx| {
        // Initialize adabraka-ui
        adabraka_ui::init(cx);

        // Install theme — resolve the configured preference into a full palette,
        // then pick the matching adabraka-ui component theme.
        ShellDeckColors::set_theme(&config.theme);
        let theme = if config.theme.is_dark() {
            Theme::dark()
        } else {
            Theme::light()
        };
        install_theme(cx, theme);

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
