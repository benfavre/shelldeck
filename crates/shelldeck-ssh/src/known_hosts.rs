use std::fs;
use std::io::Write;
use std::path::PathBuf;

/// Result of checking a server key against known_hosts.
#[derive(Debug)]
pub enum KnownHostResult {
    /// Key matches an existing entry.
    Match,
    /// Host exists in known_hosts but the key is different (possible MITM).
    Mismatch,
    /// Host not found in known_hosts (new host, TOFU).
    NotFound,
}

/// Get the path to ~/.ssh/known_hosts.
fn known_hosts_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".into());
    PathBuf::from(home).join(".ssh").join("known_hosts")
}

/// Check a server key against ~/.ssh/known_hosts.
///
/// `hostname` is the hostname (and optionally port in `[host]:port` format for non-22 ports).
/// `key_type` is e.g. "ssh-ed25519", "ssh-rsa", etc.
/// `key_base64` is the base64-encoded public key data.
pub fn check_known_host(
    hostname: &str,
    port: u16,
    key_type: &str,
    key_base64: &str,
) -> KnownHostResult {
    let path = known_hosts_path();
    let contents = match fs::read_to_string(&path) {
        Ok(c) => c,
        Err(_) => return KnownHostResult::NotFound,
    };

    // Build the host patterns to match against.
    // Standard SSH uses bare hostname for port 22, [hostname]:port otherwise.
    let host_pattern = if port == 22 {
        hostname.to_string()
    } else {
        format!("[{}]:{}", hostname, port)
    };

    let mut host_seen = false;

    for line in contents.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        // Skip hashed entries (|1|...) — we can't reverse-check them without
        // the salt, and implementing HMAC-SHA1 for this is out of scope.
        // We'll just not match on them.
        if line.starts_with('|') {
            continue;
        }

        // Format: hostnames keytype base64key [comment]
        let mut parts = line.split_whitespace();
        let hosts_field = match parts.next() {
            Some(h) => h,
            None => continue,
        };
        let line_key_type = match parts.next() {
            Some(kt) => kt,
            None => continue,
        };
        let line_key_data = match parts.next() {
            Some(kd) => kd,
            None => continue,
        };

        // Check if this line matches our hostname
        let host_matches = hosts_field
            .split(',')
            .any(|h| h == host_pattern || h == hostname);

        if !host_matches {
            continue;
        }

        host_seen = true;

        // Host matches — check the key
        if line_key_type == key_type && line_key_data == key_base64 {
            return KnownHostResult::Match;
        }
    }

    if host_seen {
        KnownHostResult::Mismatch
    } else {
        KnownHostResult::NotFound
    }
}

/// Append a new entry to ~/.ssh/known_hosts (TOFU).
pub fn add_known_host(hostname: &str, port: u16, key_type: &str, key_base64: &str) {
    let path = known_hosts_path();

    // Ensure ~/.ssh directory exists
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }

    let host_entry = if port == 22 {
        hostname.to_string()
    } else {
        format!("[{}]:{}", hostname, port)
    };

    let line = format!("{} {} {}\n", host_entry, key_type, key_base64);

    match fs::OpenOptions::new().create(true).append(true).open(&path) {
        Ok(mut file) => {
            if let Err(e) = file.write_all(line.as_bytes()) {
                tracing::warn!("Failed to write to known_hosts: {}", e);
            } else {
                tracing::info!("Added {} to known_hosts (TOFU)", host_entry);
            }
        }
        Err(e) => {
            tracing::warn!("Failed to open known_hosts for writing: {}", e);
        }
    }
}
