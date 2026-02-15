use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::server_sync::{DatabaseEngine, DiscoveredDatabase, DiscoveredSite};

/// A site or database discovered from a server, persisted globally.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManagedSite {
    pub id: Uuid,
    /// Which server connection this was discovered from (Uuid::nil() for local).
    pub connection_id: Uuid,
    /// Cached display name of the connection at discovery time.
    pub connection_name: String,
    pub site_type: ManagedSiteType,
    pub discovered_at: DateTime<Utc>,
    pub last_checked: Option<DateTime<Utc>>,
    /// Runtime-only status, not persisted.
    #[serde(skip)]
    pub status: SiteStatus,
    pub notes: Option<String>,
    pub favorite: bool,
    #[serde(default)]
    pub tags: Vec<String>,
}

/// What kind of managed site this is.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ManagedSiteType {
    NginxSite(DiscoveredSite),
    Database(DiscoveredDatabase),
}

/// Runtime status of a managed site.
#[derive(Debug, Clone, Default)]
pub enum SiteStatus {
    #[default]
    Unknown,
    Online,
    Offline,
    Error(String),
}

impl ManagedSite {
    /// Create from a discovered nginx site.
    pub fn from_nginx(connection_id: Uuid, connection_name: &str, site: DiscoveredSite) -> Self {
        Self {
            id: Uuid::new_v4(),
            connection_id,
            connection_name: connection_name.to_string(),
            site_type: ManagedSiteType::NginxSite(site),
            discovered_at: Utc::now(),
            last_checked: None,
            status: SiteStatus::Unknown,
            notes: None,
            favorite: false,
            tags: Vec::new(),
        }
    }

    /// Create from a discovered database.
    pub fn from_database(connection_id: Uuid, connection_name: &str, db: DiscoveredDatabase) -> Self {
        Self {
            id: Uuid::new_v4(),
            connection_id,
            connection_name: connection_name.to_string(),
            site_type: ManagedSiteType::Database(db),
            discovered_at: Utc::now(),
            last_checked: None,
            status: SiteStatus::Unknown,
            notes: None,
            favorite: false,
            tags: Vec::new(),
        }
    }

    /// Display name for this site.
    pub fn name(&self) -> &str {
        match &self.site_type {
            ManagedSiteType::NginxSite(s) => &s.server_name,
            ManagedSiteType::Database(d) => &d.name,
        }
    }

    /// URL for nginx sites, None for databases.
    pub fn url(&self) -> Option<String> {
        match &self.site_type {
            ManagedSiteType::NginxSite(s) => {
                let scheme = if s.ssl { "https" } else { "http" };
                let port_suffix = match (s.ssl, s.listen_port) {
                    (true, 443) | (false, 80) => String::new(),
                    _ => format!(":{}", s.listen_port),
                };
                Some(format!("{}://{}{}", scheme, s.server_name, port_suffix))
            }
            ManagedSiteType::Database(_) => None,
        }
    }

    /// Port number.
    pub fn port(&self) -> Option<u16> {
        match &self.site_type {
            ManagedSiteType::NginxSite(s) => Some(s.listen_port),
            ManagedSiteType::Database(_) => None,
        }
    }

    /// Whether the site has SSL.
    pub fn has_ssl(&self) -> bool {
        match &self.site_type {
            ManagedSiteType::NginxSite(s) => s.ssl,
            ManagedSiteType::Database(_) => false,
        }
    }

    /// Root path for nginx sites, or size display for databases.
    pub fn root_or_size(&self) -> String {
        match &self.site_type {
            ManagedSiteType::NginxSite(s) => s.root.clone(),
            ManagedSiteType::Database(d) => d.size_display(),
        }
    }
}

impl ManagedSiteType {
    /// Short label for the type.
    pub fn label(&self) -> &'static str {
        match self {
            ManagedSiteType::NginxSite(_) => "Nginx",
            ManagedSiteType::Database(d) => d.engine.label(),
        }
    }

    /// Type icon character for display.
    pub fn type_icon(&self) -> &'static str {
        match self {
            ManagedSiteType::NginxSite(_) => "W",
            ManagedSiteType::Database(d) => match d.engine {
                DatabaseEngine::Mysql => "M",
                DatabaseEngine::Postgresql => "P",
            },
        }
    }
}

impl SiteStatus {
    pub fn label(&self) -> &str {
        match self {
            SiteStatus::Unknown => "Unknown",
            SiteStatus::Online => "Online",
            SiteStatus::Offline => "Offline",
            SiteStatus::Error(_) => "Error",
        }
    }
}
