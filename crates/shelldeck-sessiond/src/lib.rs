//! ShellDeck session daemon: a per-user server that owns the PTYs, one per
//! pane, keeps the authoritative screen state and forwards the raw byte
//! stream to clients, which parse and render it themselves.
//!
//! Design: `docs/design/session-daemon.md`.
//!
//! - [`protocol`]: wire protocol v1 (frames, limits, bounded decoder).
//! - [`client`]: async client over any byte stream, plus a headless
//!   client-side emulator ([`client::PaneMirror`]).
//! - [`server`], [`proxy`], [`paths`]: the daemon itself, the stdio proxy
//!   used over SSH and the run directory checks. Unix only.
//! - [`attached`]: synchronous facade for the app: connects to (and starts)
//!   the local daemon and turns a pane into a
//!   `shelldeck_terminal::session::TerminalSession`. Unix only.
//! - [`supervisor`]: what the app uses on top of it: sessions that are
//!   returned at once, connect and attach on their own thread, and
//!   reattach after the daemon connection is lost. Unix only.
//!
//! [`attached`] and [`supervisor`] need the `app` feature (on by default),
//! which links `shelldeck-core` through the terminal crate. The daemon
//! binary is built without it.

pub mod client;
pub mod pane_cli;
pub mod protocol;
pub mod requests;

#[cfg(all(unix, feature = "app"))]
pub mod attached;
#[cfg(unix)]
pub mod paths;
#[cfg(unix)]
pub mod proxy;
#[cfg(unix)]
pub mod server;
#[cfg(all(unix, feature = "app"))]
pub mod supervisor;

/// Line every build of the daemon binary carries, so a file can be tied
/// to the release it was built for without running it: a client checks
/// for [`build_marker`] of its own version before it uploads a bundled
/// build to a server under that version's name. `--version` prints it
/// without its first four bytes.
pub const BUILD_MARKER: &str = concat!("@(#)shelldeck-sessiond ", env!("CARGO_PKG_VERSION"), "\n");

/// The [`BUILD_MARKER`] of the build of `version`.
pub fn build_marker(version: &str) -> String {
    format!("@(#)shelldeck-sessiond {version}\n")
}
