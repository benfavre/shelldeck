pub mod config;
pub mod error;
pub mod git;
pub mod models;

pub use error::{Result, ShellDeckError};
pub use models::*;

/// Application version, resolved at compile time from the workspace Cargo.toml.
/// Use this constant everywhere instead of calling `env!("CARGO_PKG_VERSION")` directly.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
