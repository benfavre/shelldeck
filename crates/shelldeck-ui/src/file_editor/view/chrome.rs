use super::*;

impl FileEditorView {
    pub(super) fn render_find_in_files(&self, cx: &mut Context<Self>) -> impl IntoElement {
        use crate::scale::px;
        let handle = cx.entity().downgrade();
        let root = self.file_browser.root().to_path_buf();

        // Query row (input is captured via keystrokes; shown inline).
        let query_text = if self.fif_query.is_empty() {
            t!("file_editor.find_in_files.placeholder").to_string()
        } else {
            self.fif_query.clone()
        };
        let query_color = if self.fif_query.is_empty() {
            ShellDeckColors::text_muted()
        } else {
            ShellDeckColors::text_primary()
        };
        let query_row = div()
            .flex()
            .items_center()
            .gap(px(8.0))
            .px(px(12.0))
            .py(px(8.0))
            .border_b_1()
            .border_color(ShellDeckColors::border())
            .child(
                div()
                    .text_size(px(11.0))
                    .font_weight(FontWeight::BOLD)
                    .text_color(ShellDeckColors::text_muted())
                    .child(t!("file_editor.find_in_files.title").to_string()),
            )
            .child(
                div()
                    .flex_1()
                    .text_size(px(13.0))
                    .text_color(query_color)
                    .whitespace_nowrap()
                    .overflow_hidden()
                    .child(query_text),
            )
            .child(
                div()
                    .text_size(px(11.0))
                    .text_color(if self.fif_searching {
                        ShellDeckColors::primary()
                    } else {
                        ShellDeckColors::text_muted()
                    })
                    .child(if self.fif_searching {
                        t!("file_editor.find_in_files.searching").to_string()
                    } else {
                        t!(
                            "file_editor.find_in_files.results",
                            count = self.fif_results.len()
                        )
                        .to_string()
                    }),
            );

        // Only render a bounded window of rows — painting thousands of rows
        // every frame is what made typing/scrolling lag.
        const MAX_DISPLAY: usize = 200;

        let mut list = div()
            .id("fif-results")
            .flex()
            .flex_col()
            .min_h(px(0.0))
            .overflow_y_scroll()
            .py(px(2.0));

        if self.fif_results.is_empty() && !self.fif_last_query.is_empty() {
            list = list.child(
                div()
                    .px(px(12.0))
                    .py(px(8.0))
                    .text_size(px(12.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(t!("file_editor.find_in_files.no_matches").to_string()),
            );
        }

        for (i, m) in self.fif_results.iter().enumerate().take(MAX_DISPLAY) {
            let selected = i == self.fif_selected;
            let h = handle.clone();
            let rel = m
                .path
                .strip_prefix(&root)
                .unwrap_or(&m.path)
                .display()
                .to_string();

            let mut row = div()
                .id(ElementId::from(SharedString::from(format!("fif-{i}"))))
                .flex()
                .flex_col()
                .px(px(12.0))
                .py(px(3.0))
                .cursor_pointer()
                .hover(|s| s.bg(ShellDeckColors::hover_bg()));
            if selected {
                row = row.bg(ShellDeckColors::selected_bg());
            }
            row = row
                .child(
                    div()
                        .text_size(px(11.0))
                        .text_color(ShellDeckColors::primary())
                        .whitespace_nowrap()
                        .overflow_hidden()
                        .child(format!("{}:{}", rel, m.line)),
                )
                .child(
                    div()
                        .text_size(px(11.0))
                        .text_color(ShellDeckColors::text_muted())
                        .whitespace_nowrap()
                        .overflow_hidden()
                        .child(m.preview.clone()),
                )
                .on_click(move |_e, _w, cx| {
                    if let Some(v) = h.upgrade() {
                        v.update(cx, |this, cx| this.open_fif_result(i, cx));
                    }
                });
            list = list.child(row);
        }

        if self.fif_results.len() > MAX_DISPLAY {
            list = list.child(
                div()
                    .px(px(12.0))
                    .py(px(6.0))
                    .text_size(px(11.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(format!(
                        "… showing first {} of {} matches — refine your query",
                        MAX_DISPLAY,
                        self.fif_results.len()
                    )),
            );
        }

        // Centered overlay near the top of the editor.
        div()
            .absolute()
            .top(px(40.0))
            .left_0()
            .right_0()
            .flex()
            .justify_center()
            .child(
                div()
                    .w(px(620.0))
                    .max_h(px(460.0))
                    .flex()
                    .flex_col()
                    .bg(ShellDeckColors::bg_surface())
                    .border_1()
                    .border_color(ShellDeckColors::border())
                    .rounded(px(10.0))
                    .shadow_xl()
                    .overflow_hidden()
                    .child(query_row)
                    .child(list),
            )
    }

    pub(super) fn render_context_menu(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let (menu_x, menu_y) = self.context_menu_position;
        let handle = cx.entity().downgrade();

        let mut menu = div()
            .absolute()
            .top(px(menu_y))
            .left(px(menu_x))
            .w(px(200.0))
            .bg(ShellDeckColors::bg_surface())
            .border_1()
            .border_color(ShellDeckColors::border())
            .rounded(px(4.0))
            .shadow_md()
            .py(px(4.0))
            .text_size(px(12.0));

        let menu_items = context_menu_items();
        for (i, item) in menu_items.iter().enumerate() {
            let h = handle.clone();
            let action = item.action;

            let row = div()
                .id(SharedString::from(format!("ctx-menu-{}", i)))
                .flex()
                .items_center()
                .justify_between()
                .w_full()
                .h(px(26.0))
                .px(px(12.0))
                .cursor_pointer()
                .hover(|s| s.bg(ShellDeckColors::hover_bg()))
                .child(
                    div()
                        .text_color(ShellDeckColors::text_primary())
                        .child(item.label.clone()),
                )
                .child(
                    div()
                        .text_color(ShellDeckColors::text_muted())
                        .text_size(px(10.0))
                        .child(item.shortcut.clone()),
                )
                .on_click(move |_event, _window, cx| {
                    if let Some(view) = h.upgrade() {
                        view.update(cx, |this, cx| {
                            this.context_menu_visible = false;
                            this.execute_context_action(action, cx);
                        });
                    }
                });

            menu = menu.child(row);
        }

        menu
    }

    pub(super) fn execute_context_action(
        &mut self,
        action: ContextMenuAction,
        cx: &mut Context<Self>,
    ) {
        match action {
            ContextMenuAction::Undo => {
                if let Some(tab) = self.active_tab_mut() {
                    if let TabContent::Text {
                        buffer,
                        highlighter,
                        ..
                    } = &mut tab.content
                    {
                        buffer.undo();
                        highlighter.parse_full(buffer.rope());
                    }
                }
            }
            ContextMenuAction::Redo => {
                if let Some(tab) = self.active_tab_mut() {
                    if let TabContent::Text {
                        buffer,
                        highlighter,
                        ..
                    } = &mut tab.content
                    {
                        buffer.redo();
                        highlighter.parse_full(buffer.rope());
                    }
                }
            }
            ContextMenuAction::Cut => {
                if let Some(tab) = self.active_tab() {
                    if let Some(text) = tab.buffer().and_then(|b| b.selected_text()) {
                        cx.write_to_clipboard(ClipboardItem::new_string(text));
                    }
                }
                if let Some(tab) = self.active_tab_mut() {
                    if let TabContent::Text {
                        buffer,
                        highlighter,
                        ..
                    } = &mut tab.content
                    {
                        buffer.delete_selection();
                        highlighter.parse_full(buffer.rope());
                    }
                }
            }
            ContextMenuAction::Copy => {
                if let Some(tab) = self.active_tab() {
                    if let Some(text) = tab.buffer().and_then(|b| b.selected_text()) {
                        cx.write_to_clipboard(ClipboardItem::new_string(text));
                    }
                }
            }
            ContextMenuAction::Paste => {
                if let Some(item) = cx.read_from_clipboard() {
                    if let Some(text) = item.text() {
                        if let Some(tab) = self.active_tab_mut() {
                            if let TabContent::Text {
                                buffer,
                                highlighter,
                                ..
                            } = &mut tab.content
                            {
                                buffer.insert_str(&text);
                                highlighter.parse_full(buffer.rope());
                            }
                        }
                    }
                }
            }
            ContextMenuAction::SelectAll => {
                if let Some(tab) = self.active_tab_mut() {
                    if let Some(buffer) = tab.buffer_mut() {
                        buffer.select_all();
                    }
                }
            }
            ContextMenuAction::ToggleComment => {
                if let Some(prefix) = self
                    .active_tab()
                    .and_then(|t| t.language())
                    .and_then(|l| l.comment_prefix())
                    .map(|s| s.to_string())
                {
                    if let Some(tab) = self.active_tab_mut() {
                        if let TabContent::Text {
                            buffer,
                            highlighter,
                            ..
                        } = &mut tab.content
                        {
                            buffer.toggle_line_comment(&prefix);
                            highlighter.parse_full(buffer.rope());
                        }
                    }
                }
            }
        }
        self.ensure_cursor_visible();
        cx.notify();
    }

    pub(super) fn render_status_bar(&self) -> impl IntoElement {
        use crate::scale::px;
        let tab = self.active_tab();
        let is_text = tab.is_some_and(|t| t.is_text());

        let type_name = tab
            .map(|t| t.content_type_name().to_string())
            .unwrap_or_else(|| t!("file_editor.status.plain_text").to_string());

        let mut bar = div()
            .flex()
            .items_center()
            .w_full()
            .h(px(STATUS_BAR_HEIGHT))
            .bg(ShellDeckColors::bg_sidebar())
            .border_t_1()
            .border_color(ShellDeckColors::border())
            .px(px(10.0))
            .gap(px(16.0))
            .text_size(px(11.0))
            .text_color(ShellDeckColors::text_muted());

        if is_text {
            let (line, col) = tab
                .and_then(|t| t.buffer())
                .map(|b| {
                    let (l, c) = b.cursor_line_col();
                    let vc = b.char_col_to_visual_col(l, c);
                    (l + 1, vc + 1)
                })
                .unwrap_or((1, 1));

            let total_lines = tab
                .and_then(|t| t.buffer())
                .map(|b| b.len_lines())
                .unwrap_or(0);

            let tab_info = tab
                .and_then(|t| t.buffer())
                .map(|b| t!("file_editor.status.spaces", count = b.tab_size()).to_string())
                .unwrap_or_default();

            bar = bar
                .child(t!("file_editor.status.line_col", line = line, col = col).to_string())
                .child(div().flex_grow())
                .child(tab_info)
                .child(type_name)
                .child(t!("file_editor.status.lines", count = total_lines).to_string());
        } else {
            bar = bar.child(type_name).child(div().flex_grow());
        }

        bar
    }
}
