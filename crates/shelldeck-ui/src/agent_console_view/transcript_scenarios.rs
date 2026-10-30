//! T3-style transcript scenarios against production reducers/control handlers.
//! TestAppContext supplies a private clipboard; no renderer/provider is started.
use super::*;
use crate::agent_console_view::activity::group_completed_work;
use shelldeck_core::agent_remote_history::{
    RemoteHistoryEntry, RemoteHistoryKind, RemoteHistoryPage,
};
use shelldeck_core::agent_session::{AgentTraceUpdate, MAX_AGENT_TRACE_EVENTS_PER_SESSION};

fn original(label: &str) -> String {
    format!(
        "{label} · é🌍\r\n{}\r\nKeep the explanation after the image.\n",
        "aB3+/xY9".repeat(120)
    )
}

fn install(
    view: &mut AgentConsoleView,
    cx: &mut Context<AgentConsoleView>,
) -> (Uuid, Vec<TranscriptId>) {
    let id = view
        .sessions
        .reconnect_remote(
            "Image inspection",
            AgentExecutionContext {
                provider: AgentProvider::Codex,
                target: AgentTarget::Ssh {
                    connection_id: Uuid::new_v4(),
                    connection_label: "Fixture".into(),
                },
                access: AgentAccessMode::ReadOnly,
                workdir: "/srv/fixture".into(),
                model: None,
            },
            "fixture-native",
            1,
        )
        .unwrap();
    view.session_ui.insert(id, AgentSessionUi::new(cx));
    view.sessions.begin_run(id, "Inspect the image", 2).unwrap();
    view.sessions
        .apply_stream_event(id, AgentStreamEvent::Text(original("Message")), 3)
        .unwrap();
    for (index, detail) in [
        AgentTraceKind::Command {
            command: "git diff".into(),
            status: AgentTraceStatus::Succeeded,
            exit_code: Some(0),
            summary: Some(original("Command")),
        },
        AgentTraceKind::Diff {
            path: "image.txt".into(),
            additions: 1,
            deletions: 0,
            status: AgentTraceStatus::Succeeded,
            preview: Some(original("Diff")),
        },
        AgentTraceKind::Test {
            name: "Image checks".into(),
            status: AgentTraceStatus::Succeeded,
            summary: Some(original("Test")),
        },
        AgentTraceKind::Tool {
            name: "Inspect image".into(),
            status: AgentTraceStatus::Succeeded,
            summary: Some(original("Tool")),
        },
    ]
    .into_iter()
    .enumerate()
    {
        view.sessions
            .apply_stream_event(
                id,
                AgentStreamEvent::Trace(AgentTraceUpdate {
                    correlation_id: Some(format!("call-{index}")),
                    detail,
                }),
                4 + index as i64,
            )
            .unwrap();
    }
    view.sessions
        .finish(id, AgentSessionStatus::Completed, 9)
        .unwrap();
    let session = view.sessions.get(id).unwrap();
    let mut keys = vec![TranscriptId::Message(session.messages.last().unwrap().id)];
    keys.extend(
        session
            .trace
            .iter()
            .map(|trace| TranscriptId::Trace(trace.id)),
    );
    view.install_native_history(
        id,
        RemoteHistoryPage {
            provider: AgentProvider::Codex,
            session_id: "fixture-native".into(),
            workdir: "/srv/fixture".into(),
            model: None,
            snapshot: "source-snapshot".into(),
            total_entries: 1,
            start: 0,
            end: 1,
            entries: vec![RemoteHistoryEntry {
                index: 0,
                kind: RemoteHistoryKind::Tool,
                title: Some("Saved diff".into()),
                text: original("Native tool"),
                at_ms: Some(1),
                part: 1,
                parts: 1,
            }],
            warnings: vec![],
        },
    );
    keys.push(TranscriptId::Native {
        snapshot: "source-snapshot".into(),
        index: 0,
    });
    (id, keys)
}

// SDTEST-2844 / SDUC-563, SDUC-564
#[::core::prelude::v1::test]
fn transcript_controls_copy_originals_and_isolate_conversation_disclosure() {
    let mut app = TestAppContext::single();
    let view = app.new(AgentConsoleView::new);
    view.update(&mut app, |view, cx| {
        view.library = AgentLibraryUi::new(cx);
        let (id, keys) = install(view, cx);
        let retained = view.sessions.get(id).unwrap().export_json().unwrap();
        let exported = view.sessions.get(id).unwrap().export_markdown();
        for key in &keys {
            let text = view.original_transcript(id, key).unwrap().to_owned();
            assert!(!fold_encoded(&text, |_| "[hidden]".into()).unwrap().contains("aB3+/xY9"));
            assert!(!view.session_ui[&id].raw_transcripts.contains(key));
            for _ in 0..2 {
                assert!(view.toggle_raw_transcript(id, key, cx));
                assert!(view.copy_original_transcript(id, key, cx));
                assert_eq!(cx.read_from_clipboard().unwrap().text().unwrap(), text,
                    "copy must contain every byte of the original in both disclosure states: {key:?}");
            }
            assert!(!view.session_ui[&id].raw_transcripts.contains(key));
        }
        assert!(view.toggle_raw_transcript(id, &keys[0], cx));
        let context = view.sessions.get(id).unwrap().context.clone();
        let other = view.sessions.create("Another conversation", context, 10).unwrap();
        view.session_ui.insert(other, AgentSessionUi::new(cx));
        cx.write_to_clipboard(ClipboardItem::new_string("Leave this clipboard alone".into()));
        for key in &keys {
            assert!(!view.toggle_raw_transcript(id, key, cx));
            assert!(!view.copy_original_transcript(id, key, cx));
            assert!(!view.copy_original_transcript(other, key, cx));
        }
        assert_eq!(cx.read_from_clipboard().unwrap().text().as_deref(), Some("Leave this clipboard alone"));
        assert!(view.session_ui[&other].raw_transcripts.is_empty());
        view.sessions.select(id).unwrap();
        assert!(view.session_ui[&id].raw_transcripts.contains(&keys[0]));
        assert!(view.copy_original_transcript(id, &keys[0], cx));
        assert_eq!(view.sessions.get(id).unwrap().export_json().unwrap(), retained);
        assert_eq!(view.sessions.get(id).unwrap().export_markdown(), exported);
        assert!(view.run_sessions.is_empty(), "disclosure/copy never admits execution");
    });
}

// SDTEST-2845 / SDUC-563, SDUC-564
#[::core::prelude::v1::test]
fn transcript_callbacks_reject_replaced_native_pages_and_missing_sources() {
    let mut app = TestAppContext::single();
    let view = app.new(AgentConsoleView::new);
    view.update(&mut app, |view, cx| {
        view.library = AgentLibraryUi::new(cx);
        let (id, keys) = install(view, cx);
        let old = keys.last().unwrap();
        let old_page = view.native_history[&id].page.clone().unwrap();
        assert!(view.toggle_raw_transcript(id, old, cx));
        let mut replacement = old_page.clone();
        replacement.snapshot = "replaced-snapshot".into();
        replacement.entries[0].text = original("Replacement native output");
        view.install_native_history(id, replacement);
        let new = TranscriptId::Native {
            snapshot: "replaced-snapshot".into(),
            index: 0,
        };
        cx.write_to_clipboard(ClipboardItem::new_string("Unchanged".into()));
        for stale in [
            old.clone(),
            TranscriptId::Native {
                snapshot: "replaced-snapshot".into(),
                index: 99,
            },
            TranscriptId::Message(Uuid::new_v4()),
            TranscriptId::Trace(Uuid::new_v4()),
        ] {
            assert!(!view.toggle_raw_transcript(id, &stale, cx));
            assert!(!view.copy_original_transcript(id, &stale, cx));
            assert_eq!(
                cx.read_from_clipboard().unwrap().text().as_deref(),
                Some("Unchanged")
            );
        }
        assert!(!view.session_ui[&id].raw_transcripts.contains(&new));
        assert!(view.copy_original_transcript(id, &new, cx));
        assert_eq!(
            cx.read_from_clipboard().unwrap().text().unwrap(),
            original("Replacement native output")
        );
        // Paging back to the same immutable source retains its explicit choice.
        view.install_native_history(id, old_page);
        assert!(view.session_ui[&id].raw_transcripts.contains(old));
        assert!(view.copy_original_transcript(id, old, cx));
        assert_eq!(
            cx.read_from_clipboard().unwrap().text().unwrap(),
            original("Native tool")
        );
        view.reset_ssh_sessions(cx);
        assert!(!view.copy_original_transcript(id, old, cx));
        assert!(!view.toggle_raw_transcript(id, old, cx));
        assert!(view
            .sessions
            .get(id)
            .unwrap()
            .export_markdown()
            .contains(&original("Message")));
    });
}

// SDTEST-2846 / SDUC-475, SDUC-564
#[::core::prelude::v1::test]
fn long_tool_replays_preserve_retained_order_active_work_and_late_failures() {
    for provider in [
        AgentProvider::Claude,
        AgentProvider::Codex,
        AgentProvider::Jcode,
        AgentProvider::DeepSeek,
    ] {
        let mut session = AgentSession::new(
            "Large tool sequence",
            AgentExecutionContext {
                provider,
                target: AgentTarget::Local,
                access: AgentAccessMode::ReadOnly,
                workdir: std::env::temp_dir().display().to_string(),
                model: None,
            },
            1,
        )
        .unwrap();
        session.begin_run("Inspect the workspace", 2).unwrap();
        session.apply_stream_event(AgentStreamEvent::Ready, 3, true);
        for index in 0..1000 {
            session.apply_stream_event(
                AgentStreamEvent::Trace(AgentTraceUpdate {
                    correlation_id: Some(format!("tool-{index}")),
                    detail: AgentTraceKind::Tool {
                        name: format!("Tool {index}"),
                        status: AgentTraceStatus::Succeeded,
                        summary: Some(format!("Output {index}")),
                    },
                }),
                4 + index,
                true,
            );
        }
        assert_eq!(session.trace.len(), MAX_AGENT_TRACE_EVENTS_PER_SESSION);
        for (index, status) in [
            (995, AgentTraceStatus::Failed),
            (996, AgentTraceStatus::Cancelled),
            (997, AgentTraceStatus::Pending),
            (999, AgentTraceStatus::Running),
        ] {
            session.apply_stream_event(
                AgentStreamEvent::TraceStatus {
                    correlation_id: format!("tool-{index}"),
                    status,
                    summary: Some(format!("Updated output {index}")),
                },
                1010,
                true,
            );
        }
        let grouped = group_completed_work(super::super::merged_timeline(&session));
        let retained: Vec<_> = grouped
            .iter()
            .flat_map(|item| match item {
                TimelineItem::Trace(trace) => vec![trace.clone()],
                TimelineItem::CompletedWork(traces) => traces.clone(),
                _ => vec![],
            })
            .collect();
        assert_eq!(
            retained, session.trace,
            "all retained details stay in source order for {provider:?}"
        );
        for index in [995, 996, 997, 999] {
            assert!(grouped.iter().any(|item| matches!(item, TimelineItem::Trace(trace) if trace.correlation_id.as_deref() == Some(&format!("tool-{index}")))),
                "unfinished/failed tool {index} must stay outside collapsed groups for {provider:?}");
        }
        session.apply_stream_event(AgentStreamEvent::Text("Partial answer".into()), 1011, true);
        session.apply_stream_event(
            AgentStreamEvent::Error("Late provider failure — retry remains possible".into()),
            1012,
            true,
        );
        session
            .finish(AgentSessionStatus::Failed, 1013, true)
            .unwrap();
        let grouped = group_completed_work(super::super::merged_timeline(&session));
        assert!(
            matches!(grouped.last(), Some(TimelineItem::Message(message)) if message.role == AgentMessageRole::Error && message.text.contains("Late provider failure"))
        );
        assert_eq!(session.status, AgentSessionStatus::Failed);
        let restored: AgentSession = serde_json::from_str(&session.export_json().unwrap()).unwrap();
        let restored_order: Vec<_> = group_completed_work(super::super::merged_timeline(&restored))
            .iter()
            .map(TimelineItem::sequence)
            .collect();
        assert_eq!(
            restored_order,
            grouped
                .iter()
                .map(TimelineItem::sequence)
                .collect::<Vec<_>>(),
            "durable reload cannot reorder the conversation for {provider:?}"
        );
    }
}

// SDTEST-2847 / SDUC-563, SDUC-564
#[::core::prelude::v1::test]
fn encoded_transcript_corpus_preserves_context_and_streaming_originals() {
    let data = "aB3+/xY9".repeat(64);
    for width in [64, 76, 128, 512] {
        for prefix in ["", "+", "-", "    "] {
            for newline in ["\n", "\r\n"] {
                let encoded = data
                    .as_bytes()
                    .chunks(width)
                    .map(|chunk| {
                        format!("{prefix}{}{newline}", std::str::from_utf8(chunk).unwrap())
                    })
                    .collect::<String>();
                let source = format!(
                    "Before · é🌍{newline}{encoded}After: <tag> x < y; **keep this**{newline}"
                );
                let folded = fold_encoded(&source, |_| "[hidden]".into()).unwrap();
                assert_eq!(
                    folded.matches("[hidden]").count(),
                    1,
                    "width={width}, prefix={prefix:?}, newline={newline:?}"
                );
                assert!(folded.starts_with("Before · é🌍"));
                assert!(folded.contains("After: <tag> x < y; **keep this**"));
                assert!(
                    !folded.contains("aB3+/xY9"),
                    "encoded tail leaked: width={width}, prefix={prefix:?}, newline={newline:?}"
                );
            }
        }
    }
    let source = format!("{data}\ndeadbeef and readable prose\n");
    assert!(fold_encoded(&source, |_| "[hidden]".into())
        .unwrap()
        .ends_with("\ndeadbeef and readable prose\n"));
    for text in [
        format!("sha256: {}", "ab12".repeat(16)),
        "<script>alert('inert source')</script>\nx < y && a > b\n```rust\nlet v = 42;\n```".into(),
        format!(
            "Read [the file](https://example.com/src/main.rs) at commit {}",
            "e1".repeat(20)
        ),
    ] {
        assert!(fold_encoded(&text, |_| "[hidden]".into()).is_none());
    }
    let mut session = AgentSession::new(
        "Streaming image",
        AgentExecutionContext {
            provider: AgentProvider::Codex,
            target: AgentTarget::Local,
            access: AgentAccessMode::ReadOnly,
            workdir: std::env::temp_dir().display().to_string(),
            model: None,
        },
        1,
    )
    .unwrap();
    session.begin_run("Inspect the image", 2).unwrap();
    for chunk in data.as_bytes().chunks(37) {
        session.apply_stream_event(
            AgentStreamEvent::TextDelta(std::str::from_utf8(chunk).unwrap().into()),
            3,
            true,
        );
    }
    let text = &session.messages.last().unwrap().text;
    assert!(fold_encoded(text, |_| "[hidden]".into()).is_some());
    assert_eq!(
        text, &data,
        "display folding never truncates a streamed answer"
    );
    assert!(session.export_markdown().contains(&data));
}
