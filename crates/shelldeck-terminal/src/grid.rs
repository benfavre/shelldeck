use crate::colors::TermColor;
use regex::Regex;

// ---------------------------------------------------------------------------
// Selection
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectionKind {
    Simple,
    Word,
    Line,
}

/// Grid position: (column, row) in 0-indexed display coordinates.
/// Row 0 is the topmost visible row (may be scrollback).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GridPos {
    pub col: usize,
    pub row: usize,
}

impl GridPos {
    pub fn new(col: usize, row: usize) -> Self {
        Self { col, row }
    }
}

impl PartialOrd for GridPos {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for GridPos {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.row.cmp(&other.row).then(self.col.cmp(&other.col))
    }
}

#[derive(Debug, Clone)]
pub struct SelectionState {
    pub start: GridPos,
    pub end: GridPos,
    pub active: bool,
    pub kind: SelectionKind,
}

// ---------------------------------------------------------------------------
// Search
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct SearchMatch {
    /// Display row (0-indexed into visible rows).
    pub row: usize,
    /// Column offset (0-indexed, byte position in the row text).
    pub col: usize,
    /// Length of the match in characters.
    pub len: usize,
}

// ---------------------------------------------------------------------------
// Cell types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CellAttributes {
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strikethrough: bool,
    pub blink: bool,
    pub inverse: bool,
    pub hidden: bool,
}

impl Default for CellAttributes {
    fn default() -> Self {
        Self {
            bold: false,
            italic: false,
            underline: false,
            strikethrough: false,
            blink: false,
            inverse: false,
            hidden: false,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Cell {
    pub c: char,
    pub fg: TermColor,
    pub bg: TermColor,
    pub attrs: CellAttributes,
}

impl Default for Cell {
    fn default() -> Self {
        Self {
            c: ' ',
            fg: TermColor::Default,
            bg: TermColor::Default,
            attrs: CellAttributes::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CursorShape {
    Block,
    Underline,
    Bar,
}

// ---------------------------------------------------------------------------
// Charset (DEC Special Graphics support)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Charset {
    Ascii,
    DecSpecialGraphics,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CharsetSlot {
    G0,
    G1,
}

/// Translate an ASCII character to its DEC Special Graphics equivalent.
/// Characters in the range 0x60..=0x7E are mapped to Unicode box-drawing
/// and other symbols; everything else passes through unchanged.
fn dec_special_char(c: char) -> char {
    match c {
        '`' => '\u{25C6}', // ◆ diamond
        'a' => '\u{2592}', // ▒ checkerboard
        'b' => '\u{2409}', // ␉ HT
        'c' => '\u{240C}', // ␌ FF
        'd' => '\u{240D}', // ␍ CR
        'e' => '\u{240A}', // ␊ LF
        'f' => '\u{00B0}', // ° degree
        'g' => '\u{00B1}', // ± plus/minus
        'h' => '\u{2424}', // ␤ NL
        'i' => '\u{240B}', // ␋ VT
        'j' => '\u{2518}', // ┘ lower-right corner
        'k' => '\u{2510}', // ┐ upper-right corner
        'l' => '\u{250C}', // ┌ upper-left corner
        'm' => '\u{2514}', // └ lower-left corner
        'n' => '\u{253C}', // ┼ crossing lines
        'o' => '\u{23BA}', // ⎺ scan line 1
        'p' => '\u{23BB}', // ⎻ scan line 3
        'q' => '\u{2500}', // ─ horizontal line (scan line 5)
        'r' => '\u{23BC}', // ⎼ scan line 7
        's' => '\u{23BD}', // ⎽ scan line 9
        't' => '\u{251C}', // ├ left tee
        'u' => '\u{2524}', // ┤ right tee
        'v' => '\u{2534}', // ┴ bottom tee
        'w' => '\u{252C}', // ┬ top tee
        'x' => '\u{2502}', // │ vertical line
        'y' => '\u{2264}', // ≤ less-than-or-equal
        'z' => '\u{2265}', // ≥ greater-than-or-equal
        '{' => '\u{03C0}', // π pi
        '|' => '\u{2260}', // ≠ not-equal
        '}' => '\u{00A3}', // £ pound sign
        '~' => '\u{00B7}', // · centered dot
        _ => c,
    }
}

#[derive(Debug, Clone)]
pub struct CursorState {
    pub row: usize,
    pub col: usize,
    pub visible: bool,
    pub shape: CursorShape,
}

impl Default for CursorState {
    fn default() -> Self {
        Self {
            row: 0,
            col: 0,
            visible: true,
            shape: CursorShape::Block,
        }
    }
}

/// Mouse tracking mode set by the application running in the terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseMode {
    /// No mouse reporting.
    None,
    /// Mode 1000: report button press and release.
    Press,
    /// Mode 1002: report press, release, and drag with button held.
    ButtonTracking,
    /// Mode 1003: report all motion, even without buttons.
    AnyMotion,
}

/// Mouse coordinate encoding format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseEncoding {
    /// Default X10 encoding (limited to 223 cols/rows).
    Normal,
    /// Mode 1006: SGR extended encoding (no coordinate limit).
    Sgr,
}

pub struct TerminalGrid {
    pub cells: Vec<Vec<Cell>>,
    pub cursor: CursorState,
    pub rows: usize,
    pub cols: usize,
    pub title: String,
    scrollback: Vec<Vec<Cell>>,
    max_scrollback: usize,
    scroll_offset: usize,
    scroll_top: usize,
    scroll_bottom: usize,
    saved_cursor: Option<(usize, usize)>,
    pub current_attrs: CellAttributes,
    pub current_fg: TermColor,
    pub current_bg: TermColor,
    tab_stops: Vec<bool>,
    pub dirty: bool,
    /// Alternate screen buffer (used by fullscreen apps like vim, less)
    alt_cells: Option<Vec<Vec<Cell>>>,
    alt_cursor: Option<CursorState>,
    /// Auto-wrap mode: when the cursor reaches the right edge, the next
    /// character will wrap to a new line. Default true.
    auto_wrap: bool,
    /// Tracks whether the cursor is in the "pending wrap" state (just wrote
    /// the last column but hasn't wrapped yet).
    pending_wrap: bool,
    /// Mouse tracking mode requested by the terminal application.
    pub mouse_mode: MouseMode,
    /// Mouse coordinate encoding format.
    pub mouse_encoding: MouseEncoding,
    /// Bracketed paste mode (mode 2004): when enabled, pasted text is
    /// bracketed with ESC[200~ ... ESC[201~.
    bracketed_paste_mode: bool,
    /// Text selection state.
    pub selection: Option<SelectionState>,
    /// G0 character set designation.
    charset_g0: Charset,
    /// G1 character set designation.
    charset_g1: Charset,
    /// Which charset slot (G0 or G1) is currently active.
    active_charset: CharsetSlot,
    /// Last printed character (for REP — CSI Pb b).
    last_char: char,
}

impl TerminalGrid {
    pub fn new(rows: usize, cols: usize) -> Self {
        let rows = rows.max(1);
        let cols = cols.max(1);

        let cells: Vec<Vec<Cell>> = (0..rows)
            .map(|_| vec![Cell::default(); cols])
            .collect();

        let mut tab_stops = vec![false; cols];
        for i in (0..cols).step_by(8) {
            tab_stops[i] = true;
        }

        Self {
            cells,
            cursor: CursorState::default(),
            rows,
            cols,
            title: String::new(),
            scrollback: Vec::new(),
            max_scrollback: 10_000,
            scroll_offset: 0,
            scroll_top: 0,
            scroll_bottom: rows.saturating_sub(1),
            saved_cursor: None,
            current_attrs: CellAttributes::default(),
            current_fg: TermColor::Default,
            current_bg: TermColor::Default,
            tab_stops,
            dirty: true,
            alt_cells: None,
            alt_cursor: None,
            auto_wrap: true,
            pending_wrap: false,
            mouse_mode: MouseMode::None,
            mouse_encoding: MouseEncoding::Normal,
            bracketed_paste_mode: false,
            selection: None,
            charset_g0: Charset::Ascii,
            charset_g1: Charset::Ascii,
            active_charset: CharsetSlot::G0,
            last_char: ' ',
        }
    }

    /// Write a character at the current cursor position and advance the cursor.
    pub fn write_char(&mut self, c: char) {
        // If we are in the pending-wrap state, wrap first.
        if self.pending_wrap {
            self.cursor.col = 0;
            if self.cursor.row == self.scroll_bottom {
                self.scroll_up(1);
            } else if self.cursor.row < self.rows - 1 {
                self.cursor.row += 1;
            }
            self.pending_wrap = false;
        }

        // Translate through the active charset (DEC Special Graphics).
        let c = if self.active_charset_is_dec_special() {
            dec_special_char(c)
        } else {
            c
        };

        self.ensure_row(self.cursor.row);

        let col = self.cursor.col.min(self.cols - 1);
        let row = self.cursor.row;

        self.cells[row][col] = Cell {
            c,
            fg: self.current_fg,
            bg: self.current_bg,
            attrs: self.current_attrs,
        };

        self.last_char = c;

        if col + 1 < self.cols {
            self.cursor.col = col + 1;
        } else if self.auto_wrap {
            // Entered pending wrap state - don't move yet, wait for next char.
            self.pending_wrap = true;
        }
        // If auto_wrap is false and we're at the last column, cursor stays.

        self.dirty = true;
    }

    /// Repeat the last printed character `n` times (REP — CSI Pb b).
    pub fn repeat_char(&mut self, n: usize) {
        let c = self.last_char;
        for _ in 0..n {
            self.write_char(c);
        }
    }

    /// Move cursor down one line, scrolling if necessary.
    pub fn newline(&mut self) {
        self.pending_wrap = false;
        if self.cursor.row == self.scroll_bottom {
            self.scroll_up(1);
        } else if self.cursor.row < self.rows - 1 {
            self.cursor.row += 1;
        }
        self.dirty = true;
    }

    /// Move cursor to column 0.
    pub fn carriage_return(&mut self) {
        self.cursor.col = 0;
        self.pending_wrap = false;
        self.dirty = true;
    }

    /// Move cursor left one position (does not wrap to previous line).
    pub fn backspace(&mut self) {
        if self.cursor.col > 0 {
            self.cursor.col -= 1;
        }
        self.pending_wrap = false;
        self.dirty = true;
    }

    /// Advance cursor to next tab stop (every 8 columns by default).
    pub fn tab(&mut self) {
        self.pending_wrap = false;
        let start = self.cursor.col + 1;
        for i in start..self.cols {
            if self.tab_stops.get(i).copied().unwrap_or(false) {
                self.cursor.col = i;
                self.dirty = true;
                return;
            }
        }
        // No tab stop found - move to last column.
        self.cursor.col = self.cols - 1;
        self.dirty = true;
    }

    /// Bell character - currently a no-op, could trigger a notification.
    pub fn bell(&mut self) {
        // Could integrate with a notification system
        tracing::debug!("BEL");
    }

    // -- Cursor movement --

    pub fn cursor_up(&mut self, n: usize) {
        self.pending_wrap = false;
        let top = self.scroll_top;
        if self.cursor.row >= top + n {
            self.cursor.row -= n;
        } else {
            self.cursor.row = top;
        }
        self.dirty = true;
    }

    pub fn cursor_down(&mut self, n: usize) {
        self.pending_wrap = false;
        let bottom = self.scroll_bottom;
        self.cursor.row = (self.cursor.row + n).min(bottom);
        self.dirty = true;
    }

    pub fn cursor_forward(&mut self, n: usize) {
        self.pending_wrap = false;
        self.cursor.col = (self.cursor.col + n).min(self.cols - 1);
        self.dirty = true;
    }

    pub fn cursor_backward(&mut self, n: usize) {
        self.pending_wrap = false;
        self.cursor.col = self.cursor.col.saturating_sub(n);
        self.dirty = true;
    }

    /// Set absolute cursor position (0-indexed).
    pub fn cursor_to(&mut self, row: usize, col: usize) {
        self.pending_wrap = false;
        self.cursor.row = row.min(self.rows - 1);
        self.cursor.col = col.min(self.cols - 1);
        self.dirty = true;
    }

    pub fn save_cursor(&mut self) {
        self.saved_cursor = Some((self.cursor.row, self.cursor.col));
    }

    pub fn restore_cursor(&mut self) {
        if let Some((row, col)) = self.saved_cursor {
            self.cursor.row = row.min(self.rows - 1);
            self.cursor.col = col.min(self.cols - 1);
            self.pending_wrap = false;
        }
        self.dirty = true;
    }

    // -- Erasing --

    /// Erase display.
    /// mode 0: erase from cursor to end of display.
    /// mode 1: erase from start of display to cursor.
    /// mode 2: erase entire display.
    /// mode 3: erase entire display + scrollback.
    pub fn erase_display(&mut self, mode: u16) {
        let bce = self.bce_cell();
        match mode {
            0 => {
                // Clear from cursor to end of line, then all lines below.
                let row = self.cursor.row;
                let col = self.cursor.col;
                if row < self.rows {
                    for c in col..self.cols {
                        self.cells[row][c] = bce.clone();
                    }
                    for r in (row + 1)..self.rows {
                        self.cells[r] = self.bce_row();
                    }
                }
            }
            1 => {
                // Clear from start of display to cursor.
                let row = self.cursor.row;
                let col = self.cursor.col;
                for r in 0..row {
                    self.cells[r] = self.bce_row();
                }
                if row < self.rows {
                    for c in 0..=col.min(self.cols - 1) {
                        self.cells[row][c] = bce.clone();
                    }
                }
            }
            2 => {
                // Clear entire display.
                for r in 0..self.rows {
                    self.cells[r] = self.bce_row();
                }
            }
            3 => {
                // Clear entire display + scrollback.
                for r in 0..self.rows {
                    self.cells[r] = self.bce_row();
                }
                self.scrollback.clear();
                self.scroll_offset = 0;
            }
            _ => {}
        }
        self.dirty = true;
    }

    /// Erase line.
    /// mode 0: erase from cursor to end of line.
    /// mode 1: erase from start of line to cursor.
    /// mode 2: erase entire line.
    pub fn erase_line(&mut self, mode: u16) {
        let row = self.cursor.row;
        if row >= self.rows {
            return;
        }
        let bce = self.bce_cell();
        match mode {
            0 => {
                for c in self.cursor.col..self.cols {
                    self.cells[row][c] = bce.clone();
                }
            }
            1 => {
                for c in 0..=self.cursor.col.min(self.cols - 1) {
                    self.cells[row][c] = bce.clone();
                }
            }
            2 => {
                self.cells[row] = self.bce_row();
            }
            _ => {}
        }
        self.dirty = true;
    }

    /// Insert `n` blank lines at the cursor row, pushing lines below down.
    /// Lines pushed past the scroll bottom are lost.
    pub fn insert_lines(&mut self, n: usize) {
        self.pending_wrap = false;
        let row = self.cursor.row;
        if row < self.scroll_top || row > self.scroll_bottom {
            return;
        }
        let n = n.min(self.scroll_bottom - row + 1);
        for _ in 0..n {
            if self.scroll_bottom < self.cells.len() {
                self.cells.remove(self.scroll_bottom);
            }
            self.cells.insert(row, self.bce_row());
        }
        // Ensure we still have the right number of rows.
        while self.cells.len() < self.rows {
            self.cells.push(self.bce_row());
        }
        self.dirty = true;
    }

    /// Delete `n` lines at the cursor row, pulling lines below up.
    /// New blank lines appear at the scroll bottom.
    pub fn delete_lines(&mut self, n: usize) {
        self.pending_wrap = false;
        let row = self.cursor.row;
        if row < self.scroll_top || row > self.scroll_bottom {
            return;
        }
        let n = n.min(self.scroll_bottom - row + 1);
        for _ in 0..n {
            if row < self.cells.len() {
                self.cells.remove(row);
            }
            let insert_pos = self.scroll_bottom.min(self.cells.len());
            self.cells.insert(insert_pos, self.bce_row());
        }
        while self.cells.len() < self.rows {
            self.cells.push(self.bce_row());
        }
        self.dirty = true;
    }

    /// Delete `n` characters at the cursor position, shifting remaining
    /// characters on the line to the left. Blank cells fill from the right.
    pub fn delete_chars(&mut self, n: usize) {
        self.pending_wrap = false;
        let row = self.cursor.row;
        let col = self.cursor.col;
        if row >= self.rows || col >= self.cols {
            return;
        }
        let n = n.min(self.cols - col);
        let bce = self.bce_cell();
        for _ in 0..n {
            if col < self.cells[row].len() {
                self.cells[row].remove(col);
            }
            self.cells[row].push(bce.clone());
        }
        // Ensure row length stays correct.
        self.cells[row].truncate(self.cols);
        self.dirty = true;
    }

    /// Insert `n` blank characters at the cursor position, shifting existing
    /// characters to the right. Characters pushed past the right margin are lost.
    pub fn insert_chars(&mut self, n: usize) {
        self.pending_wrap = false;
        let row = self.cursor.row;
        let col = self.cursor.col;
        if row >= self.rows || col >= self.cols {
            return;
        }
        let n = n.min(self.cols - col);
        let bce = self.bce_cell();
        for _ in 0..n {
            self.cells[row].insert(col, bce.clone());
        }
        self.cells[row].truncate(self.cols);
        self.dirty = true;
    }

    /// Erase `n` characters from cursor position (replace with blanks, no shift).
    pub fn erase_chars(&mut self, n: usize) {
        let row = self.cursor.row;
        let col = self.cursor.col;
        if row >= self.rows {
            return;
        }
        let bce = self.bce_cell();
        let end = (col + n).min(self.cols);
        for c in col..end {
            self.cells[row][c] = bce.clone();
        }
        self.dirty = true;
    }

    // -- Scrolling --

    /// Scroll the content within the scroll region up by `n` lines.
    /// Lines scrolled off the top of the scroll region are added to scrollback
    /// (only if the scroll region starts at line 0).
    pub fn scroll_up(&mut self, n: usize) {
        let top = self.scroll_top;
        let bottom = self.scroll_bottom;
        if top > bottom || bottom >= self.rows {
            return;
        }
        let n = n.min(bottom - top + 1);
        for _ in 0..n {
            let row = self.cells[top].clone();
            // Only add to scrollback if we're scrolling from the very top.
            if top == 0 {
                self.scrollback.push(row);
                if self.scrollback.len() > self.max_scrollback {
                    self.scrollback.remove(0);
                }
            }
            self.cells.remove(top);
            let insert_at = bottom.min(self.cells.len());
            self.cells.insert(insert_at, self.bce_row());
        }
        // Ensure we still have the right row count.
        while self.cells.len() < self.rows {
            self.cells.push(self.bce_row());
        }
        self.dirty = true;
    }

    /// Scroll the content within the scroll region down by `n` lines.
    /// Lines at the bottom are lost.
    pub fn scroll_down(&mut self, n: usize) {
        let top = self.scroll_top;
        let bottom = self.scroll_bottom;
        if top > bottom || bottom >= self.rows {
            return;
        }
        let n = n.min(bottom - top + 1);
        for _ in 0..n {
            if bottom < self.cells.len() {
                self.cells.remove(bottom);
            }
            self.cells.insert(top, self.bce_row());
        }
        while self.cells.len() < self.rows {
            self.cells.push(self.bce_row());
        }
        self.dirty = true;
    }

    /// Set the scroll region (0-indexed, inclusive on both ends).
    pub fn set_scroll_region(&mut self, top: usize, bottom: usize) {
        let top = top.min(self.rows - 1);
        let bottom = bottom.min(self.rows - 1);
        if top < bottom {
            self.scroll_top = top;
            self.scroll_bottom = bottom;
        } else {
            self.scroll_top = 0;
            self.scroll_bottom = self.rows - 1;
        }
        // After setting scroll region, cursor moves to home.
        self.cursor.row = self.scroll_top;
        self.cursor.col = 0;
        self.pending_wrap = false;
        self.dirty = true;
    }

    /// Resize the grid. Attempts to preserve content where possible.
    pub fn resize(&mut self, new_rows: usize, new_cols: usize) {
        let new_rows = new_rows.max(1);
        let new_cols = new_cols.max(1);

        // Resize each existing row (truncate or extend).
        for row in &mut self.cells {
            row.resize(new_cols, Cell::default());
        }

        // Add or remove rows.
        if new_rows > self.rows {
            // Try to pull lines from scrollback first.
            let extra = new_rows - self.rows;
            let from_scrollback = extra.min(self.scrollback.len());
            for _ in 0..from_scrollback {
                if let Some(mut row) = self.scrollback.pop() {
                    row.resize(new_cols, Cell::default());
                    self.cells.insert(0, row);
                    self.cursor.row += 1;
                }
            }
            // If still not enough, pad with blank rows.
            while self.cells.len() < new_rows {
                self.cells.push(vec![Cell::default(); new_cols]);
            }
        } else if new_rows < self.rows {
            // Move excess top rows to scrollback.
            let excess = self.rows - new_rows;
            let cursor_excess = if self.cursor.row >= new_rows {
                self.cursor.row - new_rows + 1
            } else {
                0
            };
            let remove = excess.max(cursor_excess);
            for _ in 0..remove {
                if !self.cells.is_empty() {
                    let row = self.cells.remove(0);
                    self.scrollback.push(row);
                    self.cursor.row = self.cursor.row.saturating_sub(1);
                }
            }
            self.cells.truncate(new_rows);
        }

        // Also resize scrollback rows.
        for row in &mut self.scrollback {
            row.resize(new_cols, Cell::default());
        }

        self.rows = new_rows;
        self.cols = new_cols;

        // Reset scroll region to full screen.
        self.scroll_top = 0;
        self.scroll_bottom = new_rows - 1;

        // Clamp cursor.
        self.cursor.row = self.cursor.row.min(new_rows - 1);
        self.cursor.col = self.cursor.col.min(new_cols - 1);
        self.pending_wrap = false;

        // Rebuild tab stops.
        self.tab_stops = vec![false; new_cols];
        for i in (0..new_cols).step_by(8) {
            self.tab_stops[i] = true;
        }

        if self.scrollback.len() > self.max_scrollback {
            let drain = self.scrollback.len() - self.max_scrollback;
            self.scrollback.drain(0..drain);
        }

        self.dirty = true;
    }

    pub fn set_title(&mut self, title: String) {
        self.title = title;
    }

    // -- Scrollback viewing --

    /// Scroll the view into the scrollback buffer by `n` lines.
    pub fn scroll_view_up(&mut self, n: usize) {
        let max = self.scrollback.len();
        self.scroll_offset = (self.scroll_offset + n).min(max);
        self.dirty = true;
    }

    /// Scroll the view back toward the live terminal by `n` lines.
    pub fn scroll_view_down(&mut self, n: usize) {
        self.scroll_offset = self.scroll_offset.saturating_sub(n);
        self.dirty = true;
    }

    /// Reset the view to the live terminal (bottom of scrollback).
    pub fn scroll_view_to_bottom(&mut self) {
        self.scroll_offset = 0;
        self.dirty = true;
    }

    /// Returns true if the view is at the bottom (showing live terminal).
    pub fn is_at_bottom(&self) -> bool {
        self.scroll_offset == 0
    }

    /// Return the visible rows, accounting for scroll offset.
    /// When scroll_offset > 0, some rows come from the scrollback buffer.
    pub fn visible_rows(&self) -> Vec<&Vec<Cell>> {
        if self.scroll_offset == 0 {
            return self.cells.iter().collect();
        }

        let sb_len = self.scrollback.len();
        let offset = self.scroll_offset.min(sb_len);
        let mut result = Vec::with_capacity(self.rows);

        // How many scrollback rows are visible?
        let sb_start = sb_len - offset;
        let sb_visible = offset.min(self.rows);

        for i in 0..sb_visible {
            if sb_start + i < sb_len {
                result.push(&self.scrollback[sb_start + i]);
            }
        }

        // Fill remaining with live rows.
        let live_needed = self.rows.saturating_sub(sb_visible);
        for i in 0..live_needed {
            if i < self.cells.len() {
                result.push(&self.cells[i]);
            }
        }

        result
    }

    // -- Alternate screen buffer --

    /// Switch to alternate screen buffer (used by fullscreen apps).
    pub fn enter_alt_screen(&mut self) {
        if self.alt_cells.is_some() {
            return; // Already in alt screen
        }
        self.alt_cells = Some(self.cells.clone());
        self.alt_cursor = Some(self.cursor.clone());
        // Clear the screen for the alt buffer.
        self.cells = (0..self.rows)
            .map(|_| self.new_row())
            .collect();
        self.cursor = CursorState::default();
        self.pending_wrap = false;
        self.dirty = true;
    }

    /// Switch back from alternate screen buffer.
    pub fn leave_alt_screen(&mut self) {
        if let Some(cells) = self.alt_cells.take() {
            self.cells = cells;
        }
        if let Some(cursor) = self.alt_cursor.take() {
            self.cursor = cursor;
        }
        self.pending_wrap = false;
        self.dirty = true;
    }

    /// Reset terminal state completely (RIS - Reset Initial State).
    pub fn reset(&mut self) {
        let rows = self.rows;
        let cols = self.cols;
        *self = Self::new(rows, cols);
    }

    /// Set cursor visible/hidden.
    pub fn set_cursor_visible(&mut self, visible: bool) {
        self.cursor.visible = visible;
        self.dirty = true;
    }

    /// Set bracketed paste mode (mode 2004).
    pub fn set_bracketed_paste(&mut self, enabled: bool) {
        self.bracketed_paste_mode = enabled;
    }

    /// Returns true if bracketed paste mode is active.
    pub fn bracketed_paste(&self) -> bool {
        self.bracketed_paste_mode
    }

    /// Reverse Index - move cursor up one line, scrolling down if at top of
    /// scroll region.
    pub fn reverse_index(&mut self) {
        self.pending_wrap = false;
        if self.cursor.row == self.scroll_top {
            self.scroll_down(1);
        } else if self.cursor.row > 0 {
            self.cursor.row -= 1;
        }
        self.dirty = true;
    }

    /// Index - move cursor down one line, scrolling up if at bottom of scroll
    /// region (same as newline but doesn't change column).
    pub fn index(&mut self) {
        self.pending_wrap = false;
        if self.cursor.row == self.scroll_bottom {
            self.scroll_up(1);
        } else if self.cursor.row < self.rows - 1 {
            self.cursor.row += 1;
        }
        self.dirty = true;
    }

    // -- Charset --

    /// Returns true if the currently active charset slot is DEC Special Graphics.
    fn active_charset_is_dec_special(&self) -> bool {
        match self.active_charset {
            CharsetSlot::G0 => self.charset_g0 == Charset::DecSpecialGraphics,
            CharsetSlot::G1 => self.charset_g1 == Charset::DecSpecialGraphics,
        }
    }

    /// Designate a charset to the G0 slot.
    pub fn set_charset_g0(&mut self, charset: Charset) {
        self.charset_g0 = charset;
    }

    /// Designate a charset to the G1 slot.
    pub fn set_charset_g1(&mut self, charset: Charset) {
        self.charset_g1 = charset;
    }

    /// Activate the G0 charset slot (Shift In).
    pub fn activate_g0(&mut self) {
        self.active_charset = CharsetSlot::G0;
    }

    /// Activate the G1 charset slot (Shift Out).
    pub fn activate_g1(&mut self) {
        self.active_charset = CharsetSlot::G1;
    }

    // -- Selection --

    /// Start a simple (character-level) selection at the given display position.
    /// `col` and `row` are 0-indexed display coordinates.
    pub fn start_selection(&mut self, col: usize, row: usize) {
        let pos = GridPos::new(col, row);
        self.selection = Some(SelectionState {
            start: pos,
            end: pos,
            active: true,
            kind: SelectionKind::Simple,
        });
        self.dirty = true;
    }

    /// Start a word selection (double-click): expand to word boundaries.
    pub fn start_word_selection(&mut self, col: usize, row: usize) {
        let visible = self.visible_rows();
        if row >= visible.len() {
            return;
        }
        let row_cells = visible[row];
        let (start_col, end_col) = Self::word_bounds(row_cells, col);
        self.selection = Some(SelectionState {
            start: GridPos::new(start_col, row),
            end: GridPos::new(end_col, row),
            active: true,
            kind: SelectionKind::Word,
        });
        self.dirty = true;
    }

    /// Start a line selection (triple-click): select the full row.
    pub fn start_line_selection(&mut self, _col: usize, row: usize) {
        self.selection = Some(SelectionState {
            start: GridPos::new(0, row),
            end: GridPos::new(self.cols.saturating_sub(1), row),
            active: true,
            kind: SelectionKind::Line,
        });
        self.dirty = true;
    }

    /// Update the selection endpoint during a drag.
    pub fn update_selection(&mut self, col: usize, row: usize) {
        if let Some(ref mut sel) = self.selection {
            if !sel.active {
                return;
            }
            match sel.kind {
                SelectionKind::Simple | SelectionKind::Word => {
                    sel.end = GridPos::new(col, row);
                }
                SelectionKind::Line => {
                    // Line selection: keep start col at 0, end col at max
                    sel.end = GridPos::new(self.cols.saturating_sub(1), row);
                    // Also adjust start row to be the earlier of the anchor and current
                    // We track the original anchor row in start, so just update end row
                }
            }
            self.dirty = true;
        }
    }

    /// Finalize the selection (mouse up).
    pub fn end_selection(&mut self) {
        if let Some(ref mut sel) = self.selection {
            sel.active = false;
        }
    }

    /// Clear any active selection.
    pub fn clear_selection(&mut self) {
        if self.selection.is_some() {
            self.selection = None;
            self.dirty = true;
        }
    }

    /// Check if a cell at display position (col, row) is within the selection.
    pub fn is_selected(&self, col: usize, row: usize) -> bool {
        let sel = match &self.selection {
            Some(s) => s,
            None => return false,
        };

        let (start, end) = if sel.start <= sel.end {
            (sel.start, sel.end)
        } else {
            (sel.end, sel.start)
        };

        if start.row == end.row {
            // Single-line selection
            row == start.row && col >= start.col && col <= end.col
        } else {
            // Multi-line selection
            if row == start.row {
                col >= start.col
            } else if row == end.row {
                col <= end.col
            } else {
                row > start.row && row < end.row
            }
        }
    }

    /// Extract the selected text as a string.
    pub fn selected_text(&self) -> Option<String> {
        let sel = self.selection.as_ref()?;
        let (start, end) = if sel.start <= sel.end {
            (sel.start, sel.end)
        } else {
            (sel.end, sel.start)
        };

        let visible = self.visible_rows();
        let mut text = String::new();

        for row_idx in start.row..=end.row {
            if row_idx >= visible.len() {
                break;
            }
            let row_cells = visible[row_idx];
            let col_start = if row_idx == start.row { start.col } else { 0 };
            let col_end = if row_idx == end.row {
                end.col
            } else {
                row_cells.len().saturating_sub(1)
            };

            let mut line = String::new();
            for ci in col_start..=col_end.min(row_cells.len().saturating_sub(1)) {
                line.push(row_cells[ci].c);
            }
            // Trim trailing spaces from each line
            let trimmed = line.trim_end();
            text.push_str(trimmed);

            if row_idx < end.row {
                text.push('\n');
            }
        }

        if text.is_empty() {
            None
        } else {
            Some(text)
        }
    }

    /// Return the normalized selection bounds for rendering.
    pub fn selection_bounds(&self) -> Option<(GridPos, GridPos)> {
        let sel = self.selection.as_ref()?;
        if sel.start <= sel.end {
            Some((sel.start, sel.end))
        } else {
            Some((sel.end, sel.start))
        }
    }

    /// Return the current scroll info: (total_lines, visible_lines, scroll_offset).
    pub fn scroll_info(&self) -> (usize, usize, usize) {
        (self.scrollback.len() + self.rows, self.rows, self.scroll_offset)
    }

    // -- Search --

    /// Search visible rows for `query`. Returns matches with display-relative coordinates.
    pub fn search(&self, query: &str, case_sensitive: bool, use_regex: bool) -> Vec<SearchMatch> {
        if query.is_empty() {
            return Vec::new();
        }

        let visible = self.visible_rows();
        let mut matches = Vec::new();

        let regex = if use_regex {
            let pattern = if case_sensitive {
                query.to_string()
            } else {
                format!("(?i){}", query)
            };
            Regex::new(&pattern).ok()
        } else {
            // Escape regex special chars for plain text search
            let escaped = regex::escape(query);
            let pattern = if case_sensitive {
                escaped
            } else {
                format!("(?i){}", escaped)
            };
            Regex::new(&pattern).ok()
        };

        let re = match regex {
            Some(r) => r,
            None => return Vec::new(),
        };

        for (ri, row) in visible.iter().enumerate() {
            let line: String = row.iter().map(|c| c.c).collect();
            for m in re.find_iter(&line) {
                matches.push(SearchMatch {
                    row: ri,
                    col: m.start(),
                    len: m.end() - m.start(),
                });
            }
        }

        matches
    }

    /// Get the scrollback length (for scrollbar calculations).
    pub fn scrollback_len(&self) -> usize {
        self.scrollback.len()
    }

    /// Get the current scroll offset.
    pub fn scroll_offset(&self) -> usize {
        self.scroll_offset
    }

    /// Set the scroll offset directly (clamped to valid range).
    pub fn set_scroll_offset(&mut self, offset: usize) {
        self.scroll_offset = offset.min(self.scrollback.len());
        self.dirty = true;
    }

    /// Find word boundaries around `col` in a row of cells.
    /// Returns (start_col, end_col) inclusive.
    fn word_bounds(row: &[Cell], col: usize) -> (usize, usize) {
        let col = col.min(row.len().saturating_sub(1));
        let is_word_char = |c: char| -> bool {
            c.is_alphanumeric() || c == '_' || c == '-' || c == '.'
        };

        let anchor = row[col].c;
        if !is_word_char(anchor) && anchor != ' ' {
            // Single punctuation char
            return (col, col);
        }

        // If clicking on whitespace, select the whitespace run
        let check = if anchor == ' ' {
            |c: char| c == ' '
        } else {
            is_word_char
        };

        let mut start = col;
        while start > 0 && check(row[start - 1].c) {
            start -= 1;
        }
        let mut end = col;
        while end + 1 < row.len() && check(row[end + 1].c) {
            end += 1;
        }
        (start, end)
    }

    // -- Internal helpers --

    fn ensure_row(&mut self, row: usize) {
        while self.cells.len() <= row {
            self.cells.push(self.new_row());
        }
        // Also ensure the row has enough columns.
        if self.cells[row].len() < self.cols {
            self.cells[row].resize(self.cols, Cell::default());
        }
    }

    fn new_row(&self) -> Vec<Cell> {
        vec![Cell::default(); self.cols]
    }

    /// Return a blank cell with the current background color (BCE support).
    /// xterm-256color has the `bce` capability, so erase operations must
    /// fill with the current SGR background color.
    fn bce_cell(&self) -> Cell {
        Cell {
            c: ' ',
            fg: TermColor::Default,
            bg: self.current_bg,
            attrs: CellAttributes::default(),
        }
    }

    /// Return a new blank row with the current background color (BCE).
    fn bce_row(&self) -> Vec<Cell> {
        vec![self.bce_cell(); self.cols]
    }
}
