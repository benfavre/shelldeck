use super::*;
use crate::handler::ClientHandler;
use russh::{
    keys::{ssh_key::Algorithm, PrivateKey},
    server::{self, Auth, Session},
    Channel, ChannelId,
};
use std::sync::Arc;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    task::JoinHandle,
    time::timeout,
};

const LIMIT: Duration = Duration::from_secs(5);

#[derive(Debug)]
enum Event {
    Forward(bool, String, u32),
    Exec(String, ChannelId, server::Handle),
    Input(Vec<u8>),
    Eof,
    Signal(String),
    Closed,
}
struct Server {
    events: mpsc::UnboundedSender<Event>,
    deny: bool,
    exec: Option<ChannelId>,
    port: u16,
}
impl server::Handler for Server {
    type Error = anyhow::Error;
    async fn auth_none(&mut self, _: &str) -> Result<Auth, Self::Error> {
        Ok(Auth::Accept)
    }
    async fn tcpip_forward(
        &mut self,
        address: &str,
        port: &mut u32,
        _: &mut Session,
    ) -> Result<bool, Self::Error> {
        self.events
            .send(Event::Forward(true, address.into(), *port))?;
        *port = u32::from(self.port);
        Ok(!self.deny)
    }
    async fn cancel_tcpip_forward(
        &mut self,
        address: &str,
        port: u32,
        _: &mut Session,
    ) -> Result<bool, Self::Error> {
        self.events
            .send(Event::Forward(false, address.into(), port))?;
        Ok(true)
    }
    async fn channel_open_session(
        &mut self,
        _: Channel<server::Msg>,
        _: &mut Session,
    ) -> Result<bool, Self::Error> {
        Ok(true)
    }
    async fn exec_request(
        &mut self,
        channel: ChannelId,
        command: &[u8],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.exec = Some(channel);
        session.channel_success(channel)?;
        session.data(channel, b"provider output before input".to_vec())?;
        self.events.send(Event::Exec(
            String::from_utf8(command.to_vec())?,
            channel,
            session.handle(),
        ))?;
        Ok(())
    }
    async fn data(
        &mut self,
        channel: ChannelId,
        data: &[u8],
        _: &mut Session,
    ) -> Result<(), Self::Error> {
        if self.exec == Some(channel) {
            self.events.send(Event::Input(data.to_vec()))?;
        }
        Ok(())
    }
    async fn channel_eof(
        &mut self,
        channel: ChannelId,
        _: &mut Session,
    ) -> Result<(), Self::Error> {
        if self.exec == Some(channel) {
            self.events.send(Event::Eof)?;
        }
        Ok(())
    }
    async fn signal(
        &mut self,
        _: ChannelId,
        signal: russh::Sig,
        _: &mut Session,
    ) -> Result<(), Self::Error> {
        self.events.send(Event::Signal(format!("{signal:?}")))?;
        Ok(())
    }
    async fn channel_close(
        &mut self,
        channel: ChannelId,
        _: &mut Session,
    ) -> Result<(), Self::Error> {
        if self.exec == Some(channel) {
            self.events.send(Event::Closed)?;
        }
        Ok(())
    }
}
struct Fixture {
    session: SshSession,
    events: mpsc::UnboundedReceiver<Event>,
    server: JoinHandle<()>,
}
async fn fixture(deny: bool) -> Fixture {
    fixture_with_port(deny, 43001).await
}
async fn fixture_with_port(deny: bool, port: u16) -> Fixture {
    fixture_options(deny, port, false).await
}
async fn fixture_options(deny: bool, port: u16, stall_input: bool) -> Fixture {
    let (client_stream, server_stream) = tokio::io::duplex(64 * 1024);
    let (events, receiver) = mpsc::unbounded_channel();
    let config = server::Config {
        window_size: if stall_input { 0 } else { 2 * 1024 * 1024 },
        inactivity_timeout: None,
        auth_rejection_time: Duration::from_millis(1),
        auth_rejection_time_initial: Some(Duration::from_millis(1)),
        keys: vec![PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap()],
        ..Default::default()
    };
    let server = tokio::spawn(async move {
        let running = server::run_stream(
            Arc::new(config),
            server_stream,
            Server {
                events,
                deny,
                exec: None,
                port,
            },
        )
        .await
        .unwrap();
        let _ = running.await;
    });
    let (event_tx, event_rx) = mpsc::unbounded_channel();
    let (forward_tx, forward_rx) = mpsc::unbounded_channel();
    let mut handle = russh::client::connect_stream(
        Arc::new(russh::client::Config::default()),
        client_stream,
        ClientHandler::new_trusting_server_key_for_test(event_tx, forward_tx),
    )
    .await
    .unwrap();
    assert!(handle
        .authenticate_none("shelldeck-fixture")
        .await
        .unwrap()
        .success());
    Fixture {
        session: SshSession::new(Uuid::new_v4(), handle, event_rx, forward_rx),
        events: receiver,
        server,
    }
}
async fn next(events: &mut mpsc::UnboundedReceiver<Event>) -> Event {
    timeout(LIMIT, events.recv())
        .await
        .expect("SSH event deadline")
        .expect("SSH events open")
}
fn spec() -> AgentCommandSpec {
    AgentCommandSpec {
        program: "codex".into(),
        args: vec!["exec".into(), "fixture prompt with ' and $(never)".into()],
        cwd: "/fixture".into(),
        stdin: Some(b"private\0input".to_vec()),
        remove_env: vec!["CLAUDECODE".into()],
    }
}
async fn admitted(
    events: &mut mpsc::UnboundedReceiver<Event>,
) -> (String, ChannelId, server::Handle, Vec<u8>) {
    assert!(matches!(next(events).await, Event::Forward(true, a, 0) if a == "127.0.0.1"));
    let Event::Exec(command, channel, handle) = next(events).await else {
        panic!("expected exec")
    };
    let mut input = vec![];
    loop {
        match next(events).await {
            Event::Input(bytes) => input.extend(bytes),
            Event::Eof => break,
            event => panic!("unexpected {event:?}"),
        }
    }
    (command, channel, handle, input)
}
async fn finish(handle: &server::Handle, channel: ChannelId, code: u32) {
    handle.eof(channel).await.unwrap();
    handle.exit_status_request(channel, code).await.unwrap();
    handle.close(channel).await.unwrap();
}

// SDTEST-2862 — real SSH channels, exact listener routing and immediate lease revocation.
#[tokio::test]
async fn sdtest_2862_remote_browser_forwarding_is_scoped_and_revocable() {
    let Fixture {
        mut session,
        mut events,
        server,
    } = fixture(false).await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = listener.local_addr().unwrap().to_string();
    let run_id = Uuid::new_v4();
    let token = "a7".repeat(32);
    let (config, lease) =
        BrowserBridgeConfig::new(run_id, session.connection_id, &endpoint, token.clone()).unwrap();
    assert!(!format!("{config:?}").contains(&token));
    let (output, mut received) = mpsc::unbounded_channel();
    let (_stop, stopped) = mpsc::channel(1);
    let task = tokio::spawn(async move {
        run(
            &mut session,
            run_id,
            AgentProvider::Codex,
            &spec(),
            config,
            output,
            stopped,
        )
        .await
    });
    let (command, channel, handle, input) = admitted(&mut events).await;
    assert_eq!(command, launcher_command());
    assert!(!command.contains(&token));
    assert!(!command.contains("fixture prompt"));
    let value: serde_json::Value = serde_json::from_slice(&input).unwrap();
    assert_eq!(value["capability"], token);
    assert_eq!(value["endpoint"], "127.0.0.1:43001");
    assert_eq!(value["args"], json!(spec().args));
    assert_eq!(
        received.recv().await.unwrap(),
        b"provider output before input"
    );
    for (address, port, origin) in [
        ("127.0.0.1", 43002, "127.0.0.1"),
        ("0.0.0.0", 43001, "127.0.0.1"),
        ("127.0.0.1", 43001, "192.0.2.1"),
    ] {
        let mut rejected = handle
            .channel_open_forwarded_tcpip(address, port, origin, 50000)
            .await
            .unwrap();
        assert!(matches!(
            timeout(LIMIT, rejected.wait()).await.unwrap(),
            Some(russh::ChannelMsg::Close) | None
        ));
        assert!(timeout(Duration::from_millis(20), listener.accept())
            .await
            .is_err());
    }
    let mut remote = handle
        .channel_open_forwarded_tcpip("127.0.0.1", 43001, "127.0.0.1", 50000)
        .await
        .unwrap()
        .into_stream();
    let (mut local, _) = timeout(LIMIT, listener.accept()).await.unwrap().unwrap();
    let envelope = json!({"capability":token,"request":{"id":1,"method":"tools/list"}}).to_string();
    remote.write_all(envelope.as_bytes()).await.unwrap();
    let mut bytes = vec![0; envelope.len()];
    local.read_exact(&mut bytes).await.unwrap();
    assert_eq!(bytes, envelope.as_bytes());
    local.write_all(b"reply\n").await.unwrap();
    let mut response = [0; 6];
    remote.read_exact(&mut response).await.unwrap();
    assert_eq!(&response, b"reply\n");
    drop(lease);
    assert!(matches!(next(&mut events).await, Event::Forward(false, a,43001) if a == "127.0.0.1"));
    assert_eq!(
        timeout(LIMIT, local.read(&mut [0; 1]))
            .await
            .unwrap()
            .unwrap(),
        0
    );
    assert!(
        !task.is_finished(),
        "closing browser must not terminate provider"
    );
    finish(&handle, channel, 23).await;
    assert_eq!(
        timeout(LIMIT, task).await.unwrap().unwrap().unwrap(),
        Some(23)
    );
    while let Ok(Some(event)) = timeout(Duration::from_millis(20), events.recv()).await {
        assert!(
            matches!(event, Event::Closed),
            "no extra listener cancellation or provider signal: {event:?}"
        );
    }
    server.abort();
}

// SDTEST-2863 — reject wrong identity, expired grant, denied forwarding and cancelled setup before exec.
#[tokio::test]
async fn sdtest_2863_remote_browser_refuses_stale_scope_and_unavailable_forwarding() {
    for (endpoint, token) in [
        ("0.0.0.0:80", "a".repeat(64)),
        ("127.0.0.1:0", "a".repeat(64)),
        ("[::1]:80", "a".repeat(64)),
        ("127.0.0.1:80", "secret".into()),
    ] {
        assert!(
            BrowserBridgeConfig::new(Uuid::new_v4(), Uuid::new_v4(), endpoint, token.clone())
                .is_err()
        );
    }
    for case in 0..7 {
        let Fixture {
            mut session,
            mut events,
            server,
        } = fixture(case == 3).await;
        let run_id = Uuid::new_v4();
        let (config, lease) = BrowserBridgeConfig::new(
            if case == 0 { Uuid::new_v4() } else { run_id },
            if case == 1 {
                Uuid::new_v4()
            } else {
                session.connection_id
            },
            "127.0.0.1:43000",
            "b".repeat(64),
        )
        .unwrap();
        let (output, _) = mpsc::unbounded_channel();
        let (stop, stopped) = mpsc::channel(1);
        let shared = session.shared_handle();
        let held = if case == 6 {
            Some(shared.lock().await)
        } else {
            None
        };
        let lease = if case == 2 {
            drop(lease);
            None
        } else {
            Some(lease)
        };
        if case == 4 || case == 6 {
            stop.send(()).await.unwrap();
        }
        let result = timeout(
            LIMIT,
            run(
                &mut session,
                run_id,
                if case == 5 {
                    AgentProvider::Jcode
                } else {
                    AgentProvider::Codex
                },
                &spec(),
                config,
                output,
                stopped,
            ),
        )
        .await
        .unwrap();
        if case == 4 || case == 6 {
            assert_eq!(result.unwrap(), None);
        } else {
            assert!(result.is_err());
        }
        drop(held);
        drop(lease);
        if case == 3 {
            assert!(matches!(
                next(&mut events).await,
                Event::Forward(true, _, 0)
            ));
        }
        assert!(
            events.try_recv().is_err(),
            "refusal must never execute provider"
        );
        server.abort();
    }
}

// SDTEST-2864 — large private stdin interleaves output, retains late status, and responds to Stop/drop.
#[tokio::test]
async fn sdtest_2864_private_stdin_streams_and_cancellation_closes_exact_channel() {
    for stop in [false, true] {
        let Fixture {
            session,
            mut events,
            server,
        } = fixture(false).await;
        let (output, mut received) = mpsc::unbounded_channel();
        let (cancel, cancelled) = mpsc::channel(1);
        let input = vec![b'p'; 512 * 1024];
        let expected = input.clone();
        let task = tokio::spawn(async move {
            session
                .exec_cancellable_with_input("static command", input, output, cancelled)
                .await
        });
        let Event::Exec(_, channel, handle) = next(&mut events).await else {
            panic!("exec")
        };
        assert_eq!(
            timeout(LIMIT, received.recv()).await.unwrap().unwrap(),
            b"provider output before input"
        );
        let mut bytes = vec![];
        loop {
            match next(&mut events).await {
                Event::Input(b) => bytes.extend(b),
                Event::Eof => break,
                e => panic!("{e:?}"),
            }
        }
        assert_eq!(bytes, expected);
        if stop {
            cancel.send(()).await.unwrap();
            assert_eq!(timeout(LIMIT, task).await.unwrap().unwrap().unwrap(), None);
            assert!(matches!(next(&mut events).await,Event::Signal(s) if s=="TERM"));
            // Repeated EOF is not required by SSH; Close is.
            loop {
                if matches!(next(&mut events).await, Event::Closed) {
                    break;
                }
            }
        } else {
            finish(&handle, channel, 17).await;
            assert_eq!(
                timeout(LIMIT, task).await.unwrap().unwrap().unwrap(),
                Some(17)
            );
        }
        server.abort();
    }
    // A server that stops extending its receive window must not trap Stop
    // behind a pending private-input write. Dropping the run also closes it.
    for dropped in [false, true] {
        let Fixture {
            mut session,
            mut events,
            server,
        } = fixture_options(false, 43001, true).await;
        let run_id = Uuid::new_v4();
        let (config, _lease) = BrowserBridgeConfig::new(
            run_id,
            session.connection_id,
            "127.0.0.1:43000",
            "c".repeat(64),
        )
        .unwrap();
        let mut large = spec();
        large.stdin = Some(vec![b'x'; 512 * 1024]);
        let (output, mut received) = mpsc::unbounded_channel();
        let (cancel, cancelled) = mpsc::channel(1);
        let task = tokio::spawn(async move {
            run(
                &mut session,
                run_id,
                AgentProvider::Codex,
                &large,
                config,
                output,
                cancelled,
            )
            .await
        });
        assert!(matches!(
            next(&mut events).await,
            Event::Forward(true, _, 0)
        ));
        assert!(matches!(next(&mut events).await, Event::Exec(..)));
        assert_eq!(
            timeout(LIMIT, received.recv()).await.unwrap().unwrap(),
            b"provider output before input"
        );
        assert!(
            timeout(Duration::from_millis(20), events.recv())
                .await
                .is_err(),
            "a zero SSH receive window blocks private input before EOF"
        );
        assert!(!task.is_finished());
        if dropped {
            task.abort();
            assert!(task.await.unwrap_err().is_cancelled());
        } else {
            cancel.send(()).await.unwrap();
            assert_eq!(timeout(LIMIT, task).await.unwrap().unwrap().unwrap(), None);
        }
        let mut signalled = false;
        let mut closed = false;
        let mut unbound = false;
        while !closed || !unbound {
            match next(&mut events).await {
                Event::Signal(signal) => {
                    assert_eq!(signal, "TERM");
                    signalled = true;
                }
                Event::Closed => closed = true,
                Event::Forward(false, address, port) => {
                    assert_eq!((address.as_str(), port), ("127.0.0.1", 43001));
                    unbound = true;
                }
                Event::Eof => {}
                event => panic!("unexpected blocked-input cleanup: {event:?}"),
            }
        }
        assert_eq!(signalled, !dropped);
        server.abort();
    }
}

// SDTEST-2867 — requires the packaged CEF runtime and an available display.
// The server side is a POSIX fixture, never a configured SSH host or provider.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires SHELLDECK_BROWSER_SMOKE_HELPER and a native display; CI native browser job"]
async fn sdtest_2867_native_browser_through_ssh_and_private_provider_adapter() {
    use shelldeck_browser::{mcp, BrowserClient, BrowserLaunch, Command, Event as BrowserEvent};
    use std::{os::unix::fs::PermissionsExt, process::Stdio};
    let helper = std::env::var_os("SHELLDECK_BROWSER_SMOKE_HELPER").expect("packaged CEF helper");
    let profile = tempfile::tempdir().unwrap();
    let client = BrowserClient::spawn(BrowserLaunch {
        executable: helper.into(),
        profile_dir: profile.path().join("profile"),
    })
    .unwrap();
    timeout(Duration::from_secs(30), async {
        loop {
            if client
                .drain_events()
                .iter()
                .any(|event| matches!(event, BrowserEvent::Ready { .. }))
            {
                break;
            }
            if let Some(error) = client.failure() {
                panic!("CEF helper failed: {error}");
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("CEF ready");
    let http = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/", http.local_addr().unwrap());
    let http_task = tokio::spawn(async move {
        while let Ok((mut stream, _)) = http.accept().await {
            tokio::spawn(async move {
                let mut request = [0; 8192];
                let _ = stream.read(&mut request).await;
                let html="<!doctype html><title>SSH browser fixture</title><h1>SSH browser fixture</h1><input aria-label='SSH input' style='position:absolute;left:40px;top:80px;width:240px;height:40px'>";
                let response=format!("HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{html}",html.len());
                let _ = stream.write_all(response.as_bytes()).await;
            });
        }
    });
    let id = client.create_tab(&url, 800, 600, 1.0).unwrap();
    let _foreign = client.create_tab("about:blank", 800, 600, 1.0).unwrap();
    timeout(Duration::from_secs(30), async {
        while client.latest_frame(id).is_none()
            || client
                .state(id)
                .is_none_or(|s| s.loading || s.title != "SSH browser fixture")
        {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("CEF fixture loaded and painted");
    for (provider, permission, case) in [
        (
            AgentProvider::Codex,
            mcp::BrowserPermission::ObserveOnly,
            "native-observe",
        ),
        (
            AgentProvider::Claude,
            mcp::BrowserPermission::Control,
            "native-control",
        ),
    ] {
        let grant = mcp::grant(client.clone(), [id], permission).unwrap();
        let grant_config = grant.configuration();
        // A fixture listener stands in for the remote sshd listener. Every
        // accepted proxy byte crosses a real encrypted forwarded-tcpip channel.
        let remote_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = remote_listener.local_addr().unwrap().port();
        let Fixture {
            mut session,
            mut events,
            server,
        } = fixture_with_port(false, port).await;
        let run_id = Uuid::new_v4();
        let (config, lease) = BrowserBridgeConfig::new(
            run_id,
            session.connection_id,
            &grant_config.env[mcp::ENDPOINT_ENV],
            grant_config.env[mcp::CAPABILITY_ENV].clone(),
        )
        .unwrap();
        let fake = profile.path().join("fake-provider");
        std::fs::write(
            &fake,
            include_str!("../../../scripts/fixtures/browser-provider.py"),
        )
        .unwrap();
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o700)).unwrap();
        let spec = AgentCommandSpec {
            program: fake.display().to_string(),
            args: vec!["exec".into(), "--fixture-case".into(), case.into()],
            cwd: profile.path().into(),
            stdin: Some(b"native fixture".to_vec()),
            remove_env: vec![],
        };
        let (output, mut received) = mpsc::unbounded_channel();
        let (_stop, stopped) = mpsc::channel(1);
        let task = tokio::spawn(async move {
            run(
                &mut session,
                run_id,
                provider,
                &spec,
                config,
                output,
                stopped,
            )
            .await
        });
        let (command, channel, handle, input) = admitted(&mut events).await;
        assert_eq!(command, launcher_command());
        let forwarding_handle = handle.clone();
        let forwarding = tokio::spawn(async move {
            while let Ok((mut stream, _)) = remote_listener.accept().await {
                let handle = forwarding_handle.clone();
                tokio::spawn(async move {
                    let mut channel = handle
                        .channel_open_forwarded_tcpip(
                            "127.0.0.1",
                            u32::from(port),
                            "127.0.0.1",
                            50000,
                        )
                        .await
                        .unwrap()
                        .into_stream();
                    let _ = copy_bidirectional(&mut stream, &mut channel).await;
                });
            }
        });
        // Execute precisely the payload received through SSH. No credentials,
        // user provider or real remote process is consulted by this fixture.
        let mut process = tokio::process::Command::new("python3")
            .args(["-u", "-c", LAUNCHER])
            .env("TMPDIR", profile.path())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        let mut stdin = process.stdin.take().unwrap();
        let send = async move {
            stdin.write_all(&input).await.unwrap();
            stdin.shutdown().await.unwrap();
            drop(stdin);
        };
        let (_, result) = timeout(Duration::from_secs(40), async {
            tokio::join!(send, process.wait_with_output())
        })
        .await
        .expect("bounded fake provider");
        let result = result.unwrap();
        assert!(
            result.status.success(),
            "native fake provider stderr: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        let lines: Vec<serde_json::Value> = String::from_utf8(result.stdout.clone())
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(lines[1]["native_browser"], true);
        assert!(!std::path::Path::new(lines[0]["fixture_setup"].as_str().unwrap()).exists());
        handle.data(channel, result.stdout).await.unwrap();
        finish(&handle, channel, 0).await;
        assert_eq!(
            timeout(LIMIT, task).await.unwrap().unwrap().unwrap(),
            Some(0)
        );
        loop {
            match next(&mut events).await {
                Event::Closed => {}
                Event::Forward(false, a, p) => {
                    assert_eq!((a.as_str(), p), ("127.0.0.1", u32::from(port)));
                    break;
                }
                event => panic!("unexpected native cleanup: {event:?}"),
            }
        }
        let mut all = vec![];
        while let Some(bytes) = received.recv().await {
            all.extend(bytes);
        }
        assert!(String::from_utf8(all).unwrap().contains("native_browser"));
        if permission == mcp::BrowserPermission::Control {
            let value=client.devtools(id,"Runtime.evaluate",json!({"expression":"document.querySelector('input').value","returnByValue":true}),Duration::from_secs(10)).unwrap();
            assert_eq!(value["result"]["value"], "via SSH");
        }
        drop(lease);
        drop(grant);
        forwarding.abort();
        server.abort();
    }
    client.send(Command::Shutdown).unwrap();
    timeout(Duration::from_secs(10), async {
        while client.failure().is_none() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("CEF graceful shutdown");
    http_task.abort();
}
