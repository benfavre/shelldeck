//! System tray integration.
//!
//! Cross-platform tray icon + menu + OS notifications. The tray keeps
//! ShellDeck present even when the main window is hidden or minimized,
//! surfaces live counters (SSH sessions, tunnels, unread tickets, Jean
//! confirmations), and pushes OS notifications on state deltas the user
//! opted into.
//!
//! # Architecture
//!
//! - Menu events fire on the tray thread (a dedicated GTK-owning
//!   thread on Linux; the platform run-loop elsewhere) and are
//!   marshalled back to GPUI via a `Sender<TrayCommand>` that the
//!   workspace consumes on the foreground executor.
//! - Phase A (this file): static menu — Show window / Command palette /
//!   Quit. Counters + notifications land in phases B + C.
//!
//! # Linux GTK requirement
//!
//! `tray-icon` depends on `libappindicator`, which itself sits on GTK3.
//! adabraka-gpui is Wayland/X11 native and does **not** initialise GTK,
//! so calling `TrayIconBuilder::build()` from the GPUI closure panics
//! with `"GTK has not been initialized. Call gtk::init first."`.
//!
//! The fix is a dedicated tray thread that:
//!
//! 1. Calls `gtk::init()`.
//! 2. Builds the `TrayIcon` inside the thread.
//! 3. Runs `gtk::main()` forever so GTK's event loop keeps dispatching
//!    menu clicks into the global `MenuEvent` channel.
//!
//! The main thread never touches the `TrayIcon` after handoff — the
//! event router uses the crate's global static receiver, and the
//! `mpsc::Sender` we install via `set_event_handler` is `Send`.
//!
//! # Platform notes
//!
//! - **Linux**: `libayatana-appindicator3` or `libappindicator3`,
//!   typically pre-installed on GNOME/KDE.
//! - **macOS**: `NSStatusItem`. Colored icons render as-is; a template
//!   (monochrome + alpha) is nicer but not shipped yet.
//! - **Windows**: `Shell_NotifyIcon`. Consumes an ICO.

use anyhow::{Context, Result};
use std::sync::mpsc::Receiver;
use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};

/// Actions the tray can request from the running app. The workspace
/// polls a receiver on the foreground executor and dispatches these
/// onto the main GPUI thread.
#[derive(Debug, Clone, Copy)]
pub enum TrayCommand {
    /// Bring the ShellDeck window to the front (or restore it if
    /// minimized / hidden to tray).
    ShowWindow,
    /// Open the command palette.
    OpenPalette,
    /// Quit the app.
    Quit,
}

/// Public handle over the tray subsystem. Dropping it does **not** tear
/// down the tray thread — on purpose: the tray must outlive every
/// caller on the main thread. Callers keep this only to consume the
/// command receiver.
pub struct TrayService {
    rx: Option<Receiver<TrayCommand>>,
}

impl TrayService {
    /// Build the tray icon + menu and wire the event routing.
    ///
    /// On Linux this spawns a dedicated GTK-owning thread and blocks
    /// (~50 ms) on a ready signal so the tray is guaranteed visible
    /// before `TrayService::new` returns. On other platforms the tray
    /// is constructed on the calling thread (must be the main thread).
    ///
    /// Returns an error only if the tray truly can't come up (icon
    /// decode failure, `libappindicator` absent, GTK init failure).
    /// Callers should log the error and continue without a tray rather
    /// than aborting the app.
    pub fn new() -> Result<Self> {
        let (cmd_tx, cmd_rx) = std::sync::mpsc::channel::<TrayCommand>();
        install_menu_router(cmd_tx.clone());

        // Icon bytes are cheap to decode on either thread. We do it
        // here so a bad PNG surfaces as an early hard error rather than
        // as a silent tray-thread failure.
        let icon = load_icon().context("load tray icon")?;

        spawn_tray_backend(icon)?;

        Ok(Self { rx: Some(cmd_rx) })
    }

    /// Hand off the command receiver to the caller. Panics if called
    /// twice — the workspace should consume this exactly once at
    /// startup and poll it on its own schedule.
    pub fn take_receiver(&mut self) -> Receiver<TrayCommand> {
        self.rx
            .take()
            .expect("TrayService::take_receiver called twice")
    }
}

/// Install the global menu-event handler that routes menu clicks into
/// our channel. Called once per process; a second call replaces the
/// first (documented `tray_icon` behaviour).
///
/// The item ids are set inside [`build_menu`] as stable strings so
/// this handler doesn't need to see the `MenuItem`s directly.
fn install_menu_router(cmd_tx: std::sync::mpsc::Sender<TrayCommand>) {
    MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
        let cmd = match event.id.0.as_str() {
            SHOW_ID => TrayCommand::ShowWindow,
            PALETTE_ID => TrayCommand::OpenPalette,
            QUIT_ID => TrayCommand::Quit,
            _ => return,
        };
        if let Err(e) = cmd_tx.send(cmd) {
            tracing::warn!("tray event dropped (no consumer?): {e}");
        }
    }));
}

const SHOW_ID: &str = "shelldeck.tray.show";
const PALETTE_ID: &str = "shelldeck.tray.palette";
const QUIT_ID: &str = "shelldeck.tray.quit";

/// Build the static Phase A menu. Ids are stable strings so the router
/// can match them without holding on to `MenuItem` handles.
fn build_menu() -> Result<Menu> {
    let menu = Menu::new();

    let show_item = MenuItem::with_id(SHOW_ID, "Ouvrir ShellDeck", true, None);
    let palette_item = MenuItem::with_id(PALETTE_ID, "Palette de commandes", true, None);
    let quit_item = MenuItem::with_id(QUIT_ID, "Quitter", true, None);

    menu.append(&show_item).context("append Show item")?;
    menu.append(&PredefinedMenuItem::separator())
        .context("append separator")?;
    menu.append(&palette_item).context("append Palette item")?;
    menu.append(&PredefinedMenuItem::separator())
        .context("append separator 2")?;
    menu.append(&quit_item).context("append Quit item")?;

    Ok(menu)
}

/// Load the tray PNG. 32 px is the sweet spot for tray display across
/// DEs; the platform scales it to whatever the tray area needs.
fn load_icon() -> Result<tray_icon::Icon> {
    // Embedded at compile time so the binary is self-contained (no
    // runtime file lookup, no path-in-release-artifact problem).
    let bytes = include_bytes!("../../../../packaging/icons/shelldeck-32.png");
    let img = image::load_from_memory(bytes)
        .context("decode embedded tray PNG")?
        .to_rgba8();
    let (w, h) = img.dimensions();
    tray_icon::Icon::from_rgba(img.into_raw(), w, h).context("build tray_icon::Icon from RGBA")
}

#[cfg(target_os = "linux")]
fn spawn_tray_backend(icon: tray_icon::Icon) -> Result<()> {
    // A oneshot to synchronise "tray is live" with the main thread.
    // We wait here so if GTK init or tray build fails, the error
    // bubbles up before the app opens its main window. If everything
    // is fine, the thread parks on `gtk::main()` for the rest of the
    // process's life.
    let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<()>>();

    std::thread::Builder::new()
        .name("shelldeck-tray".to_string())
        .spawn(move || {
            if let Err(e) = gtk::init() {
                let _ = ready_tx.send(Err(anyhow::anyhow!("gtk::init failed: {e}")));
                return;
            }

            let menu = match build_menu() {
                Ok(m) => m,
                Err(e) => {
                    let _ = ready_tx.send(Err(e));
                    return;
                }
            };

            let build = tray_icon::TrayIconBuilder::new()
                .with_menu(Box::new(menu))
                .with_tooltip("ShellDeck")
                .with_icon(icon)
                .build();

            let _tray = match build {
                Ok(t) => t,
                Err(e) => {
                    let _ = ready_tx.send(Err(anyhow::anyhow!("tray build failed: {e}")));
                    return;
                }
            };

            // Signal success — the main thread can now proceed with
            // the rest of app startup. The tray is guaranteed alive
            // (bound to `_tray` in this scope).
            let _ = ready_tx.send(Ok(()));

            // Park on GTK's main loop. This never returns — the tray
            // is kept up by this loop dispatching menu clicks into the
            // global `MenuEvent` channel that our router consumes.
            gtk::main();
        })
        .context("spawn shelldeck-tray thread")?;

    // Block briefly for the tray-thread status. If it fails, we
    // return the error and the caller can fall back to no-tray mode.
    ready_rx
        .recv()
        .context("tray thread died before signalling")?
}

#[cfg(not(target_os = "linux"))]
fn spawn_tray_backend(icon: tray_icon::Icon) -> Result<()> {
    // macOS + Windows: no separate event loop needed, the platform
    // run-loop drives the tray directly. The `TrayIcon` must be built
    // on the main thread but doesn't need to be kept as a named
    // binding — we intentionally leak it so it lives for the whole
    // process (dropping the icon removes the tray entry).
    let menu = build_menu()?;
    let tray = tray_icon::TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_tooltip("ShellDeck")
        .with_icon(icon)
        .build()
        .context("build tray icon")?;
    Box::leak(Box::new(tray));
    Ok(())
}
