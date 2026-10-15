//! CEF runs in this helper, never in GPUI's UI process. All CEF calls stay on its
//! UI thread; stdin posts tasks and OnPaint pushes bounded, coalesced BGRA frames.
use crate::protocol::{
    self, BrowserId, BrowserState, Command, Event, ImeEvent, KeyEventKind, PaintKind, RequestId,
};
use cef::{args::Args, *};
use serde_json::Value as JsonValue;
use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, VecDeque},
    io::{self, BufReader, BufWriter},
    path::PathBuf,
    rc::Rc as LocalRc,
    sync::{Arc, Condvar, Mutex},
    thread,
};

mod callbacks;
#[cfg(target_os = "macos")]
mod mac;
mod output;
use output::Output;

thread_local! {
    static RUNTIME: RefCell<Option<Runtime>> = const { RefCell::new(None) };
    // CEF references remain on its UI thread; posted tasks carry only integer IDs.
    static PENDING_POPUPS: RefCell<HashMap<BrowserId, (Browser, SharedTab)>> = RefCell::new(HashMap::new());
}
static NEXT_POPUP: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1 << 63);

#[derive(Clone)]
enum Work {
    Ready,
    Command(Command),
    Closed(BrowserId),
    PopupCreated(BrowserId),
}

wrap_task! {
    struct PostedTask { work: Arc<Mutex<Option<Work>>> }
    impl Task {
        fn execute(&self) {
            let Some(work) = self.work.lock().unwrap_or_else(|e| e.into_inner()).take() else { return; };
            RUNTIME.with_borrow_mut(|slot| { if let Some(runtime) = slot { runtime.dispatch(work); } });
        }
    }
}
fn post(work: Work) -> bool {
    post_task(
        ThreadId::UI,
        Some(&mut PostedTask::new(Arc::new(Mutex::new(Some(work))))),
    ) == 1
}

#[derive(Clone)]
struct SharedTab {
    id: BrowserId,
    output: Output,
    state: LocalRc<RefCell<BrowserState>>,
    viewport: LocalRc<Cell<(u32, u32, f32)>>,
    popup: LocalRc<Cell<protocol::Rect>>,
    popup_visible: LocalRc<Cell<bool>>,
    next_dialog: LocalRc<Cell<u64>>,
    dialogs: LocalRc<RefCell<HashMap<u64, JsdialogCallback>>>,
    choosers: LocalRc<RefCell<HashMap<u64, FileDialogCallback>>>,
    downloads: LocalRc<RefCell<HashMap<u64, BeforeDownloadCallback>>>,
    pending_cdp: LocalRc<RefCell<HashMap<i32, RequestId>>>,
    parent_id: Option<BrowserId>,
    tab_count: LocalRc<Cell<usize>>,
    pending_popups: LocalRc<RefCell<HashMap<i32, BrowserId>>>,
    popup_origin: Option<(LocalRc<RefCell<HashMap<i32, BrowserId>>>, i32)>,
}
impl SharedTab {
    fn child(&self, id: BrowserId, url: String, popup_id: i32) -> Self {
        let mut tab = self.clone();
        tab.id = id;
        tab.parent_id = Some(self.id);
        tab.state = LocalRc::new(RefCell::new(BrowserState {
            browser_id: id,
            url,
            title: String::new(),
            loading: true,
            can_go_back: false,
            can_go_forward: false,
        }));
        tab.viewport = LocalRc::new(Cell::new(self.viewport.get()));
        tab.popup = LocalRc::new(Cell::new(protocol::Rect {
            x: 0,
            y: 0,
            width: 0,
            height: 0,
        }));
        tab.popup_visible = LocalRc::new(Cell::new(false));
        tab.next_dialog = LocalRc::new(Cell::new(1));
        tab.dialogs = LocalRc::default();
        tab.choosers = LocalRc::default();
        tab.downloads = LocalRc::default();
        tab.pending_cdp = LocalRc::default();
        tab.pending_popups = LocalRc::default();
        tab.popup_origin = Some((self.pending_popups.clone(), popup_id));
        tab
    }
    fn emit_state(&self) {
        self.output.send(Event::State {
            state: self.state.borrow().clone(),
        });
    }
    fn next_id(&self) -> u64 {
        let id = self.next_dialog.get();
        self.next_dialog.set(id.wrapping_add(1));
        id
    }
    fn error(&self, message: impl Into<String>) {
        self.output.send(Event::Error {
            browser_id: Some(self.id),
            message: clipped(message.into(), 4096),
        });
    }
    fn popup_event(&self) {
        let r = self.popup.get();
        self.output.send(Event::Popup {
            browser_id: self.id,
            visible: self.popup_visible.get(),
            x: r.x,
            y: r.y,
            width: r.width,
            height: r.height,
        });
    }
}
struct Tab {
    browser: Browser,
    shared: SharedTab,
    _registration: Option<Registration>,
    next_cdp: i32,
}
struct Runtime {
    output: Output,
    context: RequestContext,
    tabs: HashMap<BrowserId, Tab>,
    closing: bool,
    tab_count: LocalRc<Cell<usize>>,
}
impl Runtime {
    fn dispatch(&mut self, work: Work) {
        match work {
            Work::Ready => self.output.send(Event::Ready {
                protocol_version: protocol::PROTOCOL_VERSION,
                engine_version: "CEF 154.0.34 / cef-rs 154.5.0".into(),
            }),
            Work::PopupCreated(id) => {
                let popup = PENDING_POPUPS.with_borrow_mut(|pending| pending.remove(&id));
                if let Some((browser, shared)) = popup {
                    let visible = !self.closing
                        && shared
                            .parent_id
                            .is_some_and(|parent| self.tabs.contains_key(&parent));
                    let host = browser.host();
                    self.adopt(browser, shared, visible);
                    if !visible {
                        if let Some(host) = host {
                            host.close_browser(1);
                        }
                    }
                }
            }
            Work::Closed(id) => {
                if let Some(tab) = self.tabs.remove(&id) {
                    self.tab_count.set(self.tab_count.get().saturating_sub(1));
                    for (_, request_id) in tab.shared.pending_cdp.borrow_mut().drain() {
                        self.output.send(Event::DevToolsResult {
                            browser_id: id,
                            request_id,
                            result: JsonValue::Null,
                            error: Some("The browser tab was closed".into()),
                        });
                    }
                    self.output.forget_frames(id);
                    self.output.send(Event::Closed { browser_id: id });
                }
                if self.closing && self.tabs.is_empty() && self.tab_count.get() == 0 {
                    quit_message_loop();
                }
            }
            Work::Command(command) => self.command(command),
        }
    }
    fn error(&self, id: Option<BrowserId>, message: impl Into<String>) {
        self.output.send(Event::Error {
            browser_id: id,
            message: clipped(message.into(), 4096),
        });
    }
    fn command(&mut self, command: Command) {
        if let Command::Shutdown = command {
            self.closing = true;
            if self.tabs.is_empty() && self.tab_count.get() == 0 {
                quit_message_loop();
            }
            for tab in self.tabs.values() {
                // Cancel pending native dialogs so shutdown cannot hang on a hidden prompt.
                let dialogs: Vec<_> = tab
                    .shared
                    .dialogs
                    .borrow_mut()
                    .drain()
                    .map(|(_, callback)| callback)
                    .collect();
                for callback in dialogs {
                    callback.cont(0, None);
                }
                let choosers: Vec<_> = tab
                    .shared
                    .choosers
                    .borrow_mut()
                    .drain()
                    .map(|(_, callback)| callback)
                    .collect();
                for callback in choosers {
                    callback.cancel();
                }
                tab.shared.downloads.borrow_mut().clear();
                if let Some(host) = tab.browser.host() {
                    host.close_browser(1);
                }
            }
            return;
        }
        if self.closing {
            return;
        }
        if let Command::Create {
            browser_id,
            url,
            width,
            height,
            scale_factor,
        } = command
        {
            self.create(browser_id, url, width, height, scale_factor);
            return;
        }
        let Some(id) = command.browser_id() else {
            return;
        };
        let Some(tab) = self.tabs.get_mut(&id) else {
            self.error(Some(id), "The browser tab is no longer open");
            return;
        };
        let Some(host) = tab.browser.host() else {
            return;
        };
        match command {
            Command::Close { .. } => {
                host.close_browser(0);
            }
            Command::Navigate { url, .. } => match protocol::navigation_url(&url) {
                Ok(url) => {
                    if let Some(frame) = tab.browser.main_frame() {
                        frame.load_url(Some(&url.as_str().into()));
                    }
                }
                Err(error) => tab.shared.error(error),
            },
            Command::Back { .. } => {
                if tab.browser.can_go_back() == 1 {
                    tab.browser.go_back();
                }
            }
            Command::Forward { .. } => {
                if tab.browser.can_go_forward() == 1 {
                    tab.browser.go_forward();
                }
            }
            Command::Reload { ignore_cache, .. } => {
                if ignore_cache {
                    tab.browser.reload_ignore_cache();
                } else {
                    tab.browser.reload();
                }
            }
            Command::Stop { .. } => tab.browser.stop_load(),
            Command::Resize {
                width,
                height,
                scale_factor,
                ..
            } => {
                if let Err(error) = valid_viewport(width, height, scale_factor) {
                    tab.shared.error(error);
                    return;
                }
                let old_scale = tab.shared.viewport.get().2;
                tab.shared.viewport.set((width, height, scale_factor));
                if old_scale != scale_factor {
                    host.notify_screen_info_changed();
                }
                host.was_resized();
            }
            Command::MouseMove {
                x,
                y,
                modifiers,
                leave,
                ..
            } => host.send_mouse_move_event(Some(&MouseEvent { x, y, modifiers }), leave as i32),
            Command::MouseButton {
                x,
                y,
                modifiers,
                button,
                pressed,
                click_count,
                ..
            } => {
                if pressed {
                    host.set_focus(1);
                }
                let button = match button {
                    protocol::MouseButton::Left => MouseButtonType::LEFT,
                    protocol::MouseButton::Middle => MouseButtonType::MIDDLE,
                    protocol::MouseButton::Right => MouseButtonType::RIGHT,
                };
                host.send_mouse_click_event(
                    Some(&MouseEvent { x, y, modifiers }),
                    button,
                    (!pressed) as i32,
                    click_count.clamp(1, 3),
                );
            }
            Command::MouseWheel {
                x,
                y,
                modifiers,
                delta_x,
                delta_y,
                ..
            } => {
                host.send_mouse_wheel_event(Some(&MouseEvent { x, y, modifiers }), delta_x, delta_y)
            }
            Command::Key { event, .. } => {
                let type_ = match event.kind {
                    KeyEventKind::RawKeyDown => KeyEventType::RAWKEYDOWN,
                    KeyEventKind::KeyUp => KeyEventType::KEYUP,
                    KeyEventKind::Char => KeyEventType::CHAR,
                };
                host.send_key_event(Some(&cef::KeyEvent {
                    type_,
                    windows_key_code: event.windows_key_code,
                    native_key_code: event.native_key_code,
                    modifiers: event.modifiers,
                    character: event.character.unwrap_or(0),
                    unmodified_character: event.unmodified_character.unwrap_or(0),
                    is_system_key: event.is_system_key as i32,
                    ..Default::default()
                }));
            }
            Command::Text { text, .. } => {
                host.ime_commit_text(Some(&text.as_str().into()), Some(&invalid_range()), 0)
            }
            Command::Ime {
                event:
                    ImeEvent::Commit {
                        text,
                        replacement_range,
                    },
                ..
            } => {
                let replacement = replacement_range
                    .map(|r| Range {
                        from: r.start,
                        to: r.end,
                    })
                    .unwrap_or_else(invalid_range);
                host.ime_commit_text(Some(&text.as_str().into()), Some(&replacement), 0);
            }
            Command::Focus { focused, .. } => host.set_focus(focused as i32),
            Command::Ime {
                event: ImeEvent::Cancel,
                ..
            } => host.ime_cancel_composition(),
            Command::Ime {
                event:
                    ImeEvent::SetComposition {
                        text,
                        selection_start,
                        selection_end,
                        replacement_range,
                    },
                ..
            } => {
                let length = text.encode_utf16().count() as i32;
                let replacement = replacement_range
                    .map(|r| Range {
                        from: r.start,
                        to: r.end,
                    })
                    .unwrap_or_else(invalid_range);
                host.ime_set_composition(
                    Some(&text.as_str().into()),
                    None,
                    Some(&replacement),
                    Some(&Range {
                        from: selection_start.clamp(0, length) as u32,
                        to: selection_end.clamp(0, length) as u32,
                    }),
                );
            }
            Command::DevTools {
                request_id,
                method,
                params,
                ..
            } => {
                let bytes = serde_json::to_vec(&params).unwrap_or_default();
                let mut dict = parse_json_buffer(Some(&bytes), JsonParserOptions::RFC)
                    .and_then(|value| value.dictionary());
                if !params.is_object()
                    || dict.is_none()
                    || method.len() > 256
                    || tab.shared.pending_cdp.borrow().len() >= 256
                {
                    tab.shared.output.send(Event::DevToolsResult {
                        browser_id: id,
                        request_id,
                        result: JsonValue::Null,
                        error: Some("Invalid or excessive DevTools request".into()),
                    });
                    return;
                }
                let message_id = tab.next_cdp;
                tab.next_cdp = if message_id >= i32::MAX - 1 {
                    1
                } else {
                    message_id + 1
                };
                tab.shared
                    .pending_cdp
                    .borrow_mut()
                    .insert(message_id, request_id);
                if host.execute_dev_tools_method(
                    message_id,
                    Some(&method.as_str().into()),
                    dict.as_mut(),
                ) == 0
                {
                    tab.shared.pending_cdp.borrow_mut().remove(&message_id);
                    tab.shared.output.send(Event::DevToolsResult {
                        browser_id: id,
                        request_id,
                        result: JsonValue::Null,
                        error: Some("Chromium rejected the DevTools command".into()),
                    });
                }
            }
            Command::DialogResponse {
                dialog_id,
                accept,
                prompt,
                ..
            } => {
                let callback = tab.shared.dialogs.borrow_mut().remove(&dialog_id);
                if let Some(callback) = callback {
                    callback.cont(accept as i32, Some(&prompt.as_str().into()));
                }
            }
            Command::FileChooserResponse {
                chooser_id, paths, ..
            } => {
                let callback = tab.shared.choosers.borrow_mut().remove(&chooser_id);
                if let Some(callback) = callback {
                    if paths.is_empty() {
                        callback.cancel();
                    } else {
                        let mut files = CefStringList::new();
                        for path in paths {
                            files.append(&path);
                        }
                        callback.cont(Some(&mut files));
                    }
                }
            }
            Command::DownloadResponse {
                download_id, path, ..
            } => {
                let callback = tab.shared.downloads.borrow_mut().remove(&download_id);
                if let Some(callback) = callback {
                    if let Some(path) = path.filter(|path| PathBuf::from(path).is_absolute()) {
                        callback.cont(Some(&path.as_str().into()), 0);
                    }
                }
            }
            Command::Find {
                text,
                forward,
                match_case,
                find_next,
                ..
            } => host.find(
                Some(&text.as_str().into()),
                forward as i32,
                match_case as i32,
                find_next as i32,
            ),
            Command::StopFinding {
                clear_selection, ..
            } => host.stop_finding(clear_selection as i32),
            Command::Create { .. } | Command::Shutdown => {}
        }
    }
    fn create(&mut self, id: BrowserId, url: String, width: u32, height: u32, scale: f32) {
        if id >= (1 << 63)
            || self.tabs.contains_key(&id)
            || self.tab_count.get() >= protocol::MAX_TABS
        {
            self.error(
                Some(id),
                "Browser tab limit reached or duplicate tab identifier",
            );
            return;
        }
        if let Err(error) = valid_viewport(width, height, scale) {
            self.error(Some(id), error);
            return;
        }
        let url = match protocol::navigation_url(&url) {
            Ok(url) => url,
            Err(error) => {
                self.error(Some(id), error);
                return;
            }
        };
        let shared = SharedTab {
            id,
            output: self.output.clone(),
            state: LocalRc::new(RefCell::new(BrowserState {
                browser_id: id,
                url: url.clone(),
                title: String::new(),
                loading: true,
                can_go_back: false,
                can_go_forward: false,
            })),
            viewport: LocalRc::new(Cell::new((width, height, scale))),
            popup: LocalRc::new(Cell::new(protocol::Rect {
                x: 0,
                y: 0,
                width: 0,
                height: 0,
            })),
            popup_visible: LocalRc::new(Cell::new(false)),
            next_dialog: LocalRc::new(Cell::new(1)),
            dialogs: LocalRc::default(),
            choosers: LocalRc::default(),
            downloads: LocalRc::default(),
            pending_cdp: LocalRc::default(),
            parent_id: None,
            tab_count: self.tab_count.clone(),
            pending_popups: LocalRc::default(),
            popup_origin: None,
        };
        let info = WindowInfo {
            windowless_rendering_enabled: 1,
            runtime_style: RuntimeStyle::ALLOY,
            ..Default::default()
        };
        let settings = BrowserSettings {
            windowless_frame_rate: 30,
            background_color: 0xff181820,
            ..Default::default()
        };
        let mut client = callbacks::make_client(shared.clone());
        self.tab_count.set(self.tab_count.get() + 1);
        match browser_host_create_browser_sync(
            Some(&info),
            Some(&mut client),
            Some(&url.as_str().into()),
            Some(&settings),
            None,
            Some(&mut self.context),
        ) {
            Some(browser) => self.adopt(browser, shared, true),
            None => {
                self.tab_count.set(self.tab_count.get().saturating_sub(1));
                self.error(Some(id), "Chromium could not create an off-screen browser");
            }
        }
    }
    fn adopt(&mut self, browser: Browser, shared: SharedTab, visible: bool) {
        let id = shared.id;
        let registration = browser.host().and_then(|host| {
            host.add_dev_tools_message_observer(Some(&mut callbacks::DevToolsObserver::new(
                shared.clone(),
            )))
        });
        self.tabs.insert(
            id,
            Tab {
                browser,
                shared: shared.clone(),
                _registration: registration,
                next_cdp: 1,
            },
        );
        if visible {
            self.output.send(Event::Created {
                browser_id: id,
                parent_id: shared.parent_id,
            });
            shared.emit_state();
        }
    }
}
fn clipped(mut text: String, limit: usize) -> String {
    if text.len() > limit {
        let mut end = limit;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
    }
    text
}
fn invalid_range() -> Range {
    Range {
        from: u32::MAX,
        to: u32::MAX,
    }
}

fn valid_viewport(width: u32, height: u32, scale: f32) -> Result<(), String> {
    protocol::frame_byte_len(width, height).map_err(|e| e.to_string())?;
    if !scale.is_finite()
        || !(0.5..=4.0).contains(&scale)
        || width as f32 * scale > protocol::MAX_VIEWPORT as f32
        || height as f32 * scale > protocol::MAX_VIEWPORT as f32
    {
        return Err("Browser viewport scale exceeds its pixel limit".into());
    }
    Ok(())
}

wrap_app! {
    struct RuntimeApp;
    impl App {
        fn browser_process_handler(&self) -> Option<BrowserProcessHandler> { Some(RuntimeProcessHandler::new()) }
        fn on_before_command_line_processing(&self, _process_type: Option<&CefString>, command_line: Option<&mut CommandLine>) {
            let Some(command_line) = command_line else { return; };
            for flag in ["no-startup-window", "noerrdialogs", "disable-session-crashed-bubble", "disable-gpu", "disable-gpu-compositing"] { command_line.append_switch(Some(&flag.into())); }
            // OSR owns no native Chromium window. Chromium's headless Ozone backend
            // renders into OnPaint while GPUI remains a native Wayland client.
            #[cfg(target_os = "linux")]
            command_line.append_switch_with_value(Some(&"ozone-platform".into()), Some(&"headless".into()));
        }
    }
}

wrap_browser_process_handler! {
    struct RuntimeProcessHandler;
    impl BrowserProcessHandler {
        fn on_context_initialized(&self) { post(Work::Ready); }
    }
}

pub fn run() -> Result<i32, Box<dyn std::error::Error>> {
    run_with_sandbox(std::ptr::null_mut())
}

fn run_with_sandbox(sandbox_info: *mut u8) -> Result<i32, Box<dyn std::error::Error>> {
    let args = Args::new();
    let subprocess = std::env::args().any(|arg| arg.starts_with("--type="));
    #[cfg(target_os = "macos")]
    let _sandbox = if subprocess {
        let mut sandbox = sandbox::Sandbox::new();
        sandbox.initialize(args.as_main_args());
        Some(sandbox)
    } else {
        None
    };
    #[cfg(target_os = "macos")]
    let _loader = {
        let loader = library_loader::LibraryLoader::new(&std::env::current_exe()?, subprocess);
        if !loader.load() {
            return Err("Chromium Embedded Framework could not be loaded".into());
        }
        loader
    };
    #[cfg(target_os = "macos")]
    if !subprocess {
        mac::setup();
    }
    let _ = api_hash(sys::CEF_API_VERSION_LAST, 0);
    let mut app = RuntimeApp::new();
    let code = execute_process(Some(args.as_main_args()), Some(&mut app), sandbox_info);
    if subprocess || code >= 0 {
        return Ok(code.max(0));
    }
    let mut argv = std::env::args();
    let profile = argv
        .find(|arg| arg == "--profile")
        .and_then(|_| argv.next())
        .map(PathBuf::from)
        .ok_or("An explicit --profile directory is required")?;
    if !profile.is_absolute() {
        return Err("The browser profile directory must be absolute".into());
    }
    std::fs::create_dir_all(&profile)?;
    let profile = profile.canonicalize()?;
    let cache = profile.join("default");
    let executable = std::env::current_exe()?;
    let resources = executable
        .parent()
        .ok_or("Browser helper has no parent directory")?;
    let mut settings = Settings {
        windowless_rendering_enabled: 1,
        root_cache_path: profile.to_string_lossy().as_ref().into(),
        cache_path: cache.to_string_lossy().as_ref().into(),
        persist_session_cookies: 1,
        log_file: profile
            .join("chromium.log")
            .to_string_lossy()
            .as_ref()
            .into(),
        log_severity: LogSeverity::WARNING,
        // No remote debugging socket: the private pipe is the only host control path.
        remote_debugging_port: 0,
        background_color: 0xff181820,
        ..Default::default()
    };
    #[cfg(not(target_os = "macos"))]
    {
        settings.browser_subprocess_path = executable.to_string_lossy().as_ref().into();
        settings.resources_dir_path = resources.to_string_lossy().as_ref().into();
        settings.locales_dir_path = resources.join("locales").to_string_lossy().as_ref().into();
    }
    #[cfg(target_os = "macos")]
    {
        // The separate helper has its own NSApplication/CEF message loop.
        let contents = resources
            .parent()
            .ok_or("Browser helper is outside its app bundle")?;
        let stem = executable
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or("Invalid browser helper name")?;
        settings.browser_subprocess_path = contents
            .join("Frameworks")
            .join(format!("{stem} Helper.app"))
            .join("Contents/MacOS")
            .join(format!("{stem} Helper"))
            .to_string_lossy()
            .as_ref()
            .into();
    }
    if initialize(
        Some(args.as_main_args()),
        Some(&settings),
        Some(&mut app),
        sandbox_info,
    ) != 1
    {
        return Err(
            "Chromium initialization failed; see the browser profile's chromium.log".into(),
        );
    }
    let output = Output::new();
    // One isolated helper owns one persistent profile. Its global request context
    // is initialized by OnContextInitialized before Ready reaches the parent.
    let context =
        request_context_get_global_context().ok_or("Could not create Chromium profile context")?;
    RUNTIME.with_borrow_mut(|slot| {
        *slot = Some(Runtime {
            output: output.clone(),
            context,
            tabs: HashMap::new(),
            closing: false,
            tab_count: LocalRc::new(Cell::new(0)),
        })
    });
    let writer_output = output.clone();
    let writer = thread::Builder::new()
        .name("browser-output".into())
        .spawn(move || {
            if let Err(error) = writer_output.write() {
                eprintln!("Browser output closed: {error}");
                post(Work::Command(Command::Shutdown));
            }
        })?;
    let reader_output = output.clone();
    thread::Builder::new()
        .name("browser-input".into())
        .spawn(move || {
            let mut input = BufReader::new(io::stdin().lock());
            loop {
                match protocol::read_packet::<_, Command>(&mut input) {
                    Ok((command, payload)) if payload.is_empty() => {
                        let stopping = matches!(command, Command::Shutdown);
                        if !post(Work::Command(command)) || stopping {
                            break;
                        }
                    }
                    Ok(_) => {
                        reader_output.send(Event::Error {
                            browser_id: None,
                            message: "Commands cannot contain binary payloads".into(),
                        });
                        break;
                    }
                    Err(error) => {
                        if error.kind() != io::ErrorKind::UnexpectedEof {
                            reader_output.send(Event::Error {
                                browser_id: None,
                                message: error.to_string(),
                            });
                        }
                        break;
                    }
                }
            }
            post(Work::Command(Command::Shutdown));
        })?;
    run_message_loop();
    RUNTIME.with_borrow_mut(|slot| {
        slot.take();
    });
    shutdown();
    output.finish();
    let _ = writer.join();
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    // SDTEST-2551 — SDUC-557: logical bounds cannot allocate an oversized physical CEF frame.
    #[test]
    fn sdtest_2551_browser_viewport_scale_limits() {
        assert!(valid_viewport(1600, 1000, 2.0).is_ok());
        for (width, height, scale) in [
            (0, 1, 1.0),
            (4096, 4096, 2.0),
            (10, 10, f32::NAN),
            (10, 10, f32::INFINITY),
            (10, 10, 0.0),
        ] {
            assert!(valid_viewport(width, height, scale).is_err());
        }
    }
}

/// CEF's Windows bootstrap executable supplies its sandbox context before Rust
/// executes. Packaging renames bootstrap.exe and the matching cdylib together.
#[cfg(target_os = "windows")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn RunWinMain(
    _instance: cef::sys::HINSTANCE,
    _command_line: *const u8,
    _command_show: i32,
    sandbox_info: *mut u8,
) -> i32 {
    if std::env::args().any(|arg| arg == "--mcp") {
        return match crate::mcp::run_stdio_proxy() {
            Ok(()) => 0,
            Err(error) => {
                eprintln!("ShellDeck browser bridge: {error}");
                1
            }
        };
    }
    match run_with_sandbox(sandbox_info) {
        Ok(code) => code,
        Err(error) => {
            eprintln!("ShellDeck browser: {error}");
            1
        }
    }
}
