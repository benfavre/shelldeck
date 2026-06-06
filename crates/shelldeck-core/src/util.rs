use std::io::Write;
use std::path::{Path, PathBuf};

/// Atomically write `contents` to `path`.
///
/// Writes to a temporary file in the *same directory* as `path` (so the final
/// `rename` stays on one filesystem and is therefore atomic), flushes it to
/// disk, then renames it over `path`. If anything fails, the temporary file is
/// removed on a best-effort basis so no partial/leftover files are left behind.
///
/// Uniqueness of the temp file name is derived from the process id and the
/// target file name only — no randomness or clock is used, so it works even
/// when those facilities are unavailable.
pub fn atomic_write(path: &Path, contents: &[u8]) -> std::io::Result<()> {
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("shelldeck");
    let tmp_name = format!(".{}.tmp-{}", file_name, std::process::id());
    let tmp_path = dir.join(tmp_name);

    // Write + flush the temp file, cleaning it up on any error.
    let write_result = (|| {
        let mut file = std::fs::File::create(&tmp_path)?;
        file.write_all(contents)?;
        file.flush()?;
        file.sync_all()?;
        Ok(())
    })();

    if let Err(e) = write_result {
        let _ = std::fs::remove_file(&tmp_path);
        return Err(e);
    }

    // Atomic replace.
    if let Err(e) = std::fs::rename(&tmp_path, path) {
        let _ = std::fs::remove_file(&tmp_path);
        return Err(e);
    }

    Ok(())
}

/// Shell-escape a string for safe embedding in single-quoted shell arguments.
pub fn shell_escape(s: &str) -> String {
    let escaped = s.replace('\'', "'\\''");
    format!("'{}'", escaped)
}

/// Get the user's home directory from environment, returning `None` if unavailable.
pub fn home_dir() -> Option<PathBuf> {
    std::env::var("HOME")
        .ok()
        .filter(|h| !h.is_empty())
        .map(PathBuf::from)
}

/// Get the current username from environment variables.
pub fn current_username() -> Option<String> {
    std::env::var("USER")
        .or_else(|_| std::env::var("LOGNAME"))
        .ok()
        .filter(|u| !u.is_empty())
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
}
