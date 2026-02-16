use std::collections::HashMap;
use std::sync::Arc;

use gpui::prelude::*;
use gpui::*;
use parking_lot::Mutex;
use shelldeck_terminal::colors::{NamedColor, TermColor};
use shelldeck_terminal::grid::{
    CellWidth, CursorShape, CursorState, MouseEncoding, MouseMode, SearchMatch, TerminalGrid,
    UnderlineStyle,
};
use shelldeck_terminal::session::{SessionState, TerminalSession};
use shelldeck_terminal::url::{detect_urls, UrlMatch};
use uuid::Uuid;

use shelldeck_core::config::themes::TerminalTheme;

use crate::theme::ShellDeckColors;

// ---------------------------------------------------------------------------
// Glyph cache – resolved once per font/size, reused every frame
// ---------------------------------------------------------------------------

/// Pre-resolved glyph data for ASCII characters (codepoints 0..128).
struct FontGlyphs {
    /// `ascii[ch as usize]` → `Some((font_id, glyph_id))` for printable ASCII.
    /// Each character stores its own font_id to handle font fallback correctly.
    ascii: [Option<(FontId, GlyphId)>; 128],
}

/// Cached glyph lookup table for the four font variants (regular, bold,
/// italic, bold-italic).  Built once when the font family or size changes
/// and shared across frames via `Arc`.
struct GlyphCache {
    regular: FontGlyphs,
    bold: FontGlyphs,
    italic: FontGlyphs,
    bold_italic: FontGlyphs,
    cell_width: Pixels,
    cell_height: Pixels,
    /// Y-offset from the top of a cell to the glyph baseline.
    baseline_y: Pixels,
    font_size: Pixels,
    /// Base font used for non-ASCII fallback shaping.
    base_font: Font,
}

impl GlyphCache {
    fn build(text_sys: &WindowTextSystem, font_family: &str, font_size: f32) -> Self {
        let fs = px(font_size);
        let family: SharedString = font_family.to_string().into();
        let base = font(family);

        let regular = Self::resolve_variant(text_sys, &base, fs);
        let bold = Self::resolve_variant(text_sys, &base.clone().bold(), fs);
        let italic = Self::resolve_variant(text_sys, &base.clone().italic(), fs);
        let bold_italic = Self::resolve_variant(text_sys, &base.clone().bold().italic(), fs);

        // Measure cell dimensions from a reference character.
        let ref_line = text_sys.shape_line(
            "W".into(),
            fs,
            &[TextRun {
                len: 1,
                font: base.clone(),
                color: hsla(1., 1., 1., 1.),
                background_color: None,
                underline: None,
                strikethrough: None,
            }],
            None,
        );
        let cell_width = ref_line.width;
        let cell_height = px(font_size * 1.4);
        let ascent = ref_line.ascent;
        let descent = ref_line.descent;
        let baseline_y = (cell_height - ascent - descent) / 2.0 + ascent;

        GlyphCache {
            regular,
            bold,
            italic,
            bold_italic,
            cell_width,
            cell_height,
            baseline_y,
            font_size: fs,
            base_font: base,
        }
    }

    /// Shape every printable ASCII character (32..=126) in a single
    /// `shape_line` call and extract the per-character `GlyphId`.
    fn resolve_variant(text_sys: &WindowTextSystem, f: &Font, fs: Pixels) -> FontGlyphs {
        let chars: String = (32u8..=126u8).map(|b| b as char).collect();
        let byte_len = chars.len();
        let shaped = text_sys.shape_line(
            chars.into(),
            fs,
            &[TextRun {
                len: byte_len,
                font: f.clone(),
                color: hsla(1., 1., 1., 1.),
                background_color: None,
                underline: None,
                strikethrough: None,
            }],
            None,
        );

        let mut ascii = [None; 128];

        for run in &shaped.runs {
            let fid = run.font_id;
            for glyph in &run.glyphs {
                // glyph.index is the byte offset into the shaped string which
                // starts at codepoint 32 (space), so the actual codepoint is
                // glyph.index + 32.
                let codepoint = glyph.index + 32;
                if codepoint < 128 {
                    ascii[codepoint] = Some((fid, glyph.id));
                }
            }
        }

        FontGlyphs { ascii }
    }

    /// O(1) lookup for an ASCII character with the right style variant.
    #[inline]
    fn lookup(&self, ch: char, bold: bool, italic: bool) -> Option<(FontId, GlyphId)> {
        let glyphs = match (bold, italic) {
            (true, true) => &self.bold_italic,
            (true, false) => &self.bold,
            (false, true) => &self.italic,
            (false, false) => &self.regular,
        };

        let idx = ch as u32;
        if idx < 128 {
            glyphs.ascii[idx as usize]
        } else {
            None
        }
    }
}

// ---------------------------------------------------------------------------
// Procedural block / box-drawing character renderer
// ---------------------------------------------------------------------------

/// Try to draw a block element or box-drawing character procedurally.
/// Returns `true` if the character was handled, `false` to fall through to
/// the normal font-based renderer.
#[inline]
fn paint_block_char(
    ch: char,
    x: Pixels,
    y: Pixels,
    cell_w: Pixels,
    cell_h: Pixels,
    color: Hsla,
    window: &mut Window,
) -> bool {
    match ch {
        // ---- Block Elements (U+2580–U+259F) ----

        // Upper half block
        '\u{2580}' => {
            window.paint_quad(fill(
                Bounds::new(point(x, y), size(cell_w, cell_h * 0.5)),
                color,
            ));
            true
        }
        // Lower 1/8 .. 7/8 blocks
        '\u{2581}' => {
            let h = cell_h * 0.125;
            window.paint_quad(fill(
                Bounds::new(point(x, y + cell_h - h), size(cell_w, h)),
                color,
            ));
            true
        }
        '\u{2582}' => {
            let h = cell_h * 0.25;
            window.paint_quad(fill(
                Bounds::new(point(x, y + cell_h - h), size(cell_w, h)),
                color,
            ));
            true
        }
        '\u{2583}' => {
            let h = cell_h * 0.375;
            window.paint_quad(fill(
                Bounds::new(point(x, y + cell_h - h), size(cell_w, h)),
                color,
            ));
            true
        }
        '\u{2584}' => {
            let h = cell_h * 0.5;
            window.paint_quad(fill(
                Bounds::new(point(x, y + cell_h - h), size(cell_w, h)),
                color,
            ));
            true
        }
        '\u{2585}' => {
            let h = cell_h * 0.625;
            window.paint_quad(fill(
                Bounds::new(point(x, y + cell_h - h), size(cell_w, h)),
                color,
            ));
            true
        }
        '\u{2586}' => {
            let h = cell_h * 0.75;
            window.paint_quad(fill(
                Bounds::new(point(x, y + cell_h - h), size(cell_w, h)),
                color,
            ));
            true
        }
        '\u{2587}' => {
            let h = cell_h * 0.875;
            window.paint_quad(fill(
                Bounds::new(point(x, y + cell_h - h), size(cell_w, h)),
                color,
            ));
            true
        }
        // Full block
        '\u{2588}' => {
            window.paint_quad(fill(Bounds::new(point(x, y), size(cell_w, cell_h)), color));
            true
        }
        // Left 7/8 .. 1/8 blocks
        '\u{2589}' => {
            let w = cell_w * 0.875;
            window.paint_quad(fill(Bounds::new(point(x, y), size(w, cell_h)), color));
            true
        }
        '\u{258A}' => {
            let w = cell_w * 0.75;
            window.paint_quad(fill(Bounds::new(point(x, y), size(w, cell_h)), color));
            true
        }
        '\u{258B}' => {
            let w = cell_w * 0.625;
            window.paint_quad(fill(Bounds::new(point(x, y), size(w, cell_h)), color));
            true
        }
        '\u{258C}' => {
            let w = cell_w * 0.5;
            window.paint_quad(fill(Bounds::new(point(x, y), size(w, cell_h)), color));
            true
        }
        '\u{258D}' => {
            let w = cell_w * 0.375;
            window.paint_quad(fill(Bounds::new(point(x, y), size(w, cell_h)), color));
            true
        }
        '\u{258E}' => {
            let w = cell_w * 0.25;
            window.paint_quad(fill(Bounds::new(point(x, y), size(w, cell_h)), color));
            true
        }
        '\u{258F}' => {
            let w = cell_w * 0.125;
            window.paint_quad(fill(Bounds::new(point(x, y), size(w, cell_h)), color));
            true
        }

        // Right half block
        '\u{2590}' => {
            let w = cell_w * 0.5;
            window.paint_quad(fill(Bounds::new(point(x + w, y), size(w, cell_h)), color));
            true
        }

        // Shade characters
        '\u{2591}' => {
            // Light shade (25%)
            window.paint_quad(fill(
                Bounds::new(point(x, y), size(cell_w, cell_h)),
                color.opacity(0.25),
            ));
            true
        }
        '\u{2592}' => {
            // Medium shade (50%)
            window.paint_quad(fill(
                Bounds::new(point(x, y), size(cell_w, cell_h)),
                color.opacity(0.5),
            ));
            true
        }
        '\u{2593}' => {
            // Dark shade (75%)
            window.paint_quad(fill(
                Bounds::new(point(x, y), size(cell_w, cell_h)),
                color.opacity(0.75),
            ));
            true
        }

        // Upper 1/8 block
        '\u{2594}' => {
            let h = cell_h * 0.125;
            window.paint_quad(fill(Bounds::new(point(x, y), size(cell_w, h)), color));
            true
        }
        // Right 1/8 block
        '\u{2595}' => {
            let w = cell_w * 0.125;
            window.paint_quad(fill(
                Bounds::new(point(x + cell_w - w, y), size(w, cell_h)),
                color,
            ));
            true
        }

        // ---- Box-drawing lines (U+2500–U+257F) — most common subset ----

        // ─ Horizontal line
        '\u{2500}' | '\u{2501}' => {
            let thick = if ch == '\u{2501}' { px(2.0) } else { px(1.0) };
            let mid_y = y + cell_h * 0.5 - thick * 0.5;
            window.paint_quad(fill(
                Bounds::new(point(x, mid_y), size(cell_w, thick)),
                color,
            ));
            true
        }
        // │ Vertical line
        '\u{2502}' | '\u{2503}' => {
            let thick = if ch == '\u{2503}' { px(2.0) } else { px(1.0) };
            let mid_x = x + cell_w * 0.5 - thick * 0.5;
            window.paint_quad(fill(
                Bounds::new(point(mid_x, y), size(thick, cell_h)),
                color,
            ));
            true
        }
        // ┌ Upper-left corner
        '\u{250C}' | '\u{250F}' => {
            let thick = if ch == '\u{250F}' { px(2.0) } else { px(1.0) };
            let mid_x = x + cell_w * 0.5 - thick * 0.5;
            let mid_y = y + cell_h * 0.5 - thick * 0.5;
            window.paint_quad(fill(
                Bounds::new(point(mid_x, mid_y), size(cell_w - (mid_x - x), thick)),
                color,
            ));
            window.paint_quad(fill(
                Bounds::new(point(mid_x, mid_y), size(thick, cell_h - (mid_y - y))),
                color,
            ));
            true
        }
        // ┐ Upper-right corner
        '\u{2510}' | '\u{2513}' => {
            let thick = if ch == '\u{2513}' { px(2.0) } else { px(1.0) };
            let mid_x = x + cell_w * 0.5 - thick * 0.5;
            let mid_y = y + cell_h * 0.5 - thick * 0.5;
            window.paint_quad(fill(
                Bounds::new(point(x, mid_y), size(mid_x - x + thick, thick)),
                color,
            ));
            window.paint_quad(fill(
                Bounds::new(point(mid_x, mid_y), size(thick, cell_h - (mid_y - y))),
                color,
            ));
            true
        }
        // └ Lower-left corner
        '\u{2514}' | '\u{2517}' => {
            let thick = if ch == '\u{2517}' { px(2.0) } else { px(1.0) };
            let mid_x = x + cell_w * 0.5 - thick * 0.5;
            let mid_y = y + cell_h * 0.5 - thick * 0.5;
            window.paint_quad(fill(
                Bounds::new(point(mid_x, mid_y), size(cell_w - (mid_x - x), thick)),
                color,
            ));
            window.paint_quad(fill(
                Bounds::new(point(mid_x, y), size(thick, mid_y - y + thick)),
                color,
            ));
            true
        }
        // ┘ Lower-right corner
        '\u{2518}' | '\u{251B}' => {
            let thick = if ch == '\u{251B}' { px(2.0) } else { px(1.0) };
            let mid_x = x + cell_w * 0.5 - thick * 0.5;
            let mid_y = y + cell_h * 0.5 - thick * 0.5;
            window.paint_quad(fill(
                Bounds::new(point(x, mid_y), size(mid_x - x + thick, thick)),
                color,
            ));
            window.paint_quad(fill(
                Bounds::new(point(mid_x, y), size(thick, mid_y - y + thick)),
                color,
            ));
            true
        }
        // ├ Left tee
        '\u{251C}' | '\u{2523}' => {
            let thick = if ch == '\u{2523}' { px(2.0) } else { px(1.0) };
            let mid_x = x + cell_w * 0.5 - thick * 0.5;
            let mid_y = y + cell_h * 0.5 - thick * 0.5;
            window.paint_quad(fill(
                Bounds::new(point(mid_x, y), size(thick, cell_h)),
                color,
            ));
            window.paint_quad(fill(
                Bounds::new(point(mid_x, mid_y), size(cell_w - (mid_x - x), thick)),
                color,
            ));
            true
        }
        // ┤ Right tee
        '\u{2524}' | '\u{252B}' => {
            let thick = if ch == '\u{252B}' { px(2.0) } else { px(1.0) };
            let mid_x = x + cell_w * 0.5 - thick * 0.5;
            let mid_y = y + cell_h * 0.5 - thick * 0.5;
            window.paint_quad(fill(
                Bounds::new(point(mid_x, y), size(thick, cell_h)),
                color,
            ));
            window.paint_quad(fill(
                Bounds::new(point(x, mid_y), size(mid_x - x + thick, thick)),
                color,
            ));
            true
        }
        // ┬ Top tee
        '\u{252C}' | '\u{2533}' => {
            let thick = if ch == '\u{2533}' { px(2.0) } else { px(1.0) };
            let mid_x = x + cell_w * 0.5 - thick * 0.5;
            let mid_y = y + cell_h * 0.5 - thick * 0.5;
            window.paint_quad(fill(
                Bounds::new(point(x, mid_y), size(cell_w, thick)),
                color,
            ));
            window.paint_quad(fill(
                Bounds::new(point(mid_x, mid_y), size(thick, cell_h - (mid_y - y))),
                color,
            ));
            true
        }
        // ┴ Bottom tee
        '\u{2534}' | '\u{253B}' => {
            let thick = if ch == '\u{253B}' { px(2.0) } else { px(1.0) };
            let mid_x = x + cell_w * 0.5 - thick * 0.5;
            let mid_y = y + cell_h * 0.5 - thick * 0.5;
            window.paint_quad(fill(
                Bounds::new(point(x, mid_y), size(cell_w, thick)),
                color,
            ));
            window.paint_quad(fill(
                Bounds::new(point(mid_x, y), size(thick, mid_y - y + thick)),
                color,
            ));
            true
        }
        // ┼ Cross
        '\u{253C}' | '\u{254B}' => {
            let thick = if ch == '\u{254B}' { px(2.0) } else { px(1.0) };
            let mid_x = x + cell_w * 0.5 - thick * 0.5;
            let mid_y = y + cell_h * 0.5 - thick * 0.5;
            window.paint_quad(fill(
                Bounds::new(point(x, mid_y), size(cell_w, thick)),
                color,
            ));
            window.paint_quad(fill(
                Bounds::new(point(mid_x, y), size(thick, cell_h)),
                color,
            ));
            true
        }
        // ╴ Right-end stub (light)
        '\u{2574}' => {
            let mid_x = x + cell_w * 0.5;
            let mid_y = y + cell_h * 0.5 - px(0.5);
            window.paint_quad(fill(
                Bounds::new(point(x, mid_y), size(mid_x - x, px(1.0))),
                color,
            ));
            true
        }
        // ╵ Up-end stub (light)
        '\u{2575}' => {
            let mid_x = x + cell_w * 0.5 - px(0.5);
            let mid_y = y + cell_h * 0.5;
            window.paint_quad(fill(
                Bounds::new(point(mid_x, y), size(px(1.0), mid_y - y)),
                color,
            ));
            true
        }
        // ╶ Left-end stub (light)
        '\u{2576}' => {
            let mid_x = x + cell_w * 0.5;
            let mid_y = y + cell_h * 0.5 - px(0.5);
            window.paint_quad(fill(
                Bounds::new(point(mid_x, mid_y), size(cell_w - (mid_x - x), px(1.0))),
                color,
            ));
            true
        }
        // ╷ Down-end stub (light)
        '\u{2577}' => {
            let mid_x = x + cell_w * 0.5 - px(0.5);
            let mid_y = y + cell_h * 0.5;
            window.paint_quad(fill(
                Bounds::new(point(mid_x, mid_y), size(px(1.0), cell_h - (mid_y - y))),
                color,
            ));
            true
        }

        _ => false,
    }
}

// ---------------------------------------------------------------------------
// Colour conversion helper
// ---------------------------------------------------------------------------

/// Runtime color palette for the terminal renderer. Overrides the default
/// xterm colors with theme-specific values.
#[derive(Clone)]
struct TerminalPalette {
    /// ANSI colors 0-15 as (r, g, b).
    ansi: [[u8; 3]; 16],
    /// Default foreground (r, g, b).
    foreground: [u8; 3],
    /// Default background (r, g, b).
    background: [u8; 3],
}

impl Default for TerminalPalette {
    fn default() -> Self {
        Self::from_theme(&TerminalTheme::dark())
    }
}

impl TerminalPalette {
    fn from_theme(theme: &TerminalTheme) -> Self {
        let parse = |hex: &str| -> [u8; 3] {
            let hex = hex.trim_start_matches('#');
            let r = u8::from_str_radix(&hex[0..2], 16).unwrap_or(0);
            let g = u8::from_str_radix(&hex[2..4], 16).unwrap_or(0);
            let b = u8::from_str_radix(&hex[4..6], 16).unwrap_or(0);
            [r, g, b]
        };

        let mut ansi = [[0u8; 3]; 16];
        for (i, hex) in theme.ansi_colors.iter().enumerate() {
            ansi[i] = parse(hex);
        }

        Self {
            ansi,
            foreground: parse(&theme.foreground),
            background: parse(&theme.background),
        }
    }

    /// Resolve a `TermColor` to an HSLA value using this palette.
    #[inline]
    fn resolve(&self, color: &TermColor, is_foreground: bool) -> Hsla {
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
fn brighten_for_bold(color: TermColor) -> TermColor {
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
fn dim_color(color: TermColor) -> TermColor {
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

// ---------------------------------------------------------------------------
// Terminal view types
// ---------------------------------------------------------------------------

// Actions for terminal-specific keyboard shortcuts
actions!(
    shelldeck,
    [
        CopySelection,
        PasteClipboard,
        ClearTerminal,
        ToggleSearch,
        SearchNext,
        SearchPrev,
        SplitHorizontal,
        SplitVertical,
        ClosePane,
        ZoomIn,
        ZoomOut,
        ZoomReset,
        ToggleSplitFocus,
    ]
);

/// Events emitted by the terminal view
#[derive(Debug, Clone)]
pub enum TerminalEvent {
    TabSelected(Uuid),
    TabClosed(Uuid),
    NewTabRequested,
    /// The user requested a split pane. If connection_id is Some, the workspace
    /// should open an SSH session for the split; otherwise a local terminal was
    /// already spawned.
    SplitRequested {
        connection_id: Uuid,
        direction: SplitDirection,
    },
    /// Run a script by ID (emitted from the toolbar script dropdown).
    RunScriptRequested(Uuid),
}

impl EventEmitter<TerminalEvent> for TerminalView {}

// ---------------------------------------------------------------------------
// Layout constants – centralised so they aren't scattered as magic numbers.
// ---------------------------------------------------------------------------

/// Height of the workspace titlebar in pixels (rendered outside terminal view).
const TITLEBAR_HEIGHT: f32 = 38.0;
/// Height of the terminal tab bar in pixels.
const TAB_BAR_HEIGHT: f32 = 38.0;
/// Height of the toolbar row below the tab bar.
const TOOLBAR_HEIGHT: f32 = 32.0;
/// Height of the status bar at the bottom of the window.
const STATUS_BAR_HEIGHT: f32 = 28.0;
/// Width of the sidebar resize handle.
const SIDEBAR_HANDLE_WIDTH: f32 = 4.0;
/// Width of the split-pane divider bar.
const SPLIT_DIVIDER_SIZE: f32 = 6.0;
/// Total vertical offset from the window top to the terminal grid
/// (tab bar + toolbar).
const GRID_TOP_OFFSET: f32 = TAB_BAR_HEIGHT + TOOLBAR_HEIGHT;
/// Scrollbar track width.
const SCROLLBAR_WIDTH: f32 = 6.0;
/// Right margin of the scrollbar from the grid edge.
const SCROLLBAR_MARGIN: f32 = 2.0;

/// A single tab in the terminal tab bar
#[derive(Debug, Clone)]
pub struct TerminalTab {
    pub id: Uuid,
    pub title: String,
    pub is_active: bool,
    pub state: SessionState,
    pub zoom_level: f32,
    /// The connection ID this tab is associated with, if any (None for local terminals).
    pub connection_id: Option<Uuid>,
}

/// Terminal pane holding sessions
pub struct TerminalPane {
    pub sessions: Vec<TerminalSession>,
    pub active_index: usize,
}

#[derive(Debug, Clone, Copy)]
pub enum SplitDirection {
    Horizontal,
    Vertical,
}

/// Tracks an active split-pane view.
/// The secondary session lives here, separate from the tab list.
struct SplitViewState {
    direction: SplitDirection,
    /// The secondary terminal session (not in tabs).
    secondary_session: TerminalSession,
    /// Whether the secondary pane currently has keyboard focus.
    focus_secondary: bool,
    ratio: f32,
}

pub struct TerminalView {
    pub pane: TerminalPane,
    pub tabs: Vec<TerminalTab>,
    pub font_size: f32,
    pub font_family: String,
    pub focus_handle: FocusHandle,
    _refresh_task: Option<gpui::Task<()>>,
    /// Last known grid dimensions so we can detect when a resize is needed.
    last_grid_rows: u16,
    last_grid_cols: u16,
    /// Last known secondary pane grid dimensions (may differ from primary).
    last_secondary_rows: u16,
    last_secondary_cols: u16,
    /// Set when a session is added so the next render focuses the grid (once).
    needs_focus: bool,
    /// Pre-resolved glyph cache – rebuilt only when font/size changes.
    glyph_cache: Option<Arc<GlyphCache>>,
    /// Timestamp of the last mouse-down for multi-click detection.
    last_click_time: Option<std::time::Instant>,
    /// Number of rapid consecutive clicks (1=single, 2=double, 3=triple).
    click_count: u8,
    /// Position of the last click (for multi-click proximity check).
    last_click_pos: Option<(usize, usize)>,
    /// Whether the left mouse button is held for drag-selection.
    selecting: bool,
    /// Search state
    search_visible: bool,
    search_query: String,
    search_matches: Vec<SearchMatch>,
    search_current_idx: Option<usize>,
    search_case_sensitive: bool,
    search_regex: bool,
    /// Detected URLs in the visible grid (refreshed each frame).
    detected_urls: Vec<UrlMatch>,
    /// Index of the URL currently hovered by the mouse.
    _hovered_url: Option<usize>,
    /// Context menu state (position + whether visible).
    context_menu: Option<ContextMenuState>,
    /// Active split view for the current tab (two panes displayed simultaneously).
    split_view: Option<SplitViewState>,
    /// Stored split views for inactive tabs, keyed by tab ID.
    stored_splits: HashMap<Uuid, SplitViewState>,
    /// Dynamic pixel offsets for the active grid (set during render).
    grid_x_offset: f32,
    grid_y_offset: f32,
    /// Sidebar width in pixels (updated dynamically from workspace).
    sidebar_width: f32,
    /// Whether the split divider is being dragged.
    split_dragging: bool,
    /// Whether the scrollbar thumb is being dragged.
    scrollbar_dragging: bool,
    /// Active terminal color palette (set from theme).
    palette: TerminalPalette,
    /// User-preferred cursor shape (overrides the grid's DECSCUSR shape when set).
    cursor_style_override: Option<CursorShape>,
    /// Script dropdown state for toolbar integration.
    script_dropdown_open: bool,
    /// Favorite scripts for toolbar dropdown.
    favorite_scripts: Vec<(Uuid, String, String)>,
    /// Recently run scripts for toolbar dropdown.
    recent_scripts: Vec<(Uuid, String, String)>,
    /// Current blink phase: true = cursor visible, false = cursor hidden.
    cursor_blink_on: bool,
    /// Async task that toggles `cursor_blink_on` every 530ms.
    cursor_blink_timer: Option<gpui::Task<()>>,
    /// Whether the terminal grid currently has focus (tracked for hollow cursor).
    has_focus: bool,
}

#[derive(Debug, Clone)]
struct ContextMenuState {
    position: Point<Pixels>,
    /// URL under the right-click, if any.
    url: Option<String>,
}

impl TerminalView {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            pane: TerminalPane {
                sessions: Vec::new(),
                active_index: 0,
            },
            tabs: Vec::new(),
            font_size: 14.0,
            font_family: "JetBrains Mono".to_string(),
            focus_handle: cx.focus_handle(),
            _refresh_task: None,
            last_grid_rows: 0,
            last_grid_cols: 0,
            last_secondary_rows: 0,
            last_secondary_cols: 0,
            needs_focus: false,
            glyph_cache: None,
            last_click_time: None,
            click_count: 0,
            last_click_pos: None,
            selecting: false,
            search_visible: false,
            search_query: String::new(),
            search_matches: Vec::new(),
            search_current_idx: None,
            search_case_sensitive: false,
            search_regex: false,
            detected_urls: Vec::new(),
            _hovered_url: None,
            context_menu: None,
            split_view: None,
            stored_splits: HashMap::new(),
            grid_x_offset: 260.0 + SIDEBAR_HANDLE_WIDTH,
            grid_y_offset: GRID_TOP_OFFSET,
            sidebar_width: 260.0,
            split_dragging: false,
            scrollbar_dragging: false,
            palette: TerminalPalette::default(),
            cursor_style_override: None,
            script_dropdown_open: false,
            favorite_scripts: Vec::new(),
            recent_scripts: Vec::new(),
            cursor_blink_on: true,
            cursor_blink_timer: None,
            has_focus: false,
        }
    }

    /// Update favorite and recent scripts for the toolbar dropdown.
    pub fn set_scripts(
        &mut self,
        favorites: Vec<(Uuid, String, shelldeck_core::models::script::ScriptLanguage)>,
        recent: Vec<(Uuid, String, shelldeck_core::models::script::ScriptLanguage)>,
    ) {
        self.favorite_scripts = favorites
            .into_iter()
            .map(|(id, name, lang)| (id, name, lang.badge().to_string()))
            .collect();
        self.recent_scripts = recent
            .into_iter()
            .map(|(id, name, lang)| (id, name, lang.badge().to_string()))
            .collect();
    }

    /// Update the sidebar width used for coordinate mapping.
    pub fn set_sidebar_width(&mut self, width: f32) {
        self.sidebar_width = width;
    }

    /// Apply a terminal color theme to the renderer.
    pub fn set_terminal_theme(&mut self, theme: &TerminalTheme) {
        self.palette = TerminalPalette::from_theme(theme);
    }

    /// Update the base font size (invalidates glyph cache).
    pub fn set_font_size(&mut self, size: f32) {
        self.font_size = size.clamp(8.0, 36.0);
        self.glyph_cache = None;
    }

    /// Update the font family (invalidates glyph cache).
    pub fn set_font_family(&mut self, family: String) {
        self.font_family = family;
        self.glyph_cache = None;
    }

    /// Update the cursor style preference.
    ///
    /// Pass `"default"` (or any unrecognized value) to clear the override and
    /// let the terminal application control the shape via DECSCUSR.
    pub fn set_cursor_style(&mut self, style: &str) {
        self.cursor_style_override = match style {
            "underline" => Some(CursorShape::Underline),
            "bar" => Some(CursorShape::Bar),
            "block" => Some(CursorShape::Block),
            _ => None,
        };
    }

    // ------------------------------------------------------------------
    // Cursor blink timer
    // ------------------------------------------------------------------

    /// Start (or restart) the cursor blink timer.
    ///
    /// The cursor is made visible immediately and then toggled every 530 ms.
    /// Calling this while a timer is already running cancels the old one.
    fn start_cursor_blink(&mut self, cx: &mut Context<Self>) {
        self.cursor_blink_on = true;
        let entity = cx.entity().downgrade();
        self.cursor_blink_timer = Some(cx.spawn(
            async move |_this: WeakEntity<Self>, cx: &mut AsyncApp| loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(530))
                    .await;
                let Ok(_) = entity.update(cx, |this, cx| {
                    this.cursor_blink_on = !this.cursor_blink_on;
                    cx.notify();
                }) else {
                    break;
                };
            },
        ));
    }

    /// Stop the blink timer and ensure the cursor is visible (steady).
    fn stop_cursor_blink(&mut self) {
        self.cursor_blink_timer = None;
        self.cursor_blink_on = true;
    }

    /// Reset the blink cycle so the cursor stays visible right after input,
    /// then resumes blinking after the interval.
    fn reset_cursor_blink(&mut self, cx: &mut Context<Self>) {
        // Only restart if the grid says blinking is enabled
        let grid_blink = self
            .active_session()
            .map(|s| s.grid.lock().cursor.blink)
            .unwrap_or(false);
        if grid_blink && self.has_focus {
            self.start_cursor_blink(cx);
        } else {
            self.stop_cursor_blink();
        }
    }

    /// Determine the effective cursor shape: the user override wins if set,
    /// otherwise the grid's shape (set by DECSCUSR) is used.
    fn effective_cursor_shape(&self, grid_shape: CursorShape) -> CursorShape {
        self.cursor_style_override.unwrap_or(grid_shape)
    }

    pub fn add_session(&mut self, session: TerminalSession) {
        self.add_session_with_connection(session, None);
    }

    pub fn add_session_with_connection(
        &mut self,
        session: TerminalSession,
        connection_id: Option<Uuid>,
    ) {
        // Save the current tab's split before switching away
        if let Some(current_tab) = self.tabs.get(self.pane.active_index) {
            let current_id = current_tab.id;
            if let Some(split) = self.split_view.take() {
                self.stored_splits.insert(current_id, split);
            }
        }

        let tab = TerminalTab {
            id: session.id,
            title: session.title.clone(),
            is_active: true,
            state: session.state.clone(),
            zoom_level: 1.0,
            connection_id,
        };

        // Deactivate other tabs
        for t in &mut self.tabs {
            t.is_active = false;
        }

        self.tabs.push(tab);
        self.pane.active_index = self.pane.sessions.len();
        self.pane.sessions.push(session);

        // Request focus on next render
        self.needs_focus = true;
    }

    pub fn close_tab(&mut self, id: Uuid) {
        if let Some(pos) = self.tabs.iter().position(|t| t.id == id) {
            // Clear the tab's split — either from active split_view or stored_splits
            let was_active = pos == self.pane.active_index;
            if was_active {
                self.split_view = None;
            } else {
                self.stored_splits.remove(&id);
            }

            self.tabs.remove(pos);
            self.pane.sessions.remove(pos);

            if self.pane.active_index >= self.pane.sessions.len() && !self.pane.sessions.is_empty()
            {
                self.pane.active_index = self.pane.sessions.len() - 1;
            }

            // Restore the new active tab's split if it had one
            if was_active && !self.pane.sessions.is_empty() {
                if let Some(tab) = self.tabs.get(self.pane.active_index) {
                    self.split_view = self.stored_splits.remove(&tab.id);
                }
            }

            if let Some(tab) = self.tabs.get_mut(self.pane.active_index) {
                tab.is_active = true;
            }

            // Stop the refresh task when all sessions are closed (drop cancels the task)
            if self.pane.sessions.is_empty() {
                self.split_view = None;
                self.stored_splits.clear();
                self._refresh_task = None;
            }
        }
    }

    /// Find a tab that belongs to the given connection, if any.
    pub fn find_tab_for_connection(&self, connection_id: Uuid) -> Option<Uuid> {
        self.tabs
            .iter()
            .find(|t| t.connection_id == Some(connection_id))
            .map(|t| t.id)
    }

    pub fn select_tab(&mut self, id: Uuid) {
        // Save the current tab's split (if any) before switching
        if let Some(current_tab) = self.tabs.get(self.pane.active_index) {
            let current_id = current_tab.id;
            if current_id != id {
                if let Some(split) = self.split_view.take() {
                    self.stored_splits.insert(current_id, split);
                }
            }
        }

        for (i, tab) in self.tabs.iter_mut().enumerate() {
            tab.is_active = tab.id == id;
            if tab.is_active {
                self.pane.active_index = i;
            }
        }

        // Restore the new tab's split (if it had one)
        if self.split_view.is_none() {
            self.split_view = self.stored_splits.remove(&id);
        }

        // Reset split focus to primary pane when switching tabs
        if let Some(ref mut split) = self.split_view {
            split.focus_secondary = false;
        }

        // Reset secondary dimensions so resize_if_needed picks up the new split
        self.last_secondary_rows = 0;
        self.last_secondary_cols = 0;
    }

    /// Toggle focus between the primary (tab) and secondary (split) pane.
    pub fn toggle_split_focus(&mut self) {
        if let Some(ref mut split) = self.split_view {
            split.focus_secondary = !split.focus_secondary;
        }
    }

    /// Switch to the next tab (wraps around).
    pub fn next_tab(&mut self) {
        if self.tabs.len() <= 1 {
            return;
        }
        let next = (self.pane.active_index + 1) % self.tabs.len();
        let id = self.tabs[next].id;
        self.select_tab(id);
    }

    /// Switch to the previous tab (wraps around).
    pub fn prev_tab(&mut self) {
        if self.tabs.len() <= 1 {
            return;
        }
        let prev = if self.pane.active_index == 0 {
            self.tabs.len() - 1
        } else {
            self.pane.active_index - 1
        };
        let id = self.tabs[prev].id;
        self.select_tab(id);
    }

    pub fn active_session(&self) -> Option<&TerminalSession> {
        // If split is active and focus is on the secondary pane, return that session
        if let Some(ref split) = self.split_view {
            if split.focus_secondary {
                return Some(&split.secondary_session);
            }
        }
        self.pane.sessions.get(self.pane.active_index)
    }

    /// Close all terminal sessions for graceful shutdown.
    pub fn close_all_sessions(&mut self) {
        tracing::info!("Closing {} terminal sessions", self.pane.sessions.len());
        self.split_view = None;
        self.stored_splits.clear();
        self.tabs.clear();
        self.pane.sessions.clear();
        self.pane.active_index = 0;
        self._refresh_task = None;
    }

    /// Return the number of open terminal tabs.
    pub fn tab_count(&self) -> usize {
        self.tabs.len()
    }

    /// Return the last computed grid dimensions, or a default if unknown.
    pub fn grid_size(&self) -> (u16, u16) {
        if self.last_grid_rows > 0 {
            (self.last_grid_rows, self.last_grid_cols)
        } else {
            (24, 80)
        }
    }

    /// Convert a GPUI KeyDownEvent into the byte sequence expected by a terminal.
    ///
    /// `app_cursor` indicates whether application cursor keys mode (DECCKM) is
    /// active on the grid. When true, arrow keys and Home/End emit SS3
    /// sequences instead of CSI sequences.
    fn keystroke_to_bytes(event: &KeyDownEvent, app_cursor: bool) -> Option<Vec<u8>> {
        let keystroke = &event.keystroke;
        let key = keystroke.key.as_str();
        let mods = &keystroke.modifiers;

        // Ctrl+key combos: letter & 0x1f produces the control character.
        // Skip when Shift is held so Ctrl+Shift+C/V reach the Copy/Paste actions.
        if mods.control && !mods.alt && !mods.shift && key.len() == 1 {
            let ch = key
                .chars()
                .next()
                .expect("key.len() == 1 guarantees a char");
            if ch.is_ascii_alphabetic() {
                return Some(vec![(ch.to_ascii_lowercase() as u8) & 0x1f]);
            }
        }

        // Compute xterm modifier code for modified special keys.
        // Shift=2, Alt=3, Shift+Alt=4, Ctrl=5, Shift+Ctrl=6, Alt+Ctrl=7,
        // Shift+Alt+Ctrl=8.
        let modifier_code = || -> Option<u8> {
            let val = 1
                + if mods.shift { 1 } else { 0 }
                + if mods.alt { 2 } else { 0 }
                + if mods.control { 4 } else { 0 };
            if val > 1 {
                Some(val)
            } else {
                None
            }
        };

        // ---- Function keys F1..F24 ----
        if let Some(fnum) = key.strip_prefix('f').and_then(|s| s.parse::<u8>().ok()) {
            if (1..=24).contains(&fnum) {
                return Some(Self::function_key_bytes(fnum, modifier_code()));
            }
        }

        match key {
            "enter" => Some(b"\r".to_vec()),
            "backspace" => Some(vec![0x7f]),
            "tab" => Some(b"\t".to_vec()),
            "escape" => Some(vec![0x1b]),

            // Arrow keys: SS3 in application cursor mode, CSI otherwise.
            // With modifiers always use CSI form: \x1b[1;{mod}{final}
            "up" | "down" | "right" | "left" => {
                let final_byte = match key {
                    "up" => b'A',
                    "down" => b'B',
                    "right" => b'C',
                    "left" => b'D',
                    _ => unreachable!(),
                };
                if let Some(m) = modifier_code() {
                    Some(format!("\x1b[1;{}{}", m, final_byte as char).into_bytes())
                } else if app_cursor {
                    Some(vec![0x1b, b'O', final_byte])
                } else {
                    Some(vec![0x1b, b'[', final_byte])
                }
            }

            // Home / End: SS3 in application cursor mode, CSI otherwise.
            "home" => {
                if let Some(m) = modifier_code() {
                    Some(format!("\x1b[1;{}H", m).into_bytes())
                } else if app_cursor {
                    Some(b"\x1bOH".to_vec())
                } else {
                    Some(b"\x1b[H".to_vec())
                }
            }
            "end" => {
                if let Some(m) = modifier_code() {
                    Some(format!("\x1b[1;{}F", m).into_bytes())
                } else if app_cursor {
                    Some(b"\x1bOF".to_vec())
                } else {
                    Some(b"\x1b[F".to_vec())
                }
            }

            "insert" => Some(b"\x1b[2~".to_vec()),
            "delete" => Some(b"\x1b[3~".to_vec()),
            "pageup" => Some(b"\x1b[5~".to_vec()),
            "pagedown" => Some(b"\x1b[6~".to_vec()),
            "space" => {
                // Alt+Space sends ESC followed by space
                if mods.alt {
                    Some(b"\x1b ".to_vec())
                } else {
                    Some(b" ".to_vec())
                }
            }
            _ => {
                // Alt+key: send ESC prefix before the character.
                if mods.alt && !mods.control {
                    if let Some(ref kc) = keystroke.key_char {
                        let mut bytes = vec![0x1b];
                        bytes.extend_from_slice(kc.as_bytes());
                        return Some(bytes);
                    } else if key.len() == 1 {
                        let mut bytes = vec![0x1b];
                        bytes.extend_from_slice(key.as_bytes());
                        return Some(bytes);
                    }
                }

                // Use key_char for typed characters (handles shift etc.)
                if let Some(ref kc) = keystroke.key_char {
                    Some(kc.as_bytes().to_vec())
                } else if key.len() == 1 {
                    Some(key.as_bytes().to_vec())
                } else {
                    None
                }
            }
        }
    }

    /// Build the escape sequence for a function key F1..F24, optionally with
    /// an xterm modifier code.
    fn function_key_bytes(fnum: u8, modifier: Option<u8>) -> Vec<u8> {
        // F1-F4 use SS3 finals P/Q/R/S (no modifier) or CSI 1;mod P/Q/R/S
        // F5+ use CSI code ~ format
        match fnum {
            1..=4 => {
                let final_ch = match fnum {
                    1 => 'P',
                    2 => 'Q',
                    3 => 'R',
                    4 => 'S',
                    _ => unreachable!(),
                };
                if let Some(m) = modifier {
                    format!("\x1b[1;{}{}", m, final_ch).into_bytes()
                } else {
                    format!("\x1bO{}", final_ch).into_bytes()
                }
            }
            5..=24 => {
                let code = match fnum {
                    5 => 15,
                    6 => 17,
                    7 => 18,
                    8 => 19,
                    9 => 20,
                    10 => 21,
                    11 => 23,
                    12 => 24,
                    13 => 25,
                    14 => 26,
                    15 => 28,
                    16 => 29,
                    17 => 31,
                    18 => 32,
                    19 => 33,
                    20 => 34,
                    21 => 42,
                    22 => 43,
                    23 => 44,
                    24 => 45,
                    _ => unreachable!(),
                };
                if let Some(m) = modifier {
                    format!("\x1b[{};{}~", code, m).into_bytes()
                } else {
                    format!("\x1b[{}~", code).into_bytes()
                }
            }
            _ => Vec::new(),
        }
    }

    /// Compute terminal grid dimensions (rows, cols) from the window viewport.
    fn compute_grid_size(&self, window: &Window) -> (u16, u16) {
        let viewport = window.viewport_size();
        let fs = self.effective_font_size();
        let cell_width = fs * 0.6;
        let cell_height = fs * 1.4;

        let sidebar_px = px(self.sidebar_width + SIDEBAR_HANDLE_WIDTH * 2.0);
        let mut available_width = (viewport.width - sidebar_px).max(px(cell_width * 10.0));
        let mut available_height = (viewport.height
            - px(TITLEBAR_HEIGHT)
            - px(TAB_BAR_HEIGHT)
            - px(TOOLBAR_HEIGHT)
            - px(STATUS_BAR_HEIGHT))
        .max(px(cell_height * 2.0));

        // Account for split panes
        if let Some(ref split) = self.split_view {
            let divider = px(SPLIT_DIVIDER_SIZE);
            match split.direction {
                SplitDirection::Horizontal => {
                    available_width =
                        ((available_width - divider) * split.ratio).max(px(cell_width * 10.0));
                }
                SplitDirection::Vertical => {
                    available_height =
                        ((available_height - divider) * split.ratio).max(px(cell_height * 2.0));
                }
            }
        }

        let cols = (available_width / px(cell_width)).floor() as u16;
        let rows = (available_height / px(cell_height)).floor() as u16;

        (rows.max(2), cols.max(10))
    }

    /// Compute grid dimensions for the secondary split pane using `(1.0 - ratio)`.
    fn compute_secondary_grid_size(&self, window: &Window) -> (u16, u16) {
        let viewport = window.viewport_size();
        let fs = self.effective_font_size();
        let cell_width = fs * 0.6;
        let cell_height = fs * 1.4;

        let sidebar_px = px(self.sidebar_width + SIDEBAR_HANDLE_WIDTH * 2.0);
        let mut available_width = (viewport.width - sidebar_px).max(px(cell_width * 10.0));
        let mut available_height = (viewport.height
            - px(TITLEBAR_HEIGHT)
            - px(TAB_BAR_HEIGHT)
            - px(TOOLBAR_HEIGHT)
            - px(STATUS_BAR_HEIGHT))
        .max(px(cell_height * 2.0));

        if let Some(ref split) = self.split_view {
            let divider = px(SPLIT_DIVIDER_SIZE);
            match split.direction {
                SplitDirection::Horizontal => {
                    available_width = ((available_width - divider) * (1.0 - split.ratio))
                        .max(px(cell_width * 10.0));
                }
                SplitDirection::Vertical => {
                    available_height = ((available_height - divider) * (1.0 - split.ratio))
                        .max(px(cell_height * 2.0));
                }
            }
        }

        let cols = (available_width / px(cell_width)).floor() as u16;
        let rows = (available_height / px(cell_height)).floor() as u16;

        (rows.max(2), cols.max(10))
    }

    /// Check if the viewport size changed and resize all sessions if needed.
    fn resize_if_needed(&mut self, window: &Window) {
        let (rows, cols) = self.compute_grid_size(window);
        if rows != self.last_grid_rows || cols != self.last_grid_cols {
            self.last_grid_rows = rows;
            self.last_grid_cols = cols;
            for session in &self.pane.sessions {
                session.resize(rows, cols);
            }
        }
        // Resize the split's secondary session with its own dimensions
        if self.split_view.is_some() {
            let (sec_rows, sec_cols) = self.compute_secondary_grid_size(window);
            if sec_rows != self.last_secondary_rows || sec_cols != self.last_secondary_cols {
                self.last_secondary_rows = sec_rows;
                self.last_secondary_cols = sec_cols;
                if let Some(ref split) = self.split_view {
                    split.secondary_session.resize(sec_rows, sec_cols);
                }
            }
        }
    }

    pub fn spawn_local_terminal(&mut self, cx: &mut Context<Self>) {
        let (rows, cols) = if self.last_grid_rows > 0 {
            (self.last_grid_rows, self.last_grid_cols)
        } else {
            (24, 80)
        };
        match TerminalSession::spawn_local(None, rows, cols) {
            Ok(session) => {
                self.add_session(session);
                tracing::info!("Spawned new local terminal");
            }
            Err(e) => {
                tracing::error!("Failed to spawn terminal: {}", e);
            }
        }

        self.ensure_refresh_running(cx);
    }

    /// Start the periodic refresh loop if it is not already running.
    ///
    /// Polls at ~120 Hz but only triggers a repaint when the grid is
    /// actually dirty (new data arrived).
    pub fn ensure_refresh_running(&mut self, cx: &mut Context<Self>) {
        if self._refresh_task.is_none() {
            self._refresh_task =
                Some(cx.spawn(async |this: WeakEntity<Self>, cx: &mut AsyncApp| {
                    loop {
                        cx.background_executor()
                            .timer(std::time::Duration::from_millis(8))
                            .await;
                        let result = this.update(cx, |this, cx| {
                            let mut any_dirty = false;
                            let mut any_sync = false;
                            for s in this.pane.sessions.iter() {
                                let g = s.grid.lock();
                                if g.dirty {
                                    any_dirty = true;
                                }
                                if g.synchronized_output() {
                                    any_sync = true;
                                }
                            }
                            if let Some(ref split) = this.split_view {
                                let g = split.secondary_session.grid.lock();
                                if g.dirty {
                                    any_dirty = true;
                                }
                                if g.synchronized_output() {
                                    any_sync = true;
                                }
                            }
                            // Also clear dirty flags on stored (background) splits so
                            // they don't accumulate stale dirty state, but don't trigger
                            // a repaint just for background splits.
                            for split in this.stored_splits.values() {
                                split.secondary_session.grid.lock().dirty = false;
                            }
                            // Handle OSC 52 clipboard requests from any session.
                            for session in this.pane.sessions.iter() {
                                if let Some((_sel, text)) =
                                    session.grid.lock().clipboard_request.take()
                                {
                                    cx.write_to_clipboard(ClipboardItem::new_string(text));
                                }
                            }
                            if let Some(ref split) = this.split_view {
                                if let Some((_sel, text)) =
                                    split.secondary_session.grid.lock().clipboard_request.take()
                                {
                                    cx.write_to_clipboard(ClipboardItem::new_string(text));
                                }
                            }
                            // Suppress repaint while synchronized output is active
                            // (batching updates to prevent flicker). When the app
                            // turns sync off, dirty will be set and any_sync cleared.
                            if any_dirty && !any_sync {
                                cx.notify();
                            }
                        });
                        if result.is_err() {
                            break;
                        }
                    }
                }));
        }
    }

    /// Ensure the glyph cache is populated for the current font + zoom level.
    fn ensure_glyph_cache(&mut self, window: &Window) {
        if self.glyph_cache.is_none() {
            let fs = self.effective_font_size();
            self.glyph_cache = Some(Arc::new(GlyphCache::build(
                window.text_system(),
                &self.font_family,
                fs,
            )));
        }
    }

    fn render_tab_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut tab_bar = div()
            .flex()
            .items_center()
            .w_full()
            .h(px(38.0))
            .bg(ShellDeckColors::bg_sidebar())
            .border_b_1()
            .border_color(ShellDeckColors::border())
            .px(px(4.0))
            .gap(px(1.0))
            .id("terminal-tab-bar")
            .overflow_x_scroll();

        for tab in &self.tabs {
            let tab_id = tab.id;
            let is_active = tab.is_active;
            let state_color = match &tab.state {
                SessionState::Running => ShellDeckColors::success(),
                SessionState::Exited(0) => ShellDeckColors::text_muted(),
                SessionState::Exited(_) => ShellDeckColors::warning(),
                SessionState::Error(_) => ShellDeckColors::error(),
            };

            // Tab outer container — visual styling only, no click handler
            let group_name = SharedString::from(format!("tab-group-{}", tab_id));
            let mut tab_el = div()
                .group(group_name.clone())
                .flex()
                .items_center()
                .flex_shrink_0()
                .rounded_t(px(6.0));

            if is_active {
                tab_el = tab_el
                    .bg(ShellDeckColors::terminal_bg())
                    .border_1()
                    .border_b_0()
                    .border_color(ShellDeckColors::border())
                    .text_color(ShellDeckColors::text_primary());
            } else {
                tab_el = tab_el
                    .text_color(ShellDeckColors::text_muted())
                    .hover(|el| el.bg(ShellDeckColors::hover_bg()).rounded_t(px(6.0)));
            }

            // Truncate long titles in Rust (GPUI text truncation unreliable)
            let max_chars = 28;
            let display_title = if tab.title.chars().count() > max_chars {
                let truncated: String = tab.title.chars().take(max_chars).collect();
                format!("{}\u{2026}", truncated) // ellipsis
            } else {
                tab.title.clone()
            };

            // Clickable content area (dot + title) — selects the tab
            let mut tab_content = div()
                .id(ElementId::from(SharedString::from(format!(
                    "tab-{}",
                    tab_id
                ))))
                .flex()
                .items_center()
                .gap(px(6.0))
                .px(px(10.0))
                .py(px(5.0))
                .cursor_pointer()
                .on_click(cx.listener(move |this, _event: &ClickEvent, _window, cx| {
                    this.select_tab(tab_id);
                    cx.emit(TerminalEvent::TabSelected(tab_id));
                    cx.notify();
                }))
                // Status dot
                .child(
                    div()
                        .w(px(6.0))
                        .h(px(6.0))
                        .rounded_full()
                        .bg(state_color)
                        .flex_shrink_0(),
                );

            // Split indicator — small icon next to the tab's status dot
            let tab_has_split = if is_active {
                self.split_view.is_some()
            } else {
                self.stored_splits.contains_key(&tab_id)
            };
            if tab_has_split {
                tab_content = tab_content.child(
                    div()
                        .text_size(px(10.0))
                        .text_color(ShellDeckColors::primary())
                        .flex_shrink_0()
                        .child("\u{2ABF}"), // ⫿ vertical line with horizontal stroke
                );
            }

            // Title
            let tab_content = tab_content.child(
                div()
                    .text_size(px(12.0))
                    .whitespace_nowrap()
                    .child(display_title),
            );

            tab_el = tab_el.child(tab_content);

            // Close button — separate from content, no click overlap
            let close_btn = div()
                .id(ElementId::from(SharedString::from(format!(
                    "close-tab-{}",
                    tab_id
                ))))
                .flex()
                .items_center()
                .justify_center()
                .w(px(16.0))
                .h(px(16.0))
                .mr(px(4.0))
                .rounded(px(4.0))
                .text_size(px(10.0))
                .text_color(ShellDeckColors::text_muted())
                .cursor_pointer()
                .hover(|el| {
                    el.bg(ShellDeckColors::badge_bg())
                        .text_color(ShellDeckColors::error())
                })
                .child("x")
                .on_click(cx.listener(move |this, _event: &ClickEvent, _window, cx| {
                    this.close_tab(tab_id);
                    cx.emit(TerminalEvent::TabClosed(tab_id));
                    cx.notify();
                }));

            if is_active {
                // Always show close on active tab
                tab_el = tab_el.child(close_btn);
            } else {
                // Show close only on group hover for inactive tabs
                tab_el = tab_el.child(
                    div()
                        .opacity(0.0)
                        .group_hover(group_name, |style| style.opacity(1.0))
                        .child(close_btn),
                );
            }

            tab_bar = tab_bar.child(tab_el);
        }

        // New tab button
        tab_bar = tab_bar.child(
            div()
                .id("new-tab-btn")
                .flex()
                .items_center()
                .justify_center()
                .w(px(28.0))
                .h(px(28.0))
                .ml(px(4.0))
                .cursor_pointer()
                .rounded(px(6.0))
                .text_size(px(14.0))
                .text_color(ShellDeckColors::text_muted())
                .hover(|el| {
                    el.bg(ShellDeckColors::hover_bg())
                        .text_color(ShellDeckColors::text_primary())
                })
                .child("+")
                .on_click(cx.listener(|this, _event: &ClickEvent, window, cx| {
                    this.spawn_local_terminal(cx);
                    this.focus_handle.focus(window);
                    cx.emit(TerminalEvent::NewTabRequested);
                    cx.notify();
                })),
        );

        tab_bar
    }

    /// Render the toolbar with action buttons between tab bar and terminal grid.
    fn render_toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let zoom = self
            .tabs
            .get(self.pane.active_index)
            .map(|t| t.zoom_level)
            .unwrap_or(1.0);
        let zoom_pct = format!("{}%", (zoom * 100.0).round() as u32);

        let has_selection = self
            .active_session()
            .is_some_and(|s| s.grid.lock().selected_text().is_some());

        let toolbar_btn = |id: &str, label: &str, hint: &str| {
            div()
                .id(ElementId::from(SharedString::from(id.to_string())))
                .flex()
                .items_center()
                .gap(px(4.0))
                .px(px(8.0))
                .py(px(3.0))
                .rounded(px(4.0))
                .text_size(px(11.0))
                .text_color(ShellDeckColors::text_muted())
                .cursor_pointer()
                .hover(|el| {
                    el.bg(ShellDeckColors::hover_bg())
                        .text_color(ShellDeckColors::text_primary())
                })
                .child(label.to_string())
                .child(
                    div()
                        .text_size(px(9.0))
                        .text_color(ShellDeckColors::text_muted())
                        .child(hint.to_string()),
                )
        };

        let toolbar_icon = |id: &str, label: &str| {
            div()
                .id(ElementId::from(SharedString::from(id.to_string())))
                .flex()
                .items_center()
                .justify_center()
                .w(px(28.0))
                .h(px(24.0))
                .rounded(px(4.0))
                .text_size(px(12.0))
                .text_color(ShellDeckColors::text_muted())
                .cursor_pointer()
                .hover(|el| {
                    el.bg(ShellDeckColors::hover_bg())
                        .text_color(ShellDeckColors::text_primary())
                })
                .child(label.to_string())
        };

        let (ctrl, secondary) = if cfg!(target_os = "macos") {
            ("\u{2318}", "\u{2318}")
        } else {
            ("Ctrl+", "Ctrl+")
        };
        let shift = if cfg!(target_os = "macos") {
            "\u{21E7}"
        } else {
            "Shift+"
        };

        let mut toolbar = div()
            .flex()
            .items_center()
            .w_full()
            .h(px(32.0))
            .px(px(8.0))
            .gap(px(2.0))
            .bg(ShellDeckColors::bg_sidebar())
            .border_b_1()
            .border_color(ShellDeckColors::border());

        // Left group: search, copy, paste
        toolbar = toolbar
            .child(
                toolbar_btn("tb-search", "Search", &format!("{}F", secondary)).on_click(
                    cx.listener(|this, _, _, cx| {
                        this.toggle_search();
                        cx.notify();
                    }),
                ),
            )
            .child({
                let mut btn = toolbar_btn("tb-copy", "Copy", &format!("{}{}C", ctrl, shift));
                if !has_selection {
                    btn = btn
                        .text_color(ShellDeckColors::text_muted())
                        .cursor_default();
                } else {
                    btn = btn.on_click(cx.listener(|this, _, _, cx| {
                        this.copy_selection(cx);
                        cx.notify();
                    }));
                }
                btn
            })
            .child(
                toolbar_btn("tb-paste", "Paste", &format!("{}{}V", ctrl, shift)).on_click(
                    cx.listener(|this, _, _, cx| {
                        this.paste_clipboard(cx);
                        cx.notify();
                    }),
                ),
            );

        // Separator
        toolbar = toolbar.child(
            div()
                .w(px(1.0))
                .h(px(16.0))
                .mx(px(6.0))
                .bg(ShellDeckColors::border()),
        );

        // Middle group: split
        if self.split_view.is_some() {
            toolbar = toolbar
                .child(
                    toolbar_btn("tb-rotate-split", "Rotate", "").on_click(cx.listener(
                        |this, _, window, cx| {
                            this.toggle_split_direction();
                            this.resize_if_needed(window);
                            cx.notify();
                        },
                    )),
                )
                .child(
                    toolbar_btn("tb-close-split", "Close Split", "").on_click(cx.listener(
                        |this, _, _, cx| {
                            this.close_split();
                            cx.notify();
                        },
                    )),
                );
        } else {
            toolbar = toolbar
                .child(
                    toolbar_btn("tb-split-h", "Split H", &format!("{}{}D", ctrl, shift)).on_click(
                        cx.listener(|this, _, _, cx| {
                            this.split_horizontal(cx);
                        }),
                    ),
                )
                .child(
                    toolbar_btn("tb-split-v", "Split V", "").on_click(cx.listener(
                        |this, _, _, cx| {
                            this.split_vertical(cx);
                        },
                    )),
                );
        }

        // Separator
        toolbar = toolbar.child(
            div()
                .w(px(1.0))
                .h(px(16.0))
                .mx(px(6.0))
                .bg(ShellDeckColors::border()),
        );

        // Right group: zoom controls
        toolbar = toolbar
            .child(
                toolbar_icon("tb-zoom-out", "-").on_click(cx.listener(|this, _, window, cx| {
                    this.zoom_out();
                    this.resize_if_needed(window);
                    cx.notify();
                })),
            )
            .child(
                div()
                    .id("tb-zoom-level")
                    .flex()
                    .items_center()
                    .justify_center()
                    .min_w(px(42.0))
                    .h(px(24.0))
                    .rounded(px(4.0))
                    .text_size(px(11.0))
                    .text_color(if (zoom - 1.0).abs() < 0.01 {
                        ShellDeckColors::text_muted()
                    } else {
                        ShellDeckColors::primary()
                    })
                    .cursor_pointer()
                    .hover(|el| el.bg(ShellDeckColors::hover_bg()))
                    .child(zoom_pct)
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.zoom_reset();
                        this.resize_if_needed(window);
                        cx.notify();
                    })),
            )
            .child(
                toolbar_icon("tb-zoom-in", "+").on_click(cx.listener(|this, _, window, cx| {
                    this.zoom_in();
                    this.resize_if_needed(window);
                    cx.notify();
                })),
            );

        // Separator before scripts
        let has_scripts = !self.favorite_scripts.is_empty() || !self.recent_scripts.is_empty();
        if has_scripts {
            toolbar = toolbar.child(
                div()
                    .w(px(1.0))
                    .h(px(16.0))
                    .mx(px(6.0))
                    .bg(ShellDeckColors::border()),
            );

            // Scripts dropdown button + panel
            let is_open = self.script_dropdown_open;
            let mut scripts_wrapper =
                div()
                    .relative()
                    .child(
                        toolbar_btn("tb-scripts", "Scripts", "").on_click(cx.listener(
                            |this, _, _, cx| {
                                this.script_dropdown_open = !this.script_dropdown_open;
                                cx.notify();
                            },
                        )),
                    );

            if is_open {
                let mut dropdown = div()
                    .id("scripts-dropdown")
                    .absolute()
                    .top(px(30.0))
                    .left_0()
                    .w(px(220.0))
                    .bg(ShellDeckColors::bg_surface())
                    .border_1()
                    .border_color(ShellDeckColors::border())
                    .rounded(px(8.0))
                    .shadow_lg()
                    .py(px(4.0))
                    .flex()
                    .flex_col()
                    .overflow_y_scroll();

                // Favorites section
                if !self.favorite_scripts.is_empty() {
                    dropdown = dropdown.child(
                        div()
                            .px(px(10.0))
                            .py(px(4.0))
                            .text_size(px(10.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(ShellDeckColors::text_muted())
                            .child("FAVORITES"),
                    );
                    for (id, name, lang_badge) in &self.favorite_scripts {
                        let script_id = *id;
                        let name = name.clone();
                        let badge = lang_badge.clone();
                        dropdown = dropdown.child(
                            div()
                                .id(ElementId::from(SharedString::from(format!(
                                    "fav-{}",
                                    script_id
                                ))))
                                .flex()
                                .items_center()
                                .gap(px(6.0))
                                .px(px(10.0))
                                .py(px(5.0))
                                .cursor_pointer()
                                .hover(|el| el.bg(ShellDeckColors::hover_bg()))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.script_dropdown_open = false;
                                    cx.emit(TerminalEvent::RunScriptRequested(script_id));
                                    cx.notify();
                                }))
                                .child(
                                    div()
                                        .text_size(px(9.0))
                                        .px(px(3.0))
                                        .py(px(1.0))
                                        .rounded(px(3.0))
                                        .bg(ShellDeckColors::primary().opacity(0.15))
                                        .text_color(ShellDeckColors::primary())
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .child(badge),
                                )
                                .child(
                                    div()
                                        .text_size(px(11.0))
                                        .text_color(ShellDeckColors::text_primary())
                                        .child(name),
                                ),
                        );
                    }
                }

                // Separator between sections
                if !self.favorite_scripts.is_empty() && !self.recent_scripts.is_empty() {
                    dropdown = dropdown.child(
                        div()
                            .h(px(1.0))
                            .mx(px(8.0))
                            .my(px(4.0))
                            .bg(ShellDeckColors::border()),
                    );
                }

                // Recent section
                if !self.recent_scripts.is_empty() {
                    dropdown = dropdown.child(
                        div()
                            .px(px(10.0))
                            .py(px(4.0))
                            .text_size(px(10.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(ShellDeckColors::text_muted())
                            .child("RECENT"),
                    );
                    for (id, name, lang_badge) in &self.recent_scripts {
                        let script_id = *id;
                        let name = name.clone();
                        let badge = lang_badge.clone();
                        dropdown = dropdown.child(
                            div()
                                .id(ElementId::from(SharedString::from(format!(
                                    "rec-{}",
                                    script_id
                                ))))
                                .flex()
                                .items_center()
                                .gap(px(6.0))
                                .px(px(10.0))
                                .py(px(5.0))
                                .cursor_pointer()
                                .hover(|el| el.bg(ShellDeckColors::hover_bg()))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.script_dropdown_open = false;
                                    cx.emit(TerminalEvent::RunScriptRequested(script_id));
                                    cx.notify();
                                }))
                                .child(
                                    div()
                                        .text_size(px(9.0))
                                        .px(px(3.0))
                                        .py(px(1.0))
                                        .rounded(px(3.0))
                                        .bg(ShellDeckColors::primary().opacity(0.15))
                                        .text_color(ShellDeckColors::primary())
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .child(badge),
                                )
                                .child(
                                    div()
                                        .text_size(px(11.0))
                                        .text_color(ShellDeckColors::text_primary())
                                        .child(name),
                                ),
                        );
                    }
                }

                scripts_wrapper = scripts_wrapper.child(dropdown);
            }

            toolbar = toolbar.child(scripts_wrapper);
        }

        // Spacer
        toolbar = toolbar.child(div().flex_grow());

        // Right-aligned: clear terminal
        toolbar = toolbar.child(
            toolbar_btn("tb-clear", "Clear", &format!("{}L", secondary)).on_click(cx.listener(
                |this, _, _, cx| {
                    if let Some(session) = this.active_session() {
                        let mut grid = session.grid.lock();
                        grid.erase_display(2);
                        grid.cursor_to(0, 0);
                    }
                    cx.notify();
                },
            )),
        );

        toolbar
    }

    // -----------------------------------------------------------------------
    // Terminal grid – direct glyph painting via canvas
    // -----------------------------------------------------------------------

    fn render_terminal_grid(
        &self,
        mouse_mode: MouseMode,
        mouse_encoding: MouseEncoding,
        cursor: CursorState,
        cache: Arc<GlyphCache>,
        grid: Arc<Mutex<TerminalGrid>>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let cell_height_f = self.font_size * 1.4;

        // -- event handler closures --
        let handle = cx.entity().downgrade();
        let focus = self.focus_handle.clone();

        // Left mouse down: selection or terminal mouse mode
        let h_down = handle.clone();
        let mouse_down_handler =
            move |event: &MouseDownEvent, window: &mut Window, cx: &mut App| {
                focus.focus(window);
                if mouse_mode != MouseMode::None {
                    if let Some(view) = h_down.upgrade() {
                        view.update(cx, |this, _cx| {
                            if let Some((col, row)) = this.pixel_to_cell(event.position) {
                                let bytes = Self::encode_mouse(mouse_encoding, 0, col, row, true);
                                if let Some(s) = this.active_session() {
                                    s.write_input(&bytes);
                                }
                            }
                        });
                    }
                } else {
                    // Check for scrollbar click first
                    if let Some(view) = h_down.upgrade() {
                        let is_scrollbar =
                            view.read(cx).is_in_scrollbar_area(event.position, window);
                        if is_scrollbar {
                            view.update(cx, |this, cx| {
                                this.scrollbar_dragging = true;
                                this.scrollbar_scroll_to_y(event.position.y, window);
                                cx.notify();
                            });
                            return;
                        }
                    }

                    // Check for Ctrl+Click on URLs
                    let ctrl_held = event.modifiers.control || event.modifiers.secondary();
                    if ctrl_held {
                        if let Some(view) = h_down.upgrade() {
                            view.update(cx, |this, _cx| {
                                if let Some((col, row)) = this.pixel_to_cell_zero(event.position) {
                                    // Find URL at this position
                                    if let Some(url_match) = this.detected_urls.iter().find(|u| {
                                        u.row == row && col >= u.col && col < u.col + u.len
                                    }) {
                                        let url = url_match.url.clone();
                                        let _ = open::that(&url);
                                    }
                                }
                            });
                        }
                        return;
                    }

                    // Selection mode: handle single/double/triple click
                    if let Some(view) = h_down.upgrade() {
                        view.update(cx, |this, cx| {
                            // Reset blink on click so cursor stays visible
                            this.reset_cursor_blink(cx);
                            // Dismiss context menu on any click
                            if this.context_menu.is_some() {
                                this.context_menu = None;
                                cx.notify();
                                return;
                            }
                            if let Some((col, row)) = this.pixel_to_cell_zero(event.position) {
                                let now = std::time::Instant::now();
                                let multi_click_threshold = std::time::Duration::from_millis(400);

                                // Detect multi-click
                                let is_multi = this
                                    .last_click_time
                                    .is_some_and(|t| now.duration_since(t) < multi_click_threshold)
                                    && this.last_click_pos == Some((col, row));

                                if is_multi {
                                    this.click_count = (this.click_count % 3) + 1;
                                } else {
                                    this.click_count = 1;
                                }
                                this.last_click_time = Some(now);
                                this.last_click_pos = Some((col, row));

                                if let Some(session) = this.active_session() {
                                    let mut grid = session.grid.lock();
                                    let alt_held = event.modifiers.alt;
                                    match this.click_count {
                                        1 => {
                                            grid.clear_selection();
                                            if alt_held {
                                                grid.start_block_selection(col, row);
                                            } else {
                                                grid.start_selection(col, row);
                                            }
                                        }
                                        2 => grid.start_word_selection(col, row),
                                        3 => grid.start_line_selection(col, row),
                                        _ => {}
                                    }
                                }
                                this.selecting = true;
                                cx.notify();
                            }
                        });
                    }
                }
            };

        // Left mouse up
        let h_up = handle.clone();
        let mouse_up_handler = move |event: &MouseUpEvent, _window: &mut Window, cx: &mut App| {
            if mouse_mode != MouseMode::None {
                if let Some(view) = h_up.upgrade() {
                    view.update(cx, |this, _cx| {
                        if let Some((col, row)) = this.pixel_to_cell(event.position) {
                            let bytes = Self::encode_mouse(mouse_encoding, 0, col, row, false);
                            if let Some(s) = this.active_session() {
                                s.write_input(&bytes);
                            }
                        }
                    });
                }
            } else {
                // End scrollbar drag or selection drag
                if let Some(view) = h_up.upgrade() {
                    view.update(cx, |this, _cx| {
                        this.scrollbar_dragging = false;
                        this.selecting = false;
                        if let Some(session) = this.active_session() {
                            session.grid.lock().end_selection();
                        }
                    });
                }
            }
        };

        // Mouse move: selection drag or terminal mouse reporting
        let h_move = handle.clone();
        let mouse_move_handler =
            move |event: &MouseMoveEvent, window: &mut Window, cx: &mut App| {
                if mouse_mode != MouseMode::None {
                    let should_report = match mouse_mode {
                        MouseMode::AnyMotion => true,
                        MouseMode::ButtonTracking => event.pressed_button.is_some(),
                        _ => false,
                    };
                    if should_report {
                        if let Some(view) = h_move.upgrade() {
                            view.update(cx, |this, _cx| {
                                if let Some((col, row)) = this.pixel_to_cell(event.position) {
                                    let button = match event.pressed_button {
                                        Some(MouseButton::Left) => 32u8,
                                        Some(MouseButton::Middle) => 33u8,
                                        Some(MouseButton::Right) => 34u8,
                                        _ => 35u8,
                                    };
                                    let bytes =
                                        Self::encode_mouse(mouse_encoding, button, col, row, true);
                                    if let Some(s) = this.active_session() {
                                        s.write_input(&bytes);
                                    }
                                }
                            });
                        }
                    }
                } else {
                    // Scrollbar drag or selection drag
                    if let Some(view) = h_move.upgrade() {
                        view.update(cx, |this, cx| {
                            if this.scrollbar_dragging {
                                this.scrollbar_scroll_to_y(event.position.y, window);
                                cx.notify();
                            } else if this.selecting {
                                if let Some((col, row)) = this.pixel_to_cell_zero(event.position) {
                                    if let Some(session) = this.active_session() {
                                        session.grid.lock().update_selection(col, row);
                                    }
                                    cx.notify();
                                }
                            }
                        });
                    }
                }
            };

        // Scroll wheel: scrollback or terminal mouse reporting
        let h_scroll = handle.clone();
        let scroll_handler = move |event: &ScrollWheelEvent, _window: &mut Window, cx: &mut App| {
            if mouse_mode != MouseMode::None {
                if let Some(view) = h_scroll.upgrade() {
                    view.update(cx, |this, _cx| {
                        if let Some((col, row)) = this.pixel_to_cell(event.position) {
                            let delta_y = match event.delta {
                                ScrollDelta::Lines(pt) => pt.y,
                                ScrollDelta::Pixels(pt) => pt.y / px(cell_height_f),
                            };
                            let button = if delta_y < 0.0 { 64u8 } else { 65u8 };
                            let bytes = Self::encode_mouse(mouse_encoding, button, col, row, true);
                            if let Some(s) = this.active_session() {
                                s.write_input(&bytes);
                            }
                        }
                    });
                }
            } else {
                // Scrollback navigation
                if let Some(view) = h_scroll.upgrade() {
                    view.update(cx, |this, cx| {
                        let delta_y = match event.delta {
                            ScrollDelta::Lines(pt) => pt.y,
                            ScrollDelta::Pixels(pt) => pt.y / px(cell_height_f),
                        };
                        let lines = delta_y.abs().ceil() as usize;
                        if let Some(session) = this.active_session() {
                            let mut grid = session.grid.lock();
                            if delta_y < 0.0 {
                                grid.scroll_view_up(lines);
                            } else {
                                grid.scroll_view_down(lines);
                            }
                        }
                        cx.notify();
                    });
                }
            }
        };

        let focus2 = self.focus_handle.clone();
        let focus3 = self.focus_handle.clone();
        let h_down2 = cx.entity().downgrade();
        let h_down3 = cx.entity().downgrade();
        let h_up2 = cx.entity().downgrade();
        let h_up3 = cx.entity().downgrade();

        // -- build the grid element --
        let grid_el = div()
            .id("terminal-grid")
            .track_focus(&self.focus_handle)
            .on_mouse_down(MouseButton::Left, mouse_down_handler)
            .on_mouse_down(
                MouseButton::Right,
                move |event: &MouseDownEvent, window: &mut Window, cx: &mut App| {
                    focus2.focus(window);
                    if mouse_mode != MouseMode::None {
                        if let Some(view) = h_down2.upgrade() {
                            view.update(cx, |this, _cx| {
                                if let Some((col, row)) = this.pixel_to_cell(event.position) {
                                    let bytes =
                                        Self::encode_mouse(mouse_encoding, 2, col, row, true);
                                    if let Some(s) = this.active_session() {
                                        s.write_input(&bytes);
                                    }
                                }
                            });
                        }
                    } else {
                        // Show context menu
                        if let Some(view) = h_down2.upgrade() {
                            view.update(cx, |this, cx| {
                                // Check if right-clicking on a URL
                                let url = this.pixel_to_cell_zero(event.position).and_then(
                                    |(col, row)| {
                                        this.detected_urls
                                            .iter()
                                            .find(|u| {
                                                u.row == row && col >= u.col && col < u.col + u.len
                                            })
                                            .map(|u| u.url.clone())
                                    },
                                );
                                this.context_menu = Some(ContextMenuState {
                                    position: event.position,
                                    url,
                                });
                                cx.notify();
                            });
                        }
                    }
                },
            )
            .on_mouse_down(
                MouseButton::Middle,
                move |event: &MouseDownEvent, window: &mut Window, cx: &mut App| {
                    focus3.focus(window);
                    if mouse_mode != MouseMode::None {
                        if let Some(view) = h_down3.upgrade() {
                            view.update(cx, |this, _cx| {
                                if let Some((col, row)) = this.pixel_to_cell(event.position) {
                                    let bytes =
                                        Self::encode_mouse(mouse_encoding, 1, col, row, true);
                                    if let Some(s) = this.active_session() {
                                        s.write_input(&bytes);
                                    }
                                }
                            });
                        }
                    }
                },
            )
            .on_mouse_up(MouseButton::Left, mouse_up_handler)
            .on_mouse_up(
                MouseButton::Right,
                move |event: &MouseUpEvent, _window: &mut Window, cx: &mut App| {
                    if mouse_mode != MouseMode::None {
                        if let Some(view) = h_up2.upgrade() {
                            view.update(cx, |this, _cx| {
                                if let Some((col, row)) = this.pixel_to_cell(event.position) {
                                    let bytes =
                                        Self::encode_mouse(mouse_encoding, 2, col, row, false);
                                    if let Some(s) = this.active_session() {
                                        s.write_input(&bytes);
                                    }
                                }
                            });
                        }
                    }
                },
            )
            .on_mouse_up(
                MouseButton::Middle,
                move |event: &MouseUpEvent, _window: &mut Window, cx: &mut App| {
                    if mouse_mode != MouseMode::None {
                        if let Some(view) = h_up3.upgrade() {
                            view.update(cx, |this, _cx| {
                                if let Some((col, row)) = this.pixel_to_cell(event.position) {
                                    let bytes =
                                        Self::encode_mouse(mouse_encoding, 1, col, row, false);
                                    if let Some(s) = this.active_session() {
                                        s.write_input(&bytes);
                                    }
                                }
                            });
                        }
                    }
                },
            )
            .on_mouse_move(mouse_move_handler)
            .on_scroll_wheel(scroll_handler)
            .on_key_down(move |event: &KeyDownEvent, _window, cx| {
                if let Some(view) = handle.upgrade() {
                    view.update(cx, |this, cx| {
                        // When search is visible, intercept keystrokes for the search bar
                        if this.search_visible {
                            let key = event.keystroke.key.as_str();
                            match key {
                                "escape" => {
                                    this.toggle_search();
                                    cx.notify();
                                }
                                "enter" => {
                                    if event.keystroke.modifiers.shift {
                                        this.search_prev();
                                    } else {
                                        this.search_next();
                                    }
                                    cx.notify();
                                }
                                "backspace" => {
                                    this.search_query.pop();
                                    this.update_search();
                                    cx.notify();
                                }
                                _ => {
                                    // Type into search query
                                    if let Some(ref kc) = event.keystroke.key_char {
                                        if !event.keystroke.modifiers.control
                                            && !event.keystroke.modifiers.alt
                                        {
                                            this.search_query.push_str(kc);
                                            this.update_search();
                                            cx.notify();
                                        }
                                    } else if key.len() == 1
                                        && !event.keystroke.modifiers.control
                                        && !event.keystroke.modifiers.alt
                                    {
                                        this.search_query.push_str(key);
                                        this.update_search();
                                        cx.notify();
                                    }
                                }
                            }
                            return;
                        }

                        // Normal terminal input
                        let app_cursor = this
                            .active_session()
                            .map(|s| s.grid.lock().application_cursor_keys())
                            .unwrap_or(false);
                        if let Some(bytes) = TerminalView::keystroke_to_bytes(event, app_cursor) {
                            // Clear selection on typing
                            if let Some(session) = this.active_session() {
                                session.grid.lock().clear_selection();
                            }
                            if let Some(s) = this.active_session() {
                                s.write_input(&bytes);
                            }
                            // Reset blink so cursor stays visible during typing
                            this.reset_cursor_blink(cx);
                        }
                    });
                }
            })
            .on_action({
                let h = cx.entity().downgrade();
                move |_: &CopySelection, _window: &mut Window, cx: &mut App| {
                    if let Some(view) = h.upgrade() {
                        view.update(cx, |this, cx| {
                            this.copy_selection(cx);
                        });
                    }
                }
            })
            .on_action({
                let h = cx.entity().downgrade();
                move |_: &PasteClipboard, _window: &mut Window, cx: &mut App| {
                    if let Some(view) = h.upgrade() {
                        view.update(cx, |this, cx| {
                            this.paste_clipboard(cx);
                        });
                    }
                }
            })
            .on_action({
                let h = cx.entity().downgrade();
                move |_: &ToggleSearch, _window: &mut Window, cx: &mut App| {
                    if let Some(view) = h.upgrade() {
                        view.update(cx, |this, cx| {
                            this.toggle_search();
                            cx.notify();
                        });
                    }
                }
            })
            .on_action({
                let h = cx.entity().downgrade();
                move |_: &SearchNext, _window: &mut Window, cx: &mut App| {
                    if let Some(view) = h.upgrade() {
                        view.update(cx, |this, cx| {
                            this.search_next();
                            cx.notify();
                        });
                    }
                }
            })
            .on_action({
                let h = cx.entity().downgrade();
                move |_: &SearchPrev, _window: &mut Window, cx: &mut App| {
                    if let Some(view) = h.upgrade() {
                        view.update(cx, |this, cx| {
                            this.search_prev();
                            cx.notify();
                        });
                    }
                }
            })
            .on_action({
                let h = cx.entity().downgrade();
                move |_: &ClearTerminal, _window: &mut Window, cx: &mut App| {
                    if let Some(view) = h.upgrade() {
                        view.update(cx, |this, _cx| {
                            if let Some(session) = this.active_session() {
                                let mut grid = session.grid.lock();
                                grid.erase_display(2);
                                grid.cursor_to(0, 0);
                            }
                        });
                    }
                }
            })
            .on_action({
                let h = cx.entity().downgrade();
                move |_: &SplitHorizontal, _window: &mut Window, cx: &mut App| {
                    if let Some(view) = h.upgrade() {
                        view.update(cx, |this, cx| {
                            this.split_horizontal(cx);
                        });
                    }
                }
            })
            .on_action({
                let h = cx.entity().downgrade();
                move |_: &SplitVertical, _window: &mut Window, cx: &mut App| {
                    if let Some(view) = h.upgrade() {
                        view.update(cx, |this, cx| {
                            this.split_vertical(cx);
                        });
                    }
                }
            })
            .on_action({
                let h = cx.entity().downgrade();
                move |_: &ZoomIn, _window: &mut Window, cx: &mut App| {
                    if let Some(view) = h.upgrade() {
                        view.update(cx, |this, cx| {
                            this.zoom_in();
                            this.resize_if_needed(_window);
                            cx.notify();
                        });
                    }
                }
            })
            .on_action({
                let h = cx.entity().downgrade();
                move |_: &ZoomOut, _window: &mut Window, cx: &mut App| {
                    if let Some(view) = h.upgrade() {
                        view.update(cx, |this, cx| {
                            this.zoom_out();
                            this.resize_if_needed(_window);
                            cx.notify();
                        });
                    }
                }
            })
            .on_action({
                let h = cx.entity().downgrade();
                move |_: &ZoomReset, _window: &mut Window, cx: &mut App| {
                    if let Some(view) = h.upgrade() {
                        view.update(cx, |this, cx| {
                            this.zoom_reset();
                            this.resize_if_needed(_window);
                            cx.notify();
                        });
                    }
                }
            })
            .on_action({
                let h = cx.entity().downgrade();
                move |_: &ToggleSplitFocus, _window: &mut Window, cx: &mut App| {
                    if let Some(view) = h.upgrade() {
                        view.update(cx, |this, cx| {
                            this.toggle_split_focus();
                            cx.notify();
                        });
                    }
                }
            })
            .size_full()
            .bg(ShellDeckColors::terminal_bg())
            .p(px(4.0))
            .overflow_hidden()
            // Direct glyph-painting canvas
            .child(Self::create_grid_canvas(
                cache,
                grid,
                cursor,
                self.search_matches.clone(),
                self.search_current_idx,
                self.detected_urls.clone(),
                self.palette.clone(),
                self.has_focus,
                self.cursor_blink_on,
            ));

        grid_el
    }

    /// Create a grid canvas element that paints cells, cursor, search matches,
    /// URL underlines and scrollbar.  Shared by both the active and passive
    /// split panes.
    #[allow(clippy::too_many_arguments)]
    fn create_grid_canvas(
        cache: Arc<GlyphCache>,
        grid: Arc<Mutex<TerminalGrid>>,
        cursor: CursorState,
        search_matches: Vec<SearchMatch>,
        search_current: Option<usize>,
        url_matches: Vec<UrlMatch>,
        palette: TerminalPalette,
        has_focus: bool,
        cursor_blink_on: bool,
    ) -> impl IntoElement {
        canvas(
            move |_bounds, _window, _cx| {
                (
                    cache,
                    grid,
                    cursor,
                    search_matches,
                    search_current,
                    url_matches,
                    palette,
                    has_focus,
                    cursor_blink_on,
                )
            },
            move |bounds,
                  (
                cache,
                grid,
                cursor,
                search_matches,
                search_current,
                url_matches,
                palette,
                has_focus,
                cursor_blink_on,
            ),
                  window,
                  _cx| {
                let cell_w = cache.cell_width;
                let cell_h = cache.cell_height;
                let baseline = cache.baseline_y;
                let fs = cache.font_size;

                let grid = grid.lock();
                let visible = grid.visible_rows();

                let sel_color = hsla(0.58, 0.6, 0.5, 0.35);
                let search_color = hsla(0.0, 0.0, 0.5, 0.3);
                let search_current_color = hsla(0.1, 0.9, 0.5, 0.7);

                for (ri, row) in visible.iter().enumerate() {
                    let y = bounds.origin.y + cell_h * ri as f32;

                    for (ci, cell) in row.iter().enumerate() {
                        let x = bounds.origin.x + cell_w * ci as f32;

                        let inverse = cell.attrs.inverse;
                        let (mut fg_t, bg_t) = if inverse {
                            (cell.bg, cell.fg)
                        } else {
                            (cell.fg, cell.bg)
                        };

                        if cell.attrs.bold {
                            fg_t = brighten_for_bold(fg_t);
                        }

                        let eff_fg = if cell.attrs.hidden {
                            bg_t
                        } else if cell.attrs.dim {
                            dim_color(fg_t)
                        } else {
                            fg_t
                        };

                        // Background: always paint for every cell (including spacers)
                        if bg_t != TermColor::Default || inverse {
                            window.paint_quad(fill(
                                Bounds::new(point(x, y), size(cell_w, cell_h)),
                                palette.resolve(&bg_t, inverse),
                            ));
                        }

                        if grid.is_selected(ci, ri) {
                            window.paint_quad(fill(
                                Bounds::new(point(x, y), size(cell_w, cell_h)),
                                sel_color,
                            ));
                        }

                        // Skip glyph rendering for Spacer cells -- the Wide cell's
                        // glyph already covers both columns.
                        if cell.wide == CellWidth::Spacer {
                            continue;
                        }

                        let fg_color = palette.resolve(&eff_fg, !inverse);

                        // Determine the rendering width: wide chars span 2 cell widths.
                        let glyph_w = if cell.wide == CellWidth::Wide {
                            cell_w * 2.0
                        } else {
                            cell_w
                        };

                        let ch = cell.c;
                        if ch != ' ' && ch != '\0' {
                            if paint_block_char(ch, x, y, glyph_w, cell_h, fg_color, window) {
                                // Handled by procedural block/box-drawing renderer
                            } else {
                                let f = match (cell.attrs.bold, cell.attrs.italic) {
                                    (true, true) => cache.base_font.clone().bold().italic(),
                                    (true, false) => cache.base_font.clone().bold(),
                                    (false, true) => cache.base_font.clone().italic(),
                                    _ => cache.base_font.clone(),
                                };
                                // Build the string: base char + combining chars
                                let mut char_str = ch.to_string();
                                for &comb in &cell.combining {
                                    char_str.push(comb);
                                }
                                let s: SharedString = char_str.into();
                                let blen = s.len();
                                let shaped = window.text_system().shape_line(
                                    s,
                                    fs,
                                    &[TextRun {
                                        len: blen,
                                        font: f,
                                        color: fg_color,
                                        background_color: None,
                                        underline: None,
                                        strikethrough: None,
                                    }],
                                    None,
                                );
                                let _ = shaped.paint(point(x, y), cell_h, window, _cx);
                            }
                        }

                        // Underline color: use dedicated underline_color if set,
                        // otherwise fall back to the foreground color.
                        let ul_color = if let Some(ref uc) = cell.attrs.underline_color {
                            palette.resolve(uc, true)
                        } else {
                            fg_color
                        };

                        // Styled underline rendering
                        let underline_y = y + cell_h - px(2.0);
                        let gw_f32 = glyph_w.to_f64() as f32;
                        match cell.attrs.underline {
                            UnderlineStyle::None => {}
                            UnderlineStyle::Single => {
                                window.paint_quad(fill(
                                    Bounds::new(point(x, underline_y), size(glyph_w, px(1.0))),
                                    ul_color,
                                ));
                            }
                            UnderlineStyle::Double => {
                                // Two parallel lines 2px apart
                                window.paint_quad(fill(
                                    Bounds::new(
                                        point(x, underline_y - px(1.0)),
                                        size(glyph_w, px(1.0)),
                                    ),
                                    ul_color,
                                ));
                                window.paint_quad(fill(
                                    Bounds::new(
                                        point(x, underline_y + px(1.0)),
                                        size(glyph_w, px(1.0)),
                                    ),
                                    ul_color,
                                ));
                            }
                            UnderlineStyle::Curly => {
                                // Wavy/zigzag underline: small quads alternating y position
                                let wave_period = 4.0_f32;
                                let wave_height = 2.0_f32;
                                let mut xoff = 0.0_f32;
                                let mut segment_idx = 0;
                                while xoff < gw_f32 {
                                    let seg_w = wave_period.min(gw_f32 - xoff);
                                    let y_shift = if segment_idx % 2 == 0 {
                                        0.0
                                    } else {
                                        wave_height
                                    };
                                    window.paint_quad(fill(
                                        Bounds::new(
                                            point(x + px(xoff), underline_y + px(y_shift)),
                                            size(px(seg_w), px(1.0)),
                                        ),
                                        ul_color,
                                    ));
                                    xoff += wave_period;
                                    segment_idx += 1;
                                }
                            }
                            UnderlineStyle::Dotted => {
                                // Dots: 1px on, 1px off
                                let mut xoff = 0.0_f32;
                                while xoff < gw_f32 {
                                    let dot_w = 1.0_f32.min(gw_f32 - xoff);
                                    window.paint_quad(fill(
                                        Bounds::new(
                                            point(x + px(xoff), underline_y),
                                            size(px(dot_w), px(1.0)),
                                        ),
                                        ul_color,
                                    ));
                                    xoff += 2.0;
                                }
                            }
                            UnderlineStyle::Dashed => {
                                // Dashes: 3px on, 2px off
                                let mut xoff = 0.0_f32;
                                while xoff < gw_f32 {
                                    let dash_w = 3.0_f32.min(gw_f32 - xoff);
                                    window.paint_quad(fill(
                                        Bounds::new(
                                            point(x + px(xoff), underline_y),
                                            size(px(dash_w), px(1.0)),
                                        ),
                                        ul_color,
                                    ));
                                    xoff += 5.0;
                                }
                            }
                        }

                        // Overline: drawn at the top of the cell
                        if cell.attrs.overline {
                            window.paint_quad(fill(
                                Bounds::new(point(x, y + px(1.0)), size(glyph_w, px(1.0))),
                                fg_color,
                            ));
                        }

                        // Strikethrough: horizontal line at vertical center
                        if cell.attrs.strikethrough {
                            window.paint_quad(fill(
                                Bounds::new(point(x, y + cell_h / 2.0), size(glyph_w, px(1.0))),
                                fg_color,
                            ));
                        }
                    }
                }

                // URL underlines
                let url_underline_color = hsla(0.58, 0.6, 0.6, 0.6);
                for url in &url_matches {
                    let y = bounds.origin.y + cell_h * url.row as f32;
                    let underline_y = y + cell_h - px(1.0);
                    for offset in 0..url.len {
                        let col = url.col + offset;
                        let x = bounds.origin.x + cell_w * col as f32;
                        if offset % 2 == 0 {
                            window.paint_quad(fill(
                                Bounds::new(point(x, underline_y), size(cell_w, px(1.0))),
                                url_underline_color,
                            ));
                        }
                    }
                }

                // Search match highlights
                for (mi, m) in search_matches.iter().enumerate() {
                    let is_current = search_current == Some(mi);
                    let color = if is_current {
                        search_current_color
                    } else {
                        search_color
                    };
                    for offset in 0..m.len {
                        let col = m.col + offset;
                        let y = bounds.origin.y + cell_h * m.row as f32;
                        let x = bounds.origin.x + cell_w * col as f32;
                        window.paint_quad(fill(
                            Bounds::new(point(x, y), size(cell_w, cell_h)),
                            color,
                        ));
                    }
                }

                // Cursor
                // Determine whether the cursor should be drawn at all:
                // - Must be marked visible by the grid
                // - If the grid's blink flag is set, respect the blink timer phase
                let should_draw_cursor = cursor.visible
                    && cursor.row < visible.len()
                    && (!cursor.blink || cursor_blink_on);

                if should_draw_cursor {
                    let cx_x = bounds.origin.x + cell_w * cursor.col as f32;
                    let cx_y = bounds.origin.y + cell_h * cursor.row as f32;
                    let cursor_color = ShellDeckColors::text_primary();

                    // Determine cursor width: 2 cells for wide chars, 1 for normal.
                    let cursor_w = if let Some(row) = visible.get(cursor.row) {
                        if let Some(cell) = row.get(cursor.col) {
                            if cell.wide == CellWidth::Wide {
                                cell_w * 2.0
                            } else {
                                cell_w
                            }
                        } else {
                            cell_w
                        }
                    } else {
                        cell_w
                    };

                    if !has_focus && cursor.shape == CursorShape::Block {
                        // Hollow cursor: outline only (unfocused block)
                        let bw = px(1.0);
                        // Top edge
                        window.paint_quad(fill(
                            Bounds::new(point(cx_x, cx_y), size(cursor_w, bw)),
                            cursor_color,
                        ));
                        // Bottom edge
                        window.paint_quad(fill(
                            Bounds::new(point(cx_x, cx_y + cell_h - bw), size(cursor_w, bw)),
                            cursor_color,
                        ));
                        // Left edge
                        window.paint_quad(fill(
                            Bounds::new(point(cx_x, cx_y), size(bw, cell_h)),
                            cursor_color,
                        ));
                        // Right edge
                        window.paint_quad(fill(
                            Bounds::new(point(cx_x + cursor_w - bw, cx_y), size(bw, cell_h)),
                            cursor_color,
                        ));
                    } else {
                        // Focused cursor (or unfocused non-block shapes)
                        match cursor.shape {
                            CursorShape::Block => {
                                window.paint_quad(fill(
                                    Bounds::new(point(cx_x, cx_y), size(cursor_w, cell_h)),
                                    cursor_color,
                                ));
                                // Draw the character under the cursor in the background color
                                // so it remains readable against the filled block.
                                if let Some(row) = visible.get(cursor.row) {
                                    if let Some(cell) = row.get(cursor.col) {
                                        let ch = cell.c;
                                        if ch != ' ' && ch != '\0' {
                                            let bg = ShellDeckColors::terminal_bg();
                                            if let Some((fid, gid)) =
                                                cache.lookup(ch, cell.attrs.bold, cell.attrs.italic)
                                            {
                                                let _ = window.paint_glyph(
                                                    point(cx_x, cx_y + baseline),
                                                    fid,
                                                    gid,
                                                    fs,
                                                    bg,
                                                );
                                            } else {
                                                // Non-ASCII (e.g. CJK) under cursor: use shape_line
                                                let f = match (cell.attrs.bold, cell.attrs.italic) {
                                                    (true, true) => {
                                                        cache.base_font.clone().bold().italic()
                                                    }
                                                    (true, false) => cache.base_font.clone().bold(),
                                                    (false, true) => {
                                                        cache.base_font.clone().italic()
                                                    }
                                                    _ => cache.base_font.clone(),
                                                };
                                                let mut char_str = ch.to_string();
                                                for &comb in &cell.combining {
                                                    char_str.push(comb);
                                                }
                                                let s: SharedString = char_str.into();
                                                let blen = s.len();
                                                let shaped = window.text_system().shape_line(
                                                    s,
                                                    fs,
                                                    &[TextRun {
                                                        len: blen,
                                                        font: f,
                                                        color: bg,
                                                        background_color: None,
                                                        underline: None,
                                                        strikethrough: None,
                                                    }],
                                                    None,
                                                );
                                                let _ = shaped.paint(
                                                    point(cx_x, cx_y),
                                                    cell_h,
                                                    window,
                                                    _cx,
                                                );
                                            }
                                        }
                                    }
                                }
                            }
                            CursorShape::Bar => {
                                window.paint_quad(fill(
                                    Bounds::new(point(cx_x, cx_y), size(px(2.0), cell_h)),
                                    ShellDeckColors::primary(),
                                ));
                            }
                            CursorShape::Underline => {
                                window.paint_quad(fill(
                                    Bounds::new(
                                        point(cx_x, cx_y + cell_h - px(2.0)),
                                        size(cursor_w, px(2.0)),
                                    ),
                                    ShellDeckColors::primary(),
                                ));
                            }
                        }
                    }
                }

                // Scrollbar
                let (total_lines, visible_lines, scroll_off) = grid.scroll_info();
                if total_lines > visible_lines {
                    let scrollbar_w = px(SCROLLBAR_WIDTH);
                    // Use actual grid content height, not full canvas height
                    let grid_content_height = cell_h * visible_lines as f32;
                    let track_height = grid_content_height.min(bounds.size.height);
                    let track_x =
                        bounds.origin.x + bounds.size.width - scrollbar_w - px(SCROLLBAR_MARGIN);
                    let track_y = bounds.origin.y;

                    window.paint_quad(fill(
                        Bounds::new(point(track_x, track_y), size(scrollbar_w, track_height)),
                        hsla(0.0, 0.0, 0.2, 0.3),
                    ));

                    let ratio = visible_lines as f32 / total_lines as f32;
                    let thumb_height = (track_height * ratio).max(px(20.0));
                    let scrollable = total_lines - visible_lines;
                    let position = if scrollable > 0 {
                        1.0 - (scroll_off as f32 / scrollable as f32)
                    } else {
                        1.0
                    };
                    let thumb_y = track_y + (track_height - thumb_height) * position;

                    window.paint_quad(PaintQuad {
                        bounds: Bounds::new(
                            point(track_x, thumb_y),
                            size(scrollbar_w, thumb_height),
                        ),
                        corner_radii: Corners::all(px(3.0)),
                        background: hsla(0.0, 0.0, 0.5, 0.5).into(),
                        border_widths: Edges::default(),
                        border_color: transparent_black(),
                        border_style: BorderStyle::default(),
                        continuous_corners: false,
                    });
                }
            },
        )
        .size_full()
    }

    /// Convert a pixel position (window-relative) to grid cell coordinates.
    /// Returns (col, row) 1-indexed for terminal escape sequences.
    fn pixel_to_cell(&self, position: Point<Pixels>) -> Option<(u16, u16)> {
        let fs = self.effective_font_size();
        let cell_width = fs * 0.6;
        let cell_height = fs * 1.4;

        let grid_x = position.x - px(self.grid_x_offset);
        let grid_y = position.y - px(self.grid_y_offset);

        if grid_x < px(0.0) || grid_y < px(0.0) {
            return None;
        }

        let col = (grid_x / px(cell_width)).floor() as u16 + 1;
        let row = (grid_y / px(cell_height)).floor() as u16 + 1;

        if col < 1 || row < 1 || col > self.last_grid_cols || row > self.last_grid_rows {
            return None;
        }

        Some((col, row))
    }

    /// Convert a pixel position to 0-indexed grid cell coordinates (for selection).
    fn pixel_to_cell_zero(&self, position: Point<Pixels>) -> Option<(usize, usize)> {
        let fs = self.effective_font_size();
        let cell_width = fs * 0.6;
        let cell_height = fs * 1.4;

        let grid_x = position.x - px(self.grid_x_offset);
        let grid_y = position.y - px(self.grid_y_offset);

        if grid_x < px(0.0) || grid_y < px(0.0) {
            return None;
        }

        let col = (grid_x / px(cell_width)).floor() as usize;
        let row = (grid_y / px(cell_height)).floor() as usize;

        let max_col = (self.last_grid_cols as usize).saturating_sub(1);
        let max_row = (self.last_grid_rows as usize).saturating_sub(1);

        Some((col.min(max_col), row.min(max_row)))
    }

    /// Check if a window-relative position falls on the scrollbar track
    /// (rightmost ~10px of the grid area).
    fn is_in_scrollbar_area(&self, position: Point<Pixels>, window: &Window) -> bool {
        let viewport = window.viewport_size();
        let scrollbar_start = viewport.width - px(SCROLLBAR_WIDTH + SCROLLBAR_MARGIN + 2.0);
        position.x >= scrollbar_start && position.y >= px(self.grid_y_offset)
    }

    /// Scroll the active session's grid to a position based on the mouse y
    /// coordinate within the scrollbar track.
    fn scrollbar_scroll_to_y(&mut self, y: Pixels, window: &Window) {
        let viewport = window.viewport_size();
        let track_top = px(self.grid_y_offset);
        let track_height = viewport.height - track_top - px(STATUS_BAR_HEIGHT);
        if track_height <= px(0.0) {
            return;
        }

        let relative_y = (y - track_top).max(px(0.0)).min(track_height);
        let fraction = relative_y / track_height;

        if let Some(session) = self.active_session() {
            let mut grid = session.grid.lock();
            let (total, visible, _) = grid.scroll_info();
            if total > visible {
                let scrollable = total - visible;
                // fraction 0.0 = top (max scroll_up), 1.0 = bottom (scroll_offset = 0)
                let offset = ((1.0 - fraction) * scrollable as f32).round() as usize;
                grid.set_scroll_offset(offset.min(scrollable));
            }
        }
    }

    /// Copy selected text to clipboard.
    fn copy_selection(&self, cx: &App) {
        if let Some(session) = self.active_session() {
            let grid = session.grid.lock();
            if let Some(text) = grid.selected_text() {
                cx.write_to_clipboard(ClipboardItem::new_string(text));
            }
        }
    }

    /// Toggle search bar visibility.
    fn toggle_search(&mut self) {
        self.search_visible = !self.search_visible;
        if !self.search_visible {
            self.search_query.clear();
            self.search_matches.clear();
            self.search_current_idx = None;
        }
    }

    /// Update search results based on current query.
    fn update_search(&mut self) {
        if self.search_query.is_empty() {
            self.search_matches.clear();
            self.search_current_idx = None;
            return;
        }
        let grid_arc = self.active_session().map(|s| s.grid.clone());
        if let Some(grid_arc) = grid_arc {
            let grid = grid_arc.lock();
            let matches = grid.search(
                &self.search_query,
                self.search_case_sensitive,
                self.search_regex,
            );
            drop(grid);
            if matches.is_empty() {
                self.search_current_idx = None;
            } else {
                self.search_current_idx = Some(matches.len().saturating_sub(1));
            }
            self.search_matches = matches;
        }
    }

    /// Navigate to next search match.
    fn search_next(&mut self) {
        if self.search_matches.is_empty() {
            return;
        }
        let idx = self.search_current_idx.unwrap_or(0);
        self.search_current_idx = Some((idx + 1) % self.search_matches.len());
    }

    /// Navigate to previous search match.
    fn search_prev(&mut self) {
        if self.search_matches.is_empty() {
            return;
        }
        let idx = self.search_current_idx.unwrap_or(0);
        self.search_current_idx = Some(if idx == 0 {
            self.search_matches.len() - 1
        } else {
            idx - 1
        });
    }

    /// Render the search bar overlay.
    fn render_search_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let match_info = if self.search_matches.is_empty() {
            if self.search_query.is_empty() {
                String::new()
            } else {
                "No matches".to_string()
            }
        } else {
            let current = self.search_current_idx.map(|i| i + 1).unwrap_or(0);
            format!("{} of {}", current, self.search_matches.len())
        };

        div()
            .absolute()
            .top(px(8.0))
            .right(px(16.0))
            .flex()
            .items_center()
            .gap(px(6.0))
            .px(px(10.0))
            .py(px(6.0))
            .bg(ShellDeckColors::bg_surface())
            .border_1()
            .border_color(ShellDeckColors::border())
            .rounded(px(8.0))
            .shadow_md()
            .child(div().text_size(px(13.0)).min_w(px(150.0)).child(
                if self.search_query.is_empty() {
                    div()
                        .text_color(ShellDeckColors::text_muted())
                        .child("Search...")
                } else {
                    div()
                        .text_color(ShellDeckColors::text_primary())
                        .flex()
                        .child(self.search_query.clone())
                        .child(div().w(px(1.0)).h(px(14.0)).bg(ShellDeckColors::primary()))
                },
            ))
            .child(
                div()
                    .text_size(px(11.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(match_info),
            )
            .child(
                // Prev button
                div()
                    .id("search-prev")
                    .text_size(px(12.0))
                    .text_color(ShellDeckColors::text_muted())
                    .cursor_pointer()
                    .hover(|el| el.text_color(ShellDeckColors::text_primary()))
                    .child("<")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.search_prev();
                        cx.notify();
                    })),
            )
            .child(
                // Next button
                div()
                    .id("search-next")
                    .text_size(px(12.0))
                    .text_color(ShellDeckColors::text_muted())
                    .cursor_pointer()
                    .hover(|el| el.text_color(ShellDeckColors::text_primary()))
                    .child(">")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.search_next();
                        cx.notify();
                    })),
            )
            .child(
                // Close button
                div()
                    .id("search-close")
                    .text_size(px(12.0))
                    .text_color(ShellDeckColors::text_muted())
                    .cursor_pointer()
                    .hover(|el| el.text_color(ShellDeckColors::error()))
                    .child("x")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.toggle_search();
                        cx.notify();
                    })),
            )
    }

    /// Paste clipboard content into the terminal.
    fn paste_clipboard(&self, cx: &App) {
        if let Some(item) = cx.read_from_clipboard() {
            if let Some(text) = item.text() {
                if let Some(session) = self.active_session() {
                    let bracketed = session.grid.lock().bracketed_paste();
                    if bracketed {
                        let mut data = Vec::with_capacity(text.len() + 12);
                        data.extend_from_slice(b"\x1b[200~");
                        data.extend_from_slice(text.as_bytes());
                        data.extend_from_slice(b"\x1b[201~");
                        session.write_input(&data);
                    } else {
                        session.write_input(text.as_bytes());
                    }
                }
            }
        }
    }

    /// Render the context menu overlay.
    fn render_context_menu(
        &self,
        state: &ContextMenuState,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let mut menu = div()
            .id("context-menu")
            .absolute()
            .left(state.position.x - px(self.grid_x_offset))
            .top(state.position.y - px(self.grid_y_offset))
            .min_w(px(180.0))
            .bg(ShellDeckColors::bg_surface())
            .border_1()
            .border_color(ShellDeckColors::border())
            .rounded(px(8.0))
            .shadow_lg()
            .py(px(4.0))
            .flex()
            .flex_col()
            // Capture mouse events so they don't pass through to the grid underneath.
            .on_mouse_down(MouseButton::Left, |_, _, _| {})
            .on_mouse_down(MouseButton::Right, |_, _, _| {});

        let menu_item = |id: &str, label: &str| {
            div()
                .id(ElementId::from(SharedString::from(id.to_string())))
                .px(px(12.0))
                .py(px(6.0))
                .text_size(px(13.0))
                .text_color(ShellDeckColors::text_primary())
                .cursor_pointer()
                .hover(|el| el.bg(ShellDeckColors::hover_bg()))
                .child(label.to_string())
        };

        menu = menu
            .child(
                menu_item("ctx-copy", "Copy").on_click(cx.listener(|this, _, _, cx| {
                    this.copy_selection(cx);
                    this.context_menu = None;
                    cx.notify();
                })),
            )
            .child(
                menu_item("ctx-paste", "Paste").on_click(cx.listener(|this, _, _, cx| {
                    this.paste_clipboard(cx);
                    this.context_menu = None;
                    cx.notify();
                })),
            )
            .child(
                div()
                    .h(px(1.0))
                    .mx(px(8.0))
                    .my(px(4.0))
                    .bg(ShellDeckColors::border()),
            )
            .child(
                menu_item("ctx-select-all", "Select All").on_click(cx.listener(
                    |this, _, _, cx| {
                        if let Some(session) = this.active_session() {
                            let mut grid = session.grid.lock();
                            let rows = grid.rows;
                            let cols = grid.cols;
                            grid.start_selection(0, 0);
                            grid.update_selection(cols.saturating_sub(1), rows.saturating_sub(1));
                            grid.end_selection();
                        }
                        this.context_menu = None;
                        cx.notify();
                    },
                )),
            )
            .child(
                menu_item("ctx-clear", "Clear Terminal").on_click(cx.listener(|this, _, _, cx| {
                    if let Some(session) = this.active_session() {
                        let mut grid = session.grid.lock();
                        grid.erase_display(2);
                        grid.cursor_to(0, 0);
                    }
                    this.context_menu = None;
                    cx.notify();
                })),
            )
            .child(
                div()
                    .h(px(1.0))
                    .mx(px(8.0))
                    .my(px(4.0))
                    .bg(ShellDeckColors::border()),
            )
            .child(
                menu_item("ctx-search", "Search").on_click(cx.listener(|this, _, _, cx| {
                    this.search_visible = true;
                    this.context_menu = None;
                    cx.notify();
                })),
            );

        // URL actions if applicable
        if let Some(url) = &state.url {
            let url_clone = url.clone();
            let url_copy = url.clone();
            menu = menu
                .child(
                    div()
                        .h(px(1.0))
                        .mx(px(8.0))
                        .my(px(4.0))
                        .bg(ShellDeckColors::border()),
                )
                .child(
                    menu_item("ctx-open-link", "Open Link").on_click(cx.listener(
                        move |this, _, _, cx| {
                            let _ = open::that(&url_clone);
                            this.context_menu = None;
                            cx.notify();
                        },
                    )),
                )
                .child(
                    menu_item("ctx-copy-link", "Copy Link").on_click(cx.listener(
                        move |this, _, _, cx| {
                            cx.write_to_clipboard(ClipboardItem::new_string(url_copy.clone()));
                            this.context_menu = None;
                            cx.notify();
                        },
                    )),
                );
        }

        menu
    }

    /// Render a read-only grid for the unfocused split pane.
    /// Clicking anywhere on it toggles focus to this pane.
    fn render_split_passive_grid(
        &self,
        grid_arc: Arc<parking_lot::Mutex<TerminalGrid>>,
        cache: Arc<GlyphCache>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let mut cursor = grid_arc.lock().cursor.clone();
        cursor.shape = self.effective_cursor_shape(cursor.shape);
        let focus = self.focus_handle.clone();

        let focus2 = self.focus_handle.clone();

        div()
            .id("terminal-grid-passive")
            .relative()
            .size_full()
            .bg(ShellDeckColors::terminal_bg())
            .p(px(4.0))
            .overflow_hidden()
            .cursor_pointer()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _event: &MouseDownEvent, window, cx| {
                    this.toggle_split_focus();
                    this.needs_focus = true;
                    focus.focus(window);
                    cx.notify();
                }),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                    this.toggle_split_focus();
                    this.needs_focus = true;
                    focus2.focus(window);
                    this.context_menu = Some(ContextMenuState {
                        position: event.position,
                        url: None,
                    });
                    cx.notify();
                }),
            )
            .child(Self::create_grid_canvas(
                cache,
                grid_arc,
                cursor,
                vec![],
                None,
                vec![],
                self.palette.clone(),
                false, // passive grid is never focused
                true,  // cursor always visible (no blink) in passive pane
            ))
    }

    /// Get the effective font size (base * zoom).
    fn effective_font_size(&self) -> f32 {
        let zoom = self
            .tabs
            .get(self.pane.active_index)
            .map(|t| t.zoom_level)
            .unwrap_or(1.0);
        self.font_size * zoom
    }

    /// Zoom in on the active terminal.
    fn zoom_in(&mut self) {
        if let Some(tab) = self.tabs.get_mut(self.pane.active_index) {
            tab.zoom_level = (tab.zoom_level * 1.1).min(3.0);
            self.glyph_cache = None; // Invalidate
        }
    }

    /// Zoom out on the active terminal.
    fn zoom_out(&mut self) {
        if let Some(tab) = self.tabs.get_mut(self.pane.active_index) {
            tab.zoom_level = (tab.zoom_level / 1.1).max(0.5);
            self.glyph_cache = None; // Invalidate
        }
    }

    /// Reset zoom to 1.0.
    fn zoom_reset(&mut self) {
        if let Some(tab) = self.tabs.get_mut(self.pane.active_index) {
            tab.zoom_level = 1.0;
            self.glyph_cache = None; // Invalidate
        }
    }

    /// Split the current pane horizontally (side by side).
    pub fn split_horizontal(&mut self, cx: &mut Context<Self>) {
        self.do_split(SplitDirection::Horizontal, cx);
    }

    /// Split the current pane vertically (top/bottom).
    pub fn split_vertical(&mut self, cx: &mut Context<Self>) {
        self.do_split(SplitDirection::Vertical, cx);
    }

    fn do_split(&mut self, direction: SplitDirection, cx: &mut Context<Self>) {
        if self.pane.sessions.is_empty() {
            return;
        }

        // Don't allow nested splits for now
        if self.split_view.is_some() {
            return;
        }

        // Check if the current tab is an SSH connection
        let connection_id = self
            .tabs
            .get(self.pane.active_index)
            .and_then(|t| t.connection_id);

        if let Some(conn_id) = connection_id {
            // Emit event so the workspace can open an SSH session for the split
            cx.emit(TerminalEvent::SplitRequested {
                connection_id: conn_id,
                direction,
            });
            return;
        }

        // Local terminal: spawn directly
        self.spawn_local_split(direction, cx);
    }

    /// Spawn a local terminal as the split secondary pane.
    fn spawn_local_split(&mut self, direction: SplitDirection, cx: &mut Context<Self>) {
        let (rows, cols) = if self.last_grid_rows > 0 {
            (self.last_grid_rows, self.last_grid_cols)
        } else {
            (24, 80)
        };
        let new_session = match TerminalSession::spawn_local(None, rows, cols) {
            Ok(s) => s,
            Err(e) => {
                tracing::error!("Failed to spawn split terminal: {}", e);
                return;
            }
        };

        self.split_view = Some(SplitViewState {
            direction,
            secondary_session: new_session,
            focus_secondary: false,
            ratio: 0.5,
        });

        self.ensure_refresh_running(cx);
        cx.notify();
    }

    /// Set the split secondary session from an externally-created session
    /// (e.g. an SSH session created by the workspace).
    pub fn set_split_session(
        &mut self,
        session: TerminalSession,
        direction: SplitDirection,
        cx: &mut Context<Self>,
    ) {
        // Don't overwrite an existing split
        if self.split_view.is_some() {
            return;
        }

        self.split_view = Some(SplitViewState {
            direction,
            secondary_session: session,
            focus_secondary: false,
            ratio: 0.5,
        });

        self.ensure_refresh_running(cx);
        cx.notify();
    }

    /// Toggle the split direction between horizontal and vertical.
    fn toggle_split_direction(&mut self) {
        if let Some(ref mut split) = self.split_view {
            split.direction = match split.direction {
                SplitDirection::Horizontal => SplitDirection::Vertical,
                SplitDirection::Vertical => SplitDirection::Horizontal,
            };
        }
    }

    /// Close the current split, keeping the focused pane.
    /// If the secondary pane is focused, swap it into the current tab slot.
    fn close_split(&mut self) {
        if let Some(split) = self.split_view.take() {
            if split.focus_secondary {
                // Replace the current tab's session with the secondary session
                let idx = self.pane.active_index;
                if idx < self.pane.sessions.len() {
                    let new_session = split.secondary_session;
                    // Update tab metadata to match the new session
                    if let Some(tab) = self.tabs.get_mut(idx) {
                        tab.id = new_session.id;
                        tab.title = new_session.title.clone();
                        tab.state = new_session.state.clone();
                    }
                    self.pane.sessions[idx] = new_session;
                }
            }
            // split is already taken; secondary dropped if not swapped
        }
    }

    /// Encode a mouse event as a terminal escape sequence.
    fn encode_mouse(
        encoding: MouseEncoding,
        button: u8,
        col: u16,
        row: u16,
        press: bool,
    ) -> Vec<u8> {
        match encoding {
            MouseEncoding::Sgr => {
                let suffix = if press { 'M' } else { 'm' };
                format!("\x1b[<{};{};{}{}", button, col, row, suffix).into_bytes()
            }
            MouseEncoding::Normal => {
                let b = button + 32;
                let c = (col as u8).saturating_add(32);
                let r = (row as u8).saturating_add(32);
                vec![0x1b, b'[', b'M', b, c, r]
            }
        }
    }

    fn render_empty_state(&self) -> impl IntoElement {
        let (ctrl, cmd) = if cfg!(target_os = "macos") {
            ("\u{2318}", "\u{2318}")
        } else {
            ("Ctrl+", "Ctrl+")
        };
        let shift = if cfg!(target_os = "macos") {
            "\u{21E7}"
        } else {
            "Shift+"
        };

        let shortcut_row = |keys: String, desc: &str| {
            div()
                .flex()
                .items_center()
                .gap(px(12.0))
                .py(px(3.0))
                .child(
                    div()
                        .min_w(px(140.0))
                        .text_size(px(12.0))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(ShellDeckColors::primary())
                        .child(
                            div()
                                .px(px(6.0))
                                .py(px(2.0))
                                .rounded(px(4.0))
                                .bg(ShellDeckColors::primary().opacity(0.12))
                                .child(keys),
                        ),
                )
                .child(
                    div()
                        .text_size(px(12.0))
                        .text_color(ShellDeckColors::text_muted())
                        .child(desc.to_string()),
                )
        };

        div()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .size_full()
            .bg(ShellDeckColors::terminal_bg())
            .gap(px(24.0))
            // Icon + heading
            .child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(8.0))
                    .child(
                        div()
                            .text_size(px(48.0))
                            .text_color(ShellDeckColors::text_muted())
                            .child(">_"),
                    )
                    .child(
                        div()
                            .text_size(px(16.0))
                            .text_color(ShellDeckColors::text_muted())
                            .child("No terminal sessions"),
                    )
                    .child(
                        div()
                            .text_size(px(13.0))
                            .text_color(ShellDeckColors::text_muted())
                            .child(format!(
                                "Press {}T to open a new terminal or click a connection",
                                cmd
                            )),
                    ),
            )
            // Keyboard shortcuts reference
            .child(
                div()
                    .flex()
                    .flex_col()
                    .p(px(20.0))
                    .rounded(px(8.0))
                    .bg(ShellDeckColors::bg_surface())
                    .border_1()
                    .border_color(ShellDeckColors::border())
                    .max_w(px(420.0))
                    .gap(px(2.0))
                    .child(
                        div()
                            .text_size(px(13.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(ShellDeckColors::text_primary())
                            .mb(px(8.0))
                            .child("Keyboard Shortcuts"),
                    )
                    .child(shortcut_row(format!("{}T", cmd), "New terminal"))
                    .child(shortcut_row(
                        format!("{}{}P", cmd, shift),
                        "Command palette",
                    ))
                    .child(shortcut_row(format!("{}F", cmd), "Search in terminal"))
                    .child(shortcut_row(format!("{}B", cmd), "Toggle sidebar"))
                    .child(shortcut_row(format!("{}{}D", ctrl, shift), "Split pane"))
                    .child(shortcut_row(
                        format!("{}{}C", ctrl, shift),
                        "Copy selection",
                    ))
                    .child(shortcut_row(
                        format!("{}{}V", ctrl, shift),
                        "Paste clipboard",
                    ))
                    .child(shortcut_row(
                        format!("{}= / {}-", cmd, cmd),
                        "Zoom in / out",
                    ))
                    .child(shortcut_row(format!("{},", cmd), "Settings"))
                    .child(shortcut_row("Ctrl+Tab".to_string(), "Next tab")),
            )
    }
}

impl Render for TerminalView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut container = div().flex().flex_col().size_full();

        if self.pane.sessions.is_empty() {
            container = container.child(self.render_empty_state());
        } else {
            if self.needs_focus {
                self.needs_focus = false;
                self.focus_handle.focus(window);
            }

            // --- Focus change detection & blink timer management ---
            let focused_now = self.focus_handle.is_focused(window);
            if focused_now != self.has_focus {
                self.has_focus = focused_now;
                if focused_now {
                    // Gained focus: start blink timer if grid wants blinking
                    let grid_blink = self
                        .active_session()
                        .map(|s| s.grid.lock().cursor.blink)
                        .unwrap_or(false);
                    if grid_blink {
                        self.start_cursor_blink(cx);
                    } else {
                        self.stop_cursor_blink();
                    }
                    // Send focus-in event if app requested it (mode 1004)
                    if let Some(session) = self.active_session() {
                        if session.grid.lock().focus_reporting() {
                            session.write_input(b"\x1b[I");
                        }
                    }
                } else {
                    // Lost focus: stop blinking, show steady cursor
                    self.stop_cursor_blink();
                    // Send focus-out event if app requested it (mode 1004)
                    if let Some(session) = self.active_session() {
                        if session.grid.lock().focus_reporting() {
                            session.write_input(b"\x1b[O");
                        }
                    }
                }
            }

            self.resize_if_needed(window);

            container = container
                .child(self.render_tab_bar(cx))
                .child(self.render_toolbar(cx));

            // Ensure glyph cache is ready
            self.ensure_glyph_cache(window);
            let cache = self
                .glyph_cache
                .as_ref()
                .expect("ensure_glyph_cache called above")
                .clone();

            // Check for split view — extract state we need without cloning the session.
            let split_info = self.split_view.as_ref().map(|s| {
                (
                    s.direction,
                    s.focus_secondary,
                    s.ratio,
                    s.secondary_session.grid.clone(),
                )
            });

            if let Some((split_direction, focus_secondary, split_ratio, secondary_grid)) =
                split_info
            {
                // ---- Split pane rendering ----
                // Primary pane = active tab's session, secondary pane = split's session.
                // "first" = primary (left/top), "second" = secondary (right/bottom).
                let first_is_active = !focus_secondary;

                // Clear dirty flags on both sessions
                if let Some(s) = self.pane.sessions.get(self.pane.active_index) {
                    s.grid.lock().dirty = false;
                }
                secondary_grid.lock().dirty = false;

                let primary_grid = self
                    .pane
                    .sessions
                    .get(self.pane.active_index)
                    .map(|s| s.grid.clone());

                // Compute offsets for the active pane's coordinate mapping.
                let viewport = window.viewport_size();
                let sidebar_w: f32 = self.sidebar_width + SIDEBAR_HANDLE_WIDTH;
                let top_offset: f32 = GRID_TOP_OFFSET;
                let content_w = viewport.width.to_f64() as f32 - sidebar_w;
                let content_h = viewport.height.to_f64() as f32 - top_offset - STATUS_BAR_HEIGHT;
                let divider_f: f32 = SPLIT_DIVIDER_SIZE;

                // The interactive (focused) grid determines the coordinate offset
                match split_direction {
                    SplitDirection::Horizontal => {
                        let first_w = (content_w - divider_f) * split_ratio;
                        self.grid_x_offset = if first_is_active {
                            sidebar_w
                        } else {
                            sidebar_w + first_w + divider_f
                        };
                        self.grid_y_offset = top_offset;
                    }
                    SplitDirection::Vertical => {
                        let first_h = (content_h - divider_f) * split_ratio;
                        self.grid_x_offset = sidebar_w;
                        self.grid_y_offset = if first_is_active {
                            top_offset
                        } else {
                            top_offset + first_h + divider_f
                        };
                    }
                }

                if let Some(primary_grid_arc) = primary_grid {
                    // Determine which grid is active (interactive) and which is passive
                    let (active_grid, passive_grid) = if first_is_active {
                        (primary_grid_arc, secondary_grid)
                    } else {
                        (secondary_grid, primary_grid_arc)
                    };

                    let (mouse_mode, mouse_encoding, mut cursor) = {
                        let g = active_grid.lock();
                        let visible = g.visible_rows();
                        self.detected_urls = detect_urls(&visible);
                        (g.mouse_mode, g.mouse_encoding, g.cursor.clone())
                    };
                    cursor.shape = self.effective_cursor_shape(cursor.shape);

                    let active_grid_el = self.render_terminal_grid(
                        mouse_mode,
                        mouse_encoding,
                        cursor,
                        cache.clone(),
                        active_grid,
                        cx,
                    );

                    let passive_grid_el = self.render_split_passive_grid(passive_grid, cache, cx);

                    let (first_el, second_el) = if first_is_active {
                        (
                            active_grid_el.into_any_element(),
                            passive_grid_el.into_any_element(),
                        )
                    } else {
                        (
                            passive_grid_el.into_any_element(),
                            active_grid_el.into_any_element(),
                        )
                    };

                    // Build the split flex container
                    let is_horizontal = matches!(split_direction, SplitDirection::Horizontal);
                    let split_dragging = self.split_dragging;

                    // Divider: only starts the drag. Move/up handled on split_container.
                    let divider = if is_horizontal {
                        div()
                            .id("split-divider")
                            .w(px(6.0))
                            .h_full()
                            .bg(ShellDeckColors::border())
                            .flex_shrink_0()
                            .cursor_col_resize()
                            .hover(|el| el.bg(ShellDeckColors::primary().opacity(0.5)))
                            .when(split_dragging, |el| {
                                el.bg(ShellDeckColors::primary().opacity(0.5))
                            })
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, _: &MouseDownEvent, _, cx| {
                                    this.split_dragging = true;
                                    cx.notify();
                                }),
                            )
                    } else {
                        div()
                            .id("split-divider")
                            .h(px(6.0))
                            .w_full()
                            .bg(ShellDeckColors::border())
                            .flex_shrink_0()
                            .cursor_row_resize()
                            .hover(|el| el.bg(ShellDeckColors::primary().opacity(0.5)))
                            .when(split_dragging, |el| {
                                el.bg(ShellDeckColors::primary().opacity(0.5))
                            })
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, _: &MouseDownEvent, _, cx| {
                                    this.split_dragging = true;
                                    cx.notify();
                                }),
                            )
                    };

                    let mut split_container = div()
                        .id("split-container")
                        .flex()
                        .flex_grow()
                        .size_full()
                        .overflow_hidden();

                    if !is_horizontal {
                        split_container = split_container.flex_col();
                    }

                    // Handle drag move/up on the split container so the mouse
                    // can travel freely across both panes while dragging.
                    if split_dragging {
                        split_container = split_container
                            .when(is_horizontal, |el| el.cursor_col_resize())
                            .when(!is_horizontal, |el| el.cursor_row_resize())
                            .on_mouse_move(cx.listener(
                                move |this, event: &MouseMoveEvent, window, cx| {
                                    if is_horizontal {
                                        let viewport = window.viewport_size();
                                        let sidebar_w = this.sidebar_width + SIDEBAR_HANDLE_WIDTH;
                                        let content_w = viewport.width.to_f64() as f32 - sidebar_w;
                                        let mouse_x = event.position.x.to_f64() as f32 - sidebar_w;
                                        let new_ratio = (mouse_x / content_w).clamp(0.2, 0.8);
                                        if let Some(ref mut split) = this.split_view {
                                            split.ratio = new_ratio;
                                        }
                                    } else {
                                        let viewport = window.viewport_size();
                                        let top_offset = GRID_TOP_OFFSET;
                                        let content_h = viewport.height.to_f64() as f32
                                            - top_offset
                                            - STATUS_BAR_HEIGHT;
                                        let mouse_y = event.position.y.to_f64() as f32 - top_offset;
                                        let new_ratio = (mouse_y / content_h).clamp(0.2, 0.8);
                                        if let Some(ref mut split) = this.split_view {
                                            split.ratio = new_ratio;
                                        }
                                    }
                                    this.resize_if_needed(window);
                                    cx.notify();
                                },
                            ))
                            .on_mouse_up(
                                MouseButton::Left,
                                cx.listener(|this, _: &MouseUpEvent, _, cx| {
                                    this.split_dragging = false;
                                    cx.notify();
                                }),
                            );
                    }

                    // Compute explicit pixel sizes from ratio so panes
                    // actually resize visually (flex_1 always gives 50/50).
                    let mut first_wrapper = div().relative().overflow_hidden().border_t_2();
                    let mut second_wrapper = div().relative().overflow_hidden().border_t_2();

                    if is_horizontal {
                        let first_px = (content_w - divider_f) * split_ratio;
                        let second_px = (content_w - divider_f) * (1.0 - split_ratio);
                        first_wrapper = first_wrapper.w(px(first_px)).h_full();
                        second_wrapper = second_wrapper.w(px(second_px)).h_full();
                    } else {
                        let first_px = (content_h - divider_f) * split_ratio;
                        let second_px = (content_h - divider_f) * (1.0 - split_ratio);
                        first_wrapper = first_wrapper.h(px(first_px)).w_full();
                        second_wrapper = second_wrapper.h(px(second_px)).w_full();
                    }

                    first_wrapper = first_wrapper.child(first_el);
                    second_wrapper = second_wrapper.child(second_el);

                    // Highlight the focused pane's top border
                    if first_is_active {
                        first_wrapper = first_wrapper.border_color(ShellDeckColors::primary());
                        second_wrapper = second_wrapper.border_color(transparent_black());
                    } else {
                        first_wrapper = first_wrapper.border_color(transparent_black());
                        second_wrapper = second_wrapper.border_color(ShellDeckColors::primary());
                    }

                    // Add overlays (search bar, context menu) to the active pane wrapper
                    if first_is_active {
                        if self.search_visible {
                            first_wrapper = first_wrapper.child(self.render_search_bar(cx));
                        }
                        if let Some(state) = self.context_menu.clone() {
                            first_wrapper =
                                first_wrapper.child(self.render_context_menu(&state, cx));
                        }
                    } else {
                        if self.search_visible {
                            second_wrapper = second_wrapper.child(self.render_search_bar(cx));
                        }
                        if let Some(state) = self.context_menu.clone() {
                            second_wrapper =
                                second_wrapper.child(self.render_context_menu(&state, cx));
                        }
                    }

                    split_container = split_container
                        .child(first_wrapper)
                        .child(divider)
                        .child(second_wrapper);

                    container = container.child(split_container);
                }
            } else {
                // ---- Single pane rendering ----
                self.grid_x_offset = self.sidebar_width + SIDEBAR_HANDLE_WIDTH;
                self.grid_y_offset = GRID_TOP_OFFSET;

                let grid_arc = self
                    .pane
                    .sessions
                    .get(self.pane.active_index)
                    .map(|s| s.grid.clone());

                if let Some(grid) = grid_arc {
                    let (mouse_mode, mouse_encoding, mut cursor) = {
                        let mut g = grid.lock();
                        g.dirty = false;
                        let visible = g.visible_rows();
                        self.detected_urls = detect_urls(&visible);
                        (g.mouse_mode, g.mouse_encoding, g.cursor.clone())
                    };
                    cursor.shape = self.effective_cursor_shape(cursor.shape);

                    let mut grid_wrapper =
                        div()
                            .relative()
                            .flex_grow()
                            .size_full()
                            .child(self.render_terminal_grid(
                                mouse_mode,
                                mouse_encoding,
                                cursor,
                                cache,
                                grid,
                                cx,
                            ));

                    if self.search_visible {
                        grid_wrapper = grid_wrapper.child(self.render_search_bar(cx));
                    }

                    if let Some(state) = self.context_menu.clone() {
                        grid_wrapper = grid_wrapper.child(self.render_context_menu(&state, cx));
                    }

                    container = container.child(grid_wrapper);
                }
            }
        }

        container
    }
}
