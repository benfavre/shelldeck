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

    tracing::info!("Starting ShellDeck v0.1.0");

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
    let store = ConnectionStore::load().unwrap_or_else(|e| {
        tracing::warn!("Failed to load connection store: {}", e);
        ConnectionStore::default()
    });

    tracing::info!(
        "Loaded {} manual connections, {} scripts, {} port forwards",
        store.connections.len(),
        store.scripts.len(),
        store.port_forwards.len()
    );

    // Keep store for passing to workspace
    let store_for_workspace = store.clone();

    // Start GPUI application
    Application::new().run(move |cx| {
        // Initialize adabraka-ui
        adabraka_ui::init(cx);

        // Install theme
        let is_dark = !matches!(
            config.theme,
            shelldeck_core::config::app_config::ThemePreference::Light
        );
        ShellDeckColors::set_dark_mode(is_dark);
        let theme = if is_dark {
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

        cx.open_window(window_options, |window, cx| {
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
            }

            workspace
        })
        .expect("Failed to open main window");

        tracing::info!("ShellDeck window opened");
    });

    Ok(())
}
