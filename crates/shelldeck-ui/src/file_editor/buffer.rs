use ropey::Rope;
use std::ops::Range;
use std::time::Instant;

/// A selection defined by anchor (where selection started) and head (where cursor is).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Selection {
    pub anchor: usize, // char offset
    pub head: usize,   // char offset
}

impl Selection {
    pub fn range(&self) -> Range<usize> {
        let start = self.anchor.min(self.head);
        let end = self.anchor.max(self.head);
        start..end
    }

    pub fn is_empty(&self) -> bool {
        self.anchor == self.head
    }
}

/// A single edit operation for undo/redo.
#[derive(Debug, Clone)]
enum EditOp {
    Insert {
        pos: usize,
        text: String,
    },
    Delete {
        pos: usize,
        text: String,
    },
}

/// A transaction groups multiple edit operations that should be undone/redone together.
#[derive(Debug, Clone)]
struct Transaction {
    ops: Vec<EditOp>,
    cursor_before: usize,
    cursor_after: usize,
}

/// Information about an edit for tree-sitter incremental parsing.
#[derive(Debug, Clone)]
pub struct InputEditInfo {
    pub start_byte: usize,
    pub old_end_byte: usize,
    pub new_end_byte: usize,
    pub start_row: usize,
    pub start_col: usize,
    pub old_end_row: usize,
    pub old_end_col: usize,
    pub new_end_row: usize,
    pub new_end_col: usize,
}

pub struct RopeBuffer {
    rope: Rope,
    cursor: usize,              // char offset
    selection: Option<Selection>,
    desired_col: Option<usize>, // for vertical nav
    tab_size: usize,
    undo_stack: Vec<Transaction>,
    redo_stack: Vec<Transaction>,
    current_transaction: Option<Transaction>,
    last_edit_time: Instant,
    dirty: bool,
    /// Accumulated input edits for tree-sitter since last query.
    pending_edits: Vec<InputEditInfo>,
}

impl RopeBuffer {
    pub fn new(text: &str) -> Self {
        Self {
            rope: Rope::from_str(text),
            cursor: 0,
            selection: None,
            desired_col: None,
            tab_size: 4,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            current_transaction: None,
            last_edit_time: Instant::now(),
            dirty: false,
            pending_edits: Vec::new(),
        }
    }

    pub fn rope(&self) -> &Rope {
        &self.rope
    }

    pub fn text(&self) -> String {
        self.rope.to_string()
    }

    pub fn len_chars(&self) -> usize {
        self.rope.len_chars()
    }

    pub fn len_lines(&self) -> usize {
        self.rope.len_lines()
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn selection(&self) -> Option<&Selection> {
        self.selection.as_ref()
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    pub fn set_dirty(&mut self, dirty: bool) {
        self.dirty = dirty;
    }

    pub fn set_tab_size(&mut self, size: usize) {
        self.tab_size = size;
    }

    /// Take pending edits for tree-sitter processing.
    pub fn take_pending_edits(&mut self) -> Vec<InputEditInfo> {
        std::mem::take(&mut self.pending_edits)
    }

    // -----------------------------------------------------------------------
    // Coordinate helpers
    // -----------------------------------------------------------------------

    /// Returns (line, col) for a char offset. Both 0-indexed.
    pub fn cursor_line_col(&self) -> (usize, usize) {
        self.char_to_line_col(self.cursor)
    }

    pub fn char_to_line_col(&self, char_idx: usize) -> (usize, usize) {
        let char_idx = char_idx.min(self.rope.len_chars());
        let line = self.rope.char_to_line(char_idx);
        let line_start = self.rope.line_to_char(line);
        (line, char_idx - line_start)
    }

    pub fn line_col_to_char(&self, line: usize, col: usize) -> usize {
        let line = line.min(self.rope.len_lines().saturating_sub(1));
        let line_start = self.rope.line_to_char(line);
        let line_len = self.line_len_chars(line);
        line_start + col.min(line_len)
    }

    /// Length of line in chars (excluding trailing newline).
    pub fn line_len_chars(&self, line: usize) -> usize {
        if line >= self.rope.len_lines() {
            return 0;
        }
        let line_slice = self.rope.line(line);
        let len = line_slice.len_chars();
        // Strip trailing newline
        if len > 0 {
            let last = line_slice.char(len - 1);
            if last == '\n' {
                if len > 1 && line_slice.char(len - 2) == '\r' {
                    len - 2
                } else {
                    len - 1
                }
            } else {
                len
            }
        } else {
            0
        }
    }

    /// Get text of a line (without trailing newline).
    pub fn line_text(&self, line: usize) -> String {
        if line >= self.rope.len_lines() {
            return String::new();
        }
        let slice = self.rope.line(line);
        let mut s = slice.to_string();
        if s.ends_with('\n') {
            s.pop();
            if s.ends_with('\r') {
                s.pop();
            }
        }
        s
    }

    // -----------------------------------------------------------------------
    // Transaction / undo management
    // -----------------------------------------------------------------------

    fn should_coalesce(&self) -> bool {
        if self.current_transaction.is_none() {
            return false;
        }
        self.last_edit_time.elapsed().as_millis() < 500
    }

    fn start_or_extend_transaction(&mut self, force_new: bool) {
        if force_new || !self.should_coalesce() {
            self.finalize_transaction();
            self.current_transaction = Some(Transaction {
                ops: Vec::new(),
                cursor_before: self.cursor,
                cursor_after: self.cursor,
            });
        }
    }

    fn record_op(&mut self, op: EditOp) {
        if let Some(ref mut txn) = self.current_transaction {
            txn.ops.push(op);
            txn.cursor_after = self.cursor;
        }
        self.last_edit_time = Instant::now();
        self.redo_stack.clear();
        self.dirty = true;
    }

    fn finalize_transaction(&mut self) {
        if let Some(txn) = self.current_transaction.take() {
            if !txn.ops.is_empty() {
                self.undo_stack.push(txn);
            }
        }
    }

    pub fn undo(&mut self) {
        self.finalize_transaction();
        if let Some(txn) = self.undo_stack.pop() {
            let cursor_before = txn.cursor_before;
            // Apply ops in reverse
            for op in txn.ops.iter().rev() {
                match op {
                    EditOp::Insert { pos, text } => {
                        let char_end = pos + text.chars().count();
                        self.rope.remove(*pos..char_end);
                    }
                    EditOp::Delete { pos, text } => {
                        self.rope.insert(*pos, text);
                    }
                }
            }
            self.cursor = cursor_before;
            self.selection = None;
            self.desired_col = None;
            self.redo_stack.push(txn);
            self.dirty = true;
            // Mark full reparse needed
            self.pending_edits.clear();
        }
    }

    pub fn redo(&mut self) {
        self.finalize_transaction();
        if let Some(txn) = self.redo_stack.pop() {
            let cursor_after = txn.cursor_after;
            for op in &txn.ops {
                match op {
                    EditOp::Insert { pos, text } => {
                        self.rope.insert(*pos, text);
                    }
                    EditOp::Delete { pos, text } => {
                        let char_end = pos + text.chars().count();
                        self.rope.remove(*pos..char_end);
                    }
                }
            }
            self.cursor = cursor_after;
            self.selection = None;
            self.desired_col = None;
            self.undo_stack.push(txn);
            self.dirty = true;
            self.pending_edits.clear();
        }
    }

    // -----------------------------------------------------------------------
    // Input edit tracking (for tree-sitter)
    // -----------------------------------------------------------------------

    /// Returns (line, byte_column_within_line) for a char offset.
    /// tree-sitter Point expects byte column, not char column.
    fn char_to_line_byte_col(&self, char_idx: usize) -> (usize, usize) {
        let char_idx = char_idx.min(self.rope.len_chars());
        let line = self.rope.char_to_line(char_idx);
        let byte = self.rope.char_to_byte(char_idx);
        let line_start_byte = self.rope.line_to_byte(line);
        (line, byte - line_start_byte)
    }

    fn record_input_edit_insert(&mut self, char_pos: usize, text: &str) {
        let start_byte = self.rope.char_to_byte(char_pos);
        let (start_row, start_byte_col) = self.char_to_line_byte_col(char_pos);
        let byte_len = text.len();
        let new_end_byte = start_byte + byte_len;

        // Calculate new end position
        let new_chars = text.chars().count();
        let new_end_char = char_pos + new_chars;
        let (new_end_row, new_end_byte_col) = self.char_to_line_byte_col(new_end_char);

        self.pending_edits.push(InputEditInfo {
            start_byte,
            old_end_byte: start_byte,
            new_end_byte,
            start_row,
            start_col: start_byte_col,
            old_end_row: start_row,
            old_end_col: start_byte_col,
            new_end_row,
            new_end_col: new_end_byte_col,
        });
    }

    fn record_input_edit_delete(&mut self, char_start: usize, char_end: usize) {
        // Must be called BEFORE the deletion happens on the rope
        let start_byte = self.rope.char_to_byte(char_start);
        let old_end_byte = self.rope.char_to_byte(char_end);
        let (start_row, start_byte_col) = self.char_to_line_byte_col(char_start);
        let (old_end_row, old_end_byte_col) = self.char_to_line_byte_col(char_end);

        self.pending_edits.push(InputEditInfo {
            start_byte,
            old_end_byte,
            new_end_byte: start_byte,
            start_row,
            start_col: start_byte_col,
            old_end_row,
            old_end_col: old_end_byte_col,
            new_end_row: start_row,
            new_end_col: start_byte_col,
        });
    }

    // -----------------------------------------------------------------------
    // Selection helpers
    // -----------------------------------------------------------------------

    pub fn delete_selection(&mut self) -> Option<String> {
        let sel = self.selection.take()?;
        let range = sel.range();
        if range.is_empty() {
            return None;
        }
        let deleted: String = self.rope.slice(range.start..range.end).to_string();

        self.record_input_edit_delete(range.start, range.end);
        self.rope.remove(range.start..range.end);
        self.cursor = range.start;

        self.record_op(EditOp::Delete {
            pos: range.start,
            text: deleted.clone(),
        });

        Some(deleted)
    }

    pub fn selected_text(&self) -> Option<String> {
        let sel = self.selection.as_ref()?;
        let range = sel.range();
        if range.is_empty() {
            return None;
        }
        Some(self.rope.slice(range.start..range.end).to_string())
    }

    pub fn select_all(&mut self) {
        self.selection = Some(Selection {
            anchor: 0,
            head: self.rope.len_chars(),
        });
        self.cursor = self.rope.len_chars();
    }

    fn extend_or_clear_selection(&mut self, extend: bool) {
        if extend {
            if self.selection.is_none() {
                self.selection = Some(Selection {
                    anchor: self.cursor,
                    head: self.cursor,
                });
            }
        } else {
            self.selection = None;
        }
    }

    fn update_selection_head(&mut self) {
        if let Some(ref mut sel) = self.selection {
            sel.head = self.cursor;
        }
    }

    // -----------------------------------------------------------------------
    // Editing operations
    // -----------------------------------------------------------------------

    pub fn insert_char(&mut self, ch: char) {
        self.start_or_extend_transaction(false);
        self.delete_selection();

        let pos = self.cursor;
        let s = ch.to_string();
        self.rope.insert(pos, &s);
        self.record_input_edit_insert(pos, &s);
        self.cursor = pos + 1;

        self.record_op(EditOp::Insert { pos, text: s });
        self.desired_col = None;
    }

    pub fn insert_str(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        self.start_or_extend_transaction(true); // force new for paste
        self.delete_selection();

        let pos = self.cursor;
        self.rope.insert(pos, text);
        self.record_input_edit_insert(pos, text);
        let char_count = text.chars().count();
        self.cursor = pos + char_count;

        self.record_op(EditOp::Insert {
            pos,
            text: text.to_string(),
        });
        self.desired_col = None;
    }

    pub fn insert_newline(&mut self) {
        self.start_or_extend_transaction(true); // break coalescing
        self.delete_selection();

        // Auto-indent: copy leading whitespace from current line
        let (line, _) = self.cursor_line_col();
        let line_text = self.line_text(line);
        let indent: String = line_text.chars().take_while(|c| c.is_whitespace()).collect();

        let text = format!("\n{}", indent);
        let pos = self.cursor;
        self.rope.insert(pos, &text);
        self.record_input_edit_insert(pos, &text);
        self.cursor = pos + text.chars().count();

        self.record_op(EditOp::Insert { pos, text });
        self.desired_col = None;
    }

    pub fn insert_tab(&mut self) {
        let spaces = " ".repeat(self.tab_size);
        self.start_or_extend_transaction(false);
        self.delete_selection();

        let pos = self.cursor;
        self.rope.insert(pos, &spaces);
        self.record_input_edit_insert(pos, &spaces);
        self.cursor = pos + self.tab_size;

        self.record_op(EditOp::Insert {
            pos,
            text: spaces,
        });
        self.desired_col = None;
    }

    /// Remove up to tab_size leading spaces from the current line.
    pub fn dedent(&mut self) {
        let (line, _) = self.cursor_line_col();
        let line_start = self.rope.line_to_char(line);
        let line_text = self.line_text(line);

        // Count leading spaces (up to tab_size)
        let leading_spaces: usize = line_text
            .chars()
            .take(self.tab_size)
            .take_while(|c| *c == ' ')
            .count();
        if leading_spaces == 0 {
            // Try removing a single leading tab
            if line_text.starts_with('\t') {
                self.start_or_extend_transaction(true);
                let deleted = "\t".to_string();
                self.record_input_edit_delete(line_start, line_start + 1);
                self.rope.remove(line_start..line_start + 1);
                if self.cursor > line_start {
                    self.cursor = self.cursor.saturating_sub(1);
                }
                self.record_op(EditOp::Delete {
                    pos: line_start,
                    text: deleted,
                });
            }
            return;
        }

        self.start_or_extend_transaction(true);
        let del_end = line_start + leading_spaces;
        let deleted: String = self.rope.slice(line_start..del_end).to_string();
        self.record_input_edit_delete(line_start, del_end);
        self.rope.remove(line_start..del_end);

        // Adjust cursor
        if self.cursor >= del_end {
            self.cursor -= leading_spaces;
        } else if self.cursor > line_start {
            self.cursor = line_start;
        }

        self.record_op(EditOp::Delete {
            pos: line_start,
            text: deleted,
        });
        self.desired_col = None;
    }

    pub fn backspace(&mut self) {
        if self.delete_selection().is_some() {
            return;
        }
        if self.cursor == 0 {
            return;
        }
        self.start_or_extend_transaction(false);

        let del_start = self.cursor - 1;
        let deleted: String = self.rope.slice(del_start..self.cursor).to_string();

        self.record_input_edit_delete(del_start, self.cursor);
        self.rope.remove(del_start..self.cursor);
        self.cursor = del_start;

        self.record_op(EditOp::Delete {
            pos: del_start,
            text: deleted,
        });
        self.desired_col = None;
    }

    pub fn delete(&mut self) {
        if self.delete_selection().is_some() {
            return;
        }
        if self.cursor >= self.rope.len_chars() {
            return;
        }
        self.start_or_extend_transaction(false);

        let del_end = self.cursor + 1;
        let deleted: String = self.rope.slice(self.cursor..del_end).to_string();

        self.record_input_edit_delete(self.cursor, del_end);
        self.rope.remove(self.cursor..del_end);

        self.record_op(EditOp::Delete {
            pos: self.cursor,
            text: deleted,
        });
        self.desired_col = None;
    }

    pub fn delete_line(&mut self) {
        let (line, _) = self.cursor_line_col();
        let line_start = self.rope.line_to_char(line);
        let line_end = if line + 1 < self.rope.len_lines() {
            self.rope.line_to_char(line + 1)
        } else {
            self.rope.len_chars()
        };
        if line_start == line_end {
            return;
        }

        self.start_or_extend_transaction(true);
        let deleted: String = self.rope.slice(line_start..line_end).to_string();

        self.record_input_edit_delete(line_start, line_end);
        self.rope.remove(line_start..line_end);
        self.cursor = line_start.min(self.rope.len_chars());

        self.record_op(EditOp::Delete {
            pos: line_start,
            text: deleted,
        });
        self.selection = None;
        self.desired_col = None;
    }

    pub fn duplicate_line(&mut self) {
        let (line, _) = self.cursor_line_col();
        let line_text = self.line_text(line);
        let line_end = if line + 1 < self.rope.len_lines() {
            self.rope.line_to_char(line + 1)
        } else {
            self.rope.len_chars()
        };

        self.start_or_extend_transaction(true);

        let insert_text = if line + 1 >= self.rope.len_lines() {
            format!("\n{}", line_text)
        } else {
            format!("{}\n", line_text)
        };

        self.rope.insert(line_end, &insert_text);
        self.record_input_edit_insert(line_end, &insert_text);

        self.record_op(EditOp::Insert {
            pos: line_end,
            text: insert_text,
        });
        self.desired_col = None;
    }

    // -----------------------------------------------------------------------
    // Navigation
    // -----------------------------------------------------------------------

    pub fn move_left(&mut self, extend_selection: bool) {
        if !extend_selection {
            if let Some(sel) = self.selection.take() {
                let range = sel.range();
                if !range.is_empty() {
                    self.cursor = range.start;
                    self.desired_col = None;
                    return;
                }
            }
        }
        self.extend_or_clear_selection(extend_selection);
        if self.cursor > 0 {
            self.cursor -= 1;
        }
        self.update_selection_head();
        self.desired_col = None;
    }

    pub fn move_right(&mut self, extend_selection: bool) {
        if !extend_selection {
            if let Some(sel) = self.selection.take() {
                let range = sel.range();
                if !range.is_empty() {
                    self.cursor = range.end;
                    self.desired_col = None;
                    return;
                }
            }
        }
        self.extend_or_clear_selection(extend_selection);
        if self.cursor < self.rope.len_chars() {
            self.cursor += 1;
        }
        self.update_selection_head();
        self.desired_col = None;
    }

    pub fn move_up(&mut self, extend_selection: bool) {
        self.extend_or_clear_selection(extend_selection);
        let (line, col) = self.cursor_line_col();
        if line == 0 {
            self.cursor = 0;
            self.update_selection_head();
            return;
        }
        let target_col = self.desired_col.unwrap_or(col);
        self.cursor = self.line_col_to_char(line - 1, target_col);
        self.desired_col = Some(target_col);
        self.update_selection_head();
    }

    pub fn move_down(&mut self, extend_selection: bool) {
        self.extend_or_clear_selection(extend_selection);
        let (line, col) = self.cursor_line_col();
        if line >= self.rope.len_lines().saturating_sub(1) {
            self.cursor = self.rope.len_chars();
            self.update_selection_head();
            return;
        }
        let target_col = self.desired_col.unwrap_or(col);
        self.cursor = self.line_col_to_char(line + 1, target_col);
        self.desired_col = Some(target_col);
        self.update_selection_head();
    }

    pub fn move_word_left(&mut self, extend_selection: bool) {
        self.extend_or_clear_selection(extend_selection);
        if self.cursor == 0 {
            return;
        }
        let mut pos = self.cursor - 1;
        // Skip whitespace
        while pos > 0 && self.char_at(pos).map_or(false, |c| c.is_whitespace()) {
            pos -= 1;
        }
        // Skip word chars
        while pos > 0 && self.char_at(pos - 1).map_or(false, |c| c.is_alphanumeric() || c == '_') {
            pos -= 1;
        }
        self.cursor = pos;
        self.update_selection_head();
        self.desired_col = None;
    }

    pub fn move_word_right(&mut self, extend_selection: bool) {
        self.extend_or_clear_selection(extend_selection);
        let len = self.rope.len_chars();
        if self.cursor >= len {
            return;
        }
        let mut pos = self.cursor;
        // Skip word chars
        while pos < len && self.char_at(pos).map_or(false, |c| c.is_alphanumeric() || c == '_') {
            pos += 1;
        }
        // Skip whitespace
        while pos < len && self.char_at(pos).map_or(false, |c| c.is_whitespace()) {
            pos += 1;
        }
        self.cursor = pos;
        self.update_selection_head();
        self.desired_col = None;
    }

    pub fn move_home(&mut self, extend_selection: bool) {
        self.extend_or_clear_selection(extend_selection);
        let (line, col) = self.cursor_line_col();
        let line_text = self.line_text(line);
        let indent_len = line_text.chars().take_while(|c| c.is_whitespace()).count();

        // Smart home: if already at indent, go to col 0; otherwise go to indent
        let target_col = if col == indent_len && indent_len > 0 {
            0
        } else {
            indent_len
        };

        self.cursor = self.line_col_to_char(line, target_col);
        self.update_selection_head();
        self.desired_col = None;
    }

    pub fn move_end(&mut self, extend_selection: bool) {
        self.extend_or_clear_selection(extend_selection);
        let (line, _) = self.cursor_line_col();
        let line_len = self.line_len_chars(line);
        self.cursor = self.line_col_to_char(line, line_len);
        self.update_selection_head();
        self.desired_col = None;
    }

    pub fn move_to_start(&mut self, extend_selection: bool) {
        self.extend_or_clear_selection(extend_selection);
        self.cursor = 0;
        self.update_selection_head();
        self.desired_col = None;
    }

    pub fn move_to_end(&mut self, extend_selection: bool) {
        self.extend_or_clear_selection(extend_selection);
        self.cursor = self.rope.len_chars();
        self.update_selection_head();
        self.desired_col = None;
    }

    pub fn page_up(&mut self, lines: usize, extend_selection: bool) {
        self.extend_or_clear_selection(extend_selection);
        let (line, col) = self.cursor_line_col();
        let target_col = self.desired_col.unwrap_or(col);
        let new_line = line.saturating_sub(lines);
        self.cursor = self.line_col_to_char(new_line, target_col);
        self.desired_col = Some(target_col);
        self.update_selection_head();
    }

    pub fn page_down(&mut self, lines: usize, extend_selection: bool) {
        self.extend_or_clear_selection(extend_selection);
        let (line, col) = self.cursor_line_col();
        let target_col = self.desired_col.unwrap_or(col);
        let max_line = self.rope.len_lines().saturating_sub(1);
        let new_line = (line + lines).min(max_line);
        self.cursor = self.line_col_to_char(new_line, target_col);
        self.desired_col = Some(target_col);
        self.update_selection_head();
    }

    /// Set cursor from a (line, col) position (0-indexed). Used for mouse clicks.
    pub fn set_cursor_from_position(&mut self, line: usize, col: usize, extend_selection: bool) {
        self.extend_or_clear_selection(extend_selection);
        self.cursor = self.line_col_to_char(line, col);
        self.update_selection_head();
        self.desired_col = None;
    }

    /// Go to a specific line number (1-indexed for user display).
    pub fn goto_line(&mut self, line_number: usize) {
        let line = line_number.saturating_sub(1);
        let max_line = self.rope.len_lines().saturating_sub(1);
        let target_line = line.min(max_line);
        self.cursor = self.rope.line_to_char(target_line);
        self.selection = None;
        self.desired_col = None;
    }

    /// Select the word at the current cursor position (for double-click).
    pub fn select_word_at_cursor(&mut self) {
        let len = self.rope.len_chars();
        if len == 0 {
            return;
        }
        let pos = self.cursor.min(len.saturating_sub(1));
        let ch = self.char_at(pos).unwrap_or(' ');

        if ch.is_alphanumeric() || ch == '_' {
            let mut start = pos;
            while start > 0
                && self
                    .char_at(start - 1)
                    .map_or(false, |c| c.is_alphanumeric() || c == '_')
            {
                start -= 1;
            }
            let mut end = pos + 1;
            while end < len
                && self
                    .char_at(end)
                    .map_or(false, |c| c.is_alphanumeric() || c == '_')
            {
                end += 1;
            }
            self.selection = Some(Selection {
                anchor: start,
                head: end,
            });
            self.cursor = end;
        }
    }

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    fn char_at(&self, pos: usize) -> Option<char> {
        if pos < self.rope.len_chars() {
            Some(self.rope.char(pos))
        } else {
            None
        }
    }

    /// Finalize any open transaction (call before save, undo, etc.)
    pub fn flush_transaction(&mut self) {
        self.finalize_transaction();
    }

    pub fn tab_size(&self) -> usize {
        self.tab_size
    }

    /// Convert char column to visual column (accounting for tab expansion).
    pub fn char_col_to_visual_col(&self, line: usize, char_col: usize) -> usize {
        let line_text = self.line_text(line);
        let mut vcol = 0;
        for (i, ch) in line_text.chars().enumerate() {
            if i >= char_col {
                break;
            }
            if ch == '\t' {
                vcol += self.tab_size - (vcol % self.tab_size);
            } else {
                vcol += 1;
            }
        }
        vcol
    }

    /// Convert visual column to char column (inverse of char_col_to_visual_col).
    pub fn visual_col_to_char_col(&self, line: usize, visual_col: usize) -> usize {
        let line_text = self.line_text(line);
        let mut vcol = 0;
        for (i, ch) in line_text.chars().enumerate() {
            if vcol >= visual_col {
                return i;
            }
            if ch == '\t' {
                vcol += self.tab_size - (vcol % self.tab_size);
            } else {
                vcol += 1;
            }
        }
        line_text.chars().count()
    }
}
