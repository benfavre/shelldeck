//! The app facade (`attached::LocalSessions`) against the real daemon
//! binary, started detached on a private temporary run directory and
//! stopped with `SIGTERM` at the end. Nothing here touches the user's run
//! directory, sockets, tmux or processes.
#![cfg(unix)]

use shelldeck_sessiond::attached::{AttachError, LocalSessions, SessionOptions, SessionsOptions};
use shelldeck_sessiond::client::SpawnOptions;
use shelldeck_sessiond::paths;
use shelldeck_sessiond::proxy::ProxyFailure;
use shelldeck_sessiond::supervisor::{
    Backoff, DaemonTarget, LinkEvent, Notices, PaneTarget, Reconnect, Step, Supervisor,
    SupervisorOptions,
};
use shelldeck_terminal::attached::AttachedPhase;
use shelldeck_terminal::session::TerminalSession;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use uuid::Uuid;

const BIN: &str = env!("CARGO_BIN_EXE_shelldeck-sessiond");
const LIMIT: Duration = Duration::from_secs(10);

/// Stops the daemon process started by the test even when the test fails.
struct TerminateOnDrop(u32);

impl Drop for TerminateOnDrop {
    fn drop(&mut self) {
        // SAFETY: plain signal to a daemon this test started on its own
        // private directory.
        unsafe { libc::kill(self.0 as libc::pid_t, libc::SIGTERM) };
    }
}

/// Send `SIGTERM` to every daemon process started on `dir`, found by its
/// command line (`serve ... --dir <dir>`). A test that starts the real
/// binary may leave more than one behind: a start is repeated while the
/// socket is missing, and a repeated start that was still waiting for the
/// instance lock becomes the daemon when the first one is stopped early.
/// The directory is private to one test, so nothing else can match.
fn terminate_daemons_of(dir: &Path) {
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::ffi::OsStrExt;
        let Ok(entries) = std::fs::read_dir("/proc") else {
            return;
        };
        for entry in entries.flatten() {
            let Some(pid) = entry
                .file_name()
                .to_str()
                .and_then(|name| name.parse::<u32>().ok())
            else {
                continue;
            };
            let Ok(cmdline) = std::fs::read(entry.path().join("cmdline")) else {
                continue;
            };
            let args: Vec<&[u8]> = cmdline.split(|byte| *byte == 0).collect();
            let serves = args.get(1) == Some(&&b"serve"[..]);
            let here = args
                .windows(2)
                .any(|pair| pair[0] == b"--dir" && pair[1] == dir.as_os_str().as_bytes());
            if serves && here && pid != std::process::id() {
                // SAFETY: plain signal to a daemon of this test's own
                // private directory.
                unsafe { libc::kill(pid as libc::pid_t, libc::SIGTERM) };
            }
        }
    }
    #[cfg(not(target_os = "linux"))]
    let _ = dir;
}

/// A private temporary directory whose daemons are stopped when the test
/// ends, pass or fail.
struct PrivateDir {
    run: PathBuf,
    _temp: tempfile::TempDir,
}

impl Drop for PrivateDir {
    fn drop(&mut self) {
        terminate_daemons_of(&self.run);
    }
}

fn process_exists(pid: u32) -> bool {
    // SAFETY: signal 0 only checks for existence.
    unsafe { libc::kill(pid as libc::pid_t, 0) == 0 }
}

fn screen_text(session: &TerminalSession) -> String {
    session
        .grid
        .lock()
        .visible_rows()
        .iter()
        .map(|row| {
            row.iter()
                .map(|cell| cell.c)
                .collect::<String>()
                .trim_end()
                .to_string()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn wait_until(what: &str, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + LIMIT;
    while !done() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn view(title: &str) -> SessionOptions {
    SessionOptions {
        title: title.into(),
        rows: 24,
        cols: 80,
        scrollback: 1_000,
        keep_pane_size: false,
    }
}

// SDTEST-2996 (SDUC-574): the app starts the daemon on demand, and a pane
// outlives the app's connection (quit, update) and is found again.
#[test]
fn sdtest_2996_connect_starts_the_daemon_and_panes_survive_the_client() {
    let mut temp = tempfile::Builder::new()
        .prefix("sdap-")
        .tempdir()
        .expect("temp dir");
    if temp.path().as_os_str().len() > 70 {
        temp = tempfile::Builder::new()
            .prefix("sdap-")
            .tempdir_in("/tmp")
            .expect("temp dir in /tmp");
    }
    let dir: PathBuf = temp.path().join("run");
    let _private = PrivateDir {
        run: dir.clone(),
        _temp: temp,
    };
    let options = SessionsOptions::default;

    // Looking for panes to offer back never starts a daemon.
    assert!(LocalSessions::connect_existing(&dir, options())
        .expect("no daemon is not an error")
        .is_none());
    assert!(!dir.exists(), "connect_existing created the run directory");

    // The daemon binary is an absolute path, never a PATH lookup.
    assert!(matches!(
        LocalSessions::connect(&dir, true, Path::new("shelldeck-sessiond"), options()),
        Err(AttachError::Daemon(ProxyFailure::StartFailed))
    ));
    assert!(!dir.exists());

    let sessions =
        LocalSessions::connect(&dir, true, Path::new(BIN), options()).expect("start and connect");
    let daemon_pid: u32 = std::fs::read_to_string(dir.join(paths::LOCK_NAME))
        .expect("lock file")
        .trim()
        .parse()
        .expect("daemon pid");
    assert_ne!(daemon_pid, std::process::id());
    let _cleanup = TerminateOnDrop(daemon_pid);

    let spawned = sessions
        .spawn(
            SpawnOptions {
                group: Uuid::from_u128(2),
                label: "app".into(),
                cwd: "/".into(),
                argv: vec!["/bin/sh".into()],
                env: vec![("TERM".into(), "xterm-256color".into())],
                ..SpawnOptions::default()
            },
            view("before quit"),
        )
        .expect("spawn");
    let pane = spawned.pane;
    let mark = format!("first_{}", spawned.pid);
    spawned.session.write_input(b"echo first_$$\r");
    wait_until("the echo", || screen_text(&spawned.session).contains(&mark));

    // The app quits: every session and the connection go away.
    drop(spawned);
    drop(sessions);
    assert!(process_exists(daemon_pid));

    // The next app start finds the pane, with its screen, same shell.
    let sessions = LocalSessions::connect_existing(&dir, options())
        .expect("connect")
        .expect("the daemon is still running");
    let panes = sessions.list_panes().expect("list");
    assert_eq!(panes.len(), 1);
    assert!(panes[0].pane == pane && panes[0].alive);
    let session = sessions
        .attach(pane, view("after restart"))
        .expect("attach");
    wait_until("the restored screen", || {
        screen_text(&session).contains(&mark)
    });
    session.write_input(b"echo again_$$\r");
    let again = format!("again_{}", &mark["first_".len()..]);
    wait_until("the second echo", || screen_text(&session).contains(&again));

    session.terminate();
    drop(session);
    wait_until("the pane to go away", || {
        sessions.list_panes().expect("list").is_empty()
    });
    drop(sessions);

    // SAFETY: the daemon this test started on its private directory.
    unsafe { libc::kill(daemon_pid as libc::pid_t, libc::SIGTERM) };
    wait_until("the daemon to stop", || !process_exists(daemon_pid));
}

fn private_run_dir() -> (PrivateDir, PathBuf) {
    let mut temp = tempfile::Builder::new()
        .prefix("sdap-")
        .tempdir()
        .expect("temp dir");
    if temp.path().as_os_str().len() > 70 {
        temp = tempfile::Builder::new()
            .prefix("sdap-")
            .tempdir_in("/tmp")
            .expect("temp dir in /tmp");
    }
    let dir = temp.path().join("run");
    (
        PrivateDir {
            run: dir.clone(),
            _temp: temp,
        },
        dir,
    )
}

fn daemon_pid(dir: &Path) -> Option<u32> {
    std::fs::read_to_string(dir.join(paths::LOCK_NAME))
        .ok()?
        .trim()
        .parse()
        .ok()
}

/// `binary`: the daemon binary, or `None` for an app that has none (it
/// can still use a daemon that runs).
fn new_supervisor(dir: &Path, binary: Option<&Path>, slow_first_resolve: bool) -> Supervisor {
    let dir = dir.to_path_buf();
    let binary = binary
        .map(Path::to_path_buf)
        .ok_or_else(|| "no daemon binary".to_string());
    let slow = Arc::new(AtomicBool::new(slow_first_resolve));
    Supervisor::new(
        Arc::new(move || {
            if slow.swap(false, Ordering::Relaxed) {
                // Stands for a slow daemon start: `open` must not wait.
                std::thread::sleep(Duration::from_millis(800));
            }
            Ok(DaemonTarget {
                dir: dir.clone(),
                explicit_dir: true,
                daemon_exe: binary.clone(),
            })
        }),
        SupervisorOptions {
            notices: Notices {
                connecting: Some("[connecting]".into()),
                connection_lost: Some("[lost]".into()),
                ended: Some("[ended]".into()),
                unavailable: Some("[unavailable]".into()),
                other_version: Some("[other version]".into()),
            },
            backoff: Backoff {
                start: Duration::from_millis(50),
                max: Duration::from_millis(200),
            },
            ..SupervisorOptions::default()
        },
    )
    .expect("supervisor")
}

fn shell() -> PaneTarget {
    PaneTarget::Spawn(SpawnOptions {
        group: Uuid::from_u128(3),
        label: "app".into(),
        cwd: "/".into(),
        argv: vec!["/bin/sh".into()],
        env: vec![("TERM".into(), "xterm-256color".into())],
        ..SpawnOptions::default()
    })
}

fn phase(session: &TerminalSession) -> AttachedPhase {
    session.attached_status().expect("attached").phase()
}

// SDTEST-3006 (SDUC-574): a supervised session is returned before the
// daemon is reached, goes live on its own, reattaches after the connection
// is lost, and reports a pane that died with its daemon as ended.
#[test]
fn sdtest_3006_supervised_session_connects_in_the_background_and_reconnects() {
    let (_temp, dir) = private_run_dir();
    let supervisor = new_supervisor(&dir, Some(Path::new(BIN)), true);

    // No daemon runs and the first resolve takes 800 ms: `open` returns
    // at once anyway, with a session that says it is connecting.
    let started = Instant::now();
    let session = supervisor.open(shell(), view("tab")).expect("open");
    assert!(
        started.elapsed() < Duration::from_millis(400),
        "open waited for the daemon: {:?}",
        started.elapsed()
    );
    assert_eq!(phase(&session), AttachedPhase::Connecting);
    assert_eq!(session.daemon_pane(), None);
    wait_until("the connecting notice", || {
        screen_text(&session).contains("[connecting]")
    });
    // Typed before the pane exists: delivered once it is attached.
    session.write_input(b"echo early_$$\r");

    wait_until("the session to go live", || {
        phase(&session) == AttachedPhase::Live
    });
    let first_daemon = daemon_pid(&dir).expect("daemon pid");
    let _cleanup = TerminateOnDrop(first_daemon);
    let pane = session.daemon_pane().expect("pane id once live");
    assert_eq!(supervisor.open_panes(), vec![pane]);
    wait_until("the early echo", || {
        // The shell's answer, not the echoed command line.
        screen_text(&session).lines().any(|line| {
            line.strip_prefix("early_")
                .is_some_and(|pid| !pid.is_empty() && pid.chars().all(|c| c.is_ascii_digit()))
        })
    });
    assert!(
        !screen_text(&session).contains("[connecting]"),
        "the notice must give way to the pane's screen"
    );
    let shell_pid: String = {
        session.write_input(b"echo pid_$$_end\r");
        let mut found = String::new();
        wait_until("the shell pid", || {
            let text = screen_text(&session);
            for line in text.lines() {
                if let Some(rest) = line.strip_prefix("pid_") {
                    if let Some(pid) = rest.strip_suffix("_end") {
                        if pid.chars().all(|c| c.is_ascii_digit()) {
                            found = pid.to_string();
                        }
                    }
                }
            }
            !found.is_empty()
        });
        found
    };

    // The same pane cannot be shown twice by one app.
    assert!(matches!(
        supervisor.open(PaneTarget::Existing(pane), view("twice")),
        Err(AttachError::AlreadyAttached)
    ));

    // The connection is lost while the daemon lives: notice, then the
    // session reattaches to the same shell without any call from here.
    supervisor.drop_connection();
    wait_until("the reattach", || {
        session.write_input(b"echo back_$$_end\r");
        screen_text(&session).contains(&format!("back_{shell_pid}_end"))
    });
    assert_eq!(phase(&session), AttachedPhase::Live);
    assert_eq!(session.daemon_pane(), Some(pane));
    assert_eq!(daemon_pid(&dir), Some(first_daemon), "no second daemon");

    // A second app (new supervisor) attaches to the pane by id, as layout
    // restore does, without resizing it. It has no daemon binary: the
    // daemon that runs is enough.
    let other = new_supervisor(&dir, None, false);
    assert_eq!(
        other
            .list_panes(false)
            .expect("list")
            .expect("the daemon runs")
            .iter()
            .map(|info| info.pane)
            .collect::<Vec<_>>(),
        vec![pane]
    );
    let restored = other
        .open(
            PaneTarget::Existing(pane),
            SessionOptions {
                keep_pane_size: true,
                rows: 10,
                cols: 40,
                ..view("restored")
            },
        )
        .expect("open existing");
    wait_until("the restored screen", || {
        screen_text(&restored).contains(&format!("back_{shell_pid}_end"))
    });
    {
        let grid = restored.grid.lock();
        assert_eq!((grid.rows, grid.cols), (24, 80), "pane size kept");
    }
    // A slot whose pane is gone is reported, not left connecting.
    let missing = other
        .open(PaneTarget::Existing(Uuid::from_u128(99)), view("missing"))
        .expect("open missing");
    wait_until("the missing pane to be reported", || {
        phase(&missing) == AttachedPhase::Ended { exit_code: None }
    });
    assert!(screen_text(&missing).contains("[ended]"));
    drop(missing);
    drop(restored);
    drop(other);

    // The daemon dies with its panes. The session notices, a daemon is
    // started again, and the pane is reported as ended: not frozen.
    // SAFETY: the daemon this test started on its private directory.
    unsafe { libc::kill(first_daemon as libc::pid_t, libc::SIGTERM) };
    wait_until("the pane to be reported as ended", || {
        phase(&session) == AttachedPhase::Ended { exit_code: None }
    });
    let text = screen_text(&session);
    assert!(
        text.contains("[lost]") && text.contains("[ended]"),
        "{text}"
    );
    wait_until("the old daemon to stop", || !process_exists(first_daemon));
    let second_daemon = daemon_pid(&dir).expect("a daemon was started again");
    assert_ne!(second_daemon, first_daemon);
    let _cleanup_second = TerminateOnDrop(second_daemon);
    wait_until("the ended pane to be released", || {
        supervisor.open_panes().is_empty()
    });

    // New terminals work on the new daemon; closing the tab ends the pane.
    let next = supervisor.open(shell(), view("next")).expect("open");
    wait_until("the next session to go live", || {
        phase(&next) == AttachedPhase::Live
    });
    next.terminate();
    drop(next);
    wait_until("the closed pane to go away", || {
        supervisor
            .list_panes(false)
            .expect("list")
            .is_some_and(|panes| panes.is_empty())
    });

    // A process that exits on its own is ended with its exit code and
    // keeps its final screen, without a notice.
    let exiting = supervisor.open(shell(), view("exit")).expect("open");
    wait_until("the exiting session to go live", || {
        phase(&exiting) == AttachedPhase::Live
    });
    exiting.write_input(b"echo bye; exit 3\r");
    wait_until("the exit to be reported", || {
        phase(&exiting) == AttachedPhase::Ended { exit_code: Some(3) }
    });
    assert!(!screen_text(&exiting).contains("[ended]"));
    exiting.terminate();
    drop(exiting);
    drop(session);
    drop(supervisor);

    // SAFETY: the daemon started by the supervisor on the private directory.
    unsafe { libc::kill(second_daemon as libc::pid_t, libc::SIGTERM) };
    wait_until("the second daemon to stop", || {
        !process_exists(second_daemon)
    });
}

// SDTEST-3007 (SDUC-574): a daemon that cannot be started makes the
// session unavailable, with the reason on screen, and starts nothing.
#[test]
fn sdtest_3007_supervised_session_reports_a_daemon_that_cannot_start() {
    let (_temp, dir) = private_run_dir();
    let supervisor = new_supervisor(&dir, Some(Path::new("shelldeck-sessiond")), false);
    let session = supervisor.open(shell(), view("tab")).expect("open");
    wait_until("the failure to be reported", || {
        phase(&session) == AttachedPhase::Unavailable
    });
    assert!(screen_text(&session).contains("[unavailable]"));
    assert_eq!(session.daemon_pane(), None);
    assert!(!dir.exists(), "nothing was created");
    // Typing into it and closing it are harmless.
    session.write_input(b"ls\r");
    session.terminate();
    drop(session);
    assert!(supervisor.open_panes().is_empty());
    assert!(matches!(supervisor.list_panes(false), Ok(None)));

    // Same without any binary at all: the reason given by the app is
    // what the tab shows.
    let supervisor = new_supervisor(&dir, None, false);
    let session = supervisor.open(shell(), view("tab")).expect("open");
    wait_until("the failure to be reported", || {
        phase(&session) == AttachedPhase::Unavailable
    });
    assert!(screen_text(&session).contains("no daemon binary"));
    assert!(!dir.exists(), "nothing was created");
}

/// Stands for a daemon left by a version of the app whose protocol this
/// one does not speak: it answers every `Hello` with `UnsupportedVersion`
/// and records everything else it is sent.
struct ForeignDaemon {
    stop: Arc<AtomicBool>,
    connections: Arc<std::sync::atomic::AtomicUsize>,
    /// Frames received after the `Hello` of a connection, and frames that
    /// were not a `Hello` where one was expected.
    unexpected: Arc<std::sync::atomic::AtomicUsize>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl ForeignDaemon {
    fn start(dir: &Path) -> Self {
        use shelldeck_sessiond::protocol::{ClientFrame, DaemonFrame, ErrorCode, FrameBuffer};
        use std::io::{Read, Write};
        use std::os::unix::fs::PermissionsExt;
        use std::sync::atomic::AtomicUsize;

        paths::ensure_run_dir(dir).expect("run dir");
        let socket = paths::socket_path(dir);
        let listener = std::os::unix::net::UnixListener::bind(&socket).expect("bind");
        std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600)).expect("mode");
        listener.set_nonblocking(true).expect("nonblocking");
        let stop = Arc::new(AtomicBool::new(false));
        let connections = Arc::new(AtomicUsize::new(0));
        let unexpected = Arc::new(AtomicUsize::new(0));
        let thread = {
            let (stop, connections, unexpected) =
                (stop.clone(), connections.clone(), unexpected.clone());
            std::thread::spawn(move || {
                while !stop.load(Ordering::Relaxed) {
                    let Ok((mut stream, _)) = listener.accept() else {
                        std::thread::sleep(Duration::from_millis(5));
                        continue;
                    };
                    connections.fetch_add(1, Ordering::SeqCst);
                    stream.set_nonblocking(false).expect("blocking");
                    stream
                        .set_read_timeout(Some(Duration::from_millis(300)))
                        .expect("timeout");
                    let mut frames = FrameBuffer::new();
                    let mut buf = [0u8; 4096];
                    let mut greeted = false;
                    loop {
                        while let Ok(Some(payload)) = frames.next_frame() {
                            match ClientFrame::decode(payload) {
                                Ok(ClientFrame::Hello(_)) if !greeted => {
                                    greeted = true;
                                    let refusal = DaemonFrame::Error {
                                        request_id: 0,
                                        pane: Uuid::nil(),
                                        code: ErrorCode::UnsupportedVersion,
                                        retryable: false,
                                    }
                                    .encode()
                                    .expect("encode");
                                    let _ = stream.write_all(&refusal);
                                }
                                _ => {
                                    unexpected.fetch_add(1, Ordering::SeqCst);
                                }
                            }
                        }
                        match stream.read(&mut buf) {
                            Ok(0) | Err(_) => break,
                            Ok(n) => frames.push(&buf[..n]),
                        }
                    }
                }
            })
        };
        Self {
            stop,
            connections,
            unexpected,
            thread: Some(thread),
        }
    }

    fn connections(&self) -> usize {
        self.connections.load(Ordering::SeqCst)
    }

    fn unexpected(&self) -> usize {
        self.unexpected.load(Ordering::SeqCst)
    }

    /// The other version's daemon ends (its idle exit).
    fn leave(mut self, dir: &Path) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            thread.join().expect("join");
        }
        std::fs::remove_file(paths::socket_path(dir)).expect("unlink");
    }
}

impl Drop for ForeignDaemon {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

// SDTEST-3032 (SDUC-574): a daemon left by another version of the app.
// With the same protocol it is simply used, whatever version it
// announces. With a protocol this app does not speak, it is left alone:
// nothing is started next to it, nothing but the refused `Hello` is sent
// to it, and each session says that it is managed by another version
// while keeping its pane id for the version that can show it.
#[test]
fn sdtest_3032_a_daemon_of_another_version_is_used_if_compatible_and_left_alone_if_not() {
    use shelldeck_sessiond::server::{ServerConfig, ServerHandle};
    use std::os::unix::fs::MetadataExt;

    // 1. Another release, same protocol.
    let (_temp, dir) = private_run_dir();
    let earlier = ServerHandle::start(
        &dir,
        ServerConfig {
            daemon_version: "0.1.0-earlier".into(),
            idle_exit: None,
            term_grace: Duration::from_millis(300),
            ..ServerConfig::default()
        },
    )
    .expect("daemon starts")
    .expect("the directory is free");
    // In-process daemon: the lock names this test process.
    assert_eq!(daemon_pid(&dir), Some(std::process::id()));
    let pane = {
        // The earlier app leaves a pane behind.
        let old_app = LocalSessions::connect_existing(&dir, SessionsOptions::default())
            .expect("connect")
            .expect("daemon runs");
        assert_eq!(old_app.daemon_version(), "0.1.0-earlier");
        let spawned = old_app
            .spawn(
                SpawnOptions {
                    label: "app".into(),
                    cwd: "/".into(),
                    argv: vec!["/bin/sh".into()],
                    env: vec![("TERM".into(), "xterm-256color".into())],
                    ..SpawnOptions::default()
                },
                view("old app"),
            )
            .expect("spawn");
        spawned.session.write_input(b"echo kept_$$\r");
        let mark = format!("kept_{}", spawned.pid);
        wait_until("the echo", || screen_text(&spawned.session).contains(&mark));
        (spawned.pane, mark)
    };
    // This app has its own daemon binary and still uses the one that runs.
    let supervisor = new_supervisor(&dir, Some(Path::new(BIN)), false);
    let listed = supervisor
        .list_panes(false)
        .expect("list")
        .expect("daemon runs");
    assert_eq!(listed.len(), 1);
    assert!(!supervisor.daemon_is_other_version());
    let session = supervisor
        .open(PaneTarget::Existing(pane.0), view("new app"))
        .expect("open");
    wait_until("the pane of the earlier app", || {
        phase(&session) == AttachedPhase::Live && screen_text(&session).contains(&pane.1)
    });
    session.write_input(b"echo typed_by_the_new_app\r");
    wait_until("input through the earlier daemon", || {
        screen_text(&session)
            .matches("typed_by_the_new_app")
            .count()
            == 2
    });
    // A new pane too, on that same daemon: nothing was started.
    let fresh = supervisor.open(shell(), view("fresh")).expect("open");
    wait_until("a new pane on the earlier daemon", || {
        phase(&fresh) == AttachedPhase::Live
    });
    assert_eq!(daemon_pid(&dir), Some(std::process::id()));
    assert!(!earlier.is_finished());
    drop(session);
    drop(fresh);
    drop(supervisor);
    drop(earlier);

    // 2. A protocol this app does not speak.
    let (_temp, dir) = private_run_dir();
    let foreign = ForeignDaemon::start(&dir);
    let socket_inode = std::fs::metadata(paths::socket_path(&dir))
        .expect("socket")
        .ino();
    let supervisor = new_supervisor(&dir, Some(Path::new(BIN)), false);
    assert!(!supervisor.daemon_is_other_version());

    // Listing, with or without permission to start a daemon: refused as
    // "other version", never "no daemon".
    for start in [false, true] {
        assert!(matches!(
            supervisor.list_panes(start),
            Err(AttachError::OtherVersion)
        ));
        assert!(supervisor.daemon_is_other_version());
    }
    // A saved pane of that daemon: shown as managed by another version,
    // at once (no retries), with its pane id kept.
    let saved = Uuid::from_u128(0x5a7ed);
    let started = Instant::now();
    let session = supervisor
        .open(PaneTarget::Existing(saved), view("saved tab"))
        .expect("open");
    wait_until("the other version to be reported", || {
        phase(&session) == AttachedPhase::OtherVersion
    });
    assert!(started.elapsed() < Duration::from_secs(2));
    assert!(screen_text(&session).contains("[other version]"));
    assert!(!screen_text(&session).contains("[ended]"));
    assert!(!screen_text(&session).contains("[unavailable]"));
    assert_eq!(session.daemon_pane(), Some(saved));
    // A new terminal asked of the supervisor all the same ends the same
    // way, without a pane.
    let fresh = supervisor.open(shell(), view("fresh")).expect("open");
    wait_until("the other version to be reported", || {
        phase(&fresh) == AttachedPhase::OtherVersion
    });
    assert_eq!(fresh.daemon_pane(), None);
    // Typing into such a tab and closing it reach nobody.
    session.write_input(b"exit\r");
    session.terminate();
    fresh.terminate();
    wait_until("the sessions to be over", || {
        supervisor.session_count() == 0
    });
    drop(session);
    drop(fresh);
    std::thread::sleep(Duration::from_millis(400));

    // Nothing destructive: no daemon of this version was started next to
    // it (no lock file, no log, the same socket), and all it ever
    // received is a `Hello` per connection.
    assert!(foreign.connections() >= 4);
    assert_eq!(foreign.unexpected(), 0, "frames beyond Hello were sent");
    assert_eq!(daemon_pid(&dir), None, "a daemon was started");
    assert!(!dir.join(paths::LOCK_NAME).exists());
    assert!(!dir.join(paths::LOG_NAME).exists());
    assert_eq!(
        std::fs::metadata(paths::socket_path(&dir))
            .expect("socket")
            .ino(),
        socket_inode,
        "the socket was replaced"
    );

    // The rule itself: from any state, with or without a pane.
    for has_pane in [false, true] {
        let mut machine = Reconnect::new(
            Backoff {
                start: Duration::from_millis(50),
                max: Duration::from_millis(200),
            },
            12,
            has_pane,
        );
        machine.on(LinkEvent::ConnectionLost { synced: true });
        assert_eq!(
            machine.on(LinkEvent::OtherVersion),
            Step::Finish(AttachedPhase::OtherVersion)
        );
    }

    // Once that daemon has ended by itself, this version starts its own.
    foreign.leave(&dir);
    assert!(!supervisor.recheck());
    assert!(!supervisor.daemon_is_other_version());
    let session = supervisor.open(shell(), view("after")).expect("open");
    wait_until("a pane on this version's daemon", || {
        phase(&session) == AttachedPhase::Live
    });
    let pid = daemon_pid(&dir).expect("this version's daemon runs");
    assert_ne!(pid, std::process::id());
    let _cleanup = TerminateOnDrop(pid);
    session.terminate();
    drop(session);
}
