//! Isolated, provider-free native visual fixture for the Agents cockpit.
//! Set all three XDG homes to a disposable directory before running.
use adabraka_ui::prelude::*;
use gpui::*;
use shelldeck_core::agent_remote_history::{
    RemoteHistoryEntry, RemoteHistoryKind, RemoteHistoryPage, RemoteHistoryPosition,
    RemoteHistoryQuery, REMOTE_HISTORY_PAGE_SIZE,
};
use shelldeck_core::agent_runtime::*;
use shelldeck_core::agent_session::{storage::*, *};
use shelldeck_ui::agent_console_view::{
    save_local_document, AgentConsoleEvent, AgentConsoleView, AgentFileOrigin, AgentProjectGroup,
    AgentWorktreeOption,
};
use std::{borrow::Cow, path::PathBuf, time::Duration};

struct Assets;
struct PreviewRoot(Entity<AgentConsoleView>, Option<PreviewProfile>);

// Opt-in native frame-cadence measurement; never touches provider or desktop input.
struct PreviewProfile {
    start: std::time::Instant,
    last: Option<std::time::Instant>,
    samples: Vec<Duration>,
    frames: usize,
    finished: bool,
}
impl PreviewProfile {
    fn new() -> Self {
        Self {
            start: std::time::Instant::now(),
            last: None,
            samples: Vec::new(),
            frames: 0,
            finished: false,
        }
    }
}

impl Render for PreviewRoot {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if let Some(profile) = &mut self.1 {
            if !profile.finished {
                let now = std::time::Instant::now();
                if profile.start.elapsed() >= Duration::from_secs(2) {
                    if let Some(last) = profile.last.replace(now) {
                        profile.samples.push(now.duration_since(last));
                    }
                    profile.frames += 1;
                }
                if profile.frames >= 180 {
                    profile.finished = true;
                    profile.samples.sort();
                    eprintln!(
                        "native large-conversation frame cadence: {} frames, p50={:?}, p95={:?}, max={:?}",
                        profile.samples.len(),
                        profile.samples[profile.samples.len() / 2],
                        profile.samples[profile.samples.len() * 95 / 100],
                        profile.samples.last().unwrap()
                    );
                } else {
                    let entity = cx.entity_id();
                    window.on_next_frame(move |_, cx| cx.notify(entity));
                }
            }
        }
        // The production Workspace supplies the UI font; standalone fixtures must too.
        div()
            .size_full()
            .font_family(shelldeck_ui::settings::normalize_ui_font_family(
                "Inter", cx,
            ))
            .child(self.0.clone())
    }
}
impl AssetSource for Assets {
    fn load(&self, path: &str) -> anyhow::Result<Option<Cow<'static, [u8]>>> {
        Ok(std::fs::read(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../shelldeck/assets")
                .join(path),
        )
        .ok()
        .map(Cow::Owned))
    }
    fn list(&self, _: &str) -> anyhow::Result<Vec<SharedString>> {
        Ok(Vec::new())
    }
}
fn main() -> anyhow::Result<()> {
    for key in ["XDG_CONFIG_HOME", "XDG_DATA_HOME", "XDG_CACHE_HOME"] {
        let value = std::env::var_os(key)
            .ok_or_else(|| anyhow::anyhow!("{key} must name a disposable profile"))?;
        anyhow::ensure!(
            std::path::Path::new(&value).starts_with(std::env::temp_dir()),
            "Use a disposable profile under the temporary directory"
        );
    }
    let root = PathBuf::from(std::env::var_os("XDG_DATA_HOME").unwrap()).join("preview-workspace");
    std::fs::create_dir_all(root.join("src"))?;
    std::fs::write(
        root.join("src/main.rs"),
        "fn main() { println!(\"Preview fixture\"); }\n",
    )?;
    std::fs::write(
        root.join("README.md"),
        "# Preview fixture\nNo provider or remote is contacted.\n",
    )?;
    let mut collection = AgentSessionCollection::default();
    let context = AgentExecutionContext {
        provider: AgentProvider::Codex,
        target: AgentTarget::Local,
        access: AgentAccessMode::ReadOnly,
        workdir: root.display().to_string(),
        model: None,
    };
    let first = collection.create("Improve the notifications flow", context.clone(), 1)?;
    collection.begin_run(
        first,
        "Audit the notifications flow, preserve the active connection, and verify navigation.",
        2,
    )?;
    for event in [
        AgentStreamEvent::Ready,
        AgentStreamEvent::Text("The notification connection survives navigation. I’m checking the lifecycle and unread state next.".into()),
        AgentStreamEvent::Trace(AgentTraceUpdate {correlation_id:Some("read".into()),detail:AgentTraceKind::FileRead {path:"src/main.rs".into(),status:AgentTraceStatus::Succeeded,line_start:Some(1),line_end:Some(24)}}),
        AgentStreamEvent::Trace(AgentTraceUpdate {correlation_id:Some("test".into()),detail:AgentTraceKind::Test {name:"Notification lifecycle".into(),status:AgentTraceStatus::Succeeded,summary:Some("Three navigations, one connection, no extra fetches.".into())}}),
        AgentStreamEvent::Text("Updated the lifecycle to retain the connection across navigation. The focused checks pass.\n\nYou can inspect the files, review changes, or prepare the next task below.".into()),
        AgentStreamEvent::Plan(AgentPlanUpdate {steps:vec![AgentPlanStep{text:"Inspect connection ownership".into(),status:AgentPlanStepStatus::Completed},AgentPlanStep{text:"Verify navigation and unread state".into(),status:AgentPlanStepStatus::Completed}]}),
        AgentStreamEvent::Usage(AgentUsageUpdate {input_tokens:Some(12480),output_tokens:Some(1960),context_used:Some(14440),context_size:Some(200000),..Default::default()}),
    ] { collection.apply_stream_event(first,event,3)?; }
    collection.finish(first, AgentSessionStatus::Completed, 4)?;
    collection.set_pinned(first, true, 5)?;
    collection.create("Review the terminal shortcuts", context.clone(), 6)?;
    collection.create("Investigate SSH reconnects", context, 7)?;
    collection.select(first)?;
    if std::env::var_os("SHELLDECK_PREVIEW_GROUPS").is_some() {
        let template = collection.get(first).unwrap().context.clone();
        for (target, path, name) in [
            (
                AgentTarget::Local,
                root.parent()
                    .unwrap()
                    .join("image-tools")
                    .display()
                    .to_string(),
                "Resize the product screenshots",
            ),
            (
                AgentTarget::Ssh {
                    connection_id: uuid::Uuid::from_u128(421),
                    connection_label: "Development server".into(),
                },
                "/srv/customer-project".into(),
                "Continue the remote deployment review",
            ),
        ] {
            collection.create(
                name,
                AgentExecutionContext {
                    target,
                    workdir: path,
                    ..template.clone()
                },
                10,
            )?;
        }
        collection.select(first)?;
    }
    if std::env::var_os("SHELLDECK_PREVIEW_MISSING_SSH").is_some() {
        let session = collection.get_mut(first).unwrap();
        let mut context = session.context.clone();
        context.target = AgentTarget::Ssh {
            connection_id: uuid::Uuid::from_u128(777),
            connection_label: "Removed server".into(),
        };
        context.workdir = "/srv/customer project".into();
        session.set_context(context, 8)?;
        session.rename("Recover missing SSH", 9)?;
    }
    if std::env::var_os("SHELLDECK_PREVIEW_BLOBS").is_some() {
        collection.reconnect_remote(
            "Inspect the saved image diff",
            AgentExecutionContext {
                provider: AgentProvider::Codex,
                target: AgentTarget::Ssh {
                    connection_id: uuid::Uuid::from_u128(421),
                    connection_label: "Development server".into(),
                },
                access: AgentAccessMode::ReadOnly,
                workdir: "/srv/customer-project".into(),
                model: None,
            },
            "fixture-conversation-1",
            8,
        )?;
    }
    if std::env::var_os("SHELLDECK_PREVIEW_PERF").is_some() {
        let session = collection.get_mut(first).unwrap();
        session.rename("Large conversation performance", 10)?;
        for index in 0..80 {
            session.begin_run(format!("Inspect fixture exchange {index}"), 20 + index * 4)?;
            session.apply_stream_event(
                AgentStreamEvent::Trace(AgentTraceUpdate {
                    correlation_id: Some(format!("fixture-{index}")),
                    detail: AgentTraceKind::Command {
                        command: format!("read fixture-{index}.rs"),
                        status: AgentTraceStatus::Succeeded,
                        exit_code: Some(0),
                        summary: Some("A retained command result\n".repeat(12)),
                    },
                }),
                21 + index * 4,
                true,
            );
            session.apply_stream_event(AgentStreamEvent::Text(format!("### Result {index}\n\n{}", "Readable source and **retained context** with `inline code` and a useful explanation.\n\n".repeat(12))),22+index*4,true);
            session.finish(AgentSessionStatus::Completed, 23 + index * 4, true)?;
        }
    }
    let mut snapshot = AgentSessionSnapshot {
        collection,
        ..Default::default()
    };
    if std::env::var_os("SHELLDECK_PREVIEW_COLLAPSED").is_some() {
        snapshot.preferences.collapsed_project_ids = vec!["preview-project".into()];
    }
    snapshot.preferences.inspector_visible = std::env::var_os("SHELLDECK_PREVIEW_PANEL").is_some();
    snapshot.drafts.insert(
        first,
        if std::env::var_os("SHELLDECK_PREVIEW_CROPPED_TRACE").is_some() {
            "open browser check inklura.fr".into()
        } else {
            "Add coverage for a reconnect while the page is hidden.".into()
        },
    );
    snapshot.preferences.remote_sessions_expanded =
        std::env::var_os("SHELLDECK_PREVIEW_CONNECT").is_some();
    AgentSessionStore::default().save(&snapshot)?;
    let preview_remote = std::env::var_os("SHELLDECK_PREVIEW_REMOTE").is_some();
    let preview_root = root.clone();
    let preview_editor = std::env::var_os("SHELLDECK_PREVIEW_EDITOR").is_some();
    let editor_path = root.join("src/main.rs");
    let preview_seconds = std::env::var("SHELLDECK_PREVIEW_SECONDS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(45)
        .clamp(5, 900);
    let width = std::env::var("SHELLDECK_PREVIEW_WIDTH")
        .ok()
        .and_then(|v| v.parse::<f32>().ok())
        .unwrap_or(1280.0);
    Application::new().with_assets(Assets).run(move |cx| {
        adabraka_ui::init(cx);
        adabraka_ui::components::input::init(cx);
        adabraka_ui::set_icon_base_path("icons/lucide");
        if std::env::var_os("SHELLDECK_PREVIEW_LIGHT").is_some() {
            shelldeck_ui::theme::ShellDeckColors::set_theme(&shelldeck_core::config::app_config::ThemePreference::SolarizedLight);
        }
        install_theme(cx, shelldeck_ui::theme::adabraka_theme_from_palette());
        shelldeck_ui::apply_ui_language(&if std::env::var_os("SHELLDECK_PREVIEW_FR").is_some() {
            shelldeck_core::config::app_config::UiLanguage::Fr
        } else {
            shelldeck_core::config::app_config::UiLanguage::En
        });
        cx.open_window(
            WindowOptions {
                titlebar: Some(TitlebarOptions {
                    title: Some("ShellDeck Agents — isolated preview".into()),
                    ..Default::default()
                }),
                window_bounds: Some(WindowBounds::Windowed(Bounds {
                    origin: point(px(0.0), px(0.0)),
                    size: size(px(width), px(900.0)),
                })),
                ..Default::default()
            },
            |_, cx| {
                let remote = cx.new(shelldeck_ui::fleet_view::FleetView::new);
                if preview_remote {
                    remote.update(cx, |view, _| view.set_snapshot(remote_fixture()));
                }
                let view = cx.new(|cx| {
                    let mut view = AgentConsoleView::new(cx);
                    view.set_surface_visible(true, cx);
                    view.set_project_groups(
                        vec![AgentProjectGroup {
                            id: "preview-project".into(),
                            label: "ShellDeck".into(),
                            worktrees: vec![AgentWorktreeOption {
                                id: "preview-main".into(),
                                label: "ShellDeck".into(),
                                path: preview_root.display().to_string(),
                                branch: None,
                                is_primary: true,
                                target: AgentTarget::Local,
                            }],
                        }],
                        cx,
                    );
                    if preview_remote {
                        view.set_remote_session_source(remote.clone(), cx);
                        view.set_remote_sessions_available(true, cx);
                    }
                    view
                });
                if preview_remote {
                    cx.subscribe(&remote, |remote, event: &shelldeck_ui::fleet_view::FleetViewEvent, cx| {
                        if let shelldeck_ui::fleet_view::FleetViewEvent::CheckSession(session) = event {
                            remote.update(cx, |view,cx| {
                                view.set_remote_capabilities(remote_fixture().capabilities);
                                view.set_remote_connection_results(vec![remote_connection_fixture(session.clone())]);
                                cx.notify();
                            });
                        }
                        if let shelldeck_ui::fleet_view::FleetViewEvent::FollowUp(follow_up) = event {
                            remote.update(cx, |view,cx| {
                                view.set_follow_up_result(shelldeck_core::config::platform::PlatformFollowUpResult::ReconciliationPending {
                                    follow_up: follow_up.clone(), category: "fixture-no-provider".into(),
                                },cx);
                            });
                        }
                    }).detach();
                }
                if std::env::var_os("SHELLDECK_PREVIEW_CONNECT").is_some() {
                    let selected = remote_fixture().sessions[9].session.resource.clone();
                    remote.update(cx, |remote,cx| { remote.open_session_exact(&selected,cx); cx.emit(shelldeck_ui::fleet_view::FleetViewEvent::CheckSession(selected.clone())); });
                    view.update(cx, |view,cx| view.set_remote_session_active(true,cx));
                }
                // This fixture has no Workspace orchestrator. Its Save adapter
                // accepts only local documents in the disposable fixture root.
                let fixture_root = editor_path
                    .parent()
                    .unwrap()
                    .parent()
                    .unwrap()
                    .to_path_buf();
                cx.subscribe(&view, move |view, event: &AgentConsoleEvent, cx| {
                    if std::env::var_os("SHELLDECK_PREVIEW_ACTIVITY").is_some() {
                        if let AgentConsoleEvent::Run(request) = event {
                            view.update(cx, |view,cx| {
                                view.begin_run(request.clone(),cx);
                                view.push_stream_event_for(request.id, AgentStreamEvent::Ready, cx);
                                for index in 0..10 {
                                    view.push_stream_event_for(request.id,AgentStreamEvent::Trace(AgentTraceUpdate {correlation_id:Some(format!("operation-{index}")),detail:AgentTraceKind::Command {command:format!("/bin/bash -lc \"python3 - <<'PY'\nfrom pathlib import Path\nfrom collections import Counter, defaultdict\nroot = Path('fixture-{index}')\nprint(root)\nPY\""), status:AgentTraceStatus::Succeeded,exit_code:Some(0),summary:Some("Retained fixture output, every original line remains available.\n".repeat(8))}}),cx);
                                }
                                view.push_stream_event_for(request.id,AgentStreamEvent::Trace(AgentTraceUpdate {correlation_id:Some("active-operation".into()),detail:AgentTraceKind::Command {command:"Inspect the conversation layout".into(),status:AgentTraceStatus::Running,exit_code:None,summary:Some("Checking the running layout and its readable status.".into())}}),cx);
                            });
                            let running = view.downgrade();
                            let run = request.id;
                            cx.spawn(async move |cx| {
                                cx.background_executor().timer(Duration::from_secs(35)).await;
                                let _ = running.update(cx, |view,cx| view.finish_run(run,Some(0),None,cx));
                            }).detach();
                        }
                    }
                    if std::env::var_os("SHELLDECK_PREVIEW_CROPPED_TRACE").is_some() {
                        if let AgentConsoleEvent::Run(request) = event {
                            view.update(cx, |view, cx| {
                                view.begin_run(request.clone(), cx);
                                view.push_stream_events_for(request.id, [
                                    AgentStreamEvent::Ready,
                                    AgentStreamEvent::Trace(AgentTraceUpdate {
                                        correlation_id: Some("preview-curl".into()),
                                        detail: AgentTraceKind::Command {
                                            command: "curl -sS -o /dev/null -w 'home: %{http_code} %{time_total}s %{size_download}B via %{remote_ip}' -L https://inklura.fr".into(),
                                            status: AgentTraceStatus::Succeeded, exit_code: Some(0), summary: None,
                                        },
                                    }),
                                    AgentStreamEvent::Trace(AgentTraceUpdate {
                                        correlation_id: Some("preview-browser-warning".into()),
                                        detail: AgentTraceKind::Command {
                                            command: "/home/fixture/.cache/browser/chrome/linux-152.0.7977.75/chrome-linux64/chrome --headless --remote-debugging-port=0 about:blank".into(),
                                            status: AgentTraceStatus::Running, exit_code: None, summary: None,
                                        },
                                    }),
                                    AgentStreamEvent::Usage(AgentUsageUpdate { input_tokens:Some(18420), output_tokens:Some(3), ..Default::default() }),
                                ], cx);
                            });
                        }
                    }
                    if let AgentConsoleEvent::OpenPlatformSession(coordinate) = event {
                        let opened = remote
                            .update(cx, |source, cx| {
                                let opened = source.open_session_exact(coordinate, cx);
                                if opened { cx.emit(shelldeck_ui::fleet_view::FleetViewEvent::CheckSession(coordinate.clone())); }
                                opened
                            });
                        if opened {
                            view.update(cx, |view, cx| view.set_remote_session_active(true, cx));
                        }
                    }
                    if let AgentConsoleEvent::DiscoverSshSessions {connection_id,request_id,cursor} = event {
                        view.update(cx,|view,cx|view.set_ssh_sessions(*connection_id,*request_id,Ok(ssh_sessions_page_fixture(cursor.as_deref())),cx));
                    }
                    if let AgentConsoleEvent::ReconnectSshSession {request_id,session,..} = event {
                        let page = native_history_fixture(session, &RemoteHistoryQuery::default());
                        view.update(cx,|view,cx|view.complete_ssh_reconnect(*request_id,page,cx));
                    }
                    if let AgentConsoleEvent::LoadSshHistory(request) = event {
                        let page = native_history_fixture(&request.remote, &request.query);
                        view.update(cx, |view,cx|view.complete_native_history(request.clone(),page,cx));
                    }
                    if let AgentConsoleEvent::SaveFile(request) = event {
                        let result = match &request.origin {
                            AgentFileOrigin::Local { root, .. } if root == &fixture_root => {
                                save_local_document(
                                    &request.context.workdir,
                                    request.origin.clone(),
                                    request.text.clone(),
                                )
                            }
                            _ => Err("Fixture refuses all non-fixture saves".to_string()),
                        };
                        view.update(cx, |view, cx| {
                            view.complete_agent_file_save(request, result, cx)
                        });
                    }
                })
                .detach();
                if std::env::var_os("SHELLDECK_PREVIEW_SSH").is_some() || std::env::var_os("SHELLDECK_PREVIEW_BLOBS").is_some() || std::env::var_os("SHELLDECK_PREVIEW_MISSING_SSH").is_some() {
                    let id=uuid::Uuid::from_u128(421);
                    view.update(cx,|view,cx|{
                        view.set_connections(vec![shelldeck_ui::agent_console_view::AgentConnectionOption{id,label:"Development server".into(),host:"dev.example.invalid".into()}],cx);
                        view.scan_ssh_sessions(id,cx);
                    });
                }
                if std::env::var_os("SHELLDECK_PREVIEW_ACTIVITY").is_some() || std::env::var_os("SHELLDECK_PREVIEW_PERF").is_some() {
                    let selected_view = view.downgrade();
                    cx.spawn(async move |cx| {
                        cx.background_executor().timer(Duration::from_millis(700)).await;
                        let _ = selected_view.update(cx, |view,cx| view.select_session(first,cx));
                    }).detach();
                }
                if preview_editor {
                    let editor_view = view.downgrade();
                    cx.spawn(async move |cx| {
                        cx.background_executor()
                            .timer(Duration::from_millis(500))
                            .await;
                        let _ = editor_view.update(cx, |view, cx| {
                            view.open_local_agent_preview(editor_path, cx)
                        });
                    })
                    .detach();
                }
                if std::env::var_os("SHELLDECK_PREVIEW_ATTENTION").is_some() {
                    let attention_view = view.downgrade();
                    cx.spawn(async move |cx| {
                        cx.background_executor().timer(Duration::from_millis(500)).await;
                        let _ = attention_view.update(cx, |view, cx| {
                            let pending = view.sessions().iter().find(|session| {
                                session.name == "Investigate SSH reconnects"
                            }).map(|session| (session.id, session.context.clone()));
                            if let Some((id, context)) = pending {
                                view.select_session(id, cx);
                                let request = context.run_request("Synthetic approval preview");
                                let run_id = request.id;
                                view.begin_run(request, cx);
                                view.push_stream_events_for(run_id, [
                                    AgentStreamEvent::Ready,
                                    AgentStreamEvent::Approval(AgentApprovalRequest {
                                        id: "preview-approval".into(), session_id: "preview".into(),
                                        tool_call_id: "preview-edit".into(),
                                        title: "Apply the proposed change?".into(), options: vec![],
                                    }),
                                ], cx);
                                view.select_session(first, cx);
                            }
                        });
                    }).detach();
                }
                cx.new(|_| PreviewRoot(view, std::env::var_os("SHELLDECK_PREVIEW_PROFILE").map(|_| PreviewProfile::new())))
            },
        )
        .unwrap();
        cx.spawn(async move |cx| {
            cx.background_executor()
                .timer(Duration::from_secs(preview_seconds))
                .await;
            let _ = cx.update(|cx| cx.quit());
        })
        .detach();
    });
    Ok(())
}

// Synthetic directory: no credentials, provider process, or remote network calls.
fn remote_fixture() -> shelldeck_core::config::platform::PlatformSnapshot {
    use automonique_protocol::platform::{Capabilities, CursorTopic, Freshness};
    use automonique_protocol::primitives::{EpochMillis, Revision};
    use shelldeck_core::config::platform::*;
    let sessions = (0..119)
        .map(|i| SessionRecord {
            session: ResourceRecord {
                resource: ResourceCoordinate::new(
                    ResourceAuthority::Automonique,
                    ResourceKind::Session,
                    ResourceId::new(format!("session_preview_{i:04}_demo{i:06}")).unwrap(),
                ),
                summary: PlatformText::new(if i % 8 == 0 { "closed" } else { "open" }).unwrap(),
                freshness: Freshness {
                    state: if i < 9 {
                        FreshnessState::Fresh
                    } else {
                        FreshnessState::Unknown
                    },
                    observed_at: EpochMillis::from_millis(1000 - i),
                    revision: Revision::FIRST,
                },
            },
            run: None,
            attachable: i < 9,
            controllable: false,
        })
        .collect();
    let cursor = PlatformCursor {
        authority: ResourceAuthority::Automonique,
        topic: CursorTopic::new("sessions").unwrap(),
        sequence: Revision::FIRST,
    };
    PlatformSnapshot {
        capabilities: Capabilities::platform_v1(),
        resources: vec![],
        cursor: cursor.clone(),
        sessions,
        sessions_cursor: cursor,
        view: PlatformView::default(),
    }
}

// Typed server reply for exercising connection/review without a real runtime.
fn remote_connection_fixture(
    session: shelldeck_core::config::platform::ResourceCoordinate,
) -> shelldeck_core::config::platform::RetainedSessionUpdate {
    use automonique_protocol::platform::{Freshness, SessionHistoryMessageText};
    use automonique_protocol::primitives::EpochMillis;
    use shelldeck_core::config::platform::*;
    let state = SessionCommandState::new(
        ResourceRecord {
            resource: session.clone(),
            summary: PlatformText::new("open").unwrap(),
            freshness: Freshness {
                state: FreshnessState::Fresh,
                observed_at: EpochMillis::from_millis(1),
                revision: Revision::FIRST,
            },
        },
        None,
        vec![],
    )
    .unwrap();
    let empty = std::env::var_os("SHELLDECK_PREVIEW_EMPTY_HISTORY").is_some();
    let events = if empty {
        vec![]
    } else {
        vec![SessionHistoryEvent::Message {
        cursor: 1, at: EpochMillis::from_millis(1), evidence: SessionHistoryEvidence::Authoritative,
        role: SessionHistoryRole::Assistant,
        text: SessionHistoryMessageText::new("The saved conversation is available. We were investigating the notification reconnect behavior.").unwrap(), truncated:false,
    }]
    };
    RetainedSessionUpdate {
        session: session.clone(),
        command_state: SessionCommandStateResult::State(state),
        history: RetainedHistoryUpdate::Replace {
            retention_gap: false,
            page: SessionHistoryPage::new(
                session,
                128,
                128,
                0,
                if empty { 0 } else { 1 },
                false,
                events,
            )
            .unwrap(),
        },
    }
}

fn ssh_sessions_fixture() -> shelldeck_core::agent_remote_sessions::RemoteAgentSessionInventory {
    use shelldeck_core::agent_remote_sessions::*;
    RemoteAgentSessionInventory {
        sessions: [
            AgentProvider::Claude,
            AgentProvider::Codex,
            AgentProvider::Jcode,
        ]
        .into_iter()
        .enumerate()
        .map(|(i, provider)| RemoteAgentSession {
            provider,
            session_id: format!("fixture-conversation-{i}"),
            workdir: "/srv/customer-project".into(),
            title: if std::env::var_os("SHELLDECK_PREVIEW_LONG_TITLE").is_some() && i == 0 {
                "Site : plante-artificielle-unique.com Thème : Rapport · visibilité locale à Rennes Date des faits : 2026-10-30 Chiffres de tête : - Ville : Rennes - Mots-clés relevés depuis Rennes : 2 visibles sur 20 mesurés, couverture 10 %, top 3".into()
            } else {
                [
                    "Investigate checkout failure",
                    "Improve the remote session picker",
                    "Review API latency",
                ][i]
                    .into()
            },
            updated_at_ms: 1793360000000,
            model: None,
        })
        .collect(),
        warnings: vec![],
        truncated: false,
        next_cursor: None,
    }
}

fn ssh_sessions_page_fixture(
    cursor: Option<&str>,
) -> shelldeck_core::agent_remote_sessions::RemoteAgentSessionInventory {
    let mut page = ssh_sessions_fixture();
    if cursor.is_none() {
        page.truncated = true;
        page.next_cursor = Some("fixture_page_2".into());
    } else {
        page.sessions.truncate(1);
        page.sessions[0].session_id = "fixture-older-conversation".into();
        page.sessions[0].title = "Older retained conversation".into();
    }
    page
}

// A complete synthetic three-page transcript. All callbacks stay local: clicking
// reconnect, loading older/newer pages and reloading cannot contact a provider.
fn native_history_fixture(
    remote: &shelldeck_core::agent_remote_sessions::RemoteAgentSession,
    query: &RemoteHistoryQuery,
) -> Result<RemoteHistoryPage, String> {
    let blobs = std::env::var_os("SHELLDECK_PREVIEW_BLOBS").is_some();
    let total = if blobs { 6u64 } else { 160u64 };
    let snapshot = format!("v1:{total}:{}", "a".repeat(64));
    if query
        .snapshot
        .as_ref()
        .is_some_and(|value| value != &snapshot)
    {
        return Err("The preview history snapshot changed. Reload history.".into());
    }
    let count = REMOTE_HISTORY_PAGE_SIZE as u64;
    let (start, end) = match query.position {
        RemoteHistoryPosition::Latest => (total.saturating_sub(count), total),
        RemoteHistoryPosition::Before(boundary) if boundary <= total => {
            (boundary.saturating_sub(count), boundary)
        }
        RemoteHistoryPosition::After(boundary) if boundary <= total => {
            (boundary, boundary.saturating_add(count).min(total))
        }
        _ => return Err("The preview history cursor is invalid.".into()),
    };
    let entries = (start..end)
        .map(|index| {
            let (kind, title, text) = if blobs {
                let data = "aB3+/xY9".repeat(40);
                match index {
                    0 => (RemoteHistoryKind::User, None, "Check the image changes and keep the conversation readable.".into()),
                    1 => (RemoteHistoryKind::Assistant, None, "The image is stored as encoded text. The tool details remain available below.".into()),
                    2 => (RemoteHistoryKind::Tool, Some("git diff -- assets/image.txt".into()), format!("diff --git a/assets/image.txt b/assets/image.txt\n@@ -1 +1,12 @@\n-old\n{}\nKeep this readable explanation.", (format!("+{data}\n")).repeat(12))),
                    3 => (RemoteHistoryKind::Reasoning, Some("Reasoning".into()), "The diff is image data; keep the patch available for inspection.".into()),
                    4 => (RemoteHistoryKind::Assistant, None, format!("Encoded attachment:\n\n{{\"image\":\"data:image/png;base64,{data}\"}}\n\nThe explanation after the attachment stays readable.")),
                    _ => (RemoteHistoryKind::Tool, Some("cargo check".into()), "Finished successfully.\nNo warnings in the changed code.".into()),
                }
            } else { match index % 4 {
                0 => (RemoteHistoryKind::User, None, format!("Saved request {}: investigate notification reconnects and preserve the selected workspace.", index / 4 + 1)),
                1 => (RemoteHistoryKind::Assistant, None, "I traced the saved connection to the original workspace. The retained messages and tool results are available below.".into()),
                2 => (RemoteHistoryKind::Tool, Some("Read src/notifications.rs".into()), format!("Saved tool result {}\nConnection remains attached across navigation.\nThe original workspace is /srv/customer-project.", index / 4 + 1)),
                _ => (RemoteHistoryKind::Assistant, None, "The focused checks passed. Reconnecting should restore the conversation before the next message is sent.".into()),
            }};
            RemoteHistoryEntry { index, kind, title, text, at_ms: Some(1793360000000 + index as i64 * 1000), part: 1, parts: 1 }
        })
        .collect();
    Ok(RemoteHistoryPage {
        provider: remote.provider,
        session_id: remote.session_id.clone(),
        workdir: remote.workdir.clone(),
        model: Some(
            match remote.provider {
                AgentProvider::Claude => "claude-fixture",
                AgentProvider::Codex => "codex-fixture",
                _ => "deepseek-fixture",
            }
            .into(),
        ),
        snapshot,
        total_entries: total,
        start,
        end,
        entries,
        warnings: Vec::new(),
    })
}
