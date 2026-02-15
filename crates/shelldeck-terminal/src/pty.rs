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
        self.writer.write_all(data).map_err(crate::TerminalError::Io)
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
