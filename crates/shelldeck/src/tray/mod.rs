//! Native system-tray integration through GPUI.
//!
//! ShellDeck uses GPUI's platform-native status item on every desktop. On
//! Linux that is the StatusNotifierItem (`ksni`) backend, so the application
//! does not initialise GTK or carry the unmaintained GTK3 tray dependency.

use anyhow::{anyhow, Result};
use gpui::{App, SharedString, TrayMenuItem};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use uuid::Uuid;

pub use shelldeck_ui::ai_dock::TrayLabels;

#[derive(Debug, Clone, Copy)]
pub enum TrayCommand {
    ShowWindow,
    ToggleAiDock,
    OpenClippy,
    OpenAiTasks,
    OpenPalette,
    ChooseCharacter,
    PauseCharacter,
    ReturnCharacterToCorner,
    ConnectPinned(Uuid),
    Quit,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TrayState {
    pub signed_in: bool,
    pub active_ssh: usize,
    pub open_tunnels: usize,
    pub unread_tickets: usize,
    pub monique_pending: usize,
    pub ai_tasks_running: usize,
    pub pinned_connections: Vec<PinnedConnection>,
    pub labels: TrayLabels,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PinnedConnection {
    pub id: Uuid,
    pub name: String,
}

pub struct TrayService {
    rx: Option<UnboundedReceiver<TrayCommand>>,
    state_tx: Option<UnboundedSender<TrayState>>,
    state_rx: Option<UnboundedReceiver<TrayState>>,
}

impl TrayService {
    /// Creates a platform-native tray and refuses startup if the desktop did
    /// not actually accept it. Callers use that result to keep the main window
    /// visible in headless sessions and minimal window managers.
    pub fn new(cx: &App) -> Result<Self> {
        let (cmd_tx, cmd_rx) = tokio::sync::mpsc::unbounded_channel();
        let (state_tx, state_rx) = tokio::sync::mpsc::unbounded_channel();

        cx.set_tray_icon(Some(tray_icon_bytes()));
        if !cx.is_tray_available() {
            cx.set_tray_icon(None);
            return Err(anyhow!("native tray backend unavailable"));
        }
        cx.set_tray_tooltip("ShellDeck");
        cx.set_tray_menu(build_menu(&TrayState::default()));
        cx.on_tray_menu_action(move |id, _cx| {
            let Some(command) = command_for_menu_id(id.as_ref()) else {
                return;
            };
            if let Err(error) = cmd_tx.send(command) {
                tracing::warn!(%error, "tray event dropped because its consumer stopped");
            }
        });

        Ok(Self {
            rx: Some(cmd_rx),
            state_tx: Some(state_tx),
            state_rx: Some(state_rx),
        })
    }

    /// Rebuilds GPUI's immutable native menu for each workspace snapshot.
    pub fn start_state_updates(&mut self, cx: &App) {
        let mut state_rx = self
            .state_rx
            .take()
            .expect("TrayService::start_state_updates called twice");
        cx.spawn(async move |cx| {
            while let Some(state) = state_rx.recv().await {
                let menu = build_menu(&state);
                if cx.update(|cx| cx.set_tray_menu(menu)).is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    pub fn take_receiver(&mut self) -> UnboundedReceiver<TrayCommand> {
        self.rx
            .take()
            .expect("TrayService::take_receiver called twice")
    }

    pub fn take_state_sender(&mut self) -> UnboundedSender<TrayState> {
        self.state_tx
            .take()
            .expect("TrayService::take_state_sender called twice")
    }
}

fn action(label: impl Into<SharedString>, id: &'static str) -> TrayMenuItem {
    TrayMenuItem::Action {
        label: label.into(),
        id: id.into(),
    }
}

fn label(text: impl Into<SharedString>) -> TrayMenuItem {
    TrayMenuItem::Label { label: text.into() }
}

fn protected_action(
    items: &mut Vec<TrayMenuItem>,
    signed_in: bool,
    text: impl Into<SharedString>,
    id: &'static str,
) {
    let text = text.into();
    if signed_in {
        items.push(action(text, id));
    } else {
        items.push(label(text));
    }
}

fn build_menu(state: &TrayState) -> Vec<TrayMenuItem> {
    let labels = &state.labels;
    let mut items = Vec::new();
    protected_action(
        &mut items,
        state.signed_in,
        labels.assistant.clone(),
        ASSISTANT_ID,
    );
    protected_action(
        &mut items,
        state.signed_in,
        labels.clippy.clone(),
        CLIPPY_ID,
    );
    items.push(action(labels.show.clone(), SHOW_ID));
    items.push(action(labels.palette.clone(), PALETTE_ID));
    protected_action(
        &mut items,
        state.signed_in,
        labels.choose_character.clone(),
        CHOOSE_CHARACTER_ID,
    );
    protected_action(
        &mut items,
        state.signed_in,
        labels.pause_character.clone(),
        PAUSE_CHARACTER_ID,
    );
    protected_action(
        &mut items,
        state.signed_in,
        labels.return_character.clone(),
        RETURN_CHARACTER_ID,
    );

    let pinned = if state.signed_in && !state.pinned_connections.is_empty() {
        state
            .pinned_connections
            .iter()
            .map(|connection| TrayMenuItem::Action {
                label: connection.name.clone().into(),
                id: format!("{PINNED_ID_PREFIX}{}", connection.id).into(),
            })
            .collect()
    } else {
        vec![label(labels.no_pinned.clone())]
    };
    items.push(TrayMenuItem::Submenu {
        label: labels.pinned.clone().into(),
        items: pinned,
    });
    items.push(TrayMenuItem::Separator);
    items.push(label(counter_label_ssh(state.active_ssh)));
    items.push(label(counter_label_tunnels(state.open_tunnels)));
    items.push(label(counter_label_tickets(state.unread_tickets)));
    items.push(label(counter_label_monique(state.monique_pending)));
    if state.signed_in {
        items.push(action(
            counter_label_ai_tasks(state.ai_tasks_running),
            AI_TASKS_ID,
        ));
    } else {
        items.push(label(counter_label_ai_tasks(state.ai_tasks_running)));
    }
    items.push(TrayMenuItem::Separator);
    items.push(action(labels.quit.clone(), QUIT_ID));
    items
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

fn tray_icon_bytes() -> &'static [u8] {
    #[cfg(target_os = "macos")]
    return include_bytes!("../../../../packaging/icons/shelldeck-tray-template-macos.png");
    #[cfg(target_os = "windows")]
    return include_bytes!("../../../../packaging/icons/shelldeck.ico");
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    return include_bytes!("../../../../packaging/icons/shelldeck-32.png");
}

#[cfg(test)]
mod tests {
    use super::*;

    // SDTEST-1336
    #[test]
    fn pinned_menu_id_routes_to_connection() {
        let id = Uuid::new_v4();
        assert!(matches!(
            command_for_menu_id(&format!("{PINNED_ID_PREFIX}{id}")),
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
    #[test]
    fn native_menu_snapshot_preserves_fail_closed_actions_counters_and_pins() {
        let signed_out = build_menu(&TrayState::default());
        assert!(matches!(
            signed_out.first(),
            Some(TrayMenuItem::Label { .. })
        ));
        let pinned_id = Uuid::new_v4();
        let signed_in = build_menu(&TrayState {
            signed_in: true,
            active_ssh: 2,
            pinned_connections: vec![PinnedConnection {
                id: pinned_id,
                name: "Production".into(),
            }],
            ..TrayState::default()
        });
        assert!(
            matches!(signed_in.first(), Some(TrayMenuItem::Action { id, .. }) if id.as_ref() == ASSISTANT_ID)
        );
        assert!(signed_in.iter().any(|item| {
            matches!(item, TrayMenuItem::Label { label } if label.as_ref() == counter_label_ssh(2))
        }));
        assert!(signed_in.iter().any(|item| {
            matches!(item, TrayMenuItem::Submenu { items, .. } if matches!(
                items.as_slice(),
                [TrayMenuItem::Action { label, id }]
                    if label.as_ref() == "Production"
                        && id.as_ref() == format!("{PINNED_ID_PREFIX}{pinned_id}")
            ))
        }));
    }

    // SDTEST-1337
    #[test]
    fn unknown_or_malformed_menu_id_is_ignored() {
        assert!(command_for_menu_id("shelldeck.tray.counter.ssh").is_none());
        assert!(command_for_menu_id("shelldeck.tray.pinned.invalid").is_none());
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
                assert_eq!(&pixel.0[..3], &[0, 0, 0]);
            }
        }
        assert!(visible > 36 * 36 / 8);
        assert!(visible < 36 * 36 * 3 / 4);
        for (x, y) in [(0, 0), (35, 0), (0, 35), (35, 35)] {
            assert_eq!(image.get_pixel(x, y).0[3], 0);
        }
    }

    // SDTEST-1812
    #[cfg(target_os = "windows")]
    #[test]
    fn sdtest_1812_windows_tray_asset_is_a_decodable_ico() {
        let bytes = tray_icon_bytes();
        assert_eq!(&bytes[..6], &[0, 0, 1, 0, 6, 0]);
        let icon = image::load_from_memory_with_format(bytes, image::ImageFormat::Ico)
            .expect("decode Windows tray ICO");
        assert_eq!(icon.width(), icon.height());
        assert!(icon.width() >= 32);
    }
}
