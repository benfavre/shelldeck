use crate::scale::px;
use crate::t;
use crate::theme::ShellDeckColors;
use gpui::prelude::*;
use gpui::*;

pub const NEW_TERMINAL_BINDING: &str = "secondary-t";
pub const NEXT_TAB_BINDING: &str = "ctrl-tab";
pub const TOGGLE_SIDEBAR_BINDING: &str = "secondary-b";
pub const SETTINGS_BINDING: &str = "secondary-,";
pub const COMMAND_PALETTE_BINDING: &str = "secondary-shift-p";
pub const SEARCH_BINDING: &str = "secondary-f";
pub const CLEAR_TERMINAL_BINDING: &str = "secondary-l";
pub const ZOOM_IN_BINDING: &str = "secondary-=";
pub const ZOOM_OUT_BINDING: &str = "secondary--";
pub const ZOOM_RESET_BINDING: &str = "secondary-0";
pub const CLOSE_TAB_BINDING: &str = "secondary-w";
pub const QUIT_BINDING: &str = "secondary-q";
pub const MACOS_SPLIT_BINDING: &str = "cmd-d";
pub const OTHER_SPLIT_BINDING: &str = "ctrl-shift-d";
pub const MACOS_COPY_BINDING: &str = "cmd-c";
pub const OTHER_COPY_BINDING: &str = "ctrl-shift-c";
pub const MACOS_PASTE_BINDING: &str = "cmd-v";
pub const OTHER_PASTE_BINDING: &str = "ctrl-shift-v";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ShortcutGroup {
    Navigation,
    Terminal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ShortcutSurface {
    Dashboard,
    TerminalEmpty,
    Onboarding,
    About,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ShortcutId {
    CommandPalette,
    NewTerminal,
    NextTab,
    ToggleSidebar,
    Settings,
    Search,
    Split,
    Copy,
    Paste,
    ClearTerminal,
    Zoom,
    CloseTab,
    Quit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ShortcutBinding {
    Single(&'static str),
    Platform {
        macos: &'static str,
        other: &'static str,
    },
    Pair(&'static str, &'static str),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ShortcutDefinition {
    pub id: ShortcutId,
    pub group: ShortcutGroup,
    label_key: &'static str,
    binding: ShortcutBinding,
    reference: bool,
    onboarding: Option<bool>,
}

const SHORTCUTS: &[ShortcutDefinition] = &[
    ShortcutDefinition {
        id: ShortcutId::CommandPalette,
        group: ShortcutGroup::Navigation,
        label_key: "shortcuts.command_palette",
        binding: ShortcutBinding::Single(COMMAND_PALETTE_BINDING),
        reference: true,
        onboarding: Some(false),
    },
    ShortcutDefinition {
        id: ShortcutId::NewTerminal,
        group: ShortcutGroup::Navigation,
        label_key: "shortcuts.new_terminal",
        binding: ShortcutBinding::Single(NEW_TERMINAL_BINDING),
        reference: true,
        onboarding: Some(true),
    },
    ShortcutDefinition {
        id: ShortcutId::NextTab,
        group: ShortcutGroup::Navigation,
        label_key: "shortcuts.next_tab",
        binding: ShortcutBinding::Single(NEXT_TAB_BINDING),
        reference: true,
        onboarding: None,
    },
    ShortcutDefinition {
        id: ShortcutId::ToggleSidebar,
        group: ShortcutGroup::Navigation,
        label_key: "shortcuts.toggle_sidebar",
        binding: ShortcutBinding::Single(TOGGLE_SIDEBAR_BINDING),
        reference: true,
        onboarding: Some(true),
    },
    ShortcutDefinition {
        id: ShortcutId::Settings,
        group: ShortcutGroup::Navigation,
        label_key: "shortcuts.settings",
        binding: ShortcutBinding::Single(SETTINGS_BINDING),
        reference: true,
        onboarding: Some(false),
    },
    ShortcutDefinition {
        id: ShortcutId::Search,
        group: ShortcutGroup::Terminal,
        label_key: "shortcuts.search",
        binding: ShortcutBinding::Single(SEARCH_BINDING),
        reference: true,
        onboarding: None,
    },
    ShortcutDefinition {
        id: ShortcutId::Split,
        group: ShortcutGroup::Terminal,
        label_key: "shortcuts.split",
        binding: ShortcutBinding::Platform {
            macos: MACOS_SPLIT_BINDING,
            other: OTHER_SPLIT_BINDING,
        },
        reference: true,
        onboarding: None,
    },
    ShortcutDefinition {
        id: ShortcutId::Copy,
        group: ShortcutGroup::Terminal,
        label_key: "shortcuts.copy",
        binding: ShortcutBinding::Platform {
            macos: MACOS_COPY_BINDING,
            other: OTHER_COPY_BINDING,
        },
        reference: true,
        onboarding: None,
    },
    ShortcutDefinition {
        id: ShortcutId::Paste,
        group: ShortcutGroup::Terminal,
        label_key: "shortcuts.paste",
        binding: ShortcutBinding::Platform {
            macos: MACOS_PASTE_BINDING,
            other: OTHER_PASTE_BINDING,
        },
        reference: true,
        onboarding: None,
    },
    ShortcutDefinition {
        id: ShortcutId::ClearTerminal,
        group: ShortcutGroup::Terminal,
        label_key: "shortcuts.clear_terminal",
        binding: ShortcutBinding::Single(CLEAR_TERMINAL_BINDING),
        reference: true,
        onboarding: None,
    },
    ShortcutDefinition {
        id: ShortcutId::Zoom,
        group: ShortcutGroup::Terminal,
        label_key: "shortcuts.zoom",
        binding: ShortcutBinding::Pair(ZOOM_IN_BINDING, ZOOM_OUT_BINDING),
        reference: true,
        onboarding: None,
    },
    ShortcutDefinition {
        id: ShortcutId::CloseTab,
        group: ShortcutGroup::Navigation,
        label_key: "shortcuts.close_tab",
        binding: ShortcutBinding::Single(CLOSE_TAB_BINDING),
        reference: false,
        onboarding: None,
    },
    ShortcutDefinition {
        id: ShortcutId::Quit,
        group: ShortcutGroup::Navigation,
        label_key: "shortcuts.quit",
        binding: ShortcutBinding::Single(QUIT_BINDING),
        reference: false,
        onboarding: None,
    },
];

pub(crate) fn shortcuts_for(
    surface: ShortcutSurface,
    dev_capable: bool,
) -> Vec<ShortcutDefinition> {
    SHORTCUTS
        .iter()
        .copied()
        .filter(|shortcut| match surface {
            ShortcutSurface::Dashboard | ShortcutSurface::TerminalEmpty => shortcut.reference,
            ShortcutSurface::Onboarding => shortcut
                .onboarding
                .is_some_and(|requires_dev| !requires_dev || dev_capable),
            ShortcutSurface::About => true,
        })
        .collect()
}

fn display_accelerator_for(keys: &str, macos: bool) -> String {
    let secondary = if macos { "Cmd" } else { "Ctrl" };
    let mut parts = if let Some(prefix) = keys.strip_suffix("--") {
        prefix.split('-').collect::<Vec<_>>()
    } else {
        keys.split('-').collect::<Vec<_>>()
    };
    if keys.ends_with("--") {
        parts.push("-");
    }
    parts
        .into_iter()
        .map(|part| match part {
            "secondary" => secondary.to_string(),
            "cmd" => "Cmd".to_string(),
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

pub(crate) fn display_accelerator(keys: &str) -> String {
    display_accelerator_for(keys, cfg!(target_os = "macos"))
}

fn shortcut_keys_for(shortcut: ShortcutDefinition, macos: bool) -> String {
    match shortcut.binding {
        ShortcutBinding::Single(keys) => display_accelerator_for(keys, macos),
        ShortcutBinding::Platform { macos: mac, other } => {
            display_accelerator_for(if macos { mac } else { other }, macos)
        }
        ShortcutBinding::Pair(first, second) => format!(
            "{} / {}",
            display_accelerator_for(first, macos),
            display_accelerator_for(second, macos)
        ),
    }
}

/// Shared, non-interactive shortcut reference. Keeping the row here avoids
/// making the same binding look like four different controls across the app.
fn render_shortcut_row(shortcut: ShortcutDefinition) -> Div {
    div()
        .flex()
        .items_center()
        .justify_between()
        .w_full()
        .gap(px(12.0))
        .py(px(5.0))
        .child(
            div()
                .min_w(px(0.0))
                .text_size(px(12.0))
                .text_color(ShellDeckColors::text_muted())
                .child(t!(shortcut.label_key).to_string()),
        )
        .child(
            div()
                .flex_shrink_0()
                .px(px(8.0))
                .py(px(3.0))
                .rounded(px(6.0))
                .bg(ShellDeckColors::bg_sidebar())
                .border_1()
                .border_color(ShellDeckColors::border())
                .text_size(px(11.0))
                .font_weight(FontWeight::MEDIUM)
                .text_color(ShellDeckColors::text_primary())
                .child(shortcut_keys_for(shortcut, cfg!(target_os = "macos"))),
        )
}

pub(crate) fn render_shortcut_rows(shortcuts: impl IntoIterator<Item = ShortcutDefinition>) -> Div {
    shortcuts.into_iter().fold(
        div().flex().flex_col().w_full().gap(px(2.0)),
        |rows, item| rows.child(render_shortcut_row(item)),
    )
}

#[cfg(test)]
mod tests {
    use super::{shortcut_keys_for, shortcuts_for, ShortcutId, ShortcutSurface, SHORTCUTS};

    // SDTEST-1720 — the two full Dev references are identical, onboarding is
    // a capability-safe ordered subset, and every displayed key is generated
    // from the same binding vocabulary registered by the application.
    #[test]
    fn shortcut_surfaces_share_one_ordered_platform_aware_catalog() {
        let ids = |surface, dev| {
            shortcuts_for(surface, dev)
                .into_iter()
                .map(|shortcut| shortcut.id)
                .collect::<Vec<_>>()
        };

        assert_eq!(
            ids(ShortcutSurface::Dashboard, true),
            ids(ShortcutSurface::TerminalEmpty, true)
        );
        assert_eq!(
            ids(ShortcutSurface::Onboarding, false),
            vec![ShortcutId::CommandPalette, ShortcutId::Settings]
        );
        assert_eq!(
            ids(ShortcutSurface::Onboarding, true),
            vec![
                ShortcutId::CommandPalette,
                ShortcutId::NewTerminal,
                ShortcutId::ToggleSidebar,
                ShortcutId::Settings,
            ]
        );

        let split = SHORTCUTS
            .iter()
            .copied()
            .find(|shortcut| shortcut.id == ShortcutId::Split)
            .unwrap();
        let zoom = SHORTCUTS
            .iter()
            .copied()
            .find(|shortcut| shortcut.id == ShortcutId::Zoom)
            .unwrap();
        assert_eq!(shortcut_keys_for(split, true), "Cmd+D");
        assert_eq!(shortcut_keys_for(split, false), "Ctrl+Shift+D");
        assert_eq!(shortcut_keys_for(zoom, true), "Cmd+= / Cmd+-");
        assert_eq!(shortcut_keys_for(zoom, false), "Ctrl+= / Ctrl+-");

        for shortcut in SHORTCUTS {
            let label = crate::t!(shortcut.label_key).to_string();
            assert_ne!(label, shortcut.label_key);
            assert!(!label.trim().is_empty());
        }
    }
}
