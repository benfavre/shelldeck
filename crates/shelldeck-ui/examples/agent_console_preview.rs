//! Isolated, provider-free native visual fixture for the Agents cockpit.
//! Set all three XDG homes to a disposable directory before running.
use adabraka_ui::prelude::*;
use gpui::*;
use shelldeck_core::agent_runtime::*;
use shelldeck_core::agent_session::{storage::*, *};
use shelldeck_ui::agent_console_view::AgentConsoleView;
use std::{borrow::Cow, path::PathBuf, time::Duration};

struct Assets;
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
    let mut snapshot = AgentSessionSnapshot {
        collection,
        ..Default::default()
    };
    snapshot.preferences.inspector_visible = std::env::var_os("SHELLDECK_PREVIEW_PANEL").is_some();
    snapshot.drafts.insert(
        first,
        "Add coverage for a reconnect while the page is hidden.".into(),
    );
    AgentSessionStore::default().save(&snapshot)?;
    let width = std::env::var("SHELLDECK_PREVIEW_WIDTH")
        .ok()
        .and_then(|v| v.parse::<f32>().ok())
        .unwrap_or(1280.0);
    Application::new().with_assets(Assets).run(move |cx| {
        adabraka_ui::init(cx);
        adabraka_ui::components::input::init(cx);
        adabraka_ui::set_icon_base_path("icons/lucide");
        install_theme(cx, shelldeck_ui::theme::adabraka_theme_from_palette());
        shelldeck_ui::apply_ui_language(&shelldeck_core::config::app_config::UiLanguage::En);
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
                cx.new(|cx| {
                    let mut view = AgentConsoleView::new(cx);
                    view.set_surface_visible(true, cx);
                    view
                })
            },
        )
        .unwrap();
        cx.spawn(async move |cx| {
            cx.background_executor()
                .timer(Duration::from_secs(45))
                .await;
            let _ = cx.update(|cx| cx.quit());
        })
        .detach();
    });
    Ok(())
}
