use gpui::*;

// Re-export workspace actions for keybinding registration
pub use shelldeck_ui::workspace::{
    CloseTab, NewTerminal, NextTab, OpenQuickConnect, OpenSettings, PrevTab, Quit, ToggleSidebar,
};

// Re-export terminal view actions
pub use shelldeck_ui::terminal_view::{
    CopySelection, PasteClipboard, ClearTerminal, ToggleSearch,
    SplitHorizontal, SplitVertical, ZoomIn, ZoomOut, ZoomReset,
    ToggleSplitFocus,
};
pub use shelldeck_ui::command_palette::ToggleCommandPalette;

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
        // New terminal: Cmd+T (macOS) / Ctrl+T (Linux/Win)
        KeyBinding::new("secondary-t", NewTerminal, None),
        // Toggle sidebar: Cmd+B (macOS) / Ctrl+B (Linux/Win)
        KeyBinding::new("secondary-b", ToggleSidebar, None),
        // Settings: Cmd+, (macOS) / Ctrl+, (Linux/Win)
        KeyBinding::new("secondary-,", OpenSettings, None),
        // Tab navigation (Ctrl+Tab on all platforms)
        KeyBinding::new("ctrl-tab", NextTab, None),
        KeyBinding::new("ctrl-shift-tab", PrevTab, None),
        // Close tab: Cmd+W (macOS) / Ctrl+W (Linux/Win)
        KeyBinding::new("secondary-w", CloseTab, None),
        // Clear terminal: Cmd+L (macOS) / Ctrl+L (Linux/Win)
        KeyBinding::new("secondary-l", ClearTerminal, None),
        // Search: Cmd+F (macOS) / Ctrl+F (Linux/Win) — intercepted before terminal
        KeyBinding::new("secondary-f", ToggleSearch, None),
        // Zoom: Cmd+=/- (macOS) / Ctrl+=/- (Linux/Win)
        KeyBinding::new("secondary-=", ZoomIn, None),
        KeyBinding::new("secondary--", ZoomOut, None),
        KeyBinding::new("secondary-0", ZoomReset, None),
        // Command palette: Cmd+Shift+P / Ctrl+Shift+P
        KeyBinding::new("secondary-shift-p", ToggleCommandPalette, None),
        // Toggle split focus: Alt+[ (all platforms)
        KeyBinding::new("alt-[", ToggleSplitFocus, None),
        // Quit: Cmd+Q (macOS) / Ctrl+Q (Linux/Win)
        KeyBinding::new("secondary-q", Quit, None),
    ];

    // Platform-specific bindings for actions that conflict with terminal
    // control characters when using Ctrl on Linux/Windows.
    if cfg!(target_os = "macos") {
        bindings.extend([
            // Cmd+D / Cmd+Shift+D — no terminal conflict on macOS
            KeyBinding::new("cmd-d", SplitHorizontal, None),
            KeyBinding::new("cmd-shift-d", SplitVertical, None),
            // Cmd+C / Cmd+V — no terminal conflict on macOS
            KeyBinding::new("cmd-c", CopySelection, None),
            KeyBinding::new("cmd-v", PasteClipboard, None),
        ]);
    } else {
        bindings.extend([
            // Ctrl+Shift+D — avoids Ctrl+D (EOF) conflict
            KeyBinding::new("ctrl-shift-d", SplitHorizontal, None),
            KeyBinding::new("ctrl-shift-alt-d", SplitVertical, None),
            // Ctrl+Shift+C/V — standard terminal emulator copy/paste
            KeyBinding::new("ctrl-shift-c", CopySelection, None),
            KeyBinding::new("ctrl-shift-v", PasteClipboard, None),
        ]);
    }

    cx.bind_keys(bindings);
}
