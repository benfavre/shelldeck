use super::*;
use crate::scale::px;

/// Provider-free native fixtures can inspect actual prepainted child bounds.
/// This is compiled out of release builds and never logs normal app sessions.
pub(crate) fn trace_layout_for_preview(id: SharedString, element: Div) -> Div {
    #[cfg(debug_assertions)]
    if std::env::var_os("SHELLDECK_PREVIEW_TERMINAL_METRICS").is_some() {
        return element.on_children_prepainted(move |bounds, window, _| {
            let children: Vec<[f32; 4]> = bounds.iter().map(|b| [b.origin.x.to_f64() as f32, b.origin.y.to_f64() as f32, b.size.width.to_f64() as f32, b.size.height.to_f64() as f32]).collect();
            eprintln!("TERMINAL_LAYOUT {}", serde_json::json!({"id": id.as_ref(), "scale": window.rem_size().to_f64() / crate::scale::REM_BASE as f64, "children": children}));
        });
    }
    #[cfg(not(debug_assertions))]
    let _ = id;
    element
}

impl TerminalView {
    pub(super) fn render_empty_state(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        if let Some(message) = self.required_cwd_unavailable_message() {
            return div()
                .id("terminal-workspace-folder-unavailable")
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .size_full()
                .p(px(24.0))
                .gap(px(8.0))
                .bg(ShellDeckColors::bg_primary())
                .child(
                    div()
                        .text_size(px(16.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(ShellDeckColors::text_primary())
                        .child(t!("terminal.empty.workspace_unavailable_title").to_string()),
                )
                .child(
                    div()
                        .max_w(px(620.0))
                        .text_center()
                        .text_size(px(12.0))
                        .text_color(ShellDeckColors::text_muted())
                        .child(message),
                );
        }
        let scale = window.rem_size().to_f64() as f32 / crate::scale::REM_BASE;
        let measured = if self.empty_state_width > 0.0 {
            self.empty_state_width
        } else {
            window.viewport_size().width.to_f64() as f32
        };
        let content_width = (measured / scale - 48.0).clamp(1.0, 700.0);
        let launcher_count =
            1 + usize::from(self.claude_available) + usize::from(self.codex_available);
        let columns = if content_width >= 650.0 {
            3
        } else if content_width >= 450.0 {
            2
        } else {
            1
        };
        let columns = columns.min(launcher_count);
        let card_width =
            ((content_width - 10.0 * (columns - 1) as f32) / columns as f32).min(240.0);
        let reference_width = (content_width - 42.0).max(1.0);
        let shortcut_columns = if reference_width >= 576.0 { 2 } else { 1 };
        let shortcut_width =
            (reference_width - 16.0 * (shortcut_columns - 1) as f32) / shortcut_columns as f32;

        let mut launchers = div()
            .id("terminal-empty-launchers")
            .debug_selector(|| "terminal-empty-launchers".into())
            .w(px(content_width))
            .min_w(px(0.0))
            .flex()
            .flex_wrap()
            .flex_shrink_0()
            .items_center()
            .justify_center()
            .gap(px(10.0))
            .max_w(px(700.0))
            .child(
                Self::launcher_card(
                    "empty-launch-terminal",
                    t!("terminal.empty.open_terminal").to_string(),
                    t!("terminal.empty.open_terminal_cmd").to_string(),
                    SharedString::from(t!("terminal.empty.open_terminal_cmd").to_string()),
                    ShellDeckColors::primary(),
                    Self::terminal_logo(26.0).into_any_element(),
                    card_width,
                )
                .on_click(cx.listener(|this, _, window, cx| {
                    this.spawn_local_terminal(cx);
                    this.focus_handle.focus(window);
                    cx.emit(TerminalEvent::NewTabRequested);
                    cx.notify();
                })),
            );

        if self.claude_available {
            launchers = launchers.child(
                Self::launcher_card(
                    "empty-launch-claude",
                    t!("terminal.empty.launch_claude").to_string(),
                    t!("terminal.empty.launch_claude_caption").to_string(),
                    SharedString::from(CLAUDE_CLI_COMMAND),
                    Self::claude_orange(),
                    Self::claude_logo(26.0).into_any_element(),
                    card_width,
                )
                .on_click(cx.listener(|this, _, window, cx| {
                    this.launch_claude(cx);
                    this.focus_handle.focus(window);
                })),
            );
        }

        if self.codex_available {
            launchers = launchers.child(
                Self::launcher_card(
                    "empty-launch-codex",
                    t!("terminal.empty.launch_codex").to_string(),
                    t!("terminal.empty.launch_codex_caption").to_string(),
                    SharedString::from(CODEX_CLI_COMMAND),
                    Self::codex_green(),
                    Self::codex_logo(26.0).into_any_element(),
                    card_width,
                )
                .on_click(cx.listener(|this, _, window, cx| {
                    this.launch_codex(cx);
                    this.focus_handle.focus(window);
                })),
            );
        }

        let shortcuts = shortcuts_for(ShortcutSurface::TerminalEmpty, true);
        let navigation_shortcuts = shortcuts
            .iter()
            .copied()
            .filter(|shortcut| shortcut.group == ShortcutGroup::Navigation);
        let terminal_shortcuts = shortcuts
            .iter()
            .copied()
            .filter(|shortcut| shortcut.group == ShortcutGroup::Terminal);

        let content = div()
            .flex()
            .flex_col()
            .flex_shrink_0()
            .items_center()
            .w(px(content_width))
            .min_w(px(0.0))
            .gap(px(24.0))
            // Icon + heading
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_shrink_0()
                    .items_center()
                    .w_full()
                    .gap(px(10.0))
                    .text_center()
                    .child(crate::brand::brand_badge(56.0))
                    .child(
                        div()
                            .text_size(px(22.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(ShellDeckColors::text_primary())
                            .child(t!("terminal.empty.title").to_string()),
                    )
                    .child(
                        div()
                            .text_size(px(13.0))
                            .text_color(ShellDeckColors::text_muted())
                            .child(t!("terminal.empty.hint").to_string()),
                    ),
            )
            .child(launchers)
            // Keyboard shortcuts reference
            .child(
                div()
                    .id("terminal-empty-shortcuts")
                    .debug_selector(|| "terminal-empty-shortcuts".into())
                    .flex()
                    .flex_col()
                    .flex_shrink_0()
                    .p(px(20.0))
                    .rounded(px(8.0))
                    .bg(ShellDeckColors::bg_surface())
                    .border_1()
                    .border_color(ShellDeckColors::border())
                    .w(px(content_width))
                    .gap(px(2.0))
                    .child(
                        div()
                            .text_size(px(13.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(ShellDeckColors::text_primary())
                            .mb(px(8.0))
                            .child(t!("shortcuts.title").to_string()),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .w(px(reference_width))
                            .gap(px(16.0))
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .w(px(shortcut_width))
                                    .flex_shrink_0()
                                    .child(
                                        div()
                                            .text_size(px(11.0))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(ShellDeckColors::text_muted())
                                            .mb(px(4.0))
                                            .child(t!("shortcuts.group.navigation").to_string()),
                                    )
                                    .child(
                                        crate::shortcut_reference::render_shortcut_rows_with_width(
                                            navigation_shortcuts,
                                            shortcut_width,
                                        ),
                                    ),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .w(px(shortcut_width))
                                    .flex_shrink_0()
                                    .child(
                                        div()
                                            .text_size(px(11.0))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(ShellDeckColors::text_muted())
                                            .mb(px(4.0))
                                            .child(t!("shortcuts.group.terminal").to_string()),
                                    )
                                    .child(
                                        crate::shortcut_reference::render_shortcut_rows_with_width(
                                            terminal_shortcuts,
                                            shortcut_width,
                                        ),
                                    ),
                            ),
                    ),
            );
        let content = trace_layout_for_preview("terminal-empty-content".into(), content)
            .id("terminal-empty-content")
            .debug_selector(|| "terminal-empty-content".into());
        let viewport = div()
            .flex()
            .flex_col()
            .items_center()
            .size_full()
            .min_h(px(0.0))
            .min_w(px(0.0))
            .px(px(24.0))
            .bg(ShellDeckColors::bg_primary())
            // Equal spacers center fitting content. Their minimum size keeps
            // overflowing content top-aligned and entirely reachable by scroll.
            .child(div().flex_1().min_h(px(24.0)))
            .child(content)
            .child(div().flex_1().min_h(px(24.0)));
        trace_layout_for_preview("terminal-empty-scroll".into(), viewport)
            .id("terminal-empty-scroll")
            .debug_selector(|| "terminal-empty-scroll".into())
            .overflow_y_scroll()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // SDTEST-2948 / SDUC-410: rendered sizing, not a mirror of flex properties.
    #[::core::prelude::v1::test]
    fn terminal_welcome_centers_and_scrolls_without_losing_launch_actions() {
        use shelldeck_core::config::app_config::{ThemePreference, UiLanguage};
        let previous_locale = rust_i18n::locale().to_string();
        for language in [UiLanguage::En, UiLanguage::Fr] {
            for theme in [ThemePreference::Dark, ThemePreference::SolarizedLight] {
                for (width, height, font_size) in [
                    (1200.0, 1100.0, 14.0),
                    (1734.0, 1250.0, 17.5),
                    (620.0, 420.0, 14.0),
                    (320.0, 300.0, 22.0),
                ] {
                    let mut app = TestAppContext::single();
                    app.update(|cx| {
                        adabraka_ui::init(cx);
                        crate::apply_ui_language(&language);
                        ShellDeckColors::set_theme(&theme);
                        adabraka_ui::theme::install_theme(
                            cx,
                            crate::theme::adabraka_theme_from_palette(),
                        );
                    });
                    let handle = app.add_window(|window, cx| {
                        window.set_rem_size(gpui::px(crate::scale::rem_size_for_scale(
                            crate::scale::scale_for_font_size(font_size),
                        )));
                        let mut terminal = TerminalView::new(cx);
                        terminal.claude_available = true;
                        terminal.codex_available = true;
                        terminal
                    });
                    let mut window = VisualTestContext::from_window(*handle, &app);
                    window.simulate_resize(size(gpui::px(width), gpui::px(height)));
                    window.run_until_parked();
                    for _ in 0..2 {
                        window.update(|window, cx| {
                            window.refresh();
                            window.draw(cx).clear();
                        });
                        window.run_until_parked();
                    }
                    let viewport = window.debug_bounds("terminal-empty-scroll").unwrap();
                    let content = window.debug_bounds("terminal-empty-content").unwrap();
                    assert!(
                        content.left() >= viewport.left(),
                        "left overflow: {content:?} in {viewport:?}"
                    );
                    assert!(
                        content.right() <= viewport.right(),
                        "right overflow: {content:?} in {viewport:?}"
                    );
                    if content.size.height < viewport.size.height - gpui::px(80.0) {
                        assert!(
                            (content.center().y - viewport.center().y).abs() < gpui::px(2.0),
                            "not vertically centered: {content:?} in {viewport:?}"
                        );
                    } else {
                        assert!(
                            content.top() >= viewport.top(),
                            "short window must not clip its first action"
                        );
                    }
                    let cards = [
                        "empty-launch-terminal",
                        "empty-launch-claude",
                        "empty-launch-codex",
                    ]
                    .map(|id| window.debug_bounds(id).unwrap());
                    for card in cards {
                        let scale = crate::scale::scale_for_font_size(font_size);
                        assert!(
                            (card.size.height - gpui::px(84.0 * scale)).abs() < gpui::px(2.0),
                            "launcher must stay compact: {card:?}"
                        );
                        assert!(
                            card.left() >= content.left() && card.right() <= content.right(),
                            "launcher outside content: {card:?} in {content:?}"
                        );
                    }
                    for pair in cards.windows(2) {
                        assert!(
                            pair[0].right() <= pair[1].left() || pair[0].bottom() <= pair[1].top(),
                            "launchers overlap: {pair:?}"
                        );
                    }
                    for id in [
                        "empty-launch-terminal",
                        "empty-launch-claude",
                        "empty-launch-codex",
                    ] {
                        let text = window.debug_bounds(&format!("{id}-text")).unwrap();
                        assert!(
                            text.size.width > gpui::px(70.0),
                            "launcher text collapsed: {text:?}"
                        );
                        assert!(
                            text.size.height
                                < gpui::px(62.0 * crate::scale::scale_for_font_size(font_size)),
                            "launcher text became a vertical column: {text:?}"
                        );
                    }
                    for shortcut in shortcuts_for(ShortcutSurface::TerminalEmpty, true) {
                        if let Some(label) =
                            window.debug_bounds(&format!("shortcut-label-{:?}", shortcut.id))
                        {
                            assert!(
                                label.size.width > gpui::px(80.0),
                                "shortcut label collapsed: {label:?}"
                            );
                            assert!(
                                label.size.height
                                    < gpui::px(48.0 * crate::scale::scale_for_font_size(font_size)),
                                "shortcut label became a vertical column: {label:?}"
                            );
                        }
                    }
                    let shortcuts = window.debug_bounds("terminal-empty-shortcuts").unwrap();
                    assert!(shortcuts.right() <= viewport.right());
                    assert!(shortcuts.left() >= viewport.left());
                    if content.size.height > viewport.size.height {
                        let position = viewport.center();
                        window.simulate_mouse_move(position, None, Modifiers::none());
                        window.simulate_event(ScrollWheelEvent {
                            position,
                            delta: ScrollDelta::Pixels(point(gpui::px(0.0), gpui::px(-20_000.0))),
                            touch_phase: TouchPhase::Moved,
                            ..Default::default()
                        });
                        std::thread::sleep(std::time::Duration::from_millis(80));
                        window.update(|window, cx| {
                            window.refresh();
                            window.draw(cx).clear();
                        });
                        window.run_until_parked();
                        let end = window.debug_bounds("terminal-empty-shortcuts").unwrap();
                        assert!(
                            end.bottom() <= viewport.bottom(),
                            "shortcuts cannot be scrolled into view: {end:?} in {viewport:?}"
                        );
                    }
                }
            }
        }
        rust_i18n::set_locale(&previous_locale);
    }
}
