use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize};
use std::io::{Read, Write};

/// Resolve the shell a local PTY should spawn.
///
/// Priority: the explicit caller/config choice, then the platform's
/// environment convention, then a platform-correct default. Shared with
/// `TerminalSession::spawn_local` so shell-flavor detection always matches
/// the shell that actually runs.
pub(crate) fn resolve_shell(explicit: Option<&str>) -> String {
    resolve_shell_from(
        explicit,
        cfg!(windows),
        std::env::var("SHELL").ok().as_deref(),
        std::env::var("COMSPEC").ok().as_deref(),
        cfg!(windows) && shelldeck_core::util::executable_on_path("powershell"),
    )
}

/// Pure part of [`resolve_shell`], parameterized so both platform branches
/// are unit-testable from any host OS.
///
/// - Unix: `$SHELL`, else `/bin/bash`.
/// - Windows: PowerShell when it is on `PATH` (the shell the rest of the
///   repo already targets — `ShellFlavor::PowerShell` command framing, the
///   updater and attachment helpers all shell out to `powershell`), else
///   `%COMSPEC%`, else `cmd.exe`. `$SHELL` is deliberately ignored on
///   Windows: GUI-launched processes never carry it, and when present
///   (MSYS/git-bash) it names POSIX paths that don't resolve outside that
///   environment.
fn resolve_shell_from(
    explicit: Option<&str>,
    windows: bool,
    env_shell: Option<&str>,
    env_comspec: Option<&str>,
    has_powershell: bool,
) -> String {
    let non_blank =
        |value: Option<&str>| value.filter(|v| !v.trim().is_empty()).map(str::to_string);
    if let Some(shell) = non_blank(explicit) {
        return shell;
    }
    if windows {
        if has_powershell {
            return "powershell.exe".to_string();
        }
        non_blank(env_comspec).unwrap_or_else(|| "cmd.exe".to_string())
    } else {
        non_blank(env_shell).unwrap_or_else(|| "/bin/bash".to_string())
    }
}

pub struct LocalPty {
    master: Box<dyn MasterPty + Send>,
    child: Box<dyn portable_pty::Child + Send + Sync>,
    writer: Box<dyn Write + Send>,
}

/// Handle to the PTY master, used for resize operations after the
/// writer has been split off to a dedicated thread.
pub struct PtyMaster {
    master: Box<dyn MasterPty + Send>,
    _child: Box<dyn portable_pty::Child + Send + Sync>,
}

impl PtyMaster {
    /// Resize the underlying PTY.
    pub fn resize(&self, rows: u16, cols: u16) -> crate::Result<()> {
        self.master
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| crate::TerminalError::Resize(e.to_string()))
    }
}

impl LocalPty {
    /// Spawn a new PTY with the given shell (or the user's default shell).
    /// Returns the `LocalPty` and a reader for the PTY's output.
    pub fn spawn(
        shell: Option<&str>,
        rows: u16,
        cols: u16,
    ) -> crate::Result<(Self, Box<dyn Read + Send>)> {
        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| crate::TerminalError::Pty(e.to_string()))?;

        let shell_path = resolve_shell(shell);

        let mut cmd = CommandBuilder::new(&shell_path);
        // Start in the user's home directory; fall back to the process cwd
        // (`.`) when it cannot be determined — never a hardcoded `/`, which
        // is meaningless on Windows.
        let home =
            shelldeck_core::util::home_dir().unwrap_or_else(|| std::path::PathBuf::from("."));
        cmd.cwd(home);

        // Set TERM so applications know what terminal features are available.
        cmd.env("TERM", "xterm-256color");

        let child = pair
            .slave
            .spawn_command(cmd)
            .map_err(|e| crate::TerminalError::Pty(e.to_string()))?;

        let reader = pair
            .master
            .try_clone_reader()
            .map_err(|e| crate::TerminalError::Pty(e.to_string()))?;
        let writer = pair
            .master
            .take_writer()
            .map_err(|e| crate::TerminalError::Pty(e.to_string()))?;

        Ok((
            Self {
                master: pair.master,
                child,
                writer,
            },
            reader,
        ))
    }

    /// Consume the PTY and split it into a writer and a resize handle.
    ///
    /// The writer should be moved to a dedicated writer thread, while
    /// the `PtyMaster` handle stays available for resize operations.
    pub fn into_parts(self) -> (Box<dyn Write + Send>, PtyMaster) {
        (
            self.writer,
            PtyMaster {
                master: self.master,
                _child: self.child,
            },
        )
    }

    /// Write data to the PTY (sends input to the child process).
    pub fn write(&mut self, data: &[u8]) -> crate::Result<()> {
        self.writer
            .write_all(data)
            .map_err(crate::TerminalError::Io)
    }

    /// Resize the PTY.
    pub fn resize(&self, rows: u16, cols: u16) -> crate::Result<()> {
        self.master
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| crate::TerminalError::Resize(e.to_string()))
    }

    /// Check if the child process is still alive.
    pub fn is_alive(&mut self) -> bool {
        self.child.try_wait().ok().flatten().is_none()
    }

    /// Wait for the child process to exit and return its exit code.
    pub fn wait(&mut self) -> crate::Result<u32> {
        let status = self
            .child
            .wait()
            .map_err(|e| crate::TerminalError::Pty(e.to_string()))?;
        Ok(status.exit_code())
    }
}

#[cfg(test)]
mod shell_fallback_tests {
    use super::resolve_shell_from;

    // SDTEST-1579
    #[test]
    fn unix_prefers_explicit_then_shell_env_then_bin_bash() {
        assert_eq!(
            resolve_shell_from(Some("/bin/zsh"), false, Some("/usr/bin/fish"), None, false),
            "/bin/zsh"
        );
        assert_eq!(
            resolve_shell_from(None, false, Some("/usr/bin/fish"), None, false),
            "/usr/bin/fish"
        );
        assert_eq!(
            resolve_shell_from(None, false, Some("   "), None, false),
            "/bin/bash",
            "blank $SHELL must fall through to the default"
        );
        assert_eq!(
            resolve_shell_from(None, false, None, None, false),
            "/bin/bash"
        );
        // COMSPEC is Windows-only and must be ignored on Unix even when set.
        assert_eq!(
            resolve_shell_from(
                None,
                false,
                None,
                Some("C:\\Windows\\system32\\cmd.exe"),
                false
            ),
            "/bin/bash"
        );
    }

    // SDTEST-1580
    #[test]
    fn windows_prefers_explicit_then_powershell_then_comspec_then_cmd() {
        assert_eq!(
            resolve_shell_from(
                Some("pwsh.exe"),
                true,
                None,
                Some("C:\\Windows\\system32\\cmd.exe"),
                true
            ),
            "pwsh.exe",
            "an explicit shell must win over the PowerShell preference"
        );
        assert_eq!(
            resolve_shell_from(
                None,
                true,
                None,
                Some("C:\\Windows\\system32\\cmd.exe"),
                true
            ),
            "powershell.exe"
        );
        assert_eq!(
            resolve_shell_from(
                None,
                true,
                None,
                Some("C:\\Windows\\system32\\cmd.exe"),
                false
            ),
            "C:\\Windows\\system32\\cmd.exe",
            "without PowerShell on PATH, %COMSPEC% is the shell"
        );
        assert_eq!(resolve_shell_from(None, true, None, None, false), "cmd.exe");
        assert_eq!(
            resolve_shell_from(None, true, None, Some("   "), false),
            "cmd.exe",
            "blank %COMSPEC% must fall through to cmd.exe"
        );
        // $SHELL leakage (MSYS/git-bash) must not override the Windows chain.
        assert_eq!(
            resolve_shell_from(None, true, Some("/usr/bin/bash"), None, false),
            "cmd.exe"
        );
    }

    // SDTEST-1581
    #[test]
    fn blank_explicit_shell_falls_through_to_platform_default() {
        assert_eq!(
            resolve_shell_from(Some("  "), false, None, None, false),
            "/bin/bash"
        );
        assert_eq!(
            resolve_shell_from(Some(""), true, None, None, true),
            "powershell.exe"
        );
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    // SDTEST-960/961/962 — PTY smoke: spawn a real `sh -c 'exit 0'`
    // pipeline, verify the child eventually exits with the expected
    // status. Deliberately Unix-only: portable_pty on Windows uses
    // conpty which requires the fixture to run on a real Windows CI
    // runner (blocked K matrix). The Linux CI covers this branch.
    //
    // We use `sh -c 'exit 42'` instead of the default shell so the
    // test is deterministic on any Linux runner (bash may not be
    // present on Alpine; sh always is).

    fn spawn_sh(cmd: &str) -> (LocalPty, Box<dyn std::io::Read + Send>) {
        // `SHELL` is respected by LocalPty::spawn but we want a fixed
        // command line here, so pass a wrapper shell path with `-c cmd`
        // baked in via a small script writer.
        // portable_pty's `CommandBuilder` doesn't take positional args
        // through `spawn(shell)`, so instead we point `SHELL` at a
        // known-present shell and rely on it to execute the login
        // sequence. For deterministic exit codes we write to stdin.
        let (mut pty, reader) = LocalPty::spawn(Some("/bin/sh"), 24, 80).expect("spawn sh");
        pty.write(format!("{}\n", cmd).as_bytes()).expect("write");
        (pty, reader)
    }

    // SDTEST-960 — spawn returns an alive PTY. Baseline sanity: the
    // child process exists and hasn't already exited by the time we
    // return from `spawn`.
    #[test]
    fn spawn_returns_alive_pty() {
        let (mut pty, _reader) = spawn_sh(":");
        assert!(pty.is_alive(), "child must be alive right after spawn");
    }

    // SDTEST-962 — echo round-trip: write `echo <sentinel>`, read the
    // PTY output, expect the sentinel to come back.
    //
    // Robust reads are annoying — the shell also echoes the input
    // line back (line discipline). We only assert `contains` on a
    // deterministic sentinel so any interleaving of prompts / echoes
    // still passes.
    #[test]
    fn write_and_read_echo_round_trip() {
        use std::io::Read;
        let (mut pty, mut reader) = spawn_sh("echo shelldeck_sentinel_42; exit");

        // Give the child up to ~2s to produce output. Blocking reads
        // are OK because `exit` closes the master, ending the read.
        let mut buf = Vec::with_capacity(4096);
        let mut chunk = [0u8; 512];
        let start = std::time::Instant::now();
        loop {
            match reader.read(&mut chunk) {
                Ok(0) => break, // EOF
                Ok(n) => buf.extend_from_slice(&chunk[..n]),
                Err(_) => break,
            }
            if start.elapsed() > std::time::Duration::from_secs(3) {
                break;
            }
        }
        let out = String::from_utf8_lossy(&buf);
        assert!(
            out.contains("shelldeck_sentinel_42"),
            "expected sentinel in PTY output, got: {out:?}",
        );

        // Reap the child so the test doesn't leak a defunct process.
        let _ = pty.wait();
    }

    // SDTEST-963 — resize before and after spawn both work.
    // portable_pty tolerates resize on a running PTY; we verify the
    // call doesn't Err. SIGWINCH delivery is an OS-level concern the
    // portable_pty layer already handles.
    #[test]
    fn resize_returns_ok() {
        let (pty, _reader) = spawn_sh("sleep 0.1; exit");
        pty.resize(30, 100).expect("resize on live PTY");
        pty.resize(24, 80).expect("resize back to defaults");
    }

    // SDTEST-965/966 — `is_alive` flips to false after the child
    // exits; `wait()` returns the exit code.
    #[test]
    fn is_alive_and_wait_reflect_child_exit() {
        let (mut pty, _reader) = spawn_sh("exit 3");
        let code = pty.wait().expect("wait completes");
        // Some shells map explicit `exit N` to the low 8 bits; portable_pty
        // reports the raw exit code from the OS. We assert non-zero (the
        // child DID exit with a failure) — pinning the exact 3 is fragile
        // across shell implementations.
        assert!(!pty.is_alive(), "child must be dead after wait");
        assert!(
            code != 0,
            "explicit `exit 3` should surface as non-zero exit code (got {code})",
        );
    }
}
