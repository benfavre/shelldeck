//! System tray integration.
//!
//! Cross-platform tray icon + menu + OS notifications. The tray keeps
//! ShellDeck present even when the main window is hidden or minimized,
//! surfaces live counters (SSH sessions, tunnels, unread tickets, Monique
//! confirmations), and pushes OS notifications on state deltas the user
//! opted into.
//!
//! # Architecture
//!
//! - Menu clicks fire on the tray thread and are marshalled back to
//!   GPUI via a `Sender<TrayCommand>` that the workspace consumes on
//!   the foreground executor.
//! - Live counter updates flow the other way: the workspace publishes a
//!   [`TrayState`] snapshot via a `Sender<TrayState>`. Linux applies it on
//!   the GTK tray thread; macOS and Windows apply it from GPUI's native
//!   foreground executor, where `muda` menu handles are safe to mutate.
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
//! - **macOS**: `NSStatusItem` with a dedicated monochrome template asset,
//!   allowing AppKit to adapt it to light/dark/pressed menu-bar states.
//! - **Windows**: `Shell_NotifyIcon` with the colored application icon.
//! - **macOS + Windows**: GPUI's foreground executor is the native main-loop
//!   bridge used for live menu mutations; no background thread touches the
//!   non-`Send` `muda` handles.

use anyhow::{Context, Result};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};
use uuid::Uuid;

pub use shelldeck_ui::ai_dock::TrayLabels;

/// Actions the tray can request from the running app. The workspace
/// polls a receiver on the foreground executor and dispatches these
/// onto the main GPUI thread.
#[derive(Debug, Clone, Copy)]
pub enum TrayCommand {
    /// Bring the ShellDeck window to the front (or restore it if
    /// minimized / hidden to tray).
    ShowWindow,
    /// Show or hide the compact standalone AI assistant window.
    ToggleAiDock,
    /// Open the Dock directly on the Clippy clipboard assistant.
    OpenClippy,
    /// Open the Dock directly on its durable AI task center.
    OpenAiTasks,
    /// Open the command palette.
    OpenPalette,
    /// Open Settings directly on the desktop character cards.
    ChooseCharacter,
    /// Pause or resume the optional desktop character runtime.
    PauseCharacter,
    /// Ask the desktop character to return to a safe screen corner.
    ReturnCharacterToCorner,
    /// Connect one of the persisted quick-access hosts.
    ConnectPinned(Uuid),
    /// Quit the app.
    Quit,
}

/// Snapshot of the counters the tray displays. Published by the
/// workspace whenever any tracked count changes; the tray thread
/// diffs against its last known state and only calls `MenuItem::set_text`
/// on the rows that actually moved, keeping the menu paint quiet.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TrayState {
    /// Whether authenticated tray actions may execute. Defaults to false so
    /// the native menu starts fail-closed before Workspace publishes state.
    pub signed_in: bool,
    /// SSH connections in `Connected` status.
    pub active_ssh: usize,
    /// Port forwards currently open.
    pub open_tunnels: usize,
    /// Support tickets with `unread=true`.
    pub unread_tickets: usize,
    /// Monique fleet jobs waiting for user confirmation before running.
    pub monique_pending: usize,
    /// AI tasks currently generating or executing.
    pub ai_tasks_running: usize,
    /// Persisted quick-access connections, in sidebar order.
    pub pinned_connections: Vec<PinnedConnection>,
    /// Localized copy used by the native menu thread. Keeping it in the
    /// snapshot makes a live language change observable even when counters do
    /// not move.
    pub labels: TrayLabels,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PinnedConnection {
    pub id: Uuid,
    pub name: String,
}

/// Public handle over the tray subsystem. Callers drop this once they have
/// started updates and taken the receiver + sender — the platform backend or
/// detached foreground task then owns the live `TrayIcon` and `MenuItem`s.
pub struct TrayService {
    rx: Option<UnboundedReceiver<TrayCommand>>,
    state_tx: Option<UnboundedSender<TrayState>>,
    #[cfg(not(target_os = "linux"))]
    foreground_state: Option<ForegroundTrayState>,
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
        let (cmd_tx, cmd_rx) = tokio::sync::mpsc::unbounded_channel::<TrayCommand>();
        install_menu_router(cmd_tx);

        let (state_tx, state_rx) = tokio::sync::mpsc::unbounded_channel::<TrayState>();

        // Icon bytes are cheap to decode on either thread. We do it
        // here so a bad PNG surfaces as an early hard error rather
        // than as a silent tray-thread failure.
        let icon = load_icon().context("load tray icon")?;

        #[cfg(target_os = "linux")]
        spawn_tray_backend(icon, state_rx)?;
        #[cfg(not(target_os = "linux"))]
        let foreground_state = Some(spawn_tray_backend(icon, state_rx)?);

        Ok(Self {
            rx: Some(cmd_rx),
            state_tx: Some(state_tx),
            #[cfg(not(target_os = "linux"))]
            foreground_state,
        })
    }

    /// Start applying workspace snapshots to the native menu.
    ///
    /// Linux owns its menu on the dedicated GTK thread and starts draining
    /// during [`Self::new`]. macOS and Windows keep `muda`'s non-`Send`
    /// handles on GPUI's foreground executor and await snapshots there.
    pub fn start_state_updates(&mut self, cx: &gpui::App) {
        #[cfg(target_os = "linux")]
        let _ = cx;

        #[cfg(not(target_os = "linux"))]
        {
            let foreground = self
                .foreground_state
                .take()
                .expect("TrayService::start_state_updates called twice");
            let ForegroundTrayState {
                state_rx,
                mut items,
            } = foreground;
            cx.spawn(async move |_| {
                let mut prev_state = TrayState::default();
                consume_tray_states(state_rx, |next| {
                    apply_state(&mut items, &mut prev_state, next);
                })
                .await;
            })
            .detach();
        }
    }

    /// Hand off the command receiver to the caller. Panics if called
    /// twice — the workspace should consume this exactly once at
    /// startup and await it without periodic wakeups.
    pub fn take_receiver(&mut self) -> UnboundedReceiver<TrayCommand> {
        self.rx
            .take()
            .expect("TrayService::take_receiver called twice")
    }

    /// Hand off the state sender to the caller. The workspace keeps this and
    /// pushes a fresh [`TrayState`] every time its counters change; the menu's
    /// owner thread applies each snapshot.
    pub fn take_state_sender(&mut self) -> UnboundedSender<TrayState> {
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
fn install_menu_router(cmd_tx: UnboundedSender<TrayCommand>) {
    MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
        let Some(cmd) = command_for_menu_id(event.id.0.as_str()) else {
            return;
        };
        if let Err(e) = cmd_tx.send(cmd) {
            tracing::warn!("tray event dropped (no consumer?): {e}");
        }
    }));
}

fn command_for_menu_id(id: &str) -> Option<TrayCommand> {
    match id {
        SHOW_ID => Some(TrayCommand::ShowWindow),
        ASSISTANT_ID => Some(TrayCommand::ToggleAiDock),
        CLIPPY_ID => Some(TrayCommand::OpenClippy),
        AI_TASKS_ID => Some(TrayCommand::OpenAiTasks),
        PALETTE_ID => Some(TrayCommand::OpenPalette),
        CHOOSE_CHARACTER_ID => Some(TrayCommand::ChooseCharacter),
        PAUSE_CHARACTER_ID => Some(TrayCommand::PauseCharacter),
        RETURN_CHARACTER_ID => Some(TrayCommand::ReturnCharacterToCorner),
        QUIT_ID => Some(TrayCommand::Quit),
        _ => id
            .strip_prefix(PINNED_ID_PREFIX)
            .and_then(|raw| Uuid::parse_str(raw).ok())
            .map(TrayCommand::ConnectPinned),
    }
}

const SHOW_ID: &str = "shelldeck.tray.show";
const ASSISTANT_ID: &str = "shelldeck.tray.assistant";
const CLIPPY_ID: &str = "shelldeck.tray.clippy";
const AI_TASKS_ID: &str = "shelldeck.tray.ai_tasks";
const PALETTE_ID: &str = "shelldeck.tray.palette";
const CHOOSE_CHARACTER_ID: &str = "shelldeck.tray.character.choose";
const PAUSE_CHARACTER_ID: &str = "shelldeck.tray.character.pause";
const RETURN_CHARACTER_ID: &str = "shelldeck.tray.character.return_to_dock";
const QUIT_ID: &str = "shelldeck.tray.quit";
const PINNED_ID_PREFIX: &str = "shelldeck.tray.pinned.";

// Ids for the disabled counter rows. They still need ids so the same
// widget can be found later for `set_text` (though on GTK we hold the
// MenuItem handles directly).
const COUNTER_SSH_ID: &str = "shelldeck.tray.counter.ssh";
const COUNTER_TUNNELS_ID: &str = "shelldeck.tray.counter.tunnels";
const COUNTER_TICKETS_ID: &str = "shelldeck.tray.counter.tickets";
const COUNTER_MONIQUE_ID: &str = "shelldeck.tray.counter.monique";

/// The counter `MenuItem`s live here, produced by [`build_menu`]
/// alongside their parent menu. Kept together so the tray-thread's
/// state-drain closure can reach them via a single move-capture.
struct CounterItems {
    ssh: MenuItem,
    tunnels: MenuItem,
    tickets: MenuItem,
    monique: MenuItem,
    ai_tasks: MenuItem,
}

struct MenuItems {
    assistant: MenuItem,
    clippy: MenuItem,
    show: MenuItem,
    palette: MenuItem,
    choose_character: MenuItem,
    pause_character: MenuItem,
    return_character: MenuItem,
    quit: MenuItem,
    counters: CounterItems,
    pinned_menu: Submenu,
    pinned_items: Vec<MenuItem>,
}

#[cfg(not(target_os = "linux"))]
struct ForegroundTrayState {
    state_rx: UnboundedReceiver<TrayState>,
    items: MenuItems,
}

/// Build the tray menu — click actions on top, live counters in the middle,
/// then Quit. Informational counters are disabled; the AI task row remains
/// enabled because it opens the task center.
fn build_menu() -> Result<(Menu, MenuItems)> {
    let menu = Menu::new();
    let labels = TrayLabels::localized();

    let assistant_item = MenuItem::with_id(ASSISTANT_ID, &labels.assistant, false, None);
    let clippy_item = MenuItem::with_id(CLIPPY_ID, &labels.clippy, false, None);
    let show_item = MenuItem::with_id(SHOW_ID, &labels.show, true, None);
    let palette_item = MenuItem::with_id(PALETTE_ID, &labels.palette, true, None);
    let choose_character_item =
        MenuItem::with_id(CHOOSE_CHARACTER_ID, &labels.choose_character, false, None);
    let pause_character_item =
        MenuItem::with_id(PAUSE_CHARACTER_ID, &labels.pause_character, false, None);
    let return_character_item =
        MenuItem::with_id(RETURN_CHARACTER_ID, &labels.return_character, false, None);
    let quit_item = MenuItem::with_id(QUIT_ID, &labels.quit, true, None);
    let pinned_menu = Submenu::new(&labels.pinned, false);
    let no_pinned = MenuItem::new(&labels.no_pinned, false, None);
    pinned_menu
        .append(&no_pinned)
        .context("append empty pinned row")?;

    // Counter rows: `enabled = false` so the tray marks them as
    // dimmed / unclickable — they exist for information only.
    let counters = CounterItems {
        ssh: MenuItem::with_id(COUNTER_SSH_ID, counter_label_ssh(0), false, None),
        tunnels: MenuItem::with_id(COUNTER_TUNNELS_ID, counter_label_tunnels(0), false, None),
        tickets: MenuItem::with_id(COUNTER_TICKETS_ID, counter_label_tickets(0), false, None),
        monique: MenuItem::with_id(COUNTER_MONIQUE_ID, counter_label_monique(0), false, None),
        ai_tasks: MenuItem::with_id(AI_TASKS_ID, counter_label_ai_tasks(0), false, None),
    };

    menu.append(&assistant_item)
        .context("append Assistant item")?;
    menu.append(&clippy_item).context("append Clippy item")?;
    menu.append(&show_item).context("append Show item")?;
    menu.append(&palette_item).context("append Palette item")?;
    menu.append(&choose_character_item)
        .context("append Choose character item")?;
    menu.append(&pause_character_item)
        .context("append Pause character item")?;
    menu.append(&return_character_item)
        .context("append Return character item")?;
    menu.append(&pinned_menu)
        .context("append pinned connections menu")?;
    menu.append(&PredefinedMenuItem::separator())
        .context("append separator counters-top")?;
    menu.append(&counters.ssh).context("append SSH counter")?;
    menu.append(&counters.tunnels)
        .context("append tunnels counter")?;
    menu.append(&counters.tickets)
        .context("append tickets counter")?;
    menu.append(&counters.monique)
        .context("append Monique counter")?;
    menu.append(&counters.ai_tasks)
        .context("append AI tasks counter")?;
    menu.append(&PredefinedMenuItem::separator())
        .context("append separator quit-top")?;
    menu.append(&quit_item).context("append Quit item")?;

    Ok((
        menu,
        MenuItems {
            assistant: assistant_item,
            clippy: clippy_item,
            show: show_item,
            palette: palette_item,
            choose_character: choose_character_item,
            pause_character: pause_character_item,
            return_character: return_character_item,
            quit: quit_item,
            counters,
            pinned_menu,
            pinned_items: vec![no_pinned],
        },
    ))
}

/// Apply a fresh state to the counter items. Only rewrites labels that
/// actually changed so the tray menu stays quiet under repeated
/// identical publishes. Must run on the GTK thread on Linux.
fn apply_state(items: &mut MenuItems, prev: &mut TrayState, next: TrayState) {
    let counters = &items.counters;
    let language_changed = prev.labels != next.labels;
    if prev.signed_in != next.signed_in {
        items.assistant.set_enabled(next.signed_in);
        items.clippy.set_enabled(next.signed_in);
        items.choose_character.set_enabled(next.signed_in);
        items.pause_character.set_enabled(next.signed_in);
        items.return_character.set_enabled(next.signed_in);
        items.pinned_menu.set_enabled(next.signed_in);
        counters.ai_tasks.set_enabled(next.signed_in);
    }
    if language_changed {
        items.assistant.set_text(&next.labels.assistant);
        items.clippy.set_text(&next.labels.clippy);
        items.show.set_text(&next.labels.show);
        items.palette.set_text(&next.labels.palette);
        items
            .choose_character
            .set_text(&next.labels.choose_character);
        items.pause_character.set_text(&next.labels.pause_character);
        items
            .return_character
            .set_text(&next.labels.return_character);
        items.quit.set_text(&next.labels.quit);
        items.pinned_menu.set_text(&next.labels.pinned);
    }
    if language_changed || prev.active_ssh != next.active_ssh {
        counters.ssh.set_text(counter_label_ssh(next.active_ssh));
    }
    if language_changed || prev.open_tunnels != next.open_tunnels {
        counters
            .tunnels
            .set_text(counter_label_tunnels(next.open_tunnels));
    }
    if language_changed || prev.unread_tickets != next.unread_tickets {
        counters
            .tickets
            .set_text(counter_label_tickets(next.unread_tickets));
    }
    if language_changed || prev.monique_pending != next.monique_pending {
        counters
            .monique
            .set_text(counter_label_monique(next.monique_pending));
    }
    if language_changed || prev.ai_tasks_running != next.ai_tasks_running {
        counters
            .ai_tasks
            .set_text(counter_label_ai_tasks(next.ai_tasks_running));
    }
    if language_changed || prev.pinned_connections != next.pinned_connections {
        for item in items.pinned_items.drain(..) {
            if let Err(error) = items.pinned_menu.remove(&item) {
                tracing::warn!("failed to remove pinned tray item: {error}");
            }
        }

        if next.pinned_connections.is_empty() {
            let item = MenuItem::new(&next.labels.no_pinned, false, None);
            if let Err(error) = items.pinned_menu.append(&item) {
                tracing::warn!("failed to append empty pinned tray row: {error}");
            }
            items.pinned_items.push(item);
        } else {
            for connection in &next.pinned_connections {
                let item = MenuItem::with_id(
                    format!("{PINNED_ID_PREFIX}{}", connection.id),
                    &connection.name,
                    true,
                    None,
                );
                if let Err(error) = items.pinned_menu.append(&item) {
                    tracing::warn!("failed to append pinned tray item: {error}");
                }
                items.pinned_items.push(item);
            }
        }
    }
    *prev = next;
}

/// Forward every published snapshot to the owner-thread menu mutator until
/// all senders disappear during shutdown.
#[cfg(any(not(target_os = "linux"), test))]
async fn consume_tray_states(
    mut state_rx: UnboundedReceiver<TrayState>,
    mut apply: impl FnMut(TrayState),
) {
    while let Some(next) = state_rx.recv().await {
        apply(next);
    }
}

// Label formatters use explicit zero/one/many keys because rust-i18n does not
// infer the app's desired wording for these compact native-menu rows.

fn counter_label_ssh(n: usize) -> String {
    shelldeck_ui::ai_dock::tray_counter_ssh(n)
}

fn counter_label_tunnels(n: usize) -> String {
    shelldeck_ui::ai_dock::tray_counter_tunnels(n)
}

fn counter_label_tickets(n: usize) -> String {
    shelldeck_ui::ai_dock::tray_counter_tickets(n)
}

fn counter_label_monique(n: usize) -> String {
    shelldeck_ui::ai_dock::tray_counter_monique(n)
}

fn counter_label_ai_tasks(n: usize) -> String {
    shelldeck_ui::ai_dock::tray_counter_ai_tasks(n)
}

/// Load the tray PNG. 32 px is the sweet spot for tray display across
/// DEs; the platform scales it to whatever the tray area needs.
fn load_icon() -> Result<tray_icon::Icon> {
    // Embedded at compile time so the binary is self-contained. macOS gets a
    // 36 px monochrome alpha mask (18 pt @2x); Linux and Windows keep the
    // colored 32 px app icon.
    #[cfg(target_os = "macos")]
    let bytes = include_bytes!("../../../../packaging/icons/shelldeck-tray-template-macos.png");
    #[cfg(not(target_os = "macos"))]
    let bytes = include_bytes!("../../../../packaging/icons/shelldeck-32.png");
    let img = image::load_from_memory(bytes)
        .context("decode embedded tray PNG")?
        .to_rgba8();
    let (w, h) = img.dimensions();
    tray_icon::Icon::from_rgba(img.into_raw(), w, h).context("build tray_icon::Icon from RGBA")
}

#[cfg(target_os = "linux")]
fn spawn_tray_backend(icon: tray_icon::Icon, state_rx: UnboundedReceiver<TrayState>) -> Result<()> {
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

            let (menu, mut items) = match build_menu() {
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
            // The closure owns the menu handles,
            // `prev_state` for diffing, and `state_rx` for draining
            // publishes from the workspace.
            let mut prev_state = TrayState::default();
            let mut state_rx = state_rx;
            gtk::glib::timeout_add_local(std::time::Duration::from_millis(200), move || {
                while let Ok(next) = state_rx.try_recv() {
                    apply_state(&mut items, &mut prev_state, next);
                }
                gtk::glib::ControlFlow::Continue
            });

            let _ = ready_tx.send(Ok(()));

            // Park on GTK's main loop — never returns.
            gtk::main();
        })
        .context("spawn shelldeck-tray thread")?;

    ready_rx
        .recv()
        .context("tray thread died before signalling")?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pinned_menu_id_routes_to_connection() {
        let id = Uuid::new_v4();
        let menu_id = format!("{PINNED_ID_PREFIX}{id}");

        assert!(matches!(
            command_for_menu_id(&menu_id),
            Some(TrayCommand::ConnectPinned(routed)) if routed == id
        ));
    }

    // SDTEST-1380
    #[test]
    fn assistant_menu_id_routes_to_dock_toggle() {
        assert!(matches!(
            command_for_menu_id(ASSISTANT_ID),
            Some(TrayCommand::ToggleAiDock)
        ));
    }

    // SDTEST-1484
    #[test]
    fn clippy_menu_id_routes_directly_to_clippy() {
        assert!(matches!(
            command_for_menu_id(CLIPPY_ID),
            Some(TrayCommand::OpenClippy)
        ));
    }

    // SDTEST-1490
    #[test]
    fn choose_character_menu_id_routes_to_targeted_settings() {
        assert!(matches!(
            command_for_menu_id(CHOOSE_CHARACTER_ID),
            Some(TrayCommand::ChooseCharacter)
        ));
    }

    // SDTEST-1408
    #[test]
    fn ai_tasks_menu_id_routes_to_task_center() {
        assert!(matches!(
            command_for_menu_id(AI_TASKS_ID),
            Some(TrayCommand::OpenAiTasks)
        ));
    }

    // SDTEST-1411
    #[tokio::test(flavor = "current_thread")]
    async fn tray_state_pump_forwards_every_snapshot_until_shutdown() {
        let (state_tx, state_rx) = tokio::sync::mpsc::unbounded_channel();
        for active_ssh in [1, 2, 0] {
            state_tx
                .send(TrayState {
                    active_ssh,
                    ..TrayState::default()
                })
                .expect("live tray receiver");
        }
        drop(state_tx);

        let mut observed = Vec::new();
        consume_tray_states(state_rx, |state| observed.push(state.active_ssh)).await;

        assert_eq!(observed, vec![1, 2, 0]);
    }

    // SDTEST-1410
    #[test]
    fn macos_template_asset_is_retina_monochrome_with_transparent_background() {
        let image = image::load_from_memory(include_bytes!(
            "../../../../packaging/icons/shelldeck-tray-template-macos.png"
        ))
        .expect("decode macOS tray template")
        .to_rgba8();
        assert_eq!(image.dimensions(), (36, 36));

        let mut visible = 0usize;
        for pixel in image.pixels() {
            if pixel.0[3] > 0 {
                visible += 1;
                assert_eq!(
                    &pixel.0[..3],
                    &[0, 0, 0],
                    "template RGB must stay black; AppKit colors the alpha mask"
                );
            }
        }
        assert!(visible > 36 * 36 / 8, "template mark is unexpectedly empty");
        assert!(
            visible < 36 * 36 * 3 / 4,
            "template background is not transparent"
        );
        for (x, y) in [(0, 0), (35, 0), (0, 35), (35, 35)] {
            assert_eq!(image.get_pixel(x, y).0[3], 0);
        }
    }

    #[test]
    fn unknown_or_malformed_menu_id_is_ignored() {
        assert!(command_for_menu_id("shelldeck.tray.counter.ssh").is_none());
        assert!(command_for_menu_id("shelldeck.tray.pinned.invalid").is_none());
    }
}

#[cfg(not(target_os = "linux"))]
fn spawn_tray_backend(
    icon: tray_icon::Icon,
    state_rx: UnboundedReceiver<TrayState>,
) -> Result<ForegroundTrayState> {
    // macOS + Windows: no separate event loop needed, the platform
    // run-loop drives the tray directly. The `TrayIcon` must be built
    // on the main thread but doesn't need to be kept as a named
    // binding — we intentionally leak it so it lives for the whole
    // process (dropping the icon removes the tray entry).
    let (menu, items) = build_menu()?;
    let builder = tray_icon::TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_tooltip("ShellDeck")
        .with_icon(icon);
    #[cfg(target_os = "macos")]
    let builder = builder.with_icon_as_template(true);
    let tray = builder.build().context("build tray icon")?;
    Box::leak(Box::new(tray));

    Ok(ForegroundTrayState { state_rx, items })
}
