//! Sessions that never block their creator and survive the loss of the
//! daemon connection (docs/design/session-daemon.md, section 15, "Local app
//! integration").
//!
//! [`Supervisor::open`] returns a `TerminalSession` at once. A thread per
//! session then does the slow part: connect to the daemon (starting it
//! when absent), spawn the pane if it is a new one, attach, and apply the
//! pane's events to the session grid. When the connection is lost the same
//! thread shows a notice, waits with backoff, connects again (starting the
//! daemon again when it is gone) and reattaches. A pane that did not
//! survive (it died with its daemon, was closed elsewhere or expired) is
//! reported as ended instead of leaving a frozen screen.
//!
//! The state of each session is published through
//! `TerminalSession::attached_status` (phase and pane id), so a view can
//! show "connecting" on the tab and persist a layout keyed by pane id.

use crate::attached::{
    pump, terminate_pane, AttachError, LocalSessions, PumpEnd, SessionOptions, SessionsOptions,
};
use crate::client::{AttachOptions, Client, ClientError, SpawnOptions};
use crate::protocol::{PaneInfo, MAX_SCROLLBACK};
use parking_lot::{Condvar, Mutex};
use shelldeck_terminal::attached::{AttachedFeed, AttachedPhase};
use shelldeck_terminal::session::TerminalSession;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use uuid::Uuid;

/// Where the daemon is and how to start it.
#[derive(Debug, Clone)]
pub struct DaemonTarget {
    /// Run directory of the daemon.
    pub dir: PathBuf,
    /// Pass `--dir` to a daemon started here (tests, non default
    /// directories).
    pub explicit_dir: bool,
    /// Absolute path of the daemon binary, or why there is none. Only
    /// needed to start a daemon; a running one is used without it.
    pub daemon_exe: Result<PathBuf, String>,
}

/// Resolves the target on every connect, on the thread that connects
/// (never the caller of [`Supervisor::open`]): finding the run directory
/// may touch the file system and probe a socket.
pub type TargetResolver = Arc<dyn Fn() -> Result<DaemonTarget, String> + Send + Sync>;

/// Makes the daemon connection of a [`Supervisor`]. The local daemon uses
/// a [`TargetResolver`] ([`Supervisor::new`]); a daemon on another host
/// brings its own transport ([`Supervisor::with_connector`]), for example
/// an SSH exec channel, and hands over a connected client wrapped with
/// [`LocalSessions::from_client`].
///
/// Called on the thread that connects (a session thread or a background
/// thread), never on the caller of [`Supervisor::open`]. It may block.
pub trait Connector: Send + Sync {
    /// `start`: reach the daemon whatever it takes (start it, connect the
    /// transport again). Without it, only say what is known right now.
    ///
    /// `Ok(None)` means "no daemon runs, so no pane can have survived": a
    /// session whose pane existed is then reported as ended. A transport
    /// that cannot tell (the host is unreachable) must return an error
    /// instead, so the session keeps retrying.
    fn connect(
        &self,
        start: bool,
        options: &SessionsOptions,
    ) -> Result<Option<LocalSessions>, AttachError>;
}

/// The per-user daemon of this machine, found through a resolver.
struct LocalConnector {
    resolver: TargetResolver,
}

impl Connector for LocalConnector {
    fn connect(
        &self,
        start: bool,
        options: &SessionsOptions,
    ) -> Result<Option<LocalSessions>, AttachError> {
        let target = (self.resolver)().map_err(AttachError::Unavailable)?;
        let options = options.clone();
        match (start, target.daemon_exe) {
            (true, Ok(exe)) => {
                LocalSessions::connect(&target.dir, target.explicit_dir, &exe, options).map(Some)
            }
            // Nothing to start a daemon with: one that already runs is
            // still good.
            (true, Err(reason)) => match LocalSessions::connect_existing(&target.dir, options)? {
                Some(sessions) => Ok(Some(sessions)),
                None => Err(AttachError::Unavailable(reason)),
            },
            (false, _) => LocalSessions::connect_existing(&target.dir, options),
        }
    }
}

/// Called when the last session of a supervisor is over.
type IdleFn = Arc<dyn Fn() + Send + Sync>;

/// Lines shown inside a terminal by the supervisor. User facing, so the
/// caller provides them (translated). `None` shows nothing.
#[derive(Debug, Clone, Default)]
pub struct Notices {
    /// Shown at once in a new session, replaced by the pane's screen.
    pub connecting: Option<String>,
    /// The daemon connection was lost; a reattach follows.
    pub connection_lost: Option<String>,
    /// The pane no longer exists (lost with its daemon, closed elsewhere).
    pub ended: Option<String>,
    /// The pane could not be created or reached. The reason is appended.
    pub unavailable: Option<String>,
}

/// Delay before reconnect attempt number `attempt` (0 based): `start`
/// doubled each time, capped at `max`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Backoff {
    pub start: Duration,
    pub max: Duration,
}

impl Default for Backoff {
    fn default() -> Self {
        Self {
            start: Duration::from_millis(250),
            max: Duration::from_secs(5),
        }
    }
}

impl Backoff {
    pub fn delay(&self, attempt: u32) -> Duration {
        let factor = 1u32.checked_shl(attempt.min(16)).unwrap_or(u32::MAX);
        self.start.saturating_mul(factor).min(self.max)
    }
}

#[derive(Debug, Clone)]
pub struct SupervisorOptions {
    pub sessions: SessionsOptions,
    pub notices: Notices,
    pub backoff: Backoff,
    /// Consecutive failed attempts (no snapshot in between) after which a
    /// session gives up and reports `Unavailable`.
    pub max_attempts: u32,
}

impl Default for SupervisorOptions {
    fn default() -> Self {
        Self {
            sessions: SessionsOptions::default(),
            notices: Notices::default(),
            backoff: Backoff::default(),
            max_attempts: 12,
        }
    }
}

/// The pane a new session shows.
#[derive(Debug, Clone)]
pub enum PaneTarget {
    /// Spawn a new pane. Its size is the session's size at that moment.
    Spawn(SpawnOptions),
    /// Attach to a pane that already runs (app restart).
    Existing(Uuid),
}

// ---------------------------------------------------------------------------
// Reconnect state machine (pure, no I/O)
// ---------------------------------------------------------------------------

/// What happened to a session's link to its pane.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkEvent {
    /// The daemon could not be reached or started. `daemon_absent`: no
    /// daemon runs at all, so no pane can have survived.
    ConnectFailed { daemon_absent: bool },
    /// The daemon refused to spawn the pane.
    SpawnFailed,
    /// The pane now exists on the daemon (spawn succeeded).
    Spawned,
    /// The attachment ended because the connection was lost. `synced`: the
    /// pane's screen had been received on that connection.
    ConnectionLost { synced: bool },
    /// The pane's process exited (the pane may still be listed, dead).
    Exited(i32),
    /// The daemon does not know the pane (any more).
    PaneGone,
}

/// What the supervisor does next.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    /// Go on with the current attempt.
    Proceed,
    /// Wait, then connect and attach again.
    RetryAfter(Duration),
    /// Stop: the session stays in this phase.
    Finish(AttachedPhase),
}

/// Decides, for one session, whether to retry, how long to wait and how
/// the session ends. Kept free of I/O so the rules are testable.
#[derive(Debug, Clone)]
pub struct Reconnect {
    backoff: Backoff,
    max_attempts: u32,
    /// Failed attempts since the last time the pane's screen was received.
    attempt: u32,
    /// The pane exists on a daemon (spawned here or earlier).
    has_pane: bool,
    /// The pane's screen was shown at least once.
    shown: bool,
    exit_code: Option<i32>,
}

impl Reconnect {
    pub fn new(backoff: Backoff, max_attempts: u32, has_pane: bool) -> Self {
        Self {
            backoff,
            max_attempts,
            attempt: 0,
            has_pane,
            shown: false,
            exit_code: None,
        }
    }

    /// Phase to show while waiting for the next attempt.
    pub fn waiting_phase(&self) -> AttachedPhase {
        if self.shown {
            AttachedPhase::Reconnecting
        } else {
            AttachedPhase::Connecting
        }
    }

    fn retry(&mut self) -> Step {
        if self.attempt >= self.max_attempts {
            return Step::Finish(AttachedPhase::Unavailable);
        }
        let delay = self.backoff.delay(self.attempt);
        self.attempt += 1;
        Step::RetryAfter(delay)
    }

    pub fn on(&mut self, event: LinkEvent) -> Step {
        match event {
            // A pane that was never created has nothing to wait for: the
            // caller already retried the daemon start.
            LinkEvent::ConnectFailed { .. } | LinkEvent::SpawnFailed if !self.has_pane => {
                Step::Finish(AttachedPhase::Unavailable)
            }
            LinkEvent::SpawnFailed => Step::Finish(AttachedPhase::Unavailable),
            // The PTYs belong to the daemon: without one, the pane is gone
            // for good and a new daemon would not have it.
            LinkEvent::ConnectFailed {
                daemon_absent: true,
            } => Step::Finish(AttachedPhase::Ended {
                exit_code: self.exit_code,
            }),
            LinkEvent::ConnectFailed {
                daemon_absent: false,
            } => self.retry(),
            LinkEvent::Spawned => {
                self.has_pane = true;
                Step::Proceed
            }
            LinkEvent::ConnectionLost { synced } => {
                if synced {
                    self.shown = true;
                    self.attempt = 0;
                }
                self.retry()
            }
            LinkEvent::Exited(code) => {
                self.exit_code = Some(code);
                Step::Proceed
            }
            LinkEvent::PaneGone => Step::Finish(AttachedPhase::Ended {
                exit_code: self.exit_code,
            }),
        }
    }
}

// ---------------------------------------------------------------------------
// Per-session link: what the session side and the supervisor thread share
// ---------------------------------------------------------------------------

/// Input typed before the first attach is kept up to this size, then
/// dropped (a paste into a tab that is still connecting).
const MAX_PENDING_INPUT: usize = 64 * 1024;

#[derive(Default)]
struct LinkState {
    pane: Option<Uuid>,
    /// Connection the pane is attached through right now.
    bound: Option<Client>,
    /// The pane was attached at least once: from then on, input typed
    /// while disconnected is dropped instead of replayed later.
    ever_bound: bool,
    /// The `TerminalSession` was dropped.
    dropped: bool,
    /// The user closed the tab or pane and the pane is not ended yet.
    terminate: bool,
    /// Last size the view asked for.
    size: Option<(u16, u16)>,
    pending: Vec<Vec<u8>>,
    pending_bytes: usize,
}

#[derive(Default)]
struct Link {
    state: Mutex<LinkState>,
    wake: Condvar,
}

impl Link {
    fn input(&self, data: Vec<u8>) {
        let mut state = self.state.lock();
        let state = &mut *state;
        match (&state.bound, state.pane) {
            (Some(client), Some(pane)) => match client.input(pane, &data) {
                Ok(()) | Err(ClientError::Closed) => {}
                Err(error) => tracing::warn!(%pane, %error, "input dropped"),
            },
            _ if !state.ever_bound && state.pending_bytes + data.len() <= MAX_PENDING_INPUT => {
                state.pending_bytes += data.len();
                state.pending.push(data);
            }
            _ => {}
        }
    }

    /// Runs inside the supervisor runtime (the client arms a timer).
    fn resize(&self, size: (u16, u16)) {
        let mut state = self.state.lock();
        state.size = Some(size);
        if let (Some(client), Some(pane)) = (&state.bound, state.pane) {
            if let Err(error) = client.resize(pane, size.0, size.1) {
                tracing::debug!(%pane, %error, "resize request not sent");
            }
        }
    }

    /// The session was dropped: detach, the pane keeps running.
    fn session_dropped(&self) {
        let mut state = self.state.lock();
        state.dropped = true;
        if let (Some(client), Some(pane)) = (state.bound.take(), state.pane) {
            // Ends the event stream, which ends the supervisor thread.
            let _ = client.detach(pane);
        }
        drop(state);
        self.wake.notify_all();
    }

    fn is_dropped(&self) -> bool {
        self.state.lock().dropped
    }

    /// Sleep for `delay`. `true`: the session was dropped meanwhile.
    fn wait_dropped(&self, delay: Duration) -> bool {
        let deadline = std::time::Instant::now() + delay;
        let mut state = self.state.lock();
        while !state.dropped {
            if self.wake.wait_until(&mut state, deadline).timed_out() {
                break;
            }
        }
        state.dropped
    }
}

async fn forward(
    link: Arc<Link>,
    mut input_rx: mpsc::UnboundedReceiver<Vec<u8>>,
    mut resize_rx: mpsc::UnboundedReceiver<(u16, u16)>,
) {
    loop {
        tokio::select! {
            data = input_rx.recv() => match data {
                Some(data) => link.input(data),
                // The session is gone (its input sender was dropped).
                None => break,
            },
            Some(size) = resize_rx.recv() => link.resize(size),
        }
    }
    link.session_dropped();
}

// ---------------------------------------------------------------------------
// Supervisor
// ---------------------------------------------------------------------------

struct Inner {
    connector: Arc<dyn Connector>,
    options: SupervisorOptions,
    /// Sessions whose thread still runs (connecting, attached or waiting
    /// to reconnect).
    sessions: AtomicUsize,
    on_idle: Mutex<Option<IdleFn>>,
    /// The shared connection. Held while connecting, so several sessions
    /// opening at once make one connection and start one daemon. Never
    /// taken by [`Supervisor::open`].
    connection: Mutex<Option<LocalSessions>>,
    /// Panes shown by a live session of this supervisor.
    panes: Mutex<HashSet<Uuid>>,
    /// Input forwarders and pane terminations.
    runtime: tokio::runtime::Runtime,
}

/// Opens daemon-backed sessions without blocking and keeps them attached
/// across connection losses. Cheap to clone.
#[derive(Clone)]
pub struct Supervisor {
    inner: Arc<Inner>,
}

impl Supervisor {
    /// A supervisor for the per-user daemon of this machine.
    pub fn new(resolver: TargetResolver, options: SupervisorOptions) -> std::io::Result<Self> {
        Self::with_connector(Arc::new(LocalConnector { resolver }), options)
    }

    /// A supervisor whose daemon is reached through `connector` (a daemon
    /// on another host, behind its own transport).
    pub fn with_connector(
        connector: Arc<dyn Connector>,
        options: SupervisorOptions,
    ) -> std::io::Result<Self> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .thread_name("sessiond-input")
            .enable_all()
            .build()?;
        Ok(Self {
            inner: Arc::new(Inner {
                connector,
                options,
                sessions: AtomicUsize::new(0),
                on_idle: Mutex::new(None),
                connection: Mutex::new(None),
                panes: Mutex::new(HashSet::new()),
                runtime,
            }),
        })
    }

    /// A session for `target`, returned at once in the `Connecting` phase.
    /// Everything slow happens on a thread of its own: this never waits
    /// for the daemon, so it is safe on a UI thread.
    pub fn open(
        &self,
        target: PaneTarget,
        session: SessionOptions,
    ) -> Result<TerminalSession, AttachError> {
        let scrollback = session.scrollback.clamp(1, MAX_SCROLLBACK as usize);
        let (mut terminal, feed, input_rx) = TerminalSession::spawn_attached(
            session.title.clone(),
            session.rows,
            session.cols,
            scrollback,
            // Replaced by the daemon's revision on every attach.
            shelldeck_terminal::EMULATOR_REV,
        )
        .map_err(|error| AttachError::Terminal(error.to_string()))?;

        let link = Arc::new(Link::default());
        if let PaneTarget::Existing(pane) = &target {
            if !self.inner.panes.lock().insert(*pane) {
                return Err(AttachError::AlreadyAttached);
            }
            link.state.lock().pane = Some(*pane);
            feed.status().set_pane(*pane);
        }

        let (resize_tx, resize_rx) = mpsc::unbounded_channel::<(u16, u16)>();
        terminal.set_resize_fn(Box::new(move |rows, cols| {
            let _ = resize_tx.send((rows, cols));
        }));
        let supervisor = Arc::downgrade(&self.inner);
        let terminate_link = link.clone();
        terminal.set_terminate_fn(Box::new(move || {
            let mut state = terminate_link.state.lock();
            match (&state.bound, state.pane, supervisor.upgrade()) {
                (Some(client), Some(pane), Some(inner)) => {
                    inner.runtime.spawn(terminate_pane(client.clone(), pane));
                }
                // Not attached right now: the supervisor thread ends the
                // pane as soon as it can reach it.
                _ => state.terminate = true,
            }
            drop(state);
            terminate_link.wake.notify_all();
        }));

        self.inner
            .runtime
            .spawn(forward(link.clone(), input_rx, resize_rx));

        let driver = Driver {
            supervisor: self.clone(),
            link: link.clone(),
            session: SessionOptions {
                scrollback,
                ..session
            },
        };
        self.inner.sessions.fetch_add(1, Ordering::SeqCst);
        let spawned = std::thread::Builder::new()
            .name("sessiond-pane".into())
            .spawn(move || driver.run(target, feed));
        if let Err(error) = spawned {
            self.inner.sessions.fetch_sub(1, Ordering::SeqCst);
            if let Some(pane) = link.state.lock().pane {
                self.inner.panes.lock().remove(&pane);
            }
            return Err(AttachError::Io(error));
        }
        Ok(terminal)
    }

    /// Panes of the daemon, or `None` when no daemon runs and `start` is
    /// false. Blocks for the connect and the round trip: call it from a
    /// background thread.
    pub fn list_panes(&self, start: bool) -> Result<Option<Vec<PaneInfo>>, AttachError> {
        match self.connection(start)? {
            Some(sessions) => sessions.list_panes().map(Some),
            None => Ok(None),
        }
    }

    /// Panes shown by a live session of this supervisor.
    pub fn open_panes(&self) -> Vec<Uuid> {
        self.inner.panes.lock().iter().copied().collect()
    }

    /// Sessions whose thread still runs: connecting, attached or waiting
    /// to reconnect. A session that ended or gave up no longer counts,
    /// even while its tab is still shown.
    pub fn session_count(&self) -> usize {
        self.inner.sessions.load(Ordering::SeqCst)
    }

    /// `callback` runs, on a session thread, each time the last session is
    /// over. A transport that costs something to keep (an SSH connection)
    /// uses it to let go; it must check [`session_count`](Self::session_count)
    /// again, a new session may have been opened meanwhile.
    pub fn set_on_idle(&self, callback: impl Fn() + Send + Sync + 'static) {
        *self.inner.on_idle.lock() = Some(Arc::new(callback));
    }

    /// Give the current connection up, as if it had been lost (tests, and
    /// a way to force a reconnect). Panes keep running.
    pub fn drop_connection(&self) {
        if let Some(sessions) = self.inner.connection.lock().as_ref() {
            sessions.client().close();
        }
    }

    /// Close the shared connection and forget it. Panes keep running; a
    /// session still attached reconnects by itself.
    pub fn close_connection(&self) {
        if let Some(sessions) = self.inner.connection.lock().take() {
            sessions.client().close();
        }
    }

    /// The shared connection, opened when there is none. `start`: launch
    /// the daemon when it is not running. Blocking.
    fn connection(&self, start: bool) -> Result<Option<LocalSessions>, AttachError> {
        let mut slot = self.inner.connection.lock();
        if let Some(sessions) = slot.as_ref().filter(|sessions| !sessions.is_closed()) {
            return Ok(Some(sessions.clone()));
        }
        *slot = None;
        let sessions = self
            .inner
            .connector
            .connect(start, &self.inner.options.sessions)?;
        slot.clone_from(&sessions);
        Ok(sessions)
    }
}

/// The thread behind one session.
struct Driver {
    supervisor: Supervisor,
    link: Arc<Link>,
    session: SessionOptions,
}

impl Driver {
    fn run(self, target: PaneTarget, mut feed: AttachedFeed) {
        let options = &self.supervisor.inner.options;
        let mut spawn = match target {
            PaneTarget::Spawn(spawn) => Some(spawn),
            PaneTarget::Existing(_) => None,
        };
        let mut machine = Reconnect::new(options.backoff, options.max_attempts, spawn.is_none());
        let mut reason = String::new();
        if let Some(notice) = &options.notices.connecting {
            feed.local_notice(notice);
        }
        loop {
            if self.link.is_dropped() {
                break;
            }
            match self.attempt(&mut spawn, &mut feed, &mut machine, &mut reason) {
                Step::Proceed => {}
                Step::RetryAfter(delay) => {
                    feed.status().set_phase(machine.waiting_phase());
                    feed.wake();
                    if self.link.wait_dropped(delay) {
                        break;
                    }
                }
                Step::Finish(phase) => {
                    match &phase {
                        AttachedPhase::Ended { exit_code: None } => {
                            if let Some(notice) = &options.notices.ended {
                                feed.local_notice(notice);
                            }
                        }
                        AttachedPhase::Unavailable => {
                            tracing::warn!("session daemon pane unavailable: {reason}");
                            if let Some(notice) = &options.notices.unavailable {
                                feed.local_notice(&format!("{notice} ({reason})"));
                            }
                        }
                        // A process that exited keeps its final screen,
                        // like an in-process terminal.
                        _ => {}
                    }
                    feed.status().set_phase(phase);
                    feed.wake();
                    break;
                }
            }
        }
        self.finish();
        let inner = &self.supervisor.inner;
        if inner.sessions.fetch_sub(1, Ordering::SeqCst) == 1 {
            let on_idle = inner.on_idle.lock().clone();
            if let Some(on_idle) = on_idle {
                on_idle();
            }
        }
    }

    /// One connect, spawn if needed, attach, and the whole life of that
    /// attachment. Returns what to do once it is over.
    fn attempt(
        &self,
        spawn: &mut Option<SpawnOptions>,
        feed: &mut AttachedFeed,
        machine: &mut Reconnect,
        reason: &mut String,
    ) -> Step {
        let sessions = match self.supervisor.connection(true) {
            Ok(Some(sessions)) => sessions,
            Ok(None) => {
                *reason = "the session daemon is not running".into();
                return machine.on(LinkEvent::ConnectFailed {
                    daemon_absent: true,
                });
            }
            Err(error) => {
                *reason = error.to_string();
                // Could not start one: is one running at all?
                let daemon_absent = matches!(self.supervisor.connection(false), Ok(None));
                return machine.on(LinkEvent::ConnectFailed { daemon_absent });
            }
        };

        let known = self.link.state.lock().pane;
        let pane = match known {
            Some(pane) => pane,
            None => {
                let Some(mut request) = spawn.clone() else {
                    *reason = "no pane to attach".into();
                    return Step::Finish(AttachedPhase::Unavailable);
                };
                let (rows, cols) = self.wanted_size();
                request.rows = rows;
                request.cols = cols;
                match sessions.spawn_pane(request) {
                    Ok(spawned) => {
                        tracing::info!(pane = %spawned.pane, pid = spawned.pid, "spawned a session daemon pane");
                        *spawn = None;
                        self.supervisor.inner.panes.lock().insert(spawned.pane);
                        self.link.state.lock().pane = Some(spawned.pane);
                        feed.status().set_pane(spawned.pane);
                        machine.on(LinkEvent::Spawned);
                        spawned.pane
                    }
                    Err(AttachError::Client(ClientError::Closed)) | Err(AttachError::Timeout) => {
                        *reason = "the session daemon connection was lost".into();
                        return machine.on(LinkEvent::ConnectionLost { synced: false });
                    }
                    Err(error) => {
                        *reason = error.to_string();
                        return machine.on(LinkEvent::SpawnFailed);
                    }
                }
            }
        };
        // Closed while it was connecting: nothing to show, `finish` ends
        // the pane if the user asked for that.
        if self.link.is_dropped() {
            return Step::Proceed;
        }

        let requested = self.link.state.lock().size;
        // A pane offered back before the view was laid out keeps its size
        // until the view asks for one.
        let keep_size = self.session.keep_pane_size && requested.is_none() && spawn.is_none();
        let (rows, cols) = requested.unwrap_or((self.session.rows, self.session.cols));
        feed.set_daemon_emulator_rev(sessions.daemon_emulator_rev());
        let events = match sessions.attach_events(
            pane,
            AttachOptions {
                rows: if keep_size { 0 } else { rows },
                cols: if keep_size { 0 } else { cols },
                scrollback_rows: self.session.scrollback as u32,
                read_only: false,
            },
        ) {
            Ok(events) => events,
            Err(AttachError::AlreadyAttached) => {
                *reason = "the pane is already shown in this app".into();
                return Step::Finish(AttachedPhase::Unavailable);
            }
            Err(error) => {
                *reason = error.to_string();
                return machine.on(LinkEvent::ConnectionLost { synced: false });
            }
        };
        self.bind(&sessions, pane, (!keep_size).then_some((rows, cols)));

        let pumped = pump(sessions.handle(), sessions.client(), events, feed);

        self.link.state.lock().bound = None;
        sessions.forget(pane);
        if let Some(code) = pumped.exited {
            machine.on(LinkEvent::Exited(code));
        }
        match pumped.end {
            PumpEnd::PaneGone => {
                // Drop the client's bookkeeping for the pane (the daemon
                // answers `NotAttached`, which is ignored).
                let _ = sessions.client().detach(pane);
                machine.on(LinkEvent::PaneGone)
            }
            // Detached without the session being dropped only happens when
            // the connection is being torn down: same as a loss.
            PumpEnd::ConnectionLost | PumpEnd::Detached => {
                if self.link.is_dropped() {
                    return Step::Proceed;
                }
                if let Some(notice) = &self.supervisor.inner.options.notices.connection_lost {
                    feed.local_notice(notice);
                }
                *reason = "the session daemon connection was lost".into();
                machine.on(LinkEvent::ConnectionLost {
                    synced: pumped.synced,
                })
            }
        }
    }

    fn wanted_size(&self) -> (u16, u16) {
        self.link
            .state
            .lock()
            .size
            .unwrap_or((self.session.rows, self.session.cols))
    }

    /// The pane is attached through `sessions`: input and resize requests
    /// now reach it. `attached_size`: the size the attach asked for.
    fn bind(&self, sessions: &LocalSessions, pane: Uuid, attached_size: Option<(u16, u16)>) {
        // `Client::resize` may arm a timer.
        let _runtime = sessions.handle().enter();
        let client = sessions.client();
        let mut state = self.link.state.lock();
        if state.dropped {
            // Dropped between the attach and here: nobody else detaches.
            let _ = client.detach(pane);
            return;
        }
        if state.terminate {
            state.terminate = false;
            self.supervisor
                .inner
                .runtime
                .spawn(terminate_pane(client.clone(), pane));
        }
        for data in std::mem::take(&mut state.pending) {
            let _ = client.input(pane, &data);
        }
        state.pending_bytes = 0;
        if let Some((rows, cols)) = state.size.filter(|size| Some(*size) != attached_size) {
            let _ = client.resize(pane, rows, cols);
        }
        state.ever_bound = true;
        state.bound = Some(client.clone());
    }

    /// The session is over for this thread: release the pane, and end it
    /// if the user closed the tab while it could not be reached.
    fn finish(&self) {
        let (pane, terminate) = {
            let mut state = self.link.state.lock();
            state.bound = None;
            (state.pane, std::mem::take(&mut state.terminate))
        };
        let Some(pane) = pane else {
            return;
        };
        self.supervisor.inner.panes.lock().remove(&pane);
        if terminate {
            // Never start a daemon for this: without one the pane is gone.
            if let Ok(Some(sessions)) = self.supervisor.connection(false) {
                sessions.terminate(pane);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn backoff() -> Backoff {
        Backoff {
            start: Duration::from_millis(100),
            max: Duration::from_millis(1_000),
        }
    }

    // SDTEST-3004 (SDUC-574): reconnect delays double from the start value
    // and stop growing at the cap, also for absurd attempt numbers.
    #[test]
    fn sdtest_3004_backoff_doubles_up_to_the_cap() {
        let backoff = backoff();
        let delays: Vec<u64> = (0..6)
            .map(|attempt| backoff.delay(attempt).as_millis() as u64)
            .collect();
        assert_eq!(delays, vec![100, 200, 400, 800, 1_000, 1_000]);
        assert_eq!(backoff.delay(u32::MAX), Duration::from_millis(1_000));
        let huge = Backoff {
            start: Duration::from_secs(u64::MAX / 2),
            max: Duration::MAX,
        };
        assert!(huge.delay(40) >= huge.start, "no overflow panic");
    }

    // SDTEST-3005 (SDUC-574): the reconnect rules of one session.
    #[test]
    fn sdtest_3005_reconnect_state_machine() {
        let ended = |exit_code| Step::Finish(AttachedPhase::Ended { exit_code });
        let unavailable = Step::Finish(AttachedPhase::Unavailable);

        // A new pane that cannot be created is unavailable at once: there
        // is nothing to wait for.
        for event in [
            LinkEvent::ConnectFailed {
                daemon_absent: true,
            },
            LinkEvent::ConnectFailed {
                daemon_absent: false,
            },
            LinkEvent::SpawnFailed,
        ] {
            let mut machine = Reconnect::new(backoff(), 3, false);
            assert_eq!(machine.waiting_phase(), AttachedPhase::Connecting);
            assert_eq!(machine.on(event), unavailable);
        }

        // Spawned, shown, then the connection drops: reconnect with a
        // growing delay; each successful reattach starts the delays over.
        let mut machine = Reconnect::new(backoff(), 3, false);
        assert_eq!(machine.on(LinkEvent::Spawned), Step::Proceed);
        assert_eq!(
            machine.on(LinkEvent::ConnectionLost { synced: true }),
            Step::RetryAfter(Duration::from_millis(100))
        );
        assert_eq!(machine.waiting_phase(), AttachedPhase::Reconnecting);
        assert_eq!(
            machine.on(LinkEvent::ConnectFailed {
                daemon_absent: false
            }),
            Step::RetryAfter(Duration::from_millis(200))
        );
        assert_eq!(
            machine.on(LinkEvent::ConnectionLost { synced: false }),
            Step::RetryAfter(Duration::from_millis(400))
        );
        assert_eq!(
            machine.on(LinkEvent::ConnectionLost { synced: true }),
            Step::RetryAfter(Duration::from_millis(100)),
            "a reattach that showed the pane resets the backoff"
        );

        // Attempts that never show the pane are bounded.
        let mut machine = Reconnect::new(backoff(), 3, true);
        for _ in 0..3 {
            assert!(matches!(
                machine.on(LinkEvent::ConnectFailed {
                    daemon_absent: false
                }),
                Step::RetryAfter(_)
            ));
        }
        assert_eq!(
            machine.on(LinkEvent::ConnectFailed {
                daemon_absent: false
            }),
            unavailable
        );
        // Not shown yet: still "connecting" while it waits.
        assert_eq!(
            Reconnect::new(backoff(), 3, true).waiting_phase(),
            AttachedPhase::Connecting
        );

        // The daemon is gone and none can be started: the pane died with
        // it. Reported as ended, never retried.
        let mut machine = Reconnect::new(backoff(), 3, true);
        machine.on(LinkEvent::ConnectionLost { synced: true });
        assert_eq!(
            machine.on(LinkEvent::ConnectFailed {
                daemon_absent: true
            }),
            ended(None)
        );

        // A new daemon does not know the pane: ended.
        let mut machine = Reconnect::new(backoff(), 3, true);
        machine.on(LinkEvent::ConnectionLost { synced: true });
        assert_eq!(machine.on(LinkEvent::PaneGone), ended(None));

        // A pane whose process exited keeps its exit code through a
        // later removal or connection loss.
        let mut machine = Reconnect::new(backoff(), 3, true);
        assert_eq!(machine.on(LinkEvent::Exited(3)), Step::Proceed);
        assert!(matches!(
            machine.on(LinkEvent::ConnectionLost { synced: true }),
            Step::RetryAfter(_)
        ));
        assert_eq!(machine.on(LinkEvent::PaneGone), ended(Some(3)));

        // A daemon that refuses the spawn is final even for the machine
        // of an existing pane (defensive: it never spawns).
        let mut machine = Reconnect::new(backoff(), 3, true);
        assert_eq!(machine.on(LinkEvent::SpawnFailed), unavailable);
    }
}
