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
        let alt = ks.modifiers.alt;

        // ---- Unsaved warning mode: capture keystrokes ----
        if self.pending_close_tab.is_some() {
            // Only allow Escape to cancel, or specific keys handled by the warning bar buttons
            if ks.key.as_str() == "escape" {
                self.pending_close_tab = None;
                cx.notify();
            }
            return;
        }

        // ---- Search/Replace mode: capture keystrokes ----
        if self.search_visible || self.replace_visible {
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

        // ---- Context menu: dismiss on any key ----
        if self.context_menu_visible {
            self.context_menu_visible = false;
            cx.notify();
            if ks.key.as_str() == "escape" {
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
                "l" => {
                    // Select line
                    if let Some(tab) = self.active_tab_mut() {
                        tab.buffer.select_line();
                    }
                    self.ensure_cursor_visible();
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
                        self.replace_visible = false;
                        self.replace_query.clear();
                        self.search_focus_replace = false;
                    }
                    self.goto_line_visible = false;
                    cx.notify();
                    return;
                }
                "h" => {
                    // Toggle replace bar (opens search too if not visible)
                    self.replace_visible = !self.replace_visible;
                    if self.replace_visible {
                        self.search_visible = true;
                    } else {
                        self.replace_query.clear();
                        self.search_focus_replace = false;
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
                    self.replace_visible = false;
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
                "/" => {
                    // Toggle line comment
                    if let Some(prefix) = self
                        .active_tab()
                        .and_then(|t| t.language.comment_prefix())
                        .map(|s| s.to_string())
                    {
                        if let Some(tab) = self.active_tab_mut() {
                            tab.buffer.toggle_line_comment(&prefix);
                            tab.highlighter.parse_full(tab.buffer.rope());
                        }
                        self.ensure_cursor_visible();
                        cx.notify();
                    }
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

        // ---- Alt shortcuts ----
        if alt && !ctrl {
            match ks.key.as_str() {
                "up" => {
                    // Move line up
                    if let Some(tab) = self.active_tab_mut() {
                        tab.buffer.move_line_up();
                        tab.highlighter.parse_full(tab.buffer.rope());
                    }
                    self.ensure_cursor_visible();
                    self.reset_cursor_blink(cx);
                    cx.notify();
                    return;
                }
                "down" => {
                    // Move line down
                    if let Some(tab) = self.active_tab_mut() {
                        tab.buffer.move_line_down();
                        tab.highlighter.parse_full(tab.buffer.rope());
                    }
                    self.ensure_cursor_visible();
                    self.reset_cursor_blink(cx);
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
                    // Dedent: try multi-line first, then single-line
                    if let Some(tab) = self.active_tab_mut() {
                        if !tab.buffer.dedent_selection() {
                            tab.buffer.dedent();
                        }
                        let pending = tab.buffer.take_pending_edits();
                        if !pending.is_empty() {
                            tab.highlighter
                                .parse_incremental(tab.buffer.rope(), &pending);
                        }
                    }
                    self.ensure_cursor_visible();
                    self.reset_cursor_blink(cx);
                } else {
                    // Indent: try multi-line first, then single-line
                    if let Some(tab) = self.active_tab_mut() {
                        if !tab.buffer.indent_selection() {
                            tab.buffer.insert_tab();
                        }
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
                    // Try pair-aware backspace first, then normal
                    if !tab.buffer.try_backspace_pair() {
                        tab.buffer.backspace();
                    }
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
                    self.replace_visible = false;
                    self.search_query.clear();
                    self.replace_query.clear();
                    self.search_matches.clear();
                    self.search_current_idx = None;
                    self.search_focus_replace = false;
                    cx.notify();
                } else if self.goto_line_visible {
                    self.goto_line_visible = false;
                    self.goto_line_query.clear();
                    cx.notify();
                }
            }
            _ => {
                // Printable character input
                if !ctrl && !alt {
                    if let Some(ref key_char) = ks.key_char {
                        for ch in key_char.chars() {
                            if !ch.is_control() {
                                if let Some(tab) = self.active_tab_mut() {
                                    // Try auto-pair first, then normal insert
                                    if !tab.buffer.try_auto_pair(ch) {
                                        tab.buffer.insert_char(ch);
                                    }
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
    // Search key handling (also handles replace bar input)
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
                self.replace_visible = false;
                self.search_query.clear();
                self.replace_query.clear();
                self.search_matches.clear();
                self.search_current_idx = None;
                self.search_focus_replace = false;
                cx.notify();
                true
            }
            "enter" => {
                if ctrl && ks.modifiers.shift {
                    // Ctrl+Shift+Enter = replace all
                    self.replace_all(cx);
                } else if self.search_focus_replace && self.replace_visible {
                    // Enter in replace field = replace next
                    self.replace_next(cx);
                } else if ks.modifiers.shift {
                    self.search_prev();
                } else {
                    self.search_next();
                }
                cx.notify();
                true
            }
            "tab" => {
                // Toggle focus between search and replace fields
                if self.replace_visible {
                    self.search_focus_replace = !self.search_focus_replace;
                    cx.notify();
                }
                true
            }
            "backspace" => {
                if self.search_focus_replace && self.replace_visible {
                    self.replace_query.pop();
                } else {
                    self.search_query.pop();
                    self.perform_search();
                }
                cx.notify();
                true
            }
            "f" if ctrl => {
                // Ctrl+F again closes search
                self.search_visible = false;
                self.replace_visible = false;
                self.search_query.clear();
                self.replace_query.clear();
                self.search_matches.clear();
                self.search_current_idx = None;
                self.search_focus_replace = false;
                cx.notify();
                true
            }
            "h" if ctrl => {
                // Ctrl+H toggles replace bar
                self.replace_visible = !self.replace_visible;
                if !self.replace_visible {
                    self.replace_query.clear();
                    self.search_focus_replace = false;
                }
                cx.notify();
                true
            }
            _ => {
                // Alt+C toggles case sensitivity
                if ks.modifiers.alt && ks.key.as_str() == "c" {
                    self.search_case_sensitive = !self.search_case_sensitive;
                    self.perform_search();
                    cx.notify();
                    return true;
                }

                if !ctrl && !ks.modifiers.alt {
                    if let Some(ref key_char) = ks.key_char {
                        for ch in key_char.chars() {
                            if !ch.is_control() {
                                if self.search_focus_replace && self.replace_visible {
                                    self.replace_query.push(ch);
                                } else {
                                    self.search_query.push(ch);
                                }
                            }
                        }
                        if !self.search_focus_replace {
                            self.perform_search();
                        }
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
