use crate::colors::{NamedColor, TermColor};
use crate::grid::{
    CursorShape, MouseEncoding, MouseMode, PromptMark, TerminalGrid, UnderlineStyle,
};
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
    ///
    /// This method works with the raw `vte::Params` to correctly distinguish
    /// colon-separated sub-parameters (e.g. `4:3` for curly underline) from
    /// semicolon-separated parameters (e.g. `4;3` for underline + italic).
    ///
    /// In vte 0.13, `Params::iter()` yields `&[u16]` slices where each slice
    /// is one semicolon-separated parameter group. Within a group, colon-
    /// separated sub-parameters share the same slice.
    fn apply_sgr(grid: &mut TerminalGrid, params: &vte::Params) {
        let groups: Vec<&[u16]> = params.iter().collect();

        if groups.is_empty() {
            // No params = reset.
            grid.current_attrs = Default::default();
            grid.current_fg = TermColor::Default;
            grid.current_bg = TermColor::Default;
            return;
        }

        let mut i = 0;
        while i < groups.len() {
            let group = groups[i];
            let code = group[0];

            match code {
                0 => {
                    grid.current_attrs = Default::default();
                    grid.current_fg = TermColor::Default;
                    grid.current_bg = TermColor::Default;
                }
                1 => grid.current_attrs.bold = true,
                2 => grid.current_attrs.dim = true,
                3 => grid.current_attrs.italic = true,
                4 => {
                    // SGR 4 with possible colon sub-parameters for underline style.
                    // `4` alone or `4:1` = single, `4:0` = none, `4:2` = double,
                    // `4:3` = curly, `4:4` = dotted, `4:5` = dashed.
                    if group.len() > 1 {
                        // Colon sub-parameter present (e.g. `4:3`).
                        grid.current_attrs.underline = match group[1] {
                            0 => UnderlineStyle::None,
                            1 => UnderlineStyle::Single,
                            2 => UnderlineStyle::Double,
                            3 => UnderlineStyle::Curly,
                            4 => UnderlineStyle::Dotted,
                            5 => UnderlineStyle::Dashed,
                            _ => UnderlineStyle::Single,
                        };
                    } else {
                        // Plain `4` = single underline.
                        grid.current_attrs.underline = UnderlineStyle::Single;
                    }
                }
                5 | 6 => grid.current_attrs.blink = true,
                7 => grid.current_attrs.inverse = true,
                8 => grid.current_attrs.hidden = true,
                9 => grid.current_attrs.strikethrough = true,

                21 => grid.current_attrs.underline = UnderlineStyle::Double,
                22 => {
                    // Normal intensity: resets both bold and dim.
                    grid.current_attrs.bold = false;
                    grid.current_attrs.dim = false;
                }
                23 => grid.current_attrs.italic = false,
                24 => grid.current_attrs.underline = UnderlineStyle::None,
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
                // Also supports colon sub-params: 38:5:N or 38:2:R:G:B.
                38 => {
                    if group.len() > 1 {
                        if let Some(color) = parse_extended_color_subparams(group) {
                            grid.current_fg = color;
                        }
                    } else if let Some(color) = parse_extended_color_groups(&groups, &mut i) {
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
                // Also supports colon sub-params: 48:5:N or 48:2:R:G:B.
                48 => {
                    if group.len() > 1 {
                        if let Some(color) = parse_extended_color_subparams(group) {
                            grid.current_bg = color;
                        }
                    } else if let Some(color) = parse_extended_color_groups(&groups, &mut i) {
                        grid.current_bg = color;
                    }
                }

                49 => grid.current_bg = TermColor::Default,

                // SGR 53: overline on.
                53 => grid.current_attrs.overline = true,
                // SGR 55: overline off.
                55 => grid.current_attrs.overline = false,

                // SGR 58: set underline color.
                // Supports colon sub-params: 58:5:N or 58:2:R:G:B.
                // Also semicolon-separated: 58;5;N or 58;2;R;G;B.
                58 => {
                    if group.len() > 1 {
                        if let Some(color) = parse_extended_color_subparams(group) {
                            grid.current_attrs.underline_color = Some(color);
                        }
                    } else if let Some(color) = parse_extended_color_groups(&groups, &mut i) {
                        grid.current_attrs.underline_color = Some(color);
                    }
                }

                // SGR 59: reset underline color to default.
                59 => grid.current_attrs.underline_color = None,

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

/// Parse extended color from colon-separated sub-parameters within a single group.
/// The group slice is e.g. `[38, 5, N]` or `[38, 2, R, G, B]`.
fn parse_extended_color_subparams(group: &[u16]) -> Option<TermColor> {
    if group.len() < 2 {
        return None;
    }
    match group[1] {
        5 => {
            // 256-color: code:5:N
            if group.len() >= 3 {
                Some(TermColor::Indexed(group[2] as u8))
            } else {
                None
            }
        }
        2 => {
            // True color: code:2:R:G:B (or code:2:colorspace:R:G:B)
            if group.len() >= 6 {
                // code:2:colorspace:R:G:B — skip colorspace at index 2.
                let r = group[3] as u8;
                let g = group[4] as u8;
                let b = group[5] as u8;
                Some(TermColor::Rgb(r, g, b))
            } else if group.len() >= 5 {
                // code:2:R:G:B — no colorspace.
                let r = group[2] as u8;
                let g = group[3] as u8;
                let b = group[4] as u8;
                Some(TermColor::Rgb(r, g, b))
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Parse extended color from semicolon-separated groups.
/// `i` points at the group containing `38`, `48`, or `58`.
/// On success, `i` is advanced past the consumed groups.
fn parse_extended_color_groups(groups: &[&[u16]], i: &mut usize) -> Option<TermColor> {
    if *i + 1 >= groups.len() {
        return None;
    }
    let mode = groups[*i + 1][0];
    match mode {
        5 => {
            // 256-color mode: code;5;N
            if *i + 2 < groups.len() {
                let idx = groups[*i + 2][0] as u8;
                *i += 2;
                Some(TermColor::Indexed(idx))
            } else {
                *i += 1;
                None
            }
        }
        2 => {
            // True color mode: code;2;R;G;B
            if *i + 4 < groups.len() {
                let r = groups[*i + 2][0] as u8;
                let g = groups[*i + 3][0] as u8;
                let b = groups[*i + 4][0] as u8;
                *i += 4;
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

/// Parse an OSC color specification into (r, g, b).
/// Supports:
/// - `rgb:RR/GG/BB` (hex, each component 2 or 4 hex digits)
/// - `#RRGGBB`
fn parse_osc_color(spec: &str) -> Option<(u8, u8, u8)> {
    if let Some(rest) = spec.strip_prefix("rgb:") {
        // Format: rgb:RR/GG/BB or rgb:RRRR/GGGG/BBBB
        let parts: Vec<&str> = rest.split('/').collect();
        if parts.len() == 3 {
            let r = parse_hex_component(parts[0])?;
            let g = parse_hex_component(parts[1])?;
            let b = parse_hex_component(parts[2])?;
            return Some((r, g, b));
        }
    } else if let Some(rest) = spec.strip_prefix('#') {
        // Format: #RRGGBB
        if rest.len() == 6 {
            let r = u8::from_str_radix(&rest[0..2], 16).ok()?;
            let g = u8::from_str_radix(&rest[2..4], 16).ok()?;
            let b = u8::from_str_radix(&rest[4..6], 16).ok()?;
            return Some((r, g, b));
        }
    }
    None
}

/// Parse a hex color component. If it's 4 digits (e.g., "FFFF"), scale down
/// to 8-bit by taking the high byte. If it's 2 digits, parse directly.
fn parse_hex_component(s: &str) -> Option<u8> {
    match s.len() {
        2 => u8::from_str_radix(s, 16).ok(),
        4 => {
            let val = u16::from_str_radix(s, 16).ok()?;
            Some((val >> 8) as u8)
        }
        1 => {
            // Single hex digit: replicate (e.g., "F" -> 0xFF)
            let val = u8::from_str_radix(s, 16).ok()?;
            Some(val << 4 | val)
        }
        _ => None,
    }
}

/// Minimal base64 decoder (standard alphabet, no padding required).
/// Returns the decoded string, or None if decoding fails.
fn base64_decode(input: &str) -> Option<String> {
    const TABLE: [u8; 256] = {
        let mut t = [255u8; 256];
        let mut i = 0u8;
        // A-Z = 0..25
        while i < 26 {
            t[(b'A' + i) as usize] = i;
            i += 1;
        }
        // a-z = 26..51
        i = 0;
        while i < 26 {
            t[(b'a' + i) as usize] = 26 + i;
            i += 1;
        }
        // 0-9 = 52..61
        i = 0;
        while i < 10 {
            t[(b'0' + i) as usize] = 52 + i;
            i += 1;
        }
        t[b'+' as usize] = 62;
        t[b'/' as usize] = 63;
        t
    };

    let bytes: Vec<u8> = input
        .bytes()
        .filter(|&b| b != b'=' && b != b'\n' && b != b'\r')
        .collect();
    let mut output = Vec::with_capacity(bytes.len() * 3 / 4);

    let mut i = 0;
    while i < bytes.len() {
        let a = TABLE[bytes[i] as usize];
        if a == 255 {
            return None;
        }
        let b = if i + 1 < bytes.len() {
            TABLE[bytes[i + 1] as usize]
        } else {
            return None;
        };
        if b == 255 {
            return None;
        }
        output.push((a << 2) | (b >> 4));

        if i + 2 < bytes.len() {
            let c = TABLE[bytes[i + 2] as usize];
            if c == 255 {
                return None;
            }
            output.push((b << 4) | (c >> 2));

            if i + 3 < bytes.len() {
                let d = TABLE[bytes[i + 3] as usize];
                if d == 255 {
                    return None;
                }
                output.push((c << 6) | d);
            }
        }

        i += 4;
    }

    String::from_utf8(output).ok()
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
            0x0A..=0x0C => grid.newline(), // LF, VT, FF all treated as newline
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
                Self::apply_sgr(&mut grid, params);
            }
            // DECSTBM - Set Scrolling Region
            'r' => {
                if !private_mode {
                    let top = param(params, 0, 1) as usize;
                    let bottom = param(params, 1, grid.rows as u16) as usize;
                    grid.set_scroll_region(top.saturating_sub(1), bottom.saturating_sub(1));
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
                                1 => grid.set_application_cursor_keys(true),
                                // DECOM - Origin Mode
                                6 => grid.set_origin_mode(true),
                                // DECAWM - Auto-Wrap Mode
                                7 => grid.set_auto_wrap(true),
                                // Cursor blink
                                12 => grid.cursor.blink = true,
                                25 => grid.set_cursor_visible(true),
                                1049 => grid.enter_alt_screen(),
                                47 | 1047 => grid.enter_alt_screen(),
                                // Mouse tracking modes
                                1000 => grid.mouse_mode = MouseMode::Press,
                                1002 => grid.mouse_mode = MouseMode::ButtonTracking,
                                1003 => grid.mouse_mode = MouseMode::AnyMotion,
                                1006 => grid.mouse_encoding = MouseEncoding::Sgr,
                                // Focus event reporting
                                1004 => grid.set_focus_reporting(true),
                                // Save cursor (DECSET 1048)
                                1048 => grid.save_cursor(),
                                2004 => grid.set_bracketed_paste(true),
                                // Synchronized output
                                2026 => grid.set_synchronized_output(true),
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
                                1 => grid.set_application_cursor_keys(false),
                                // DECOM - Origin Mode
                                6 => grid.set_origin_mode(false),
                                // DECAWM - Auto-Wrap Mode
                                7 => grid.set_auto_wrap(false),
                                // Cursor blink off
                                12 => grid.cursor.blink = false,
                                25 => grid.set_cursor_visible(false),
                                1049 => grid.leave_alt_screen(),
                                47 | 1047 => grid.leave_alt_screen(),
                                // Mouse tracking modes off
                                1000 | 1002 | 1003 => grid.mouse_mode = MouseMode::None,
                                1006 => grid.mouse_encoding = MouseEncoding::Normal,
                                // Focus event reporting off
                                1004 => grid.set_focus_reporting(false),
                                // Restore cursor (DECRST 1048)
                                1048 => grid.restore_cursor(),
                                2004 => grid.set_bracketed_paste(false),
                                // Synchronized output off
                                2026 => grid.set_synchronized_output(false),
                                _ => {}
                            }
                        }
                    }
                }
            }
            // DA - Device Attributes
            'c' => {
                let has_gt = intermediates.first().copied() == Some(b'>');
                if has_gt {
                    // DA2 - Secondary Device Attributes (CSI > c or CSI > 0 c)
                    let p = param(params, 0, 0);
                    if p == 0 {
                        // Respond: VT100, version 1, ROM 0
                        grid.write_response(b"\x1b[>0;1;0c".to_vec());
                    }
                } else if !private_mode {
                    // DA1 - Primary Device Attributes (CSI c or CSI 0 c)
                    let p = param(params, 0, 0);
                    if p == 0 {
                        // Respond: VT220 with ANSI color support
                        grid.write_response(b"\x1b[?62;22c".to_vec());
                    }
                }
            }
            // DSR - Device Status Report
            'n' => {
                if !private_mode {
                    let p = param(params, 0, 0);
                    match p {
                        5 => {
                            // Device status report - terminal OK
                            grid.write_response(b"\x1b[0n".to_vec());
                        }
                        6 => {
                            // Cursor position report (CPR)
                            let row = grid.cursor.row + 1;
                            let col = grid.cursor.col + 1;
                            grid.write_response(format!("\x1b[{};{}R", row, col).into_bytes());
                        }
                        _ => {}
                    }
                }
            }
            // DECRPM - Mode Report (CSI ? {mode} $ p)
            'p' => {
                if private_mode && intermediates.contains(&b'$') {
                    // This is actually intermediates=['?', '$'] for DECRPM
                    // but vte may deliver '?' as private and '$' in intermediates
                    let mode = param(params, 0, 0);
                    let status = match mode {
                        1 => {
                            // DECCKM - application cursor keys
                            if grid.application_cursor_keys() {
                                1
                            } else {
                                2
                            }
                        }
                        6 => {
                            // DECOM - origin mode
                            if grid.origin_mode() {
                                1
                            } else {
                                2
                            }
                        }
                        7 => {
                            // DECAWM - auto-wrap mode
                            if grid.auto_wrap() {
                                1
                            } else {
                                2
                            }
                        }
                        12 => {
                            // Cursor blink
                            if grid.cursor.blink {
                                1
                            } else {
                                2
                            }
                        }
                        25 => {
                            // DECTCEM - cursor visible
                            if grid.cursor.visible {
                                1
                            } else {
                                2
                            }
                        }
                        47 => {
                            // Alternate screen buffer
                            2 // report as reset (we track via 1049)
                        }
                        1000 => {
                            // Mouse press tracking
                            if grid.mouse_mode == MouseMode::Press {
                                1
                            } else {
                                2
                            }
                        }
                        1002 => {
                            // Mouse button tracking
                            if grid.mouse_mode == MouseMode::ButtonTracking {
                                1
                            } else {
                                2
                            }
                        }
                        1003 => {
                            // Mouse any-motion tracking
                            if grid.mouse_mode == MouseMode::AnyMotion {
                                1
                            } else {
                                2
                            }
                        }
                        1006 => {
                            // SGR mouse encoding
                            if grid.mouse_encoding == MouseEncoding::Sgr {
                                1
                            } else {
                                2
                            }
                        }
                        1049 => {
                            // Alternate screen buffer (with save/restore cursor)
                            2 // not easily queryable; report reset
                        }
                        1004 => {
                            // Focus event reporting
                            if grid.focus_reporting() {
                                1
                            } else {
                                2
                            }
                        }
                        1048 => {
                            // Save/restore cursor - not directly queryable as a toggle
                            2
                        }
                        2004 => {
                            // Bracketed paste mode
                            if grid.bracketed_paste() {
                                1
                            } else {
                                2
                            }
                        }
                        2026 => {
                            // Synchronized output
                            if grid.synchronized_output() {
                                1
                            } else {
                                2
                            }
                        }
                        _ => 0, // not recognized
                    };
                    grid.write_response(format!("\x1b[?{};{}$y", mode, status).into_bytes());
                }
            }
            // TBC - Tab Clear
            'g' => {
                let mode = param(params, 0, 0) as u8;
                grid.clear_tab_stop(mode);
            }
            // CHT - Cursor Horizontal Tab (advance cursor by N tab stops)
            'I' => {
                let n = param(params, 0, 1) as usize;
                for _ in 0..n {
                    let col = grid.cursor.col;
                    grid.cursor.col = grid.next_tab_stop(col);
                }
                grid.dirty = true;
            }
            // CBT - Cursor Backward Tab (move cursor back by N tab stops)
            'Z' => {
                let n = param(params, 0, 1) as usize;
                for _ in 0..n {
                    let col = grid.cursor.col;
                    grid.cursor.col = grid.prev_tab_stop(col);
                }
                grid.dirty = true;
            }
            // DECSCUSR - Set Cursor Shape (CSI Ps SP q)
            'q' => {
                if intermediates.first().copied() == Some(b' ') {
                    let ps = param(params, 0, 0);
                    match ps {
                        0 | 1 => {
                            // Blinking block (default)
                            grid.cursor.shape = CursorShape::Block;
                            grid.cursor.blink = true;
                        }
                        2 => {
                            // Steady block
                            grid.cursor.shape = CursorShape::Block;
                            grid.cursor.blink = false;
                        }
                        3 => {
                            // Blinking underline
                            grid.cursor.shape = CursorShape::Underline;
                            grid.cursor.blink = true;
                        }
                        4 => {
                            // Steady underline
                            grid.cursor.shape = CursorShape::Underline;
                            grid.cursor.blink = false;
                        }
                        5 => {
                            // Blinking bar
                            grid.cursor.shape = CursorShape::Bar;
                            grid.cursor.blink = true;
                        }
                        6 => {
                            // Steady bar
                            grid.cursor.shape = CursorShape::Bar;
                            grid.cursor.blink = false;
                        }
                        _ => {}
                    }
                    grid.dirty = true;
                }
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
        let cmd_num: Option<u16> = cmd_str.parse().ok();

        // Reconstruct the payload from params[1..] joined by ';'.
        let payload = || -> String {
            params[1..]
                .iter()
                .filter_map(|p| std::str::from_utf8(p).ok())
                .collect::<Vec<_>>()
                .join(";")
        };

        match cmd_num {
            // OSC 0 / OSC 2 - Set window title.
            Some(0) | Some(2) => {
                if params.len() > 1 {
                    self.grid.lock().set_title(payload());
                }
            }
            // OSC 4 - Set color palette entry.
            // Format: 4;index;colorspec
            Some(4) => {
                if params.len() >= 3 {
                    let index_str = std::str::from_utf8(params[1]).unwrap_or("");
                    let color_str = std::str::from_utf8(params[2]).unwrap_or("");
                    if let Ok(index) = index_str.parse::<u8>() {
                        if let Some((r, g, b)) = parse_osc_color(color_str) {
                            self.grid.lock().set_palette_color(index, r, g, b);
                        }
                    }
                }
            }
            // OSC 7 - Current working directory.
            // Format: 7;file://hostname/path/to/dir
            Some(7) => {
                if params.len() > 1 {
                    let url = payload();
                    if let Some(path) = url.strip_prefix("file://") {
                        // Skip hostname part (up to first / after //)
                        if let Some(slash_pos) = path.find('/') {
                            self.grid.lock().working_directory =
                                Some(path[slash_pos..].to_string());
                        }
                    }
                }
            }
            // OSC 8 - Hyperlinks.
            // Format: 8;params;URI to start, 8;; to end.
            Some(8) => {
                // params[0] = "8", params[1] = link params, params[2] = URI
                // When URI is empty, the hyperlink is cleared.
                let uri = if params.len() >= 3 {
                    std::str::from_utf8(params[2]).unwrap_or("").to_string()
                } else if params.len() == 2 {
                    // Could be 8;; with empty URI split as just params[1]=""
                    String::new()
                } else {
                    String::new()
                };

                let mut grid = self.grid.lock();
                if uri.is_empty() {
                    grid.hyperlink = None;
                } else {
                    grid.hyperlink = Some(uri);
                }
            }
            // OSC 52 - Clipboard operations.
            // Format: 52;selection;base64data
            Some(52) => {
                if params.len() >= 3 {
                    let selection = std::str::from_utf8(params[1]).unwrap_or("c");
                    let data = std::str::from_utf8(params[2]).unwrap_or("");
                    if data != "?" {
                        // Decode base64 data
                        if let Some(decoded) = base64_decode(data) {
                            self.grid.lock().clipboard_request =
                                Some((selection.to_string(), decoded));
                        }
                    }
                }
            }
            // OSC 133 - Shell integration / prompt markers.
            // Format: 133;A, 133;B, 133;C, 133;D[;exitcode]
            Some(133) => {
                if params.len() > 1 {
                    let marker = std::str::from_utf8(params[1]).unwrap_or("");
                    let mut grid = self.grid.lock();
                    match marker.chars().next() {
                        Some('A') => {
                            grid.prompt_mark = Some(PromptMark::PromptStart);
                            // Record the current cursor row as a prompt line.
                            let row = grid.cursor.row;
                            grid.prompt_lines.push(row);
                        }
                        Some('B') => {
                            grid.prompt_mark = Some(PromptMark::CommandStart);
                        }
                        Some('C') => {
                            grid.prompt_mark = Some(PromptMark::CommandExecuted);
                        }
                        Some('D') => {
                            // Exit code may follow after a semicolon.
                            // params[1] could be "D" with exit code in params[2],
                            // or it could be "D;exitcode" as a single string.
                            let exit_code = if params.len() > 2 {
                                std::str::from_utf8(params[2])
                                    .ok()
                                    .and_then(|s| s.parse::<i32>().ok())
                            } else {
                                marker.get(2..).and_then(|s| s.parse::<i32>().ok())
                            };
                            grid.prompt_mark = Some(PromptMark::CommandFinished(exit_code));
                        }
                        _ => {}
                    }
                }
            }
            _ => {
                tracing::trace!("Unhandled OSC: {:?}", cmd_str);
            }
        }
    }

    fn hook(&mut self, _params: &vte::Params, _intermediates: &[u8], _ignore: bool, _action: char) {
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
            // DECKPAM - Keypad Application Mode
            b'=' => grid.set_application_keypad(true),
            // HTS - Horizontal Tab Set (set tab stop at current column)
            // Only when no intermediates (ESC H without intermediates).
            // Note: CSI H (cursor position) is handled in csi_dispatch, not here.
            b'H' if intermediates.is_empty() => grid.set_tab_stop(),
            // DECKPNM - Keypad Numeric Mode
            b'>' => grid.set_application_keypad(false),
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
