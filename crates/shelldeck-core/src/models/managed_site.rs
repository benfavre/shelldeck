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
    pub fn from_database(
        connection_id: Uuid,
        connection_name: &str,
        db: DiscoveredDatabase,
    ) -> Self {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::server_sync::{DatabaseEngine, DiscoveredDatabase, DiscoveredSite};

    fn mk_site(server_name: &str, listen_port: u16, ssl: bool) -> DiscoveredSite {
        DiscoveredSite {
            server_name: server_name.into(),
            root: "/var/www".into(),
            listen_port,
            ssl,
            config_path: "/etc/nginx/sites-enabled/x.conf".into(),
        }
    }

    // SDTEST-045 — from_nginx maps a DiscoveredSite → ManagedSite
    // with the source data preserved via `name()`, `port()`,
    // `has_ssl()`, `url()`.
    #[test]
    fn from_nginx_preserves_server_name_port_and_ssl() {
        let conn_id = Uuid::new_v4();
        let site = ManagedSite::from_nginx(conn_id, "prod", mk_site("example.com", 443, true));
        assert_eq!(site.connection_id, conn_id);
        assert_eq!(site.connection_name, "prod");
        assert_eq!(site.name(), "example.com");
        assert_eq!(site.port(), Some(443));
        assert!(site.has_ssl());
    }

    // SDTEST-046 — url() elides the port when it's the standard
    // scheme default (443 for https, 80 for http); keeps it otherwise.
    #[test]
    fn url_elides_default_ports_and_keeps_custom_ones() {
        let conn_id = Uuid::new_v4();

        let https = ManagedSite::from_nginx(conn_id, "p", mk_site("a.example", 443, true));
        assert_eq!(https.url().as_deref(), Some("https://a.example"));

        let http = ManagedSite::from_nginx(conn_id, "p", mk_site("b.example", 80, false));
        assert_eq!(http.url().as_deref(), Some("http://b.example"));

        let http_alt = ManagedSite::from_nginx(conn_id, "p", mk_site("c.example", 8080, false));
        assert_eq!(http_alt.url().as_deref(), Some("http://c.example:8080"));

        let https_alt = ManagedSite::from_nginx(conn_id, "p", mk_site("d.example", 8443, true));
        assert_eq!(https_alt.url().as_deref(), Some("https://d.example:8443"));
    }

    // SDTEST-047 — from_database preserves engine (MySQL / PostgreSQL)
    // and reports None for url/port (databases have no HTTP URL).
    #[test]
    fn from_database_preserves_engine_and_reports_no_url() {
        let conn_id = Uuid::new_v4();
        let db = DiscoveredDatabase {
            name: "mydb".into(),
            engine: DatabaseEngine::Postgresql,
            size_bytes: Some(1024 * 1024),
            table_count: Some(42),
        };
        let site = ManagedSite::from_database(conn_id, "prod-db", db);

        assert_eq!(site.name(), "mydb");
        assert!(matches!(
            site.site_type,
            ManagedSiteType::Database(ref d) if matches!(d.engine, DatabaseEngine::Postgresql),
        ));
        assert!(site.url().is_none(), "databases have no HTTP URL");
        assert!(site.port().is_none(), "databases have no HTTP port");
        assert!(
            !site.has_ssl(),
            "database SSL is not surfaced through ManagedSite"
        );
    }
}
