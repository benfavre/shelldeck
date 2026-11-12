//! Tests of the session daemon transport: a real russh client and server
//! over an in-memory duplex, a real daemon on a private temporary run
//! directory and real PTYs. Nothing here touches the user's run directory,
//! sockets or processes.
//!
//! The server side of the fixture answers the exec request in one of four
//! ways: it relays the channel to the daemon socket the way
//! `shelldeck-sessiond proxy` does, it runs the command with the real
//! `/bin/sh`, it plays a fixed script, or it refuses.

use super::{
    connect, ProxyRefusal, RemoteBinary, SessiondConnectError, SessiondConnectOptions,
    SessiondConnection, TunnelEnd,
};
use crate::handler::ClientHandler;
use crate::session::SshSession;
use russh::keys::{ssh_key::Algorithm, PrivateKey};
use russh::server::{self, Auth, Msg, Session};
use russh::{Channel, ChannelId};
use shelldeck_sessiond::client::{AttachOptions, PaneEvent, PaneEvents, SpawnOptions};
use shelldeck_sessiond::paths;
use shelldeck_sessiond::protocol::{self, DaemonFrame, ErrorCode, SIGNAL_KILL};
use shelldeck_sessiond::server::{ServeOutcome, ServerConfig, ServerHandle};
use std::collections::HashMap;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio::time::timeout;
use uuid::Uuid;

const STEP: Duration = Duration::from_secs(20);
const INSTALLED: &str = "/opt/shelldeck/bin/shelldeck-sessiond";

// ---------------------------------------------------------------------------
// Fixture
// ---------------------------------------------------------------------------

#[derive(Clone, Default)]
struct Script {
    /// Sent before the marker.
    banner: Vec<u8>,
    /// Whether the nonce marker is sent at all.
    marker: bool,
    /// Sent after the marker.
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    exit_code: Option<u32>,
    eof_before_status: bool,
}

#[derive(Clone)]
enum ExecMode {
    /// Relay the channel to the daemon serving `dir`, like the real proxy:
    /// marker first, then bytes both ways, exit status 0 when the daemon
    /// closes the socket.
    Proxy {
        dir: PathBuf,
        banner: Vec<u8>,
    },
    /// Run the received command with the real `/bin/sh -c`.
    Shell {
        home: Option<PathBuf>,
    },
    Script(Script),
    Refuse,
    /// A whole host. The proxy command reaches the daemon serving `dir`
    /// when the versioned binary it names is installed and executable
    /// under `home` (else it ends with the status a shell would give:
    /// 127 or 126). Every other command runs in the real `/bin/sh` with
    /// that `HOME`.
    Host(HostMode),
}

#[derive(Clone)]
struct HostMode {
    home: PathBuf,
    dir: PathBuf,
    /// `PATH` of the commands run by `/bin/sh`, when not the inherited one.
    path: Option<PathBuf>,
    /// Flip one byte of what the client sends to a command's stdin.
    tamper_stdin: bool,
    /// Refuse the exec request of the proxy command.
    refuse_proxy: bool,
}

/// The file a proxy command starts, when `command` is the fixed command
/// of the versioned install location.
fn proxied_binary(command: &str, home: &Path) -> Option<PathBuf> {
    let name = command
        .strip_prefix("exec \"$HOME/.shelldeck/bin/")?
        .split_once("\" proxy --hello ")?
        .0;
    Some(home.join(".shelldeck/bin").join(name))
}

#[derive(Debug, PartialEq, Eq)]
enum ServerEvent {
    Exec(String),
    ChannelEof,
    ChannelClose,
}

struct InMemoryServer {
    events: mpsc::UnboundedSender<ServerEvent>,
    mode: ExecMode,
    /// Channel data on its way to the process or socket behind the exec.
    inputs: HashMap<ChannelId, mpsc::UnboundedSender<Vec<u8>>>,
}

/// End of the remote command in OpenSSH order: EOF, exit status, Close.
async fn finish(handle: &server::Handle, channel: ChannelId, exit_code: Option<u32>) {
    let _ = handle.eof(channel).await;
    if let Some(code) = exit_code {
        let _ = handle.exit_status_request(channel, code).await;
    }
    let _ = handle.close(channel).await;
}

async fn relay_to_daemon(
    handle: server::Handle,
    channel: ChannelId,
    dir: PathBuf,
    prelude: Vec<u8>,
    mut input: mpsc::UnboundedReceiver<Vec<u8>>,
) {
    let stream = match paths::connect_checked(&dir) {
        Ok(stream) => stream,
        Err(_) => {
            let _ = handle
                .extended_data(channel, 1, b"SDSD-ERR 3\n".to_vec())
                .await;
            return finish(&handle, channel, Some(3)).await;
        }
    };
    stream.set_nonblocking(true).expect("nonblocking socket");
    let stream = tokio::net::UnixStream::from_std(stream).expect("tokio socket");
    let (mut from_daemon, mut to_daemon) = stream.into_split();
    let _ = handle.data(channel, prelude).await;
    tokio::spawn(async move {
        while let Some(bytes) = input.recv().await {
            if to_daemon.write_all(&bytes).await.is_err() {
                break;
            }
        }
        // Channel EOF or Close: the daemon sees the client go away.
        let _ = to_daemon.shutdown().await;
    });
    let mut buf = vec![0u8; 16 * 1024];
    loop {
        match from_daemon.read(&mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                if handle.data(channel, buf[..n].to_vec()).await.is_err() {
                    return;
                }
            }
        }
    }
    finish(&handle, channel, Some(0)).await;
}

async fn run_shell(
    handle: server::Handle,
    channel: ChannelId,
    command: String,
    home: Option<PathBuf>,
    path: Option<PathBuf>,
    mut tamper_stdin: bool,
    mut input: mpsc::UnboundedReceiver<Vec<u8>>,
) {
    let mut process = tokio::process::Command::new("/bin/sh");
    process
        .arg("-c")
        .arg(command)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    if let Some(home) = home {
        process.env("HOME", home);
    }
    if let Some(path) = path {
        process.env("PATH", path);
    }
    let mut child = process.spawn().expect("spawn /bin/sh");
    let mut stdin = child.stdin.take().expect("stdin");
    let mut stdout = child.stdout.take().expect("stdout");
    let mut stderr = child.stderr.take().expect("stderr");
    tokio::spawn(async move {
        while let Some(mut bytes) = input.recv().await {
            if let (true, Some(first)) = (tamper_stdin, bytes.first_mut()) {
                *first ^= 0xff;
                tamper_stdin = false;
            }
            if stdin.write_all(&bytes).await.is_err() {
                break;
            }
        }
        // Dropping stdin here is the EOF of the remote command.
    });
    let stderr_handle = handle.clone();
    let stderr_task = tokio::spawn(async move {
        let mut buf = vec![0u8; 4096];
        while let Ok(n @ 1..) = stderr.read(&mut buf).await {
            let _ = stderr_handle
                .extended_data(channel, 1, buf[..n].to_vec())
                .await;
        }
    });
    let mut buf = vec![0u8; 16 * 1024];
    while let Ok(n @ 1..) = stdout.read(&mut buf).await {
        if handle.data(channel, buf[..n].to_vec()).await.is_err() {
            break;
        }
    }
    let _ = stderr_task.await;
    let status = child.wait().await.expect("wait for /bin/sh");
    finish(&handle, channel, status.code().map(|code| code as u32)).await;
}

impl server::Handler for InMemoryServer {
    type Error = anyhow::Error;

    async fn auth_none(&mut self, _user: &str) -> Result<Auth, Self::Error> {
        Ok(Auth::Accept)
    }

    async fn channel_open_session(
        &mut self,
        _channel: Channel<Msg>,
        reply: russh::server::ChannelOpenHandle,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        reply.accept().await;
        Ok(())
    }

    async fn exec_request(
        &mut self,
        channel: ChannelId,
        command: &[u8],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        let command = String::from_utf8(command.to_vec())?;
        let _ = self.events.send(ServerEvent::Exec(command.clone()));
        let nonce = command.rsplit(' ').next().unwrap_or_default().to_owned();
        let marker = protocol::proxy_marker(&nonce).unwrap_or_default();
        let (input_tx, input_rx) = mpsc::unbounded_channel();
        match self.mode.clone() {
            ExecMode::Refuse => {
                session.channel_failure(channel)?;
            }
            ExecMode::Proxy { dir, banner } => {
                session.channel_success(channel)?;
                self.inputs.insert(channel, input_tx);
                let prelude = [banner, marker].concat();
                tokio::spawn(relay_to_daemon(
                    session.handle(),
                    channel,
                    dir,
                    prelude,
                    input_rx,
                ));
            }
            ExecMode::Shell { home } => {
                session.channel_success(channel)?;
                self.inputs.insert(channel, input_tx);
                tokio::spawn(run_shell(
                    session.handle(),
                    channel,
                    command,
                    home,
                    None,
                    false,
                    input_rx,
                ));
            }
            ExecMode::Host(host) => match proxied_binary(&command, &host.home) {
                Some(_) if host.refuse_proxy => session.channel_failure(channel)?,
                Some(binary) => {
                    session.channel_success(channel)?;
                    let mode = std::fs::metadata(&binary).map(|meta| meta.permissions().mode());
                    let handle = session.handle();
                    match mode {
                        Ok(mode) if mode & 0o100 != 0 => {
                            self.inputs.insert(channel, input_tx);
                            tokio::spawn(relay_to_daemon(
                                handle, channel, host.dir, marker, input_rx,
                            ));
                        }
                        other => {
                            let status = if other.is_ok() { 126 } else { 127 };
                            tokio::spawn(
                                async move { finish(&handle, channel, Some(status)).await },
                            );
                        }
                    }
                }
                None => {
                    session.channel_success(channel)?;
                    self.inputs.insert(channel, input_tx);
                    tokio::spawn(run_shell(
                        session.handle(),
                        channel,
                        command,
                        Some(host.home),
                        host.path,
                        host.tamper_stdin,
                        input_rx,
                    ));
                }
            },
            ExecMode::Script(script) => {
                session.channel_success(channel)?;
                let mut stdout = script.banner;
                if script.marker {
                    stdout.extend_from_slice(&marker);
                }
                stdout.extend_from_slice(&script.stdout);
                session.data(channel, stdout)?;
                session.extended_data(channel, 1, script.stderr)?;
                if let Some(code) = script.exit_code {
                    if script.eof_before_status {
                        session.eof(channel)?;
                    }
                    session.exit_status_request(channel, code)?;
                    if !script.eof_before_status {
                        session.eof(channel)?;
                    }
                    session.close(channel)?;
                }
            }
        }
        Ok(())
    }

    async fn data(
        &mut self,
        channel: ChannelId,
        data: &[u8],
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        if let Some(input) = self.inputs.get(&channel) {
            let _ = input.send(data.to_vec());
        }
        Ok(())
    }

    async fn channel_eof(
        &mut self,
        channel: ChannelId,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.inputs.remove(&channel);
        let _ = self.events.send(ServerEvent::ChannelEof);
        Ok(())
    }

    async fn channel_close(
        &mut self,
        channel: ChannelId,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.inputs.remove(&channel);
        let _ = self.events.send(ServerEvent::ChannelClose);
        Ok(())
    }
}

struct Fixture {
    session: Arc<SshSession>,
    events: mpsc::UnboundedReceiver<ServerEvent>,
    server_task: JoinHandle<()>,
}

/// A new in-memory SSH server answering exec requests with `mode`, and a
/// client session connected to it.
async fn open_session(
    mode: ExecMode,
    server_event_tx: mpsc::UnboundedSender<ServerEvent>,
) -> (SshSession, JoinHandle<()>) {
    let (client_stream, server_stream) = tokio::io::duplex(64 * 1024);
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
                mode,
                inputs: HashMap::new(),
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
        SshSession::new(Uuid::new_v4(), handle, event_rx, forwarded_rx),
        server_task,
    )
}

impl Fixture {
    async fn start(mode: ExecMode) -> Self {
        let (server_event_tx, events) = mpsc::unbounded_channel();
        let (session, server_task) = open_session(mode, server_event_tx).await;
        Self {
            session: Arc::new(session),
            events,
            server_task,
        }
    }

    async fn next_event(&mut self) -> ServerEvent {
        timeout(STEP, self.events.recv())
            .await
            .expect("server event timed out")
            .expect("in-memory server event stream closed")
    }

    /// The client released a channel the remote side still had open: EOF
    /// then Close reach the server.
    async fn expect_released(&mut self) {
        assert_eq!(self.next_event().await, ServerEvent::ChannelEof);
        assert_eq!(self.next_event().await, ServerEvent::ChannelClose);
    }

    /// The remote side closed the channel first. The client's answering
    /// Close is the RFC 4254 reply; since russh 0.63 the server consumes it
    /// without calling `channel_close` (the channel left its table when it
    /// sent its own Close), so the reply may or may not be reported. What
    /// must not happen is any other traffic on the way out.
    async fn expect_closed(&mut self) {
        let window = Duration::from_millis(400);
        loop {
            match timeout(window, self.events.recv()).await {
                Err(_) | Ok(None) => return,
                Ok(Some(ServerEvent::ChannelEof | ServerEvent::ChannelClose)) => continue,
                Ok(Some(other)) => panic!("expected only the channel release, got {other:?}"),
            }
        }
    }

    async fn connect(&self, binary: &str) -> Result<SessiondConnection, SessiondConnectError> {
        timeout(STEP, connect(&self.session, options(binary)))
            .await
            .expect("connect timed out")
    }

    /// The connect attempt that must fail, with its error.
    async fn connect_error(&self, binary: &str) -> SessiondConnectError {
        match self.connect(binary).await {
            Ok(_) => panic!("connect unexpectedly succeeded"),
            Err(error) => error,
        }
    }
}

fn options(binary: &str) -> SessiondConnectOptions {
    SessiondConnectOptions {
        binary: RemoteBinary::Absolute(binary.to_owned()),
        startup_timeout: Duration::from_secs(10),
        ..SessiondConnectOptions::default()
    }
}

/// A private temporary directory with a short path (`sun_path` is 108
/// bytes) made only of characters the configured binary path accepts.
fn temp_dir() -> tempfile::TempDir {
    let temp = tempfile::Builder::new()
        .prefix("sdssh-")
        .tempdir()
        .expect("temp dir");
    if temp.path().as_os_str().len() <= 40 {
        return temp;
    }
    tempfile::Builder::new()
        .prefix("sdssh-")
        .tempdir_in("/tmp")
        .expect("temp dir in /tmp")
}

fn start_daemon(dir: &Path) -> ServerHandle {
    let config = ServerConfig {
        idle_exit: None,
        term_grace: Duration::from_millis(500),
        ..ServerConfig::default()
    };
    // Another test forking a pane child holds a copy of the instance lock
    // descriptor until its exec, so the lock can look taken for an instant.
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match ServerHandle::start(dir, config.clone()).expect("daemon starts") {
            Some(handle) => return handle,
            None => assert!(Instant::now() < deadline, "the instance lock stays taken"),
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn exec_command(event: ServerEvent) -> String {
    match event {
        ServerEvent::Exec(command) => command,
        other => panic!("expected the exec request, got {other:?}"),
    }
}

fn assert_proxy_command(command: &str, binary: &str) -> String {
    let prefix = format!("exec '{binary}' proxy --hello ");
    let nonce = command
        .strip_prefix(&prefix)
        .unwrap_or_else(|| panic!("unexpected remote command: {command}"));
    assert_eq!(nonce.len(), 32, "{command}");
    assert!(
        nonce
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
        "{command}"
    );
    nonce.to_owned()
}

fn count(haystack: &[u8], needle: &[u8]) -> usize {
    haystack
        .windows(needle.len())
        .filter(|window| *window == needle)
        .count()
}

/// Pump pane events until the restored snapshot arrived.
async fn until_restored(events: &mut PaneEvents) {
    loop {
        match timeout(STEP, events.next()).await.expect("pane event") {
            Some(PaneEvent::Restore { .. }) => return,
            Some(PaneEvent::Closed) | None => panic!("pane stream ended before the snapshot"),
            Some(_) => {}
        }
    }
}

/// Pump pane events until the raw output holds `needle` `times` times.
async fn until_output(events: &mut PaneEvents, needle: &[u8], times: usize) -> Vec<u8> {
    let mut output = Vec::new();
    while count(&output, needle) < times {
        match timeout(STEP, events.next()).await.expect("pane event") {
            Some(PaneEvent::Bytes { data, .. }) => output.extend_from_slice(&data),
            Some(PaneEvent::Closed) | None => panic!(
                "pane stream ended, output so far: {:?}",
                String::from_utf8_lossy(&output)
            ),
            Some(_) => {}
        }
    }
    output
}

fn cat_pane() -> SpawnOptions {
    SpawnOptions {
        group: Uuid::new_v4(),
        label: "ssh".into(),
        rows: 24,
        cols: 80,
        argv: vec!["/bin/cat".into()],
        ..SpawnOptions::default()
    }
}

const ATTACH: AttachOptions = AttachOptions {
    rows: 24,
    cols: 80,
    scrollback_rows: 0,
    read_only: false,
};

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

// SDTEST-2985
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sdtest_2985_daemon_over_ssh_spawns_attaches_echoes_and_survives_the_channel() {
    let temp = temp_dir();
    let run = temp.path().join("run");
    let daemon = start_daemon(&run);
    let mut fixture = Fixture::start(ExecMode::Proxy {
        dir: run,
        // What a login banner or an rc file prints before the proxy runs,
        // with a binary byte and a look-alike of the marker.
        banner: b"Last login: today\r\n\x00\xff\x1eSHELLDECK_SESSIOND_0_READY\x1f".to_vec(),
    })
    .await;

    let connection = fixture.connect(INSTALLED).await.expect("connect");
    let first_nonce = assert_proxy_command(&exec_command(fixture.next_event().await), INSTALLED);
    let client = connection.client;
    assert_eq!(client.hello().proto, protocol::PROTOCOL_VERSION);

    let spawned = timeout(STEP, client.spawn(cat_pane()))
        .await
        .expect("spawn in time")
        .expect("spawn");
    let mut events = client.attach(spawned.pane, ATTACH).expect("attach");
    until_restored(&mut events).await;
    client
        .input(spawned.pane, b"ping-over-ssh\n")
        .expect("input");
    // Once from the terminal echo, once from cat.
    until_output(&mut events, b"ping-over-ssh", 2).await;

    // Every byte value crosses the channel unchanged in both directions
    // (no PTY on the SSH channel). `cat -v` style checks are not needed:
    // the pane PTY echoes printable bytes as they are.
    let wide = "caf\u{e9} \u{1f600} \u{4e2d}\u{6587}\n";
    client.input(spawned.pane, wide.as_bytes()).expect("input");
    until_output(&mut events, wide.trim_end().as_bytes(), 2).await;

    // Dropping the client releases the channel; the pane stays.
    drop(events);
    drop(client);
    fixture.expect_released().await;
    assert_eq!(
        timeout(STEP, connection.ended).await.expect("tunnel end"),
        Ok(TunnelEnd::LocalClose)
    );

    // A second channel on the same SSH connection finds the pane and its
    // screen again.
    let connection = fixture.connect(INSTALLED).await.expect("reconnect");
    let second_nonce = assert_proxy_command(&exec_command(fixture.next_event().await), INSTALLED);
    assert_ne!(first_nonce, second_nonce, "the nonce is fresh per channel");
    let client = connection.client;
    let panes = timeout(STEP, client.list_panes())
        .await
        .expect("list in time")
        .expect("list");
    assert!(panes.iter().any(|info| info.pane == spawned.pane));
    let mut events = client.attach(spawned.pane, ATTACH).expect("reattach");
    until_restored(&mut events).await;
    client.input(spawned.pane, b"again\n").expect("input");
    until_output(&mut events, b"again", 2).await;
    client.kill(spawned.pane, SIGNAL_KILL).expect("kill");

    // The daemon going away ends the proxy: the tunnel reports a remote
    // exit with the status, not a lost connection.
    assert_eq!(daemon.stop(), ServeOutcome::Shutdown);
    assert_eq!(
        timeout(STEP, connection.ended).await.expect("tunnel end"),
        Ok(TunnelEnd::RemoteExit {
            exit_status: Some(0)
        })
    );
    loop {
        match timeout(STEP, events.next()).await.expect("pane event") {
            Some(PaneEvent::Closed) | None => break,
            Some(_) => {}
        }
    }
    assert!(client.is_closed());
    fixture.server_task.abort();
}

// SDTEST-2987
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sdtest_2987_missing_or_unusable_remote_binary_is_a_typed_fallback_error() {
    // The real /bin/sh runs the real fixed command.
    let mut fixture = Fixture::start(ExecMode::Shell { home: None }).await;
    let temp = temp_dir();

    let missing = temp.path().join("bin").join("shelldeck-sessiond-0.0.0");
    let missing = missing.to_str().expect("utf-8 path");
    let error = fixture.connect_error(missing).await;
    assert!(
        matches!(error, SessiondConnectError::BinaryMissing),
        "{error:?}"
    );
    assert!(error.is_unavailable());
    assert_proxy_command(&exec_command(fixture.next_event().await), missing);
    fixture.expect_closed().await;

    // Present but not executable: also a reason to fall back.
    let plain = temp.path().join("not-executable");
    std::fs::write(&plain, b"not a program").expect("write file");
    std::fs::set_permissions(&plain, std::fs::Permissions::from_mode(0o644)).expect("chmod");
    let plain = plain.to_str().expect("utf-8 path");
    let error = fixture.connect_error(plain).await;
    assert!(
        matches!(error, SessiondConnectError::BinaryNotExecutable),
        "{error:?}"
    );
    assert!(error.is_unavailable());
    assert_proxy_command(&exec_command(fixture.next_event().await), plain);
    fixture.expect_closed().await;

    // A program that exits without the marker is not a fallback case.
    let error = fixture.connect_error("/bin/false").await;
    assert!(
        matches!(
            error,
            SessiondConnectError::ProxyExited {
                exit_status: Some(1)
            }
        ),
        "{error:?}"
    );
    assert!(!error.is_unavailable());
    assert_proxy_command(&exec_command(fixture.next_event().await), "/bin/false");
    fixture.expect_closed().await;
    fixture.server_task.abort();
}

async fn scripted_error(script: Script) -> SessiondConnectError {
    let remote_closes = script.exit_code.is_some();
    let mut fixture = Fixture::start(ExecMode::Script(script)).await;
    let error = fixture.connect_error(INSTALLED).await;
    assert_proxy_command(&exec_command(fixture.next_event().await), INSTALLED);
    // Whatever the failure, the channel is released.
    if remote_closes {
        fixture.expect_closed().await;
    } else {
        fixture.expect_released().await;
    }
    fixture.server_task.abort();
    error
}

// SDTEST-2988
#[tokio::test]
async fn sdtest_2988_startup_failures_are_classified_and_release_the_channel() {
    // OpenSSH order: EOF first, the exit status after it.
    let error = scripted_error(Script {
        stderr: b"sh: 1: exec: /opt/shelldeck/bin/shelldeck-sessiond: not found\n".to_vec(),
        exit_code: Some(127),
        eof_before_status: true,
        ..Script::default()
    })
    .await;
    assert!(
        matches!(error, SessiondConnectError::BinaryMissing),
        "{error:?}"
    );

    // Status before EOF gives the same answer.
    let error = scripted_error(Script {
        exit_code: Some(127),
        ..Script::default()
    })
    .await;
    assert!(
        matches!(error, SessiondConnectError::BinaryMissing),
        "{error:?}"
    );

    // The proxy's own refusals, after rc noise on stderr.
    for (code, refusal) in [
        (1, ProxyRefusal::PathCheck),
        (2, ProxyRefusal::PeerCheck),
        (3, ProxyRefusal::StartFailed),
    ] {
        let error = scripted_error(Script {
            banner: b"motd\n".to_vec(),
            stderr: format!("stty: not a tty\nSDSD-ERR {code}\n").into_bytes(),
            exit_code: Some(code),
            eof_before_status: true,
            ..Script::default()
        })
        .await;
        assert!(
            matches!(error, SessiondConnectError::ProxyRefused(found) if found == refusal),
            "{error:?}"
        );
        assert!(!error.is_unavailable());
    }

    // An unknown refusal code is only an early exit.
    let error = scripted_error(Script {
        stderr: b"SDSD-ERR 9\n".to_vec(),
        exit_code: Some(9),
        ..Script::default()
    })
    .await;
    assert!(
        matches!(
            error,
            SessiondConnectError::ProxyExited {
                exit_status: Some(9)
            }
        ),
        "{error:?}"
    );

    // Output without the marker is bounded.
    let error = scripted_error(Script {
        banner: vec![b'x'; 70 * 1024],
        ..Script::default()
    })
    .await;
    assert!(matches!(error, SessiondConnectError::Prelude), "{error:?}");
}

// SDTEST-2989
#[tokio::test]
async fn sdtest_2989_version_mismatch_refused_exec_and_bad_setting_are_distinct() {
    // The remote daemon answers the Hello with UnsupportedVersion.
    let refusal = DaemonFrame::Error {
        request_id: 0,
        pane: Uuid::nil(),
        code: ErrorCode::UnsupportedVersion,
        retryable: false,
    }
    .encode()
    .expect("encode");
    let error = scripted_error(Script {
        marker: true,
        stdout: refusal,
        ..Script::default()
    })
    .await;
    assert!(
        matches!(error, SessiondConnectError::VersionMismatch),
        "{error:?}"
    );
    assert!(!error.is_unavailable());

    // Stderr after the marker fails closed: the tunnel ends and the
    // channel is released, whether or not the handshake got through first.
    let hello_ack = DaemonFrame::HelloAck(protocol::HelloAck {
        proto: protocol::PROTOCOL_VERSION,
        features: 0,
        daemon_version: "0.0.0".into(),
        instance: Uuid::new_v4(),
        max_frame: protocol::MAX_FRAME_BYTES as u32,
        emulator_rev: 1,
    })
    .encode()
    .expect("encode");
    let mut fixture = Fixture::start(ExecMode::Script(Script {
        marker: true,
        stdout: hello_ack,
        stderr: b"unexpected\n".to_vec(),
        ..Script::default()
    }))
    .await;
    let outcome = fixture.connect(INSTALLED).await;
    assert_proxy_command(&exec_command(fixture.next_event().await), INSTALLED);
    fixture.expect_released().await;
    match outcome {
        Ok(connection) => assert_eq!(
            timeout(STEP, connection.ended).await.expect("tunnel end"),
            Ok(TunnelEnd::UnexpectedStderr)
        ),
        Err(error) => assert!(
            matches!(error, SessiondConnectError::Client(_)),
            "{error:?}"
        ),
    }
    fixture.server_task.abort();

    // A server that refuses exec requests cannot run the daemon at all.
    let mut fixture = Fixture::start(ExecMode::Refuse).await;
    let error = fixture.connect_error(INSTALLED).await;
    assert!(
        matches!(error, SessiondConnectError::ExecRefused),
        "{error:?}"
    );
    assert!(error.is_unavailable());
    assert_proxy_command(&exec_command(fixture.next_event().await), INSTALLED);
    fixture.expect_released().await;

    // A setting that is not a plain absolute path never reaches the wire.
    for bad in [
        "shelldeck-sessiond",
        "/opt/x'; touch /tmp/pwned; '",
        "/opt/$(id)",
        "/opt/../bin/sh",
    ] {
        let error = fixture.connect_error(bad).await;
        assert!(
            matches!(error, SessiondConnectError::InvalidBinary),
            "{bad}: {error:?}"
        );
    }
    let error = timeout(
        STEP,
        connect(
            &fixture.session,
            SessiondConnectOptions {
                binary: RemoteBinary::Versioned("1.0; id".into()),
                ..SessiondConnectOptions::default()
            },
        ),
    )
    .await
    .expect("connect in time")
    .err()
    .expect("refused");
    assert!(
        matches!(error, SessiondConnectError::InvalidBinary),
        "{error:?}"
    );
    assert!(
        fixture.events.try_recv().is_err(),
        "no exec request was sent"
    );

    // The default names the per-user install of this app version.
    let connection = timeout(
        STEP,
        connect(&fixture.session, SessiondConnectOptions::default()),
    )
    .await
    .expect("connect in time");
    assert!(matches!(
        connection.err(),
        Some(SessiondConnectError::ExecRefused)
    ));
    assert_eq!(
        exec_command(fixture.next_event().await)
            .rsplit_once(' ')
            .expect("command")
            .0,
        format!(
            "exec \"$HOME/.shelldeck/bin/shelldeck-sessiond-{}\" proxy --hello",
            env!("CARGO_PKG_VERSION")
        )
    );
    fixture.server_task.abort();
}

// SDTEST-2990
#[tokio::test]
async fn sdtest_2990_a_proxy_that_never_answers_times_out_and_is_released() {
    // Channel accepted, nothing ever sent.
    let mut fixture = Fixture::start(ExecMode::Script(Script::default())).await;
    let started = Instant::now();
    let error = timeout(
        STEP,
        connect(
            &fixture.session,
            SessiondConnectOptions {
                startup_timeout: Duration::from_millis(200),
                ..options(INSTALLED)
            },
        ),
    )
    .await
    .expect("connect in time")
    .err()
    .expect("timeout");
    assert!(matches!(error, SessiondConnectError::Timeout), "{error:?}");
    assert!(started.elapsed() < Duration::from_secs(5));
    assert_proxy_command(&exec_command(fixture.next_event().await), INSTALLED);
    fixture.expect_released().await;

    // A cancelled connect releases the remote process as well.
    let pending = tokio::spawn({
        let session = fixture.session.clone();
        async move { connect(&session, options(INSTALLED)).await.map(|_| ()) }
    });
    assert_proxy_command(&exec_command(fixture.next_event().await), INSTALLED);
    pending.abort();
    fixture.expect_released().await;

    // Losing the SSH connection during startup is not a missing binary.
    let pending = tokio::spawn({
        let session = fixture.session.clone();
        async move { connect(&session, options(INSTALLED)).await.map(|_| ()) }
    });
    assert_proxy_command(&exec_command(fixture.next_event().await), INSTALLED);
    fixture
        .session
        .shared_handle()
        .lock()
        .await
        .disconnect(russh::Disconnect::ByApplication, "", "en")
        .await
        .expect("disconnect");
    let error = timeout(STEP, pending)
        .await
        .expect("connect ends")
        .expect("task")
        .expect_err("connection lost");
    assert!(
        matches!(
            error,
            SessiondConnectError::Ssh(crate::SshError::SessionClosed)
        ),
        "{error:?}"
    );
    assert!(!error.is_unavailable());
    fixture.server_task.abort();
}

/// Stops the daemon a live test started under its private home.
struct TerminateDaemons(PathBuf);

impl Drop for TerminateDaemons {
    fn drop(&mut self) {
        let Ok(hosts) = std::fs::read_dir(self.0.join(".shelldeck").join("run")) else {
            return;
        };
        for host in hosts.flatten() {
            let lock = host.path().join(paths::LOCK_NAME);
            let Ok(pid) = std::fs::read_to_string(lock) else {
                continue;
            };
            if let Ok(pid) = pid.trim().parse::<u32>() {
                let _ = std::process::Command::new("kill")
                    .arg("-TERM")
                    .arg(pid.to_string())
                    .status();
            }
        }
    }
}

// SDTEST-2991: the real binary behind the real fixed command, run by the
// real /bin/sh under a private HOME. Opt in with
// SHELLDECK_SESSIOND_BIN=<absolute path of a built shelldeck-sessiond>.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs a built shelldeck-sessiond: set SHELLDECK_SESSIOND_BIN"]
async fn sdtest_2991_real_proxy_binary_starts_the_daemon_and_serves_a_pane_over_ssh() {
    let binary = std::env::var("SHELLDECK_SESSIOND_BIN")
        .expect("SHELLDECK_SESSIOND_BIN names a built shelldeck-sessiond");
    let home = temp_dir();
    // `~/.shelldeck/run/<host>/` must hold the socket path, or the daemon
    // would fall back to the shared /tmp location of the real user.
    let host = std::process::Command::new("uname")
        .arg("-n")
        .output()
        .expect("uname -n");
    let socket_path_len = home.path().as_os_str().len()
        + "/.shelldeck/run/".len()
        + host.stdout.len().min(64)
        + 1
        + paths::SOCKET_NAME.len();
    assert!(socket_path_len <= 100, "temporary home path is too long");
    let _cleanup = TerminateDaemons(home.path().to_path_buf());
    let mut fixture = Fixture::start(ExecMode::Shell {
        home: Some(home.path().to_path_buf()),
    })
    .await;

    // No daemon runs under this home: the proxy starts one.
    let connection = fixture.connect(&binary).await.expect("connect");
    assert_proxy_command(&exec_command(fixture.next_event().await), &binary);
    let client = connection.client;
    let spawned = timeout(STEP, client.spawn(cat_pane()))
        .await
        .expect("spawn in time")
        .expect("spawn");
    let mut events = client.attach(spawned.pane, ATTACH).expect("attach");
    until_restored(&mut events).await;
    client.input(spawned.pane, b"real-proxy\n").expect("input");
    until_output(&mut events, b"real-proxy", 2).await;

    drop(events);
    drop(client);
    fixture.expect_released().await;
    // The real proxy exits when its stdin closes; either end may be
    // reported first.
    let end = timeout(STEP, connection.ended)
        .await
        .expect("tunnel end")
        .expect("end");
    assert!(
        matches!(end, TunnelEnd::LocalClose | TunnelEnd::RemoteExit { .. }),
        "{end:?}"
    );

    // The daemon the first proxy started still owns the pane.
    let connection = fixture.connect(&binary).await.expect("reconnect");
    let client = connection.client;
    let panes = timeout(STEP, client.list_panes())
        .await
        .expect("list in time")
        .expect("list");
    assert!(panes.iter().any(|info| info.pane == spawned.pane));
    client.kill(spawned.pane, SIGNAL_KILL).expect("kill");
    fixture.server_task.abort();
}

// Probe, install, fallback rules and the host object of the app.
#[path = "sessiond_remote_tests.rs"]
mod remote;
