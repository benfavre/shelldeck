use std::ffi::{OsStr, OsString};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static ATOMIC_WRITE_NONCE: AtomicU64 = AtomicU64::new(0);

/// Atomically write `contents` to `path`.
///
/// Writes to a temporary file in the *same directory* as `path` (so the final
/// `rename` stays on one filesystem and is therefore atomic), flushes it to
/// disk, then renames it over `path`. If anything fails, the temporary file is
/// removed on a best-effort basis so no partial/leftover files are left behind.
///
/// Temporary files are opened with `create_new`, and a process-local nonce
/// prevents concurrent writers to the same target from sharing an inode.
pub fn atomic_write(path: &Path, contents: &[u8]) -> std::io::Result<()> {
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("shelldeck");
    let (mut file, tmp_path) = loop {
        let nonce = ATOMIC_WRITE_NONCE.fetch_add(1, Ordering::Relaxed);
        let tmp_name = format!(".{}.tmp-{}-{nonce}", file_name, std::process::id());
        let tmp_path = dir.join(tmp_name);
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp_path)
        {
            Ok(file) => break (file, tmp_path),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    };

    // Write + flush the temp file, cleaning it up on any error.
    let write_result = (|| {
        file.write_all(contents)?;
        file.flush()?;
        file.sync_all()?;
        Ok(())
    })();

    if let Err(e) = write_result {
        let _ = std::fs::remove_file(&tmp_path);
        return Err(e);
    }
    drop(file);

    // Atomic replace. `std::fs::rename` cannot replace an existing destination
    // on Windows, so the platform seam uses MoveFileExW there. Keeping the
    // replacement in one helper also means the same overwrite tests exercise
    // the native primitive on every release runner.
    if let Err(e) = atomic_replace(&tmp_path, path) {
        let _ = std::fs::remove_file(&tmp_path);
        return Err(e);
    }

    // On Unix, syncing the file is not enough to make the directory entry
    // durable across a power loss. Windows does not allow opening directories
    // through `File::open`, so its replace primitive remains the durability
    // boundary there.
    #[cfg(unix)]
    if let Ok(directory) = std::fs::File::open(dir) {
        if let Err(error) = directory.sync_all() {
            // Some Unix filesystems do not support directory fsync. The file
            // and rename are still complete there; do not make every save
            // fail on an unsupported durability enhancement.
            if !matches!(
                error.kind(),
                std::io::ErrorKind::InvalidInput | std::io::ErrorKind::PermissionDenied
            ) {
                return Err(error);
            }
        }
    }

    Ok(())
}

#[cfg(not(windows))]
fn atomic_replace(source: &Path, destination: &Path) -> std::io::Result<()> {
    std::fs::rename(source, destination)
}

#[cfg(windows)]
fn atomic_replace(source: &Path, destination: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use std::time::Duration;
    use windows_sys::Win32::Storage::FileSystem::{
        MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    };

    let source: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
    let destination: Vec<u16> = destination
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();
    // Two writers replacing the same destination can briefly observe a
    // sharing/access/lock violation even though both source handles are
    // already closed. Retry only those documented Win32 contention errors;
    // every other failure remains immediate and fail-closed. The bound keeps
    // an externally held file from stalling a save indefinitely.
    const MAX_ATTEMPTS: usize = 8;
    for attempt in 0..MAX_ATTEMPTS {
        // SAFETY: both paths are owned, NUL-terminated UTF-16 buffers that
        // remain alive for the duration of the call. The flags request a
        // same-volume replacement and make the move wait for filesystem
        // write-through.
        let replaced = unsafe {
            MoveFileExW(
                source.as_ptr(),
                destination.as_ptr(),
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            )
        };
        if replaced != 0 {
            return Ok(());
        }

        let error = std::io::Error::last_os_error();
        if attempt + 1 == MAX_ATTEMPTS || !is_windows_file_contention(&error) {
            return Err(error);
        }
        std::thread::yield_now();
        std::thread::sleep(Duration::from_millis(1));
    }
    unreachable!("bounded replacement loop always returns")
}

/// Win32 reports both non-blocking file locks and short replacement races
/// through raw system codes instead of consistently mapping them to
/// `ErrorKind::WouldBlock`.
#[cfg(windows)]
pub(crate) fn is_windows_file_contention(error: &std::io::Error) -> bool {
    use windows_sys::Win32::Foundation::{
        ERROR_ACCESS_DENIED, ERROR_LOCK_VIOLATION, ERROR_SHARING_VIOLATION,
    };

    error.raw_os_error().is_some_and(|code| {
        matches!(
            code as u32,
            ERROR_ACCESS_DENIED | ERROR_SHARING_VIOLATION | ERROR_LOCK_VIOLATION
        )
    })
}

/// Shell-escape a string for safe embedding in single-quoted shell arguments.
pub fn shell_escape(s: &str) -> String {
    let escaped = s.replace('\'', "'\\''");
    format!("'{}'", escaped)
}

/// Get the user's home directory, returning `None` if it cannot be determined.
///
/// Order: `$HOME` (Unix convention, also honored on Windows when set), then
/// `%USERPROFILE%` (Windows), then the `directories` crate's platform lookup
/// (native API on Windows/macOS, passwd db on Unix). Never fabricates a path.
pub fn home_dir() -> Option<PathBuf> {
    home_dir_from_env(
        std::env::var("HOME").ok().as_deref(),
        std::env::var("USERPROFILE").ok().as_deref(),
    )
    .or_else(|| directories::BaseDirs::new().map(|dirs| dirs.home_dir().to_path_buf()))
}

/// Env-only part of [`home_dir`], injectable for tests.
fn home_dir_from_env(home: Option<&str>, userprofile: Option<&str>) -> Option<PathBuf> {
    [home, userprofile]
        .into_iter()
        .flatten()
        .find(|value| !value.trim().is_empty())
        .map(PathBuf::from)
}

/// Get the current username from environment variables.
///
/// Order: `$USER`, `$LOGNAME` (Unix), then `%USERNAME%` (Windows).
pub fn current_username() -> Option<String> {
    username_from_env(
        std::env::var("USER").ok().as_deref(),
        std::env::var("LOGNAME").ok().as_deref(),
        std::env::var("USERNAME").ok().as_deref(),
    )
}

/// Env-only part of [`current_username`], injectable for tests.
fn username_from_env(
    user: Option<&str>,
    logname: Option<&str>,
    username: Option<&str>,
) -> Option<String> {
    [user, logname, username]
        .into_iter()
        .flatten()
        .map(str::trim)
        .find(|value| !value.is_empty())
        .map(str::to_string)
}

/// Best-effort machine hostname, never empty.
///
/// Order: `$HOSTNAME`, `%COMPUTERNAME%` (Windows), then the platform lookup
/// (`gethostname(2)` on Unix, with `/etc/hostname` as a last resort). Falls
/// back to `"ShellDeck"` only when everything else fails.
pub fn hostname() -> String {
    hostname_from_env(
        std::env::var("HOSTNAME").ok().as_deref(),
        std::env::var("COMPUTERNAME").ok().as_deref(),
    )
    .or_else(platform_hostname)
    .unwrap_or_else(|| "ShellDeck".to_string())
}

/// Env-only part of [`hostname`], injectable for tests.
fn hostname_from_env(hostname: Option<&str>, computername: Option<&str>) -> Option<String> {
    [hostname, computername]
        .into_iter()
        .flatten()
        .map(str::trim)
        .find(|value| !value.is_empty())
        .map(str::to_string)
}

#[cfg(unix)]
fn platform_hostname() -> Option<String> {
    let mut buf = [0u8; 256];
    // SAFETY: `buf` is valid for `buf.len()` writes for the duration of the
    // call. POSIX leaves NUL-termination unspecified on truncation, so the
    // last byte is force-terminated below before scanning.
    let rc = unsafe { libc::gethostname(buf.as_mut_ptr() as *mut libc::c_char, buf.len()) };
    if rc == 0 {
        buf[buf.len() - 1] = 0;
        let end = buf.iter().position(|&b| b == 0).unwrap_or(0);
        let name = String::from_utf8_lossy(&buf[..end]).trim().to_string();
        if !name.is_empty() {
            return Some(name);
        }
    }
    // Linux last resort (the file simply doesn't exist elsewhere).
    std::fs::read_to_string("/etc/hostname")
        .ok()
        .map(|contents| contents.trim().to_string())
        .filter(|name| !name.is_empty())
}

#[cfg(not(unix))]
fn platform_hostname() -> Option<String> {
    // On Windows the OS sets %COMPUTERNAME% in every normal session, so the
    // env path above already covers it; a GetComputerNameW syscall would add
    // a windows-sys dependency for a case that cannot practically occur.
    None
}

/// True when `name` resolves to an executable file on `PATH`.
///
/// Honors `PATHEXT` on Windows (falling back to `.COM/.EXE/.BAT/.CMD` like
/// `cmd.exe` when it is unset); on Unix the bare name is looked up as-is and
/// must carry an executable permission bit.
pub fn executable_on_path(name: &str) -> bool {
    executable_on_path_in(
        name,
        std::env::var_os("PATH").as_deref(),
        &path_extensions(),
    )
}

fn path_extensions() -> Vec<OsString> {
    path_extensions_from(std::env::var("PATHEXT").ok().as_deref(), cfg!(windows))
}

/// Pure part of the `PATHEXT` handling, injectable for tests: on Windows the
/// candidate suffixes come from `PATHEXT` (with the `cmd.exe` defaults as
/// fallback); elsewhere only the bare name (empty suffix) is probed.
fn path_extensions_from(pathext: Option<&str>, windows: bool) -> Vec<OsString> {
    if !windows {
        return vec![OsString::new()];
    }
    let parsed: Vec<OsString> = pathext
        .map(|value| {
            value
                .split(';')
                .map(str::trim)
                .filter(|ext| !ext.is_empty())
                .map(OsString::from)
                .collect()
        })
        .unwrap_or_default();
    if parsed.is_empty() {
        [".COM", ".EXE", ".BAT", ".CMD"]
            .into_iter()
            .map(OsString::from)
            .collect()
    } else {
        parsed
    }
}

/// Pure part of [`executable_on_path`], injectable for tests.
fn executable_on_path_in(name: &str, path: Option<&OsStr>, extensions: &[OsString]) -> bool {
    if name.is_empty() {
        return false;
    }
    let Some(path) = path else {
        return false;
    };
    std::env::split_paths(path).any(|directory| {
        extensions.iter().any(|extension| {
            let mut filename = OsString::from(name);
            filename.push(extension);
            is_executable_file(&directory.join(filename))
        })
    })
}

/// A regular file that the platform would let us execute.
fn is_executable_file(path: &Path) -> bool {
    let Ok(metadata) = std::fs::metadata(path) else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        // Windows executability is determined by the extension, which the
        // PATHEXT candidates above already encode.
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Create a unique temp directory for a test and return its path.
    fn unique_temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "shelldeck-test-{}-{}-{}",
            tag,
            std::process::id(),
            // A monotonic-ish counter per call so parallel tests don't collide.
            COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

    /// Count `.tmp` (or `.tmp-*`) leftover files in a directory.
    fn count_tmp_files(dir: &Path) -> usize {
        std::fs::read_dir(dir)
            .expect("read_dir")
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains(".tmp-"))
            .count()
    }

    #[test]
    fn atomic_write_creates_new_file() {
        let dir = unique_temp_dir("create");
        let path = dir.join("data.txt");

        atomic_write(&path, b"hello world").expect("write");

        let read = std::fs::read(&path).expect("read back");
        assert_eq!(read, b"hello world");
        assert_eq!(count_tmp_files(&dir), 0, "no leftover tmp files");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn atomic_write_overwrites_existing_file() {
        let dir = unique_temp_dir("overwrite");
        let path = dir.join("data.txt");

        std::fs::write(&path, b"old contents that are longer").expect("seed");
        atomic_write(&path, b"new").expect("write");

        let read = std::fs::read(&path).expect("read back");
        assert_eq!(read, b"new");
        assert_eq!(count_tmp_files(&dir), 0, "no leftover tmp files");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn atomic_write_leaves_no_tmp_files() {
        let dir = unique_temp_dir("notmp");
        let path = dir.join("nested.json");

        atomic_write(&path, b"{}").expect("write1");
        atomic_write(&path, b"[]").expect("write2");
        atomic_write(&path, b"null").expect("write3");

        assert_eq!(count_tmp_files(&dir), 0);
        assert_eq!(std::fs::read(&path).unwrap(), b"null");

        std::fs::remove_dir_all(&dir).ok();
    }

    // SDTEST-1735
    #[test]
    fn concurrent_atomic_writers_use_distinct_temporary_files() {
        let dir = unique_temp_dir("concurrent");
        let path = dir.join("catalog.json");
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(3));
        let writers: Vec<_> = [b"first".as_slice(), b"second".as_slice()]
            .into_iter()
            .map(|payload| {
                let path = path.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    atomic_write(&path, payload)
                })
            })
            .collect();

        barrier.wait();
        for writer in writers {
            writer.join().expect("writer thread").expect("atomic write");
        }
        let result = std::fs::read(&path).expect("final payload");
        assert!(result == b"first" || result == b"second");
        assert_eq!(count_tmp_files(&dir), 0);
        std::fs::remove_dir_all(&dir).ok();
    }

    // SDTEST-1742
    #[cfg(windows)]
    #[test]
    fn windows_atomic_replace_retries_only_file_contention_errors() {
        use windows_sys::Win32::Foundation::{
            ERROR_ACCESS_DENIED, ERROR_FILE_NOT_FOUND, ERROR_LOCK_VIOLATION,
            ERROR_SHARING_VIOLATION,
        };

        for code in [
            ERROR_ACCESS_DENIED,
            ERROR_SHARING_VIOLATION,
            ERROR_LOCK_VIOLATION,
        ] {
            assert!(is_windows_file_contention(
                &std::io::Error::from_raw_os_error(code as i32)
            ));
        }
        assert!(!is_windows_file_contention(
            &std::io::Error::from_raw_os_error(ERROR_FILE_NOT_FOUND as i32)
        ));
    }

    // ------------------------------------------------------------------
    // Platform helpers — env portions are injectable, so no test below
    // reads or mutates the runner's real environment.
    // ------------------------------------------------------------------

    #[test]
    fn home_dir_prefers_home_then_userprofile_and_rejects_blanks() {
        assert_eq!(
            home_dir_from_env(Some("/home/ben"), Some("C:\\Users\\ben")),
            Some(PathBuf::from("/home/ben"))
        );
        assert_eq!(
            home_dir_from_env(None, Some("C:\\Users\\ben")),
            Some(PathBuf::from("C:\\Users\\ben"))
        );
        assert_eq!(
            home_dir_from_env(Some(""), Some("C:\\Users\\ben")),
            Some(PathBuf::from("C:\\Users\\ben")),
            "empty HOME must fall through, not produce an empty path"
        );
        assert_eq!(home_dir_from_env(Some("   "), Some("")), None);
        assert_eq!(home_dir_from_env(None, None), None);
    }

    #[test]
    fn username_prefers_user_then_logname_then_username() {
        assert_eq!(
            username_from_env(Some("ben"), Some("log"), Some("win")),
            Some("ben".to_string())
        );
        assert_eq!(
            username_from_env(None, Some("log"), Some("win")),
            Some("log".to_string())
        );
        assert_eq!(
            username_from_env(Some(""), None, Some("win")),
            Some("win".to_string()),
            "empty USER must fall through to the Windows variable"
        );
        assert_eq!(username_from_env(None, None, None), None);
    }

    #[test]
    fn hostname_env_prefers_hostname_then_computername_and_trims() {
        assert_eq!(
            hostname_from_env(Some("  srv1  "), Some("WIN-PC")),
            Some("srv1".to_string())
        );
        assert_eq!(
            hostname_from_env(Some("   "), Some("WIN-PC")),
            Some("WIN-PC".to_string()),
            "whitespace-only HOSTNAME must fall through"
        );
        assert_eq!(hostname_from_env(None, None), None);
    }

    #[test]
    fn hostname_is_never_empty() {
        // Whatever the runner's env / platform, the chain must bottom out at
        // a non-empty value (final fallback "ShellDeck").
        assert!(!hostname().is_empty());
    }

    #[test]
    fn path_extensions_windows_parses_pathext_and_defaults() {
        // Non-Windows: exactly one empty suffix (bare-name lookup).
        assert_eq!(path_extensions_from(None, false), vec![OsString::new()]);
        assert_eq!(
            path_extensions_from(Some(".COM;.EXE"), false),
            vec![OsString::new()],
            "PATHEXT is Windows-only and must be ignored elsewhere"
        );

        // Windows: PATHEXT entries, trimmed, empties dropped.
        assert_eq!(
            path_extensions_from(Some(".COM; .EXE ;;.PS1"), true),
            vec![
                OsString::from(".COM"),
                OsString::from(".EXE"),
                OsString::from(".PS1")
            ]
        );

        // Windows without (usable) PATHEXT: cmd.exe defaults.
        let defaults = [".COM", ".EXE", ".BAT", ".CMD"]
            .into_iter()
            .map(OsString::from)
            .collect::<Vec<_>>();
        assert_eq!(path_extensions_from(None, true), defaults);
        assert_eq!(path_extensions_from(Some(" ; ; "), true), defaults);
    }

    /// Mark a file executable where that is a distinct permission bit.
    fn make_executable(path: &Path) {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
                .expect("chmod +x");
        }
        #[cfg(not(unix))]
        {
            let _ = path;
        }
    }

    #[test]
    fn executable_lookup_searches_all_path_dirs_with_extensions() {
        let dir_a = unique_temp_dir("path-a");
        let dir_b = unique_temp_dir("path-b");

        // Bare name in the *second* PATH entry.
        let tool = dir_b.join("mytool");
        std::fs::write(&tool, b"#!/bin/sh\n").expect("write tool");
        make_executable(&tool);
        // Suffixed name (Windows-style candidate) in the first entry.
        let batch = dir_a.join("build.CMD");
        std::fs::write(&batch, b"@echo off\n").expect("write batch");
        make_executable(&batch);

        let path = std::env::join_paths([dir_a.clone(), dir_b.clone()]).expect("join_paths");
        let bare = [OsString::new()];
        let win_exts = [OsString::from(".COM"), OsString::from(".CMD")];

        assert!(executable_on_path_in("mytool", Some(&path), &bare));
        assert!(executable_on_path_in("build", Some(&path), &win_exts));
        assert!(
            !executable_on_path_in("build", Some(&path), &bare),
            "without the suffix candidates the batch file must not match"
        );
        assert!(!executable_on_path_in("missing", Some(&path), &bare));
        assert!(!executable_on_path_in("", Some(&path), &bare));
        assert!(!executable_on_path_in("mytool", None, &bare));

        std::fs::remove_dir_all(&dir_a).ok();
        std::fs::remove_dir_all(&dir_b).ok();
    }

    #[test]
    fn executable_lookup_rejects_directories_and_non_executables() {
        let dir = unique_temp_dir("path-neg");

        // A directory named like the command is not an executable.
        std::fs::create_dir_all(dir.join("subcmd")).expect("mkdir");

        let path = std::env::join_paths([dir.clone()]).expect("join_paths");
        let bare = [OsString::new()];
        assert!(!executable_on_path_in("subcmd", Some(&path), &bare));

        // Unix only: a plain file without +x must not count as executable.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let plain = dir.join("datafile");
            std::fs::write(&plain, b"not a program").expect("write plain");
            std::fs::set_permissions(&plain, std::fs::Permissions::from_mode(0o644))
                .expect("chmod -x");
            assert!(!executable_on_path_in("datafile", Some(&path), &bare));
        }

        std::fs::remove_dir_all(&dir).ok();
    }
}
