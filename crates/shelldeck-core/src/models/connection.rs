use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ConnectionSource {
    SshConfig,
    Manual,
    /// Pulled from the Inklura Manage cloud-sync endpoint. These are managed by
    /// the sync process: refreshed on every sync and removed when they disappear
    /// from the remote set. See `config::cloud_sync`.
    CloudSync,
}

impl ConnectionSource {
    /// Short, human-friendly label for badges and UI.
    pub fn label(&self) -> &'static str {
        match self {
            ConnectionSource::SshConfig => "ssh config",
            ConnectionSource::Manual => "manual",
            ConnectionSource::CloudSync => "cloud",
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum ConnectionStatus {
    #[default]
    Disconnected,
    Connecting,
    Connected,
    Error(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Connection {
    pub id: Uuid,
    pub alias: String,
    pub hostname: String,
    pub port: u16,
    pub user: String,
    pub identity_file: Option<PathBuf>,
    pub proxy_jump: Option<String>,
    pub group: Option<String>,
    pub tags: Vec<String>,
    pub auto_forwards: Vec<Uuid>,
    pub auto_scripts: Vec<Uuid>,
    pub source: ConnectionSource,
    pub forward_agent: bool,
    /// Inklura Manage site this connection is bound to (cloud-synced profiles
    /// only). `#[serde(default)]` keeps pre-site stores parsing.
    #[serde(default)]
    pub site_id: Option<Uuid>,
    /// Human-friendly site label for the sidebar badge / site grouping.
    #[serde(default)]
    pub site_label: Option<String>,
    #[serde(skip)]
    pub status: ConnectionStatus,
}

impl Connection {
    pub fn new_manual(alias: String, hostname: String, user: String) -> Self {
        Self {
            id: Uuid::new_v4(),
            alias,
            hostname,
            port: 22,
            user,
            identity_file: None,
            proxy_jump: None,
            group: None,
            tags: Vec::new(),
            auto_forwards: Vec::new(),
            auto_scripts: Vec::new(),
            source: ConnectionSource::Manual,
            forward_agent: false,
            site_id: None,
            site_label: None,
            status: ConnectionStatus::Disconnected,
        }
    }

    pub fn display_name(&self) -> &str {
        if self.alias.is_empty() {
            &self.hostname
        } else {
            &self.alias
        }
    }

    pub fn connection_string(&self) -> String {
        format!("{}@{}:{}", self.user, self.hostname, self.port)
    }
}

#[cfg(test)]
mod tests {
    use super::Connection;

    // SDTEST-032 — display_name is what every sidebar row shows; must
    // never render empty. Contract: alias wins if non-empty, else
    // hostname. There is NO fallback to UUID today (contrary to my
    // initial SDUC-104 — corrected in USE_CASES.md).
    #[test]
    fn display_name_prefers_alias_falls_back_to_hostname() {
        let c = Connection::new_manual("myserver".into(), "10.0.0.1".into(), "alice".into());
        assert_eq!(c.display_name(), "myserver");

        let anon = Connection::new_manual("".into(), "prod.example".into(), "root".into());
        assert_eq!(anon.display_name(), "prod.example");
    }

    // display_name returns a borrowed slice — no allocation. Regression
    // sensor if someone refactors to `String` and starts allocating on
    // every sidebar row paint (perf regression class).
    #[test]
    fn display_name_returns_borrowed_slice() {
        let c = Connection::new_manual("h".into(), "".into(), "u".into());
        let name: &str = c.display_name();
        // Both `alias.as_str()` and `hostname.as_str()` must point INTO
        // `self`; the address comparison proves it.
        assert!(std::ptr::eq(name.as_ptr(), c.alias.as_ptr()));
    }

    // SDTEST-033 — connection_string shape is `user@host:port`. Port
    // is ALWAYS included, even when it's the default 22 (contract
    // choice — the caller can strip it if they want, but this fn is
    // opinionated toward unambiguous strings).
    #[test]
    fn connection_string_always_includes_port() {
        let c = Connection::new_manual("s".into(), "h.example".into(), "root".into());
        assert_eq!(c.connection_string(), "root@h.example:22");

        let mut custom = Connection::new_manual("s".into(), "h.example".into(), "u".into());
        custom.port = 2222;
        assert_eq!(custom.connection_string(), "u@h.example:2222");
    }

    #[test]
    fn new_manual_sets_manual_source_and_default_port() {
        let c = Connection::new_manual("a".into(), "h".into(), "u".into());
        assert!(matches!(c.source, super::ConnectionSource::Manual));
        assert_eq!(c.port, 22);
        assert!(c.identity_file.is_none());
        assert!(c.site_id.is_none());
    }
}
