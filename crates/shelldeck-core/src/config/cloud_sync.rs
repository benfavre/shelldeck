//! Cloud Sync — pull SSH connection profiles from the Inklura Manage portal
//! (`manage.inklura.fr`) into ShellDeck's connection store.
//!
//! The flow is deliberately simple and side-effect-bounded:
//!   1. [`fetch_sync`] does a device check-in (`POST`) and returns the remote
//!      profile set (falling back to `GET` if the server's route-method cache
//!      lags and answers 404/405).
//!   2. [`merge_profiles`] upserts those profiles into a [`ConnectionStore`] by
//!      UUID, marking them [`ConnectionSource::CloudSync`], and prunes cloud
//!      entries that vanished remotely. Manual / SSH-config connections are
//!      never touched by the prune.
//!   3. [`sync_now`] wires the two together: fetch, load store, merge, save.
//!
//! Everything here returns the crate [`Result`]; nothing panics.

use crate::config::store::ConnectionStore;
use crate::error::{Result, ShellDeckError};
use crate::models::connection::{Connection, ConnectionSource, ConnectionStatus};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::Duration;
use uuid::Uuid;

/// Persisted `[cloud_sync]` configuration section (part of `AppConfig`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CloudSyncConfig {
    /// Master on/off switch.
    pub enabled: bool,
    /// Base URL of the management portal, e.g. `https://manage.inklura.fr`.
    pub base_url: String,
    /// Bearer token issued by the portal (`sd_...`).
    pub token: String,
    /// Whether to sync automatically at app startup.
    pub sync_on_startup: bool,
    /// UUID of the currently selected Inklura Manage site, or `None` for "all
    /// sites" (no filter). Stored as a string so an empty/legacy config parses.
    pub active_site_id: Option<String>,
    /// Display label for the active site (chip text).
    pub active_site_label: Option<String>,
    /// Persisted app-mode selection for super-admins (User/Support/Dev).
    /// Defaults to Dev; ignored for non-super-admins (forced to User).
    pub mode: crate::config::cloud_account::AppMode,
}

impl Default for CloudSyncConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            base_url: "https://manage.inklura.fr".to_string(),
            token: String::new(),
            sync_on_startup: true,
            active_site_id: None,
            active_site_label: None,
            mode: crate::config::cloud_account::AppMode::default(),
        }
    }
}

impl CloudSyncConfig {
    /// True when sync can actually run: enabled, and both a token and base URL
    /// are present.
    pub fn is_configured(&self) -> bool {
        self.enabled && !self.token.is_empty() && !self.base_url.is_empty()
    }
}

/// A single connection profile as returned by the sync endpoint.
///
/// Tolerant of missing / null optional fields, an absent `port` (defaults to
/// 22) and absent `tags` (defaults to empty), per the server contract.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteProfile {
    pub id: String,
    #[serde(default)]
    pub alias: String,
    #[serde(default)]
    pub hostname: String,
    #[serde(default = "default_port")]
    pub port: u16,
    #[serde(default)]
    pub user: String,
    #[serde(default)]
    pub proxy_jump: Option<String>,
    #[serde(default)]
    pub group: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub forward_agent: bool,
    #[serde(default)]
    pub identity_file: Option<String>,
    /// Inklura Manage site binding (set in the manage console). Absent/null on
    /// unbound profiles.
    #[serde(default)]
    pub site_id: Option<Uuid>,
    #[serde(default)]
    pub site_label: Option<String>,
    /// Free-form notes from the portal. ShellDeck has no notes field on
    /// `Connection`, so this is parsed for completeness but not merged.
    #[serde(default)]
    pub notes: Option<String>,
}

fn default_port() -> u16 {
    22
}

/// Full payload from the sync endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncPayload {
    #[serde(default)]
    pub version: u32,
    #[serde(default)]
    pub generated_at: Option<String>,
    #[serde(default)]
    pub connections: Vec<RemoteProfile>,
}

/// Outcome of a merge: how many connections were added, updated, or removed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MergeStats {
    pub added: usize,
    pub updated: usize,
    pub removed: usize,
}

impl MergeStats {
    /// True if the merge changed anything (so the store needs saving).
    pub fn changed(&self) -> bool {
        self.added + self.updated + self.removed > 0
    }
}

/// Merge a set of remote profiles into `store` (pure — no I/O).
///
/// - Upsert by UUID `id`. Entries with an unparseable `id` are skipped.
/// - Existing connection with the same id → overwrite the mutable fields and
///   force `source = CloudSync`, while PRESERVING `auto_forwards`,
///   `auto_scripts` and the (skip-serialized) live `status`.
/// - Missing id → push a new `CloudSync` connection.
/// - Prune connections whose `source == CloudSync` that are no longer present
///   remotely. Manual / SSH-config connections are never removed.
pub fn merge_profiles(store: &mut ConnectionStore, remote: &[RemoteProfile]) -> MergeStats {
    let mut stats = MergeStats::default();
    let mut remote_ids: std::collections::HashSet<Uuid> = std::collections::HashSet::new();

    for rp in remote {
        let id = match Uuid::parse_str(&rp.id) {
            Ok(id) => id,
            Err(_) => {
                tracing::warn!("Cloud sync: skipping profile with invalid id {:?}", rp.id);
                continue;
            }
        };
        remote_ids.insert(id);

        let identity_file = rp
            .identity_file
            .as_ref()
            .filter(|s| !s.is_empty())
            .map(PathBuf::from);

        if let Some(existing) = store.connections.iter_mut().find(|c| c.id == id) {
            // Overwrite managed fields; preserve auto_forwards / auto_scripts /
            // status. Force the source so a profile always reads as cloud-managed.
            existing.alias = rp.alias.clone();
            existing.hostname = rp.hostname.clone();
            existing.port = rp.port;
            existing.user = rp.user.clone();
            existing.proxy_jump = rp.proxy_jump.clone();
            existing.group = rp.group.clone();
            existing.tags = rp.tags.clone();
            existing.forward_agent = rp.forward_agent;
            existing.identity_file = identity_file;
            existing.site_id = rp.site_id;
            existing.site_label = rp.site_label.clone();
            existing.source = ConnectionSource::CloudSync;
            stats.updated += 1;
        } else {
            store.connections.push(Connection {
                id,
                alias: rp.alias.clone(),
                hostname: rp.hostname.clone(),
                port: rp.port,
                user: rp.user.clone(),
                identity_file,
                proxy_jump: rp.proxy_jump.clone(),
                group: rp.group.clone(),
                tags: rp.tags.clone(),
                auto_forwards: Vec::new(),
                auto_scripts: Vec::new(),
                source: ConnectionSource::CloudSync,
                forward_agent: rp.forward_agent,
                site_id: rp.site_id,
                site_label: rp.site_label.clone(),
                status: ConnectionStatus::default(),
            });
            stats.added += 1;
        }
    }

    // Prune cloud-managed connections that disappeared from the remote set.
    let before = store.connections.len();
    store
        .connections
        .retain(|c| c.source != ConnectionSource::CloudSync || remote_ids.contains(&c.id));
    stats.removed = before - store.connections.len();

    stats
}

/// Best-effort machine hostname, dependency-free.
fn machine_hostname() -> String {
    if let Ok(h) = std::env::var("HOSTNAME") {
        let h = h.trim();
        if !h.is_empty() {
            return h.to_string();
        }
    }
    if let Ok(h) = std::fs::read_to_string("/etc/hostname") {
        let h = h.trim();
        if !h.is_empty() {
            return h.to_string();
        }
    }
    "unknown".to_string()
}

/// `{os}-{arch}` platform string, e.g. `linux-x86_64`.
fn platform_string() -> String {
    format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH)
}

/// Fetch the remote profile set, performing the device check-in.
///
/// Does `POST {base_url}/api/manage/shelldeck/sync` with the check-in body; on
/// a 404/405 (server route-method cache lag) it falls back to a plain `GET`.
/// A 401 is surfaced as a clear "token rejected" error.
pub fn fetch_sync(cfg: &CloudSyncConfig, app_version: &str) -> Result<SyncPayload> {
    let base = cfg.base_url.trim_end_matches('/');
    let url = format!("{}/api/manage/shelldeck/sync", base);

    let client = reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(4))
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| ShellDeckError::Connection(format!("failed to build HTTP client: {}", e)))?;

    let body = serde_json::json!({
        "version": app_version,
        "hostname": machine_hostname(),
        "platform": platform_string(),
    });

    let resp = client
        .post(&url)
        .bearer_auth(&cfg.token)
        .json(&body)
        .send()
        .map_err(|e| ShellDeckError::Connection(format!("cloud sync request failed: {}", e)))?;

    // Route-method cache lag: retry as GET (no check-in body).
    let status = resp.status().as_u16();
    if status == 404 || status == 405 {
        let get_resp = client
            .get(&url)
            .bearer_auth(&cfg.token)
            .send()
            .map_err(|e| ShellDeckError::Connection(format!("cloud sync GET failed: {}", e)))?;
        return parse_response(get_resp);
    }

    parse_response(resp)
}

/// Interpret an HTTP response into a [`SyncPayload`] or a descriptive error.
fn parse_response(resp: reqwest::blocking::Response) -> Result<SyncPayload> {
    let status = resp.status();
    if status.as_u16() == 401 {
        return Err(ShellDeckError::Connection(
            "sync token rejected (401) — check the cloud_sync token in shelldeck.toml".to_string(),
        ));
    }
    if !status.is_success() {
        return Err(ShellDeckError::Connection(format!(
            "cloud sync failed: HTTP {}",
            status.as_u16()
        )));
    }
    resp.json::<SyncPayload>()
        .map_err(|e| ShellDeckError::Serialization(format!("invalid sync payload: {}", e)))
}

/// Fetch, load the store, merge, and save (only if anything changed).
///
/// Never panics; every failure path is a [`ShellDeckError`].
pub fn sync_now(cfg: &CloudSyncConfig, app_version: &str) -> Result<MergeStats> {
    let payload = fetch_sync(cfg, app_version)?;
    let mut store = ConnectionStore::load()?;
    let stats = merge_profiles(&mut store, &payload.connections);
    if stats.changed() {
        store.save()?;
    }
    Ok(stats)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cloud_conn(id: Uuid, alias: &str) -> Connection {
        let mut c =
            Connection::new_manual(alias.to_string(), "old-host".to_string(), "old".to_string());
        c.id = id;
        c.source = ConnectionSource::CloudSync;
        c
    }

    fn remote(id: Uuid, alias: &str, host: &str) -> RemoteProfile {
        RemoteProfile {
            id: id.to_string(),
            alias: alias.to_string(),
            hostname: host.to_string(),
            port: 22,
            user: "infra".to_string(),
            proxy_jump: None,
            group: Some("Production".to_string()),
            tags: vec!["prod".to_string()],
            forward_agent: false,
            identity_file: None,
            site_id: None,
            site_label: None,
            notes: None,
        }
    }

    #[test]
    fn merge_adds_new_profiles() {
        let mut store = ConnectionStore::default();
        let id = Uuid::new_v4();
        let stats = merge_profiles(&mut store, &[remote(id, "activ-2", "1.2.3.4")]);

        assert_eq!(
            stats,
            MergeStats {
                added: 1,
                updated: 0,
                removed: 0
            }
        );
        assert_eq!(store.connections.len(), 1);
        let c = &store.connections[0];
        assert_eq!(c.id, id);
        assert_eq!(c.alias, "activ-2");
        assert_eq!(c.hostname, "1.2.3.4");
        assert_eq!(c.user, "infra");
        assert_eq!(c.group.as_deref(), Some("Production"));
        assert_eq!(c.tags, vec!["prod".to_string()]);
        assert_eq!(c.source, ConnectionSource::CloudSync);
    }

    #[test]
    fn merge_copies_site_binding() {
        let mut store = ConnectionStore::default();
        let id = Uuid::new_v4();
        let site = Uuid::new_v4();
        let mut rp = remote(id, "demo", "1.2.3.4");
        rp.site_id = Some(site);
        rp.site_label = Some("Inklura".to_string());

        merge_profiles(&mut store, &[rp]);
        let c = &store.connections[0];
        assert_eq!(c.site_id, Some(site));
        assert_eq!(c.site_label.as_deref(), Some("Inklura"));

        // Re-sync with the binding cleared → connection's binding clears too.
        let cleared = remote(id, "demo", "1.2.3.4");
        merge_profiles(&mut store, &[cleared]);
        assert!(store.connections[0].site_id.is_none());
        assert!(store.connections[0].site_label.is_none());
    }

    #[test]
    fn merge_updates_existing_and_preserves_local_only_fields() {
        let mut store = ConnectionStore::default();
        let id = Uuid::new_v4();
        let mut existing = cloud_conn(id, "old-alias");
        // Local-only associations that must survive a sync.
        let fwd = Uuid::new_v4();
        let script = Uuid::new_v4();
        existing.auto_forwards = vec![fwd];
        existing.auto_scripts = vec![script];
        existing.status = ConnectionStatus::Connected;
        store.connections.push(existing);

        let stats = merge_profiles(&mut store, &[remote(id, "new-alias", "9.9.9.9")]);

        assert_eq!(
            stats,
            MergeStats {
                added: 0,
                updated: 1,
                removed: 0
            }
        );
        assert_eq!(store.connections.len(), 1);
        let c = &store.connections[0];
        assert_eq!(c.alias, "new-alias");
        assert_eq!(c.hostname, "9.9.9.9");
        // Preserved:
        assert_eq!(c.auto_forwards, vec![fwd]);
        assert_eq!(c.auto_scripts, vec![script]);
        assert_eq!(c.status, ConnectionStatus::Connected);
        assert_eq!(c.source, ConnectionSource::CloudSync);
    }

    #[test]
    fn merge_removes_vanished_cloud_profiles() {
        let mut store = ConnectionStore::default();
        let keep = Uuid::new_v4();
        let gone = Uuid::new_v4();
        store.connections.push(cloud_conn(keep, "keep"));
        store.connections.push(cloud_conn(gone, "gone"));

        let stats = merge_profiles(&mut store, &[remote(keep, "keep", "1.1.1.1")]);

        assert_eq!(
            stats,
            MergeStats {
                added: 0,
                updated: 1,
                removed: 1
            }
        );
        assert_eq!(store.connections.len(), 1);
        assert_eq!(store.connections[0].id, keep);
    }

    #[test]
    fn merge_never_touches_manual_or_ssh_config() {
        let mut store = ConnectionStore::default();
        let manual = Connection::new_manual("manual".into(), "m.example".into(), "root".into());
        let manual_id = manual.id;
        let mut ssh = Connection::new_manual("ssh".into(), "s.example".into(), "root".into());
        ssh.source = ConnectionSource::SshConfig;
        let ssh_id = ssh.id;
        store.connections.push(manual);
        store.connections.push(ssh);

        // A remote set that shares NO ids with the local manual/ssh entries and
        // is otherwise empty of matches — the prune must leave both alone.
        let stats = merge_profiles(&mut store, &[remote(Uuid::new_v4(), "cloud", "c.example")]);

        assert_eq!(stats.added, 1);
        assert_eq!(stats.removed, 0);
        assert!(store
            .connections
            .iter()
            .any(|c| c.id == manual_id && c.source == ConnectionSource::Manual));
        assert!(store
            .connections
            .iter()
            .any(|c| c.id == ssh_id && c.source == ConnectionSource::SshConfig));
    }

    #[test]
    fn merge_skips_unparseable_ids() {
        let mut store = ConnectionStore::default();
        let bad = RemoteProfile {
            id: "not-a-uuid".to_string(),
            ..remote(Uuid::new_v4(), "x", "h")
        };
        let stats = merge_profiles(&mut store, &[bad]);
        assert_eq!(stats, MergeStats::default());
        assert!(store.connections.is_empty());
    }

    #[test]
    fn cloud_sync_config_parses_without_active_site_fields() {
        // A [cloud_sync] section written before the site switcher existed.
        let toml = r#"
enabled = true
base_url = "https://manage.inklura.fr"
token = "sd_x"
sync_on_startup = false
"#;
        let cfg: CloudSyncConfig = toml::from_str(toml).expect("parse legacy cloud_sync");
        assert!(cfg.enabled);
        assert!(cfg.active_site_id.is_none());
        assert!(cfg.active_site_label.is_none());
        // Mode defaults to Dev when absent from an older config.
        assert_eq!(cfg.mode, crate::config::cloud_account::AppMode::Dev);

        // Round-trip with an active site set.
        let mut with_site = CloudSyncConfig::default();
        with_site.active_site_id = Some("site-uuid".to_string());
        with_site.active_site_label = Some("Inklura".to_string());
        let s = toml::to_string(&with_site).unwrap();
        let back: CloudSyncConfig = toml::from_str(&s).unwrap();
        assert_eq!(back.active_site_id.as_deref(), Some("site-uuid"));
        assert_eq!(back.active_site_label.as_deref(), Some("Inklura"));
    }

    #[test]
    fn is_configured_semantics() {
        let mut cfg = CloudSyncConfig::default();
        assert!(!cfg.is_configured(), "disabled by default");
        cfg.enabled = true;
        assert!(!cfg.is_configured(), "no token");
        cfg.token = "sd_abc".to_string();
        assert!(cfg.is_configured());
        cfg.base_url.clear();
        assert!(!cfg.is_configured(), "empty base_url");
    }

    #[test]
    fn remote_profile_parses_nulls_and_missing_fields() {
        // Minimal object: only `id`. Everything else absent → defaults.
        let rp: RemoteProfile = serde_json::from_str(r#"{"id":"abc"}"#).expect("parse minimal");
        assert_eq!(rp.id, "abc");
        assert_eq!(rp.port, 22);
        assert!(rp.tags.is_empty());
        assert!(rp.proxy_jump.is_none());
        assert!(!rp.forward_agent);

        // Explicit nulls for optionals.
        let json = r#"{
            "id":"id2","alias":"a","hostname":"h","user":"u",
            "proxy_jump":null,"group":null,"identity_file":null,"notes":null
        }"#;
        let rp: RemoteProfile = serde_json::from_str(json).expect("parse nulls");
        assert!(rp.proxy_jump.is_none());
        assert!(rp.group.is_none());
        assert!(rp.identity_file.is_none());
        assert_eq!(rp.port, 22);
    }

    #[test]
    fn sync_payload_parses_contract_example() {
        let json = r#"{
          "version": 1, "generated_at": "2026-07-02T12:00:00Z",
          "connections": [ { "id": "0e7b...", "alias": "activ-2", "hostname": "1.2.3.4",
            "port": 22, "user": "infra", "proxy_jump": null, "group": "Production",
            "tags": ["prod"], "forward_agent": false, "identity_file": null, "notes": null } ] }"#;
        let payload: SyncPayload = serde_json::from_str(json).expect("parse payload");
        assert_eq!(payload.version, 1);
        assert_eq!(
            payload.generated_at.as_deref(),
            Some("2026-07-02T12:00:00Z")
        );
        assert_eq!(payload.connections.len(), 1);
        assert_eq!(payload.connections[0].alias, "activ-2");
    }

    #[test]
    fn merge_reports_no_change_when_nothing_moves() {
        let mut store = ConnectionStore::default();
        assert!(!merge_profiles(&mut store, &[]).changed());
    }

    /// Live end-to-end check against a real sync endpoint. Ignored by default —
    /// requires network and a valid token, supplied via env so no secret lives
    /// in the repo:
    ///
    /// ```bash
    /// SHELLDECK_SYNC_URL=https://manage.inklura.fr \
    /// SHELLDECK_SYNC_TOKEN=sd_... \
    ///   cargo test -p shelldeck-core -- --ignored live_fetch_sync
    /// ```
    #[test]
    #[ignore = "network + token required; set SHELLDECK_SYNC_TOKEN"]
    fn live_fetch_sync() {
        let base_url = std::env::var("SHELLDECK_SYNC_URL")
            .unwrap_or_else(|_| "https://manage.inklura.fr".to_string());
        let token = std::env::var("SHELLDECK_SYNC_TOKEN")
            .expect("set SHELLDECK_SYNC_TOKEN for the live test");
        let cfg = CloudSyncConfig {
            enabled: true,
            base_url,
            token,
            sync_on_startup: false,
            ..Default::default()
        };
        let payload = super::fetch_sync(&cfg, crate::VERSION).expect("live fetch_sync");
        // A well-formed payload: every profile must have a parseable UUID.
        for c in &payload.connections {
            assert!(
                Uuid::parse_str(&c.id).is_ok(),
                "profile id should be a UUID, got {:?}",
                c.id
            );
        }
        eprintln!(
            "live_fetch_sync: version {}, {} connection(s)",
            payload.version,
            payload.connections.len()
        );
    }
}
