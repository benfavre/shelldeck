//! The session daemon: listener, pane registry and lifecycle
//! (docs/design/session-daemon.md, section 3).

mod conn;
pub mod launch;
pub mod log;
mod pane;

use crate::paths::{self, PathError};
use crate::protocol::{DaemonFrame, ErrorCode, PaneInfo, MAX_CONNECTIONS, MAX_PANES};
pub use launch::PaneLauncher;
use pane::Pane;
use parking_lot::Mutex;
use std::collections::HashMap;
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::os::unix::io::AsRawFd;
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use uuid::Uuid;

/// Timeouts and caps of the daemon. Tests shorten them; production uses
/// [`ServerConfig::default`].
#[derive(Debug, Clone)]
pub struct ServerConfig {
    /// The first frame must be `Hello` within this time.
    pub hello_timeout: Duration,
    /// A started frame must complete within this time.
    pub frame_timeout: Duration,
    /// A socket that accepts no byte for this long is closed.
    pub write_timeout: Duration,
    /// Exit after this long with zero panes and zero clients. `None`
    /// disables the idle exit.
    pub idle_exit: Option<Duration>,
    /// How long a dead pane stays listed before it expires.
    pub dead_pane_ttl: Duration,
    /// Queued `Output` bytes per attachment before the client is dropped
    /// with `Error(Resync)`.
    pub output_queue_cap: usize,
    /// Queued snapshot and scrollback bytes per connection.
    pub bulk_queue_cap: usize,
    /// Queued input bytes per pane.
    pub input_queue_cap: usize,
    /// PTY input stall after which a pane's queued input is discarded.
    pub input_stall: Duration,
    /// On shutdown: wait this long after the hangup before `SIGKILL`.
    pub term_grace: Duration,
    /// After the child's `wait`: drain the PTY for at most this long.
    pub exit_drain: Duration,
    /// History chunks are queued only while the attachment queue is shorter.
    pub backfill_low_water: usize,
    /// A start that finds the instance lock taken and no socket (the
    /// holder is on its way out, or has not bound yet) waits this long
    /// for the lock instead of giving up at once.
    pub lock_wait: Duration,
    /// Memory allowed for the history of all panes together, in bytes
    /// (rows x columns x cell size). Above it the largest histories are
    /// halved, panes nobody looks at first. Screens are never touched.
    pub history_budget: u64,
    /// How often the history budget is checked.
    pub history_check: Duration,
    /// Start pane children through the daemon executable, which restores
    /// the descriptor limit first. Set when the daemon raised its own
    /// limit; `None` starts them directly.
    pub pane_launcher: Option<PaneLauncher>,
    /// The daemon's own executable as a path that stays valid for the
    /// panes (not `/proc/self/exe`): where the pane commands are.
    pub cli_exe: Option<std::path::PathBuf>,
    /// Version announced in `HelloAck`.
    pub daemon_version: String,
    /// Fault injection for tests: PTY output that contains these bytes
    /// makes the pane's parser panic. Never set in production.
    #[doc(hidden)]
    pub fault_parser_panic: Option<Vec<u8>>,
    /// For tests: called when the daemon has decided on an idle exit,
    /// after its last ordinary `accept` and before it takes its socket
    /// out of the way. Never set in production.
    #[doc(hidden)]
    pub before_leaving: Option<TestHook>,
}

/// A callback a test plants in [`ServerConfig`].
#[doc(hidden)]
#[derive(Clone)]
pub struct TestHook(pub Arc<dyn Fn() + Send + Sync>);

impl std::fmt::Debug for TestHook {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TestHook")
    }
}

impl ServerConfig {
    /// What every pane is told about itself, on top of the environment its
    /// client asked for: its id (`SHELLDECK_PANE`), and, when the daemon
    /// knows its own executable, that path (`SHELLDECK_SESSIOND`) and a
    /// `PATH` that ends with its directory, so the pane commands of
    /// `docs/pane-cli.md` work from inside a pane. `path` is the daemon's
    /// own `PATH`; a directory already in it is not added twice, and it is
    /// added last so it shadows nothing.
    pub fn pane_identity_env(
        &self,
        pane: uuid::Uuid,
        path: Option<std::ffi::OsString>,
    ) -> Vec<(String, String)> {
        let mut env = vec![("SHELLDECK_PANE".to_string(), pane.to_string())];
        let Some(exe) = self.cli_exe.as_deref().and_then(|exe| exe.to_str()) else {
            return env;
        };
        env.push(("SHELLDECK_SESSIOND".to_string(), exe.to_string()));
        let Some(dir) = self.cli_exe.as_deref().and_then(|exe| exe.parent()) else {
            return env;
        };
        let mut dirs: Vec<std::path::PathBuf> = path
            .as_deref()
            .map(|path| std::env::split_paths(path).collect())
            .unwrap_or_default();
        if !dirs.iter().any(|known| known == dir) {
            dirs.push(dir.to_path_buf());
            if let Some(joined) = std::env::join_paths(dirs)
                .ok()
                .and_then(|joined| joined.into_string().ok())
            {
                env.push(("PATH".to_string(), joined));
            }
        }
        env
    }
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            hello_timeout: Duration::from_secs(10),
            frame_timeout: Duration::from_secs(30),
            write_timeout: Duration::from_secs(30),
            idle_exit: Some(Duration::from_secs(60)),
            dead_pane_ttl: Duration::from_secs(3600),
            output_queue_cap: 4 * 1024 * 1024,
            bulk_queue_cap: 64 * 1024 * 1024,
            input_queue_cap: 1024 * 1024,
            input_stall: Duration::from_secs(10),
            term_grace: Duration::from_secs(3),
            exit_drain: Duration::from_millis(250),
            backfill_low_water: 1024 * 1024,
            lock_wait: Duration::from_secs(5),
            history_budget: DEFAULT_HISTORY_BUDGET,
            history_check: Duration::from_secs(1),
            pane_launcher: None,
            cli_exe: None,
            daemon_version: env!("CARGO_PKG_VERSION").to_string(),
            fault_parser_panic: None,
            before_leaving: None,
        }
    }
}

/// Default of [`ServerConfig::history_budget`]: 2 GiB.
pub const DEFAULT_HISTORY_BUDGET: u64 = 2 * 1024 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum ServeError {
    #[error(transparent)]
    Path(#[from] PathError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServeOutcome {
    /// Another daemon holds the lock: this one did nothing.
    AlreadyRunning,
    /// No pane and no client for the idle timeout.
    IdleExit,
    /// Asked to stop (`SIGTERM`, or a test handle).
    Shutdown,
}

pub(crate) struct Shared {
    pub(crate) config: Arc<ServerConfig>,
    pub(crate) instance: Uuid,
    panes: Mutex<HashMap<Uuid, Arc<Pane>>>,
    connections: AtomicUsize,
    next_conn: AtomicU64,
    shutting_down: AtomicBool,
}

impl Shared {
    fn next_conn_id(&self) -> u64 {
        self.next_conn.fetch_add(1, Ordering::Relaxed)
    }

    fn is_shutting_down(&self) -> bool {
        self.shutting_down.load(Ordering::Relaxed)
    }

    fn pane(&self, id: &Uuid) -> Option<Arc<Pane>> {
        self.panes.lock().get(id).cloned()
    }

    fn pane_count(&self) -> usize {
        self.panes.lock().len()
    }

    fn insert_pane(&self, pane: Arc<Pane>) -> bool {
        let mut panes = self.panes.lock();
        if panes.len() >= MAX_PANES {
            return false;
        }
        panes.insert(pane.id, pane);
        true
    }

    fn remove_pane(&self, id: &Uuid) {
        let pane = self.panes.lock().remove(id);
        if let Some(pane) = pane {
            pane.close();
        }
    }

    /// Keep the history of all panes within the budget: while it is over,
    /// halve the largest history, panes without a viewer first. A pane
    /// keeps the lower limit afterwards. Returns the bytes in use at the
    /// end.
    fn enforce_history_budget(&self) -> u64 {
        let budget = self.config.history_budget;
        loop {
            let panes: Vec<Arc<Pane>> = self.panes.lock().values().cloned().collect();
            let usage: Vec<(Arc<Pane>, pane::HistoryUsage)> = panes
                .into_iter()
                .map(|pane| {
                    let usage = pane.history_usage();
                    (pane, usage)
                })
                .collect();
            let total: u64 = usage.iter().map(|(_, usage)| usage.bytes).sum();
            if total <= budget {
                return total;
            }
            // Unattached before attached, then the largest.
            let victim = usage
                .iter()
                .filter(|(_, usage)| usage.rows >= 2)
                .max_by_key(|(_, usage)| (!usage.attached, usage.bytes));
            let Some((pane, before)) = victim else {
                // Nothing left to trim: the screens alone are over budget.
                return total;
            };
            let kept = pane.halve_history();
            tracing::info!(
                pane = %pane.id,
                rows_before = before.rows,
                rows_kept = kept,
                total_bytes = total,
                budget_bytes = budget,
                "history over budget, trimmed"
            );
        }
    }

    /// The reported status of the panes that are still running, in the
    /// order of `list_panes`.
    fn list_status(&self) -> Vec<crate::protocol::PaneStatus> {
        let mut panes: Vec<Arc<Pane>> = self.panes.lock().values().cloned().collect();
        panes.sort_by_key(|pane| {
            let info = pane.info();
            (info.created_at, info.pane)
        });
        panes
            .iter()
            .filter(|pane| pane.info().alive)
            .filter_map(|pane| pane.status())
            .collect()
    }

    fn list_panes(&self) -> Vec<PaneInfo> {
        let panes: Vec<Arc<Pane>> = self.panes.lock().values().cloned().collect();
        let mut infos: Vec<PaneInfo> = panes.iter().map(|pane| pane.info()).collect();
        infos.sort_by_key(|info| (info.created_at, info.pane));
        infos
    }
}

/// One of the `MAX_CONNECTIONS` slots, taken in `Server::accept`. Dropping
/// it releases the slot, whichever way the connection ends (normal close,
/// no descriptor for the writer, a thread that cannot start, a panic).
pub(crate) struct ConnSlot(Arc<Shared>);

impl Drop for ConnSlot {
    fn drop(&mut self) {
        self.0.connections.fetch_sub(1, Ordering::Relaxed);
    }
}

/// How often a start that waits for the instance lock tries again.
const LOCK_POLL: Duration = Duration::from_millis(10);
/// How long an idle daemon keeps listening after it took its socket out of
/// the way, for connections that were already on their way.
const LEAVING_LINGER_MS: i32 = 30;

/// Wait after an `accept` failure that is not "nothing to accept".
const ACCEPT_BACKOFF: Duration = Duration::from_millis(50);

fn out_of_descriptors(error: &std::io::Error) -> bool {
    matches!(
        error.raw_os_error(),
        Some(libc::EMFILE) | Some(libc::ENFILE)
    )
}

fn spare_descriptor() -> Option<std::fs::File> {
    std::fs::File::open("/dev/null").ok()
}

/// Tell a connection that cannot be served why (`LimitExceeded`), before
/// it is closed.
pub(crate) fn refuse(stream: &std::os::unix::net::UnixStream) {
    if paths::check_peer(stream).is_err() {
        return;
    }
    let _ = stream.set_nonblocking(false);
    let _ = stream.set_write_timeout(Some(Duration::from_secs(1)));
    if let Ok(frame) = (DaemonFrame::Error {
        request_id: 0,
        pane: Uuid::nil(),
        code: ErrorCode::LimitExceeded,
        retryable: true,
    })
    .encode()
    {
        let mut stream = stream;
        let _ = stream.write_all(&frame);
    }
}

/// A daemon that holds the instance lock and listens on its socket.
pub struct Server {
    shared: Arc<Shared>,
    listener: UnixListener,
    socket: PathBuf,
    /// Held while the socket exists: only the holder may unlink it. Given
    /// up as soon as the socket is unlinked (see `shutdown`).
    lock: std::fs::File,
}

impl Server {
    /// Take the single-instance lock, replace a stale socket and listen.
    /// `Ok(None)` means another daemon is already serving this directory.
    pub fn bind(dir: &Path, config: ServerConfig) -> Result<Option<Self>, ServeError> {
        Self::bind_with(dir, config, || {})
    }

    /// [`bind`](Self::bind), with `locked` called once this process holds
    /// the instance lock and before the socket exists: the place to open
    /// what only the one daemon of a directory may open (its log), before
    /// any client can connect.
    pub fn bind_with(
        dir: &Path,
        config: ServerConfig,
        locked: impl FnOnce(),
    ) -> Result<Option<Self>, ServeError> {
        paths::ensure_run_dir(dir)?;
        let socket = paths::socket_path(dir);
        let lock = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .mode(0o600)
            .open(dir.join(paths::LOCK_NAME))?;
        // A lock that is taken next to a socket means a daemon serves the
        // directory. A lock taken without a socket is a daemon that has
        // unlinked its socket and is about to release the lock, or one
        // that has not bound yet: wait for whichever comes first, so a
        // start during the old daemon's exit is not lost.
        let waiting_since = Instant::now();
        loop {
            // SAFETY: flock on a descriptor this function owns.
            if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
                break;
            }
            let error = std::io::Error::last_os_error();
            if error.kind() != std::io::ErrorKind::WouldBlock {
                return Err(error.into());
            }
            if std::fs::symlink_metadata(&socket).is_ok()
                || waiting_since.elapsed() >= config.lock_wait
            {
                return Ok(None);
            }
            std::thread::sleep(LOCK_POLL);
        }

        // Left behind by a daemon that was killed in the middle of its
        // idle exit: only the lock holder removes it.
        let _ = std::fs::remove_file(socket.with_file_name(paths::SOCKET_LEAVING_NAME));

        locked();

        // The lock file names its holder, for diagnostics and tests.
        let mut lock = lock;
        lock.set_len(0)?;
        writeln!(lock, "{}", std::process::id())?;

        // Only the lock holder removes a stale socket.
        match std::fs::symlink_metadata(&socket) {
            Ok(_) => {
                paths::check_socket(&socket)?;
                std::fs::remove_file(&socket)?;
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
        // The 0700 directory protects the socket during bind; no umask call
        // (it is process-wide and would leak into panes).
        let listener = UnixListener::bind(&socket)?;
        std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600))?;
        paths::check_socket(&socket)?;
        if std::fs::symlink_metadata(&socket)?.permissions().mode() & 0o777 != 0o600 {
            return Err(PathError::Socket(socket).into());
        }
        listener.set_nonblocking(true)?;

        Ok(Some(Self {
            shared: Arc::new(Shared {
                config: Arc::new(config),
                instance: Uuid::new_v4(),
                panes: Mutex::new(HashMap::new()),
                connections: AtomicUsize::new(0),
                next_conn: AtomicU64::new(1),
                shutting_down: AtomicBool::new(false),
            }),
            listener,
            socket,
            lock,
        }))
    }

    /// Accept and serve until idle exit or until `stop` is set.
    pub fn run(self, stop: &AtomicBool) -> ServeOutcome {
        let shared = &self.shared;
        let mut last_busy = Instant::now();
        let mut last_history_check = Instant::now();
        // Kept open so that, with every other descriptor in use, a pending
        // connection can still be accepted for the time it takes to refuse
        // it. Without it the connection stays in the backlog, the listener
        // stays readable and this loop would spin.
        let mut spare = spare_descriptor();
        let outcome = loop {
            if stop.load(Ordering::Relaxed) {
                break ServeOutcome::Shutdown;
            }
            let mut pollfd = libc::pollfd {
                fd: self.listener.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            };
            // SAFETY: one valid pollfd for a descriptor owned by `self`.
            unsafe { libc::poll(&mut pollfd, 1, 50) };
            loop {
                match self.listener.accept() {
                    Ok((stream, _)) => self.accept(stream),
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                    Err(e)
                        if matches!(
                            e.kind(),
                            std::io::ErrorKind::Interrupted | std::io::ErrorKind::ConnectionAborted
                        ) => {}
                    Err(e) => {
                        if out_of_descriptors(&e) && spare.take().is_some() {
                            // The spare is closed: accept on its number,
                            // say why, close, take the spare back.
                            let refused = match self.listener.accept() {
                                Ok((stream, _)) => {
                                    refuse(&stream);
                                    true
                                }
                                Err(_) => false,
                            };
                            spare = spare_descriptor();
                            if refused && spare.is_some() {
                                continue;
                            }
                        }
                        // Anything else, or no spare to work with: back
                        // off instead of polling a listener that stays
                        // readable.
                        tracing::warn!(error = %e, "accept failed");
                        std::thread::sleep(ACCEPT_BACKOFF);
                        if spare.is_none() {
                            spare = spare_descriptor();
                        }
                        break;
                    }
                }
            }

            // Dead panes are kept for a while so a reattach can still show
            // the result, then expire.
            let expired: Vec<Uuid> = shared
                .panes
                .lock()
                .values()
                .filter(|pane| pane.expired(shared.config.dead_pane_ttl))
                .map(|pane| pane.id)
                .collect();
            for id in expired {
                shared.remove_pane(&id);
            }

            if last_history_check.elapsed() >= shared.config.history_check {
                last_history_check = Instant::now();
                shared.enforce_history_budget();
            }

            let busy = shared.pane_count() > 0 || shared.connections.load(Ordering::Relaxed) > 0;
            if busy {
                last_busy = Instant::now();
            } else if shared
                .config
                .idle_exit
                .is_some_and(|idle| last_busy.elapsed() >= idle)
            {
                if self.leave_if_unwanted() {
                    break ServeOutcome::IdleExit;
                }
                // A client arrived at the last moment: keep serving.
                last_busy = Instant::now();
            }
        };
        tracing::info!(?outcome, "session daemon stopping");
        self.shutdown();
        outcome
    }

    /// Last step before an idle exit. The socket is renamed out of the
    /// way, so no new connection can reach this listener, then the
    /// listener is read once more: a client whose `connect` had already
    /// found the socket is in the backlog by then. If there is one, the
    /// socket goes back to its name and the daemon keeps serving (`false`).
    /// Otherwise the socket is removed for good (`true`).
    ///
    /// Without this, a connection that arrives between the last `accept`
    /// and the close of the listener is reset, and its client has to guess
    /// that the daemon was leaving.
    fn leave_if_unwanted(&self) -> bool {
        if let Some(hook) = &self.shared.config.before_leaving {
            (hook.0)();
        }
        let leaving = self.socket.with_file_name(paths::SOCKET_LEAVING_NAME);
        if std::fs::rename(&self.socket, &leaving).is_err() {
            // The socket is gone already: nothing can connect.
            return true;
        }
        let mut pollfd = libc::pollfd {
            fd: self.listener.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: one valid pollfd for a descriptor owned by `self`.
        unsafe { libc::poll(&mut pollfd, 1, LEAVING_LINGER_MS) };
        let mut pending = Vec::new();
        while let Ok((stream, _)) = self.listener.accept() {
            pending.push(stream);
        }
        if pending.is_empty() {
            let _ = std::fs::remove_file(&leaving);
            return true;
        }
        if std::fs::rename(&leaving, &self.socket).is_err() {
            // Cannot serve under the public name any more: let these
            // clients see a closed connection and start a new daemon.
            let _ = std::fs::remove_file(&leaving);
            return true;
        }
        tracing::info!(
            connections = pending.len(),
            "a client connected during the idle exit, staying"
        );
        for stream in pending {
            self.accept(stream);
        }
        false
    }

    fn accept(&self, stream: std::os::unix::net::UnixStream) {
        // Close a peer with another uid before reading a byte.
        if paths::check_peer(&stream).is_err() {
            return;
        }
        let _ = stream.set_nonblocking(false);
        let shared = &self.shared;
        if shared.connections.load(Ordering::Relaxed) >= MAX_CONNECTIONS {
            refuse(&stream);
            return;
        }
        shared.connections.fetch_add(1, Ordering::Relaxed);
        let slot = ConnSlot(shared.clone());
        let shared_for_conn = shared.clone();
        // If the thread cannot start, the closure is dropped and the slot
        // with it.
        let _ = std::thread::Builder::new()
            .name("sessiond-conn-reader".into())
            .spawn(move || conn::run(shared_for_conn, stream, slot));
    }

    /// Stop serving: unlink the socket, give the instance lock up, hang up
    /// every pane (closing the master hangs up its session), wait for the
    /// grace period, then `SIGKILL` the process group of every pane child
    /// still alive.
    ///
    /// The lock is released before the grace period, not at process exit:
    /// from the moment the socket is unlinked this daemon never touches
    /// the run directory again, so a new daemon may take over at once. A
    /// client that starts one while this one is still ending its panes is
    /// served without waiting for them.
    fn shutdown(self) {
        let shared = &self.shared;
        shared.shutting_down.store(true, Ordering::Relaxed);
        let _ = std::fs::remove_file(&self.socket);
        drop(self.listener);
        // The log goes with the directory: the next daemon opens it.
        log::detach();
        // An explicit unlock, not a close: pane children forked a moment
        // ago still hold a copy of the descriptor until their `exec`.
        // SAFETY: flock on a descriptor owned by `self`.
        unsafe { libc::flock(self.lock.as_raw_fd(), libc::LOCK_UN) };

        let panes: Vec<Arc<Pane>> = shared.panes.lock().drain().map(|(_, pane)| pane).collect();
        let alive: Vec<&Arc<Pane>> = panes.iter().filter(|pane| pane.is_alive()).collect();
        for pane in &panes {
            pane.close();
        }
        let deadline = Instant::now() + shared.config.term_grace;
        while Instant::now() < deadline && alive.iter().any(|pane| pane.group_exists()) {
            std::thread::sleep(Duration::from_millis(10));
        }
        for pane in alive {
            if pane.group_exists() {
                pane.kill_group();
            }
        }
    }
}

/// Run a daemon in the foreground on `dir` until idle exit or `stop`.
pub fn serve(
    dir: &Path,
    config: ServerConfig,
    stop: &AtomicBool,
) -> Result<ServeOutcome, ServeError> {
    match Server::bind(dir, config)? {
        Some(server) => Ok(server.run(stop)),
        None => Ok(ServeOutcome::AlreadyRunning),
    }
}

/// A daemon running on a background thread of this process (tests, and
/// later an embedded fallback).
pub struct ServerHandle {
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<ServeOutcome>>,
}

impl ServerHandle {
    /// Bind synchronously (the socket is connectable when this returns),
    /// then serve on a thread. `Ok(None)`: another daemon holds the lock.
    pub fn start(dir: &Path, config: ServerConfig) -> Result<Option<Self>, ServeError> {
        let Some(server) = Server::bind(dir, config)? else {
            return Ok(None);
        };
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = stop.clone();
        let thread = std::thread::Builder::new()
            .name("sessiond-accept".into())
            .spawn(move || server.run(&thread_stop))?;
        Ok(Some(Self {
            stop,
            thread: Some(thread),
        }))
    }

    /// Stop the daemon and kill its panes.
    pub fn stop(mut self) -> ServeOutcome {
        self.stop.store(true, Ordering::Relaxed);
        self.join_inner()
    }

    /// Wait for the daemon to end by itself (idle exit).
    pub fn join(mut self) -> ServeOutcome {
        self.join_inner()
    }

    pub fn is_finished(&self) -> bool {
        self.thread.as_ref().is_none_or(|t| t.is_finished())
    }

    fn join_inner(&mut self) -> ServeOutcome {
        self.thread
            .take()
            .and_then(|thread| thread.join().ok())
            .unwrap_or(ServeOutcome::Shutdown)
    }
}

impl Drop for ServerHandle {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.join_inner();
    }
}
