//! Isolated visual fixture for the production terminal welcome surface.
//! XDG homes must point at a disposable profile under the temporary directory.
use gpui::*;
use shelldeck_core::config::app_config::{ThemePreference, UiLanguage};
use shelldeck_ui::{terminal_view::TerminalView, theme::ShellDeckColors};
use std::{borrow::Cow, path::PathBuf};

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

struct Preview(AnyView, SharedString);
impl Render for Preview {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .font_family(self.1.clone())
            .child(self.0.clone())
    }
}

fn main() -> anyhow::Result<()> {
    for key in ["XDG_CONFIG_HOME", "XDG_DATA_HOME", "XDG_CACHE_HOME"] {
        anyhow::ensure!(std::env::var_os(key).is_some_and(|value|
            std::path::Path::new(&value).starts_with(std::env::temp_dir())),
            "{key} must name a disposable profile under the temporary directory");
    }
    let number = |key, fallback| {
        std::env::var(key)
            .ok()
            .and_then(|value| value.parse::<f32>().ok())
            .unwrap_or(fallback)
    };
    let width = number("SHELLDECK_PREVIEW_WIDTH", 1280.0);
    let height = number("SHELLDECK_PREVIEW_HEIGHT", 1000.0);
    let scale = number("SHELLDECK_PREVIEW_SCALE", 1.0);
    Application::new().with_assets(Assets).run(move |cx| {
        adabraka_ui::init(cx);
        adabraka_ui::components::input::init(cx);
        adabraka_ui::set_icon_base_path("icons/lucide");
        ShellDeckColors::set_theme(&if std::env::var_os("SHELLDECK_PREVIEW_LIGHT").is_some() {
            ThemePreference::SolarizedLight
        } else {
            ThemePreference::Dark
        });
        adabraka_ui::theme::install_theme(cx, shelldeck_ui::theme::adabraka_theme_from_palette());
        shelldeck_ui::apply_ui_language(&if std::env::var_os("SHELLDECK_PREVIEW_FR").is_some() {
            UiLanguage::Fr
        } else {
            UiLanguage::En
        });
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds {
                    origin: point(px(0.0), px(0.0)),
                    size: size(px(width), px(height)),
                })),
                ..Default::default()
            },
            |window, cx| {
                window.set_rem_size(px(16.0 * scale));
                let view: AnyView =
                    if let Ok(base) = std::env::var("SHELLDECK_PREVIEW_AUTOMATION_BASE") {
                        cx.new(|cx| {
                            let mut view = shelldeck_ui::automations_view::AutomationsView::new(cx);
                            if std::env::var_os("SHELLDECK_PREVIEW_AUTOMATION_LOCAL").is_some() {
                                let agents = cx.new(shelldeck_ui::agent_console_view::AgentConsoleView::new);
                                agents.update(cx, |agents, cx| {
                                    agents.set_workflows_authorized(true, cx);
                                    agents.create_session("Weekly research", shelldeck_core::agent_session::AgentExecutionContext {
                                        provider: shelldeck_core::agent_runtime::AgentProvider::Codex,
                                        target: shelldeck_core::agent_runtime::AgentTarget::Local,
                                        access: shelldeck_core::agent_runtime::AgentAccessMode::ReadOnly,
                                        workdir: std::env::temp_dir().to_string_lossy().into_owned(),
                                        model: None,
                                    }, cx).expect("private readonly fixture session");
                                });
                                view.set_local_agents(agents, cx);
                            }
                            view.set_context(
                                Some(shelldeck_ui::automations_view::AutomationContext {
                                    base,
                                    token: "sd_preview".into(),
                                    site: "preview-site".into(),
                                }),
                                "Preview workspace".into(),
                                true,
                                cx,
                            );
                            if std::env::var_os("SHELLDECK_PREVIEW_AUTOMATION_FORM").is_some() {
                                view.start_task(Some(0), cx);
                            }
                            view
                        })
                        .into()
                    } else {
                        cx.new(TerminalView::new).into()
                    };
                let font = shelldeck_ui::settings::normalize_ui_font_family("Inter", cx).into();
                cx.new(|_| Preview(view, font))
            },
        )
        .expect("open terminal welcome preview");
        cx.activate(true);
    });
    Ok(())
}
