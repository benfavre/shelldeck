//! One pane: a PTY, its child, a headless grid, a parser and the byte
//! counter, plus the per-attachment FIFOs that carry everything a client
//! receives for the pane in stream order
//! (docs/design/session-daemon.md, sections 5 and 7).

use super::conn::ConnOut;
use super::ServerConfig;
use crate::protocol::{
    self, Attach, DaemonFrame, ErrorCode, PaneInfo, ATTACH_READ_ONLY, FLAG_LAST,
    MAX_ATTACHMENTS_PER_PANE, MAX_OUTPUT_BYTES, MAX_SCROLLBACK_CHUNK, MAX_SNAPSHOT_CHUNK,
    MAX_SNAPSHOT_TOTAL,
};
use parking_lot::{Condvar, Mutex};
use shelldeck_terminal::boundary::StreamBoundary;
use shelldeck_terminal::grid::TerminalGrid;
use shelldeck_terminal::parser::TerminalProcessor;
use shelldeck_terminal::pty::PtyMaster;
use std::collections::VecDeque;
use std::io::{Read, Write};
use std::os::unix::io::{AsRawFd, BorrowedFd, OwnedFd};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Weak};
use std::time::{Duration, Instant};
use uuid::Uuid;

const RESIZE_INTERVAL: Duration = Duration::from_millis(50);
const OWNER_HYSTERESIS: Duration = Duration::from_secs(1);
const READER_TICK_MS: i32 = 20;
const BACKFILL_ROWS: usize = 256;
/// How long the PTY writer waits for room before it looks again at whether
/// its frame was abandoned.
const WRITER_TICK_MS: i32 = 50;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum FrameKind {
    /// Live output: counted against the per-attachment cap.
    Output,
    /// Snapshot and scrollback: counted against the per-connection cap.
    Bulk,
    /// Small ordered markers (`Ready`, applied `Resize`, `Exited`).
    Marker,
}

struct QueuedFrame {
    bytes: Vec<u8>,
    kind: FrameKind,
}

#[derive(Default)]
struct AttachmentQueue {
    frames: VecDeque<QueuedFrame>,
    output_bytes: usize,
    bulk_bytes: usize,
    /// No frame will be added any more (detached, dropped or pane removed).
    closed: bool,
    backfill_pending: bool,
}

/// The FIFO of one (connection, pane) attachment. Frames are pushed under
/// the pane lock, which makes their order equal to stream order, and popped
/// by the connection's writer. Pushing never blocks.
pub(super) struct Attachment {
    pub(super) pane_id: Uuid,
    pub(super) pane: Weak<Pane>,
    conn: Arc<ConnOut>,
    queue: Mutex<AttachmentQueue>,
}

impl Attachment {
    pub(super) fn is_closed(&self) -> bool {
        self.queue.lock().closed
    }

    /// Queue a frame. Returns `false` when the attachment was dropped
    /// because its queued output exceeds the cap (slow client): the queue is
    /// freed and `Error(Resync)` is the last frame the client gets for it.
    fn push(&self, bytes: Vec<u8>, kind: FrameKind, attach_id: u32, cap: usize) -> bool {
        let mut queue = self.queue.lock();
        if queue.closed {
            return true;
        }
        let len = bytes.len();
        match kind {
            FrameKind::Output if queue.output_bytes + len > cap => {
                self.conn.release_bulk(queue.bulk_bytes);
                queue.frames.clear();
                queue.output_bytes = 0;
                queue.bulk_bytes = 0;
                queue.backfill_pending = false;
                queue.closed = true;
                if let Ok(error) = (DaemonFrame::Error {
                    request_id: attach_id,
                    pane: self.pane_id,
                    code: ErrorCode::Resync,
                    retryable: true,
                })
                .encode()
                {
                    queue.frames.push_back(QueuedFrame {
                        bytes: error,
                        kind: FrameKind::Marker,
                    });
                }
                drop(queue);
                self.conn.notify();
                return false;
            }
            FrameKind::Output => queue.output_bytes += len,
            FrameKind::Bulk => {
                queue.bulk_bytes += len;
                self.conn.reserve_bulk(len);
            }
            FrameKind::Marker => {}
        }
        queue.frames.push_back(QueuedFrame { bytes, kind });
        drop(queue);
        self.conn.notify();
        true
    }

    /// End the attachment because its pane was removed: frames already
    /// queued (final output, `Exited`) are still delivered, followed by
    /// `last`, so every viewer sees the end of the pane.
    fn finish(&self, last: Option<Vec<u8>>) {
        let mut queue = self.queue.lock();
        if queue.closed {
            return;
        }
        queue.backfill_pending = false;
        queue.closed = true;
        if let Some(bytes) = last {
            queue.frames.push_back(QueuedFrame {
                bytes,
                kind: FrameKind::Marker,
            });
        }
        drop(queue);
        self.conn.notify();
    }

    /// Stop the attachment: queued frames are discarded.
    fn close(&self) {
        let mut queue = self.queue.lock();
        self.conn.release_bulk(queue.bulk_bytes);
        queue.frames.clear();
        queue.output_bytes = 0;
        queue.bulk_bytes = 0;
        queue.backfill_pending = false;
        queue.closed = true;
        drop(queue);
        self.conn.notify();
    }

    /// Frames for one writer turn (about `budget` bytes), and whether the
    /// attachment is finished (closed and drained).
    pub(super) fn pop_turn(&self, budget: usize) -> (Vec<Vec<u8>>, bool) {
        let mut queue = self.queue.lock();
        let mut frames = Vec::new();
        let mut taken = 0;
        while taken < budget {
            let Some(frame) = queue.frames.pop_front() else {
                break;
            };
            taken += frame.bytes.len();
            match frame.kind {
                FrameKind::Output => queue.output_bytes -= frame.bytes.len(),
                FrameKind::Bulk => {
                    queue.bulk_bytes -= frame.bytes.len();
                    self.conn.release_bulk(frame.bytes.len());
                }
                FrameKind::Marker => {}
            }
            frames.push(frame.bytes);
        }
        let finished = queue.closed && queue.frames.is_empty();
        (frames, finished)
    }

    /// Whether the writer should ask the pane for the next history chunk.
    pub(super) fn wants_backfill(&self, low_water: usize) -> bool {
        let queue = self.queue.lock();
        queue.backfill_pending && !queue.closed && queue.output_bytes + queue.bulk_bytes < low_water
    }

    fn set_backfill_pending(&self, pending: bool) {
        self.queue.lock().backfill_pending = pending;
    }
}

// ---------------------------------------------------------------------------
// Input queue
// ---------------------------------------------------------------------------

#[derive(Debug, PartialEq, Eq)]
pub(super) enum InputPush {
    Queued,
    /// The PTY made no progress for the stall timeout: the queue was
    /// discarded, this frame included.
    Stalled,
    Closed,
}

struct InputInner {
    frames: VecDeque<Vec<u8>>,
    bytes: usize,
    closed: bool,
    /// Bumped each time the writer thread takes a frame and each time the
    /// PTY accepts bytes of the frame being written.
    progress: u64,
    /// Bumped each time the queue is discarded for a stall: the frame the
    /// writer holds at that moment is abandoned too.
    generation: u64,
}

/// Bounded input queue of one pane, drained by its single writer thread:
/// frames reach the PTY whole and in arrival order.
pub(super) struct InputQueue {
    inner: Mutex<InputInner>,
    not_empty: Condvar,
    space: Condvar,
    cap: usize,
}

impl InputQueue {
    fn new(cap: usize) -> Self {
        Self {
            inner: Mutex::new(InputInner {
                frames: VecDeque::new(),
                bytes: 0,
                closed: false,
                progress: 0,
                generation: 0,
            }),
            not_empty: Condvar::new(),
            space: Condvar::new(),
            cap,
        }
    }

    /// Queue client input, waiting for room. While it waits the calling
    /// connection reads nothing else, which is the end-to-end flow control
    /// for large pastes.
    pub(super) fn push_blocking(&self, data: Vec<u8>, stall: Duration) -> InputPush {
        let mut inner = self.inner.lock();
        loop {
            if inner.closed {
                return InputPush::Closed;
            }
            if inner.frames.is_empty() || inner.bytes + data.len() <= self.cap {
                inner.bytes += data.len();
                inner.frames.push_back(data);
                drop(inner);
                self.not_empty.notify_one();
                return InputPush::Queued;
            }
            let progress = inner.progress;
            let timed_out = self.space.wait_for(&mut inner, stall).timed_out();
            if timed_out && inner.progress == progress && !inner.closed {
                inner.frames.clear();
                inner.bytes = 0;
                inner.generation = inner.generation.wrapping_add(1);
                drop(inner);
                // Other connections waiting for room have it now.
                self.space.notify_all();
                return InputPush::Stalled;
            }
        }
    }

    /// Queue a terminal query reply from the daemon's grid. Dropped when the
    /// queue is full: the PTY reader must never wait.
    fn push_reply(&self, data: Vec<u8>) {
        let mut inner = self.inner.lock();
        if inner.closed || inner.bytes + data.len() > self.cap {
            return;
        }
        inner.bytes += data.len();
        inner.frames.push_back(data);
        drop(inner);
        self.not_empty.notify_one();
    }

    /// The next frame for the PTY, with the queue generation it belongs
    /// to. `None`: the queue is closed.
    fn pop(&self) -> Option<(Vec<u8>, u64)> {
        let mut inner = self.inner.lock();
        loop {
            if let Some(frame) = inner.frames.pop_front() {
                inner.bytes -= frame.len();
                inner.progress = inner.progress.wrapping_add(1);
                let generation = inner.generation;
                drop(inner);
                self.space.notify_all();
                return Some((frame, generation));
            }
            if inner.closed {
                return None;
            }
            self.not_empty.wait(&mut inner);
        }
    }

    /// The PTY accepted bytes of the frame being written.
    fn note_progress(&self) {
        let mut inner = self.inner.lock();
        inner.progress = inner.progress.wrapping_add(1);
    }

    /// Whether the frame popped at `generation` must not be written any
    /// further: the queue was closed, or discarded for a stall.
    fn abandoned(&self, generation: u64) -> bool {
        let inner = self.inner.lock();
        inner.closed || inner.generation != generation
    }

    fn close(&self) {
        let mut inner = self.inner.lock();
        inner.closed = true;
        inner.frames.clear();
        inner.bytes = 0;
        drop(inner);
        self.not_empty.notify_all();
        self.space.notify_all();
    }
}

// ---------------------------------------------------------------------------
// Pane
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ExitInfo {
    pub(super) exit_code: i32,
    pub(super) signal: u8,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct HistoryUsage {
    pub(super) bytes: u64,
    pub(super) rows: usize,
    pub(super) attached: bool,
}

struct Backfill {
    /// Absolute history index: rows older than this are still to send.
    next_end: u64,
    remaining: u32,
    sent: u32,
    epoch: u64,
}

struct Subscriber {
    conn_id: u64,
    attachment: Arc<Attachment>,
    attach_id: u32,
    read_only: bool,
    /// Size this client asked for. `None`: it attached with "keep the
    /// current size" and has not sent a `Resize` yet.
    requested: Option<(u16, u16)>,
    last_input: Option<Instant>,
    backfill: Option<Backfill>,
    /// Dropped for `Resync` (slow client): no output is queued until its
    /// next `Attach`, but its input, resize and size ownership still
    /// count, so a flooding pane can be interrupted from the keyboard.
    suspended: bool,
}

struct PaneState {
    grid: Arc<Mutex<TerminalGrid>>,
    processor: TerminalProcessor,
    parser: vte::Parser,
    boundary: StreamBoundary,
    replies: std::sync::mpsc::Receiver<Vec<u8>>,
    /// PTY output bytes read and parsed since spawn.
    offset: u64,
    rows: u16,
    cols: u16,
    subscribers: Vec<Subscriber>,
    master: Option<PtyMaster>,
    exit: Option<ExitInfo>,
    died_at: Option<Instant>,
    /// A client asked for the pane to be ended (`Kill`): nobody wants its
    /// last screen, so it is removed as soon as it has exited instead of
    /// being kept for a later reattach.
    discard: bool,
    /// Connection whose attachment owns the pane size.
    owner: Option<u64>,
    pending_resize: Option<(u16, u16)>,
    last_resize: Option<Instant>,
}

pub(super) struct Pane {
    pub(super) id: Uuid,
    pub(super) pid: u32,
    group: Uuid,
    label: String,
    created_at: u64,
    state: Mutex<PaneState>,
    pub(super) input: InputQueue,
    config: Arc<ServerConfig>,
    stop: AtomicBool,
}

pub(super) struct SpawnedPty {
    pub(super) master: PtyMaster,
    pub(super) writer: Box<dyn Write + Send>,
    pub(super) pid: u32,
}

impl Pane {
    /// Register a spawned PTY as a pane and start its reader and writer
    /// threads.
    pub(super) fn start(
        pty: SpawnedPty,
        group: Uuid,
        label: String,
        rows: u16,
        cols: u16,
        scrollback: usize,
        config: Arc<ServerConfig>,
    ) -> std::io::Result<Arc<Self>> {
        let raw = pty
            .master
            .as_raw_fd()
            .ok_or_else(|| std::io::Error::other("PTY master has no file descriptor"))?;
        // The reader owns its own descriptor, so closing the master later
        // never invalidates the one being polled.
        // SAFETY: `raw` is the open master descriptor, alive for this call.
        let read_fd: OwnedFd = unsafe { BorrowedFd::borrow_raw(raw) }.try_clone_to_owned()?;
        // The writer has its own descriptor too, and the master is put in
        // non-blocking mode: a child that never reads its terminal must
        // not hold the writer thread inside `write` for ever (it has to
        // notice a discarded queue and a closed pane). The flag belongs to
        // the open file, so it also applies to the reader, which polls
        // before it reads.
        let write_fd: OwnedFd = read_fd.try_clone()?;
        // SAFETY: fcntl on a descriptor this function owns.
        unsafe {
            let flags = libc::fcntl(write_fd.as_raw_fd(), libc::F_GETFL);
            if flags == -1
                || libc::fcntl(
                    write_fd.as_raw_fd(),
                    libc::F_SETFL,
                    flags | libc::O_NONBLOCK,
                ) == -1
            {
                return Err(std::io::Error::last_os_error());
            }
        }

        let mut grid = TerminalGrid::with_scrollback(rows as usize, cols as usize, scrollback);
        // Only the daemon's grid answers terminal queries (DA, DSR, CPR):
        // client grids never do, or the application would get one reply per
        // attached emulator.
        let (reply_tx, replies) = std::sync::mpsc::channel();
        grid.set_response_tx(reply_tx);
        let grid = Arc::new(Mutex::new(grid));

        let pane = Arc::new(Self {
            id: Uuid::new_v4(),
            pid: pty.pid,
            group,
            label,
            created_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0),
            state: Mutex::new(PaneState {
                processor: TerminalProcessor::new(grid.clone()),
                grid,
                parser: vte::Parser::new(),
                boundary: StreamBoundary::new(),
                replies,
                offset: 0,
                rows,
                cols,
                subscribers: Vec::new(),
                master: Some(pty.master),
                exit: None,
                died_at: None,
                discard: false,
                owner: None,
                pending_resize: None,
                last_resize: None,
            }),
            input: InputQueue::new(config.input_queue_cap),
            config,
            stop: AtomicBool::new(false),
        });

        let reader = pane.clone();
        std::thread::Builder::new()
            .name("sessiond-pane-reader".into())
            .spawn(move || reader.read_loop(read_fd))?;
        let writer = pane.clone();
        // Kept only for its `Drop`, which ends a line-mode child's input
        // with a newline and the EOF character, as for any PTY of the app.
        let pty_writer = pty.writer;
        std::thread::Builder::new()
            .name("sessiond-pane-writer".into())
            .spawn(move || {
                writer.write_loop(write_fd);
                writer.input.close();
                drop(pty_writer);
            })?;
        Ok(pane)
    }

    // -- PTY writer ---------------------------------------------------------

    /// One writer per pane: frames reach the PTY whole and in arrival
    /// order. A frame the PTY does not take is retried until the queue is
    /// discarded for a stall or the pane is closed.
    fn write_loop(&self, write_fd: OwnedFd) {
        let mut file = std::fs::File::from(write_fd);
        while let Some((frame, generation)) = self.input.pop() {
            let mut written = 0;
            while written < frame.len() {
                if self.input.abandoned(generation) {
                    break;
                }
                match file.write(&frame[written..]) {
                    Ok(0) => return,
                    Ok(n) => {
                        written += n;
                        self.input.note_progress();
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        let mut pollfd = libc::pollfd {
                            fd: file.as_raw_fd(),
                            events: libc::POLLOUT,
                            revents: 0,
                        };
                        // SAFETY: one valid pollfd for a descriptor owned
                        // by `file`.
                        unsafe { libc::poll(&mut pollfd, 1, WRITER_TICK_MS) };
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                    // EIO: the slave side is gone.
                    Err(_) => return,
                }
            }
        }
    }

    // -- PTY reader ---------------------------------------------------------

    fn read_loop(self: Arc<Self>, read_fd: OwnedFd) {
        let mut file = std::fs::File::from(read_fd);
        let mut buf = vec![0u8; MAX_OUTPUT_BYTES];
        let mut eof = false;
        let mut exit: Option<(ExitInfo, Instant)> = None;
        let mut last_wait = Instant::now();
        loop {
            if self.stop.load(Ordering::Relaxed) {
                return;
            }
            self.tick();

            let mut readable = false;
            if !eof {
                let mut pollfd = libc::pollfd {
                    fd: file.as_raw_fd(),
                    events: libc::POLLIN,
                    revents: 0,
                };
                // SAFETY: `pollfd` is a valid array of one entry and the
                // descriptor is owned by `file`.
                let rc = unsafe { libc::poll(&mut pollfd, 1, READER_TICK_MS) };
                readable = rc > 0;
            } else {
                std::thread::sleep(Duration::from_millis(READER_TICK_MS as u64));
            }
            if readable {
                match file.read(&mut buf) {
                    Ok(0) => eof = true,
                    Ok(n) => {
                        if !self.ingest(&buf[..n]) {
                            return;
                        }
                    }
                    Err(e)
                        if matches!(
                            e.kind(),
                            std::io::ErrorKind::Interrupted | std::io::ErrorKind::WouldBlock
                        ) => {}
                    // EIO: every slave descriptor is closed.
                    Err(_) => eof = true,
                }
            }

            // Pane exit is the child's `wait`, not PTY EOF: a background job
            // can keep the slave side open.
            if exit.is_none() && (!readable || last_wait.elapsed() >= Duration::from_millis(50)) {
                last_wait = Instant::now();
                if let Some(info) = self.try_wait() {
                    exit = Some((info, Instant::now() + self.config.exit_drain));
                }
            }
            if let Some((info, deadline)) = exit {
                // Drain what the child left behind, bounded.
                if eof || Instant::now() >= deadline {
                    self.finish(info);
                    return;
                }
            }
        }
    }

    fn try_wait(&self) -> Option<ExitInfo> {
        let mut state = self.state.lock();
        let status = state.master.as_mut()?.try_wait().ok().flatten()?;
        let signal = status.signal().map(signal_number).unwrap_or(0);
        let exit_code = if signal != 0 {
            128 + signal as i32
        } else {
            status.exit_code() as i32
        };
        Some(ExitInfo { exit_code, signal })
    }

    /// Parse a chunk of PTY output and fan it out. Returns `false` when the
    /// pane died (parser panic).
    fn ingest(&self, bytes: &[u8]) -> bool {
        let mut guard = self.state.lock();
        let state = &mut *guard;
        let fault = self.config.fault_parser_panic.as_deref();
        let parsed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            if let Some(marker) = fault.filter(|marker| !marker.is_empty()) {
                if bytes.windows(marker.len()).any(|window| window == marker) {
                    panic!("injected parser fault");
                }
            }
            state.processor.process_bytes(&mut state.parser, bytes);
        }));
        if parsed.is_err() {
            // Panic isolation: this pane is dead, the others are not.
            drop(guard);
            tracing::error!(pane = %self.id, "parser panicked, closing pane");
            self.signal_group(libc::SIGHUP);
            self.finish(ExitInfo {
                exit_code: -1,
                signal: 0,
            });
            return false;
        }
        state.boundary.advance(bytes);
        let offset = state.offset;
        state.offset += bytes.len() as u64;
        // Replies go through the input queue, ordered with client input.
        while let Ok(reply) = state.replies.try_recv() {
            self.input.push_reply(reply);
        }
        let cap = self.config.output_queue_cap;
        let id = self.id;
        let mut dropped = false;
        for sub in state.subscribers.iter().filter(|sub| !sub.suspended) {
            let frame = protocol::encode_output(&id, sub.attach_id, offset, bytes);
            if !sub
                .attachment
                .push(frame, FrameKind::Output, sub.attach_id, cap)
            {
                dropped = true;
            }
        }
        if dropped {
            // Slow clients are dropped; the PTY and the others keep going.
            // The subscriber stays registered, without output, until its
            // next `Attach` or its disconnect.
            for sub in &mut state.subscribers {
                if sub.attachment.is_closed() {
                    sub.suspended = true;
                    sub.backfill = None;
                }
            }
        }
        true
    }

    /// The child exited: queue `Exited` after the final output and close the
    /// master. The pane stays listed with its final screen.
    fn finish(&self, info: ExitInfo) {
        let mut state = self.state.lock();
        if state.exit.is_some() {
            return;
        }
        state.exit = Some(info);
        state.died_at = Some(Instant::now());
        tracing::info!(
            pane = %self.id,
            exit_code = info.exit_code,
            signal = info.signal,
            "pane ended"
        );
        state.pending_resize = None;
        let frame = self.exited_frame(state.offset, info);
        for sub in &state.subscribers {
            sub.attachment.push(
                frame.clone(),
                FrameKind::Marker,
                sub.attach_id,
                self.config.output_queue_cap,
            );
        }
        let master = state.master.take();
        drop(state);
        drop(master);
        self.input.close();
    }

    fn exited_frame(&self, offset: u64, info: ExitInfo) -> Vec<u8> {
        DaemonFrame::Exited {
            pane: self.id,
            offset,
            exit_code: info.exit_code,
            signal: info.signal,
        }
        .encode()
        .unwrap_or_default()
    }

    // -- Size ---------------------------------------------------------------

    /// Apply a size change now: `TIOCSWINSZ`, grid resize and the applied
    /// `Resize` frame, in one step under the pane lock so every client
    /// resizes at the same stream position.
    fn apply_size(state: &mut PaneState, id: Uuid, cap: usize, rows: u16, cols: u16) {
        state.pending_resize = None;
        if state.exit.is_some() || (rows, cols) == (state.rows, state.cols) {
            return;
        }
        if let Some(master) = state.master.as_ref() {
            let _ = master.resize(rows, cols);
        }
        let pulled = {
            let mut grid = state.grid.lock();
            // `resize` pulls rows back from history when the screen grows; a
            // client with less history cannot reproduce that and reattaches.
            let pulled = (rows as usize)
                .saturating_sub(grid.rows)
                .min(grid.scrollback_len());
            grid.resize(rows as usize, cols as usize);
            let end = grid.history_end();
            for sub in &mut state.subscribers {
                if let Some(backfill) = sub.backfill.as_mut() {
                    backfill.next_end = backfill.next_end.min(end);
                }
            }
            pulled.min(u16::MAX as usize) as u16
        };
        state.rows = rows;
        state.cols = cols;
        state.last_resize = Some(Instant::now());
        for sub in &state.subscribers {
            if let Ok(frame) = (DaemonFrame::Resized {
                pane: id,
                attach_id: sub.attach_id,
                offset: state.offset,
                rows,
                cols,
                pulled,
            })
            .encode()
            {
                sub.attachment
                    .push(frame, FrameKind::Marker, sub.attach_id, cap);
            }
        }
    }

    /// Apply at most one size change per pane per 50 ms (trailing edge).
    fn request_size(state: &mut PaneState, id: Uuid, cap: usize, rows: u16, cols: u16) {
        let due = state
            .last_resize
            .is_none_or(|at| at.elapsed() >= RESIZE_INTERVAL);
        if due {
            Self::apply_size(state, id, cap, rows, cols);
        } else if (rows, cols) == (state.rows, state.cols) {
            state.pending_resize = None;
        } else {
            state.pending_resize = Some((rows, cols));
        }
    }

    /// Called by the reader thread every few milliseconds.
    fn tick(&self) {
        let mut state = self.state.lock();
        if let Some((rows, cols)) = state.pending_resize {
            Self::request_size(
                &mut state,
                self.id,
                self.config.output_queue_cap,
                rows,
                cols,
            );
        }
    }

    /// The owner went away: the most recently active writable attachment
    /// takes over the size.
    fn reassign_owner(state: &mut PaneState, id: Uuid, cap: usize) {
        if state
            .owner
            .is_some_and(|owner| state.subscribers.iter().any(|s| s.conn_id == owner))
        {
            return;
        }
        let next = state
            .subscribers
            .iter()
            .filter(|sub| !sub.read_only)
            .max_by_key(|sub| sub.last_input)
            .map(|sub| (sub.conn_id, sub.requested));
        state.owner = next.map(|(conn_id, _)| conn_id);
        if let Some((_, Some((rows, cols)))) = next {
            Self::request_size(state, id, cap, rows, cols);
        }
    }

    // -- Client operations --------------------------------------------------

    /// Attach, or refresh in place when this connection already has the
    /// pane attached. The snapshot is serialized and queued under the pane
    /// lock, together with `Ready`, before the subscriber is registered: the
    /// PTY is paused only for that in-memory walk.
    pub(super) fn attach(
        self: &Arc<Self>,
        conn: &Arc<ConnOut>,
        request: &Attach,
    ) -> Result<Arc<Attachment>, (ErrorCode, bool)> {
        let cap = self.config.output_queue_cap;
        let mut guard = self.state.lock();
        let state = &mut *guard;
        let read_only = request.flags & ATTACH_READ_ONLY != 0;
        // Rows and cols 0: keep the pane's current size. The client does
        // not know its view size yet (app restart); it must not resize a
        // live pane to a placeholder, which would also cut the history to
        // that width. It takes the size with its first `Resize`.
        let requested =
            (request.rows != 0 && request.cols != 0).then_some((request.rows, request.cols));
        let position = state
            .subscribers
            .iter()
            .position(|sub| sub.conn_id == conn.id);
        if position.is_none() && state.subscribers.len() >= MAX_ATTACHMENTS_PER_PANE {
            return Err((ErrorCode::LimitExceeded, false));
        }

        // Size first, so the snapshot is taken at the final size.
        let owner_present = state
            .owner
            .is_some_and(|owner| state.subscribers.iter().any(|s| s.conn_id == owner));
        if let Some((rows, cols)) = requested {
            if !read_only && (!owner_present || state.owner == Some(conn.id)) {
                state.owner = Some(conn.id);
                Self::apply_size(state, self.id, cap, rows, cols);
            }
        }

        let (snapshot, history_end, epoch) = {
            let grid = state.grid.lock();
            (
                grid.snapshot_with_tail(state.boundary.tail()),
                grid.history_end(),
                grid.history_epoch(),
            )
        };
        if snapshot.len() > MAX_SNAPSHOT_TOTAL {
            return Err((ErrorCode::Internal, false));
        }
        if conn.bulk_bytes() + snapshot.len() > self.config.bulk_queue_cap {
            return Err((ErrorCode::Busy, true));
        }

        // A refresh reuses the FIFO; an attachment dropped for `Resync` is
        // closed for good and gets a new one.
        let reusable = position
            .map(|index| state.subscribers[index].attachment.clone())
            .filter(|attachment| !attachment.is_closed());
        let attachment = match reusable {
            Some(attachment) => attachment,
            None => Arc::new(Attachment {
                pane_id: self.id,
                pane: Arc::downgrade(self),
                conn: conn.clone(),
                queue: Mutex::new(AttachmentQueue::default()),
            }),
        };

        let attach_id = request.attach_id;
        let chunks: Vec<&[u8]> = if snapshot.is_empty() {
            vec![&[]]
        } else {
            snapshot.chunks(MAX_SNAPSHOT_CHUNK).collect()
        };
        let last = chunks.len() - 1;
        for (seq, chunk) in chunks.into_iter().enumerate() {
            let frame = DaemonFrame::Snapshot {
                pane: self.id,
                attach_id,
                seq: seq as u32,
                flags: if seq == last { FLAG_LAST } else { 0 },
                bytes: chunk.to_vec(),
            }
            .encode()
            .map_err(|_| (ErrorCode::Internal, false))?;
            attachment.push(frame, FrameKind::Bulk, attach_id, cap);
        }
        let ready = DaemonFrame::Ready {
            pane: self.id,
            attach_id,
            offset: state.offset,
            rows: state.rows,
            cols: state.cols,
        }
        .encode()
        .map_err(|_| (ErrorCode::Internal, false))?;
        attachment.push(ready, FrameKind::Marker, attach_id, cap);
        if let Some(info) = state.exit {
            let frame = self.exited_frame(state.offset, info);
            attachment.push(frame, FrameKind::Marker, attach_id, cap);
        }

        let backfill = (request.scrollback_rows > 0).then_some(Backfill {
            next_end: history_end,
            remaining: request.scrollback_rows,
            sent: 0,
            epoch,
        });
        attachment.set_backfill_pending(backfill.is_some());
        match position {
            Some(index) => {
                let sub = &mut state.subscribers[index];
                sub.attachment = attachment.clone();
                sub.attach_id = attach_id;
                sub.read_only = read_only;
                if requested.is_some() {
                    sub.requested = requested;
                }
                sub.backfill = backfill;
                sub.suspended = false;
            }
            None => state.subscribers.push(Subscriber {
                conn_id: conn.id,
                attachment: attachment.clone(),
                attach_id,
                read_only,
                requested,
                last_input: None,
                backfill,
                suspended: false,
            }),
        }
        Ok(attachment)
    }

    /// Remove a connection's attachment. Never touches the PTY: the child
    /// does not notice that a client went away.
    pub(super) fn detach(&self, conn_id: u64) {
        let mut state = self.state.lock();
        let before = state.subscribers.len();
        state.subscribers.retain(|sub| {
            if sub.conn_id == conn_id {
                sub.attachment.close();
                false
            } else {
                true
            }
        });
        if state.subscribers.len() != before {
            Self::reassign_owner(&mut state, self.id, self.config.output_queue_cap);
        }
    }

    /// Validate an `Input` frame and update size ownership (last active
    /// client wins, with a 1 s hysteresis). The caller then queues the bytes.
    pub(super) fn note_input(&self, conn_id: u64) -> Result<bool, ErrorCode> {
        let cap = self.config.output_queue_cap;
        let mut guard = self.state.lock();
        let state = &mut *guard;
        let now = Instant::now();
        let index = state
            .subscribers
            .iter()
            .position(|sub| sub.conn_id == conn_id)
            .ok_or(ErrorCode::NotAttached)?;
        if state.subscribers[index].read_only {
            return Err(ErrorCode::ReadOnly);
        }
        if state.exit.is_some() {
            return Ok(false);
        }
        state.subscribers[index].last_input = Some(now);
        if state.owner != Some(conn_id) {
            let owner_active = state.owner.is_some_and(|owner| {
                state.subscribers.iter().any(|sub| {
                    sub.conn_id == owner
                        && sub
                            .last_input
                            .is_some_and(|at| now.duration_since(at) < OWNER_HYSTERESIS)
                })
            });
            if !owner_active {
                state.owner = Some(conn_id);
                if let Some((rows, cols)) = state.subscribers[index].requested {
                    Self::request_size(state, self.id, cap, rows, cols);
                }
            }
        }
        Ok(true)
    }

    pub(super) fn resize(&self, conn_id: u64, rows: u16, cols: u16) -> Result<(), ErrorCode> {
        let mut guard = self.state.lock();
        let state = &mut *guard;
        let sub = state
            .subscribers
            .iter_mut()
            .find(|sub| sub.conn_id == conn_id)
            .ok_or(ErrorCode::NotAttached)?;
        if sub.read_only {
            return Err(ErrorCode::ReadOnly);
        }
        sub.requested = Some((rows, cols));
        // A pane nobody owns (its clients attached with "keep the current
        // size", or the owner left) takes the first size it is asked for.
        let owner_present = state
            .owner
            .is_some_and(|owner| state.subscribers.iter().any(|s| s.conn_id == owner));
        if !owner_present {
            state.owner = Some(conn_id);
        }
        if state.owner == Some(conn_id) {
            Self::request_size(state, self.id, self.config.output_queue_cap, rows, cols);
        }
        Ok(())
    }

    pub(super) fn is_read_only_for(&self, conn_id: u64) -> bool {
        self.state
            .lock()
            .subscribers
            .iter()
            .any(|sub| sub.conn_id == conn_id && sub.read_only)
    }

    pub(super) fn is_alive(&self) -> bool {
        self.state.lock().exit.is_none()
    }

    /// A client asked for this pane to be ended: see `PaneState::discard`.
    pub(super) fn discard_at_exit(&self) {
        self.state.lock().discard = true;
    }

    /// Whether the dead pane is to be removed now: kept for `ttl` so a
    /// reattach can still show how it ended, unless a client asked for
    /// its end.
    pub(super) fn expired(&self, ttl: Duration) -> bool {
        let state = self.state.lock();
        state
            .died_at
            .is_some_and(|at| state.discard || at.elapsed() >= ttl)
    }

    /// Send a signal to the pane child's process group (the child is a
    /// session and group leader).
    pub(super) fn signal_group(&self, signal: i32) {
        if self.pid == 0 || !self.is_alive() {
            return;
        }
        // SAFETY: plain syscall; a negative pid addresses the process group.
        unsafe {
            libc::kill(-(self.pid as libc::pid_t), signal);
        }
    }

    pub(super) fn group_exists(&self) -> bool {
        if self.pid == 0 {
            return false;
        }
        // SAFETY: signal 0 only checks for existence.
        unsafe { libc::kill(-(self.pid as libc::pid_t), 0) == 0 }
    }

    pub(super) fn kill_group(&self) {
        if self.pid != 0 {
            // SAFETY: plain syscall on the pane's own process group.
            unsafe {
                libc::kill(-(self.pid as libc::pid_t), libc::SIGKILL);
            }
        }
    }

    /// Close the pane for good: stop the threads, close the master (which
    /// hangs up the child's session) and end every attachment. What is
    /// already queued for a client is still delivered, then
    /// `Error(NoSuchPane)` tells it the pane is gone.
    pub(super) fn close(&self) {
        self.stop.store(true, Ordering::Relaxed);
        let mut state = self.state.lock();
        for sub in state.subscribers.drain(..) {
            let gone = DaemonFrame::Error {
                request_id: sub.attach_id,
                pane: self.id,
                code: ErrorCode::NoSuchPane,
                retryable: false,
            }
            .encode()
            .ok();
            sub.attachment.finish(gone);
        }
        state.owner = None;
        let master = state.master.take();
        drop(state);
        drop(master);
        self.input.close();
    }

    /// Build and queue the next history chunk for one attachment (called by
    /// the connection writer, one chunk per turn, only while the queue is
    /// short so live output keeps priority).
    pub(super) fn backfill_step(&self, conn_id: u64) {
        let cap = self.config.output_queue_cap;
        let mut guard = self.state.lock();
        let state = &mut *guard;
        let Some(sub) = state
            .subscribers
            .iter_mut()
            .find(|sub| sub.conn_id == conn_id)
        else {
            return;
        };
        let Some(backfill) = sub.backfill.as_mut() else {
            sub.attachment.set_backfill_pending(false);
            return;
        };
        let grid = state.grid.lock();
        let mut chunk = None;
        if backfill.remaining > 0 && backfill.epoch == grid.history_epoch() {
            let mut rows = BACKFILL_ROWS.min(backfill.remaining as usize);
            while rows > 0 {
                let candidate = grid.history_rows(backfill.next_end, rows);
                if candidate.bytes.len() <= MAX_SCROLLBACK_CHUNK {
                    if candidate.count > 0 {
                        chunk = Some(candidate);
                    }
                    break;
                }
                rows /= 2;
            }
        }
        drop(grid);
        let (count, flags, bytes) = match chunk {
            Some(chunk) => {
                backfill.next_end = chunk.next_end;
                (chunk.count, 0, chunk.bytes)
            }
            // Nothing older is retained, or the history was cleared: end.
            None => (0, FLAG_LAST, Vec::new()),
        };
        let frame = DaemonFrame::ScrollbackChunk {
            pane: self.id,
            attach_id: sub.attach_id,
            start: backfill.sent,
            count,
            flags,
            bytes,
        }
        .encode();
        backfill.sent += count as u32;
        backfill.remaining = backfill.remaining.saturating_sub(count as u32);
        if flags & FLAG_LAST != 0 {
            sub.backfill = None;
            sub.attachment.set_backfill_pending(false);
        }
        if let Ok(frame) = frame {
            sub.attachment
                .push(frame, FrameKind::Bulk, sub.attach_id, cap);
        }
    }

    /// Memory held by this pane's history, as the budget counts it.
    pub(super) fn history_usage(&self) -> HistoryUsage {
        let state = self.state.lock();
        let grid = state.grid.lock();
        let rows = grid.scrollback_len();
        HistoryUsage {
            bytes: (rows as u64)
                .saturating_mul(grid.cols as u64)
                .saturating_mul(std::mem::size_of::<shelldeck_terminal::grid::Cell>() as u64),
            rows,
            attached: !state.subscribers.is_empty(),
        }
    }

    /// Drop the older half of the history and keep the pane at that lower
    /// limit. The screen is not touched. Returns the rows kept.
    pub(super) fn halve_history(&self) -> usize {
        let state = self.state.lock();
        let mut grid = state.grid.lock();
        let keep = (grid.scrollback_len() / 2).max(1);
        grid.set_max_scrollback(keep);
        grid.scrollback_len()
    }

    pub(super) fn info(&self) -> PaneInfo {
        let state = self.state.lock();
        let (title, cwd) = {
            let grid = state.grid.lock();
            (
                grid.title.clone(),
                grid.working_directory.clone().unwrap_or_default(),
            )
        };
        PaneInfo {
            pane: self.id,
            group: self.group,
            label: self.label.clone(),
            title,
            cwd,
            rows: state.rows,
            cols: state.cols,
            created_at: self.created_at,
            alive: state.exit.is_none(),
            exit_code: state.exit.map(|e| e.exit_code).unwrap_or(0),
            clients: state
                .subscribers
                .iter()
                .filter(|sub| !sub.suspended)
                .count() as u16,
            offset: state.offset,
        }
    }
}

/// portable-pty reports a terminating signal by its `strsignal` name only.
fn signal_number(name: &str) -> u8 {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _guard = LOCK.lock();
    for signal in 1..=64 {
        // SAFETY: strsignal returns a pointer to a NUL-terminated string (or
        // NULL) that stays valid until the next call; calls are serialized
        // by `LOCK` and the string is copied before the lock is released.
        let text = unsafe {
            let ptr = libc::strsignal(signal);
            if ptr.is_null() {
                continue;
            }
            std::ffi::CStr::from_ptr(ptr).to_string_lossy().into_owned()
        };
        if text == name {
            return signal as u8;
        }
    }
    0
}
