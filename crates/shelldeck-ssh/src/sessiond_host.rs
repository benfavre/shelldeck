//! The session daemon of one remote host, for the app
//! (docs/design/session-daemon.md, section 15, "Remote app integration").
//!
//! A [`RemoteHost`] owns the one SSH connection that carries every pane of
//! that host, and a [`Supervisor`] whose daemon connection is an exec
//! channel on it ([`crate::sessiond::connect`]). Tabs and splits are
//! sessions of that supervisor, exactly like local ones: they are returned
//! at once, attach on their own thread and reattach after a loss. When the
//! SSH connection itself is lost, the next attempt dials again.
//!
//! [`RemoteHost::prepare`] is the first contact with a host: dial, probe
//! for the binary, install it when the user agreed, connect, list the
//! panes left by an earlier run. Its other outcome is a typed fallback
//! with the SSH connection handed back, so the caller can open a tmux or
//! plain shell on it instead.
//!
//! Every method that says "blocking" waits for network round trips: call
//! it from a plain thread, never from a UI thread and never from a task
//! of the runtime given to [`RemoteHost::new`].

use crate::session::SshSession;
use crate::sessiond::{
    self, RemoteBinary, SessiondConnectError, SessiondConnectOptions, TunnelEnd,
};
use crate::sessiond_install::{
    self, connect_fallback_reason, decide, Decision, FallbackReason, InstallConsent, InstallError,
    RemoteProbe,
};
use crate::SshError;
use parking_lot::Mutex;
use shelldeck_sessiond::attached::{AttachError, LocalSessions, SessionsOptions};
use shelldeck_sessiond::client::ClientError;
use shelldeck_sessiond::protocol::PaneInfo;
use shelldeck_sessiond::supervisor::{
    Connector, Supervisor, SupervisorOptions, UNLIMITED_ATTEMPTS,
};
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock, Weak};
use std::time::Duration;

pub type DialFuture = Pin<Box<dyn Future<Output = Result<SshSession, SshError>> + Send>>;
/// Opens a new SSH connection to the host. Called for the first contact
/// and again after the connection was lost.
pub type Dial = Arc<dyn Fn() -> DialFuture + Send + Sync>;

/// Something the owner of the host may want to show.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostEvent {
    /// The last session ended: the daemon channel and the SSH connection
    /// were released. Panes keep running on the host.
    Idle,
    /// The SSH connection was lost; the sessions are reconnecting.
    ConnectionLost,
    /// The daemon is reachable again after a loss.
    Reconnected,
}

type EventFn = Arc<dyn Fn(HostEvent) + Send + Sync>;

#[derive(Debug, Clone)]
pub struct RemoteHostOptions {
    /// Version of the remote binary: `shelldeck-sessiond-<version>` in
    /// `$HOME/.shelldeck/bin`. The app's own version.
    pub version: String,
    /// Where the bundled builds are looked up, in order.
    pub artifact_dirs: Vec<PathBuf>,
    pub supervisor: SupervisorOptions,
    /// Limit of one SSH dial (connect and authenticate).
    pub dial_timeout: Duration,
    /// Limits of the proxy start and the daemon handshake. The binary of
    /// these options is ignored: `version` decides.
    pub connect: SessiondConnectOptions,
}

impl Default for RemoteHostOptions {
    fn default() -> Self {
        Self {
            version: env!("CARGO_PKG_VERSION").to_string(),
            artifact_dirs: Vec::new(),
            supervisor: SupervisorOptions {
                // A host can be away for any length of time (network
                // loss, suspended laptop) and nothing else would reattach
                // a tab that gave up.
                max_attempts: UNLIMITED_ATTEMPTS,
                ..SupervisorOptions::default()
            },
            dial_timeout: Duration::from_secs(45),
            connect: SessiondConnectOptions::default(),
        }
    }
}

/// Outcome of [`RemoteHost::prepare`].
pub enum Prepared {
    /// The daemon answers. `panes`: every pane it has, to offer back those
    /// an earlier run left.
    Ready { panes: Vec<PaneInfo> },
    /// No session daemon for this connection. `session`: the SSH
    /// connection, for the fallback backend; `None` when the host still
    /// needs it (other sessions are reconnecting), in which case the
    /// caller dials its own. `ask`: offer the install to the user.
    Fallback {
        reason: FallbackReason,
        ask: bool,
        session: Option<Arc<SshSession>>,
    },
    /// The host could not be reached at all.
    ConnectFailed(SshError),
}

struct HostInner {
    handle: tokio::runtime::Handle,
    dial: Dial,
    options: RemoteHostOptions,
    supervisor: Supervisor,
    /// The shared SSH connection. Held while dialing, so two callers never
    /// dial at once.
    ssh: Mutex<Option<Arc<SshSession>>>,
    /// A daemon connection made by `prepare`, waiting for the supervisor
    /// to take it.
    prepared: Mutex<Option<LocalSessions>>,
    /// One `prepare` or `install` at a time.
    setup: Mutex<()>,
    ready: AtomicBool,
    lost: AtomicBool,
    /// An upload to this host was followed by "cannot execute" (a home
    /// directory mounted `noexec`, an exec that is refused). Nothing is
    /// uploaded to it again in this run: every new tab would otherwise
    /// send the whole binary once more before its shell opens.
    cannot_run: AtomicBool,
    on_event: Mutex<Option<EventFn>>,
}

/// The supervisor's way to the daemon: an exec channel on the host's SSH
/// connection.
struct HostConnector {
    host: OnceLock<Weak<HostInner>>,
}

impl Connector for HostConnector {
    fn connect(
        &self,
        start: bool,
        options: &SessionsOptions,
    ) -> Result<Option<LocalSessions>, AttachError> {
        let unreachable = || AttachError::Unavailable("the remote host is not connected".into());
        let host = self
            .host
            .get()
            .and_then(Weak::upgrade)
            .ok_or_else(unreachable)?;
        let prepared = host.prepared.lock().take();
        if let Some(sessions) = prepared.filter(|sessions| !sessions.is_closed()) {
            return Ok(Some(sessions));
        }
        if !start {
            // Never `Ok(None)`: an unreachable host says nothing about
            // its panes.
            return Err(unreachable());
        }
        // An SSH connection that died unnoticed only shows when a channel
        // cannot be opened on it: dial once more in that case.
        let mut last = None;
        for _ in 0..2 {
            let session = host
                .ensure_ssh()
                .map_err(|error| AttachError::Unavailable(error.to_string()))?;
            match host.connect_tunnel(&session, options.clone()) {
                Ok(sessions) => {
                    host.ready.store(true, Ordering::SeqCst);
                    if host.lost.swap(false, Ordering::SeqCst) {
                        host.emit(HostEvent::Reconnected);
                    }
                    return Ok(Some(sessions));
                }
                Err(error @ SessiondConnectError::Ssh(_)) => {
                    host.connection_lost(&session);
                    last = Some(error);
                }
                Err(error) => return Err(AttachError::Unavailable(error.to_string())),
            }
        }
        Err(AttachError::Unavailable(
            last.map(|error| error.to_string()).unwrap_or_default(),
        ))
    }
}

impl HostInner {
    fn emit(&self, event: HostEvent) {
        let handler = self.on_event.lock().clone();
        if let Some(handler) = handler {
            handler(event);
        }
    }

    /// Run `future` on the runtime and wait for it here. `None`: it did
    /// not finish in time and was cancelled.
    fn block<T, F>(&self, limit: Duration, future: F) -> Option<T>
    where
        T: Send + 'static,
        F: Future<Output = T> + Send + 'static,
    {
        let (tx, rx) = std::sync::mpsc::channel();
        let task = self.handle.spawn(async move {
            let _ = tx.send(future.await);
        });
        let result = rx.recv_timeout(limit).ok();
        if result.is_none() {
            task.abort();
        }
        result
    }

    /// The shared SSH connection, dialed when there is none.
    fn ensure_ssh(&self) -> Result<Arc<SshSession>, SshError> {
        let mut slot = self.ssh.lock();
        if let Some(session) = slot.as_ref() {
            return Ok(session.clone());
        }
        let dial = (self.dial)();
        let session = self
            .block(self.options.dial_timeout, dial)
            .ok_or(SshError::Timeout)??;
        let session = Arc::new(session);
        *slot = Some(session.clone());
        Ok(session)
    }

    /// Drop `session` if it is still the shared one. `false`: it was
    /// replaced or released already.
    fn forget_ssh(&self, session: &Arc<SshSession>) -> bool {
        let mut slot = self.ssh.lock();
        let shared = slot
            .as_ref()
            .is_some_and(|current| Arc::ptr_eq(current, session));
        if shared {
            *slot = None;
        }
        shared
    }

    fn connect_options(&self) -> SessiondConnectOptions {
        SessiondConnectOptions {
            binary: RemoteBinary::Versioned(self.options.version.clone()),
            ..self.options.connect.clone()
        }
    }

    /// Open the daemon channel on `session`.
    fn connect_tunnel(
        self: &Arc<Self>,
        session: &Arc<SshSession>,
        options: SessionsOptions,
    ) -> Result<LocalSessions, SessiondConnectError> {
        let connect = self.connect_options();
        // The proxy start, the daemon handshake, and a margin.
        let limit = connect.startup_timeout + connect.client.hello_timeout + Duration::from_secs(5);
        let on_session = session.clone();
        let connection = self
            .block(limit, async move {
                sessiond::connect(&on_session, connect).await
            })
            .ok_or(SessiondConnectError::Timeout)??;

        // Tell a lost SSH connection from a proxy that merely ended. A
        // plain thread, not a task: it may drop the last reference to the
        // host, which owns runtimes.
        let host = Arc::downgrade(self);
        let watched = session.clone();
        let ended = connection.ended;
        let _ = std::thread::Builder::new()
            .name("sessiond-ssh-watch".into())
            .spawn(move || {
                if let Ok(TunnelEnd::ConnectionLost) = ended.blocking_recv() {
                    if let Some(host) = host.upgrade() {
                        host.connection_lost(&watched);
                    }
                }
            });
        LocalSessions::from_client(connection.client, options)
            .map_err(|error| SessiondConnectError::Client(ClientError::Io(error)))
    }

    /// `session` turned out to be dead. Reported once per loss, whoever
    /// notices first (the channel watcher or a failed reconnect).
    fn connection_lost(&self, session: &Arc<SshSession>) {
        if !self.forget_ssh(session) {
            return;
        }
        self.ready.store(false, Ordering::SeqCst);
        if !self.lost.swap(true, Ordering::SeqCst) {
            self.emit(HostEvent::ConnectionLost);
        }
    }

    /// Let go of the daemon channel and the SSH connection when no session
    /// needs them. `true`: released.
    fn release_if_idle(&self) -> bool {
        // A pane that is still being ended (tab closed, sign-out) needs
        // the connection until it is gone: releasing now would leave it
        // running on the host after its first hang-up.
        if !self.supervisor.is_idle() {
            return false;
        }
        self.ready.store(false, Ordering::SeqCst);
        self.lost.store(false, Ordering::SeqCst);
        self.supervisor.close_connection();
        let prepared = self.prepared.lock().take();
        if let Some(prepared) = &prepared {
            prepared.close();
        }
        drop(prepared);
        let session = self.ssh.lock().take();
        if let Some(session) = session {
            // Best effort: dropping the last handle closes it as well.
            let _ = self.block(Duration::from_secs(2), async move {
                let _ = session.disconnect().await;
            });
        }
        true
    }

    /// A build for `target` that may be uploaded: an executable for that
    /// platform, built for this release (a build left by an earlier
    /// release does not count). Reads the file.
    fn has_artifact(&self, target: &str) -> bool {
        sessiond_install::load_artifact(&self.options.artifact_dirs, target, &self.options.version)
            .is_ok()
    }

    /// The rules of `decide`, and no upload (and no offer of one) to a
    /// host that already showed it cannot run what is uploaded.
    fn decision(&self, probe: Option<&RemoteProbe>, consent: InstallConsent) -> Decision {
        let decision = decide(probe, consent, |target| self.has_artifact(target));
        let would_upload = matches!(
            decision,
            Decision::Install { .. } | Decision::Fallback { ask: true, .. }
        );
        if would_upload && self.cannot_run.load(Ordering::SeqCst) {
            return Decision::Fallback {
                reason: FallbackReason::Unavailable,
                ask: false,
            };
        }
        decision
    }

    fn probe(&self, session: &Arc<SshSession>) -> Option<RemoteProbe> {
        let session = session.clone();
        let version = self.options.version.clone();
        match self.block(Duration::from_secs(20), async move {
            sessiond_install::probe(&session, &version).await
        }) {
            Some(Ok(probe)) => Some(probe),
            Some(Err(error)) => {
                tracing::info!("session daemon probe failed: {error}");
                None
            }
            None => None,
        }
    }

    fn upload(&self, session: &Arc<SshSession>, target: &str) -> Result<(), InstallFailure> {
        let artifact = sessiond_install::load_artifact(
            &self.options.artifact_dirs,
            target,
            &self.options.version,
        )
        .map_err(|error| InstallFailure::new(InstallFailureKind::NoArtifact, &error))?;
        let session = session.clone();
        let version = self.options.version.clone();
        let installed = self
            .block(Duration::from_secs(200), async move {
                sessiond_install::install(&session, &version, &artifact).await
            })
            .unwrap_or(Err(InstallError::Timeout));
        match installed {
            Ok(digest) => {
                tracing::info!(%digest, target, "session daemon installed on the remote host");
                Ok(())
            }
            Err(error) => {
                if matches!(error, InstallError::CannotExecute) {
                    self.cannot_run.store(true, Ordering::SeqCst);
                }
                let kind = match &error {
                    InstallError::ChecksumMismatch => InstallFailureKind::Checksum,
                    InstallError::NoChecksumTool => InstallFailureKind::NoChecksumTool,
                    InstallError::CannotExecute => InstallFailureKind::CannotExecute,
                    InstallError::Timeout => InstallFailureKind::Timeout,
                    InstallError::Ssh(_) => InstallFailureKind::Connection,
                    InstallError::BadArtifact => InstallFailureKind::NoArtifact,
                    InstallError::InvalidVersion | InstallError::Failed { .. } => {
                        InstallFailureKind::Other
                    }
                };
                Err(InstallFailure::new(kind, &error))
            }
        }
    }

    /// The SSH connection for a fallback backend, unless reconnecting
    /// sessions still need it.
    fn hand_over_ssh(&self) -> Option<Arc<SshSession>> {
        if !self.supervisor.is_idle() {
            return None;
        }
        self.ssh.lock().take()
    }
}

/// The cause of an [`InstallFailure`], for the text shown to the user
/// (which the application translates; [`InstallFailure::detail`] is in
/// English and goes to the log).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallFailureKind {
    /// No usable build for the host ships with the app (missing, for
    /// another release, not an executable for that platform, too large).
    NoArtifact,
    /// The copy on the host does not have the digest of the original.
    Checksum,
    /// The host has no tool to compute a SHA-256.
    NoChecksumTool,
    /// The host does not run files from the home directory (`noexec`).
    CannotExecute,
    /// The upload did not finish in time.
    Timeout,
    /// The SSH connection could not be opened or broke.
    Connection,
    /// The host is in a state where nothing is uploaded: the reason is
    /// the one a connection would fall back with.
    Fallback(FallbackReason),
    /// The install script failed in another way.
    Other,
}

/// Why an upload did not happen or did not verify.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{detail}")]
pub struct InstallFailure {
    pub kind: InstallFailureKind,
    /// English detail, for the log.
    pub detail: String,
}

impl InstallFailure {
    fn new(kind: InstallFailureKind, detail: &dyn std::fmt::Display) -> Self {
        Self {
            kind,
            detail: detail.to_string(),
        }
    }
}

/// The session daemon of one remote host. Cheap to clone.
#[derive(Clone)]
pub struct RemoteHost {
    inner: Arc<HostInner>,
}

impl RemoteHost {
    /// `handle`: the runtime the SSH connection lives on. It must outlive
    /// the host and keep running (a multi-thread runtime); nothing is
    /// dialed here.
    pub fn new(
        handle: tokio::runtime::Handle,
        dial: Dial,
        options: RemoteHostOptions,
    ) -> std::io::Result<Self> {
        let connector = Arc::new(HostConnector {
            host: OnceLock::new(),
        });
        let supervisor = Supervisor::with_connector(connector.clone(), options.supervisor.clone())?;
        let inner = Arc::new(HostInner {
            handle,
            dial,
            options,
            supervisor,
            ssh: Mutex::new(None),
            prepared: Mutex::new(None),
            setup: Mutex::new(()),
            ready: AtomicBool::new(false),
            lost: AtomicBool::new(false),
            cannot_run: AtomicBool::new(false),
            on_event: Mutex::new(None),
        });
        let _ = connector.host.set(Arc::downgrade(&inner));
        let host = Arc::downgrade(&inner);
        inner.supervisor.set_on_idle(move || {
            if let Some(host) = host.upgrade() {
                if host.release_if_idle() {
                    host.emit(HostEvent::Idle);
                }
            }
        });
        Ok(Self { inner })
    }

    /// Sessions of this host: `open` never blocks.
    pub fn supervisor(&self) -> &Supervisor {
        &self.inner.supervisor
    }

    /// `handler` runs on a background thread of the host.
    pub fn set_event_handler(&self, handler: impl Fn(HostEvent) + Send + Sync + 'static) {
        *self.inner.on_event.lock() = Some(Arc::new(handler));
    }

    /// The daemon of this host answered and the connection has not been
    /// lost or released since: a new session can be opened right away.
    /// Never blocks.
    pub fn is_ready(&self) -> bool {
        self.inner.ready.load(Ordering::SeqCst)
    }

    /// First contact for one connection: dial, probe, install when
    /// `consent` allows it, connect and list. Blocking.
    ///
    /// The upload happens only with [`InstallConsent::Allow`].
    pub fn prepare(&self, consent: InstallConsent) -> Prepared {
        let host = &self.inner;
        let _setup = host.setup.lock();
        if self.is_ready() {
            // Another connection to this host got there first.
            if let Ok(Some(panes)) = host.supervisor.list_panes(true) {
                return Prepared::Ready { panes };
            }
            host.ready.store(false, Ordering::SeqCst);
        }
        let session = match host.ensure_ssh() {
            Ok(session) => session,
            Err(error) => return Prepared::ConnectFailed(error),
        };
        let fallback = |reason, ask| Prepared::Fallback {
            reason,
            ask,
            session: host.hand_over_ssh(),
        };
        let probe = host.probe(&session);
        let mut uploaded = false;
        match host.decision(probe.as_ref(), consent) {
            Decision::Connect => {}
            Decision::Install { target } => {
                if let Err(error) = host.upload(&session, target) {
                    tracing::warn!("session daemon not installed on the remote host: {error}");
                    let reason = if host.cannot_run.load(Ordering::SeqCst) {
                        FallbackReason::Unavailable
                    } else {
                        FallbackReason::InstallFailed
                    };
                    return fallback(reason, false);
                }
                uploaded = true;
            }
            Decision::Fallback { reason, ask } => return fallback(reason, ask),
        }
        let sessions = match host.connect_tunnel(&session, host.options.supervisor.sessions.clone())
        {
            Ok(sessions) => sessions,
            Err(error) => {
                tracing::info!("session daemon not used on the remote host: {error}");
                if uploaded && matches!(error, SessiondConnectError::BinaryNotExecutable) {
                    // Uploaded, verified, and the host still says it is
                    // not executable: uploading again cannot help.
                    host.cannot_run.store(true, Ordering::SeqCst);
                }
                return fallback(connect_fallback_reason(&error), false);
            }
        };
        match sessions.list_panes() {
            Ok(panes) => {
                // Panes closed while the host was out of reach are ended
                // when the supervisor takes this connection: never
                // offered back.
                let panes = host.supervisor.without_doomed(panes);
                *host.prepared.lock() = Some(sessions);
                host.ready.store(true, Ordering::SeqCst);
                host.lost.store(false, Ordering::SeqCst);
                Prepared::Ready { panes }
            }
            Err(error) => {
                tracing::info!("session daemon panes not listed on the remote host: {error}");
                sessions.close();
                drop(sessions);
                fallback(FallbackReason::ConnectFailed, false)
            }
        }
    }

    /// The user just agreed to the install: upload the binary now, so the
    /// next connection to this host uses the daemon. Dials when the host
    /// is not connected and releases the connection afterwards if nothing
    /// else uses it. Blocking.
    pub fn install(&self) -> Result<(), InstallFailure> {
        let host = &self.inner;
        let _setup = host.setup.lock();
        let session = host
            .ensure_ssh()
            .map_err(|error| InstallFailure::new(InstallFailureKind::Connection, &error))?;
        let result = match host.probe(&session) {
            None => Err(InstallFailure::new(
                InstallFailureKind::Fallback(FallbackReason::ProbeFailed),
                &"the remote host did not answer the probe",
            )),
            Some(probe) => match host.decision(Some(&probe), InstallConsent::Allow) {
                Decision::Connect => Ok(()),
                Decision::Install { target } => host.upload(&session, target),
                Decision::Fallback { reason, .. } => Err(InstallFailure::new(
                    InstallFailureKind::Fallback(reason),
                    &format_args!("nothing is uploaded to this host: {reason:?}"),
                )),
            },
        };
        drop(session);
        if !self.is_ready() {
            host.release_if_idle();
        }
        result
    }

    /// Release the daemon channel and the SSH connection if no session
    /// uses them (a `Ready` that was not followed by a session). Blocking.
    pub fn release_if_idle(&self) {
        self.inner.release_if_idle();
    }
}
