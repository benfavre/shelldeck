//! Where the session daemon files are, and what happens when they are not
//! there (docs/design/session-daemon.md, section 15, "Daemon files that
//! are missing").
//!
//! Three places, in this order:
//!
//! 1. Beside the application binary. This is where the tar.gz install and
//!    its updater put them, and where the AppImage and the macOS bundle
//!    carry them.
//! 2. From an AppImage or a macOS bundle the daemon is not run in place.
//!    It outlives the application, and an AppImage is a mount that goes
//!    away when the application exits (a bundle may be a read-only
//!    translocated copy, or be replaced): the daemon is copied to the
//!    per-user directory once and started from there.
//! 3. The per-user directory, `sessiond/` in the configuration directory.
//!    An install whose updater predates the daemon has nothing beside the
//!    binary; the files are fetched there (`shelldeck_update::sessiond`).
//!
//! The builds for SSH servers are only ever read (they are uploaded), so
//! they are used wherever they sit.

use crate::t;
use parking_lot::Mutex;
use shelldeck_core::config::app_config::AppConfig;
use shelldeck_update::sessiond::{self as download, FetchErrorKind, LOCAL_DAEMON};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// Written beside fetched files: only what was fetched is ever cleaned up.
const FETCHED_MARKER: &str = ".fetched";
/// Everything a release may put in the per-user directory.
const FETCHED_FILES: [&str; 3] = [
    LOCAL_DAEMON,
    "shelldeck-sessiond-x86_64-unknown-linux-musl",
    "shelldeck-sessiond-aarch64-unknown-linux-musl",
];

/// The per-user directory of fetched or copied daemon files.
pub(crate) fn user_dir() -> PathBuf {
    AppConfig::config_dir().join("sessiond")
}

fn exe_dir() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(PathBuf::from))
}

/// Whether `exe_dir` is a place that does not outlive the application or
/// may be replaced under a running daemon: inside the AppImage mount
/// (`appdir` is `$APPDIR`, set by the AppImage runtime) or inside a macOS
/// application bundle.
fn is_transient(exe_dir: &Path, appdir: Option<&Path>, macos: bool) -> bool {
    if appdir.is_some_and(|appdir| appdir.is_absolute() && exe_dir.starts_with(appdir)) {
        return true;
    }
    macos
        && exe_dir
            .ancestors()
            .any(|dir| dir.extension().is_some_and(|extension| extension == "app"))
}

fn exe_dir_is_transient(exe_dir: &Path) -> bool {
    let appdir = std::env::var_os("APPDIR").map(PathBuf::from);
    is_transient(exe_dir, appdir.as_deref(), cfg!(target_os = "macos"))
}

/// Where the builds for SSH servers are looked up, in order:
/// `SHELLDECK_SESSIOND_ARTIFACT_DIR` (development), the directory of the
/// app binary, then the per-user directory.
pub(crate) fn artifact_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(dir) = std::env::var_os("SHELLDECK_SESSIOND_ARTIFACT_DIR").map(PathBuf::from) {
        if dir.is_absolute() {
            dirs.push(dir);
        }
    }
    dirs.extend(exe_dir());
    dirs.push(user_dir());
    dirs
}

/// The daemon for this machine: `SHELLDECK_SESSIOND_BIN` (development),
/// else as described at the top of this file. Always an absolute path to
/// an existing file, never a `PATH` lookup.
///
/// Cheap (one or two `stat`) except for the first call from an AppImage
/// or a bundle, which may copy the daemon out.
pub(crate) fn local_daemon() -> Result<PathBuf, String> {
    if let Some(path) = std::env::var_os("SHELLDECK_SESSIOND_BIN").map(PathBuf::from) {
        if path.is_absolute() && path.is_file() {
            return Ok(path);
        }
        return Err("SHELLDECK_SESSIOND_BIN is not an absolute path to a file".into());
    }
    let exe_dir = exe_dir().ok_or("the app executable has no directory")?;
    let bundled = exe_dir.join(LOCAL_DAEMON);
    if bundled.is_file() {
        if !exe_dir_is_transient(&exe_dir) {
            return Ok(bundled);
        }
        // The bundled file cannot change while this process runs: one
        // copy (or one comparison) per run.
        static COPIED: OnceLock<Result<PathBuf, String>> = OnceLock::new();
        return COPIED
            .get_or_init(|| copy_out(&bundled, &user_dir()))
            .clone();
    }
    let fetched = user_dir().join(LOCAL_DAEMON);
    if fetched.is_file() {
        Ok(fetched)
    } else {
        Err(format!("{} is not installed", bundled.display()))
    }
}

/// Put a copy of `bundled` in `dir` under the daemon's name, unless the
/// file there is already the same. The copy is written beside its final
/// name and renamed, so a daemon running from the previous file keeps it.
fn copy_out(bundled: &Path, dir: &Path) -> Result<PathBuf, String> {
    let destination = dir.join(LOCAL_DAEMON);
    let describe = |error: std::io::Error| format!("{}: {error}", destination.display());
    // Read and written rather than copied: nothing but the bytes is
    // wanted (no extended attribute of a downloaded bundle).
    let bytes =
        std::fs::read(bundled).map_err(|error| format!("{}: {error}", bundled.display()))?;
    let same = std::fs::metadata(&destination)
        .is_ok_and(|metadata| metadata.is_file() && metadata.len() == bytes.len() as u64)
        && std::fs::read(&destination).is_ok_and(|installed| installed == bytes);
    if same {
        return Ok(destination);
    }
    std::fs::create_dir_all(dir).map_err(describe)?;
    let staged = dir.join(format!(".{LOCAL_DAEMON}.copy"));
    let _ = std::fs::remove_file(&staged);
    let written = std::fs::write(&staged, &bytes)
        .and_then(|()| make_executable(&staged))
        .and_then(|()| std::fs::rename(&staged, &destination));
    if let Err(error) = written {
        let _ = std::fs::remove_file(&staged);
        return Err(describe(error));
    }
    Ok(destination)
}

#[cfg(unix)]
fn make_executable(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
}

#[cfg(not(unix))]
fn make_executable(_path: &Path) -> std::io::Result<()> {
    Ok(())
}

/// What Settings and the workspace show about the daemon for this machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Status {
    /// The daemon is there.
    Ready,
    /// Not there. It can be fetched, and is when the setting is on.
    Missing,
    /// Being fetched.
    Fetching,
    /// The last fetch of this run failed.
    FetchFailed(FetchErrorKind),
    /// Not there, and this copy of the app has no way to get it.
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Fetch {
    Idle,
    Running,
    Failed(FetchErrorKind),
}

fn fetch_state() -> &'static Mutex<Fetch> {
    static STATE: OnceLock<Mutex<Fetch>> = OnceLock::new();
    STATE.get_or_init(|| Mutex::new(Fetch::Idle))
}

/// Whether the daemon is there, without copying or fetching anything.
fn present() -> bool {
    if let Some(path) = std::env::var_os("SHELLDECK_SESSIOND_BIN").map(PathBuf::from) {
        return path.is_absolute() && path.is_file();
    }
    exe_dir().is_some_and(|dir| dir.join(LOCAL_DAEMON).is_file())
        || user_dir().join(LOCAL_DAEMON).is_file()
}

fn status_of(present: bool, fetch: &Fetch, offered: bool) -> Status {
    match (present, fetch) {
        (true, _) => Status::Ready,
        (false, Fetch::Running) => Status::Fetching,
        (false, _) if !offered => Status::Unavailable,
        (false, Fetch::Failed(kind)) => Status::FetchFailed(kind.clone()),
        (false, Fetch::Idle) => Status::Missing,
    }
}

/// A few `stat`, nothing else: fit for a render.
pub(crate) fn status() -> Status {
    status_of(present(), &fetch_state().lock(), download::offered())
}

/// Claim the fetch. False when the daemon is there, a fetch already runs
/// or this copy of the app cannot fetch.
pub(crate) fn begin_fetch() -> bool {
    if present() || !download::offered() {
        return false;
    }
    let mut state = fetch_state().lock();
    if *state == Fetch::Running {
        return false;
    }
    *state = Fetch::Running;
    true
}

/// Fetch the daemon files into the per-user directory, after
/// [`begin_fetch`] returned true. Blocks on the network: background
/// threads only.
pub(crate) fn fetch() -> Result<(), FetchErrorKind> {
    let dir = user_dir();
    let result = download::fetch_blocking(&dir);
    let mut state = fetch_state().lock();
    match result {
        Ok(()) => {
            // Best effort: without it the files are simply never cleaned.
            let _ = std::fs::write(dir.join(FETCHED_MARKER), shelldeck_update::CURRENT_VERSION);
            *state = Fetch::Idle;
            tracing::info!("Session daemon files fetched to {}", dir.display());
            Ok(())
        }
        Err(error) => {
            tracing::warn!("Session daemon files not fetched: {error}");
            *state = Fetch::Failed(error.kind.clone());
            Err(error.kind)
        }
    }
}

/// The reason shown for a fetch that failed.
pub(crate) fn fetch_failure_text(kind: &FetchErrorKind) -> String {
    match kind {
        FetchErrorKind::OtherVersion(version) => t!(
            "toast.session_daemon_files.reason.other_version",
            version = version.as_str()
        )
        .to_string(),
        other => t!(fetch_failure_key(other)).to_string(),
    }
}

fn fetch_failure_key(kind: &FetchErrorKind) -> &'static str {
    match kind {
        FetchErrorKind::NotOffered => "toast.session_daemon_files.reason.not_offered",
        FetchErrorKind::NotPublished => "toast.session_daemon_files.reason.not_published",
        FetchErrorKind::OtherVersion(_) => "toast.session_daemon_files.reason.other_version",
        FetchErrorKind::Download => "toast.session_daemon_files.reason.download",
        FetchErrorKind::Invalid => "toast.session_daemon_files.reason.invalid",
        FetchErrorKind::Write => "toast.session_daemon_files.reason.write",
    }
}

/// Delete fetched files that an update has since installed beside the
/// application binary. Only files this module fetched (the marker), only
/// when the daemon beside the binary is the one in use. A daemon still
/// running from a deleted file keeps it.
pub(crate) fn remove_superseded() {
    let Some(exe_dir) = exe_dir() else {
        return;
    };
    if std::env::var_os("SHELLDECK_SESSIOND_BIN").is_some() || exe_dir_is_transient(&exe_dir) {
        return;
    }
    remove_superseded_in(&exe_dir, &user_dir());
}

fn remove_superseded_in(exe_dir: &Path, dir: &Path) {
    if !exe_dir.join(LOCAL_DAEMON).is_file() || !dir.join(FETCHED_MARKER).is_file() {
        return;
    }
    for name in FETCHED_FILES {
        let path = dir.join(name);
        if path
            .symlink_metadata()
            .is_ok_and(|metadata| metadata.is_file())
        {
            let _ = std::fs::remove_file(path);
        }
    }
    let _ = std::fs::remove_file(dir.join(FETCHED_MARKER));
    // Only when nothing else is in it.
    let _ = std::fs::remove_dir(dir);
}

#[cfg(test)]
mod tests {
    use super::*;

    // SDTEST-3059 (SDUC-574): the daemon is run in place from an ordinary
    // install and copied out of an AppImage mount or a macOS bundle.
    #[test]
    fn sdtest_3059_appimage_and_bundle_directories_are_not_run_in_place() {
        let install = Path::new("/home/u/.shelldeck/bin");
        assert!(!is_transient(install, None, false));
        // `$APPDIR` of some other AppImage in the environment.
        assert!(!is_transient(
            install,
            Some(Path::new("/tmp/.mount_other")),
            false
        ));
        assert!(is_transient(
            Path::new("/tmp/.mount_ShellDabc/usr/bin"),
            Some(Path::new("/tmp/.mount_ShellDabc")),
            false
        ));
        // Not a path at all: ignored.
        assert!(!is_transient(install, Some(Path::new("")), false));
        let bundle = Path::new("/Applications/Inklura Desktop.app/Contents/MacOS");
        assert!(is_transient(bundle, None, true));
        // The same shape of path means nothing elsewhere.
        assert!(!is_transient(bundle, None, false));
        assert!(!is_transient(Path::new("/usr/local/bin"), None, true));
    }

    // SDTEST-3060 (SDUC-574): the copy out of the image is made once,
    // replaced by rename when the bundled daemon changed, and leaves no
    // temporary file.
    #[test]
    fn sdtest_3060_bundled_daemon_is_copied_out_and_refreshed() {
        let temp = tempfile::tempdir().unwrap();
        let bundled = temp.path().join("mount").join(LOCAL_DAEMON);
        std::fs::create_dir_all(bundled.parent().unwrap()).unwrap();
        std::fs::write(&bundled, b"\x7fELF daemon one").unwrap();
        let dir = temp.path().join("config").join("sessiond");
        let listing = || {
            let mut names: Vec<String> = std::fs::read_dir(&dir)
                .unwrap()
                .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
                .collect();
            names.sort();
            names
        };

        let copy = copy_out(&bundled, &dir).unwrap();
        assert_eq!(copy, dir.join(LOCAL_DAEMON));
        assert_eq!(std::fs::read(&copy).unwrap(), b"\x7fELF daemon one");
        assert_eq!(listing(), [LOCAL_DAEMON]);
        #[cfg(unix)]
        {
            use std::os::unix::fs::{MetadataExt, PermissionsExt};
            let metadata = std::fs::metadata(&copy).unwrap();
            assert_eq!(metadata.permissions().mode() & 0o777, 0o755);
            // Unchanged: the file is not rewritten (a running daemon's
            // file stays the installed one).
            let inode = metadata.ino();
            copy_out(&bundled, &dir).unwrap();
            assert_eq!(std::fs::metadata(&copy).unwrap().ino(), inode);
            // Another image, same length: replaced, by a new file.
            std::fs::write(&bundled, b"\x7fELF daemon two").unwrap();
            copy_out(&bundled, &dir).unwrap();
            assert_ne!(std::fs::metadata(&copy).unwrap().ino(), inode);
        }
        std::fs::write(&bundled, b"\x7fELF daemon three, longer").unwrap();
        copy_out(&bundled, &dir).unwrap();
        assert_eq!(
            std::fs::read(&copy).unwrap(),
            b"\x7fELF daemon three, longer"
        );
        assert_eq!(listing(), [LOCAL_DAEMON]);

        // The image has no daemon: an error, and the copy is untouched.
        std::fs::remove_file(&bundled).unwrap();
        assert!(copy_out(&bundled, &dir).is_err());
        assert_eq!(
            std::fs::read(&copy).unwrap(),
            b"\x7fELF daemon three, longer"
        );
    }

    // SDTEST-3061 (SDUC-574): what is shown about a missing daemon.
    #[test]
    fn sdtest_3061_status_of_the_daemon_files() {
        let failed = Fetch::Failed(FetchErrorKind::Download);
        // There: nothing else matters.
        assert_eq!(status_of(true, &Fetch::Idle, false), Status::Ready);
        assert_eq!(status_of(true, &failed, true), Status::Ready);
        // Missing in a copy of the app that can fetch it.
        assert_eq!(status_of(false, &Fetch::Idle, true), Status::Missing);
        assert_eq!(status_of(false, &Fetch::Running, true), Status::Fetching);
        assert_eq!(
            status_of(false, &failed, true),
            Status::FetchFailed(FetchErrorKind::Download)
        );
        // Missing in one that cannot.
        assert_eq!(status_of(false, &Fetch::Idle, false), Status::Unavailable);
    }

    // SDTEST-3062 (SDUC-574): fetched files are deleted once an update
    // installed the daemon beside the application, and only then; files
    // that were not fetched are never deleted.
    #[test]
    fn sdtest_3062_fetched_files_are_removed_once_superseded() {
        let temp = tempfile::tempdir().unwrap();
        let exe_dir = temp.path().join("bin");
        let dir = temp.path().join("sessiond");
        std::fs::create_dir_all(&exe_dir).unwrap();
        std::fs::create_dir_all(&dir).unwrap();
        let remote = FETCHED_FILES[1];
        std::fs::write(dir.join(LOCAL_DAEMON), b"\x7fELF").unwrap();
        std::fs::write(dir.join(remote), b"\x7fELF").unwrap();

        // Put there by hand (no marker): kept, whatever is installed.
        std::fs::write(exe_dir.join(LOCAL_DAEMON), b"\x7fELF").unwrap();
        remove_superseded_in(&exe_dir, &dir);
        assert!(dir.join(LOCAL_DAEMON).is_file() && dir.join(remote).is_file());

        // Fetched, and still the only daemon: kept.
        std::fs::write(dir.join(FETCHED_MARKER), b"1.2.3").unwrap();
        std::fs::remove_file(exe_dir.join(LOCAL_DAEMON)).unwrap();
        remove_superseded_in(&exe_dir, &dir);
        assert!(dir.join(LOCAL_DAEMON).is_file() && dir.join(remote).is_file());

        // An update installed the daemon beside the application.
        std::fs::write(exe_dir.join(LOCAL_DAEMON), b"\x7fELF").unwrap();
        std::fs::write(dir.join("notes.txt"), b"mine").unwrap();
        remove_superseded_in(&exe_dir, &dir);
        assert!(!dir.join(LOCAL_DAEMON).exists() && !dir.join(remote).exists());
        assert!(!dir.join(FETCHED_MARKER).exists());
        // Someone else's file, so the directory stays.
        assert!(dir.join("notes.txt").is_file());
    }

    // SDTEST-3063 (SDUC-574): every reason a fetch can fail for has a
    // sentence in both languages.
    #[test]
    fn sdtest_3063_every_fetch_failure_has_a_translated_reason() {
        let kinds = [
            FetchErrorKind::NotOffered,
            FetchErrorKind::NotPublished,
            FetchErrorKind::OtherVersion("1.2.3".into()),
            FetchErrorKind::Download,
            FetchErrorKind::Invalid,
            FetchErrorKind::Write,
        ];
        for locale in ["en", "fr"] {
            let text = std::fs::read_to_string(format!(
                "{}/../shelldeck-core/locales/{locale}.toml",
                env!("CARGO_MANIFEST_DIR")
            ))
            .unwrap();
            for kind in &kinds {
                let key = fetch_failure_key(kind);
                assert!(
                    text.lines()
                        .any(|line| line.starts_with(&format!("{key} = \""))),
                    "{locale}: {key}"
                );
            }
        }
    }
}
