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
/// Steps through the full app and the focused Agents, Terminal and Editor views.
pub const CYCLE_PRESENTATION_BINDING: &str = "secondary-shift-l";
pub const QUIT_BINDING: &str = "secondary-q";
pub const MACOS_SPLIT_BINDING: &str = "cmd-d";
pub const OTHER_SPLIT_BINDING: &str = "ctrl-shift-d";
pub const MACOS_COPY_BINDING: &str = "cmd-c";
pub const OTHER_COPY_BINDING: &str = "ctrl-shift-c";
pub const MACOS_PASTE_BINDING: &str = "cmd-v";
pub const OTHER_PASTE_BINDING: &str = "ctrl-shift-v";

/// An application shortcut the user can change in the `[keybindings]` table
/// of the configuration file, by its [`id`](Self::id).
///
/// This is the one list both the key registration (`shelldeck/src/actions.rs`)
/// and every label (menus, palette, tooltips, shortcut references) read, so a
/// shortcut is never shown with keys it is not bound to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum BindableAction {
    QuickConnect,
    AiAssistant,
    NewTerminal,
    ToggleSidebar,
    ToggleMenuBar,
    Settings,
    NextTab,
    PrevTab,
    CloseTab,
    ClearTerminal,
    Search,
    ZoomIn,
    ZoomOut,
    ZoomReset,
    CommandPalette,
    NextView,
    FileEditor,
    ToggleSplitFocus,
    Quit,
    SplitHorizontal,
    SplitVertical,
    MaximizePane,
    Copy,
    Paste,
    NextProject,
    PrevProject,
}

impl BindableAction {
    pub const ALL: [Self; 26] = [
        Self::QuickConnect,
        Self::AiAssistant,
        Self::NewTerminal,
        Self::ToggleSidebar,
        Self::ToggleMenuBar,
        Self::Settings,
        Self::NextTab,
        Self::PrevTab,
        Self::CloseTab,
        Self::ClearTerminal,
        Self::Search,
        Self::ZoomIn,
        Self::ZoomOut,
        Self::ZoomReset,
        Self::CommandPalette,
        Self::NextView,
        Self::FileEditor,
        Self::ToggleSplitFocus,
        Self::Quit,
        Self::SplitHorizontal,
        Self::SplitVertical,
        Self::MaximizePane,
        Self::Copy,
        Self::Paste,
        Self::NextProject,
        Self::PrevProject,
    ];

    /// The key of this action in `[keybindings]`. Part of the configuration
    /// file format: never rename one.
    pub fn id(self) -> &'static str {
        match self {
            Self::QuickConnect => "quick_connect",
            Self::AiAssistant => "ai_assistant",
            Self::NewTerminal => "new_terminal",
            Self::ToggleSidebar => "toggle_sidebar",
            Self::ToggleMenuBar => "toggle_menu_bar",
            Self::Settings => "settings",
            Self::NextTab => "next_tab",
            Self::PrevTab => "previous_tab",
            Self::CloseTab => "close_tab",
            Self::ClearTerminal => "clear_terminal",
            Self::Search => "search",
            Self::ZoomIn => "zoom_in",
            Self::ZoomOut => "zoom_out",
            Self::ZoomReset => "zoom_reset",
            Self::CommandPalette => "command_palette",
            Self::NextView => "next_view",
            Self::FileEditor => "file_editor",
            Self::ToggleSplitFocus => "toggle_split_focus",
            Self::Quit => "quit",
            Self::SplitHorizontal => "split_horizontal",
            Self::SplitVertical => "split_vertical",
            Self::MaximizePane => "maximize_pane",
            Self::Copy => "copy",
            Self::Paste => "paste",
            Self::NextProject => "next_project",
            Self::PrevProject => "previous_project",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|action| action.id() == id)
    }

    /// The built-in keys. Split, copy and paste differ by platform: with
    /// Ctrl they would collide with terminal control characters on Linux
    /// and Windows. Empty: the action has no keys until the user gives it
    /// some (next and previous project: every free chord left is one a
    /// program in the terminal may want).
    fn default_keys_for(self, macos: bool) -> &'static str {
        match self {
            Self::QuickConnect => "secondary-k",
            Self::AiAssistant => "secondary-shift-k",
            Self::NewTerminal => NEW_TERMINAL_BINDING,
            Self::ToggleSidebar => TOGGLE_SIDEBAR_BINDING,
            Self::ToggleMenuBar => "secondary-shift-m",
            Self::Settings => SETTINGS_BINDING,
            Self::NextTab => NEXT_TAB_BINDING,
            Self::PrevTab => "ctrl-shift-tab",
            Self::CloseTab => CLOSE_TAB_BINDING,
            Self::ClearTerminal => CLEAR_TERMINAL_BINDING,
            Self::Search => SEARCH_BINDING,
            Self::ZoomIn => ZOOM_IN_BINDING,
            Self::ZoomOut => ZOOM_OUT_BINDING,
            Self::ZoomReset => ZOOM_RESET_BINDING,
            Self::CommandPalette => COMMAND_PALETTE_BINDING,
            Self::NextView => CYCLE_PRESENTATION_BINDING,
            Self::FileEditor => "secondary-e",
            Self::ToggleSplitFocus => "alt-[",
            Self::Quit => QUIT_BINDING,
            Self::SplitHorizontal if macos => MACOS_SPLIT_BINDING,
            Self::SplitHorizontal => OTHER_SPLIT_BINDING,
            Self::SplitVertical if macos => "cmd-shift-d",
            Self::SplitVertical => "ctrl-shift-alt-d",
            Self::MaximizePane => "secondary-shift-enter",
            Self::Copy if macos => MACOS_COPY_BINDING,
            Self::Copy => OTHER_COPY_BINDING,
            Self::Paste if macos => MACOS_PASTE_BINDING,
            Self::Paste => OTHER_PASTE_BINDING,
            Self::NextProject | Self::PrevProject => "",
        }
    }
}

/// Why an entry of `[keybindings]` was not applied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeybindingProblem {
    /// The table names an action that does not exist.
    UnknownAction(String),
    /// The keys cannot be parsed.
    InvalidKeys { action: &'static str, keys: String },
    /// Several actions would answer to the same keys. Every changed action
    /// among them keeps its built-in keys.
    Conflict {
        keys: String,
        actions: Vec<&'static str>,
    },
}

impl KeybindingProblem {
    pub fn message(&self) -> String {
        match self {
            Self::UnknownAction(name) => {
                t!("keybindings.problem.unknown", name = name.as_str()).to_string()
            }
            Self::InvalidKeys { action, keys } => t!(
                "keybindings.problem.invalid",
                action = *action,
                keys = keys.as_str()
            )
            .to_string(),
            Self::Conflict { keys, actions } => t!(
                "keybindings.problem.conflict",
                keys = display_accelerator(keys),
                actions = actions.join(", ")
            )
            .to_string(),
        }
    }
}

type Overrides = std::collections::BTreeMap<BindableAction, String>;

/// The user's changes, by action. An empty value unbinds the action.
static OVERRIDES: std::sync::RwLock<Overrides> = std::sync::RwLock::new(Overrides::new());
static PROBLEMS: std::sync::Mutex<Vec<KeybindingProblem>> = std::sync::Mutex::new(Vec::new());

/// One keystroke in a canonical spelling, so `shift-ctrl-d`, `Ctrl-Shift-D`
/// and (off macOS) `secondary-shift-d` compare equal.
fn normalize_keys(keys: &str, macos: bool) -> String {
    keys.split_whitespace()
        .map(|stroke| {
            let stroke = stroke.to_ascii_lowercase();
            // The key itself may be `-`, as in `secondary--`.
            let (modifiers, key) = match stroke.strip_suffix("--") {
                Some(modifiers) => (modifiers.to_string(), "-".to_string()),
                None => match stroke.rsplit_once('-') {
                    Some((modifiers, key)) => (modifiers.to_string(), key.to_string()),
                    None => (String::new(), stroke.clone()),
                },
            };
            let mut modifiers: Vec<&str> = modifiers
                .split('-')
                .filter(|modifier| !modifier.is_empty())
                .map(|modifier| match modifier {
                    "secondary" if macos => "cmd",
                    "secondary" => "ctrl",
                    "control" => "ctrl",
                    "super" | "win" | "command" => "cmd",
                    "option" => "alt",
                    other => other,
                })
                .collect();
            modifiers.sort_unstable();
            modifiers.dedup();
            modifiers.push(&key);
            modifiers.join("-")
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Turn the `[keybindings]` table into the changes to apply.
///
/// An unknown action or keys that do not parse are skipped. When several
/// actions end up on the same keys, every changed action among them goes
/// back to its built-in keys: a shortcut that silently does one of two
/// things is worse than a change that is refused and reported.
pub fn resolve_keybindings(
    config: &std::collections::BTreeMap<String, String>,
    macos: bool,
    is_valid: impl Fn(&str) -> bool,
) -> (Overrides, Vec<KeybindingProblem>) {
    let mut overrides = Overrides::new();
    let mut problems = Vec::new();
    for (name, keys) in config {
        let Some(action) = BindableAction::from_id(name.trim()) else {
            problems.push(KeybindingProblem::UnknownAction(name.clone()));
            continue;
        };
        let keys = keys.trim();
        if !keys.is_empty() && !is_valid(keys) {
            problems.push(KeybindingProblem::InvalidKeys {
                action: action.id(),
                keys: keys.to_string(),
            });
            continue;
        }
        let unchanged =
            normalize_keys(keys, macos) == normalize_keys(action.default_keys_for(macos), macos);
        if !unchanged {
            overrides.insert(action, keys.to_string());
        }
    }

    loop {
        let mut by_keys: std::collections::BTreeMap<String, Vec<BindableAction>> =
            std::collections::BTreeMap::new();
        for action in BindableAction::ALL {
            let keys = overrides
                .get(&action)
                .map_or(action.default_keys_for(macos), String::as_str);
            // Unbound, by the user or from the start: shares nothing.
            if !keys.is_empty() {
                by_keys
                    .entry(normalize_keys(keys, macos))
                    .or_default()
                    .push(action);
            }
        }
        let mut reverted = false;
        for (keys, actions) in by_keys {
            if actions.len() < 2 {
                continue;
            }
            let changed: Vec<BindableAction> = actions
                .iter()
                .copied()
                .filter(|action| overrides.contains_key(action))
                .collect();
            if changed.is_empty() {
                // Two built-in shortcuts on the same keys: a defect of the
                // defaults (SDTEST-3100), not something to loop on.
                continue;
            }
            problems.push(KeybindingProblem::Conflict {
                keys,
                actions: actions.iter().map(|action| action.id()).collect(),
            });
            for action in changed {
                overrides.remove(&action);
            }
            reverted = true;
        }
        if !reverted {
            return (overrides, problems);
        }
    }
}

/// Apply the user's `[keybindings]` for this process. Call before the keys
/// are registered. Problems are kept for [`take_keybinding_problems`].
pub fn apply_keybindings(config: &std::collections::BTreeMap<String, String>) {
    let (overrides, problems) = resolve_keybindings(config, cfg!(target_os = "macos"), |keys| {
        keys.split_whitespace()
            .all(|stroke| Keystroke::parse(stroke).is_ok())
    });
    for problem in &problems {
        tracing::warn!("keybindings: {problem:?}");
    }
    *OVERRIDES.write().unwrap_or_else(|e| e.into_inner()) = overrides;
    *PROBLEMS.lock().unwrap_or_else(|e| e.into_inner()) = problems;
}

/// What could not be applied at startup, once, for the first window to show.
pub fn take_keybinding_problems() -> Vec<KeybindingProblem> {
    std::mem::take(&mut *PROBLEMS.lock().unwrap_or_else(|e| e.into_inner()))
}

fn keys_for(action: BindableAction, macos: bool) -> Option<String> {
    let overrides = OVERRIDES.read().unwrap_or_else(|e| e.into_inner());
    match overrides.get(&action) {
        Some(keys) if keys.is_empty() => None,
        Some(keys) => Some(keys.clone()),
        None => Some(action.default_keys_for(macos).to_string()).filter(|keys| !keys.is_empty()),
    }
}

/// The keys an action is bound to, in GPUI spelling. `None` when the user
/// unbound it.
pub fn keys(action: BindableAction) -> Option<String> {
    keys_for(action, cfg!(target_os = "macos"))
}

/// The keys an action is bound to, as shown to the user (`Ctrl+Shift+L`).
pub fn label(action: BindableAction) -> Option<String> {
    keys(action).map(|keys| display_accelerator(&keys))
}

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
pub(crate) struct ShortcutDefinition {
    pub id: ShortcutId,
    pub group: ShortcutGroup,
    label_key: &'static str,
    reference: bool,
    onboarding: Option<bool>,
}

const SHORTCUTS: &[ShortcutDefinition] = &[
    ShortcutDefinition {
        id: ShortcutId::CommandPalette,
        group: ShortcutGroup::Navigation,
        label_key: "shortcuts.command_palette",
        reference: true,
        onboarding: Some(false),
    },
    ShortcutDefinition {
        id: ShortcutId::NewTerminal,
        group: ShortcutGroup::Navigation,
        label_key: "shortcuts.new_terminal",
        reference: true,
        onboarding: Some(true),
    },
    ShortcutDefinition {
        id: ShortcutId::NextTab,
        group: ShortcutGroup::Navigation,
        label_key: "shortcuts.next_tab",
        reference: true,
        onboarding: None,
    },
    ShortcutDefinition {
        id: ShortcutId::ToggleSidebar,
        group: ShortcutGroup::Navigation,
        label_key: "shortcuts.toggle_sidebar",
        reference: true,
        onboarding: Some(true),
    },
    ShortcutDefinition {
        id: ShortcutId::Settings,
        group: ShortcutGroup::Navigation,
        label_key: "shortcuts.settings",
        reference: true,
        onboarding: Some(false),
    },
    ShortcutDefinition {
        id: ShortcutId::Search,
        group: ShortcutGroup::Terminal,
        label_key: "shortcuts.search",
        reference: true,
        onboarding: None,
    },
    ShortcutDefinition {
        id: ShortcutId::Split,
        group: ShortcutGroup::Terminal,
        label_key: "shortcuts.split",
        reference: true,
        onboarding: None,
    },
    ShortcutDefinition {
        id: ShortcutId::Copy,
        group: ShortcutGroup::Terminal,
        label_key: "shortcuts.copy",
        reference: true,
        onboarding: None,
    },
    ShortcutDefinition {
        id: ShortcutId::Paste,
        group: ShortcutGroup::Terminal,
        label_key: "shortcuts.paste",
        reference: true,
        onboarding: None,
    },
    ShortcutDefinition {
        id: ShortcutId::ClearTerminal,
        group: ShortcutGroup::Terminal,
        label_key: "shortcuts.clear_terminal",
        reference: true,
        onboarding: None,
    },
    ShortcutDefinition {
        id: ShortcutId::Zoom,
        group: ShortcutGroup::Terminal,
        label_key: "shortcuts.zoom",
        reference: true,
        onboarding: None,
    },
    ShortcutDefinition {
        id: ShortcutId::CloseTab,
        group: ShortcutGroup::Navigation,
        label_key: "shortcuts.close_tab",
        reference: false,
        onboarding: None,
    },
    ShortcutDefinition {
        id: ShortcutId::Quit,
        group: ShortcutGroup::Navigation,
        label_key: "shortcuts.quit",
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
        // A row for a shortcut the user removed would show an empty key.
        .filter(|shortcut| !shortcut_keys_for(*shortcut, cfg!(target_os = "macos")).is_empty())
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

impl ShortcutId {
    /// The actions a reference row stands for. Zoom shows two.
    fn actions(self) -> &'static [BindableAction] {
        match self {
            Self::CommandPalette => &[BindableAction::CommandPalette],
            Self::NewTerminal => &[BindableAction::NewTerminal],
            Self::NextTab => &[BindableAction::NextTab],
            Self::ToggleSidebar => &[BindableAction::ToggleSidebar],
            Self::Settings => &[BindableAction::Settings],
            Self::Search => &[BindableAction::Search],
            Self::Split => &[BindableAction::SplitHorizontal],
            Self::Copy => &[BindableAction::Copy],
            Self::Paste => &[BindableAction::Paste],
            Self::ClearTerminal => &[BindableAction::ClearTerminal],
            Self::Zoom => &[BindableAction::ZoomIn, BindableAction::ZoomOut],
            Self::CloseTab => &[BindableAction::CloseTab],
            Self::Quit => &[BindableAction::Quit],
        }
    }
}

/// The keys of a reference row as shown to the user. Empty when the user
/// unbound every action of the row.
fn shortcut_keys_for(shortcut: ShortcutDefinition, macos: bool) -> String {
    shortcut
        .id
        .actions()
        .iter()
        .filter_map(|action| keys_for(*action, macos))
        .map(|keys| display_accelerator_for(&keys, macos))
        .collect::<Vec<_>>()
        .join(" / ")
}

/// Shared, non-interactive shortcut reference. Keeping the row here avoids
/// making the same binding look like four different controls across the app.
fn render_shortcut_row(shortcut: ShortcutDefinition, width: Option<f32>) -> Stateful<Div> {
    let stacked = width.is_some_and(|width| width < 280.0);
    let label_id = format!("shortcut-label-{:?}", shortcut.id);
    let mut label = div()
        .id(SharedString::from(label_id.clone()))
        .debug_selector(move || label_id.clone())
        .min_w(px(0.0))
        .text_size(px(12.0))
        .text_color(ShellDeckColors::text_muted())
        .child(t!(shortcut.label_key).to_string());
    if let Some(width) = width {
        label = label
            .w(px(if stacked { width } else { width - 144.0 }))
            .line_height(px(16.0))
            .line_clamp(2)
            .max_h(px(32.0))
            .flex_shrink_0();
    }
    let row_id: SharedString = format!("shortcut-row-{:?}", shortcut.id).into();
    let mut row = div().flex().w_full().gap(px(12.0)).py(px(5.0));
    if stacked {
        row = row.flex_col().items_start().gap(px(6.0));
    } else {
        row = row.items_center().justify_between();
    }
    if let Some(width) = width {
        row = row.w(px(width));
    }
    let row = row.child(label).child(
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
    );
    crate::terminal_view::trace_layout_for_preview(row_id.clone(), row).id(row_id)
}

pub(crate) fn render_shortcut_rows(shortcuts: impl IntoIterator<Item = ShortcutDefinition>) -> Div {
    shortcuts.into_iter().fold(
        div().flex().flex_col().w_full().gap(px(2.0)),
        |rows, item| rows.child(render_shortcut_row(item, None)),
    )
}

pub(crate) fn render_shortcut_rows_with_width(
    shortcuts: impl IntoIterator<Item = ShortcutDefinition>,
    width: f32,
) -> Div {
    shortcuts.into_iter().fold(
        div().flex().flex_col().w(px(width)).gap(px(2.0)),
        |rows, item| rows.child(render_shortcut_row(item, Some(width))),
    )
}

#[cfg(test)]
mod tests {
    use super::{
        normalize_keys, resolve_keybindings, shortcut_keys_for, shortcuts_for, BindableAction,
        KeybindingProblem, ShortcutId, ShortcutSurface, SHORTCUTS,
    };
    use std::collections::BTreeMap;

    fn table(entries: &[(&str, &str)]) -> BTreeMap<String, String> {
        entries
            .iter()
            .map(|(name, keys)| (name.to_string(), keys.to_string()))
            .collect()
    }

    // SDTEST-3100 / SDUC-577. The built-in shortcuts never share keys, on
    // either platform, and their ids are distinct: the conflict rule of the
    // user's table relies on both.
    #[test]
    fn sdtest_3100_built_in_shortcuts_have_distinct_ids_and_keys() {
        for macos in [false, true] {
            let mut seen = BTreeMap::new();
            for action in BindableAction::ALL {
                let keys = normalize_keys(action.default_keys_for(macos), macos);
                // An action without built-in keys shares none.
                if keys.is_empty() {
                    continue;
                }
                if let Some(other) = seen.insert(keys.clone(), action) {
                    panic!("{other:?} and {action:?} share {keys} (macos: {macos})");
                }
            }
        }
        let mut ids: Vec<&str> = BindableAction::ALL.iter().map(|a| a.id()).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), BindableAction::ALL.len());
        for action in BindableAction::ALL {
            assert_eq!(BindableAction::from_id(action.id()), Some(action));
        }
    }

    // SDTEST-3101 / SDUC-577. The table changes and removes shortcuts; what
    // cannot be applied is reported and leaves the built-in keys in place.
    #[test]
    fn sdtest_3101_the_keybindings_table_rebinds_unbinds_and_reports() {
        let valid = |keys: &str| !keys.contains("nonsense");
        let resolve = |entries: &[(&str, &str)]| resolve_keybindings(&table(entries), false, valid);

        // A change, a removal (frees Ctrl+B for tmux) and a no-op.
        let (overrides, problems) = resolve(&[
            ("next_view", "ctrl-alt-v"),
            ("toggle_sidebar", ""),
            ("new_terminal", "Ctrl-T"),
        ]);
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(
            overrides.get(&BindableAction::NextView).unwrap(),
            "ctrl-alt-v"
        );
        assert_eq!(overrides.get(&BindableAction::ToggleSidebar).unwrap(), "");
        assert!(
            !overrides.contains_key(&BindableAction::NewTerminal),
            "the built-in keys in another spelling are not a change"
        );

        let (overrides, problems) =
            resolve(&[("open_the_pod_bay_doors", "ctrl-o"), ("quit", "nonsense-q")]);
        assert!(overrides.is_empty());
        assert_eq!(
            problems,
            vec![
                KeybindingProblem::UnknownAction("open_the_pod_bay_doors".into()),
                KeybindingProblem::InvalidKeys {
                    action: "quit",
                    keys: "nonsense-q".into()
                },
            ]
        );
    }

    // SDTEST-3102 / SDUC-577. One set of keys never means two actions: a
    // change that lands on keys in use is refused, unless the other action
    // was moved or removed in the same table.
    #[test]
    fn sdtest_3102_a_change_onto_keys_in_use_is_refused_and_reported() {
        let resolve =
            |entries: &[(&str, &str)]| resolve_keybindings(&table(entries), false, |_| true);

        // Ctrl+T is "new terminal".
        let (overrides, problems) = resolve(&[("next_view", "shift-ctrl-t"), ("quit", "ctrl-t")]);
        assert!(overrides.contains_key(&BindableAction::NextView));
        assert!(!overrides.contains_key(&BindableAction::Quit));
        assert_eq!(
            problems,
            vec![KeybindingProblem::Conflict {
                keys: "ctrl-t".into(),
                actions: vec!["new_terminal", "quit"],
            }]
        );

        // Moving the owner out of the way makes the same change valid.
        let (overrides, problems) = resolve(&[("quit", "ctrl-t"), ("new_terminal", "ctrl-alt-t")]);
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(overrides.len(), 2);
        // So does removing it.
        let (overrides, problems) = resolve(&[("quit", "ctrl-t"), ("new_terminal", "")]);
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(overrides.get(&BindableAction::Quit).unwrap(), "ctrl-t");

        // Two changes onto the same new keys: both refused. Reverting one
        // must not leave the other bound to keys the report called refused.
        let (overrides, problems) = resolve(&[("quit", "ctrl-alt-x"), ("search", "alt-ctrl-x")]);
        assert!(overrides.is_empty());
        assert_eq!(problems.len(), 1);

        // A swap is two changes that free each other.
        let (overrides, problems) = resolve(&[
            ("new_terminal", "secondary-w"),
            ("close_tab", "secondary-t"),
        ]);
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(overrides.len(), 2);
        assert_eq!(normalize_keys("secondary--", false), "ctrl--");
        assert_eq!(normalize_keys("Shift-Secondary-P", true), "cmd-shift-p");
    }

    // SDTEST-3151 / SDUC-584. Next and previous project have no built-in
    // keys: nothing is registered for them until the table gives them
    // some, and those keys obey the same rule as every other change.
    #[test]
    fn sdtest_3151_the_project_actions_have_no_keys_until_the_table_gives_some() {
        let resolve =
            |entries: &[(&str, &str)]| resolve_keybindings(&table(entries), false, |_| true);
        for macos in [false, true] {
            assert_eq!(super::keys_for(BindableAction::NextProject, macos), None);
            assert_eq!(super::keys_for(BindableAction::PrevProject, macos), None);
        }

        let (overrides, problems) = resolve(&[
            ("next_project", "ctrl-alt-]"),
            ("previous_project", "ctrl-alt-["),
        ]);
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(
            overrides.get(&BindableAction::NextProject).unwrap(),
            "ctrl-alt-]"
        );
        assert_eq!(overrides.len(), 2);

        // Leaving them without keys is not a change, and not a conflict
        // between the two.
        let (overrides, problems) = resolve(&[("next_project", ""), ("previous_project", "")]);
        assert!(overrides.is_empty() && problems.is_empty(), "{problems:?}");

        // Keys in use stay with their action.
        let (overrides, problems) = resolve(&[("next_project", "ctrl-tab")]);
        assert!(overrides.is_empty());
        assert_eq!(
            problems,
            vec![KeybindingProblem::Conflict {
                keys: "ctrl-tab".into(),
                actions: vec!["next_tab", "next_project"],
            }]
        );
    }

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
