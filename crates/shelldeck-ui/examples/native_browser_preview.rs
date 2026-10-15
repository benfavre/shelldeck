//! Isolated native browser fixture. No provider or external website is used.
//! Stage the packaged helper beside this executable (browser/shelldeck-browser)
//! and set all three XDG homes to disposable directories before running.
use adabraka_ui::prelude::*;
use gpui::*;
use shelldeck_ui::native_browser::{NativeBrowserEvent, NativeBrowserView};
use std::{
    borrow::Cow,
    io::{Read, Write},
    net::TcpListener,
    path::PathBuf,
    time::{Duration, Instant},
};

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

struct PreviewRoot {
    browser: Entity<NativeBrowserView>,
    font: String,
}
impl Render for PreviewRoot {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .font_family(self.font.clone())
            .child(self.browser.clone())
    }
}

fn main() -> anyhow::Result<()> {
    for key in ["XDG_CONFIG_HOME", "XDG_DATA_HOME", "XDG_CACHE_HOME"] {
        let directory = std::env::var_os(key)
            .ok_or_else(|| anyhow::anyhow!("{key} must name a disposable profile"))?;
        anyhow::ensure!(
            std::path::Path::new(&directory).starts_with(std::env::temp_dir()),
            "Use a disposable profile under the temporary directory"
        );
    }
    let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))?;
    let url = format!("http://{}/", listener.local_addr()?);
    eprintln!("Native browser fixture: {url}");
    std::thread::spawn(move || {
        for mut stream in listener.incoming().flatten() {
            let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
            let mut request = [0; 8192];
            if stream.read(&mut request).is_err() {
                continue;
            }
            let body = FIXTURE.as_bytes();
            let header = format!("HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\n\r\n", body.len());
            let _ = stream.write_all(header.as_bytes());
            let _ = stream.write_all(body);
        }
    });
    let width = std::env::var("SHELLDECK_PREVIEW_WIDTH")
        .ok()
        .and_then(|value| value.parse::<f32>().ok())
        .unwrap_or(1100.0)
        .clamp(480.0, 1920.0);
    let seconds = std::env::var("SHELLDECK_PREVIEW_SECONDS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(60)
        .clamp(5, 300);
    let audit = std::env::var_os("SHELLDECK_PREVIEW_AUDIT").is_some();
    // Give the bounded audit time to fail explicitly even if the caller asks
    // for a short visual preview. An early window close still fails below.
    let seconds = if audit { seconds.max(30) } else { seconds };
    let failed = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let audit_failed = failed.clone();
    let completed = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(!audit));
    let audit_completed = completed.clone();
    Application::new().with_assets(Assets).run(move |cx| {
        adabraka_ui::init(cx);
        adabraka_ui::components::input::init(cx);
        adabraka_ui::set_icon_base_path("icons/lucide");
        install_theme(cx, shelldeck_ui::theme::adabraka_theme_from_palette());
        shelldeck_ui::apply_ui_language(&shelldeck_core::config::app_config::UiLanguage::En);
        cx.open_window(
            WindowOptions {
                titlebar: Some(TitlebarOptions {
                    title: Some("ShellDeck Browser — isolated fixture".into()),
                    ..Default::default()
                }),
                window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                    point(px(40.0), px(40.0)),
                    size(px(width), px(800.0)),
                ))),
                ..Default::default()
            },
            |window, cx| {
                window.set_rem_size(px(16.0));
                let view = cx
                    .new(|cx| NativeBrowserView::new("native-browser-isolated-fixture".into(), cx));
                cx.subscribe(&view, |_, event: &NativeBrowserEvent, _| match event {
                    NativeBrowserEvent::Ready => eprintln!("Native browser fixture connected"),
                    NativeBrowserEvent::Error(error) => {
                        eprintln!("Native browser fixture error: {error}")
                    }
                    NativeBrowserEvent::ShareScreenshot { png, .. } => {
                        if let Some(directory) = std::env::var_os("XDG_CACHE_HOME") {
                            let output = PathBuf::from(directory).join("browser-shared.png");
                            let _ = std::fs::create_dir_all(output.parent().unwrap());
                            if let Err(error) = std::fs::write(&output, png) {
                                eprintln!("Could not save fixture capture: {error}");
                            } else {
                                eprintln!("Native browser screenshot: {}", output.display());
                            }
                        }
                    }
                    _ => {}
                })
                .detach();
                view.update(cx, |view, cx| view.navigate(url, cx));
                if audit {
                    let view = view.downgrade();
                    let failed = audit_failed.clone();
                    let completed = audit_completed.clone();
                    cx.spawn(async move |cx| {
                        let deadline = Instant::now() + Duration::from_secs(25);
                        let result = loop {
                            match view
                                .update(cx, |view, _| (view.control(), view.active_browser_id()))
                            {
                                Ok((Some(client), Some(id))) => {
                                    break cx
                                        .background_executor()
                                        .spawn(async move { audit_browser(client, id, deadline) })
                                        .await;
                                }
                                Err(error) => {
                                    break Err(anyhow::anyhow!(
                                        "browser fixture closed before its audit: {error}"
                                    ))
                                }
                                _ if Instant::now() >= deadline => {
                                    break Err(anyhow::anyhow!(
                                        "browser fixture did not connect before the audit deadline"
                                    ))
                                }
                                _ => {
                                    cx.background_executor()
                                        .timer(Duration::from_millis(100))
                                        .await
                                }
                            }
                        };
                        match result {
                            Ok(()) => {
                                completed.store(true, std::sync::atomic::Ordering::SeqCst);
                                eprintln!("Native browser fixture audit passed");
                            }
                            Err(error) => {
                                eprintln!("Native browser fixture audit failed: {error:#}");
                                failed.store(true, std::sync::atomic::Ordering::SeqCst);
                                let _ = cx.update(|cx| cx.quit());
                            }
                        }
                    })
                    .detach();
                }
                let font = shelldeck_ui::settings::normalize_ui_font_family("Inter", cx);
                cx.new(|_| PreviewRoot {
                    browser: view,
                    font,
                })
            },
        )
        .unwrap();
        cx.spawn(async move |cx| {
            cx.background_executor()
                .timer(Duration::from_secs(seconds))
                .await;
            let _ = cx.update(|cx| cx.quit());
        })
        .detach();
    });
    anyhow::ensure!(
        !failed.load(std::sync::atomic::Ordering::SeqCst),
        "native browser fixture audit failed"
    );
    anyhow::ensure!(
        completed.load(std::sync::atomic::Ordering::SeqCst),
        "native browser fixture closed before the audit completed successfully"
    );
    Ok(())
}

fn audit_browser(
    client: shelldeck_browser::BrowserClient,
    id: shelldeck_browser::BrowserId,
    deadline: Instant,
) -> anyhow::Result<()> {
    let mut last_state = "waiting for the fixture navigation".to_owned();
    let (frame, (width, height, scale), dom) = loop {
        anyhow::ensure!(
            Instant::now() < deadline,
            "browser audit timed out: {last_state}"
        );
        if let Some(error) = client.failure() {
            anyhow::bail!("browser helper failed during its audit: {error}");
        }
        match (
            client.latest_frame(id),
            client.viewport(id),
            client.state(id),
        ) {
            (Some(frame), Some(viewport), Some(state))
                if !state.loading && state.url.starts_with("http://127.0.0.1:") =>
            {
                let timeout = deadline
                    .saturating_duration_since(Instant::now())
                    .min(Duration::from_secs(2));
                match client.devtools(id, "Runtime.evaluate", serde_json::json!({"expression":"({title:document.title,width:innerWidth,height:innerHeight,input:!!document.querySelector('#name'),enterForm:!!document.querySelector('#enter-form'),submits:Number(document.querySelector('#enter-form')?.dataset.submits||0),textarea:!!document.querySelector('#notes'),select:!!document.querySelector('#option')})", "returnByValue":true}), timeout) {
                    Ok(dom) => {
                        let value = &dom["result"]["value"];
                        let controls = value["title"] == "Native Chromium fixture" && value["input"] == true && value["enterForm"] == true && value["textarea"] == true && value["select"] == true;
                        // A correctly sized about:blank frame must not count as
                        // a painted fixture while its first real paint is queued.
                        let painted = frame.bgra.chunks_exact(4).next().is_some_and(|first| frame.bgra.chunks_exact(4).any(|pixel| pixel != first));
                        if controls && painted { break (frame, viewport, dom); }
                        last_state = format!("fixture DOM/paint not ready (painted={painted}): {dom}");
                    }
                    Err(error) => last_state = format!("fixture DOM not ready: {error}"),
                }
            }
            (_, _, Some(state)) => {
                last_state = format!(
                    "navigation not ready (loading={}, url={})",
                    state.loading, state.url
                );
            }
            _ => {}
        }
        // This is a bounded, opt-in fixture wait on its background worker, not
        // a production browser repaint loop. Production painting stays reactive.
        std::thread::sleep(Duration::from_millis(100));
    };
    anyhow::ensure!(
        (frame.width as f32 - width as f32 * scale).abs() <= 1.0
            && (frame.height as f32 - height as f32 * scale).abs() <= 1.0,
        "CEF frame {}x{} does not match logical viewport {width}x{height} at scale {scale}",
        frame.width,
        frame.height
    );
    let value = &dom["result"]["value"];
    anyhow::ensure!(
        value["title"] == "Native Chromium fixture"
            && value["input"] == true
            && value["enterForm"] == true
            && value["textarea"] == true
            && value["select"] == true,
        "the live DOM did not contain fixture controls: {dom}"
    );
    anyhow::ensure!(
        value["width"] == width && value["height"] == height,
        "DOM viewport disagrees with CEF geometry: {dom}"
    );
    let directory = PathBuf::from(std::env::var_os("XDG_CACHE_HOME").unwrap());
    std::fs::create_dir_all(&directory)?;
    let mut rgba = frame.bgra.clone();
    for pixel in rgba.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }
    let image = image::RgbaImage::from_raw(frame.width, frame.height, rgba)
        .ok_or_else(|| anyhow::anyhow!("invalid frame byte length"))?;
    image.save_with_format(
        directory.join("browser-viewport.png"),
        image::ImageFormat::Png,
    )?;
    std::fs::write(
        directory.join("browser-dom.json"),
        serde_json::to_vec_pretty(&dom)?,
    )?;
    eprintln!(
        "Browser viewport {}x{} and live DOM verified; evidence in {}",
        frame.width,
        frame.height,
        directory.display()
    );
    Ok(())
}

const FIXTURE: &str = r#"<!doctype html><html lang="en"><meta charset="utf-8"><title>Native Chromium fixture</title>
<style>body{margin:0;background:#f4f6fb;color:#17233c;font:16px system-ui}main{max-width:740px;margin:38px auto;padding:0 28px}h1{font-size:30px;line-height:1.1}p{line-height:1.6;color:#4b5b74}section{background:white;padding:24px;border:1px solid #d8deeb;border-radius:12px;margin:22px 0}label{display:block;margin:15px 0 8px}input,textarea,select,button{font:inherit;padding:10px;border:1px solid #9aabc5;border-radius:6px}input,textarea{box-sizing:border-box;width:100%}textarea{min-height:100px}button{background:#2359d7;color:white;border-color:#2359d7;cursor:pointer;margin:8px 10px 0 0}output{display:block;white-space:pre-wrap;margin-top:16px;font-weight:600}a{color:#2359d7}</style>
<main><p>ShellDeck · live native Chromium</p><h1>One page for you and your agent.</h1><p>This page is served only on loopback. Try text input, Unicode, selection, dropdowns, scrolling, browser history, find, and screenshots.</p>
<section><form id="enter-form" onsubmit="event.preventDefault();this.dataset.submits=String(Number(this.dataset.submits||0)+1);document.querySelector('output').textContent='Submitted '+this.dataset.submits+': '+document.querySelector('#name').value"><label for="name">Text and IME</label><input id="name" autocomplete="off" placeholder="Type café, 日本語, or 😀"></form><label for="notes">Retained form state</label><textarea id="notes" placeholder="These edits survive a switch to Conversation or another browser tab."></textarea><label for="option">Native popup menu</label><select id="option"><option>First option</option><option>Second option</option><option>Third option</option></select><div><button id="save" onclick="document.querySelector('output').textContent='Saved locally: '+document.querySelector('#name').value">Check live input</button><button onclick="alert('Native page dialog works')">Page dialog</button></div><output aria-live="polite">Ready.</output></section>
<section><h2>Navigation and selection</h2><p><a href="/second">Open the second history entry</a> · <a href="/new" target="_blank">Open a new retained tab</a></p><p>Select this text with your mouse. Use Ctrl/Cmd+F to find “Chromium”, then navigate matches with the native toolbar.</p><input type="file"><p><a download="fixture.txt" href="data:text/plain,ShellDeck%20browser%20fixture">Download a small fixture</a></p></section>
<section style="margin-top:420px"><h2>Scroll reached</h2><p>Wheel and trackpad deltas reach the actual Chromium viewport. Resize the window and check that clicks still land on the right controls.</p></section></main></html>"#;
