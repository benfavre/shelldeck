//! Wire protocol v1 of the session daemon
//! (docs/design/session-daemon.md, section 4).
//!
//! Frame: `u32 BE length | u8 version (=1) | u8 type | body`, where `length`
//! counts version, type and body. Integers are big-endian, strings are
//! `u16 length + UTF-8`, blobs are `u32 length + bytes`, UUIDs are 16 raw
//! bytes. Every limit is checked on encode and on decode; decoding rejects
//! trailing bytes. There is no free text and no generic command field.

use uuid::Uuid;

pub const PROTOCOL_VERSION: u16 = 1;
pub const WIRE_VERSION: u8 = 1;
pub const MAGIC: [u8; 4] = *b"SDSD";

pub const MAX_FRAME_BYTES: usize = 1024 * 1024;
pub const MIN_FRAME_BYTES: usize = 2;
pub const MAX_HELLO_FRAME: usize = 128;
pub const MAX_SPAWN_FRAME: usize = 64 * 1024;
pub const MAX_ATTACH_FRAME: usize = 64;
pub const MAX_DETACH_FRAME: usize = 32;

pub const MAX_VERSION_STRING: usize = 64;
pub const MAX_LABEL: usize = 255;
pub const MAX_TITLE: usize = 255;
pub const MAX_CWD: usize = 4096;
pub const MAX_LIST_CWD: usize = 1024;
pub const MAX_ARGV: usize = 64;
pub const MAX_ARG: usize = 4096;
pub const MAX_ENV: usize = 32;
pub const MAX_ENV_NAME: usize = 64;
pub const MAX_ENV_VALUE: usize = 1024;
pub const MAX_DIMENSION: u16 = 1000;
pub const MAX_SCROLLBACK: u32 = 100_000;
pub const DEFAULT_SCROLLBACK: u32 = 10_000;

pub const MAX_INPUT_BYTES: usize = 64 * 1024;
pub const MAX_OUTPUT_BYTES: usize = 64 * 1024;
pub const MAX_SNAPSHOT_CHUNK: usize = 256 * 1024;
pub const MAX_SNAPSHOT_TOTAL: usize = 32 * 1024 * 1024;
pub const MAX_SCROLLBACK_CHUNK: usize = 256 * 1024;

pub const MAX_PANES: usize = 256;
/// `HelloAck::features` bit: the daemon answers `ListStatus` with the
/// status the program of each pane reported (OSC 7501). A daemon without
/// the bit closes the connection on that frame, so a client checks first.
pub const FEATURE_PANE_STATUS: u32 = 1;
/// Limits of the strings of a `PaneStatus`.
pub const MAX_STATUS_WORD: usize = 16;
pub const MAX_STATUS_APP: usize = 64;
pub const MAX_STATUS_TEXT: usize = 255;
pub const MAX_CONNECTIONS: usize = 32;
pub const MAX_ATTACHMENTS_PER_PANE: usize = 16;

pub const ATTACH_READ_ONLY: u8 = 1;
pub const FLAG_LAST: u8 = 1;

pub const SIGNAL_HUP: u8 = 1;
pub const SIGNAL_KILL: u8 = 9;
pub const SIGNAL_TERM: u8 = 15;

pub mod frame_type {
    pub const HELLO: u8 = 0x01;
    pub const LIST_PANES: u8 = 0x02;
    pub const SPAWN: u8 = 0x03;
    pub const ATTACH: u8 = 0x04;
    pub const INPUT: u8 = 0x05;
    pub const RESIZE: u8 = 0x06;
    pub const DETACH: u8 = 0x07;
    pub const KILL: u8 = 0x08;
    /// Only to a daemon that announced `FEATURE_PANE_STATUS`.
    pub const LIST_STATUS: u8 = 0x09;

    pub const HELLO_ACK: u8 = 0x81;
    pub const PANE_LIST: u8 = 0x82;
    pub const SPAWNED: u8 = 0x83;
    pub const SNAPSHOT: u8 = 0x84;
    pub const READY: u8 = 0x85;
    pub const OUTPUT: u8 = 0x86;
    pub const RESIZED: u8 = 0x87;
    pub const SCROLLBACK_CHUNK: u8 = 0x88;
    pub const EXITED: u8 = 0x89;
    pub const STATUS_LIST: u8 = 0x8a;
    pub const ERROR: u8 = 0xff;
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ProtocolError {
    #[error("frame length {0} is outside the allowed range")]
    FrameLength(usize),
    #[error("unsupported wire version {0}")]
    WireVersion(u8),
    #[error("unknown frame type {0:#04x}")]
    UnknownType(u8),
    #[error("frame is truncated")]
    Truncated,
    #[error("trailing bytes after frame body")]
    TrailingBytes,
    #[error("invalid UTF-8 in {0}")]
    InvalidUtf8(&'static str),
    #[error("{0} exceeds its limit")]
    TooLong(&'static str),
    #[error("invalid value for {0}")]
    InvalidValue(&'static str),
    #[error("environment variable name is not allowed")]
    DisallowedEnv,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u16)]
pub enum ErrorCode {
    BadFrame = 1,
    UnsupportedVersion = 2,
    NoSuchPane = 3,
    SpawnFailed = 4,
    Resync = 5,
    Busy = 6,
    LimitExceeded = 7,
    NotAttached = 8,
    ReadOnly = 9,
    Internal = 10,
}

impl ErrorCode {
    pub fn from_u16(value: u16) -> Option<Self> {
        Some(match value {
            1 => Self::BadFrame,
            2 => Self::UnsupportedVersion,
            3 => Self::NoSuchPane,
            4 => Self::SpawnFailed,
            5 => Self::Resync,
            6 => Self::Busy,
            7 => Self::LimitExceeded,
            8 => Self::NotAttached,
            9 => Self::ReadOnly,
            10 => Self::Internal,
            _ => return None,
        })
    }
}

/// Names a `Spawn` frame may set. Everything else comes from the daemon's
/// scrubbed environment, so a client cannot poison `PATH` or `LD_PRELOAD`.
pub fn env_name_allowed(name: &str) -> bool {
    matches!(
        name,
        "TERM"
            | "COLORTERM"
            | "LANG"
            | "TERM_PROGRAM"
            | "TERM_PROGRAM_VERSION"
            | "SSH_AUTH_SOCK"
            // What the pane's SSH connection was when it was spawned, for
            // prompts and rc files that ask "am I over SSH". Given by the
            // client per pane: the daemon's own copies are scrubbed.
            | "SSH_CONNECTION"
            | "SSH_CLIENT"
            | "DISPLAY"
            | "WAYLAND_DISPLAY"
            | "XAUTHORITY"
    ) || is_prefixed_name(name, "LC_")
        || is_prefixed_name(name, "SHELLDECK_")
}

fn is_prefixed_name(name: &str, prefix: &str) -> bool {
    name.strip_prefix(prefix).is_some_and(|rest| {
        !rest.is_empty()
            && rest
                .bytes()
                .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
    })
}

// ---------------------------------------------------------------------------
// Messages
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hello {
    pub proto_min: u16,
    pub proto_max: u16,
    pub features: u32,
    pub client_version: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Spawn {
    pub request_id: u32,
    pub group: Uuid,
    pub label: String,
    pub rows: u16,
    pub cols: u16,
    pub scrollback_max: u32,
    /// Empty means the user's home directory.
    pub cwd: String,
    /// Empty means the user's login shell, started as a login shell.
    pub argv: Vec<String>,
    pub env: Vec<(String, String)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attach {
    pub pane: Uuid,
    pub attach_id: u32,
    /// Size the client wants, or 0 x 0 to keep the pane's current size
    /// (the client does not know its view size yet). Such an attach never
    /// resizes the pane; the client's first `Resize` does.
    pub rows: u16,
    pub cols: u16,
    pub scrollback_rows: u32,
    pub flags: u8,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientFrame {
    Hello(Hello),
    ListPanes,
    ListStatus,
    Spawn(Spawn),
    Attach(Attach),
    Input { pane: Uuid, bytes: Vec<u8> },
    Resize { pane: Uuid, rows: u16, cols: u16 },
    Detach { pane: Uuid },
    Kill { pane: Uuid, signal: u8 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HelloAck {
    pub proto: u16,
    pub features: u32,
    pub daemon_version: String,
    pub instance: Uuid,
    pub max_frame: u32,
    pub emulator_rev: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaneInfo {
    pub pane: Uuid,
    pub group: Uuid,
    pub label: String,
    pub title: String,
    pub cwd: String,
    pub rows: u16,
    pub cols: u16,
    pub created_at: u64,
    pub alive: bool,
    pub exit_code: i32,
    pub clients: u16,
    pub offset: u64,
}

/// What the program of a pane last reported about itself (OSC 7501), as
/// the daemon's own terminal recorded it. Words, not codes, so a newer
/// daemon can report a state an older client only passes along.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaneStatus {
    pub pane: Uuid,
    /// `idle`, `working`, `done`, `blocked` or `error`.
    pub state: String,
    /// What a blocked program waits for (`permission`, `question`, `auth`),
    /// or empty.
    pub kind: String,
    /// 0..=100, or -1 when the program gave none.
    pub progress: i8,
    /// Machine name of the program (`claude-code`), or empty.
    pub app: String,
    /// One line for people, or empty.
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DaemonFrame {
    HelloAck(HelloAck),
    PaneList(Vec<PaneInfo>),
    /// Reply to `ListStatus`: only the panes whose program reported.
    StatusList(Vec<PaneStatus>),
    Spawned {
        request_id: u32,
        pane: Uuid,
        pid: u32,
        cwd: String,
    },
    Snapshot {
        pane: Uuid,
        attach_id: u32,
        seq: u32,
        flags: u8,
        bytes: Vec<u8>,
    },
    Ready {
        pane: Uuid,
        attach_id: u32,
        offset: u64,
        rows: u16,
        cols: u16,
    },
    Output {
        pane: Uuid,
        attach_id: u32,
        offset: u64,
        bytes: Vec<u8>,
    },
    Resized {
        pane: Uuid,
        attach_id: u32,
        offset: u64,
        rows: u16,
        cols: u16,
        pulled: u16,
    },
    ScrollbackChunk {
        pane: Uuid,
        attach_id: u32,
        start: u32,
        count: u16,
        flags: u8,
        bytes: Vec<u8>,
    },
    Exited {
        pane: Uuid,
        offset: u64,
        exit_code: i32,
        signal: u8,
    },
    Error {
        /// The request it answers, or the `attach_id` for a pane error that
        /// concerns one attachment (`Resync`); 0 if none.
        request_id: u32,
        pane: Uuid,
        code: ErrorCode,
        retryable: bool,
    },
}

// ---------------------------------------------------------------------------
// Encoder
// ---------------------------------------------------------------------------

struct Encoder {
    buf: Vec<u8>,
}

impl Encoder {
    fn new(frame_type: u8, capacity: usize) -> Self {
        let mut buf = Vec::with_capacity(capacity + 6);
        buf.extend_from_slice(&[0, 0, 0, 0, WIRE_VERSION, frame_type]);
        Self { buf }
    }

    fn u8(&mut self, v: u8) {
        self.buf.push(v);
    }
    fn u16(&mut self, v: u16) {
        self.buf.extend_from_slice(&v.to_be_bytes());
    }
    fn u32(&mut self, v: u32) {
        self.buf.extend_from_slice(&v.to_be_bytes());
    }
    fn u64(&mut self, v: u64) {
        self.buf.extend_from_slice(&v.to_be_bytes());
    }
    fn i32(&mut self, v: i32) {
        self.buf.extend_from_slice(&v.to_be_bytes());
    }
    fn uuid(&mut self, v: &Uuid) {
        self.buf.extend_from_slice(v.as_bytes());
    }

    fn string(&mut self, v: &str, max: usize, what: &'static str) -> Result<(), ProtocolError> {
        if v.len() > max || v.len() > u16::MAX as usize {
            return Err(ProtocolError::TooLong(what));
        }
        self.u16(v.len() as u16);
        self.buf.extend_from_slice(v.as_bytes());
        Ok(())
    }

    fn blob(&mut self, v: &[u8], max: usize, what: &'static str) -> Result<(), ProtocolError> {
        if v.len() > max {
            return Err(ProtocolError::TooLong(what));
        }
        self.u32(v.len() as u32);
        self.buf.extend_from_slice(v);
        Ok(())
    }

    fn finish(mut self, max_frame: usize) -> Result<Vec<u8>, ProtocolError> {
        let len = self.buf.len() - 4;
        if len > max_frame.min(MAX_FRAME_BYTES) {
            return Err(ProtocolError::FrameLength(len));
        }
        self.buf[..4].copy_from_slice(&(len as u32).to_be_bytes());
        Ok(self.buf)
    }
}

/// Longest prefix of `value` that fits `max` bytes on a char boundary.
fn truncated(value: &str, max: usize) -> &str {
    let mut end = value.len().min(max);
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    &value[..end]
}

fn check_dimension(value: u16, what: &'static str) -> Result<u16, ProtocolError> {
    if (1..=MAX_DIMENSION).contains(&value) {
        Ok(value)
    } else {
        Err(ProtocolError::InvalidValue(what))
    }
}

fn check_signal(signal: u8) -> Result<u8, ProtocolError> {
    match signal {
        SIGNAL_HUP | SIGNAL_TERM | SIGNAL_KILL => Ok(signal),
        _ => Err(ProtocolError::InvalidValue("signal")),
    }
}

impl Spawn {
    fn validate(&self) -> Result<(), ProtocolError> {
        check_dimension(self.rows, "rows")?;
        check_dimension(self.cols, "cols")?;
        if self.scrollback_max > MAX_SCROLLBACK {
            return Err(ProtocolError::InvalidValue("scrollback_max"));
        }
        if self.argv.len() > MAX_ARGV {
            return Err(ProtocolError::TooLong("argv"));
        }
        if self.argv.first().is_some_and(String::is_empty) {
            return Err(ProtocolError::InvalidValue("argv"));
        }
        if self.env.len() > MAX_ENV {
            return Err(ProtocolError::TooLong("env"));
        }
        if self.env.iter().any(|(name, _)| !env_name_allowed(name)) {
            return Err(ProtocolError::DisallowedEnv);
        }
        let has_nul = |s: &String| s.contains('\0');
        if has_nul(&self.cwd)
            || self.argv.iter().any(has_nul)
            || self.env.iter().any(|(_, value)| has_nul(value))
        {
            return Err(ProtocolError::InvalidValue("NUL byte"));
        }
        Ok(())
    }
}

impl ClientFrame {
    /// Encode to a complete frame, length prefix included.
    pub fn encode(&self) -> Result<Vec<u8>, ProtocolError> {
        match self {
            Self::Hello(hello) => {
                let mut e = Encoder::new(frame_type::HELLO, 64);
                e.buf.extend_from_slice(&MAGIC);
                e.u16(hello.proto_min);
                e.u16(hello.proto_max);
                e.u32(hello.features);
                e.string(&hello.client_version, MAX_VERSION_STRING, "client_version")?;
                e.finish(MAX_HELLO_FRAME)
            }
            Self::ListPanes => Encoder::new(frame_type::LIST_PANES, 0).finish(MIN_FRAME_BYTES),
            Self::ListStatus => Encoder::new(frame_type::LIST_STATUS, 0).finish(MIN_FRAME_BYTES),
            Self::Spawn(spawn) => {
                spawn.validate()?;
                let mut e = Encoder::new(frame_type::SPAWN, 256);
                e.u32(spawn.request_id);
                e.uuid(&spawn.group);
                e.string(&spawn.label, MAX_LABEL, "label")?;
                e.u16(spawn.rows);
                e.u16(spawn.cols);
                e.u32(spawn.scrollback_max);
                e.string(&spawn.cwd, MAX_CWD, "cwd")?;
                e.u8(spawn.argv.len() as u8);
                for arg in &spawn.argv {
                    e.string(arg, MAX_ARG, "argv")?;
                }
                e.u8(spawn.env.len() as u8);
                for (name, value) in &spawn.env {
                    e.string(name, MAX_ENV_NAME, "env name")?;
                    e.string(value, MAX_ENV_VALUE, "env value")?;
                }
                e.finish(MAX_SPAWN_FRAME)
            }
            Self::Attach(attach) => {
                if (attach.rows, attach.cols) != (0, 0) {
                    check_dimension(attach.rows, "rows")?;
                    check_dimension(attach.cols, "cols")?;
                }
                let mut e = Encoder::new(frame_type::ATTACH, 32);
                e.uuid(&attach.pane);
                e.u32(attach.attach_id);
                e.u16(attach.rows);
                e.u16(attach.cols);
                e.u32(attach.scrollback_rows);
                e.u8(attach.flags);
                e.finish(MAX_ATTACH_FRAME)
            }
            Self::Input { pane, bytes } => {
                let mut e = Encoder::new(frame_type::INPUT, bytes.len() + 20);
                e.uuid(pane);
                e.blob(bytes, MAX_INPUT_BYTES, "input")?;
                e.finish(MAX_FRAME_BYTES)
            }
            Self::Resize { pane, rows, cols } => {
                let mut e = Encoder::new(frame_type::RESIZE, 20);
                e.uuid(pane);
                e.u16(check_dimension(*rows, "rows")?);
                e.u16(check_dimension(*cols, "cols")?);
                e.finish(MAX_FRAME_BYTES)
            }
            Self::Detach { pane } => {
                let mut e = Encoder::new(frame_type::DETACH, 16);
                e.uuid(pane);
                e.finish(MAX_DETACH_FRAME)
            }
            Self::Kill { pane, signal } => {
                let mut e = Encoder::new(frame_type::KILL, 17);
                e.uuid(pane);
                e.u8(check_signal(*signal)?);
                e.finish(MAX_FRAME_BYTES)
            }
        }
    }

    /// Decode a frame payload (version byte, type byte, body).
    pub fn decode(payload: &[u8]) -> Result<Self, ProtocolError> {
        let (frame_type, mut d) = Decoder::open(payload)?;
        let frame = match frame_type {
            frame_type::HELLO => {
                d.limit(MAX_HELLO_FRAME)?;
                if d.take(4)? != MAGIC {
                    return Err(ProtocolError::InvalidValue("magic"));
                }
                Self::Hello(Hello {
                    proto_min: d.u16()?,
                    proto_max: d.u16()?,
                    features: d.u32()?,
                    client_version: d.string(MAX_VERSION_STRING, "client_version")?,
                })
            }
            frame_type::LIST_PANES => Self::ListPanes,
            frame_type::LIST_STATUS => Self::ListStatus,
            frame_type::SPAWN => {
                d.limit(MAX_SPAWN_FRAME)?;
                let request_id = d.u32()?;
                let group = d.uuid()?;
                let label = d.string(MAX_LABEL, "label")?;
                let rows = d.u16()?;
                let cols = d.u16()?;
                let scrollback_max = d.u32()?;
                let cwd = d.string(MAX_CWD, "cwd")?;
                let argc = d.u8()? as usize;
                if argc > MAX_ARGV {
                    return Err(ProtocolError::TooLong("argv"));
                }
                let mut argv = Vec::with_capacity(argc);
                for _ in 0..argc {
                    argv.push(d.string(MAX_ARG, "argv")?);
                }
                let envc = d.u8()? as usize;
                if envc > MAX_ENV {
                    return Err(ProtocolError::TooLong("env"));
                }
                let mut env = Vec::with_capacity(envc);
                for _ in 0..envc {
                    let name = d.string(MAX_ENV_NAME, "env name")?;
                    let value = d.string(MAX_ENV_VALUE, "env value")?;
                    env.push((name, value));
                }
                let spawn = Spawn {
                    request_id,
                    group,
                    label,
                    rows,
                    cols,
                    scrollback_max,
                    cwd,
                    argv,
                    env,
                };
                spawn.validate()?;
                Self::Spawn(spawn)
            }
            frame_type::ATTACH => {
                d.limit(MAX_ATTACH_FRAME)?;
                let pane = d.uuid()?;
                let attach_id = d.u32()?;
                // 0 x 0 means "keep the pane's current size".
                let (rows, cols) = match (d.u16()?, d.u16()?) {
                    (0, 0) => (0, 0),
                    (rows, cols) => (
                        check_dimension(rows, "rows")?,
                        check_dimension(cols, "cols")?,
                    ),
                };
                Self::Attach(Attach {
                    pane,
                    attach_id,
                    rows,
                    cols,
                    scrollback_rows: d.u32()?,
                    flags: match d.u8()? {
                        flags if flags & !ATTACH_READ_ONLY == 0 => flags,
                        _ => return Err(ProtocolError::InvalidValue("attach flags")),
                    },
                })
            }
            frame_type::INPUT => Self::Input {
                pane: d.uuid()?,
                bytes: d.blob(MAX_INPUT_BYTES, "input")?,
            },
            frame_type::RESIZE => Self::Resize {
                pane: d.uuid()?,
                rows: check_dimension(d.u16()?, "rows")?,
                cols: check_dimension(d.u16()?, "cols")?,
            },
            frame_type::DETACH => {
                d.limit(MAX_DETACH_FRAME)?;
                Self::Detach { pane: d.uuid()? }
            }
            frame_type::KILL => Self::Kill {
                pane: d.uuid()?,
                signal: check_signal(d.u8()?)?,
            },
            other => return Err(ProtocolError::UnknownType(other)),
        };
        d.finish()?;
        Ok(frame)
    }
}

impl DaemonFrame {
    /// Encode to a complete frame, length prefix included.
    pub fn encode(&self) -> Result<Vec<u8>, ProtocolError> {
        match self {
            Self::HelloAck(ack) => {
                let mut e = Encoder::new(frame_type::HELLO_ACK, 96);
                e.u16(ack.proto);
                e.u32(ack.features);
                e.string(&ack.daemon_version, MAX_VERSION_STRING, "daemon_version")?;
                e.uuid(&ack.instance);
                e.u32(ack.max_frame);
                e.u16(ack.emulator_rev);
                e.finish(MAX_FRAME_BYTES)
            }
            Self::PaneList(panes) => {
                if panes.len() > MAX_PANES {
                    return Err(ProtocolError::TooLong("pane list"));
                }
                let mut e = Encoder::new(frame_type::PANE_LIST, panes.len() * 128);
                e.u16(panes.len() as u16);
                for pane in panes {
                    e.uuid(&pane.pane);
                    e.uuid(&pane.group);
                    e.string(&pane.label, MAX_LABEL, "label")?;
                    e.string(truncated(&pane.title, MAX_TITLE), MAX_TITLE, "title")?;
                    e.string(truncated(&pane.cwd, MAX_LIST_CWD), MAX_LIST_CWD, "cwd")?;
                    e.u16(pane.rows);
                    e.u16(pane.cols);
                    e.u64(pane.created_at);
                    e.u8(u8::from(pane.alive));
                    e.i32(pane.exit_code);
                    e.u16(pane.clients);
                    e.u64(pane.offset);
                }
                e.finish(MAX_FRAME_BYTES)
            }
            Self::StatusList(statuses) => {
                if statuses.len() > MAX_PANES {
                    return Err(ProtocolError::TooLong("status list"));
                }
                let mut e = Encoder::new(frame_type::STATUS_LIST, statuses.len() * 64);
                e.u16(statuses.len() as u16);
                for status in statuses {
                    e.uuid(&status.pane);
                    e.string(&status.state, MAX_STATUS_WORD, "state")?;
                    e.string(&status.kind, MAX_STATUS_WORD, "kind")?;
                    e.u8(status.progress as u8);
                    e.string(
                        truncated(&status.app, MAX_STATUS_APP),
                        MAX_STATUS_APP,
                        "app",
                    )?;
                    e.string(
                        truncated(&status.text, MAX_STATUS_TEXT),
                        MAX_STATUS_TEXT,
                        "text",
                    )?;
                }
                e.finish(MAX_FRAME_BYTES)
            }
            Self::Spawned {
                request_id,
                pane,
                pid,
                cwd,
            } => {
                let mut e = Encoder::new(frame_type::SPAWNED, 64 + cwd.len());
                e.u32(*request_id);
                e.uuid(pane);
                e.u32(*pid);
                e.string(cwd, MAX_CWD, "cwd")?;
                e.finish(MAX_FRAME_BYTES)
            }
            Self::Snapshot {
                pane,
                attach_id,
                seq,
                flags,
                bytes,
            } => {
                let mut e = Encoder::new(frame_type::SNAPSHOT, bytes.len() + 32);
                e.uuid(pane);
                e.u32(*attach_id);
                e.u32(*seq);
                e.u8(*flags);
                e.blob(bytes, MAX_SNAPSHOT_CHUNK, "snapshot chunk")?;
                e.finish(MAX_FRAME_BYTES)
            }
            Self::Ready {
                pane,
                attach_id,
                offset,
                rows,
                cols,
            } => {
                let mut e = Encoder::new(frame_type::READY, 32);
                e.uuid(pane);
                e.u32(*attach_id);
                e.u64(*offset);
                e.u16(*rows);
                e.u16(*cols);
                e.finish(MAX_FRAME_BYTES)
            }
            Self::Output {
                pane,
                attach_id,
                offset,
                bytes,
            } => {
                let mut e = Encoder::new(frame_type::OUTPUT, bytes.len() + 32);
                e.uuid(pane);
                e.u32(*attach_id);
                e.u64(*offset);
                e.blob(bytes, MAX_OUTPUT_BYTES, "output")?;
                e.finish(MAX_FRAME_BYTES)
            }
            Self::Resized {
                pane,
                attach_id,
                offset,
                rows,
                cols,
                pulled,
            } => {
                let mut e = Encoder::new(frame_type::RESIZED, 36);
                e.uuid(pane);
                e.u32(*attach_id);
                e.u64(*offset);
                e.u16(*rows);
                e.u16(*cols);
                e.u16(*pulled);
                e.finish(MAX_FRAME_BYTES)
            }
            Self::ScrollbackChunk {
                pane,
                attach_id,
                start,
                count,
                flags,
                bytes,
            } => {
                let mut e = Encoder::new(frame_type::SCROLLBACK_CHUNK, bytes.len() + 36);
                e.uuid(pane);
                e.u32(*attach_id);
                e.u32(*start);
                e.u16(*count);
                e.u8(*flags);
                e.blob(bytes, MAX_SCROLLBACK_CHUNK, "scrollback chunk")?;
                e.finish(MAX_FRAME_BYTES)
            }
            Self::Exited {
                pane,
                offset,
                exit_code,
                signal,
            } => {
                let mut e = Encoder::new(frame_type::EXITED, 32);
                e.uuid(pane);
                e.u64(*offset);
                e.i32(*exit_code);
                e.u8(*signal);
                e.finish(MAX_FRAME_BYTES)
            }
            Self::Error {
                request_id,
                pane,
                code,
                retryable,
            } => {
                let mut e = Encoder::new(frame_type::ERROR, 24);
                e.u32(*request_id);
                e.uuid(pane);
                e.u16(*code as u16);
                e.u8(u8::from(*retryable));
                e.finish(MAX_FRAME_BYTES)
            }
        }
    }

    /// Decode a frame payload (version byte, type byte, body).
    pub fn decode(payload: &[u8]) -> Result<Self, ProtocolError> {
        let (frame_type, mut d) = Decoder::open(payload)?;
        let frame = match frame_type {
            frame_type::HELLO_ACK => Self::HelloAck(HelloAck {
                proto: d.u16()?,
                features: d.u32()?,
                daemon_version: d.string(MAX_VERSION_STRING, "daemon_version")?,
                instance: d.uuid()?,
                max_frame: d.u32()?,
                emulator_rev: d.u16()?,
            }),
            frame_type::PANE_LIST => {
                let count = d.u16()? as usize;
                if count > MAX_PANES {
                    return Err(ProtocolError::TooLong("pane list"));
                }
                let mut panes = Vec::with_capacity(count);
                for _ in 0..count {
                    panes.push(PaneInfo {
                        pane: d.uuid()?,
                        group: d.uuid()?,
                        label: d.string(MAX_LABEL, "label")?,
                        title: d.string(MAX_TITLE, "title")?,
                        cwd: d.string(MAX_LIST_CWD, "cwd")?,
                        rows: d.u16()?,
                        cols: d.u16()?,
                        created_at: d.u64()?,
                        alive: d.bool("alive")?,
                        exit_code: d.i32()?,
                        clients: d.u16()?,
                        offset: d.u64()?,
                    });
                }
                Self::PaneList(panes)
            }
            frame_type::STATUS_LIST => {
                let count = d.u16()? as usize;
                if count > MAX_PANES {
                    return Err(ProtocolError::TooLong("status list"));
                }
                let mut statuses = Vec::with_capacity(count);
                for _ in 0..count {
                    statuses.push(PaneStatus {
                        pane: d.uuid()?,
                        state: d.string(MAX_STATUS_WORD, "state")?,
                        kind: d.string(MAX_STATUS_WORD, "kind")?,
                        progress: d.u8()? as i8,
                        app: d.string(MAX_STATUS_APP, "app")?,
                        text: d.string(MAX_STATUS_TEXT, "text")?,
                    });
                }
                Self::StatusList(statuses)
            }
            frame_type::SPAWNED => Self::Spawned {
                request_id: d.u32()?,
                pane: d.uuid()?,
                pid: d.u32()?,
                cwd: d.string(MAX_CWD, "cwd")?,
            },
            frame_type::SNAPSHOT => Self::Snapshot {
                pane: d.uuid()?,
                attach_id: d.u32()?,
                seq: d.u32()?,
                flags: d.flags()?,
                bytes: d.blob(MAX_SNAPSHOT_CHUNK, "snapshot chunk")?,
            },
            frame_type::READY => Self::Ready {
                pane: d.uuid()?,
                attach_id: d.u32()?,
                offset: d.u64()?,
                rows: check_dimension(d.u16()?, "rows")?,
                cols: check_dimension(d.u16()?, "cols")?,
            },
            frame_type::OUTPUT => Self::Output {
                pane: d.uuid()?,
                attach_id: d.u32()?,
                offset: d.u64()?,
                bytes: d.blob(MAX_OUTPUT_BYTES, "output")?,
            },
            frame_type::RESIZED => Self::Resized {
                pane: d.uuid()?,
                attach_id: d.u32()?,
                offset: d.u64()?,
                rows: check_dimension(d.u16()?, "rows")?,
                cols: check_dimension(d.u16()?, "cols")?,
                pulled: d.u16()?,
            },
            frame_type::SCROLLBACK_CHUNK => Self::ScrollbackChunk {
                pane: d.uuid()?,
                attach_id: d.u32()?,
                start: d.u32()?,
                count: d.u16()?,
                flags: d.flags()?,
                bytes: d.blob(MAX_SCROLLBACK_CHUNK, "scrollback chunk")?,
            },
            frame_type::EXITED => Self::Exited {
                pane: d.uuid()?,
                offset: d.u64()?,
                exit_code: d.i32()?,
                signal: d.u8()?,
            },
            frame_type::ERROR => Self::Error {
                request_id: d.u32()?,
                pane: d.uuid()?,
                code: ErrorCode::from_u16(d.u16()?)
                    .ok_or(ProtocolError::InvalidValue("error code"))?,
                retryable: d.bool("retryable")?,
            },
            other => return Err(ProtocolError::UnknownType(other)),
        };
        d.finish()?;
        Ok(frame)
    }
}

/// Encode an `Output` frame without building a [`DaemonFrame`] (hot path of
/// the daemon's PTY reader).
pub fn encode_output(pane: &Uuid, attach_id: u32, offset: u64, bytes: &[u8]) -> Vec<u8> {
    debug_assert!(bytes.len() <= MAX_OUTPUT_BYTES);
    let mut e = Encoder::new(frame_type::OUTPUT, bytes.len() + 32);
    e.uuid(pane);
    e.u32(attach_id);
    e.u64(offset);
    e.u32(bytes.len() as u32);
    e.buf.extend_from_slice(bytes);
    let len = e.buf.len() - 4;
    e.buf[..4].copy_from_slice(&(len as u32).to_be_bytes());
    e.buf
}

// ---------------------------------------------------------------------------
// Decoder
// ---------------------------------------------------------------------------

/// Bounded typed decoder over one frame payload.
struct Decoder<'a> {
    payload: &'a [u8],
    pos: usize,
}

impl<'a> Decoder<'a> {
    fn open(payload: &'a [u8]) -> Result<(u8, Self), ProtocolError> {
        if payload.len() < MIN_FRAME_BYTES || payload.len() > MAX_FRAME_BYTES {
            return Err(ProtocolError::FrameLength(payload.len()));
        }
        if payload[0] != WIRE_VERSION {
            return Err(ProtocolError::WireVersion(payload[0]));
        }
        Ok((payload[1], Self { payload, pos: 2 }))
    }

    /// Per-type frame cap.
    fn limit(&self, max: usize) -> Result<(), ProtocolError> {
        if self.payload.len() > max {
            Err(ProtocolError::FrameLength(self.payload.len()))
        } else {
            Ok(())
        }
    }

    fn take(&mut self, len: usize) -> Result<&'a [u8], ProtocolError> {
        let end = self.pos.checked_add(len).ok_or(ProtocolError::Truncated)?;
        let bytes = self
            .payload
            .get(self.pos..end)
            .ok_or(ProtocolError::Truncated)?;
        self.pos = end;
        Ok(bytes)
    }

    fn u8(&mut self) -> Result<u8, ProtocolError> {
        Ok(self.take(1)?[0])
    }

    fn bool(&mut self, what: &'static str) -> Result<bool, ProtocolError> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(ProtocolError::InvalidValue(what)),
        }
    }

    fn flags(&mut self) -> Result<u8, ProtocolError> {
        match self.u8()? {
            flags if flags & !FLAG_LAST == 0 => Ok(flags),
            _ => Err(ProtocolError::InvalidValue("flags")),
        }
    }

    fn u16(&mut self) -> Result<u16, ProtocolError> {
        let b = self.take(2)?;
        Ok(u16::from_be_bytes([b[0], b[1]]))
    }

    fn u32(&mut self) -> Result<u32, ProtocolError> {
        let b = self.take(4)?;
        Ok(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn i32(&mut self) -> Result<i32, ProtocolError> {
        Ok(self.u32()? as i32)
    }

    fn u64(&mut self) -> Result<u64, ProtocolError> {
        let mut raw = [0u8; 8];
        raw.copy_from_slice(self.take(8)?);
        Ok(u64::from_be_bytes(raw))
    }

    fn uuid(&mut self) -> Result<Uuid, ProtocolError> {
        let mut raw = [0u8; 16];
        raw.copy_from_slice(self.take(16)?);
        Ok(Uuid::from_bytes(raw))
    }

    fn string(&mut self, max: usize, what: &'static str) -> Result<String, ProtocolError> {
        let len = self.u16()? as usize;
        if len > max {
            return Err(ProtocolError::TooLong(what));
        }
        let bytes = self.take(len)?;
        String::from_utf8(bytes.to_vec()).map_err(|_| ProtocolError::InvalidUtf8(what))
    }

    fn blob(&mut self, max: usize, what: &'static str) -> Result<Vec<u8>, ProtocolError> {
        let len = self.u32()? as usize;
        if len > max {
            return Err(ProtocolError::TooLong(what));
        }
        Ok(self.take(len)?.to_vec())
    }

    fn finish(self) -> Result<(), ProtocolError> {
        if self.pos == self.payload.len() {
            Ok(())
        } else {
            Err(ProtocolError::TrailingBytes)
        }
    }
}

// ---------------------------------------------------------------------------
// Incremental framing
// ---------------------------------------------------------------------------

/// Incremental frame splitter. It holds at most one frame plus whatever the
/// caller pushed in one read, and rejects an out-of-range length as soon as
/// the four length bytes are known, before the payload is buffered.
#[derive(Debug, Default)]
pub struct FrameBuffer {
    buf: Vec<u8>,
    start: usize,
}

impl FrameBuffer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, data: &[u8]) {
        if self.start > 0 && self.start == self.buf.len() {
            self.buf.clear();
            self.start = 0;
        } else if self.start > 64 * 1024 {
            self.buf.drain(..self.start);
            self.start = 0;
        }
        self.buf.extend_from_slice(data);
    }

    /// Bytes of an incomplete frame waiting for the rest.
    pub fn pending(&self) -> usize {
        self.buf.len() - self.start
    }

    /// Payload length the next frame declares, once its four length bytes
    /// are buffered. Lets a reader refuse a frame before the peer has sent
    /// (or trickled) all of it.
    pub fn declared_len(&self) -> Option<usize> {
        let available = self.buf.get(self.start..self.start + 4)?;
        Some(u32::from_be_bytes([available[0], available[1], available[2], available[3]]) as usize)
    }

    /// Next complete frame payload (version byte, type byte, body).
    pub fn next_frame(&mut self) -> Result<Option<&[u8]>, ProtocolError> {
        let available = &self.buf[self.start..];
        if available.len() < 4 {
            return Ok(None);
        }
        let len =
            u32::from_be_bytes([available[0], available[1], available[2], available[3]]) as usize;
        if !(MIN_FRAME_BYTES..=MAX_FRAME_BYTES).contains(&len) {
            return Err(ProtocolError::FrameLength(len));
        }
        if available.len() < 4 + len {
            return Ok(None);
        }
        let begin = self.start + 4;
        self.start = begin + len;
        Ok(Some(&self.buf[begin..begin + len]))
    }
}

// ---------------------------------------------------------------------------
// Remote proxy prelude
// ---------------------------------------------------------------------------

fn nonce_is_valid(nonce: &str) -> bool {
    nonce.len() == 32
        && nonce
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// The marker `shelldeck-sessiond proxy --hello <nonce>` writes before the
/// stream becomes protocol v1. `None` unless the nonce is 32 lowercase hex
/// characters.
pub fn proxy_marker(nonce: &str) -> Option<Vec<u8>> {
    nonce_is_valid(nonce).then(|| format!("\x1eSHELLDECK_SESSIOND_{nonce}_READY\x1f").into_bytes())
}

/// The fixed remote command of the SSH transport. Only the app version and
/// the nonce are interpolated, both checked against a strict alphabet.
pub fn proxy_command(version: &str, nonce: &str) -> Option<String> {
    let version_ok = (1..=32).contains(&version.len())
        && version
            .bytes()
            .all(|b| b.is_ascii_digit() || b.is_ascii_lowercase() || b == b'.' || b == b'-');
    (version_ok && nonce_is_valid(nonce)).then(|| {
        format!("exec \"$HOME/.shelldeck/bin/shelldeck-sessiond-{version}\" proxy --hello {nonce}")
    })
}

/// Longest accepted configured binary path of [`proxy_command_at`].
pub const MAX_PROXY_BINARY_PATH: usize = 512;

/// Like [`proxy_command`] for a daemon installed at a configured absolute
/// path (administrator install, tests). The path is configuration, never
/// data from the remote host or a session: it must be absolute, free of
/// `.` and `..` components and made only of `A-Z a-z 0-9 . _ + - /`, so
/// nothing in it can be shell syntax. It is single-quoted all the same.
pub fn proxy_command_at(binary: &str, nonce: &str) -> Option<String> {
    let path_ok = binary.starts_with('/')
        && binary.len() <= MAX_PROXY_BINARY_PATH
        && !binary.ends_with('/')
        && binary
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'+' | b'-' | b'/'))
        && binary
            .split('/')
            .skip(1)
            .all(|part| !part.is_empty() && part != "." && part != "..");
    (path_ok && nonce_is_valid(nonce)).then(|| format!("exec '{binary}' proxy --hello {nonce}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn payload(frame: &[u8]) -> &[u8] {
        let len = u32::from_be_bytes([frame[0], frame[1], frame[2], frame[3]]) as usize;
        assert_eq!(
            len,
            frame.len() - 4,
            "length prefix counts version, type and body"
        );
        &frame[4..]
    }

    fn sample_spawn() -> Spawn {
        Spawn {
            request_id: 7,
            group: Uuid::from_u128(0x1111),
            label: "build".into(),
            rows: 24,
            cols: 80,
            scrollback_max: 5000,
            cwd: "/srv/app".into(),
            argv: vec!["/bin/sh".into(), "-c".into(), "echo \u{e9}".into()],
            env: vec![
                ("TERM".into(), "xterm-256color".into()),
                ("LC_ALL".into(), "fr_FR.UTF-8".into()),
                ("SHELLDECK_PANE".into(), "1".into()),
            ],
        }
    }

    fn client_frames() -> Vec<ClientFrame> {
        let pane = Uuid::from_u128(0xabcdef);
        vec![
            ClientFrame::Hello(Hello {
                proto_min: 1,
                proto_max: 1,
                features: 0x8000_0001,
                client_version: "0.20.0".into(),
            }),
            ClientFrame::ListPanes,
            ClientFrame::Spawn(sample_spawn()),
            ClientFrame::Attach(Attach {
                pane,
                attach_id: 3,
                rows: 50,
                cols: 200,
                scrollback_rows: 10_000,
                flags: ATTACH_READ_ONLY,
            }),
            // 0 x 0: keep the pane's current size.
            ClientFrame::Attach(Attach {
                pane,
                attach_id: 4,
                rows: 0,
                cols: 0,
                scrollback_rows: 0,
                flags: 0,
            }),
            ClientFrame::Input {
                pane,
                bytes: vec![0, 1, 2, 0xff, 0x1b],
            },
            ClientFrame::Resize {
                pane,
                rows: 1,
                cols: 1000,
            },
            ClientFrame::Detach { pane },
            ClientFrame::Kill {
                pane,
                signal: SIGNAL_TERM,
            },
            ClientFrame::ListStatus,
        ]
    }

    fn daemon_frames() -> Vec<DaemonFrame> {
        let pane = Uuid::from_u128(0xabcdef);
        vec![
            DaemonFrame::HelloAck(HelloAck {
                proto: 1,
                features: 0,
                daemon_version: "0.20.0".into(),
                instance: Uuid::from_u128(42),
                max_frame: MAX_FRAME_BYTES as u32,
                emulator_rev: 1,
            }),
            DaemonFrame::PaneList(vec![PaneInfo {
                pane,
                group: Uuid::from_u128(9),
                label: "l".into(),
                title: "vim".into(),
                cwd: "/tmp".into(),
                rows: 24,
                cols: 80,
                created_at: 1_800_000_000,
                alive: false,
                exit_code: -1,
                clients: 2,
                offset: u64::MAX - 1,
            }]),
            DaemonFrame::Spawned {
                request_id: 7,
                pane,
                pid: 4242,
                cwd: "/srv/app".into(),
            },
            DaemonFrame::Snapshot {
                pane,
                attach_id: 3,
                seq: 1,
                flags: FLAG_LAST,
                bytes: vec![1, 2, 3],
            },
            DaemonFrame::Ready {
                pane,
                attach_id: 3,
                offset: 1 << 40,
                rows: 24,
                cols: 80,
            },
            DaemonFrame::Output {
                pane,
                attach_id: 3,
                offset: 99,
                bytes: b"\x1b[1mhi".to_vec(),
            },
            DaemonFrame::Resized {
                pane,
                attach_id: 3,
                offset: 100,
                rows: 30,
                cols: 90,
                pulled: 6,
            },
            DaemonFrame::ScrollbackChunk {
                pane,
                attach_id: 3,
                start: 256,
                count: 2,
                flags: 0,
                bytes: vec![9; 10],
            },
            DaemonFrame::Exited {
                pane,
                offset: 100,
                exit_code: 3,
                signal: 15,
            },
            DaemonFrame::Error {
                request_id: 7,
                pane: Uuid::nil(),
                code: ErrorCode::SpawnFailed,
                retryable: true,
            },
            DaemonFrame::StatusList(vec![
                PaneStatus {
                    pane,
                    state: "blocked".into(),
                    kind: "permission".into(),
                    progress: -1,
                    app: "claude-code".into(),
                    text: "Allow the edit?".into(),
                },
                PaneStatus {
                    pane,
                    state: "working".into(),
                    kind: String::new(),
                    progress: 40,
                    app: String::new(),
                    text: String::new(),
                },
            ]),
        ]
    }

    // SDTEST-2966
    #[test]
    fn sdtest_2966_every_frame_type_round_trips() {
        for frame in client_frames() {
            let encoded = frame.encode().expect("encode");
            assert_eq!(ClientFrame::decode(payload(&encoded)), Ok(frame));
        }
        for frame in daemon_frames() {
            let encoded = frame.encode().expect("encode");
            assert_eq!(DaemonFrame::decode(payload(&encoded)), Ok(frame));
        }
        // The wire layout is a contract: pin one frame byte for byte.
        let kill = ClientFrame::Kill {
            pane: Uuid::from_u128(1),
            signal: SIGNAL_KILL,
        };
        let mut expected = vec![0, 0, 0, 19, 1, 0x08];
        expected.extend_from_slice(&[0; 15]);
        expected.extend_from_slice(&[1, 9]);
        assert_eq!(kill.encode().expect("encode"), expected);
        // The hot-path encoder matches the typed one.
        let pane = Uuid::from_u128(5);
        assert_eq!(
            encode_output(&pane, 2, 77, b"abc"),
            DaemonFrame::Output {
                pane,
                attach_id: 2,
                offset: 77,
                bytes: b"abc".to_vec()
            }
            .encode()
            .expect("encode")
        );
    }

    // SDTEST-2967
    #[test]
    fn sdtest_2967_frames_fail_closed_on_direction_version_type_and_trailing_bytes() {
        let detach = ClientFrame::Detach {
            pane: Uuid::from_u128(1),
        }
        .encode()
        .expect("encode");
        let body = payload(&detach).to_vec();

        // A client frame is not a daemon frame and the reverse.
        assert_eq!(
            DaemonFrame::decode(&body),
            Err(ProtocolError::UnknownType(frame_type::DETACH))
        );
        let ready = daemon_frames()[4].encode().expect("encode");
        assert_eq!(
            ClientFrame::decode(payload(&ready)),
            Err(ProtocolError::UnknownType(frame_type::READY))
        );

        let mut wrong_version = body.clone();
        wrong_version[0] = 2;
        assert_eq!(
            ClientFrame::decode(&wrong_version),
            Err(ProtocolError::WireVersion(2))
        );

        let mut unknown = body.clone();
        unknown[1] = 0x42;
        assert_eq!(
            ClientFrame::decode(&unknown),
            Err(ProtocolError::UnknownType(0x42))
        );

        let mut trailing = body.clone();
        trailing.push(0);
        assert_eq!(
            ClientFrame::decode(&trailing),
            Err(ProtocolError::TrailingBytes)
        );
        assert_eq!(
            ClientFrame::decode(&body[..body.len() - 1]),
            Err(ProtocolError::Truncated)
        );
        assert_eq!(
            ClientFrame::decode(&[WIRE_VERSION]),
            Err(ProtocolError::FrameLength(1))
        );
        // Every truncation of every frame is an error, never a panic.
        for frame in client_frames() {
            let encoded = frame.encode().expect("encode");
            let body = payload(&encoded);
            for cut in 0..body.len() {
                assert!(ClientFrame::decode(&body[..cut]).is_err());
            }
        }
        for frame in daemon_frames() {
            let encoded = frame.encode().expect("encode");
            let body = payload(&encoded);
            for cut in 0..body.len() {
                assert!(DaemonFrame::decode(&body[..cut]).is_err());
            }
        }
    }

    // SDTEST-2968
    #[test]
    fn sdtest_2968_oversize_length_is_rejected_before_the_payload_is_buffered() {
        let mut buffer = FrameBuffer::new();
        buffer.push(&((MAX_FRAME_BYTES as u32 + 1).to_be_bytes()));
        assert_eq!(
            buffer.next_frame().map(|f| f.map(<[u8]>::to_vec)),
            Err(ProtocolError::FrameLength(MAX_FRAME_BYTES + 1))
        );
        let mut buffer = FrameBuffer::new();
        buffer.push(&1u32.to_be_bytes());
        assert!(matches!(
            buffer.next_frame(),
            Err(ProtocolError::FrameLength(1))
        ));

        // Frames split at arbitrary points come out whole and in order.
        let frames: Vec<Vec<u8>> = client_frames()
            .iter()
            .map(|f| f.encode().expect("encode"))
            .collect();
        let stream: Vec<u8> = frames.concat();
        for chunk in [1usize, 3, 7, 4096] {
            let mut buffer = FrameBuffer::new();
            let mut decoded = Vec::new();
            for piece in stream.chunks(chunk) {
                buffer.push(piece);
                while let Some(body) = buffer.next_frame().expect("frame") {
                    decoded.push(ClientFrame::decode(body).expect("decode"));
                }
            }
            assert_eq!(decoded, client_frames());
            assert_eq!(buffer.pending(), 0);
        }

        // Encoding refuses what decoding would refuse.
        let too_big = ClientFrame::Input {
            pane: Uuid::nil(),
            bytes: vec![0; MAX_INPUT_BYTES + 1],
        };
        assert_eq!(too_big.encode(), Err(ProtocolError::TooLong("input")));
        let big_output = DaemonFrame::Output {
            pane: Uuid::nil(),
            attach_id: 1,
            offset: 0,
            bytes: vec![0; MAX_OUTPUT_BYTES + 1],
        };
        assert_eq!(big_output.encode(), Err(ProtocolError::TooLong("output")));
    }

    fn spawn_body(mutate: impl FnOnce(&mut Spawn)) -> Result<ClientFrame, ProtocolError> {
        // Build the body by hand so that encode-side validation cannot hide
        // a missing decode-side check.
        let mut spawn = sample_spawn();
        mutate(&mut spawn);
        let mut body = vec![WIRE_VERSION, frame_type::SPAWN];
        body.extend_from_slice(&spawn.request_id.to_be_bytes());
        body.extend_from_slice(spawn.group.as_bytes());
        let put = |body: &mut Vec<u8>, s: &str| {
            body.extend_from_slice(&(s.len() as u16).to_be_bytes());
            body.extend_from_slice(s.as_bytes());
        };
        put(&mut body, &spawn.label);
        body.extend_from_slice(&spawn.rows.to_be_bytes());
        body.extend_from_slice(&spawn.cols.to_be_bytes());
        body.extend_from_slice(&spawn.scrollback_max.to_be_bytes());
        put(&mut body, &spawn.cwd);
        body.push(spawn.argv.len() as u8);
        for arg in &spawn.argv {
            put(&mut body, arg);
        }
        body.push(spawn.env.len() as u8);
        for (name, value) in &spawn.env {
            put(&mut body, name);
            put(&mut body, value);
        }
        ClientFrame::decode(&body)
    }

    // SDTEST-2969
    #[test]
    fn sdtest_2969_spawn_rejects_disallowed_env_and_out_of_range_fields() {
        assert_eq!(spawn_body(|_| {}), Ok(ClientFrame::Spawn(sample_spawn())));
        for name in ["SSH_CONNECTION", "SSH_CLIENT", "SSH_AUTH_SOCK", "LC_ALL"] {
            assert!(
                spawn_body(|s| s.env.push((name.into(), "v".into()))).is_ok(),
                "{name}"
            );
        }
        for name in [
            "SSH_TTY",
            "PATH",
            "LD_PRELOAD",
            "LC_",
            "lc_all",
            "SHELLDECK_x",
            "TERMINFO",
            "",
        ] {
            assert_eq!(
                spawn_body(|s| s.env.push((name.into(), "v".into()))),
                Err(ProtocolError::DisallowedEnv),
                "{name}"
            );
        }
        assert_eq!(
            spawn_body(|s| s.rows = 0),
            Err(ProtocolError::InvalidValue("rows"))
        );
        assert_eq!(
            spawn_body(|s| s.cols = 1001),
            Err(ProtocolError::InvalidValue("cols"))
        );
        assert_eq!(
            spawn_body(|s| s.scrollback_max = MAX_SCROLLBACK + 1),
            Err(ProtocolError::InvalidValue("scrollback_max"))
        );
        assert_eq!(
            spawn_body(|s| s.label = "x".repeat(MAX_LABEL + 1)),
            Err(ProtocolError::TooLong("label"))
        );
        assert_eq!(
            spawn_body(|s| s.argv = vec!["a".into(); MAX_ARGV + 1]),
            Err(ProtocolError::TooLong("argv"))
        );
        assert_eq!(
            spawn_body(|s| s.env = vec![("TERM".into(), "x".into()); MAX_ENV + 1]),
            Err(ProtocolError::TooLong("env"))
        );
        assert_eq!(
            spawn_body(|s| s.env[0].1 = "x".repeat(MAX_ENV_VALUE + 1)),
            Err(ProtocolError::TooLong("env value"))
        );
        // 64 arguments of 4096 bytes fit the field limits, not the frame cap.
        assert!(matches!(
            spawn_body(|s| s.argv = vec!["a".repeat(MAX_ARG); 32]),
            Err(ProtocolError::FrameLength(_))
        ));

        // Invalid UTF-8 in a string field.
        let mut hello = ClientFrame::Hello(Hello {
            proto_min: 1,
            proto_max: 1,
            features: 0,
            client_version: "ab".into(),
        })
        .encode()
        .expect("encode");
        let last = hello.len() - 1;
        hello[last] = 0xff;
        assert_eq!(
            ClientFrame::decode(payload(&hello)),
            Err(ProtocolError::InvalidUtf8("client_version"))
        );
        // Wrong magic, bad signal, unknown error code.
        hello[6] = b'X';
        assert_eq!(
            ClientFrame::decode(payload(&hello)),
            Err(ProtocolError::InvalidValue("magic"))
        );
        assert_eq!(
            ClientFrame::Kill {
                pane: Uuid::nil(),
                signal: 2
            }
            .encode(),
            Err(ProtocolError::InvalidValue("signal"))
        );
        let mut error = daemon_frames()[9].encode().expect("encode");
        let at = error.len() - 2;
        error[at] = 99;
        assert_eq!(
            DaemonFrame::decode(payload(&error)),
            Err(ProtocolError::InvalidValue("error code"))
        );
    }

    // SDTEST-2970
    #[test]
    fn sdtest_2970_proxy_command_interpolates_only_a_checked_version_and_nonce() {
        let nonce = "0123456789abcdef0123456789abcdef";
        assert_eq!(
            proxy_command("0.20.0", nonce).as_deref(),
            Some(
                "exec \"$HOME/.shelldeck/bin/shelldeck-sessiond-0.20.0\" proxy --hello \
                 0123456789abcdef0123456789abcdef"
            )
        );
        assert_eq!(
            proxy_marker(nonce).expect("marker"),
            b"\x1eSHELLDECK_SESSIOND_0123456789abcdef0123456789abcdef_READY\x1f"
        );
        for bad in [
            "",
            "0.20.0; rm -rf ~",
            "0.20.0\"",
            "V1",
            "$(id)",
            &"1".repeat(33),
        ] {
            assert_eq!(proxy_command(bad, nonce), None, "{bad}");
        }
        for bad in [
            "",
            "0123456789ABCDEF0123456789ABCDEF",
            "0123456789abcdef0123456789abcde",
            "0123456789abcdef0123456789abcdef0",
            "0123456789abcdef0123456789abcde;",
        ] {
            assert_eq!(proxy_command("0.20.0", bad), None, "{bad}");
            assert_eq!(proxy_marker(bad), None, "{bad}");
        }
    }

    // SDTEST-2986
    #[test]
    fn sdtest_2986_proxy_command_at_accepts_only_a_plain_absolute_path() {
        let nonce = "0123456789abcdef0123456789abcdef";
        assert_eq!(
            proxy_command_at("/opt/shelldeck/bin/shelldeck-sessiond-0.20.0", nonce).as_deref(),
            Some(
                "exec '/opt/shelldeck/bin/shelldeck-sessiond-0.20.0' proxy --hello \
                 0123456789abcdef0123456789abcdef"
            )
        );
        let long = format!("/{}", "a".repeat(MAX_PROXY_BINARY_PATH));
        for bad in [
            "",
            "/",
            "relative/shelldeck-sessiond",
            "~/bin/shelldeck-sessiond",
            "/opt/../bin/sh",
            "/opt/./sessiond",
            "/opt//sessiond",
            "/opt/sessiond/",
            "/opt/it's",
            "/opt/a b",
            "/opt/$HOME",
            "/opt/a;id",
            "/opt/a\nb",
            "/opt/\u{e9}",
            long.as_str(),
        ] {
            assert_eq!(proxy_command_at(bad, nonce), None, "{bad:?}");
        }
        assert_eq!(proxy_command_at("/opt/sessiond", "short"), None);
    }
}
