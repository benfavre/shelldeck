//! The daemon-attached session backend against a real session daemon: the
//! daemon runs in-process on a private temporary run directory, with real
//! PTYs. Nothing here touches the user's run directory, sockets, tmux or
//! processes.
#![cfg(unix)]

use shelldeck_sessiond::attached::{LocalSessions, SessionOptions, SessionsOptions};
use shelldeck_sessiond::client::SpawnOptions;
use shelldeck_sessiond::server::{ServerConfig, ServerHandle};
use shelldeck_terminal::session::TerminalSession;
use std::path::PathBuf;
use std::time::{Duration, Instant};
use uuid::Uuid;

const LIMIT: Duration = Duration::from_secs(10);

/// A private run directory with a short path (`sun_path` is 108 bytes).
struct RunDir {
    _temp: tempfile::TempDir,
    path: PathBuf,
}

fn run_dir() -> RunDir {
    let mut temp = tempfile::Builder::new()
        .prefix("sdat-")
        .tempdir()
        .expect("temp dir");
    if temp.path().as_os_str().len() > 70 {
        temp = tempfile::Builder::new()
            .prefix("sdat-")
            .tempdir_in("/tmp")
            .expect("temp dir in /tmp");
    }
    let path = temp.path().join("run");
    RunDir { _temp: temp, path }
}

fn start_daemon(dir: &RunDir) -> ServerHandle {
    let config = ServerConfig {
        idle_exit: None,
        term_grace: Duration::from_millis(500),
        ..ServerConfig::default()
    };
    // Tests share one process: a pane child being forked by another test
    // holds a copy of the instance lock until its exec.
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match ServerHandle::start(&dir.path, config.clone()).expect("daemon starts") {
            Some(handle) => return handle,
            None => assert!(Instant::now() < deadline, "the instance lock stays taken"),
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn connect(dir: &RunDir) -> LocalSessions {
    LocalSessions::connect_existing(&dir.path, SessionsOptions::default())
        .expect("connect")
        .expect("the daemon is running")
}

fn shell() -> SpawnOptions {
    SpawnOptions {
        group: Uuid::from_u128(1),
        label: "attached-test".into(),
        cwd: "/".into(),
        argv: vec!["/bin/sh".into()],
        env: vec![("TERM".into(), "xterm-256color".into())],
        ..SpawnOptions::default()
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

fn wait_for_screen(session: &TerminalSession, needle: &str) {
    let deadline = Instant::now() + LIMIT;
    while !screen_text(session).contains(needle) {
        assert!(
            Instant::now() < deadline,
            "{needle:?} never reached the grid: {:?}",
            screen_text(session)
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

// SDTEST-2993 (SDUC-574): attach, echo, drop, reattach. The shell survives
// the drop with the same pid and its screen comes back from the snapshot.
#[test]
fn sdtest_2993_drop_detaches_and_reattach_finds_the_same_shell() {
    let dir = run_dir();
    let daemon = start_daemon(&dir);
    let sessions = connect(&dir);

    let spawned = sessions.spawn(shell(), view("first")).expect("spawn");
    assert_ne!(spawned.pid, 0);
    assert_eq!(spawned.cwd, "/");
    assert_eq!(
        spawned.session.initial_cwd(),
        Some(std::path::Path::new("/"))
    );
    let pane = spawned.pane;
    let pid_mark = format!("first_{}", spawned.pid);

    // `$$` is expanded by the shell itself: the answer proves the echo
    // went through the PTY of the process the daemon reported.
    spawned.session.write_input(b"echo first_$$\r");
    wait_for_screen(&spawned.session, &pid_mark);
    assert_eq!(sessions.attached_panes(), vec![pane]);

    // Dropping the session detaches. The pane stays, alive, with no client.
    drop(spawned);
    wait_until("the detach", || {
        let panes = sessions.list_panes().expect("list");
        panes.len() == 1 && panes[0].pane == pane && panes[0].alive && panes[0].clients == 0
    });
    assert!(sessions.attached_panes().is_empty());

    // A new connection (a restarted app) finds the pane. The old screen is
    // there before anything is typed: it came from the daemon's snapshot.
    drop(sessions);
    let sessions = connect(&dir);
    let listed = sessions.list_panes().expect("list");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].label, "attached-test");
    let session = sessions.attach(pane, view("second")).expect("attach");
    wait_for_screen(&session, &pid_mark);

    // Same process: `$$` still expands to the pid reported at spawn.
    session.write_input(b"echo again_$$\r");
    wait_for_screen(&session, &format!("again_{}", &pid_mark["first_".len()..]));

    // One pane, one attachment per connection.
    assert!(sessions.attach(pane, view("twice")).is_err());

    drop(session);
    drop(sessions);
    daemon.stop();
}

// SDTEST-2994 (SDUC-574): the view only requests a size. The grid and the
// child both end up at the size the daemon applied.
#[test]
fn sdtest_2994_resize_request_reaches_the_child_and_comes_back_to_the_grid() {
    let dir = run_dir();
    let daemon = start_daemon(&dir);
    let sessions = connect(&dir);
    let spawned = sessions.spawn(shell(), view("resize")).expect("spawn");
    let session = &spawned.session;
    session.write_input(b"echo ready\r");
    wait_for_screen(session, "ready");

    session.resize(31, 97);
    wait_until("the applied resize", || {
        let grid = session.grid.lock();
        (grid.rows, grid.cols) == (31, 97)
    });
    // `stty size` reads the window size from the tty itself.
    session.write_input(b"stty size\r");
    wait_for_screen(session, "31 97");

    session.terminate();
    drop(spawned);
    drop(sessions);
    daemon.stop();
}

// SDTEST-2995 (SDUC-574): closing a tab ends the pane (`terminate`). A
// shell that exits on its own stays listed as dead, with its final screen
// for every viewer, until its tab is closed.
#[test]
fn sdtest_2995_terminate_removes_the_pane_and_an_exited_shell_keeps_its_final_screen() {
    let dir = run_dir();
    let daemon = start_daemon(&dir);
    let sessions = connect(&dir);

    let closed = sessions.spawn(shell(), view("closed")).expect("spawn");
    let exited = sessions.spawn(shell(), view("exited")).expect("spawn");
    let kept = sessions.spawn(shell(), view("kept")).expect("spawn");
    wait_until("three panes", || {
        sessions.list_panes().expect("list").len() == 3
    });

    // The UI calls `terminate` and then drops the session.
    closed.session.terminate();
    let closed_pid = closed.pid;
    drop(closed);
    exited.session.write_input(b"echo last_$((40+2)); exit\r");

    wait_until(
        "the closed pane to go away and the exited one to die",
        || {
            let panes = sessions.list_panes().expect("list");
            panes.len() == 2
                && panes
                    .iter()
                    .all(|info| info.alive == (info.pane == kept.pane))
        },
    );
    wait_until("the closed shell to be reaped", || {
        !std::path::Path::new(&format!("/proc/{closed_pid}")).exists()
    });
    // The session that watched the exit does not remove the pane: another
    // app attached to it, even later, still gets the final screen.
    wait_for_screen(&exited.session, "last_42");
    let other = connect(&dir);
    let late = other.attach(exited.pane, view("late")).expect("attach");
    wait_for_screen(&late, "last_42");
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(sessions.list_panes().expect("list").len(), 2);

    // Closing the tab of the dead pane removes it.
    exited.session.terminate();
    drop(exited);
    wait_until("the dead pane to go away", || {
        sessions.list_panes().expect("list").len() == 1
    });
    drop(late);
    drop(other);

    kept.session.terminate();
    drop(kept);
    wait_until("the last pane to go away", || {
        sessions.list_panes().expect("list").is_empty()
    });
    drop(sessions);
    daemon.stop();
}

// SDTEST-2998 (SDUC-574): panes offered back at app start are attached
// before the view knows its size. Such an attach must not resize the pane:
// a placeholder 24x80 would send SIGWINCH to the program and cut the
// daemon's history to 80 columns for good.
#[test]
fn sdtest_2998_attach_without_a_size_keeps_the_pane_size_and_history_width() {
    let dir = run_dir();
    let daemon = start_daemon(&dir);
    let sessions = connect(&dir);
    let spawned = sessions.spawn(shell(), view("before quit")).expect("spawn");
    let pane = spawned.pane;
    spawned.session.resize(40, 150);
    wait_until("the applied resize", || {
        let grid = spawned.session.grid.lock();
        (grid.rows, grid.cols) == (40, 150)
    });
    // One line of 140 columns, then enough output to push it into history.
    let wide = "w".repeat(140);
    spawned
        .session
        .write_input(format!("printf '%s\\n' {wide}; seq 1 60; echo filled_$((1+1))\r").as_bytes());
    wait_for_screen(&spawned.session, "filled_2");
    drop(spawned);
    drop(sessions);

    let history_has_wide_line = |session: &TerminalSession| {
        // The oldest rows, through the local scroll position.
        let mut grid = session.grid.lock();
        let oldest = grid.scrollback_len();
        grid.set_scroll_offset(oldest);
        let found = grid
            .visible_rows()
            .iter()
            .any(|row| row.iter().map(|cell| cell.c).collect::<String>().trim_end() == wide);
        grid.set_scroll_offset(0);
        found
    };

    // The app restarts: the view has no size yet.
    let sessions = connect(&dir);
    let restored = sessions
        .attach(
            pane,
            SessionOptions {
                keep_pane_size: true,
                ..view("after restart")
            },
        )
        .expect("attach");
    wait_for_screen(&restored, "filled_2");
    wait_until("the backfilled wide line", || {
        history_has_wide_line(&restored)
    });
    {
        let grid = restored.grid.lock();
        assert_eq!((grid.rows, grid.cols), (40, 150), "the pane size was kept");
    }
    let info = &sessions.list_panes().expect("list")[0];
    assert_eq!((info.rows, info.cols), (40, 150));

    // The first size the view computes is taken by the pane.
    restored.resize(30, 100);
    wait_until("the applied resize", || {
        let grid = restored.grid.lock();
        (grid.rows, grid.cols) == (30, 100)
    });
    restored.write_input(b"stty size\r");
    wait_for_screen(&restored, "30 100");

    restored.terminate();
    drop(restored);
    drop(sessions);
    daemon.stop();
}
