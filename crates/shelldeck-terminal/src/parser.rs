use crate::colors::{NamedColor, TermColor};
use crate::grid::{MouseEncoding, MouseMode, TerminalGrid};
use parking_lot::Mutex;
use std::sync::Arc;

pub struct TerminalProcessor {
    grid: Arc<Mutex<TerminalGrid>>,
}

impl TerminalProcessor {
    pub fn new(grid: Arc<Mutex<TerminalGrid>>) -> Self {
        Self { grid }
    }

    pub fn process_bytes(&mut self, parser: &mut vte::Parser, bytes: &[u8]) {
        for byte in bytes {
            parser.advance(self, *byte);
        }
    }

    /// Parse SGR (Select Graphic Rendition) parameters and apply them to the
    /// grid's current text attributes.
    fn apply_sgr(grid: &mut TerminalGrid, params: &[u16]) {
        if params.is_empty() {
            // No params = reset.
            grid.current_attrs = Default::default();
            grid.current_fg = TermColor::Default;
            grid.current_bg = TermColor::Default;
            return;
        }

        let mut i = 0;
        while i < params.len() {
            match params[i] {
                0 => {
                    grid.current_attrs = Default::default();
                    grid.current_fg = TermColor::Default;
                    grid.current_bg = TermColor::Default;
                }
                1 => grid.current_attrs.bold = true,
                2 => grid.current_attrs.bold = false, // dim / faint, treat as not bold
                3 => grid.current_attrs.italic = true,
                4 => grid.current_attrs.underline = true,
                5 | 6 => grid.current_attrs.blink = true,
                7 => grid.current_attrs.inverse = true,
                8 => grid.current_attrs.hidden = true,
                9 => grid.current_attrs.strikethrough = true,

                21 => grid.current_attrs.underline = true, // double underline (treat as underline)
                22 => grid.current_attrs.bold = false,
                23 => grid.current_attrs.italic = false,
                24 => grid.current_attrs.underline = false,
                25 => grid.current_attrs.blink = false,
                27 => grid.current_attrs.inverse = false,
                28 => grid.current_attrs.hidden = false,
                29 => grid.current_attrs.strikethrough = false,

                // Foreground colors (standard 8).
                30 => grid.current_fg = TermColor::Named(NamedColor::Black),
                31 => grid.current_fg = TermColor::Named(NamedColor::Red),
                32 => grid.current_fg = TermColor::Named(NamedColor::Green),
                33 => grid.current_fg = TermColor::Named(NamedColor::Yellow),
                34 => grid.current_fg = TermColor::Named(NamedColor::Blue),
                35 => grid.current_fg = TermColor::Named(NamedColor::Magenta),
                36 => grid.current_fg = TermColor::Named(NamedColor::Cyan),
                37 => grid.current_fg = TermColor::Named(NamedColor::White),

                // Extended foreground: 38;5;N (256 color) or 38;2;R;G;B (true color).
                38 => {
                    if let Some(color) = parse_extended_color(params, &mut i) {
                        grid.current_fg = color;
                    }
                }

                39 => grid.current_fg = TermColor::Default,

                // Background colors (standard 8).
                40 => grid.current_bg = TermColor::Named(NamedColor::Black),
                41 => grid.current_bg = TermColor::Named(NamedColor::Red),
                42 => grid.current_bg = TermColor::Named(NamedColor::Green),
                43 => grid.current_bg = TermColor::Named(NamedColor::Yellow),
                44 => grid.current_bg = TermColor::Named(NamedColor::Blue),
                45 => grid.current_bg = TermColor::Named(NamedColor::Magenta),
                46 => grid.current_bg = TermColor::Named(NamedColor::Cyan),
                47 => grid.current_bg = TermColor::Named(NamedColor::White),

                // Extended background: 48;5;N or 48;2;R;G;B.
                48 => {
                    if let Some(color) = parse_extended_color(params, &mut i) {
                        grid.current_bg = color;
                    }
                }

                49 => grid.current_bg = TermColor::Default,

                // Bright foreground colors.
                90 => grid.current_fg = TermColor::Named(NamedColor::BrightBlack),
                91 => grid.current_fg = TermColor::Named(NamedColor::BrightRed),
                92 => grid.current_fg = TermColor::Named(NamedColor::BrightGreen),
                93 => grid.current_fg = TermColor::Named(NamedColor::BrightYellow),
                94 => grid.current_fg = TermColor::Named(NamedColor::BrightBlue),
                95 => grid.current_fg = TermColor::Named(NamedColor::BrightMagenta),
                96 => grid.current_fg = TermColor::Named(NamedColor::BrightCyan),
                97 => grid.current_fg = TermColor::Named(NamedColor::BrightWhite),

                // Bright background colors.
                100 => grid.current_bg = TermColor::Named(NamedColor::BrightBlack),
                101 => grid.current_bg = TermColor::Named(NamedColor::BrightRed),
                102 => grid.current_bg = TermColor::Named(NamedColor::BrightGreen),
                103 => grid.current_bg = TermColor::Named(NamedColor::BrightYellow),
                104 => grid.current_bg = TermColor::Named(NamedColor::BrightBlue),
                105 => grid.current_bg = TermColor::Named(NamedColor::BrightMagenta),
                106 => grid.current_bg = TermColor::Named(NamedColor::BrightCyan),
                107 => grid.current_bg = TermColor::Named(NamedColor::BrightWhite),

                _ => {} // Ignore unknown SGR codes.
            }
            i += 1;
        }
    }
}

/// Parse extended color sequences: `5;N` (256-color) or `2;R;G;B` (true color).
/// `i` points at the `38` or `48` in the params array. On success, `i` is
/// advanced past the consumed sub-parameters (so the outer loop's `i += 1`
/// will land on the next unrelated parameter).
fn parse_extended_color(params: &[u16], i: &mut usize) -> Option<TermColor> {
    if *i + 1 >= params.len() {
        return None;
    }
    match params[*i + 1] {
        5 => {
            // 256-color mode: 38;5;N
            if *i + 2 < params.len() {
                let idx = params[*i + 2] as u8;
                *i += 2; // advance past 5 and N
                Some(TermColor::Indexed(idx))
            } else {
                *i += 1;
                None
            }
        }
        2 => {
            // True color mode: 38;2;R;G;B
            if *i + 4 < params.len() {
                let r = params[*i + 2] as u8;
                let g = params[*i + 3] as u8;
                let b = params[*i + 4] as u8;
                *i += 4; // advance past 2, R, G, B
                Some(TermColor::Rgb(r, g, b))
            } else {
                *i += 1;
                None
            }
        }
        _ => None,
    }
}

/// Extract the first parameter from a vte::Params iterator, defaulting to `default`.
fn param(params: &vte::Params, index: usize, default: u16) -> u16 {
    params
        .iter()
        .nth(index)
        .and_then(|sub| sub.first().copied())
        .map(|v| if v == 0 { default } else { v })
        .unwrap_or(default)
}

/// Collect all params into a flat u16 slice (flattening sub-params).
fn collect_params(params: &vte::Params) -> Vec<u16> {
    let mut out = Vec::new();
    for sub in params.iter() {
        for &val in sub {
            out.push(val);
        }
    }
    out
}

impl vte::Perform for TerminalProcessor {
    fn print(&mut self, c: char) {
        self.grid.lock().write_char(c);
    }

    fn execute(&mut self, byte: u8) {
        let mut grid = self.grid.lock();
        match byte {
            0x07 => grid.bell(),
            0x08 => grid.backspace(),
            0x09 => grid.tab(),
            0x0A | 0x0B | 0x0C => grid.newline(), // LF, VT, FF all treated as newline
            0x0D => grid.carriage_return(),
            0x0E => grid.activate_g1(), // SO - Shift Out (activate G1)
            0x0F => grid.activate_g0(), // SI - Shift In (activate G0)
            _ => {}
        }
    }

    fn csi_dispatch(
        &mut self,
        params: &vte::Params,
        intermediates: &[u8],
        _ignore: bool,
        action: char,
    ) {
        let mut grid = self.grid.lock();
        let private_mode = intermediates.first().copied() == Some(b'?');

        match action {
            // CUU - Cursor Up
            'A' => {
                let n = param(params, 0, 1) as usize;
                grid.cursor_up(n);
            }
            // CUD - Cursor Down
            'B' => {
                let n = param(params, 0, 1) as usize;
                grid.cursor_down(n);
            }
            // CUF - Cursor Forward
            'C' => {
                let n = param(params, 0, 1) as usize;
                grid.cursor_forward(n);
            }
            // CUB - Cursor Backward
            'D' => {
                let n = param(params, 0, 1) as usize;
                grid.cursor_backward(n);
            }
            // CNL - Cursor Next Line
            'E' => {
                let n = param(params, 0, 1) as usize;
                grid.cursor_down(n);
                grid.carriage_return();
            }
            // CPL - Cursor Previous Line
            'F' => {
                let n = param(params, 0, 1) as usize;
                grid.cursor_up(n);
                grid.carriage_return();
            }
            // CHA - Cursor Horizontal Absolute
            'G' => {
                let col = param(params, 0, 1) as usize;
                let row = grid.cursor.row;
                grid.cursor_to(row, col.saturating_sub(1));
            }
            // CUP / HVP - Cursor Position
            'H' | 'f' => {
                let row = param(params, 0, 1) as usize;
                let col = param(params, 1, 1) as usize;
                grid.cursor_to(row.saturating_sub(1), col.saturating_sub(1));
            }
            // ED - Erase in Display
            'J' => {
                let mode = param(params, 0, 0);
                grid.erase_display(mode);
            }
            // EL - Erase in Line
            'K' => {
                let mode = param(params, 0, 0);
                grid.erase_line(mode);
            }
            // IL - Insert Lines
            'L' => {
                let n = param(params, 0, 1) as usize;
                grid.insert_lines(n);
            }
            // DL - Delete Lines
            'M' => {
                let n = param(params, 0, 1) as usize;
                grid.delete_lines(n);
            }
            // DCH - Delete Characters
            'P' => {
                let n = param(params, 0, 1) as usize;
                grid.delete_chars(n);
            }
            // SU - Scroll Up
            'S' => {
                let n = param(params, 0, 1) as usize;
                grid.scroll_up(n);
            }
            // SD - Scroll Down
            'T' => {
                let n = param(params, 0, 1) as usize;
                grid.scroll_down(n);
            }
            // ECH - Erase Characters
            'X' => {
                let n = param(params, 0, 1) as usize;
                grid.erase_chars(n);
            }
            // ICH - Insert Characters
            '@' => {
                let n = param(params, 0, 1) as usize;
                grid.insert_chars(n);
            }
            // REP - Repeat preceding graphic character
            'b' => {
                let n = param(params, 0, 1) as usize;
                grid.repeat_char(n);
            }
            // VPA - Vertical Line Position Absolute
            'd' => {
                let row = param(params, 0, 1) as usize;
                let col = grid.cursor.col;
                grid.cursor_to(row.saturating_sub(1), col);
            }
            // SGR - Select Graphic Rendition
            'm' => {
                let sgr_params = collect_params(params);
                Self::apply_sgr(&mut grid, &sgr_params);
            }
            // DECSTBM - Set Scrolling Region
            'r' => {
                if !private_mode {
                    let top = param(params, 0, 1) as usize;
                    let bottom = param(params, 1, grid.rows as u16) as usize;
                    grid.set_scroll_region(
                        top.saturating_sub(1),
                        bottom.saturating_sub(1),
                    );
                }
            }
            // Save cursor position
            's' => {
                if !private_mode {
                    grid.save_cursor();
                }
            }
            // Restore cursor position
            'u' => {
                grid.restore_cursor();
            }
            // SM/RM - Set/Reset Mode
            'h' => {
                if private_mode {
                    for sub in params.iter() {
                        for &p in sub {
                            match p {
                                25 => grid.set_cursor_visible(true),
                                1049 => grid.enter_alt_screen(),
                                47 | 1047 => grid.enter_alt_screen(),
                                // Mouse tracking modes
                                1000 => grid.mouse_mode = MouseMode::Press,
                                1002 => grid.mouse_mode = MouseMode::ButtonTracking,
                                1003 => grid.mouse_mode = MouseMode::AnyMotion,
                                1006 => grid.mouse_encoding = MouseEncoding::Sgr,
                                2004 => grid.set_bracketed_paste(true),
                                _ => {}
                            }
                        }
                    }
                }
            }
            'l' => {
                if private_mode {
                    for sub in params.iter() {
                        for &p in sub {
                            match p {
                                25 => grid.set_cursor_visible(false),
                                1049 => grid.leave_alt_screen(),
                                47 | 1047 => grid.leave_alt_screen(),
                                // Mouse tracking modes off
                                1000 | 1002 | 1003 => grid.mouse_mode = MouseMode::None,
                                1006 => grid.mouse_encoding = MouseEncoding::Normal,
                                2004 => grid.set_bracketed_paste(false),
                                _ => {}
                            }
                        }
                    }
                }
            }
            // Device Status Report (DSR) - we can't respond from here so ignore.
            'n' => {}
            // Tab clear
            'g' => {
                // 0 = clear current tab stop, 3 = clear all
                // We don't need to implement this for basic terminal use.
            }
            _ => {
                tracing::trace!(
                    "Unhandled CSI: params={:?} intermediates={:?} action={}",
                    collect_params(params),
                    intermediates,
                    action
                );
            }
        }
    }

    fn osc_dispatch(&mut self, params: &[&[u8]], _bell_terminated: bool) {
        if params.is_empty() {
            return;
        }

        // Parse the first param as the OSC command number.
        let cmd = params[0];
        let cmd_str = std::str::from_utf8(cmd).unwrap_or("");
        let cmd_num: Option<u8> = cmd_str.parse().ok();

        match cmd_num {
            // Set window title.
            Some(0) | Some(2) => {
                if params.len() > 1 {
                    let title = params[1..]
                        .iter()
                        .filter_map(|p| std::str::from_utf8(p).ok())
                        .collect::<Vec<_>>()
                        .join(";");
                    self.grid.lock().set_title(title);
                }
            }
            // OSC 52 - clipboard (ignore for now).
            Some(52) => {}
            // OSC 4 - set color palette (ignore).
            Some(4) => {}
            // OSC 7 - current directory (ignore).
            Some(7) => {}
            // OSC 8 - hyperlinks (ignore).
            Some(8) => {}
            _ => {
                tracing::trace!("Unhandled OSC: {:?}", cmd_str);
            }
        }
    }

    fn hook(
        &mut self,
        _params: &vte::Params,
        _intermediates: &[u8],
        _ignore: bool,
        _action: char,
    ) {
        // DCS sequences - not commonly needed for basic terminal emulation.
    }

    fn put(&mut self, _byte: u8) {
        // DCS data bytes.
    }

    fn unhook(&mut self) {
        // End of DCS sequence.
    }

    fn esc_dispatch(&mut self, intermediates: &[u8], _ignore: bool, byte: u8) {
        let mut grid = self.grid.lock();
        match byte {
            // DECSC - Save Cursor
            b'7' => grid.save_cursor(),
            // DECRC - Restore Cursor
            b'8' => grid.restore_cursor(),
            // IND - Index (move cursor down, scroll if at bottom)
            b'D' => grid.index(),
            // NEL - Next Line
            b'E' => {
                grid.carriage_return();
                grid.newline();
            }
            // RI - Reverse Index (move cursor up, scroll if at top)
            b'M' => grid.reverse_index(),
            // RIS - Full Reset
            b'c' => grid.reset(),
            // DECKPAM - Keypad Application Mode (ignore)
            b'=' => {}
            // DECKPNM - Keypad Numeric Mode (ignore)
            b'>' => {}
            // SCS - Select Character Set
            _ if !intermediates.is_empty() => {
                use crate::grid::Charset;
                let charset = match byte {
                    b'0' => Some(Charset::DecSpecialGraphics),
                    b'B' => Some(Charset::Ascii),
                    _ => None,
                };
                if let Some(cs) = charset {
                    match intermediates.first() {
                        Some(b'(') => grid.set_charset_g0(cs),
                        Some(b')') => grid.set_charset_g1(cs),
                        _ => {}
                    }
                }
            }
            _ => {
                tracing::trace!(
                    "Unhandled ESC: intermediates={:?} byte=0x{:02x} ({})",
                    intermediates,
                    byte,
                    byte as char,
                );
            }
        }
    }
}

