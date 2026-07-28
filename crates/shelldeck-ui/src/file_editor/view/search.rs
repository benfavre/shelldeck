use super::*;

impl FileEditorView {
    // -----------------------------------------------------------------------
    // Search
    // -----------------------------------------------------------------------

    pub(crate) fn perform_search(&mut self) {
        self.search_matches.clear();
        self.search_current_idx = None;

        if self.search_query.is_empty() {
            return;
        }

        if let Some(buffer) = self.active_tab().and_then(|t| t.buffer()) {
            let text = buffer.text();
            let query = &self.search_query;
            let query_char_len = query.chars().count();

            if self.search_case_sensitive {
                // Case-sensitive search
                let mut byte_start = 0;
                while let Some(byte_pos) = text[byte_start..].find(query.as_str()) {
                    let abs_byte = byte_start + byte_pos;
                    let char_start = text[..abs_byte].chars().count();
                    self.search_matches
                        .push(char_start..char_start + query_char_len);
                    byte_start = abs_byte + query.len();
                }
            } else {
                // Case-insensitive search
                let query_lower = query.to_lowercase();
                let text_lower = text.to_lowercase();
                let mut byte_start = 0;
                while let Some(byte_pos) = text_lower[byte_start..].find(&query_lower) {
                    let abs_byte = byte_start + byte_pos;
                    let char_start = text[..abs_byte].chars().count();
                    self.search_matches
                        .push(char_start..char_start + query_char_len);
                    byte_start = abs_byte + query_lower.len();
                }
            }
            if !self.search_matches.is_empty() {
                self.search_current_idx = Some(0);
            }
        }
    }

    pub(crate) fn search_next(&mut self) {
        if self.search_matches.is_empty() {
            return;
        }
        let idx = self
            .search_current_idx
            .map(|i| (i + 1) % self.search_matches.len())
            .unwrap_or(0);
        self.search_current_idx = Some(idx);
        let char_pos = self.search_matches.get(idx).map(|r| r.start);
        if let Some(pos) = char_pos {
            let tab_idx = self.active_tab_index;
            if let Some(tab) = self.tabs.get_mut(tab_idx) {
                if let Some(buffer) = tab.buffer_mut() {
                    let (line, col) = buffer.char_to_line_col(pos);
                    buffer.set_cursor_from_position(line, col, false);
                }
            }
            self.ensure_cursor_visible();
        }
    }

    pub(crate) fn search_prev(&mut self) {
        if self.search_matches.is_empty() {
            return;
        }
        let match_len = self.search_matches.len();
        let idx = self
            .search_current_idx
            .map(|i| if i == 0 { match_len - 1 } else { i - 1 })
            .unwrap_or(0);
        self.search_current_idx = Some(idx);
        let char_pos = self.search_matches.get(idx).map(|r| r.start);
        if let Some(pos) = char_pos {
            let tab_idx = self.active_tab_index;
            if let Some(tab) = self.tabs.get_mut(tab_idx) {
                if let Some(buffer) = tab.buffer_mut() {
                    let (line, col) = buffer.char_to_line_col(pos);
                    buffer.set_cursor_from_position(line, col, false);
                }
            }
            self.ensure_cursor_visible();
        }
    }

    // -----------------------------------------------------------------------
    // Replace
    // -----------------------------------------------------------------------

    pub(crate) fn replace_next(&mut self, cx: &mut Context<Self>) {
        if self.search_matches.is_empty() {
            return;
        }
        let idx = self.search_current_idx.unwrap_or(0);
        let match_range = match self.search_matches.get(idx) {
            Some(r) => r.clone(),
            None => return,
        };

        let tab_idx = self.active_tab_index;
        if let Some(tab) = self.tabs.get_mut(tab_idx) {
            if let TabContent::Text {
                buffer,
                highlighter,
                ..
            } = &mut tab.content
            {
                // Set selection to the match and delete it, then insert replacement
                let (sl, sc) = buffer.char_to_line_col(match_range.start);
                let (el, ec) = buffer.char_to_line_col(match_range.end);
                buffer.set_cursor_from_position(sl, sc, false);
                buffer.set_cursor_from_position(el, ec, true);
                buffer.delete_selection();
                buffer.insert_str(&self.replace_query);
                highlighter.parse_full(buffer.rope());
            }
        }

        // Re-run search to update matches
        self.perform_search();
        // Try to keep the same index (it will point to the next match)
        if !self.search_matches.is_empty() {
            let new_idx = idx.min(self.search_matches.len() - 1);
            self.search_current_idx = Some(new_idx);
        }
        self.ensure_cursor_visible();
        cx.notify();
    }

    pub(crate) fn replace_all(&mut self, cx: &mut Context<Self>) {
        if self.search_matches.is_empty() {
            return;
        }

        let tab_idx = self.active_tab_index;
        let replace_text = self.replace_query.clone();

        if let Some(tab) = self.tabs.get_mut(tab_idx) {
            if let TabContent::Text {
                buffer,
                highlighter,
                ..
            } = &mut tab.content
            {
                // Replace from end to start to preserve earlier positions
                let mut matches: Vec<std::ops::Range<usize>> = self.search_matches.clone();
                matches.reverse();

                buffer.flush_transaction();
                for m in &matches {
                    let (sl, sc) = buffer.char_to_line_col(m.start);
                    let (el, ec) = buffer.char_to_line_col(m.end);
                    buffer.set_cursor_from_position(sl, sc, false);
                    buffer.set_cursor_from_position(el, ec, true);
                    buffer.delete_selection();
                    buffer.insert_str(&replace_text);
                }
                highlighter.parse_full(buffer.rope());
            }
        }

        self.perform_search();
        cx.notify();
    }
}
