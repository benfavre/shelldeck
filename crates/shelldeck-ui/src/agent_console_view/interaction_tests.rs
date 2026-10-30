//! Render the production cockpit and dispatch input through its actual controls.
//! The GPUI test platform owns the clipboard, clock and window; providers never run.
use super::*;
use shelldeck_core::agent_remote_history::{
    RemoteHistoryEntry, RemoteHistoryKind, RemoteHistoryPage,
};
use shelldeck_core::config::app_config::ThemePreference;
use std::{cell::RefCell, rc::Rc};

static INTERACTION_LOCK: parking_lot::ReentrantMutex<()> = parking_lot::ReentrantMutex::new(());
use transcript::TranscriptId;

struct Cockpit {
    app: TestAppContext,
    window: VisualTestContext,
    view: Entity<AgentConsoleView>,
    session: Uuid,
    source: TranscriptId,
    original: String,
    previous_locale: String,
    events: Rc<RefCell<Vec<AgentConsoleEvent>>>,
    _events_subscription: Subscription,
    _directory: tempfile::TempDir,
    _guard: parking_lot::ReentrantMutexGuard<'static, ()>,
}

impl Cockpit {
    fn new(theme: ThemePreference, width: f32, height: f32, native: bool) -> Self {
        let guard = INTERACTION_LOCK.lock();
        let previous_locale = rust_i18n::locale().to_string();
        let mut app = TestAppContext::single();
        app.update(|cx| {
            adabraka_ui::init(cx);
            adabraka_ui::components::input::init(cx);
            adabraka_ui::set_icon_base_path("icons/lucide");
            ShellDeckColors::set_theme(&theme);
            adabraka_ui::theme::install_theme(cx, crate::theme::adabraka_theme_from_palette());
        });
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().display().to_string();
        let original = format!(
            "Before · é🌍\n{}After: preserve this explanation.\n",
            ("aB3+/xY9".repeat(8) + "\n").repeat(80)
        );
        let text = original.clone();
        let handle = app.add_window(move |_, cx| {
            let mut view = AgentConsoleView::new(cx);
            view.library = AgentLibraryUi::with_store(
                cx,
                shelldeck_core::agent_session::storage::AgentSessionStore::new(
                    std::path::Path::new(&root).join("sessions.json"),
                ),
            );
            view.sessions = AgentSessionCollection::default();
            view.session_ui.clear();
            view.tools.navigator_visible = false;
            view.tools.panel_visible = false;
            view.set_surface_visible(true, cx);
            let connection_id = Uuid::new_v4();
            let context = AgentExecutionContext {
                provider: AgentProvider::Codex,
                target: if native {
                    AgentTarget::Ssh {
                        connection_id,
                        connection_label: "Fixture".into(),
                    }
                } else {
                    AgentTarget::Local
                },
                access: AgentAccessMode::ReadOnly,
                workdir: if native { "/srv/fixture".into() } else { root },
                model: None,
            };
            let id = if native {
                view.connections.push(AgentConnectionOption {
                    id: connection_id,
                    label: "Fixture".into(),
                    host: "fixture.invalid".into(),
                });
                view.sessions
                    .reconnect_remote("Native transcript", context, "fixture-native", 1)
                    .unwrap()
            } else {
                let id = view.sessions.create("Transcript", context, 1).unwrap();
                view.sessions.begin_run(id, "Inspect the image", 2).unwrap();
                view.sessions
                    .apply_stream_event(id, AgentStreamEvent::Text(text.clone()), 3)
                    .unwrap();
                view.sessions
                    .finish(id, AgentSessionStatus::Completed, 4)
                    .unwrap();
                id
            };
            view.session_ui.insert(id, AgentSessionUi::new(cx));
            if native {
                view.install_native_history(
                    id,
                    RemoteHistoryPage {
                        provider: AgentProvider::Codex,
                        session_id: "fixture-native".into(),
                        workdir: "/srv/fixture".into(),
                        model: None,
                        snapshot: "fixture-snapshot".into(),
                        total_entries: 1,
                        start: 0,
                        end: 1,
                        entries: vec![RemoteHistoryEntry {
                            index: 0,
                            kind: RemoteHistoryKind::Tool,
                            title: Some("Read image.txt".into()),
                            text,
                            at_ms: Some(1),
                            part: 1,
                            parts: 1,
                        }],
                        warnings: vec![],
                    },
                );
            }
            view.apply_selected_context(cx);
            view.request_surface_focus(cx);
            view
        });
        let view = handle.root(&mut app).unwrap();
        let events = Rc::new(RefCell::new(Vec::new()));
        let observer = events.clone();
        let events_subscription = app.update(|cx| {
            cx.subscribe(&view, move |_, event: &AgentConsoleEvent, _| {
                observer.borrow_mut().push(event.clone())
            })
        });
        let window = VisualTestContext::from_window(*handle, &app);
        window.simulate_resize(size(gpui::px(width), gpui::px(height)));
        let (session, source) = view.read_with(&app, |view, _| {
            let session = view.sessions.selected().unwrap();
            let source = if native {
                TranscriptId::Native {
                    snapshot: "fixture-snapshot".into(),
                    index: 0,
                }
            } else {
                TranscriptId::Message(session.messages.last().unwrap().id)
            };
            (session.id, source)
        });
        let mut cockpit = Self {
            app,
            window,
            view,
            session,
            source,
            original,
            previous_locale,
            events,
            _events_subscription: events_subscription,
            _directory: directory,
            _guard: guard,
        };
        cockpit.redraw();
        cockpit
    }

    fn redraw(&mut self) {
        self.window.run_until_parked();
        for _ in 0..2 {
            self.window.update(|window, cx| {
                window.refresh();
                window.draw(cx).clear();
            });
            self.window.run_until_parked();
        }
    }

    fn bounds(&mut self, selector: &str) -> Option<Bounds<Pixels>> {
        self.window.debug_bounds(selector)
    }

    fn click(&mut self, selector: &str) {
        let bounds = self
            .bounds(selector)
            .unwrap_or_else(|| panic!("Missing rendered control: {selector}"));
        let position = bounds.center();
        let viewport = self.window.update(|window, _| window.viewport_size());
        if selector.starts_with("agent-history-") {
            let scroll = self.bounds("agent-timeline-scroll").unwrap();
            assert!(
                scroll.contains(&position),
                "Control is clipped by the timeline: {selector} {bounds:?}; scroll={scroll:?}"
            );
        }
        assert!(
            position.x >= gpui::px(0.0)
                && position.x <= viewport.width
                && position.y >= gpui::px(0.0)
                && position.y <= viewport.height,
            "Control is outside the visible window: {selector} {bounds:?}"
        );
        self.window
            .simulate_mouse_move(position, None, Modifiers::none());
        self.window.simulate_click(position, Modifiers::none());
        self.redraw();
    }

    fn composer(&mut self) -> Bounds<Pixels> {
        self.bounds(&format!("agent-composer-{}-frame", self.session))
            .expect("The composer must be rendered")
    }

    fn wheel(&mut self, amount: f32) {
        let position = self.bounds("agent-timeline-scroll").unwrap().center();
        self.window.simulate_event(ScrollWheelEvent {
            position,
            delta: ScrollDelta::Pixels(point(gpui::px(0.0), gpui::px(amount))),
            touch_phase: TouchPhase::Moved,
            ..Default::default()
        });
        self.redraw();
    }

    fn start(&mut self) -> Uuid {
        let session = self.session;
        let run = self.view.update(&mut self.app, |view, cx| {
            let request = view
                .sessions
                .begin_run(session, "Continue checking", 5)
                .unwrap();
            let run = request.id;
            view.begin_run(request, cx);
            view.apply_selected_context(cx);
            view.push_stream_event_for(run, AgentStreamEvent::Ready, cx);
            run
        });
        self.redraw();
        run
    }

    fn type_draft(&mut self, text: &str) {
        self.window.dispatch_action(AgentFocusComposer);
        self.redraw();
        self.window.simulate_input(text);
        self.redraw();
        assert!(
            self.draft().contains(text),
            "real text input must reach the active composer"
        );
    }

    fn draft(&self) -> String {
        self.view.read_with(&self.app, |view, cx| {
            view.session_ui[&self.session]
                .prompt
                .read(cx)
                .content()
                .to_owned()
        })
    }

    fn at_end(&self) -> bool {
        self.view.read_with(&self.app, |view, _| {
            (view.scroll.max_offset().height + view.scroll.offset().y).abs() <= gpui::px(2.0)
        })
    }
}

impl Drop for Cockpit {
    fn drop(&mut self) {
        rust_i18n::set_locale(&self.previous_locale);
        ShellDeckColors::set_theme(&ThemePreference::Dark);
        self.app.update(|cx| {
            adabraka_ui::theme::install_theme(cx, crate::theme::adabraka_theme_from_palette())
        });
        self.app.quit();
    }
}

// SDTEST-2848 / SDUC-563, SDUC-564 — actual rendered clicks and layout, not handler calls.
#[::core::prelude::v1::test]
fn rendered_transcript_reveal_copy_and_collapse_keep_the_composer_accessible() {
    for theme in [ThemePreference::Dark, ThemePreference::SolarizedLight] {
        for (width, height) in [(1280.0, 900.0), (900.0, 700.0), (620.0, 560.0)] {
            for native in [false, true] {
                let mut ui = Cockpit::new(theme.clone(), width, height, native);
                let selector = ui.source.selector("folded");
                if native {
                    assert!(
                        ui.bounds(&selector).is_none(),
                        "saved tool output starts collapsed"
                    );
                    ui.click("agent-history-toggle-0");
                }
                let folded = ui.bounds(&selector).unwrap();
                assert!(
                    folded.size.height < gpui::px(180.0),
                    "encoded data must not flood the rendered transcript: {folded:?}"
                );
                let composer = ui.composer();
                assert!(composer.bottom() <= gpui::px(height));
                assert!(composer.left() >= gpui::px(0.0) && composer.right() <= gpui::px(width));
                ui.click(&ui.source.selector("copy"));
                assert_eq!(
                    ui.app.read_from_clipboard().unwrap().text().unwrap(),
                    ui.original
                );
                ui.click(&ui.source.selector("toggle"));
                assert!(
                    ui.view
                        .read_with(&ui.app, |view, _| view.session_ui[&ui.session]
                            .raw_transcripts
                            .contains(&ui.source)),
                    "the rendered toggle must call the production disclosure handler"
                );
                assert!(
                    ui.bounds(&selector).is_none(),
                    "the folded body must be replaced"
                );
                let raw = ui.bounds(&ui.source.selector("raw")).unwrap();
                assert!(raw.size.height > folded.size.height + gpui::px(300.0));
                assert_eq!(
                    ui.composer(),
                    composer,
                    "revealing a large payload cannot move or shrink the composer"
                );
                ui.click(&ui.source.selector("copy"));
                assert_eq!(
                    ui.app.read_from_clipboard().unwrap().text().unwrap(),
                    ui.original
                );
                ui.click(&ui.source.selector("toggle"));
                assert!(ui.bounds(&ui.source.selector("raw")).is_none());
                assert!(ui.bounds(&selector).is_some());
                if native {
                    ui.click("agent-history-toggle-0");
                    assert!(ui.bounds(&selector).is_none());
                }
                assert_eq!(ui.composer(), composer);
                assert!(ui
                    .view
                    .read_with(&ui.app, |view, _| view.run_sessions.is_empty()));
            }
        }
    }
}

// SDTEST-1713 / SDUC-475, SDUC-477
#[::core::prelude::v1::test]
fn rendered_send_stop_controls_share_the_same_compact_footprint() {
    let mut ui = Cockpit::new(ThemePreference::Dark, 620.0, 560.0, false);
    let send = ui
        .bounds(&format!("agent-composer-{}-commit", ui.session))
        .unwrap();
    assert_eq!(send.size, size(gpui::px(28.0), gpui::px(28.0)));
    let run = ui.view.update(&mut ui.app, |view, cx| {
        let request = view
            .sessions
            .begin_run(ui.session, "Continue checking", 5)
            .unwrap();
        let run = request.id;
        view.begin_run(request, cx);
        view.push_stream_event_for(run, AgentStreamEvent::Ready, cx);
        run
    });
    ui.redraw();
    let stop = ui.bounds("agent-stop-control").unwrap();
    assert_eq!(
        stop.size, send.size,
        "Stop must not grow the footer beyond Send's 28px footprint"
    );
    ui.click("agent-stop-control");
    assert_eq!(
        ui.view.read_with(&ui.app, |view, _| view
            .sessions
            .get(ui.session)
            .unwrap()
            .status),
        AgentSessionStatus::Stopping
    );
    assert!(ui
        .view
        .read_with(&ui.app, |view, _| view.run_sessions.contains_key(&run)));
}

// SDTEST-2850 / SDUC-475, SDUC-477 — real wheel events, streamed layout and Jump to latest.
#[::core::prelude::v1::test]
fn rendered_streaming_preserves_reading_position_and_fixed_composer() {
    for (width, height) in [(1280.0, 900.0), (620.0, 560.0)] {
        let mut ui = Cockpit::new(ThemePreference::Dark, width, height, false);
        let run = ui.start();
        ui.view.update(&mut ui.app, |view, cx| {
            view.push_stream_events_for(
                run,
                (0..45).map(|index| {
                    AgentStreamEvent::Text(format!(
                        "Reply {index}\n{}",
                        "A variable-height response.\n".repeat(1 + index % 6)
                    ))
                }),
                cx,
            );
        });
        ui.redraw();
        if !ui.at_end() {
            ui.click("agent-jump-latest");
        }
        assert!(ui.at_end(), "the latest message must be reachable");
        let composer = ui.composer();
        ui.view.update(&mut ui.app, |view, cx| {
            view.push_stream_event_for(
                run,
                AgentStreamEvent::TextDelta("\nStreaming at the bottom.\n".repeat(20)),
                cx,
            )
        });
        ui.redraw();
        assert!(
            ui.at_end(),
            "output follows a reader who stayed at the bottom"
        );
        assert_eq!(ui.composer(), composer);
        ui.wheel(250.0);
        assert!(!ui.at_end(), "a real wheel event must leave the end");
        let before = ui.view.read_with(&ui.app, |view, _| view.scroll.offset());
        ui.view.update(&mut ui.app, |view, cx| {
            view.push_stream_event_for(
                run,
                AgentStreamEvent::TextDelta(
                    "\nArriving while the reader inspects older output.\n".repeat(25),
                ),
                cx,
            )
        });
        ui.redraw();
        let after = ui.view.read_with(&ui.app, |view, _| view.scroll.offset());
        assert_eq!(
            after, before,
            "streaming cannot steal the reader's position"
        );
        assert_eq!(ui.composer(), composer);
        ui.click("agent-jump-latest");
        assert!(ui.at_end());
        ui.view.update(&mut ui.app, |view, cx| {
            view.push_stream_event_for(
                run,
                AgentStreamEvent::TextDelta("\nFollowing again.".repeat(8)),
                cx,
            )
        });
        ui.redraw();
        assert!(ui.at_end(), "explicit return to latest restores following");
        assert_eq!(ui.composer(), composer);
    }
}

// SDTEST-2849 / SDUC-550, SDUC-551, SDUC-564
#[::core::prelude::v1::test]
fn rendered_navigation_preserves_drafts_and_disclosure_between_conversations() {
    let mut ui = Cockpit::new(ThemePreference::Dark, 1280.0, 900.0, false);
    let first = ui.session;
    let first_source = ui.source.clone();
    ui.type_draft("Keep draft A · é🌍");
    ui.click(&first_source.selector("toggle"));
    ui.view.update(&mut ui.app, |view, cx| {
        view.tools.navigator_visible = true;
        cx.notify();
    });
    ui.redraw();
    ui.window.dispatch_action(AgentNewChat);
    ui.redraw();
    let second = ui
        .view
        .read_with(&ui.app, |view, _| view.sessions.selected_id().unwrap());
    assert_ne!(second, first);
    ui.session = second;
    ui.type_draft("Keep draft B");
    ui.click(&format!("agent-nav-session-{first}"));
    ui.session = first;
    assert_eq!(ui.draft(), "Keep draft A · é🌍");
    assert!(ui.bounds(&first_source.selector("raw")).is_some());
    ui.wheel(100000.0);
    ui.click(&first_source.selector("copy"));
    assert_eq!(
        ui.app.read_from_clipboard().unwrap().text().unwrap(),
        ui.original
    );
    ui.click(&format!("agent-nav-session-{second}"));
    ui.session = second;
    assert_eq!(ui.draft(), "Keep draft B");
    assert!(
        ui.bounds(&first_source.selector("raw")).is_none(),
        "another conversation's raw output must leave the frame"
    );
    assert!(ui.bounds(&first_source.selector("copy")).is_none());
    ui.window.dispatch_action(AgentFocusComposer);
    ui.window.simulate_input(" plus a focused edit");
    ui.redraw();
    assert_eq!(ui.draft(), "Keep draft B plus a focused edit");
    ui.click(&format!("agent-nav-session-{first}"));
    ui.session = first;
    assert_eq!(ui.draft(), "Keep draft A · é🌍");
}

// SDTEST-2851 / SDUC-551, SDUC-564
#[::core::prelude::v1::test]
fn rendered_enter_queues_one_followup_and_completion_admits_it_once() {
    let mut ui = Cockpit::new(ThemePreference::Dark, 900.0, 700.0, false);
    let run = ui.start();
    ui.type_draft("A follow-up entered through the actual composer");
    ui.window.simulate_keystrokes("enter");
    ui.redraw();
    assert!(ui.draft().is_empty());
    assert!(
        ui.bounds("agent-queue-edit-0").is_some(),
        "the accepted queue entry must be rendered and editable"
    );
    assert!(!ui
        .events
        .borrow()
        .iter()
        .any(|event| matches!(event, AgentConsoleEvent::Run(_))));
    ui.view.update(&mut ui.app, |view, cx| {
        view.finish_run(run, Some(0), None, cx)
    });
    ui.redraw();
    let requests: Vec<_> = ui
        .events
        .borrow()
        .iter()
        .filter_map(|event| match event {
            AgentConsoleEvent::Run(request) => Some(request.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0].prompt,
        "A follow-up entered through the actual composer"
    );
    assert_ne!(requests[0].id, run);
    ui.view.update(&mut ui.app, |view, cx| {
        view.finish_run(run, Some(0), None, cx)
    });
    ui.redraw();
    assert_eq!(
        ui.events
            .borrow()
            .iter()
            .filter(|event| matches!(event, AgentConsoleEvent::Run(_)))
            .count(),
        1,
        "late completion cannot dispatch the same queued task twice"
    );
    assert!(ui.bounds("agent-queue-edit-0").is_none());
}

// SDTEST-2852 / SDUC-563, SDUC-564
#[::core::prelude::v1::test]
fn rendered_native_history_paging_exposes_retry_and_preserves_the_draft() {
    use shelldeck_core::agent_remote_history::RemoteHistoryPosition;
    fn page(template: &RemoteHistoryPage, start: u64, end: u64) -> RemoteHistoryPage {
        let mut page = template.clone();
        page.snapshot = format!("v1:130:{}", "a".repeat(64));
        page.total_entries = 130;
        page.start = start;
        page.end = end;
        page.entries = (start..end)
            .map(|index| RemoteHistoryEntry {
                index,
                kind: RemoteHistoryKind::Tool,
                title: Some(format!("Saved operation {index}")),
                text: format!("Original result {index}"),
                at_ms: Some(index as i64),
                part: 1,
                parts: 1,
            })
            .collect();
        page
    }
    fn request(ui: &Cockpit) -> NativeHistoryRequest {
        ui.events
            .borrow()
            .iter()
            .rev()
            .find_map(|event| match event {
                AgentConsoleEvent::LoadSshHistory(request) => Some(request.clone()),
                _ => None,
            })
            .expect("The paging control must request history")
    }
    let mut ui = Cockpit::new(ThemePreference::Dark, 900.0, 700.0, true);
    let template = ui.view.read_with(&ui.app, |view, _| {
        view.native_history[&ui.session].page.clone().unwrap()
    });
    ui.view.update(&mut ui.app, |view, cx| {
        view.install_native_history(ui.session, page(&template, 66, 130));
        cx.notify();
    });
    ui.redraw();
    ui.type_draft("Keep this while browsing saved history");
    ui.click("agent-jump-latest");
    ui.click("agent-history-bottom-older");
    let first = request(&ui);
    assert_eq!(first.query.position, RemoteHistoryPosition::Before(66));
    let request_count = ui
        .events
        .borrow()
        .iter()
        .filter(|event| matches!(event, AgentConsoleEvent::LoadSshHistory(_)))
        .count();
    ui.click("agent-history-bottom-older");
    assert_eq!(
        ui.events
            .borrow()
            .iter()
            .filter(|event| matches!(event, AgentConsoleEvent::LoadSshHistory(_)))
            .count(),
        request_count,
        "a loading page cannot launch duplicate reads"
    );
    ui.view.update(&mut ui.app, |view, cx| {
        view.complete_native_history(
            first.clone(),
            Err("Fixture connection failed; retry this page".into()),
            cx,
        )
    });
    ui.redraw();
    assert_eq!(ui.draft(), "Keep this while browsing saved history");
    ui.click("agent-history-retry-bottom");
    let retry = request(&ui);
    assert_ne!(retry.request_id, first.request_id);
    ui.view.update(&mut ui.app, |view, cx| {
        view.complete_native_history(retry, Ok(page(&template, 2, 66)), cx)
    });
    ui.redraw();
    assert!(ui.bounds("agent-history-entry-2").is_some());
    assert!(ui.bounds("agent-history-entry-129").is_none());
    ui.click("agent-history-top-older");
    let older = request(&ui);
    assert_eq!(older.query.position, RemoteHistoryPosition::Before(2));
    ui.view.update(&mut ui.app, |view, cx| {
        view.complete_native_history(older, Ok(page(&template, 0, 2)), cx)
    });
    ui.redraw();
    assert!(ui.bounds("agent-history-entry-0").is_some());
    ui.click("agent-history-top-newer");
    let newer = request(&ui);
    assert_eq!(newer.query.position, RemoteHistoryPosition::After(2));
    ui.view.update(&mut ui.app, |view, cx| {
        view.complete_native_history(newer, Ok(page(&template, 2, 66)), cx)
    });
    ui.redraw();
    ui.click("agent-history-top-latest");
    let latest = request(&ui);
    assert_eq!(latest.query.position, RemoteHistoryPosition::Latest);
    ui.view.update(&mut ui.app, |view, cx| {
        view.complete_native_history(latest, Ok(page(&template, 66, 130)), cx)
    });
    ui.redraw();
    // A delayed failure from the original read cannot replace the newer page.
    ui.view.update(&mut ui.app, |view, cx| {
        view.complete_native_history(first, Err("Stale error".into()), cx)
    });
    ui.redraw();
    assert!(ui.bounds("agent-history-entry-129").is_some());
    assert_eq!(ui.draft(), "Keep this while browsing saved history");
    assert!(!ui.events.borrow().iter().any(|event| matches!(
        event,
        AgentConsoleEvent::Run(_) | AgentConsoleEvent::ApprovalDecision { .. }
    )));
}

// SDTEST-2853 / SDUC-550, SDUC-551, SDUC-564
#[::core::prelude::v1::test]
fn rendered_restart_restores_drafts_without_restoring_raw_disclosure() {
    let mut original = Cockpit::new(ThemePreference::Dark, 900.0, 700.0, false);
    let id = original.session;
    let source = original.source.clone();
    original.type_draft("Unsent draft survives a real store flush");
    original.click(&source.selector("toggle"));
    original.view.update(&mut original.app, |view, cx| {
        view.flush_library(cx).unwrap()
    });
    let store = original
        .view
        .read_with(&original.app, |view, _| view.draft_store());
    let mut restored = Cockpit::new(ThemePreference::Dark, 900.0, 700.0, false);
    restored.view.update(&mut restored.app, |view, cx| {
        let context = view.sessions.selected().unwrap().context.clone();
        view.library = AgentLibraryUi::with_store(cx, store);
        view.sessions = AgentSessionCollection::default();
        view.session_ui.clear();
        let placeholder = view.sessions.create("New chat", context, 1).unwrap();
        view.session_ui.insert(placeholder, AgentSessionUi::new(cx));
        view.apply_selected_context(cx);
        view.set_surface_visible(true, cx);
        view.restore_library(cx);
    });
    restored.redraw();
    restored.session = restored.view.read_with(&restored.app, |view, _| {
        view.sessions.selected_id().unwrap()
    });
    assert_eq!(restored.session, id);
    assert_eq!(restored.draft(), "Unsent draft survives a real store flush");
    assert!(restored.bounds(&source.selector("raw")).is_none());
    restored.wheel(100000.0);
    assert!(restored.bounds(&source.selector("folded")).is_some());
    restored.click(&source.selector("copy"));
    assert_eq!(
        restored.app.read_from_clipboard().unwrap().text().unwrap(),
        original.original
    );
    restored.window.dispatch_action(AgentFocusComposer);
    restored.window.simulate_input(" plus a focused edit");
    restored.redraw();
    assert_eq!(
        restored.draft(),
        "Unsent draft survives a real store flush plus a focused edit"
    );
    assert!(!restored
        .events
        .borrow()
        .iter()
        .any(|event| matches!(event, AgentConsoleEvent::Run(_))));
}

// SDTEST-2854 / SDUC-554 — decisions use rendered controls and survive duplicate clicks.
#[::core::prelude::v1::test]
fn rendered_questions_and_approvals_preserve_drafts_and_submit_once() {
    use shelldeck_core::agent_runtime::{
        AgentApprovalOption, AgentApprovalOptionKind, AgentApprovalRequest, AgentQuestionOption,
        AgentQuestionRequest,
    };
    let mut ui = Cockpit::new(ThemePreference::Dark, 900.0, 700.0, false);
    let run = ui.start();
    ui.type_draft("An unrelated unsent draft");
    ui.view.update(&mut ui.app, |view, cx| {
        view.push_stream_event_for(
            run,
            AgentStreamEvent::Question(AgentQuestionRequest {
                id: "question-1".into(),
                prompt: "Choose the next task".into(),
                options: vec![AgentQuestionOption {
                    label: "Review".into(),
                    description: "Inspect changes".into(),
                }],
                multiple: false,
                response_supported: true,
            }),
            cx,
        )
    });
    ui.redraw();
    ui.wheel(-100000.0);
    assert!(ui.bounds("agent-live-question").is_some());
    ui.click("agent-question-0");
    ui.click("agent-question-0");
    assert_eq!(ui.events.borrow().iter().filter(|event| matches!(event,
        AgentConsoleEvent::QuestionAnswer { run_id, question_id, answer } if *run_id == run && question_id == "question-1" && answer == "Review"
    )).count(), 1);
    ui.view.update(&mut ui.app, |view, cx| {
        view.push_stream_events_for(
            run,
            [
                AgentStreamEvent::UserInputAcknowledged {
                    interaction_id: "question-1".into(),
                },
                AgentStreamEvent::InteractionResolved {
                    id: "question-1".into(),
                },
                AgentStreamEvent::Approval(AgentApprovalRequest {
                    id: "permission-1".into(),
                    session_id: "provider-session".into(),
                    tool_call_id: "tool-1".into(),
                    title: "Allow this command?".into(),
                    options: vec![AgentApprovalOption {
                        id: "once".into(),
                        label: "Allow once".into(),
                        kind: AgentApprovalOptionKind::AllowOnce,
                    }],
                }),
            ],
            cx,
        )
    });
    ui.redraw();
    ui.wheel(-100000.0);
    assert!(ui.bounds("agent-live-question").is_none());
    assert!(ui.bounds("agent-live-approval").is_some());
    ui.click("agent-approval-0");
    ui.click("agent-approval-0");
    ui.click("agent-approval-cancel");
    assert_eq!(ui.events.borrow().iter().filter(|event| matches!(event,
        AgentConsoleEvent::ApprovalDecision { run_id, request_id, .. } if *run_id == run && request_id == "permission-1"
    )).count(), 1, "a decision awaiting acknowledgement must disable all choices");
    assert!(ui.events.borrow().iter().any(|event| matches!(event,
        AgentConsoleEvent::ApprovalDecision { run_id, request_id, option_id: Some(option) } if *run_id == run && request_id == "permission-1" && option == "once"
    )));
    assert_eq!(ui.draft(), "An unrelated unsent draft");
    ui.view.update(&mut ui.app, |view, cx| {
        view.reject_approval_decision(run, "stale-permission", "Old transport error".into(), cx);
        view.reject_approval_decision(Uuid::new_v4(), "permission-1", "Other run".into(), cx);
    });
    ui.redraw();
    ui.click("agent-approval-0");
    assert_eq!(
        ui.events
            .borrow()
            .iter()
            .filter(|event| matches!(event, AgentConsoleEvent::ApprovalDecision { .. }))
            .count(),
        1
    );
    ui.view.update(&mut ui.app, |view, cx| {
        view.reject_approval_decision(run, "permission-1", "Channel closed; try again".into(), cx)
    });
    ui.redraw();
    ui.wheel(-100000.0);
    ui.click("agent-approval-0");
    ui.click("agent-approval-cancel");
    assert_eq!(
        ui.events
            .borrow()
            .iter()
            .filter(|event| matches!(event, AgentConsoleEvent::ApprovalDecision { .. }))
            .count(),
        2,
        "an exact transport rejection permits one explicit retry"
    );
    ui.view.update(&mut ui.app, |view, cx| {
        view.push_stream_event_for(
            run,
            AgentStreamEvent::InteractionResolved {
                id: "permission-1".into(),
            },
            cx,
        )
    });
    ui.redraw();
    assert!(ui.bounds("agent-live-approval").is_none());
    assert_eq!(ui.draft(), "An unrelated unsent draft");
}

// SDTEST-2855 / SDUC-551, SDUC-564 — reselecting a search result must return keyboard focus.
#[::core::prelude::v1::test]
fn rendered_reselecting_the_current_chat_returns_focus_from_search() {
    let mut ui = Cockpit::new(ThemePreference::Dark, 1280.0, 900.0, false);
    ui.type_draft("Keep this draft");
    ui.view.update(&mut ui.app, |view, cx| {
        view.tools.navigator_visible = true;
        cx.notify();
    });
    ui.redraw();
    ui.click("agent-nav-search");
    ui.window.simulate_input("Transcript");
    ui.redraw();
    ui.click(&format!("agent-nav-session-{}", ui.session));
    // No focus shortcut: selecting the result itself must restore typing.
    ui.window.simulate_input(" plus a focused edit");
    ui.redraw();
    assert_eq!(ui.draft(), "Keep this draft plus a focused edit");
    assert_eq!(
        ui.view.read_with(&ui.app, |view, cx| view
            .navigator_search
            .read(cx)
            .content()
            .to_owned()),
        "Transcript"
    );
    assert!(!ui
        .events
        .borrow()
        .iter()
        .any(|event| matches!(event, AgentConsoleEvent::Run(_))));
}

// SDTEST-2738 / SDUC-499 — actual missing-target recovery, filters and Send readiness.
#[::core::prelude::v1::test]
fn rendered_missing_ssh_keeps_drafts_and_navigation_until_explicit_repair() {
    use shelldeck_core::config::app_config::UiLanguage;
    fn replace_input(ui: &mut Cockpit, selector: &str, text: &str) {
        ui.click(selector);
        ui.window.simulate_keystrokes(if cfg!(target_os = "macos") {
            "cmd-a"
        } else {
            "ctrl-a"
        });
        ui.window.simulate_keystrokes("backspace");
        ui.window.simulate_input(text);
        ui.redraw();
    }
    for language in [UiLanguage::En, UiLanguage::Fr] {
        for theme in [ThemePreference::Dark, ThemePreference::SolarizedLight] {
            for (width, height) in [(900.0, 700.0), (620.0, 560.0)] {
                let mut ui = Cockpit::new(theme.clone(), width, height, false);
                crate::apply_ui_language(&language);
                let remote = ui.session;
                let missing = Uuid::new_v4();
                let replacement = Uuid::new_v4();
                let local = ui.view.update(&mut ui.app, |view, cx| {
                    let context = view.sessions.selected().unwrap().context.clone();
                    let local = view
                        .create_session("Local check", context.clone(), cx)
                        .unwrap();
                    let mut remote_context = context;
                    remote_context.target = AgentTarget::Ssh {
                        connection_id: missing,
                        connection_label: "Removed server".into(),
                    };
                    remote_context.workdir = "/srv/customer project".into();
                    let session = view.sessions.get_mut(remote).unwrap();
                    session.rename("Remote repair", 6).unwrap();
                    session.set_context(remote_context, 7).unwrap();
                    view.connections = vec![AgentConnectionOption {
                        id: replacement,
                        label: "Replacement server".into(),
                        host: "replacement.invalid".into(),
                    }];
                    view.select_session(remote, cx);
                    local
                });
                ui.redraw();
                assert!(ui.bounds("agent-project-trigger").is_some());
                assert!(ui.bounds("agent-reconnect-trigger").is_some());
                ui.type_draft("Keep my remote draft · é🌍");
                let commit = format!("agent-composer-{remote}-commit");
                ui.click(&commit);
                ui.window.simulate_keystrokes("enter");
                ui.redraw();
                assert!(ui
                    .events
                    .borrow()
                    .iter()
                    .all(|event| !matches!(event, AgentConsoleEvent::Run(_))));
                // Disabled Send leaves the multiline editor available: Enter
                // inserts a newline rather than starting a missing target.
                assert_eq!(ui.draft(), "Keep my remote draft · é🌍\n");

                ui.click("agent-project-trigger");
                replace_input(&mut ui, "agent-project-search", "No matching project 971");
                assert!(ui.bounds("agent-project-no-matches").is_some());
                assert!(ui
                    .bounds(&format!("agent-project-session-{local}"))
                    .is_none());
                replace_input(&mut ui, "agent-project-search", "Local check");
                assert!(ui.bounds("agent-project-no-matches").is_none());
                ui.click(&format!("agent-project-session-{local}"));
                ui.session = local;
                assert!(ui.draft().is_empty());
                ui.click(&format!("agent-composer-{local}-commit"));
                assert!(ui
                    .events
                    .borrow()
                    .iter()
                    .all(|event| !matches!(event, AgentConsoleEvent::Run(_))));
                ui.type_draft("A local draft stays separate");

                ui.click("agent-project-trigger");
                ui.click(&format!("agent-project-session-{remote}"));
                ui.session = remote;
                assert_eq!(ui.draft(), "Keep my remote draft · é🌍\n");
                ui.click("agent-reconnect-trigger");
                replace_input(&mut ui, "agent-reconnect-search", "No matching server 971");
                assert!(ui.bounds("agent-reconnect-no-matches").is_some());
                replace_input(&mut ui, "agent-reconnect-search", "Replacement");
                assert!(ui.bounds("agent-reconnect-no-matches").is_none());
                ui.click(&format!("agent-reconnect-None-{replacement}"));
                assert!(ui.bounds("agent-reconnect-trigger").is_none());
                assert_eq!(ui.draft(), "Keep my remote draft · é🌍\n");
                ui.click(&commit);
                let requests = ui
                    .events
                    .borrow()
                    .iter()
                    .filter_map(|event| match event {
                        AgentConsoleEvent::Run(request) => Some(request.clone()),
                        _ => None,
                    })
                    .collect::<Vec<_>>();
                assert_eq!(
                    requests.len(),
                    1,
                    "typing and explicit repair must refresh Send immediately"
                );
                assert_eq!(requests[0].prompt, "Keep my remote draft · é🌍");
                assert_eq!(requests[0].workdir, "/srv/customer project");
                assert!(
                    matches!(&requests[0].target, AgentTarget::Ssh { connection_id, .. } if *connection_id == replacement)
                );
                assert_eq!(
                    ui.view
                        .read_with(&ui.app, |view, cx| view.session_ui[&local]
                            .prompt
                            .read(cx)
                            .content()
                            .to_owned()),
                    "A local draft stays separate"
                );
            }
        }
    }
}
