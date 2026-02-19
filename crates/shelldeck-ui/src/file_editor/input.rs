use gpui::*;

use super::view::FileEditorView;

impl FileEditorView {
    /// Main key-down handler. Dispatched from the view's `on_key_down`.
    pub fn handle_key_down(
        &mut self,
        event: &KeyDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let ks = &event.keystroke;
        let shift = ks.modifiers.shift;
        let ctrl = ks.modifiers.control || ks.modifiers.secondary();

        // ---- Search mode: capture keystrokes ----
        if self.search_visible {
            if self.handle_search_key(event, ctrl, cx) {
                return;
            }
        }

        // ---- Go-to-line mode ----
        if self.goto_line_visible {
            if self.handle_goto_line_key(event, ctrl, cx) {
                return;
            }
        }

        // ---- Ctrl shortcuts ----
        if ctrl {
            match ks.key.as_str() {
                "s" => {
                    self.save_file(cx);
                    cx.notify();
                    return;
                }
                "z" => {
                    if let Some(tab) = self.active_tab_mut() {
                        tab.buffer.undo();
                        tab.highlighter.parse_full(tab.buffer.rope());
                    }
                    self.ensure_cursor_visible();
                    cx.notify();
                    return;
                }
                "y" => {
                    if let Some(tab) = self.active_tab_mut() {
                        tab.buffer.redo();
                        tab.highlighter.parse_full(tab.buffer.rope());
                    }
                    self.ensure_cursor_visible();
                    cx.notify();
                    return;
                }
                "a" => {
                    if let Some(tab) = self.active_tab_mut() {
                        tab.buffer.select_all();
                    }
                    cx.notify();
                    return;
                }
                "d" => {
                    if let Some(tab) = self.active_tab_mut() {
                        tab.buffer.duplicate_line();
                        let pending = tab.buffer.take_pending_edits();
                        if !pending.is_empty() {
                            tab.highlighter
                                .parse_incremental(tab.buffer.rope(), &pending);
                        }
                    }
                    cx.notify();
                    return;
                }
                "c" => {
                    // Copy
                    if let Some(tab) = self.active_tab() {
                        if let Some(text) = tab.buffer.selected_text() {
                            cx.write_to_clipboard(ClipboardItem::new_string(text));
                        }
                    }
                    return;
                }
                "x" => {
                    // Cut
                    if let Some(tab) = self.active_tab() {
                        if let Some(text) = tab.buffer.selected_text() {
                            cx.write_to_clipboard(ClipboardItem::new_string(text));
                        }
                    }
                    if let Some(tab) = self.active_tab_mut() {
                        tab.buffer.delete_selection();
                        let pending = tab.buffer.take_pending_edits();
                        if !pending.is_empty() {
                            tab.highlighter
                                .parse_incremental(tab.buffer.rope(), &pending);
                        }
                    }
                    cx.notify();
                    return;
                }
                "v" => {
                    // Paste
                    if let Some(item) = cx.read_from_clipboard() {
                        if let Some(text) = item.text() {
                            if let Some(tab) = self.active_tab_mut() {
                                tab.buffer.insert_str(&text);
                                let pending = tab.buffer.take_pending_edits();
                                if !pending.is_empty() {
                                    tab.highlighter
                                        .parse_incremental(tab.buffer.rope(), &pending);
                                }
                            }
                            self.ensure_cursor_visible();
                            self.reset_cursor_blink(cx);
                            cx.notify();
                        }
                    }
                    return;
                }
                "f" => {
                    self.search_visible = !self.search_visible;
                    if !self.search_visible {
                        self.search_query.clear();
                        self.search_matches.clear();
                        self.search_current_idx = None;
                    }
                    self.goto_line_visible = false;
                    cx.notify();
                    return;
                }
                "g" => {
                    self.goto_line_visible = !self.goto_line_visible;
                    if !self.goto_line_visible {
                        self.goto_line_query.clear();
                    }
                    self.search_visible = false;
                    cx.notify();
                    return;
                }
                "b" => {
                    self.file_browser_visible = !self.file_browser_visible;
                    cx.notify();
                    return;
                }
                "w" => {
                    let idx = self.active_tab_index;
                    self.close_tab(idx, cx);
                    return;
                }
                _ => {}
            }

            // Ctrl+Shift shortcuts
            if shift {
                match ks.key.as_str() {
                    "k" => {
                        // Delete line
                        if let Some(tab) = self.active_tab_mut() {
                            tab.buffer.delete_line();
                            let pending = tab.buffer.take_pending_edits();
                            if !pending.is_empty() {
                                tab.highlighter
                                    .parse_incremental(tab.buffer.rope(), &pending);
                            }
                        }
                        cx.notify();
                        return;
                    }
                    _ => {}
                }
            }

            // Ctrl+Arrow for word movement
            match ks.key.as_str() {
                "left" => {
                    if let Some(tab) = self.active_tab_mut() {
                        tab.buffer.move_word_left(shift);
                    }
                    cx.notify();
                    return;
                }
                "right" => {
                    if let Some(tab) = self.active_tab_mut() {
                        tab.buffer.move_word_right(shift);
                    }
                    cx.notify();
                    return;
                }
                _ => {}
            }
        }

        // ---- Non-modifier keys ----
        match ks.key.as_str() {
            "left" => {
                if let Some(tab) = self.active_tab_mut() {
                    tab.buffer.move_left(shift);
                }
                self.ensure_cursor_visible();
                self.reset_cursor_blink(cx);
                cx.notify();
            }
            "right" => {
                if let Some(tab) = self.active_tab_mut() {
                    tab.buffer.move_right(shift);
                }
                self.ensure_cursor_visible();
                self.reset_cursor_blink(cx);
                cx.notify();
            }
            "up" => {
                if let Some(tab) = self.active_tab_mut() {
                    tab.buffer.move_up(shift);
                }
                self.ensure_cursor_visible();
                self.reset_cursor_blink(cx);
                cx.notify();
            }
            "down" => {
                if let Some(tab) = self.active_tab_mut() {
                    tab.buffer.move_down(shift);
                }
                self.ensure_cursor_visible();
                self.reset_cursor_blink(cx);
                cx.notify();
            }
            "home" => {
                if let Some(tab) = self.active_tab_mut() {
                    if ctrl {
                        tab.buffer.move_to_start(shift);
                    } else {
                        tab.buffer.move_home(shift);
                    }
                }
                self.ensure_cursor_visible();
                cx.notify();
            }
            "end" => {
                if let Some(tab) = self.active_tab_mut() {
                    if ctrl {
                        tab.buffer.move_to_end(shift);
                    } else {
                        tab.buffer.move_end(shift);
                    }
                }
                self.ensure_cursor_visible();
                cx.notify();
            }
            "pageup" => {
                let lines = self.scroll_lines_per_page;
                if let Some(tab) = self.active_tab_mut() {
                    tab.buffer.page_up(lines, shift);
                    tab.scroll_offset = (tab.scroll_offset - lines as f32).max(0.0);
                }
                self.ensure_cursor_visible();
                cx.notify();
            }
            "pagedown" => {
                let lines = self.scroll_lines_per_page;
                if let Some(tab) = self.active_tab_mut() {
                    tab.buffer.page_down(lines, shift);
                    tab.scroll_offset += lines as f32;
                }
                self.clamp_scroll();
                self.ensure_cursor_visible();
                cx.notify();
            }
            "enter" => {
                if let Some(tab) = self.active_tab_mut() {
                    tab.buffer.insert_newline();
                    let pending = tab.buffer.take_pending_edits();
                    if !pending.is_empty() {
                        tab.highlighter
                            .parse_incremental(tab.buffer.rope(), &pending);
                    }
                }
                self.ensure_cursor_visible();
                self.reset_cursor_blink(cx);
                cx.notify();
            }
            "tab" => {
                if shift {
                    if let Some(tab) = self.active_tab_mut() {
                        tab.buffer.dedent();
                        let pending = tab.buffer.take_pending_edits();
                        if !pending.is_empty() {
                            tab.highlighter
                                .parse_incremental(tab.buffer.rope(), &pending);
                        }
                    }
                    self.ensure_cursor_visible();
                    self.reset_cursor_blink(cx);
                } else {
                    if let Some(tab) = self.active_tab_mut() {
                        tab.buffer.insert_tab();
                        let pending = tab.buffer.take_pending_edits();
                        if !pending.is_empty() {
                            tab.highlighter
                                .parse_incremental(tab.buffer.rope(), &pending);
                        }
                    }
                    self.ensure_cursor_visible();
                    self.reset_cursor_blink(cx);
                }
                cx.notify();
            }
            "backspace" => {
                if let Some(tab) = self.active_tab_mut() {
                    tab.buffer.backspace();
                    let pending = tab.buffer.take_pending_edits();
                    if !pending.is_empty() {
                        tab.highlighter
                            .parse_incremental(tab.buffer.rope(), &pending);
                    }
                }
                self.ensure_cursor_visible();
                self.reset_cursor_blink(cx);
                cx.notify();
            }
            "delete" => {
                if let Some(tab) = self.active_tab_mut() {
                    tab.buffer.delete();
                    let pending = tab.buffer.take_pending_edits();
                    if !pending.is_empty() {
                        tab.highlighter
                            .parse_incremental(tab.buffer.rope(), &pending);
                    }
                }
                self.ensure_cursor_visible();
                self.reset_cursor_blink(cx);
                cx.notify();
            }
            "escape" => {
                // Dismiss search or goto-line
                if self.search_visible {
                    self.search_visible = false;
                    self.search_query.clear();
                    self.search_matches.clear();
                    self.search_current_idx = None;
                    cx.notify();
                } else if self.goto_line_visible {
                    self.goto_line_visible = false;
                    self.goto_line_query.clear();
                    cx.notify();
                }
            }
            _ => {
                // Printable character input
                if !ctrl && !ks.modifiers.alt {
                    if let Some(ref key_char) = ks.key_char {
                        for ch in key_char.chars() {
                            if !ch.is_control() {
                                if let Some(tab) = self.active_tab_mut() {
                                    tab.buffer.insert_char(ch);
                                    let pending = tab.buffer.take_pending_edits();
                                    if !pending.is_empty() {
                                        tab.highlighter
                                            .parse_incremental(tab.buffer.rope(), &pending);
                                    }
                                }
                            }
                        }
                        self.ensure_cursor_visible();
                        self.reset_cursor_blink(cx);
                        cx.notify();
                    }
                }
            }
        }
    }

    // -----------------------------------------------------------------------
    // Search key handling
    // -----------------------------------------------------------------------

    fn handle_search_key(
        &mut self,
        event: &KeyDownEvent,
        ctrl: bool,
        cx: &mut Context<Self>,
    ) -> bool {
        let ks = &event.keystroke;

        match ks.key.as_str() {
            "escape" => {
                self.search_visible = false;
                self.search_query.clear();
                self.search_matches.clear();
                self.search_current_idx = None;
                cx.notify();
                true
            }
            "enter" => {
                if ks.modifiers.shift {
                    self.search_prev();
                } else {
                    self.search_next();
                }
                cx.notify();
                true
            }
            "backspace" => {
                self.search_query.pop();
                self.perform_search();
                cx.notify();
                true
            }
            "f" if ctrl => {
                // Ctrl+F again closes search
                self.search_visible = false;
                self.search_query.clear();
                self.search_matches.clear();
                self.search_current_idx = None;
                cx.notify();
                true
            }
            _ => {
                if !ctrl && !ks.modifiers.alt {
                    if let Some(ref key_char) = ks.key_char {
                        for ch in key_char.chars() {
                            if !ch.is_control() {
                                self.search_query.push(ch);
                            }
                        }
                        self.perform_search();
                        cx.notify();
                        return true;
                    }
                }
                false
            }
        }
    }

    // -----------------------------------------------------------------------
    // Go-to-line key handling
    // -----------------------------------------------------------------------

    fn handle_goto_line_key(
        &mut self,
        event: &KeyDownEvent,
        ctrl: bool,
        cx: &mut Context<Self>,
    ) -> bool {
        let ks = &event.keystroke;

        match ks.key.as_str() {
            "escape" => {
                self.goto_line_visible = false;
                self.goto_line_query.clear();
                cx.notify();
                true
            }
            "enter" => {
                if let Ok(line_num) = self.goto_line_query.parse::<usize>() {
                    if let Some(tab) = self.active_tab_mut() {
                        tab.buffer.goto_line(line_num);
                    }
                    self.ensure_cursor_visible();
                }
                self.goto_line_visible = false;
                self.goto_line_query.clear();
                cx.notify();
                true
            }
            "backspace" => {
                self.goto_line_query.pop();
                cx.notify();
                true
            }
            "g" if ctrl => {
                self.goto_line_visible = false;
                self.goto_line_query.clear();
                cx.notify();
                true
            }
            _ => {
                if !ctrl && !ks.modifiers.alt {
                    if let Some(ref key_char) = ks.key_char {
                        for ch in key_char.chars() {
                            if ch.is_ascii_digit() {
                                self.goto_line_query.push(ch);
                            }
                        }
                        cx.notify();
                        return true;
                    }
                }
                false
            }
        }
    }
}
