use serde::{Deserialize, Serialize};

/// Terminal color theme with all color slots.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TerminalTheme {
    pub name: String,
    /// ANSI colors 0-15 as hex strings (e.g., "#1e1e1e")
    pub ansi_colors: [String; 16],
    /// Default foreground
    pub foreground: String,
    /// Default background
    pub background: String,
    /// Cursor color
    pub cursor: String,
    /// Selection background
    pub selection: String,
    /// Search match highlight
    pub search_match: String,
    /// Current search match
    pub search_current: String,
}

impl Default for TerminalTheme {
    fn default() -> Self {
        Self::dark()
    }
}

impl TerminalTheme {
    /// Dark theme (default)
    pub fn dark() -> Self {
        Self {
            name: "Dark".to_string(),
            ansi_colors: [
                "#000000".to_string(), // 0: Black
                "#cd0000".to_string(), // 1: Red
                "#00cd00".to_string(), // 2: Green
                "#cdcd00".to_string(), // 3: Yellow
                "#0000ee".to_string(), // 4: Blue
                "#cd00cd".to_string(), // 5: Magenta
                "#00cdcd".to_string(), // 6: Cyan
                "#e5e5e5".to_string(), // 7: White
                "#7f7f7f".to_string(), // 8: Bright Black
                "#ff0000".to_string(), // 9: Bright Red
                "#00ff00".to_string(), // 10: Bright Green
                "#ffff00".to_string(), // 11: Bright Yellow
                "#5c5cff".to_string(), // 12: Bright Blue
                "#ff00ff".to_string(), // 13: Bright Magenta
                "#00ffff".to_string(), // 14: Bright Cyan
                "#ffffff".to_string(), // 15: Bright White
            ],
            foreground: "#cccccc".to_string(),
            background: "#1e1e1e".to_string(),
            cursor: "#e5e5e5".to_string(),
            selection: "#264f78".to_string(),
            search_match: "#515151".to_string(),
            search_current: "#e2a54a".to_string(),
        }
    }

    /// Light theme
    pub fn light() -> Self {
        Self {
            name: "Light".to_string(),
            ansi_colors: [
                "#000000".to_string(),
                "#c91b00".to_string(),
                "#00c200".to_string(),
                "#c7c400".to_string(),
                "#0225c7".to_string(),
                "#ca30c7".to_string(),
                "#00c5c7".to_string(),
                "#c7c7c7".to_string(),
                "#686868".to_string(),
                "#ff6e67".to_string(),
                "#5ffa68".to_string(),
                "#fffc67".to_string(),
                "#6871ff".to_string(),
                "#ff77ff".to_string(),
                "#60fdff".to_string(),
                "#ffffff".to_string(),
            ],
            foreground: "#2e2e2e".to_string(),
            background: "#f5f5f5".to_string(),
            cursor: "#2e2e2e".to_string(),
            selection: "#b4d5fe".to_string(),
            search_match: "#d3d3d3".to_string(),
            search_current: "#e2a54a".to_string(),
        }
    }

    /// Pastel Dark theme
    pub fn pastel_dark() -> Self {
        Self {
            name: "Pastel Dark".to_string(),
            ansi_colors: [
                "#2d2d2d".to_string(),
                "#f2777a".to_string(),
                "#99cc99".to_string(),
                "#ffcc66".to_string(),
                "#6699cc".to_string(),
                "#cc99cc".to_string(),
                "#66cccc".to_string(),
                "#d3d0c8".to_string(),
                "#747369".to_string(),
                "#f2777a".to_string(),
                "#99cc99".to_string(),
                "#ffcc66".to_string(),
                "#6699cc".to_string(),
                "#cc99cc".to_string(),
                "#66cccc".to_string(),
                "#f2f0ec".to_string(),
            ],
            foreground: "#d3d0c8".to_string(),
            background: "#2d2d2d".to_string(),
            cursor: "#d3d0c8".to_string(),
            selection: "#515151".to_string(),
            search_match: "#515151".to_string(),
            search_current: "#ffcc66".to_string(),
        }
    }

    /// High Contrast theme
    pub fn high_contrast() -> Self {
        Self {
            name: "High Contrast".to_string(),
            ansi_colors: [
                "#000000".to_string(),
                "#ff0000".to_string(),
                "#00ff00".to_string(),
                "#ffff00".to_string(),
                "#0066ff".to_string(),
                "#ff00ff".to_string(),
                "#00ffff".to_string(),
                "#ffffff".to_string(),
                "#808080".to_string(),
                "#ff0000".to_string(),
                "#00ff00".to_string(),
                "#ffff00".to_string(),
                "#0066ff".to_string(),
                "#ff00ff".to_string(),
                "#00ffff".to_string(),
                "#ffffff".to_string(),
            ],
            foreground: "#ffffff".to_string(),
            background: "#000000".to_string(),
            cursor: "#ffffff".to_string(),
            selection: "#0044aa".to_string(),
            search_match: "#444444".to_string(),
            search_current: "#ff8800".to_string(),
        }
    }

    /// All built-in themes
    pub fn builtins() -> Vec<Self> {
        vec![
            Self::dark(),
            Self::light(),
            Self::pastel_dark(),
            Self::high_contrast(),
        ]
    }
}
