use super::*;
use shelldeck_core::config::app_config::ThemePreference;

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

// SDTEST-2937: choosing This computer routes a real template form to the
// existing local task store; saving stays inactive and activation is separate.
#[::core::prelude::v1::test]
fn rendered_task_target_choice_saves_to_the_local_scheduler() {
    use crate::agent_console_view::AgentConsoleView;
    use shelldeck_core::agent_runtime::{AgentAccessMode, AgentProvider, AgentTarget};
    use shelldeck_core::agent_session::AgentExecutionContext;
    let mut app = TestAppContext::single();
    app.update(|cx| {
        adabraka_ui::init(cx);
        adabraka_ui::components::input::init(cx);
    });
    let agents = app.update(|cx| cx.new(AgentConsoleView::new));
    app.run_until_parked();
    let context = AgentExecutionContext {
        provider: AgentProvider::Codex,
        target: AgentTarget::Local,
        access: AgentAccessMode::ReadOnly,
        workdir: std::env::temp_dir().to_string_lossy().into_owned(),
        model: None,
    };
    let session = agents.update(&mut app, |agents, cx| {
        agents.set_workflows_authorized(true, cx);
        agents
            .create_session("Local task fixture", context.clone(), cx)
            .unwrap()
    });
    let handle = app.add_window(|_, cx| {
        let mut view = AutomationsView::new(cx);
        view.context = Some(AutomationContext {
            base: "https://fixture.invalid".into(),
            token: "sd_fixture".into(),
            site: "fixture".into(),
        });
        view.data = Some(AutomationData::default());
        view.set_local_agents(agents.clone(), cx);
        view.start_task(Some(0), cx);
        view
    });
    let view = handle.root(&mut app).unwrap();
    let mut window = VisualTestContext::from_window(*handle, &app);
    window.simulate_resize(size(gpui::px(1200.0), gpui::px(1600.0)));
    redraw(&mut window);
    let choice = window.debug_bounds("automation-target-1").unwrap();
    window.simulate_click(choice.center(), Modifiers::none());
    redraw(&mut window);
    assert!(
        view.read_with(&app, |view, _| view.form.as_ref().unwrap().target
            == TaskTarget::Local)
    );
    window.simulate_resize(size(gpui::px(1200.0), gpui::px(800.0)));
    redraw(&mut window);
    let viewport = window.debug_bounds("automations-scroll").unwrap();
    assert!(
        window
            .debug_bounds("automation-save-draft")
            .unwrap()
            .bottom()
            > viewport.bottom()
    );
    window.simulate_event(ScrollWheelEvent {
        position: viewport.center(),
        delta: ScrollDelta::Pixels(point(gpui::px(0.0), gpui::px(-4000.0))),
        touch_phase: TouchPhase::Moved,
        ..Default::default()
    });
    std::thread::sleep(std::time::Duration::from_millis(90));
    redraw(&mut window);
    let save = window.debug_bounds("automation-save-draft").unwrap();
    assert!(
        save.bottom() <= viewport.bottom(),
        "Local tasks must scroll to their save action: {save:?}; {viewport:?}"
    );
    window.simulate_click(save.center(), Modifiers::none());
    redraw(&mut window);
    let tasks = agents.read_with(&app, |agents, _| agents.local_scheduled_tasks());
    let id = view.read_with(&app, |view, _| view.local_editor.unwrap());
    let task = tasks.iter().find(|task| task.id == id).unwrap();
    assert_eq!(task.context, context);
    assert_eq!(task.session_id, session);
    assert!(!task.enabled);
    assert_eq!(task.calendar.as_ref().unwrap().timezone, "Europe/Paris");
    assert!(view.read_with(&app, |view, _| view.local_editor == Some(id)
        && view.form.is_none()));
    view.update(&mut app, |view, cx| {
        view.local_command(
            id,
            crate::agent_console_view::LocalTaskCommand::Activate,
            cx,
        )
    });
    assert!(agents.read_with(&app, |agents, _| agents
        .local_scheduled_tasks()
        .iter()
        .find(|task| task.id == id)
        .unwrap()
        .enabled));
    view.update(&mut app, |view, cx| {
        view.local_command(id, crate::agent_console_view::LocalTaskCommand::Pause, cx)
    });
    assert!(!agents.read_with(&app, |agents, _| agents
        .local_scheduled_tasks()
        .iter()
        .find(|task| task.id == id)
        .unwrap()
        .enabled));
    view.update(&mut app, |view, cx| {
        view.local_command(id, crate::agent_console_view::LocalTaskCommand::Delete, cx)
    });
    assert!(agents.read_with(&app, |agents, _| agents
        .local_scheduled_tasks()
        .iter()
        .all(|task| task.id != id)));
    app.run_until_parked();
}
// SDTEST-2933 / SDUC-572: input travels through the real template and form;
// invalid saves preserve edits, and changing accounts clears the private draft.
#[::core::prelude::v1::test]
fn rendered_task_templates_preserve_invalid_drafts_and_clear_on_scope_change() {
    let previous_locale = rust_i18n::locale().to_string();
    for (theme, locale) in [
        (ThemePreference::Dark, "en"),
        (ThemePreference::SolarizedLight, "fr"),
    ] {
        rust_i18n::set_locale(locale);
        assert_eq!(
            t!("automations.title").to_string(),
            if locale == "fr" {
                "Automatisations"
            } else {
                "Automations"
            }
        );
        let mut app = TestAppContext::single();
        app.update(|cx| {
            adabraka_ui::init(cx);
            adabraka_ui::components::input::init(cx);
            ShellDeckColors::set_theme(&theme);
            adabraka_ui::theme::install_theme(cx, crate::theme::adabraka_theme_from_palette());
        });
        let handle = app.add_window(|_, cx| {
            let mut view = AutomationsView::new(cx);
            view.context = Some(AutomationContext {
                base: "https://fixture.invalid".into(),
                token: "sd_fixture".into(),
                site: "first-site".into(),
            });
            view.data = Some(AutomationData::default());
            view
        });
        let view = handle.root(&mut app).unwrap();
        let mut window = VisualTestContext::from_window(*handle, &app);
        window.simulate_resize(size(gpui::px(1200.0), gpui::px(1000.0)));
        redraw(&mut window);
        let content = window.debug_bounds("automation-template-content").unwrap();
        let viewport = window.debug_bounds("automations-scroll").unwrap();
        assert!(
            content.top() >= viewport.top() && content.bottom() <= viewport.bottom(),
            "The complete gallery must fit: {content:?}; {viewport:?}"
        );
        assert!(
            (content.center().y - viewport.center().y).abs() <= gpui::px(1.0),
            "Center the complete gallery: {content:?}; {viewport:?}"
        );
        let last = window.debug_bounds("automation-template-6").unwrap();
        assert!(
            last.bottom() <= viewport.bottom() - gpui::px(12.0),
            "Every template row must remain visible with bottom clearance: {last:?}; {content:?}"
        );
        let bounds = window.debug_bounds("automation-template-0").unwrap();
        window.simulate_click(bounds.center(), Modifiers::none());
        redraw(&mut window);
        assert!(window.debug_bounds("automation-name").is_some());
        let initial = view.read_with(&app, |view, cx| {
            view.form
                .as_ref()
                .unwrap()
                .prompt
                .read(cx)
                .content()
                .to_string()
        });
        assert!(!initial.is_empty() && !initial.starts_with("automations."));
        let bounds = window.debug_bounds("automation-prompt").unwrap();
        window.simulate_click(bounds.center(), Modifiers::none());
        window.simulate_input(" private draft addition");
        redraw(&mut window);
        let edited = view.read_with(&app, |view, cx| {
            view.form
                .as_ref()
                .unwrap()
                .prompt
                .read(cx)
                .content()
                .to_string()
        });
        assert!(edited.contains("private draft addition"));
        view.update(&mut app, |view, cx| {
            view.form
                .as_ref()
                .unwrap()
                .name
                .update(cx, |state, cx| state.replace_content("", cx));
            cx.notify();
        });
        redraw(&mut window);
        let viewport = window.debug_bounds("automations-scroll").unwrap();
        if window
            .debug_bounds("automation-save-draft")
            .unwrap()
            .bottom()
            > viewport.bottom()
        {
            window.simulate_event(ScrollWheelEvent {
                position: point(viewport.right() - gpui::px(8.0), viewport.center().y),
                delta: ScrollDelta::Pixels(point(gpui::px(0.0), gpui::px(-4000.0))),
                touch_phase: TouchPhase::Moved,
                ..Default::default()
            });
            std::thread::sleep(std::time::Duration::from_millis(90));
            redraw(&mut window);
        }
        let save = window.debug_bounds("automation-save-draft").unwrap();
        assert!(
            save.bottom() <= viewport.bottom(),
            "The draft action must be reachable after scrolling"
        );
        window.simulate_click(save.center(), Modifiers::none());
        redraw(&mut window);
        assert!(view.read_with(&app, |view, cx| view.error.is_some()
            && view.form.as_ref().unwrap().prompt.read(cx).content() == edited));
        view.update(&mut app, |view, cx| {
            view.set_context(None, String::new(), false, cx)
        });
        redraw(&mut window);
        assert!(view.read_with(&app, |view, _| view.form.is_none()
            && view.editor.is_none()
            && view.data.is_none()
            && view.error.is_none()));
        assert!(window.debug_bounds("automation-prompt").is_none());
    }
    rust_i18n::set_locale(&previous_locale);
}
