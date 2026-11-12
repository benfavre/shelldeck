//! Run directory, socket path and the ownership checks every party runs
//! (docs/design/session-daemon.md, section 3).
//!
//! The daemon, every client and the proxy apply the same checks: the run
//! directory must be a real directory owned by the caller with no group or
//! other permission bit, the socket must be a socket owned by the caller,
//! and the connected peer must have the caller's uid. A failure is never
//! repaired in place: the daemon path is simply unavailable.

use std::ffi::{OsStr, OsString};
use std::io;
use std::os::unix::fs::{DirBuilderExt, FileTypeExt, MetadataExt};
use std::os::unix::io::AsRawFd;
use std::path::{Path, PathBuf};

pub const SOCKET_NAME: &str = "sessiond-v1.sock";
pub const LOCK_NAME: &str = "sessiond-v1.lock";
pub const LOG_NAME: &str = "sessiond.log";
/// The one rotated generation of the log.
pub const LOG_OLD_NAME: &str = "sessiond.log.1";
/// Name the socket has for an instant while an idle daemon checks that no
/// connection is still on its way (see `Server::run`).
pub const SOCKET_LEAVING_NAME: &str = "sessiond-v1.sock.leaving";

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

/// The machine identity the operating system keeps across host name
/// changes, if it has one: `/etc/machine-id` (systemd), the D-Bus copy of
/// it, or the hardware UUID on macOS.
fn machine_id() -> Option<String> {
    for path in ["/etc/machine-id", "/var/lib/dbus/machine-id"] {
        if let Ok(text) = std::fs::read_to_string(path) {
            let id = text.trim();
            if !id.is_empty() {
                return Some(id.to_string());
            }
        }
    }
    #[cfg(target_os = "macos")]
    {
        let mut buf = [0u8; 64];
        let mut len: libc::size_t = buf.len();
        // SAFETY: the name is NUL terminated, the buffer is valid for
        // `len` bytes and `len` is updated to the bytes written.
        let rc = unsafe {
            libc::sysctlbyname(
                c"kern.uuid".as_ptr(),
                buf.as_mut_ptr().cast(),
                &mut len,
                std::ptr::null_mut(),
                0,
            )
        };
        if rc == 0 {
            let end = buf[..len.min(buf.len())]
                .iter()
                .position(|b| *b == 0)
                .unwrap_or(len.min(buf.len()));
            let id = String::from_utf8_lossy(&buf[..end]).trim().to_string();
            if !id.is_empty() {
                return Some(id);
            }
        }
    }
    None
}

/// Name of this machine's run directory under `~/.shelldeck/run/`.
///
/// `m-` and 16 hex digits derived from the machine identity and the host
/// name when the system has an identity. The identity itself is not
/// stored (systemd asks applications not to expose `/etc/machine-id`),
/// only a keyed SHA-256. Without a usable identity (fewer than 8
/// alphanumeric characters, or the `uninitialized` placeholder of an early
/// boot) the name is the host name, as in the first release.
///
/// The host name is part of the hash because the identity alone does not
/// tell machines apart everywhere: virtual machines cloned from one image
/// have the same `/etc/machine-id`, and when they also share a home
/// directory (NFS) one name for both would make each take the other's
/// socket for a stale one of its own. The price is that the name changes
/// with the host name; [`live_run_dir`] covers that, a daemon that still
/// runs in the directory of the earlier name is found and used.
pub fn run_dir_name(machine_id: Option<&str>, host: &str) -> String {
    run_dir_names(machine_id, host).swap_remove(0)
}

/// The names this machine's run directory may have, most preferred first:
/// the hashed name, the same hash cut to 8 hex digits, and the host name.
/// The later ones are for a home directory whose path leaves no room for
/// the full name in a socket address (see [`default_run_dir`]); the host
/// name is also where the first release put the directory. Without a
/// usable machine identity: the host name only.
pub fn run_dir_names(machine_id: Option<&str>, host: &str) -> Vec<String> {
    use sha2::{Digest, Sha256};
    let id = machine_id.map(str::trim).filter(|id| {
        *id != "uninitialized"
            && id.len() <= 128
            && id.chars().filter(char::is_ascii_alphanumeric).count() >= 8
            && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
    });
    match id {
        Some(id) => {
            let mut hasher = Sha256::new();
            hasher.update(b"shelldeck-sessiond run directory v2\0");
            hasher.update(id.to_ascii_lowercase().as_bytes());
            hasher.update(b"\0");
            hasher.update(host.to_ascii_lowercase().as_bytes());
            let digest = hasher.finalize();
            let hex: String = digest[..8].iter().map(|b| format!("{b:02x}")).collect();
            vec![
                format!("m-{hex}"),
                format!("m-{}", &hex[..8]),
                host.to_string(),
            ]
        }
        None => vec![host.to_string()],
    }
}

/// The per-user run directory: `~/.shelldeck/run/<machine>/` (see
/// [`run_dir_name`]), or `/tmp/shelldeck-<uid>/` when no name fits.
/// `XDG_RUNTIME_DIR` is deliberately not used: logind removes it when the
/// last login session ends, which is exactly when the daemon must survive.
///
/// A daemon that is already serving a sibling directory is preferred over
/// an empty directory for this machine: a daemon started under an earlier
/// host name, or by an earlier release, lives in a directory with another
/// name, and its panes would otherwise keep running where no client
/// looks. See [`live_run_dir`].
pub fn default_run_dir() -> Result<PathBuf, PathError> {
    let home = home_dir().ok_or(PathError::NoHome)?;
    Ok(run_dir_under(
        &home,
        machine_id().as_deref(),
        &host_component(),
        effective_uid(),
    ))
}

/// [`default_run_dir`] for a given home directory, machine and user.
///
/// The socket path must fit `sun_path` (107 bytes on Linux). The first
/// name of [`run_dir_names`] that fits under `home` is used, so a long
/// home path gets a shorter directory name before the daemon leaves the
/// home directory at all. Only when none fits is it `/tmp/shelldeck-<uid>`,
/// which another local user could create first (the daemon then refuses
/// it and is unavailable, it is never taken over).
///
/// Whatever is chosen, a daemon that answers in a sibling directory under
/// `home` wins: also over the `/tmp` location, so an update that changes
/// the preferred name never abandons a running daemon.
pub fn run_dir_under(home: &Path, machine_id: Option<&str>, host: &str, uid: u32) -> PathBuf {
    let root = home.join(".shelldeck").join("run");
    let preferred = run_dir_names(machine_id, host)
        .into_iter()
        .map(|name| root.join(name))
        .find(|dir| fits_sun_path(dir))
        .unwrap_or_else(|| PathBuf::from(format!("/tmp/shelldeck-{uid}")));
    live_run_dir(&root, preferred)
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

/// What a daemon started by an application running from an AppImage must
/// not keep of its environment, as `(name, new value)` pairs (`None`: the
/// variable is removed).
///
/// The image is a mount that exists while the application runs. Its
/// runtime and launcher point `PATH`, `LD_LIBRARY_PATH` and the like into
/// it, and the daemon passes its environment on to every terminal: without
/// this, a shell opened later would look for programs and libraries in a
/// mount that is gone, or load the libraries bundled for the application.
/// Every entry of a `:` separated value that lies inside `appdir` is
/// dropped, and so are the variables of the AppImage runtime itself.
///
/// `appdir` is `$APPDIR`. Nothing changes when it is absent or relative.
pub fn appimage_env_changes(
    vars: impl IntoIterator<Item = (OsString, OsString)>,
    appdir: Option<&OsStr>,
) -> Vec<(OsString, Option<OsString>)> {
    use std::os::unix::ffi::{OsStrExt, OsStringExt};

    let Some(appdir) = appdir.map(Path::new).filter(|dir| dir.is_absolute()) else {
        return Vec::new();
    };
    let mut changes = Vec::new();
    for (name, value) in vars {
        if ["APPDIR", "APPIMAGE", "OWD", "ARGV0"]
            .iter()
            .any(|runtime| name == *runtime)
        {
            changes.push((name, None));
            continue;
        }
        let entries: Vec<&[u8]> = value.as_bytes().split(|byte| *byte == b':').collect();
        let kept: Vec<&[u8]> = entries
            .iter()
            .copied()
            .filter(|entry| !Path::new(OsStr::from_bytes(entry)).starts_with(appdir))
            .collect();
        if kept.len() == entries.len() {
            continue;
        }
        // An empty entry means "the current directory" in a search path:
        // what is left of a value made only of image entries is removed,
        // not kept as such an entry.
        let kept: Vec<&[u8]> = kept.into_iter().filter(|entry| !entry.is_empty()).collect();
        let new = (!kept.is_empty()).then(|| OsString::from_vec(kept.join(&b':')));
        changes.push((name, new));
    }
    changes
}

#[cfg(test)]
mod tests {
    use super::*;

    // SDTEST-3066 (SDUC-574): a daemon started from an AppImage keeps
    // nothing of the image in its environment, which every terminal
    // inherits: no search path entry inside the mount, none of the
    // variables of the AppImage runtime. Elsewhere nothing is touched.
    #[test]
    fn sdtest_3066_a_daemon_keeps_nothing_of_the_appimage_mount() {
        let vars = |pairs: &[(&str, &str)]| -> Vec<(OsString, OsString)> {
            pairs
                .iter()
                .map(|(name, value)| (OsString::from(name), OsString::from(value)))
                .collect()
        };
        let environment = vars(&[
            ("APPDIR", "/tmp/.mount_abc"),
            ("APPIMAGE", "/home/u/Inklura.AppImage"),
            ("OWD", "/home/u"),
            ("ARGV0", "./Inklura.AppImage"),
            ("PATH", "/tmp/.mount_abc/usr/bin:/usr/local/bin:/usr/bin"),
            ("LD_LIBRARY_PATH", "/tmp/.mount_abc/usr/lib:"),
            ("XDG_DATA_DIRS", "/usr/share:/tmp/.mount_abc/usr/share"),
            (
                "GSETTINGS_SCHEMA_DIR",
                "/tmp/.mount_abc/usr/share/glib-2.0/schemas",
            ),
            // A directory whose name only starts the same way is not
            // inside the image.
            ("OTHER", "/tmp/.mount_abcdef/bin:/tmp/.mount_abc"),
            ("HOME", "/home/u"),
            ("LANG", "fr_FR.UTF-8"),
        ]);
        let mut changes =
            appimage_env_changes(environment.clone(), Some(OsStr::new("/tmp/.mount_abc")));
        changes.sort();
        let expect =
            |name: &str, value: Option<&str>| (OsString::from(name), value.map(OsString::from));
        assert_eq!(
            changes,
            [
                expect("APPDIR", None),
                expect("APPIMAGE", None),
                expect("ARGV0", None),
                expect("GSETTINGS_SCHEMA_DIR", None),
                expect("LD_LIBRARY_PATH", None),
                expect("OTHER", Some("/tmp/.mount_abcdef/bin")),
                expect("OWD", None),
                expect("PATH", Some("/usr/local/bin:/usr/bin")),
                expect("XDG_DATA_DIRS", Some("/usr/share")),
            ]
        );
        // Not started from an image: the environment is left alone, also
        // the names an image would have set.
        assert!(appimage_env_changes(environment.clone(), None).is_empty());
        assert!(appimage_env_changes(environment, Some(OsStr::new("relative"))).is_empty());
    }

    // SDTEST-3025 (SDUC-574): the run directory name is derived from the
    // machine identity and the host name, shows neither, and falls back
    // to the host name only when the system has no usable identity.
    #[test]
    fn sdtest_3025_run_directory_name_tells_machines_apart() {
        let id = "0123456789abcdef0123456789abcdef";
        let name = run_dir_name(Some(id), "laptop");
        assert_eq!(name, run_dir_name(Some(&format!("{id}\n")), "laptop"));
        assert_eq!(name, run_dir_name(Some(&id.to_uppercase()), "LAPTOP"));
        assert_eq!(name.len(), 18);
        assert!(name.starts_with("m-"));
        assert!(name[2..].chars().all(|c| c.is_ascii_hexdigit()));
        // Neither the identity nor the host name is readable from it.
        assert!(!name.contains(&id[..8]));
        assert!(!name.contains("laptop"));
        // Another machine on the same home directory: another directory.
        assert_ne!(
            name,
            run_dir_name(Some("fedcba9876543210fedcba9876543210"), "laptop")
        );
        // Two clones of one image (same identity) that share a home
        // directory: told apart by their host names.
        assert_ne!(name, run_dir_name(Some(id), "node2"));
        assert_ne!(
            run_dir_name(Some(id), "ab"),
            run_dir_name(Some(&format!("{id}a")), "b"),
            "the two inputs are kept apart"
        );
        // A macOS style hardware UUID is accepted too.
        assert!(
            run_dir_name(Some("8D8AC610-566D-4EF0-9C22-186B2A5ED793"), "mac").starts_with("m-")
        );
        // The shorter names, for long home paths.
        let names = run_dir_names(Some(id), "laptop");
        assert_eq!(names.len(), 3);
        assert_eq!(names[0], name);
        assert_eq!(names[1], name[..10]);
        assert_eq!(names[2], "laptop");

        // No usable identity: the host name, as in the first release.
        for unusable in [
            None,
            Some(""),
            Some("   "),
            Some("uninitialized"),
            Some("abc"),
            Some("../../etc/passwd"),
        ] {
            assert_eq!(run_dir_name(unusable, "laptop"), "laptop", "{unusable:?}");
            assert_eq!(run_dir_names(unusable, "laptop"), vec!["laptop"]);
        }
        let long = "a".repeat(129);
        assert_eq!(run_dir_name(Some(&long), "laptop"), "laptop");

        // What this machine resolves to is a private path under the home
        // directory (or the /tmp fallback), whatever the identity source.
        let component = run_dir_name(machine_id().as_deref(), &host_component());
        assert!(!component.is_empty() && !component.contains('/'));
    }

    // SDTEST-3045 (SDUC-574): a home directory whose path leaves no room
    // for the full name gets a shorter one before the daemon leaves the
    // home directory, and `/tmp` is the last resort only.
    #[test]
    fn sdtest_3045_a_long_home_path_gets_a_shorter_run_directory_name() {
        let id = Some("0123456789abcdef0123456789abcdef");
        let names = run_dir_names(id, "pc1");
        // Budget: home + "/.shelldeck/run/" + name + "/" + socket name.
        let fixed = "/.shelldeck/run/".len() + 1 + SOCKET_NAME.len();
        let home_of = |len: usize| PathBuf::from(format!("/{}", "h".repeat(len - 1)));
        let longest_home = |name: &str| SUN_PATH_MAX - fixed - name.len();

        // A short home: the full name.
        let home = home_of(20);
        assert_eq!(
            run_dir_under(&home, id, "pc1", 1000),
            home.join(".shelldeck/run").join(&names[0])
        );
        // The longest home the full name fits, and one byte more.
        let home = home_of(longest_home(&names[0]));
        let dir = run_dir_under(&home, id, "pc1", 1000);
        assert_eq!(dir, home.join(".shelldeck/run").join(&names[0]));
        assert_eq!(socket_path(&dir).as_os_str().len(), SUN_PATH_MAX);
        let home = home_of(longest_home(&names[0]) + 1);
        assert_eq!(
            run_dir_under(&home, id, "pc1", 1000),
            home.join(".shelldeck/run").join(&names[1])
        );
        // Too long for the short hash, not for a short host name (where
        // the first release put the directory).
        let home = home_of(longest_home(&names[1]) + 1);
        assert_eq!(
            run_dir_under(&home, id, "pc1", 1000),
            home.join(".shelldeck/run/pc1")
        );
        // Nothing fits: the last resort.
        let home = home_of(longest_home("pc1") + 1);
        assert_eq!(
            run_dir_under(&home, id, "pc1", 1000),
            PathBuf::from("/tmp/shelldeck-1000")
        );
        // Every result holds a socket path that fits.
        for len in 2..120 {
            let dir = run_dir_under(&home_of(len), id, "a-rather-long-host-name.example", 7);
            assert!(fits_sun_path(&dir), "{}", dir.display());
        }
    }
}
