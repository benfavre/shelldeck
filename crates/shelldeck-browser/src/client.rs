use crate::protocol::*;
use parking_lot::Mutex;
use serde_json::Value;
use std::{
    collections::{HashMap, HashSet, VecDeque},
    io,
    path::{Path, PathBuf},
    process::{Child, Command as ProcessCommand, Stdio},
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc, Arc, Weak,
    },
    time::Duration,
};

const EVENT_LIMIT: usize = 256;
const COMMAND_LIMIT: usize = 256;
const PENDING_LIMIT: usize = 32;
type Reply = Result<Value, String>;
type Pending = (BrowserId, mpsc::SyncSender<Reply>);
type Waker = Arc<dyn Fn() + Send + Sync>;

#[derive(Clone, Debug)]
pub struct BrowserLaunch {
    pub executable: PathBuf,
    pub profile_dir: PathBuf,
}

#[derive(Clone, Debug)]
pub struct BrowserFrame {
    pub browser_id: BrowserId,
    pub kind: PaintKind,
    pub width: u32,
    pub height: u32,
    pub dirty_rects: Vec<Rect>,
    pub bgra: Vec<u8>,
}

struct Inner {
    child: Mutex<Option<Child>>,
    commands: mpsc::SyncSender<Command>,
    events: Mutex<VecDeque<Event>>,
    latest: Mutex<HashMap<(BrowserId, PaintKind), Arc<BrowserFrame>>>,
    changed: Mutex<HashSet<(BrowserId, PaintKind)>>,
    states: Mutex<HashMap<BrowserId, BrowserState>>,
    viewports: Mutex<HashMap<BrowserId, (u32, u32, f32)>>,
    popups: Mutex<HashMap<BrowserId, Rect>>,
    parents: Mutex<HashMap<BrowserId, BrowserId>>,
    owned: Mutex<HashSet<BrowserId>>,
    pending: Mutex<HashMap<RequestId, Pending>>,
    waker: Mutex<Option<Waker>>,
    next_id: AtomicU64,
    next_request: AtomicU64,
    failure: Mutex<Option<String>>,
    control_epoch: AtomicU64,
    launch: BrowserLaunch,
}

/// Shared retained browser connection. All writes are bounded and never block the UI.
#[derive(Clone)]
pub struct BrowserClient {
    inner: Arc<Inner>,
}

impl std::fmt::Debug for BrowserClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BrowserClient")
            .field("tabs", &self.inner.owned.lock().len())
            .finish()
    }
}

impl BrowserClient {
    pub fn spawn(launch: BrowserLaunch) -> io::Result<Self> {
        if !launch.profile_dir.is_absolute() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "browser profile must be an explicit absolute path",
            ));
        }
        std::fs::create_dir_all(&launch.profile_dir)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&launch.profile_dir, std::fs::Permissions::from_mode(0o700))?;
        }
        let mut process = ProcessCommand::new(&launch.executable);
        process
            .arg("--runtime")
            .arg("--profile")
            .arg(&launch.profile_dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());
        // Affect the helper only; never alter GPUI's process-global environment.
        #[cfg(any(target_os = "linux", target_os = "windows"))]
        if let Some(directory) = launch.executable.parent() {
            #[cfg(target_os = "linux")]
            {
                let mut paths = vec![directory.to_path_buf()];
                if let Some(existing) = std::env::var_os("LD_LIBRARY_PATH") {
                    paths.extend(std::env::split_paths(&existing));
                }
                if let Ok(value) = std::env::join_paths(paths) {
                    process.env("LD_LIBRARY_PATH", value);
                }
            }
            #[cfg(target_os = "windows")]
            {
                let mut paths = vec![directory.to_path_buf()];
                if let Some(existing) = std::env::var_os("PATH") {
                    paths.extend(std::env::split_paths(&existing));
                }
                if let Ok(value) = std::env::join_paths(paths) {
                    process.env("PATH", value);
                }
            }
        }
        #[cfg(target_os = "windows")]
        {
            use std::os::windows::process::CommandExt;
            process.creation_flags(0x08000000); // CREATE_NO_WINDOW, helper owns no console.
        }
        let mut child = process.spawn()?;
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| io::Error::other("browser helper has no stdin"))?;
        let mut stdout = child
            .stdout
            .take()
            .ok_or_else(|| io::Error::other("browser helper has no stdout"))?;
        let (sender, receiver) = mpsc::sync_channel(COMMAND_LIMIT);
        let client = Self {
            inner: Arc::new(Inner {
                child: Mutex::new(Some(child)),
                commands: sender,
                events: Mutex::new(VecDeque::new()),
                latest: Mutex::new(HashMap::new()),
                changed: Mutex::new(HashSet::new()),
                states: Mutex::new(HashMap::new()),
                viewports: Mutex::new(HashMap::new()),
                popups: Mutex::new(HashMap::new()),
                parents: Mutex::new(HashMap::new()),
                owned: Mutex::new(HashSet::new()),
                pending: Mutex::new(HashMap::new()),
                waker: Mutex::new(None),
                next_id: AtomicU64::new(1),
                next_request: AtomicU64::new(1),
                failure: Mutex::new(None),
                control_epoch: AtomicU64::new(1),
                launch,
            }),
        };
        let weak = Arc::downgrade(&client.inner);
        std::thread::Builder::new()
            .name("browser-writer".into())
            .spawn(move || {
                while let Ok(command) = receiver.recv() {
                    if let Err(error) = write_packet(&mut stdin, &command, &[]) {
                        fail(&weak, error.to_string());
                        break;
                    }
                    if matches!(command, Command::Shutdown) {
                        break;
                    }
                }
            })?;
        let weak = Arc::downgrade(&client.inner);
        std::thread::Builder::new()
            .name("browser-reader".into())
            .spawn(move || loop {
                match read_packet::<_, Event>(&mut stdout) {
                    Ok((event, payload)) => {
                        let Some(inner) = weak.upgrade() else {
                            break;
                        };
                        if let Err(error) = receive(&inner, event, payload) {
                            fail(&weak, error.to_string());
                            break;
                        }
                    }
                    Err(error) => {
                        fail(&weak, format!("Browser engine disconnected: {error}"));
                        break;
                    }
                }
            })?;
        Ok(client)
    }

    pub fn set_waker(&self, waker: Waker) {
        *self.inner.waker.lock() = Some(waker);
        wake(&self.inner);
    }
    pub fn clear_waker(&self) {
        *self.inner.waker.lock() = None;
    }
    pub fn launch(&self) -> &BrowserLaunch {
        &self.inner.launch
    }
    pub fn failure(&self) -> Option<String> {
        self.inner.failure.lock().clone()
    }
    pub fn owned_ids(&self) -> Vec<BrowserId> {
        self.inner.owned.lock().iter().copied().collect()
    }
    pub fn owns(&self, id: BrowserId) -> bool {
        self.inner.owned.lock().contains(&id)
    }
    pub fn parent_id(&self, browser_id: BrowserId) -> Option<BrowserId> {
        self.inner.parents.lock().get(&browser_id).copied()
    }
    pub fn states(&self) -> Vec<BrowserState> {
        self.inner.states.lock().values().cloned().collect()
    }
    pub fn state(&self, id: BrowserId) -> Option<BrowserState> {
        self.inner.states.lock().get(&id).cloned()
    }
    pub fn drain_events(&self) -> Vec<Event> {
        self.inner.events.lock().drain(..).collect()
    }
    pub fn viewport(&self, browser_id: BrowserId) -> Option<(u32, u32, f32)> {
        self.inner.viewports.lock().get(&browser_id).copied()
    }
    pub fn latest_frame(&self, browser_id: BrowserId) -> Option<Arc<BrowserFrame>> {
        self.inner
            .latest
            .lock()
            .get(&(browser_id, PaintKind::View))
            .cloned()
    }
    pub fn latest_popup(&self, browser_id: BrowserId) -> Option<(Rect, Arc<BrowserFrame>)> {
        let rect = self.inner.popups.lock().get(&browser_id).copied()?;
        let frame = self
            .inner
            .latest
            .lock()
            .get(&(browser_id, PaintKind::Popup))
            .cloned()?;
        Some((rect, frame))
    }
    pub fn take_frames(&self) -> Vec<Arc<BrowserFrame>> {
        let keys: Vec<_> = self.inner.changed.lock().drain().collect();
        let latest = self.inner.latest.lock();
        keys.into_iter()
            .filter_map(|key| latest.get(&key).cloned())
            .collect()
    }

    pub fn create_tab(
        &self,
        url: &str,
        width: u32,
        height: u32,
        scale_factor: f32,
    ) -> io::Result<BrowserId> {
        let id = self.inner.next_id.fetch_add(1, Ordering::Relaxed);
        self.send(Command::Create {
            browser_id: id,
            url: navigation_url(url).map_err(io::Error::other)?,
            width,
            height,
            scale_factor,
        })?;
        Ok(id)
    }

    pub fn send(&self, command: Command) -> io::Result<()> {
        if let Some(error) = self.failure() {
            return Err(io::Error::new(io::ErrorKind::BrokenPipe, error));
        }
        validate_command(&command)?;
        let mut owned = self.inner.owned.lock();
        let creating = if let Command::Create { browser_id, .. } = &command {
            if *browser_id >= (1_u64 << 63) {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "high browser IDs are reserved for correlated Chromium popups",
                ));
            }
            if owned.len() >= MAX_TABS || owned.contains(browser_id) {
                return Err(io::Error::other(
                    "browser tab limit reached or tab already exists",
                ));
            }
            owned.insert(*browser_id);
            Some(*browser_id)
        } else {
            if let Some(id) = command.browser_id() {
                if !owned.contains(&id) {
                    return Err(io::Error::new(
                        io::ErrorKind::PermissionDenied,
                        "browser tab is not owned by this client",
                    ));
                }
            }
            None
        };
        let viewport = match &command {
            Command::Create {
                browser_id,
                width,
                height,
                scale_factor,
                ..
            }
            | Command::Resize {
                browser_id,
                width,
                height,
                scale_factor,
            } => Some((*browser_id, (*width, *height, *scale_factor))),
            _ => None,
        };
        let result = self.inner.commands.try_send(command).map_err(|error| {
            io::Error::new(
                io::ErrorKind::WouldBlock,
                format!("browser command queue unavailable: {error}"),
            )
        });
        if result.is_err() {
            if let Some(id) = creating {
                owned.remove(&id);
            }
        }
        if result.is_ok() {
            if let Some((id, viewport)) = viewport {
                self.inner.viewports.lock().insert(id, viewport);
            }
        }
        result
    }

    /// Run on a background executor. The UI never waits for DevTools replies.
    pub fn devtools(
        &self,
        browser_id: BrowserId,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> Result<Value, String> {
        let request_id = self.inner.next_request.fetch_add(1, Ordering::Relaxed);
        let (sender, receiver) = mpsc::sync_channel(1);
        {
            let mut pending = self.inner.pending.lock();
            if pending.len() >= PENDING_LIMIT {
                return Err("too many outstanding browser requests".into());
            }
            pending.insert(request_id, (browser_id, sender));
        }
        let result = self
            .send(Command::DevTools {
                browser_id,
                request_id,
                method: method.into(),
                params,
            })
            .map_err(|e| e.to_string())
            .and_then(|()| {
                receiver
                    .recv_timeout(timeout)
                    .map_err(|_| "browser request timed out".into())
            })
            .and_then(|v| v);
        self.inner.pending.lock().remove(&request_id);
        result
    }

    /// Invalidates all existing control grants while leaving observation grants usable.
    pub fn revoke_control_grants(&self) {
        self.inner.control_epoch.fetch_add(1, Ordering::SeqCst);
    }
    pub(crate) fn control_epoch(&self) -> u64 {
        self.inner.control_epoch.load(Ordering::SeqCst)
    }
}

pub fn discover_helper() -> Option<PathBuf> {
    let executable = std::env::current_exe().ok()?;
    discover_helper_near(&executable)
}

pub fn discover_helper_near(executable: &Path) -> Option<PathBuf> {
    let root = executable.parent()?;
    let name = if cfg!(target_os = "windows") {
        "shelldeck-browser.exe"
    } else {
        "shelldeck-browser"
    };
    let candidates = [
        root.join("browser")
            .join(env!("CARGO_PKG_VERSION"))
            .join(name),
        root.join("browser").join(name),
        root.join(name),
        root.join("..")
            .join("Frameworks")
            .join("browser")
            .join("ShellDeck Browser.app")
            .join("Contents")
            .join("MacOS")
            .join(name),
        root.join("..")
            .join("Frameworks")
            .join("ShellDeck Browser.app")
            .join("Contents")
            .join("MacOS")
            .join(name),
    ];
    candidates.into_iter().find(|path| path.is_file())
}

fn validate_command(command: &Command) -> io::Result<()> {
    match command {
        Command::Create { url, .. } | Command::Navigate { url, .. } => {
            navigation_url(url).map_err(io::Error::other)?;
            if let Command::Create {
                width,
                height,
                scale_factor,
                ..
            } = command
            {
                valid_viewport(*width, *height, *scale_factor)?;
            }
        }
        Command::Resize {
            width,
            height,
            scale_factor,
            ..
        } => valid_viewport(*width, *height, *scale_factor)?,
        Command::Text { text, .. }
        | Command::Ime {
            event: ImeEvent::Commit { text, .. },
            ..
        }
        | Command::Ime {
            event: ImeEvent::SetComposition { text, .. },
            ..
        } if text.len() > 64 * 1024 => {
            return Err(io::Error::other("browser text input too large"))
        }
        _ => {}
    }
    if serde_json::to_vec(command).map_err(io::Error::other)?.len() > 1024 * 1024 - 128 {
        return Err(io::Error::other("browser command too large"));
    }
    Ok(())
}

fn valid_viewport(width: u32, height: u32, scale: f32) -> io::Result<()> {
    frame_byte_len(width, height)?;
    if !scale.is_finite() || !(0.25..=4.0).contains(&scale) {
        return Err(io::Error::other("invalid browser display scale"));
    }
    Ok(())
}

fn receive(inner: &Arc<Inner>, event: Event, payload: Vec<u8>) -> io::Result<()> {
    if !matches!(&event, Event::Paint { .. }) && !payload.is_empty() {
        return Err(io::Error::other(
            "Unexpected bytes on browser control event",
        ));
    }
    match event {
        Event::Paint {
            browser_id,
            kind,
            width,
            height,
            dirty_rects,
        } => {
            if !inner.owned.lock().contains(&browser_id) {
                return Ok(());
            }
            if frame_byte_len(width, height)? != payload.len() {
                return Err(io::Error::other(
                    "browser frame length does not match dimensions",
                ));
            }
            let key = (browser_id, kind);
            let frame = Arc::new(BrowserFrame {
                browser_id,
                kind,
                width,
                height,
                dirty_rects,
                bgra: payload,
            });
            {
                let mut latest = inner.latest.lock();
                latest.insert(key, frame);
                while latest.values().map(|frame| frame.bgra.len()).sum::<usize>()
                    > 128 * 1024 * 1024
                {
                    let Some(old) = latest.keys().find(|other| **other != key).copied() else {
                        break;
                    };
                    latest.remove(&old);
                }
            }
            inner.changed.lock().insert(key);
        }
        Event::DevToolsResult {
            browser_id,
            request_id,
            result,
            error,
        } => {
            if let Some((expected_id, sender)) = inner.pending.lock().remove(&request_id) {
                let reply = if browser_id == expected_id {
                    error.map_or(Ok(result), Err)
                } else {
                    Err("Browser response targeted a different tab".into())
                };
                let _ = sender.try_send(reply);
            }
        }
        event => {
            if !payload.is_empty() {
                return Err(io::Error::other(
                    "unexpected payload on browser control event",
                ));
            }
            match &event {
                Event::Ready {
                    protocol_version, ..
                } if *protocol_version != PROTOCOL_VERSION => {
                    return Err(io::Error::other("browser helper protocol version mismatch"))
                }
                Event::State { state } if inner.owned.lock().contains(&state.browser_id) => {
                    inner.states.lock().insert(state.browser_id, state.clone());
                }
                Event::Created {
                    browser_id,
                    parent_id: Some(parent_id),
                } => {
                    let mut owned = inner.owned.lock();
                    if *browser_id < (1_u64 << 63)
                        || !owned.contains(parent_id)
                        || owned.len() >= MAX_TABS
                        || owned.contains(browser_id)
                    {
                        let _ = inner.commands.try_send(Command::Close {
                            browser_id: *browser_id,
                        });
                        return Ok(());
                    }
                    owned.insert(*browser_id);
                    inner.parents.lock().insert(*browser_id, *parent_id);
                    let viewport = inner.viewports.lock().get(parent_id).copied();
                    if let Some(viewport) = viewport {
                        inner.viewports.lock().insert(*browser_id, viewport);
                    }
                }
                Event::Popup {
                    browser_id,
                    visible,
                    x,
                    y,
                    width,
                    height,
                } => {
                    if *visible && *width > 0 && *height > 0 {
                        self::insert_popup(
                            inner,
                            *browser_id,
                            Rect {
                                x: *x,
                                y: *y,
                                width: *width,
                                height: *height,
                            },
                        );
                    } else {
                        inner.popups.lock().remove(browser_id);
                    }
                }
                Event::Closed { browser_id } => {
                    inner.owned.lock().remove(browser_id);
                    inner.states.lock().remove(browser_id);
                    inner.viewports.lock().remove(browser_id);
                    inner.popups.lock().remove(browser_id);
                    inner.parents.lock().remove(browser_id);
                    inner.latest.lock().retain(|(id, _), _| id != browser_id);
                    inner.changed.lock().retain(|(id, _)| id != browser_id);
                }
                _ => {}
            }
            push_event(inner, event);
        }
    }
    wake(inner);
    Ok(())
}

fn push_event(inner: &Inner, event: Event) {
    let mut events = inner.events.lock();
    if events.len() >= EVENT_LIMIT {
        events.pop_front();
    }
    events.push_back(event);
}
fn wake(inner: &Inner) {
    let callback = inner.waker.lock().clone();
    if let Some(callback) = callback {
        callback();
    }
}
fn fail(weak: &Weak<Inner>, message: String) {
    if let Some(inner) = weak.upgrade() {
        if inner.failure.lock().is_some() {
            return;
        }
        *inner.failure.lock() = Some(message.clone());
        for (_, (_, sender)) in inner.pending.lock().drain() {
            let _ = sender.try_send(Err(message.clone()));
        }
        push_event(
            &inner,
            Event::Error {
                browser_id: None,
                message,
            },
        );
        wake(&inner);
    }
}
impl Drop for Inner {
    fn drop(&mut self) {
        let _ = self.commands.try_send(Command::Shutdown);
        if let Some(child) = self.child.get_mut().take() {
            // Chromium must flush its dedicated cookies/profile while GPUI stays
            // responsive. The writer owns only stdin + Weak<Inner>; channel EOF
            // also closes stdin and requests a graceful helper shutdown.
            let process = Arc::new(Mutex::new(Some(child)));
            let background = process.clone();
            if std::thread::Builder::new()
                .name("browser-reaper".into())
                .spawn(move || {
                    if let Some(mut child) = background.lock().take() {
                        reap_process(&mut child, std::thread::sleep);
                    }
                })
                .is_err()
            {
                // Thread creation exhaustion must not strand a browser helper.
                // kill() is a non-waiting OS request; never wait on the UI thread.
                if let Some(mut child) = process.lock().take() {
                    let _ = child.kill();
                }
            }
        }
    }
}

trait ReapProcess {
    fn exited(&mut self) -> io::Result<bool>;
    fn terminate(&mut self);
    fn reap(&mut self);
}
impl ReapProcess for Child {
    fn exited(&mut self) -> io::Result<bool> {
        self.try_wait().map(|status| status.is_some())
    }
    fn terminate(&mut self) {
        let _ = self.kill();
    }
    fn reap(&mut self) {
        let _ = self.wait();
    }
}
fn reap_process(process: &mut impl ReapProcess, mut pause: impl FnMut(Duration)) {
    for _ in 0..60 {
        match process.exited() {
            Ok(true) => return,
            Ok(false) => pause(Duration::from_millis(50)),
            Err(_) => break,
        }
    }
    if matches!(process.exited(), Ok(true)) {
        return;
    }
    process.terminate();
    process.reap();
}

#[cfg(test)]
pub(crate) fn test_client() -> (BrowserClient, mpsc::Receiver<Command>) {
    let (sender, receiver) = mpsc::sync_channel(COMMAND_LIMIT);
    let client = BrowserClient {
        inner: Arc::new(Inner {
            child: Mutex::new(None),
            commands: sender,
            events: Mutex::new(VecDeque::new()),
            latest: Mutex::new(HashMap::new()),
            changed: Mutex::new(HashSet::new()),
            states: Mutex::new(HashMap::new()),
            viewports: Mutex::new(HashMap::new()),
            popups: Mutex::new(HashMap::new()),
            parents: Mutex::new(HashMap::new()),
            owned: Mutex::new(HashSet::new()),
            pending: Mutex::new(HashMap::new()),
            waker: Mutex::new(None),
            next_id: AtomicU64::new(1),
            next_request: AtomicU64::new(1),
            failure: Mutex::new(None),
            control_epoch: AtomicU64::new(1),
            launch: BrowserLaunch {
                executable: PathBuf::from("browser-helper"),
                profile_dir: std::env::temp_dir().join("browser-test-unused"),
            },
        }),
    };
    (client, receiver)
}

fn insert_popup(inner: &Inner, browser_id: BrowserId, rect: Rect) {
    if inner.owned.lock().contains(&browser_id) {
        inner.popups.lock().insert(browser_id, rect);
    }
}

#[cfg(test)]
pub(crate) fn test_receive(
    client: &BrowserClient,
    event: Event,
    payload: Vec<u8>,
) -> io::Result<()> {
    receive(&client.inner, event, payload)
}

#[cfg(test)]
mod tests {
    use super::*;
    // SDTEST-2534 — SDUC-557: paint bursts replace frames rather than grow queues; foreign tabs and malformed buffers fail closed.
    #[test]
    fn sdtest_2534_browser_mailbox_and_command_pressure() {
        let (client, receiver) = test_client();
        let id = client.create_tab("about:blank", 8, 8, 1.0).unwrap();
        assert!(
            matches!(receiver.recv().unwrap(), Command::Create{browser_id,..} if browser_id==id)
        );
        for n in 0..200 {
            receive(
                &client.inner,
                Event::Paint {
                    browser_id: id,
                    kind: PaintKind::View,
                    width: 2,
                    height: 2,
                    dirty_rects: vec![],
                },
                vec![n; 16],
            )
            .unwrap();
        }
        let frames = client.take_frames();
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].bgra, vec![199; 16]);
        assert!(client.take_frames().is_empty());
        assert!(client.drain_events().is_empty());
        receive(
            &client.inner,
            Event::Paint {
                browser_id: 99,
                kind: PaintKind::View,
                width: 2,
                height: 2,
                dirty_rects: vec![],
            },
            vec![0; 16],
        )
        .unwrap();
        assert!(client.latest_frame(99).is_none());
        assert!(receive(
            &client.inner,
            Event::Paint {
                browser_id: id,
                kind: PaintKind::View,
                width: 2,
                height: 2,
                dirty_rects: vec![]
            },
            vec![0; 15]
        )
        .is_err());
        assert!(client
            .send(Command::Navigate {
                browser_id: 99,
                url: "https://example.org".into()
            })
            .is_err());
        for _ in 0..COMMAND_LIMIT {
            client
                .send(Command::Reload {
                    browser_id: id,
                    ignore_cache: false,
                })
                .unwrap();
        }
        assert_eq!(
            client
                .send(Command::Reload {
                    browser_id: id,
                    ignore_cache: false
                })
                .unwrap_err()
                .kind(),
            io::ErrorKind::WouldBlock
        );
    }
    // SDTEST-2539 — SDUC-557: Chromium popup adoption requires a known parent,
    // reserved child IDs and room in the shared tab limit; forgery closes only the refused child.
    #[test]
    fn sdtest_2539_browser_popup_parent_and_capacity_fences() {
        let (client, receiver) = test_client();
        let parent = client.create_tab("about:blank", 8, 8, 1.0).unwrap();
        receiver.recv().unwrap();
        let first = (1_u64 << 63) + 1;
        receive(
            &client.inner,
            Event::Created {
                browser_id: first,
                parent_id: Some(999),
            },
            vec![],
        )
        .unwrap();
        assert!(!client.owns(first));
        assert!(matches!(receiver.recv().unwrap(),Command::Close{browser_id} if browser_id==first));
        receive(
            &client.inner,
            Event::Created {
                browser_id: 12,
                parent_id: Some(parent),
            },
            vec![],
        )
        .unwrap();
        assert!(!client.owns(12));
        receiver.recv().unwrap();
        for offset in 0..MAX_TABS - 1 {
            receive(
                &client.inner,
                Event::Created {
                    browser_id: first + offset as u64,
                    parent_id: Some(parent),
                },
                vec![],
            )
            .unwrap();
        }
        assert!(client.owns(first));
        assert_eq!(client.viewport(first), Some((8, 8, 1.0)));
        assert_eq!(client.owned_ids().len(), MAX_TABS);
        let excess = first + MAX_TABS as u64;
        receive(
            &client.inner,
            Event::Created {
                browser_id: excess,
                parent_id: Some(parent),
            },
            vec![],
        )
        .unwrap();
        assert!(!client.owns(excess));
        assert!(
            matches!(receiver.recv().unwrap(),Command::Close{browser_id} if browser_id==excess)
        );
        assert!(client
            .send(Command::Create {
                browser_id: excess,
                url: "about:blank".into(),
                width: 8,
                height: 8,
                scale_factor: 1.0
            })
            .is_err());
    }
    // SDTEST-2540 — SDUC-557: dropped browser helpers receive a bounded graceful
    // profile-flush window before forced termination, entirely off the UI thread.
    #[test]
    fn sdtest_2540_browser_graceful_reaper_and_stuck_helper() {
        struct FakeProcess {
            polls: usize,
            exit_after: Option<usize>,
            actions: Vec<&'static str>,
        }
        impl ReapProcess for FakeProcess {
            fn exited(&mut self) -> io::Result<bool> {
                self.polls += 1;
                self.actions.push("poll");
                Ok(self.exit_after.is_some_and(|limit| self.polls >= limit))
            }
            fn terminate(&mut self) {
                self.actions.push("kill");
            }
            fn reap(&mut self) {
                self.actions.push("wait");
            }
        }
        let mut normal = FakeProcess {
            polls: 0,
            exit_after: Some(3),
            actions: vec![],
        };
        let mut pauses = vec![];
        reap_process(&mut normal, |duration| pauses.push(duration));
        assert_eq!(normal.actions, vec!["poll", "poll", "poll"]);
        assert_eq!(pauses, vec![Duration::from_millis(50); 2]);
        let mut stuck = FakeProcess {
            polls: 0,
            exit_after: None,
            actions: vec![],
        };
        let mut elapsed = Duration::ZERO;
        reap_process(&mut stuck, |duration| elapsed += duration);
        assert_eq!(elapsed, Duration::from_secs(3));
        assert_eq!(&stuck.actions[stuck.actions.len() - 2..], &["kill", "wait"]);
    }
    // SDTEST-2536 — SDUC-557: late/wrong-tab DevTools replies never satisfy another target's request and timeouts release slots.
    #[test]
    fn sdtest_2536_browser_devtools_correlated_target_and_timeout() {
        let (client, receiver) = test_client();
        let id = client.create_tab("about:blank", 8, 8, 1.0).unwrap();
        receiver.recv().unwrap();
        let waiting = client.clone();
        let task = std::thread::spawn(move || {
            waiting.devtools(
                id,
                "Page.captureScreenshot",
                serde_json::json!({}),
                Duration::from_secs(1),
            )
        });
        let Command::DevTools { request_id, .. } =
            receiver.recv_timeout(Duration::from_secs(1)).unwrap()
        else {
            panic!("expected devtools request")
        };
        receive(
            &client.inner,
            Event::DevToolsResult {
                browser_id: id + 1,
                request_id,
                result: serde_json::json!({"data":"wrong tab"}),
                error: None,
            },
            vec![],
        )
        .unwrap();
        assert!(task.join().unwrap().unwrap_err().contains("different tab"));
        assert!(client
            .devtools(
                id,
                "Accessibility.getFullAXTree",
                serde_json::json!({}),
                Duration::from_millis(1)
            )
            .unwrap_err()
            .contains("timed out"));
        assert!(client.inner.pending.lock().is_empty());
    }
}
