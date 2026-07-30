use crate::handler::{ClientHandler, ForwardedTcpIpEvent, SshEvent};
use crate::SshError;
use chrono::{DateTime, Utc};
use russh::client;
use russh::{Channel, ChannelMsg, ChannelReadHalf, ChannelWriteHalf};
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};
use uuid::Uuid;

/// Shared handle type used by both SshSession and tunnel tasks.
pub type SharedHandle = Arc<Mutex<client::Handle<ClientHandler>>>;

pub struct SshSession {
    pub connection_id: Uuid,
    pub connected_at: DateTime<Utc>,
    handle: SharedHandle,
    event_rx: mpsc::UnboundedReceiver<SshEvent>,
    forwarded_tcpip_rx: Option<mpsc::UnboundedReceiver<ForwardedTcpIpEvent>>,
    /// When connected via ProxyJump, this holds the jump host session to keep it alive.
    /// Dropping this will tear down the jump connection (and thus the tunnel).
    _jump_session: Option<Box<SshSession>>,
}

pub struct ExecResult {
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub exit_code: Option<u32>,
}

impl ExecResult {
    /// Get stdout as a UTF-8 string, lossy.
    pub fn stdout_string(&self) -> String {
        String::from_utf8_lossy(&self.stdout).into_owned()
    }

    /// Get stderr as a UTF-8 string, lossy.
    pub fn stderr_string(&self) -> String {
        String::from_utf8_lossy(&self.stderr).into_owned()
    }

    /// Whether the command exited with code 0.
    pub fn success(&self) -> bool {
        self.exit_code == Some(0)
    }
}

impl SshSession {
    pub fn new(
        connection_id: Uuid,
        handle: client::Handle<ClientHandler>,
        event_rx: mpsc::UnboundedReceiver<SshEvent>,
        forwarded_tcpip_rx: mpsc::UnboundedReceiver<ForwardedTcpIpEvent>,
    ) -> Self {
        Self {
            connection_id,
            connected_at: Utc::now(),
            handle: Arc::new(Mutex::new(handle)),
            event_rx,
            forwarded_tcpip_rx: Some(forwarded_tcpip_rx),
            _jump_session: None,
        }
    }

    /// Create a session that was established via a ProxyJump.
    /// The `jump_session` is kept alive so that the underlying tunnel channel
    /// remains open for the duration of this session.
    pub fn new_with_jump(
        connection_id: Uuid,
        handle: client::Handle<ClientHandler>,
        event_rx: mpsc::UnboundedReceiver<SshEvent>,
        forwarded_tcpip_rx: mpsc::UnboundedReceiver<ForwardedTcpIpEvent>,
        jump_session: SshSession,
    ) -> Self {
        Self {
            connection_id,
            connected_at: Utc::now(),
            handle: Arc::new(Mutex::new(handle)),
            event_rx,
            forwarded_tcpip_rx: Some(forwarded_tcpip_rx),
            _jump_session: Some(Box::new(jump_session)),
        }
    }

    /// Open an interactive shell channel with PTY.
    pub async fn open_shell(&self, rows: u32, cols: u32) -> crate::Result<SshChannel> {
        let handle = self.handle.lock().await;
        let channel = handle
            .channel_open_session()
            .await
            .map_err(|e| SshError::Channel(e.to_string()))?;

        // Request PTY
        channel
            .request_pty(
                false, // don't want explicit reply
                "xterm-256color",
                cols,
                rows,
                0,
                0,   // pixel dimensions
                &[], // terminal modes
            )
            .await
            .map_err(|e| SshError::Channel(format!("PTY request failed: {}", e)))?;

        // Request shell
        channel
            .request_shell(false)
            .await
            .map_err(|e| SshError::Channel(format!("Shell request failed: {}", e)))?;

        Ok(SshChannel { channel })
    }

    /// Execute a command and collect the full result.
    pub async fn exec(&self, command: &str) -> crate::Result<ExecResult> {
        let handle = self.handle.lock().await;
        let mut channel = handle
            .channel_open_session()
            .await
            .map_err(|e| SshError::Channel(e.to_string()))?;

        channel
            .exec(true, command)
            .await
            .map_err(|e| SshError::Channel(e.to_string()))?;

        // Drop the handle lock before reading - we don't need it anymore
        drop(handle);

        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let mut exit_code = None;

        loop {
            match channel.wait().await {
                Some(ChannelMsg::Data { data }) => {
                    stdout.extend_from_slice(&data);
                }
                Some(ChannelMsg::ExtendedData { data, ext: 1 }) => {
                    stderr.extend_from_slice(&data);
                }
                Some(ChannelMsg::ExitStatus { exit_status }) => {
                    exit_code = Some(exit_status);
                }
                Some(ChannelMsg::Eof) | None => break,
                _ => {}
            }
        }

        Ok(ExecResult {
            stdout,
            stderr,
            exit_code,
        })
    }

    /// Execute a command with streaming output.
    /// Data (both stdout and stderr) is sent through `output_tx` as it arrives.
    /// Returns the exit code when the command finishes.
    pub async fn exec_streaming(
        &self,
        command: &str,
        output_tx: mpsc::UnboundedSender<Vec<u8>>,
    ) -> crate::Result<Option<u32>> {
        let handle = self.handle.lock().await;
        let mut channel = handle
            .channel_open_session()
            .await
            .map_err(|e| SshError::Channel(e.to_string()))?;

        channel
            .exec(true, command)
            .await
            .map_err(|e| SshError::Channel(e.to_string()))?;

        // Drop the handle lock before reading
        drop(handle);

        let mut exit_code = None;

        loop {
            match channel.wait().await {
                Some(ChannelMsg::Data { data }) => {
                    let _ = output_tx.send(data.to_vec());
                }
                Some(ChannelMsg::ExtendedData { data, .. }) => {
                    let _ = output_tx.send(data.to_vec());
                }
                Some(ChannelMsg::ExitStatus { exit_status }) => {
                    exit_code = Some(exit_status);
                }
                Some(ChannelMsg::Eof) | None => break,
                _ => {}
            }
        }

        Ok(exit_code)
    }

    /// Execute a command with streaming output and cancellation support.
    /// Data is sent through `output_tx` as it arrives.
    /// If a message is received on `shutdown_rx`, the SSH channel is closed.
    /// Returns the exit code when the command finishes (or None if cancelled).
    pub async fn exec_cancellable(
        &self,
        command: &str,
        output_tx: mpsc::UnboundedSender<Vec<u8>>,
        mut shutdown_rx: mpsc::Receiver<()>,
    ) -> crate::Result<Option<u32>> {
        let handle = self.handle.lock().await;
        let mut channel = handle
            .channel_open_session()
            .await
            .map_err(|e| SshError::Channel(e.to_string()))?;

        channel
            .exec(true, command)
            .await
            .map_err(|e| SshError::Channel(e.to_string()))?;

        // Drop the handle lock before reading
        drop(handle);

        let mut exit_code = None;

        loop {
            tokio::select! {
                msg = channel.wait() => {
                    match msg {
                        Some(ChannelMsg::Data { data }) => {
                            let _ = output_tx.send(data.to_vec());
                        }
                        Some(ChannelMsg::ExtendedData { data, .. }) => {
                            let _ = output_tx.send(data.to_vec());
                        }
                        Some(ChannelMsg::ExitStatus { exit_status }) => {
                            exit_code = Some(exit_status);
                        }
                        Some(ChannelMsg::Eof) | None => break,
                        _ => {}
                    }
                }
                _ = shutdown_rx.recv() => {
                    // Cancellation requested — close the SSH channel
                    let _ = channel.eof().await;
                    break;
                }
            }
        }

        Ok(exit_code)
    }

    /// Disconnect the session gracefully.
    pub async fn disconnect(&self) -> crate::Result<()> {
        let handle = self.handle.lock().await;
        handle
            .disconnect(
                russh::Disconnect::ByApplication,
                "ShellDeck disconnect",
                "en",
            )
            .await
            .map_err(|e| SshError::Russh(e.to_string()))
    }

    /// Get a clone of the shared handle for use with tunnels or other operations.
    pub fn shared_handle(&self) -> SharedHandle {
        self.handle.clone()
    }

    /// Get a mutable reference to the event receiver for handler-level events.
    pub fn event_rx(&mut self) -> &mut mpsc::UnboundedReceiver<SshEvent> {
        &mut self.event_rx
    }

    /// Take the forwarded TCP/IP event receiver out of this session.
    ///
    /// This is used by `TunnelManager::start_remote_forward` to receive
    /// server-initiated forwarded-tcpip channels. Returns `None` if the
    /// receiver has already been taken.
    pub fn take_forwarded_tcpip_rx(
        &mut self,
    ) -> Option<mpsc::UnboundedReceiver<ForwardedTcpIpEvent>> {
        self.forwarded_tcpip_rx.take()
    }
}

pub struct SshChannel {
    channel: Channel<client::Msg>,
}

impl SshChannel {
    /// Write data to the channel (keyboard input).
    /// The `data` method on Channel takes `impl AsyncRead + Unpin`,
    /// so we wrap the byte slice in a Cursor.
    pub async fn write(&self, data: &[u8]) -> crate::Result<()> {
        let cursor = std::io::Cursor::new(data.to_vec());
        self.channel
            .data(cursor)
            .await
            .map_err(|e| SshError::Channel(e.to_string()))
    }

    /// Wait for the next message from the channel.
    pub async fn read(&mut self) -> Option<ChannelMsg> {
        self.channel.wait().await
    }

    /// Request terminal window size change.
    pub async fn resize(&self, rows: u32, cols: u32) -> crate::Result<()> {
        self.channel
            .window_change(cols, rows, 0, 0)
            .await
            .map_err(|e| SshError::Channel(e.to_string()))
    }

    /// Send EOF to the channel, signalling that no more input will be sent.
    pub async fn eof(&self) -> crate::Result<()> {
        self.channel
            .eof()
            .await
            .map_err(|e| SshError::Channel(e.to_string()))
    }

    /// Consume the channel into an AsyncRead + AsyncWrite stream.
    /// Useful for integrating with tokio::io::copy or similar utilities.
    pub fn into_stream(self) -> russh::ChannelStream<client::Msg> {
        self.channel.into_stream()
    }

    /// Get the underlying channel for advanced operations.
    pub fn into_inner(self) -> Channel<client::Msg> {
        self.channel
    }

    /// Split the channel for concurrent reading, writing, and resizing.
    ///
    /// Returns independent read and write/control halves that can be moved to
    /// separate tasks.
    pub fn split(self) -> (SshChannelReader, SshChannelWriter) {
        let (read_half, write_half) = self.channel.split();
        (
            SshChannelReader {
                channel: read_half,
                saw_exit: false,
            },
            SshChannelWriter {
                channel: write_half,
            },
        )
    }
}

/// Read handle for an SSH channel.
pub struct SshChannelReader {
    channel: ChannelReadHalf,
    saw_exit: bool,
}

/// Write and terminal-control handle for an SSH channel.
pub struct SshChannelWriter {
    channel: ChannelWriteHalf<client::Msg>,
}

/// Result from reading the SSH channel.
#[derive(Debug, PartialEq, Eq)]
pub enum SshChannelData {
    /// Data from the channel (stdout or stderr).
    Data(Vec<u8>),
    /// The remote shell ended through EOF, close, or an exit status.
    CleanEnd,
    /// The transport disappeared without a normal channel terminator.
    ConnectionLost,
}

fn classify_channel_message(
    message: Option<ChannelMsg>,
    saw_exit: &mut bool,
) -> Option<SshChannelData> {
    match message {
        Some(ChannelMsg::Data { data }) => Some(SshChannelData::Data(data.to_vec())),
        Some(ChannelMsg::ExtendedData { data, .. }) => Some(SshChannelData::Data(data.to_vec())),
        Some(ChannelMsg::Eof) | Some(ChannelMsg::Close) => Some(SshChannelData::CleanEnd),
        Some(ChannelMsg::ExitStatus { .. }) | Some(ChannelMsg::ExitSignal { .. }) => {
            *saw_exit = true;
            None
        }
        None if *saw_exit => Some(SshChannelData::CleanEnd),
        None => Some(SshChannelData::ConnectionLost),
        _ => None,
    }
}

impl SshChannelReader {
    /// Wait for the next data from the channel.
    ///
    /// Returns data while the channel is active, [`SshChannelData::CleanEnd`]
    /// for a normal shell exit, or [`SshChannelData::ConnectionLost`] when the
    /// transport disappears without a normal channel terminator.
    pub async fn read(&mut self) -> SshChannelData {
        loop {
            if let Some(data) =
                classify_channel_message(self.channel.wait().await, &mut self.saw_exit)
            {
                return data;
            }
        }
    }
}

impl SshChannelWriter {
    /// Write all data to the SSH channel.
    pub async fn write_all(&self, data: &[u8]) -> crate::Result<()> {
        self.channel
            .data(std::io::Cursor::new(data.to_vec()))
            .await
            .map_err(|e| SshError::Channel(e.to_string()))
    }

    /// Request terminal window size change.
    pub async fn resize(&self, rows: u32, cols: u32) -> crate::Result<()> {
        self.channel
            .window_change(cols, rows, 0, 0)
            .await
            .map_err(|e| SshError::Channel(e.to_string()))
    }
}

#[cfg(test)]
mod channel_end_tests {
    use super::{classify_channel_message, SshChannelData};
    use russh::ChannelMsg;

    // SDTEST-1413
    #[test]
    fn protocol_terminators_are_clean_but_unmarked_channel_loss_is_unexpected() {
        let mut saw_exit = false;
        assert_eq!(
            classify_channel_message(Some(ChannelMsg::Eof), &mut saw_exit),
            Some(SshChannelData::CleanEnd)
        );
        assert_eq!(
            classify_channel_message(Some(ChannelMsg::Close), &mut saw_exit),
            Some(SshChannelData::CleanEnd)
        );

        let mut saw_exit = false;
        assert_eq!(
            classify_channel_message(
                Some(ChannelMsg::ExitStatus { exit_status: 0 }),
                &mut saw_exit,
            ),
            None
        );
        assert!(saw_exit);
        assert_eq!(
            classify_channel_message(None, &mut saw_exit),
            Some(SshChannelData::CleanEnd)
        );

        let mut saw_exit = false;
        assert_eq!(
            classify_channel_message(None, &mut saw_exit),
            Some(SshChannelData::ConnectionLost)
        );
    }
}
