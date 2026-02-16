use crate::grid::TerminalGrid;
use crate::parser::TerminalProcessor;
use crate::pty::LocalPty;
use chrono::{DateTime, Utc};
use parking_lot::Mutex;
use std::io::{Read, Write};
use std::sync::Arc;
use tokio::sync::mpsc;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionState {
    Running,
    Exited(i32),
    Error(String),
}

/// Callback to resize the underlying PTY or SSH channel.
type ResizeFn = Box<dyn Fn(u16, u16) + Send>;

pub struct TerminalSession {
    pub id: Uuid,
    pub title: String,
    pub created_at: DateTime<Utc>,
    pub state: SessionState,
    pub grid: Arc<Mutex<TerminalGrid>>,
    input_tx: mpsc::UnboundedSender<Vec<u8>>,
    resize_fn: Option<ResizeFn>,
}

impl TerminalSession {
    /// Spawn a new local terminal session.
    pub fn spawn_local(shell: Option<&str>, rows: u16, cols: u16) -> crate::Result<Self> {
        let grid = Arc::new(Mutex::new(TerminalGrid::new(rows as usize, cols as usize)));
        let (input_tx, mut input_rx) = mpsc::unbounded_channel::<Vec<u8>>();

        // Create a response channel so the VTE parser can send responses
        // (e.g., DSR cursor position, DA reports) back to the PTY.
        let (response_tx, response_rx) = std::sync::mpsc::channel::<Vec<u8>>();
        grid.lock().set_response_tx(response_tx);

        let (pty, reader) = LocalPty::spawn(shell, rows, cols)?;

        // Split PTY: writer goes to the writer thread, master stays for resize.
        let (mut writer, master) = pty.into_parts();

        let grid_clone = grid.clone();
        let response_input_tx = input_tx.clone();

        // Spawn reader thread: reads PTY output and feeds to VTE parser.
        // Also drains any pending responses from the parser and forwards
        // them to the writer thread via input_tx.
        std::thread::Builder::new()
            .name("pty-reader".into())
            .spawn(move || {
                let mut parser = vte::Parser::new();
                let mut processor = TerminalProcessor::new(grid_clone);
                let mut reader = reader;
                let mut buf = [0u8; 4096];

                loop {
                    match reader.read(&mut buf) {
                        Ok(0) => break,
                        Ok(n) => {
                            processor.process_bytes(&mut parser, &buf[..n]);
                            // Drain any responses queued by the parser (DSR, DA, etc.)
                            // and forward them to the PTY writer.
                            while let Ok(response) = response_rx.try_recv() {
                                let _ = response_input_tx.send(response);
                            }
                        }
                        Err(e) => {
                            tracing::debug!("PTY reader error: {}", e);
                            break;
                        }
                    }
                }
                tracing::debug!("PTY reader thread exiting");
            })
            .map_err(|e| {
                crate::TerminalError::Pty(format!("Failed to spawn reader thread: {}", e))
            })?;

        // Spawn writer thread: forwards input to PTY.
        std::thread::Builder::new()
            .name("pty-writer".into())
            .spawn(move || {
                while let Some(data) = input_rx.blocking_recv() {
                    if writer.write_all(&data).is_err() {
                        break;
                    }
                }
                tracing::debug!("PTY writer thread exiting");
            })
            .map_err(|e| {
                crate::TerminalError::Pty(format!("Failed to spawn writer thread: {}", e))
            })?;

        // Build a resize callback that resizes the underlying PTY.
        // The master handle is Send so this closure can live in the session.
        let resize_fn: ResizeFn = Box::new(move |rows, cols| {
            if let Err(e) = master.resize(rows, cols) {
                tracing::warn!("PTY resize failed: {}", e);
            }
        });

        Ok(Self {
            id: Uuid::new_v4(),
            title: "Terminal".to_string(),
            created_at: Utc::now(),
            state: SessionState::Running,
            grid,
            input_tx,
            resize_fn: Some(resize_fn),
        })
    }

    /// Spawn a new SSH terminal session.
    ///
    /// Unlike `spawn_local`, this does not create a PTY. Instead it returns
    /// channel endpoints that the caller wires to an SSH channel:
    ///
    /// - `data_tx` (`UnboundedSender<Vec<u8>>`): the caller pushes SSH
    ///   channel stdout into this sender; a background thread drains it
    ///   through the VTE parser into the grid.
    /// - `input_rx` (`UnboundedReceiver<Vec<u8>>`): the caller reads
    ///   keyboard data from this receiver and forwards it to the SSH
    ///   channel's stdin.
    pub fn spawn_ssh(
        title: String,
        rows: u16,
        cols: u16,
    ) -> (
        Self,
        mpsc::UnboundedSender<Vec<u8>>,
        mpsc::UnboundedReceiver<Vec<u8>>,
    ) {
        let grid = Arc::new(Mutex::new(TerminalGrid::new(rows as usize, cols as usize)));
        let (input_tx, input_rx) = mpsc::unbounded_channel::<Vec<u8>>();
        let (data_tx, mut data_rx) = mpsc::unbounded_channel::<Vec<u8>>();

        // Create a response channel so the VTE parser can send responses
        // (e.g., DSR cursor position, DA reports) back to the SSH channel.
        let (response_tx, response_rx) = std::sync::mpsc::channel::<Vec<u8>>();
        grid.lock().set_response_tx(response_tx);

        let grid_clone = grid.clone();
        let response_input_tx = input_tx.clone();

        // Spawn reader thread: receives SSH channel data and feeds to VTE parser → grid.
        // Also drains any pending responses from the parser and forwards
        // them via input_tx back to the SSH channel's stdin.
        std::thread::Builder::new()
            .name("ssh-reader".into())
            .spawn(move || {
                let mut parser = vte::Parser::new();
                let mut processor = TerminalProcessor::new(grid_clone);

                while let Some(data) = data_rx.blocking_recv() {
                    processor.process_bytes(&mut parser, &data);
                    // Drain any responses queued by the parser (DSR, DA, etc.)
                    // and forward them to the SSH channel's stdin.
                    while let Ok(response) = response_rx.try_recv() {
                        let _ = response_input_tx.send(response);
                    }
                }
                tracing::debug!("SSH reader thread exiting");
            })
            .expect("Failed to spawn SSH reader thread");

        let session = Self {
            id: Uuid::new_v4(),
            title,
            created_at: Utc::now(),
            state: SessionState::Running,
            grid,
            input_tx,
            resize_fn: None,
        };

        (session, data_tx, input_rx)
    }

    /// Send input data to the terminal (e.g., keyboard input).
    pub fn write_input(&self, data: &[u8]) {
        let _ = self.input_tx.send(data.to_vec());
    }

    /// Resize the terminal grid and underlying PTY / SSH channel.
    pub fn resize(&self, rows: u16, cols: u16) {
        self.grid.lock().resize(rows as usize, cols as usize);
        if let Some(ref resize_fn) = self.resize_fn {
            resize_fn(rows, cols);
        }
    }

    /// Set the resize callback (e.g. for SSH sessions where the resize
    /// channel is set up after session creation).
    pub fn set_resize_fn(&mut self, f: ResizeFn) {
        self.resize_fn = Some(f);
    }

    /// Check if the session is still running.
    pub fn is_running(&self) -> bool {
        self.state == SessionState::Running
    }
}
