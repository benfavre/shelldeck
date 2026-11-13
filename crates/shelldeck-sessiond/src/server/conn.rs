//! One client connection: a reader thread that decodes and executes frames
//! and a writer thread that drains the control queue and the attachment
//! FIFOs round-robin (docs/design/session-daemon.md, sections 4 and 5).

use super::pane::{Attachment, InputPush, Pane, SpawnedPty};
use super::{ConnSlot, Shared};
use crate::protocol::{
    ClientFrame, DaemonFrame, ErrorCode, FrameBuffer, HelloAck, Spawn, DEFAULT_SCROLLBACK,
    MAX_FRAME_BYTES, MAX_HELLO_FRAME, MAX_OUTPUT_BYTES, MAX_PANES, PROTOCOL_VERSION,
};
use parking_lot::{Condvar, Mutex};
use shelldeck_terminal::pty::LocalPty;
use std::collections::{HashMap, VecDeque};
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use uuid::Uuid;

/// Replies queued for a client that does not read them: past this the
/// connection is closed.
const MAX_CONTROL_BYTES: usize = 4 * 1024 * 1024;
const READ_TICK: Duration = Duration::from_millis(100);

/// Outbound side of a connection, shared with the panes it is attached to.
pub(super) struct ConnOut {
    pub(super) id: u64,
    control: Mutex<(VecDeque<Vec<u8>>, usize)>,
    attachments: Mutex<Vec<Arc<Attachment>>>,
    wake: Mutex<bool>,
    wake_cv: Condvar,
    /// Flush the control queue, then close.
    closing: AtomicBool,
    closed: AtomicBool,
    bulk_bytes: AtomicUsize,
}

impl ConnOut {
    fn new(id: u64) -> Self {
        Self {
            id,
            control: Mutex::new((VecDeque::new(), 0)),
            attachments: Mutex::new(Vec::new()),
            wake: Mutex::new(false),
            wake_cv: Condvar::new(),
            closing: AtomicBool::new(false),
            closed: AtomicBool::new(false),
            bulk_bytes: AtomicUsize::new(0),
        }
    }

    pub(super) fn notify(&self) {
        *self.wake.lock() = true;
        self.wake_cv.notify_one();
    }

    fn wait(&self, timeout: Duration) {
        let mut woken = self.wake.lock();
        if !*woken {
            self.wake_cv.wait_for(&mut woken, timeout);
        }
        *woken = false;
    }

    pub(super) fn bulk_bytes(&self) -> usize {
        self.bulk_bytes.load(Ordering::Relaxed)
    }

    pub(super) fn reserve_bulk(&self, bytes: usize) {
        self.bulk_bytes.fetch_add(bytes, Ordering::Relaxed);
    }

    pub(super) fn release_bulk(&self, bytes: usize) {
        self.bulk_bytes.fetch_sub(bytes, Ordering::Relaxed);
    }

    fn send(&self, frame: &DaemonFrame) {
        let Ok(bytes) = frame.encode() else {
            tracing::error!("daemon frame failed to encode");
            return;
        };
        let mut control = self.control.lock();
        control.1 += bytes.len();
        control.0.push_back(bytes);
        let overflow = control.1 > MAX_CONTROL_BYTES;
        drop(control);
        if overflow {
            self.closed.store(true, Ordering::Relaxed);
        }
        self.notify();
    }

    fn error(&self, request_id: u32, pane: Uuid, code: ErrorCode, retryable: bool) {
        self.send(&DaemonFrame::Error {
            request_id,
            pane,
            code,
            retryable,
        });
    }

    fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Relaxed)
    }
}

/// Serve one accepted, peer-checked connection until it closes. Closing a
/// connection only detaches: panes are not affected. `slot` is this
/// connection's share of `MAX_CONNECTIONS`: it is released when the writer
/// ends, or on any earlier way out of this function.
pub(super) fn run(shared: Arc<Shared>, stream: UnixStream, slot: ConnSlot) {
    let out = Arc::new(ConnOut::new(shared.next_conn_id()));
    let writer_stream = match stream.try_clone() {
        Ok(clone) => clone,
        // Out of descriptors for the writer's half: say so instead of
        // closing without a word. `slot` is dropped here.
        Err(_) => {
            super::refuse(&stream);
            return;
        }
    };
    let writer = {
        let out = out.clone();
        let shared = shared.clone();
        std::thread::Builder::new()
            .name("sessiond-conn-writer".into())
            .spawn(move || {
                let _slot = slot;
                write_loop(&shared, &out, writer_stream);
            })
    };
    if writer.is_err() {
        return;
    }

    let mut attached: HashMap<Uuid, Arc<Attachment>> = HashMap::new();
    // A panic while executing a frame must still detach and stop the
    // writer, or the slot and the attachments would be held for good.
    let read = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        read_loop(&shared, &out, &stream, &mut attached);
    }));
    if read.is_err() {
        tracing::error!(conn = out.id, "connection reader panicked");
    }

    for (_, attachment) in attached.drain() {
        if let Some(pane) = attachment.pane.upgrade() {
            pane.detach(out.id);
        }
    }
    // Let the writer flush a final Error frame, then stop it.
    out.closing.store(true, Ordering::Relaxed);
    out.notify();
}

fn read_loop(
    shared: &Arc<Shared>,
    out: &Arc<ConnOut>,
    mut stream: &UnixStream,
    attached: &mut HashMap<Uuid, Arc<Attachment>>,
) {
    let config = &shared.config;
    let _ = stream.set_read_timeout(Some(READ_TICK.min(config.hello_timeout)));
    let opened = Instant::now();
    let mut greeted = false;
    let mut partial_since: Option<Instant> = None;
    let mut frames = FrameBuffer::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        if out.is_closed() || shared.is_shutting_down() {
            return;
        }
        // Handshake and partial-frame deadlines, checked on every turn: a
        // peer that trickles bytes makes every read succeed and must not
        // escape them. Both only detach.
        if !greeted && opened.elapsed() >= config.hello_timeout {
            return;
        }
        if partial_since.is_some_and(|at| at.elapsed() >= config.frame_timeout) {
            return;
        }
        match stream.read(&mut buf) {
            Ok(0) => return,
            Ok(n) => frames.push(&buf[..n]),
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock
                        | std::io::ErrorKind::TimedOut
                        | std::io::ErrorKind::Interrupted
                ) =>
            {
                continue;
            }
            Err(_) => return,
        }
        // The first frame is `Hello`: refuse a longer one on its length
        // bytes alone instead of buffering up to 1 MiB of it.
        if !greeted
            && frames
                .declared_len()
                .is_some_and(|len| len > MAX_HELLO_FRAME)
        {
            out.error(0, Uuid::nil(), ErrorCode::BadFrame, false);
            return;
        }
        let mut completed = false;
        loop {
            let frame = match frames.next_frame() {
                Ok(Some(payload)) => ClientFrame::decode(payload),
                Ok(None) => break,
                Err(e) => Err(e),
            };
            let frame = match frame {
                Ok(frame) => frame,
                Err(e) => {
                    // Fail closed: say why when we still can, then close.
                    tracing::debug!(conn = out.id, error = %e, "bad frame");
                    out.error(0, Uuid::nil(), ErrorCode::BadFrame, false);
                    return;
                }
            };
            completed = true;
            if !greeted {
                if !handshake(shared, out, frame) {
                    return;
                }
                greeted = true;
                continue;
            }
            if !handle(shared, out, attached, frame) {
                out.error(0, Uuid::nil(), ErrorCode::BadFrame, false);
                return;
            }
        }
        // The deadline belongs to one frame: it restarts when a frame
        // completed in this turn, so a long paste is not cut short.
        partial_since = match (frames.pending() > 0, partial_since) {
            (false, _) => None,
            (true, Some(at)) if !completed => Some(at),
            (true, _) => Some(Instant::now()),
        };
    }
}

fn handshake(shared: &Shared, out: &ConnOut, frame: ClientFrame) -> bool {
    let ClientFrame::Hello(hello) = frame else {
        out.error(0, Uuid::nil(), ErrorCode::BadFrame, false);
        return false;
    };
    if hello.proto_min > PROTOCOL_VERSION
        || hello.proto_max < PROTOCOL_VERSION
        || hello.proto_min > hello.proto_max
    {
        out.error(0, Uuid::nil(), ErrorCode::UnsupportedVersion, false);
        return false;
    }
    out.send(&DaemonFrame::HelloAck(HelloAck {
        proto: PROTOCOL_VERSION,
        features: crate::protocol::FEATURE_PANE_STATUS,
        daemon_version: shared.config.daemon_version.clone(),
        instance: shared.instance,
        max_frame: MAX_FRAME_BYTES as u32,
        emulator_rev: shelldeck_terminal::EMULATOR_REV,
    }));
    true
}

/// Execute one frame. `false` means a protocol violation: close.
fn handle(
    shared: &Arc<Shared>,
    out: &Arc<ConnOut>,
    attached: &mut HashMap<Uuid, Arc<Attachment>>,
    frame: ClientFrame,
) -> bool {
    // The pane decides whether this connection may write. An attachment
    // dropped for `Resync` (slow client) stays writable: its output is
    // suspended until the next `Attach`, but a key such as Ctrl-C must
    // still reach a pane that is flooding. Attachments of a pane that is
    // gone are forgotten here.
    let live = |attached: &mut HashMap<Uuid, Arc<Attachment>>, pane: &Uuid| {
        let found = attached.get(pane).and_then(|a| a.pane.upgrade());
        if found.is_none() {
            attached.remove(pane);
        }
        found
    };
    match frame {
        ClientFrame::Hello(_) => return false,
        ClientFrame::ListPanes => {
            let panes = shared.list_panes();
            out.send(&DaemonFrame::PaneList(panes));
        }
        ClientFrame::ListStatus => {
            out.send(&DaemonFrame::StatusList(shared.list_status()));
        }
        ClientFrame::Spawn(spawn) => {
            let request_id = spawn.request_id;
            match spawn_pane(shared, spawn) {
                Ok((pane, cwd)) => out.send(&DaemonFrame::Spawned {
                    request_id,
                    pane: pane.id,
                    pid: pane.pid,
                    cwd,
                }),
                Err(code) => out.error(request_id, Uuid::nil(), code, false),
            }
        }
        ClientFrame::Attach(attach) => {
            let Some(pane) = shared.pane(&attach.pane) else {
                out.error(attach.attach_id, attach.pane, ErrorCode::NoSuchPane, false);
                return true;
            };
            match pane.attach(out, &attach) {
                Ok(attachment) => {
                    let known = attached
                        .get(&attach.pane)
                        .is_some_and(|a| Arc::ptr_eq(a, &attachment));
                    if !known {
                        attached.insert(attach.pane, attachment.clone());
                        out.attachments.lock().push(attachment);
                    }
                    out.notify();
                }
                Err((code, retryable)) => out.error(attach.attach_id, attach.pane, code, retryable),
            }
        }
        ClientFrame::Input { pane: id, bytes } => {
            let Some(pane) = live(attached, &id) else {
                out.error(0, id, ErrorCode::NotAttached, false);
                return true;
            };
            match pane.note_input(out.id) {
                Ok(true) => {
                    if pane.input.push_blocking(bytes, shared.config.input_stall)
                        == InputPush::Stalled
                    {
                        out.error(0, id, ErrorCode::Busy, true);
                    }
                }
                Ok(false) => {}
                Err(code) => out.error(0, id, code, false),
            }
        }
        ClientFrame::Resize {
            pane: id,
            rows,
            cols,
        } => match live(attached, &id) {
            Some(pane) => {
                if let Err(code) = pane.resize(out.id, rows, cols) {
                    out.error(0, id, code, false);
                }
            }
            None => out.error(0, id, ErrorCode::NotAttached, false),
        },
        ClientFrame::Detach { pane: id } => match attached.remove(&id) {
            Some(attachment) => {
                if let Some(pane) = attachment.pane.upgrade() {
                    pane.detach(out.id);
                }
            }
            None => out.error(0, id, ErrorCode::NotAttached, false),
        },
        ClientFrame::Kill { pane: id, signal } => {
            let Some(pane) = shared.pane(&id) else {
                out.error(0, id, ErrorCode::NoSuchPane, false);
                return true;
            };
            if pane.is_read_only_for(out.id) {
                out.error(0, id, ErrorCode::ReadOnly, false);
            } else if pane.is_alive() {
                // The three signals a client may send all mean "end this
                // pane". A client that goes away right after (sign-out,
                // app quit) never comes back to remove the dead pane,
                // which would otherwise keep the daemon from going idle
                // for the whole dead pane period.
                pane.discard_at_exit();
                pane.signal_group(signal as i32);
            } else {
                // On a dead pane Kill means "close".
                shared.remove_pane(&id);
                attached.remove(&id);
            }
        }
    }
    true
}

fn spawn_pane(shared: &Arc<Shared>, spawn: Spawn) -> Result<(Arc<Pane>, String), ErrorCode> {
    if shared.pane_count() >= MAX_PANES {
        return Err(ErrorCode::LimitExceeded);
    }
    // The daemon opens the directory itself and reports the canonical path.
    // There is no fallback to another directory.
    let requested = if spawn.cwd.is_empty() {
        crate::paths::home_dir().ok_or(ErrorCode::SpawnFailed)?
    } else {
        std::path::PathBuf::from(&spawn.cwd)
    };
    let cwd = std::fs::canonicalize(&requested).map_err(|_| ErrorCode::SpawnFailed)?;
    if !cwd.is_dir() {
        return Err(ErrorCode::SpawnFailed);
    }
    let cwd_text = cwd.to_str().ok_or(ErrorCode::SpawnFailed)?.to_string();

    // With a raised descriptor limit the pane starts through the daemon
    // executable, which puts the original limit back before the program
    // runs. A program that does not exist is refused here, as it is when
    // the PTY library starts it directly.
    let launched;
    let launcher = shared
        .config
        .pane_launcher
        .as_ref()
        // The executable can be gone (an update removed it): the pane then
        // starts directly and keeps the daemon's limit.
        .filter(|launcher| launcher.exe.exists());
    let argv: &[String] = match launcher {
        Some(launcher) => {
            launched = launcher.command(&spawn.argv, &cwd).ok_or_else(|| {
                tracing::warn!("spawn failed: no such program");
                ErrorCode::SpawnFailed
            })?;
            &launched
        }
        None => &spawn.argv,
    };
    // The pane knows itself: its id, and where the pane commands are
    // (`docs/pane-cli.md`), whatever the client asked for.
    let id = Uuid::new_v4();
    let mut env = spawn.env.clone();
    env.extend(
        shared
            .config
            .pane_identity_env(id, std::env::var_os("PATH")),
    );
    let (pty, _reader) = LocalPty::spawn_command(argv, &env, &cwd, spawn.rows, spawn.cols)
        .map_err(|e| {
            tracing::warn!(error = %e, "spawn failed");
            ErrorCode::SpawnFailed
        })?;
    let (writer, master) = pty.into_parts();
    let pid = master.process_id().unwrap_or(0);
    let scrollback = match spawn.scrollback_max {
        0 => DEFAULT_SCROLLBACK,
        rows => rows,
    } as usize;
    let pane = Pane::start(
        SpawnedPty {
            master,
            writer,
            pid,
            id,
        },
        spawn.group,
        spawn.label,
        spawn.rows,
        spawn.cols,
        scrollback,
        shared.config.clone(),
    )
    .map_err(|_| ErrorCode::SpawnFailed)?;
    if !shared.insert_pane(pane.clone()) {
        pane.close();
        pane.kill_group();
        return Err(ErrorCode::LimitExceeded);
    }
    tracing::info!(pane = %pane.id, pid, rows = spawn.rows, cols = spawn.cols, "pane spawned");
    Ok((pane, cwd_text))
}

fn write_loop(shared: &Shared, out: &ConnOut, mut stream: UnixStream) {
    let config = &shared.config;
    // A socket that accepts no byte for the write timeout is closed.
    let _ = stream.set_write_timeout(Some(config.write_timeout));
    let mut next = 0usize;
    'outer: loop {
        let mut wrote = false;
        loop {
            let frame = {
                let mut control = out.control.lock();
                let frame = control.0.pop_front();
                if let Some(frame) = &frame {
                    control.1 -= frame.len();
                }
                frame
            };
            let Some(frame) = frame else { break };
            if stream.write_all(&frame).is_err() {
                break 'outer;
            }
            wrote = true;
        }
        if out.is_closed() || shared.is_shutting_down() {
            break;
        }

        // Round-robin over the attachments, 64 KiB per turn each, so a
        // flooding pane cannot starve the others on this connection.
        let attachments = out.attachments.lock().clone();
        let count = attachments.len();
        for step in 0..count {
            let attachment = &attachments[(next + step) % count];
            let (frames, finished) = attachment.pop_turn(MAX_OUTPUT_BYTES);
            for frame in &frames {
                if stream.write_all(frame).is_err() {
                    break 'outer;
                }
                wrote = true;
            }
            if finished {
                out.attachments
                    .lock()
                    .retain(|a| !Arc::ptr_eq(a, attachment));
            } else if attachment.wants_backfill(config.backfill_low_water) {
                if let Some(pane) = attachment.pane.upgrade() {
                    pane.backfill_step(out.id);
                    wrote = true;
                }
            }
        }
        if count > 0 {
            next = (next + 1) % count;
        }

        if !wrote {
            if out.closing.load(Ordering::Relaxed) {
                break;
            }
            out.wait(Duration::from_millis(200));
        }
    }
    out.closed.store(true, Ordering::Relaxed);
    let _ = stream.shutdown(std::net::Shutdown::Both);
}
