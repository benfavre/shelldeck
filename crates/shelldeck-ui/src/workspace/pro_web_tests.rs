use super::*;
use std::prelude::v1::test;

// Central logout calls the credential store. These tests must never read or
// remove the developer's real Manage credential, even with isolated XDG paths.
struct TestCredentials(String);
impl TestCredentials {
    fn new() -> Self {
        keyring::set_default_credential_builder(keyring::mock::default_credential_builder());
        Self(rust_i18n::locale().to_string())
    }
}
impl Drop for TestCredentials {
    fn drop(&mut self) {
        rust_i18n::set_locale(&self.0);
        keyring::set_default_credential_builder(keyring::default::default_credential_builder());
    }
}

fn fixture(cx: &mut Context<Workspace>) -> Workspace {
    let assistant = cx.new(|cx| {
        AiAssistantView::new(
            AiContext::new(AiSurface::Global, "fixture", serde_json::json!({})),
            cx,
        )
    });
    let mut config = AppConfig::default();
    // Deliberately invalid: these tests exercise the address form without
    // launching a browser or contacting the hosted default.
    config.general.pro_web_url = "invalid-fixture-origin".into();
    config.account = Some(AccountInfo {
        email: "fixture@example.test".into(),
        name: "Fixture".into(),
        is_superadmin: true,
        is_admin: true,
        is_inklura_support: false,
        roles: vec!["superadmin".into()],
    });
    config.cloud_sync.enabled = true;
    config.cloud_sync.mode = AppMode::Dev;
    config.cloud_sync.token = "synthetic-offline".into();
    config.cloud_sync.base_url = "https://fixture.invalid".into();
    Workspace::new(
        cx,
        config,
        vec![],
        ConnectionStore::default(),
        WorkspaceAiBindings {
            assistant,
            tasks: vec![],
            config: Rc::new(RefCell::new(AiConfig::default())),
            clippy_config: Rc::new(RefCell::new(ClippyConfig::default())),
        },
    )
}

// SDTEST-3172 / SDUC-586: exercise the actual invalidation boundary, not only
// the helper it is supposed to call. No browser process or HTTP request runs.
#[test]
fn sdtest_3172_pro_workspace_retains_tools_and_closes_its_session() {
    let _credentials = TestCredentials::new();
    let mut app = TestAppContext::single();
    let workspace = app.update(|cx| cx.new(fixture));
    workspace.update(&mut app, |view, cx| {
        let fixture_origin = view.app_config.general.pro_web_url.clone();
        view.app_config.general.pro_web_url = AppConfig::default().general.pro_web_url;
        view.app_config.account.as_mut().unwrap().email.clear();
        view.toggle_pro(cx);
        assert_eq!(
            view.pro_url_input
                .as_ref()
                .unwrap()
                .read(cx)
                .content()
                .to_string(),
            shelldeck_core::config::pro_web::DEFAULT_URL,
            "opening Pro uses the chosen default instead of asking for an address"
        );
        assert!(
            view.pro_error,
            "the chosen address is submitted immediately"
        );
        assert!(
            view.pro_browser.is_none(),
            "missing identity stops browser launch"
        );
        view.close_pro_session(cx);
        view.app_config.account.as_mut().unwrap().email = "fixture@example.test".into();
        view.app_config.general.pro_web_url = fixture_origin;
        let terminal = view.terminal.entity_id();
        let agents = view.agent_console.entity_id();
        view.toggle_pro(cx);
        assert!(view.pro_open);
        assert!(
            view.pro_browser.is_none(),
            "invalid configured origins are not opened"
        );
        view.toggle_pro(cx);
        assert!(!view.pro_open);
        assert_eq!(terminal, view.terminal.entity_id());
        assert_eq!(agents, view.agent_console.entity_id());

        view.app_config.general.pro_web_url = "https://pro.example.test/".into();
        view.pro_url_input
            .as_ref()
            .unwrap()
            .update(cx, |input, cx| {
                input.replace_content("https://pro.example.test/", cx)
            });
        view.app_config.account.as_mut().unwrap().email.clear();
        view.connect_pro(cx);
        assert!(view.pro_error);
        assert!(
            view.pro_browser.is_none(),
            "identity-less accounts cannot share a browser profile"
        );
        view.app_config.account.as_mut().unwrap().email = "fixture@example.test".into();
        view.toggle_pro(cx);
        assert!(
            view.pro_browser.is_none(),
            "returning to the address form does not submit edits"
        );
        view.toggle_pro(cx);
        let scope = profile_scope(
            &view.account_base_url(),
            "fixture@example.test",
            &view.app_config.general.pro_web_url,
        );
        let browser = cx.new(|cx| NativeBrowserView::new(scope, cx));
        view.pro_browser = Some(browser.clone());
        view.active_view = ActiveView::Agents;
        view.toggle_pro(cx);
        view.agent_console.update(cx, |agents, cx| {
            use shelldeck_core::agent_runtime::{AgentRunRequest, AgentStreamEvent};
            let selected = agents.selected_session_id().unwrap();
            let context = agents
                .sessions()
                .iter()
                .find(|session| session.id == selected)
                .unwrap()
                .context
                .clone();
            let request = AgentRunRequest::new(
                context.provider,
                context.target,
                context.access,
                context.workdir,
                None,
                "Synthetic reply",
            );
            let run_id = request.id;
            agents.begin_run(request, cx);
            agents.push_stream_event_for(
                run_id,
                AgentStreamEvent::Text("Reply while Pro is on screen".into()),
                cx,
            );
            assert!(
                agents
                    .sessions()
                    .iter()
                    .find(|session| session.id == selected)
                    .unwrap()
                    .unread_count
                    > 0,
                "Pro must not mark replies in hidden Agents as read"
            );
        });
        view.reconcile_pro_account(cx);
        assert!(view.pro_open, "same-account whoami refresh keeps Pro open");
        view.set_presentation(WorkspacePresentation::Main, cx);
        assert_eq!(
            view.agent_console
                .read(cx)
                .sessions()
                .iter()
                .map(|session| session.unread_count)
                .sum::<u32>(),
            0,
            "returning to the selected Agents view marks the reply read"
        );
        assert!(
            !view.pro_open,
            "full-app switch must reveal the requested view"
        );
        assert_eq!(
            view.pro_browser.as_ref().unwrap().entity_id(),
            browser.entity_id()
        );
        view.toggle_pro(cx);
        view.set_active_view(ActiveView::Terminal, cx);
        assert!(
            !view.pro_open,
            "native tool navigation must reveal its destination"
        );
        view.toggle_pro(cx);
        view.set_mode(AppMode::Dev, cx);
        assert!(
            !view.pro_open,
            "reselecting the current mode returns to tools"
        );
        view.toggle_pro(cx);
        view.app_config.account.as_mut().unwrap().email = "replacement@example.test".into();
        view.reconcile_pro_account(cx);
        assert!(
            view.pro_browser.is_none(),
            "account replacement disposes the old browser"
        );

        view.pro_browser = Some(browser);
        view.pro_open = true;
        view.invalidate_cloud_session(cx);
        assert!(view.pro_browser.is_none());
        assert!(!view.pro_open);
        assert!(view.pro_url_input.is_none());
        assert!(!view.signed_in());
        assert!(view.settings.read(cx).config.account.is_none());
        view.toggle_pro(cx);
        assert!(!view.pro_open, "signed-out accounts cannot enter Pro");
    });
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

fn visible_bounds(window: &mut VisualTestContext, selector: &str) -> Bounds<Pixels> {
    let bounds = window
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("Missing {selector}"));
    let viewport = window.update(|window, _| window.viewport_size());
    assert!(
        bounds.origin.x >= gpui::px(0.0)
            && bounds.origin.y >= gpui::px(0.0)
            && bounds.right() <= viewport.width
            && bounds.bottom() <= viewport.height,
        "{selector} clipped: {bounds:?}, viewport={viewport:?}"
    );
    bounds
}

fn click(window: &mut VisualTestContext, selector: &str) {
    if selector == "connect-inklura-pro" {
        for _ in 0..8 {
            let button = window.debug_bounds(selector).unwrap();
            let scroll = window.debug_bounds("pro-setup").unwrap();
            if button.bottom() <= scroll.bottom() && button.origin.y >= scroll.origin.y {
                break;
            }
            // The address input owns its own wheel events. Scroll the page's
            // padding, as a person does to reach the form's submit action.
            let position = point(
                scroll.origin.x + gpui::px(8.0),
                scroll.origin.y + gpui::px(16.0),
            );
            window.simulate_mouse_move(position, None, Modifiers::none());
            window.simulate_event(ScrollWheelEvent {
                position,
                delta: ScrollDelta::Pixels(point(gpui::px(0.0), gpui::px(-100.0))),
                touch_phase: TouchPhase::Moved,
                ..Default::default()
            });
            std::thread::sleep(std::time::Duration::from_millis(80));
            redraw(window);
        }
        let button = window.debug_bounds(selector).unwrap();
        let scroll = window.debug_bounds("pro-setup").unwrap();
        assert!(
            button.bottom() <= scroll.bottom() && button.origin.y >= scroll.origin.y,
            "Pro setup must scroll to the submit control: {button:?} in {scroll:?}"
        );
    }
    let position = visible_bounds(window, selector).center();
    window.simulate_mouse_move(position, None, Modifiers::none());
    window.simulate_click(position, Modifiers::none());
    redraw(window);
}

// SDTEST-3174 / SDUC-586: production Workspace rendering, layout and controls.
// The GPUI test platform verifies input/layout, not native GPU painting.
#[test]
fn sdtest_3174_pro_rendered_navigation_and_compact_titlebar() {
    let _credentials = TestCredentials::new();
    for locale in ["fr", "en"] {
        rust_i18n::set_locale(locale);
        let mut app = TestAppContext::single();
        app.update(|cx| {
            adabraka_ui::init(cx);
            adabraka_ui::components::input::init(cx);
            adabraka_ui::set_icon_base_path("icons/lucide");
            adabraka_ui::theme::install_theme(cx, crate::theme::adabraka_theme_from_palette());
        });
        let handle = app.add_window(|_, cx| fixture(cx));
        let workspace = handle.root(&mut app).unwrap();
        let mut window = VisualTestContext::from_window(*handle, &app);
        for (width, font_size) in [(1280.0, 14.0), (680.0, 14.0), (600.0, 20.0), (600.0, 10.0)] {
            workspace.update(&mut app, |view, cx| {
                view.ui_font_size = font_size;
                view.site_directory = Some(SitesPayload::default());
                view.app_config.cloud_sync.active_site_label =
                    Some("A very long business site name".into());
                cx.notify();
            });
            window.simulate_resize(size(gpui::px(width), gpui::px(400.0)));
            redraw(&mut window);
            let titlebar = visible_bounds(&mut window, "workspace-titlebar");
            let button = visible_bounds(&mut window, "open-inklura-pro");
            assert!(
                button.bottom() <= titlebar.bottom(),
                "Pro must fit at every UI scale"
            );
            visible_bounds(&mut window, "titlebar-window-controls");
            click(&mut window, "open-inklura-pro");
            assert!(workspace.read_with(&app, |view, _| view.pro_open));
            click(&mut window, "connect-inklura-pro");
            assert!(workspace.read_with(&app, |view, _| view.pro_error));
            assert!(workspace.read_with(&app, |view, _| view.pro_browser.is_none()));
            click(&mut window, "open-inklura-pro");
            assert!(!workspace.read_with(&app, |view, _| view.pro_open));
        }
        window.simulate_resize(size(gpui::px(1280.0), gpui::px(800.0)));
        redraw(&mut window);
        click(&mut window, "open-inklura-pro");
        click(&mut window, "presentation-switch-full");
        assert!(!workspace.read_with(&app, |view, _| view.pro_open));
        workspace.update(&mut app, |view, cx| {
            view.pro_browser =
                Some(cx.new(|cx| NativeBrowserView::new_workspace("fixture".into(), cx)));
            view.toggle_pro(cx);
        });
        redraw(&mut window);
        assert!(
            window.debug_bounds("native-browser-use-chrome").is_none(),
            "Pro cannot offer the shared Chrome profile"
        );
        app.quit();
    }
}
