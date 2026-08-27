use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize};
use std::io::{Read, Write};
use std::time::{Duration, Instant};

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

/// How long a hung-up child may take to exit on its own before it is killed.
const REAP_GRACE: Duration = Duration::from_secs(2);

/// Owns the PTY child process and guarantees it is reaped.
///
/// On Unix `portable_pty`'s child *is* `std::process::Child`, whose `Drop`
/// neither kills nor waits. Closing a terminal therefore left one zombie per
/// tab for the whole lifetime of the app (reproduced: state `Z` three seconds
/// after the PTY was dropped).
///
/// **Contract.** Dropping this hangs up the terminal and reaps the child, in
/// that order:
///
/// 1. The master file descriptor is closed first — that is what raises SIGHUP
///    on the child's controlling terminal. Field declaration order is what
///    guarantees it, so do not reorder the fields of [`LocalPty`] or
///    [`PtyMaster`].
/// 2. The child then gets [`REAP_GRACE`] to exit on its own. A shell answering
///    SIGHUP uses that window to flush its history and run its exit traps —
///    which is why this is not an immediate `kill`.
/// 3. Only a child still alive after the grace period is killed, then reaped.
///
/// Reaping runs on a detached thread, so dropping a terminal never blocks the
/// thread that closed it.
struct ChildReaper {
    child: Option<Box<dyn portable_pty::Child + Send + Sync>>,
    /// Reports which branch the reap took: `false` when the child exited on
    /// its own inside the grace period, `true` when it had to be killed.
    /// Timing alone cannot tell those apart — an immediate kill also looks
    /// fast — so the tests observe the branch instead of the clock.
    #[cfg(test)]
    reaped_by_kill: Option<std::sync::mpsc::Sender<bool>>,
}

impl ChildReaper {
    fn new(child: Box<dyn portable_pty::Child + Send + Sync>) -> Self {
        Self {
            child: Some(child),
            #[cfg(test)]
            reaped_by_kill: None,
        }
    }

    #[cfg(test)]
    fn observe_reap(&mut self, tx: std::sync::mpsc::Sender<bool>) {
        self.reaped_by_kill = Some(tx);
    }

    /// `true` while the child has neither exited nor been reaped.
    fn is_alive(&mut self) -> bool {
        match self.child.as_mut() {
            Some(child) => child.try_wait().ok().flatten().is_none(),
            None => false,
        }
    }

    fn wait(&mut self) -> crate::Result<u32> {
        let child = self
            .child
            .as_mut()
            .ok_or_else(|| crate::TerminalError::Pty("PTY child already reaped".into()))?;
        let status = child
            .wait()
            .map_err(|e| crate::TerminalError::Pty(e.to_string()))?;
        Ok(status.exit_code())
    }

    #[cfg(test)]
    fn process_id(&self) -> Option<u32> {
        self.child.as_ref().and_then(|child| child.process_id())
    }
}

impl Drop for ChildReaper {
    fn drop(&mut self) {
        let Some(mut child) = self.child.take() else {
            return;
        };

        #[cfg(test)]
        let observer = self.reaped_by_kill.take();
        #[cfg(test)]
        let report_clean_exit = || {
            if let Some(tx) = observer.as_ref() {
                let _ = tx.send(false);
            }
        };
        #[cfg(not(test))]
        let report_clean_exit = || {};

        // The child may already be gone. An earlier `wait` / `is_alive` may
        // have reaped it — but the common case is that the writer's EOF made
        // the shell exit before this drop ran, which is a race the dev machine
        // usually loses and a loaded CI runner usually wins. `try_wait` reaps
        // it either way, so the contract is met and there is nothing to
        // escalate.
        if matches!(child.try_wait(), Ok(Some(_))) {
            report_clean_exit();
            return;
        }

        let spawned = std::thread::Builder::new()
            .name("pty-reaper".into())
            .spawn(move || {
                #[cfg(test)]
                let report = |killed: bool| {
                    if let Some(tx) = observer.as_ref() {
                        let _ = tx.send(killed);
                    }
                };
                #[cfg(not(test))]
                let report = |_killed: bool| {};

                let deadline = Instant::now() + REAP_GRACE;
                loop {
                    match child.try_wait() {
                        Ok(Some(_)) => return report(false),
                        Ok(None) => {}
                        // The child is unobservable; killing it would be a
                        // guess, and waiting forever would leak this thread.
                        Err(_) => return,
                    }
                    if Instant::now() >= deadline {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(20));
                }
                let _ = child.kill();
                let _ = child.wait();
                report(true);
            });

        if let Err(e) = spawned {
            // The child moved into the closure, so it is gone with it. Doing
            // the reap inline instead would block whoever closed the terminal
            // for the whole grace period — the wrong trade for a failure that
            // only happens when the process can no longer spawn threads at all.
            tracing::warn!("Failed to spawn PTY reaper thread, child left unreaped: {e}");
        }
    }
}

pub struct LocalPty {
    // Drop order is load-bearing, see `ChildReaper`: the writer sends EOF, then
    // the master fd closes and hangs up the terminal, then the child is reaped.
    writer: Box<dyn Write + Send>,
    master: Box<dyn MasterPty + Send>,
    child: ChildReaper,
}

/// Handle to the PTY master, used for resize operations after the
/// writer has been split off to a dedicated thread.
pub struct PtyMaster {
    // Declared before the child so the hangup precedes the reap — see
    // `ChildReaper`. The child is held for its `Drop`, never read, hence the
    // underscore.
    master: Box<dyn MasterPty + Send>,
    _child: ChildReaper,
}

impl PtyMaster {
    #[cfg(test)]
    fn observe_reap(&mut self, tx: std::sync::mpsc::Sender<bool>) {
        self._child.observe_reap(tx);
    }

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
        let home =
            shelldeck_core::util::home_dir().unwrap_or_else(|| std::path::PathBuf::from("."));
        Self::spawn_at(shell, rows, cols, &home)
    }

    /// Spawn a PTY rooted at a caller-authorized existing directory.
    pub fn spawn_at(
        shell: Option<&str>,
        rows: u16,
        cols: u16,
        cwd: &std::path::Path,
    ) -> crate::Result<(Self, Box<dyn Read + Send>)> {
        if !cwd.is_dir() {
            return Err(crate::TerminalError::Pty(format!(
                "working directory `{}` is unavailable",
                cwd.display()
            )));
        }

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
        // The caller resolved this working directory from its authority
        // boundary; the PTY boundary independently enforces availability.
        cmd.cwd(cwd);

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
                writer,
                master: pair.master,
                child: ChildReaper::new(child),
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
        self.child.is_alive()
    }

    /// Wait for the child process to exit and return its exit code.
    pub fn wait(&mut self) -> crate::Result<u32> {
        self.child.wait()
    }
}

#[cfg(test)]
mod shell_fallback_tests {
    use super::{resolve_shell_from, LocalPty};

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

    #[test]
    fn missing_cwd_is_rejected_at_the_pty_boundary() {
        let missing = std::env::temp_dir().join(format!(
            "shelldeck-missing-pty-cwd-{}",
            uuid::Uuid::new_v4()
        ));
        assert!(!missing.exists());
        assert!(
            LocalPty::spawn_at(None, 24, 80, &missing).is_err(),
            "an explicit missing cwd must never fall back to the user home"
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

    /// Linux-only: `/proc/<pid>/stat` is what distinguishes *reaped* from
    /// *zombie*. A dead-but-unreaped child still has a `/proc` entry, so
    /// "the process is gone" is not the same question as "is_alive() is false".
    ///
    /// The state letter is the field right after the comm field, which is
    /// parenthesised and may itself contain spaces — hence the split on the
    /// last `)` rather than on whitespace.
    #[cfg(target_os = "linux")]
    fn proc_state(pid: u32) -> Option<char> {
        let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
        let after_comm = stat.rsplit_once(')')?.1;
        after_comm.split_whitespace().next()?.chars().next()
    }

    /// Wait for `pid` to disappear from the process table entirely, returning
    /// how long that took. A lingering `Z` is a failure, not a pass.
    #[cfg(target_os = "linux")]
    fn time_until_reaped(pid: u32, limit: std::time::Duration) -> Option<std::time::Duration> {
        let start = Instant::now();
        loop {
            match proc_state(pid) {
                None => return Some(start.elapsed()),
                Some(_) if start.elapsed() >= limit => return None,
                Some(_) => std::thread::sleep(Duration::from_millis(10)),
            }
        }
    }

    // SDTEST-967 — dropping the PTY hangs up the terminal and reaps the child.
    //
    // Before `ChildReaper`, this left one zombie per closed terminal tab for
    // the lifetime of the app: the shell did exit on SIGHUP, but nothing ever
    // called `waitpid`, so it stayed in state `Z`.
    #[cfg(target_os = "linux")]
    #[test]
    fn dropping_the_pty_hangs_up_and_reaps_the_child() {
        let (pty, _reader) = LocalPty::spawn(Some("/bin/sh"), 24, 80).expect("spawn sh");
        let pid = pty.child.process_id().expect("child pid");
        assert!(
            proc_state(pid).is_some(),
            "child must exist right after spawn",
        );

        let (writer, mut master) = pty.into_parts();
        let (reap_tx, reap_rx) = std::sync::mpsc::channel();
        master.observe_reap(reap_tx);
        drop(writer);
        drop(master);

        time_until_reaped(pid, REAP_GRACE * 3).unwrap_or_else(|| {
            panic!("child {pid} was never reaped (state {:?})", proc_state(pid))
        });

        // The hangup did the work and the shell exited on its own terms. That
        // distinction is the whole reason the grace period exists: a kill here
        // would cost the user their shell history and exit traps.
        assert!(
            !reap_rx
                .recv_timeout(REAP_GRACE)
                .expect("reaper never reported"),
            "child was killed instead of hanging up cleanly",
        );
    }

    // SDTEST-969 — a child that never exits on its own is killed once the
    // grace period is spent, and reaped. Driven directly through `ChildReaper`
    // with a plain `std::process::Child`: no PTY means no SIGHUP, so only the
    // escalation can end this process.
    #[cfg(target_os = "linux")]
    #[test]
    fn a_child_that_ignores_the_hangup_is_killed_after_the_grace_period() {
        let child = std::process::Command::new("sleep")
            .arg("120")
            .spawn()
            .expect("spawn sleep");
        let pid = child.id();
        let mut reaper = ChildReaper::new(Box::new(child));
        let (reap_tx, reap_rx) = std::sync::mpsc::channel();
        reaper.observe_reap(reap_tx);

        let start = Instant::now();
        drop(reaper);

        assert!(
            reap_rx
                .recv_timeout(REAP_GRACE * 3)
                .expect("reaper never reported"),
            "a child that never exits must reach the kill branch",
        );
        assert!(
            start.elapsed() >= REAP_GRACE,
            "the grace period was skipped — killed after only {:?}",
            start.elapsed(),
        );
        time_until_reaped(pid, REAP_GRACE).unwrap_or_else(|| {
            panic!(
                "child {pid} survived the reaper (state {:?})",
                proc_state(pid)
            )
        });
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
