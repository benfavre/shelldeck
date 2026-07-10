use crate::config::app_config::AppConfig;
use crate::error::{Result, ShellDeckError};
use crate::models::{Connection, ManagedSite, ManagedSiteType, PortForward, Script, SyncProfile};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tracing::info;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ConnectionStore {
    pub connections: Vec<Connection>,
    pub port_forwards: Vec<PortForward>,
    pub scripts: Vec<Script>,
    #[serde(default)]
    pub sync_profiles: Vec<SyncProfile>,
    #[serde(default)]
    pub managed_sites: Vec<ManagedSite>,
}

impl ConnectionStore {
    /// Get the store file path.
    pub fn store_path() -> PathBuf {
        AppConfig::config_dir().join("connections.json")
    }

    /// Load store from disk, or return empty defaults.
    pub fn load() -> Result<Self> {
        Self::load_from(&Self::store_path())
    }

    /// Save store to disk.
    pub fn save(&self) -> Result<()> {
        self.save_to(&Self::store_path())
    }

    /// Load store from a specific path, or create and save empty defaults there.
    pub(crate) fn load_from(path: &Path) -> Result<Self> {
        if path.exists() {
            let content = std::fs::read_to_string(path)?;
            let store: Self = serde_json::from_str(&content).map_err(|e| {
                ShellDeckError::Serialization(format!(
                    "Failed to parse store at {}: {}",
                    path.display(),
                    e
                ))
            })?;
            info!("Loaded connection store from {}", path.display());
            Ok(store)
        } else {
            let store = Self::default();
            store.save_to(path)?;
            info!("Created empty connection store at {}", path.display());
            Ok(store)
        }
    }

    /// Save store to a specific path atomically.
    pub(crate) fn save_to(&self, path: &Path) -> Result<()> {
        if let Some(dir) = path.parent() {
            if !dir.as_os_str().is_empty() && !dir.exists() {
                std::fs::create_dir_all(dir)?;
            }
        }

        let content = serde_json::to_string_pretty(self).map_err(|e| {
            ShellDeckError::Serialization(format!("Failed to serialize store: {}", e))
        })?;
        crate::util::atomic_write(path, content.as_bytes())?;
        info!("Saved connection store to {}", path.display());

        Ok(())
    }

    // --- Connection methods ---

    /// Add a new connection to the store and save.
    pub fn add_connection(&mut self, connection: Connection) -> Result<()> {
        self.connections.push(connection);
        self.save()
    }

    /// Remove a connection by ID, also removing associated port forwards. Returns true if found.
    pub fn remove_connection(&mut self, id: Uuid) -> Result<bool> {
        let original_len = self.connections.len();
        self.connections.retain(|c| c.id != id);
        // Also remove port forwards for this connection
        self.port_forwards.retain(|pf| pf.connection_id != id);

        if self.connections.len() != original_len {
            self.save()?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Update a connection in place. Returns true if found.
    pub fn update_connection(&mut self, connection: Connection) -> Result<bool> {
        if let Some(existing) = self.connections.iter_mut().find(|c| c.id == connection.id) {
            *existing = connection;
            self.save()?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Get a connection by ID.
    pub fn get_connection(&self, id: Uuid) -> Option<&Connection> {
        self.connections.iter().find(|c| c.id == id)
    }

    /// Get a mutable reference to a connection by ID.
    pub fn get_connection_mut(&mut self, id: Uuid) -> Option<&mut Connection> {
        self.connections.iter_mut().find(|c| c.id == id)
    }

    // --- PortForward methods ---

    /// Add a port forward and save.
    pub fn add_port_forward(&mut self, forward: PortForward) -> Result<()> {
        self.port_forwards.push(forward);
        self.save()
    }

    /// Remove a port forward by ID. Returns true if found.
    pub fn remove_port_forward(&mut self, id: Uuid) -> Result<bool> {
        let original_len = self.port_forwards.len();
        self.port_forwards.retain(|pf| pf.id != id);

        if self.port_forwards.len() != original_len {
            self.save()?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Update a port forward in place. Returns true if found.
    pub fn update_port_forward(&mut self, forward: PortForward) -> Result<bool> {
        if let Some(existing) = self.port_forwards.iter_mut().find(|pf| pf.id == forward.id) {
            *existing = forward;
            self.save()?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Get port forwards for a given connection.
    pub fn get_forwards_for_connection(&self, connection_id: Uuid) -> Vec<&PortForward> {
        self.port_forwards
            .iter()
            .filter(|pf| pf.connection_id == connection_id)
            .collect()
    }

    // --- Script methods ---

    /// Add a script and save.
    pub fn add_script(&mut self, script: Script) -> Result<()> {
        self.scripts.push(script);
        self.save()
    }

    /// Remove a script by ID. Returns true if found.
    pub fn remove_script(&mut self, id: Uuid) -> Result<bool> {
        let original_len = self.scripts.len();
        self.scripts.retain(|s| s.id != id);

        if self.scripts.len() != original_len {
            self.save()?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Update a script in place. Returns true if found.
    pub fn update_script(&mut self, script: Script) -> Result<bool> {
        if let Some(existing) = self.scripts.iter_mut().find(|s| s.id == script.id) {
            *existing = script;
            self.save()?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Get a script by ID.
    pub fn get_script(&self, id: Uuid) -> Option<&Script> {
        self.scripts.iter().find(|s| s.id == id)
    }

    // --- SyncProfile methods ---

    /// Add a sync profile and save.
    pub fn add_sync_profile(&mut self, profile: SyncProfile) -> Result<()> {
        self.sync_profiles.push(profile);
        self.save()
    }

    /// Remove a sync profile by ID. Returns true if found.
    pub fn remove_sync_profile(&mut self, id: Uuid) -> Result<bool> {
        let original_len = self.sync_profiles.len();
        self.sync_profiles.retain(|p| p.id != id);

        if self.sync_profiles.len() != original_len {
            self.save()?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Update a sync profile in place. Returns true if found.
    pub fn update_sync_profile(&mut self, profile: SyncProfile) -> Result<bool> {
        if let Some(existing) = self.sync_profiles.iter_mut().find(|p| p.id == profile.id) {
            *existing = profile;
            self.save()?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Get a sync profile by ID.
    pub fn get_sync_profile(&self, id: Uuid) -> Option<&SyncProfile> {
        self.sync_profiles.iter().find(|p| p.id == id)
    }

    // --- ManagedSite methods ---

    /// Add a managed site, deduplicating by connection_id + name + type.
    pub fn add_managed_site(&mut self, site: ManagedSite) -> Result<()> {
        let name = site.name().to_string();
        let conn_id = site.connection_id;
        let is_nginx = matches!(site.site_type, ManagedSiteType::NginxSite(_));

        // Dedup: skip if same connection + name + type already exists
        let exists = self.managed_sites.iter().any(|s| {
            s.connection_id == conn_id
                && s.name() == name
                && matches!(
                    (&s.site_type, is_nginx),
                    (ManagedSiteType::NginxSite(_), true) | (ManagedSiteType::Database(_), false)
                )
        });

        if !exists {
            self.managed_sites.push(site);
            self.save()?;
        }
        Ok(())
    }

    /// Replace managed sites for the scanned connections.
    ///
    /// Clears all existing sites whose `connection_id` appears in the batch,
    /// then inserts the fresh results and saves.  This ensures stale entries
    /// from previous scans are removed automatically.
    pub fn add_managed_sites_bulk(&mut self, sites: Vec<ManagedSite>) -> Result<()> {
        if sites.is_empty() {
            return Ok(());
        }

        // Collect which connections are being refreshed
        let refreshed_conns: std::collections::HashSet<Uuid> =
            sites.iter().map(|s| s.connection_id).collect();

        // Remove old entries for those connections
        self.managed_sites
            .retain(|s| !refreshed_conns.contains(&s.connection_id));

        // Add all fresh results
        self.managed_sites.extend(sites);
        self.save()
    }

    /// Remove a managed site by ID. Returns true if found.
    pub fn remove_managed_site(&mut self, id: Uuid) -> Result<bool> {
        let original_len = self.managed_sites.len();
        self.managed_sites.retain(|s| s.id != id);
        if self.managed_sites.len() != original_len {
            self.save()?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Update a managed site in place. Returns true if found.
    pub fn update_managed_site(&mut self, site: ManagedSite) -> Result<bool> {
        if let Some(existing) = self.managed_sites.iter_mut().find(|s| s.id == site.id) {
            *existing = site;
            self.save()?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Get all managed sites for a given connection.
    pub fn get_sites_for_connection(&self, connection_id: Uuid) -> Vec<&ManagedSite> {
        self.managed_sites
            .iter()
            .filter(|s| s.connection_id == connection_id)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{ForwardDirection, ScriptTarget};

    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

    fn temp_path(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "shelldeck-store-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir.join(name)
    }

    #[test]
    fn round_trip_with_data() {
        let path = temp_path("connections.json");

        let mut store = ConnectionStore::default();
        let conn = Connection::new_manual(
            "prod".to_string(),
            "example.com".to_string(),
            "root".to_string(),
        );
        let conn_id = conn.id;
        let fwd = PortForward::new_local(conn_id, 8080, "127.0.0.1", 80);
        let fwd_id = fwd.id;
        let script = Script::new(
            "deploy".to_string(),
            "echo deploying".to_string(),
            ScriptTarget::Remote(conn_id),
        );
        let script_id = script.id;

        store.connections.push(conn);
        store.port_forwards.push(fwd);
        store.scripts.push(script);

        store.save_to(&path).expect("save_to");
        let loaded = ConnectionStore::load_from(&path).expect("load_from");

        assert_eq!(loaded.connections.len(), 1);
        assert_eq!(loaded.connections[0].id, conn_id);
        assert_eq!(loaded.connections[0].alias, "prod");
        assert_eq!(loaded.connections[0].hostname, "example.com");

        assert_eq!(loaded.port_forwards.len(), 1);
        assert_eq!(loaded.port_forwards[0].id, fwd_id);
        assert_eq!(loaded.port_forwards[0].local_port, 8080);
        assert_eq!(
            loaded.port_forwards[0].direction,
            ForwardDirection::LocalToRemote
        );

        assert_eq!(loaded.scripts.len(), 1);
        assert_eq!(loaded.scripts[0].id, script_id);
        assert_eq!(loaded.scripts[0].name, "deploy");
        assert_eq!(loaded.scripts[0].body, "echo deploying");

        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }

    #[test]
    fn load_from_missing_creates_empty() {
        let path = temp_path("connections.json");
        assert!(!path.exists());

        let loaded = ConnectionStore::load_from(&path).expect("load_from");

        assert!(loaded.connections.is_empty());
        assert!(loaded.port_forwards.is_empty());
        assert!(loaded.scripts.is_empty());
        assert!(path.exists(), "load_from should create the file");

        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }

    #[test]
    fn load_from_corrupt_returns_err() {
        let path = temp_path("connections.json");
        std::fs::write(&path, b"{ this is not valid json ]").expect("seed garbage");

        let result = ConnectionStore::load_from(&path);
        assert!(result.is_err(), "corrupt store should error, not panic");

        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }

    // SDTEST-084 — Manual, SshConfig, and CloudSync connections must
    // coexist in a single store round-trip without one squashing
    // another. Regression sensor for cloud_sync merge (SDUC-104): a
    // sync must not silently prune a manual/ssh-config connection
    // just because they live in the same file.
    #[test]
    fn round_trip_preserves_manual_ssh_config_and_cloud_sync_sources() {
        use crate::models::connection::{Connection, ConnectionSource};

        let path = temp_path("connections.json");
        let mut store = ConnectionStore::default();

        let mut manual = Connection::new_manual("m".into(), "m.example".into(), "u".into());
        manual.tags = vec!["manual-tag".into()];

        let mut ssh_cfg = Connection::new_manual("s".into(), "s.example".into(), "u".into());
        ssh_cfg.source = ConnectionSource::SshConfig;

        let mut cloud = Connection::new_manual("c".into(), "c.example".into(), "u".into());
        cloud.source = ConnectionSource::CloudSync;

        let ids = (manual.id, ssh_cfg.id, cloud.id);
        store.connections.push(manual);
        store.connections.push(ssh_cfg);
        store.connections.push(cloud);

        store.save_to(&path).expect("save_to");
        let loaded = ConnectionStore::load_from(&path).expect("load_from");

        assert_eq!(loaded.connections.len(), 3);
        let by_id: std::collections::HashMap<_, _> =
            loaded.connections.iter().map(|c| (c.id, c)).collect();

        assert!(
            matches!(by_id[&ids.0].source, ConnectionSource::Manual),
            "manual source lost on round trip",
        );
        assert!(
            matches!(by_id[&ids.1].source, ConnectionSource::SshConfig),
            "ssh_config source lost on round trip",
        );
        assert!(
            matches!(by_id[&ids.2].source, ConnectionSource::CloudSync),
            "cloud_sync source lost on round trip",
        );
        assert_eq!(by_id[&ids.0].tags, vec!["manual-tag".to_string()]);

        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }
}
