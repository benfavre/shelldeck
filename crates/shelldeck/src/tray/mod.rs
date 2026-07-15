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
//! - Menu clicks fire on the tray thread and are marshalled back to
//!   GPUI via a `Sender<TrayCommand>` that the workspace consumes on
//!   the foreground executor.
//! - Live counter updates flow the other way: the workspace publishes a
//!   [`TrayState`] snapshot via a `Sender<TrayState>` and the tray
//!   thread rewrites its counter menu items with the new values.
//! - Phase A: static menu (Show/Palette/Quit). Phase B (this file):
//!   live counters. Phase C: OS notifications on deltas. Phase D:
//!   opt-in per notification category.
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
//! 2. Builds the `TrayIcon` (and its `MenuItem`s) inside the thread.
//! 3. Registers a `glib::timeout_add_local` that periodically drains
//!    the state channel from within GTK's loop — `MenuItem::set_text`
//!    is `!Send` and can only run on the GTK thread.
//! 4. Parks on `gtk::main()` so GTK's event loop keeps dispatching
//!    both menu clicks (via the global `MenuEvent` channel) and our
//!    state-drain timeout.
//!
//! # Platform notes
//!
//! - **Linux**: `libayatana-appindicator3` or `libappindicator3`,
//!   typically pre-installed on GNOME/KDE.
//! - **macOS**: `NSStatusItem`. Colored icons render as-is; a template
//!   (monochrome + alpha) is nicer but not shipped yet. Live counter
//!   updates are a follow-up (needs a `dispatch_async(main_queue)`
//!   bridge instead of the GTK timeout).
//! - **Windows**: `Shell_NotifyIcon`. Live counter updates likewise
//!   need `PostMessage` glue; not wired yet.

use anyhow::{Context, Result};
use std::sync::mpsc::{Receiver, Sender};
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

/// Snapshot of the counters the tray displays. Published by the
/// workspace whenever any tracked count changes; the tray thread
/// diffs against its last known state and only calls `MenuItem::set_text`
/// on the rows that actually moved, keeping the menu paint quiet.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TrayState {
    /// SSH connections in `Connected` status.
    pub active_ssh: usize,
    /// Port forwards currently open.
    pub open_tunnels: usize,
    /// Support tickets with `unread=true`.
    pub unread_tickets: usize,
    /// Jean fleet jobs waiting for user confirmation before running.
    pub jean_pending: usize,
}

/// Public handle over the tray subsystem. Callers drop this once
/// they've taken the receiver + sender — the tray thread owns the
/// live `TrayIcon` and `MenuItem`s.
pub struct TrayService {
    rx: Option<Receiver<TrayCommand>>,
    state_tx: Option<Sender<TrayState>>,
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
        install_menu_router(cmd_tx);

        let (state_tx, state_rx) = std::sync::mpsc::channel::<TrayState>();

        // Icon bytes are cheap to decode on either thread. We do it
        // here so a bad PNG surfaces as an early hard error rather
        // than as a silent tray-thread failure.
        let icon = load_icon().context("load tray icon")?;

        spawn_tray_backend(icon, state_rx)?;

        Ok(Self {
            rx: Some(cmd_rx),
            state_tx: Some(state_tx),
        })
    }

    /// Hand off the command receiver to the caller. Panics if called
    /// twice — the workspace should consume this exactly once at
    /// startup and poll it on its own schedule.
    pub fn take_receiver(&mut self) -> Receiver<TrayCommand> {
        self.rx
            .take()
            .expect("TrayService::take_receiver called twice")
    }

    /// Hand off the state sender to the caller. The workspace keeps
    /// this and pushes a fresh [`TrayState`] every time its counters
    /// change; the tray thread updates its menu labels from within
    /// the GTK loop.
    pub fn take_state_sender(&mut self) -> Sender<TrayState> {
        self.state_tx
            .take()
            .expect("TrayService::take_state_sender called twice")
    }
}

/// Install the global menu-event handler that routes menu clicks into
/// our channel. Called once per process; a second call replaces the
/// first (documented `tray_icon` behaviour).
///
/// The item ids are stable strings set inside [`build_menu`] so this
/// handler doesn't need to see the `MenuItem`s directly.
fn install_menu_router(cmd_tx: Sender<TrayCommand>) {
    MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
        let cmd = match event.id.0.as_str() {
            SHOW_ID => TrayCommand::ShowWindow,
            PALETTE_ID => TrayCommand::OpenPalette,
            QUIT_ID => TrayCommand::Quit,
            _ => return, // counter rows are non-clickable disabled items
        };
        if let Err(e) = cmd_tx.send(cmd) {
            tracing::warn!("tray event dropped (no consumer?): {e}");
        }
    }));
}

const SHOW_ID: &str = "shelldeck.tray.show";
const PALETTE_ID: &str = "shelldeck.tray.palette";
const QUIT_ID: &str = "shelldeck.tray.quit";

// Ids for the disabled counter rows. They still need ids so the same
// widget can be found later for `set_text` (though on GTK we hold the
// MenuItem handles directly).
const COUNTER_SSH_ID: &str = "shelldeck.tray.counter.ssh";
const COUNTER_TUNNELS_ID: &str = "shelldeck.tray.counter.tunnels";
const COUNTER_TICKETS_ID: &str = "shelldeck.tray.counter.tickets";
const COUNTER_JEAN_ID: &str = "shelldeck.tray.counter.jean";

/// The four counter `MenuItem`s live here, produced by [`build_menu`]
/// alongside their parent menu. Kept together so the tray-thread's
/// state-drain closure can reach them via a single move-capture.
struct CounterItems {
    ssh: MenuItem,
    tunnels: MenuItem,
    tickets: MenuItem,
    jean: MenuItem,
}

/// Build the tray menu — click actions on top, live counters in the
/// middle (disabled = non-clickable placeholders whose text is
/// rewritten on state updates), Quit at the bottom.
fn build_menu() -> Result<(Menu, CounterItems)> {
    let menu = Menu::new();

    let show_item = MenuItem::with_id(SHOW_ID, "Ouvrir ShellDeck", true, None);
    let palette_item = MenuItem::with_id(PALETTE_ID, "Palette de commandes", true, None);
    let quit_item = MenuItem::with_id(QUIT_ID, "Quitter", true, None);

    // Counter rows: `enabled = false` so the tray marks them as
    // dimmed / unclickable — they exist for information only.
    let counters = CounterItems {
        ssh: MenuItem::with_id(
            COUNTER_SSH_ID,
            &counter_label_ssh(0),
            false,
            None,
        ),
        tunnels: MenuItem::with_id(
            COUNTER_TUNNELS_ID,
            &counter_label_tunnels(0),
            false,
            None,
        ),
        tickets: MenuItem::with_id(
            COUNTER_TICKETS_ID,
            &counter_label_tickets(0),
            false,
            None,
        ),
        jean: MenuItem::with_id(
            COUNTER_JEAN_ID,
            &counter_label_jean(0),
            false,
            None,
        ),
    };

    menu.append(&show_item).context("append Show item")?;
    menu.append(&palette_item).context("append Palette item")?;
    menu.append(&PredefinedMenuItem::separator())
        .context("append separator counters-top")?;
    menu.append(&counters.ssh).context("append SSH counter")?;
    menu.append(&counters.tunnels)
        .context("append tunnels counter")?;
    menu.append(&counters.tickets)
        .context("append tickets counter")?;
    menu.append(&counters.jean).context("append Jean counter")?;
    menu.append(&PredefinedMenuItem::separator())
        .context("append separator quit-top")?;
    menu.append(&quit_item).context("append Quit item")?;

    Ok((menu, counters))
}

/// Apply a fresh state to the counter items. Only rewrites labels that
/// actually changed so the tray menu stays quiet under repeated
/// identical publishes. Must run on the GTK thread on Linux.
fn apply_state(counters: &CounterItems, prev: &mut TrayState, next: TrayState) {
    if prev.active_ssh != next.active_ssh {
        counters.ssh.set_text(&counter_label_ssh(next.active_ssh));
    }
    if prev.open_tunnels != next.open_tunnels {
        counters
            .tunnels
            .set_text(&counter_label_tunnels(next.open_tunnels));
    }
    if prev.unread_tickets != next.unread_tickets {
        counters
            .tickets
            .set_text(&counter_label_tickets(next.unread_tickets));
    }
    if prev.jean_pending != next.jean_pending {
        counters
            .jean
            .set_text(&counter_label_jean(next.jean_pending));
    }
    *prev = next;
}

// Label formatters. French to match the app's default locale; plurals
// are hand-picked because a single-count row is common enough to be
// worth the specialisation.

fn counter_label_ssh(n: usize) -> String {
    match n {
        0 => "Aucune connexion SSH active".to_string(),
        1 => "1 connexion SSH active".to_string(),
        n => format!("{n} connexions SSH actives"),
    }
}

fn counter_label_tunnels(n: usize) -> String {
    match n {
        0 => "Aucun tunnel ouvert".to_string(),
        1 => "1 tunnel ouvert".to_string(),
        n => format!("{n} tunnels ouverts"),
    }
}

fn counter_label_tickets(n: usize) -> String {
    match n {
        0 => "Aucun ticket non lu".to_string(),
        1 => "1 ticket non lu".to_string(),
        n => format!("{n} tickets non lus"),
    }
}

fn counter_label_jean(n: usize) -> String {
    match n {
        0 => "Aucune validation Jean en attente".to_string(),
        1 => "1 validation Jean en attente".to_string(),
        n => format!("{n} validations Jean en attente"),
    }
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
fn spawn_tray_backend(
    icon: tray_icon::Icon,
    state_rx: Receiver<TrayState>,
) -> Result<()> {
    // Oneshot to synchronise "tray is live" with the main thread. If
    // GTK init or tray build fails, the error bubbles up before the
    // app opens its main window. On success the thread parks on
    // `gtk::main()` for the rest of the process's life.
    let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<()>>();

    std::thread::Builder::new()
        .name("shelldeck-tray".to_string())
        .spawn(move || {
            if let Err(e) = gtk::init() {
                let _ = ready_tx.send(Err(anyhow::anyhow!("gtk::init failed: {e}")));
                return;
            }

            let (menu, counters) = match build_menu() {
                Ok(pair) => pair,
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

            // Register the state-drain inside GTK's main context.
            // Runs every 200 ms — snappy enough for the human, cheap
            // enough to run forever. `timeout_add_local` requires
            // being called from the main context (we are, we're the
            // GTK thread).
            //
            // The closure owns `counters` (the MenuItem handles),
            // `prev_state` for diffing, and `state_rx` for draining
            // publishes from the workspace.
            let mut prev_state = TrayState::default();
            gtk::glib::timeout_add_local(
                std::time::Duration::from_millis(200),
                move || {
                    while let Ok(next) = state_rx.try_recv() {
                        apply_state(&counters, &mut prev_state, next);
                    }
                    gtk::glib::ControlFlow::Continue
                },
            );

            let _ = ready_tx.send(Ok(()));

            // Park on GTK's main loop — never returns.
            gtk::main();
        })
        .context("spawn shelldeck-tray thread")?;

    ready_rx
        .recv()
        .context("tray thread died before signalling")?
}

#[cfg(not(target_os = "linux"))]
fn spawn_tray_backend(
    icon: tray_icon::Icon,
    state_rx: Receiver<TrayState>,
) -> Result<()> {
    // macOS + Windows: no separate event loop needed, the platform
    // run-loop drives the tray directly. The `TrayIcon` must be built
    // on the main thread but doesn't need to be kept as a named
    // binding — we intentionally leak it so it lives for the whole
    // process (dropping the icon removes the tray entry).
    let (menu, _counters) = build_menu()?;
    let tray = tray_icon::TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_tooltip("ShellDeck")
        .with_icon(icon)
        .build()
        .context("build tray icon")?;
    Box::leak(Box::new(tray));

    // TODO(companion/tray-macos-windows): live-counter updates need a
    // platform bridge equivalent to the GTK `timeout_add_local` used on
    // Linux (`dispatch_async(main_queue)` on macOS, `PostMessage` +
    // WndProc on Windows). Until then, drain the receiver on a
    // background thread and drop the values so the workspace-side
    // channel doesn't back up.
    std::thread::Builder::new()
        .name("shelldeck-tray-state-drain".to_string())
        .spawn(move || while state_rx.recv().is_ok() {})
        .context("spawn tray state-drain thread")?;
    Ok(())
}
