#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TermColor {
    #[default]
    Default,
    Named(NamedColor),
    Indexed(u8),
    Rgb(u8, u8, u8),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NamedColor {
    Black,
    Red,
    Green,
    Yellow,
    Blue,
    Magenta,
    Cyan,
    White,
    BrightBlack,
    BrightRed,
    BrightGreen,
    BrightYellow,
    BrightBlue,
    BrightMagenta,
    BrightCyan,
    BrightWhite,
}


impl NamedColor {
    /// Convert named color to (r, g, b, a) using standard xterm colors.
    pub fn to_rgb(self) -> (u8, u8, u8, u8) {
        match self {
            Self::Black => (0, 0, 0, 255),
            Self::Red => (205, 0, 0, 255),
            Self::Green => (0, 205, 0, 255),
            Self::Yellow => (205, 205, 0, 255),
            Self::Blue => (0, 0, 238, 255),
            Self::Magenta => (205, 0, 205, 255),
            Self::Cyan => (0, 205, 205, 255),
            Self::White => (229, 229, 229, 255),
            Self::BrightBlack => (127, 127, 127, 255),
            Self::BrightRed => (255, 0, 0, 255),
            Self::BrightGreen => (0, 255, 0, 255),
            Self::BrightYellow => (255, 255, 0, 255),
            Self::BrightBlue => (92, 92, 255, 255),
            Self::BrightMagenta => (255, 0, 255, 255),
            Self::BrightCyan => (0, 255, 255, 255),
            Self::BrightWhite => (255, 255, 255, 255),
        }
    }

    /// Convert a named color to its xterm 256-color index (0-15).
    pub fn to_index(self) -> u8 {
        match self {
            Self::Black => 0,
            Self::Red => 1,
            Self::Green => 2,
            Self::Yellow => 3,
            Self::Blue => 4,
            Self::Magenta => 5,
            Self::Cyan => 6,
            Self::White => 7,
            Self::BrightBlack => 8,
            Self::BrightRed => 9,
            Self::BrightGreen => 10,
            Self::BrightYellow => 11,
            Self::BrightBlue => 12,
            Self::BrightMagenta => 13,
            Self::BrightCyan => 14,
            Self::BrightWhite => 15,
        }
    }
}

/// Convert a 256-color xterm index to (r, g, b, a).
///
/// Indices 0-15: standard/bright named colors
/// Indices 16-231: 6x6x6 color cube
/// Indices 232-255: 24-step grayscale ramp
pub fn index_to_rgb(index: u8) -> (u8, u8, u8, u8) {
    match index {
        // Standard colors (0-7)
        0 => (0, 0, 0, 255),
        1 => (205, 0, 0, 255),
        2 => (0, 205, 0, 255),
        3 => (205, 205, 0, 255),
        4 => (0, 0, 238, 255),
        5 => (205, 0, 205, 255),
        6 => (0, 205, 205, 255),
        7 => (229, 229, 229, 255),
        // Bright colors (8-15)
        8 => (127, 127, 127, 255),
        9 => (255, 0, 0, 255),
        10 => (0, 255, 0, 255),
        11 => (255, 255, 0, 255),
        12 => (92, 92, 255, 255),
        13 => (255, 0, 255, 255),
        14 => (0, 255, 255, 255),
        15 => (255, 255, 255, 255),
        // 6x6x6 color cube (16-231)
        16..=231 => {
            let idx = index - 16;
            let r_idx = idx / 36;
            let g_idx = (idx % 36) / 6;
            let b_idx = idx % 6;
            let r = if r_idx == 0 { 0 } else { 55 + 40 * r_idx };
            let g = if g_idx == 0 { 0 } else { 55 + 40 * g_idx };
            let b = if b_idx == 0 { 0 } else { 55 + 40 * b_idx };
            (r, g, b, 255)
        }
        // Grayscale ramp (232-255)
        232..=255 => {
            let level = 8 + 10 * (index - 232);
            (level, level, level, 255)
        }
    }
}

impl TermColor {
    /// Convert to RGBA tuple (r, g, b, a) for rendering.
    /// `is_foreground` determines the default color when `Self::Default`.
    pub fn to_rgba(&self, is_foreground: bool) -> (u8, u8, u8, u8) {
        match self {
            Self::Default => {
                if is_foreground {
                    (204, 204, 204, 255)
                } else {
                    (30, 30, 30, 255)
                }
            }
            Self::Named(c) => c.to_rgb(),
            Self::Indexed(i) => index_to_rgb(*i),
            Self::Rgb(r, g, b) => (*r, *g, *b, 255),
        }
    }
}
