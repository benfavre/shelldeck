use crate::colors::TermColor;
use regex::Regex;
use smallvec::SmallVec;
use std::collections::HashMap;
use unicode_width::UnicodeWidthChar;

// ---------------------------------------------------------------------------
// Ring Buffer
// ---------------------------------------------------------------------------

/// A fixed-capacity ring buffer that automatically evicts the oldest item
/// when pushing beyond capacity. Used for scrollback storage to avoid O(n)
/// `Vec::remove(0)` operations.
pub struct RingBuffer<T> {
    buf: Vec<Option<T>>,
    head: usize, // Index where the next item will be written
    len: usize,  // Current number of items
    capacity: usize,
}

impl<T> RingBuffer<T> {
    pub fn new(capacity: usize) -> Self {
        let capacity = capacity.max(1);
        let mut buf = Vec::with_capacity(capacity);
        buf.resize_with(capacity, || None);
        Self {
            buf,
            head: 0,
            len: 0,
            capacity,
        }
    }

    /// Push an item. If the buffer is full, the oldest item is evicted.
    pub fn push(&mut self, item: T) {
        self.buf[self.head] = Some(item);
        self.head = (self.head + 1) % self.capacity;
        if self.len < self.capacity {
            self.len += 1;
        }
    }

    pub fn len(&self) -> usize {
        self.len
    }
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// The starting index in the underlying buffer (the oldest item).
    fn start(&self) -> usize {
        if self.len < self.capacity {
            0
        } else {
            self.head // when full, head points to the oldest slot
        }
    }

    /// Get item by logical index (0 = oldest, len-1 = newest).
    pub fn get(&self, index: usize) -> Option<&T> {
        if index >= self.len {
            return None;
        }
        let real_idx = (self.start() + index) % self.capacity;
        self.buf[real_idx].as_ref()
    }

    /// Get mutable item by logical index (0 = oldest, len-1 = newest).
    pub fn get_mut(&mut self, index: usize) -> Option<&mut T> {
        if index >= self.len {
            return None;
        }
        let start = self.start();
        let real_idx = (start + index) % self.capacity;
        self.buf[real_idx].as_mut()
    }

    /// Pop the newest item (index len-1).
    pub fn pop(&mut self) -> Option<T> {
        if self.len == 0 {
            return None;
        }
        self.head = if self.head == 0 {
            self.capacity - 1
        } else {
            self.head - 1
        };
        self.len -= 1;
        self.buf[self.head].take()
    }

    pub fn clear(&mut self) {
        for item in &mut self.buf {
            *item = None;
        }
        self.head = 0;
        self.len = 0;
    }

    /// Iterate from oldest to newest.
    pub fn iter(&self) -> RingBufferIter<'_, T> {
        RingBufferIter {
            buf: self,
            index: 0,
        }
    }
}

pub struct RingBufferIter<'a, T> {
    buf: &'a RingBuffer<T>,
    index: usize,
}

impl<'a, T> Iterator for RingBufferIter<'a, T> {
    type Item = &'a T;

    fn next(&mut self) -> Option<Self::Item> {
        if self.index >= self.buf.len {
            return None;
        }
        let item = self.buf.get(self.index);
        self.index += 1;
        item
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.buf.len - self.index;
        (remaining, Some(remaining))
    }
}

impl<'a, T> ExactSizeIterator for RingBufferIter<'a, T> {}

// ---------------------------------------------------------------------------
// Prompt marks (OSC 133 - Shell Integration)
// ---------------------------------------------------------------------------

/// Prompt markers emitted by shell integration (OSC 133).
/// These allow the terminal to identify prompt boundaries, enabling
/// features like "jump to previous/next prompt".
#[derive(Debug, Clone, PartialEq)]
pub enum PromptMark {
    /// 133;A - A fresh prompt has started.
    PromptStart,
    /// 133;B - The user has started typing a command.
    CommandStart,
    /// 133;C - The command was executed.
    CommandExecuted,
    /// 133;D;exitcode - The command finished (with optional exit code).
    CommandFinished(Option<i32>),
}

// ---------------------------------------------------------------------------
// Selection
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectionKind {
    Simple,
    Block,
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
// Line flags (for reflow tracking)
// ---------------------------------------------------------------------------

/// Per-row flags used to track soft-wrap state for text reflow on resize.
#[derive(Debug, Clone, Copy, Default)]
pub struct LineFlags {
    /// True if this line is a continuation of the previous line due to
    /// auto-wrap (soft wrap). False if the line started after a hard
    /// newline (LF/CR+LF) or is the first line.
    pub soft_wrapped: bool,
}

// ---------------------------------------------------------------------------
// Cell types
// ---------------------------------------------------------------------------

/// Underline style variants as defined by SGR 4 sub-parameters and SGR 21.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum UnderlineStyle {
    #[default]
    None,
    Single,
    Double,
    Curly,
    Dotted,
    Dashed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CellAttributes {
    pub bold: bool,
    pub dim: bool,
    pub italic: bool,
    pub underline: UnderlineStyle,
    pub strikethrough: bool,
    pub blink: bool,
    pub inverse: bool,
    pub hidden: bool,
    pub overline: bool,
    pub underline_color: Option<TermColor>,
    /// OSC 8 hyperlink URL associated with this cell, if any.
    pub hyperlink: Option<String>,
}

impl Default for CellAttributes {
    fn default() -> Self {
        Self {
            bold: false,
            dim: false,
            italic: false,
            underline: UnderlineStyle::None,
            strikethrough: false,
            blink: false,
            inverse: false,
            hidden: false,
            overline: false,
            underline_color: None,
            hyperlink: None,
        }
    }
}

/// Width classification for a terminal cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CellWidth {
    /// Standard 1-column character.
    #[default]
    Normal,
    /// First cell of a 2-column character (CJK, emoji, etc.).
    Wide,
    /// Second cell of a 2-column character (placeholder, not rendered independently).
    Spacer,
}

#[derive(Debug, Clone)]
pub struct Cell {
    pub c: char,
    /// Combining characters attached to this cell (e.g. diacritics).
    pub combining: SmallVec<[char; 2]>,
    pub fg: TermColor,
    pub bg: TermColor,
    pub attrs: CellAttributes,
    /// Whether this cell is normal width, the first half of a wide char, or a spacer.
    pub wide: CellWidth,
}

impl Default for Cell {
    fn default() -> Self {
        Self {
            c: ' ',
            combining: SmallVec::new(),
            fg: TermColor::Default,
            bg: TermColor::Default,
            attrs: CellAttributes::default(),
            wide: CellWidth::Normal,
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
    pub blink: bool,
}

impl Default for CursorState {
    fn default() -> Self {
        Self {
            row: 0,
            col: 0,
            visible: true,
            shape: CursorShape::Block,
            blink: false,
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
    /// Per-row flags tracking soft-wrap state for reflow on resize.
    pub line_flags: Vec<LineFlags>,
    pub cursor: CursorState,
    pub rows: usize,
    pub cols: usize,
    pub title: String,
    scrollback: RingBuffer<Vec<Cell>>,
    /// Per-row flags for scrollback lines (mirrors scrollback RingBuffer).
    scrollback_line_flags: RingBuffer<LineFlags>,
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
    /// Per-row dirty flags: one flag per visible row.
    row_dirty: Vec<bool>,
    /// Quick check: true if any row is dirty.
    any_dirty: bool,
    /// Alternate screen buffer (used by fullscreen apps like vim, less)
    alt_cells: Option<Vec<Vec<Cell>>>,
    alt_cursor: Option<CursorState>,
    /// Saved line flags for the primary screen when in alt screen.
    alt_line_flags: Option<Vec<LineFlags>>,
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
    /// Application cursor keys mode (DECCKM, mode 1): when enabled, arrow
    /// keys send SS3 sequences (\x1bO…) instead of CSI sequences (\x1b[…).
    application_cursor_keys: bool,
    /// Application keypad mode (DECKPAM): when enabled, keypad keys send
    /// application-mode sequences.
    application_keypad: bool,
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
    /// Channel for sending responses back to the PTY (e.g., DSR, DA replies).
    response_tx: Option<std::sync::mpsc::Sender<Vec<u8>>>,
    /// Origin mode (DECOM, DECSET mode 6): when enabled, cursor positions
    /// from CUP are relative to the scroll region top margin, and the cursor
    /// is constrained to the scroll region.
    origin_mode: bool,
    /// Focus event reporting mode (DECSET mode 1004): when enabled, the
    /// terminal sends ESC[I on focus in and ESC[O on focus out.
    /// NOTE: The UI layer must check this flag and send the appropriate
    /// sequences when the terminal view gains/loses focus.
    focus_reporting: bool,
    /// Synchronized output mode (mode 2026): when enabled, the UI should
    /// buffer screen updates and only render when the mode is turned off
    /// (DECRST 2026). This prevents flicker during large screen updates.
    /// NOTE: The UI layer must check this flag to implement buffered rendering.
    synchronized_output: bool,
    pub working_directory: Option<String>,
    pub hyperlink: Option<String>,
    pub prompt_mark: Option<PromptMark>,
    pub prompt_lines: Vec<usize>,
    pub clipboard_request: Option<(String, String)>,
    pub palette_overrides: HashMap<u8, (u8, u8, u8)>,
}

impl TerminalGrid {
    pub fn new(rows: usize, cols: usize) -> Self {
        Self::with_scrollback(rows, cols, 10_000)
    }

    /// Create a new grid with a configurable scrollback limit.
    pub fn with_scrollback(rows: usize, cols: usize, max_scrollback: usize) -> Self {
        let rows = rows.max(1);
        let cols = cols.max(1);
        let max_scrollback = max_scrollback.max(1);

        let cells: Vec<Vec<Cell>> = (0..rows).map(|_| vec![Cell::default(); cols]).collect();

        let mut tab_stops = vec![false; cols];
        for i in (0..cols).step_by(8) {
            tab_stops[i] = true;
        }

        Self {
            cells,
            line_flags: vec![LineFlags::default(); rows],
            cursor: CursorState::default(),
            rows,
            cols,
            title: String::new(),
            scrollback: RingBuffer::new(max_scrollback),
            scrollback_line_flags: RingBuffer::new(max_scrollback),
            max_scrollback,
            scroll_offset: 0,
            scroll_top: 0,
            scroll_bottom: rows.saturating_sub(1),
            saved_cursor: None,
            current_attrs: CellAttributes::default(),
            current_fg: TermColor::Default,
            current_bg: TermColor::Default,
            tab_stops,
            dirty: true,
            row_dirty: vec![true; rows],
            any_dirty: true,
            alt_cells: None,
            alt_cursor: None,
            alt_line_flags: None,
            auto_wrap: true,
            pending_wrap: false,
            mouse_mode: MouseMode::None,
            mouse_encoding: MouseEncoding::Normal,
            bracketed_paste_mode: false,
            application_cursor_keys: false,
            application_keypad: false,
            selection: None,
            charset_g0: Charset::Ascii,
            charset_g1: Charset::Ascii,
            active_charset: CharsetSlot::G0,
            last_char: ' ',
            response_tx: None,
            origin_mode: false,
            focus_reporting: false,
            synchronized_output: false,
            working_directory: None,
            hyperlink: None,
            prompt_mark: None,
            prompt_lines: Vec::new(),
            clipboard_request: None,
            palette_overrides: HashMap::new(),
        }
    }

    // -- Per-row dirty tracking --

    /// Returns true if any row is dirty.
    pub fn is_any_dirty(&self) -> bool {
        self.any_dirty
    }

    /// Returns true if the given row is dirty. Out-of-bounds rows are
    /// considered dirty (safe default).
    pub fn is_row_dirty(&self, row: usize) -> bool {
        self.row_dirty.get(row).copied().unwrap_or(true)
    }

    /// Clear all dirty flags (both per-row and the global flag).
    pub fn clear_dirty(&mut self) {
        self.row_dirty.fill(false);
        self.any_dirty = false;
        self.dirty = false;
    }

    /// Mark a single row as dirty.
    pub fn mark_row_dirty(&mut self, row: usize) {
        if row < self.row_dirty.len() {
            self.row_dirty[row] = true;
        }
        self.any_dirty = true;
        self.dirty = true;
    }

    /// Mark all rows as dirty.
    pub fn mark_all_dirty(&mut self) {
        self.row_dirty.fill(true);
        self.any_dirty = true;
        self.dirty = true;
    }

    /// Write a character at the current cursor position and advance the cursor.
    /// Handles wide (CJK/emoji) characters that occupy 2 columns, combining
    /// characters (zero-width diacritics), and normal single-width characters.
    pub fn write_char(&mut self, c: char) {
        // Translate through the active charset (DEC Special Graphics).
        let c = if self.active_charset_is_dec_special() {
            dec_special_char(c)
        } else {
            c
        };

        let width = UnicodeWidthChar::width(c).unwrap_or(0);

        // --- Zero-width / combining character ---
        if width == 0 {
            let row = self.cursor.row;
            self.ensure_row(row);
            let target_col = if self.cursor.col > 0 {
                self.cursor.col - 1
            } else {
                0
            };
            // If the target is a Spacer, attach to the Wide cell before it.
            let attach_col =
                if target_col > 0 && self.cells[row][target_col].wide == CellWidth::Spacer {
                    target_col - 1
                } else {
                    target_col
                };
            if attach_col < self.cols {
                self.cells[row][attach_col].combining.push(c);
            }
            self.dirty = true;
            return;
        }

        // --- Normal or wide character: resolve pending wrap first ---
        if self.pending_wrap {
            self.cursor.col = 0;
            if self.cursor.row == self.scroll_bottom {
                self.scroll_up(1);
                if self.scroll_bottom < self.line_flags.len() {
                    self.line_flags[self.scroll_bottom].soft_wrapped = true;
                }
            } else if self.cursor.row < self.rows - 1 {
                self.cursor.row += 1;
                if self.cursor.row < self.line_flags.len() {
                    self.line_flags[self.cursor.row].soft_wrapped = true;
                }
            }
            self.pending_wrap = false;
        }

        self.ensure_row(self.cursor.row);

        let mut attrs = self.current_attrs.clone();
        if self.hyperlink.is_some() {
            attrs.hyperlink = self.hyperlink.clone();
        }

        let row = self.cursor.row;
        let col = self.cursor.col.min(self.cols - 1);

        if width == 2 {
            // --- Wide character (2 columns) ---
            // If at the very last column (no room for 2 cells), wrap first.
            if col + 1 >= self.cols && self.auto_wrap {
                self.cursor.col = 0;
                if self.cursor.row == self.scroll_bottom {
                    self.scroll_up(1);
                    if self.scroll_bottom < self.line_flags.len() {
                        self.line_flags[self.scroll_bottom].soft_wrapped = true;
                    }
                } else if self.cursor.row < self.rows - 1 {
                    self.cursor.row += 1;
                    if self.cursor.row < self.line_flags.len() {
                        self.line_flags[self.cursor.row].soft_wrapped = true;
                    }
                }
                self.ensure_row(self.cursor.row);
            }

            let row = self.cursor.row;
            let col = self.cursor.col.min(self.cols.saturating_sub(2));

            // Clear any wide-char pair that we're about to overwrite.
            self.clear_wide_pair_at(row, col);
            if col + 1 < self.cols {
                self.clear_wide_pair_at(row, col + 1);
            }

            self.cells[row][col] = Cell {
                c,
                combining: SmallVec::new(),
                fg: self.current_fg,
                bg: self.current_bg,
                attrs: attrs.clone(),
                wide: CellWidth::Wide,
            };

            if col + 1 < self.cols {
                self.cells[row][col + 1] = Cell {
                    c: ' ',
                    combining: SmallVec::new(),
                    fg: self.current_fg,
                    bg: self.current_bg,
                    attrs,
                    wide: CellWidth::Spacer,
                };
            }

            self.last_char = c;

            if col + 2 < self.cols {
                self.cursor.col = col + 2;
            } else if self.auto_wrap {
                self.cursor.col = self.cols - 1;
                self.pending_wrap = true;
            } else {
                self.cursor.col = self.cols - 1;
            }
        } else {
            // --- Normal single-width character ---
            self.clear_wide_pair_at(row, col);

            self.cells[row][col] = Cell {
                c,
                combining: SmallVec::new(),
                fg: self.current_fg,
                bg: self.current_bg,
                attrs,
                wide: CellWidth::Normal,
            };

            self.last_char = c;

            if col + 1 < self.cols {
                self.cursor.col = col + 1;
            } else if self.auto_wrap {
                self.pending_wrap = true;
            }
        }

        self.dirty = true;
    }

    /// If the cell at (row, col) is part of a wide character pair, clear both
    /// halves to prevent "half characters" from lingering on screen.
    fn clear_wide_pair_at(&mut self, row: usize, col: usize) {
        if row >= self.rows || col >= self.cols {
            return;
        }
        match self.cells[row][col].wide {
            CellWidth::Wide => {
                if col + 1 < self.cols && self.cells[row][col + 1].wide == CellWidth::Spacer {
                    self.cells[row][col + 1] = Cell::default();
                }
            }
            CellWidth::Spacer => {
                if col > 0 && self.cells[row][col - 1].wide == CellWidth::Wide {
                    self.cells[row][col - 1] = Cell::default();
                }
            }
            CellWidth::Normal => {}
        }
    }

    /// Repeat the last printed character `n` times (REP — CSI Pb b).
    pub fn repeat_char(&mut self, n: usize) {
        let c = self.last_char;
        for _ in 0..n {
            self.write_char(c);
        }
    }

    /// Move cursor down one line, scrolling if necessary.
    /// This is a hard newline (LF), so the new line is NOT soft-wrapped.
    pub fn newline(&mut self) {
        self.pending_wrap = false;
        if self.cursor.row == self.scroll_bottom {
            self.scroll_up(1);
            // Hard newline: the new blank line at scroll_bottom is not soft-wrapped.
            if self.scroll_bottom < self.line_flags.len() {
                self.line_flags[self.scroll_bottom].soft_wrapped = false;
            }
        } else if self.cursor.row < self.rows - 1 {
            self.cursor.row += 1;
            // Hard newline: ensure the target line is not marked as soft-wrapped.
            if self.cursor.row < self.line_flags.len() {
                self.line_flags[self.cursor.row].soft_wrapped = false;
            }
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

    /// Advance cursor to next tab stop.
    pub fn tab(&mut self) {
        self.pending_wrap = false;
        self.cursor.col = self.next_tab_stop(self.cursor.col);
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
    /// When origin mode is active, the row is relative to the scroll region
    /// top margin and clamped to the scroll region.
    pub fn cursor_to(&mut self, row: usize, col: usize) {
        self.pending_wrap = false;
        let (clamped_row, clamped_col) = self.cursor_position_clamped(row, col);
        self.cursor.row = clamped_row;
        self.cursor.col = clamped_col;
        self.dirty = true;
    }

    /// Clamp a cursor position. When origin mode is active, the row is
    /// offset by scroll_top and constrained to the scroll region. Otherwise
    /// it is clamped to the full screen.
    pub fn cursor_position_clamped(&self, row: usize, col: usize) -> (usize, usize) {
        if self.origin_mode {
            let effective_row = (self.scroll_top + row).min(self.scroll_bottom);
            let effective_col = col.min(self.cols - 1);
            (effective_row, effective_col)
        } else {
            (row.min(self.rows - 1), col.min(self.cols - 1))
        }
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
                    // If erasing starts on the Spacer half of a wide char,
                    // also clear the Wide cell to the left.
                    self.clear_wide_pair_at(row, col);
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
                    let end_col = col.min(self.cols - 1);
                    // If erasing ends on the Wide half but not its Spacer,
                    // also clear the Spacer to the right.
                    self.clear_wide_pair_at(row, end_col);
                    for c in 0..=end_col {
                        self.cells[row][c] = bce.clone();
                    }
                }
            }
            2 => {
                // Clear entire display.
                for r in 0..self.rows {
                    self.cells[r] = self.bce_row();
                }
                // Reset all line flags since content is cleared.
                for f in &mut self.line_flags {
                    *f = LineFlags::default();
                }
            }
            3 => {
                // Clear entire display + scrollback.
                for r in 0..self.rows {
                    self.cells[r] = self.bce_row();
                }
                for f in &mut self.line_flags {
                    *f = LineFlags::default();
                }
                self.scrollback.clear();
                self.scrollback_line_flags.clear();
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
                self.clear_wide_pair_at(row, self.cursor.col);
                for c in self.cursor.col..self.cols {
                    self.cells[row][c] = bce.clone();
                }
            }
            1 => {
                let end_col = self.cursor.col.min(self.cols - 1);
                self.clear_wide_pair_at(row, end_col);
                for c in 0..=end_col {
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

        // Drain n lines from the bottom of the scroll region (O(n)).
        let drain_start = self.scroll_bottom + 1 - n;
        let drain_end = (self.scroll_bottom + 1).min(self.cells.len());
        if drain_start < drain_end {
            self.cells.drain(drain_start..drain_end);
        }
        let flags_drain_end = (self.scroll_bottom + 1).min(self.line_flags.len());
        if drain_start < flags_drain_end {
            self.line_flags.drain(drain_start..flags_drain_end);
        }

        // Insert n blank rows at cursor position.
        let new_rows: Vec<Vec<Cell>> = (0..n).map(|_| self.bce_row()).collect();
        let new_flags: Vec<LineFlags> = vec![LineFlags::default(); n];
        let cells_insert = row.min(self.cells.len());
        let flags_insert = row.min(self.line_flags.len());
        self.cells.splice(cells_insert..cells_insert, new_rows);
        self.line_flags
            .splice(flags_insert..flags_insert, new_flags);

        while self.cells.len() < self.rows {
            self.cells.push(self.bce_row());
        }
        while self.line_flags.len() < self.rows {
            self.line_flags.push(LineFlags::default());
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

        // Drain n lines at cursor position (O(n)).
        let drain_end = (row + n).min(self.cells.len());
        if row < drain_end {
            self.cells.drain(row..drain_end);
        }
        let flags_drain_end = (row + n).min(self.line_flags.len());
        if row < flags_drain_end {
            self.line_flags.drain(row..flags_drain_end);
        }

        // Insert n blank rows at the bottom of the scroll region.
        let insert_pos = (self.scroll_bottom + 1 - n).min(self.cells.len());
        let new_rows: Vec<Vec<Cell>> = (0..n).map(|_| self.bce_row()).collect();
        let new_flags: Vec<LineFlags> = vec![LineFlags::default(); n];
        let flags_insert = insert_pos.min(self.line_flags.len());
        self.cells.splice(insert_pos..insert_pos, new_rows);
        self.line_flags
            .splice(flags_insert..flags_insert, new_flags);

        while self.cells.len() < self.rows {
            self.cells.push(self.bce_row());
        }
        while self.line_flags.len() < self.rows {
            self.line_flags.push(LineFlags::default());
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
        // If we're deleting from the middle of a wide char, clear both halves first.
        self.clear_wide_pair_at(row, col);
        let n = n.min(self.cols - col);
        // Clear wide pairs at the boundary of the drain range.
        let drain_end = (col + n).min(self.cells[row].len());
        if drain_end > col {
            self.clear_wide_pair_at(row, drain_end.saturating_sub(1));
        }
        // Drain n cells at once (O(n) instead of O(n²)).
        let actual_drain = drain_end.min(self.cells[row].len());
        if col < actual_drain {
            self.cells[row].drain(col..actual_drain);
        }
        // Fill blanks at the end.
        let bce = self.bce_cell();
        self.cells[row].resize(self.cols, bce);
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
        // If inserting into the middle of a wide char, clear both halves.
        self.clear_wide_pair_at(row, col);
        let n = n.min(self.cols - col);
        let bce = self.bce_cell();
        // Splice n blank cells at once (O(n) instead of O(n²)).
        let blanks: Vec<Cell> = vec![bce; n];
        self.cells[row].splice(col..col, blanks);
        self.cells[row].truncate(self.cols);
        // If truncation split a wide char at the right edge, clear the orphan.
        if self.cols > 0 && self.cells[row][self.cols - 1].wide == CellWidth::Wide {
            self.cells[row][self.cols - 1] = Cell::default();
        }
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
        // Handle wide-char boundaries at both edges of the erase range.
        self.clear_wide_pair_at(row, col);
        if end > 0 && end < self.cols {
            self.clear_wide_pair_at(row, end);
        }
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

        // Save to scrollback before draining (only when scrolling from top).
        if top == 0 {
            for i in top..top + n {
                let flags = if i < self.line_flags.len() {
                    self.line_flags[i]
                } else {
                    LineFlags::default()
                };
                self.scrollback.push(self.cells[i].clone());
                self.scrollback_line_flags.push(flags);
            }
        }

        // Drain top n rows and their flags in one O(n) operation each.
        self.cells.drain(top..top + n);
        let flags_end = (top + n).min(self.line_flags.len());
        if top < flags_end {
            self.line_flags.drain(top..flags_end);
        }

        // Insert n blank rows at the bottom of the scroll region.
        let insert_at = bottom + 1 - n;
        let new_rows: Vec<Vec<Cell>> = (0..n).map(|_| self.bce_row()).collect();
        let new_flags: Vec<LineFlags> = vec![LineFlags::default(); n];
        let cells_insert = insert_at.min(self.cells.len());
        let flags_insert = insert_at.min(self.line_flags.len());
        self.cells.splice(cells_insert..cells_insert, new_rows);
        self.line_flags
            .splice(flags_insert..flags_insert, new_flags);

        // Ensure we still have the right row count.
        while self.cells.len() < self.rows {
            self.cells.push(self.bce_row());
        }
        while self.line_flags.len() < self.rows {
            self.line_flags.push(LineFlags::default());
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

        // Drain bottom n rows in one O(n) operation.
        let drain_start = bottom + 1 - n;
        let drain_end = (bottom + 1).min(self.cells.len());
        if drain_start < drain_end {
            self.cells.drain(drain_start..drain_end);
        }
        let flags_drain_end = (bottom + 1).min(self.line_flags.len());
        if drain_start < flags_drain_end {
            self.line_flags.drain(drain_start..flags_drain_end);
        }

        // Insert n blank rows at the top of the scroll region.
        let new_rows: Vec<Vec<Cell>> = (0..n).map(|_| self.bce_row()).collect();
        let new_flags: Vec<LineFlags> = vec![LineFlags::default(); n];
        let cells_insert = top.min(self.cells.len());
        let flags_insert = top.min(self.line_flags.len());
        self.cells.splice(cells_insert..cells_insert, new_rows);
        self.line_flags
            .splice(flags_insert..flags_insert, new_flags);

        while self.cells.len() < self.rows {
            self.cells.push(self.bce_row());
        }
        while self.line_flags.len() < self.rows {
            self.line_flags.push(LineFlags::default());
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
    /// When the width increases, soft-wrapped lines are re-joined (reflowed).
    pub fn resize(&mut self, new_rows: usize, new_cols: usize) {
        let new_rows = new_rows.max(1);
        let new_cols = new_cols.max(1);
        let old_cols = self.cols;

        // Reflow: when the terminal grows wider, merge soft-wrapped lines.
        // When it shrinks, re-wrap long lines.
        if new_cols > old_cols {
            self.reflow_on_grow(new_cols);
        } else if new_cols < old_cols {
            self.reflow_on_shrink(new_cols);
        }

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
                    let flags = self.scrollback_line_flags.pop().unwrap_or_default();
                    self.line_flags.insert(0, flags);
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
                    let flags = if !self.line_flags.is_empty() {
                        self.line_flags.remove(0)
                    } else {
                        LineFlags::default()
                    };
                    self.scrollback_line_flags.push(flags);
                    self.cursor.row = self.cursor.row.saturating_sub(1);
                }
            }
            self.cells.truncate(new_rows);
        }

        // Ensure line_flags matches cells length.
        self.line_flags
            .resize(self.cells.len(), LineFlags::default());

        // Also resize scrollback rows.
        for i in 0..self.scrollback.len() {
            if let Some(row) = self.scrollback.get_mut(i) {
                row.resize(new_cols, Cell::default());
            }
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

        // Ring buffer handles capacity limits automatically, no drain needed.

        // Reset per-row dirty tracking to match new row count.
        self.row_dirty = vec![true; new_rows];
        self.any_dirty = true;
        self.dirty = true;
    }

    /// Reflow soft-wrapped lines when the terminal width increases.
    /// Walks the screen rows and merges consecutive soft-wrapped lines
    /// when their combined content fits within `new_cols`.
    fn reflow_on_grow(&mut self, new_cols: usize) {
        let mut i = 1;
        while i < self.cells.len() && i < self.line_flags.len() {
            if self.line_flags[i].soft_wrapped {
                // This row is a continuation of the previous row.
                let prev_content_len = self.cells[i - 1]
                    .iter()
                    .rposition(|c| c.c != ' ' && c.c != '\0')
                    .map(|p| p + 1)
                    .unwrap_or(0);
                let curr_content_len = self.cells[i]
                    .iter()
                    .rposition(|c| c.c != ' ' && c.c != '\0')
                    .map(|p| p + 1)
                    .unwrap_or(0);

                if prev_content_len + curr_content_len <= new_cols {
                    // Merge: append current row content to previous row.
                    // First, ensure prev row is big enough.
                    if self.cells[i - 1].len() < new_cols {
                        self.cells[i - 1].resize(new_cols, Cell::default());
                    }
                    for j in 0..curr_content_len {
                        if prev_content_len + j < self.cells[i - 1].len() {
                            self.cells[i - 1][prev_content_len + j] = self.cells[i][j].clone();
                        }
                    }
                    self.cells.remove(i);
                    self.line_flags.remove(i);
                    // Adjust cursor row if it was at or below the removed row.
                    if self.cursor.row >= i {
                        self.cursor.row = self.cursor.row.saturating_sub(1);
                    }
                    // Don't increment i, check the new row at this index.
                    continue;
                }
            }
            i += 1;
        }
    }

    /// Reflow when the terminal shrinks: split lines that are wider than
    /// `new_cols` into multiple soft-wrapped rows.
    fn reflow_on_shrink(&mut self, new_cols: usize) {
        let mut i = 0;
        while i < self.cells.len() {
            let content_len = self.cells[i]
                .iter()
                .rposition(|c| c.c != ' ' && c.c != '\0')
                .map(|p| p + 1)
                .unwrap_or(0);

            if content_len > new_cols {
                // Split this row: keep first new_cols cells, move the rest to a new row.
                let overflow: Vec<Cell> = self.cells[i].split_off(new_cols);
                let mut new_row = overflow;
                new_row.resize(self.cols, Cell::default());

                // Mark current row as soft-wrapped.
                if i < self.line_flags.len() {
                    self.line_flags[i].soft_wrapped = true;
                }

                // Insert overflow as a new row below.
                let insert_at = i + 1;
                if insert_at <= self.cells.len() {
                    self.cells.insert(insert_at, new_row);
                    self.line_flags.insert(insert_at, LineFlags::default());
                }

                // Adjust cursor position if it was on or below the split point.
                if self.cursor.row > i {
                    self.cursor.row += 1;
                } else if self.cursor.row == i && self.cursor.col >= new_cols {
                    self.cursor.row += 1;
                    self.cursor.col -= new_cols;
                }

                // Don't increment i — check the overflow row too (it might also be too long).
            } else {
                i += 1;
            }
        }
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
            let sb_idx = sb_start + i;
            if let Some(row) = self.scrollback.get(sb_idx) {
                result.push(row);
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
        self.alt_line_flags = Some(self.line_flags.clone());
        // Clear the screen for the alt buffer.
        self.cells = (0..self.rows).map(|_| self.new_row()).collect();
        self.line_flags = vec![LineFlags::default(); self.rows];
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
        if let Some(flags) = self.alt_line_flags.take() {
            self.line_flags = flags;
        }
        self.pending_wrap = false;
        self.dirty = true;
    }

    /// Reset terminal state completely (RIS - Reset Initial State).
    pub fn reset(&mut self) {
        let rows = self.rows;
        let cols = self.cols;
        let max_scrollback = self.max_scrollback;
        *self = Self::with_scrollback(rows, cols, max_scrollback);
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

    /// Set application cursor keys mode (DECCKM, mode 1).
    pub fn set_application_cursor_keys(&mut self, enabled: bool) {
        self.application_cursor_keys = enabled;
    }

    /// Returns true if application cursor keys mode is active.
    pub fn application_cursor_keys(&self) -> bool {
        self.application_cursor_keys
    }

    /// Set application keypad mode (DECKPAM).
    pub fn set_application_keypad(&mut self, enabled: bool) {
        self.application_keypad = enabled;
    }

    /// Returns true if application keypad mode is active.
    pub fn application_keypad(&self) -> bool {
        self.application_keypad
    }

    // -- Origin mode (DECOM) --

    /// Set origin mode (DECOM, DECSET mode 6).
    /// When enabled, cursor positioning is relative to the scroll region
    /// and the cursor is constrained to the scroll region.
    pub fn set_origin_mode(&mut self, enabled: bool) {
        self.origin_mode = enabled;
        // When origin mode is set or reset, the cursor moves to the home position.
        if enabled {
            self.cursor.row = self.scroll_top;
        } else {
            self.cursor.row = 0;
        }
        self.cursor.col = 0;
        self.pending_wrap = false;
        self.dirty = true;
    }

    /// Returns true if origin mode (DECOM) is active.
    pub fn origin_mode(&self) -> bool {
        self.origin_mode
    }

    // -- Auto-wrap mode (DECAWM) --

    /// Set auto-wrap mode (DECAWM, DECSET mode 7).
    pub fn set_auto_wrap(&mut self, enabled: bool) {
        self.auto_wrap = enabled;
    }

    /// Returns true if auto-wrap mode is active.
    pub fn auto_wrap(&self) -> bool {
        self.auto_wrap
    }

    // -- Tab stop management --

    /// Set a tab stop at the current cursor column (HTS - ESC H).
    pub fn set_tab_stop(&mut self) {
        let col = self.cursor.col;
        if col < self.tab_stops.len() {
            self.tab_stops[col] = true;
        }
    }

    /// Clear tab stop(s).
    /// mode 0: clear tab stop at current column.
    /// mode 3: clear all tab stops.
    pub fn clear_tab_stop(&mut self, mode: u8) {
        match mode {
            0 => {
                let col = self.cursor.col;
                if col < self.tab_stops.len() {
                    self.tab_stops[col] = false;
                }
            }
            3 => {
                for stop in &mut self.tab_stops {
                    *stop = false;
                }
            }
            _ => {}
        }
    }

    /// Find the next tab stop after `from_col`. Returns the column of the
    /// next tab stop, or the last column if none is found.
    pub fn next_tab_stop(&self, from_col: usize) -> usize {
        for i in (from_col + 1)..self.cols {
            if self.tab_stops.get(i).copied().unwrap_or(false) {
                return i;
            }
        }
        // No tab stop found - return last column.
        self.cols - 1
    }

    /// Find the previous tab stop before `from_col`. Returns the column of
    /// the previous tab stop, or 0 if none is found.
    pub fn prev_tab_stop(&self, from_col: usize) -> usize {
        if from_col == 0 {
            return 0;
        }
        for i in (0..from_col).rev() {
            if self.tab_stops.get(i).copied().unwrap_or(false) {
                return i;
            }
        }
        0
    }

    // -- Focus reporting (mode 1004) --

    /// Set focus event reporting mode (DECSET mode 1004).
    /// When enabled, the UI layer should send ESC[I (focus in) and
    /// ESC[O (focus out) when the terminal gains/loses focus.
    pub fn set_focus_reporting(&mut self, enabled: bool) {
        self.focus_reporting = enabled;
    }

    /// Returns true if focus event reporting is active.
    pub fn focus_reporting(&self) -> bool {
        self.focus_reporting
    }

    // -- Synchronized output (mode 2026) --

    /// Set synchronized output mode (mode 2026).
    /// When enabled, the UI should buffer screen updates and only flush
    /// when this mode is turned off. This prevents flicker during large
    /// batched screen updates.
    pub fn set_synchronized_output(&mut self, enabled: bool) {
        self.synchronized_output = enabled;
        self.dirty = true;
    }

    /// Returns true if synchronized output mode is active.
    pub fn synchronized_output(&self) -> bool {
        self.synchronized_output
    }

    // -- Response channel --

    /// Set the response channel sender for writing responses back to the PTY.
    pub fn set_response_tx(&mut self, tx: std::sync::mpsc::Sender<Vec<u8>>) {
        self.response_tx = Some(tx);
    }

    /// Send a response back to the PTY (e.g., DSR cursor position report).
    /// Does nothing if no response channel is set.
    pub fn write_response(&self, data: Vec<u8>) {
        if let Some(ref tx) = self.response_tx {
            let _ = tx.send(data);
        }
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

    /// Start a block/rectangular selection (Alt+click) at the given display position.
    /// In block mode, the selection forms a rectangle defined by two corners.
    pub fn start_block_selection(&mut self, col: usize, row: usize) {
        let pos = GridPos::new(col, row);
        self.selection = Some(SelectionState {
            start: pos,
            end: pos,
            active: true,
            kind: SelectionKind::Block,
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
                SelectionKind::Simple | SelectionKind::Word | SelectionKind::Block => {
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

        // Block/rectangular selection: the selected region is a rectangle
        // defined by two corners, independent of stream order.
        if sel.kind == SelectionKind::Block {
            let min_row = sel.start.row.min(sel.end.row);
            let max_row = sel.start.row.max(sel.end.row);
            let min_col = sel.start.col.min(sel.end.col);
            let max_col = sel.start.col.max(sel.end.col);
            return row >= min_row && row <= max_row && col >= min_col && col <= max_col;
        }

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
        let visible = self.visible_rows();

        // Block/rectangular selection: extract the same column range from each row.
        if sel.kind == SelectionKind::Block {
            let min_row = sel.start.row.min(sel.end.row);
            let max_row = sel.start.row.max(sel.end.row);
            let min_col = sel.start.col.min(sel.end.col);
            let max_col = sel.start.col.max(sel.end.col);

            let mut lines = Vec::new();
            for row_idx in min_row..=max_row {
                if row_idx >= visible.len() {
                    break;
                }
                let row_cells = visible[row_idx];
                let mut line = String::new();
                for cell in &row_cells[min_col..=max_col.min(row_cells.len().saturating_sub(1))] {
                    // Skip spacer cells (second half of wide chars).
                    if cell.wide == CellWidth::Spacer {
                        continue;
                    }
                    line.push(cell.c);
                    // Append any combining characters.
                    for &comb in &cell.combining {
                        line.push(comb);
                    }
                }
                lines.push(line.trim_end().to_string());
            }

            let text = lines.join("\n");
            if text.is_empty() {
                return None;
            }
            return Some(text);
        }

        // Stream selection (Simple, Word, Line).
        let (start, end) = if sel.start <= sel.end {
            (sel.start, sel.end)
        } else {
            (sel.end, sel.start)
        };

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
            for cell in &row_cells[col_start..=col_end.min(row_cells.len().saturating_sub(1))] {
                // Skip spacer cells (second half of wide chars).
                if cell.wide == CellWidth::Spacer {
                    continue;
                }
                line.push(cell.c);
                // Append any combining characters.
                for &comb in &cell.combining {
                    line.push(comb);
                }
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
        (
            self.scrollback.len() + self.rows,
            self.rows,
            self.scroll_offset,
        )
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
            // Build the line text skipping spacer cells, and track the
            // mapping from character-index-in-string to column-index.
            let mut line = String::new();
            let mut char_to_col: Vec<usize> = Vec::new();
            for (ci, cell) in row.iter().enumerate() {
                if cell.wide == CellWidth::Spacer {
                    continue;
                }
                char_to_col.push(ci);
                line.push(cell.c);
                for &comb in &cell.combining {
                    line.push(comb);
                }
            }
            for m in re.find_iter(&line) {
                let start_char = m.start();
                let end_char = m.end();
                let col = if start_char < char_to_col.len() {
                    char_to_col[start_char]
                } else {
                    continue;
                };
                matches.push(SearchMatch {
                    row: ri,
                    col,
                    len: end_char - start_char,
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
        let mut col = col.min(row.len().saturating_sub(1));
        // If clicking on a Spacer, move to its Wide cell.
        if row[col].wide == CellWidth::Spacer && col > 0 {
            col -= 1;
        }
        let is_word_char =
            |c: char| -> bool { c.is_alphanumeric() || c == '_' || c == '-' || c == '.' };

        let anchor = row[col].c;
        if !is_word_char(anchor) && anchor != ' ' {
            // Single punctuation char (or wide char that's not a word char)
            let end = if row[col].wide == CellWidth::Wide && col + 1 < row.len() {
                col + 1
            } else {
                col
            };
            return (col, end);
        }

        // If clicking on whitespace, select the whitespace run
        let check = if anchor == ' ' {
            |c: char| c == ' '
        } else {
            is_word_char
        };

        let mut start = col;
        while start > 0 && row[start - 1].wide != CellWidth::Spacer && check(row[start - 1].c) {
            start -= 1;
        }
        let mut end = col;
        while end + 1 < row.len() {
            // Skip over spacers when scanning forward.
            if row[end + 1].wide == CellWidth::Spacer {
                end += 1;
                continue;
            }
            if check(row[end + 1].c) {
                end += 1;
            } else {
                break;
            }
        }
        (start, end)
    }

    // -- OSC palette --

    /// Set a custom palette color override (OSC 4).
    pub fn set_palette_color(&mut self, index: u8, r: u8, g: u8, b: u8) {
        self.palette_overrides.insert(index, (r, g, b));
        self.dirty = true;
    }

    /// Get a palette color override, if any.
    pub fn get_palette_color(&self, index: u8) -> Option<(u8, u8, u8)> {
        self.palette_overrides.get(&index).copied()
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
            bg: self.current_bg,
            ..Cell::default()
        }
    }

    /// Return a new blank row with the current background color (BCE).
    fn bce_row(&self) -> Vec<Cell> {
        vec![self.bce_cell(); self.cols]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::colors::{NamedColor, TermColor};

    /// Collect the printable characters of a row into a String (spacers excluded).
    fn row_text(grid: &TerminalGrid, row: usize) -> String {
        grid.cells[row]
            .iter()
            .filter(|c| c.wide != CellWidth::Spacer)
            .map(|c| c.c)
            .collect()
    }

    fn write_str(grid: &mut TerminalGrid, s: &str) {
        for c in s.chars() {
            grid.write_char(c);
        }
    }

    // ---- RingBuffer ----

    #[test]
    fn ring_buffer_evicts_oldest_when_full() {
        let mut rb: RingBuffer<i32> = RingBuffer::new(3);
        for v in 0..5 {
            rb.push(v);
        }
        assert_eq!(rb.len(), 3);
        // Oldest two (0,1) evicted; remaining are 2,3,4 oldest-to-newest.
        let collected: Vec<i32> = rb.iter().copied().collect();
        assert_eq!(collected, vec![2, 3, 4]);
        assert_eq!(rb.get(0), Some(&2));
        assert_eq!(rb.get(2), Some(&4));
        assert_eq!(rb.get(3), None);
    }

    #[test]
    fn ring_buffer_pop_returns_newest() {
        let mut rb: RingBuffer<i32> = RingBuffer::new(4);
        rb.push(10);
        rb.push(20);
        rb.push(30);
        assert_eq!(rb.pop(), Some(30));
        assert_eq!(rb.pop(), Some(20));
        assert_eq!(rb.len(), 1);
        assert_eq!(rb.get(0), Some(&10));
    }

    // ---- write_char / cursor advance ----

    #[test]
    fn write_char_advances_cursor_and_stores_glyph() {
        let mut g = TerminalGrid::new(5, 10);
        g.write_char('A');
        assert_eq!(g.cells[0][0].c, 'A');
        assert_eq!(g.cursor.col, 1);
        assert_eq!(g.cursor.row, 0);
    }

    #[test]
    fn write_string_fills_cells_in_order() {
        let mut g = TerminalGrid::new(5, 10);
        write_str(&mut g, "Hello");
        assert_eq!(&row_text(&g, 0)[..5], "Hello");
        assert_eq!(g.cursor.col, 5);
    }

    #[test]
    fn line_wraps_at_right_edge() {
        let mut g = TerminalGrid::new(5, 4);
        // Write exactly cols chars -> last char enters pending_wrap, cursor stays.
        write_str(&mut g, "abcd");
        assert_eq!(g.cursor.row, 0);
        assert_eq!(g.cursor.col, 3); // clamped at last col, pending wrap
        // Next char triggers the actual wrap.
        g.write_char('e');
        assert_eq!(g.cursor.row, 1);
        assert_eq!(g.cursor.col, 1);
        assert_eq!(g.cells[1][0].c, 'e');
        assert_eq!(g.cells[0][3].c, 'd');
        // Wrapped line should be marked soft-wrapped.
        assert!(g.line_flags[1].soft_wrapped);
    }

    #[test]
    fn auto_wrap_disabled_overwrites_last_column() {
        let mut g = TerminalGrid::new(3, 4);
        g.set_auto_wrap(false);
        write_str(&mut g, "abcdef");
        // With wrapping off, all writes pile onto the last column of row 0.
        assert_eq!(g.cursor.row, 0);
        assert_eq!(g.cursor.col, 3);
        assert_eq!(g.cells[0][3].c, 'f');
    }

    #[test]
    fn wide_char_occupies_two_cells() {
        let mut g = TerminalGrid::new(3, 10);
        g.write_char('世'); // width 2
        assert_eq!(g.cells[0][0].wide, CellWidth::Wide);
        assert_eq!(g.cells[0][1].wide, CellWidth::Spacer);
        assert_eq!(g.cursor.col, 2);
    }

    #[test]
    fn combining_char_attaches_to_previous_cell() {
        let mut g = TerminalGrid::new(3, 10);
        g.write_char('e');
        g.write_char('\u{0301}'); // combining acute accent (width 0)
        assert_eq!(g.cells[0][0].c, 'e');
        assert_eq!(g.cells[0][0].combining.as_slice(), &['\u{0301}']);
        // Combining char does not advance the cursor.
        assert_eq!(g.cursor.col, 1);
    }

    // ---- newline / carriage return / backspace / tab ----

    #[test]
    fn newline_moves_down_carriage_return_resets_col() {
        let mut g = TerminalGrid::new(5, 10);
        write_str(&mut g, "abc");
        g.newline();
        assert_eq!(g.cursor.row, 1);
        assert_eq!(g.cursor.col, 3); // newline does not change column
        g.carriage_return();
        assert_eq!(g.cursor.col, 0);
    }

    #[test]
    fn backspace_does_not_wrap_past_col_zero() {
        let mut g = TerminalGrid::new(5, 10);
        g.backspace();
        assert_eq!(g.cursor.col, 0);
        write_str(&mut g, "ab");
        g.backspace();
        assert_eq!(g.cursor.col, 1);
    }

    #[test]
    fn tab_advances_to_next_eight_stop() {
        let mut g = TerminalGrid::new(5, 40);
        g.tab();
        assert_eq!(g.cursor.col, 8);
        g.cursor.col = 9;
        g.tab();
        assert_eq!(g.cursor.col, 16);
    }

    // ---- scroll-up when passing bottom + scrollback accumulation ----

    #[test]
    fn newline_at_bottom_scrolls_and_accumulates_scrollback() {
        let mut g = TerminalGrid::new(3, 10);
        write_str(&mut g, "row0");
        g.newline();
        g.carriage_return();
        write_str(&mut g, "row1");
        g.newline();
        g.carriage_return();
        write_str(&mut g, "row2");
        // Cursor at bottom row now; another newline scrolls.
        assert_eq!(g.cursor.row, 2);
        assert_eq!(g.scrollback_len(), 0);
        g.newline();
        assert_eq!(g.cursor.row, 2); // stays at bottom
        assert_eq!(g.scrollback_len(), 1);
        // "row0" scrolled into scrollback; top visible row is now "row1".
        assert_eq!(&row_text(&g, 0)[..4], "row1");
    }

    #[test]
    fn scrollback_capped_by_max_scrollback() {
        let mut g = TerminalGrid::with_scrollback(2, 5, 3);
        // Force many scrolls.
        for _ in 0..20 {
            g.cursor.row = g.rows - 1;
            g.newline();
        }
        assert_eq!(g.scrollback_len(), 3);
    }

    // ---- clear-screen / clear-line variants ----

    #[test]
    fn erase_display_mode2_clears_everything() {
        let mut g = TerminalGrid::new(3, 5);
        write_str(&mut g, "abcde");
        g.newline();
        g.carriage_return();
        write_str(&mut g, "fghij");
        g.erase_display(2);
        assert_eq!(row_text(&g, 0), "     ");
        assert_eq!(row_text(&g, 1), "     ");
    }

    #[test]
    fn erase_display_mode0_clears_cursor_to_end() {
        let mut g = TerminalGrid::new(2, 5);
        write_str(&mut g, "abcde");
        g.cursor_to(0, 2);
        g.erase_display(0);
        assert_eq!(row_text(&g, 0), "ab   ");
    }

    #[test]
    fn erase_display_mode1_clears_start_to_cursor() {
        let mut g = TerminalGrid::new(2, 5);
        write_str(&mut g, "abcde");
        g.cursor_to(0, 2);
        g.erase_display(1);
        // cols 0..=2 cleared, 3..4 kept.
        assert_eq!(row_text(&g, 0), "   de");
    }

    #[test]
    fn erase_line_variants() {
        let mut g = TerminalGrid::new(2, 5);
        write_str(&mut g, "abcde");
        g.cursor_to(0, 2);
        g.erase_line(0); // cursor to end
        assert_eq!(row_text(&g, 0), "ab   ");

        g.cursor_to(0, 0);
        write_str(&mut g, "ABCDE");
        g.cursor_to(0, 2);
        g.erase_line(1); // start to cursor
        assert_eq!(row_text(&g, 0), "   DE");

        g.erase_line(2); // whole line
        assert_eq!(row_text(&g, 0), "     ");
    }

    #[test]
    fn erase_display_mode3_clears_scrollback() {
        let mut g = TerminalGrid::new(2, 5);
        for _ in 0..5 {
            g.cursor.row = g.rows - 1;
            g.newline();
        }
        assert!(g.scrollback_len() > 0);
        g.erase_display(3);
        assert_eq!(g.scrollback_len(), 0);
        assert_eq!(g.scroll_offset(), 0);
    }

    // ---- cursor positioning (absolute & relative) with clamping ----

    #[test]
    fn cursor_to_clamps_to_bounds() {
        let mut g = TerminalGrid::new(4, 6);
        g.cursor_to(100, 100);
        assert_eq!(g.cursor.row, 3);
        assert_eq!(g.cursor.col, 5);
        g.cursor_to(1, 2);
        assert_eq!(g.cursor.row, 1);
        assert_eq!(g.cursor.col, 2);
    }

    #[test]
    fn relative_cursor_movement_clamps() {
        let mut g = TerminalGrid::new(5, 8);
        g.cursor_to(2, 3);
        g.cursor_up(10);
        assert_eq!(g.cursor.row, 0); // clamped to scroll_top
        g.cursor_down(100);
        assert_eq!(g.cursor.row, 4); // clamped to scroll_bottom
        g.cursor_forward(100);
        assert_eq!(g.cursor.col, 7);
        g.cursor_backward(100);
        assert_eq!(g.cursor.col, 0);
    }

    #[test]
    fn origin_mode_makes_cursor_relative_to_scroll_region() {
        let mut g = TerminalGrid::new(10, 8);
        g.set_scroll_region(2, 6);
        g.set_origin_mode(true);
        // In origin mode, row 0 maps to scroll_top (2).
        g.cursor_to(0, 0);
        assert_eq!(g.cursor.row, 2);
        // Row beyond region clamps to scroll_bottom.
        g.cursor_to(100, 0);
        assert_eq!(g.cursor.row, 6);
    }

    #[test]
    fn save_and_restore_cursor() {
        let mut g = TerminalGrid::new(6, 10);
        g.cursor_to(3, 4);
        g.save_cursor();
        g.cursor_to(0, 0);
        g.restore_cursor();
        assert_eq!(g.cursor.row, 3);
        assert_eq!(g.cursor.col, 4);
    }

    // ---- scroll region ----

    #[test]
    fn set_scroll_region_homes_cursor_and_bounds_scroll() {
        let mut g = TerminalGrid::new(10, 5);
        g.set_scroll_region(2, 5);
        assert_eq!(g.cursor.row, 2);
        assert_eq!(g.cursor.col, 0);
        // Invalid region (top >= bottom) resets to full screen.
        g.set_scroll_region(5, 3);
        g.cursor_down(100);
        assert_eq!(g.cursor.row, 9);
    }

    #[test]
    fn scroll_region_confines_scrolling() {
        let mut g = TerminalGrid::new(5, 4);
        // Mark each row with a distinct char at col 0.
        for r in 0..5 {
            g.cursor_to(r, 0);
            g.write_char((b'0' + r as u8) as char);
        }
        g.set_scroll_region(1, 3);
        g.cursor_to(3, 0); // at scroll_bottom
        g.index(); // scroll within region
        // Row 0 (outside region) untouched, row 4 untouched.
        assert_eq!(g.cells[0][0].c, '0');
        assert_eq!(g.cells[4][0].c, '4');
        // Inside region, content shifted up: old row2 -> row1.
        assert_eq!(g.cells[1][0].c, '2');
        assert_eq!(g.cells[3][0].c, ' '); // new blank line
    }

    // ---- insert/delete lines and chars ----

    #[test]
    fn insert_and_delete_chars() {
        let mut g = TerminalGrid::new(2, 6);
        write_str(&mut g, "abcdef");
        g.cursor_to(0, 1);
        g.insert_chars(2);
        // "a" + 2 blanks + "bcd" (last 2 pushed off).
        assert_eq!(row_text(&g, 0), "a  bcd");

        g.cursor_to(0, 1);
        g.delete_chars(2);
        // delete the 2 blanks we inserted -> back to a bcd + blanks.
        assert_eq!(row_text(&g, 0), "abcd  ");
    }

    #[test]
    fn insert_and_delete_lines() {
        let mut g = TerminalGrid::new(4, 4);
        for r in 0..4 {
            g.cursor_to(r, 0);
            g.write_char((b'A' + r as u8) as char);
        }
        g.cursor_to(1, 0);
        g.insert_lines(1);
        // Row1 now blank, old row1 (B) shifted to row2.
        assert_eq!(g.cells[0][0].c, 'A');
        assert_eq!(g.cells[1][0].c, ' ');
        assert_eq!(g.cells[2][0].c, 'B');

        g.cursor_to(1, 0);
        g.delete_lines(1);
        // The blank line removed, B pulled back up to row1.
        assert_eq!(g.cells[1][0].c, 'B');
    }

    #[test]
    fn erase_chars_replaces_without_shift() {
        let mut g = TerminalGrid::new(2, 6);
        write_str(&mut g, "abcdef");
        g.cursor_to(0, 1);
        g.erase_chars(2);
        assert_eq!(row_text(&g, 0), "a  def");
    }

    // ---- alt-screen switch preserves / restores ----

    #[test]
    fn alt_screen_preserves_and_restores_primary() {
        let mut g = TerminalGrid::new(3, 6);
        write_str(&mut g, "primary");
        let primary_text = row_text(&g, 0);
        g.enter_alt_screen();
        // Alt screen starts blank.
        assert_eq!(row_text(&g, 0), "      ");
        write_str(&mut g, "alt");
        assert_eq!(&row_text(&g, 0)[..3], "alt");
        // Entering again is a no-op.
        g.enter_alt_screen();
        assert_eq!(&row_text(&g, 0)[..3], "alt");
        g.leave_alt_screen();
        // Primary content restored.
        assert_eq!(row_text(&g, 0), primary_text);
    }

    // ---- resize ----

    #[test]
    fn resize_preserves_content_and_clamps_cursor() {
        let mut g = TerminalGrid::new(4, 10);
        write_str(&mut g, "hello");
        g.cursor_to(3, 9);
        g.resize(2, 6);
        assert_eq!(g.rows, 2);
        assert_eq!(g.cols, 6);
        assert!(g.cursor.row < 2);
        assert!(g.cursor.col < 6);
        // Each row now has exactly new_cols cells.
        assert!(g.cells.iter().all(|r| r.len() == 6));
    }

    #[test]
    fn resize_grow_reflows_soft_wrapped_lines() {
        let mut g = TerminalGrid::new(4, 4);
        // Write 6 chars to force a soft wrap at col 4.
        write_str(&mut g, "abcdef");
        assert!(g.line_flags[1].soft_wrapped);
        g.resize(4, 10);
        // After growing, the wrapped continuation merges back into one line.
        assert_eq!(&row_text(&g, 0)[..6], "abcdef");
    }

    // ---- index / reverse_index ----

    #[test]
    fn reverse_index_scrolls_down_at_top() {
        let mut g = TerminalGrid::new(3, 4);
        for r in 0..3 {
            g.cursor_to(r, 0);
            g.write_char((b'A' + r as u8) as char);
        }
        g.cursor_to(0, 0);
        g.reverse_index();
        // Content shifts down; top row blank, old row0 (A) now at row1.
        assert_eq!(g.cells[0][0].c, ' ');
        assert_eq!(g.cells[1][0].c, 'A');
        assert_eq!(g.cursor.row, 0);
    }

    // ---- BCE (background color erase) ----

    #[test]
    fn erase_uses_current_background_color() {
        let mut g = TerminalGrid::new(2, 4);
        g.current_bg = TermColor::Named(NamedColor::Red);
        g.erase_line(2);
        assert!(g
            .cells[0]
            .iter()
            .all(|c| c.bg == TermColor::Named(NamedColor::Red)));
    }

    // ---- dirty tracking ----

    #[test]
    fn dirty_tracking_clears_and_sets() {
        let mut g = TerminalGrid::new(3, 4);
        g.clear_dirty();
        assert!(!g.is_any_dirty());
        g.mark_row_dirty(1);
        assert!(g.is_any_dirty());
        assert!(g.is_row_dirty(1));
        assert!(!g.is_row_dirty(0));
    }

    // ---- scrollback viewing ----

    #[test]
    fn scroll_view_up_and_to_bottom() {
        let mut g = TerminalGrid::new(2, 4);
        for _ in 0..5 {
            g.cursor.row = g.rows - 1;
            g.newline();
        }
        assert!(g.is_at_bottom());
        g.scroll_view_up(2);
        assert_eq!(g.scroll_offset(), 2);
        assert!(!g.is_at_bottom());
        // Cannot scroll past available scrollback.
        g.scroll_view_up(1000);
        assert_eq!(g.scroll_offset(), g.scrollback_len());
        g.scroll_view_to_bottom();
        assert!(g.is_at_bottom());
    }

    // ---- reset ----

    #[test]
    fn reset_clears_grid_but_keeps_dimensions() {
        let mut g = TerminalGrid::new(4, 8);
        write_str(&mut g, "junk");
        for _ in 0..5 {
            g.cursor.row = g.rows - 1;
            g.newline();
        }
        g.reset();
        assert_eq!(g.rows, 4);
        assert_eq!(g.cols, 8);
        assert_eq!(g.scrollback_len(), 0);
        assert_eq!(g.cursor.row, 0);
        assert_eq!(g.cursor.col, 0);
        assert_eq!(row_text(&g, 0), "        ");
    }

    // ---- selection ----

    #[test]
    fn simple_selection_membership_and_text() {
        let mut g = TerminalGrid::new(2, 6);
        write_str(&mut g, "hello");
        g.start_selection(0, 0);
        g.update_selection(2, 0);
        g.end_selection();
        assert!(g.is_selected(0, 0));
        assert!(g.is_selected(2, 0));
        assert!(!g.is_selected(3, 0));
        assert_eq!(g.selected_text().as_deref(), Some("hel"));
    }
}
