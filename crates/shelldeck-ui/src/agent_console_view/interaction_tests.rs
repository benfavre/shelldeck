//! Render the production cockpit and dispatch input through its actual controls.
//! The GPUI test platform owns the clipboard, clock and window; providers never run.
use super::*;
use shelldeck_core::agent_remote_history::{
    RemoteHistoryEntry, RemoteHistoryKind, RemoteHistoryPage, RemoteHistoryPosition,
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
        self.wheel_in("agent-timeline-scroll", amount);
    }

    fn wheel_in(&mut self, selector: &str, amount: f32) {
        let position = self.bounds(selector).unwrap().center();
        self.window
            .simulate_mouse_move(position, None, Modifiers::none());
        self.window.simulate_event(ScrollWheelEvent {
            position,
            delta: ScrollDelta::Pixels(point(gpui::px(0.0), gpui::px(amount))),
            touch_phase: TouchPhase::Moved,
            ..Default::default()
        });
        // GPUI's 70 ms smooth scroll uses std::time::Instant rather than the
        // test dispatcher clock. Let that animation settle before measuring.
        std::thread::sleep(std::time::Duration::from_millis(80));
        self.redraw();
    }

    fn open_navigator(&mut self) {
        self.view.update(&mut self.app, |view, cx| {
            view.tools.navigator_visible = true;
            view.tools.navigator_overlay = true;
            cx.notify();
        });
        self.redraw();
    }

    fn search_navigator(&mut self, query: &str) {
        self.click("agent-nav-search");
        self.window
            .simulate_keystrokes(if cfg!(target_os = "macos") {
                "cmd-a"
            } else {
                "ctrl-a"
            });
        self.window.simulate_keystrokes("backspace");
        self.window.simulate_input(query);
        self.redraw();
    }

    fn click_navigation(&mut self, selector: &str) {
        for _ in 0..4 {
            let row = self
                .bounds(selector)
                .unwrap_or_else(|| panic!("Missing navigation row: {selector}"));
            let viewport = self.bounds("agent-navigator-scroll").unwrap();
            if viewport.contains(&row.center()) {
                self.click(selector);
                return;
            }
            self.wheel_in(
                "agent-navigator-scroll",
                (viewport.center().y - row.center().y).to_f64() as f32,
            );
        }
        panic!("Navigation control is unreachable: {selector}");
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
        self.view
            .read_with(&self.app, |view, _| view.timeline.at_end(&view.scroll))
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
        let before = ui.view.read_with(&ui.app, |view, _| {
            let top = view.timeline.list.logical_scroll_top();
            (view.timeline.rows[top.item_ix].id, top.offset_in_item)
        });
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
        let after = ui.view.read_with(&ui.app, |view, _| {
            let top = view.timeline.list.logical_scroll_top();
            (view.timeline.rows[top.item_ix].id, top.offset_in_item)
        });
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
    ui.wheel(240.0);
    assert!(!ui.at_end());
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
    assert!(ui.bounds("agent-history-entry-66").is_some());
    ui.wheel(-100000.0);
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

// SDTEST-2857 / SDUC-562, SDUC-563 — reproduce discovery -> Reconnect with a 240-byte title.
#[::core::prelude::v1::test]
fn rendered_long_native_titles_reconnect_and_preserve_existing_drafts() {
    use shelldeck_core::agent_remote_sessions::parse_inventory;
    use shelldeck_core::agent_session::MAX_AGENT_SESSION_NAME_BYTES;
    use shelldeck_core::config::app_config::UiLanguage;
    for provider in [
        AgentProvider::Claude,
        AgentProvider::Codex,
        AgentProvider::Jcode,
    ] {
        for (width, height) in [(900.0, 700.0), (620.0, 560.0)] {
            let mut ui = Cockpit::new(ThemePreference::Dark, width, height, false);
            crate::apply_ui_language(&UiLanguage::Fr);
            let local = ui.session;
            ui.type_draft("Keep the outgoing local draft");
            let title = "Site : plante-artificielle-unique.com Thème : Rapport · visibilité locale à Rennes Date des faits : 2026-10-30 Chiffres de tête : - Ville : Rennes - Mots-clés relevés depuis Rennes : 2 visibles sur 20 mesurés, couverture 10 %, top 3";
            let wire = serde_json::json!({"version":1,"sessions":[{
                "provider":provider,"session_id":"80582780-28b5-433c-a51c-afdac90e0baa",
                "workdir":"/home/infra-sj278/.cache/seo-reading-cwd", "title":title,
                "updated_at_ms":1793360000000_u64,"model":null
            }],"warnings":[],"truncated":false});
            let remote = parse_inventory(&wire.to_string())
                .unwrap()
                .sessions
                .remove(0);
            let connection = Uuid::new_v4();
            let preview = |ui: &mut Cockpit| {
                ui.view.update(&mut ui.app, |view, cx| {
                    view.set_connections(
                        vec![AgentConnectionOption {
                            id: connection,
                            label: "Original server".into(),
                            host: "fixture.invalid".into(),
                        }],
                        cx,
                    );
                    view.ssh_sessions.preview = Some((connection, remote.clone()));
                    view.sessions.set_surface_visible(false);
                    cx.notify();
                });
                ui.redraw();
                // Scroll the actual preview owner, including when the full title wraps.
                let position = ui.bounds("agent-ssh-preview").unwrap().center();
                ui.window.simulate_event(ScrollWheelEvent {
                    position,
                    delta: ScrollDelta::Pixels(point(gpui::px(0.0), gpui::px(-100000.0))),
                    touch_phase: TouchPhase::Moved,
                    ..Default::default()
                });
                ui.redraw();
            };
            let page = RemoteHistoryPage {
                provider,
                session_id: remote.session_id.clone(),
                workdir: remote.workdir.clone(),
                model: None,
                snapshot: format!("v1:1:{}", "a".repeat(64)),
                total_entries: 1,
                start: 0,
                end: 1,
                entries: vec![RemoteHistoryEntry {
                    index: 0,
                    kind: RemoteHistoryKind::Assistant,
                    title: None,
                    text: "Complete saved native reply".into(),
                    at_ms: Some(1),
                    part: 1,
                    parts: 1,
                }],
                warnings: vec![],
            };
            let mut imported = None;
            for attempt in 0..2 {
                preview(&mut ui);
                ui.click("agent-ssh-reconnect");
                let request_id = ui
                    .events
                    .borrow()
                    .iter()
                    .rev()
                    .find_map(|event| match event {
                        AgentConsoleEvent::ReconnectSshSession {
                            connection_id,
                            request_id,
                            session,
                        } => {
                            assert_eq!(*connection_id, connection);
                            assert_eq!(
                                session, &remote,
                                "discovery retains the complete title and original authority"
                            );
                            Some(*request_id)
                        }
                        _ => None,
                    })
                    .unwrap();
                ui.view.update(&mut ui.app, |view, cx| {
                    view.complete_ssh_reconnect(request_id, Ok(page.clone()), cx)
                });
                ui.redraw();
                assert!(
                    ui.bounds("agent-ssh-preview").is_none(),
                    "reconnect must leave the preview without a name-length error"
                );
                let id = ui
                    .view
                    .read_with(&ui.app, |view, _| view.sessions.selected_id().unwrap());
                if let Some(expected) = imported {
                    assert_eq!(id, expected);
                } else {
                    imported = Some(id);
                }
                ui.session = id;
                ui.view.read_with(&ui.app, |view, cx| {
                    let session = view.sessions.get(id).unwrap();
                    assert!(session.name.len() <= MAX_AGENT_SESSION_NAME_BYTES);
                    assert!(session.name.ends_with('…'));
                    let request = session.run_request("Continue this native conversation");
                    assert_eq!(request.resume_session.as_deref(),Some(remote.session_id.as_str()));
                    assert_eq!(request.provider,provider);
                    assert_eq!(request.workdir,remote.workdir);
                    assert!(matches!(request.target,AgentTarget::Ssh {connection_id,..} if connection_id==connection));
                    assert_eq!(request.access,AgentAccessMode::ReadOnly);
                    assert_eq!(view.native_history[&id].page.as_ref().unwrap(),&page);
                    assert_eq!(view.session_ui[&local].prompt.read(cx).content(),"Keep the outgoing local draft");
                    assert_eq!(view.sessions.active_count(),0);
                });
                if attempt == 0 {
                    ui.type_draft("Preserve my native draft");
                } else {
                    assert_eq!(ui.draft(), "Preserve my native draft");
                }
                assert!(!ui
                    .events
                    .borrow()
                    .iter()
                    .any(|event| matches!(event, AgentConsoleEvent::Run(_))));
            }
        }
    }
}

// SDTEST-2858 / SDUC-475, SDUC-564 — technical output must retain its natural height.
#[::core::prelude::v1::test]
fn rendered_warning_cards_and_expanded_work_do_not_crop_output() {
    use shelldeck_core::agent_session::AgentTraceUpdate;
    for theme in [ThemePreference::Dark, ThemePreference::SolarizedLight] {
        for (width, height) in [(900.0, 700.0), (620.0, 560.0), (900.0, 400.0)] {
            for with_output in [false, true] {
                let mut ui = Cockpit::new(theme.clone(), width, height, false);
                let run = ui.start();
                ui.view.update(&mut ui.app,|view,cx| view.push_stream_events_for(run,[
                AgentStreamEvent::Trace(AgentTraceUpdate {correlation_id:Some("curl".into()),detail:AgentTraceKind::Command {
                    command:"curl -sS -o /dev/null -w 'home: %{http_code} %{time_total}s %{size_download}B via %{remote_ip}' -L https://inklura.fr".into(),
                    status:AgentTraceStatus::Succeeded, exit_code:Some(0), summary:Some("home: 200 0.41s 18420B\nNavigation complete".into())
                }}),
                AgentStreamEvent::Trace(AgentTraceUpdate {correlation_id:Some("browser-warning".into()),detail:AgentTraceKind::Command {
                    command:format!("/home/fixture/.cache/ms-playwright/chromium-{}/chrome-linux64/chrome: browser process warning — inspect retained stderr and retry the original operation", "152.0.7977.75".repeat(3)),
                    status:AgentTraceStatus::Running, exit_code:None,
                    summary:with_output.then(|| "Browser launch warning:\nFirst retained output line.\nSecond retained output line.\nFinal retained output line · é🌍".into())
                }}),
                AgentStreamEvent::Usage(shelldeck_core::agent_runtime::AgentUsageUpdate { input_tokens:Some(18420), output_tokens:Some(3), ..Default::default() }),
            ],cx));
                ui.redraw();
                let (completed, warning) = ui.view.read_with(&ui.app, |view, _| {
                    let traces = &view.sessions.get(ui.session).unwrap().trace;
                    (
                        traces
                            .iter()
                            .find(|trace| trace.correlation_id.as_deref() == Some("curl"))
                            .unwrap()
                            .id,
                        traces
                            .iter()
                            .find(|trace| {
                                trace.correlation_id.as_deref() == Some("browser-warning")
                            })
                            .unwrap()
                            .id,
                    )
                });
                let composer = ui.composer();
                for expanded in [false, true] {
                    if expanded {
                        let selector = format!("agent-work-{completed}");
                        ui.wheel(100000.0);
                        for _ in 0..8 {
                            let Some(control) = ui.bounds(&selector) else {
                                ui.wheel(-100.0);
                                continue;
                            };
                            let viewport = ui.bounds("agent-timeline-scroll").unwrap();
                            if viewport.contains(&control.center()) {
                                break;
                            }
                            ui.wheel((viewport.center().y - control.center().y).to_f64() as f32);
                        }
                        let control = ui.bounds(&selector).unwrap();
                        let viewport = ui.bounds("agent-timeline-scroll").unwrap();
                        assert!(viewport.contains(&control.center()),"{width}x{height}, output={with_output}: control={control:?}, viewport={viewport:?}");
                        ui.click(&selector);
                    }
                    // Expanded work can place the footer outside the virtual
                    // viewport. Reach the warning and footer as a reader does.
                    ui.wheel(-100000.0);
                    let card = ui.bounds(&format!("agent-trace-{warning}")).unwrap();
                    let content = ui
                        .bounds(&format!("agent-trace-{warning}-content"))
                        .unwrap();
                    let title = ui.bounds(&format!("agent-trace-{warning}-title")).unwrap();
                    let body = ui.bounds(&TranscriptId::Trace(warning).selector("folded"));
                    let icon = ui.bounds(&format!("agent-trace-{warning}-icon")).unwrap();
                    let preparing = ui.bounds("agent-preparing-output").unwrap();
                    let progress = ui.bounds("agent-run-progress").unwrap();
                    assert!(
                        card.bottom() <= preparing.top(),
                        "warning output cannot overlap the preparing indicator"
                    );
                    assert!(
                        preparing.bottom() <= progress.top(),
                        "run details must remain below the preparing indicator"
                    );
                    assert!(
                        progress.size.height >= gpui::px(36.0),
                        "the run-details disclosure must keep its natural height"
                    );
                    assert!(
                        card.size.height >= gpui::px(36.0),
                        "a trace cannot shrink below its icon plus padding: {card:?}"
                    );
                    for child in [Some(content), Some(title), body, Some(icon)]
                        .into_iter()
                        .flatten()
                    {
                        assert!(
                            child.top() >= card.top() && child.bottom() <= card.bottom(),
                            "trace child is vertically clipped: card={card:?}, child={child:?}"
                        );
                        assert!(
                            child.left() >= card.left() && child.right() <= card.right(),
                            "trace child is horizontally clipped: card={card:?}, child={child:?}"
                        );
                    }
                    if with_output {
                        let body = body.unwrap();
                        assert!(
                            body.size.height >= gpui::px(52.0),
                            "all four literal output lines must have room: {body:?}"
                        );
                    }
                    assert_eq!(
                        ui.composer(),
                        composer,
                        "technical output must scroll independently of the composer"
                    );
                    ui.wheel(-100000.0);
                    let card = ui.bounds(&format!("agent-trace-{warning}")).unwrap();
                    let viewport = ui.bounds("agent-timeline-scroll").unwrap();
                    assert!(
                        card.bottom() <= viewport.bottom(),
                        "the bottom of the complete warning must be reachable by scrolling"
                    );
                }
            }
        }
    }
}

// SDTEST-2860 / SDUC-551, SDUC-564 — closed folders retain the open chat and pins stay above projects.

// SDTEST-2868 — the collapsed command preview never paints a multiline shell script beside its label.
#[::core::prelude::v1::test]
fn rendered_completed_operation_previews_keep_multiline_commands_readable() {
    use shelldeck_core::agent_session::AgentTraceUpdate;
    for theme in [ThemePreference::Dark, ThemePreference::SolarizedLight] {
        for (width, rem) in [(900.0, 16.0), (620.0, 20.0)] {
            let mut ui = Cockpit::new(theme.clone(), width, 700.0, false);
            let run = ui.start();
            let command="/bin/bash -lc \"python3 - <<'PY'\nfrom pathlib import Path\nfrom collections import Counter, defaultdict\nroot=Path('/srv/a project/é🌍')\nprint(root)\nPY\"";
            ui.view.update(&mut ui.app, |view, cx| {
                view.push_stream_event_for(
                    run,
                    AgentStreamEvent::Trace(AgentTraceUpdate {
                        correlation_id: Some("multiline".into()),
                        detail: AgentTraceKind::Command {
                            command: command.into(),
                            status: AgentTraceStatus::Succeeded,
                            exit_code: Some(0),
                            summary: Some("Retained output".into()),
                        },
                    }),
                    cx,
                )
            });
            ui.window
                .update(|window, _| window.set_rem_size(gpui::px(rem)));
            ui.redraw();
            ui.wheel(-100000.0);
            let id = ui.view.read_with(&ui.app, |view, _| {
                view.sessions
                    .get(ui.session)
                    .unwrap()
                    .trace
                    .last()
                    .unwrap()
                    .id
            });
            let preview = ui.bounds(&format!("agent-work-preview-{id}")).unwrap();
            let control = ui.bounds(&format!("agent-work-{id}")).unwrap();
            assert!(
                control.bottom() <= preview.top(),
                "preview is beneath the disclosure"
            );
            assert!(
                preview.size.height <= gpui::px(28.0 * rem / 16.0),
                "collapsed shell preview must occupy one line: {preview:?}"
            );
            assert!(preview.right() <= ui.composer().right() + gpui::px(24.0));
            ui.click(&format!("agent-work-{id}"));
            let title = ui.bounds(&format!("agent-trace-{id}-title")).unwrap();
            assert!(
                title.size.height > preview.size.height,
                "expansion retains every command line"
            );
            ui.view.read_with(&ui.app,|view,_|assert!(matches!(&view.sessions.get(ui.session).unwrap().trace.last().unwrap().detail,AgentTraceKind::Command {command:actual,..} if actual==command)));
        }
    }
}

// SDTEST-2869 — real scroll and floating collapse controls keep the reader in the same chat.
#[::core::prelude::v1::test]
fn rendered_floating_operation_collapse_returns_to_the_matching_summary() {
    use shelldeck_core::agent_session::AgentTraceUpdate;
    for theme in [ThemePreference::Dark, ThemePreference::SolarizedLight] {
        let mut ui = Cockpit::new(theme, 760.0, 650.0, false);
        let run = ui.start();
        ui.type_draft("Keep my next instruction");
        ui.view.update(&mut ui.app, |view, cx| {
            let updates = (0..6).map(|i| {
                AgentStreamEvent::Trace(AgentTraceUpdate {
                    correlation_id: Some(format!("long-{i}")),
                    detail: AgentTraceKind::Command {
                        command: format!("inspect-operation-{i}"),
                        status: AgentTraceStatus::Succeeded,
                        exit_code: Some(0),
                        summary: Some(
                            (0..30)
                                .map(|line| format!("Operation {i} retained output line {line}\n"))
                                .collect::<String>(),
                        ),
                    },
                })
            });
            view.push_stream_events_for(run, updates, cx);
        });
        ui.redraw();
        let id = ui.view.read_with(&ui.app, |view, _| {
            view.sessions
                .get(ui.session)
                .unwrap()
                .trace
                .iter()
                .find(|trace| trace.correlation_id.as_deref() == Some("long-0"))
                .unwrap()
                .id
        });
        let composer = ui.composer();
        ui.click(&format!("agent-work-{id}"));
        assert!(
            ui.bounds("agent-collapse-floating").is_none(),
            "the inline header is still visible"
        );
        ui.wheel(-420.0);
        let floating = ui
            .bounds("agent-collapse-floating")
            .expect("floating collapse is reachable after the header scrolls away");
        let viewport = ui.bounds("agent-timeline-scroll").unwrap();
        assert!(viewport.contains(&floating.center()));
        assert!(
            floating.bottom() < composer.top(),
            "the floating action never covers the composer"
        );
        let before = ui.view.read_with(&ui.app, |view, _| view.scroll.offset());
        ui.view.update(&mut ui.app, |view, cx| {
            view.push_stream_event_for(
                run,
                AgentStreamEvent::Text("A new response arrives below while you read.".into()),
                cx,
            )
        });
        ui.redraw();
        assert_eq!(
            ui.view.read_with(&ui.app, |view, _| view.scroll.offset()),
            before,
            "incoming work must not steal the reading position"
        );
        ui.click("agent-collapse-floating");
        assert!(ui.bounds("agent-collapse-floating").is_none());
        let header = ui.bounds(&format!("agent-work-{id}")).unwrap();
        assert!(
            ui.bounds("agent-timeline-scroll")
                .unwrap()
                .contains(&header.center()),
            "collapse returns to this group's summary"
        );
        assert_eq!(ui.composer(), composer);
        assert_eq!(ui.draft(), "Keep my next instruction");
        ui.view.read_with(&ui.app, |view, _| {
            assert_eq!(view.sessions.selected_id(), Some(ui.session));
            assert!(view.session_ui[&ui.session].expanded_activity.is_empty());
            assert_eq!(
                view.sessions
                    .get(ui.session)
                    .unwrap()
                    .trace
                    .iter()
                    .filter(|trace| trace
                        .correlation_id
                        .as_deref()
                        .is_some_and(|key| key.starts_with("long-")))
                    .count(),
                6
            );
        });
    }
}

#[::core::prelude::v1::test]
fn rendered_project_folders_keep_pins_current_chat_search_and_drafts() {
    use super::navigation_groups::directory_group_key;
    use shelldeck_core::config::app_config::UiLanguage;
    for (theme, language) in [
        (ThemePreference::Dark, UiLanguage::En),
        (ThemePreference::SolarizedLight, UiLanguage::Fr),
    ] {
        for (width, height) in [(900.0, 700.0), (620.0, 560.0)] {
            let mut ui = Cockpit::new(theme.clone(), width, height, false);
            crate::apply_ui_language(&language);
            let first = ui.session;
            ui.type_draft("Keep the selected project's draft · é🌍");
            let (background, pinned, other, folder_key) =
                ui.view.update(&mut ui.app, |view, cx| {
                    let context = view.sessions.get(first).unwrap().context.clone();
                    view.set_project_groups(
                        vec![AgentProjectGroup {
                            id: "catalog".into(),
                            label: "Customer project".into(),
                            worktrees: vec![AgentWorktreeOption {
                                id: "primary".into(),
                                label: "Customer project".into(),
                                path: context.workdir.clone(),
                                target: context.target.clone(),
                                branch: if matches!(&theme, ThemePreference::SolarizedLight) {
                                    Some("main".into())
                                } else {
                                    None
                                },
                                is_primary: true,
                            }],
                        }],
                        cx,
                    );
                    let background = view
                        .create_session("Background approval", context.clone(), cx)
                        .unwrap();
                    view.sessions
                        .begin_run(background, "Wait for a decision", 20)
                        .unwrap();
                    view.sessions
                        .apply_stream_event(
                            background,
                            AgentStreamEvent::Question(
                                shelldeck_core::agent_runtime::AgentQuestionRequest {
                                    id: "which".into(),
                                    prompt: "Which destination?".into(),
                                    options: vec![],
                                    multiple: false,
                                    response_supported: false,
                                },
                            ),
                            21,
                        )
                        .unwrap();
                    let pinned = view
                        .create_session("Pinned task", context.clone(), cx)
                        .unwrap();
                    view.sessions.set_pinned(pinned, true, 22).unwrap();
                    let mut elsewhere = context;
                    elsewhere.workdir = "/tmp/shelldeck-uncatalogued-project".into();
                    let key =
                        directory_group_key("chat-folder", &elsewhere.target, &elsewhere.workdir);
                    let other = view
                        .create_session("Another folder", elsewhere, cx)
                        .unwrap();
                    view.select_session(first, cx);
                    (background, pinned, other, key)
                });
            ui.open_navigator();
            let guide = ui.bounds("agent-tree-children-catalog").unwrap();
            let parent = ui.bounds("agent-project-collapse-catalog").unwrap();
            let child = ui.bounds(&format!("agent-nav-session-{first}")).unwrap();
            assert_eq!(
                guide.left(),
                parent.center().x,
                "the guide aligns beneath its parent chevron"
            );
            assert!(
                child.left() >= guide.left() + gpui::px(6.0) && child.right() <= guide.right(),
                "conversation controls stay inset and contained beside the guide"
            );
            if matches!(&theme, ThemePreference::SolarizedLight) {
                let nested = ui.bounds("agent-tree-children-worktree-primary").unwrap();
                assert!(
                    nested.left() > guide.left() && child.left() >= nested.left() + gpui::px(6.0),
                    "worktree conversations keep a second hierarchy level"
                );
            }
            assert!(
                ui.bounds(&format!("agent-nav-session-{pinned}"))
                    .unwrap()
                    .top()
                    < ui.bounds("agent-project-collapse-catalog").unwrap().top()
            );
            assert!(ui.bounds(&format!("agent-folder-{folder_key}")).is_some());
            ui.click_navigation("agent-project-collapse-catalog");
            assert!(
                ui.bounds(&format!("agent-nav-session-{first}")).is_some(),
                "the current chat remains in its closed project"
            );
            assert!(ui
                .bounds(&format!("agent-nav-session-{background}"))
                .is_none());
            assert!(
                ui.bounds("agent-group-summary-catalog").is_some(),
                "hidden pending work is summarized"
            );
            ui.click_navigation(&format!("agent-nav-session-{pinned}"));
            ui.session = pinned;
            ui.type_draft("A separate pinned draft");
            ui.open_navigator();
            ui.click_navigation(&format!("agent-nav-menu-{pinned}"));
            ui.click(&format!("agent-nav-action-{pinned}-TogglePin"));
            assert!(ui.bounds("agent-folder-pinned-sessions").is_none());
            assert!(
                ui.bounds(&format!("agent-nav-session-{pinned}")).is_some(),
                "unpinning keeps the open chat visible in its closed project"
            );
            assert_eq!(ui.draft(), "A separate pinned draft");
            ui.click_navigation(&format!("agent-nav-menu-{pinned}"));
            ui.click(&format!("agent-nav-action-{pinned}-TogglePin"));
            assert!(ui.bounds("agent-folder-pinned-sessions").is_some());
            assert!(ui.bounds(&format!("agent-nav-session-{first}")).is_none());
            let saved = ui
                .view
                .read_with(&ui.app, |view, _| view.library.preferences.clone());
            ui.search_navigator("background approval");
            assert!(ui
                .bounds(&format!("agent-nav-session-{background}"))
                .is_some());
            assert!(ui.bounds(&format!("agent-nav-session-{other}")).is_none());
            assert_eq!(
                ui.view
                    .read_with(&ui.app, |view, _| view.library.preferences.clone()),
                saved
            );
            ui.search_navigator("Transcript");
            ui.click_navigation(&format!("agent-nav-session-{first}"));
            ui.session = first;
            assert_eq!(ui.draft(), "Keep the selected project's draft · é🌍");
            ui.open_navigator();
            ui.search_navigator("");
            assert!(ui.bounds(&format!("agent-nav-session-{first}")).is_some());
            assert!(ui
                .bounds(&format!("agent-nav-session-{background}"))
                .is_none());
            ui.click_navigation(&format!("agent-folder-toggle-{folder_key}"));
            assert!(ui.bounds(&format!("agent-nav-session-{other}")).is_none());
            let loaded = ui.view.update(&mut ui.app, |view, cx| {
                view.flush_library(cx).unwrap();
                view.draft_store().load(100).unwrap()
            });
            assert!(loaded
                .preferences
                .collapsed_project_ids
                .contains(&"catalog".to_owned()));
            assert!(loaded
                .preferences
                .collapsed_project_ids
                .contains(&folder_key));
            assert_eq!(
                loaded.drafts[&first],
                "Keep the selected project's draft · é🌍"
            );
            assert_eq!(loaded.drafts[&pinned], "A separate pinned draft");
            // A legacy closed catch-all becomes closed folders on an actual store restore.
            let store = ui.view.read_with(&ui.app, |view, _| view.draft_store());
            let catalog = ui
                .view
                .read_with(&ui.app, |view, _| view.project_groups.clone());
            let mut legacy = loaded;
            legacy
                .preferences
                .collapsed_project_ids
                .retain(|id| id != &folder_key);
            legacy
                .preferences
                .collapsed_project_ids
                .push("unassigned-sessions".into());
            store.save(&legacy).unwrap();
            let mut restored = Cockpit::new(theme.clone(), width, height, false);
            restored.view.update(&mut restored.app, |view, cx| {
                let context = view.sessions.selected().unwrap().context.clone();
                view.library = AgentLibraryUi::with_store(cx, store);
                view.sessions = AgentSessionCollection::default();
                view.session_ui.clear();
                let placeholder = view.sessions.create("New chat", context, 1).unwrap();
                view.session_ui.insert(placeholder, AgentSessionUi::new(cx));
                view.set_project_groups(catalog, cx);
                view.apply_selected_context(cx);
                view.restore_library(cx);
            });
            restored.redraw();
            restored.session = first;
            restored.open_navigator();
            assert_eq!(restored.draft(), "Keep the selected project's draft · é🌍");
            assert!(restored
                .bounds(&format!("agent-nav-session-{first}"))
                .is_some());
            assert!(restored
                .bounds(&format!("agent-nav-session-{other}"))
                .is_none());
            restored.view.read_with(&restored.app, |view, _| {
                assert!(view
                    .library
                    .preferences
                    .collapsed_project_ids
                    .contains(&folder_key));
                assert!(!view
                    .library
                    .preferences
                    .collapsed_project_ids
                    .contains(&"unassigned-sessions".to_owned()));
            });
            assert!(!ui
                .events
                .borrow()
                .iter()
                .any(|event| matches!(event, AgentConsoleEvent::Run(_))));
        }
    }
}

// SDTEST-2861 / SDUC-562, SDUC-563 — SSH folders combine providers without combining machines or authority.
#[::core::prelude::v1::test]
fn rendered_ssh_folders_preserve_source_search_collapse_and_independent_pages() {
    use super::navigation_groups::directory_group_key;
    use shelldeck_core::agent_remote_sessions::{RemoteAgentSession, RemoteAgentSessionInventory};
    for (width, height) in [(900.0, 700.0), (620.0, 560.0)] {
        let mut ui = Cockpit::new(ThemePreference::Dark, width, height, false);
        ui.type_draft("Keep my local draft while browsing remote projects");
        let a = Uuid::from_u128(501);
        let b = Uuid::from_u128(502);
        let remote = |index: usize, path: &str| RemoteAgentSession {
            provider: [
                AgentProvider::Claude,
                AgentProvider::Codex,
                AgentProvider::Jcode,
            ][index % 3],
            session_id: format!("native-{index}"),
            workdir: path.into(),
            title: format!("Saved task {index}"),
            updated_at_ms: index as u64,
            model: None,
        };
        let alpha = (0..12)
            .map(|index| remote(index, "/srv/customer/api"))
            .collect::<Vec<_>>();
        let beta = (12..23)
            .map(|index| remote(index, "/srv/other/api"))
            .collect::<Vec<_>>();
        ui.view.update(&mut ui.app, |view, cx| {
            view.set_connections(
                vec![
                    AgentConnectionOption {
                        id: a,
                        label: "Server A".into(),
                        host: "a.invalid".into(),
                    },
                    AgentConnectionOption {
                        id: b,
                        label: "Server B".into(),
                        host: "b.invalid".into(),
                    },
                ],
                cx,
            );
            view.scan_ssh_sessions(a, cx);
            view.scan_ssh_sessions(b, cx);
        });
        let requests = ui
            .events
            .borrow()
            .iter()
            .filter_map(|event| match event {
                AgentConsoleEvent::DiscoverSshSessions {
                    connection_id,
                    request_id,
                    ..
                } => Some((*connection_id, *request_id)),
                _ => None,
            })
            .collect::<Vec<_>>();
        ui.view.update(&mut ui.app, |view, cx| {
            for (id, request) in requests {
                view.set_ssh_sessions(
                    id,
                    request,
                    Ok(RemoteAgentSessionInventory {
                        sessions: if id == a {
                            alpha.iter().chain(&beta).cloned().collect()
                        } else {
                            vec![remote(0, "/srv/customer/api")]
                        },
                        warnings: vec![],
                        truncated: false,
                        next_cursor: None,
                    }),
                    cx,
                );
            }
        });
        ui.open_navigator();
        let target = |id| AgentTarget::Ssh {
            connection_id: id,
            connection_label: "ignored alias".into(),
        };
        let alpha_key = directory_group_key("ssh-folder", &target(a), "/srv/customer/api");
        let beta_key = directory_group_key("ssh-folder", &target(a), "/srv/other/api");
        let b_key = directory_group_key("ssh-folder", &target(b), "/srv/customer/api");
        let row = |server, session: &RemoteAgentSession| {
            format!(
                "agent-ssh-conversation-{server}-{:?}-{}",
                session.provider, session.session_id
            )
        };
        let folder = ui.bounds(&format!("agent-folder-{alpha_key}")).unwrap();
        for session in alpha.iter().take(3) {
            let bounds = ui.bounds(&row(a, session)).unwrap();
            assert!(
                bounds.top() >= folder.top() && bounds.bottom() <= folder.bottom(),
                "all three providers share the folder"
            );
        }
        assert!(
            ui.bounds(&format!("agent-folder-{b_key}")).is_some(),
            "same path on another server stays separate"
        );
        assert!(ui.bounds(&row(a, &alpha[11])).is_none());
        assert!(ui.bounds(&row(a, &beta[10])).is_none());
        ui.click_navigation(&format!("agent-ssh-more-{alpha_key}"));
        assert!(ui.bounds(&row(a, &alpha[11])).is_some());
        assert!(
            ui.bounds(&row(a, &beta[10])).is_none(),
            "show more only grows its own folder"
        );
        ui.click_navigation(&row(a, &alpha[11]));
        assert_eq!(
            ui.view
                .read_with(&ui.app, |view, _| view.ssh_sessions.preview.clone()),
            Some((a, alpha[11].clone()))
        );
        if ui.bounds("agent-navigator-scroll").is_none() {
            ui.click("agent-ssh-preview-navigator");
        }
        ui.click_navigation(&format!("agent-folder-toggle-{alpha_key}"));
        assert!(
            ui.bounds(&row(a, &alpha[11])).is_some(),
            "closed folder retains the open preview"
        );
        assert!(ui.bounds(&row(a, &alpha[0])).is_none());
        ui.click_navigation(&format!("agent-folder-toggle-ssh-server:{a}"));
        assert!(
            ui.bounds(&row(a, &alpha[11])).is_some(),
            "closed server retains the open preview"
        );
        assert!(ui.bounds(&format!("agent-folder-{beta_key}")).is_none());
        let saved = ui
            .view
            .read_with(&ui.app, |view, _| view.library.preferences.clone());
        ui.search_navigator("Server A task 11");
        assert!(ui.bounds(&row(a, &alpha[11])).is_some());
        assert!(ui.bounds(&format!("agent-folder-{b_key}")).is_none());
        assert_eq!(
            ui.view
                .read_with(&ui.app, |view, _| view.library.preferences.clone()),
            saved
        );
        ui.search_navigator("no matching project");
        assert!(ui.bounds(&row(a, &alpha[11])).is_none());
        ui.search_navigator("");
        assert!(ui.bounds(&row(a, &alpha[11])).is_some());
        assert!(ui.bounds(&row(a, &alpha[0])).is_none());
        ui.click_navigation(&format!("agent-folder-toggle-ssh-server:{a}"));
        assert!(
            ui.bounds(&row(a, &alpha[0])).is_none(),
            "opening the server preserves its folder's saved collapse"
        );
        assert!(ui.bounds(&row(a, &beta[0])).is_some());
        ui.click_navigation(&format!("agent-ssh-more-{beta_key}"));
        assert!(ui.bounds(&row(a, &beta[10])).is_some());
        assert_eq!(
            ui.draft(),
            "Keep my local draft while browsing remote projects"
        );
        let loaded = ui.view.update(&mut ui.app, |view, cx| {
            view.flush_library(cx).unwrap();
            view.draft_store().load(100).unwrap()
        });
        assert!(loaded
            .preferences
            .collapsed_project_ids
            .contains(&alpha_key));
        assert!(!ui.events.borrow().iter().any(|event| matches!(
            event,
            AgentConsoleEvent::Run(_) | AgentConsoleEvent::ReconnectSshSession { .. }
        )));
    }
}

// SDTEST-2871 — real production layout work stays proportional to the visible viewport.
#[::core::prelude::v1::test]
fn rendered_large_conversation_bounds_transcript_work_to_the_viewport() {
    let mut ui = Cockpit::new(ThemePreference::Dark, 900.0, 700.0, false);
    seed_large_conversation(&mut ui);
    let started = std::time::Instant::now();
    ui.redraw();
    let count = ui
        .view
        .read_with(&ui.app, |view, _| view.rendered_transcripts.get());
    eprintln!(
        "large-conversation layout: {count} transcript renders in {:?}",
        started.elapsed()
    );
    assert!(
        count < 64,
        "offscreen Markdown must not be parsed and laid out on every frame: {count}"
    );
    let mut timings = Vec::new();
    for _ in 0..8 {
        let now = std::time::Instant::now();
        ui.redraw();
        timings.push(now.elapsed());
    }
    timings.sort();
    eprintln!(
        "steady large-conversation redraw p50={:?}, max={:?}",
        timings[timings.len() / 2],
        timings.last().unwrap()
    );
    let composer = ui.composer();
    for _ in 0..4 {
        ui.view
            .read_with(&ui.app, |view, _| view.rendered_transcripts.set(0));
        ui.wheel(180.0);
        let count = ui
            .view
            .read_with(&ui.app, |view, _| view.rendered_transcripts.get());
        assert!(
            count < 64,
            "scrolling must remain bounded to visible/overdraw rows: {count}"
        );
        assert_eq!(ui.composer(), composer);
    }
}

fn seed_large_conversation(ui: &mut Cockpit) {
    use shelldeck_core::agent_session::AgentTraceUpdate;
    ui.view.update(&mut ui.app,|view,_| {
        let session=view.sessions.get_mut(ui.session).unwrap();
        session.messages.clear();session.trace.clear();session.turns.clear();session.boundaries.clear();
        for index in 0..80 {
            session.begin_run(format!("Inspect fixture exchange {index}"),10+index*4).unwrap();
            session.apply_stream_event(AgentStreamEvent::Trace(AgentTraceUpdate {correlation_id:Some(format!("fixture-{index}")),detail:AgentTraceKind::Command {
                command:format!("read fixture-{index}.rs"),status:AgentTraceStatus::Succeeded,exit_code:Some(0),summary:Some("A retained command result\n".repeat(12))
            }}),11+index*4,true);
            session.apply_stream_event(AgentStreamEvent::Text(format!("### Result {index}\n\n{}", "Readable source and **retained context** with `inline code` and a useful explanation.\n\n".repeat(12))),12+index*4,true);
            session.finish(AgentSessionStatus::Completed,13+index*4,true).unwrap();
        }
        view.rendered_transcripts.set(0);
    });
}

// SDTEST-2870 — only a visible, running operation leases paint frames; text remains opaque.
#[::core::prelude::v1::test]
fn rendered_operation_motion_stops_when_hidden_finished_or_reduced() {
    use shelldeck_core::agent_session::AgentTraceUpdate;
    let frames = || activity::MOTION_FRAMES.with(|count| count.get());
    let reset = || activity::MOTION_FRAMES.with(|count| count.set(0));
    {
        let mut ui = Cockpit::new(ThemePreference::Dark, 900.0, 700.0, false);
        let run = ui.start();
        let update = |status, summary| {
            AgentStreamEvent::Trace(AgentTraceUpdate {
                correlation_id: Some("motion".into()),
                detail: AgentTraceKind::Command {
                    command: "inspect the running operation".into(),
                    status,
                    exit_code: None,
                    summary: Some(summary),
                },
            })
        };
        ui.view.update(&mut ui.app, |view, cx| {
            view.push_stream_event_for(
                run,
                update(AgentTraceStatus::Running, "Short output".into()),
                cx,
            )
        });
        ui.redraw();
        reset();
        ui.redraw();
        assert!(
            frames() > 0,
            "a visible operation animates its small indicator"
        );
        ui.view.update(&mut ui.app, |view, cx| {
            view.push_stream_event_for(
                run,
                update(AgentTraceStatus::Succeeded, "Completed output".into()),
                cx,
            )
        });
        reset();
        ui.redraw();
        assert_eq!(
            frames(),
            0,
            "completed work schedules no recurring operation frames"
        );
        ui.view.update(&mut ui.app, |view, cx| {
            view.push_stream_event_for(
                run,
                update(
                    AgentTraceStatus::Running,
                    "Long retained output line\n".repeat(120),
                ),
                cx,
            )
        });
        ui.redraw();
        ui.wheel(-100000.0);
        reset();
        ui.redraw();
        assert_eq!(
            frames(),
            0,
            "an offscreen running indicator must not animate the window"
        );
        ui.view
            .update(&mut ui.app, |view, cx| view.finish_run(run, None, None, cx));
        ui.wheel(100000.0);
        reset();
        ui.redraw();
        assert_eq!(frames(), 0, "cancellation stops operation motion");
        let trace = ui.view.read_with(&ui.app, |view, _| {
            view.sessions
                .get(ui.session)
                .unwrap()
                .trace
                .iter()
                .find(|trace| trace.correlation_id.as_deref() == Some("motion"))
                .unwrap()
                .id
        });
        assert!(
            ui.bounds(&format!("agent-operation-outcome-{trace}-Cancelled"))
                .is_some(),
            "unfinished work no longer says Running after Stop"
        );
        assert!(ui
            .bounds(&format!("agent-operation-outcome-{trace}-Running"))
            .is_none());
        let ended = ui.start();
        ui.view.update(&mut ui.app, |view, cx| {
            view.push_stream_event_for(
                ended,
                AgentStreamEvent::Trace(AgentTraceUpdate {
                    correlation_id: Some("missing-final-result".into()),
                    detail: AgentTraceKind::Command {
                        command: "Inspect a missing final result".into(),
                        status: AgentTraceStatus::Running,
                        exit_code: None,
                        summary: None,
                    },
                }),
                cx,
            );
            view.finish_run(ended, Some(0), None, cx);
        });
        reset();
        ui.redraw();
        let unfinished = ui.view.read_with(&ui.app, |view, _| {
            view.sessions
                .get(ui.session)
                .unwrap()
                .trace
                .iter()
                .find(|trace| trace.correlation_id.as_deref() == Some("missing-final-result"))
                .unwrap()
                .id
        });
        assert!(ui.bounds(&format!("agent-operation-outcome-{unfinished}-Unknown")).is_some(), "provider completion without a final operation event cannot fabricate success or keep Running");
        assert_eq!(frames(), 0);
    }
    // The test platform has no OS motion preference setter. Render the actual
    // canvas with the reduced-motion policy supplied by the production caller.
    struct Reduced;
    impl Render for Reduced {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div()
                .size(gpui::px(32.0))
                .child(activity::running_dot(ShellDeckColors::primary(), true))
        }
    }
    let mut app = TestAppContext::single();
    let handle = app.add_window(|_, _| Reduced);
    let mut window = VisualTestContext::from_window(*handle, &app);
    reset();
    window.update(|window, cx| window.draw(cx).clear());
    assert_eq!(
        frames(),
        0,
        "reduced motion paints a static indicator without another frame"
    );
}

// SDTEST-2872 — virtual rows preserve actual scroll, disclosure, copy and streaming authority.
#[::core::prelude::v1::test]
fn rendered_virtual_timeline_preserves_reveal_copy_reading_and_following() {
    let mut ui = Cockpit::new(ThemePreference::SolarizedLight, 900.0, 700.0, false);
    seed_large_conversation(&mut ui);
    ui.redraw();
    assert!(ui
        .view
        .read_with(&ui.app, |view, _| view.timeline.virtualized));
    let composer = ui.composer();
    let (first_trace, last_message, last_original) = ui.view.read_with(&ui.app, |view, _| {
        let session = view.sessions.get(ui.session).unwrap();
        let last = session.messages.last().unwrap();
        (
            session.trace.first().unwrap().id,
            last.id,
            last.text.clone(),
        )
    });
    assert!(ui.at_end());
    // The last message's actions are below a tall Markdown body; actual wheel
    // input moves its end into view before copying exact canonical bytes.
    ui.wheel(-100000.0);
    ui.click(&format!("agent-message-copy-{last_message}"));
    assert_eq!(
        ui.app.read_from_clipboard().unwrap().text().unwrap(),
        last_original
    );
    ui.wheel(240.0);
    assert!(!ui.at_end());
    assert_eq!(ui.composer(), composer);
    // Same-length snapshots still invalidate the active row's cached height.
    let run = ui.view.update(&mut ui.app, |view, cx| {
        let request = view
            .sessions
            .begin_run(ui.session, "A subsequent turn", 400)
            .unwrap();
        let run = request.id;
        view.begin_run(request, cx);
        run
    });
    ui.redraw();
    // Explicitly starting a new turn follows its end; wheel input opts out again.
    assert!(ui.at_end());
    let composer = ui.composer();
    ui.wheel(240.0);
    let before = ui.view.read_with(&ui.app, |view, _| {
        let top = view.timeline.list.logical_scroll_top();
        (view.timeline.rows[top.item_ix].id, top.offset_in_item)
    });
    ui.view.update(&mut ui.app, |view, cx| {
        view.push_stream_event_for(
            run,
            AgentStreamEvent::Text("Streaming output below your reading position.".into()),
            cx,
        )
    });
    ui.redraw();
    let after = ui.view.read_with(&ui.app, |view, _| {
        let top = view.timeline.list.logical_scroll_top();
        (view.timeline.rows[top.item_ix].id, top.offset_in_item)
    });
    assert_eq!(
        after, before,
        "a virtual list preserves its logical row and in-row reading offset"
    );
    let (snapshot_id, snapshot) = ui.view.read_with(&ui.app, |view, _| {
        let last = view
            .sessions
            .get(ui.session)
            .unwrap()
            .messages
            .last()
            .unwrap();
        let mut text = "x\n\n".repeat(last.text.len() / 3);
        text.push_str(&"x".repeat(last.text.len() - text.len()));
        (last.id, text)
    });
    ui.view.update(&mut ui.app, |view, cx| {
        view.push_stream_event_for(run, AgentStreamEvent::TextSnapshot(snapshot.clone()), cx)
    });
    ui.redraw();
    assert_eq!(
        ui.view.read_with(&ui.app, |view, _| {
            let top = view.timeline.list.logical_scroll_top();
            (view.timeline.rows[top.item_ix].id, top.offset_in_item)
        }),
        before,
        "same-length replacements cannot steal the reading anchor"
    );

    ui.view
        .update(&mut ui.app, |view, cx| view.reveal_trace(first_trace, cx));
    ui.redraw();
    let trace = ui
        .bounds(&format!("agent-trace-{first_trace}"))
        .expect("inspector reveal materializes the exact retained operation");
    assert!(ui
        .bounds("agent-timeline-scroll")
        .unwrap()
        .contains(&trace.origin));
    ui.click("agent-collapse-floating");
    let header = ui.bounds(&format!("agent-work-{first_trace}")).unwrap();
    assert!(ui
        .bounds("agent-timeline-scroll")
        .unwrap()
        .contains(&header.center()));
    assert_eq!(ui.composer(), composer);
    ui.click("agent-jump-latest");
    assert!(ui.at_end());
    ui.click(&format!("agent-message-copy-{snapshot_id}"));
    assert_eq!(
        ui.app.read_from_clipboard().unwrap().text().unwrap(),
        snapshot
    );
    ui.view.update(&mut ui.app, |view, cx| {
        view.push_stream_event_for(
            run,
            AgentStreamEvent::TextDelta(" Continued streaming.".into()),
            cx,
        )
    });
    ui.redraw();
    assert!(ui.at_end(), "streaming follows only after explicit Jump");
    ui.wheel(240.0);
    let retained_anchor = ui.view.read_with(&ui.app, |view, _| {
        let top = view.timeline.list.logical_scroll_top();
        (view.timeline.rows[top.item_ix].id, top.offset_in_item)
    });
    ui.view.update(&mut ui.app, |view, cx| {
        view.push_stream_events_for(
            run,
            (0..140).map(|index| AgentStreamEvent::UserInputAccepted {
                text: format!("Accepted follow-up fixture {index}"),
            }),
            cx,
        )
    });
    ui.redraw();
    assert_eq!(
        ui.view.read_with(&ui.app, |view, _| {
            let top = view.timeline.list.logical_scroll_top();
            (view.timeline.rows[top.item_ix].id, top.offset_in_item)
        }),
        retained_anchor,
        "canonical retention pruning preserves the surviving reader's identity"
    );
    ui.click("agent-jump-latest");
    ui.type_draft("Keep this follow-up while checking old work");
    let other = ui.view.update(&mut ui.app, |view, cx| {
        let context = view.sessions.get(ui.session).unwrap().context.clone();
        view.create_session("Another conversation", context, cx)
            .unwrap()
    });
    ui.redraw();
    ui.view.update(&mut ui.app, |view, cx| {
        view.push_stream_event_for(
            run,
            AgentStreamEvent::TextDelta(" Background update.".into()),
            cx,
        )
    });
    ui.redraw();
    assert_eq!(
        ui.view
            .read_with(&ui.app, |view, _| view.sessions.selected_id()),
        Some(other)
    );
    ui.view
        .update(&mut ui.app, |view, cx| view.select_session(ui.session, cx));
    ui.redraw();
    assert_eq!(ui.draft(), "Keep this follow-up while checking old work");
    ui.window
        .simulate_resize(size(gpui::px(620.0), gpui::px(560.0)));
    ui.view
        .read_with(&ui.app, |view, _| view.rendered_transcripts.set(0));
    ui.redraw();
    assert!(
        ui.view
            .read_with(&ui.app, |view, _| view.rendered_transcripts.get())
            < 64,
        "resizing remeasures only viewport rows"
    );

    assert!(ui.view.read_with(&ui.app, |view, _| view
        .sessions
        .get(ui.session)
        .unwrap()
        .trace
        .iter()
        .any(|trace| trace.id == first_trace)));
}

// SDTEST-2873 — a large expanded group has virtual children rather than one enormous list item.
#[::core::prelude::v1::test]
fn rendered_large_operation_group_virtualizes_children_without_losing_its_summary() {
    use shelldeck_core::agent_session::AgentTraceUpdate;
    let mut ui = Cockpit::new(ThemePreference::Dark, 900.0, 700.0, false);
    let run = ui.start();
    ui.type_draft("Preserve the instruction while reading operations");
    ui.view.update(&mut ui.app, |view, cx| {
        view.push_stream_events_for(
            run,
            (0..240).map(|index| {
                AgentStreamEvent::Trace(AgentTraceUpdate {
                    correlation_id: Some(format!("many-{index}")),
                    detail: AgentTraceKind::Command {
                        command: format!("inspect operation {index}"),
                        status: AgentTraceStatus::Succeeded,
                        exit_code: Some(0),
                        summary: Some("Retained operation output\n".repeat(20)),
                    },
                })
            }),
            cx,
        )
    });
    ui.redraw();
    ui.wheel(-100000.0);
    let group = ui.view.read_with(&ui.app, |view, _| {
        view.sessions
            .get(ui.session)
            .unwrap()
            .trace
            .iter()
            .find(|trace| trace.correlation_id.as_deref() == Some("many-0"))
            .unwrap()
            .id
    });
    assert!(
        ui.view
            .read_with(&ui.app, |view, _| view.timeline.rows.len())
            < 16,
        "collapsed operations must not materialize their hidden children"
    );
    let composer = ui.composer();
    ui.view
        .read_with(&ui.app, |view, _| view.rendered_transcripts.set(0));
    ui.click(&format!("agent-work-{group}"));
    assert!(ui
        .view
        .read_with(&ui.app, |view, _| view.timeline.virtualized));
    assert!(
        ui.view
            .read_with(&ui.app, |view, _| view.rendered_transcripts.get())
            < 64,
        "expanding does not lay out every output"
    );
    let header = ui.bounds(&format!("agent-work-{group}")).unwrap();
    assert!(
        ui.bounds("agent-timeline-scroll")
            .unwrap()
            .contains(&header.center()),
        "expansion preserves the summary instead of jumping to the final operation"
    );
    ui.wheel(-480.0);
    assert!(
        !ui.at_end(),
        "a fast wheel gesture cannot jump over unmeasured operations to the end"
    );
    ui.click("agent-collapse-floating");
    assert!(ui.bounds("agent-collapse-floating").is_none());
    let header = ui.bounds(&format!("agent-work-{group}")).unwrap();
    assert!(ui
        .bounds("agent-timeline-scroll")
        .unwrap()
        .contains(&header.center()));
    assert_eq!(ui.composer(), composer);
    assert_eq!(
        ui.draft(),
        "Preserve the instruction while reading operations"
    );
    ui.view.read_with(&ui.app,|view,_| {
        let session=view.sessions.get(ui.session).unwrap();
        assert_eq!(session.trace.iter().filter(|trace|trace.correlation_id.as_deref().is_some_and(|key|key.starts_with("many-"))).count(),240);
        assert!(session.trace.iter().all(|trace|matches!(&trace.detail,AgentTraceKind::Command {summary:Some(text),..} if text.ends_with("Retained operation output\n"))));
    });
}

// SDTEST-2874 — imported pages share the lazy scroll owner and exact source actions.
#[::core::prelude::v1::test]
fn rendered_large_native_history_keeps_paging_copy_and_drafts_with_bounded_work() {
    let mut ui = Cockpit::new(ThemePreference::Dark, 900.0, 700.0, true);
    ui.type_draft("Keep this while reading a large imported page");
    ui.view.update(&mut ui.app, |view, cx| {
        let mut page = view.native_history[&ui.session].page.clone().unwrap();
        page.total_entries = 128;
        page.end = 64;
        for index in 1..64 {
            page.entries.push(RemoteHistoryEntry {
                index,
                kind: RemoteHistoryKind::Assistant,
                title: None,
                text: format!(
                    "### Imported exchange {index}\n\n{}",
                    "Readable **native context** and `source` stay available.\n\n".repeat(20)
                ),
                at_ms: None,
                part: 1,
                parts: 1,
            });
        }
        view.install_native_history(ui.session, page);
        view.rendered_transcripts.set(0);
        cx.notify();
    });
    ui.redraw();
    assert!(ui
        .view
        .read_with(&ui.app, |view, _| view.timeline.virtualized));
    assert!(
        ui.view
            .read_with(&ui.app, |view, _| view.rendered_transcripts.get())
            < 32
    );
    let composer = ui.composer();
    ui.wheel(100000.0);
    ui.click("agent-history-toggle-0");
    ui.click(&ui.source.clone().selector("copy"));
    assert_eq!(
        ui.app.read_from_clipboard().unwrap().text().unwrap(),
        ui.original
    );
    ui.click("agent-history-toggle-0");
    ui.wheel(-100000.0);
    assert!(ui.bounds("agent-history-entry-63").is_some());
    ui.click("agent-history-bottom-newer");
    let request = ui
        .events
        .borrow()
        .iter()
        .rev()
        .find_map(|event| match event {
            AgentConsoleEvent::LoadSshHistory(request) => Some(request.clone()),
            _ => None,
        })
        .unwrap();
    assert_eq!(request.query.position, RemoteHistoryPosition::After(64));
    ui.view.update(&mut ui.app, |view, cx| {
        let mut page = view.native_history[&ui.session].page.clone().unwrap();
        page.start = 64;
        page.end = 128;
        for entry in &mut page.entries {
            entry.index += 64;
        }
        view.complete_native_history(request, Ok(page), cx);
        view.rendered_transcripts.set(0);
    });
    ui.redraw();
    let first = ui
        .bounds("agent-history-entry-64")
        .expect("a completed page opens at its start");
    assert!(ui
        .bounds("agent-timeline-scroll")
        .unwrap()
        .contains(&first.center()));
    assert!(ui.bounds("agent-history-entry-0").is_none());
    assert!(
        ui.view
            .read_with(&ui.app, |view, _| view.rendered_transcripts.get())
            < 32
    );
    // Fresh-history loading keeps one toolbar and returns through the original source.
    ui.click("agent-history-fresh");
    assert!(ui.bounds("agent-history-back").is_some());
    assert!(ui.bounds("agent-history-entry-64").is_none());
    let fresh = ui
        .events
        .borrow()
        .iter()
        .rev()
        .find_map(|event| match event {
            AgentConsoleEvent::LoadSshHistory(request) => Some(request.clone()),
            _ => None,
        })
        .unwrap();
    ui.view.update(&mut ui.app, |view, cx| {
        let page = RemoteHistoryPage {
            provider: AgentProvider::Codex,
            session_id: "fixture-native".into(),
            workdir: "/srv/fixture".into(),
            model: None,
            snapshot: "fresh-snapshot".into(),
            start: 0,
            end: 1,
            total_entries: 1,
            entries: vec![RemoteHistoryEntry {
                index: 0,
                kind: RemoteHistoryKind::Assistant,
                title: None,
                text: "Fresh source is read without replaying tools.".into(),
                at_ms: None,
                part: 1,
                parts: 1,
            }],
            warnings: vec![],
        };
        view.complete_native_history(fresh, Ok(page), cx);
    });
    ui.redraw();
    assert!(ui.bounds("agent-history-entry-0").is_some());
    ui.click("agent-history-back");
    let restored = ui
        .events
        .borrow()
        .iter()
        .rev()
        .find_map(|event| match event {
            AgentConsoleEvent::LoadSshHistory(request) => Some(request),
            _ => None,
        })
        .unwrap()
        .query
        .snapshot
        .clone();
    assert_eq!(restored.as_deref(), Some("fixture-snapshot"));
    assert_eq!(ui.composer(), composer);
    assert_eq!(ui.draft(), "Keep this while reading a large imported page");
    assert!(!ui
        .events
        .borrow()
        .iter()
        .any(|event| matches!(event, AgentConsoleEvent::Run(_))));
}

// SDTEST-2875 — real redraws reuse unchanged rows without freezing actual controls.
#[::core::prelude::v1::test]
fn rendered_cached_rows_skip_unchanged_transcripts_and_keep_live_actions_current() {
    let mut ui = Cockpit::new(ThemePreference::Dark, 900.0, 700.0, false);
    seed_large_conversation(&mut ui);
    ui.redraw();
    ui.wheel(-100000.0);
    ui.redraw();
    let (message, original) = ui.view.read_with(&ui.app, |view, _| {
        let message = view
            .sessions
            .get(ui.session)
            .unwrap()
            .messages
            .last()
            .unwrap();
        (message.id, message.text.clone())
    });
    let copy_position = ui
        .bounds(&format!("agent-message-copy-{message}"))
        .unwrap()
        .center();
    ui.view
        .read_with(&ui.app, |view, _| view.rendered_transcripts.set(0));
    for _ in 0..5 {
        ui.view.update(&mut ui.app, |_, cx| cx.notify());
        ui.window.run_until_parked();
    }
    assert_eq!(
        ui.view
            .read_with(&ui.app, |view, _| view.rendered_transcripts.get()),
        0,
        "unrelated notifications cannot rebuild immutable visible Markdown"
    );
    // Cached prepaint reuses native hitboxes; the test-only selector map is
    // intentionally frame-local. Dispatch at the real measured control position.
    ui.window
        .simulate_mouse_move(copy_position, None, Modifiers::none());
    ui.window.simulate_click(copy_position, Modifiers::none());
    ui.redraw();
    assert_eq!(
        ui.app.read_from_clipboard().unwrap().text().unwrap(),
        original
    );
    let run = ui.start();
    ui.view.update(&mut ui.app, |view, cx| {
        view.push_stream_event_for(
            run,
            AgentStreamEvent::Text("A current streaming answer.".into()),
            cx,
        )
    });
    ui.redraw();
    let current = ui.view.read_with(&ui.app, |view, _| {
        view.sessions
            .get(ui.session)
            .unwrap()
            .messages
            .last()
            .unwrap()
            .id
    });
    ui.view.update(&mut ui.app, |view, cx| {
        view.push_stream_event_for(
            run,
            AgentStreamEvent::TextSnapshot("A replaced streaming body.".into()),
            cx,
        )
    });
    ui.redraw();
    ui.app
        .update(|cx| cx.write_to_clipboard(ClipboardItem::new_string("not copied".into())));
    ui.click(&format!("agent-message-copy-{current}"));
    assert_eq!(
        ui.app.read_from_clipboard().unwrap().text().unwrap(),
        "A replaced streaming body."
    );
    ui.window
        .update(|window, _| window.set_rem_size(gpui::px(20.0)));
    ui.window
        .simulate_resize(size(gpui::px(620.0), gpui::px(560.0)));
    ui.redraw();
    let composer = ui.composer();
    ui.wheel(240.0);
    ui.wheel(-100000.0);
    assert_eq!(ui.composer(), composer);
    ui.app
        .update(|cx| cx.write_to_clipboard(ClipboardItem::new_string("not copied".into())));
    ui.click(&format!("agent-message-copy-{current}"));
    assert_eq!(
        ui.app.read_from_clipboard().unwrap().text().unwrap(),
        "A replaced streaming body."
    );
    rust_i18n::set_locale("fr");
    ShellDeckColors::set_theme(&ThemePreference::SolarizedLight);
    ui.app.update(|cx| {
        adabraka_ui::theme::install_theme(cx, crate::theme::adabraka_theme_from_palette());
        cx.refresh_windows();
    });
    ui.redraw();
    ui.wheel(-100000.0);
    ui.app.update(|cx| {
        cx.write_to_clipboard(ClipboardItem::new_string(
            "not copied after theme switch".into(),
        ))
    });
    ui.click(&format!("agent-message-copy-{current}"));
    assert_eq!(
        ui.app.read_from_clipboard().unwrap().text().unwrap(),
        "A replaced streaming body."
    );
    ui.type_draft("Retain this draft while rebuilding caches");
    ui.view.update(&mut ui.app, |view, cx| {
        let context = view.sessions.selected().unwrap().context.clone();
        view.create_session("A separate cache source", context, cx)
            .unwrap();
    });
    ui.redraw();
    assert!(ui
        .bounds(&format!("agent-message-copy-{current}"))
        .is_none());
    ui.view
        .update(&mut ui.app, |view, cx| view.select_session(ui.session, cx));
    ui.redraw();
    assert_eq!(ui.draft(), "Retain this draft while rebuilding caches");
    assert!(
        ui.view
            .read_with(&ui.app, |view, _| view.timeline.row_cache_len())
            <= 64
    );
}

// SDTEST-2877 — cached native scenes retain the exact glyph and quad after sorting.
#[::core::prelude::v1::test]
fn cached_scene_replay_preserves_glyph_and_quad_identity_after_batch_sorting() {
    gpui::verify_scene_replay_preserves_sorted_primitives();
}

// SDTEST-2878 / SDUC-564 — DWM fallback ratios must retain high-refresh timing without a panic.
#[::core::prelude::v1::test]
fn display_refresh_ratios_retain_high_and_fractional_rates() {
    use gpui::refresh_interval::duration_from_ratio;
    for (counts, frequency) in [
        (1, 60),
        (1, 120),
        (1, 144),
        (1, 240),
        (1001, 60000),
        (83333, 10000000),
    ] {
        let actual = duration_from_ratio(counts, frequency)
            .unwrap()
            .as_secs_f64();
        assert!((actual - counts as f64 / frequency as f64).abs() < 1e-9);
    }
    assert!(duration_from_ratio(0, 120).is_none());
    assert!(duration_from_ratio(1, 0).is_none());
    assert!(duration_from_ratio(u64::MAX, 1).is_none());
}

// SDTEST-2880 / SDUC-475, SDUC-564 — long prose bounds layout within a single reply.
#[::core::prelude::v1::test]
fn rendered_long_prose_virtualizes_blocks_and_keeps_exact_actions_and_anchors() {
    for theme in [ThemePreference::Dark, ThemePreference::SolarizedLight] {
        for (width, height) in [(900.0, 700.0), (620.0, 560.0)] {
            let mut ui = Cockpit::new(theme.clone(), width, height, false);
            let run = ui.start();
            let original = format!("# Retained document\n\n{}\n[reference]: https://example.test/target\n",
                "Retained **context** with `source` and a [reference link][reference] stays readable · é🌍.\n\n".repeat(200));
            ui.view.update(&mut ui.app, |view, cx| {
                view.push_stream_event_for(
                    run,
                    AgentStreamEvent::TextSnapshot(original.clone()),
                    cx,
                )
            });
            ui.redraw();
            let message = ui.view.read_with(&ui.app, |view, _| {
                let message = view
                    .sessions
                    .get(ui.session)
                    .unwrap()
                    .messages
                    .last()
                    .unwrap()
                    .id;
                assert!(
                    view.timeline
                        .rows
                        .iter()
                        .filter(|row| matches!(row.kind, timeline::Kind::MessagePart(_, _)))
                        .count()
                        > 20
                );
                message
            });
            let composer = ui.composer();
            ui.click(&format!("agent-message-copy-{message}"));
            assert_eq!(
                ui.app.read_from_clipboard().unwrap().text().unwrap(),
                original
            );
            ui.type_draft("Keep this instruction while reading a long reply");
            ui.view
                .read_with(&ui.app, |view, _| view.timeline.take_markdown_work());
            for _ in 0..5 {
                ui.wheel(180.0);
                let work = ui
                    .view
                    .read_with(&ui.app, |view, _| view.timeline.take_markdown_work());
                assert!(
                    work > 0 && work < 128,
                    "three harness draws must render viewport blocks rather than all 201: {work}"
                );
            }
            let anchor = |ui: &Cockpit| {
                ui.view.read_with(&ui.app, |view, _| {
                    let top = view.timeline.list.logical_scroll_top();
                    (
                        view.timeline.rows[top.item_ix].id,
                        view.timeline.rows[top.item_ix].kind.clone(),
                        top.offset_in_item,
                    )
                })
            };
            let before = anchor(&ui);
            let replacement = original.replace("Retained", "Updated!");
            assert_eq!(replacement.len(), original.len());
            ui.view.update(&mut ui.app, |view, cx| {
                view.push_stream_event_for(
                    run,
                    AgentStreamEvent::TextSnapshot(replacement.clone()),
                    cx,
                )
            });
            ui.redraw();
            assert!(
                anchor(&ui) == before,
                "same-length snapshots preserve the exact block and reading offset"
            );
            ui.window
                .simulate_resize(size(gpui::px(620.0), gpui::px(560.0)));
            ui.redraw();
            ui.wheel(-100000.0);
            ui.click(&format!("agent-message-copy-{message}"));
            assert_eq!(
                ui.app.read_from_clipboard().unwrap().text().unwrap(),
                replacement
            );
            assert_eq!(
                ui.draft(),
                "Keep this instruction while reading a long reply"
            );
            if width == 620.0 {
                assert_eq!(ui.composer(), composer);
            }
        }
    }
}

// SDTEST-2882 / SDUC-551, SDUC-564 — transcript repaint does not rebuild or disable the sidebar.
#[::core::prelude::v1::test]
fn rendered_timeline_motion_reuses_navigator_and_search_stays_live() {
    let mut ui = Cockpit::new(ThemePreference::Dark, 1100.0, 800.0, false);
    seed_large_conversation(&mut ui);
    ui.open_navigator();
    let run = ui.start();
    ui.view.update(&mut ui.app, |view, cx| {
        view.push_stream_events_for(
            run,
            [
                AgentStreamEvent::Ready,
                AgentStreamEvent::Text("An existing streaming reply.".into()),
            ],
            cx,
        )
    });
    ui.type_draft("Keep this unsent instruction");
    ui.wheel(180.0);
    let search = ui.bounds("agent-nav-search").unwrap().center();
    let position = ui.bounds("agent-timeline-scroll").unwrap().center();
    ui.window
        .simulate_mouse_move(position, None, Modifiers::none());
    ui.window.run_until_parked();
    ui.view
        .read_with(&ui.app, |view, _| view.rendered_navigators.set(0));
    for _ in 0..5 {
        ui.window.simulate_event(ScrollWheelEvent {
            position,
            delta: ScrollDelta::Pixels(point(gpui::px(0.0), gpui::px(120.0))),
            touch_phase: TouchPhase::Moved,
            ..Default::default()
        });
        ui.window.run_until_parked();
    }
    assert_eq!(
        ui.view
            .read_with(&ui.app, |view, _| view.rendered_navigators.get()),
        0,
        "scrolling must reuse the cached navigator"
    );
    ui.view.update(&mut ui.app, |view, cx| {
        view.push_stream_event_for(
            run,
            AgentStreamEvent::TextDelta(" More readable context.".into()),
            cx,
        )
    });
    ui.window.run_until_parked();
    assert_eq!(
        ui.view
            .read_with(&ui.app, |view, _| view.rendered_navigators.get()),
        0,
        "selected reply deltas with empty search do not change navigator content"
    );
    // Hit the pre-cache control using its actual retained hitbox.
    ui.window.simulate_click(search, Modifiers::none());
    ui.window.simulate_input("freshneedle");
    ui.window.run_until_parked();
    assert!(
        ui.view
            .read_with(&ui.app, |view, _| view.rendered_navigators.get())
            > 0
    );
    ui.view
        .read_with(&ui.app, |view, _| view.rendered_navigators.set(0));
    ui.view.update(&mut ui.app, |view, cx| {
        view.push_stream_event_for(run, AgentStreamEvent::TextDelta(" freshneedle".into()), cx)
    });
    ui.window.run_until_parked();
    assert!(
        ui.view
            .read_with(&ui.app, |view, _| view.rendered_navigators.get())
            > 0,
        "active transcript searches must refresh as streamed text starts matching"
    );
    assert!(ui
        .bounds(&format!("agent-nav-session-{}", ui.session))
        .is_some());
    assert_eq!(ui.draft(), "Keep this unsent instruction");
}
