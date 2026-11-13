#[cfg(feature = "app")]
pub mod attached;
pub mod boundary;
pub mod colors;
pub mod error;
pub mod grid;
pub mod parser;
pub mod program_status;
pub mod pty;
#[cfg(feature = "app")]
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
/// 3: a screen that loses rows gives up the blank ones under the cursor
/// before it moves rows from the top to the history.
pub const EMULATOR_REV: u16 = 3;
