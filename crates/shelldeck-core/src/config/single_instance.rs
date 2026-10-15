//! Single-instance guard + deep-link forwarding.
//!
//! ShellDeck should run as **one** process per user session: clicking a
//! second `.desktop`/dock entry, or following a `shelldeck://` deep link
//! while the app is open, must focus the *existing* window and route the
//! link there — not spawn a duplicate.
//!
//! ## How it works (portable, no platform `cfg`)
//!
//! The primary instance binds a **loopback TCP listener** on an ephemeral
//! port and writes `{port, token}` to `instance.json` in the config dir
//! (0600 on Unix). This mirrors the OIDC `browser_connect_listen` pattern
//! already used in [`crate::config::cloud_account`] — a loopback socket
//! with a shared-secret handshake is the idiomatic choice here and needs
//! zero Unix-socket / named-pipe `cfg` branching.
//!
//! A launching process:
//! 1. reads `instance.json`,
//! 2. tries to `connect` to `127.0.0.1:<port>` and send `<token>\n<payload>\n`,
//! 3. on a matching-token `OK` reply → [`Acquire::AlreadyRunning`] (exit),
//! 4. otherwise locks the persistent startup file, rechecks discovery, then
//!    binds and publishes a new primary only if no existing primary answers.
//!
//! The token defends against another local process on the same port
//! stealing the hand-off — a payload with the wrong token is dropped.

use crate::config::app_config::AppConfig;
use crate::util::atomic_write;
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};
use uuid::Uuid;

/// How long a launching process waits for the primary to ack the hand-off
/// before giving up and becoming primary itself.
const CONNECT_TIMEOUT: Duration = Duration::from_millis(800);
const STARTUP_LOCK_TIMEOUT: Duration = Duration::from_secs(5);

/// Discovery file: `{port, token}` of the live primary instance.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct InstanceInfo {
    port: u16,
    token: String,
}

/// Result of [`acquire`].
pub enum Acquire {
    /// This process is the primary. Hold on to [`Primary`] for the whole
    /// app lifetime, then call [`Primary::listen`] to receive forwarded
    /// deep links.
    Primary(Primary),
    /// Another instance is already running; if a payload was supplied it
    /// has been forwarded to it. The caller should exit cleanly.
    AlreadyRunning,
    /// Startup could not safely acquire or publish the primary identity.
    /// Callers must report failure instead of starting another runtime.
    Failed(std::io::Error),
}

/// Owns the loopback listener + secret for the running instance.
pub struct Primary {
    listener: TcpListener,
    token: String,
    info_path: PathBuf,
}

/// Path of the instance-discovery file.
fn info_path() -> PathBuf {
    AppConfig::config_dir().join("instance.json")
}

/// Try to become the single instance, forwarding `payload` (a
/// `shelldeck://…` URL, or `None` for a bare launch) to an existing
/// primary if one answers.
pub fn acquire(payload: Option<&str>) -> Acquire {
    acquire_at(info_path(), payload)
}

/// Testable core of [`acquire`] with an explicit discovery-file path.
fn acquire_at(path: PathBuf, payload: Option<&str>) -> Acquire {
    // 1. Is a primary already listening? Forward + bow out if so.
    if let Some(info) = read_info(&path) {
        if forward_to_primary(&info, payload) {
            return Acquire::AlreadyRunning;
        }
        // Stale file (crashed primary / wrong token): fall through and
        // take over the role.
    }

    // Only cold starts/takeovers touch the startup lock. Keep its inode in
    // place after unlocking: unlinking allows two locks on different inodes.
    let _startup_lock = match acquire_startup_lock(&path) {
        Ok(lock) => lock,
        Err(error) => return Acquire::Failed(error),
    };
    // Another cold launch may have published while this process waited.
    if let Some(info) = read_info(&path) {
        if forward_to_primary(&info, payload) {
            return Acquire::AlreadyRunning;
        }
    }

    // Bind and publish while holding the startup lock.
    let listener = match TcpListener::bind("127.0.0.1:0") {
        Ok(l) => l,
        Err(error) => return Acquire::Failed(error),
    };
    let port = match listener.local_addr() {
        Ok(address) => address.port(),
        Err(error) => return Acquire::Failed(error),
    };
    let token = Uuid::new_v4().simple().to_string();

    let info = InstanceInfo {
        port,
        token: token.clone(),
    };
    if let Err(error) = write_info(&path, &info) {
        return Acquire::Failed(error);
    }

    Acquire::Primary(Primary {
        listener,
        token,
        info_path: path,
    })
}

fn startup_lock_path(path: &Path) -> PathBuf {
    let mut lock_path = path.as_os_str().to_os_string();
    lock_path.push(".startup.lock");
    PathBuf::from(lock_path)
}

fn acquire_startup_lock(path: &Path) -> std::io::Result<File> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent)?;
    }
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options.open(startup_lock_path(path))?;
    let deadline = Instant::now() + STARTUP_LOCK_TIMEOUT;
    loop {
        match FileExt::try_lock_exclusive(&file) {
            Ok(()) => return Ok(file),
            Err(error) if error.raw_os_error() == fs2::lock_contended_error().raw_os_error() => {
                if Instant::now() >= deadline {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::TimedOut,
                        "Timed out waiting for another ShellDeck startup",
                    ));
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(error) => return Err(error),
        }
    }
}

impl Primary {
    /// Spawn the accept loop on a detached background thread and return a
    /// receiver of forwarded deep-link payloads. If `initial` is set (this
    /// process was itself launched *with* a deep link and no primary was
    /// running), it is delivered first so the freshly-opened window still
    /// honours the link that started it.
    ///
    /// The discovery file lives for the whole app lifetime (so the socket
    /// stays discoverable); the listener scrubs it when delivery stops.
    /// Process shutdown or a hard crash can leave the file behind, which is
    /// handled gracefully by the stale-file take-over path in [`acquire`].
    pub fn listen(self, initial: Option<String>) -> Receiver<String> {
        let (tx, rx) = mpsc::channel::<String>();

        self.listen_with(initial, move |payload| tx.send(payload).is_ok());

        rx
    }

    /// Start the accept loop and deliver payloads directly to `deliver`.
    /// Returning `false` from the callback stops the listener. This lets GUI
    /// callers bridge into an async channel without periodically polling the
    /// standard-library receiver returned by [`Primary::listen`].
    pub fn listen_with(
        self,
        initial: Option<String>,
        mut deliver: impl FnMut(String) -> bool + Send + 'static,
    ) {
        let Primary {
            listener,
            token,
            info_path,
        } = self;

        if let Some(link) = initial {
            if !deliver(link) {
                let _ = std::fs::remove_file(&info_path);
                return;
            }
        }

        if let Err(error) = std::thread::Builder::new()
            .name("shelldeck-ipc".to_string())
            .spawn(move || {
                loop {
                    match listener.accept() {
                        Ok((stream, _addr)) => {
                            if let Some(payload) = handle_handoff(stream, &token) {
                                // Empty payload is an authenticated bare
                                // launch: show the retained main surface.
                                // Receiver gone → app shutting down, stop.
                                if !deliver(payload) {
                                    break;
                                }
                            }
                        }
                        Err(e) => {
                            tracing::warn!("single-instance accept failed: {e}");
                            std::thread::sleep(Duration::from_millis(500));
                        }
                    }
                }
                let _ = std::fs::remove_file(&info_path);
            })
        {
            tracing::warn!("single-instance listener thread failed to start: {error}");
        }
    }

    /// Remove the discovery file. Best-effort — the OS also cleans the
    /// loopback socket when the process exits, but scrubbing the file
    /// avoids a stale-port hand-off race on the next launch. Used by tests
    /// and by callers that never call [`Primary::listen`].
    pub fn cleanup(&self) {
        let _ = std::fs::remove_file(&self.info_path);
    }
}

/// Read + parse the discovery file, returning `None` if absent/corrupt.
fn read_info(path: &PathBuf) -> Option<InstanceInfo> {
    let raw = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&raw).ok()
}

/// Write the discovery file (atomic), tightening perms to 0600 on Unix so
/// the hand-off token isn't world-readable.
fn write_info(path: &PathBuf, info: &InstanceInfo) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_vec(info).map_err(std::io::Error::other)?;
    atomic_write(path, &json)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

/// Connect to the primary and hand off `payload`. Returns `true` only when
/// the primary answered `OK` (so the caller knows it's safe to exit).
fn forward_to_primary(info: &InstanceInfo, payload: Option<&str>) -> bool {
    let addr = format!("127.0.0.1:{}", info.port);
    let socket_addr = match addr.parse() {
        Ok(a) => a,
        Err(_) => return false,
    };
    let mut stream = match TcpStream::connect_timeout(&socket_addr, CONNECT_TIMEOUT) {
        Ok(s) => s,
        Err(_) => return false,
    };
    let _ = stream.set_read_timeout(Some(CONNECT_TIMEOUT));
    let _ = stream.set_write_timeout(Some(CONNECT_TIMEOUT));

    // Protocol: one line token, one line payload (may be empty for a bare
    // "focus me" ping).
    let msg = format!("{}\n{}\n", info.token, payload.unwrap_or(""));
    if stream.write_all(msg.as_bytes()).is_err() {
        return false;
    }
    let _ = stream.flush();

    let mut resp = String::new();
    let _ = stream.read_to_string(&mut resp);
    resp.trim() == "OK"
}

/// Server side of the hand-off. Validates the token, acks `OK`, and
/// returns the payload line (empty string for a bare focus ping, `None`
/// on a token mismatch or malformed request).
fn handle_handoff(mut stream: TcpStream, expected_token: &str) -> Option<String> {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    let mut reader = BufReader::new(stream.try_clone().ok()?);

    let mut token_line = String::new();
    reader.read_line(&mut token_line).ok()?;
    if token_line.trim() != expected_token {
        return None;
    }

    let mut payload_line = String::new();
    reader.read_line(&mut payload_line).ok()?;

    let _ = stream.write_all(b"OK\n");
    let _ = stream.flush();

    Some(payload_line.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_info_path(tag: &str) -> PathBuf {
        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "shelldeck-si-{}-{}-{}",
            tag,
            std::process::id(),
            COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("instance.json")
    }

    // SDTEST-1321 — the first process becomes primary and writes a valid
    // discovery file; a second `acquire` with a payload forwards it and
    // bows out, and the primary receives the forwarded link. This is the
    // core single-instance invariant: exactly one window, deep links
    // routed to it. A regression here would spawn duplicate windows or
    // silently drop links.
    #[test]
    fn primary_then_secondary_forwards_payload() {
        let path = temp_info_path("forward");

        // First acquire → primary.
        let primary = match acquire_at(path.clone(), None) {
            Acquire::Primary(p) => p,
            Acquire::AlreadyRunning => panic!("first acquire must be primary"),
            Acquire::Failed(error) => panic!("acquire failed: {error}"),
        };
        assert!(path.exists(), "discovery file written");
        let rx = primary.listen(None);

        // Second acquire with a payload → forwarded, AlreadyRunning.
        let link = "shelldeck://issue/iss_42";
        match acquire_at(path.clone(), Some(link)) {
            Acquire::AlreadyRunning => {}
            Acquire::Primary(_) => panic!("second acquire must forward, not take over"),
            Acquire::Failed(error) => panic!("handoff failed: {error}"),
        }

        let received = rx
            .recv_timeout(Duration::from_secs(3))
            .expect("primary should receive the forwarded link");
        assert_eq!(received, link);
    }

    // SDTEST-2635 — SDUC-558
    #[test]
    fn sdtest_2635_agents_initial_and_secondary_intents_reach_one_primary_exactly_once() {
        use crate::config::deep_link::DeepLink;

        let path = temp_info_path("agents");
        let intent = "shelldeck://agents";
        let primary = match acquire_at(path.clone(), Some(intent)) {
            Acquire::Primary(primary) => primary,
            Acquire::AlreadyRunning => panic!("initial Agents launch must become primary"),
            Acquire::Failed(error) => panic!("acquire failed: {error}"),
        };
        let original = read_info(&path).expect("primary published discovery");
        let receiver = primary.listen(Some(intent.to_owned()));
        let initial = receiver
            .recv_timeout(Duration::from_secs(3))
            .expect("cold launch delivers its initial presentation intent");
        assert_eq!(DeepLink::parse(&initial), Some(DeepLink::Agents));

        match acquire_at(path.clone(), Some(intent)) {
            Acquire::AlreadyRunning => {}
            Acquire::Primary(primary) => {
                primary.cleanup();
                panic!("secondary Agents launch must reuse the existing primary");
            }
            Acquire::Failed(error) => panic!("handoff failed: {error}"),
        }
        let forwarded = receiver
            .recv_timeout(Duration::from_secs(3))
            .expect("secondary launch delivers to the existing primary");
        assert_eq!(DeepLink::parse(&forwarded), Some(DeepLink::Agents));

        match acquire_at(path.clone(), None) {
            Acquire::AlreadyRunning => {}
            Acquire::Primary(primary) => {
                primary.cleanup();
                panic!("bare launch must reuse the existing Agents primary");
            }
            Acquire::Failed(error) => panic!("handoff failed: {error}"),
        }
        let main = receiver
            .recv_timeout(Duration::from_secs(3))
            .expect("bare launch delivers a main-surface request");
        assert!(main.is_empty(), "bare launch remains distinct from Agents");
        let retained = read_info(&path).expect("primary discovery remains installed");
        assert_eq!(retained.port, original.port);
        assert_eq!(retained.token, original.token);
        assert!(receiver.try_recv().is_err(), "no intent may be replayed");
    }

    // SDTEST-2643 — SDUC-558
    #[test]
    fn sdtest_2643_concurrent_cold_launches_keep_one_primary_and_deliver_every_intent() {
        use std::collections::BTreeSet;
        use std::sync::{Arc, Barrier};

        const LAUNCHES: usize = 8;
        let path = temp_info_path("concurrent-agents");
        let held_lock = acquire_startup_lock(&path).unwrap();
        let barrier = Arc::new(Barrier::new(LAUNCHES + 1));
        let (results_tx, results_rx) = mpsc::channel();
        let mut workers = Vec::new();
        for index in 0..LAUNCHES {
            let path = path.clone();
            let barrier = barrier.clone();
            let results = results_tx.clone();
            workers.push(std::thread::spawn(move || {
                let intent = format!("shelldeck://agents?launch={index}");
                barrier.wait();
                let server = match acquire_at(path, Some(&intent)) {
                    Acquire::Primary(primary) => Some(std::thread::spawn(move || {
                        let Primary {
                            listener,
                            token,
                            info_path,
                        } = primary;
                        listener.set_nonblocking(true).unwrap();
                        let deadline = Instant::now() + Duration::from_secs(10);
                        let mut received = vec![intent];
                        while received.len() < LAUNCHES {
                            assert!(Instant::now() < deadline, "bounded primary timed out");
                            match listener.accept() {
                                Ok((stream, _)) => {
                                    if let Some(payload) = handle_handoff(stream, &token) {
                                        received.push(payload);
                                    }
                                }
                                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                                    std::thread::sleep(Duration::from_millis(2));
                                }
                                Err(error) => panic!("primary accept failed: {error}"),
                            }
                        }
                        std::fs::remove_file(info_path).unwrap();
                        received
                    })),
                    Acquire::AlreadyRunning => None,
                    Acquire::Failed(error) => panic!("concurrent launch failed: {error}"),
                };
                results.send(server).unwrap();
            }));
        }
        drop(results_tx);
        barrier.wait();
        assert!(
            matches!(
                results_rx.recv_timeout(Duration::from_millis(100)),
                Err(mpsc::RecvTimeoutError::Timeout)
            ),
            "cold launches must respect the existing startup-lock inode"
        );
        drop(held_lock);

        let mut servers = Vec::new();
        for _ in 0..LAUNCHES {
            if let Some(server) = results_rx.recv_timeout(Duration::from_secs(15)).unwrap() {
                servers.push(server);
            }
        }
        for worker in workers {
            worker.join().unwrap();
        }
        assert_eq!(servers.len(), 1, "only one cold launch may own a runtime");
        let received = servers.pop().unwrap().join().unwrap();
        assert_eq!(received.len(), LAUNCHES);
        assert_eq!(
            received.into_iter().collect::<BTreeSet<_>>(),
            (0..LAUNCHES)
                .map(|index| format!("shelldeck://agents?launch={index}"))
                .collect()
        );
        assert!(
            startup_lock_path(&path).is_file(),
            "lock inode stays installed"
        );

        // Lock/publication failures never pretend to hand off or return a host.
        let blocked_lock = temp_info_path("blocked-startup-lock");
        std::fs::create_dir(startup_lock_path(&blocked_lock)).unwrap();
        assert!(matches!(acquire_at(blocked_lock, None), Acquire::Failed(_)));
        let blocked_publication = temp_info_path("blocked-publication");
        std::fs::create_dir(&blocked_publication).unwrap();
        assert!(matches!(
            acquire_at(blocked_publication, None),
            Acquire::Failed(_)
        ));
    }

    // SDTEST-1322 — a stale discovery file (dead primary, nothing
    // listening) must not strand the app: the next launch takes over the
    // primary role instead of forwarding into the void.
    #[test]
    fn stale_discovery_file_is_taken_over() {
        let path = temp_info_path("stale");
        // Point at a port nobody is listening on.
        write_info(
            &path,
            &InstanceInfo {
                port: 1, // privileged + unbound in tests → connect fails fast
                token: "dead".to_string(),
            },
        )
        .unwrap();

        match acquire_at(path.clone(), Some("shelldeck://ticket/x")) {
            Acquire::Primary(p) => {
                // The discovery file was rewritten with a fresh token.
                let info = read_info(&path).expect("rewritten discovery file");
                assert_ne!(info.token, "dead");
                assert_ne!(info.port, 1);
                p.cleanup();
            }
            Acquire::AlreadyRunning => panic!("stale file must be taken over"),
            Acquire::Failed(error) => panic!("takeover failed: {error}"),
        }
    }

    // SDTEST-1323 — a hand-off carrying the wrong token is dropped: a
    // rogue local process cannot inject deep links into ShellDeck.
    #[test]
    fn wrong_token_handoff_is_rejected() {
        let path = temp_info_path("token");
        let primary = match acquire_at(path.clone(), None) {
            Acquire::Primary(p) => p,
            Acquire::AlreadyRunning => panic!("must be primary"),
            Acquire::Failed(error) => panic!("acquire failed: {error}"),
        };
        let info = read_info(&path).expect("discovery file");
        let rx = primary.listen(None);

        // Forge a connection with a bogus token.
        let bogus = InstanceInfo {
            port: info.port,
            token: "not-the-real-token".to_string(),
        };
        let forwarded = forward_to_primary(&bogus, Some("shelldeck://issue/evil"));
        assert!(!forwarded, "wrong token must not get an OK ack");
        assert!(
            rx.recv_timeout(Duration::from_millis(400)).is_err(),
            "wrong-token payload must never reach the receiver"
        );
    }
}
