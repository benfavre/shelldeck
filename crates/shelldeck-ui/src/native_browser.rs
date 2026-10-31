//! Retained, live Chromium view shared by the user and the scoped agent bridge.
//!
//! Browser pixels are CEF BGRA off-screen frames. The renderer never substitutes
//! screenshots or an external browser when the native helper is unavailable.
mod geometry;
mod shortcuts;
mod surface;

use crate::scale::px;
use crate::t;
use crate::theme::ShellDeckColors;
use adabraka_ui::components::confirm_dialog::Dialog as UiDialog;
use adabraka_ui::components::input::{Input, InputSize, InputState, InputVariant};
use adabraka_ui::overlays::context_menu::{ContextMenu, ContextMenuItem};
use adabraka_ui::prelude::*;
use gpui::*;
use shelldeck_browser::{
    BrowserClient, BrowserFrame, BrowserId, BrowserState, Command, Event, ImeEvent, PaintKind,
    Rect, Utf16Range,
};
use std::ops::Range;
use std::sync::Arc;

const MAX_TABS: usize = 8;
const MAX_SCREENSHOT_BYTES: usize = 8 * 1024 * 1024;

#[derive(Clone)]
pub enum NativeBrowserEvent {
    /// The helper is connected; the same client is available through control().
    Ready,
    ActiveTabChanged,
    CloseRequested,
    /// Explicit user consent, initially false and never persisted across app restarts.
    ControlChanged {
        allowed: bool,
    },
    ShareScreenshot {
        png: Vec<u8>,
        url: String,
        title: String,
    },
    Error(String),
}

struct BrowserTab {
    state: BrowserState,
    image: Option<Arc<RenderImage>>,
    popup_image: Option<Arc<RenderImage>>,
    popup_rect: Option<Rect>,
    error: Option<String>,
    cursor: CursorStyle,
    closing: bool,
}

impl BrowserTab {
    fn new(browser_id: BrowserId, url: String) -> Self {
        Self {
            state: BrowserState {
                browser_id,
                title: String::new(),
                url,
                loading: true,
                can_go_back: false,
                can_go_forward: false,
            },
            image: None,
            popup_image: None,
            popup_rect: None,
            error: None,
            cursor: CursorStyle::Arrow,
            closing: false,
        }
    }
}

struct PageDialog {
    browser_id: BrowserId,
    dialog_id: u64,
    kind: String,
    message: String,
    default_prompt: Option<String>,
}

struct PageContextMenu {
    browser_id: BrowserId,
    x: i32,
    y: i32,
    link_url: String,
    selected_text: String,
    editable: bool,
}

pub struct NativeBrowserView {
    scope: String,
    client: Option<BrowserClient>,
    launching: bool,
    pending_urls: Vec<String>,
    tabs: Vec<BrowserTab>,
    active: Option<BrowserId>,
    address: Entity<InputState>,
    address_pending: Option<String>,
    address_force_update: bool,
    focus_page_requested: bool,
    dialog_input: Entity<InputState>,
    dialog_focus: FocusHandle,
    dialog_focus_requested: bool,
    find_input: Entity<InputState>,
    find_visible: bool,
    find_result: Option<(i32, i32)>,
    page_dialog: Option<PageDialog>,
    context_menu: Option<PageContextMenu>,
    focus: FocusHandle,
    has_focus: bool,
    viewport: Option<Bounds<Pixels>>,
    viewport_size: Option<(u32, u32, u32)>,
    pressed_buttons: u32,
    pointer_inside: bool,
    agent_control_allowed: bool,
    agent_control_available: bool,
    agent_observation_available: bool,
    error: Option<String>,
    status: Option<String>,
    engine_version: Option<String>,
    sharing: bool,
    /// Only bounded selection/composition context is mirrored, never page input
    /// values discovered by JavaScript. Password fields remain owned by Chromium.
    selected_text: String,
    selected_range: Range<usize>,
    composition_text: String,
    marked_range: Option<Range<usize>>,
    ime_character_bounds: Vec<Rect>,
    launch_task: Option<Task<()>>,
    event_task: Option<Task<()>>,
    share_task: Option<Task<()>>,
    request_tasks: std::collections::HashMap<uuid::Uuid, Task<()>>,
    _release: Subscription,
}

impl EventEmitter<NativeBrowserEvent> for NativeBrowserView {}

impl NativeBrowserView {
    pub fn new(scope: String, cx: &mut Context<Self>) -> Self {
        shortcuts::init(cx);
        let release = cx.on_release(|view, cx| {
            if let Some(client) = view.client.take() {
                let _ = client.send(Command::Shutdown);
            }
            for tab in &mut view.tabs {
                for image in [tab.image.take(), tab.popup_image.take()]
                    .into_iter()
                    .flatten()
                {
                    cx.drop_image(image, None);
                }
            }
        });
        Self {
            scope,
            client: None,
            launching: false,
            pending_urls: Vec::new(),
            tabs: Vec::new(),
            active: None,
            address: cx.new(InputState::new),
            address_pending: None,
            address_force_update: false,
            focus_page_requested: false,
            dialog_input: cx.new(InputState::new),
            dialog_focus: cx.focus_handle(),
            dialog_focus_requested: false,
            find_input: cx.new(InputState::new),
            find_visible: false,
            find_result: None,
            page_dialog: None,
            context_menu: None,
            focus: cx.focus_handle(),
            has_focus: false,
            viewport: None,
            viewport_size: None,
            pressed_buttons: 0,
            pointer_inside: false,
            agent_control_allowed: false,
            agent_control_available: true,
            agent_observation_available: true,
            error: None,
            status: None,
            engine_version: None,
            sharing: false,
            selected_text: String::new(),
            selected_range: 0..0,
            composition_text: String::new(),
            marked_range: None,
            ime_character_bounds: Vec::new(),
            launch_task: None,
            event_task: None,
            share_task: None,
            request_tasks: Default::default(),
            _release: release,
        }
    }

    pub fn scope(&self) -> &str {
        &self.scope
    }

    pub fn control(&self) -> Option<BrowserClient> {
        self.client.clone()
    }

    #[cfg(test)]
    pub(crate) fn bind_fixture_client(&mut self, client: BrowserClient, id: BrowserId) {
        self.client = Some(client);
        self.tabs.push(BrowserTab::new(id, "about:blank".into()));
        self.active = Some(id);
    }

    pub fn active_browser_id(&self) -> Option<BrowserId> {
        self.active
    }

    pub fn request_focus(&mut self, cx: &mut Context<Self>) {
        if self.page_dialog.is_some() {
            self.dialog_focus_requested = true;
        } else {
            self.focus_page_requested = true;
        }
        cx.notify();
    }

    pub fn browser_ids(&self) -> Vec<BrowserId> {
        self.tabs.iter().map(|tab| tab.state.browser_id).collect()
    }

    pub fn agent_control_allowed(&self) -> bool {
        self.agent_control_allowed
    }

    pub fn set_agent_observation_available(&mut self, available: bool, cx: &mut Context<Self>) {
        self.agent_observation_available = available;
        cx.notify();
    }

    pub fn set_agent_control_available(&mut self, available: bool, cx: &mut Context<Self>) {
        if self.agent_control_available != available {
            self.agent_control_available = available;
            if !available {
                self.revoke_agent_control(cx);
            }
            cx.notify();
        }
    }

    pub fn revoke_agent_control(&mut self, cx: &mut Context<Self>) {
        if self.agent_control_allowed {
            self.agent_control_allowed = false;
            cx.emit(NativeBrowserEvent::ControlChanged { allowed: false });
            cx.notify();
        }
    }

    /// Explicit lifecycle boundary for logout/context disposal. Browser tabs are
    /// otherwise retained across Conversation/Editor/Browser switches.
    pub fn close_all(&mut self, cx: &mut Context<Self>) {
        self.revoke_agent_control(cx);
        if let Some(client) = self.client.take() {
            let _ = client.send(Command::Shutdown);
        }
        self.launch_task = None;
        self.event_task = None;
        self.share_task = None;
        self.request_tasks.clear();
        for tab in self.tabs.drain(..) {
            for image in [tab.image, tab.popup_image].into_iter().flatten() {
                cx.drop_image(image, None);
            }
        }
        self.active = None;
        self.pending_urls.clear();
        self.page_dialog = None;
        self.context_menu = None;
        self.launching = false;
        self.sharing = false;
        self.viewport_size = None;
        cx.emit(NativeBrowserEvent::ActiveTabChanged);
        cx.notify();
    }

    /// Opens the address in the active retained tab, starting CEF only when
    /// needed. New views intentionally cost no browser subprocess or network I/O.
    pub fn navigate(&mut self, url: String, cx: &mut Context<Self>) {
        let Ok(url) = shelldeck_browser::navigation_url(&url) else {
            self.error = Some(t!("native_browser.invalid_url").to_string());
            cx.notify();
            return;
        };
        self.error = None;
        self.focus_page_requested = true;
        if let Some(browser_id) = self.active {
            self.address_pending = Some(url.clone());
            self.send(Command::Navigate { browser_id, url }, cx);
        } else if self.client.is_none() && !self.pending_urls.is_empty() {
            if let Some(pending) = self.pending_urls.last_mut() {
                *pending = url;
            }
            self.ensure_engine(cx);
        } else {
            self.new_tab(url, cx);
        }
    }

    fn new_tab(&mut self, url: String, cx: &mut Context<Self>) {
        if self.tabs.len() + self.pending_urls.len() >= MAX_TABS {
            self.error = Some(t!("native_browser.tab_limit", count = MAX_TABS).to_string());
            cx.notify();
            return;
        }
        if let Some(client) = self.client.as_ref() {
            let (width, height, scale) = self.viewport_size.unwrap_or((800, 600, 1000));
            match client.create_tab(&url, width, height, scale as f32 / 1000.0) {
                Ok(browser_id) => {
                    self.tabs.push(BrowserTab::new(browser_id, url.clone()));
                    self.select_tab(browser_id, cx);
                }
                Err(error) => self.set_error(error.to_string(), cx),
            }
        } else {
            self.pending_urls.push(url);
            self.ensure_engine(cx);
        }
        cx.notify();
    }

    fn ensure_engine(&mut self, cx: &mut Context<Self>) {
        if self.client.is_some() || self.launching {
            return;
        }
        self.launching = true;
        self.error = None;
        let scope = self.scope.clone();
        let task = cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let profile = shelldeck_core::config::AppConfig::config_dir()
                        .join("browser-profiles")
                        .join(profile_component(&scope));
                    let executable = shelldeck_browser::discover_helper().ok_or_else(|| {
                        std::io::Error::new(
                            std::io::ErrorKind::NotFound,
                            t!("native_browser.helper_missing").to_string(),
                        )
                    })?;
                    let launch = shelldeck_browser::BrowserLaunch {
                        executable,
                        profile_dir: profile,
                    };
                    BrowserClient::spawn(launch)
                })
                .await;
            let _ = this.update(cx, |view, cx| {
                view.launching = false;
                match result {
                    Ok(client) => {
                        let (tx, rx) = smol::channel::bounded::<()>(1);
                        client.set_waker(Arc::new(move || {
                            let _ = tx.try_send(());
                        }));
                        view.client = Some(client.clone());
                        let pending = std::mem::take(&mut view.pending_urls);
                        for url in pending {
                            view.new_tab(url, cx);
                        }
                        view.consume_events(cx);
                        view.event_task = Some(cx.spawn(async move |this, cx| {
                            while rx.recv().await.is_ok() {
                                if this.update(cx, |view, cx| view.consume_events(cx)).is_err() {
                                    break;
                                }
                            }
                        }));
                    }
                    Err(error) => {
                        // Keep addresses for an explicit retry. No external
                        // process or snapshot fallback is selected here.
                        view.set_error(
                            format!(
                                "{}\n{}",
                                t!(if error.kind() == std::io::ErrorKind::NotFound {
                                    "native_browser.unavailable"
                                } else {
                                    "native_browser.start_failed"
                                }),
                                error
                            ),
                            cx,
                        );
                    }
                }
                if let Some(task) = view.launch_task.take() {
                    task.detach();
                }
                cx.notify();
            });
        });
        self.launch_task = Some(task);
    }

    fn send(&mut self, command: Command, cx: &mut Context<Self>) {
        if matches!(
            &command,
            Command::Navigate { .. }
                | Command::Back { .. }
                | Command::Forward { .. }
                | Command::Reload { .. }
        ) {
            if let Some(tab) = self
                .tabs
                .iter_mut()
                .find(|tab| Some(tab.state.browser_id) == command.browser_id())
            {
                tab.error = None;
                tab.state.loading = true;
            }
            self.status = None;
            cx.notify();
        }
        if let Some(client) = &self.client {
            if let Err(error) = client.send(command) {
                self.set_error(error.to_string(), cx);
            }
        }
    }

    fn set_error(&mut self, error: String, cx: &mut Context<Self>) {
        self.error = Some(error.clone());
        cx.emit(NativeBrowserEvent::Error(error));
        cx.notify();
    }

    fn selected_tab(&self) -> Option<&BrowserTab> {
        self.tabs
            .iter()
            .find(|tab| Some(tab.state.browser_id) == self.active)
    }

    fn focus_address(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(value) = self.address_pending.take() {
            self.address
                .update(cx, |state, cx| state.set_value(value, window, cx));
        }
        self.address.read(cx).focus_handle(cx).focus(window);
        self.address.update(cx, |state, cx| {
            state.select_all(&adabraka_ui::components::input::SelectAll, window, cx)
        });
    }

    fn select_tab(&mut self, browser_id: BrowserId, cx: &mut Context<Self>) {
        if self.active == Some(browser_id) {
            return;
        }
        if let Some(previous) = self.active {
            self.send(
                Command::Focus {
                    browser_id: previous,
                    focused: false,
                },
                cx,
            );
        }
        self.active = Some(browser_id);
        self.context_menu = None;
        self.address_pending = self.selected_tab().map(|tab| {
            if tab.state.url == "about:blank" {
                String::new()
            } else {
                tab.state.url.clone()
            }
        });
        self.address_force_update = true;
        self.selected_text.clear();
        self.selected_range = 0..0;
        self.composition_text.clear();
        self.marked_range = None;
        self.find_result = None;
        self.ime_character_bounds.clear();
        self.pressed_buttons = 0;
        self.viewport_size = None;
        self.send(
            Command::Focus {
                browser_id,
                focused: self.has_focus,
            },
            cx,
        );
        cx.emit(NativeBrowserEvent::ActiveTabChanged);
        cx.notify();
    }

    fn close_tab(&mut self, browser_id: BrowserId, cx: &mut Context<Self>) {
        let Some(tab) = self
            .tabs
            .iter()
            .find(|tab| tab.state.browser_id == browser_id)
        else {
            return;
        };
        if tab.closing {
            return;
        }
        let Some(client) = self.client.as_ref() else {
            return;
        };
        match client.send(Command::Close { browser_id }) {
            Ok(()) => {
                if let Some(tab) = self
                    .tabs
                    .iter_mut()
                    .find(|tab| tab.state.browser_id == browser_id)
                {
                    tab.closing = true;
                }
            }
            Err(error) => self.set_error(error.to_string(), cx),
        }
        // Chromium may ask beforeunload. Retain the live page, focused tab, and
        // ownership until its actual Closed event; Cancel keeps the form usable.
        cx.notify();
    }

    fn remove_tab(&mut self, browser_id: BrowserId, cx: &mut Context<Self>) {
        let Some(index) = self
            .tabs
            .iter()
            .position(|tab| tab.state.browser_id == browser_id)
        else {
            return;
        };
        let removed = self.tabs.remove(index);
        for image in [removed.image, removed.popup_image].into_iter().flatten() {
            cx.drop_image(image, None);
        }
        if self
            .page_dialog
            .as_ref()
            .is_some_and(|dialog| dialog.browser_id == browser_id)
        {
            self.page_dialog = None;
        }
        if self.active == Some(browser_id) {
            self.active = None;
            if let Some(next) = self.tabs.get(index.min(self.tabs.len().saturating_sub(1))) {
                self.select_tab(next.state.browser_id, cx);
            } else {
                self.address_pending = Some(String::new());
            }
        }
        cx.emit(NativeBrowserEvent::ActiveTabChanged);
        cx.notify();
    }

    fn consume_events(&mut self, cx: &mut Context<Self>) {
        let Some(client) = self.client.clone() else {
            return;
        };
        for event in client.drain_events() {
            match event {
                Event::Ready { engine_version, .. } => {
                    self.engine_version = Some(engine_version);
                    cx.emit(NativeBrowserEvent::Ready);
                }
                Event::State { state } => {
                    if let Some(tab) = self
                        .tabs
                        .iter_mut()
                        .find(|tab| tab.state.browser_id == state.browser_id)
                    {
                        if self.active == Some(state.browser_id) && tab.state.url != state.url {
                            self.address_pending = Some(state.url.clone());
                        }
                        tab.state = state;
                    }
                }
                Event::Closed { browser_id } => self.remove_tab(browser_id, cx),
                Event::Created {
                    browser_id,
                    parent_id: Some(parent_id),
                } if client.owns(browser_id)
                    && self
                        .tabs
                        .iter()
                        .any(|tab| tab.state.browser_id == parent_id)
                    && !self
                        .tabs
                        .iter()
                        .any(|tab| tab.state.browser_id == browser_id) =>
                {
                    // The helper creates true Chromium popup children so
                    // window.opener/postMessage and OAuth returns survive.
                    // The transport validates parent ownership and child IDs.
                    if self.tabs.len() >= MAX_TABS {
                        self.send(Command::Close { browser_id }, cx);
                        self.error =
                            Some(t!("native_browser.tab_limit", count = MAX_TABS).to_string());
                    } else {
                        let url = client
                            .state(browser_id)
                            .map(|state| state.url)
                            .unwrap_or_else(|| "about:blank".into());
                        self.tabs.push(BrowserTab::new(browser_id, url));
                        self.select_tab(browser_id, cx);
                    }
                }
                Event::CursorChanged { browser_id, cursor } => {
                    if let Some(tab) = self
                        .tabs
                        .iter_mut()
                        .find(|tab| tab.state.browser_id == browser_id)
                    {
                        tab.cursor = cursor_style(&cursor);
                    }
                }
                Event::ContextMenu {
                    browser_id,
                    x,
                    y,
                    link_url,
                    selected_text,
                    editable,
                } if Some(browser_id) == self.active => {
                    self.context_menu = Some(PageContextMenu {
                        browser_id,
                        x,
                        y,
                        link_url,
                        selected_text,
                        editable,
                    });
                }
                Event::FindResult {
                    browser_id,
                    count,
                    active_match,
                    ..
                } if Some(browser_id) == self.active => {
                    self.find_result = Some((active_match, count))
                }
                // This notification is only CEF's explicit user-link tab
                // disposition (Ctrl/middle click). JavaScript/target popups
                // retain their opener via parent-correlated Created above.
                Event::NewWindow { browser_id, url }
                    if client.owns(browser_id)
                        && shelldeck_browser::navigation_url(&url).is_ok() =>
                {
                    self.new_tab(url, cx);
                }
                Event::Popup {
                    browser_id,
                    visible,
                    x,
                    y,
                    width,
                    height,
                } => {
                    if let Some(tab) = self
                        .tabs
                        .iter_mut()
                        .find(|tab| tab.state.browser_id == browser_id)
                    {
                        tab.popup_rect = visible.then_some(Rect {
                            x,
                            y,
                            width,
                            height,
                        });
                        if !visible {
                            if let Some(image) = tab.popup_image.take() {
                                cx.drop_image(image, None);
                            }
                        }
                    }
                }
                Event::ImeRangeChanged {
                    browser_id,
                    range,
                    character_bounds,
                } if Some(browser_id) == self.active => {
                    self.selected_range = range.start as usize..range.end as usize;
                    self.ime_character_bounds = character_bounds;
                }
                Event::TextSelectionChanged {
                    browser_id,
                    selected_text,
                    range,
                } if Some(browser_id) == self.active => {
                    self.selected_text = selected_text;
                    self.selected_range = range.start as usize..range.end as usize;
                }
                Event::Dialog {
                    browser_id,
                    dialog_id,
                    kind,
                    message,
                    default_prompt,
                } => {
                    if self.page_dialog.is_none() {
                        self.select_tab(browser_id, cx);
                        self.page_dialog = Some(PageDialog {
                            browser_id,
                            dialog_id,
                            kind,
                            message,
                            default_prompt: Some(default_prompt),
                        });
                        self.dialog_focus_requested = true;
                    } else {
                        if kind == "before_unload" {
                            if let Some(tab) = self
                                .tabs
                                .iter_mut()
                                .find(|tab| tab.state.browser_id == browser_id)
                            {
                                tab.closing = false;
                            }
                        }
                        self.send(
                            Command::DialogResponse {
                                browser_id,
                                dialog_id,
                                accept: false,
                                prompt: String::new(),
                            },
                            cx,
                        );
                    }
                }
                Event::FileChooser {
                    browser_id,
                    chooser_id,
                    title,
                    multiple,
                    ..
                } => self.choose_upload(browser_id, chooser_id, title, multiple, cx),
                Event::Download {
                    browser_id,
                    download_id,
                    suggested_name,
                    ..
                } => self.choose_download(browser_id, download_id, suggested_name, cx),
                Event::DownloadProgress {
                    complete: true,
                    canceled: false,
                    ..
                } => self.status = Some(t!("native_browser.download_complete").to_string()),
                Event::Error {
                    browser_id: Some(browser_id),
                    message,
                } => {
                    if let Some(tab) = self
                        .tabs
                        .iter_mut()
                        .find(|tab| tab.state.browser_id == browser_id)
                    {
                        tab.error = Some(message);
                        tab.state.loading = false;
                        tab.closing = false;
                    }
                }
                Event::Error {
                    browser_id: None,
                    message,
                } => {
                    self.revoke_agent_control(cx);
                    self.set_error(message, cx);
                }
                _ => {}
            }
        }
        // BrowserClient retains at most the newest frame per tab/surface. A slow
        // UI never queues old animation frames or grows an unbounded texture set.
        for frame in client.take_frames() {
            if let Some(tab) = self
                .tabs
                .iter_mut()
                .find(|tab| tab.state.browser_id == frame.browser_id)
            {
                if let Some(image) = render_image(&frame) {
                    let previous = match frame.kind {
                        PaintKind::View => tab.image.replace(image),
                        PaintKind::Popup => tab.popup_image.replace(image),
                    };
                    if let Some(image) = previous {
                        cx.drop_image(image, None);
                    }
                }
            }
        }
        if let Some(error) = client.failure() {
            self.revoke_agent_control(cx);
            self.pending_urls = self
                .tabs
                .iter()
                .map(|tab| tab.state.url.clone())
                .take(MAX_TABS)
                .collect();
            for tab in self.tabs.drain(..) {
                for image in [tab.image, tab.popup_image].into_iter().flatten() {
                    cx.drop_image(image, None);
                }
            }
            self.active = None;
            self.client = None;
            self.page_dialog = None;
            self.request_tasks.clear();
            self.set_error(
                format!("{}\n{}", t!("native_browser.start_failed"), error),
                cx,
            );
            cx.emit(NativeBrowserEvent::ActiveTabChanged);
        }
        cx.notify();
    }

    fn choose_upload(
        &mut self,
        browser_id: BrowserId,
        chooser_id: u64,
        title: String,
        multiple: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(client) = self.client.clone() else {
            return;
        };
        if self.request_tasks.len() >= 16 {
            let _ = client.send(Command::FileChooserResponse {
                browser_id,
                chooser_id,
                paths: Vec::new(),
            });
            return;
        }
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple,
            prompt: Some(title.into()),
            starting_directory: None,
        });
        let request_id = uuid::Uuid::new_v4();
        let task = cx.spawn(async move |this, cx| {
            let paths = match receiver.await {
                Ok(Ok(Some(paths))) => paths
                    .into_iter()
                    .map(|path| path.to_string_lossy().into_owned())
                    .collect(),
                _ => Vec::new(),
            };
            let _ = client.send(Command::FileChooserResponse {
                browser_id,
                chooser_id,
                paths,
            });
            let _ = this.update(cx, |view, _| {
                if let Some(task) = view.request_tasks.remove(&request_id) {
                    task.detach();
                }
            });
        });
        self.request_tasks.insert(request_id, task);
    }

    fn choose_download(
        &mut self,
        browser_id: BrowserId,
        download_id: u64,
        name: String,
        cx: &mut Context<Self>,
    ) {
        let Some(client) = self.client.clone() else {
            return;
        };
        if self.request_tasks.len() >= 16 {
            let _ = client.send(Command::DownloadResponse {
                browser_id,
                download_id,
                path: None,
            });
            return;
        }
        let safe_name = std::path::Path::new(&name)
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("download")
            .to_string();
        let directory = std::env::current_dir().unwrap_or_else(|_| std::env::temp_dir());
        let receiver = cx.prompt_for_new_path(&directory, Some(&safe_name));
        let request_id = uuid::Uuid::new_v4();
        let task = cx.spawn(async move |this, cx| {
            let path = match receiver.await {
                Ok(Ok(Some(path))) => Some(path.to_string_lossy().into_owned()),
                _ => None,
            };
            let _ = client.send(Command::DownloadResponse {
                browser_id,
                download_id,
                path,
            });
            let _ = this.update(cx, |view, _| {
                if let Some(task) = view.request_tasks.remove(&request_id) {
                    task.detach();
                }
            });
        });
        self.request_tasks.insert(request_id, task);
    }

    fn resolve_dialog(&mut self, accept: bool, cx: &mut Context<Self>) {
        if let Some(dialog) = self.page_dialog.take() {
            if !accept && dialog.kind == "before_unload" {
                if let Some(tab) = self
                    .tabs
                    .iter_mut()
                    .find(|tab| tab.state.browser_id == dialog.browser_id)
                {
                    tab.closing = false;
                }
            }
            let prompt = self.dialog_input.read(cx).content().to_string();
            self.send(
                Command::DialogResponse {
                    browser_id: dialog.browser_id,
                    dialog_id: dialog.dialog_id,
                    accept,
                    prompt,
                },
                cx,
            );
            self.focus_page_requested = true;
            cx.notify();
        }
    }

    fn find(&mut self, forward: bool, find_next: bool, cx: &mut Context<Self>) {
        if let Some(browser_id) = self.active {
            let text = self.find_input.read(cx).content().to_string();
            if text.is_empty() {
                self.send(
                    Command::StopFinding {
                        browser_id,
                        clear_selection: true,
                    },
                    cx,
                );
            } else {
                self.send(
                    Command::Find {
                        browser_id,
                        text,
                        forward,
                        match_case: false,
                        find_next,
                    },
                    cx,
                );
            }
        }
    }

    fn close_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.find_visible = false;
        self.find_result = None;
        if let Some(browser_id) = self.active {
            self.send(
                Command::StopFinding {
                    browser_id,
                    clear_selection: true,
                },
                cx,
            );
        }
        self.focus.focus(window);
        cx.notify();
    }

    fn render_context_menu(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let menu = self.context_menu.as_ref()?;
        let tab = self.selected_tab()?;
        let browser_id = menu.browser_id;
        let bounds = self.viewport?;
        let position = point(
            gpui::px((menu.x as f32).clamp(0.0, (f32::from(bounds.size.width) - 245.0).max(0.0))),
            gpui::px((menu.y as f32).clamp(0.0, (f32::from(bounds.size.height) - 300.0).max(0.0))),
        );
        let mut items = Vec::new();
        for (key, disabled, command) in [
            ("back", !tab.state.can_go_back, Command::Back { browser_id }),
            (
                "forward",
                !tab.state.can_go_forward,
                Command::Forward { browser_id },
            ),
            (
                "reload",
                false,
                Command::Reload {
                    browser_id,
                    ignore_cache: false,
                },
            ),
        ] {
            let parent = cx.weak_entity();
            items.push(
                ContextMenuItem::new(key, t!(format!("native_browser.{key}")).to_string())
                    .disabled(disabled)
                    .on_click(move |_, cx| {
                        let _ = parent.update(cx, |view, cx| view.send(command.clone(), cx));
                    }),
            );
        }
        items.push(ContextMenuItem::separator());
        let url = tab.state.url.clone();
        items.push(
            ContextMenuItem::new("copy-url", t!("native_browser.copy_url").to_string()).on_click(
                move |_, cx| cx.write_to_clipboard(ClipboardItem::new_string(url.clone())),
            ),
        );
        if !menu.link_url.is_empty() {
            let url = menu.link_url.clone();
            items.push(
                ContextMenuItem::new("copy-link", t!("native_browser.copy_link").to_string())
                    .on_click(move |_, cx| {
                        cx.write_to_clipboard(ClipboardItem::new_string(url.clone()))
                    }),
            );
            if let Ok(url) = shelldeck_browser::navigation_url(&menu.link_url) {
                let parent = cx.weak_entity();
                items.push(
                    ContextMenuItem::new(
                        "open-link",
                        t!("native_browser.open_link_tab").to_string(),
                    )
                    .on_click(move |_, cx| {
                        let _ = parent.update(cx, |view, cx| view.new_tab(url.clone(), cx));
                    }),
                );
            }
        }
        if !menu.selected_text.is_empty() {
            let text = menu.selected_text.clone();
            items.push(
                ContextMenuItem::new(
                    "copy-selection",
                    t!("native_browser.copy_selection").to_string(),
                )
                .on_click(move |_, cx| {
                    cx.write_to_clipboard(ClipboardItem::new_string(text.clone()))
                }),
            );
        }
        if menu.editable {
            let parent = cx.weak_entity();
            items.push(
                ContextMenuItem::new("paste-text", t!("native_browser.paste_text").to_string())
                    .on_click(move |_, cx| {
                        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
                            let _ = parent.update(cx, |view, cx| {
                                view.send(Command::Text { browser_id, text }, cx)
                            });
                        }
                    }),
            );
        }
        let parent = cx.weak_entity();
        let menu = ContextMenu::new(position)
            .items(items)
            .on_close(move |_, cx| {
                let _ = parent.update(cx, |view, cx| {
                    view.context_menu = None;
                    cx.notify();
                });
            });
        Some(
            div()
                .absolute()
                .inset_0()
                .occlude()
                .on_any_mouse_down(|_, _, cx| cx.stop_propagation())
                .child(menu)
                .into_any_element(),
        )
    }

    fn share_screenshot(&mut self, cx: &mut Context<Self>) {
        if self.sharing {
            return;
        }
        let Some(client) = &self.client else {
            return;
        };
        let Some(tab) = self.selected_tab() else {
            return;
        };
        let Some(frame) = client.latest_frame(tab.state.browser_id) else {
            return;
        };
        let url = tab.state.url.clone();
        let title = tab.state.title.clone();
        let popup = client.latest_popup(tab.state.browser_id);
        let scale = client
            .viewport(tab.state.browser_id)
            .map(|(width, _, _)| frame.width as f32 / width as f32)
            .unwrap_or(1.0);
        self.sharing = true;
        self.share_task = Some(cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    screenshot_png(
                        &frame,
                        popup.as_ref().map(|(rect, frame)| (*rect, frame.as_ref())),
                        scale,
                    )
                })
                .await;
            let _ = this.update(cx, |view, cx| {
                view.sharing = false;
                if let Some(task) = view.share_task.take() {
                    task.detach();
                }
                match result {
                    Ok(png) => cx.emit(NativeBrowserEvent::ShareScreenshot { png, url, title }),
                    Err(error) => view.set_error(error, cx),
                }
                cx.notify();
            });
        }));
    }
}

impl Focusable for NativeBrowserView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

fn render_image(frame: &BrowserFrame) -> Option<Arc<RenderImage>> {
    // GPUI RenderImage expects BGRA. Do not perform an RGBA conversion here.
    let buffer = image::RgbaImage::from_raw(frame.width, frame.height, frame.bgra.clone())?;
    Some(Arc::new(RenderImage::new(vec![image::Frame::new(buffer)])))
}

fn screenshot_png(
    frame: &BrowserFrame,
    popup: Option<(Rect, &BrowserFrame)>,
    scale: f32,
) -> Result<Vec<u8>, String> {
    let mut rgba = frame.bgra.clone();
    for pixel in rgba.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }
    let mut image = image::RgbaImage::from_raw(frame.width, frame.height, rgba)
        .ok_or_else(|| t!("native_browser.invalid_frame").to_string())?;
    if let Some((rect, popup)) = popup {
        let mut rgba = popup.bgra.clone();
        for pixel in rgba.chunks_exact_mut(4) {
            pixel.swap(0, 2);
        }
        if let Some(popup) = image::RgbaImage::from_raw(popup.width, popup.height, rgba) {
            image::imageops::overlay(
                &mut image,
                &popup,
                (rect.x as f32 * scale).round() as i64,
                (rect.y as f32 * scale).round() as i64,
            );
        }
    }
    let mut output = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(image)
        .write_to(&mut output, image::ImageFormat::Png)
        .map_err(|error| error.to_string())?;
    let bytes = output.into_inner();
    if bytes.len() > MAX_SCREENSHOT_BYTES {
        return Err(t!("native_browser.screenshot_too_large").to_string());
    }
    Ok(bytes)
}

fn profile_component(scope: &str) -> String {
    // Root supplies a context UUID; hashing also makes this API safe for opaque
    // caller-owned identities without allowing path separators into a profile.
    let digest = scope
        .as_bytes()
        .iter()
        .fold(0xcbf29ce484222325u64, |hash, byte| {
            (hash ^ *byte as u64).wrapping_mul(0x100000001b3)
        });
    format!("{digest:016x}")
}

fn cursor_style(cursor: &str) -> CursorStyle {
    match cursor {
        "pointer" => CursorStyle::PointingHand,
        "text" => CursorStyle::IBeam,
        "vertical-text" => CursorStyle::IBeamCursorForVerticalLayout,
        "crosshair" => CursorStyle::Crosshair,
        "grab" => CursorStyle::OpenHand,
        "grabbing" => CursorStyle::ClosedHand,
        "not-allowed" => CursorStyle::OperationNotAllowed,
        "ew-resize" => CursorStyle::ResizeLeftRight,
        "ns-resize" => CursorStyle::ResizeUpDown,
        "n-resize" => CursorStyle::ResizeUp,
        "s-resize" => CursorStyle::ResizeDown,
        "w-resize" => CursorStyle::ResizeLeft,
        "e-resize" => CursorStyle::ResizeRight,
        "nesw-resize" => CursorStyle::ResizeUpRightDownLeft,
        "nwse-resize" => CursorStyle::ResizeUpLeftDownRight,
        "col-resize" => CursorStyle::ResizeColumn,
        "row-resize" => CursorStyle::ResizeRow,
        _ => CursorStyle::Arrow,
    }
}

impl Render for NativeBrowserView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if std::mem::take(&mut self.focus_page_requested) {
            self.focus.focus(window);
        }
        if self.address_force_update || !self.address.read(cx).focus_handle(cx).is_focused(window) {
            if let Some(value) = self.address_pending.take() {
                self.address
                    .update(cx, |state, cx| state.set_value(value, window, cx));
            }
            self.address_force_update = false;
        }
        let focused = self.focus.is_focused(window);
        if focused != self.has_focus {
            self.has_focus = focused;
            if let Some(browser_id) = self.active {
                self.send(
                    Command::Focus {
                        browser_id,
                        focused,
                    },
                    cx,
                );
            }
        }
        if let Some(dialog) = &mut self.page_dialog {
            if let Some(value) = dialog.default_prompt.take() {
                self.dialog_input
                    .update(cx, |state, cx| state.set_value(value, window, cx));
            }
            if self.dialog_focus_requested {
                self.dialog_focus_requested = false;
                if dialog.kind == "prompt" {
                    self.dialog_input.read(cx).focus_handle(cx).focus(window);
                } else {
                    self.dialog_focus.focus(window);
                }
            }
        }
        let mut strip = div()
            .id("native-browser-tabs")
            .flex()
            .items_center()
            .gap(px(6.0))
            .min_w_0()
            .overflow_x_scroll()
            .px(px(8.0))
            .py(px(5.0))
            .border_b_1()
            .border_color(ShellDeckColors::border());
        for tab in &self.tabs {
            let browser_id = tab.state.browser_id;
            let label = if tab.state.title.is_empty() {
                if tab.state.url == "about:blank" {
                    t!("native_browser.new_tab").to_string()
                } else {
                    tab.state.url.clone()
                }
            } else {
                tab.state.title.clone()
            };
            strip = strip.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .min_w_0()
                    .max_w(px(220.0))
                    .rounded(px(5.0))
                    .overflow_hidden()
                    .bg(if Some(browser_id) == self.active {
                        ShellDeckColors::selected_bg()
                    } else {
                        ShellDeckColors::bg_surface()
                    })
                    .child(
                        Button::new(("native-browser-tab", browser_id), label.clone())
                            .tooltip(label)
                            .size(ButtonSize::Sm)
                            .variant(ButtonVariant::Ghost)
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .on_click(
                                cx.listener(move |view, _, _, cx| view.select_tab(browser_id, cx)),
                            ),
                    )
                    .child(
                        Button::new(("native-browser-close", browser_id), "")
                            .icon("x")
                            .tooltip(t!("native_browser.close_tab").to_string())
                            .size(ButtonSize::Sm)
                            .variant(ButtonVariant::Ghost)
                            .disabled(tab.closing)
                            .flex_shrink_0()
                            .on_click(
                                cx.listener(move |view, _, _, cx| view.close_tab(browser_id, cx)),
                            ),
                    ),
            );
        }
        strip = strip.child(
            Button::new("native-browser-new", "")
                .icon("plus")
                .tooltip(t!("native_browser.new_tab").to_string())
                .size(ButtonSize::Sm)
                .variant(ButtonVariant::Ghost)
                .disabled(self.tabs.len() + self.pending_urls.len() >= MAX_TABS || self.launching)
                .on_click(cx.listener(|view, _, window, cx| {
                    view.new_tab("about:blank".into(), cx);
                    view.focus_address(window, cx);
                })),
        );
        strip = strip.child(div().flex_1()).child(
            Button::new("native-browser-close-pane", "")
                .icon("x")
                .tooltip(t!("agents.live_browser.close").to_string())
                .size(ButtonSize::Sm)
                .variant(ButtonVariant::Ghost)
                .on_click(cx.listener(|_, _, _, cx| cx.emit(NativeBrowserEvent::CloseRequested))),
        );
        let tab = self.selected_tab();
        let can_back = tab.is_some_and(|tab| tab.state.can_go_back);
        let can_forward = tab.is_some_and(|tab| tab.state.can_go_forward);
        let loading = tab.is_some_and(|tab| tab.state.loading);
        let has_frame = tab.is_some_and(|tab| tab.image.is_some());
        let parent = cx.weak_entity();
        let address = Input::new(&self.address)
            .size(InputSize::Sm)
            .variant(InputVariant::Outline)
            .placeholder(t!("native_browser.address").to_string())
            .on_enter(move |value, cx| {
                let _ = parent.update(cx, |view, cx| view.navigate(value.to_string(), cx));
            });
        let navigation = div()
            .flex()
            .items_center()
            .gap(px(6.0))
            .p(px(8.0))
            .min_w_0()
            .flex_shrink_0()
            .child(
                Button::new("native-browser-back", "")
                    .icon("chevron-left")
                    .tooltip(t!("native_browser.back").to_string())
                    .size(ButtonSize::Sm)
                    .variant(ButtonVariant::Ghost)
                    .disabled(!can_back)
                    .on_click(cx.listener(|view, _, _, cx| {
                        if let Some(browser_id) = view.active {
                            view.send(Command::Back { browser_id }, cx);
                        }
                    })),
            )
            .child(
                Button::new("native-browser-forward", "")
                    .icon("arrow-right")
                    .tooltip(t!("native_browser.forward").to_string())
                    .size(ButtonSize::Sm)
                    .variant(ButtonVariant::Ghost)
                    .disabled(!can_forward)
                    .on_click(cx.listener(|view, _, _, cx| {
                        if let Some(browser_id) = view.active {
                            view.send(Command::Forward { browser_id }, cx);
                        }
                    })),
            )
            .child(
                Button::new("native-browser-reload", "")
                    .icon(if loading { "x" } else { "rotate-ccw" })
                    .tooltip(
                        t!(if loading {
                            "native_browser.stop"
                        } else {
                            "native_browser.reload"
                        })
                        .to_string(),
                    )
                    .size(ButtonSize::Sm)
                    .variant(ButtonVariant::Ghost)
                    .disabled(self.active.is_none())
                    .on_click(cx.listener(move |view, _, _, cx| {
                        if let Some(browser_id) = view.active {
                            view.send(
                                if loading {
                                    Command::Stop { browser_id }
                                } else {
                                    Command::Reload {
                                        browser_id,
                                        ignore_cache: false,
                                    }
                                },
                                cx,
                            );
                        }
                    })),
            )
            .child(div().flex_1().min_w_0().child(address))
            .child(
                Button::new("native-browser-go", "")
                    .icon("arrow-right")
                    .tooltip(t!("native_browser.go").to_string())
                    .size(ButtonSize::Sm)
                    .variant(ButtonVariant::Ghost)
                    .disabled(self.launching)
                    .on_click(cx.listener(|view, _, _, cx| {
                        let url = view.address.read(cx).content().to_string();
                        view.navigate(url, cx);
                    })),
            );
        let mut viewport = div()
            .id("native-browser-viewport")
            .relative()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .overflow_hidden()
            .bg(ShellDeckColors::bg_primary())
            .track_focus(&self.focus)
            .key_context("NativeBrowser")
            .child(surface::BrowserSurface::new(cx.entity()));
        for button in [MouseButton::Left, MouseButton::Middle, MouseButton::Right] {
            viewport = viewport
                .on_mouse_down(
                    button,
                    cx.listener(move |view, event: &MouseDownEvent, window, cx| {
                        view.mouse_button(surface::ButtonEvent::Press(event), window, cx)
                    }),
                )
                .on_mouse_up(
                    button,
                    cx.listener(move |view, event: &MouseUpEvent, window, cx| {
                        view.mouse_button(surface::ButtonEvent::Release(event), window, cx)
                    }),
                )
                .on_mouse_up_out(
                    button,
                    cx.listener(move |view, event: &MouseUpEvent, window, cx| {
                        view.mouse_button(surface::ButtonEvent::Release(event), window, cx)
                    }),
                );
        }
        viewport = viewport
            .on_scroll_wheel(
                cx.listener(|view, event: &ScrollWheelEvent, _, cx| view.mouse_wheel(event, cx)),
            )
            .on_key_down(cx.listener(|view, event: &KeyDownEvent, window, cx| {
                view.key(event.keystroke.clone(), false, window, cx)
            }))
            .on_key_up(cx.listener(|view, event: &KeyUpEvent, window, cx| {
                view.key(event.keystroke.clone(), true, window, cx)
            }));
        if self.tabs.is_empty() {
            viewport = viewport.child(
                div()
                    .absolute()
                    .inset_0()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap(px(12.0))
                    .p(px(24.0))
                    .child(crate::icons::lucide_icon(
                        "globe",
                        28.0,
                        ShellDeckColors::text_muted(),
                    ))
                    .child(
                        div()
                            .text_size(px(15.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(t!("native_browser.title").to_string()),
                    )
                    .child(
                        div()
                            .max_w(px(460.0))
                            .text_center()
                            .text_color(ShellDeckColors::text_muted())
                            .child(
                                t!(if self.launching {
                                    "native_browser.starting"
                                } else {
                                    "native_browser.empty"
                                })
                                .to_string(),
                            ),
                    ),
            );
        }
        if let Some(menu) = self.render_context_menu(cx) {
            viewport = viewport.child(menu);
        }
        let mut shell = shortcuts::handlers(
            div()
                .id("native-browser")
                .key_context("NativeBrowserChrome")
                .relative()
                .flex()
                .flex_col()
                .size_full()
                .min_w_0()
                .min_h_0()
                .overflow_hidden()
                .bg(ShellDeckColors::bg_surface())
                .text_color(ShellDeckColors::text_primary())
                .text_size(px(12.0))
                .child(strip)
                .child(navigation),
            cx,
        );
        if self.find_visible {
            let parent = cx.weak_entity();
            let enter = cx.weak_entity();
            shell = shell.child(
                div()
                    .id("native-browser-find-row")
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .px(px(8.0))
                    .pb(px(8.0))
                    .flex_shrink_0()
                    .min_w_0()
                    .capture_key_down(cx.listener(|view, event: &KeyDownEvent, window, cx| {
                        if event.keystroke.key == "escape" {
                            view.close_find(window, cx);
                            cx.stop_propagation();
                        }
                    }))
                    .child(
                        div().flex_1().min_w_0().child(
                            Input::new(&self.find_input)
                                .size(InputSize::Sm)
                                .variant(InputVariant::Outline)
                                .placeholder(t!("native_browser.find").to_string())
                                .on_change(move |_, cx| {
                                    let _ =
                                        parent.update(cx, |view, cx| view.find(true, false, cx));
                                })
                                .on_enter(move |_, cx| {
                                    let _ = enter.update(cx, |view, cx| view.find(true, true, cx));
                                }),
                        ),
                    )
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_color(ShellDeckColors::text_muted())
                            .child(
                                self.find_result
                                    .map(|(active, count)| format!("{active} / {count}"))
                                    .unwrap_or_default(),
                            ),
                    )
                    .child(
                        Button::new("native-browser-find-previous", "")
                            .icon("chevron-up")
                            .size(ButtonSize::Sm)
                            .variant(ButtonVariant::Ghost)
                            .tooltip(t!("native_browser.find_previous").to_string())
                            .on_click(cx.listener(|view, _, _, cx| view.find(false, true, cx))),
                    )
                    .child(
                        Button::new("native-browser-find-next", "")
                            .icon("chevron-down")
                            .size(ButtonSize::Sm)
                            .variant(ButtonVariant::Ghost)
                            .tooltip(t!("native_browser.find_next").to_string())
                            .on_click(cx.listener(|view, _, _, cx| view.find(true, true, cx))),
                    )
                    .child(
                        Button::new("native-browser-find-close", "")
                            .icon("x")
                            .size(ButtonSize::Sm)
                            .variant(ButtonVariant::Ghost)
                            .tooltip(t!("native_browser.close_find").to_string())
                            .on_click(
                                cx.listener(|view, _, window, cx| view.close_find(window, cx)),
                            ),
                    ),
            );
        }
        let error = self
            .error
            .clone()
            .or_else(|| self.selected_tab().and_then(|tab| tab.error.clone()));
        if let Some(error) = error {
            let mut banner = div()
                .flex()
                .items_start()
                .gap(px(8.0))
                .px(px(12.0))
                .py(px(8.0))
                .flex_shrink_0()
                .bg(ShellDeckColors::error().opacity(0.08))
                .text_color(ShellDeckColors::error())
                .child(div().flex_1().min_w_0().line_clamp(4).child(error));
            if self.client.is_none() {
                banner = banner.child(
                    Button::new(
                        "native-browser-retry",
                        t!("native_browser.retry").to_string(),
                    )
                    .size(ButtonSize::Sm)
                    .variant(ButtonVariant::Ghost)
                    .on_click(cx.listener(|view, _, _, cx| view.ensure_engine(cx))),
                );
            }
            shell = shell.child(banner);
        }
        shell = shell.child(viewport).child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap(px(8.0))
                .px(px(8.0))
                .py(px(4.0))
                .min_w_0()
                .flex_shrink_0()
                .border_t_1()
                .border_color(ShellDeckColors::border())
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_color(ShellDeckColors::text_muted())
                        .child(if let Some(status) = &self.status {
                            status.clone()
                        } else {
                            t!(if loading {
                                "native_browser.loading"
                            } else if !self.agent_observation_available {
                                "native_browser.manual_only"
                            } else {
                                "native_browser.shared_page"
                            })
                            .to_string()
                        }),
                )
                .when(self.agent_observation_available, |footer| {
                    footer.child(
                        Button::new(
                            "native-browser-control",
                            t!(if self.agent_control_allowed {
                                "native_browser.revoke_control"
                            } else {
                                "native_browser.allow_control"
                            })
                            .to_string(),
                        )
                        .size(ButtonSize::Sm)
                        .variant(if self.agent_control_allowed {
                            ButtonVariant::Secondary
                        } else {
                            ButtonVariant::Ghost
                        })
                        .disabled(self.client.is_none() || !self.agent_control_available)
                        .tooltip(
                            t!(if self.agent_control_available {
                                "native_browser.control_info"
                            } else {
                                "native_browser.control_unavailable"
                            })
                            .to_string(),
                        )
                        .on_click(cx.listener(|view, _, _, cx| {
                            if !view.agent_control_available {
                                return;
                            }
                            view.agent_control_allowed = !view.agent_control_allowed;
                            cx.emit(NativeBrowserEvent::ControlChanged {
                                allowed: view.agent_control_allowed,
                            });
                            cx.notify();
                        })),
                    )
                })
                .child(
                    Button::new("native-browser-share", "")
                        .icon("paperclip")
                        .tooltip(t!("native_browser.share_screenshot").to_string())
                        .size(ButtonSize::Sm)
                        .variant(ButtonVariant::Ghost)
                        .disabled(!has_frame || self.sharing)
                        .on_click(cx.listener(|view, _, _, cx| view.share_screenshot(cx))),
                ),
        );
        if let Some(dialog) = &self.page_dialog {
            let mut content = div()
                .id("native-browser-dialog-content")
                .flex()
                .flex_col()
                .gap(px(12.0))
                .p(px(16.0))
                .max_h(px(280.0))
                .overflow_y_scroll()
                .child(dialog.message.clone());
            if dialog.kind == "prompt" {
                content = content.child(
                    Input::new(&self.dialog_input)
                        .size(InputSize::Sm)
                        .variant(InputVariant::Outline),
                );
            }
            let modal = UiDialog::new()
                .width(gpui::px(440.0))
                .header(
                    div().p(px(16.0)).font_weight(FontWeight::SEMIBOLD).child(
                        t!(if dialog.kind == "before_unload" {
                            "native_browser.leave_page_title"
                        } else {
                            "native_browser.page_dialog"
                        })
                        .to_string(),
                    ),
                )
                .content(content)
                .footer(
                    div()
                        .flex()
                        .justify_end()
                        .gap(px(8.0))
                        .p(px(12.0))
                        .child(
                            Button::new(
                                "native-browser-dialog-cancel",
                                t!(if dialog.kind == "before_unload" {
                                    "native_browser.stay_on_page"
                                } else {
                                    "native_browser.cancel"
                                })
                                .to_string(),
                            )
                            .size(ButtonSize::Sm)
                            .variant(ButtonVariant::Ghost)
                            .on_click(cx.listener(|view, _, _, cx| view.resolve_dialog(false, cx))),
                        )
                        .child(
                            Button::new(
                                "native-browser-dialog-accept",
                                t!(if dialog.kind == "before_unload" {
                                    "native_browser.leave_page"
                                } else {
                                    "native_browser.accept"
                                })
                                .to_string(),
                            )
                            .size(ButtonSize::Sm)
                            .on_click(cx.listener(|view, _, _, cx| view.resolve_dialog(true, cx))),
                        ),
                );
            shell = shell.child(
                div()
                    .absolute()
                    .inset_0()
                    .track_focus(&self.dialog_focus)
                    .key_context("NativeBrowserDialog")
                    .capture_key_down(cx.listener(|view, event: &KeyDownEvent, _, cx| {
                        match event.keystroke.key.as_str() {
                            "escape" => {
                                view.resolve_dialog(false, cx);
                                cx.stop_propagation();
                            }
                            "enter" => {
                                view.resolve_dialog(true, cx);
                                cx.stop_propagation();
                            }
                            _ => {}
                        }
                    }))
                    .child(modal),
            );
        }
        shell
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

    fn frame(width: u32, height: u32, bgra: Vec<u8>) -> BrowserFrame {
        BrowserFrame {
            browser_id: 1,
            kind: PaintKind::View,
            width,
            height,
            dirty_rects: Vec::new(),
            bgra,
        }
    }

    // SDTEST-2573 / SDUC-557: the live GPUI texture stays BGRA, while a shared
    // PNG has conventional RGBA channels; otherwise every red/blue page lies.
    #[test]
    fn sdtest_2573_browser_frame_and_shared_png_preserve_color_channels() {
        let source = frame(2, 1, vec![0, 0, 255, 255, 255, 0, 0, 255]);
        let texture = render_image(&source).unwrap();
        assert_eq!(texture.as_bytes(0).unwrap(), source.bgra);
        let png = screenshot_png(&source, None, 1.0).unwrap();
        let decoded = image::load_from_memory(&png).unwrap().into_rgba8();
        assert_eq!(decoded.into_raw(), vec![255, 0, 0, 255, 0, 0, 255, 255]);
        assert!(screenshot_png(&frame(2, 1, vec![0; 7]), None, 1.0).is_err());
    }

    // SDTEST-2574 / SDUC-557: retained profile directories are bound to opaque
    // session/context identities and cannot escape through a caller's scope.
    #[test]
    fn sdtest_2574_browser_profile_identity_cannot_escape_its_directory() {
        let scope = "../../another/session\\profile";
        let first = profile_component(scope);
        assert!(first.len() == 16 && first.bytes().all(|byte| byte.is_ascii_hexdigit()));
        assert_eq!(first, profile_component(scope));
        assert_ne!(first, profile_component("another-session-context"));
    }

    // SDTEST-2575 / SDUC-557: screenshot sharing includes the visible native
    // select popup at its HiDPI position, clipping it at the viewport boundary.
    #[test]
    fn sdtest_2575_shared_screenshot_composes_and_clips_browser_popups() {
        let page = frame(4, 2, vec![255; 32]);
        let popup = frame(
            2,
            2,
            vec![
                0, 0, 255, 255, 0, 0, 255, 255, 0, 0, 255, 255, 0, 0, 255, 255,
            ],
        );
        let rect = Rect {
            x: -1,
            y: 0,
            width: 1,
            height: 1,
        };
        let png = screenshot_png(&page, Some((rect, &popup)), 1.0).unwrap();
        let decoded = image::load_from_memory(&png).unwrap().into_rgba8();
        assert_eq!(decoded.get_pixel(0, 0).0, [255, 0, 0, 255]);
        assert_eq!(decoded.get_pixel(1, 0).0, [255, 255, 255, 255]);
        let rect = Rect {
            x: 1,
            y: 0,
            width: 1,
            height: 1,
        };
        let png = screenshot_png(&page, Some((rect, &popup)), 2.0).unwrap();
        let decoded = image::load_from_memory(&png).unwrap().into_rgba8();
        assert_eq!(decoded.get_pixel(1, 0).0, [255, 255, 255, 255]);
        assert_eq!(decoded.get_pixel(2, 0).0, [255, 0, 0, 255]);
    }
}
