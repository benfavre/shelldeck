use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A remote file or directory entry parsed from ls/stat output.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileEntry {
    pub name: String,
    pub path: String,
    pub size: u64,
    pub permissions: String,
    pub modified: Option<String>,
    pub is_dir: bool,
    pub owner: String,
    pub group: String,
}

impl FileEntry {
    /// Human-readable file size.
    pub fn size_display(&self) -> String {
        if self.is_dir {
            return "-".to_string();
        }
        let size = self.size as f64;
        if size < 1024.0 {
            format!("{} B", self.size)
        } else if size < 1024.0 * 1024.0 {
            format!("{:.1} KB", size / 1024.0)
        } else if size < 1024.0 * 1024.0 * 1024.0 {
            format!("{:.1} MB", size / (1024.0 * 1024.0))
        } else {
            format!("{:.2} GB", size / (1024.0 * 1024.0 * 1024.0))
        }
    }
}

/// A discovered nginx site parsed from /etc/nginx/sites-enabled/*.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoveredSite {
    pub server_name: String,
    pub root: String,
    pub config_path: String,
    pub listen_port: u16,
    pub ssl: bool,
}

/// Database engine type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DatabaseEngine {
    Mysql,
    Postgresql,
}

impl DatabaseEngine {
    pub fn label(&self) -> &'static str {
        match self {
            DatabaseEngine::Mysql => "MySQL",
            DatabaseEngine::Postgresql => "PostgreSQL",
        }
    }
}

/// A discovered database from SHOW DATABASES or pg_stat.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoveredDatabase {
    pub name: String,
    pub engine: DatabaseEngine,
    pub size_bytes: Option<u64>,
    pub table_count: Option<u32>,
}

impl DiscoveredDatabase {
    /// Human-readable database size.
    pub fn size_display(&self) -> String {
        match self.size_bytes {
            None => "unknown".to_string(),
            Some(bytes) => {
                let size = bytes as f64;
                if size < 1024.0 {
                    format!("{} B", bytes)
                } else if size < 1024.0 * 1024.0 {
                    format!("{:.1} KB", size / 1024.0)
                } else if size < 1024.0 * 1024.0 * 1024.0 {
                    format!("{:.1} MB", size / (1024.0 * 1024.0))
                } else {
                    format!("{:.2} GB", size / (1024.0 * 1024.0 * 1024.0))
                }
            }
        }
    }
}

/// What kind of item to sync.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SyncItemKind {
    Directory {
        source_path: String,
        dest_path: String,
        exclude_patterns: Vec<String>,
    },
    Database {
        name: String,
        engine: DatabaseEngine,
        source_credentials: String,
        dest_credentials: String,
    },
    NginxSite {
        site: DiscoveredSite,
        sync_config: bool,
        sync_root: bool,
    },
}

impl SyncItemKind {
    /// Short description for display.
    pub fn label(&self) -> String {
        match self {
            SyncItemKind::Directory { source_path, .. } => {
                format!("Directory: {}", source_path)
            }
            SyncItemKind::Database { name, engine, .. } => {
                format!("{}: {}", engine.label(), name)
            }
            SyncItemKind::NginxSite { site, .. } => {
                format!("Nginx: {}", site.server_name)
            }
        }
    }
}

/// One item in a sync profile.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncItem {
    pub id: Uuid,
    pub kind: SyncItemKind,
    pub enabled: bool,
}

/// Configurable sync behavior.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncOptions {
    pub compress: bool,
    pub dry_run: bool,
    pub delete_extra: bool,
    pub bandwidth_limit: Option<u32>,
    pub skip_existing: bool,
}

impl Default for SyncOptions {
    fn default() -> Self {
        Self {
            compress: true,
            dry_run: false,
            delete_extra: false,
            bandwidth_limit: None,
            skip_existing: false,
        }
    }
}

/// A persisted sync profile configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncProfile {
    pub id: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub source_connection_id: Uuid,
    pub dest_connection_id: Uuid,
    pub items: Vec<SyncItem>,
    pub options: SyncOptions,
    pub created_at: DateTime<Utc>,
    pub last_synced: Option<DateTime<Utc>>,
}

/// Runtime status of a sync operation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SyncOperationStatus {
    Pending,
    Connecting,
    Discovering,
    Running,
    Completed,
    Failed,
    Cancelled,
}

/// Per-item progress within a sync operation.
#[derive(Debug, Clone)]
pub struct SyncProgress {
    pub item_id: Uuid,
    pub status: SyncOperationStatus,
    pub bytes_transferred: u64,
    pub total_bytes: Option<u64>,
    pub files_transferred: u32,
    pub total_files: Option<u32>,
    pub current_file: Option<String>,
    pub error_message: Option<String>,
}

impl SyncProgress {
    /// Progress percentage (0..100), or None if total is unknown.
    pub fn percent(&self) -> Option<f64> {
        self.total_bytes.map(|total| {
            if total == 0 {
                100.0
            } else {
                (self.bytes_transferred as f64 / total as f64 * 100.0).min(100.0)
            }
        })
    }
}

/// A running sync job aggregating multiple items.
#[derive(Debug, Clone)]
pub struct SyncOperation {
    pub id: Uuid,
    pub profile_id: Uuid,
    pub status: SyncOperationStatus,
    pub item_progress: Vec<SyncProgress>,
    pub log_lines: Vec<String>,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
}

impl SyncOperation {
    /// Overall progress percentage across all items.
    pub fn overall_percent(&self) -> Option<f64> {
        if self.item_progress.is_empty() {
            return None;
        }
        let mut total_bytes: u64 = 0;
        let mut transferred: u64 = 0;
        let mut has_total = false;
        for p in &self.item_progress {
            if let Some(tb) = p.total_bytes {
                total_bytes += tb;
                transferred += p.bytes_transferred;
                has_total = true;
            }
        }
        if has_total && total_bytes > 0 {
            Some((transferred as f64 / total_bytes as f64 * 100.0).min(100.0))
        } else if has_total {
            Some(100.0)
        } else {
            None
        }
    }
}
