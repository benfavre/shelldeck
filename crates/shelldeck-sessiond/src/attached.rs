//! Local sessions for the app: a synchronous facade over the async client
//! (docs/design/session-daemon.md, sections 9 and 10).
//!
//! [`LocalSessions`] owns one connection to the per-user daemon and a small
//! private Tokio runtime, so callers (the GPUI thread, tests) need no async
//! context. Each pane becomes a `TerminalSession` created with
//! `TerminalSession::spawn_attached`:
//!
//! - a reader thread per pane applies the pane's events to the session
//!   grid (snapshot, raw bytes, applied resizes, history backfill);
//! - a task forwards keyboard input and resize requests to the daemon;
//! - dropping the session detaches, the pane keeps running;
//!   `TerminalSession::terminate` ends the pane.

use crate::client::{
    AttachOptions, Client, ClientError, ClientOptions, PaneEvent, PaneEvents, SpawnOptions,
};
use crate::protocol::{ErrorCode, PaneInfo, MAX_SCROLLBACK, SIGNAL_HUP, SIGNAL_KILL};
use crate::proxy::{self, ProxyFailure};
use parking_lot::Mutex;
use shelldeck_terminal::attached::{AttachedFeed, AttachedOutcome};
use shelldeck_terminal::session::TerminalSession;
use std::collections::HashSet;
use std::future::Future;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
pub enum AttachError {
    #[error("the session daemon could not be reached or started: {0:?}")]
    Daemon(ProxyFailure),
    #[error(transparent)]
    Client(#[from] ClientError),
    #[error("the session daemon did not answer in time")]
    Timeout,
    #[error("pane is already attached in this process")]
    AlreadyAttached,
    #[error("terminal session: {0}")]
    Terminal(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

#[derive(Debug, Clone)]
pub struct SessionsOptions {
    pub client: ClientOptions,
    /// How long a request (handshake, spawn, list) may take.
    pub request_timeout: Duration,
    /// Line shown in a terminal whose daemon connection is gone. The text
    /// is user facing, so the caller provides it (translated).
    pub connection_lost_notice: Option<String>,
}

impl Default for SessionsOptions {
    fn default() -> Self {
        Self {
            client: ClientOptions::default(),
            request_timeout: Duration::from_secs(10),
            connection_lost_notice: None,
        }
    }
}

/// How a session is shown and how much history it keeps.
#[derive(Debug, Clone)]
pub struct SessionOptions {
    pub title: String,
    pub rows: u16,
    pub cols: u16,
    /// History rows kept by the client grid and backfilled on attach.
    pub scrollback: usize,
    /// Attach without asking for a size: the pane keeps the size it has
    /// and `rows` and `cols` only size the local grid until the snapshot
    /// arrives. For panes offered back before the view has been laid out
    /// (app start): the first real `resize` of the session takes the size.
    pub keep_pane_size: bool,
}

/// A freshly spawned pane with its attached session.
pub struct SpawnedSession {
    pub session: TerminalSession,
    pub pane: Uuid,
    pub pid: u32,
    /// Canonical working directory the daemon opened.
    pub cwd: String,
}

struct Inner {
    client: Client,
    options: SessionsOptions,
    /// Panes attached through this connection. `Client::attach` on a pane
    /// that is already attached would replace its event stream.
    attached: Arc<Mutex<HashSet<Uuid>>>,
    // Dropped last: `Drop` detaches every pane first so the reader threads,
    // which wait on this runtime, end.
    runtime: tokio::runtime::Runtime,
}

impl Drop for Inner {
    fn drop(&mut self) {
        let panes: Vec<Uuid> = self.attached.lock().drain().collect();
        for pane in panes {
            let _ = self.client.detach(pane);
        }
    }
}

/// One connection to the local session daemon.
#[derive(Clone)]
pub struct LocalSessions {
    inner: Arc<Inner>,
}

impl LocalSessions {
    /// Connect to the daemon serving `dir`, starting `daemon_exe` (absolute
    /// path, never a `PATH` lookup) when none is running. `explicit_dir`
    /// passes `--dir` to the started daemon (tests and non default
    /// directories).
    pub fn connect(
        dir: &Path,
        explicit_dir: bool,
        daemon_exe: &Path,
        options: SessionsOptions,
    ) -> Result<Self, AttachError> {
        let runtime = new_runtime()?;
        // The connection ending before `HelloAck` means the daemon was on
        // its way out (idle exit): treat it as not running and start it
        // once more.
        let mut last = None;
        for _ in 0..2 {
            let stream = proxy::connect_or_start(dir, explicit_dir, daemon_exe)
                .map_err(AttachError::Daemon)?;
            match handshake(&runtime, stream, &options) {
                Ok(client) => return Ok(Self::from_parts(runtime, client, options)),
                Err(error) if daemon_went_away(&error) => last = Some(error),
                Err(error) => return Err(error),
            }
        }
        Err(last.unwrap_or(AttachError::Client(ClientError::Closed)))
    }

    /// Connect only if a daemon is already running on `dir`; never starts
    /// one. `Ok(None)`: no daemon, so no pane to offer back.
    pub fn connect_existing(
        dir: &Path,
        options: SessionsOptions,
    ) -> Result<Option<Self>, AttachError> {
        let Some(stream) = proxy::connect_existing(dir).map_err(AttachError::Daemon)? else {
            return Ok(None);
        };
        let runtime = new_runtime()?;
        match handshake(&runtime, stream, &options) {
            Ok(client) => Ok(Some(Self::from_parts(runtime, client, options))),
            Err(error) if daemon_went_away(&error) => Ok(None),
            Err(error) => Err(error),
        }
    }

    fn from_parts(
        runtime: tokio::runtime::Runtime,
        client: Client,
        options: SessionsOptions,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                client,
                options,
                attached: Arc::new(Mutex::new(HashSet::new())),
                runtime,
            }),
        }
    }

    /// Whether the connection to the daemon is gone. A closed connection is
    /// not reopened: the caller connects again.
    pub fn is_closed(&self) -> bool {
        self.inner.client.is_closed()
    }

    /// Emulator revision announced by the daemon.
    pub fn daemon_emulator_rev(&self) -> u16 {
        self.inner.client.hello().emulator_rev
    }

    pub fn list_panes(&self) -> Result<Vec<PaneInfo>, AttachError> {
        let client = self.inner.client.clone();
        wait(
            &self.inner.runtime,
            self.inner.options.request_timeout,
            async move { client.list_panes().await },
        )?
        .map_err(AttachError::from)
    }

    /// Panes currently attached through this connection.
    pub fn attached_panes(&self) -> Vec<Uuid> {
        self.inner.attached.lock().iter().copied().collect()
    }

    /// Spawn a pane and attach a new session to it. `spawn.rows` and
    /// `spawn.cols` are taken from `session`.
    pub fn spawn(
        &self,
        mut spawn: SpawnOptions,
        session: SessionOptions,
    ) -> Result<SpawnedSession, AttachError> {
        spawn.rows = session.rows;
        spawn.cols = session.cols;
        let session = SessionOptions {
            // The pane was just created at this size.
            keep_pane_size: false,
            ..session
        };
        let client = self.inner.client.clone();
        let spawned = wait(
            &self.inner.runtime,
            self.inner.options.request_timeout,
            async move { client.spawn(spawn).await },
        )??;
        match self.attach(spawned.pane, session) {
            Ok(mut session) => {
                session.set_initial_cwd(PathBuf::from(&spawned.cwd));
                Ok(SpawnedSession {
                    session,
                    pane: spawned.pane,
                    pid: spawned.pid,
                    cwd: spawned.cwd,
                })
            }
            Err(error) => {
                // Nothing shows this pane: do not leave it running unseen.
                self.terminate(spawned.pane);
                Err(error)
            }
        }
    }

    /// Attach a new session to an existing pane. The session shows the
    /// daemon's screen as soon as the snapshot arrives, then live output.
    pub fn attach(
        &self,
        pane: Uuid,
        session: SessionOptions,
    ) -> Result<TerminalSession, AttachError> {
        let inner = &self.inner;
        if inner.client.is_closed() {
            return Err(AttachError::Client(ClientError::Closed));
        }
        if !inner.attached.lock().insert(pane) {
            return Err(AttachError::AlreadyAttached);
        }
        let scrollback = session.scrollback.clamp(1, MAX_SCROLLBACK as usize);
        let built = TerminalSession::spawn_attached(
            session.title,
            session.rows,
            session.cols,
            scrollback,
            self.daemon_emulator_rev(),
        )
        .map_err(|error| AttachError::Terminal(error.to_string()));
        let (mut terminal, feed, input_rx) = match built {
            Ok(parts) => parts,
            Err(error) => {
                inner.attached.lock().remove(&pane);
                return Err(error);
            }
        };

        let events = {
            // The client spawns its timers on the current runtime.
            let _runtime = inner.runtime.enter();
            inner.client.attach(
                pane,
                AttachOptions {
                    rows: if session.keep_pane_size {
                        0
                    } else {
                        session.rows
                    },
                    cols: if session.keep_pane_size {
                        0
                    } else {
                        session.cols
                    },
                    scrollback_rows: scrollback as u32,
                    read_only: false,
                },
            )
        };
        let events = match events {
            Ok(events) => events,
            Err(error) => {
                inner.attached.lock().remove(&pane);
                return Err(error.into());
            }
        };

        let (resize_tx, resize_rx) = mpsc::unbounded_channel::<(u16, u16)>();
        terminal.set_resize_fn(Box::new(move |rows, cols| {
            let _ = resize_tx.send((rows, cols));
        }));
        let sessions = Arc::downgrade(inner);
        terminal.set_terminate_fn(Box::new(move || {
            if let Some(inner) = sessions.upgrade() {
                LocalSessions { inner }.terminate(pane);
            }
        }));

        inner.runtime.spawn(forward_input(
            inner.attached.clone(),
            inner.client.clone(),
            pane,
            input_rx,
            resize_rx,
        ));

        let reader = PaneReader {
            handle: inner.runtime.handle().clone(),
            client: inner.client.clone(),
            lost_notice: inner.options.connection_lost_notice.clone(),
        };
        let spawned = std::thread::Builder::new()
            .name("sessiond-reader".into())
            .spawn(move || reader.run(events, feed));
        if let Err(error) = spawned {
            // Dropping `terminal` closes its input channel, which detaches.
            return Err(AttachError::Io(error));
        }
        Ok(terminal)
    }

    /// End a pane: hang up its process group, kill it if it does not exit,
    /// then remove the dead pane from the daemon. Returns at once; the
    /// work happens in the background.
    pub fn terminate(&self, pane: Uuid) {
        let client = self.inner.client.clone();
        self.inner.runtime.spawn(terminate_pane(client, pane));
    }
}

/// Whether a handshake failed because the daemon exited under it. A
/// connection that was still in the listener's backlog when the daemon
/// stopped is never accepted: the kernel resets it, so the handshake ends
/// with a reset or a broken pipe instead of a clean EOF.
fn daemon_went_away(error: &AttachError) -> bool {
    match error {
        AttachError::Client(ClientError::Closed) => true,
        AttachError::Client(ClientError::Io(error)) | AttachError::Io(error) => matches!(
            error.kind(),
            std::io::ErrorKind::ConnectionReset
                | std::io::ErrorKind::BrokenPipe
                | std::io::ErrorKind::ConnectionAborted
                | std::io::ErrorKind::UnexpectedEof
                | std::io::ErrorKind::NotConnected
        ),
        _ => false,
    }
}

fn new_runtime() -> std::io::Result<tokio::runtime::Runtime> {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .thread_name("sessiond-client")
        .enable_all()
        .build()
}

/// Run a future on the private runtime and wait for it from any thread,
/// including one that is itself inside another runtime.
fn wait<T, F>(
    runtime: &tokio::runtime::Runtime,
    timeout: Duration,
    future: F,
) -> Result<T, AttachError>
where
    T: Send + 'static,
    F: Future<Output = T> + Send + 'static,
{
    let (tx, rx) = std::sync::mpsc::channel();
    runtime.spawn(async move {
        let _ = tx.send(future.await);
    });
    rx.recv_timeout(timeout).map_err(|_| AttachError::Timeout)
}

fn handshake(
    runtime: &tokio::runtime::Runtime,
    stream: UnixStream,
    options: &SessionsOptions,
) -> Result<Client, AttachError> {
    stream.set_nonblocking(true)?;
    let client_options = options.client.clone();
    wait(runtime, options.request_timeout, async move {
        let stream = tokio::net::UnixStream::from_std(stream)?;
        Client::connect(stream, client_options).await
    })?
    .map_err(AttachError::from)
}

/// Keyboard input and resize requests of one session. Ends when the
/// session is dropped (its input channel closes), which detaches.
async fn forward_input(
    attached: Arc<Mutex<HashSet<Uuid>>>,
    client: Client,
    pane: Uuid,
    mut input_rx: mpsc::UnboundedReceiver<Vec<u8>>,
    mut resize_rx: mpsc::UnboundedReceiver<(u16, u16)>,
) {
    loop {
        tokio::select! {
            data = input_rx.recv() => match data {
                Some(data) => match client.input(pane, &data) {
                    Ok(()) => {}
                    Err(ClientError::Closed) => break,
                    Err(error) => tracing::warn!(%pane, %error, "input dropped"),
                },
                None => break,
            },
            Some((rows, cols)) = resize_rx.recv() => {
                if let Err(error) = client.resize(pane, rows, cols) {
                    tracing::debug!(%pane, %error, "resize request not sent");
                }
            }
        }
    }
    // `NotAttached` here only means the connection owner detached first.
    let _ = client.detach(pane);
    attached.lock().remove(&pane);
}

async fn terminate_pane(client: Client, pane: Uuid) {
    const POLL: Duration = Duration::from_millis(50);
    const KILL_AFTER: u32 = 20;
    const GIVE_UP_AFTER: u32 = 60;
    if client.kill(pane, SIGNAL_HUP).is_err() {
        return;
    }
    for step in 1..=GIVE_UP_AFTER {
        tokio::time::sleep(POLL).await;
        let Ok(panes) = client.list_panes().await else {
            return;
        };
        match panes.iter().find(|info| info.pane == pane) {
            None => return,
            Some(info) if !info.alive => {
                // On a dead pane `Kill` means "remove".
                let _ = client.kill(pane, SIGNAL_HUP);
                return;
            }
            Some(_) if step == KILL_AFTER => {
                let _ = client.kill(pane, SIGNAL_KILL);
            }
            Some(_) => {}
        }
    }
    tracing::warn!(%pane, "pane did not exit after SIGKILL");
}

struct PaneReader {
    handle: tokio::runtime::Handle,
    client: Client,
    lost_notice: Option<String>,
}

impl PaneReader {
    /// Apply the pane's events in order. Parsing happens here, off the
    /// runtime, so a busy pane never delays the connection reader; the
    /// client's per-pane feed cap bounds what can queue up meanwhile.
    fn run(self, mut events: PaneEvents, mut feed: AttachedFeed) {
        let pane = events.pane();
        loop {
            let event = match events.try_next() {
                Some(event) => event,
                None => match self.handle.block_on(events.next()) {
                    Some(event) => event,
                    None => break,
                },
            };
            match event {
                PaneEvent::Restore {
                    snapshot,
                    keep_history,
                    ..
                } => {
                    if let Err(error) = feed.restore(&snapshot, keep_history) {
                        tracing::error!(%pane, %error, "snapshot rejected");
                        feed.mark_stale();
                    }
                }
                PaneEvent::Bytes { data, .. } => feed.bytes(&data),
                PaneEvent::Resize {
                    rows, cols, pulled, ..
                } => {
                    if feed.resize(rows, cols, pulled) == AttachedOutcome::NeedsReattach {
                        let _runtime = self.handle.enter();
                        let _ = self.client.reattach(pane, true);
                    }
                }
                PaneEvent::History { rows, count, .. } => {
                    if count > 0 {
                        if let Err(error) = feed.history(&rows) {
                            tracing::warn!(%pane, %error, "history chunk rejected");
                        }
                    }
                }
                PaneEvent::Exited { exit_code, .. } => {
                    // The dead pane is not removed here: another client
                    // attached to it may not have read its final output
                    // yet. It goes away when the user closes the tab
                    // (`terminate`) or when the daemon expires it, and
                    // dead panes are never offered back.
                    tracing::debug!(%pane, exit_code, "pane exited");
                    feed.wake();
                }
                PaneEvent::Resyncing => feed.mark_stale(),
                PaneEvent::Error(ErrorCode::NoSuchPane) => {
                    // Closed by another client, or expired: the last
                    // screen stays, nothing more will come.
                    tracing::debug!(%pane, "pane is gone");
                    break;
                }
                PaneEvent::Error(code) => {
                    tracing::warn!(%pane, ?code, "session daemon reported an error");
                }
                PaneEvent::Closed => {
                    if let Some(notice) = &self.lost_notice {
                        feed.local_notice(notice);
                    }
                    break;
                }
            }
        }
        feed.wake();
    }
}
