//! Client side of a pane owned by the session daemon.
//!
//! An [`AttachedFeed`] applies what the daemon sends for one pane (a
//! snapshot, then the raw byte stream, applied resizes and history chunks)
//! to the grid of a [`TerminalSession`](crate::session::TerminalSession)
//! created by `spawn_attached`. It is transport agnostic: the
//! `shelldeck-sessiond` client library drives it.
//!
//! Two rules make this backend different from the local PTY and SSH ones:
//!
//! - the grid never answers terminal queries (no `response_tx`): the
//!   daemon's grid is the only one that does, otherwise the application
//!   would receive one reply per attached client;
//! - the grid is never resized from the view: it follows the size the
//!   daemon applied, at the exact position in the byte stream.

use crate::grid::{SnapshotError, TerminalGrid};
use crate::parser::TerminalProcessor;
use parking_lot::Mutex;
use std::sync::Arc;
use tokio::sync::mpsc;

/// What the driver must do after an applied resize.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttachedOutcome {
    Applied,
    /// The resize cannot be reproduced locally (less history than the
    /// daemon pulled back on screen, or another emulator revision): the
    /// feed ignores everything until the next [`AttachedFeed::restore`].
    NeedsReattach,
}

/// Applies the daemon's view of one pane to the session grid, in order.
pub struct AttachedFeed {
    grid: Arc<Mutex<TerminalGrid>>,
    processor: TerminalProcessor,
    parser: vte::Parser,
    notifier: Arc<Mutex<Option<mpsc::UnboundedSender<()>>>>,
    same_emulator: bool,
    /// Out of step with the daemon until the next restore.
    stale: bool,
}

impl AttachedFeed {
    pub(crate) fn new(
        grid: Arc<Mutex<TerminalGrid>>,
        notifier: Arc<Mutex<Option<mpsc::UnboundedSender<()>>>>,
        daemon_emulator_rev: u16,
    ) -> Self {
        Self {
            processor: TerminalProcessor::new(grid.clone()),
            grid,
            parser: vte::Parser::new(),
            notifier,
            same_emulator: daemon_emulator_rev == crate::EMULATOR_REV,
            stale: true,
        }
    }

    /// Whether the grid reflects the daemon's stream.
    pub fn in_sync(&self) -> bool {
        !self.stale
    }

    /// Replace the screen from a daemon snapshot. Unless `keep_history`,
    /// the local history is dropped. The scroll position stays local.
    pub fn restore(&mut self, snapshot: &[u8], keep_history: bool) -> Result<(), SnapshotError> {
        let tail = self.grid.lock().restore(snapshot, keep_history)?;
        // A fresh parser starts at ground; the tail puts it where the
        // daemon's parser was when the snapshot was taken.
        self.parser = vte::Parser::new();
        self.processor.process_bytes(&mut self.parser, &tail);
        self.stale = false;
        self.wake();
        Ok(())
    }

    /// Raw pane output, contiguous with everything applied before.
    pub fn bytes(&mut self, data: &[u8]) {
        if self.stale {
            return;
        }
        self.processor.process_bytes(&mut self.parser, data);
        self.wake();
    }

    /// The daemon resized the pane at this point of the stream. `pulled`
    /// is the number of history rows its grid moved back on screen.
    pub fn resize(&mut self, rows: u16, cols: u16, pulled: u16) -> AttachedOutcome {
        if self.stale {
            return AttachedOutcome::Applied;
        }
        {
            let mut grid = self.grid.lock();
            if !self.same_emulator || grid.scrollback_len() < pulled as usize {
                self.stale = true;
                return AttachedOutcome::NeedsReattach;
            }
            grid.resize(rows as usize, cols as usize);
        }
        self.wake();
        AttachedOutcome::Applied
    }

    /// One history chunk, newest first, inserted at the old end.
    pub fn history(&mut self, chunk: &[u8]) -> Result<(), SnapshotError> {
        if self.stale {
            return Ok(());
        }
        self.grid.lock().prepend_history(chunk)?;
        self.wake();
        Ok(())
    }

    /// The stream lost sync: ignore everything until the next restore and
    /// keep showing the last screen meanwhile.
    pub fn mark_stale(&mut self) {
        self.stale = true;
    }

    /// Text shown by the client only (for example "connection lost"). It
    /// is not part of the pane's stream, so the feed goes stale.
    pub fn local_notice(&mut self, text: &str) {
        let mut line = Vec::with_capacity(text.len() + 16);
        line.extend_from_slice(b"\x1b[0m\r\n");
        line.extend_from_slice(text.as_bytes());
        line.extend_from_slice(b"\r\n");
        self.parser = vte::Parser::new();
        self.processor.process_bytes(&mut self.parser, &line);
        self.stale = true;
        self.wake();
    }

    /// Wake the UI to repaint (event-driven, no polling).
    pub fn wake(&self) {
        if let Some(tx) = self.notifier.lock().as_ref() {
            let _ = tx.send(());
        }
    }
}
