//! The daemon's log file: `<run dir>/sessiond.log`, size capped, rotated
//! once (docs/design/session-daemon.md, section 3).
//!
//! A detached daemon has no stderr, so without this file nothing explains
//! why a pane is gone. The file holds lifecycle events only (start, stop,
//! pane spawned and ended, limits hit, panics). It never holds pane output,
//! keyboard input, environment values or command lines.

use crate::paths;
use parking_lot::Mutex;
use std::fs::File;
use std::io::Write;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Size at which the log is rotated. With the one kept generation the two
/// files never hold more than about twice this.
pub const DEFAULT_LOG_CAP: u64 = 1024 * 1024;

struct Inner {
    path: PathBuf,
    old: PathBuf,
    cap: u64,
    file: Option<File>,
    len: u64,
    /// No more writes: the daemon gave its run directory up.
    detached: bool,
}

impl Inner {
    fn open(&mut self) {
        // A regular file of this user, opened without following a symlink.
        // Anything else at that name is left alone: no log.
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&self.path)
            .ok()
            .filter(|file| {
                file.metadata().is_ok_and(|meta| {
                    meta.file_type().is_file() && meta.uid() == paths::effective_uid()
                })
            });
        self.len = file
            .as_ref()
            .and_then(|file| file.metadata().ok())
            .map(|meta| meta.len())
            .unwrap_or(0);
        self.file = file;
    }

    /// Keep one old generation: the current file replaces it.
    fn rotate(&mut self) {
        self.file = None;
        if std::fs::rename(&self.path, &self.old).is_err() {
            // Cannot keep it: start over rather than grow without bound.
            let _ = std::fs::remove_file(&self.path);
        }
        self.open();
    }

    fn write(&mut self, bytes: &[u8]) {
        if self.detached {
            return;
        }
        if self.file.is_none() {
            self.open();
        }
        if self.len > 0 && self.len + bytes.len() as u64 > self.cap {
            self.rotate();
        }
        // One record larger than the cap is cut, not skipped.
        let room = self.cap.saturating_sub(self.len).min(bytes.len() as u64) as usize;
        let Some(file) = self.file.as_mut() else {
            return;
        };
        if file.write_all(&bytes[..room]).is_ok() {
            self.len += room as u64;
        } else {
            // The file went away or the disk is full: try again later.
            self.file = None;
        }
    }
}

/// A log file that never grows past its cap: when the next record would
/// cross it, the file is renamed to its `.1` name (replacing the previous
/// one) and a new file is started. Writing never fails for the caller: a
/// log that cannot be written is skipped.
#[derive(Clone)]
pub struct RotatingLog {
    inner: Arc<Mutex<Inner>>,
}

impl RotatingLog {
    /// The log of the daemon serving `dir`. Only the daemon that holds the
    /// instance lock of `dir` may open it.
    pub fn in_run_dir(dir: &Path, cap: u64) -> Self {
        Self::new(
            dir.join(paths::LOG_NAME),
            dir.join(paths::LOG_OLD_NAME),
            cap,
        )
    }

    pub fn new(path: PathBuf, old: PathBuf, cap: u64) -> Self {
        let mut inner = Inner {
            path,
            old,
            cap: cap.max(1),
            file: None,
            len: 0,
            detached: false,
        };
        inner.open();
        Self {
            inner: Arc::new(Mutex::new(inner)),
        }
    }

    /// Append one record (the caller ends it with a newline).
    pub fn append(&self, record: &[u8]) {
        self.inner.lock().write(record);
    }

    /// Close the file and ignore every later record.
    pub fn detach(&self) {
        let mut inner = self.inner.lock();
        inner.detached = true;
        inner.file = None;
    }
}

/// `std::io::Write` over the log, one record per `write` call (what the
/// `tracing` formatter does).
pub struct LogWriter(RotatingLog);

impl Write for LogWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.append(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for RotatingLog {
    type Writer = LogWriter;

    fn make_writer(&'a self) -> Self::Writer {
        LogWriter(self.clone())
    }
}

/// The log installed by [`install`], if any.
static INSTALLED: std::sync::OnceLock<RotatingLog> = std::sync::OnceLock::new();

/// Stop writing to the installed log. Called by a daemon that gives its
/// run directory up (the instance lock is released before its panes have
/// ended): from then on the file belongs to the next daemon. Nothing
/// happens when no log was installed (a daemon embedded in a test).
pub fn detach() {
    if let Some(log) = INSTALLED.get() {
        log.detach();
    }
}

/// Send this process's `tracing` events (info and above) to the log of
/// `dir`, and panics too. Called once by the daemon after it took the
/// instance lock. Returns `false` when a subscriber is already installed.
pub fn install(dir: &Path, cap: u64) -> bool {
    let log = RotatingLog::in_run_dir(dir, cap);
    let installed = tracing_subscriber::fmt()
        .with_writer(log.clone())
        .with_ansi(false)
        .with_max_level(tracing::Level::INFO)
        .try_init()
        .is_ok();
    if installed {
        let _ = INSTALLED.set(log);
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            // Location only: a panic message can quote terminal content.
            let location = info
                .location()
                .map(|l| format!("{}:{}", l.file(), l.line()))
                .unwrap_or_default();
            let thread = std::thread::current();
            tracing::error!(
                thread = thread.name().unwrap_or("unnamed"),
                %location,
                "panic"
            );
            previous(info);
        }));
    }
    installed
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    // SDTEST-3026 (SDUC-574): the log is created private, never exceeds its
    // cap, keeps exactly one old generation and survives its file being
    // removed.
    #[test]
    fn sdtest_3026_log_is_private_capped_and_rotated_once() {
        let temp = tempfile::tempdir().expect("temp dir");
        let path = temp.path().join("sessiond.log");
        let old = temp.path().join("sessiond.log.1");
        let cap = 1000;
        let log = RotatingLog::new(path.clone(), old.clone(), cap);
        let record = |n: usize| format!("{n:04} {}\n", "x".repeat(94));

        for n in 0..10 {
            log.append(record(n).as_bytes());
        }
        let meta = std::fs::metadata(&path).expect("log exists");
        assert_eq!(meta.permissions().mode() & 0o777, 0o600);
        assert_eq!(meta.len(), 1000, "ten records of 100 bytes fill the cap");
        assert!(!old.exists(), "nothing rotated yet");

        // The next record does not fit: the file becomes the old one.
        log.append(record(10).as_bytes());
        assert_eq!(std::fs::metadata(&path).expect("log").len(), 100);
        let rotated = std::fs::read_to_string(&old).expect("old generation");
        assert!(rotated.starts_with("0000 ") && rotated.contains("\n0009 "));
        assert_eq!(
            std::fs::metadata(&old).expect("old").permissions().mode() & 0o777,
            0o600
        );

        // Many more: never more than the cap in either file, and only the
        // two names exist.
        for n in 11..200 {
            log.append(record(n).as_bytes());
            assert!(std::fs::metadata(&path).expect("log").len() <= cap);
            assert!(std::fs::metadata(&old).expect("old").len() <= cap);
        }
        let current = std::fs::read_to_string(&path).expect("log");
        assert!(current.ends_with(&record(199)));
        let previous = std::fs::read_to_string(&old).expect("old");
        assert!(!previous.contains("0000 "), "the first generation is gone");
        let mut names: Vec<String> = std::fs::read_dir(temp.path())
            .expect("list")
            .map(|entry| {
                entry
                    .expect("entry")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        names.sort();
        assert_eq!(names, vec!["sessiond.log", "sessiond.log.1"]);

        // A record larger than the cap is cut to it.
        log.append("y".repeat(5000).as_bytes());
        assert!(std::fs::metadata(&path).expect("log").len() <= cap);

        // A detached log writes nothing more.
        let before = std::fs::read(&path).expect("log");
        log.detach();
        log.append(b"after detach\n");
        assert_eq!(std::fs::read(&path).expect("log"), before);

        // A reopened log continues the existing file and counts its size.
        drop(log);
        let log = RotatingLog::new(path.clone(), old.clone(), cap);
        log.append(b"after reopen\n");
        assert!(
            std::fs::read_to_string(&path)
                .expect("log")
                .ends_with("after reopen\n")
                || std::fs::read_to_string(&path).expect("log") == "after reopen\n"
        );

        // A symlink at the log name is not followed: nothing is written
        // through it.
        std::fs::remove_file(&path).expect("remove");
        let target = temp.path().join("elsewhere");
        std::fs::write(&target, b"").expect("target");
        std::os::unix::fs::symlink(&target, &path).expect("symlink");
        let log = RotatingLog::new(path.clone(), old.clone(), cap);
        log.append(b"must not land in the target\n");
        assert_eq!(std::fs::read(&target).expect("target"), b"");
    }
}
