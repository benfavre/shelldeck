use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ForwardDirection {
    LocalToRemote,
    RemoteToLocal,
    Dynamic,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum ForwardStatus {
    #[default]
    Inactive,
    Active,
    Error,
    Stopping,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PortForward {
    pub id: Uuid,
    pub connection_id: Uuid,
    pub direction: ForwardDirection,
    pub local_host: String,
    pub local_port: u16,
    pub remote_host: String,
    pub remote_port: u16,
    pub auto_start: bool,
    pub label: Option<String>,
    #[serde(skip)]
    pub status: ForwardStatus,
    #[serde(skip)]
    pub bytes_sent: u64,
    #[serde(skip)]
    pub bytes_received: u64,
}

impl PortForward {
    pub fn new_local(
        connection_id: Uuid,
        local_port: u16,
        remote_host: &str,
        remote_port: u16,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            connection_id,
            direction: ForwardDirection::LocalToRemote,
            local_host: "127.0.0.1".to_string(),
            local_port,
            remote_host: remote_host.to_string(),
            remote_port,
            auto_start: false,
            label: None,
            status: ForwardStatus::Inactive,
            bytes_sent: 0,
            bytes_received: 0,
        }
    }

    pub fn new_remote(
        connection_id: Uuid,
        remote_port: u16,
        local_host: &str,
        local_port: u16,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            connection_id,
            direction: ForwardDirection::RemoteToLocal,
            local_host: local_host.to_string(),
            local_port,
            remote_host: "127.0.0.1".to_string(),
            remote_port,
            auto_start: false,
            label: None,
            status: ForwardStatus::Inactive,
            bytes_sent: 0,
            bytes_received: 0,
        }
    }

    /// Preset: Chrome DevTools -> Remote
    pub fn chrome_devtools_preset(connection_id: Uuid) -> Self {
        let mut fwd = Self::new_remote(connection_id, 9222, "127.0.0.1", 9222);
        fwd.label = Some("Chrome DevTools -> Remote".to_string());
        fwd
    }

    /// Preset: Remote Web Server -> Local
    pub fn web_server_preset(connection_id: Uuid, remote_port: u16) -> Self {
        let mut fwd = Self::new_local(connection_id, remote_port, "localhost", remote_port);
        fwd.label = Some(format!("Remote :{} -> Local", remote_port));
        fwd
    }

    /// Preset: opencode web -> Local (default port from `opencode web --port 4096`)
    pub fn opencode_preset(connection_id: Uuid) -> Self {
        let mut fwd = Self::new_local(connection_id, 4096, "localhost", 4096);
        fwd.label = Some("OpenCode Web -> Local".to_string());
        fwd
    }

    /// Preset: Dev Server -> Local (remote :3060 forwarded to local :3060)
    pub fn dev_server_preset(connection_id: Uuid) -> Self {
        let mut fwd = Self::new_local(connection_id, 3060, "localhost", 3060);
        fwd.label = Some("Dev Server -> Local".to_string());
        fwd
    }

    pub fn description(&self) -> String {
        match self.direction {
            ForwardDirection::LocalToRemote => {
                format!(
                    "L {}:{} -> {}:{}",
                    self.local_host, self.local_port, self.remote_host, self.remote_port
                )
            }
            ForwardDirection::RemoteToLocal => {
                format!(
                    "R {}:{} -> {}:{}",
                    self.remote_host, self.remote_port, self.local_host, self.local_port
                )
            }
            ForwardDirection::Dynamic => {
                format!("D SOCKS5 on {}:{}", self.local_host, self.local_port)
            }
        }
    }
}
