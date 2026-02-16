//! Pure functions for building SSH commands and parsing their textual output.
//!
//! No SSH or async dependencies — fully unit-testable.

use super::server_sync::{
    DatabaseEngine, DiscoveredDatabase, DiscoveredSite, FileEntry, SyncOptions,
};

// ---------------------------------------------------------------------------
// File listing
// ---------------------------------------------------------------------------

/// Build a `stat`-based command that outputs machine-parseable file info.
/// Each entry is one line: `type\tperms\towner\tgroup\tsize\tmtime\tname`
pub fn ls_command(path: &str) -> String {
    // Use stat --printf for machine-parseable output, one line per file.
    // %F = file type, %A = perms, %U = owner, %G = group, %s = size, %y = mtime, %n = name
    format!(
        r#"for f in {}/*; do [ -e "$f" ] && stat --printf='%F\t%A\t%U\t%G\t%s\t%y\t%n\n' "$f"; done 2>/dev/null"#,
        shell_escape(path)
    )
}

/// Fallback `ls -la` command for systems without GNU stat.
pub fn ls_command_fallback(path: &str) -> String {
    format!(
        "ls -la --time-style=long-iso {} 2>/dev/null",
        shell_escape(path)
    )
}

/// Parse output from `ls_command` (stat --printf format).
pub fn parse_stat_output(output: &str, base_path: &str) -> Vec<FileEntry> {
    let mut entries = Vec::new();
    for line in output.lines() {
        let parts: Vec<&str> = line.splitn(7, '\t').collect();
        if parts.len() < 7 {
            continue;
        }
        let file_type = parts[0];
        let permissions = parts[1].to_string();
        let owner = parts[2].to_string();
        let group = parts[3].to_string();
        let size: u64 = parts[4].parse().unwrap_or(0);
        let modified = Some(parts[5].to_string());
        let full_path = parts[6].to_string();

        let name = full_path
            .rsplit('/')
            .next()
            .unwrap_or(&full_path)
            .to_string();

        if name == "." || name == ".." {
            continue;
        }

        let is_dir = file_type.contains("directory");

        let path = if base_path == "/" {
            format!("/{}", name)
        } else {
            format!("{}/{}", base_path.trim_end_matches('/'), name)
        };

        entries.push(FileEntry {
            name,
            path,
            size,
            permissions,
            modified,
            is_dir,
            owner,
            group,
        });
    }
    // Sort: directories first, then alphabetical
    entries.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    entries
}

/// Parse output from `ls_command_fallback` (ls -la --time-style=long-iso format).
pub fn parse_ls_output(output: &str, base_path: &str) -> Vec<FileEntry> {
    let mut entries = Vec::new();
    for line in output.lines() {
        // Skip "total N" line
        if line.starts_with("total ") || line.is_empty() {
            continue;
        }
        // Format: drwxr-xr-x 2 user group 4096 2024-01-15 10:30 filename
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 8 {
            continue;
        }
        let permissions = parts[0].to_string();
        let owner = parts[2].to_string();
        let group = parts[3].to_string();
        let size: u64 = parts[4].parse().unwrap_or(0);
        let modified = Some(format!("{} {}", parts[5], parts[6]));
        // Name may contain spaces — rejoin from index 7 onward
        let name = parts[7..].join(" ");

        if name == "." || name == ".." {
            continue;
        }

        let is_dir = permissions.starts_with('d');

        let path = if base_path == "/" {
            format!("/{}", name)
        } else {
            format!("{}/{}", base_path.trim_end_matches('/'), name)
        };

        entries.push(FileEntry {
            name,
            path,
            size,
            permissions,
            modified,
            is_dir,
            owner,
            group,
        });
    }
    entries.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    entries
}

/// List files on the local machine using std::fs (no SSH needed).
pub fn list_local_files(path: &str) -> Vec<FileEntry> {
    let dir = match std::fs::read_dir(path) {
        Ok(d) => d,
        Err(_) => return Vec::new(),
    };
    let mut entries = Vec::new();
    for entry in dir.flatten() {
        let meta = match entry.metadata() {
            Ok(m) => m,
            Err(_) => continue,
        };
        let name = entry.file_name().to_string_lossy().to_string();
        if name == "." || name == ".." {
            continue;
        }
        let is_dir = meta.is_dir();
        let size = if is_dir { 0 } else { meta.len() };
        let entry_path = if path == "/" {
            format!("/{}", name)
        } else {
            format!("{}/{}", path.trim_end_matches('/'), name)
        };

        // Format permissions from mode (Unix)
        #[cfg(unix)]
        let permissions = {
            use std::os::unix::fs::MetadataExt;
            let mode = meta.mode();
            format_mode(mode, is_dir)
        };
        #[cfg(not(unix))]
        let permissions = if is_dir {
            "drwxr-xr-x".to_string()
        } else {
            "-rw-r--r--".to_string()
        };

        // Owner/group from uid/gid (Unix)
        #[cfg(unix)]
        let (owner, group) = {
            use std::os::unix::fs::MetadataExt;
            (meta.uid().to_string(), meta.gid().to_string())
        };
        #[cfg(not(unix))]
        let (owner, group) = ("user".to_string(), "user".to_string());

        let modified = meta.modified().ok().map(|t| {
            let dt: chrono::DateTime<chrono::Utc> = t.into();
            dt.format("%Y-%m-%d %H:%M:%S").to_string()
        });

        entries.push(FileEntry {
            name,
            path: entry_path,
            size,
            permissions,
            modified,
            is_dir,
            owner,
            group,
        });
    }
    entries.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    entries
}

#[cfg(unix)]
fn format_mode(mode: u32, is_dir: bool) -> String {
    let mut s = String::with_capacity(10);
    s.push(if is_dir { 'd' } else { '-' });
    for shift in [6, 3, 0] {
        let bits = (mode >> shift) & 0o7;
        s.push(if bits & 4 != 0 { 'r' } else { '-' });
        s.push(if bits & 2 != 0 { 'w' } else { '-' });
        s.push(if bits & 1 != 0 { 'x' } else { '-' });
    }
    s
}

// ---------------------------------------------------------------------------
// Nginx discovery
// ---------------------------------------------------------------------------

/// Build a command that prints all nginx site configs with `---FILE:path` markers.
pub fn nginx_discover_command() -> &'static str {
    r#"timeout 10 sh -c 'for f in /etc/nginx/sites-enabled/*; do [ -f "$f" ] && echo "---FILE:$f" && cat "$f"; done' 2>/dev/null"#
}

/// Parse concatenated nginx config output into discovered sites.
///
/// Parses per-server-block (not per-file) so that configs with multiple
/// `server { }` blocks (e.g. HTTP redirect + HTTPS) produce separate entries.
/// Skips commented-out lines and filters out default/catch-all server names
/// (`_`, `localhost`, empty).
pub fn parse_nginx_configs(output: &str) -> Vec<DiscoveredSite> {
    let mut sites = Vec::new();
    let file_blocks: Vec<&str> = output.split("---FILE:").collect();

    for block in file_blocks {
        let block = block.trim();
        if block.is_empty() {
            continue;
        }

        // First line is the file path
        let mut lines = block.lines();
        let config_path = match lines.next() {
            Some(p) => p.trim().to_string(),
            None => continue,
        };

        // Parse individual server { } blocks within this config file.
        let mut in_server = false;
        let mut brace_depth: i32 = 0;
        let mut server_name = String::new();
        let mut root = String::new();
        let mut listen_port: u16 = 80;
        let mut ssl = false;

        for line in lines {
            let trimmed = line.trim();

            // Skip comments and blank lines
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }

            // Detect start of a new server block (top-level only)
            if !in_server {
                let first_word = trimmed
                    .split(|c: char| c.is_whitespace() || c == '{')
                    .next()
                    .unwrap_or("");
                if first_word == "server" {
                    in_server = true;
                    brace_depth = 0;
                    server_name.clear();
                    root.clear();
                    listen_port = 80;
                    ssl = false;
                }
            }

            // Count braces
            for ch in trimmed.chars() {
                if ch == '{' {
                    brace_depth += 1;
                } else if ch == '}' {
                    brace_depth -= 1;
                    if brace_depth <= 0 && in_server {
                        // Server block closed — emit if valid
                        if !server_name.is_empty()
                            && server_name != "_"
                            && server_name != "localhost"
                            && server_name != "\"\""
                        {
                            // Deduplicate: skip if we already have this server_name
                            // from the same file (e.g. HTTP redirect + HTTPS main block)
                            let already = sites.iter().any(|s: &DiscoveredSite| {
                                s.config_path == config_path && s.server_name == server_name
                            });
                            if !already {
                                sites.push(DiscoveredSite {
                                    server_name: server_name.clone(),
                                    root: root.clone(),
                                    config_path: config_path.clone(),
                                    listen_port,
                                    ssl,
                                });
                            } else {
                                // Merge SSL flag into previous entry from this file
                                if ssl {
                                    if let Some(prev) = sites.iter_mut().find(|s| {
                                        s.config_path == config_path && s.server_name == server_name
                                    }) {
                                        prev.ssl = true;
                                        // Prefer the SSL port/root if the previous was a bare redirect
                                        if listen_port == 443 {
                                            prev.listen_port = 443;
                                        }
                                        if !root.is_empty() && prev.root.is_empty() {
                                            prev.root = root.clone();
                                        }
                                    }
                                }
                            }
                        }
                        in_server = false;
                    }
                }
            }

            if !in_server {
                continue;
            }

            // Parse directives (only at server-block depth 1, not in nested
            // location/if blocks)
            if brace_depth != 1 {
                continue;
            }

            let trimmed_semi = trimmed.trim_end_matches(';');

            if trimmed_semi.starts_with("server_name") {
                let name = trimmed_semi
                    .strip_prefix("server_name")
                    .unwrap_or("")
                    .split_whitespace()
                    .next()
                    .unwrap_or("")
                    .to_string();
                if !name.is_empty() {
                    server_name = name;
                }
            } else if trimmed_semi.starts_with("root") && !trimmed_semi.starts_with("root_") {
                root = trimmed_semi
                    .strip_prefix("root")
                    .unwrap_or("")
                    .trim()
                    .to_string();
            } else if trimmed_semi.starts_with("listen") {
                let listen_parts: Vec<&str> = trimmed_semi
                    .strip_prefix("listen")
                    .unwrap_or("")
                    .split_whitespace()
                    .collect();
                if let Some(port_str) = listen_parts.first() {
                    // Handle [::]:443 or 443 or 0.0.0.0:80
                    let port_part = port_str
                        .trim_start_matches("[::]:")
                        .trim_start_matches("0.0.0.0:");
                    if let Ok(p) = port_part.parse::<u16>() {
                        listen_port = p;
                    }
                }
                if listen_parts.contains(&"ssl") {
                    ssl = true;
                }
            } else if trimmed.contains("ssl_certificate")
                && !trimmed.contains("ssl_certificate_key")
            {
                ssl = true;
            }
        }
    }
    sites
}

// ---------------------------------------------------------------------------
// Database discovery
// ---------------------------------------------------------------------------

/// Build a MySQL discovery command. Outputs: `db_name\tsize_bytes\ttable_count`
pub fn mysql_discover_command(credentials: &str) -> String {
    // credentials might be "-u root -pPASSWORD" or empty for socket auth
    // timeout + /dev/null stdin prevents hanging on password prompts
    format!(
        r#"timeout 10 mysql {} --batch --skip-column-names -e "SELECT s.schema_name, IFNULL(SUM(t.data_length + t.index_length), 0) AS size_bytes, COUNT(t.table_name) AS table_count FROM information_schema.schemata s LEFT JOIN information_schema.tables t ON s.schema_name = t.table_schema WHERE s.schema_name NOT IN ('information_schema','mysql','performance_schema','sys') GROUP BY s.schema_name ORDER BY s.schema_name" < /dev/null 2>/dev/null"#,
        credentials
    )
}

/// Parse MySQL discovery output (tab-separated: name, size, table_count).
pub fn parse_mysql_discovery(output: &str) -> Vec<DiscoveredDatabase> {
    let mut databases = Vec::new();
    for line in output.lines() {
        let parts: Vec<&str> = line.split('\t').collect();
        if parts.len() < 3 {
            continue;
        }
        let name = parts[0].to_string();
        let size_bytes: Option<u64> = parts[1].parse().ok();
        let table_count: Option<u32> = parts[2].parse().ok();

        databases.push(DiscoveredDatabase {
            name,
            engine: DatabaseEngine::Mysql,
            size_bytes,
            table_count,
        });
    }
    databases
}

/// Build a PostgreSQL discovery command. Outputs: `db_name\tsize_bytes\ttable_count`
pub fn pg_discover_command(credentials: &str) -> String {
    // credentials might be "-U postgres" or "-U user -h host"
    // timeout + /dev/null stdin prevents hanging on password prompts
    format!(
        r#"timeout 10 psql {} --tuples-only --no-align --field-separator=$'\t' -c "SELECT d.datname, pg_database_size(d.datname), (SELECT COUNT(*) FROM information_schema.tables WHERE table_catalog = d.datname AND table_schema = 'public') FROM pg_database d WHERE d.datistemplate = false AND d.datname != 'postgres' ORDER BY d.datname" < /dev/null 2>/dev/null"#,
        credentials
    )
}

/// Parse PostgreSQL discovery output (tab-separated: name, size, table_count).
pub fn parse_pg_discovery(output: &str) -> Vec<DiscoveredDatabase> {
    let mut databases = Vec::new();
    for line in output.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let parts: Vec<&str> = line.split('\t').collect();
        if parts.len() < 3 {
            continue;
        }
        let name = parts[0].trim().to_string();
        let size_bytes: Option<u64> = parts[1].trim().parse().ok();
        let table_count: Option<u32> = parts[2].trim().parse().ok();

        databases.push(DiscoveredDatabase {
            name,
            engine: DatabaseEngine::Postgresql,
            size_bytes,
            table_count,
        });
    }
    databases
}

// ---------------------------------------------------------------------------
// Sync commands
// ---------------------------------------------------------------------------

/// Build an rsync command for directory sync.
pub fn rsync_command(
    source_path: &str,
    dest_user: &str,
    dest_host: &str,
    dest_path: &str,
    options: &SyncOptions,
    excludes: &[String],
) -> String {
    let mut cmd = String::from("rsync -avz --progress --info=progress2");

    if options.compress {
        // -z already added above
    }
    if options.dry_run {
        cmd.push_str(" --dry-run");
    }
    if options.delete_extra {
        cmd.push_str(" --delete");
    }
    if options.skip_existing {
        cmd.push_str(" --ignore-existing");
    }
    if let Some(bw) = options.bandwidth_limit {
        cmd.push_str(&format!(" --bwlimit={}", bw));
    }
    for pattern in excludes {
        cmd.push_str(&format!(" --exclude={}", shell_escape(pattern)));
    }

    cmd.push_str(&format!(
        " {} {}@{}:{}",
        shell_escape(source_path),
        dest_user,
        dest_host,
        shell_escape(dest_path)
    ));
    cmd
}

/// Build a MySQL database sync command (pipe via SSH).
pub fn mysql_sync_command(
    db: &str,
    src_creds: &str,
    dest_user: &str,
    dest_host: &str,
    dest_creds: &str,
    compress: bool,
) -> String {
    if compress {
        format!(
            "mysqldump {} {} --single-transaction --routines --triggers | gzip | ssh {}@{} 'gunzip | mysql {} {}'",
            src_creds, shell_escape(db), dest_user, dest_host, dest_creds, shell_escape(db)
        )
    } else {
        format!(
            "mysqldump {} {} --single-transaction --routines --triggers | ssh {}@{} 'mysql {} {}'",
            src_creds,
            shell_escape(db),
            dest_user,
            dest_host,
            dest_creds,
            shell_escape(db)
        )
    }
}

/// Build a PostgreSQL database sync command (pipe via SSH).
pub fn pg_sync_command(
    db: &str,
    src_creds: &str,
    dest_user: &str,
    dest_host: &str,
    dest_creds: &str,
    compress: bool,
) -> String {
    if compress {
        format!(
            "pg_dump {} {} | gzip | ssh {}@{} 'gunzip | psql {} {}'",
            src_creds,
            shell_escape(db),
            dest_user,
            dest_host,
            dest_creds,
            shell_escape(db)
        )
    } else {
        format!(
            "pg_dump {} {} | ssh {}@{} 'psql {} {}'",
            src_creds,
            shell_escape(db),
            dest_user,
            dest_host,
            dest_creds,
            shell_escape(db)
        )
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Shell-escape a string using single quotes.
fn shell_escape(s: &str) -> String {
    let escaped = s.replace('\'', "'\\''");
    format!("'{}'", escaped)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_stat_output() {
        let output = "directory\tdrwxr-xr-x\troot\troot\t4096\t2024-01-15 10:30:00.000\t/var/www/html\nregular file\t-rw-r--r--\twww-data\twww-data\t1024\t2024-01-14 09:00:00.000\t/var/www/index.html";
        let entries = parse_stat_output(output, "/var/www");
        assert_eq!(entries.len(), 2);
        assert!(entries[0].is_dir); // directories first
        assert_eq!(entries[0].name, "html");
        assert_eq!(entries[1].name, "index.html");
        assert_eq!(entries[1].size, 1024);
    }

    #[test]
    fn test_parse_ls_output() {
        let output = "total 8\ndrwxr-xr-x 2 root root 4096 2024-01-15 10:30 html\n-rw-r--r-- 1 www www 512 2024-01-14 09:00 index.html";
        let entries = parse_ls_output(output, "/var/www");
        assert_eq!(entries.len(), 2);
        assert!(entries[0].is_dir);
        assert_eq!(entries[0].name, "html");
    }

    #[test]
    fn test_parse_nginx_configs() {
        let output = r#"---FILE:/etc/nginx/sites-enabled/example.conf
server {
    listen 443 ssl;
    server_name example.com;
    root /var/www/example;
    ssl_certificate /etc/ssl/example.pem;
}
---FILE:/etc/nginx/sites-enabled/api.conf
server {
    listen 80;
    server_name api.example.com;
    root /var/www/api;
}"#;
        let sites = parse_nginx_configs(output);
        assert_eq!(sites.len(), 2);
        assert_eq!(sites[0].server_name, "example.com");
        assert!(sites[0].ssl);
        assert_eq!(sites[0].listen_port, 443);
        assert_eq!(sites[1].server_name, "api.example.com");
        assert!(!sites[1].ssl);
    }

    #[test]
    fn test_parse_mysql_discovery() {
        let output = "mydb\t104857600\t42\nother_db\t52428800\t15";
        let dbs = parse_mysql_discovery(output);
        assert_eq!(dbs.len(), 2);
        assert_eq!(dbs[0].name, "mydb");
        assert_eq!(dbs[0].engine, DatabaseEngine::Mysql);
        assert_eq!(dbs[0].size_bytes, Some(104857600));
        assert_eq!(dbs[0].table_count, Some(42));
    }

    #[test]
    fn test_parse_pg_discovery() {
        let output = " mydb\t104857600\t10\n other_db\t52428800\t5\n";
        let dbs = parse_pg_discovery(output);
        assert_eq!(dbs.len(), 2);
        assert_eq!(dbs[0].name, "mydb");
        assert_eq!(dbs[0].engine, DatabaseEngine::Postgresql);
    }

    #[test]
    fn test_rsync_command() {
        let opts = SyncOptions {
            compress: true,
            dry_run: true,
            delete_extra: false,
            bandwidth_limit: Some(1000),
            skip_existing: false,
        };
        let cmd = rsync_command(
            "/var/www",
            "deploy",
            "server.com",
            "/var/www",
            &opts,
            &["*.log".into()],
        );
        assert!(cmd.contains("--dry-run"));
        assert!(cmd.contains("--bwlimit=1000"));
        assert!(cmd.contains("--exclude="));
        assert!(cmd.contains("deploy@server.com"));
    }
}
