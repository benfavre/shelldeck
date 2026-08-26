use gpui::*;
use shelldeck_ui::shortcut_reference::{
    CLEAR_TERMINAL_BINDING, CLOSE_TAB_BINDING, COMMAND_PALETTE_BINDING, MACOS_COPY_BINDING,
    MACOS_PASTE_BINDING, MACOS_SPLIT_BINDING, NEW_TERMINAL_BINDING, NEXT_TAB_BINDING,
    OTHER_COPY_BINDING, OTHER_PASTE_BINDING, OTHER_SPLIT_BINDING, QUIT_BINDING, SEARCH_BINDING,
    SETTINGS_BINDING, TOGGLE_SIDEBAR_BINDING, ZOOM_IN_BINDING, ZOOM_OUT_BINDING,
    ZOOM_RESET_BINDING,
};

// Re-export workspace actions for keybinding registration
pub use shelldeck_ui::workspace::{
    CloseTab, CloudSyncNow, ConnectBextCloud, NewRequest, NewTerminal, NextTab, OpenAiAssistant,
    OpenBextCloud, OpenFileEditorView, OpenFleet, OpenMoniqueConsole, OpenQuickConnect,
    OpenSettings, OpenSupportRequests, PrevTab, Quit, SwitchSite, ToggleMenuBar, ToggleSidebar,
};

// Re-export terminal view actions
pub use shelldeck_ui::command_palette::{
    ApplyTerminalTheme, OpenManageArea, SetAppMode, ToggleCommandPalette,
};
pub use shelldeck_ui::terminal_view::{
    ClearTerminal, CopySelection, PasteClipboard, SplitHorizontal, SplitVertical, ToggleSearch,
    ToggleSplitFocus, ZoomIn, ZoomOut, ZoomReset,
};

/// Register all keyboard shortcuts.
///
/// Uses the `secondary` modifier for cross-platform correctness:
///   - macOS: `secondary` = Cmd
///   - Linux/Windows: `secondary` = Ctrl
///
/// Clipboard and split shortcuts need platform-specific bindings because
/// Ctrl+C (SIGINT), Ctrl+V (literal-next), and Ctrl+D (EOF) conflict
/// with terminal control characters on Linux/Windows.
pub fn register_keybindings(cx: &mut App) {
    let mut bindings = vec![
        // Quick connect: Cmd+K (macOS) / Ctrl+K (Linux/Win)
        KeyBinding::new("secondary-k", OpenQuickConnect, None),
        // Contextual AI assistant: keep Quick Connect's established Cmd/Ctrl+K.
        KeyBinding::new("secondary-shift-k", OpenAiAssistant, None),
        // New terminal: Cmd+T (macOS) / Ctrl+T (Linux/Win)
        KeyBinding::new(NEW_TERMINAL_BINDING, NewTerminal, None),
        // Toggle sidebar: Cmd+B (macOS) / Ctrl+B (Linux/Win)
        KeyBinding::new(TOGGLE_SIDEBAR_BINDING, ToggleSidebar, None),
        // Toggle application menu bar; remains available after the row hides.
        KeyBinding::new("secondary-shift-m", ToggleMenuBar, None),
        // Settings: Cmd+, (macOS) / Ctrl+, (Linux/Win)
        KeyBinding::new(SETTINGS_BINDING, OpenSettings, None),
        // Tab navigation (Ctrl+Tab on all platforms)
        KeyBinding::new(NEXT_TAB_BINDING, NextTab, None),
        KeyBinding::new("ctrl-shift-tab", PrevTab, None),
        // Close tab: Cmd+W (macOS) / Ctrl+W (Linux/Win)
        KeyBinding::new(CLOSE_TAB_BINDING, CloseTab, None),
        // Clear terminal: Cmd+L (macOS) / Ctrl+L (Linux/Win)
        KeyBinding::new(CLEAR_TERMINAL_BINDING, ClearTerminal, None),
        // Search: Cmd+F (macOS) / Ctrl+F (Linux/Win) — intercepted before terminal
        KeyBinding::new(SEARCH_BINDING, ToggleSearch, None),
        // Zoom: Cmd+=/- (macOS) / Ctrl+=/- (Linux/Win)
        KeyBinding::new(ZOOM_IN_BINDING, ZoomIn, None),
        KeyBinding::new(ZOOM_OUT_BINDING, ZoomOut, None),
        KeyBinding::new(ZOOM_RESET_BINDING, ZoomReset, None),
        // Command palette: Cmd+Shift+P / Ctrl+Shift+P
        KeyBinding::new(COMMAND_PALETTE_BINDING, ToggleCommandPalette, None),
        // File editor: Cmd+E (macOS) / Ctrl+E (Linux/Win)
        KeyBinding::new("secondary-e", OpenFileEditorView, None),
        // Toggle split focus: Alt+[ (all platforms)
        KeyBinding::new("alt-[", ToggleSplitFocus, None),
        // Quit: Cmd+Q (macOS) / Ctrl+Q (Linux/Win)
        KeyBinding::new(QUIT_BINDING, Quit, None),
    ];

    // Platform-specific bindings for actions that conflict with terminal
    // control characters when using Ctrl on Linux/Windows.
    if cfg!(target_os = "macos") {
        bindings.extend([
            // Cmd+D / Cmd+Shift+D — no terminal conflict on macOS
            KeyBinding::new(MACOS_SPLIT_BINDING, SplitHorizontal, None),
            KeyBinding::new("cmd-shift-d", SplitVertical, None),
            // Cmd+C / Cmd+V — no terminal conflict on macOS
            KeyBinding::new(MACOS_COPY_BINDING, CopySelection, None),
            KeyBinding::new(MACOS_PASTE_BINDING, PasteClipboard, None),
        ]);
    } else {
        bindings.extend([
            // Ctrl+Shift+D — avoids Ctrl+D (EOF) conflict
            KeyBinding::new(OTHER_SPLIT_BINDING, SplitHorizontal, None),
            KeyBinding::new("ctrl-shift-alt-d", SplitVertical, None),
            // Ctrl+Shift+C/V — standard terminal emulator copy/paste
            KeyBinding::new(OTHER_COPY_BINDING, CopySelection, None),
            KeyBinding::new(OTHER_PASTE_BINDING, PasteClipboard, None),
        ]);
    }

    cx.bind_keys(bindings);
}
