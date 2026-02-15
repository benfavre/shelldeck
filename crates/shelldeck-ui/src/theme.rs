use gpui::*;
use std::sync::atomic::{AtomicBool, Ordering};

static DARK_MODE: AtomicBool = AtomicBool::new(true);

/// ShellDeck brand colors — responds to dark/light mode.
pub struct ShellDeckColors;

impl ShellDeckColors {
    pub fn set_dark_mode(dark: bool) {
        DARK_MODE.store(dark, Ordering::Relaxed);
    }

    pub fn is_dark() -> bool {
        DARK_MODE.load(Ordering::Relaxed)
    }

    /// Primary brand blue
    pub fn primary() -> Hsla {
        if Self::is_dark() {
            hsla(0.63, 0.85, 0.55, 1.0)
        } else {
            hsla(0.63, 0.80, 0.42, 1.0)
        }
    }

    /// Primary hover
    pub fn primary_hover() -> Hsla {
        if Self::is_dark() {
            hsla(0.63, 0.85, 0.50, 1.0)
        } else {
            hsla(0.63, 0.80, 0.38, 1.0)
        }
    }

    /// Success green
    pub fn success() -> Hsla {
        if Self::is_dark() {
            hsla(0.40, 0.70, 0.45, 1.0)
        } else {
            hsla(0.40, 0.65, 0.38, 1.0)
        }
    }

    /// Warning amber
    pub fn warning() -> Hsla {
        if Self::is_dark() {
            hsla(0.10, 0.85, 0.55, 1.0)
        } else {
            hsla(0.10, 0.80, 0.45, 1.0)
        }
    }

    /// Error red
    pub fn error() -> Hsla {
        if Self::is_dark() {
            hsla(0.0, 0.75, 0.50, 1.0)
        } else {
            hsla(0.0, 0.70, 0.45, 1.0)
        }
    }

    /// Background primary
    pub fn bg_primary() -> Hsla {
        if Self::is_dark() {
            hsla(0.0, 0.0, 0.10, 1.0)
        } else {
            hsla(0.0, 0.0, 0.96, 1.0)
        }
    }

    /// Sidebar background
    pub fn bg_sidebar() -> Hsla {
        if Self::is_dark() {
            hsla(0.0, 0.0, 0.08, 1.0)
        } else {
            hsla(0.0, 0.0, 0.98, 1.0)
        }
    }

    /// Surface background (cards, panels)
    pub fn bg_surface() -> Hsla {
        if Self::is_dark() {
            hsla(0.0, 0.0, 0.13, 1.0)
        } else {
            hsla(0.0, 0.0, 0.93, 1.0)
        }
    }

    /// Border color
    pub fn border() -> Hsla {
        if Self::is_dark() {
            hsla(0.0, 0.0, 0.20, 1.0)
        } else {
            hsla(0.0, 0.0, 0.82, 1.0)
        }
    }

    /// Text primary
    pub fn text_primary() -> Hsla {
        if Self::is_dark() {
            hsla(0.0, 0.0, 0.90, 1.0)
        } else {
            hsla(0.0, 0.0, 0.12, 1.0)
        }
    }

    /// Text secondary/muted
    pub fn text_muted() -> Hsla {
        if Self::is_dark() {
            hsla(0.0, 0.0, 0.55, 1.0)
        } else {
            hsla(0.0, 0.0, 0.45, 1.0)
        }
    }

    /// Terminal background
    pub fn terminal_bg() -> Hsla {
        if Self::is_dark() {
            hsla(0.0, 0.0, 0.07, 1.0)
        } else {
            hsla(0.0, 0.0, 0.98, 1.0)
        }
    }

    /// Hover background for interactive items
    pub fn hover_bg() -> Hsla {
        if Self::is_dark() {
            hsla(0.0, 0.0, 1.0, 0.06)
        } else {
            hsla(0.0, 0.0, 0.0, 0.06)
        }
    }

    /// Selected/active item background
    pub fn selected_bg() -> Hsla {
        if Self::is_dark() {
            hsla(0.0, 0.0, 0.18, 1.0)
        } else {
            hsla(0.0, 0.0, 0.88, 1.0)
        }
    }

    /// Small badge/chip background
    pub fn badge_bg() -> Hsla {
        if Self::is_dark() {
            hsla(0.0, 0.0, 0.20, 1.0)
        } else {
            hsla(0.0, 0.0, 0.88, 1.0)
        }
    }

    /// Toggle switch off-state background
    pub fn toggle_off_bg() -> Hsla {
        if Self::is_dark() {
            hsla(0.0, 0.0, 0.25, 1.0)
        } else {
            hsla(0.0, 0.0, 0.78, 1.0)
        }
    }

    /// Toggle switch off-state knob color
    pub fn toggle_off_knob() -> Hsla {
        if Self::is_dark() {
            hsla(0.0, 0.0, 0.50, 1.0)
        } else {
            hsla(0.0, 0.0, 0.96, 1.0)
        }
    }

    /// Overlay backdrop color (semi-transparent)
    pub fn backdrop() -> Hsla {
        hsla(0.0, 0.0, 0.0, 0.45)
    }

    // ---- Syntax highlighting colors ----

    /// Keywords (if, then, for, while, etc.) — purple, bold
    pub fn syntax_keyword() -> Hsla {
        if Self::is_dark() {
            hsla(0.76, 0.70, 0.68, 1.0)
        } else {
            hsla(0.76, 0.65, 0.42, 1.0)
        }
    }

    /// Builtins (echo, cd, grep, etc.) — cyan
    pub fn syntax_builtin() -> Hsla {
        if Self::is_dark() {
            hsla(0.50, 0.65, 0.65, 1.0)
        } else {
            hsla(0.50, 0.60, 0.38, 1.0)
        }
    }

    /// Comments — gray, italic
    pub fn syntax_comment() -> Hsla {
        if Self::is_dark() {
            hsla(0.0, 0.0, 0.45, 1.0)
        } else {
            hsla(0.0, 0.0, 0.55, 1.0)
        }
    }

    /// Strings — green
    pub fn syntax_string() -> Hsla {
        if Self::is_dark() {
            hsla(0.35, 0.60, 0.60, 1.0)
        } else {
            hsla(0.35, 0.55, 0.35, 1.0)
        }
    }

    /// Variables ($VAR, ${VAR}) — orange
    pub fn syntax_variable() -> Hsla {
        if Self::is_dark() {
            hsla(0.08, 0.75, 0.65, 1.0)
        } else {
            hsla(0.08, 0.70, 0.45, 1.0)
        }
    }

    /// Operators (|, &&, >, etc.) — light gray
    pub fn syntax_operator() -> Hsla {
        if Self::is_dark() {
            hsla(0.0, 0.0, 0.70, 1.0)
        } else {
            hsla(0.0, 0.0, 0.35, 1.0)
        }
    }

    /// Numbers — teal
    pub fn syntax_number() -> Hsla {
        if Self::is_dark() {
            hsla(0.47, 0.55, 0.60, 1.0)
        } else {
            hsla(0.47, 0.50, 0.38, 1.0)
        }
    }

    /// Command substitution ($(...), `...`) — yellow-orange
    pub fn syntax_command_sub() -> Hsla {
        if Self::is_dark() {
            hsla(0.12, 0.70, 0.65, 1.0)
        } else {
            hsla(0.12, 0.65, 0.42, 1.0)
        }
    }

    /// Template variable `{{name}}` — distinct warm orange
    pub fn syntax_template_var() -> Hsla {
        if Self::is_dark() {
            hsla(0.08, 0.85, 0.65, 1.0) // warm orange
        } else {
            hsla(0.08, 0.80, 0.45, 1.0)
        }
    }

    /// Line number foreground
    pub fn line_number_fg() -> Hsla {
        if Self::is_dark() {
            hsla(0.0, 0.0, 0.38, 1.0)
        } else {
            hsla(0.0, 0.0, 0.60, 1.0)
        }
    }

    /// Line number gutter background
    pub fn line_number_bg() -> Hsla {
        if Self::is_dark() {
            hsla(0.0, 0.0, 0.09, 1.0)
        } else {
            hsla(0.0, 0.0, 0.94, 1.0)
        }
    }

    /// Active cursor line background
    pub fn cursor_line_bg() -> Hsla {
        if Self::is_dark() {
            hsla(0.0, 0.0, 1.0, 0.04)
        } else {
            hsla(0.0, 0.0, 0.0, 0.04)
        }
    }

    /// Subtle hint background (status bar pill, etc.)
    pub fn hint_bg() -> Hsla {
        if Self::is_dark() {
            hsla(0.0, 0.0, 0.15, 0.5)
        } else {
            hsla(0.0, 0.0, 0.0, 0.06)
        }
    }

    /// Connected status
    pub fn status_connected() -> Hsla {
        Self::success()
    }

    /// Disconnected status
    pub fn status_disconnected() -> Hsla {
        Self::text_muted()
    }

    /// Error status
    pub fn status_error() -> Hsla {
        Self::error()
    }
}
