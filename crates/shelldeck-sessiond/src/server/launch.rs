//! Descriptor limit of the daemon and of its pane children
//! (docs/design/session-daemon.md, section 3).
//!
//! 256 panes at four descriptors each, plus the connections, do not fit
//! the usual soft limit of 1024 (256 on macOS). The daemon therefore raises
//! its own soft `RLIMIT_NOFILE`. A resource limit is inherited across
//! `fork` and `exec`, and programs behave differently under a high limit
//! (some close or scan every possible descriptor), so a pane child must
//! start with the limit the daemon was started with. The PTY library has
//! no hook between `fork` and `exec`, so the daemon starts each pane
//! through its own executable: `shelldeck-sessiond exec-pane` restores the
//! limit and replaces itself with the pane program. The pid stays the
//! same, so the pane's process group and exit status are those of the
//! program.

// `rlim_t` is `u64` on the platforms built today, not on every Unix.
#![allow(clippy::unnecessary_cast)]

use std::ffi::{CString, OsStr};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

/// Soft descriptor limit the daemon asks for (bounded by the hard limit).
pub const NOFILE_TARGET: u64 = 4096;

/// How pane children are started when the daemon raised its own limit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaneLauncher {
    /// Absolute path of the daemon executable.
    pub exe: PathBuf,
    /// Soft `RLIMIT_NOFILE` pane children start with: the daemon's limit
    /// before it was raised.
    pub nofile_soft: u64,
}

fn nofile_limit() -> Option<libc::rlimit> {
    let mut limit = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: `limit` is valid for writes.
    (unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut limit) } == 0).then_some(limit)
}

/// Current soft `RLIMIT_NOFILE` of this process.
pub fn nofile_soft() -> Option<u64> {
    nofile_limit().map(|limit| limit.rlim_cur as u64)
}

fn set_nofile_soft(soft: u64) -> bool {
    let Some(mut limit) = nofile_limit() else {
        return false;
    };
    let soft = soft as libc::rlim_t;
    limit.rlim_cur = if limit.rlim_max == libc::RLIM_INFINITY {
        soft
    } else {
        soft.min(limit.rlim_max)
    };
    // SAFETY: `limit` is a valid rlimit; only the soft value changes.
    unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &limit) == 0 }
}

/// Raise the soft descriptor limit of this process to `target` (or the
/// hard limit if lower). Returns the soft limit from before when it was
/// raised, `None` when it was already that high or could not be changed.
pub fn raise_nofile_limit(target: u64) -> Option<u64> {
    let before = nofile_limit()?;
    let wanted = if before.rlim_max == libc::RLIM_INFINITY {
        target
    } else {
        target.min(before.rlim_max as u64)
    };
    if before.rlim_cur != libc::RLIM_INFINITY
        && (before.rlim_cur as u64) < wanted
        && set_nofile_soft(wanted)
    {
        Some(before.rlim_cur as u64)
    } else {
        None
    }
}

fn is_executable_file(path: &Path) -> bool {
    let Ok(c_path) = CString::new(path.as_os_str().as_bytes()) else {
        return false;
    };
    // SAFETY: `c_path` is a valid NUL-terminated path.
    path.is_file() && unsafe { libc::access(c_path.as_ptr(), libc::X_OK) } == 0
}

/// Find the program a pane would run, the way the PTY library does when
/// it starts it directly: a name with a slash is taken relative to `cwd`,
/// a bare name is searched in `path` (the daemon's `PATH`). `None`: there
/// is nothing to run, and the spawn must fail before a pane exists.
pub fn resolve_program(program: &str, cwd: &Path, path: Option<&OsStr>) -> Option<PathBuf> {
    if program.is_empty() {
        return None;
    }
    if program.contains('/') {
        let candidate = cwd.join(program);
        return is_executable_file(&candidate).then_some(candidate);
    }
    std::env::split_paths(path?)
        .map(|dir| cwd.join(dir).join(program))
        .find(|candidate| is_executable_file(candidate))
}

impl PaneLauncher {
    /// The command line that starts `argv` (empty: the login shell) in a
    /// pane through the launcher. `None`: the program does not exist or is
    /// not executable.
    pub fn command(&self, argv: &[String], cwd: &Path) -> Option<Vec<String>> {
        let mut command = vec![
            self.exe.to_str()?.to_string(),
            "exec-pane".to_string(),
            "--nofile".to_string(),
            self.nofile_soft.to_string(),
        ];
        if let Some(program) = argv.first() {
            let path = std::env::var_os("PATH");
            let resolved = resolve_program(program, cwd, path.as_deref())?;
            command.push("--program".to_string());
            command.push(resolved.to_str()?.to_string());
            command.push("--".to_string());
            command.extend(argv.iter().cloned());
        }
        Some(command)
    }
}

/// Body of `shelldeck-sessiond exec-pane`: restore the soft descriptor
/// limit, then become the pane program. `program` with `argv` (its
/// `argv[0]` included) runs that program; without them the login shell of
/// `$SHELL` runs, with the leading dash that makes it a login shell.
///
/// Returns only on failure, with the reason.
pub fn exec_pane(nofile: Option<u64>, program: Option<&str>, argv: &[String]) -> std::io::Error {
    if let Some(soft) = nofile {
        // Best effort: a pane with the daemon's limit is better than no
        // pane.
        set_nofile_soft(soft);
    }
    let (program, argv): (String, Vec<String>) = match (program, argv.is_empty()) {
        (Some(program), false) => (program.to_string(), argv.to_vec()),
        (None, true) => {
            let shell = std::env::var("SHELL")
                .ok()
                .filter(|shell| shell.starts_with('/'))
                .unwrap_or_else(|| "/bin/sh".to_string());
            let name = shell.rsplit('/').next().unwrap_or("sh");
            let login = format!("-{name}");
            (shell, vec![login])
        }
        _ => return std::io::Error::other("exec-pane needs --program with a command, or neither"),
    };
    let Ok(c_program) = CString::new(program) else {
        return std::io::Error::other("program path contains a NUL byte");
    };
    let Ok(c_argv) = argv
        .into_iter()
        .map(CString::new)
        .collect::<Result<Vec<_>, _>>()
    else {
        return std::io::Error::other("argument contains a NUL byte");
    };
    let mut pointers: Vec<*const libc::c_char> = c_argv.iter().map(|arg| arg.as_ptr()).collect();
    pointers.push(std::ptr::null());
    // SAFETY: `c_program` and every element of `pointers` are valid
    // NUL-terminated strings that outlive the call, and the array ends
    // with a null pointer. `execv` only returns on failure.
    unsafe { libc::execv(c_program.as_ptr(), pointers.as_ptr()) };
    std::io::Error::last_os_error()
}

#[cfg(test)]
mod tests {
    use super::*;

    // SDTEST-3027 (SDUC-574): the launcher command line and the program
    // lookup that keeps "no such program" a spawn error.
    #[test]
    fn sdtest_3027_launcher_command_and_program_lookup() {
        let launcher = PaneLauncher {
            exe: PathBuf::from("/opt/shelldeck/shelldeck-sessiond"),
            nofile_soft: 1024,
        };
        let root = Path::new("/");
        // Login shell: no program, nothing after the limit.
        assert_eq!(
            launcher.command(&[], root).expect("login shell"),
            [
                "/opt/shelldeck/shelldeck-sessiond",
                "exec-pane",
                "--nofile",
                "1024"
            ]
        );
        // A command keeps its own argv[0]; the program is resolved first.
        let argv = vec![
            "/bin/sh".to_string(),
            "-c".to_string(),
            "echo hi".to_string(),
        ];
        assert_eq!(
            launcher.command(&argv, root).expect("command"),
            [
                "/opt/shelldeck/shelldeck-sessiond",
                "exec-pane",
                "--nofile",
                "1024",
                "--program",
                "/bin/sh",
                "--",
                "/bin/sh",
                "-c",
                "echo hi"
            ]
        );
        // A shell-style command line is one (missing) program name.
        assert_eq!(launcher.command(&["echo hi; id".to_string()], root), None);
        assert_eq!(
            launcher.command(&["/no/such/program".to_string()], root),
            None
        );

        let path = OsStr::new("/nonexistent-dir:/bin:/usr/bin");
        let sh = resolve_program("sh", root, Some(path)).expect("sh is in /bin or /usr/bin");
        assert!(sh.is_absolute() && sh.ends_with("sh"));
        assert_eq!(resolve_program("sh", root, None), None);
        assert_eq!(
            resolve_program("sh", root, Some(OsStr::new("/nonexistent-dir"))),
            None
        );
        assert_eq!(resolve_program("", root, Some(path)), None);
        // A directory and a file without the execute bit are not programs.
        assert_eq!(resolve_program("/bin", root, Some(path)), None);
        assert_eq!(resolve_program("/etc/passwd", root, Some(path)), None);
        // Relative with a slash: against the working directory only.
        assert_eq!(
            resolve_program("bin/sh", root, None),
            Some(PathBuf::from("/bin/sh"))
        );
        assert_eq!(
            resolve_program("bin/sh", Path::new("/usr/share"), Some(path)),
            None
        );

        // A limit that is already high enough is left alone.
        let soft = nofile_soft().expect("limit");
        assert_eq!(raise_nofile_limit(soft.min(16)), None);
        assert_eq!(nofile_soft(), Some(soft));
    }
}
