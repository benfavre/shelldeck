//! ShellDeck's isolated Chromium helper transport and scoped computer-use bridge.
//! The UI owns the client; providers receive revocable access only to granted tabs.

pub mod client;
pub mod mcp;
pub mod protocol;
#[cfg(feature = "runtime")]
pub mod runtime;

pub use client::{
    discover_helper, discover_helper_near, BrowserClient, BrowserFrame, BrowserLaunch,
};
pub use protocol::*;
