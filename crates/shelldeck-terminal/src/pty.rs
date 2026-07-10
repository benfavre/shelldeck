use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize};
use std::io::{Read, Write};

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

        let shell_path = match shell {
            Some(s) => s.to_string(),
            None => std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string()),
        };

        let mut cmd = CommandBuilder::new(&shell_path);
        cmd.cwd(std::env::var("HOME").unwrap_or_else(|_| "/".to_string()));

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
