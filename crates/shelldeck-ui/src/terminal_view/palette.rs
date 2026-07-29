use super::*;

// ---------------------------------------------------------------------------
// Colour conversion helper
// ---------------------------------------------------------------------------

/// Runtime color palette for the terminal renderer. Overrides the default
/// xterm colors with theme-specific values.
#[derive(Clone)]
pub(super) struct TerminalPalette {
    /// ANSI colors 0-15 as (r, g, b).
    pub(super) ansi: [[u8; 3]; 16],
    /// Default foreground (r, g, b).
    pub(super) foreground: [u8; 3],
    /// Default background (r, g, b).
    pub(super) background: [u8; 3],
    /// Cursor color.
    pub(super) cursor: Hsla,
    /// Selection background.
    pub(super) selection: Hsla,
    /// Search match highlight background.
    pub(super) search_match: Hsla,
    /// Current (focused) search match background.
    pub(super) search_current: Hsla,
}

impl Default for TerminalPalette {
    fn default() -> Self {
        Self::from_theme(&TerminalTheme::dark())
    }
}

/// Parse a `#rrggbb` hex string to RGB bytes, tolerating a missing `#` and
/// malformed input (which falls back to black).
fn parse_hex_rgb(hex: &str) -> [u8; 3] {
    let hex = hex.trim_start_matches('#');
    if hex.len() < 6 {
        return [0, 0, 0];
    }
    let r = u8::from_str_radix(&hex[0..2], 16).unwrap_or(0);
    let g = u8::from_str_radix(&hex[2..4], 16).unwrap_or(0);
    let b = u8::from_str_radix(&hex[4..6], 16).unwrap_or(0);
    [r, g, b]
}

/// Convert RGB bytes to an opaque `Hsla`.
#[inline]
fn rgb_to_hsla(rgb: [u8; 3]) -> Hsla {
    Hsla::from(rgba(
        (rgb[0] as u32) << 24 | (rgb[1] as u32) << 16 | (rgb[2] as u32) << 8 | 0xFF,
    ))
}

impl TerminalPalette {
    pub(super) fn from_theme(theme: &TerminalTheme) -> Self {
        let mut ansi = [[0u8; 3]; 16];
        for (i, hex) in theme.ansi_colors.iter().enumerate() {
            ansi[i] = parse_hex_rgb(hex);
        }

        Self {
            ansi,
            foreground: parse_hex_rgb(&theme.foreground),
            background: parse_hex_rgb(&theme.background),
            cursor: rgb_to_hsla(parse_hex_rgb(&theme.cursor)),
            // Selection / search highlights are translucent so the glyphs
            // underneath stay legible.
            selection: rgb_to_hsla(parse_hex_rgb(&theme.selection)).opacity(0.45),
            search_match: rgb_to_hsla(parse_hex_rgb(&theme.search_match)).opacity(0.55),
            search_current: rgb_to_hsla(parse_hex_rgb(&theme.search_current)).opacity(0.75),
        }
    }

    /// The theme's default background as an opaque `Hsla`.
    #[inline]
    pub(super) fn background_color(&self) -> Hsla {
        rgb_to_hsla(self.background)
    }

    /// Resolve a `TermColor` to an HSLA value using this palette.
    #[inline]
    pub(super) fn resolve(&self, color: &TermColor, is_foreground: bool) -> Hsla {
        let (r, g, b) = match color {
            TermColor::Default => {
                if is_foreground {
                    (self.foreground[0], self.foreground[1], self.foreground[2])
                } else {
                    (self.background[0], self.background[1], self.background[2])
                }
            }
            TermColor::Named(c) => {
                let idx = c.to_index() as usize;
                (self.ansi[idx][0], self.ansi[idx][1], self.ansi[idx][2])
            }
            TermColor::Indexed(i) if (*i as usize) < 16 => {
                let idx = *i as usize;
                (self.ansi[idx][0], self.ansi[idx][1], self.ansi[idx][2])
            }
            TermColor::Indexed(i) => {
                let (r, g, b, _) = shelldeck_terminal::colors::index_to_rgb(*i);
                (r, g, b)
            }
            TermColor::Rgb(r, g, b) => (*r, *g, *b),
        };
        Hsla::from(rgba(
            (r as u32) << 24 | (g as u32) << 16 | (b as u32) << 8 | 0xFF,
        ))
    }
}

/// When bold is set and the foreground is a standard named color (0-7),
/// brighten it to the bright variant (8-15).  This matches the traditional
/// terminal convention that htop and many other TUI programs rely on.
#[inline]
pub(super) fn brighten_for_bold(color: TermColor) -> TermColor {
    match color {
        TermColor::Named(c) => TermColor::Named(match c {
            NamedColor::Black => NamedColor::BrightBlack,
            NamedColor::Red => NamedColor::BrightRed,
            NamedColor::Green => NamedColor::BrightGreen,
            NamedColor::Yellow => NamedColor::BrightYellow,
            NamedColor::Blue => NamedColor::BrightBlue,
            NamedColor::Magenta => NamedColor::BrightMagenta,
            NamedColor::Cyan => NamedColor::BrightCyan,
            NamedColor::White => NamedColor::BrightWhite,
            other => other, // already bright
        }),
        TermColor::Indexed(i) if i < 8 => TermColor::Indexed(i + 8),
        other => other,
    }
}

/// Dim/faint a foreground color by halving the RGB component values.
/// For named and indexed colors, convert to RGB first, then dim.
/// For the default foreground, produce a mid-gray.
pub(super) fn dim_color(color: TermColor) -> TermColor {
    match color {
        TermColor::Rgb(r, g, b) => TermColor::Rgb(r / 2, g / 2, b / 2),
        TermColor::Default => {
            // Default foreground is typically ~(204, 204, 204); dim to half.
            TermColor::Rgb(102, 102, 102)
        }
        other => {
            let (r, g, b, _) = other.to_rgba(true);
            TermColor::Rgb(r / 2, g / 2, b / 2)
        }
    }
}
