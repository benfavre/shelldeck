//! Async client of the session daemon over any byte stream: a Unix socket
//! locally, an SSH exec channel running `shelldeck-sessiond proxy` remotely
//! (docs/design/session-daemon.md, sections 5, 8 and 10).
//!
//! One [`Client`] owns one connection. It demultiplexes frames by pane into
//! per-pane event queues, enforces the offset rules and restarts an attach
//! when a pane loses sync. Each pane's events are meant to be applied to a
//! client-side emulator: [`PaneMirror`] is that emulator without a UI.
//!
//! All methods must be called from within a Tokio runtime.

use crate::protocol::{
    self, Attach, ClientFrame, DaemonFrame, ErrorCode, FrameBuffer, Hello, HelloAck, PaneInfo,
    ProtocolError, Spawn, ATTACH_READ_ONLY, FLAG_LAST, MAX_INPUT_BYTES, MAX_SNAPSHOT_TOTAL,
    PROTOCOL_VERSION,
};
use parking_lot::Mutex;
use shelldeck_terminal::grid::{SnapshotError, TerminalGrid};
use shelldeck_terminal::parser::TerminalProcessor;
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::{mpsc, oneshot, Notify};
use uuid::Uuid;

/// Outbound frames queued off the caller's thread; beyond this a send fails
/// visibly instead of growing without bound.
pub const MAX_OUTBOUND_BYTES: usize = 16 * 1024 * 1024;
/// Pending output per pane before the client drops it and reattaches.
pub const MAX_FEED_BYTES: usize = 8 * 1024 * 1024;
const MAX_PRELUDE_BYTES: usize = 64 * 1024;
const RESIZE_INTERVAL: Duration = Duration::from_millis(50);
const BACKOFF_START: Duration = Duration::from_millis(250);
const BACKOFF_MAX: Duration = Duration::from_secs(2);
const BACKOFF_RESET: Duration = Duration::from_secs(10);

#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("connection to the session daemon is closed")]
    Closed,
    #[error("the session daemon did not answer in time")]
    Timeout,
    #[error("protocol error: {0}")]
    Protocol(#[from] ProtocolError),
    #[error("the session daemon refused: {0:?}")]
    Daemon(ErrorCode),
    #[error("outbound queue is full")]
    OutboundFull,
    #[error("pane is not attached on this connection")]
    NotAttached,
    #[error("proxy prelude is invalid")]
    Prelude,
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[cfg(unix)]
    #[error(transparent)]
    Path(#[from] crate::paths::PathError),
}

#[derive(Debug, Clone)]
pub struct ClientOptions {
    pub client_version: String,
    /// `Hello` must be answered within this time.
    pub hello_timeout: Duration,
    /// Per-pane feed cap, see [`MAX_FEED_BYTES`].
    pub max_feed_bytes: usize,
}

impl Default for ClientOptions {
    fn default() -> Self {
        Self {
            client_version: env!("CARGO_PKG_VERSION").to_string(),
            hello_timeout: Duration::from_secs(10),
            max_feed_bytes: MAX_FEED_BYTES,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct SpawnOptions {
    pub group: Uuid,
    pub label: String,
    pub rows: u16,
    pub cols: u16,
    /// 0 means the daemon default.
    pub scrollback_max: u32,
    /// Empty means the home directory.
    pub cwd: String,
    /// Empty means the login shell.
    pub argv: Vec<String>,
    pub env: Vec<(String, String)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpawnedPane {
    pub pane: Uuid,
    pub pid: u32,
    /// Canonical working directory the daemon opened.
    pub cwd: String,
}

#[derive(Debug, Clone, Copy)]
pub struct AttachOptions {
    /// Size wanted for the pane. 0 x 0 keeps the pane's current size until
    /// the first [`Client::resize`]: use it when the view size is not
    /// known yet, so a live pane is never resized to a placeholder.
    pub rows: u16,
    pub cols: u16,
    /// History rows to backfill after the snapshot; 0 for none.
    pub scrollback_rows: u32,
    pub read_only: bool,
}

/// What a pane's consumer applies, in order, to its own emulator.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaneEvent {
    /// Replace the screen from a snapshot taken at `offset`. Unless
    /// `keep_history`, the local history must be dropped.
    Restore {
        snapshot: Vec<u8>,
        offset: u64,
        rows: u16,
        cols: u16,
        keep_history: bool,
    },
    /// Raw PTY output starting at stream offset `offset`, contiguous with
    /// everything applied before.
    Bytes { offset: u64, data: Vec<u8> },
    /// The daemon resized the pane at this stream position.
    Resize {
        offset: u64,
        rows: u16,
        cols: u16,
        pulled: u16,
    },
    /// History rows, newest first, to insert at the old end.
    History {
        rows: Vec<u8>,
        count: u16,
        last: bool,
    },
    Exited {
        offset: u64,
        exit_code: i32,
        signal: u8,
    },
    /// The pane lost sync; a new attach is under way and a `Restore`
    /// follows. Keep showing the last screen meanwhile.
    Resyncing,
    /// The daemon reported an error for this pane.
    Error(ErrorCode),
    /// The connection is gone.
    Closed,
}

// ---------------------------------------------------------------------------
// Per-pane event queue
// ---------------------------------------------------------------------------

#[derive(Default)]
struct QueueInner {
    events: VecDeque<PaneEvent>,
    bytes: usize,
    closed: bool,
}

struct EventQueue {
    inner: Mutex<QueueInner>,
    notify: Notify,
    max_bytes: usize,
}

impl EventQueue {
    fn new(max_bytes: usize) -> Self {
        Self {
            inner: Mutex::new(QueueInner::default()),
            notify: Notify::new(),
            max_bytes,
        }
    }

    /// Queue an event. `false`: the pending output exceeds the feed cap.
    fn push(&self, event: PaneEvent) -> bool {
        let mut inner = self.inner.lock();
        if inner.closed {
            return true;
        }
        if let PaneEvent::Bytes { data, .. } = &event {
            if inner.bytes + data.len() > self.max_bytes {
                return false;
            }
            inner.bytes += data.len();
        }
        inner.events.push_back(event);
        drop(inner);
        self.notify.notify_one();
        true
    }

    fn clear(&self) {
        let mut inner = self.inner.lock();
        inner.events.clear();
        inner.bytes = 0;
    }

    fn close(&self) {
        self.inner.lock().closed = true;
        self.notify.notify_one();
    }

    fn pop(&self) -> Result<Option<PaneEvent>, ()> {
        let mut inner = self.inner.lock();
        match inner.events.pop_front() {
            Some(event) => {
                if let PaneEvent::Bytes { data, .. } = &event {
                    inner.bytes -= data.len();
                }
                Ok(Some(event))
            }
            None if inner.closed => Err(()),
            None => Ok(None),
        }
    }
}

/// Ordered events of one attached pane.
pub struct PaneEvents {
    pane: Uuid,
    queue: Arc<EventQueue>,
}

impl PaneEvents {
    pub fn pane(&self) -> Uuid {
        self.pane
    }

    /// Next event, or `None` once the attachment is over (detached, or the
    /// connection closed and everything was delivered).
    pub async fn next(&mut self) -> Option<PaneEvent> {
        loop {
            let notified = self.queue.notify.notified();
            match self.queue.pop() {
                Ok(Some(event)) => return Some(event),
                Ok(None) => notified.await,
                Err(()) => return None,
            }
        }
    }

    /// Next event if one is queued.
    pub fn try_next(&mut self) -> Option<PaneEvent> {
        self.queue.pop().ok().flatten()
    }
}

// ---------------------------------------------------------------------------
// Connection state
// ---------------------------------------------------------------------------

struct PendingAttach {
    id: u32,
    /// History rows asked for with this attach.
    scrollback_rows: u32,
    snapshot: Vec<u8>,
    next_seq: u32,
    complete: bool,
}

struct PaneCtl {
    queue: Arc<EventQueue>,
    rows: u16,
    cols: u16,
    scrollback_rows: u32,
    read_only: bool,
    /// Attach whose frames are being applied.
    current: Option<u32>,
    in_sync: bool,
    /// Stream offset of the next byte to apply.
    expected: u64,
    pending: Option<PendingAttach>,
    history_rows: u32,
    /// History rows the current attach asked for: the daemon may not send
    /// more.
    history_limit: u32,
    backoff: Duration,
    last_overload: Option<Instant>,
    last_resize: Option<Instant>,
    queued_resize: Option<(u16, u16)>,
    resize_timer: bool,
}

struct State {
    next_request: u32,
    next_attach: u32,
    spawns: HashMap<u32, oneshot::Sender<Result<SpawnedPane, ClientError>>>,
    lists: VecDeque<oneshot::Sender<Vec<PaneInfo>>>,
    panes: HashMap<Uuid, PaneCtl>,
}

struct Inner {
    hello: HelloAck,
    out: mpsc::UnboundedSender<Vec<u8>>,
    out_bytes: Arc<AtomicUsize>,
    closed: AtomicBool,
    state: Mutex<State>,
    options: ClientOptions,
    /// Dropped with the last `Client`: tells the reader task to stop, so
    /// the transport is released even if the peer never closes its side.
    _released: oneshot::Sender<()>,
}

impl Inner {
    fn send(&self, frame: &ClientFrame) -> Result<(), ClientError> {
        if self.closed.load(Ordering::Relaxed) {
            return Err(ClientError::Closed);
        }
        let bytes = frame.encode()?;
        if self.out_bytes.load(Ordering::Relaxed) + bytes.len() > MAX_OUTBOUND_BYTES {
            return Err(ClientError::OutboundFull);
        }
        self.out_bytes.fetch_add(bytes.len(), Ordering::Relaxed);
        self.out.send(bytes).map_err(|_| ClientError::Closed)
    }

    /// Start a new attach for a pane: allocate an id, expect its snapshot,
    /// send `Attach` now or after `delay` (overload backoff).
    fn start_attach(self: &Arc<Self>, state: &mut State, pane: Uuid, delay: Option<Duration>) {
        let id = state.next_attach;
        state.next_attach = state.next_attach.wrapping_add(1).max(1);
        let Some(ctl) = state.panes.get_mut(&pane) else {
            return;
        };
        // A refresh that is still in sync keeps its history.
        let scrollback_rows = if ctl.in_sync { 0 } else { ctl.scrollback_rows };
        ctl.pending = Some(PendingAttach {
            id,
            scrollback_rows,
            snapshot: Vec::new(),
            next_seq: 0,
            complete: false,
        });
        let frame = ClientFrame::Attach(Attach {
            pane,
            attach_id: id,
            rows: ctl.rows,
            cols: ctl.cols,
            scrollback_rows,
            flags: if ctl.read_only { ATTACH_READ_ONLY } else { 0 },
        });
        match delay {
            None => {
                let _ = self.send(&frame);
            }
            Some(delay) => {
                let inner = self.clone();
                tokio::spawn(async move {
                    tokio::time::sleep(delay).await;
                    // A newer attach may have replaced this one meanwhile.
                    let still_wanted = inner
                        .state
                        .lock()
                        .panes
                        .get(&pane)
                        .is_some_and(|ctl| ctl.pending.as_ref().is_some_and(|p| p.id == id));
                    if still_wanted {
                        let _ = inner.send(&frame);
                    }
                });
            }
        }
    }

    /// The pane lost sync: drop what is pending and attach again.
    fn resync(self: &Arc<Self>, state: &mut State, pane: Uuid, overload: bool) {
        let Some(ctl) = state.panes.get_mut(&pane) else {
            return;
        };
        ctl.in_sync = false;
        let delay = overload.then(|| {
            // 250 ms doubling to 2 s, reset after 10 s without overload.
            if ctl
                .last_overload
                .is_none_or(|at| at.elapsed() >= BACKOFF_RESET)
            {
                ctl.backoff = BACKOFF_START;
            }
            ctl.last_overload = Some(Instant::now());
            let delay = ctl.backoff;
            ctl.backoff = (ctl.backoff * 2).min(BACKOFF_MAX);
            delay
        });
        ctl.queue.push(PaneEvent::Resyncing);
        self.start_attach(state, pane, delay);
    }

    fn close(&self) {
        if self.closed.swap(true, Ordering::Relaxed) {
            return;
        }
        let mut state = self.state.lock();
        for (_, waiter) in state.spawns.drain() {
            let _ = waiter.send(Err(ClientError::Closed));
        }
        state.lists.clear();
        for (_, ctl) in state.panes.drain() {
            ctl.queue.push(PaneEvent::Closed);
            ctl.queue.close();
        }
    }

    /// Apply one daemon frame. `Err` is fatal for the connection.
    fn dispatch(self: &Arc<Self>, frame: DaemonFrame) -> Result<(), ClientError> {
        let mut guard = self.state.lock();
        let state = &mut *guard;
        match frame {
            DaemonFrame::HelloAck(_) => return Err(ProtocolError::InvalidValue("HelloAck").into()),
            DaemonFrame::PaneList(panes) => {
                if let Some(waiter) = state.lists.pop_front() {
                    let _ = waiter.send(panes);
                }
            }
            DaemonFrame::Spawned {
                request_id,
                pane,
                pid,
                cwd,
            } => {
                if let Some(waiter) = state.spawns.remove(&request_id) {
                    let _ = waiter.send(Ok(SpawnedPane { pane, pid, cwd }));
                }
            }
            DaemonFrame::Error {
                request_id,
                pane,
                code,
                ..
            } => {
                if pane.is_nil() {
                    if let Some(waiter) = state.spawns.remove(&request_id) {
                        let _ = waiter.send(Err(ClientError::Daemon(code)));
                    } else {
                        // BadFrame, UnsupportedVersion, LimitExceeded: the
                        // daemon is closing this connection.
                        return Err(ClientError::Daemon(code));
                    }
                } else if let Some(ctl) = state.panes.get_mut(&pane) {
                    let ours = request_id == 0
                        || ctl.current == Some(request_id)
                        || ctl.pending.as_ref().is_some_and(|p| p.id == request_id);
                    match code {
                        // Anything else is about an attach we already
                        // replaced.
                        ErrorCode::Resync if ours => {
                            ctl.queue.clear();
                            self.resync(state, pane, true);
                        }
                        ErrorCode::Resync => {}
                        ErrorCode::Busy
                            if ctl.pending.as_ref().is_some_and(|p| p.id == request_id) =>
                        {
                            self.resync(state, pane, true);
                        }
                        _ if ours => {
                            ctl.queue.push(PaneEvent::Error(code));
                        }
                        _ => {}
                    }
                }
            }
            DaemonFrame::Snapshot {
                pane,
                attach_id,
                seq,
                flags,
                bytes,
            } => {
                let Some(ctl) = state.panes.get_mut(&pane) else {
                    return Ok(());
                };
                let Some(pending) = ctl.pending.as_mut().filter(|p| p.id == attach_id) else {
                    return Ok(());
                };
                if seq != pending.next_seq || pending.complete {
                    self.resync(state, pane, false);
                    return Ok(());
                }
                if pending.snapshot.len() + bytes.len() > MAX_SNAPSHOT_TOTAL {
                    return Err(ProtocolError::TooLong("snapshot").into());
                }
                pending.snapshot.extend_from_slice(&bytes);
                pending.next_seq += 1;
                pending.complete = flags & FLAG_LAST != 0;
            }
            DaemonFrame::Ready {
                pane,
                attach_id,
                offset,
                rows,
                cols,
            } => {
                let Some(ctl) = state.panes.get_mut(&pane) else {
                    return Ok(());
                };
                if ctl.pending.as_ref().is_none_or(|p| p.id != attach_id) {
                    return Ok(());
                }
                let pending = ctl.pending.take().expect("checked above");
                if !pending.complete {
                    self.resync(state, pane, false);
                    return Ok(());
                }
                // A refresh that missed no byte keeps the local history.
                let keep_history = ctl.in_sync && ctl.expected == offset;
                ctl.current = Some(attach_id);
                ctl.in_sync = true;
                ctl.expected = offset;
                ctl.history_rows = 0;
                ctl.history_limit = pending.scrollback_rows;
                ctl.queue.push(PaneEvent::Restore {
                    snapshot: pending.snapshot,
                    offset,
                    rows,
                    cols,
                    keep_history,
                });
            }
            DaemonFrame::Output {
                pane,
                attach_id,
                offset,
                bytes,
            } => {
                let Some(ctl) = state.panes.get_mut(&pane) else {
                    return Ok(());
                };
                if ctl.current != Some(attach_id) || !ctl.in_sync {
                    return Ok(());
                }
                let end = offset + bytes.len() as u64;
                if end <= ctl.expected {
                    // Already reflected in the snapshot.
                    return Ok(());
                }
                if offset > ctl.expected {
                    // Gap: out of sync.
                    self.resync(state, pane, false);
                    return Ok(());
                }
                let skip = (ctl.expected - offset) as usize;
                let data = if skip == 0 {
                    bytes
                } else {
                    bytes[skip..].to_vec()
                };
                let event = PaneEvent::Bytes {
                    offset: ctl.expected,
                    data,
                };
                ctl.expected = end;
                if !ctl.queue.push(event) {
                    // The consumer is slower than the link: drop this
                    // pane's pending bytes and reattach.
                    ctl.queue.clear();
                    self.resync(state, pane, true);
                }
            }
            DaemonFrame::Resized {
                pane,
                attach_id,
                offset,
                rows,
                cols,
                pulled,
            } => {
                let Some(ctl) = state.panes.get_mut(&pane) else {
                    return Ok(());
                };
                if ctl.current != Some(attach_id) || !ctl.in_sync {
                    return Ok(());
                }
                if offset != ctl.expected {
                    self.resync(state, pane, false);
                    return Ok(());
                }
                ctl.queue.push(PaneEvent::Resize {
                    offset,
                    rows,
                    cols,
                    pulled,
                });
            }
            DaemonFrame::ScrollbackChunk {
                pane,
                attach_id,
                start,
                count,
                flags,
                bytes,
            } => {
                let Some(ctl) = state.panes.get_mut(&pane) else {
                    return Ok(());
                };
                if ctl.current != Some(attach_id) || !ctl.in_sync {
                    return Ok(());
                }
                if start != ctl.history_rows {
                    self.resync(state, pane, false);
                    return Ok(());
                }
                // More history than this attach asked for is a daemon that
                // does not follow the protocol: fail closed rather than
                // decode what was never requested.
                if ctl.history_rows.saturating_add(count as u32) > ctl.history_limit {
                    return Err(ProtocolError::InvalidValue("scrollback chunk").into());
                }
                ctl.history_rows += count as u32;
                ctl.queue.push(PaneEvent::History {
                    rows: bytes,
                    count,
                    last: flags & FLAG_LAST != 0,
                });
            }
            DaemonFrame::Exited {
                pane,
                offset,
                exit_code,
                signal,
            } => {
                let Some(ctl) = state.panes.get_mut(&pane) else {
                    return Ok(());
                };
                if !ctl.in_sync {
                    return Ok(());
                }
                if offset != ctl.expected {
                    self.resync(state, pane, false);
                    return Ok(());
                }
                ctl.queue.push(PaneEvent::Exited {
                    offset,
                    exit_code,
                    signal,
                });
            }
        }
        Ok(())
    }
}

/// A connection to a session daemon.
#[derive(Clone)]
pub struct Client {
    inner: Arc<Inner>,
}

impl Client {
    /// Handshake on a connected stream and start the reader and writer
    /// tasks. EOF before `HelloAck` is [`ClientError::Closed`]: the caller
    /// treats the daemon as not running and may start it once more.
    pub async fn connect<S>(stream: S, options: ClientOptions) -> Result<Self, ClientError>
    where
        S: AsyncRead + AsyncWrite + Send + Unpin + 'static,
    {
        Self::connect_inner(stream, options, FrameBuffer::new()).await
    }

    /// Like [`connect`](Self::connect) on the stdio of
    /// `shelldeck-sessiond proxy --hello <nonce>`: everything before the
    /// nonce marker (login banners, rc noise) is discarded, with a 64 KiB
    /// and handshake-timeout cap.
    pub async fn connect_via_proxy<S>(
        mut stream: S,
        nonce: &str,
        options: ClientOptions,
    ) -> Result<Self, ClientError>
    where
        S: AsyncRead + AsyncWrite + Send + Unpin + 'static,
    {
        let marker = protocol::proxy_marker(nonce).ok_or(ClientError::Prelude)?;
        let mut seen = Vec::new();
        let mut buf = [0u8; 4096];
        let leftover = tokio::time::timeout(options.hello_timeout, async {
            loop {
                let n = stream.read(&mut buf).await?;
                if n == 0 {
                    return Err(ClientError::Closed);
                }
                seen.extend_from_slice(&buf[..n]);
                if let Some(at) = find(&seen, &marker) {
                    return Ok(seen.split_off(at + marker.len()));
                }
                if seen.len() > MAX_PRELUDE_BYTES {
                    return Err(ClientError::Prelude);
                }
            }
        })
        .await
        .map_err(|_| ClientError::Timeout)??;
        let mut frames = FrameBuffer::new();
        frames.push(&leftover);
        Self::connect_inner(stream, options, frames).await
    }

    async fn connect_inner<S>(
        stream: S,
        options: ClientOptions,
        mut frames: FrameBuffer,
    ) -> Result<Self, ClientError>
    where
        S: AsyncRead + AsyncWrite + Send + Unpin + 'static,
    {
        let (mut reader, mut writer) = tokio::io::split(stream);
        let hello = ClientFrame::Hello(Hello {
            proto_min: PROTOCOL_VERSION,
            proto_max: PROTOCOL_VERSION,
            features: 0,
            client_version: options.client_version.clone(),
        })
        .encode()?;
        writer.write_all(&hello).await?;
        writer.flush().await?;

        let mut buf = vec![0u8; 64 * 1024];
        let ack = tokio::time::timeout(options.hello_timeout, async {
            loop {
                if let Some(payload) = frames.next_frame()? {
                    return match DaemonFrame::decode(payload)? {
                        DaemonFrame::HelloAck(ack) if ack.proto == PROTOCOL_VERSION => Ok(ack),
                        DaemonFrame::HelloAck(_) => {
                            Err(ClientError::Daemon(ErrorCode::UnsupportedVersion))
                        }
                        DaemonFrame::Error { code, .. } => Err(ClientError::Daemon(code)),
                        _ => Err(ProtocolError::InvalidValue("frame before HelloAck").into()),
                    };
                }
                let n = reader.read(&mut buf).await?;
                if n == 0 {
                    return Err(ClientError::Closed);
                }
                frames.push(&buf[..n]);
            }
        })
        .await
        .map_err(|_| ClientError::Timeout)??;

        let (out, mut out_rx) = mpsc::unbounded_channel::<Vec<u8>>();
        let out_bytes = Arc::new(AtomicUsize::new(0));
        let (released_tx, mut released) = oneshot::channel::<()>();
        let inner = Arc::new(Inner {
            hello: ack,
            out,
            out_bytes: out_bytes.clone(),
            closed: AtomicBool::new(false),
            state: Mutex::new(State {
                next_request: 1,
                next_attach: 1,
                spawns: HashMap::new(),
                lists: VecDeque::new(),
                panes: HashMap::new(),
            }),
            options,
            _released: released_tx,
        });

        let writer_inner = Arc::downgrade(&inner);
        tokio::spawn(async move {
            while let Some(frame) = out_rx.recv().await {
                let result = async {
                    writer.write_all(&frame).await?;
                    writer.flush().await
                }
                .await;
                out_bytes.fetch_sub(frame.len(), Ordering::Relaxed);
                if result.is_err() {
                    break;
                }
            }
            let _ = writer.shutdown().await;
            if let Some(inner) = writer_inner.upgrade() {
                inner.close();
            }
        });

        // The reader never stops reading: it only demultiplexes into the
        // per-pane queues, so one slow pane cannot stall the connection.
        let reader_inner = Arc::downgrade(&inner);
        tokio::spawn(async move {
            let result: Result<(), ClientError> = async {
                loop {
                    loop {
                        let Some(inner) = reader_inner.upgrade() else {
                            return Ok(());
                        };
                        match frames.next_frame()? {
                            Some(payload) => inner.dispatch(DaemonFrame::decode(payload)?)?,
                            None => break,
                        }
                    }
                    let n = tokio::select! {
                        _ = &mut released => return Ok(()),
                        read = reader.read(&mut buf) => read?,
                    };
                    if n == 0 {
                        return Ok(());
                    }
                    frames.push(&buf[..n]);
                }
            }
            .await;
            if let Err(error) = &result {
                tracing::debug!(%error, "session daemon connection ended");
            }
            if let Some(inner) = reader_inner.upgrade() {
                inner.close();
            }
        });

        Ok(Self { inner })
    }

    /// What the daemon announced in `HelloAck`.
    pub fn hello(&self) -> &HelloAck {
        &self.inner.hello
    }

    pub fn is_closed(&self) -> bool {
        self.inner.closed.load(Ordering::Relaxed)
    }

    /// Give the connection up: every attached pane's stream ends with
    /// `Closed` and pending requests fail. The transport is released when
    /// the last clone of the client is dropped. Panes keep running.
    pub fn close(&self) {
        self.inner.close();
    }

    pub async fn list_panes(&self) -> Result<Vec<PaneInfo>, ClientError> {
        let (tx, rx) = oneshot::channel();
        {
            // Replies carry no id: they are matched in request order, so the
            // waiter is queued under the same lock as the send.
            let mut state = self.inner.state.lock();
            self.inner.send(&ClientFrame::ListPanes)?;
            state.lists.push_back(tx);
        }
        rx.await.map_err(|_| ClientError::Closed)
    }

    pub async fn spawn(&self, options: SpawnOptions) -> Result<SpawnedPane, ClientError> {
        let (tx, rx) = oneshot::channel();
        {
            let mut state = self.inner.state.lock();
            let request_id = state.next_request;
            state.next_request = state.next_request.wrapping_add(1).max(1);
            self.inner.send(&ClientFrame::Spawn(Spawn {
                request_id,
                group: options.group,
                label: options.label,
                rows: options.rows,
                cols: options.cols,
                scrollback_max: options.scrollback_max,
                cwd: options.cwd,
                argv: options.argv,
                env: options.env,
            }))?;
            state.spawns.insert(request_id, tx);
        }
        rx.await.map_err(|_| ClientError::Closed)?
    }

    /// Attach to a pane. The first event is `Restore` (or `Error`), then
    /// live `Bytes`. Attaching a pane this client already has attached
    /// replaces the previous event stream.
    pub fn attach(&self, pane: Uuid, options: AttachOptions) -> Result<PaneEvents, ClientError> {
        if self.is_closed() {
            return Err(ClientError::Closed);
        }
        let queue = Arc::new(EventQueue::new(self.inner.options.max_feed_bytes));
        let mut state = self.inner.state.lock();
        let previous = state.panes.insert(
            pane,
            PaneCtl {
                queue: queue.clone(),
                rows: options.rows,
                cols: options.cols,
                scrollback_rows: options.scrollback_rows,
                read_only: options.read_only,
                current: None,
                in_sync: false,
                expected: 0,
                pending: None,
                history_rows: 0,
                history_limit: 0,
                backoff: BACKOFF_START,
                last_overload: None,
                last_resize: None,
                queued_resize: None,
                resize_timer: false,
            },
        );
        if let Some(previous) = previous {
            previous.queue.close();
        }
        self.inner.start_attach(&mut state, pane, None);
        Ok(PaneEvents { pane, queue })
    }

    /// Refresh a pane from the daemon's grid. `drop_history` forces a full
    /// resync (the consumer could not reproduce an applied resize);
    /// otherwise a refresh that missed no byte keeps the local history.
    pub fn reattach(&self, pane: Uuid, drop_history: bool) -> Result<(), ClientError> {
        let mut state = self.inner.state.lock();
        if !state.panes.contains_key(&pane) {
            return Err(ClientError::NotAttached);
        }
        if drop_history {
            self.inner.resync(&mut state, pane, false);
        } else {
            self.inner.start_attach(&mut state, pane, None);
        }
        Ok(())
    }

    /// Send input. A sequence that must not interleave with another
    /// client's typing (a key, a bracketed paste) should be one call of at
    /// most 64 KiB; longer input is split into several frames.
    pub fn input(&self, pane: Uuid, bytes: &[u8]) -> Result<(), ClientError> {
        for chunk in bytes.chunks(MAX_INPUT_BYTES) {
            self.inner.send(&ClientFrame::Input {
                pane,
                bytes: chunk.to_vec(),
            })?;
        }
        Ok(())
    }

    /// Request a pane size. At most one request per pane per 50 ms reaches
    /// the daemon (the latest size wins). The local grid must not be
    /// resized here: it follows the applied `Resize` event.
    pub fn resize(&self, pane: Uuid, rows: u16, cols: u16) -> Result<(), ClientError> {
        let mut state = self.inner.state.lock();
        let ctl = state.panes.get_mut(&pane).ok_or(ClientError::NotAttached)?;
        ctl.rows = rows;
        ctl.cols = cols;
        let due = ctl
            .last_resize
            .is_none_or(|at| at.elapsed() >= RESIZE_INTERVAL);
        if due {
            ctl.last_resize = Some(Instant::now());
            ctl.queued_resize = None;
            return self.inner.send(&ClientFrame::Resize { pane, rows, cols });
        }
        ctl.queued_resize = Some((rows, cols));
        if !ctl.resize_timer {
            ctl.resize_timer = true;
            let wait = RESIZE_INTERVAL
                .saturating_sub(ctl.last_resize.map(|at| at.elapsed()).unwrap_or_default());
            let inner = self.inner.clone();
            tokio::spawn(async move {
                tokio::time::sleep(wait).await;
                let mut state = inner.state.lock();
                let Some(ctl) = state.panes.get_mut(&pane) else {
                    return;
                };
                ctl.resize_timer = false;
                if let Some((rows, cols)) = ctl.queued_resize.take() {
                    ctl.last_resize = Some(Instant::now());
                    let _ = inner.send(&ClientFrame::Resize { pane, rows, cols });
                }
            });
        }
        Ok(())
    }

    /// Detach from a pane. The pane keeps running.
    pub fn detach(&self, pane: Uuid) -> Result<(), ClientError> {
        let ctl = self.inner.state.lock().panes.remove(&pane);
        match ctl {
            Some(ctl) => {
                ctl.queue.close();
                self.inner.send(&ClientFrame::Detach { pane })
            }
            None => Err(ClientError::NotAttached),
        }
    }

    /// Signal the pane child's process group (1 HUP, 15 TERM, 9 KILL). On a
    /// dead pane this removes the pane.
    pub fn kill(&self, pane: Uuid, signal: u8) -> Result<(), ClientError> {
        self.inner.send(&ClientFrame::Kill { pane, signal })
    }
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

/// Connect to the daemon serving `dir` after the run directory, socket and
/// peer checks. `Io(NotFound)` or `Io(ConnectionRefused)` mean "not
/// running".
#[cfg(unix)]
pub async fn connect_unix(
    dir: &std::path::Path,
    options: ClientOptions,
) -> Result<Client, ClientError> {
    let stream = crate::paths::connect_checked(dir)?;
    stream.set_nonblocking(true)?;
    let stream = tokio::net::UnixStream::from_std(stream)?;
    Client::connect(stream, options).await
}

// ---------------------------------------------------------------------------
// Headless client emulator
// ---------------------------------------------------------------------------

/// What the consumer must do after applying an event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MirrorOutcome {
    Applied,
    /// The applied resize cannot be reproduced locally (less history than
    /// the daemon pulled, or another emulator revision): call
    /// [`Client::reattach`] with `drop_history`.
    NeedsReattach,
}

/// A client-side emulator for one pane: its own grid and parser, fed with
/// [`PaneEvent`]s. The grid never answers terminal queries (no
/// `response_tx`): only the daemon's grid does.
pub struct PaneMirror {
    grid: Arc<Mutex<TerminalGrid>>,
    processor: TerminalProcessor,
    parser: vte::Parser,
    same_emulator: bool,
    /// Stream offset of the next byte this mirror expects.
    offset: u64,
    /// Out of step until the next `Restore`.
    stale: bool,
    exited: Option<(i32, u8)>,
}

impl PaneMirror {
    pub fn new(rows: u16, cols: u16, max_scrollback: usize, daemon_emulator_rev: u16) -> Self {
        let grid = Arc::new(Mutex::new(TerminalGrid::with_scrollback(
            rows as usize,
            cols as usize,
            max_scrollback,
        )));
        Self {
            processor: TerminalProcessor::new(grid.clone()),
            grid,
            parser: vte::Parser::new(),
            same_emulator: daemon_emulator_rev == shelldeck_terminal::EMULATOR_REV,
            offset: 0,
            stale: true,
            exited: None,
        }
    }

    pub fn grid(&self) -> &Arc<Mutex<TerminalGrid>> {
        &self.grid
    }

    pub fn offset(&self) -> u64 {
        self.offset
    }

    /// Whether the mirror reflects the stream up to [`offset`](Self::offset).
    pub fn in_sync(&self) -> bool {
        !self.stale
    }

    pub fn exited(&self) -> Option<(i32, u8)> {
        self.exited
    }

    pub fn apply(&mut self, event: &PaneEvent) -> Result<MirrorOutcome, SnapshotError> {
        match event {
            PaneEvent::Restore {
                snapshot,
                offset,
                keep_history,
                ..
            } => {
                let tail = self.grid.lock().restore(snapshot, *keep_history)?;
                // A fresh parser starts at ground; the tail puts it where
                // the daemon's parser was at the snapshot.
                self.parser = vte::Parser::new();
                self.processor.process_bytes(&mut self.parser, &tail);
                self.offset = *offset;
                self.stale = false;
                self.exited = None;
            }
            PaneEvent::Bytes { offset, data } => {
                if !self.stale {
                    debug_assert_eq!(*offset, self.offset);
                    self.processor.process_bytes(&mut self.parser, data);
                    self.offset = offset + data.len() as u64;
                }
            }
            PaneEvent::Resize {
                rows, cols, pulled, ..
            } => {
                if !self.stale {
                    let mut grid = self.grid.lock();
                    if !self.same_emulator || grid.scrollback_len() < *pulled as usize {
                        self.stale = true;
                        return Ok(MirrorOutcome::NeedsReattach);
                    }
                    grid.resize(*rows as usize, *cols as usize);
                }
            }
            PaneEvent::History { rows, count, .. } => {
                if !self.stale && *count > 0 {
                    self.grid.lock().prepend_history(rows)?;
                }
            }
            PaneEvent::Exited {
                exit_code, signal, ..
            } => {
                if !self.stale {
                    self.exited = Some((*exit_code, *signal));
                }
            }
            PaneEvent::Resyncing => self.stale = true,
            PaneEvent::Error(_) | PaneEvent::Closed => {}
        }
        Ok(MirrorOutcome::Applied)
    }
}
