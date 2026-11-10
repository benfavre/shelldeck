use super::*;
use crate::scale::px;

impl TerminalView {
    pub(super) fn render_empty_state(&self, cx: &mut Context<Self>) -> impl IntoElement {
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
        let mut launchers = div()
            .id("terminal-empty-launchers")
            .debug_selector(|| "terminal-empty-launchers".into())
            .w_full()
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
            .id("terminal-empty-content")
            .debug_selector(|| "terminal-empty-content".into())
            .flex()
            .flex_col()
            .flex_shrink_0()
            .items_center()
            .w_full()
            .min_w(px(0.0))
            .max_w(px(700.0))
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
                    .w_full()
                    .max_w(px(700.0))
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
                            .gap(px(16.0))
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .flex_1()
                                    .flex_basis(px(250.0))
                                    .min_w(px(0.0))
                                    .child(
                                        div()
                                            .text_size(px(11.0))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(ShellDeckColors::text_muted())
                                            .mb(px(4.0))
                                            .child(t!("shortcuts.group.navigation").to_string()),
                                    )
                                    .child(render_shortcut_rows(navigation_shortcuts)),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .flex_1()
                                    .flex_basis(px(250.0))
                                    .min_w(px(0.0))
                                    .child(
                                        div()
                                            .text_size(px(11.0))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(ShellDeckColors::text_muted())
                                            .mb(px(4.0))
                                            .child(t!("shortcuts.group.terminal").to_string()),
                                    )
                                    .child(render_shortcut_rows(terminal_shortcuts)),
                            ),
                    ),
            );
        div()
            .id("terminal-empty-scroll")
            .debug_selector(|| "terminal-empty-scroll".into())
            .flex()
            .flex_col()
            .items_center()
            .size_full()
            .min_h(px(0.0))
            .min_w(px(0.0))
            .overflow_y_scroll()
            .px(px(24.0))
            .bg(ShellDeckColors::bg_primary())
            // Equal spacers center fitting content. Their minimum size keeps
            // overflowing content top-aligned and entirely reachable by scroll.
            .child(div().flex_1().min_h(px(24.0)))
            .child(content)
            .child(div().flex_1().min_h(px(24.0)))
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
