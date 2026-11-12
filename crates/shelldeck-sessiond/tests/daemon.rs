//! Integration tests of the session daemon: a real daemon (in-process on a
//! private temporary directory, or the real binary for the proxy test),
//! real PTYs and the real client library. Nothing here touches the user's
//! run directory, sockets or processes.
#![cfg(unix)]

use shelldeck_sessiond::client::{
    connect_unix, AttachOptions, Client, ClientError, ClientOptions, MirrorOutcome, PaneEvent,
    PaneEvents, PaneMirror, SpawnOptions,
};
use shelldeck_sessiond::paths;
use shelldeck_sessiond::protocol::{
    Attach, ClientFrame, DaemonFrame, ErrorCode, FrameBuffer, Hello, PaneInfo, SIGNAL_KILL,
    SIGNAL_TERM,
};
use shelldeck_sessiond::server::{ServeError, ServeOutcome, Server, ServerConfig, ServerHandle};
use std::io::{Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use uuid::Uuid;

const BIN: &str = env!("CARGO_BIN_EXE_shelldeck-sessiond");
const STEP: Duration = Duration::from_secs(30);

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

/// A private run directory with a short path (`sun_path` is 108 bytes).
struct RunDir {
    _temp: tempfile::TempDir,
    path: PathBuf,
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

impl Drop for RunDir {
    fn drop(&mut self) {
        // Runs before the temporary directory itself is removed.
        terminate_daemons_of(&self.path);
    }
}

fn run_dir() -> RunDir {
    let mut temp = tempfile::Builder::new()
        .prefix("sdsd-")
        .tempdir()
        .expect("temp dir");
    if temp.path().as_os_str().len() > 70 {
        temp = tempfile::Builder::new()
            .prefix("sdsd-")
            .tempdir_in("/tmp")
            .expect("temp dir in /tmp");
    }
    let path = temp.path().join("run");
    RunDir { _temp: temp, path }
}

fn test_config() -> ServerConfig {
    ServerConfig {
        idle_exit: None,
        term_grace: Duration::from_millis(500),
        ..ServerConfig::default()
    }
}

fn start(dir: &RunDir, config: ServerConfig) -> ServerHandle {
    // The instance lock is an flock. Tests share one process, and a pane
    // child being forked by another test holds a copy of every descriptor
    // until its exec, so the lock of a daemon that just stopped can look
    // taken for an instant.
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match ServerHandle::start(&dir.path, config.clone()).expect("daemon starts") {
            Some(handle) => return handle,
            None => assert!(Instant::now() < deadline, "the instance lock stays taken"),
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// Stops a daemon process started by a test even when the test fails.
struct TerminateOnDrop(u32);

impl Drop for TerminateOnDrop {
    fn drop(&mut self) {
        // SAFETY: plain signal to a daemon this test started on its own
        // private directory.
        unsafe { libc::kill(self.0 as libc::pid_t, libc::SIGTERM) };
    }
}

async fn connect(dir: &RunDir) -> Client {
    connect_unix(&dir.path, ClientOptions::default())
        .await
        .expect("client connects")
}

fn command(argv: &[&str]) -> SpawnOptions {
    SpawnOptions {
        group: Uuid::from_u128(77),
        label: "test".into(),
        rows: 24,
        cols: 80,
        scrollback_max: 0,
        cwd: "/".into(),
        argv: argv.iter().map(|s| s.to_string()).collect(),
        env: vec![("TERM".into(), "xterm-256color".into())],
    }
}

fn sh(script: &str) -> SpawnOptions {
    command(&["/bin/sh", "-c", script])
}

fn attach_options(scrollback_rows: u32) -> AttachOptions {
    AttachOptions {
        rows: 24,
        cols: 80,
        scrollback_rows,
        read_only: false,
    }
}

fn process_exists(pid: u32) -> bool {
    // SAFETY: signal 0 only checks that the process exists.
    unsafe { libc::kill(pid as libc::pid_t, 0) == 0 }
}

async fn wait_for(what: &str, mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + STEP;
    while !condition() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

async fn pane_info(client: &Client, pane: Uuid) -> Option<PaneInfo> {
    client
        .list_panes()
        .await
        .expect("list")
        .into_iter()
        .find(|info| info.pane == pane)
}

/// The pane's stream offset once it has stopped moving.
async fn settled_offset(client: &Client, pane: Uuid) -> u64 {
    let mut last = u64::MAX;
    let mut stable = 0;
    loop {
        let offset = pane_info(client, pane).await.expect("pane listed").offset;
        stable = if offset == last { stable + 1 } else { 0 };
        if stable >= 5 {
            return offset;
        }
        last = offset;
        tokio::time::sleep(Duration::from_millis(40)).await;
    }
}

/// One client's view of a pane: its event stream applied to its own
/// emulator, with the raw bytes of the current attach kept for accounting.
struct View {
    client: Client,
    pane: Uuid,
    events: PaneEvents,
    mirror: PaneMirror,
    /// Stream offset of `segment[0]`: where the last snapshot was taken.
    segment_start: u64,
    /// Every byte applied since the last `Restore`.
    segment: Vec<u8>,
    restores: usize,
    kept_history: Vec<bool>,
    resyncs: usize,
    forced_reattaches: usize,
    history_done: bool,
    errors: Vec<ErrorCode>,
    exited: Option<(u64, i32, u8)>,
    last_snapshot: Vec<u8>,
}

impl View {
    fn attach(client: &Client, pane: Uuid, options: AttachOptions) -> Self {
        let events = client.attach(pane, options).expect("attach");
        Self {
            client: client.clone(),
            pane,
            events,
            mirror: PaneMirror::new(
                options.rows,
                options.cols,
                10_000,
                client.hello().emulator_rev,
            ),
            segment_start: 0,
            segment: Vec::new(),
            restores: 0,
            kept_history: Vec::new(),
            resyncs: 0,
            forced_reattaches: 0,
            history_done: false,
            errors: Vec::new(),
            exited: None,
            last_snapshot: Vec::new(),
        }
    }

    fn apply(&mut self, event: PaneEvent) {
        match &event {
            PaneEvent::Restore {
                snapshot,
                offset,
                keep_history,
                ..
            } => {
                self.restores += 1;
                self.kept_history.push(*keep_history);
                self.segment_start = *offset;
                self.segment.clear();
                self.history_done = false;
                self.last_snapshot = snapshot.clone();
            }
            PaneEvent::Bytes { offset, data } => {
                if self.mirror.in_sync() {
                    assert_eq!(
                        *offset,
                        self.segment_start + self.segment.len() as u64,
                        "the client library delivered a gap or a duplicate"
                    );
                    self.segment.extend_from_slice(data);
                }
            }
            PaneEvent::History { last, .. } => self.history_done |= *last,
            PaneEvent::Exited {
                offset,
                exit_code,
                signal,
            } => self.exited = Some((*offset, *exit_code, *signal)),
            PaneEvent::Resyncing => self.resyncs += 1,
            PaneEvent::Error(code) => self.errors.push(*code),
            PaneEvent::Resize { .. } | PaneEvent::Closed => {}
        }
        if self.mirror.apply(&event).expect("event applies") == MirrorOutcome::NeedsReattach {
            self.forced_reattaches += 1;
            self.client.reattach(self.pane, true).expect("reattach");
        }
    }

    async fn until(&mut self, what: &str, condition: impl Fn(&View) -> bool) {
        let deadline = tokio::time::Instant::now() + STEP;
        while !condition(self) {
            let event = tokio::time::timeout_at(deadline, self.events.next())
                .await
                .unwrap_or_else(|_| {
                    panic!(
                        "timed out waiting for {what}; offset {} screen:\n{}",
                        self.mirror.offset(),
                        self.screen()
                    )
                })
                .unwrap_or_else(|| panic!("event stream ended while waiting for {what}"));
            self.apply(event);
        }
    }

    /// Apply whatever is already queued, without waiting.
    fn drain(&mut self) {
        while let Some(event) = self.events.try_next() {
            self.apply(event);
        }
    }

    fn screen(&self) -> String {
        let grid = self.mirror.grid().lock();
        grid.cells
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

    fn snapshot(&self) -> Vec<u8> {
        self.mirror.grid().lock().snapshot()
    }

    fn offset(&self) -> u64 {
        self.mirror.offset()
    }

    fn in_sync_at(&self, offset: u64) -> bool {
        self.mirror.in_sync() && self.mirror.offset() == offset
    }
}

/// The daemon's own grid at a quiescent point, through a fresh attach: the
/// snapshot bytes and the offset they reflect.
async fn daemon_snapshot(dir: &RunDir, pane: Uuid) -> (Vec<u8>, u64) {
    let client = connect(dir).await;
    let mut events = client
        .attach(
            pane,
            AttachOptions {
                rows: 24,
                cols: 80,
                scrollback_rows: 0,
                // Read only: this probe must never take the pane size.
                read_only: true,
            },
        )
        .expect("attach");
    loop {
        let event = tokio::time::timeout(STEP, events.next())
            .await
            .expect("snapshot in time")
            .expect("stream open");
        if let PaneEvent::Restore {
            snapshot, offset, ..
        } = event
        {
            return (snapshot, offset);
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

// SDTEST-2971
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sdtest_2971_spawn_attach_and_input_echo() {
    let dir = run_dir();
    let daemon = start(&dir, test_config());
    let client = connect(&dir).await;
    assert_eq!(
        client.hello().emulator_rev,
        shelldeck_terminal::EMULATOR_REV
    );

    // The daemon opens the cwd itself and reports the canonical path.
    let cwd = std::fs::canonicalize(dir.path.parent().expect("parent")).expect("canonical");
    let mut options = command(&["/bin/cat"]);
    options.cwd = format!("{}/./", cwd.display());
    options.label = "echo pane".into();
    let spawned = client.spawn(options).await.expect("spawn");
    assert_eq!(Path::new(&spawned.cwd), cwd);
    assert!(process_exists(spawned.pid));

    // No fallback directory, and argv is never a shell command line.
    let mut missing = command(&["/bin/cat"]);
    missing.cwd = "/nonexistent/shelldeck-sessiond".into();
    assert!(matches!(
        client.spawn(missing).await,
        Err(ClientError::Daemon(ErrorCode::SpawnFailed))
    ));
    assert!(matches!(
        client.spawn(command(&["echo hi; true"])).await,
        Err(ClientError::Daemon(ErrorCode::SpawnFailed))
    ));

    let mut view = View::attach(&client, spawned.pane, attach_options(0));
    view.until("the first snapshot", |v| v.restores == 1).await;
    client
        .input(spawned.pane, b"hello-sessiond\n")
        .expect("input");
    // Once from the tty echo, once from cat.
    view.until("the echo", |v| {
        v.screen().matches("hello-sessiond").count() == 2
    })
    .await;

    let info = pane_info(&client, spawned.pane).await.expect("listed");
    assert!(info.alive);
    assert_eq!(info.clients, 1);
    assert_eq!(info.label, "echo pane");
    assert_eq!(info.group, Uuid::from_u128(77));
    assert_eq!((info.rows, info.cols), (24, 80));
    // The last bytes of the echo may still be on their way to this client.
    let offset = settled_offset(&client, spawned.pane).await;
    view.until("the end of the echo", |v| v.in_sync_at(offset))
        .await;
    assert_eq!(view.offset(), offset);

    assert_eq!(daemon.stop(), ServeOutcome::Shutdown);
    wait_for("the pane child to die with the daemon", || {
        !process_exists(spawned.pid)
    })
    .await;
}

// SDTEST-2972
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sdtest_2972_pane_survives_detach_and_reattach_reproduces_the_screen() {
    let dir = run_dir();
    let _daemon = start(&dir, test_config());
    let first = connect(&dir).await;
    let spawned = first
        .spawn(sh(
            "printf '\\033[1;31mred\\033[0m plain\\r\\n\\033[5;10Hmoved \\033[4munder'; exec cat",
        ))
        .await
        .expect("spawn");
    let mut view = View::attach(&first, spawned.pane, attach_options(0));
    view.until("the script output", |v| v.screen().contains("moved under"))
        .await;
    let offset = view.offset();
    let before = view.snapshot();

    // The client goes away entirely: connection closed, nothing attached.
    drop(view);
    drop(first);
    let second = connect(&dir).await;
    let mut clients = u16::MAX;
    while clients != 0 {
        clients = pane_info(&second, spawned.pane)
            .await
            .expect("pane still listed")
            .clients;
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let info = pane_info(&second, spawned.pane).await.expect("listed");
    assert!(info.alive, "detach must not hang up the pane");
    assert!(process_exists(spawned.pid));
    assert_eq!(info.offset, offset);

    // Reattach: the snapshot alone reproduces the screen, attributes and
    // cursor, byte for byte.
    let mut view = View::attach(&second, spawned.pane, attach_options(0));
    view.until("the snapshot", |v| v.restores == 1).await;
    assert_eq!(view.offset(), offset);
    assert_eq!(view.snapshot(), before);
    assert!(view.segment.is_empty());

    // And the pane is still usable.
    second.input(spawned.pane, b"again\n").expect("input");
    view.until("new output", |v| v.screen().matches("again").count() == 2)
        .await;

    // A detach (without closing the connection) behaves the same.
    second.detach(spawned.pane).expect("detach");
    while view.events.next().await.is_some() {}
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(process_exists(spawned.pid));
    assert!(matches!(
        second.detach(spawned.pane),
        Err(ClientError::NotAttached)
    ));
}

// SDTEST-2973
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn sdtest_2973_heavy_output_attach_race_loses_and_duplicates_nothing() {
    const FLOOD_BYTES: u64 = 1_500_000;
    let dir = run_dir();
    let _daemon = start(&dir, test_config());
    let client_a = connect(&dir).await;
    let spawned = client_a
        .spawn(command(&[
            BIN,
            "debug-flood",
            "--seed",
            "20261111",
            "--bytes",
            &FLOOD_BYTES.to_string(),
        ]))
        .await
        .expect("spawn");
    let pane = spawned.pane;

    // A is attached from the start and records the whole stream. Each
    // client parses on its own task, like separate processes would.
    let mut a = View::attach(&client_a, pane, attach_options(0));
    a.until("A's first snapshot", |v| v.restores == 1).await;
    let a_task = tokio::spawn(async move {
        a.until("the end of the flood", |v| {
            v.screen().contains("FLOOD-DONE")
        })
        .await;
        a
    });

    // B attaches, reads a little, detaches and attaches again, repeatedly,
    // while the pane floods. C attaches once in the middle and stays.
    let client_b = connect(&dir).await;
    let client_c = connect(&dir).await;
    let mut b_offsets = Vec::new();
    let mut c: Option<View> = None;
    // (offset, grid) of a late attacher right at its snapshot and a little
    // later, checked below against a replay of A's stream to that offset.
    let mut checkpoints: Vec<(u64, Vec<u8>)> = Vec::new();
    let b = loop {
        let mut b = View::attach(&client_b, pane, attach_options(0));
        b.until("B's snapshot", |v| v.restores == 1).await;
        b_offsets.push(b.segment_start);
        checkpoints.push((b.offset(), b.snapshot()));
        let some_output = b.offset() + 1500 * b_offsets.len() as u64;
        b.until("a little output", |v| v.offset() >= some_output)
            .await;
        checkpoints.push((b.offset(), b.snapshot()));
        if b_offsets.len() == 3 {
            let mut view = View::attach(&client_c, pane, attach_options(0));
            view.until("C's snapshot", |v| v.restores == 1).await;
            checkpoints.push((view.offset(), view.snapshot()));
            c = Some(view);
        }
        if b_offsets.len() == 6 {
            break b;
        }
        client_b.detach(pane).expect("detach");
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    let c = c.expect("C attached");

    let a = a_task.await.expect("A's task");
    let final_offset = settled_offset(&client_a, pane).await;
    assert!(final_offset > FLOOD_BYTES);
    assert!(
        b_offsets.windows(2).all(|w| w[0] < w[1]) && b_offsets[5] < final_offset,
        "B must attach at distinct offsets during the flood: {b_offsets:?}"
    );
    assert!(c.segment_start > 0 && c.segment_start < final_offset);

    let (daemon_grid, daemon_offset) = daemon_snapshot(&dir, pane).await;
    assert_eq!(daemon_offset, final_offset);
    let catch_up = |mut view: View| {
        tokio::spawn(async move {
            view.until("the final offset", |v| v.in_sync_at(final_offset))
                .await;
            view
        })
    };
    let (a_task, b_task, c_task) = (catch_up(a), catch_up(b), catch_up(c));
    let mut a = a_task.await.expect("A's task");
    let b = b_task.await.expect("B's task");
    let c = c_task.await.expect("C's task");
    for (name, view) in [("A", &a), ("B", &b), ("C", &c)] {
        // The reconstructed grid equals the daemon's: screen, cursor, modes.
        assert!(
            view.snapshot() == daemon_grid,
            "{name}'s grid differs from the daemon's at offset {final_offset}:\n{}",
            view.screen()
        );
        assert_eq!(view.resyncs, 0, "{name} lost sync");
        assert_eq!(view.restores, 1, "{name} needed more than one snapshot");
        // Every byte after the snapshot is accounted for exactly once.
        assert_eq!(
            view.segment_start + view.segment.len() as u64,
            final_offset,
            "{name}"
        );
    }
    // Each late attach was exact when it happened, not only at the end: a
    // replay of A's complete stream up to the attacher's offset gives the
    // same grid as its snapshot (plus tail) and the first bytes after it.
    let mut replay = PaneMirror::new(24, 80, 10_000, client_a.hello().emulator_rev);
    replay
        .apply(&PaneEvent::Restore {
            snapshot: a.last_snapshot.clone(),
            offset: a.segment_start,
            rows: 24,
            cols: 80,
            keep_history: false,
        })
        .expect("A's first snapshot");
    checkpoints.sort_by_key(|(offset, _)| *offset);
    assert!(checkpoints.len() >= 13);
    for (offset, grid) in &checkpoints {
        let from = (replay.offset() - a.segment_start) as usize;
        let to = (offset - a.segment_start) as usize;
        replay
            .apply(&PaneEvent::Bytes {
                offset: replay.offset(),
                data: a.segment[from..to].to_vec(),
            })
            .expect("replay");
        assert!(
            replay.grid().lock().snapshot() == *grid,
            "a client that attached mid-stream differs from the stream replay at offset {offset}"
        );
    }

    // B and C saw exactly the bytes A saw from their attach offset on.
    for (name, view) in [("B", &b), ("C", &c)] {
        let skip = (view.segment_start - a.segment_start) as usize;
        assert!(
            a.segment[skip..] == view.segment[..],
            "{name}'s stream differs from A's after offset {}",
            view.segment_start
        );
    }

    // Stop the generator: the exit is reported after the final output.
    client_a.input(pane, b"\n").expect("input");
    a.until("the exit", |v| v.exited.is_some()).await;
    let (exit_offset, exit_code, signal) = a.exited.expect("exited");
    assert_eq!((exit_code, signal), (0, 0));
    assert_eq!(exit_offset, a.offset());
}

// SDTEST-2974
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn sdtest_2974_slow_clients_are_dropped_and_resynced_without_stalling_the_pane() {
    const FLOOD_BYTES: u64 = 1_000_000;
    const QUEUE_CAP: usize = 128 * 1024;
    let dir = run_dir();
    let _daemon = start(
        &dir,
        ServerConfig {
            output_queue_cap: QUEUE_CAP,
            ..test_config()
        },
    );
    let client = connect(&dir).await;
    let spawned = client
        .spawn(sh(&format!(
            "read go; exec {BIN} debug-flood --seed 5 --bytes {FLOOD_BYTES}"
        )))
        .await
        .expect("spawn");
    let pane = spawned.pane;

    // A raw client that attaches and then reads nothing at all.
    let mut stalled = paths::connect_checked(&dir.path).expect("raw connect");
    stalled
        .set_read_timeout(Some(Duration::from_secs(20)))
        .expect("timeout");
    let hello = ClientFrame::Hello(Hello {
        proto_min: 1,
        proto_max: 1,
        features: 0,
        client_version: "stalled".into(),
    });
    let attach = ClientFrame::Attach(Attach {
        pane,
        attach_id: 41,
        rows: 24,
        cols: 80,
        scrollback_rows: 0,
        flags: 0,
    });
    stalled
        .write_all(
            &[
                hello.encode().expect("hello"),
                attach.encode().expect("attach"),
            ]
            .concat(),
        )
        .expect("write");

    // A library client whose consumer is slower than the link: its feed cap
    // is tiny and nobody pumps it during the flood.
    let slow_client = connect_unix(
        &dir.path,
        ClientOptions {
            max_feed_bytes: 32 * 1024,
            ..ClientOptions::default()
        },
    )
    .await
    .expect("connect");
    let mut slow = View::attach(&slow_client, pane, attach_options(0));

    // A healthy client keeps full speed through all of it.
    let mut fast = View::attach(&client, pane, attach_options(0));
    fast.until("the snapshot", |v| v.restores == 1).await;
    slow.until("the snapshot", |v| v.restores == 1).await;
    while pane_info(&client, pane).await.expect("listed").clients != 3 {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    client.input(pane, b"go\n").expect("input");
    fast.until("the end of the flood", |v| {
        v.screen().contains("FLOOD-DONE")
    })
    .await;
    let final_offset = settled_offset(&client, pane).await;
    assert!(final_offset > FLOOD_BYTES);
    assert_eq!(fast.resyncs, 0, "the healthy client was disturbed");
    assert_eq!(fast.restores, 1);
    assert_eq!(fast.segment_start + fast.segment.len() as u64, final_offset);

    // The stalled client was dropped at the cap: its last frame is
    // Error(Resync) for its attach, after at most the cap of output.
    let mut frames = FrameBuffer::new();
    let mut buf = vec![0u8; 64 * 1024];
    let mut output_bytes = 0usize;
    let mut expected = None;
    let dropped = 'read: loop {
        let n = stalled.read(&mut buf).expect("the daemon keeps writing");
        assert!(n > 0, "connection closed before Error(Resync)");
        frames.push(&buf[..n]);
        while let Some(payload) = frames.next_frame().expect("frame") {
            match DaemonFrame::decode(payload).expect("decode") {
                DaemonFrame::Ready { offset, .. } => expected = Some(offset),
                DaemonFrame::Output { offset, bytes, .. } => {
                    assert_eq!(
                        Some(offset),
                        expected,
                        "output before the drop is contiguous"
                    );
                    expected = Some(offset + bytes.len() as u64);
                    output_bytes += bytes.len();
                }
                DaemonFrame::Error {
                    request_id,
                    pane: error_pane,
                    code,
                    retryable,
                } => break 'read (request_id, error_pane, code, retryable),
                _ => {}
            }
        }
    };
    assert_eq!(dropped, (41, pane, ErrorCode::Resync, true));
    assert!(output_bytes < FLOOD_BYTES as usize / 2, "{output_bytes}");
    // Dropped means detached: only the two library clients remain.
    assert_eq!(pane_info(&client, pane).await.expect("listed").clients, 2);

    // The slow consumer overflowed its feed, was resynced from a fresh
    // snapshot (after the backoff) and ends up exactly on the daemon's grid.
    let (daemon_grid, daemon_offset) = daemon_snapshot(&dir, pane).await;
    assert_eq!(daemon_offset, final_offset);
    slow.until("the resync to settle", |v| v.in_sync_at(final_offset))
        .await;
    assert!(slow.resyncs >= 1, "the feed cap was never hit");
    assert!(slow.restores >= 2);
    assert_eq!(slow.snapshot(), daemon_grid);
    assert_eq!(fast.snapshot(), daemon_grid);
}

// SDTEST-2975
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sdtest_2975_two_clients_share_a_pane_and_get_exactly_one_query_reply() {
    let dir = run_dir();
    let _daemon = start(&dir, test_config());
    let client_a = connect(&dir).await;
    let client_b = connect(&dir).await;
    let client_c = connect(&dir).await;
    // Wait for "go", ask the terminal to identify itself (DA1), show the
    // reply as hex, then echo whatever else arrives: a second reply from
    // another emulator would show up after the dot.
    let spawned = client_a
        .spawn(sh(
            "read go; stty -echo -icanon min 0 time 20; printf 'ASK\\033[c'; \
             R=$(dd bs=256 count=1 2>/dev/null | od -An -tx1 | tr -d ' \\n'); \
             printf 'REPLY=%s.' \"$R\"; stty icanon echo; printf ' READY'; exec cat",
        ))
        .await
        .expect("spawn");
    let pane = spawned.pane;
    let mut a = View::attach(&client_a, pane, attach_options(0));
    let mut b = View::attach(&client_b, pane, attach_options(0));
    let mut c = View::attach(
        &client_c,
        pane,
        AttachOptions {
            read_only: true,
            ..attach_options(0)
        },
    );
    for view in [&mut a, &mut b, &mut c] {
        view.until("the snapshot", |v| v.restores == 1).await;
    }
    assert_eq!(pane_info(&client_a, pane).await.expect("listed").clients, 3);

    client_a.input(pane, b"go\n").expect("input");
    a.until("the reply", |v| v.screen().contains("READY")).await;
    let screen = a.screen();
    let reply = screen
        .split("REPLY=")
        .nth(1)
        .and_then(|rest| rest.split('.').next())
        .expect("reply printed")
        .to_string();
    assert!(
        reply.starts_with("1b5b3f") && reply.ends_with("63"),
        "not a DA1 reply: {reply}"
    );
    assert_eq!(
        reply.matches("1b5b").count(),
        1,
        "the application must get one reply, from the daemon's grid only"
    );

    // Input from both writers reaches the pane whole; everyone sees it.
    client_a.input(pane, b"from-a\n").expect("input");
    a.until("a's line", |v| v.screen().matches("from-a").count() == 2)
        .await;
    client_b.input(pane, b"from-b\n").expect("input");
    a.until("b's line", |v| v.screen().matches("from-b").count() == 2)
        .await;
    let offset = settled_offset(&client_a, pane).await;
    for view in [&mut a, &mut b, &mut c] {
        view.until("everything", |v| v.in_sync_at(offset)).await;
    }
    assert!(
        !a.screen().contains("?6") && !a.screen().contains("[?"),
        "a second query reply leaked into the pane:\n{}",
        a.screen()
    );
    assert_eq!(a.snapshot(), b.snapshot());
    assert_eq!(a.snapshot(), c.snapshot());

    // A read-only attachment cannot write, resize or kill.
    client_c.input(pane, b"nope\n").expect("queued");
    client_c.resize(pane, 10, 10).expect("queued");
    client_c.kill(pane, SIGNAL_TERM).expect("queued");
    c.until("three refusals", |v| v.errors.len() == 3).await;
    assert_eq!(c.errors, vec![ErrorCode::ReadOnly; 3]);
    let info = pane_info(&client_a, pane).await.expect("listed");
    assert!(info.alive);
    assert_eq!((info.rows, info.cols, info.offset), (24, 80, offset));
}

// SDTEST-2976
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sdtest_2976_pane_exit_is_reported_after_the_final_output() {
    let dir = run_dir();
    let _daemon = start(&dir, test_config());
    let client = connect(&dir).await;

    let spawned = client
        .spawn(sh("read go; printf 'last words'; exit 7"))
        .await
        .expect("spawn");
    let mut view = View::attach(&client, spawned.pane, attach_options(0));
    view.until("the snapshot", |v| v.restores == 1).await;
    client.input(spawned.pane, b"go\n").expect("input");
    view.until("the exit", |v| v.exited.is_some()).await;
    let (offset, exit_code, signal) = view.exited.expect("exited");
    assert_eq!((exit_code, signal), (7, 0));
    assert_eq!(offset, view.offset(), "Exited follows the final output");
    assert!(view.screen().contains("last words"));

    // The dead pane stays listed with its final screen and status.
    let info = pane_info(&client, spawned.pane)
        .await
        .expect("still listed");
    assert!(!info.alive);
    assert_eq!(info.exit_code, 7);
    let late = connect(&dir).await;
    let mut late_view = View::attach(&late, spawned.pane, attach_options(0));
    late_view.until("the exit", |v| v.exited.is_some()).await;
    assert_eq!(late_view.exited, view.exited);
    assert_eq!(late_view.snapshot(), view.snapshot());

    // Kill on a dead pane removes it.
    client.kill(spawned.pane, SIGNAL_TERM).expect("kill");
    assert!(pane_info(&client, spawned.pane).await.is_none());
    assert!(!late.is_closed());

    // A signal death carries the signal.
    let sleeper = client
        .spawn(command(&["/bin/sleep", "600"]))
        .await
        .expect("spawn");
    let mut view = View::attach(&client, sleeper.pane, attach_options(0));
    view.until("the snapshot", |v| v.restores == 1).await;
    client.kill(sleeper.pane, SIGNAL_KILL).expect("kill");
    view.until("the exit", |v| v.exited.is_some()).await;
    let (_, exit_code, signal) = view.exited.expect("exited");
    assert_eq!((exit_code, signal), (137, 9));
    assert!(!process_exists(sleeper.pid));

    // Attaching a pane that does not exist is an error for that pane only.
    let mut ghost = View::attach(&client, Uuid::from_u128(1), attach_options(0));
    ghost.until("the refusal", |v| !v.errors.is_empty()).await;
    assert_eq!(ghost.errors, vec![ErrorCode::NoSuchPane]);
    assert!(!client.is_closed());
}

// SDTEST-2977
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sdtest_2977_single_instance_idle_exit_stale_socket_and_private_directory() {
    let dir = run_dir();
    let socket = paths::socket_path(&dir.path);

    // Second instance: does nothing while the first keeps serving.
    let first = start(&dir, test_config());
    assert!(ServerHandle::start(&dir.path, test_config())
        .expect("no error")
        .is_none());
    let client = connect(&dir).await;
    assert!(client.list_panes().await.expect("list").is_empty());
    assert_eq!(
        std::fs::metadata(&socket)
            .expect("socket")
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    assert_eq!(
        std::fs::metadata(&dir.path)
            .expect("dir")
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
    drop(client);
    assert_eq!(first.stop(), ServeOutcome::Shutdown);
    assert!(!socket.exists(), "the socket is unlinked on exit");

    // Idle exit: no pane and no client for the idle timeout. A connected
    // client holds it off.
    let idle = start(
        &dir,
        ServerConfig {
            idle_exit: Some(Duration::from_millis(300)),
            ..test_config()
        },
    );
    let client = connect(&dir).await;
    tokio::time::sleep(Duration::from_millis(600)).await;
    assert!(
        !idle.is_finished(),
        "a connected client keeps the daemon up"
    );
    drop(client);
    let outcome = tokio::task::spawn_blocking(move || idle.join())
        .await
        .expect("join");
    assert_eq!(outcome, ServeOutcome::IdleExit);
    assert!(!socket.exists());

    // Stale socket left by a crash: replaced by the next lock holder.
    drop(std::os::unix::net::UnixListener::bind(&socket).expect("stale socket"));
    assert!(socket.exists());
    assert!(matches!(
        connect_unix(&dir.path, ClientOptions::default()).await,
        Err(ClientError::Path(paths::PathError::Io(_)))
    ));
    let recovered = start(&dir, test_config());
    let client = connect(&dir).await;
    assert!(client.list_panes().await.expect("list").is_empty());
    drop(client);

    // A run directory readable by others is refused by daemon and client,
    // and never "fixed".
    let open_dir = dir.path.with_file_name("open");
    std::fs::create_dir(&open_dir).expect("mkdir");
    std::fs::set_permissions(&open_dir, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    assert!(matches!(
        Server::bind(&open_dir, test_config()),
        Err(ServeError::Path(paths::PathError::RunDir(_)))
    ));
    assert!(matches!(
        connect_unix(&open_dir, ClientOptions::default()).await,
        Err(ClientError::Path(paths::PathError::RunDir(_)))
    ));
    assert_eq!(
        std::fs::metadata(&open_dir)
            .expect("dir")
            .permissions()
            .mode()
            & 0o777,
        0o755
    );

    // A symlink to the real directory is refused too, although the daemon
    // behind it is healthy.
    let link = dir.path.with_file_name("link");
    std::os::unix::fs::symlink(&dir.path, &link).expect("symlink");
    assert!(matches!(
        connect_unix(&link, ClientOptions::default()).await,
        Err(ClientError::Path(paths::PathError::RunDir(_)))
    ));
    drop(recovered);
}

// SDTEST-2978
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sdtest_2978_frames_before_hello_bad_frames_and_stalled_frames_only_detach() {
    let dir = run_dir();
    let _daemon = start(
        &dir,
        ServerConfig {
            hello_timeout: Duration::from_millis(300),
            frame_timeout: Duration::from_millis(300),
            ..test_config()
        },
    );
    let client = connect(&dir).await;
    let spawned = client.spawn(command(&["/bin/cat"])).await.expect("spawn");

    let read_frames = |stream: &mut std::os::unix::net::UnixStream| -> Vec<DaemonFrame> {
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .expect("timeout");
        let mut frames = FrameBuffer::new();
        let mut decoded = Vec::new();
        let mut buf = [0u8; 4096];
        loop {
            match stream.read(&mut buf) {
                Ok(0) => return decoded,
                Ok(n) => {
                    frames.push(&buf[..n]);
                    while let Some(payload) = frames.next_frame().expect("frame") {
                        decoded.push(DaemonFrame::decode(payload).expect("decode"));
                    }
                }
                Err(e) => panic!("the daemon did not close the connection: {e}"),
            }
        }
    };
    let bad_frame = DaemonFrame::Error {
        request_id: 0,
        pane: Uuid::nil(),
        code: ErrorCode::BadFrame,
        retryable: false,
    };
    let hello = ClientFrame::Hello(Hello {
        proto_min: 1,
        proto_max: 1,
        features: 0,
        client_version: "raw".into(),
    })
    .encode()
    .expect("hello");

    // A frame before Hello.
    let mut raw = paths::connect_checked(&dir.path).expect("connect");
    raw.write_all(&ClientFrame::ListPanes.encode().expect("list"))
        .expect("write");
    assert_eq!(read_frames(&mut raw), vec![bad_frame.clone()]);

    // An unsupported protocol range.
    let mut raw = paths::connect_checked(&dir.path).expect("connect");
    let future = ClientFrame::Hello(Hello {
        proto_min: 2,
        proto_max: 3,
        features: 0,
        client_version: "future".into(),
    });
    raw.write_all(&future.encode().expect("hello"))
        .expect("write");
    assert_eq!(
        read_frames(&mut raw),
        vec![DaemonFrame::Error {
            request_id: 0,
            pane: Uuid::nil(),
            code: ErrorCode::UnsupportedVersion,
            retryable: false,
        }]
    );

    // After a good Hello: an unknown type, a daemon-direction frame, an
    // oversize length and a disallowed environment name all fail closed.
    let mut spawn_with_path = ClientFrame::Spawn(shelldeck_sessiond::protocol::Spawn {
        request_id: 1,
        group: Uuid::nil(),
        label: String::new(),
        rows: 24,
        cols: 80,
        scrollback_max: 0,
        cwd: String::new(),
        argv: vec!["/bin/true".into()],
        env: vec![("TERM".into(), "x".into())],
    })
    .encode()
    .expect("spawn");
    let at = spawn_with_path
        .windows(4)
        .position(|w| w == b"TERM")
        .expect("env name");
    spawn_with_path[at..at + 4].copy_from_slice(b"PATH");
    let bad: [Vec<u8>; 4] = [
        vec![0, 0, 0, 2, 1, 0x42],
        vec![0, 0, 0, 2, 1, 0x82],
        (2 * 1024 * 1024u32).to_be_bytes().to_vec(),
        spawn_with_path,
    ];
    for bytes in bad {
        let mut raw = paths::connect_checked(&dir.path).expect("connect");
        raw.write_all(&[hello.clone(), bytes].concat())
            .expect("write");
        let frames = read_frames(&mut raw);
        assert!(matches!(frames.first(), Some(DaemonFrame::HelloAck(_))));
        assert_eq!(frames[1..], *std::slice::from_ref(&bad_frame));
    }

    // Silence instead of Hello, and half a frame followed by silence: both
    // are closed after the (shortened) timeouts.
    let started = Instant::now();
    let mut raw = paths::connect_checked(&dir.path).expect("connect");
    assert_eq!(read_frames(&mut raw), vec![]);
    let mut raw = paths::connect_checked(&dir.path).expect("connect");
    raw.write_all(&[hello.clone(), vec![0, 0, 0, 40, 1]].concat())
        .expect("write");
    let frames = read_frames(&mut raw);
    assert!(matches!(frames[..], [DaemonFrame::HelloAck(_)]));
    assert!(started.elapsed() >= Duration::from_millis(600));

    // A first frame that declares more than a Hello may hold is refused
    // on its four length bytes, not buffered.
    let mut raw = paths::connect_checked(&dir.path).expect("connect");
    raw.write_all(&(1024 * 1024u32).to_be_bytes())
        .expect("write");
    assert_eq!(read_frames(&mut raw), vec![bad_frame.clone()]);

    // Trickling one byte every 50 ms makes every read succeed: the
    // handshake and partial-frame deadlines still apply.
    for greeted in [false, true] {
        let mut raw = paths::connect_checked(&dir.path).expect("connect");
        let mut prefix = if greeted { hello.clone() } else { Vec::new() };
        prefix.extend_from_slice(&[0, 0, 0, 100]);
        raw.write_all(&prefix).expect("write");
        raw.set_read_timeout(Some(Duration::from_millis(50)))
            .expect("timeout");
        let started = Instant::now();
        let mut buf = [0u8; 256];
        let closed = loop {
            if started.elapsed() > Duration::from_secs(3) {
                break false;
            }
            if raw.write_all(&[1]).is_err() {
                break true;
            }
            match raw.read(&mut buf) {
                Ok(0) => break true,
                Ok(_) => {}
                Err(e)
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) => {}
                Err(_) => break true,
            }
        };
        assert!(
            closed,
            "a trickling peer (greeted: {greeted}) outlived the 300 ms deadline"
        );
    }

    // None of that touched the pane or the well-behaved client.
    assert!(process_exists(spawned.pid));
    let info = pane_info(&client, spawned.pane).await.expect("listed");
    assert!(info.alive);
    assert_eq!(client.list_panes().await.expect("list").len(), 1);
}

// SDTEST-2979
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sdtest_2979_backfill_and_applied_resize_keep_every_client_on_the_daemon_grid() {
    let dir = run_dir();
    let _daemon = start(&dir, test_config());
    let client_w = connect(&dir).await;
    let spawned = client_w
        .spawn(sh(
            "read go; i=0; while [ $i -lt 200 ]; do echo \"line $i\"; i=$((i+1)); done; exec cat",
        ))
        .await
        .expect("spawn");
    let pane = spawned.pane;

    // W watches from the start: its history comes from the live stream.
    let mut w = View::attach(&client_w, pane, attach_options(0));
    w.until("the snapshot", |v| v.restores == 1).await;
    client_w.input(pane, b"go\n").expect("input");
    w.until("the output", |v| v.screen().contains("line 199"))
        .await;
    let offset = settled_offset(&client_w, pane).await;
    w.until("the last byte", |v| v.in_sync_at(offset)).await;

    // B attaches afterwards and backfills the history, newest first.
    let client_b = connect(&dir).await;
    let mut b = View::attach(&client_b, pane, attach_options(100_000));
    b.until("the end of the backfill", |v| v.history_done).await;
    // N attaches without history.
    let client_n = connect(&dir).await;
    let mut n = View::attach(&client_n, pane, attach_options(0));
    n.until("the snapshot", |v| v.restores == 1).await;

    let history = |view: &View| -> Vec<String> {
        // Rows above the screen, oldest first, read through the scroll view.
        let mut grid = view.mirror.grid().lock();
        let len = grid.scrollback_len();
        let mut rows = Vec::new();
        for back in (1..=len).rev() {
            grid.set_scroll_offset(back);
            let text: String = grid.visible_rows()[0].iter().map(|c| c.c).collect();
            rows.push(text.trim_end().to_string());
        }
        grid.set_scroll_offset(0);
        rows
    };
    let b_history = history(&b);
    assert_eq!(
        b_history,
        history(&w),
        "backfill differs from the live history"
    );
    assert!(b_history.len() > 170, "{}", b_history.len());
    assert_eq!(b_history.last().map(String::as_str), Some("line 176"));
    assert!(b_history.windows(2).all(|pair| {
        let number = |s: &str| s.strip_prefix("line ").and_then(|n| n.parse::<u32>().ok());
        matches!((number(&pair[0]), number(&pair[1])), (Some(x), Some(y)) if y == x + 1)
            || !pair[0].starts_with("line ")
    }));
    assert_eq!(b.snapshot(), w.snapshot());
    assert!(history(&n).is_empty());

    // W owns the size (first writable attachment). Growing by 6 rows pulls
    // 6 rows back from history on the daemon.
    client_w.resize(pane, 30, 80).expect("resize");
    for view in [&mut w, &mut b, &mut n] {
        view.until("the applied resize", |v| {
            v.mirror.in_sync() && v.mirror.grid().lock().rows == 30
        })
        .await;
    }
    let (daemon_grid, daemon_offset) = daemon_snapshot(&dir, pane).await;
    assert_eq!(daemon_offset, offset);
    for (name, view) in [("W", &w), ("B", &b), ("N", &n)] {
        assert_eq!(view.snapshot(), daemon_grid, "{name}");
    }
    // W and B had the rows and reproduced the resize locally. N had no
    // history: it could not, and resynced from a fresh snapshot instead.
    assert_eq!((w.restores, w.forced_reattaches), (1, 0));
    assert_eq!((b.restores, b.forced_reattaches), (1, 0));
    assert_eq!((n.restores, n.forced_reattaches), (2, 1));
    assert_eq!(n.kept_history, vec![false, false]);
    assert_eq!(history(&b), history(&w));
    assert_eq!(history(&b).len(), b_history.len() - 6);

    // A refresh that missed no byte keeps the local history.
    client_b.reattach(pane, false).expect("refresh");
    b.until("the refresh", |v| v.restores == 2).await;
    assert_eq!(b.kept_history, vec![false, true]);
    assert_eq!(history(&b), history(&w));
    assert_eq!(b.snapshot(), daemon_grid);

    // A burst of resize requests is throttled on both sides and every
    // client still ends on the daemon's grid.
    for i in 0..100u16 {
        client_w
            .resize(pane, 20 + i % 15, 60 + i % 40)
            .expect("resize");
    }
    client_w.resize(pane, 22, 70).expect("resize");
    for view in [&mut w, &mut b, &mut n] {
        view.until("the final size", |v| {
            let grid = v.mirror.grid().lock();
            v.mirror.in_sync() && (grid.rows, grid.cols) == (22, 70)
        })
        .await;
    }
    tokio::time::sleep(Duration::from_millis(150)).await;
    let (daemon_grid, daemon_offset) = daemon_snapshot(&dir, pane).await;
    for (name, view) in [("W", &mut w), ("B", &mut b), ("N", &mut n)] {
        view.until("quiescence", |v| v.in_sync_at(daemon_offset))
            .await;
        view.drain();
        assert_eq!(
            view.snapshot(),
            daemon_grid,
            "{name} after the resize burst"
        );
    }
    let info = pane_info(&client_w, pane).await.expect("listed");
    assert_eq!((info.rows, info.cols), (22, 70));
}

// SDTEST-2980
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sdtest_2980_proxy_starts_the_daemon_and_carries_a_full_session_over_stdio() {
    use tokio::process::Command;
    let dir = run_dir();
    let nonce = "00112233445566778899aabbccddeeff";
    let dir_arg = dir.path.to_str().expect("utf-8 path").to_string();

    // Login banner noise before the marker, as on a real SSH exec channel.
    let mut child = Command::new("/bin/sh")
        .arg("-c")
        .arg("echo 'Welcome to host'; printf '\\036SHELLDECK_SESSIOND_bogus_READY\\037'; exec \"$0\" proxy --dir \"$1\" --hello \"$2\"")
        .arg(BIN)
        .arg(&dir_arg)
        .arg(nonce)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .expect("proxy starts");
    let stream = tokio::io::join(
        child.stdout.take().expect("stdout"),
        child.stdin.take().expect("stdin"),
    );
    // No daemon was running: the proxy started one, detached.
    let client = Client::connect_via_proxy(stream, nonce, ClientOptions::default())
        .await
        .expect("handshake through the proxy");
    let daemon_pid: u32 = std::fs::read_to_string(dir.path.join(paths::LOCK_NAME))
        .expect("lock file")
        .trim()
        .parse()
        .expect("daemon pid");
    assert_ne!(daemon_pid, std::process::id());
    let _cleanup = TerminateOnDrop(daemon_pid);
    assert!(process_exists(daemon_pid));

    // Full cycle through the pipe pair: spawn, attach, input, output.
    let spawned = client.spawn(command(&["/bin/cat"])).await.expect("spawn");
    let mut view = View::attach(&client, spawned.pane, attach_options(0));
    view.until("the snapshot", |v| v.restores == 1).await;
    client
        .input(spawned.pane, b"through-the-proxy\n")
        .expect("input");
    view.until("the echo", |v| {
        v.screen().matches("through-the-proxy").count() == 2
    })
    .await;

    // The pane child of a detached daemon keeps default SIGHUP handling:
    // neither ignored nor blocked.
    let status =
        std::fs::read_to_string(format!("/proc/{}/status", spawned.pid)).unwrap_or_default();
    for field in ["SigIgn:", "SigBlk:"] {
        if let Some(mask) = status
            .lines()
            .find_map(|line| line.strip_prefix(field))
            .and_then(|hex| u64::from_str_radix(hex.trim(), 16).ok())
        {
            assert_eq!(mask & 1, 0, "{field} must not contain SIGHUP");
        }
    }

    // EOF on the proxy's stdin: the proxy exits, the daemon and pane stay.
    drop(view);
    drop(client);
    let status = tokio::time::timeout(STEP, child.wait())
        .await
        .expect("proxy exits on stdin EOF")
        .expect("wait");
    assert!(status.success());
    assert!(process_exists(daemon_pid));
    assert!(process_exists(spawned.pid));

    // A second proxy reaches the same daemon and the same pane.
    let mut child = Command::new(BIN)
        .args(["proxy", "--dir", &dir_arg, "--hello", nonce])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .expect("proxy starts");
    let stream = tokio::io::join(
        child.stdout.take().expect("stdout"),
        child.stdin.take().expect("stdin"),
    );
    let client = Client::connect_via_proxy(stream, nonce, ClientOptions::default())
        .await
        .expect("handshake");
    let mut view = View::attach(&client, spawned.pane, attach_options(0));
    view.until("the snapshot", |v| v.restores == 1).await;
    assert_eq!(view.screen().matches("through-the-proxy").count(), 2);

    // A missing or oversize prelude fails within its caps.
    let prelude = |script: &'static str, timeout: Duration| async move {
        let mut child = Command::new("/bin/sh")
            .arg("-c")
            .arg(script)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .expect("sh");
        let stream = tokio::io::join(
            child.stdout.take().expect("stdout"),
            child.stdin.take().expect("stdin"),
        );
        let options = ClientOptions {
            hello_timeout: timeout,
            ..ClientOptions::default()
        };
        Client::connect_via_proxy(stream, nonce, options)
            .await
            .err()
    };
    assert!(matches!(
        prelude("head -c 100000 /dev/zero; sleep 5", STEP).await,
        Some(ClientError::Prelude)
    ));
    assert!(matches!(
        prelude("echo banner; sleep 5", Duration::from_millis(300)).await,
        Some(ClientError::Timeout)
    ));
    assert!(matches!(
        prelude("echo banner", STEP).await,
        Some(ClientError::Closed)
    ));
    assert!(matches!(
        Client::connect_via_proxy(
            tokio::io::duplex(64).0,
            "not-a-nonce",
            ClientOptions::default()
        )
        .await
        .err(),
        Some(ClientError::Prelude)
    ));

    // SIGTERM: the daemon hangs up its panes and leaves nothing behind.
    drop(view);
    drop(client);
    // SAFETY: plain signal to the daemon this test started.
    unsafe { libc::kill(daemon_pid as libc::pid_t, libc::SIGTERM) };
    wait_for("the daemon and its pane to exit", || {
        !process_exists(daemon_pid) && !process_exists(spawned.pid)
    })
    .await;
    assert!(!paths::socket_path(&dir.path).exists());
}

// ---------------------------------------------------------------------------
// Raw connections (tests that need to see every frame, or to not read)
// ---------------------------------------------------------------------------

fn raw_hello(name: &str) -> Vec<u8> {
    ClientFrame::Hello(Hello {
        proto_min: 1,
        proto_max: 1,
        features: 0,
        client_version: name.into(),
    })
    .encode()
    .expect("hello")
}

fn raw_attach(pane: Uuid, attach_id: u32) -> Vec<u8> {
    ClientFrame::Attach(Attach {
        pane,
        attach_id,
        rows: 24,
        cols: 80,
        scrollback_rows: 0,
        flags: 0,
    })
    .encode()
    .expect("attach")
}

/// Read frames until `done` says so. Panics on EOF or after 20 s.
fn read_until(
    stream: &mut std::os::unix::net::UnixStream,
    frames: &mut FrameBuffer,
    mut done: impl FnMut(&DaemonFrame) -> bool,
) -> Vec<DaemonFrame> {
    stream
        .set_read_timeout(Some(Duration::from_secs(20)))
        .expect("timeout");
    let mut seen = Vec::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        while let Some(payload) = frames.next_frame().expect("frame") {
            let frame = DaemonFrame::decode(payload).expect("decode");
            let last = done(&frame);
            seen.push(frame);
            if last {
                return seen;
            }
        }
        let n = stream.read(&mut buf).expect("the daemon keeps writing");
        assert!(n > 0, "connection closed, frames so far: {}", seen.len());
        frames.push(&buf[..n]);
    }
}

// SDTEST-2999
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sdtest_2999_a_client_dropped_for_resync_can_still_interrupt_the_flood() {
    let dir = run_dir();
    let _daemon = start(
        &dir,
        ServerConfig {
            output_queue_cap: 128 * 1024,
            ..test_config()
        },
    );
    let client = connect(&dir).await;
    // An endless flood, started by the client that will then stop reading.
    let spawned = client
        .spawn(sh("read go; exec yes flood"))
        .await
        .expect("spawn");
    let pane = spawned.pane;

    let mut raw = paths::connect_checked(&dir.path).expect("raw connect");
    let go = ClientFrame::Input {
        pane,
        bytes: b"go\n".to_vec(),
    };
    raw.write_all(
        &[
            raw_hello("slow"),
            raw_attach(pane, 41),
            go.encode().expect("input"),
        ]
        .concat(),
    )
    .expect("write");

    // Nothing is read: the attachment overflows and is dropped. A dropped
    // attachment gets no output and is not counted as a viewer.
    let deadline = Instant::now() + STEP;
    loop {
        let info = pane_info(&client, pane).await.expect("listed");
        if info.offset > 1024 * 1024 && info.clients == 0 {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the slow client was never dropped"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // Ctrl-C from the dropped client, before any new Attach: the key must
    // reach the PTY, or a flooding pane could never be interrupted.
    let interrupt = ClientFrame::Input {
        pane,
        bytes: vec![0x03],
    };
    raw.write_all(&interrupt.encode().expect("input"))
        .expect("write");
    let deadline = Instant::now() + STEP;
    loop {
        if !pane_info(&client, pane).await.expect("listed").alive {
            break;
        }
        assert!(Instant::now() < deadline, "Ctrl-C never reached the pane");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(!process_exists(spawned.pid));

    // The connection saw Error(Resync) for its attach and no refusal of
    // its input; a new Attach then gets the final screen and the exit.
    raw.write_all(&raw_attach(pane, 42)).expect("write");
    let mut frames = FrameBuffer::new();
    let seen = tokio::task::spawn_blocking(move || {
        read_until(&mut raw, &mut frames, |frame| {
            matches!(frame, DaemonFrame::Exited { .. })
        })
    })
    .await
    .expect("join");
    let errors: Vec<(u32, ErrorCode)> = seen
        .iter()
        .filter_map(|frame| match frame {
            DaemonFrame::Error {
                request_id, code, ..
            } => Some((*request_id, *code)),
            _ => None,
        })
        .collect();
    assert_eq!(errors, vec![(41, ErrorCode::Resync)]);
    assert!(seen
        .iter()
        .any(|frame| matches!(frame, DaemonFrame::Ready { attach_id: 42, .. })));
    assert!(matches!(
        seen.last(),
        Some(DaemonFrame::Exited { signal: 2, .. })
    ));
}

// SDTEST-3000
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sdtest_3000_removing_a_dead_pane_still_delivers_its_end_to_other_viewers() {
    let dir = run_dir();
    let _daemon = start(&dir, test_config());
    let client = connect(&dir).await;
    let spawned = client
        .spawn(sh("read go; printf 'last words'; exit 3"))
        .await
        .expect("spawn");
    let pane = spawned.pane;

    // A second viewer that has not read anything yet when the first one
    // sees the exit and closes the pane.
    let mut other = paths::connect_checked(&dir.path).expect("raw connect");
    other
        .write_all(&[raw_hello("other"), raw_attach(pane, 7)].concat())
        .expect("write");
    let mut view = View::attach(&client, pane, attach_options(0));
    view.until("the snapshot", |v| v.restores == 1).await;
    while pane_info(&client, pane).await.expect("listed").clients != 2 {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    client.input(pane, b"go\n").expect("input");
    view.until("the exit", |v| v.exited.is_some()).await;
    client.kill(pane, SIGNAL_TERM).expect("remove");
    while pane_info(&client, pane).await.is_some() {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    // The remover is told as well, after its own copy of the end.
    view.until("the removal notice", |v| !v.errors.is_empty())
        .await;
    assert_eq!(view.errors, vec![ErrorCode::NoSuchPane]);

    // The other viewer still gets the final output, then Exited, then
    // Error(NoSuchPane) for its attach as the last frame of the pane.
    let mut frames = FrameBuffer::new();
    let seen = tokio::task::spawn_blocking(move || {
        read_until(&mut other, &mut frames, |frame| {
            matches!(frame, DaemonFrame::Error { .. })
        })
    })
    .await
    .expect("join");
    let output: Vec<u8> = seen
        .iter()
        .filter_map(|frame| match frame {
            DaemonFrame::Output { bytes, .. } => Some(bytes.clone()),
            _ => None,
        })
        .flatten()
        .collect();
    assert!(String::from_utf8_lossy(&output).contains("last words"));
    let tail = &seen[seen.len() - 2..];
    assert!(
        matches!(tail[0], DaemonFrame::Exited { exit_code: 3, .. }),
        "{tail:?}"
    );
    assert_eq!(
        tail[1],
        DaemonFrame::Error {
            request_id: 7,
            pane,
            code: ErrorCode::NoSuchPane,
            retryable: false,
        }
    );
}

// SDTEST-3054 (SDUC-574): a pane ended on request leaves nothing behind.
// The client that asked goes away at once (a sign-out, an app quit) and
// never removes the dead pane: the daemon drops it as soon as it has
// exited, so it does not stay listed, and does not keep the daemon from
// going idle, for the dead pane period. A pane that ended by itself is
// still kept, for a later reattach to show how it ended.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sdtest_3054_a_pane_ended_on_request_is_removed_when_it_exits() {
    let dir = run_dir();
    let _daemon = start(&dir, test_config());
    let client = connect(&dir).await;
    let by_itself = client.spawn(sh("exit 4")).await.expect("spawn").pane;
    let on_request = client.spawn(sh("sleep 1000")).await.expect("spawn").pane;
    let ignores = client
        .spawn(sh("trap '' HUP; echo ready; sleep 1000"))
        .await
        .expect("spawn")
        .pane;
    // The trap must be in place before the hangup is sent.
    let deadline = Instant::now() + STEP;
    while pane_info(&client, ignores).await.expect("listed").offset == 0 {
        assert!(Instant::now() < deadline, "the shell never started");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    let asking = connect(&dir).await;
    asking
        .kill(on_request, shelldeck_sessiond::protocol::SIGNAL_HUP)
        .expect("kill");
    asking
        .kill(ignores, shelldeck_sessiond::protocol::SIGNAL_HUP)
        .expect("kill");
    drop(asking);

    let deadline = Instant::now() + STEP;
    while pane_info(&client, on_request).await.is_some() {
        assert!(
            Instant::now() < deadline,
            "the pane ended on request is still listed"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    // The hangup was ignored: still running, and not forgotten. It goes
    // the same way once something does end it.
    assert!(pane_info(&client, ignores).await.expect("listed").alive);
    client.kill(ignores, SIGNAL_KILL).expect("kill");
    let deadline = Instant::now() + STEP;
    while pane_info(&client, ignores).await.is_some() {
        assert!(Instant::now() < deadline, "the killed pane is still listed");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let deadline = Instant::now() + STEP;
    let kept = loop {
        let info = pane_info(&client, by_itself).await.expect("still listed");
        if !info.alive {
            break info;
        }
        assert!(Instant::now() < deadline, "the pane never exited");
        tokio::time::sleep(Duration::from_millis(10)).await;
    };
    assert_eq!(kept.exit_code, 4, "kept with how it ended");
}

/// A foreground daemon process started by a test, stopped on drop.
struct DaemonProcess(std::process::Child);

impl Drop for DaemonProcess {
    fn drop(&mut self) {
        // SAFETY: plain signal to a daemon this test started on its own
        // private directory.
        unsafe { libc::kill(self.0.id() as libc::pid_t, libc::SIGTERM) };
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if matches!(self.0.try_wait(), Ok(Some(_))) {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Greeting {
    Served,
    Refused,
    Dropped,
}

/// One raw connection attempt: Hello, then what the daemon did with it.
/// The stream is returned so a served connection can be kept open.
fn greet(dir: &Path) -> (Greeting, std::os::unix::net::UnixStream) {
    let mut stream = paths::connect_checked(dir).expect("connect");
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .expect("timeout");
    // A daemon that closes at once may reset the write.
    let _ = stream.write_all(&raw_hello("fd"));
    let mut frames = FrameBuffer::new();
    let mut buf = [0u8; 4096];
    let greeting = loop {
        if let Some(payload) = frames.next_frame().expect("frame") {
            break match DaemonFrame::decode(payload).expect("decode") {
                DaemonFrame::HelloAck(_) => Greeting::Served,
                DaemonFrame::Error {
                    code: ErrorCode::LimitExceeded,
                    ..
                } => Greeting::Refused,
                other => panic!("unexpected frame {other:?}"),
            };
        }
        match stream.read(&mut buf) {
            Ok(0) => break Greeting::Dropped,
            Ok(n) => frames.push(&buf[..n]),
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                panic!("the daemon left a connection waiting: {e}")
            }
            Err(_) => break Greeting::Dropped,
        }
    };
    (greeting, stream)
}

// SDTEST-3001
#[test]
fn sdtest_3001_descriptor_exhaustion_refuses_at_once_and_leaks_no_connection_slot() {
    // The real binary under a small descriptor limit: stdio, lock,
    // listener, spare, log and a few connections, far below the 32
    // connection slots. A connection takes two descriptors. With 13 the
    // last one that fits uses them all and the next `accept` fails; with
    // 12 one descriptor is left, so the next connection is accepted and
    // then has none for its writer. Both must be told `LimitExceeded`.
    //
    // macOS differs for the first case, and the daemon cannot change it:
    // when `accept` has no descriptor to give, the kernel closes the
    // pending connection instead of leaving it queued (first seen when
    // this test ran there: every attempt ended as `Dropped`, at once).
    // The client then sees a closed connection, not the words. What the
    // daemon owes on every platform still holds and is still checked: an
    // answer at once, no spinning accept loop, no connection slot lost.
    for limit in [13, 12] {
        descriptor_exhaustion(limit);
    }
}

fn descriptor_exhaustion(limit: u32) {
    let dir = run_dir();
    let child = std::process::Command::new("/bin/sh")
        .arg("-c")
        .arg(format!("ulimit -n {limit}; exec \"$0\" serve --dir \"$1\""))
        .arg(BIN)
        .arg(&dir.path)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("daemon starts");
    let _daemon = DaemonProcess(child);
    let deadline = Instant::now() + Duration::from_secs(10);
    while paths::connect_checked(&dir.path).is_err() {
        assert!(Instant::now() < deadline, "the daemon never listened");
        std::thread::sleep(Duration::from_millis(20));
    }

    // Fill it up: some connections are served, then descriptors run out.
    // The probe above was a connection too: until the daemon has let go
    // of it, two descriptors are missing and one client fewer fits (seen
    // once in nine runs). Count again until the expected number fits.
    let expected = 3 - usize::from(limit == 12);
    let deadline = Instant::now() + Duration::from_secs(10);
    let held = loop {
        let mut held = Vec::new();
        loop {
            let (greeting, stream) = greet(&dir.path);
            if greeting != Greeting::Served {
                break;
            }
            held.push(stream);
            assert!(held.len() < 12, "the descriptor limit did not apply");
        }
        if held.len() >= expected || cfg!(target_os = "macos") || Instant::now() >= deadline {
            break held;
        }
        drop(held);
        std::thread::sleep(Duration::from_millis(50));
    };
    let capacity = held.len();
    if cfg!(target_os = "macos") {
        // The daemon's own descriptors are not counted here for macOS:
        // only that the limit applied and left room for a connection.
        assert!((1..=3).contains(&capacity), "limit {limit}: {capacity}");
    } else {
        assert_eq!(capacity, expected, "limit {limit}");
    }

    // Many more attempts than there are slots. Each one is answered at
    // once (never left in the backlog), and none of them may cost a slot.
    let mut refused = 0;
    for _ in 0..40 {
        let started = Instant::now();
        let (greeting, _stream) = greet(&dir.path);
        // Refused in words, whichever way the descriptors ran out. On
        // macOS the kernel may have closed the connection before the
        // daemon could accept it.
        assert!(
            greeting == Greeting::Refused
                || (cfg!(target_os = "macos") && greeting == Greeting::Dropped),
            "limit {limit}: {greeting:?}"
        );
        assert!(started.elapsed() < Duration::from_secs(2));
        refused += usize::from(greeting == Greeting::Refused);
    }
    assert!(
        refused > 0 || cfg!(target_os = "macos"),
        "no attempt was told LimitExceeded"
    );

    // With the descriptors back, the same number of clients is served
    // again: no slot was leaked by the 40 failures.
    drop(held);
    let deadline = Instant::now() + Duration::from_secs(10);
    let served = loop {
        let attempts: Vec<_> = (0..capacity).map(|_| greet(&dir.path)).collect();
        let served = attempts
            .iter()
            .filter(|(greeting, _)| *greeting == Greeting::Served)
            .count();
        if served == capacity || Instant::now() >= deadline {
            break served;
        }
        drop(attempts);
        std::thread::sleep(Duration::from_millis(50));
    };
    assert_eq!(served, capacity);
}

// SDTEST-3002
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sdtest_3002_a_daemon_under_another_host_name_is_found() {
    let root = run_dir();
    std::fs::create_dir(&root.path).expect("run root");
    std::fs::set_permissions(&root.path, std::fs::Permissions::from_mode(0o700)).expect("chmod");
    let dir_for = |name: &str| RunDir {
        _temp: tempfile::tempdir().expect("unused"),
        path: root.path.join(name),
    };
    // The daemon was started when the machine was called "laptop".
    let old = dir_for("laptop");
    let daemon = start(&old, test_config());
    // A directory sorted before it holds a socket nobody serves (another
    // machine on a shared home, or a crash): never picked.
    let stale = root.path.join("a-other-machine");
    std::fs::create_dir(&stale).expect("mkdir");
    std::fs::set_permissions(&stale, std::fs::Permissions::from_mode(0o700)).expect("chmod");
    drop(std::os::unix::net::UnixListener::bind(paths::socket_path(&stale)).expect("stale socket"));

    // The host name changed: the directory for the new name is empty.
    let renamed = root.path.join("laptop.lan");
    assert_eq!(paths::live_run_dir(&root.path, renamed.clone()), old.path);
    let client = connect_unix(
        &paths::live_run_dir(&root.path, renamed.clone()),
        ClientOptions::default(),
    )
    .await
    .expect("the running daemon is reached");
    drop(client);

    // The same when the name for this machine does not fit a socket
    // address any more and the choice would be the last resort outside
    // the home directory (an update that made the name longer): the
    // running daemon is not abandoned.
    let outside = tempfile::tempdir().expect("outside");
    let last_resort = outside.path().join("shelldeck-uid");
    assert_eq!(
        paths::live_run_dir(&root.path, last_resort.clone()),
        old.path
    );

    // A daemon under the current name always wins.
    let current = dir_for("laptop.lan");
    let second = start(&current, test_config());
    assert_eq!(paths::live_run_dir(&root.path, renamed.clone()), renamed);
    drop(second);

    // With no daemon anywhere the current name is used.
    assert_eq!(daemon.stop(), ServeOutcome::Shutdown);
    assert_eq!(paths::live_run_dir(&root.path, renamed.clone()), renamed);
    assert_eq!(
        paths::live_run_dir(&root.path, last_resort.clone()),
        last_resort
    );
}

// SDTEST-3003
#[test]
fn sdtest_3003_a_start_during_the_old_daemons_exit_is_repeated() {
    let dir = run_dir();
    // A daemon on its way out: the socket is already unlinked, the
    // instance lock is still held (the window between the two is the grace
    // period when panes are alive).
    let leaving = start(&dir, test_config());
    std::fs::remove_file(paths::socket_path(&dir.path)).expect("unlink");
    let stopper = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(400));
        leaving.stop()
    });

    // The first `serve --detach` finds the lock taken and does nothing.
    // The start is repeated until the old daemon is gone.
    let started = Instant::now();
    let stream = shelldeck_sessiond::proxy::connect_or_start(&dir.path, true, Path::new(BIN));
    let daemon_pid: Option<u32> = std::fs::read_to_string(dir.path.join(paths::LOCK_NAME))
        .ok()
        .and_then(|text| text.trim().parse().ok())
        .filter(|pid| *pid != std::process::id());
    let _cleanup = daemon_pid.map(TerminateOnDrop);
    assert_eq!(stopper.join().expect("join"), ServeOutcome::Shutdown);
    let mut stream = stream.expect("the new daemon is reached");
    assert!(started.elapsed() < Duration::from_secs(3));
    let pid = daemon_pid.expect("a daemon process holds the lock");

    // It is a working daemon.
    stream.write_all(&raw_hello("retry")).expect("write");
    let mut frames = FrameBuffer::new();
    let seen = read_until(&mut stream, &mut frames, |_| true);
    assert!(matches!(seen[..], [DaemonFrame::HelloAck(_)]));
    drop(stream);

    // SAFETY: the daemon this test started on its private directory.
    unsafe { libc::kill(pid as libc::pid_t, libc::SIGTERM) };
    let deadline = Instant::now() + Duration::from_secs(10);
    while process_exists(pid) {
        assert!(Instant::now() < deadline, "the daemon did not stop");
        std::thread::sleep(Duration::from_millis(20));
    }
}

// ---------------------------------------------------------------------------
// Hardening: input backpressure, size ownership, shutdown, panic isolation,
// descriptor limit, log, idle exit, lock handoff, history budget
// ---------------------------------------------------------------------------

/// A real daemon in the foreground on `dir`, with extra `serve` options.
fn serve_process(dir: &RunDir, shell_prefix: &str, options: &[&str]) -> DaemonProcess {
    let script = format!("{shell_prefix}dir=$1; shift; exec \"$0\" serve --dir \"$dir\" \"$@\"");
    let home = dir.path.parent().expect("parent");
    let child = std::process::Command::new("/bin/sh")
        .arg("-c")
        .arg(script)
        .arg(BIN)
        .arg(&dir.path)
        .args(options)
        // The login shell of a pane must not run the user's own profile.
        .env("HOME", home)
        .env("SHELL", "/bin/sh")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("daemon starts");
    let daemon = DaemonProcess(child);
    let deadline = Instant::now() + Duration::from_secs(10);
    while paths::connect_checked(&dir.path).is_err() {
        assert!(Instant::now() < deadline, "the daemon never listened");
        std::thread::sleep(Duration::from_millis(10));
    }
    daemon
}

// SDTEST-2981 (SDUC-574): a pane whose program does not read its terminal
// holds back only the connection that pastes into it. After the stall
// timeout the queued input is discarded and that connection is told
// `Busy`; every other connection and pane is served meanwhile.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sdtest_2981_stalled_input_blocks_only_the_pasting_connection_and_is_discarded() {
    let dir = run_dir();
    let _daemon = start(
        &dir,
        ServerConfig {
            input_queue_cap: 128 * 1024,
            input_stall: Duration::from_millis(800),
            ..test_config()
        },
    );
    let pasting = connect(&dir).await;
    let other = connect(&dir).await;

    // Raw mode: in line mode the kernel drops input beyond one line
    // instead of holding the writer back.
    let deaf = pasting
        .spawn(sh("stty raw -echo; echo deaf-ready; exec sleep 600"))
        .await
        .expect("spawn");
    let echo = other.spawn(command(&["/bin/cat"])).await.expect("spawn");
    let mut deaf_view = View::attach(&pasting, deaf.pane, attach_options(0));
    deaf_view
        .until("the deaf pane", |v| v.screen().contains("deaf-ready"))
        .await;
    let mut echo_view = View::attach(&other, echo.pane, attach_options(0));
    echo_view.until("the echo pane", |v| v.restores == 1).await;

    // 400 KiB into a terminal nobody reads: the kernel takes a few tens of
    // KiB, the queue 128 KiB, the rest waits in the connection.
    let chunk = vec![b'x'; 16 * 1024];
    for _ in 0..25 {
        pasting.input(deaf.pane, &chunk).expect("input queued");
    }
    // A request behind the paste on the same connection waits with it.
    let behind = {
        let pasting = pasting.clone();
        tokio::spawn(async move { pasting.list_panes().await })
    };

    // The other connection is served at once, and so is its pane.
    assert_eq!(other.list_panes().await.expect("list").len(), 2);
    other.input(echo.pane, b"still-served\n").expect("input");
    echo_view
        .until("the echo of the other pane", |v| {
            v.screen().matches("still-served").count() == 2
        })
        .await;
    assert!(
        !behind.is_finished(),
        "the pasting connection was not held back by its own paste"
    );

    // The stall timeout discards the queue and says so, for that pane.
    deaf_view
        .until("Busy for the stalled pane", |v| {
            v.errors.contains(&ErrorCode::Busy)
        })
        .await;
    assert!(deaf_view.errors.iter().all(|code| *code == ErrorCode::Busy));
    assert!(echo_view.errors.is_empty());
    // Once the paste is through (queued or discarded), the connection
    // answers again.
    let listed = tokio::time::timeout(STEP, behind)
        .await
        .expect("the pasting connection answers again")
        .expect("join")
        .expect("list");
    assert_eq!(listed.len(), 2);

    // The pane itself was never harmed, and it can still be ended: its
    // writer is not stuck inside a write to the full terminal.
    let info = pane_info(&other, deaf.pane).await.expect("listed");
    assert!(info.alive);
    assert!(process_exists(deaf.pid));
    pasting.kill(deaf.pane, SIGNAL_KILL).expect("kill");
    deaf_view.until("the exit", |v| v.exited.is_some()).await;
    assert_eq!(deaf_view.exited.map(|(_, _, signal)| signal), Some(9));
    pasting.kill(deaf.pane, SIGNAL_TERM).expect("remove");
    let deadline = Instant::now() + STEP;
    while pane_info(&other, deaf.pane).await.is_some() {
        assert!(Instant::now() < deadline, "the pane was not removed");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    // The echo pane went through all of it untouched.
    other.input(echo.pane, b"after-all\n").expect("input");
    echo_view
        .until("the echo after the stall", |v| {
            v.screen().matches("after-all").count() == 2
        })
        .await;
}

// SDTEST-2982 (SDUC-574): the pane has the size of the client that typed
// last, and a client that is typing keeps it for one second: another
// client's key does not flip the size back and forth.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sdtest_2982_size_follows_the_last_active_client_with_hysteresis() {
    let dir = run_dir();
    let _daemon = start(&dir, test_config());
    let small = connect(&dir).await;
    let large = connect(&dir).await;
    let watcher = connect(&dir).await;
    let pane = small
        .spawn(command(&["/bin/cat"]))
        .await
        .expect("spawn")
        .pane;
    let size = |client: Client| async move {
        let info = pane_info(&client, pane).await.expect("listed");
        (info.rows, info.cols)
    };
    let wait_size = |wanted: (u16, u16)| {
        let watcher = watcher.clone();
        async move {
            let deadline = Instant::now() + STEP;
            loop {
                let info = pane_info(&watcher, pane).await.expect("listed");
                if (info.rows, info.cols) == wanted {
                    return;
                }
                assert!(
                    Instant::now() < deadline,
                    "the pane is {}x{}, expected {wanted:?}",
                    info.rows,
                    info.cols
                );
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }
    };

    // The first attachment owns the size; a second one does not take it
    // by attaching.
    let mut small_view = View::attach(&small, pane, attach_options(0));
    small_view.until("the snapshot", |v| v.restores == 1).await;
    let large_options = AttachOptions {
        rows: 30,
        cols: 100,
        scrollback_rows: 0,
        read_only: false,
    };
    let mut large_view = View::attach(&large, pane, large_options);
    large_view.until("the snapshot", |v| v.restores == 1).await;
    assert_eq!(size(watcher.clone()).await, (24, 80));

    // The owner never typed: the first key of the other client takes it.
    large.input(pane, b"L").expect("input");
    wait_size((30, 100)).await;

    // While that client keeps typing, keys of the first one change
    // nothing: the size stays where it is, for all of 1.5 s.
    let typing = {
        let large = large.clone();
        tokio::spawn(async move {
            for _ in 0..15 {
                large.input(pane, b"l").expect("input");
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        })
    };
    for _ in 0..10 {
        small.input(pane, b"s").expect("input");
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(size(watcher.clone()).await, (30, 100));
    }
    typing.await.expect("typing");
    assert_eq!(size(watcher.clone()).await, (30, 100));

    // More than a second after its last key the owner is no longer
    // active: the next key of the other client takes the size.
    tokio::time::sleep(Duration::from_millis(1_200)).await;
    assert_eq!(
        size(watcher.clone()).await,
        (30, 100),
        "time alone changes nothing"
    );
    small.input(pane, b"S").expect("input");
    wait_size((24, 80)).await;
    // And now that one is the active owner.
    large.input(pane, b"l").expect("input");
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(size(watcher.clone()).await, (24, 80));

    // A resize request of a client that does not own the size is only
    // remembered; one of the owner applies.
    large.resize(pane, 40, 120).expect("resize");
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(size(watcher.clone()).await, (24, 80));
    small.resize(pane, 20, 70).expect("resize");
    wait_size((20, 70)).await;

    // Every applied size reached both clients, in stream order.
    let offset = settled_offset(&watcher, pane).await;
    for view in [&mut small_view, &mut large_view] {
        view.until("the final size and offset", |v| {
            let grid = v.mirror.grid().lock();
            (grid.rows, grid.cols) == (20, 70) && v.in_sync_at(offset)
        })
        .await;
    }
    assert_eq!(small_view.snapshot(), large_view.snapshot());
}

// SDTEST-2983 (SDUC-574): `SIGTERM` to the real daemon. A pane whose
// program ignores the hangup lives through the grace period and is then
// killed with its whole process group; the socket is gone at once and the
// instance lock is handed over before the grace period, so a new daemon
// serves the directory while the old one is still ending its panes.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sdtest_2983_sigterm_kills_a_pane_that_ignores_the_hangup_after_the_grace_period() {
    let dir = run_dir();
    let grace = Duration::from_millis(2_500);
    let mut old = serve_process(&dir, "", &["--term-grace-ms", "2500"]);
    let old_pid = old.0.id();
    let client = connect(&dir).await;
    let stubborn = client
        .spawn(sh("trap '' HUP; echo armed; while :; do sleep 1; done"))
        .await
        .expect("spawn");
    let polite = client.spawn(command(&["/bin/cat"])).await.expect("spawn");
    let mut view = View::attach(&client, stubborn.pane, attach_options(0));
    view.until("the trap", |v| v.screen().contains("armed"))
        .await;
    let group_exists = |pid: u32| {
        // SAFETY: signal 0 to a process group only checks for existence.
        unsafe { libc::kill(-(pid as libc::pid_t), 0) == 0 }
    };
    assert!(group_exists(stubborn.pid) && group_exists(polite.pid));

    // SAFETY: the daemon this test started on its private directory.
    unsafe { libc::kill(old_pid as libc::pid_t, libc::SIGTERM) };
    let terminated = Instant::now();

    // The socket goes first; the ordinary pane dies of the hangup.
    wait_for("the socket to be unlinked", || {
        !paths::socket_path(&dir.path).exists()
    })
    .await;
    wait_for("the ordinary pane to die of the hangup", || {
        !group_exists(polite.pid)
    })
    .await;
    // The stubborn one is still there, and so is the daemon, waiting.
    assert!(
        terminated.elapsed() < grace,
        "too slow to observe the grace"
    );
    assert!(group_exists(stubborn.pid), "killed before the grace period");
    assert!(matches!(old.0.try_wait(), Ok(None)));

    // A new daemon takes the directory over right now.
    let new = serve_process(&dir, "", &[]);
    let new_client = connect(&dir).await;
    assert!(new_client.list_panes().await.expect("list").is_empty());
    let holder: u32 = std::fs::read_to_string(dir.path.join(paths::LOCK_NAME))
        .expect("lock file")
        .trim()
        .parse()
        .expect("pid");
    assert_eq!(holder, new.0.id());
    assert!(
        matches!(old.0.try_wait(), Ok(None)) && terminated.elapsed() < grace,
        "the new daemon had to wait for the old one"
    );

    // After the grace period the group is killed and the old daemon ends.
    let status = tokio::task::spawn_blocking(move || {
        let status = old.0.wait();
        (status, old)
    })
    .await
    .expect("join");
    assert!(status.0.expect("wait").success());
    assert!(
        terminated.elapsed() >= grace - Duration::from_millis(100),
        "the daemon did not wait for the grace period"
    );
    wait_for("the stubborn group to be killed", || {
        !group_exists(stubborn.pid)
    })
    .await;

    // The old daemon's exit did not touch what the new one owns.
    assert!(paths::socket_path(&dir.path).exists());
    let spawned = new_client
        .spawn(command(&["/bin/cat"]))
        .await
        .expect("the new daemon works");
    assert!(process_exists(spawned.pid));
    assert!(view.errors.is_empty());
}

// SDTEST-2984 (SDUC-574): a panic while parsing one pane's output marks
// that pane dead (exit code -1, signal 0) and hangs up its program. The
// other panes, the connections and the daemon go on.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sdtest_2984_a_parser_panic_ends_only_its_pane() {
    let dir = run_dir();
    let daemon = start(
        &dir,
        ServerConfig {
            fault_parser_panic: Some(b"SDSD-PARSER-FAULT".to_vec()),
            ..test_config()
        },
    );
    let client = connect(&dir).await;
    // The marker is assembled by printf, so neither the command line nor
    // the echo of the typed line contains it.
    let faulty = client
        .spawn(sh(
            "echo waiting; read x; printf 'SDSD-PARSER-%s\\n' FAULT; exec sleep 600",
        ))
        .await
        .expect("spawn");
    let healthy = client.spawn(command(&["/bin/cat"])).await.expect("spawn");
    let mut faulty_view = View::attach(&client, faulty.pane, attach_options(0));
    faulty_view
        .until("the faulty pane", |v| v.screen().contains("waiting"))
        .await;
    let mut healthy_view = View::attach(&client, healthy.pane, attach_options(0));
    healthy_view
        .until("the healthy pane", |v| v.restores == 1)
        .await;
    // A second connection watching the faulty pane.
    let other = connect(&dir).await;
    let mut other_view = View::attach(&other, faulty.pane, attach_options(0));
    other_view.until("the snapshot", |v| v.restores == 1).await;

    client.input(faulty.pane, b"go\n").expect("input");
    for view in [&mut faulty_view, &mut other_view] {
        view.until("the pane to be reported dead", |v| v.exited.is_some())
            .await;
        let (_, exit_code, signal) = view.exited.expect("exited");
        assert_eq!((exit_code, signal), (-1, 0));
        // The chunk that panicked was not forwarded.
        assert!(!view.screen().contains("SDSD-PARSER-FAULT"));
    }
    wait_for("the faulty pane's program to be hung up", || {
        !process_exists(faulty.pid)
    })
    .await;
    let info = pane_info(&client, faulty.pane).await.expect("still listed");
    assert!(!info.alive);
    assert_eq!(info.exit_code, -1);

    // Everything else goes on: the other pane, new panes, new clients.
    assert!(!daemon.is_finished());
    client.input(healthy.pane, b"unharmed\n").expect("input");
    healthy_view
        .until("the echo of the healthy pane", |v| {
            v.screen().matches("unharmed").count() == 2
        })
        .await;
    assert!(process_exists(healthy.pid));
    assert!(healthy_view.errors.is_empty() && healthy_view.exited.is_none());
    let late = connect(&dir).await;
    let another = late.spawn(command(&["/bin/cat"])).await.expect("spawn");
    let mut another_view = View::attach(&late, another.pane, attach_options(0));
    another_view.until("a new pane", |v| v.restores == 1).await;
    assert_eq!(late.list_panes().await.expect("list").len(), 3);

    // The dead pane can be closed like any other.
    client.kill(faulty.pane, SIGNAL_TERM).expect("remove");
    let deadline = Instant::now() + STEP;
    while pane_info(&late, faulty.pane).await.is_some() {
        assert!(Instant::now() < deadline, "the dead pane was not removed");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(daemon.stop(), ServeOutcome::Shutdown);
}

#[cfg(target_os = "linux")]
fn soft_and_hard_nofile(pid: u32) -> (String, String) {
    let limits = std::fs::read_to_string(format!("/proc/{pid}/limits")).expect("limits");
    let line = limits
        .lines()
        .find(|line| line.starts_with("Max open files"))
        .expect("Max open files");
    let mut fields = line.split_whitespace().skip(3);
    (
        fields.next().expect("soft").to_string(),
        fields.next().expect("hard").to_string(),
    )
}

// SDTEST-3028 (SDUC-574): the real daemon raises its own soft descriptor
// limit and starts every pane with the limit it was started with, both
// for a command and for the login shell. A program that does not exist is
// still a spawn error. The daemon writes a private log that holds no
// terminal content.
#[cfg(target_os = "linux")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sdtest_3028_daemon_raises_its_descriptor_limit_and_panes_keep_the_original() {
    let dir = run_dir();
    let daemon = serve_process(&dir, "ulimit -Sn 100; ", &[]);
    let daemon_pid = daemon.0.id();
    let (soft, hard) = soft_and_hard_nofile(daemon_pid);
    let expected = match hard.parse::<u64>() {
        Ok(hard) => hard.min(4096),
        Err(_) => 4096,
    };
    if expected <= 100 {
        eprintln!("SDTEST-3028 skipped: the hard descriptor limit is {hard}");
        return;
    }
    assert_eq!(soft, expected.to_string(), "the daemon's own soft limit");

    let client = connect(&dir).await;
    // A command: the shell builtin reports the limit it runs under.
    let reported = client
        .spawn(sh(
            "echo soft=$(ulimit -Sn) hard=$(ulimit -Hn); exec sleep 600",
        ))
        .await
        .expect("spawn");
    let mut view = View::attach(&client, reported.pane, attach_options(0));
    view.until("the limits", |v| v.screen().contains("soft="))
        .await;
    assert!(
        view.screen().contains(&format!("soft=100 hard={hard}")),
        "pane limits: {}",
        view.screen()
    );
    // The pane process is the program itself (same pid as reported).
    assert_eq!(soft_and_hard_nofile(reported.pid).0, "100");
    // The shell prints the limits and then replaces itself: wait for the
    // `exec`, which may not have happened yet on a busy machine.
    let mut cmdline = Vec::new();
    for _ in 0..100 {
        cmdline = std::fs::read(format!("/proc/{}/cmdline", reported.pid)).expect("cmdline");
        if cmdline.starts_with(b"sleep\0") {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(cmdline.starts_with(b"sleep\0"), "{cmdline:?}");

    // The login shell: argv[0] with the leading dash, same limit.
    let login = client
        .spawn(SpawnOptions {
            argv: Vec::new(),
            ..command(&[])
        })
        .await
        .expect("login shell");
    wait_for("the login shell to replace the launcher", || {
        std::fs::read(format!("/proc/{}/cmdline", login.pid))
            .is_ok_and(|cmdline| cmdline.starts_with(b"-sh\0"))
    })
    .await;
    assert_eq!(soft_and_hard_nofile(login.pid).0, "100");
    // It is a session and group leader with the PTY as its terminal, and
    // it neither ignores nor blocks the hangup. A shell may block signals
    // for a moment while it starts (seen once as a blocked SIGHUP right
    // after the exec, on a loaded machine): what counts is the state it
    // settles in.
    let mask = |name: &str| {
        let status =
            std::fs::read_to_string(format!("/proc/{}/status", login.pid)).expect("status");
        let line = status
            .lines()
            .find(|line| line.starts_with(name))
            .expect("mask");
        u64::from_str_radix(line.split_whitespace().nth(1).expect("value"), 16).expect("hex")
    };
    wait_for("the login shell to leave the hangup unblocked", || {
        mask("SigBlk:") & 1 == 0
    })
    .await;
    assert_eq!(mask("SigIgn:") & 1, 0, "SIGHUP ignored");

    // No such program: refused before a pane exists, as without the
    // launcher.
    for argv in [&["/nonexistent/program"][..], &["echo hi; true"][..]] {
        assert!(matches!(
            client.spawn(command(argv)).await,
            Err(ClientError::Daemon(ErrorCode::SpawnFailed))
        ));
    }
    assert_eq!(client.list_panes().await.expect("list").len(), 2);

    // Something to keep out of the log.
    let secret = client.spawn(command(&["/bin/cat"])).await.expect("spawn");
    let mut secret_view = View::attach(&client, secret.pane, attach_options(0));
    secret_view.until("the snapshot", |v| v.restores == 1).await;
    client
        .input(secret.pane, b"hunter2-typed-secret\n")
        .expect("input");
    secret_view
        .until("the echo", |v| {
            v.screen().matches("hunter2-typed-secret").count() == 2
        })
        .await;
    client.kill(secret.pane, SIGNAL_KILL).expect("kill");
    secret_view.until("the exit", |v| v.exited.is_some()).await;

    let log_path = dir.path.join(paths::LOG_NAME);
    let mut log = String::new();
    let deadline = Instant::now() + STEP;
    while !log.contains("pane ended") {
        assert!(Instant::now() < deadline, "log so far:\n{log}");
        tokio::time::sleep(Duration::from_millis(10)).await;
        log = std::fs::read_to_string(&log_path).unwrap_or_default();
    }
    assert_eq!(
        std::fs::metadata(&log_path)
            .expect("log")
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    assert!(log.contains("session daemon started"), "{log}");
    assert!(log.contains(&format!("pid={daemon_pid}")), "{log}");
    assert!(log.contains("nofile_before=Some(100)"), "{log}");
    assert_eq!(log.matches("pane spawned").count(), 3, "{log}");
    assert!(log.contains("spawn failed"), "{log}");
    for leaked in [
        "hunter2",
        "soft=100",
        "echo hi",
        "sleep 600",
        "/nonexistent/program",
    ] {
        assert!(!log.contains(leaked), "the log contains {leaked:?}:\n{log}");
    }

    // A second `serve` next to the running one neither serves nor writes.
    let before = log.len();
    let second = std::process::Command::new(BIN)
        .arg("serve")
        .arg("--dir")
        .arg(&dir.path)
        .status()
        .expect("second serve");
    assert!(second.success());
    assert_eq!(
        std::fs::read_to_string(&log_path).expect("log").len(),
        before
    );
}

// SDTEST-3029 (SDUC-574): the daemon side of the idle-exit race. A start
// that finds the lock taken and no socket waits for the lock instead of
// doing nothing, and a client that connects around the moment of an idle
// exit is either served or finds no socket: it is never accepted into a
// backlog that is then closed on it.
#[test]
fn sdtest_3029_idle_exit_and_lock_handoff_lose_no_start_and_no_connection() {
    // 1. Lock taken, socket gone (a daemon on its way out): the new start
    // waits and takes over.
    let dir = run_dir();
    let leaving = start(&dir, test_config());
    std::fs::remove_file(paths::socket_path(&dir.path)).expect("unlink");
    // Without waiting it is "already running", as before.
    let impatient = ServerConfig {
        lock_wait: Duration::ZERO,
        ..test_config()
    };
    assert!(ServerHandle::start(&dir.path, impatient)
        .expect("no error")
        .is_none());
    let stopper = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(300));
        leaving.stop()
    });
    let started = Instant::now();
    let next = ServerHandle::start(&dir.path, test_config())
        .expect("no error")
        .expect("the waiting start takes the directory over");
    assert!(started.elapsed() >= Duration::from_millis(250));
    assert!(started.elapsed() < Duration::from_secs(4));
    assert_eq!(stopper.join().expect("join"), ServeOutcome::Shutdown);
    let (greeting, stream) = greet(&dir.path);
    assert_eq!(greeting, Greeting::Served);
    drop(stream);
    // A socket left under the leaving name by a daemon killed in the
    // middle of its idle exit is cleared by the next start.
    let stale = dir.path.join(paths::SOCKET_LEAVING_NAME);
    std::fs::write(&stale, b"").expect("stale file");
    // Lock taken and a socket present: a daemon serves, the start gives
    // up at once.
    let started = Instant::now();
    assert!(ServerHandle::start(&dir.path, test_config())
        .expect("no error")
        .is_none());
    assert!(started.elapsed() < Duration::from_secs(1));
    assert!(
        stale.exists(),
        "a start that does not get the lock removes nothing"
    );
    assert_eq!(next.stop(), ServeOutcome::Shutdown);
    let after = start(&dir, test_config());
    assert!(!stale.exists());
    assert_eq!(after.stop(), ServeOutcome::Shutdown);

    // 2. The race itself, made certain: a client connects after the
    // daemon's last ordinary accept and before the socket is out of the
    // way. It must be served, and the daemon must stay for it.
    {
        let dir = run_dir();
        let (late_tx, late_rx) = std::sync::mpsc::channel();
        let once = std::sync::atomic::AtomicBool::new(false);
        let path = dir.path.clone();
        let hook = shelldeck_sessiond::server::TestHook(std::sync::Arc::new(move || {
            if !once.swap(true, std::sync::atomic::Ordering::SeqCst) {
                let _ = late_tx.send(paths::connect_checked(&path));
            }
        }));
        let daemon = start(
            &dir,
            ServerConfig {
                idle_exit: Some(Duration::from_millis(60)),
                before_leaving: Some(hook),
                ..test_config()
            },
        );
        let mut stream = late_rx
            .recv_timeout(Duration::from_secs(10))
            .expect("the daemon reached its idle exit")
            .expect("the socket was still there");
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .expect("timeout");
        stream
            .write_all(&raw_hello("last moment"))
            .expect("the connection was reset by the leaving daemon");
        let mut frames = FrameBuffer::new();
        let seen = read_until(&mut stream, &mut frames, |_| true);
        assert!(matches!(seen[..], [DaemonFrame::HelloAck(_)]), "{seen:?}");
        std::thread::sleep(Duration::from_millis(200));
        assert!(!daemon.is_finished(), "the daemon left a served client");
        assert!(
            paths::socket_path(&dir.path).exists(),
            "the socket has its name back"
        );
        assert!(!dir.path.join(paths::SOCKET_LEAVING_NAME).exists());
        // Others can still connect while it stays.
        let (greeting, other) = greet(&dir.path);
        assert_eq!(greeting, Greeting::Served);
        drop(other);
        drop(stream);
        // Alone again, it leaves for good.
        assert_eq!(daemon.join(), ServeOutcome::IdleExit);
        assert!(!paths::socket_path(&dir.path).exists());
    }

    // 3. Connections aimed at the moment of the idle exit, 40 times with
    // a growing delay that sweeps across it (a regression guard for the
    // outcomes, not a proof: it also passes without the step above). The
    // sweep starts at no delay at all: on a loaded machine `start` can
    // return when part of the 60 ms is already gone, and a sweep that
    // began at 40 ms once found the daemon gone every time (macOS CI).
    let mut served = 0;
    let mut absent = 0;
    for round in 0..40u64 {
        let dir = run_dir();
        let idle = ServerConfig {
            idle_exit: Some(Duration::from_millis(60)),
            ..test_config()
        };
        let daemon = start(&dir, idle);
        std::thread::sleep(Duration::from_millis(round * 5));
        match paths::connect_checked(&dir.path) {
            Ok(mut stream) => {
                // Accepted: it must be served, whatever the daemon was
                // about to do.
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .expect("timeout");
                stream
                    .write_all(&raw_hello("late"))
                    .unwrap_or_else(|e| panic!("round {round}: reset after connect: {e}"));
                let mut frames = FrameBuffer::new();
                let seen = read_until(&mut stream, &mut frames, |_| true);
                assert!(
                    matches!(seen[..], [DaemonFrame::HelloAck(_)]),
                    "round {round}: {seen:?}"
                );
                // It holds the daemon up for as long as it stays.
                std::thread::sleep(Duration::from_millis(150));
                assert!(!daemon.is_finished(), "round {round}: left a served client");
                assert!(paths::socket_path(&dir.path).exists());
                drop(stream);
                served += 1;
            }
            Err(paths::PathError::Io(e))
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused
                ) =>
            {
                absent += 1;
            }
            Err(e) => panic!("round {round}: {e}"),
        }
        assert_eq!(daemon.join(), ServeOutcome::IdleExit);
        assert!(!paths::socket_path(&dir.path).exists());
        assert!(!dir.path.join(paths::SOCKET_LEAVING_NAME).exists());
    }
    // The sweep covered both sides of the exit.
    assert!(served > 0 && absent > 0, "served {served}, absent {absent}");
}

/// History rows the daemon holds for `pane`, and its screen, through a
/// short-lived attachment that asks for all of it.
async fn daemon_history(dir: &RunDir, pane: Uuid) -> (usize, String) {
    let client = connect(dir).await;
    let mut view = View::attach(
        &client,
        pane,
        AttachOptions {
            rows: 24,
            cols: 80,
            scrollback_rows: 100_000,
            read_only: true,
        },
    );
    view.mirror = PaneMirror::new(24, 80, 100_000, client.hello().emulator_rev);
    view.until("the whole history", |v| v.history_done).await;
    let rows = view.mirror.grid().lock().scrollback_len();
    (rows, view.screen())
}

// SDTEST-3030 (SDUC-574): the history budget. Over budget, the history of
// a pane nobody looks at is halved first, the newest rows are kept and no
// screen is touched; a budget that cannot be met stops at one row per
// pane instead of looping.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sdtest_3030_history_over_budget_is_trimmed_unattached_panes_first() {
    let row_bytes = 80 * std::mem::size_of::<shelldeck_terminal::grid::Cell>() as u64;
    let lines = "i=0; while [ $i -lt 3000 ]; do echo line-$i; i=$((i+1)); done; echo ALL-DONE; exec sleep 600";
    let gated = format!("echo gate; read x; {lines}");

    let dir = run_dir();
    let _daemon = start(
        &dir,
        ServerConfig {
            // Room for 4000 history rows of 80 columns.
            history_budget: 4000 * row_bytes,
            history_check: Duration::from_millis(20),
            ..test_config()
        },
    );
    let client = connect(&dir).await;
    let unwatched = client.spawn(sh(&gated)).await.expect("spawn");
    let watched = client.spawn(sh(&gated)).await.expect("spawn");

    // One pane fills its history: 3000 rows are within the budget.
    {
        let mut view = View::attach(&client, unwatched.pane, attach_options(0));
        view.until("the gate", |v| v.screen().contains("gate"))
            .await;
        client.input(unwatched.pane, b"\n").expect("input");
        view.until("all lines", |v| v.screen().contains("ALL-DONE"))
            .await;
        client.detach(unwatched.pane).expect("detach");
    }
    tokio::time::sleep(Duration::from_millis(200)).await;
    let (full, screen) = daemon_history(&dir, unwatched.pane).await;
    assert!((2950..=3010).contains(&full), "history rows: {full}");
    assert!(screen.contains("line-2999") && screen.contains("ALL-DONE"));

    // The second pane, watched the whole time, fills up as well: together
    // they are over budget, and the unwatched one pays.
    let mut watched_view = View::attach(&client, watched.pane, attach_options(0));
    watched_view
        .until("the gate", |v| v.screen().contains("gate"))
        .await;
    client.input(watched.pane, b"\n").expect("input");
    watched_view
        .until("all lines", |v| v.screen().contains("ALL-DONE"))
        .await;
    tokio::time::sleep(Duration::from_millis(300)).await;

    let (kept_watched, watched_screen) = daemon_history(&dir, watched.pane).await;
    let (kept_unwatched, unwatched_screen) = daemon_history(&dir, unwatched.pane).await;
    assert_eq!(kept_watched, full, "the watched pane lost history");
    assert!(
        (full / 4 - 2..=full / 4 + 2).contains(&kept_unwatched),
        "halved twice from {full}: {kept_unwatched}"
    );
    assert!(kept_watched + kept_unwatched <= 4000);
    // Screens are intact, and what is left of the history is its newest
    // part: the row just above the screen is still there.
    for screen in [&watched_screen, &unwatched_screen] {
        assert!(screen.contains("line-2999") && screen.contains("ALL-DONE"));
    }
    {
        let client = connect(&dir).await;
        let mut view = View::attach(
            &client,
            unwatched.pane,
            AttachOptions {
                scrollback_rows: 100_000,
                read_only: true,
                ..attach_options(0)
            },
        );
        view.until("the history", |v| v.history_done).await;
        let mut grid = view.mirror.grid().lock();
        let text = |row: &[shelldeck_terminal::grid::Cell]| {
            row.iter().map(|cell| cell.c).collect::<String>()
        };
        let first_on_screen = text(&grid.cells[0]);
        let number: usize = first_on_screen
            .trim()
            .strip_prefix("line-")
            .expect("a numbered line at the top of the screen")
            .parse()
            .expect("number");
        // Scrolled back by n rows, the top visible row is the n-th newest
        // history row.
        grid.set_scroll_offset(1);
        let newest = text(grid.visible_rows()[0]);
        assert_eq!(newest.trim(), format!("line-{}", number - 1));
        grid.set_scroll_offset(kept_unwatched);
        let oldest = text(grid.visible_rows()[0]);
        assert_eq!(
            oldest.trim(),
            format!("line-{}", number - kept_unwatched),
            "the kept rows are the newest ones, without a gap"
        );
    }
    // The lower limit stays: more output does not grow it back.
    assert!(process_exists(unwatched.pid) && process_exists(watched.pid));

    // A budget nothing can meet: every history ends at one row, the
    // screens stay, and the daemon keeps serving instead of spinning.
    let dir = run_dir();
    let _daemon = start(
        &dir,
        ServerConfig {
            history_budget: 0,
            history_check: Duration::from_millis(20),
            ..test_config()
        },
    );
    let client = connect(&dir).await;
    let pane = client.spawn(sh(lines)).await.expect("spawn");
    let mut view = View::attach(&client, pane.pane, attach_options(0));
    view.until("all lines", |v| v.screen().contains("ALL-DONE"))
        .await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let (kept, screen) = daemon_history(&dir, pane.pane).await;
    assert_eq!(kept, 1);
    assert!(screen.contains("line-2999") && screen.contains("ALL-DONE"));
    client.input(pane.pane, b"x").expect("input");
    assert_eq!(client.list_panes().await.expect("list").len(), 1);
}

// SDTEST-3047 (SDUC-574): the daemon binary carries the build marker of
// its release, which is how a client tells a bundled build of its own
// release from a stale one before uploading it to a server, and
// `--version` prints that same line.
#[test]
fn sdtest_3047_the_binary_carries_the_build_marker_of_its_release() {
    let marker = shelldeck_sessiond::BUILD_MARKER;
    assert_eq!(
        marker,
        shelldeck_sessiond::build_marker(env!("CARGO_PKG_VERSION"))
    );
    let binary = std::fs::read(env!("CARGO_BIN_EXE_shelldeck-sessiond")).expect("read binary");
    assert!(
        binary
            .windows(marker.len())
            .any(|window| window == marker.as_bytes()),
        "the build marker is missing from the binary"
    );
    let other = shelldeck_sessiond::build_marker("0.0.0-other");
    assert!(!binary
        .windows(other.len())
        .any(|window| window == other.as_bytes()));
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_shelldeck-sessiond"))
        .arg("--version")
        .output()
        .expect("run --version");
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        format!("shelldeck-sessiond {}\n", env!("CARGO_PKG_VERSION"))
    );
}

/// One pane command of the real binary against a private run directory:
/// exit status, stdout, stderr.
fn pane_cli(dir: &RunDir, args: &[&str]) -> (i32, String, String) {
    let output = tokio::task::block_in_place(|| {
        std::process::Command::new(BIN)
            .arg(args[0])
            .arg("--dir")
            .arg(&dir.path)
            .args(&args[1..])
            .stdin(std::process::Stdio::null())
            .output()
            .expect("run the pane command")
    });
    (
        output.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

/// Poll `read` until the pane shows `needle`.
fn pane_shows(dir: &RunDir, pane: &str, lines: &str, needle: &str) -> String {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let (code, text, err) = pane_cli(dir, &["read", pane, "--lines", lines]);
        assert_eq!(code, 0, "{err}");
        if text.contains(needle) {
            return text;
        }
        assert!(
            Instant::now() < deadline,
            "{needle:?} never showed:\n{text}"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

// SDTEST-3106 (SDUC-578): the pane commands of the shipped binary, the way a
// script or an agent uses them: start a named pane, type into it, read its
// screen and history, wait for a reported status and for its exit status.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sdtest_3106_pane_commands_start_type_read_and_wait_from_the_command_line() {
    let dir = run_dir();
    let _daemon = start(&dir, test_config());
    let client = connect(&dir).await;

    let (code, out, err) = pane_cli(
        &dir,
        &[
            "run", "--name", "helper", "--rows", "10", "--cols", "60", "--", "/bin/sh",
        ],
    );
    assert_eq!(code, 0, "{err}");
    let id = out.trim().to_string();
    let pane: Uuid = id.parse().expect("run prints the pane id");

    let (_, table, _) = pane_cli(&dir, &["ls"]);
    for expected in [id.as_str(), "helper", "running", "10x60"] {
        assert!(table.contains(expected), "{expected:?} missing:\n{table}");
    }
    let (_, json, _) = pane_cli(&dir, &["ls", "--json"]);
    assert!(json.contains(&format!("\"id\":\"{id}\"")), "{json}");
    assert!(json.contains("\"name\":\"helper\""), "{json}");
    assert!(json.contains("\"alive\":true,\"exit_code\":null"), "{json}");

    // Typed by name, read by id prefix. The shell computes the marker, so
    // the echo of the typed command cannot satisfy the check.
    let (code, _, err) = pane_cli(&dir, &["send", "helper", "--", "echo", "marker-$((6*7))"]);
    assert_eq!(code, 0, "{err}");
    pane_shows(&dir, &id[..8], "5", "marker-42");

    // More output than the 10-row screen holds: the history comes back,
    // and --lines bounds what is printed, ending at the last line with text.
    let (code, _, err) = pane_cli(
        &dir,
        &[
            "send",
            "helper",
            "--",
            "for i in $(seq 1 30); do echo line-$i; done",
        ],
    );
    assert_eq!(code, 0, "{err}");
    let all = pane_shows(&dir, "helper", "100", "line-30");
    assert!(all.lines().any(|line| line == "line-1"), "history:\n{all}");
    let (_, tail, _) = pane_cli(&dir, &["read", "helper", "--lines", "3"]);
    assert_eq!(tail.lines().count(), 3, "{tail:?}");
    assert!(
        tail.contains("line-30") && !tail.contains("line-27"),
        "{tail:?}"
    );

    // Typing and reading attach without a size: the pane keeps its own.
    let info = pane_info(&client, pane).await.expect("listed");
    assert_eq!((info.rows, info.cols), (10, 60));

    // `wait --until done` returns when the program reports it (OSC 7501).
    // It sees reports made after it attached, so type once it is attached.
    let mut waiting = std::process::Command::new(BIN)
        .args(["wait", "--dir"])
        .arg(&dir.path)
        .args(["helper", "--until", "done", "--timeout", "30"])
        .stdin(std::process::Stdio::null())
        .spawn()
        .expect("start wait");
    let attached = Instant::now() + Duration::from_secs(10);
    while pane_info(&client, pane).await.expect("listed").clients == 0 {
        assert!(Instant::now() < attached, "wait never attached");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(waiting.try_wait().expect("try_wait").is_none());
    let (code, _, err) = pane_cli(
        &dir,
        &[
            "send",
            "helper",
            "--",
            "printf '\\033]7501;state=done\\033\\\\'",
        ],
    );
    assert_eq!(code, 0, "{err}");
    let status = tokio::task::block_in_place(|| waiting.wait().expect("wait exits"));
    assert_eq!(status.code(), Some(0));

    // A pane that keeps running: the timeout is reported like timeout(1).
    let (code, _, err) = pane_cli(&dir, &["wait", "helper", "--timeout", "1"]);
    assert_eq!(code, 124, "{err}");
    let (code, _, err) = pane_cli(&dir, &["wait", "helper", "--until", "finished"]);
    assert_eq!(code, 1);
    assert!(err.contains("--until takes"), "{err}");

    // The exit status of the pane is the exit status of `wait`, also for a
    // pane that already ended. Nothing can be typed into it any more.
    let mut waiting = std::process::Command::new(BIN)
        .args(["wait", "--dir"])
        .arg(&dir.path)
        .args(["helper", "--timeout", "30"])
        .stdin(std::process::Stdio::null())
        .spawn()
        .expect("start wait");
    let (code, _, err) = pane_cli(&dir, &["send", "helper", "--", "exit", "7"]);
    assert_eq!(code, 0, "{err}");
    let status = tokio::task::block_in_place(|| waiting.wait().expect("wait exits"));
    assert_eq!(status.code(), Some(7));
    let (code, _, _) = pane_cli(&dir, &["wait", "helper"]);
    assert_eq!(code, 7);
    let (code, _, err) = pane_cli(&dir, &["send", "helper", "--", "echo", "late"]);
    assert_eq!(code, 1);
    assert!(err.contains("has exited"), "{err}");

    let (code, _, err) = pane_cli(&dir, &["read", "nobody"]);
    assert_eq!(code, 1);
    assert!(err.contains("no pane matches"), "{err}");
}

// SDTEST-3107 (SDUC-578): `run` starts the daemon when none runs; the other
// pane commands never do, since they act on panes that already exist.
// Linux only: the daemon it starts is detached, and only there does the
// test harness find and stop it (`terminate_daemons_of`).
#[cfg(target_os = "linux")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sdtest_3107_only_run_starts_a_daemon() {
    let dir = run_dir();

    let (code, out, err) = pane_cli(&dir, &["ls"]);
    assert_eq!(code, 1, "{out}");
    assert!(err.contains("no session daemon"), "{err}");
    assert!(!dir.path.exists(), "ls created the run directory");

    let (code, out, err) = pane_cli(&dir, &["run", "--", "/bin/sh", "-c", "sleep 30"]);
    assert_eq!(code, 0, "{err}");
    let id = out.trim().to_string();
    let _: Uuid = id.parse().expect("run prints the pane id");
    let (code, table, err) = pane_cli(&dir, &["ls"]);
    assert_eq!(code, 0, "{err}");
    assert!(table.contains(&id), "{table}");
    // The pane is removed before the directory goes away with its daemon.
    let (code, _, err) = pane_cli(&dir, &["kill", &id, "9"]);
    assert_eq!(code, 0, "{err}");
}
