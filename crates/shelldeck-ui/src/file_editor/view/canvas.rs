use super::*;

/// State bundle handed from the prepaint pass to `paint_editor`. Groups the
/// 20+ pieces that used to be positional params so a caller can't silently
/// swap two `usize`s and misalign the whole paint. Fields borrow against
/// the prepaint tuple; nothing is cloned per frame.
struct PaintCtx<'a> {
    bounds: Bounds<Pixels>,
    cache: &'a GlyphCache,
    line_texts: &'a [String],
    line_numbers: &'a [usize],
    fold_markers: &'a [u8],
    highlights: &'a [Vec<HighlightSpan>],
    total_lines: usize,
    first_visible: usize,
    cursor_line: usize,
    cursor_col: usize,
    sel_coords: Option<(usize, usize, usize, usize)>,
    gutter_w: f32,
    show_line_numbers: bool,
    cursor_blink_on: bool,
    has_focus: bool,
    search_match_coords: &'a [(usize, usize, usize, usize)],
    search_current_coord: Option<usize>,
    tab_size: usize,
    bracket_match: Option<(usize, usize)>,
    word_highlight_coords: &'a [(usize, usize, usize, usize)],
    extra_cursors: &'a [(usize, usize)],
    h_scroll: usize,
}

impl FileEditorView {
    // -----------------------------------------------------------------------
    // Rendering: Tab bar
    // -----------------------------------------------------------------------

    pub(super) fn render_tab_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        // Chrome scales with the app UI size (rems track the window rem size).
        use crate::scale::px;
        let handle = cx.entity().downgrade();

        let mut tab_bar = div()
            .flex()
            .items_center()
            .w_full()
            .h(px(TAB_BAR_HEIGHT))
            .bg(ShellDeckColors::bg_sidebar())
            .border_b_1()
            .border_color(ShellDeckColors::border())
            .px(px(4.0))
            .gap(px(2.0));

        for (i, tab) in self.tabs.iter().enumerate() {
            let is_active = i == self.active_tab_index;
            let tab_id = tab.id;
            let h1 = handle.clone();
            let h2 = handle.clone();

            let mut tab_el = div()
                .id(SharedString::from(format!("editor-tab-{}", i)))
                .flex()
                .items_center()
                .gap(px(4.0))
                .px(px(10.0))
                .py(px(4.0))
                .rounded(px(4.0))
                .cursor_pointer()
                .text_size(px(12.0));

            if is_active {
                tab_el = tab_el
                    .bg(ShellDeckColors::bg_primary())
                    .text_color(ShellDeckColors::text_primary());
            } else {
                tab_el = tab_el
                    .text_color(ShellDeckColors::text_muted())
                    .hover(|s| s.bg(ShellDeckColors::hover_bg()));
            }

            let idx = i;
            tab_el = tab_el.on_click(move |_event, _window, cx| {
                if let Some(view) = h1.upgrade() {
                    view.update(cx, |this, cx| {
                        this.active_tab_index = idx;
                        cx.notify();
                    });
                }
            });

            let name = tab.display_name();
            tab_el = tab_el.child(name);

            // Close button
            if self.tabs.len() > 1 || tab.is_dirty() {
                let close_btn = div()
                    .id(SharedString::from(format!("close-tab-{}", i)))
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_color(ShellDeckColors::text_muted())
                    .cursor_pointer()
                    .hover(|s| s.text_color(ShellDeckColors::text_primary()))
                    .child(
                        svg()
                            .path("icons/lucide/x.svg")
                            .size(px(10.0))
                            .text_color(ShellDeckColors::text_muted()),
                    )
                    .on_click(move |_event, _window, cx| {
                        if let Some(view) = h2.upgrade() {
                            view.update(cx, |this, cx| {
                                // Find tab by id
                                if let Some(pos) = this.tabs.iter().position(|t| t.id == tab_id) {
                                    this.close_tab(pos, cx);
                                }
                            });
                        }
                    });
                tab_el = tab_el.child(close_btn);
            }

            tab_bar = tab_bar.child(tab_el);
        }

        // Language indicator on the right
        if let Some(tab) = self.active_tab() {
            let spacer = div().flex_grow();
            let lang_label = div()
                .text_size(px(11.0))
                .text_color(ShellDeckColors::text_muted())
                .px(px(8.0))
                .child(tab.content_type_name().to_string());
            tab_bar = tab_bar.child(spacer).child(lang_label);
        }

        tab_bar
    }

    // -----------------------------------------------------------------------
    // Rendering: Search bar
    // -----------------------------------------------------------------------

    pub(super) fn render_search_bar(&self) -> impl IntoElement {
        use crate::scale::px;
        let match_count = self.search_matches.len();
        let current = self
            .search_current_idx
            .map(|i| format!("{}/{}", i + 1, match_count))
            .unwrap_or_else(|| format!("0/{}", match_count));

        let search_focused = !self.search_focus_replace;
        let search_border = if search_focused {
            ShellDeckColors::primary()
        } else {
            ShellDeckColors::border()
        };

        let case_label = if self.search_case_sensitive {
            "[Aa]"
        } else {
            "[aa]"
        };
        let case_color = if self.search_case_sensitive {
            ShellDeckColors::primary()
        } else {
            ShellDeckColors::text_muted()
        };

        div()
            .flex()
            .items_center()
            .w_full()
            .h(px(SEARCH_BAR_HEIGHT))
            .bg(ShellDeckColors::bg_surface())
            .border_b_1()
            .border_color(ShellDeckColors::border())
            .px(px(8.0))
            .gap(px(6.0))
            .text_size(px(12.0))
            .child(
                div()
                    .text_color(ShellDeckColors::text_muted())
                    .child(t!("file_editor.search.find").to_string()),
            )
            .child(
                div()
                    .flex_grow()
                    .px(px(4.0))
                    .py(px(2.0))
                    .rounded(px(3.0))
                    .border_1()
                    .border_color(search_border)
                    .bg(ShellDeckColors::bg_primary())
                    .text_color(ShellDeckColors::text_primary())
                    .child(if self.search_query.is_empty() {
                        t!("file_editor.search.placeholder").to_string()
                    } else {
                        self.search_query.clone()
                    }),
            )
            .child(
                div()
                    .text_color(case_color)
                    .text_size(px(10.0))
                    .child(case_label),
            )
            .child(
                div()
                    .text_color(ShellDeckColors::text_muted())
                    .child(current),
            )
    }

    pub(super) fn render_replace_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        use crate::scale::px;
        let h_next = cx.entity().downgrade();
        let h_all = cx.entity().downgrade();
        let replace_focused = self.search_focus_replace;
        let replace_border = if replace_focused {
            ShellDeckColors::primary()
        } else {
            ShellDeckColors::border()
        };

        div()
            .flex()
            .items_center()
            .w_full()
            .h(px(REPLACE_BAR_HEIGHT))
            .bg(ShellDeckColors::bg_surface())
            .border_b_1()
            .border_color(ShellDeckColors::border())
            .px(px(8.0))
            .gap(px(6.0))
            .text_size(px(12.0))
            .child(
                div()
                    .text_color(ShellDeckColors::text_muted())
                    .child(t!("file_editor.replace.label").to_string()),
            )
            .child(
                div()
                    .flex_grow()
                    .px(px(4.0))
                    .py(px(2.0))
                    .rounded(px(3.0))
                    .border_1()
                    .border_color(replace_border)
                    .bg(ShellDeckColors::bg_primary())
                    .text_color(ShellDeckColors::text_primary())
                    .child(if self.replace_query.is_empty() {
                        t!("file_editor.replace.placeholder").to_string()
                    } else {
                        self.replace_query.clone()
                    }),
            )
            .child(
                div()
                    .id("replace-next-btn")
                    .px(px(6.0))
                    .py(px(2.0))
                    .rounded(px(3.0))
                    .text_size(px(10.0))
                    .text_color(ShellDeckColors::text_muted())
                    .cursor_pointer()
                    .hover(|s| {
                        s.bg(ShellDeckColors::hover_bg())
                            .text_color(ShellDeckColors::text_primary())
                    })
                    .child(t!("file_editor.replace.action").to_string())
                    .on_click(move |_event, _window, cx| {
                        if let Some(view) = h_next.upgrade() {
                            view.update(cx, |this, cx| {
                                this.replace_next(cx);
                            });
                        }
                    }),
            )
            .child(
                div()
                    .id("replace-all-btn")
                    .px(px(6.0))
                    .py(px(2.0))
                    .rounded(px(3.0))
                    .text_size(px(10.0))
                    .text_color(ShellDeckColors::text_muted())
                    .cursor_pointer()
                    .hover(|s| {
                        s.bg(ShellDeckColors::hover_bg())
                            .text_color(ShellDeckColors::text_primary())
                    })
                    .child(t!("file_editor.replace.all").to_string())
                    .on_click(move |_event, _window, cx| {
                        if let Some(view) = h_all.upgrade() {
                            view.update(cx, |this, cx| {
                                this.replace_all(cx);
                            });
                        }
                    }),
            )
    }

    // -----------------------------------------------------------------------
    // Rendering: Go-to-line bar
    // -----------------------------------------------------------------------

    pub(super) fn render_goto_line_bar(&self) -> impl IntoElement {
        use crate::scale::px;
        div()
            .flex()
            .items_center()
            .w_full()
            .h(px(GOTO_LINE_BAR_HEIGHT))
            .bg(ShellDeckColors::bg_surface())
            .border_b_1()
            .border_color(ShellDeckColors::border())
            .px(px(8.0))
            .gap(px(6.0))
            .text_size(px(12.0))
            .child(
                div()
                    .text_color(ShellDeckColors::text_muted())
                    .child(t!("file_editor.goto_line.label").to_string()),
            )
            .child(
                div()
                    .flex_grow()
                    .px(px(4.0))
                    .py(px(2.0))
                    .rounded(px(3.0))
                    .bg(ShellDeckColors::bg_primary())
                    .text_color(ShellDeckColors::text_primary())
                    .child(if self.goto_line_query.is_empty() {
                        t!("file_editor.goto_line.placeholder").to_string()
                    } else {
                        self.goto_line_query.clone()
                    }),
            )
    }

    // -----------------------------------------------------------------------
    // Rendering: Canvas (the main editor surface)
    // -----------------------------------------------------------------------

    pub(super) fn render_editor_canvas(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        self.ensure_glyph_cache(window);
        let cache = match self.glyph_cache.as_ref() {
            Some(c) => c.clone(),
            None => return div().id("editor-canvas-empty"),
        };

        let tab_idx = self.active_tab_index;
        let scroll_lines_per_page = self.scroll_lines_per_page;
        let cursor_blink_on = self.cursor_blink_on;
        let show_line_numbers = self.show_line_numbers;
        let has_focus = self.focus_handle.is_focused(window);

        let tab = match self.tabs.get_mut(tab_idx) {
            Some(t) => t,
            None => return div().id("editor-canvas-empty2"),
        };

        let (buffer, highlighter) = match &mut tab.content {
            TabContent::Text {
                buffer,
                highlighter,
                ..
            } => (buffer, highlighter),
            _ => return div().id("editor-canvas-non-text"),
        };

        // Process pending edits for tree-sitter
        let pending = buffer.take_pending_edits();
        if !pending.is_empty() {
            highlighter.parse_incremental(buffer.rope(), &pending);
        }

        // ---- Folding-aware visible-line model ----
        // `visible` lists the buffer lines currently shown (== 0..n when nothing
        // is folded). Scroll, cursor and clicks all operate in this visual-row
        // space; `to_visual` maps a buffer line to its row (nearest visible
        // at/above it for lines hidden inside a fold).
        let visible: Vec<usize> = buffer.visible_lines();
        let visible_count = visible.len().max(1);
        let to_visual = |bl: usize| -> usize {
            match visible.binary_search(&bl) {
                Ok(i) => i,
                Err(i) => i.saturating_sub(1),
            }
        };

        let first_visible = (tab.scroll_offset as usize).min(visible_count.saturating_sub(1));
        let last_visible = (first_visible + scroll_lines_per_page + 1).min(visible.len());
        let window: Vec<usize> = visible[first_visible..last_visible].to_vec();
        // Horizontal scroll — visual columns. Snapped to int in paint so all
        // col-based math stays clean; the float storage is kept only for
        // trackpad accumulation.
        let h_scroll = tab.h_scroll_offset.max(0.0) as usize;

        // Highlights for the buffer span covering the window, picked per visible line.
        let (hl_start, hl_end) = if window.is_empty() {
            (0usize, 0usize)
        } else {
            (window[0], window[window.len() - 1] + 1)
        };
        let hl_range = highlighter.highlights_for_range(buffer.rope(), hl_start, hl_end);
        let highlights: Vec<Vec<HighlightSpan>> = window
            .iter()
            .map(|&bl| hl_range.get(bl - hl_start).cloned().unwrap_or_default())
            .collect();

        // Line texts + real line numbers + fold markers for each displayed row.
        let mut line_texts: Vec<String> = Vec::with_capacity(window.len());
        let mut line_numbers: Vec<usize> = Vec::with_capacity(window.len());
        let mut fold_markers: Vec<u8> = Vec::with_capacity(window.len());
        for &bl in &window {
            line_texts.push(buffer.line_text(bl));
            line_numbers.push(bl);
            fold_markers.push(if buffer.is_folded_header(bl) {
                2
            } else if buffer.is_foldable(bl) {
                1
            } else {
                0
            });
        }

        let total_lines = buffer.len_lines();
        let tab_size = buffer.tab_size();
        let (cursor_buf_line, cursor_char_col) = buffer.cursor_line_col();
        let cursor_line = to_visual(cursor_buf_line);
        let cursor_col = buffer.char_col_to_visual_col(cursor_buf_line, cursor_char_col);
        let extra_cursor_coords: Vec<(usize, usize)> = buffer
            .extra_cursors()
            .iter()
            .map(|&p| {
                let (l, c) = buffer.char_to_line_col(p);
                (to_visual(l), buffer.char_col_to_visual_col(l, c))
            })
            .collect();
        let selection = buffer.selection().cloned();

        // Gutter layout — VS Code / Zed pattern, left → right:
        //
        //   [ breakpoint col ][ line numbers ][ fold col ][ code ]
        //         1 cell         digits + 1        1 cell
        //
        // TODO(breakpoints): the leftmost column is reserved for a future
        // breakpoint / debug marker gutter. It's rendered empty for now — the
        // reserved cell keeps the layout stable so shipping breakpoints later
        // doesn't shift line numbers under the user's mouse.
        let line_count = total_lines;
        let digits = if line_count == 0 {
            1
        } else {
            (line_count as f64).log10().floor() as usize + 1
        };
        let char_width = cache.cell_width.to_f64() as f32;
        let breakpoint_col_w = char_width; // reserved for future breakpoints
                                           // Fold column is 1.5 cells so the chevron has ~half-cell padding on
                                           // its right and doesn't butt against the gutter/code rail.
        let fold_col_w = char_width * 1.5;
        // Breathing room between the vertical rail and the first code column
        // — matches the padding the chevron gets on its side (see
        // `.agents/spacing.md`). Baked into `gutter_w` so every col-based
        // paint (numbers, cursor, selection, indent guides…) picks it up
        // without individual offsets.
        let code_left_pad = char_width * 0.5;
        let gutter_w = if self.show_line_numbers {
            breakpoint_col_w + (digits as f32 + 1.0) * char_width + fold_col_w + code_left_pad
        } else {
            // Line numbers off: still keep the breakpoint reservation + fold
            // column so future toggles don't reshuffle the layout.
            breakpoint_col_w + fold_col_w + code_left_pad
        };

        // Bracket match (converted to visual-row space)
        let bracket_match: Option<(usize, usize)> = buffer
            .find_matching_bracket()
            .map(|(l, c)| (to_visual(l), c));

        // Selection as (start_row, start_visual_col, end_row, end_visual_col) for canvas
        let sel_coords: Option<(usize, usize, usize, usize)> = selection.as_ref().and_then(|s| {
            let range = s.range();
            if range.is_empty() {
                return None;
            }
            let (sl, sc) = buffer.char_to_line_col(range.start);
            let (el, ec) = buffer.char_to_line_col(range.end);
            let sv = buffer.char_col_to_visual_col(sl, sc);
            let ev = buffer.char_col_to_visual_col(el, ec);
            Some((to_visual(sl), sv, to_visual(el), ev))
        });

        // Word highlighting: find all occurrences of the word under cursor
        let word_highlight_coords: Vec<(usize, usize, usize, usize)> = {
            if let Some(word) = buffer.word_at_cursor() {
                let occurrences = buffer.find_word_occurrences(&word, hl_start, hl_end);
                occurrences
                    .into_iter()
                    .map(|(line, sc, ec)| {
                        let sv = buffer.char_col_to_visual_col(line, sc);
                        let ev = buffer.char_col_to_visual_col(line, ec);
                        let vr = to_visual(line);
                        (vr, sv, vr, ev)
                    })
                    .collect()
            } else {
                Vec::new()
            }
        };

        // Convert search matches from char ranges to (start_line, start_visual_col, end_line, end_visual_col)
        // Only keep matches that overlap the visible range
        // Re-borrow buffer immutably for search match coordinate conversion
        let tab = &self.tabs[tab_idx];
        let buffer = tab.buffer().unwrap();
        let mut search_match_coords: Vec<(usize, usize, usize, usize)> = Vec::new();
        let mut search_current_coord: Option<usize> = None;
        for (mi, m) in self.search_matches.iter().enumerate() {
            let (sl, sc) = buffer.char_to_line_col(m.start);
            let (el, ec) = buffer.char_to_line_col(m.end);
            let svl = to_visual(sl);
            let evl = to_visual(el);
            if evl >= first_visible && svl < last_visible {
                if Some(mi) == self.search_current_idx {
                    search_current_coord = Some(search_match_coords.len());
                }
                let sv = buffer.char_col_to_visual_col(sl, sc);
                let ev = buffer.char_col_to_visual_col(el, ec);
                search_match_coords.push((svl, sv, evl, ev));
            }
        }

        let handle = cx.entity().downgrade();
        let focus = self.focus_handle.clone();
        let origin_arc = self.canvas_origin.clone();
        let height_arc = self.canvas_height.clone();

        // Mouse handlers
        let h_down = handle.clone();
        let h_right = handle.clone();
        let h_move = handle.clone();
        let h_up = handle.clone();
        let h_wheel = handle.clone();
        let focus_down = focus.clone();

        div()
            .flex_grow()
            .w_full()
            .min_h(px(0.0))
            .overflow_hidden()
            .id("editor-canvas-container")
            .on_mouse_down(
                MouseButton::Left,
                move |event: &MouseDownEvent, window, cx| {
                    focus_down.focus(window);
                    if let Some(view) = h_down.upgrade() {
                        view.update(cx, |this, cx| {
                            this.handle_mouse_down(event, window, cx);
                        });
                    }
                },
            )
            .on_mouse_down(
                MouseButton::Right,
                move |event: &MouseDownEvent, _window, cx| {
                    if let Some(view) = h_right.upgrade() {
                        view.update(cx, |this, cx| {
                            this.handle_right_click(event, cx);
                        });
                    }
                },
            )
            .on_mouse_move(move |event: &MouseMoveEvent, _window, cx| {
                if let Some(view) = h_move.upgrade() {
                    view.update(cx, |this, cx| {
                        this.handle_mouse_move(event, cx);
                    });
                }
            })
            .on_mouse_up(
                MouseButton::Left,
                move |_event: &MouseUpEvent, _window, cx| {
                    if let Some(view) = h_up.upgrade() {
                        view.update(cx, |this, cx| {
                            this.mouse_selecting = false;
                            this.mouse_click_origin = None;
                            this.scrollbar_dragging = false;
                            this.minimap_dragging = false;
                            cx.notify();
                        });
                    }
                },
            )
            .on_scroll_wheel(move |event: &ScrollWheelEvent, _window, cx| {
                if let Some(view) = h_wheel.upgrade() {
                    view.update(cx, |this, cx| {
                        this.handle_scroll(event, cx);
                    });
                }
            })
            .child(
                canvas(
                    move |bounds, _window, _cx| {
                        // Store canvas origin and height for mouse handlers
                        let ox = bounds.origin.x.to_f64() as f32;
                        let oy = bounds.origin.y.to_f64() as f32;
                        let packed = ((ox.to_bits() as u64) << 32) | (oy.to_bits() as u64);
                        origin_arc.store(packed, std::sync::atomic::Ordering::Relaxed);
                        let h = bounds.size.height.to_f64() as f32;
                        height_arc.store(h.to_bits(), std::sync::atomic::Ordering::Relaxed);
                        (
                            cache,
                            line_texts,
                            line_numbers,
                            fold_markers,
                            highlights,
                            total_lines,
                            first_visible,
                            cursor_line,
                            cursor_col,
                            sel_coords,
                            gutter_w,
                            show_line_numbers,
                            cursor_blink_on,
                            has_focus,
                            search_match_coords,
                            search_current_coord,
                            tab_size,
                            bracket_match,
                            word_highlight_coords,
                            extra_cursor_coords,
                            h_scroll,
                        )
                    },
                    move |bounds,
                          (
                        cache,
                        line_texts,
                        line_numbers,
                        fold_markers,
                        highlights,
                        total_lines,
                        first_visible,
                        cursor_line,
                        cursor_col,
                        sel_coords,
                        gutter_w,
                        show_line_numbers,
                        cursor_blink_on,
                        has_focus,
                        search_match_coords,
                        search_current_coord,
                        tab_size,
                        bracket_match,
                        word_highlight_coords,
                        extra_cursor_coords,
                        h_scroll,
                    ),
                          window,
                          cx| {
                        Self::paint_editor(
                            PaintCtx {
                                bounds,
                                cache: &cache,
                                line_texts: &line_texts,
                                line_numbers: &line_numbers,
                                fold_markers: &fold_markers,
                                highlights: &highlights,
                                total_lines,
                                first_visible,
                                cursor_line,
                                cursor_col,
                                sel_coords,
                                gutter_w,
                                show_line_numbers,
                                cursor_blink_on,
                                has_focus,
                                search_match_coords: &search_match_coords,
                                search_current_coord,
                                tab_size,
                                bracket_match,
                                word_highlight_coords: &word_highlight_coords,
                                extra_cursors: &extra_cursor_coords,
                                h_scroll,
                            },
                            window,
                            cx,
                        );
                    },
                )
                .size_full(),
            )
    }

    // -----------------------------------------------------------------------
    // Paint: the actual pixel-level rendering
    // -----------------------------------------------------------------------

    fn paint_editor(ctx: PaintCtx<'_>, window: &mut Window, cx: &mut App) {
        let PaintCtx {
            bounds,
            cache,
            line_texts,
            line_numbers,
            fold_markers,
            highlights,
            total_lines,
            first_visible,
            cursor_line,
            cursor_col,
            sel_coords,
            gutter_w,
            show_line_numbers,
            cursor_blink_on,
            has_focus,
            search_match_coords,
            search_current_coord,
            tab_size,
            bracket_match,
            word_highlight_coords,
            extra_cursors,
            h_scroll,
        } = ctx;
        let cell_w = cache.cell_width;
        // Text starts at `bounds.origin.x + gutter_px` for column 0 in a
        // non-scrolled view; when `h_scroll > 0` we back-off by that many
        // cells so painted text (and the col-based math for cursor /
        // selection / highlights) all shift left together.
        let h_scroll_px = cell_w * h_scroll as f32;
        let cell_h = cache.cell_height;
        let fs = cache.font_size;
        let gutter_px = px(gutter_w);

        let sel_color = hsla(0.58, 0.6, 0.5, 0.35);
        let search_color = hsla(0.12, 0.8, 0.5, 0.35);
        let search_current_color = hsla(0.12, 0.9, 0.55, 0.55);
        // Theme-following highlight tints (derived from the active palette so
        // they track theme changes instead of being a fixed blue/grey).
        let word_highlight_color = ShellDeckColors::primary().opacity(0.18);

        // Rail x-position — cached so the gutter background can stop right
        // before it, leaving the `code_left_pad` slot to inherit the code
        // panel's own background (see `.agents/spacing.md`).
        let rail_x = bounds.origin.x + gutter_px - cache.cell_width * 0.5 - px(1.0);
        let gutter_bg_w = rail_x - bounds.origin.x;

        // Paint gutter background — only up to the rail. Everything right of
        // the rail (rail itself + code_left_pad + code) keeps the code
        // panel's `bg_primary`, so the padding reads as air, not as an
        // orphan strip of gutter tint.
        window.paint_quad(fill(
            Bounds::new(bounds.origin, size(gutter_bg_w, bounds.size.height)),
            ShellDeckColors::line_number_bg(),
        ));

        // Thin vertical rail at the gutter / code boundary — mirrors VS Code
        // and Zed. Uses the theme border tint so it stays subtle on every
        // palette and never fights with syntax coloring.
        window.paint_quad(fill(
            Bounds::new(
                point(rail_x, bounds.origin.y),
                size(px(1.0), bounds.size.height),
            ),
            ShellDeckColors::border(),
        ));

        // Compute digit count for line numbers
        let digits = if total_lines == 0 {
            1
        } else {
            (total_lines as f64).log10().floor() as usize + 1
        };

        let indent_guide_color = ShellDeckColors::text_muted().opacity(0.25);
        let indent_guide_active_color = ShellDeckColors::text_muted().opacity(0.5);

        // Indent guides: compute max indent depth per visible line, then draw vertical lines
        {
            // For each line, count leading spaces to determine indent level
            let tab_w = cell_w * tab_size as f32;
            for (ri, line_text) in line_texts.iter().enumerate() {
                let y = bounds.origin.y + cell_h * ri as f32;
                let abs_line = first_visible + ri;

                // Count leading whitespace columns
                let mut leading_vcols = 0usize;
                for ch in line_text.chars() {
                    if ch == ' ' {
                        leading_vcols += 1;
                    } else if ch == '\t' {
                        leading_vcols += tab_size - (leading_vcols % tab_size);
                    } else {
                        break;
                    }
                }

                // For blank lines, look at surrounding lines to infer indent depth
                let effective_indent = if line_text.trim().is_empty() {
                    // Use max of prev/next non-blank line indent
                    let prev_indent = if ri > 0 {
                        count_leading_vcols(&line_texts[ri - 1], tab_size)
                    } else {
                        0
                    };
                    let next_indent = if ri + 1 < line_texts.len() {
                        count_leading_vcols(&line_texts[ri + 1], tab_size)
                    } else {
                        0
                    };
                    prev_indent.min(next_indent)
                } else {
                    leading_vcols
                };

                let indent_levels = effective_indent / tab_size;

                // Determine which indent level the cursor is in (for active highlight)
                let cursor_indent_level = if abs_line == cursor_line {
                    Some(cursor_col / tab_size)
                } else {
                    None
                };

                for level in 1..=indent_levels {
                    let guide_x =
                        bounds.origin.x + gutter_px - h_scroll_px + tab_w * level as f32 - tab_w;
                    let color = if cursor_indent_level == Some(level.saturating_sub(1)) {
                        indent_guide_active_color
                    } else {
                        indent_guide_color
                    };
                    window.paint_quad(fill(
                        Bounds::new(point(guide_x, y), size(px(1.0), cell_h)),
                        color,
                    ));
                }
            }
        }

        // Pass 1: Paint line backgrounds, line numbers, search highlights, selection
        for (ri, line_text) in line_texts.iter().enumerate() {
            let abs_line = first_visible + ri;
            let y = bounds.origin.y + cell_h * ri as f32;

            // Current line highlight
            if abs_line == cursor_line {
                window.paint_quad(fill(
                    Bounds::new(
                        point(bounds.origin.x + gutter_px, y),
                        size(bounds.size.width - gutter_px, cell_h),
                    ),
                    ShellDeckColors::cursor_line_bg(),
                ));
            }

            // Word occurrence highlights (behind text, behind search)
            for &(wh_sl, wh_sc, wh_el, wh_ec) in word_highlight_coords {
                if abs_line < wh_sl || abs_line > wh_el {
                    continue;
                }
                let line_visual_len = visual_line_width(line_text, tab_size);
                let sc = if abs_line == wh_sl { wh_sc } else { 0 };
                let ec = if abs_line == wh_el {
                    wh_ec
                } else {
                    line_visual_len
                };
                if sc < ec {
                    let sx = bounds.origin.x + gutter_px - h_scroll_px + cell_w * sc as f32;
                    let sw = cell_w * (ec - sc) as f32;
                    window.paint_quad(fill(
                        Bounds::new(point(sx, y), size(sw, cell_h)),
                        word_highlight_color,
                    ));
                }
            }

            // Search match highlights (behind text)
            for (mi, &(sm_sl, sm_sc, sm_el, sm_ec)) in search_match_coords.iter().enumerate() {
                if abs_line < sm_sl || abs_line > sm_el {
                    continue;
                }
                let color = if Some(mi) == search_current_coord {
                    search_current_color
                } else {
                    search_color
                };
                let line_visual_len = visual_line_width(line_text, tab_size);
                let sc = if abs_line == sm_sl { sm_sc } else { 0 };
                let ec = if abs_line == sm_el {
                    sm_ec
                } else {
                    line_visual_len
                };
                if sc < ec {
                    let sx = bounds.origin.x + gutter_px - h_scroll_px + cell_w * sc as f32;
                    let sw = cell_w * (ec - sc) as f32;
                    window.paint_quad(fill(Bounds::new(point(sx, y), size(sw, cell_h)), color));
                }
            }

            // Selection overlay (behind text)
            if let Some((sel_start_line, sel_start_col, sel_end_line, sel_end_col)) = sel_coords {
                if abs_line >= sel_start_line && abs_line <= sel_end_line {
                    let line_visual_len = visual_line_width(line_text, tab_size);
                    let start_col = if abs_line == sel_start_line {
                        sel_start_col
                    } else {
                        0
                    };
                    let end_col = if abs_line == sel_end_line {
                        sel_end_col
                    } else {
                        line_visual_len + 1
                    };
                    if start_col < end_col {
                        let sel_x =
                            bounds.origin.x + gutter_px - h_scroll_px + cell_w * start_col as f32;
                        let sel_w = cell_w * (end_col - start_col) as f32;
                        window.paint_quad(fill(
                            Bounds::new(point(sel_x, y), size(sel_w, cell_h)),
                            sel_color,
                        ));
                    }
                }
            }

            // Line number, right-aligned inside its column. The number column
            // starts right after the reserved breakpoint cell.
            if show_line_numbers {
                let real_line = line_numbers.get(ri).copied().unwrap_or(abs_line);
                let line_num = format!("{:>width$}", real_line + 1, width = digits);
                let num_color = if abs_line == cursor_line {
                    ShellDeckColors::text_primary()
                } else {
                    ShellDeckColors::line_number_fg()
                };
                let num_str: SharedString = line_num.into();
                let num_len = num_str.len();
                let shaped_num = window.text_system().shape_line(
                    num_str,
                    fs,
                    &[TextRun {
                        len: num_len,
                        font: cache.base_font.clone(),
                        color: num_color,
                        background_color: None,
                        underline: None,
                        strikethrough: None,
                    }],
                    None,
                );
                // Gutter columns are all `cell_w` wide:
                // [breakpoint(1)][number(digits)][padding(1)][fold(1)]
                // Number column starts at breakpoint (1 cell) + right-align
                // inside `digits` cells (numbers are already zero-padded via
                // the `{:>width$}` format spec above).
                let num_x = bounds.origin.x + cell_w;
                let _ = shaped_num.paint(point(num_x, y), cell_h, window, cx);
            }

            // Fold chevron sits in its own column — right before the
            // gutter/code rail. VS Code / Zed layout: number → chevron → code.
            // `⌄` (u2304) / `›` (u203A) match the sharp chevron look the
            // maintainer targets. Column is 1.5 cells wide with the chevron
            // in the LEFT half so it doesn't butt against the vertical rail
            // (see `.agents/spacing.md`).
            if let Some(&fm) = fold_markers.get(ri) {
                if fm != 0 {
                    let marker = if fm == 2 { "›" } else { "⌄" };
                    let ms: SharedString = marker.into();
                    let shaped_m = window.text_system().shape_line(
                        ms,
                        fs,
                        &[TextRun {
                            len: marker.len(),
                            font: cache.base_font.clone(),
                            color: ShellDeckColors::text_muted(),
                            background_color: None,
                            underline: None,
                            strikethrough: None,
                        }],
                        None,
                    );
                    // Fold column is 1.5 cells + `code_left_pad` after the
                    // rail; chevron sits in the LEFT cell of the fold column
                    // so it has ~half-cell of air before the rail.
                    let mx = bounds.origin.x + px(gutter_w) - cell_w * 2.0;
                    let _ = shaped_m.paint(point(mx, y), cell_h, window, cx);
                }
            }
        }

        // Pass 2: Paint text characters on top of backgrounds
        for (ri, line_text) in line_texts.iter().enumerate() {
            let y = bounds.origin.y + cell_h * ri as f32;
            let text_x = bounds.origin.x + gutter_px - h_scroll_px;
            let line_highlights = highlights.get(ri);

            let mut col = 0usize;
            for (byte_idx, ch) in line_text.char_indices() {
                let x = text_x + cell_w * col as f32;
                let char_byte_end = byte_idx + ch.len_utf8();

                let (fg_color, bold, italic) = if let Some(spans) = line_highlights {
                    Self::color_for_byte_pos(spans, byte_idx, char_byte_end)
                } else {
                    (ShellDeckColors::text_primary(), false, false)
                };

                if ch != ' ' && ch != '\t' {
                    // Always shape through the text system — the paint_glyph
                    // fast-path silently drops bold/italic runs on some GPUI
                    // builds (same rule the terminal follows: see
                    // AGENTS.md “Critical Rules”).
                    let f = match (bold, italic) {
                        (true, true) => cache.base_font.clone().bold().italic(),
                        (true, false) => cache.base_font.clone().bold(),
                        (false, true) => cache.base_font.clone().italic(),
                        _ => cache.base_font.clone(),
                    };
                    let s: SharedString = ch.to_string().into();
                    let shaped = window.text_system().shape_line(
                        s,
                        fs,
                        &[TextRun {
                            len: ch.len_utf8(),
                            font: f,
                            color: fg_color,
                            background_color: None,
                            underline: None,
                            strikethrough: None,
                        }],
                        None,
                    );
                    let _ = shaped.paint(point(x, y), cell_h, window, cx);
                }

                if ch == '\t' {
                    col += tab_size - (col % tab_size);
                } else {
                    col += 1;
                }
            }
        }

        // Paint cursor
        if has_focus && cursor_blink_on {
            let cursor_visible_line = cursor_line.saturating_sub(first_visible);
            if cursor_line >= first_visible && cursor_line < first_visible + line_texts.len() {
                let cursor_x =
                    bounds.origin.x + gutter_px - h_scroll_px + cell_w * cursor_col as f32;
                let cursor_y = bounds.origin.y + cell_h * cursor_visible_line as f32;
                window.paint_quad(fill(
                    Bounds::new(point(cursor_x, cursor_y), size(px(2.0), cell_h)),
                    ShellDeckColors::primary(),
                ));
            }
        }

        // Paint extra cursors (multi-cursor) — always shown while focused so
        // they're visible regardless of the blink phase.
        if has_focus {
            for &(el, ec) in extra_cursors {
                if el >= first_visible && el < first_visible + line_texts.len() {
                    let vis = el - first_visible;
                    let ex = bounds.origin.x + gutter_px - h_scroll_px + cell_w * ec as f32;
                    let ey = bounds.origin.y + cell_h * vis as f32;
                    window.paint_quad(fill(
                        Bounds::new(point(ex, ey), size(px(2.0), cell_h)),
                        ShellDeckColors::primary(),
                    ));
                }
            }
        }

        // Paint matching bracket highlight
        if let Some((match_line, match_vcol)) = bracket_match {
            if match_line >= first_visible && match_line < first_visible + line_texts.len() {
                let vis_row = match_line - first_visible;
                let bx = bounds.origin.x + gutter_px - h_scroll_px + cell_w * match_vcol as f32;
                let by = bounds.origin.y + cell_h * vis_row as f32;
                let bracket_bg = ShellDeckColors::primary().opacity(0.3);
                window.paint_quad(fill(
                    Bounds::new(point(bx, by), size(cell_w, cell_h)),
                    bracket_bg,
                ));
            }
        }

        // Paint scrollbar
        if total_lines > 0 {
            let scrollbar_width = px(SCROLLBAR_WIDTH);
            let scrollbar_x = bounds.origin.x + bounds.size.width - scrollbar_width;
            let viewport_lines = line_texts.len().max(1) as f32;
            let thumb_height = (viewport_lines / total_lines as f32) * bounds.size.height;
            let thumb_height = thumb_height.max(px(20.0));
            let thumb_y =
                bounds.origin.y + (first_visible as f32 / total_lines as f32) * bounds.size.height;

            // Track
            window.paint_quad(fill(
                Bounds::new(
                    point(scrollbar_x, bounds.origin.y),
                    size(scrollbar_width, bounds.size.height),
                ),
                hsla(0.0, 0.0, 0.0, 0.1),
            ));

            // Thumb
            window.paint_quad(fill(
                Bounds::new(
                    point(scrollbar_x, thumb_y),
                    size(scrollbar_width, thumb_height),
                ),
                hsla(0.0, 0.0, 0.5, 0.3),
            ));
        }
    }

    pub(super) fn color_for_byte_pos(
        spans: &[HighlightSpan],
        byte_start: usize,
        byte_end: usize,
    ) -> (Hsla, bool, bool) {
        // Find the most specific (last) span that contains this byte position
        for span in spans.iter().rev() {
            if span.range.start <= byte_start && span.range.end >= byte_end {
                return (span.color, span.bold, span.italic);
            }
        }
        (ShellDeckColors::text_primary(), false, false)
    }

    // -----------------------------------------------------------------------
    // Mouse handlers
    // -----------------------------------------------------------------------

    /// Get the canvas origin (x, y) in window coordinates, set during paint.
    pub(super) fn canvas_origin_xy(&self) -> (f32, f32) {
        let packed = self
            .canvas_origin
            .load(std::sync::atomic::Ordering::Relaxed);
        let ox = f32::from_bits((packed >> 32) as u32);
        let oy = f32::from_bits(packed as u32);
        (ox, oy)
    }

    pub(super) fn header_height(&self) -> f32 {
        TAB_BAR_HEIGHT
            + if self.search_visible {
                SEARCH_BAR_HEIGHT
            } else {
                0.0
            }
            + if self.replace_visible {
                REPLACE_BAR_HEIGHT
            } else {
                0.0
            }
            + if self.goto_line_visible {
                GOTO_LINE_BAR_HEIGHT
            } else {
                0.0
            }
            + if self.pending_close_tab.is_some() {
                32.0
            } else {
                0.0
            }
    }

    pub(super) fn handle_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Dismiss context menu on any click
        if self.context_menu_visible {
            self.context_menu_visible = false;
            cx.notify();
            return;
        }

        // Non-text tabs: no text interaction
        if !self.active_tab().is_some_and(|t| t.is_text()) {
            return;
        }

        self.reset_cursor_blink(cx);

        let cache = match self.glyph_cache.as_ref() {
            Some(c) => c.clone(),
            None => return,
        };

        let tab_idx = self.active_tab_index;
        if tab_idx >= self.tabs.len() {
            return;
        }

        let cell_w = cache.cell_width.to_f64() as f32;
        let cell_h = cache.cell_height.to_f64() as f32;

        let total_lines = self.tabs[tab_idx].buffer().unwrap().len_lines();
        let gutter_w = self.compute_gutter_w(cell_w, total_lines);

        let pos = event.position;
        let (canvas_ox, canvas_oy) = self.canvas_origin_xy();

        // Position relative to canvas origin
        let rel_x = pos.x.to_f64() as f32 - canvas_ox;
        let rel_y = pos.y.to_f64() as f32 - canvas_oy;

        if rel_y < 0.0 || rel_x < 0.0 {
            return;
        }

        // Check if click is in scrollbar area
        // Canvas fills from canvas_ox to the right edge of the viewport
        let canvas_w = _window.viewport_size().width.to_f64() as f32 - canvas_ox;
        if rel_x >= canvas_w - SCROLLBAR_WIDTH {
            // Scrollbar click
            let canvas_h = f32::from_bits(
                self.canvas_height
                    .load(std::sync::atomic::Ordering::Relaxed),
            );
            if canvas_h > 0.0 && total_lines > 0 {
                let ratio = rel_y / canvas_h;
                if let Some(tab) = self.tabs.get_mut(tab_idx) {
                    tab.scroll_offset = ratio * total_lines as f32;
                }
                self.clamp_scroll();
                self.scrollbar_dragging = true;
                cx.notify();
            }
            return;
        }

        let row = (rel_y / cell_h) as usize;
        let scroll = self.tabs[tab_idx].scroll_offset as usize;
        // Map the clicked visual row to a buffer line through the fold-aware
        // visible-line list (identity when nothing is folded).
        let visible = self.tabs[tab_idx].buffer().unwrap().visible_lines();
        let vrow = (row + scroll).min(visible.len().saturating_sub(1));
        let abs_line = visible.get(vrow).copied().unwrap_or(0);

        let text_x = rel_x - gutter_w;
        if text_x < 0.0 {
            // Gutter click: toggle a fold when the row is foldable/folded.
            if let Some(buf) = self.tabs.get_mut(tab_idx).and_then(|t| t.buffer_mut()) {
                if buf.is_foldable(abs_line) || buf.is_folded_header(abs_line) {
                    buf.toggle_fold_at(abs_line);
                    self.clamp_scroll();
                    cx.notify();
                }
            }
            return;
        }

        let h_scroll = self.tabs[tab_idx].h_scroll_offset.max(0.0) as usize;
        // `text_x` is the visual x inside the code area — the paint offsets
        // it left by `h_scroll * cell_w`, so we add the same offset back to
        // land on the correct character after a sideways scroll.
        let visual_col = (text_x / cell_w) as usize + h_scroll;
        let char_col = self.tabs[tab_idx]
            .buffer()
            .unwrap()
            .visual_col_to_char_col(abs_line, visual_col);

        let extend = event.modifiers.shift;

        if event.click_count >= 3 {
            // Triple-click: select entire line
            self.tabs[tab_idx]
                .buffer_mut()
                .unwrap()
                .set_cursor_from_position(abs_line, 0, false);
            self.tabs[tab_idx].buffer_mut().unwrap().select_line();
        } else if event.click_count == 2 {
            self.tabs[tab_idx]
                .buffer_mut()
                .unwrap()
                .set_cursor_from_position(abs_line, char_col, false);
            self.tabs[tab_idx]
                .buffer_mut()
                .unwrap()
                .select_word_at_cursor();
        } else {
            self.tabs[tab_idx]
                .buffer_mut()
                .unwrap()
                .set_cursor_from_position(abs_line, char_col, extend);
            self.mouse_selecting = true;
            self.mouse_click_origin = Some((pos.x.to_f64() as f32, pos.y.to_f64() as f32));
        }

        self.ensure_cursor_visible();
        self.reset_cursor_blink(cx);
        cx.notify();
    }

    pub(super) fn handle_right_click(&mut self, event: &MouseDownEvent, cx: &mut Context<Self>) {
        // No context menu for non-text tabs
        if !self.active_tab().is_some_and(|t| t.is_text()) {
            return;
        }
        let x = event.position.x.to_f64() as f32;
        let y = event.position.y.to_f64() as f32;
        self.context_menu_position = (x, y);
        self.context_menu_visible = true;
        cx.notify();
    }

    pub(super) fn handle_mouse_move(&mut self, event: &MouseMoveEvent, cx: &mut Context<Self>) {
        // Handle scrollbar dragging
        if self.scrollbar_dragging {
            let tab_idx = self.active_tab_index;
            if tab_idx < self.tabs.len() {
                let (_, canvas_oy) = self.canvas_origin_xy();
                let rel_y = (event.position.y.to_f64() as f32 - canvas_oy).max(0.0);
                let total_lines = self.tabs[tab_idx].buffer().map_or(0, |b| b.len_lines());
                // Use approximate viewport height
                let cell_h = self
                    .glyph_cache
                    .as_ref()
                    .map(|c| c.cell_height.to_f64() as f32)
                    .unwrap_or(20.0);
                let viewport_h = (self.scroll_lines_per_page as f32 * cell_h).max(1.0);
                if total_lines > 0 {
                    let ratio = rel_y / viewport_h;
                    self.tabs[tab_idx].scroll_offset = ratio * total_lines as f32;
                    self.clamp_scroll();
                }
            }
            cx.notify();
            return;
        }

        if !self.mouse_selecting {
            return;
        }

        // Dead zone: ignore small movements near the click origin to prevent
        // accidental cursor displacement during a click
        if let Some((ox, oy)) = self.mouse_click_origin {
            let mx = event.position.x.to_f64() as f32;
            let my = event.position.y.to_f64() as f32;
            let dist_sq = (mx - ox) * (mx - ox) + (my - oy) * (my - oy);
            if dist_sq < 9.0 {
                // Less than 3px movement — ignore
                return;
            }
            // Beyond dead zone — clear origin so we don't check again
            self.mouse_click_origin = None;
        }

        let cache = match self.glyph_cache.as_ref() {
            Some(c) => c.clone(),
            None => return,
        };

        let tab_idx = self.active_tab_index;
        if tab_idx >= self.tabs.len() {
            return;
        }

        let cell_w = cache.cell_width.to_f64() as f32;
        let cell_h = cache.cell_height.to_f64() as f32;

        let total_lines = self.tabs[tab_idx].buffer().map_or(0, |b| b.len_lines());
        let gutter_w = self.compute_gutter_w(cell_w, total_lines);
        let h_scroll = self.tabs[tab_idx].h_scroll_offset.max(0.0) as usize;

        let (canvas_ox, canvas_oy) = self.canvas_origin_xy();
        let rel_x = (event.position.x.to_f64() as f32 - canvas_ox - gutter_w).max(0.0);
        let rel_y = (event.position.y.to_f64() as f32 - canvas_oy).max(0.0);

        // Add h_scroll back so drag-selection lands on the right character
        // once the code area has been scrolled sideways.
        let visual_col = (rel_x / cell_w) as usize + h_scroll;
        let row = (rel_y / cell_h) as usize;
        let scroll = self.tabs[tab_idx].scroll_offset as usize;
        let visible = self.tabs[tab_idx].buffer().unwrap().visible_lines();
        let vrow = (row + scroll).min(visible.len().saturating_sub(1));
        let abs_line = visible.get(vrow).copied().unwrap_or(0);
        let char_col = self.tabs[tab_idx]
            .buffer()
            .unwrap()
            .visual_col_to_char_col(abs_line, visual_col);

        self.tabs[tab_idx]
            .buffer_mut()
            .unwrap()
            .set_cursor_from_position(abs_line, char_col, true);

        cx.notify();
    }

    pub(super) fn handle_scroll(&mut self, event: &ScrollWheelEvent, cx: &mut Context<Self>) {
        let (cell_w, cell_h) = self
            .glyph_cache
            .as_ref()
            .map(|c| (c.cell_width.to_f64() as f32, c.cell_height.to_f64() as f32))
            .unwrap_or((8.0, 20.0));
        let (dy, dx) = match event.delta {
            ScrollDelta::Lines(d) => (-d.y * 3.0, -d.x * 3.0),
            ScrollDelta::Pixels(d) => (
                -d.y.to_f64() as f32 / cell_h.max(1.0),
                -d.x.to_f64() as f32 / cell_w.max(1.0),
            ),
        };

        if let Some(tab) = self.active_tab_mut() {
            tab.scroll_offset += dy;
            // Horizontal: trackpad horizontal / shift+wheel_y-mapped-to-x.
            // Clamped to >= 0; the paint layer skips out-of-view chars, so a
            // large positive offset just parks the view past the last char.
            tab.h_scroll_offset = (tab.h_scroll_offset + dx).max(0.0);
        }
        self.clamp_scroll();
        cx.notify();
    }

    pub fn is_file_browser_resizing(&self) -> bool {
        self.file_browser_resizing
    }

    // -----------------------------------------------------------------------
    // Minimap
    // -----------------------------------------------------------------------

    pub(super) fn render_minimap(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let tab_idx = self.active_tab_index;
        let tab = match self.tabs.get(tab_idx) {
            Some(t) => t,
            None => return div().id("minimap-empty"),
        };

        let buffer = match tab.buffer() {
            Some(b) => b,
            None => return div().id("minimap-non-text"),
        };

        let total_lines = buffer.len_lines();
        let scroll_offset = tab.scroll_offset;
        let scroll_lines_per_page = self.scroll_lines_per_page;

        // Collect line visual lengths for the minimap (no full-file syntax highlighting
        // to avoid invalidating the editor's highlight cache).
        let minimap_max_lines = total_lines.min(5000);
        let tab_size = buffer.tab_size();

        let mut line_lengths: Vec<usize> = Vec::with_capacity(minimap_max_lines);
        for line_idx in 0..minimap_max_lines {
            let line_text = buffer.line_text(line_idx);
            let visual_len = visual_line_width(&line_text, tab_size);
            line_lengths.push(visual_len);
        }

        let handle = cx.entity().downgrade();
        let h_down = handle.clone();
        let h_move = handle.clone();
        let h_up = handle.clone();
        let minimap_oy_arc = self.minimap_origin_y.clone();

        div()
            .id("minimap-container")
            .w(px(MINIMAP_WIDTH))
            .h_full()
            .flex_shrink_0()
            .bg(ShellDeckColors::bg_primary())
            .border_l_1()
            .border_color(ShellDeckColors::border())
            .on_mouse_down(
                MouseButton::Left,
                move |event: &MouseDownEvent, _window, cx| {
                    if let Some(view) = h_down.upgrade() {
                        view.update(cx, |this, cx| {
                            this.minimap_dragging = true;
                            let y = event.position.y.to_f64() as f32;
                            this.handle_minimap_click(y, cx);
                        });
                    }
                },
            )
            .on_mouse_move(move |event: &MouseMoveEvent, _window, cx| {
                if let Some(view) = h_move.upgrade() {
                    view.update(cx, |this, cx| {
                        if this.minimap_dragging {
                            let y = event.position.y.to_f64() as f32;
                            this.handle_minimap_click(y, cx);
                        }
                    });
                }
            })
            .on_mouse_up(MouseButton::Left, move |_event, _window, cx| {
                if let Some(view) = h_up.upgrade() {
                    view.update(cx, |this, cx| {
                        this.minimap_dragging = false;
                        cx.notify();
                    });
                }
            })
            .child(
                canvas(
                    move |bounds, _window, _cx| {
                        // Store the minimap canvas origin Y for accurate click handling
                        let oy = bounds.origin.y.to_f64() as f32;
                        minimap_oy_arc.store(oy.to_bits(), std::sync::atomic::Ordering::Relaxed);
                        (
                            line_lengths,
                            total_lines,
                            scroll_offset,
                            scroll_lines_per_page,
                        )
                    },
                    move |bounds,
                          (line_lengths, total_lines, scroll_offset, scroll_lines_per_page),
                          window,
                          _cx| {
                        Self::paint_minimap(
                            bounds,
                            &line_lengths,
                            total_lines,
                            scroll_offset,
                            scroll_lines_per_page,
                            window,
                        );
                    },
                )
                .size_full(),
            )
    }

    pub(super) fn handle_minimap_click(&mut self, window_y: f32, cx: &mut Context<Self>) {
        let tab_idx = self.active_tab_index;
        let total_lines = self
            .tabs
            .get(tab_idx)
            .and_then(|t| t.buffer())
            .map_or(0, |b| b.len_lines());
        if total_lines == 0 {
            return;
        }

        // Use stored minimap canvas origin for accurate positioning
        let origin_y = f32::from_bits(
            self.minimap_origin_y
                .load(std::sync::atomic::Ordering::Relaxed),
        );
        let rel_y = (window_y - origin_y).max(0.0);

        // Calculate which line in the minimap was clicked
        let clicked_line = (rel_y / MINIMAP_LINE_HEIGHT) as usize;
        let half_page = self.scroll_lines_per_page / 2;

        if let Some(tab) = self.tabs.get_mut(tab_idx) {
            tab.scroll_offset = clicked_line.saturating_sub(half_page) as f32;
        }
        self.clamp_scroll();
        cx.notify();
    }

    pub(super) fn paint_minimap(
        bounds: Bounds<Pixels>,
        line_lengths: &[usize],
        total_lines: usize,
        scroll_offset: f32,
        scroll_lines_per_page: usize,
        window: &mut Window,
    ) {
        let line_h = px(MINIMAP_LINE_HEIGHT);
        let char_w = px(MINIMAP_CHAR_WIDTH);
        let code_color = hsla(0.0, 0.0, 0.6, 0.35);
        let viewport_bg = hsla(0.58, 0.3, 0.5, 0.15);
        let viewport_border = hsla(0.58, 0.4, 0.5, 0.3);

        // Paint viewport indicator
        let vp_y = bounds.origin.y + line_h * scroll_offset;
        let vp_h = line_h * scroll_lines_per_page as f32;
        window.paint_quad(fill(
            Bounds::new(point(bounds.origin.x, vp_y), size(bounds.size.width, vp_h)),
            viewport_bg,
        ));
        // Top and bottom border of viewport
        window.paint_quad(fill(
            Bounds::new(
                point(bounds.origin.x, vp_y),
                size(bounds.size.width, px(1.0)),
            ),
            viewport_border,
        ));
        window.paint_quad(fill(
            Bounds::new(
                point(bounds.origin.x, vp_y + vp_h - px(1.0)),
                size(bounds.size.width, px(1.0)),
            ),
            viewport_border,
        ));

        // Paint lines as simple blocks showing code density
        let max_render_lines = (bounds.size.height.to_f64() as f32 / MINIMAP_LINE_HEIGHT) as usize;
        let render_count = line_lengths.len().min(max_render_lines).min(total_lines);

        for (line_idx, &visual_len) in line_lengths.iter().enumerate().take(render_count) {
            if visual_len == 0 {
                continue;
            }
            let y = bounds.origin.y + line_h * line_idx as f32;
            if y > bounds.origin.y + bounds.size.height {
                break;
            }
            let w = char_w * (visual_len as f32).min(60.0);
            window.paint_quad(fill(
                Bounds::new(point(bounds.origin.x + px(4.0), y), size(w, line_h)),
                code_color,
            ));
        }
    }
}
