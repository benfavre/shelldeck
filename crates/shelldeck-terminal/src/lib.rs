pub mod attached;
pub mod boundary;
pub mod colors;
pub mod error;
pub mod grid;
pub mod parser;
pub mod pty;
pub mod session;
pub mod url;

pub use error::{Result, TerminalError};

/// Revision of the emulator behaviour: bump it whenever a change to the
/// parser or the grid alters grid state for the same input bytes. The
/// session daemon announces it so a client built from another revision
/// knows its own grid may drift and refreshes from the daemon's.
///
/// 2: a resize that merges soft-wrapped lines keeps the row count (blank
/// rows at the bottom) instead of leaving a short screen.
pub const EMULATOR_REV: u16 = 2;
