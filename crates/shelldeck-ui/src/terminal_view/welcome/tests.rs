use super::*;
use gpui::px;
use shelldeck_core::config::app_config::ThemePreference;

struct WelcomeHost {
    terminal: Entity<TerminalView>,
    actions: [usize; 4],
}

impl Render for WelcomeHost {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .on_action(cx.listener(|this, _: &OpenQuickConnect, _, _| this.actions[0] += 1))
            .on_action(cx.listener(|this, _: &ToggleCommandPalette, _, _| this.actions[1] += 1))
            .on_action(cx.listener(|this, _: &OpenSettings, _, _| this.actions[2] += 1))
            .on_action(cx.listener(|this, _: &OpenAutomations, _, _| this.actions[3] += 1))
            .child(self.terminal.clone())
    }
}

fn redraw(window: &mut VisualTestContext) {
    window.run_until_parked();
    for _ in 0..2 {
        window.update(|window, cx| {
            window.refresh();
            window.draw(cx).clear();
        });
        window.run_until_parked();
    }
}

// SDTEST-2930 / SDUC-410: real rendered geometry catches top alignment,
// clipped wrapped cards and unreachable content at large UI scales.
#[::core::prelude::v1::test]
fn rendered_welcome_centers_when_fitting_and_scrolls_when_short() {
    let previous_locale = rust_i18n::locale().to_string();
    for (theme, locale) in [
        (ThemePreference::Dark, "en"),
        (ThemePreference::SolarizedLight, "fr"),
    ] {
        for (width, height, rem) in [
            (1200.0, 1000.0, 16.0),
            (620.0, 500.0, 16.0),
            (620.0, 700.0, 24.0),
        ] {
            rust_i18n::set_locale(locale);
            let mut app = TestAppContext::single();
            app.update(|cx| {
                adabraka_ui::init(cx);
                ShellDeckColors::set_theme(&theme);
                adabraka_ui::theme::install_theme(cx, crate::theme::adabraka_theme_from_palette());
            });
            let handle = app.add_window(|_, cx| {
                let mut terminal = TerminalView::new(cx);
                terminal.claude_available = true;
                terminal.codex_available = true;
                terminal
            });
            let mut window = VisualTestContext::from_window(*handle, &app);
            window.simulate_resize(size(px(width), px(height)));
            window.update(|window, _| window.set_rem_size(px(rem)));
            redraw(&mut window);
            let content = window.debug_bounds("terminal-welcome-content").unwrap();
            let viewport = window.debug_bounds("terminal-empty-scroll").unwrap();
            let padding = px(24.0 * rem / 16.0);
            if content.size.height + padding * 2.0 <= viewport.size.height {
                assert!(
                    (content.center().y - viewport.center().y).abs() <= px(1.0),
                    "{content:?} {viewport:?}"
                );
            } else {
                assert!(
                    (content.top() - viewport.top() - padding).abs() <= px(1.0),
                    "The first action must remain reachable: {content:?}"
                );
                window.simulate_event(ScrollWheelEvent {
                    position: viewport.center(),
                    delta: ScrollDelta::Pixels(point(px(0.0), px(-4000.0))),
                    touch_phase: TouchPhase::Moved,
                    ..Default::default()
                });
                std::thread::sleep(std::time::Duration::from_millis(90));
                redraw(&mut window);
                let scrolled = window.debug_bounds("terminal-welcome-content").unwrap();
                assert!(scrolled.top() < content.top(), "Short windows must scroll");
                let shortcuts = window.debug_bounds("terminal-empty-shortcuts").unwrap();
                assert!(
                    shortcuts.bottom() <= viewport.bottom(),
                    "The reference must be fully reachable: {shortcuts:?}"
                );
            }
            for selector in [
                "empty-launch-terminal",
                "empty-launch-claude",
                "empty-launch-codex",
                "terminal-empty-shortcuts",
            ] {
                let bounds = window.debug_bounds(selector).unwrap();
                assert!(
                    bounds.left() >= viewport.left() && bounds.right() <= viewport.right(),
                    "Horizontal overflow: {selector} {bounds:?}"
                );
            }
        }
    }
    rust_i18n::set_locale(&previous_locale);
}

// SDTEST-2931 / SDUC-410: clicks traverse the production focus/dispatch path;
// no shell, provider, network connection or real Workspace is started.
#[::core::prelude::v1::test]
fn rendered_welcome_routes_navigation_and_hides_uninstalled_providers() {
    let mut app = TestAppContext::single();
    app.update(|cx| {
        adabraka_ui::init(cx);
        adabraka_ui::theme::install_theme(cx, crate::theme::adabraka_theme_from_palette());
    });
    let handle = app.add_window(|_, cx| WelcomeHost {
        terminal: cx.new(TerminalView::new),
        actions: [0; 4],
    });
    let host = handle.root(&mut app).unwrap();
    let terminal = host.read_with(&app, |host, _| host.terminal.clone());
    let mut window = VisualTestContext::from_window(*handle, &app);
    window.simulate_resize(size(px(1200.0), px(1000.0)));
    for (claude, codex) in [(false, false), (true, false), (false, true), (true, true)] {
        terminal.update(&mut app, |view, cx| {
            view.claude_available = claude;
            view.codex_available = codex;
            cx.notify();
        });
        redraw(&mut window);
        assert!(window.debug_bounds("empty-launch-terminal").is_some());
        assert_eq!(window.debug_bounds("empty-launch-claude").is_some(), claude);
        assert_eq!(window.debug_bounds("empty-launch-codex").is_some(), codex);
    }
    for selector in [
        "terminal-empty-connect",
        "terminal-empty-palette",
        "terminal-empty-settings",
        "terminal-empty-automations",
    ] {
        let bounds = window.debug_bounds(selector).unwrap();
        window.simulate_click(bounds.center(), Modifiers::none());
        redraw(&mut window);
    }
    assert_eq!(host.read_with(&app, |host, _| host.actions), [1; 4]);
}
