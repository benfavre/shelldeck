//! The app facade (`attached::LocalSessions`) against the real daemon
//! binary, started detached on a private temporary run directory and
//! stopped with `SIGTERM` at the end. Nothing here touches the user's run
//! directory, sockets, tmux or processes.
#![cfg(unix)]

use shelldeck_sessiond::attached::{AttachError, LocalSessions, SessionOptions, SessionsOptions};
use shelldeck_sessiond::client::SpawnOptions;
use shelldeck_sessiond::paths;
use shelldeck_sessiond::proxy::ProxyFailure;
use shelldeck_terminal::session::TerminalSession;
use std::path::{Path, PathBuf};
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
