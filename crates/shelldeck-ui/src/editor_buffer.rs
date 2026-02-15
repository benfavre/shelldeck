/// Cursor-aware text buffer for the script body editor.
/// Plain struct with no GPUI dependency.
pub struct EditorBuffer {
    text: String,
    cursor: usize, // byte offset
    desired_col: Option<usize>,
    tab_size: usize,
}

impl EditorBuffer {
    pub fn new() -> Self {
        Self {
            text: String::new(),
            cursor: 0,
            desired_col: None,
            tab_size: 2,
        }
    }

    pub fn from_text(text: String) -> Self {
        let cursor = text.len();
        Self {
            text,
            cursor,
            desired_col: None,
            tab_size: 2,
        }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    pub fn line_count(&self) -> usize {
        self.text.split('\n').count()
    }

    /// Returns (line, col) both 0-indexed, in char offsets.
    pub fn cursor_line_col(&self) -> (usize, usize) {
        let before = &self.text[..self.cursor];
        let line = before.matches('\n').count();
        let line_start = before.rfind('\n').map(|p| p + 1).unwrap_or(0);
        let col = before[line_start..].chars().count();
        (line, col)
    }

    // ---- Editing ----

    pub fn insert_char(&mut self, c: char) {
        self.text.insert(self.cursor, c);
        self.cursor += c.len_utf8();
        self.desired_col = None;
    }

    pub fn insert_str(&mut self, s: &str) {
        self.text.insert_str(self.cursor, s);
        self.cursor += s.len();
        self.desired_col = None;
    }

    pub fn insert_tab(&mut self) {
        let spaces: String = std::iter::repeat(' ').take(self.tab_size).collect();
        self.insert_str(&spaces);
    }

    /// Insert newline with auto-indent matching the current line's leading whitespace.
    pub fn insert_newline(&mut self) {
        let before = &self.text[..self.cursor];
        let line_start = before.rfind('\n').map(|p| p + 1).unwrap_or(0);
        let current_line = &before[line_start..];
        let indent: String = current_line
            .chars()
            .take_while(|c| *c == ' ' || *c == '\t')
            .collect();
        let insertion = format!("\n{}", indent);
        self.text.insert_str(self.cursor, &insertion);
        self.cursor += insertion.len();
        self.desired_col = None;
    }

    pub fn backspace(&mut self) {
        if self.cursor == 0 {
            return;
        }
        // Find previous char boundary
        let prev = self.text[..self.cursor]
            .char_indices()
            .next_back()
            .map(|(i, _)| i)
            .unwrap_or(0);
        self.text.drain(prev..self.cursor);
        self.cursor = prev;
        self.desired_col = None;
    }

    pub fn delete(&mut self) {
        if self.cursor >= self.text.len() {
            return;
        }
        // Find next char boundary
        let next = self.cursor
            + self.text[self.cursor..]
                .chars()
                .next()
                .map(|c| c.len_utf8())
                .unwrap_or(0);
        self.text.drain(self.cursor..next);
        self.desired_col = None;
    }

    // ---- Navigation ----

    pub fn move_left(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let prev = self.text[..self.cursor]
            .char_indices()
            .next_back()
            .map(|(i, _)| i)
            .unwrap_or(0);
        self.cursor = prev;
        self.desired_col = None;
    }

    pub fn move_right(&mut self) {
        if self.cursor >= self.text.len() {
            return;
        }
        let next = self.cursor
            + self.text[self.cursor..]
                .chars()
                .next()
                .map(|c| c.len_utf8())
                .unwrap_or(0);
        self.cursor = next;
        self.desired_col = None;
    }

    pub fn move_up(&mut self) {
        let (line, col) = self.cursor_line_col();
        let target_col = self.desired_col.unwrap_or(col);
        if line == 0 {
            return;
        }
        self.cursor = self.line_col_to_byte(line - 1, target_col);
        self.desired_col = Some(target_col);
    }

    pub fn move_down(&mut self) {
        let (line, col) = self.cursor_line_col();
        let target_col = self.desired_col.unwrap_or(col);
        if line + 1 >= self.line_count() {
            return;
        }
        self.cursor = self.line_col_to_byte(line + 1, target_col);
        self.desired_col = Some(target_col);
    }

    pub fn move_home(&mut self) {
        let before = &self.text[..self.cursor];
        let line_start = before.rfind('\n').map(|p| p + 1).unwrap_or(0);
        self.cursor = line_start;
        self.desired_col = None;
    }

    pub fn move_end(&mut self) {
        let after = &self.text[self.cursor..];
        let line_end = after
            .find('\n')
            .map(|p| self.cursor + p)
            .unwrap_or(self.text.len());
        self.cursor = line_end;
        self.desired_col = None;
    }

    // ---- Helpers ----

    /// Convert (line, col) in char offsets to byte offset, clamped to the line length.
    fn line_col_to_byte(&self, target_line: usize, target_col: usize) -> usize {
        let mut byte = 0;
        for (i, line) in self.text.split('\n').enumerate() {
            if i == target_line {
                let char_count = line.chars().count();
                let clamped_col = target_col.min(char_count);
                let col_bytes: usize = line.chars().take(clamped_col).map(|c| c.len_utf8()).sum();
                return byte + col_bytes;
            }
            byte += line.len() + 1; // +1 for '\n'
        }
        self.text.len()
    }
}
