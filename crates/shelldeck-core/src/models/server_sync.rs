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

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn progress(bytes_transferred: u64, total_bytes: Option<u64>) -> SyncProgress {
        SyncProgress {
            item_id: Uuid::new_v4(),
            status: SyncOperationStatus::Running,
            bytes_transferred,
            total_bytes,
            files_transferred: 0,
            total_files: None,
            current_file: None,
            error_message: None,
        }
    }

    fn op(items: Vec<SyncProgress>) -> SyncOperation {
        SyncOperation {
            id: Uuid::new_v4(),
            profile_id: Uuid::new_v4(),
            status: SyncOperationStatus::Running,
            item_progress: items,
            log_lines: Vec::new(),
            started_at: Utc::now(),
            finished_at: None,
        }
    }

    // SDTEST-019a — percent(): None when total unknown, Some in [0, 100]
    // otherwise. Contract correction vs my initial inventory: the fn
    // returns a percentage (0..=100), NOT a ratio (0..=1). USE_CASES.md
    // updated to match.
    #[test]
    fn percent_is_none_when_total_unknown() {
        assert!(progress(500, None).percent().is_none());
    }

    #[test]
    fn percent_zero_total_returns_100() {
        // Empty item (nothing to transfer) reports 100% — guards against
        // 0/0 in the UI progress bar.
        assert_eq!(progress(0, Some(0)).percent(), Some(100.0));
    }

    #[test]
    fn percent_clamps_to_100_even_if_transferred_exceeds_total() {
        // rsync sometimes reports > total when compressed / verifying;
        // the UI bar must not overshoot.
        let p = progress(1500, Some(1000)).percent().unwrap();
        assert!((0.0..=100.0).contains(&p), "clamp violated: {p}");
        assert_eq!(p, 100.0);
    }

    #[test]
    fn percent_normal_case() {
        assert_eq!(progress(250, Some(1000)).percent(), Some(25.0));
        assert_eq!(progress(1000, Some(1000)).percent(), Some(100.0));
        assert_eq!(progress(0, Some(1000)).percent(), Some(0.0));
    }

    // SDTEST-019b — overall_percent() is SIZE-weighted, not
    // item-count-weighted. A 1 GB item at 50% dominates ten 1 KB
    // items at 100% — the aggregate is roughly 50%.
    #[test]
    fn overall_percent_is_size_weighted_not_count_weighted() {
        let big = progress(500_000_000, Some(1_000_000_000)); // 1 GB, 50%
        let tinies: Vec<_> = (0..10).map(|_| progress(1_000, Some(1_000))).collect();

        let mut items = vec![big];
        items.extend(tinies);
        let overall = op(items).overall_percent().unwrap();

        // count-weighted would be (0.5 + 10 * 1.0) / 11 ≈ 0.955 → 95.5%
        // size-weighted is (500M + 10K) / (1G + 10K) ≈ 0.5 → ~50%
        assert!(
            (49.0..=51.0).contains(&overall),
            "size-weighted expected ~50%, got {overall}%",
        );
    }

    #[test]
    fn overall_percent_empty_operation_is_none() {
        assert!(op(vec![]).overall_percent().is_none());
    }

    #[test]
    fn overall_percent_none_when_no_item_knows_its_total() {
        let items = vec![progress(500, None), progress(750, None)];
        assert!(op(items).overall_percent().is_none());
    }
}
