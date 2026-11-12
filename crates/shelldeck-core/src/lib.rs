pub mod acp;
pub mod agent_browser;
pub mod agent_browser_tools;
pub mod agent_codex;
pub mod agent_delivery;
pub mod agent_local_sessions;
pub mod agent_remote_history;
pub mod agent_remote_sessions;
pub mod agent_review;
pub mod agent_runtime;
pub mod agent_session;
pub mod agent_workflows;
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

/// Public product name. Persistent storage, keychain and protocol identifiers
/// intentionally retain their original names so existing installations upgrade.
pub const APP_NAME: &str = "Inklura Desktop";
pub const AGENTS_APP_NAME: &str = "Inklura Desktop Agents";
pub const TERMINAL_APP_NAME: &str = "Inklura Desktop Terminal";
pub const EDITOR_APP_NAME: &str = "Inklura Desktop Editor";
