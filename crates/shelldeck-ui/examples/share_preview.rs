//! Isolated native Share rendering. No account, network or real files.
#[cfg(debug_assertions)]
fn main() -> anyhow::Result<()> {
    use gpui::*;
    use shelldeck_ui::share_view::ShareView;
    use std::{borrow::Cow, path::PathBuf, time::Duration};
    struct PreviewRoot(Entity<ShareView>);
    impl Render for PreviewRoot {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div()
                .size_full()
                .font_family("sans-serif")
                .child(self.0.clone())
        }
    }
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
            Ok(vec![])
        }
    }
    for key in ["XDG_CONFIG_HOME", "XDG_DATA_HOME", "XDG_CACHE_HOME"] {
        anyhow::ensure!(std::env::var_os(key).is_some_and(|value|std::path::Path::new(&value).starts_with(std::env::temp_dir())),"Disposable XDG profile required");
    }
    let width = std::env::var("SHELLDECK_PREVIEW_WIDTH")
        .ok()
        .and_then(|v| v.parse::<f32>().ok())
        .unwrap_or(1100.0);
    let height = std::env::var("SHELLDECK_PREVIEW_HEIGHT")
        .ok()
        .and_then(|v| v.parse::<f32>().ok())
        .unwrap_or(800.0);
    let snapshot = serde_json::from_value(serde_json::json!({
        "owner":{"email":"demo@example.invalid","name":"Aperçu"},
        "plan":{"id":"free","label":"Gratuit","max_file_bytes":104857600,"storage_bytes":1073741824,"max_files":1000,"retention_days":[1,7,30],"residency":"global","storage_ready":true},
        "usage":{"bytes":2457600,"files":2},"default_retention_seconds":604800,"list_limit":500,
        "files":[{"id":"abcdef","filename":"Rapport de visite.pdf","bytes":2400000,"content_type":"application/pdf","kind":"pdf","created_at":"2026-11-06T12:00:00Z","expires_at":"2026-11-13T12:00:00Z","views":3,"url":"https://share.inklura.fr/r/abcdef.pdf","viewer_url":"https://share.inklura.fr/s/abcdef"},
        {"id":"fedcba","filename":"Capture annotée.png","bytes":57600,"content_type":"image/png","kind":"image","created_at":"2026-11-06T12:00:00Z","expires_at":"2026-11-13T12:00:00Z","views":1,"url":"https://share.inklura.fr/r/fedcba.png","viewer_url":"https://share.inklura.fr/s/fedcba"}]
    }))?;
    Application::new().with_assets(Assets).run(move |cx| {
        adabraka_ui::init(cx);
        adabraka_ui::components::input::init(cx);
        adabraka_ui::set_icon_base_path("icons/lucide");
        adabraka_ui::theme::install_theme(cx, shelldeck_ui::theme::adabraka_theme_from_palette());
        shelldeck_ui::apply_ui_language(&shelldeck_core::config::app_config::UiLanguage::Fr);
        cx.open_window(
            WindowOptions {
                titlebar: Some(TitlebarOptions {
                    title: Some("Inklura Desktop Share — isolated fixture".into()),
                    ..Default::default()
                }),
                window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                    point(px(0.0), px(0.0)),
                    size(px(width), px(height)),
                ))),
                ..Default::default()
            },
            |window, cx| {
                window.set_rem_size(px(16.0));
                let share = cx.new(|cx| {
                    ShareView::preview(
                        snapshot,
                        (0..20)
                            .map(|n| PathBuf::from(format!("Rapport-de-visite-{n}.pdf")))
                            .collect(),
                        cx,
                    )
                });
                cx.new(|_| PreviewRoot(share))
            },
        )
        .unwrap();
        cx.spawn(async move |cx| {
            cx.background_executor()
                .timer(Duration::from_secs(25))
                .await;
            let _ = cx.update(|cx| cx.quit());
        })
        .detach();
    });
    Ok(())
}
#[cfg(not(debug_assertions))]
fn main() {
    eprintln!("This isolated preview requires a debug build");
}
