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

    /// Open ShellDeck's fixed, typed workspace subsystem on a raw PTY.
    ///
    /// No remote command or path is interpolated into an SSH exec request.
    /// The remote sshd must map `shelldeck-workspace-v1` to the trusted helper.
    pub async fn open_workspace_helper(
        &self,
        rows: u32,
        cols: u32,
    ) -> crate::Result<crate::workspace_helper::WorkspaceHelperChannel> {
        let handle = self.handle.lock().await;
        let channel = handle
            .channel_open_session()
            .await
            .map_err(|error| SshError::Channel(error.to_string()))?;
        channel
            .request_pty(
                true,
                "xterm-256color",
                cols,
                rows,
                0,
                0,
                &crate::workspace_helper::raw_workspace_pty_modes(),
            )
            .await
            .map_err(|error| SshError::Channel(format!("workspace PTY request failed: {error}")))?;
        channel
            .request_subsystem(true, crate::workspace_helper::WORKSPACE_SUBSYSTEM)
            .await
            .map_err(|error| {
                SshError::Channel(format!("workspace subsystem request failed: {error}"))
            })?;
        Ok(crate::workspace_helper::WorkspaceHelperChannel::new(
            channel,
        ))
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
    pub(crate) fn from_workspace_helper(channel: Channel<client::Msg>) -> Self {
        Self { channel }
    }

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

#[cfg(test)]
mod in_memory_ssh_tests {
    use super::SshSession;
    use crate::handler::ClientHandler;
    use russh::keys::{ssh_key::Algorithm, PrivateKey};
    use russh::server::{self, Auth, Msg, Session};
    use russh::{Channel, ChannelId, Pty};
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::sync::mpsc;
    use tokio::task::JoinHandle;
    use tokio::time::timeout;
    use uuid::Uuid;

    #[derive(Clone)]
    enum ExecBehavior {
        Complete {
            stdout: Vec<u8>,
            stderr: Vec<u8>,
            exit_code: u32,
        },
        WaitForCancellation,
    }

    #[derive(Debug, PartialEq, Eq)]
    enum ServerEvent {
        Pty {
            term: String,
            cols: u32,
            rows: u32,
            modes: Vec<(Pty, u32)>,
        },
        Shell,
        Subsystem(String),
        Exec(Vec<u8>),
        Resize {
            cols: u32,
            rows: u32,
        },
        ChannelEof,
    }

    struct InMemoryServer {
        events: mpsc::UnboundedSender<ServerEvent>,
        exec_behavior: ExecBehavior,
    }

    impl server::Handler for InMemoryServer {
        type Error = anyhow::Error;

        async fn auth_none(&mut self, _user: &str) -> Result<Auth, Self::Error> {
            Ok(Auth::Accept)
        }

        async fn channel_open_session(
            &mut self,
            _channel: Channel<Msg>,
            _session: &mut Session,
        ) -> Result<bool, Self::Error> {
            Ok(true)
        }

        async fn pty_request(
            &mut self,
            channel: ChannelId,
            term: &str,
            cols: u32,
            rows: u32,
            _pixel_width: u32,
            _pixel_height: u32,
            modes: &[(Pty, u32)],
            session: &mut Session,
        ) -> Result<(), Self::Error> {
            let _ = self.events.send(ServerEvent::Pty {
                term: term.to_owned(),
                cols,
                rows,
                modes: modes.to_vec(),
            });
            session.channel_success(channel)?;
            Ok(())
        }

        async fn subsystem_request(
            &mut self,
            channel: ChannelId,
            name: &str,
            session: &mut Session,
        ) -> Result<(), Self::Error> {
            let _ = self.events.send(ServerEvent::Subsystem(name.to_owned()));
            session.channel_success(channel)?;
            Ok(())
        }

        async fn shell_request(
            &mut self,
            channel: ChannelId,
            session: &mut Session,
        ) -> Result<(), Self::Error> {
            let _ = self.events.send(ServerEvent::Shell);
            session.channel_success(channel)?;
            Ok(())
        }

        async fn exec_request(
            &mut self,
            channel: ChannelId,
            command: &[u8],
            session: &mut Session,
        ) -> Result<(), Self::Error> {
            let _ = self.events.send(ServerEvent::Exec(command.to_vec()));
            session.channel_success(channel)?;

            if let ExecBehavior::Complete {
                stdout,
                stderr,
                exit_code,
            } = &self.exec_behavior
            {
                session.data(channel, stdout.clone())?;
                session.extended_data(channel, 1, stderr.clone())?;
                session.exit_status_request(channel, *exit_code)?;
                session.eof(channel)?;
            }

            Ok(())
        }

        async fn window_change_request(
            &mut self,
            channel: ChannelId,
            cols: u32,
            rows: u32,
            _pixel_width: u32,
            _pixel_height: u32,
            session: &mut Session,
        ) -> Result<(), Self::Error> {
            let _ = self.events.send(ServerEvent::Resize { cols, rows });
            session.channel_success(channel)?;
            Ok(())
        }

        async fn channel_eof(
            &mut self,
            _channel: ChannelId,
            _session: &mut Session,
        ) -> Result<(), Self::Error> {
            let _ = self.events.send(ServerEvent::ChannelEof);
            Ok(())
        }
    }

    async fn start_session(
        exec_behavior: ExecBehavior,
    ) -> (
        Arc<SshSession>,
        mpsc::UnboundedReceiver<ServerEvent>,
        JoinHandle<()>,
    ) {
        let (client_stream, server_stream) = tokio::io::duplex(64 * 1024);
        let (server_event_tx, server_event_rx) = mpsc::unbounded_channel();

        let server_config = server::Config {
            inactivity_timeout: None,
            auth_rejection_time: Duration::from_millis(1),
            auth_rejection_time_initial: Some(Duration::from_millis(1)),
            keys: vec![PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519)
                .expect("generate in-memory SSH host key")],
            ..Default::default()
        };

        let server_task = tokio::spawn(async move {
            let running = server::run_stream(
                Arc::new(server_config),
                server_stream,
                InMemoryServer {
                    events: server_event_tx,
                    exec_behavior,
                },
            )
            .await
            .expect("start in-memory SSH server");
            let _ = running.await;
        });

        let (event_tx, event_rx) = mpsc::unbounded_channel();
        let (forwarded_tx, forwarded_rx) = mpsc::unbounded_channel();
        let handler = ClientHandler::new_trusting_server_key_for_test(event_tx, forwarded_tx);
        let mut handle = russh::client::connect_stream(
            Arc::new(russh::client::Config::default()),
            client_stream,
            handler,
        )
        .await
        .expect("connect to in-memory SSH server");
        assert!(handle
            .authenticate_none("shelldeck-test")
            .await
            .expect("authenticate in-memory SSH client")
            .success());

        (
            Arc::new(SshSession::new(
                Uuid::new_v4(),
                handle,
                event_rx,
                forwarded_rx,
            )),
            server_event_rx,
            server_task,
        )
    }

    async fn next_event(events: &mut mpsc::UnboundedReceiver<ServerEvent>) -> ServerEvent {
        timeout(Duration::from_secs(2), events.recv())
            .await
            .expect("server event timed out")
            .expect("in-memory server event stream closed")
    }

    // SDTEST-520, SDTEST-525
    #[tokio::test]
    async fn shell_requests_pty_dimensions_and_propagates_resize() {
        let (session, mut events, server_task) =
            start_session(ExecBehavior::WaitForCancellation).await;

        let channel = session.open_shell(32, 120).await.expect("open shell");
        assert_eq!(
            next_event(&mut events).await,
            ServerEvent::Pty {
                term: "xterm-256color".to_owned(),
                cols: 120,
                rows: 32,
                modes: Vec::new(),
            }
        );
        assert_eq!(next_event(&mut events).await, ServerEvent::Shell);

        channel.resize(48, 160).await.expect("resize shell");
        assert_eq!(
            next_event(&mut events).await,
            ServerEvent::Resize {
                cols: 160,
                rows: 48,
            }
        );

        server_task.abort();
    }

    // SDTEST-1786
    #[tokio::test]
    async fn workspace_helper_uses_only_fixed_subsystem_and_raw_bounded_control_pty() {
        let (session, mut events, server_task) =
            start_session(ExecBehavior::WaitForCancellation).await;

        let _helper = session
            .open_workspace_helper(36, 132)
            .await
            .expect("open fixed workspace subsystem");
        let ServerEvent::Pty {
            term,
            cols,
            rows,
            modes,
        } = next_event(&mut events).await
        else {
            panic!("workspace helper must request a PTY first");
        };
        assert_eq!(term, "xterm-256color");
        assert_eq!((cols, rows), (132, 36));
        for required in [Pty::ISIG, Pty::ICANON, Pty::ECHO, Pty::OPOST] {
            assert!(modes.contains(&(required, 0)));
        }
        assert_eq!(
            next_event(&mut events).await,
            ServerEvent::Subsystem(crate::workspace_helper::WORKSPACE_SUBSYSTEM.to_owned())
        );
        assert!(
            events.try_recv().is_err(),
            "no arbitrary exec or shell request"
        );

        server_task.abort();
    }

    // SDTEST-521
    #[tokio::test]
    async fn exec_collects_stdout_stderr_and_exit_status() {
        let (session, mut events, server_task) = start_session(ExecBehavior::Complete {
            stdout: b"standard output\n".to_vec(),
            stderr: b"standard error\n".to_vec(),
            exit_code: 23,
        })
        .await;

        let result = timeout(Duration::from_secs(2), session.exec("printf test"))
            .await
            .expect("exec timed out")
            .expect("exec failed");

        assert_eq!(
            next_event(&mut events).await,
            ServerEvent::Exec(b"printf test".to_vec())
        );
        assert_eq!(result.stdout, b"standard output\n");
        assert_eq!(result.stderr, b"standard error\n");
        assert_eq!(result.exit_code, Some(23));
        assert!(!result.success());

        server_task.abort();
    }

    // SDTEST-524
    #[tokio::test]
    async fn cancellable_exec_sends_channel_eof_and_returns_no_exit_status() {
        let (session, mut events, server_task) =
            start_session(ExecBehavior::WaitForCancellation).await;
        let (output_tx, _output_rx) = mpsc::unbounded_channel();
        let (shutdown_tx, shutdown_rx) = mpsc::channel(1);

        let exec_task = tokio::spawn(async move {
            session
                .exec_cancellable("sleep forever", output_tx, shutdown_rx)
                .await
        });
        assert_eq!(
            next_event(&mut events).await,
            ServerEvent::Exec(b"sleep forever".to_vec())
        );

        shutdown_tx.send(()).await.expect("request cancellation");
        let exit_code = timeout(Duration::from_secs(2), exec_task)
            .await
            .expect("cancellable exec timed out")
            .expect("cancellable exec task panicked")
            .expect("cancellable exec failed");
        assert_eq!(exit_code, None);
        assert_eq!(next_event(&mut events).await, ServerEvent::ChannelEof);

        server_task.abort();
    }
}

#[cfg(test)]
mod live_workspace_subsystem_tests {
    use crate::client::SshClient;
    use crate::workspace_helper::WorkspacePrepareRequest;
    use russh::ChannelMsg;
    use shelldeck_core::models::{Connection, ConnectionSource, ConnectionStatus};
    use std::path::PathBuf;
    use std::time::Duration;
    use tokio::time::timeout;
    use uuid::Uuid;

    // SDTEST-1809
    #[tokio::test]
    #[ignore = "requires SHELLDECK_LIVE_SSH=1 and an installed shelldeck-workspace-v1 subsystem"]
    async fn fixed_workspace_subsystem_prepares_releases_and_resumes_exact_remote_repository() {
        if std::env::var("SHELLDECK_LIVE_SSH").as_deref() != Ok("1") {
            eprintln!("skipped: SHELLDECK_LIVE_SSH is not set");
            return;
        }
        let required = |name: &str| {
            std::env::var(name).unwrap_or_else(|_| panic!("{name} is required for SDTEST-1809"))
        };
        let hostname = required("SHELLDECK_LIVE_SSH_HOST");
        let port = required("SHELLDECK_LIVE_SSH_PORT")
            .parse::<u16>()
            .expect("SHELLDECK_LIVE_SSH_PORT must be a non-zero u16");
        assert_ne!(port, 0, "SHELLDECK_LIVE_SSH_PORT must be non-zero");
        let user = required("SHELLDECK_LIVE_SSH_USER");
        let identity_file = PathBuf::from(required("SHELLDECK_LIVE_SSH_IDENTITY"));
        let remote_workspace = required("SHELLDECK_LIVE_SSH_WORKSPACE");
        let connection = Connection {
            id: Uuid::new_v4(),
            alias: "SDTEST-1809".to_owned(),
            hostname,
            port,
            user,
            identity_file: Some(identity_file),
            proxy_jump: None,
            group: None,
            tags: Vec::new(),
            auto_forwards: Vec::new(),
            auto_scripts: Vec::new(),
            source: ConnectionSource::Manual,
            forward_agent: false,
            site_id: None,
            site_label: None,
            status: ConnectionStatus::Disconnected,
        };
        let session = timeout(
            Duration::from_secs(10),
            SshClient::new().connect(&connection),
        )
        .await
        .expect("live SSH connection timed out")
        .expect("live SSH connection failed");

        let release_operation = Uuid::new_v4();
        let release_workspace = Uuid::new_v4();
        let mut release_helper = timeout(
            Duration::from_secs(5),
            session.open_workspace_helper(24, 80),
        )
        .await
        .expect("release helper open timed out")
        .expect("release helper open failed");
        let release_receipt = timeout(
            Duration::from_secs(5),
            release_helper.prepare(WorkspacePrepareRequest {
                operation: release_operation,
                workspace: release_workspace,
                remote_root: remote_workspace.clone(),
            }),
        )
        .await
        .expect("release prepare timed out")
        .expect("release prepare failed");
        assert!(release_receipt.matches(release_operation, release_workspace));
        timeout(
            Duration::from_secs(5),
            release_helper.release(&release_receipt),
        )
        .await
        .expect("release timed out")
        .expect("release failed");

        let resume_operation = Uuid::new_v4();
        let resume_workspace = Uuid::new_v4();
        let mut resume_helper = timeout(
            Duration::from_secs(5),
            session.open_workspace_helper(31, 109),
        )
        .await
        .expect("resume helper open timed out")
        .expect("resume helper open failed");
        let resume_receipt = timeout(
            Duration::from_secs(5),
            resume_helper.prepare(WorkspacePrepareRequest {
                operation: resume_operation,
                workspace: resume_workspace,
                remote_root: remote_workspace.clone(),
            }),
        )
        .await
        .expect("resume prepare timed out")
        .expect("resume prepare failed");
        assert!(resume_receipt.matches(resume_operation, resume_workspace));
        assert!(!resume_receipt.head_oid.is_empty());
        assert!(!resume_receipt.branch.is_empty());

        let mut shell = timeout(
            Duration::from_secs(5),
            resume_helper.resume(&resume_receipt),
        )
        .await
        .expect("resume timed out")
        .expect("resume failed");
        shell
            .write(b"printf 'SHELLDECK_WORKSPACE_READY:%s\\n' \"$PWD\"; exit\n")
            .await
            .expect("write resumed shell marker");
        let output = timeout(Duration::from_secs(10), async {
            let mut output = Vec::new();
            while let Some(message) = shell.read().await {
                match message {
                    ChannelMsg::Data { data } => output.extend_from_slice(&data),
                    ChannelMsg::Eof | ChannelMsg::Close => break,
                    _ => {}
                }
            }
            output
        })
        .await
        .expect("resumed shell output timed out");
        let output = String::from_utf8_lossy(&output);
        let marker = "SHELLDECK_WORKSPACE_READY:";
        let observed_cwd = output
            .lines()
            .map(|line| line.trim_end_matches('\r'))
            .filter_map(|line| line.rsplit_once(marker).map(|(_, cwd)| cwd))
            // The PTY may prefix the shell prompt to command output. The
            // echoed printf format is another marker occurrence, but is not
            // an absolute path and therefore cannot be the helper-authorized
            // remote root.
            .find(|cwd| cwd.starts_with('/'))
            .unwrap_or_else(|| {
                panic!("resumed shell did not emit the workspace-ready marker: {output:?}")
            });
        assert_eq!(
            observed_cwd, remote_workspace,
            "resumed shell did not prove the exact retained workspace CWD"
        );
        session.disconnect().await.expect("disconnect live session");
    }
}
