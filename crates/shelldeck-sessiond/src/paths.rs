//! Run directory, socket path and the ownership checks every party runs
//! (docs/design/session-daemon.md, section 3).
//!
//! The daemon, every client and the proxy apply the same checks: the run
//! directory must be a real directory owned by the caller with no group or
//! other permission bit, the socket must be a socket owned by the caller,
//! and the connected peer must have the caller's uid. A failure is never
//! repaired in place: the daemon path is simply unavailable.

use std::io;
use std::os::unix::fs::{DirBuilderExt, FileTypeExt, MetadataExt};
use std::os::unix::io::AsRawFd;
use std::path::{Path, PathBuf};

pub const SOCKET_NAME: &str = "sessiond-v1.sock";
pub const LOCK_NAME: &str = "sessiond-v1.lock";
pub const LOG_NAME: &str = "sessiond.log";

/// Longest socket path that fits `sockaddr_un.sun_path` with its NUL.
#[cfg(target_os = "linux")]
const SUN_PATH_MAX: usize = 107;
#[cfg(not(target_os = "linux"))]
const SUN_PATH_MAX: usize = 103;

#[derive(Debug, thiserror::Error)]
pub enum PathError {
    #[error("run directory {0} is not a private directory owned by this user")]
    RunDir(PathBuf),
    #[error("socket {0} is not a socket owned by this user")]
    Socket(PathBuf),
    #[error("socket path {0} is too long")]
    TooLong(PathBuf),
    #[error("peer of the session socket is another user")]
    Peer,
    #[error("no home directory")]
    NoHome,
    #[error(transparent)]
    Io(#[from] io::Error),
}

pub fn effective_uid() -> u32 {
    // SAFETY: geteuid has no preconditions and cannot fail.
    unsafe { libc::geteuid() }
}

pub fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .filter(|home| !home.is_empty())
        .map(PathBuf::from)
        .filter(|home| home.is_absolute())
}

fn host_component() -> String {
    let mut buf = [0u8; 256];
    // SAFETY: the buffer is valid for its length; gethostname writes at most
    // that many bytes.
    let ok = unsafe { libc::gethostname(buf.as_mut_ptr().cast(), buf.len()) } == 0;
    let raw = if ok {
        let end = buf.iter().position(|b| *b == 0).unwrap_or(buf.len());
        &buf[..end]
    } else {
        &b""[..]
    };
    let host: String = raw
        .iter()
        .map(|b| *b as char)
        .filter(|c| c.is_ascii_alphanumeric() || *c == '.' || *c == '-')
        .take(64)
        .collect();
    if host.is_empty() || host.chars().all(|c| c == '.') {
        "localhost".to_string()
    } else {
        host
    }
}

pub fn socket_path(dir: &Path) -> PathBuf {
    dir.join(SOCKET_NAME)
}

fn fits_sun_path(dir: &Path) -> bool {
    socket_path(dir).as_os_str().len() <= SUN_PATH_MAX
}

/// The per-user run directory: `~/.shelldeck/run/<host>/`, or
/// `/tmp/shelldeck-<uid>/` when the socket path would not fit `sun_path`.
/// `XDG_RUNTIME_DIR` is deliberately not used: logind removes it when the
/// last login session ends, which is exactly when the daemon must survive.
///
/// The host name can change while a daemon runs (a laptop renamed per
/// network, `hostnamectl`). A daemon that is already serving a sibling
/// directory is therefore preferred over an empty directory for the
/// current name: otherwise its panes would keep running where no client
/// looks. See [`live_run_dir`].
pub fn default_run_dir() -> Result<PathBuf, PathError> {
    let home = home_dir().ok_or(PathError::NoHome)?;
    let root = home.join(".shelldeck").join("run");
    let dir = root.join(host_component());
    if fits_sun_path(&dir) {
        return Ok(live_run_dir(&root, dir));
    }
    Ok(PathBuf::from(format!("/tmp/shelldeck-{}", effective_uid())))
}

/// Most sibling directories looked at by [`live_run_dir`].
const MAX_RUN_DIR_SCAN: usize = 32;

/// `preferred`, unless no daemon answers there and one does in another
/// directory under `root`: then that directory. A directory counts only
/// when it passes the same checks as any connection (private directory,
/// owned socket, peer uid), and a socket left by another machine on a
/// shared home refuses the connection, so it is never picked.
pub fn live_run_dir(root: &Path, preferred: PathBuf) -> PathBuf {
    if std::fs::symlink_metadata(socket_path(&preferred)).is_ok() {
        return preferred;
    }
    let Ok(entries) = std::fs::read_dir(root) else {
        return preferred;
    };
    let mut candidates: Vec<PathBuf> = entries
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| *path != preferred && fits_sun_path(path))
        .take(MAX_RUN_DIR_SCAN)
        .collect();
    candidates.sort();
    candidates
        .into_iter()
        .find(|dir| connect_checked(dir).is_ok())
        .unwrap_or(preferred)
}

/// Resolve an explicit `--dir` or the default, and check the path length.
pub fn resolve_run_dir(explicit: Option<&Path>) -> Result<PathBuf, PathError> {
    let dir = match explicit {
        Some(dir) => dir.to_path_buf(),
        None => default_run_dir()?,
    };
    if !dir.is_absolute() || !fits_sun_path(&dir) {
        return Err(PathError::TooLong(socket_path(&dir)));
    }
    Ok(dir)
}

/// Create the run directory with mode 0700 if it is missing (the daemon
/// only), then verify it. The process umask is not touched.
pub fn ensure_run_dir(dir: &Path) -> Result<(), PathError> {
    if std::fs::symlink_metadata(dir).is_err() {
        if let Some(parent) = dir.parent() {
            std::fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(parent)?;
        }
        match std::fs::DirBuilder::new().mode(0o700).create(dir) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e.into()),
        }
    }
    check_run_dir(dir)
}

/// `lstat` the run directory: a real directory (no symlink), owned by the
/// caller, with no group or other permission bit.
pub fn check_run_dir(dir: &Path) -> Result<(), PathError> {
    let meta = std::fs::symlink_metadata(dir).map_err(|_| PathError::RunDir(dir.into()))?;
    if !meta.file_type().is_dir() || meta.uid() != effective_uid() || meta.mode() & 0o077 != 0 {
        return Err(PathError::RunDir(dir.into()));
    }
    Ok(())
}

/// The socket must be a socket owned by the caller.
pub fn check_socket(path: &Path) -> Result<(), PathError> {
    let meta = std::fs::symlink_metadata(path)?;
    if !meta.file_type().is_socket() || meta.uid() != effective_uid() {
        return Err(PathError::Socket(path.into()));
    }
    Ok(())
}

/// Uid of the process at the other end of a connected Unix socket.
pub fn peer_uid(socket: &impl AsRawFd) -> io::Result<u32> {
    #[cfg(any(target_os = "linux", target_os = "android"))]
    {
        let mut cred = libc::ucred {
            pid: 0,
            uid: u32::MAX,
            gid: u32::MAX,
        };
        let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
        // SAFETY: `cred` and `len` are valid for writes of the sizes passed,
        // and the fd is open for the duration of the call.
        let rc = unsafe {
            libc::getsockopt(
                socket.as_raw_fd(),
                libc::SOL_SOCKET,
                libc::SO_PEERCRED,
                (&mut cred as *mut libc::ucred).cast(),
                &mut len,
            )
        };
        if rc != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(cred.uid)
    }
    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    {
        let mut uid: libc::uid_t = libc::uid_t::MAX;
        let mut gid: libc::gid_t = libc::gid_t::MAX;
        // SAFETY: both pointers are valid for writes and the fd is open.
        let rc = unsafe { libc::getpeereid(socket.as_raw_fd(), &mut uid, &mut gid) };
        if rc != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(uid)
    }
}

/// The peer must run as the caller's uid. Checked by the daemon on every
/// accepted connection and by clients and the proxy before `Hello`.
pub fn check_peer(socket: &impl AsRawFd) -> Result<(), PathError> {
    match peer_uid(socket) {
        Ok(uid) if uid == effective_uid() => Ok(()),
        _ => Err(PathError::Peer),
    }
}

/// Full client-side check, then connect: run directory, socket, peer.
pub fn connect_checked(dir: &Path) -> Result<std::os::unix::net::UnixStream, PathError> {
    check_run_dir(dir)?;
    let path = socket_path(dir);
    check_socket(&path)?;
    let stream = std::os::unix::net::UnixStream::connect(&path)?;
    check_peer(&stream)?;
    Ok(stream)
}
