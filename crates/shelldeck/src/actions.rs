use gpui::*;
use shelldeck_ui::shortcut_reference::BindableAction;

// Re-export workspace actions for keybinding registration
pub use shelldeck_ui::workspace::{
    CheckForUpdates, CloseTab, CloudSyncNow, ConnectBextCloud, CyclePresentation, NewRequest,
    NewTerminal, NextTab, OpenAiAssistant, OpenBextCloud, OpenFileEditorView, OpenFleet,
    OpenMoniqueConsole, OpenQuickConnect, OpenSettings, OpenSupportRequests, PrevTab, Quit,
    SwitchSite, ToggleMenuBar, ToggleSidebar,
};

// Re-export terminal view actions
pub use shelldeck_ui::command_palette::{
    ApplyTerminalTheme, OpenManageArea, SetAppMode, SetWorkspacePresentation, ToggleCommandPalette,
};
pub use shelldeck_ui::terminal_view::{
    ClearTerminal, CopySelection, PasteClipboard, SplitHorizontal, SplitVertical, TogglePaneZoom,
    ToggleSearch, ToggleSplitFocus, ZoomIn, ZoomOut, ZoomReset,
};

/// Register the application shortcuts.
///
/// The keys come from `shortcut_reference::keys`: the built-in ones, or the
/// user's `[keybindings]` (apply those first). The built-in keys use the
/// `secondary` modifier (Cmd on macOS, Ctrl elsewhere); split, copy and paste
/// differ by platform because Ctrl+C (SIGINT), Ctrl+V (literal-next) and
/// Ctrl+D (EOF) are terminal control characters on Linux and Windows.
pub fn register_keybindings(cx: &mut App) {
    use BindableAction as B;
    let mut bindings = Vec::new();
    let mut bind = |action: BindableAction, binding: &dyn Fn(&str) -> KeyBinding| {
        // An action the user unbound has no keys.
        if let Some(keys) = shelldeck_ui::shortcut_reference::keys(action) {
            bindings.push(binding(&keys));
        }
    };
    bind(B::QuickConnect, &|k| {
        KeyBinding::new(k, OpenQuickConnect, None)
    });
    bind(B::AiAssistant, &|k| {
        KeyBinding::new(k, OpenAiAssistant, None)
    });
    bind(B::NewTerminal, &|k| KeyBinding::new(k, NewTerminal, None));
    bind(B::ToggleSidebar, &|k| {
        KeyBinding::new(k, ToggleSidebar, None)
    });
    // Remains available after the menu row hides.
    bind(B::ToggleMenuBar, &|k| {
        KeyBinding::new(k, ToggleMenuBar, None)
    });
    bind(B::Settings, &|k| KeyBinding::new(k, OpenSettings, None));
    bind(B::NextTab, &|k| KeyBinding::new(k, NextTab, None));
    bind(B::PrevTab, &|k| KeyBinding::new(k, PrevTab, None));
    bind(B::CloseTab, &|k| KeyBinding::new(k, CloseTab, None));
    bind(B::ClearTerminal, &|k| {
        KeyBinding::new(k, ClearTerminal, None)
    });
    // Intercepted before the terminal.
    bind(B::Search, &|k| KeyBinding::new(k, ToggleSearch, None));
    bind(B::ZoomIn, &|k| KeyBinding::new(k, ZoomIn, None));
    bind(B::ZoomOut, &|k| KeyBinding::new(k, ZoomOut, None));
    bind(B::ZoomReset, &|k| KeyBinding::new(k, ZoomReset, None));
    bind(B::CommandPalette, &|k| {
        KeyBinding::new(k, ToggleCommandPalette, None)
    });
    // Full app, Agents, Terminal, Editor.
    bind(B::NextView, &|k| {
        KeyBinding::new(k, CyclePresentation, None)
    });
    bind(B::FileEditor, &|k| {
        KeyBinding::new(k, OpenFileEditorView, None)
    });
    bind(B::ToggleSplitFocus, &|k| {
        KeyBinding::new(k, ToggleSplitFocus, None)
    });
    bind(B::Quit, &|k| KeyBinding::new(k, Quit, None));
    bind(B::SplitHorizontal, &|k| {
        KeyBinding::new(k, SplitHorizontal, None)
    });
    bind(B::SplitVertical, &|k| {
        KeyBinding::new(k, SplitVertical, None)
    });
    bind(B::MaximizePane, &|k| {
        KeyBinding::new(k, TogglePaneZoom, None)
    });
    bind(B::Copy, &|k| KeyBinding::new(k, CopySelection, None));
    bind(B::Paste, &|k| KeyBinding::new(k, PasteClipboard, None));

    cx.bind_keys(bindings);
}
