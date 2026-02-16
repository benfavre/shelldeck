use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ConnectionSource {
    SshConfig,
    Manual,
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
