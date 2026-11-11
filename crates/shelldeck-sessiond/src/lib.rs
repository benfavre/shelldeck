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

pub mod client;
pub mod protocol;

#[cfg(unix)]
pub mod attached;
#[cfg(unix)]
pub mod paths;
#[cfg(unix)]
pub mod proxy;
#[cfg(unix)]
pub mod server;
