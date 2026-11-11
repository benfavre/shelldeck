//! Tests of the remote binary (probe, upload, checksum), the fallback
//! outcomes and the host object the app uses, on the in-memory SSH
//! fixture of the parent module.
//!
//! The "remote host" is the real `/bin/sh` under a private temporary
//! `HOME`, and an in-process daemon on a private temporary run directory.
//! The uploaded "binary" is a made-up ELF image that is never executed:
//! the fixture relays the proxy command to the in-process daemon once the
//! installed file exists and is executable. Nothing here touches the
//! user's home, sockets or processes, and no real host is contacted.

use super::*;
use crate::sessiond_host::{
    Dial, HostEvent, InstallFailureKind, Prepared, RemoteHost, RemoteHostOptions,
};
use crate::sessiond_install::{
    artifact_file_name, install, probe, sha256_hex, target_triple, FallbackReason, InstallConsent,
    InstallError, ProbeError, RemoteBinaryState,
};
use crate::SshError;
use shelldeck_sessiond::attached::SessionOptions;
use shelldeck_sessiond::supervisor::{Backoff, PaneTarget, SupervisorOptions, UNLIMITED_ATTEMPTS};
use shelldeck_terminal::attached::AttachedPhase;
use shelldeck_terminal::session::TerminalSession;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

const VERSION: &str = "9.9.9-test";
const LIMIT: Duration = Duration::from_secs(20);
const PANE_LABEL: &str = "shelldeck-remote";

fn uname(flag: &str) -> String {
    let output = std::process::Command::new("uname")
        .arg(flag)
        .output()
        .expect("uname");
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

/// The bundled target of the machine running the tests (it plays the
/// remote host).
fn local_target() -> &'static str {
    target_triple(&uname("-s"), &uname("-m")).expect("the tests run on Linux x86_64 or aarch64")
}

/// A made-up executable image for `target`: a valid header for the format
/// check, then 300 KiB of pseudo-random bytes so the upload spans many
/// SSH packets.
fn fake_artifact(target: &str, seed: u32) -> Vec<u8> {
    let machine: u16 = if target.starts_with("x86_64") {
        62
    } else {
        183
    };
    let mut bytes = b"\x7fELF\x02\x01\x01\x00".to_vec();
    bytes.resize(18, 0);
    bytes.extend_from_slice(&machine.to_le_bytes());
    let mut state = seed | 1;
    while bytes.len() < 300 * 1024 {
        state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        bytes.push((state >> 24) as u8);
    }
    // Built "for" the version the tests install it as.
    bytes.extend_from_slice(shelldeck_sessiond::build_marker(VERSION).as_bytes());
    bytes
}

fn installed_path(home: &Path) -> PathBuf {
    home.join(".shelldeck/bin")
        .join(format!("shelldeck-sessiond-{VERSION}"))
}

fn mode_of(path: &Path) -> u32 {
    std::fs::metadata(path)
        .expect("metadata")
        .permissions()
        .mode()
        & 0o7777
}

fn bin_entries(home: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(home.join(".shelldeck/bin"))
        .expect("read bin dir")
        .map(|entry| {
            entry
                .expect("entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    names.sort();
    names
}

fn host_mode(home: &Path, dir: &Path) -> HostMode {
    HostMode {
        home: home.to_path_buf(),
        dir: dir.to_path_buf(),
        path: None,
        tamper_stdin: false,
        refuse_proxy: false,
    }
}

/// Exec requests received so far, in order.
fn drain_commands(events: &mut mpsc::UnboundedReceiver<ServerEvent>) -> Vec<String> {
    let mut commands = Vec::new();
    while let Ok(event) = events.try_recv() {
        if let ServerEvent::Exec(command) = event {
            commands.push(command);
        }
    }
    commands
}

// SDTEST-3017 (SDUC-574): probe and upload against the real /bin/sh under
// a private HOME.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sdtest_3017_probe_and_upload_install_a_verified_private_binary() {
    let home = temp_dir();
    let run = temp_dir();
    let mut fixture = Fixture::start(ExecMode::Host(host_mode(home.path(), run.path()))).await;
    let session = fixture.session.clone();
    let target = local_target();

    // Nothing installed: the probe says so, names the platform and writes
    // nothing.
    let probed = timeout(STEP, probe(&session, VERSION))
        .await
        .expect("probe in time")
        .expect("probe");
    assert_eq!(probed.binary, RemoteBinaryState::Absent);
    assert_eq!(
        (probed.os.as_str(), probed.arch.as_str()),
        (uname("-s").as_str(), uname("-m").as_str())
    );
    assert_eq!(probed.target(), Some(target));
    assert!(
        !home.path().join(".shelldeck").exists(),
        "a probe writes nothing"
    );

    // Upload: the remote digest equals the local one, the file is private
    // and executable, and no temporary file stays.
    let artifact = fake_artifact(target, 1);
    let digest = timeout(STEP, install(&session, VERSION, &artifact))
        .await
        .expect("install in time")
        .expect("install");
    assert_eq!(digest, sha256_hex(&artifact));
    let installed = installed_path(home.path());
    assert_eq!(std::fs::read(&installed).expect("installed file"), artifact);
    assert_eq!(mode_of(&installed), 0o700);
    assert_eq!(mode_of(&home.path().join(".shelldeck")), 0o700);
    assert_eq!(mode_of(&home.path().join(".shelldeck/bin")), 0o700);
    assert_eq!(
        bin_entries(home.path()),
        vec![format!("shelldeck-sessiond-{VERSION}")]
    );
    // Nothing else appeared in the home directory.
    let top: Vec<_> = std::fs::read_dir(home.path())
        .expect("read home")
        .map(|entry| entry.expect("entry").file_name())
        .collect();
    assert_eq!(top, vec![std::ffi::OsString::from(".shelldeck")]);

    let probed = timeout(STEP, probe(&session, VERSION))
        .await
        .expect("probe in time")
        .expect("probe");
    assert_eq!(probed.binary, RemoteBinaryState::Present);
    // Another version is a different file.
    let other = timeout(STEP, probe(&session, "9.9.8"))
        .await
        .expect("probe in time")
        .expect("probe");
    assert_eq!(other.binary, RemoteBinaryState::Absent);

    // A file that lost its execute bit is reported as such, and an upload
    // over it replaces it.
    std::fs::set_permissions(&installed, std::fs::Permissions::from_mode(0o600)).expect("chmod");
    let probed = timeout(STEP, probe(&session, VERSION))
        .await
        .expect("probe in time")
        .expect("probe");
    assert_eq!(probed.binary, RemoteBinaryState::NotExecutable);
    let newer = fake_artifact(target, 2);
    timeout(STEP, install(&session, VERSION, &newer))
        .await
        .expect("install in time")
        .expect("install over an existing file");
    assert_eq!(std::fs::read(&installed).expect("installed file"), newer);
    assert_eq!(mode_of(&installed), 0o700);

    // Only fixed scripts were sent, each through `sh -c`, never `sudo`.
    let commands = drain_commands(&mut fixture.events);
    assert_eq!(commands.len(), 6, "{commands:?}");
    for command in &commands {
        assert!(command.starts_with("sh -c '"), "{command}");
        assert!(!command.contains("sudo"), "{command}");
        assert!(command.contains("$HOME/.shelldeck/bin"), "{command}");
    }

    // A version that could be shell syntax never reaches the host.
    let error = probe(&session, "1.0;id").await.expect_err("bad version");
    assert!(matches!(error, ProbeError::InvalidVersion), "{error:?}");
    let error = install(&session, "1.0;id", &artifact)
        .await
        .expect_err("bad version");
    assert!(matches!(error, InstallError::InvalidVersion), "{error:?}");
    let error = install(&session, VERSION, &[]).await.expect_err("empty");
    assert!(matches!(error, InstallError::BadArtifact), "{error:?}");
    assert!(drain_commands(&mut fixture.events).is_empty());
    fixture.server_task.abort();
}

// SDTEST-3049 (SDUC-574): an install prunes the builds of older releases
// on the host (real /bin/sh, private HOME). The three most recently
// installed builds stay, the one just installed among them; anything
// that is not exactly a build of the app (another name, a directory, a
// symlink) is never touched; an older release installing its build again
// makes that one the newest.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sdtest_3049_an_install_keeps_the_three_newest_builds_on_the_host() {
    let home = temp_dir();
    let run = temp_dir();
    let fixture = Fixture::start(ExecMode::Host(host_mode(home.path(), run.path()))).await;
    let session = fixture.session.clone();
    let bin = home.path().join(".shelldeck/bin");
    std::fs::create_dir_all(&bin).expect("bin directory");
    let aged = |name: &str, days: u64| {
        let path = bin.join(name);
        std::fs::write(&path, name).expect("write");
        std::fs::File::options()
            .write(true)
            .open(&path)
            .and_then(|file| {
                file.set_modified(std::time::SystemTime::now() - Duration::from_secs(days * 86_400))
            })
            .expect("set the modification time");
    };
    // Four builds of older releases.
    aged("shelldeck-sessiond-0.17.0", 40);
    aged("shelldeck-sessiond-0.18.0", 30);
    aged("shelldeck-sessiond-0.18.2", 20);
    aged("shelldeck-sessiond-0.19.0", 10);
    // Not builds of the app, all older than every build.
    let bystanders = [
        "notes.txt",
        "shelldeck-sessiond-0.16.0 copy",
        "shelldeck-sessiond-OLD",
        "shelldeck-workspace-helper",
    ];
    for name in bystanders {
        aged(name, 400);
    }
    std::fs::create_dir(bin.join("shelldeck-sessiond-0.1.0")).expect("directory");
    std::os::unix::fs::symlink("notes.txt", bin.join("shelldeck-sessiond-0.2.0")).expect("symlink");
    let listing = |builds: &[&str]| {
        let mut names: Vec<String> = bystanders
            .iter()
            .chain(["shelldeck-sessiond-0.1.0", "shelldeck-sessiond-0.2.0"].iter())
            .chain(builds)
            .map(|name| name.to_string())
            .collect();
        names.sort();
        names
    };
    let current = format!("shelldeck-sessiond-{VERSION}");

    let artifact = fake_artifact(local_target(), 9);
    timeout(STEP, install(&session, VERSION, &artifact))
        .await
        .expect("install in time")
        .expect("install");
    assert_eq!(
        bin_entries(home.path()),
        listing(&[
            &current,
            "shelldeck-sessiond-0.19.0",
            "shelldeck-sessiond-0.18.2"
        ])
    );
    assert_eq!(
        std::fs::read(bin.join(&current)).expect("installed"),
        artifact
    );

    // An app of an older release connects and installs its build again.
    timeout(STEP, install(&session, "0.17.0", &artifact))
        .await
        .expect("install in time")
        .expect("install of an older release");
    assert_eq!(
        bin_entries(home.path()),
        listing(&[
            "shelldeck-sessiond-0.17.0",
            &current,
            "shelldeck-sessiond-0.19.0"
        ])
    );
    fixture.server_task.abort();
}

// SDTEST-3018 (SDUC-574): an upload that does not verify installs nothing.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sdtest_3018_an_upload_that_fails_its_checksum_installs_nothing() {
    let home = temp_dir();
    let run = temp_dir();
    let target = local_target();
    let installed = installed_path(home.path());

    // A good install first, to see that a later bad upload leaves it alone.
    let good = fake_artifact(target, 3);
    let fixture = Fixture::start(ExecMode::Host(host_mode(home.path(), run.path()))).await;
    timeout(STEP, install(&fixture.session, VERSION, &good))
        .await
        .expect("install in time")
        .expect("install");
    fixture.server_task.abort();

    // One byte changes on the way: the remote digest differs.
    let fixture = Fixture::start(ExecMode::Host(HostMode {
        tamper_stdin: true,
        ..host_mode(home.path(), run.path())
    }))
    .await;
    let other = fake_artifact(target, 4);
    let error = timeout(STEP, install(&fixture.session, VERSION, &other))
        .await
        .expect("install in time")
        .expect_err("a corrupted upload must fail");
    assert!(matches!(error, InstallError::ChecksumMismatch), "{error:?}");
    assert_eq!(std::fs::read(&installed).expect("installed file"), good);
    assert_eq!(
        bin_entries(home.path()),
        vec![format!("shelldeck-sessiond-{VERSION}")],
        "the temporary file was removed"
    );
    fixture.server_task.abort();

    // A host with no tool to compute a digest: nothing is installed
    // unverified.
    let tools = temp_dir();
    for tool in ["sh", "cat", "mkdir", "chmod", "mv", "rm", "uname"] {
        let real = ["/bin", "/usr/bin"]
            .iter()
            .map(|dir| Path::new(dir).join(tool))
            .find(|path| path.exists())
            .unwrap_or_else(|| panic!("{tool} not found"));
        std::os::unix::fs::symlink(real, tools.path().join(tool)).expect("symlink");
    }
    let bare_home = temp_dir();
    let fixture = Fixture::start(ExecMode::Host(HostMode {
        path: Some(tools.path().to_path_buf()),
        ..host_mode(bare_home.path(), run.path())
    }))
    .await;
    let error = timeout(STEP, install(&fixture.session, VERSION, &other))
        .await
        .expect("install in time")
        .expect_err("an unverifiable upload must fail");
    assert!(matches!(error, InstallError::NoChecksumTool), "{error:?}");
    assert!(bin_entries(bare_home.path()).is_empty());
    // The probe needs none of those tools.
    let probed = timeout(STEP, probe(&fixture.session, VERSION))
        .await
        .expect("probe in time")
        .expect("probe");
    assert_eq!(probed.binary, RemoteBinaryState::Absent);
    fixture.server_task.abort();
}

// ---------------------------------------------------------------------------
// Host object
// ---------------------------------------------------------------------------

/// A remote host for [`RemoteHost`]: every dial starts a new in-memory SSH
/// server in front of the same private home and the same daemon.
struct World {
    home: tempfile::TempDir,
    _run: tempfile::TempDir,
    artifacts: tempfile::TempDir,
    daemon: Option<ServerHandle>,
    events: mpsc::UnboundedReceiver<ServerEvent>,
    event_tx: mpsc::UnboundedSender<ServerEvent>,
    servers: Arc<parking_lot::Mutex<Vec<JoinHandle<()>>>>,
    /// Handles of the dialed connections, to cut them.
    connections: Arc<parking_lot::Mutex<Vec<crate::session::SharedHandle>>>,
    dials: Arc<AtomicUsize>,
    mode: Arc<parking_lot::Mutex<HostMode>>,
    refuse_dial: Arc<AtomicBool>,
    // Last: everything above may still use it while it is dropped.
    runtime: tokio::runtime::Runtime,
}

impl World {
    fn new() -> Self {
        let home = temp_dir();
        let run = temp_dir();
        // The daemon makes its own (private) run directory.
        let run_dir = run.path().join("run");
        let daemon = start_daemon(&run_dir);
        let (event_tx, events) = mpsc::unbounded_channel();
        let mode = host_mode(home.path(), &run_dir);
        Self {
            home,
            _run: run,
            artifacts: temp_dir(),
            daemon: Some(daemon),
            events,
            event_tx,
            servers: Arc::default(),
            connections: Arc::default(),
            dials: Arc::default(),
            mode: Arc::new(parking_lot::Mutex::new(mode)),
            refuse_dial: Arc::default(),
            runtime: tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
                .expect("runtime"),
        }
    }

    fn host(&self) -> RemoteHost {
        let (mode, events) = (self.mode.clone(), self.event_tx.clone());
        let (servers, dials) = (self.servers.clone(), self.dials.clone());
        let refuse = self.refuse_dial.clone();
        let connections = self.connections.clone();
        let dial: Dial = Arc::new(move || {
            let (mode, events) = (mode.lock().clone(), events.clone());
            let connections = connections.clone();
            let (servers, dials, refuse) = (servers.clone(), dials.clone(), refuse.clone());
            Box::pin(async move {
                dials.fetch_add(1, Ordering::SeqCst);
                if refuse.load(Ordering::SeqCst) {
                    return Err(SshError::ConnectionFailed("refused by the test".into()));
                }
                let (session, server) = open_session(ExecMode::Host(mode), events).await;
                servers.lock().push(server);
                connections.lock().push(session.shared_handle());
                Ok(session)
            })
        });
        RemoteHost::new(
            self.runtime.handle().clone(),
            dial,
            RemoteHostOptions {
                version: VERSION.into(),
                artifact_dirs: vec![self.artifacts.path().to_path_buf()],
                supervisor: SupervisorOptions {
                    backoff: Backoff {
                        start: Duration::from_millis(50),
                        max: Duration::from_millis(400),
                    },
                    // As the app sets it for a remote host.
                    max_attempts: UNLIMITED_ATTEMPTS,
                    ..SupervisorOptions::default()
                },
                ..RemoteHostOptions::default()
            },
        )
        .expect("remote host")
    }

    fn put_artifact(&self, bytes: &[u8]) {
        let name = artifact_file_name(local_target());
        std::fs::write(self.artifacts.path().join(name), bytes).expect("write artifact");
    }

    fn dials(&self) -> usize {
        self.dials.load(Ordering::SeqCst)
    }

    /// The SSH connections go away without a channel EOF or Close, as on
    /// a network loss.
    fn cut_connections(&self) {
        let connections: Vec<_> = self.connections.lock().drain(..).collect();
        self.runtime.block_on(async {
            for connection in connections {
                let _ = connection
                    .lock()
                    .await
                    .disconnect(russh::Disconnect::ByApplication, "", "en")
                    .await;
            }
        });
        for server in self.servers.lock().drain(..) {
            server.abort();
        }
    }

    fn commands(&mut self) -> Vec<String> {
        drain_commands(&mut self.events)
    }
}

impl Drop for World {
    fn drop(&mut self) {
        self.cut_connections();
        if let Some(daemon) = self.daemon.take() {
            daemon.stop();
        }
    }
}

fn wait_until(what: &str, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + LIMIT;
    while !done() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn screen_text(session: &TerminalSession) -> String {
    session
        .grid
        .lock()
        .visible_rows()
        .iter()
        .map(|row| row.iter().map(|cell| cell.c).collect::<String>())
        .map(|row| row.trim_end().to_string())
        .collect::<Vec<_>>()
        .join("\n")
}

fn phase(session: &TerminalSession) -> AttachedPhase {
    session.attached_status().expect("attached session").phase()
}

fn view(title: &str) -> SessionOptions {
    SessionOptions {
        title: title.into(),
        rows: 24,
        cols: 80,
        scrollback: 1_000,
        keep_pane_size: false,
    }
}

fn cat() -> PaneTarget {
    PaneTarget::Spawn(SpawnOptions {
        label: PANE_LABEL.into(),
        argv: vec!["/bin/cat".into()],
        ..SpawnOptions::default()
    })
}

fn is_proxy(command: &str) -> bool {
    command.contains(" proxy --hello ")
}

fn expect_fallback(prepared: Prepared) -> (FallbackReason, bool, Option<Arc<SshSession>>) {
    match prepared {
        Prepared::Fallback {
            reason,
            ask,
            session,
        } => (reason, ask, session),
        Prepared::Ready { .. } => panic!("expected a fallback, the daemon is ready"),
        Prepared::ConnectFailed(error) => panic!("expected a fallback, got {error}"),
    }
}

// SDTEST-3019 (SDUC-574): every way a connection ends without the daemon,
// and what was (not) written on the host each time.
#[test]
fn sdtest_3019_without_consent_or_a_usable_binary_the_host_falls_back_untouched() {
    let mut world = World::new();
    let host = world.host();
    let target = local_target();
    let good = fake_artifact(target, 5);
    world.put_artifact(&good);
    let dot_dir = world.home.path().join(".shelldeck");

    // Never asked: fall back and offer the install. One probe was sent,
    // nothing was written, and the SSH connection is handed over alive.
    let (reason, ask, session) = expect_fallback(host.prepare(InstallConsent::Ask));
    assert_eq!((reason, ask), (FallbackReason::ConsentPending, true));
    assert!(!dot_dir.exists());
    assert!(!host.is_ready());
    let commands = world.commands();
    assert_eq!(commands.len(), 1, "{commands:?}");
    assert!(commands[0].contains("SDSD-PROBE-"), "{commands:?}");
    let session = session.expect("the connection is handed over for the fallback backend");
    let echoed = world
        .runtime
        .block_on(session.exec("echo fallback-shell"))
        .expect("exec on the handed over connection");
    assert_eq!(echoed.stdout_string().trim(), "fallback-shell");
    drop(session);
    world.commands();

    // "Never for this host": no upload, and no further question.
    let (reason, ask, session) = expect_fallback(host.prepare(InstallConsent::Never));
    assert_eq!((reason, ask), (FallbackReason::ConsentDeclined, false));
    assert!(session.is_some());
    assert!(!dot_dir.exists());
    assert_eq!(world.commands().len(), 1, "only the probe");
    assert_eq!(world.dials(), 2, "each fallback gave its connection away");

    // Agreed, but the bundled file is not an executable for that host:
    // it does not count as a build, nothing is sent beyond the probe and
    // nothing is offered.
    world.put_artifact(b"#!/bin/sh\necho not a daemon\n");
    for consent in [InstallConsent::Allow, InstallConsent::Ask] {
        let (reason, ask, _) = expect_fallback(host.prepare(consent));
        assert_eq!((reason, ask), (FallbackReason::NoArtifact, false));
        assert!(!dot_dir.exists());
        assert_eq!(world.commands().len(), 1, "only the probe");
    }

    // SDTEST-3044: agreed, and the bundled file is the build of another
    // release (left beside the app by an update without remote builds):
    // never uploaded under this release's name, and nothing is offered.
    let mut stale = fake_artifact(target, 5);
    stale.truncate(300 * 1024);
    stale.extend_from_slice(shelldeck_sessiond::build_marker("9.9.8-test").as_bytes());
    world.put_artifact(&stale);
    for consent in [InstallConsent::Allow, InstallConsent::Ask] {
        let (reason, ask, _) = expect_fallback(host.prepare(consent));
        assert_eq!((reason, ask), (FallbackReason::NoArtifact, false));
        assert!(!dot_dir.exists());
        assert_eq!(world.commands().len(), 1, "only the probe");
    }

    // Agreed, no build bundled for the host's platform.
    std::fs::remove_file(world.artifacts.path().join(artifact_file_name(target))).expect("remove");
    let (reason, ask, _) = expect_fallback(host.prepare(InstallConsent::Allow));
    assert_eq!((reason, ask), (FallbackReason::NoArtifact, false));
    let (reason, ask, _) = expect_fallback(host.prepare(InstallConsent::Ask));
    assert_eq!(
        (reason, ask),
        (FallbackReason::NoArtifact, false),
        "nothing to offer"
    );
    assert!(!dot_dir.exists());
    world.commands();

    // Agreed, and the upload is damaged on the way: nothing installed.
    world.put_artifact(&good);
    world.mode.lock().tamper_stdin = true;
    let (reason, _, _) = expect_fallback(host.prepare(InstallConsent::Allow));
    assert_eq!(reason, FallbackReason::InstallFailed);
    assert!(bin_entries(world.home.path()).is_empty());
    let commands = world.commands();
    assert_eq!(
        commands.len(),
        2,
        "probe and upload, no proxy: {commands:?}"
    );
    assert!(!commands.iter().any(|command| is_proxy(command)));
    world.mode.lock().tamper_stdin = false;

    // Installed, but the account refuses to run it (forced command): the
    // typed "unavailable" fallback, connection handed over.
    world.mode.lock().refuse_proxy = true;
    let (reason, ask, session) = expect_fallback(host.prepare(InstallConsent::Allow));
    assert_eq!((reason, ask), (FallbackReason::Unavailable, false));
    assert!(session.is_some());
    assert_eq!(
        std::fs::read(installed_path(world.home.path())).expect("installed"),
        good
    );
    assert!(!host.is_ready());
    world.mode.lock().refuse_proxy = false;

    // Present without the execute bit and "never": not replaced, and the
    // proxy is not even tried.
    let installed = installed_path(world.home.path());
    std::fs::set_permissions(&installed, std::fs::Permissions::from_mode(0o600)).expect("chmod");
    world.commands();
    let (reason, _, _) = expect_fallback(host.prepare(InstallConsent::Never));
    assert_eq!(reason, FallbackReason::ConsentDeclined);
    assert_eq!(mode_of(&installed), 0o600);
    assert_eq!(world.commands().len(), 1, "only the probe");

    // The host cannot be reached at all: not a fallback, a connect failure.
    world.refuse_dial.store(true, Ordering::SeqCst);
    assert!(matches!(
        host.prepare(InstallConsent::Allow),
        Prepared::ConnectFailed(SshError::ConnectionFailed(_))
    ));
    world.refuse_dial.store(false, Ordering::SeqCst);

    // The user answers the offer later: the binary is uploaded right away
    // and the connection is released again.
    std::fs::remove_file(&installed).expect("remove");
    host.install().expect("install after the user agreed");
    assert_eq!(std::fs::read(&installed).expect("installed"), good);
    assert_eq!(mode_of(&installed), 0o700);
    assert!(!host.is_ready());
    // From then on no answer is needed: the binary is there.
    assert!(matches!(
        host.prepare(InstallConsent::Never),
        Prepared::Ready { .. }
    ));
    assert!(host.is_ready());
    host.release_if_idle();
    assert!(!host.is_ready());
}

// SDTEST-3046 (SDUC-574): a host that cannot execute what is uploaded (a
// home directory mounted `noexec`) gets the binary once, not once per
// connection. The stand-in for the mount is a `chmod` that changes
// nothing, so the uploaded file never becomes executable: the install
// script notices, removes the file and says so, and the host is not
// uploaded to, nor asked about, again in this run.
#[test]
fn sdtest_3046_a_host_that_cannot_execute_the_upload_is_uploaded_to_once() {
    let mut world = World::new();
    let tools = temp_dir();
    for tool in ["sh", "cat", "sha256sum", "mkdir", "mv", "rm", "uname"] {
        let real = ["/bin", "/usr/bin"]
            .iter()
            .map(|dir| Path::new(dir).join(tool))
            .find(|path| path.exists())
            .unwrap_or_else(|| panic!("{tool} not found"));
        std::os::unix::fs::symlink(real, tools.path().join(tool)).expect("symlink");
    }
    let fake_chmod = tools.path().join("chmod");
    std::fs::write(&fake_chmod, "#!/bin/sh\nexit 0\n").expect("write chmod");
    std::fs::set_permissions(&fake_chmod, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    world.mode.lock().path = Some(tools.path().to_path_buf());
    let host = world.host();
    world.put_artifact(&fake_artifact(local_target(), 7));

    // First connection: probe, upload, "cannot execute". Nothing stays on
    // the host and the fallback names the reason.
    let (reason, ask, session) = expect_fallback(host.prepare(InstallConsent::Allow));
    assert_eq!((reason, ask), (FallbackReason::Unavailable, false));
    assert!(
        session.is_some(),
        "the connection goes to the fallback shell"
    );
    assert!(bin_entries(world.home.path()).is_empty());
    let commands = world.commands();
    assert_eq!(commands.len(), 2, "probe and upload: {commands:?}");
    assert!(commands[1].contains("SDSD-INSTALL-"), "{commands:?}");

    // Every later connection of this run: one probe, no upload, whatever
    // the recorded answer, and no question.
    for consent in [
        InstallConsent::Allow,
        InstallConsent::Allow,
        InstallConsent::Ask,
    ] {
        let (reason, ask, _) = expect_fallback(host.prepare(consent));
        assert_eq!((reason, ask), (FallbackReason::Unavailable, false));
        let commands = world.commands();
        assert_eq!(commands.len(), 1, "only the probe: {commands:?}");
        assert!(commands[0].contains("SDSD-PROBE-"), "{commands:?}");
    }
    // The answer given later to an earlier question uploads nothing.
    // That refusal is typed, so the app can say it in the user's language.
    let refused = host
        .install()
        .expect_err("no upload to a host that cannot run it");
    assert_eq!(
        refused.kind,
        InstallFailureKind::Fallback(FallbackReason::Unavailable)
    );
    assert!(!world
        .commands()
        .iter()
        .any(|command| command.contains("SDSD-INSTALL-")));
    assert!(bin_entries(world.home.path()).is_empty());
}

// SDTEST-3048 (SDUC-574): two review cases on the remote path.
//
// Closing the last tab of a host ends its pane even when the program
// ignores the hang-up: the SSH connection is kept until the pane was
// killed and removed, and only then released (`Idle`).
//
// A host that stays away much longer than a local daemon would be waited
// for does not end its tabs: they reattach when it is back.
#[test]
fn sdtest_3048_a_closed_remote_pane_is_ended_and_a_long_outage_is_waited_out() {
    let world = World::new();
    let host = world.host();
    world.put_artifact(&fake_artifact(local_target(), 8));
    let (event_tx, host_events) = std::sync::mpsc::channel();
    host.set_event_handler(move |event| {
        let _ = event_tx.send(event);
    });
    let ready_panes = |host: &RemoteHost| match host.prepare(InstallConsent::Allow) {
        Prepared::Ready { panes } => panes,
        _ => panic!("the daemon should be ready"),
    };
    assert!(ready_panes(&host).is_empty());

    let stubborn = PaneTarget::Spawn(SpawnOptions {
        label: PANE_LABEL.into(),
        argv: vec![
            "/bin/sh".into(),
            "-c".into(),
            "trap '' HUP; echo stubborn_ready; while :; do sleep 1; done".into(),
        ],
        ..SpawnOptions::default()
    });
    let session = host
        .supervisor()
        .open(stubborn, view("stubborn"))
        .expect("open");
    wait_until("the pane runs", || {
        screen_text(&session).contains("stubborn_ready")
    });
    let pane = session.daemon_pane().expect("pane");

    // The user closes the tab: the last session of the host.
    session.terminate();
    drop(session);
    assert_eq!(host_events.recv_timeout(LIMIT), Ok(HostEvent::Idle));
    assert!(!host.is_ready(), "released once the pane was gone");
    // Nothing is left on the host to be offered back.
    let panes = ready_panes(&host);
    assert!(
        !panes.iter().any(|info| info.pane == pane),
        "the closed pane is still on the host: {panes:?}"
    );

    // A tab on a host that goes away for 20 failed dials (a local session
    // gives up after 12).
    let session = host.supervisor().open(cat(), view("away")).expect("open");
    wait_until("the pane is live", || {
        phase(&session) == AttachedPhase::Live
    });
    session.write_input(b"before-outage\n");
    wait_until("the echo", || {
        screen_text(&session).matches("before-outage").count() == 2
    });
    let dials_before = world.dials();
    world.refuse_dial.store(true, Ordering::SeqCst);
    world.cut_connections();
    wait_until("the tab to wait for the host", || {
        phase(&session) == AttachedPhase::Reconnecting
    });
    wait_until("twenty failed dials", || world.dials() >= dials_before + 20);
    assert_eq!(phase(&session), AttachedPhase::Reconnecting);
    assert_eq!(host.supervisor().session_count(), 1, "still trying");
    world.refuse_dial.store(false, Ordering::SeqCst);
    wait_until("the tab to reattach", || {
        phase(&session) == AttachedPhase::Live
    });
    assert!(screen_text(&session).contains("before-outage"));
    session.write_input(b"after-outage\n");
    wait_until("the echo after the outage", || {
        screen_text(&session).matches("after-outage").count() == 2
    });
    session.terminate();
    drop(session);
    wait_until("the host to be released", || !host.is_ready());
}

// SDTEST-3020 (SDUC-574): the whole remote path, as the app drives it.
#[test]
fn sdtest_3020_remote_panes_share_one_connection_survive_and_are_offered_back() {
    let mut world = World::new();
    let host = world.host();
    let artifact = fake_artifact(local_target(), 6);
    world.put_artifact(&artifact);
    let (event_tx, host_events) = std::sync::mpsc::channel();
    host.set_event_handler(move |event| {
        let _ = event_tx.send(event);
    });
    let next_event = || host_events.recv_timeout(LIMIT).expect("host event");

    // First contact with consent: probe, upload, verify, connect.
    let panes = match host.prepare(InstallConsent::Allow) {
        Prepared::Ready { panes } => panes,
        _ => panic!("the daemon should be ready"),
    };
    assert!(panes.is_empty(), "a new host has no pane to offer back");
    assert!(host.is_ready());
    assert_eq!(
        std::fs::read(installed_path(world.home.path())).expect("installed"),
        artifact
    );
    let commands = world.commands();
    assert_eq!(commands.len(), 3, "{commands:?}");
    assert!(commands[0].contains("SDSD-PROBE-"));
    assert!(commands[1].contains("SDSD-INSTALL-"));
    assert_eq!(
        commands[2].split(" proxy --hello ").next(),
        Some(format!("exec \"$HOME/.shelldeck/bin/shelldeck-sessiond-{VERSION}\"").as_str())
    );

    // A tab and a split: two panes, spawned and attached through the
    // tunnel, each echoing its own input.
    let supervisor = host.supervisor();
    let opened = Instant::now();
    let tab = supervisor.open(cat(), view("tab")).expect("open");
    let split = supervisor.open(cat(), view("split")).expect("open");
    assert!(
        opened.elapsed() < Duration::from_millis(500),
        "open never waits"
    );
    wait_until("both panes live", || {
        phase(&tab) == AttachedPhase::Live && phase(&split) == AttachedPhase::Live
    });
    tab.write_input(b"remote-echo\n");
    split.write_input(b"split-echo\n");
    wait_until("the tab echo", || {
        screen_text(&tab).matches("remote-echo").count() == 2
    });
    wait_until("the split echo", || {
        screen_text(&split).matches("split-echo").count() == 2
    });
    assert!(!screen_text(&tab).contains("split-echo"));
    let (tab_pane, split_pane) = (
        tab.daemon_pane().expect("pane"),
        split.daemon_pane().expect("pane"),
    );
    assert_ne!(tab_pane, split_pane);
    // One SSH connection and one exec channel carry both panes.
    assert_eq!(world.dials(), 1);
    assert!(world.commands().is_empty(), "no further exec request");
    assert_eq!(supervisor.session_count(), 2);

    // The SSH connection is lost: both sessions reattach over a new one,
    // the panes and their screens are intact.
    world.cut_connections();
    assert_eq!(next_event(), HostEvent::ConnectionLost);
    assert_eq!(next_event(), HostEvent::Reconnected);
    wait_until("both panes live again", || {
        phase(&tab) == AttachedPhase::Live && phase(&split) == AttachedPhase::Live
    });
    assert_eq!(world.dials(), 2);
    assert_eq!(tab.daemon_pane(), Some(tab_pane));
    tab.write_input(b"after-loss\n");
    wait_until("echo after the loss", || {
        screen_text(&tab).matches("after-loss").count() == 2
    });
    assert_eq!(screen_text(&tab).matches("remote-echo").count(), 2);
    let commands = world.commands();
    assert_eq!(commands.len(), 1, "one new proxy channel: {commands:?}");
    assert!(is_proxy(&commands[0]));

    // The app quits (sessions dropped, not terminated): the connection is
    // released and the panes keep running on the host.
    drop(tab);
    drop(split);
    assert_eq!(next_event(), HostEvent::Idle);
    assert!(!host.is_ready());
    assert_eq!(supervisor.session_count(), 0);

    // Connecting to the host again offers both panes back, without any
    // install or consent (the binary is there).
    let panes = match host.prepare(InstallConsent::Never) {
        Prepared::Ready { panes } => panes,
        _ => panic!("the daemon should be ready"),
    };
    assert_eq!(world.dials(), 3);
    let offered: Vec<Uuid> = panes
        .iter()
        .filter(|info| info.alive && info.label == PANE_LABEL)
        .map(|info| info.pane)
        .collect();
    assert_eq!(offered.len(), 2, "{panes:?}");
    assert!(offered.contains(&tab_pane) && offered.contains(&split_pane));
    let commands = world.commands();
    assert_eq!(
        commands.len(),
        2,
        "probe and proxy, no upload: {commands:?}"
    );

    let back = supervisor
        .open(PaneTarget::Existing(tab_pane), view("tab"))
        .expect("reattach");
    wait_until("the pane's screen is back", || {
        phase(&back) == AttachedPhase::Live && screen_text(&back).matches("after-loss").count() == 2
    });
    assert_eq!(screen_text(&back).matches("remote-echo").count(), 2);
    back.write_input(b"still-here\n");
    wait_until("echo after reattach", || {
        screen_text(&back).matches("still-here").count() == 2
    });

    // Closing the tab ends its pane, and with the last session over the
    // connection is released. The other pane stays on the host.
    back.terminate();
    assert_eq!(next_event(), HostEvent::Idle);
    // Ended by the hangup `terminate` sends (128 + SIGHUP).
    assert_eq!(
        phase(&back),
        AttachedPhase::Ended {
            exit_code: Some(129)
        }
    );
    drop(back);
    let panes = match host.prepare(InstallConsent::Never) {
        Prepared::Ready { panes } => panes,
        _ => panic!("the daemon should be ready"),
    };
    assert!(!panes.iter().any(|info| info.pane == tab_pane && info.alive));
    assert!(panes
        .iter()
        .any(|info| info.pane == split_pane && info.alive));
    // A `Ready` that no session follows is released by the caller.
    host.release_if_idle();
    assert!(!host.is_ready());
}

// SDTEST-3024 (SDUC-574): the same path with nothing simulated on the
// "remote" side: the real binary is uploaded by the real install script,
// started by the real proxy command under a private HOME, and serves real
// panes. Opt in with SHELLDECK_SESSIOND_BIN=<absolute path of a built
// shelldeck-sessiond for this machine, at most 64 MiB (strip a debug
// build)>.
#[test]
#[ignore = "needs a built shelldeck-sessiond of at most 64 MiB: set SHELLDECK_SESSIOND_BIN"]
fn sdtest_3024_real_binary_is_uploaded_started_and_serves_panes_over_ssh() {
    let binary = std::env::var("SHELLDECK_SESSIOND_BIN")
        .expect("SHELLDECK_SESSIOND_BIN names a built shelldeck-sessiond");
    let bytes = std::fs::read(&binary).expect("read the daemon binary");
    let home = temp_dir();
    // `~/.shelldeck/run/<machine>/` must hold the socket path, or the
    // daemon would fall back to the shared /tmp location of the real user.
    let socket_path_len = home.path().as_os_str().len()
        + "/.shelldeck/run/".len()
        + uname("-n").len().clamp(18, 64)
        + 1
        + paths::SOCKET_NAME.len();
    assert!(socket_path_len <= 100, "temporary home path is too long");
    let _cleanup = TerminateDaemons(home.path().to_path_buf());
    let artifacts = temp_dir();
    std::fs::write(
        artifacts.path().join(artifact_file_name(local_target())),
        &bytes,
    )
    .expect("write artifact");

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("runtime");
    let (server_events, _server_events_rx) = mpsc::unbounded_channel();
    let remote_home = home.path().to_path_buf();
    let dial: Dial = Arc::new(move || {
        let (events, home) = (server_events.clone(), remote_home.clone());
        Box::pin(async move {
            // Every command, the proxy included, runs in the real /bin/sh.
            let (session, _server) =
                open_session(ExecMode::Shell { home: Some(home) }, events).await;
            Ok(session)
        })
    });
    let host = RemoteHost::new(
        runtime.handle().clone(),
        dial,
        RemoteHostOptions {
            // The real binary carries the marker of its own release.
            version: env!("CARGO_PKG_VERSION").into(),
            artifact_dirs: vec![artifacts.path().to_path_buf()],
            ..RemoteHostOptions::default()
        },
    )
    .expect("remote host");
    let (event_tx, host_events) = std::sync::mpsc::channel();
    host.set_event_handler(move |event| {
        let _ = event_tx.send(event);
    });

    // Without the user's answer nothing is uploaded.
    let (reason, ask, _) = expect_fallback(host.prepare(InstallConsent::Ask));
    assert_eq!((reason, ask), (FallbackReason::ConsentPending, true));
    assert!(!home.path().join(".shelldeck").exists());

    // With it: upload, verify, start the daemon through the proxy.
    let panes = match host.prepare(InstallConsent::Allow) {
        Prepared::Ready { panes } => panes,
        _ => panic!("the daemon should be ready"),
    };
    assert!(panes.is_empty());
    let installed = home
        .path()
        .join(".shelldeck/bin")
        .join(concat!("shelldeck-sessiond-", env!("CARGO_PKG_VERSION")));
    assert_eq!(
        sha256_hex(&std::fs::read(&installed).expect("installed")),
        sha256_hex(&bytes)
    );
    assert_eq!(mode_of(&installed), 0o700);

    let session = host.supervisor().open(cat(), view("real")).expect("open");
    wait_until("the pane is live", || {
        phase(&session) == AttachedPhase::Live
    });
    session.write_input(b"real-remote\n");
    wait_until("the echo", || {
        screen_text(&session).matches("real-remote").count() == 2
    });
    let pane = session.daemon_pane().expect("pane");

    // The app quits: the daemon (detached by the proxy) keeps the pane.
    drop(session);
    assert_eq!(host_events.recv_timeout(LIMIT), Ok(HostEvent::Idle));
    let panes = match host.prepare(InstallConsent::Never) {
        Prepared::Ready { panes } => panes,
        _ => panic!("the daemon should be ready"),
    };
    assert!(panes.iter().any(|info| info.pane == pane && info.alive));
    let back = host
        .supervisor()
        .open(PaneTarget::Existing(pane), view("real"))
        .expect("reattach");
    wait_until("the screen is back", || {
        screen_text(&back).matches("real-remote").count() == 2
    });
    back.terminate();
    assert_eq!(host_events.recv_timeout(LIMIT), Ok(HostEvent::Idle));
}
