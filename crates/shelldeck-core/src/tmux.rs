//! Typed controls for a tmux client. No terminal keystrokes or user commands.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TmuxAction {
    SplitHorizontal,
    SplitVertical,
    NextPane,
    NextLayout,
    ToggleZoom,
}
