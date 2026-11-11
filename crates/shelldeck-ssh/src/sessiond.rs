//! Remote transport of the session daemon (docs/design/session-daemon.md,
//! section 8): one extra exec channel on an existing SSH connection runs
//! `shelldeck-sessiond proxy`, a stdio pipe to the per-user Unix socket on
//! the remote host. No port is opened and no PTY is requested, so the
//! channel is 8-bit clean.
//!
//! The remote command is fixed: only a checked app version or a checked
//! configured absolute path, and a random nonce, are placed in it
//! ([`shelldeck_sessiond::protocol::proxy_command`],
//! [`shelldeck_sessiond::protocol::proxy_command_at`]). Nothing from a
//! session, a host entry or the remote side is interpolated.
//!
//! [`connect`] returns a typed error. [`SessiondConnectError::is_unavailable`]
//! tells the caller that this host cannot run the daemon (binary absent or
//! not executable, exec refused): the tmux mode is the fallback for it.

use crate::session::SshSession;
use crate::SshError;
use russh::client;
use russh::{ChannelMsg, ChannelReadHalf, ChannelWriteHalf};
use shelldeck_sessiond::client::{Client, ClientError, ClientOptions};
use shelldeck_sessiond::protocol::{self, ErrorCode};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::oneshot;
use uuid::Uuid;

/// Bytes accepted before the nonce marker (login banner, rc noise).
const MAX_PRELUDE_BYTES: usize = 64 * 1024;
/// Remote stderr kept for classifying a failed start.
const MAX_STDERR_BYTES: usize = 8 * 1024;
/// EOF and Close each get this long when the channel is released.
const CLOSE_TIMEOUT: Duration = Duration::from_secs(2);
/// After the remote EOF: how long to wait for the exit status and Close.
const EXIT_STATUS_GRACE: Duration = Duration::from_secs(2);
/// Buffer between the SSH channel and the daemon client. The client reader
/// never stops reading (it only demultiplexes into bounded per-pane
/// queues), so this buffer does not stall the SSH connection loop.
const STREAM_BUFFER_BYTES: usize = 256 * 1024;
/// Largest write handed to russh in one call.
const WRITE_CHUNK_BYTES: usize = 32 * 1024;

/// Which remote executable the proxy command starts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RemoteBinary {
    /// `$HOME/.shelldeck/bin/shelldeck-sessiond-<version>` (the per-user
    /// install location). The version is the app version, `0-9 a-z . -`.
    Versioned(String),
    /// A configured absolute path, for an administrator install. Only
    /// `A-Z a-z 0-9 . _ + - /` without `.` or `..` components is accepted.
    Absolute(String),
}

impl RemoteBinary {
    /// The binary matching this build of the app.
    pub fn for_this_build() -> Self {
        Self::Versioned(env!("CARGO_PKG_VERSION").to_string())
    }

    fn command(&self, nonce: &str) -> Option<String> {
        match self {
            Self::Versioned(version) => protocol::proxy_command(version, nonce),
            Self::Absolute(path) => protocol::proxy_command_at(path, nonce),
        }
    }
}

#[derive(Debug, Clone)]
pub struct SessiondConnectOptions {
    pub binary: RemoteBinary,
    /// From the exec request to the nonce marker. The daemon handshake
    /// that follows has its own limit in `client.hello_timeout`.
    pub startup_timeout: Duration,
    pub client: ClientOptions,
}

impl Default for SessiondConnectOptions {
    fn default() -> Self {
        Self {
            binary: RemoteBinary::for_this_build(),
            startup_timeout: Duration::from_secs(10),
            client: ClientOptions::default(),
        }
    }
}

/// Why the proxy itself refused to serve (`SDSD-ERR <code>` on stderr).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProxyRefusal {
    /// The remote run directory or socket has the wrong type, owner or mode.
    PathCheck,
    /// The remote socket is served by another user.
    PeerCheck,
    /// The daemon was not running and could not be started.
    StartFailed,
}

impl ProxyRefusal {
    fn from_code(code: u8) -> Option<Self> {
        match code {
            1 => Some(Self::PathCheck),
            2 => Some(Self::PeerCheck),
            3 => Some(Self::StartFailed),
            _ => None,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SessiondConnectError {
    /// The configured version or path is not acceptable in the fixed
    /// command. Nothing was sent to the remote host.
    #[error("session daemon binary setting is not valid")]
    InvalidBinary,
    /// The remote shell found no such file (exit status 127).
    #[error("session daemon is not installed on the remote host")]
    BinaryMissing,
    /// The remote file exists but cannot be executed (exit status 126),
    /// for example a binary for another architecture or a noexec mount.
    #[error("session daemon binary cannot be executed on the remote host")]
    BinaryNotExecutable,
    /// The server refused the exec request (forced command, restricted
    /// account).
    #[error("the SSH server refused to run the session daemon proxy")]
    ExecRefused,
    #[error("session daemon proxy refused: {0:?}")]
    ProxyRefused(ProxyRefusal),
    /// The proxy ended before its marker for another reason.
    #[error("session daemon proxy exited before it was ready (status {exit_status:?})")]
    ProxyExited { exit_status: Option<u32> },
    /// More than 64 KiB arrived without the nonce marker.
    #[error("session daemon proxy sent no valid marker")]
    Prelude,
    /// The remote daemon does not speak this client's protocol version:
    /// the installed binary is from an incompatible release.
    #[error("the remote session daemon speaks another protocol version")]
    VersionMismatch,
    #[error("the session daemon proxy did not start in time")]
    Timeout,
    #[error(transparent)]
    Ssh(#[from] SshError),
    #[error(transparent)]
    Client(ClientError),
}

impl SessiondConnectError {
    /// This host cannot run the session daemon as configured. The caller
    /// falls back to the tmux mode without reporting a failure.
    pub fn is_unavailable(&self) -> bool {
        matches!(
            self,
            Self::BinaryMissing | Self::BinaryNotExecutable | Self::ExecRefused
        )
    }
}

/// How the exec channel of a connected tunnel ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TunnelEnd {
    /// The local side dropped or shut down the client.
    LocalClose,
    /// The remote proxy ended and the channel closed in order. The exit
    /// status is absent when the server sent none.
    RemoteExit { exit_status: Option<u32> },
    /// The remote side wrote to stderr after the marker. The channel was
    /// closed: treat the daemon transport of this host as failed.
    UnexpectedStderr,
    /// The SSH connection went away without a channel EOF or Close.
    ConnectionLost,
}

/// A daemon client running over an SSH exec channel.
///
/// Dropping every clone of `client` releases the channel: the remote proxy
/// gets EOF and Close and exits, and the panes keep running in the daemon.
pub struct SessiondConnection {
    pub client: Client,
    /// Resolves once when the channel is gone. `ConnectionLost` means the
    /// SSH connection must be re-established before a new [`connect`];
    /// `RemoteExit` means only the proxy or the daemon ended.
    pub ended: oneshot::Receiver<TunnelEnd>,
}

/// Releases the remote process when a connect is cancelled or a tunnel
/// task is dropped: a dropped channel half alone sends neither EOF nor
/// Close.
struct CloseGuard(Option<Arc<ChannelWriteHalf<client::Msg>>>);

impl CloseGuard {
    async fn close_now(&mut self) {
        if let Some(writer) = self.0.take() {
            bounded_close(&writer).await;
        }
    }
}

impl Drop for CloseGuard {
    fn drop(&mut self) {
        if let (Some(writer), Ok(runtime)) = (self.0.take(), tokio::runtime::Handle::try_current())
        {
            runtime.spawn(async move { bounded_close(&writer).await });
        }
    }
}

async fn bounded_close(writer: &ChannelWriteHalf<client::Msg>) {
    let _ = tokio::time::timeout(CLOSE_TIMEOUT, writer.eof()).await;
    let _ = tokio::time::timeout(CLOSE_TIMEOUT, writer.close()).await;
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

/// `SDSD-ERR <code>` on a line of its own, anywhere in the remote stderr.
fn proxy_refusal(stderr: &[u8]) -> Option<ProxyRefusal> {
    stderr
        .split(|byte| *byte == b'\n')
        .filter_map(|line| line.strip_prefix(b"SDSD-ERR "))
        .filter_map(|code| std::str::from_utf8(code).ok()?.trim().parse::<u8>().ok())
        .find_map(ProxyRefusal::from_code)
}

/// What the channel said before the proxy marker.
#[derive(Default)]
struct Startup {
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    exit_status: Option<u32>,
    /// EOF, Close or an exit status was seen: the channel ended in order.
    terminated: bool,
}

impl Startup {
    fn failure(&self) -> SessiondConnectError {
        if let Some(refusal) = proxy_refusal(&self.stderr) {
            return SessiondConnectError::ProxyRefused(refusal);
        }
        match self.exit_status {
            Some(127) => SessiondConnectError::BinaryMissing,
            Some(126) => SessiondConnectError::BinaryNotExecutable,
            None if !self.terminated => SshError::SessionClosed.into(),
            exit_status => SessiondConnectError::ProxyExited { exit_status },
        }
    }
}

/// Read until the nonce marker and return the bytes after it. An EOF does
/// not end the wait: OpenSSH may send it before the exit status, and the
/// status is what tells a missing binary from any other early exit.
async fn await_marker(
    reader: &mut ChannelReadHalf,
    marker: &[u8],
) -> Result<Vec<u8>, SessiondConnectError> {
    let mut startup = Startup::default();
    loop {
        match reader.wait().await {
            Some(ChannelMsg::Data { data }) => {
                // Only the new bytes and a marker-sized overlap are searched.
                let from = startup.stdout.len().saturating_sub(marker.len());
                startup.stdout.extend_from_slice(&data);
                if let Some(at) = startup
                    .stdout
                    .get(from..)
                    .and_then(|tail| find(tail, marker))
                {
                    return Ok(startup.stdout.split_off(from + at + marker.len()));
                }
                if startup.stdout.len() > MAX_PRELUDE_BYTES {
                    return Err(SessiondConnectError::Prelude);
                }
            }
            Some(ChannelMsg::ExtendedData { data, .. }) => {
                let room = MAX_STDERR_BYTES.saturating_sub(startup.stderr.len());
                startup
                    .stderr
                    .extend_from_slice(data.get(..room.min(data.len())).unwrap_or_default());
            }
            Some(ChannelMsg::ExitStatus { exit_status }) => {
                startup.exit_status = Some(exit_status);
                startup.terminated = true;
            }
            Some(ChannelMsg::Eof) => startup.terminated = true,
            Some(ChannelMsg::Failure) => return Err(SessiondConnectError::ExecRefused),
            Some(ChannelMsg::Close) => {
                startup.terminated = true;
                return Err(startup.failure());
            }
            None => return Err(startup.failure()),
            Some(_) => {}
        }
    }
}

/// Channel to client direction. Returns how the remote side ended, or
/// `None` when the local side is gone.
async fn pump_inbound(
    reader: &mut ChannelReadHalf,
    mut to_client: tokio::io::WriteHalf<tokio::io::DuplexStream>,
) -> Option<TunnelEnd> {
    let mut exit_status = None;
    let mut terminated = false;
    loop {
        let message = if terminated {
            // The stream is over; wait a bounded time for the exit status
            // and Close, which may follow the EOF.
            match tokio::time::timeout(EXIT_STATUS_GRACE, reader.wait()).await {
                Ok(message) => message,
                Err(_) => break,
            }
        } else {
            reader.wait().await
        };
        match message {
            Some(ChannelMsg::Data { data }) => {
                if to_client.write_all(&data).await.is_err() {
                    return None;
                }
            }
            Some(ChannelMsg::ExtendedData { .. }) => {
                // The proxy writes nothing to stderr once it is ready, so
                // this is not the program the marker promised: fail closed.
                let _ = to_client.shutdown().await;
                return Some(TunnelEnd::UnexpectedStderr);
            }
            Some(ChannelMsg::ExitStatus {
                exit_status: status,
            }) => {
                exit_status = Some(status);
                terminated = true;
            }
            Some(ChannelMsg::Eof) => {
                terminated = true;
                // The client sees the end of the stream now, not after the
                // grace period.
                let _ = to_client.shutdown().await;
            }
            Some(ChannelMsg::Close) => {
                terminated = true;
                break;
            }
            None => break,
            Some(_) => {}
        }
    }
    let _ = to_client.shutdown().await;
    Some(if terminated {
        TunnelEnd::RemoteExit { exit_status }
    } else {
        TunnelEnd::ConnectionLost
    })
}

/// Client to channel direction. Returns when the local side closed its
/// write half or went away, or the channel no longer accepts data.
async fn pump_outbound(
    mut from_client: tokio::io::ReadHalf<tokio::io::DuplexStream>,
    writer: &ChannelWriteHalf<client::Msg>,
) {
    let mut buf = vec![0u8; WRITE_CHUNK_BYTES];
    loop {
        match from_client.read(&mut buf).await {
            Ok(0) | Err(_) => return,
            Ok(n) => {
                // Waits on the peer window: a remote that stops reading
                // throttles only this direction of this channel.
                let chunk = buf.get(..n).unwrap_or_default();
                if writer.data(chunk).await.is_err() {
                    return;
                }
            }
        }
    }
}

async fn run_tunnel(
    mut reader: ChannelReadHalf,
    mut guard: CloseGuard,
    writer: Arc<ChannelWriteHalf<client::Msg>>,
    stream: tokio::io::DuplexStream,
    ended: oneshot::Sender<TunnelEnd>,
) {
    let (from_client, to_client) = tokio::io::split(stream);
    let end = {
        let inbound = pump_inbound(&mut reader, to_client);
        let outbound = pump_outbound(from_client, &writer);
        tokio::pin!(inbound);
        tokio::pin!(outbound);
        // Both directions run concurrently: a write blocked on the peer
        // window never stops the channel from being drained.
        tokio::select! {
            end = &mut inbound => end.unwrap_or(TunnelEnd::LocalClose),
            _ = &mut outbound => TunnelEnd::LocalClose,
        }
    };
    guard.close_now().await;
    let _ = ended.send(end);
}

/// Open an exec channel on `session` running the session daemon proxy and
/// connect a daemon client through it.
///
/// Must be called inside a Tokio runtime. Cancelling the returned future
/// releases the remote process.
pub async fn connect(
    session: &SshSession,
    options: SessiondConnectOptions,
) -> Result<SessiondConnection, SessiondConnectError> {
    // 32 lowercase hex characters, fresh for every channel: a banner or an
    // rc file cannot contain the marker by accident or on purpose.
    let nonce = Uuid::new_v4().simple().to_string();
    let command = options
        .binary
        .command(&nonce)
        .ok_or(SessiondConnectError::InvalidBinary)?;
    let marker = protocol::proxy_marker(&nonce).ok_or(SessiondConnectError::InvalidBinary)?;

    let channel = {
        // The connection handle is locked only to open the channel.
        let handle = session.shared_handle();
        let handle = handle.lock().await;
        handle
            .channel_open_session()
            .await
            .map_err(|error| SshError::Channel(error.to_string()))?
    };
    let (mut reader, writer) = channel.split();
    let writer = Arc::new(writer);
    let mut guard = CloseGuard(Some(writer.clone()));

    let startup = tokio::time::timeout(options.startup_timeout, async {
        writer
            .exec(true, command)
            .await
            .map_err(|error| SshError::Channel(error.to_string()))?;
        await_marker(&mut reader, &marker).await
    })
    .await;
    let leftover = match startup {
        Ok(Ok(leftover)) => leftover,
        Ok(Err(error)) => {
            guard.close_now().await;
            return Err(error);
        }
        Err(_) => {
            guard.close_now().await;
            return Err(SessiondConnectError::Timeout);
        }
    };

    let (client_side, mut tunnel_side) = tokio::io::duplex(STREAM_BUFFER_BYTES);
    // Bytes that followed the marker in the same packet. The proxy only
    // relays, so before the client's Hello this is normally empty.
    let seeded =
        leftover.len() <= STREAM_BUFFER_BYTES && tunnel_side.write_all(&leftover).await.is_ok();
    if !seeded {
        guard.close_now().await;
        return Err(SessiondConnectError::Prelude);
    }
    let (ended_tx, ended) = oneshot::channel();
    tokio::spawn(run_tunnel(reader, guard, writer, tunnel_side, ended_tx));

    // A failed handshake drops `client_side`, which ends the tunnel task
    // and closes the channel.
    let client =
        Client::connect(client_side, options.client)
            .await
            .map_err(|error| match error {
                ClientError::Daemon(ErrorCode::UnsupportedVersion) => {
                    SessiondConnectError::VersionMismatch
                }
                ClientError::Timeout => SessiondConnectError::Timeout,
                other => SessiondConnectError::Client(other),
            })?;
    Ok(SessiondConnection { client, ended })
}

#[cfg(all(test, unix))]
#[path = "sessiond_tests.rs"]
mod tests;
