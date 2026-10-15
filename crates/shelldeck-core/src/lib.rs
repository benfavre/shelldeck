pub mod acp;
pub mod agent_runtime;
pub mod agent_workflows;
pub mod agent_delivery;
pub mod agent_codex;
pub mod agent_browser;
pub mod agent_review;
pub mod agent_session;
pub mod ai;
pub mod companion;
pub mod config;
pub mod error;
pub mod git;
pub mod models;
pub mod tmux;
pub mod util;
pub mod workspace_navigation;
pub mod workspace_review;

pub use error::{Result, ShellDeckError};
pub use models::*;

/// Application version, resolved at compile time from the workspace Cargo.toml.
/// Use this constant everywhere instead of calling `env!("CARGO_PKG_VERSION")` directly.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
