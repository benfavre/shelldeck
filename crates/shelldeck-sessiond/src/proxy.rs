//! `shelldeck-sessiond proxy`: a stdio to Unix socket byte pipe, run over an
//! SSH exec channel (docs/design/session-daemon.md, section 8).
//!
//! The proxy is not a command runner or a forwarder: it takes no command,
//! host, port or socket path from the remote command line, connects only to
//! the per-user socket after the path and peer checks, starts only its own
//! executable with `serve`, and never looks at frame boundaries.

use crate::paths::{self, PathError};
use crate::protocol;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// How long a freshly started daemon gets to accept connections.
const START_TIMEOUT: Duration = Duration::from_secs(3);
/// While the socket stays missing, start the daemon again this often: a
/// daemon that is exiting has already unlinked its socket but still holds
/// the instance lock, so a start in that window does nothing.
const START_RETRY: Duration = Duration::from_millis(250);

/// Fixed failure codes, printed as `SDSD-ERR <code>` on stderr.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProxyFailure {
    /// Run directory or socket has the wrong type, owner or mode.
    PathCheck = 1,
    /// The peer of the socket is another user.
    PeerCheck = 2,
    /// The daemon is not running and could not be started.
    StartFailed = 3,
}

enum Attempt {
    Connected(UnixStream),
    NotRunning,
    Failed(ProxyFailure),
}

fn attempt(dir: &Path) -> Attempt {
    match paths::connect_checked(dir) {
        Ok(stream) => Attempt::Connected(stream),
        Err(PathError::Peer) => Attempt::Failed(ProxyFailure::PeerCheck),
        Err(PathError::Io(e))
            if matches!(
                e.kind(),
                std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused
            ) =>
        {
            Attempt::NotRunning
        }
        // A missing run directory only means no daemon ever ran here; one
        // that exists with the wrong owner or mode is never repaired.
        Err(PathError::RunDir(_)) if std::fs::symlink_metadata(dir).is_err() => Attempt::NotRunning,
        Err(_) => Attempt::Failed(ProxyFailure::PathCheck),
    }
}

/// Connect to the daemon serving `dir` only if one is already running.
/// `Ok(None)`: no daemon (nothing is started).
pub fn connect_existing(dir: &Path) -> Result<Option<UnixStream>, ProxyFailure> {
    match attempt(dir) {
        Attempt::Connected(stream) => Ok(Some(stream)),
        Attempt::NotRunning => Ok(None),
        Attempt::Failed(failure) => Err(failure),
    }
}

/// Start a detached daemon: `<exe> serve --detach [--dir <dir>]`. The
/// started process forks and returns at once, so this never waits on the
/// daemon itself. `exe` must be an absolute path, never a `PATH` lookup.
pub fn start_daemon(exe: &Path, dir: Option<&Path>) -> std::io::Result<()> {
    if !exe.is_absolute() {
        return Err(std::io::Error::other("daemon path must be absolute"));
    }
    let mut command = Command::new(exe);
    command.arg("serve").arg("--detach");
    if let Some(dir) = dir {
        command.arg("--dir").arg(dir);
    }
    let status = command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(std::io::Error::other("daemon failed to start"))
    }
}

/// Connect to the daemon serving `dir`, starting it with `exe` when it is
/// not running. `explicit_dir` is passed on to the daemon (`--dir`, tests
/// only: the app never puts it in a remote command).
pub fn connect_or_start(
    dir: &Path,
    explicit_dir: bool,
    exe: &Path,
) -> Result<UnixStream, ProxyFailure> {
    match attempt(dir) {
        Attempt::Connected(stream) => return Ok(stream),
        Attempt::Failed(failure) => return Err(failure),
        Attempt::NotRunning => {}
    }
    start_daemon(exe, explicit_dir.then_some(dir)).map_err(|_| ProxyFailure::StartFailed)?;
    let deadline = Instant::now() + START_TIMEOUT;
    let mut started = Instant::now();
    loop {
        match attempt(dir) {
            Attempt::Connected(stream) => return Ok(stream),
            Attempt::Failed(failure) => return Err(failure),
            Attempt::NotRunning if Instant::now() >= deadline => {
                return Err(ProxyFailure::StartFailed)
            }
            Attempt::NotRunning => {
                if started.elapsed() >= START_RETRY {
                    // A second `serve` next to a live daemon exits 0 and
                    // does nothing, so repeating the start is harmless.
                    let _ = start_daemon(exe, explicit_dir.then_some(dir));
                    started = Instant::now();
                }
                std::thread::sleep(Duration::from_millis(25));
            }
        }
    }
}

/// Copy `input` to the socket and the socket to `output` until either side
/// closes. EOF on `input` closes the socket, so nothing lingers after the
/// SSH channel is gone.
pub fn bridge(
    stream: UnixStream,
    mut input: impl Read + Send + 'static,
    mut output: impl Write,
) -> std::io::Result<()> {
    let mut to_socket = stream.try_clone()?;
    std::thread::Builder::new()
        .name("sessiond-proxy-stdin".into())
        .spawn(move || {
            let mut buf = vec![0u8; 64 * 1024];
            loop {
                match input.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        if to_socket.write_all(&buf[..n]).is_err() {
                            break;
                        }
                    }
                }
            }
            let _ = to_socket.shutdown(std::net::Shutdown::Both);
        })?;

    let mut from_socket = stream;
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        match from_socket.read(&mut buf) {
            Ok(0) => return Ok(()),
            Ok(n) => {
                output.write_all(&buf[..n])?;
                output.flush()?;
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) => return Ok(()),
        }
    }
}

/// Entry point of the `proxy` subcommand. Returns the process exit code.
pub fn run(dir: Option<&Path>, nonce: &str) -> u8 {
    let fail = |failure: ProxyFailure| {
        eprintln!("SDSD-ERR {}", failure as u8);
        failure as u8
    };
    let Some(marker) = protocol::proxy_marker(nonce) else {
        return fail(ProxyFailure::StartFailed);
    };
    let Ok(run_dir) = paths::resolve_run_dir(dir) else {
        return fail(ProxyFailure::PathCheck);
    };
    let Ok(exe) = std::env::current_exe() else {
        return fail(ProxyFailure::StartFailed);
    };
    let stream = match connect_or_start(&run_dir, dir.is_some(), &exe) {
        Ok(stream) => stream,
        Err(failure) => return fail(failure),
    };

    let mut stdout = std::io::stdout().lock();
    if stdout
        .write_all(&marker)
        .and_then(|_| stdout.flush())
        .is_err()
    {
        return fail(ProxyFailure::StartFailed);
    }
    match bridge(stream, std::io::stdin(), stdout) {
        Ok(()) => 0,
        Err(_) => 1,
    }
}
