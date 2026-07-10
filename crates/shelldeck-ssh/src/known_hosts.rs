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

/// Build the host pattern SSH uses in a known_hosts entry:
/// `hostname` for port 22, `[hostname]:port` otherwise.
fn host_pattern(hostname: &str, port: u16) -> String {
    if port == 22 {
        hostname.to_string()
    } else {
        format!("[{}]:{}", hostname, port)
    }
}

/// Pure check against an already-loaded `known_hosts` file body. Extracted
/// so the parser is testable without touching `$HOME` (process-global env
/// mutation would race with parallel tests).
pub fn check_known_host_in(
    contents: &str,
    hostname: &str,
    port: u16,
    key_type: &str,
    key_base64: &str,
) -> KnownHostResult {
    let pattern = host_pattern(hostname, port);
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
            .any(|h| h == pattern || h == hostname);

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
    check_known_host_in(&contents, hostname, port, key_type, key_base64)
}

/// Format the line that would be appended to `known_hosts` for the given
/// (host, port, key). Extracted so the exact wire format — including the
/// trailing newline and `[host]:port` bracketing rule — can be tested
/// without touching the filesystem.
pub fn build_known_host_line(
    hostname: &str,
    port: u16,
    key_type: &str,
    key_base64: &str,
) -> String {
    format!(
        "{} {} {}\n",
        host_pattern(hostname, port),
        key_type,
        key_base64,
    )
}

/// Append a new entry to `path` (TOFU). Extracted so tests can exercise
/// the append semantics without mutating `$HOME`. Creates parent dirs
/// if missing and never truncates an existing file.
pub fn add_known_host_to(
    path: &std::path::Path,
    hostname: &str,
    port: u16,
    key_type: &str,
    key_base64: &str,
) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let line = build_known_host_line(hostname, port, key_type, key_base64);
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    file.write_all(line.as_bytes())?;
    Ok(())
}

/// Append a new entry to ~/.ssh/known_hosts (TOFU).
pub fn add_known_host(hostname: &str, port: u16, key_type: &str, key_base64: &str) {
    let path = known_hosts_path();

    // Ensure ~/.ssh directory exists
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }

    let host_entry = host_pattern(hostname, port);
    match add_known_host_to(&path, hostname, port, key_type, key_base64) {
        Ok(_) => tracing::info!("Added {} to known_hosts (TOFU)", host_entry),
        Err(e) => tracing::warn!("Failed to write to known_hosts: {}", e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // SDTEST-580 — Match against a plain hostname entry.
    #[test]
    fn match_on_plain_hostname_entry() {
        let contents = "\
example.com ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAA
other.com ssh-rsa AAAAB3NzaC1yc2EAAAA
";
        let r = check_known_host_in(
            contents,
            "example.com",
            22,
            "ssh-ed25519",
            "AAAAC3NzaC1lZDI1NTE5AAAA",
        );
        assert!(matches!(r, KnownHostResult::Match));
    }

    // SDTEST-581 — Mismatch: host present, key differs (potential MITM).
    // Security-critical: SSH client MUST NOT connect on a Mismatch.
    #[test]
    fn mismatch_when_host_present_but_key_differs() {
        let contents = "example.com ssh-ed25519 EXPECTED_KEY\n";
        let r = check_known_host_in(contents, "example.com", 22, "ssh-ed25519", "WRONG_KEY");
        assert!(
            matches!(r, KnownHostResult::Mismatch),
            "MITM sensor — got {r:?}",
        );
    }

    // SDTEST-581bis — Mismatch when the key type differs but host is present.
    #[test]
    fn mismatch_when_key_type_differs() {
        let contents = "example.com ssh-ed25519 K\n";
        let r = check_known_host_in(contents, "example.com", 22, "ssh-rsa", "K");
        assert!(matches!(r, KnownHostResult::Mismatch));
    }

    // SDTEST-582 — Fresh host: not in file at all.
    #[test]
    fn not_found_for_unknown_host() {
        let contents = "other.com ssh-ed25519 K\n";
        let r = check_known_host_in(contents, "example.com", 22, "ssh-ed25519", "K");
        assert!(matches!(r, KnownHostResult::NotFound));
    }

    // Non-22 port: entry must use `[host]:port` bracketing.
    #[test]
    fn non_default_port_uses_bracketed_pattern() {
        let contents = "[example.com]:2222 ssh-ed25519 K\n";
        let r = check_known_host_in(contents, "example.com", 2222, "ssh-ed25519", "K");
        assert!(matches!(r, KnownHostResult::Match));

        // Same host on port 22 must NOT match a [host]:2222 entry.
        let r = check_known_host_in(contents, "example.com", 22, "ssh-ed25519", "K");
        assert!(matches!(r, KnownHostResult::NotFound));
    }

    // Comma-separated host list: match on any alias.
    #[test]
    fn multi_host_alias_line_matches_each_alias() {
        let contents = "example.com,ex,10.0.0.5 ssh-ed25519 K\n";
        for host in ["example.com", "ex", "10.0.0.5"] {
            let r = check_known_host_in(contents, host, 22, "ssh-ed25519", "K");
            assert!(matches!(r, KnownHostResult::Match), "alias {host}");
        }
    }

    // SDTEST-583 — Hashed hostname entries (`|1|...`) are DELIBERATELY
    // ignored by our parser (impl comment says "out of scope"). Ensure
    // that policy still holds: a hashed entry for `example.com` must
    // return NotFound (not accidentally Match against unhashed key
    // material, which would be a silent trust break).
    #[test]
    fn hashed_entries_are_skipped() {
        // The salt/hash is fake — the parser must skip the whole line
        // without attempting to decode it.
        let contents = "|1|SALT=|HASH= ssh-ed25519 K\n";
        let r = check_known_host_in(contents, "example.com", 22, "ssh-ed25519", "K");
        assert!(matches!(r, KnownHostResult::NotFound));
    }

    // Comments (starting with `#`) and blank lines are silently ignored.
    #[test]
    fn comments_and_blank_lines_are_ignored() {
        let contents = "\
# managed by shelldeck

example.com ssh-ed25519 K
# trailing comment
";
        let r = check_known_host_in(contents, "example.com", 22, "ssh-ed25519", "K");
        assert!(matches!(r, KnownHostResult::Match));
    }

    // Ragged lines (fewer than 3 fields) never panic and don't cause a
    // false Match.
    #[test]
    fn ragged_lines_do_not_panic_or_false_match() {
        let contents = "\
example.com
example.com ssh-ed25519
example.com ssh-ed25519 K
";
        let r = check_known_host_in(contents, "example.com", 22, "ssh-ed25519", "K");
        assert!(matches!(r, KnownHostResult::Match));
    }

    // SDTEST-584 — an empty / missing known_hosts file returns NotFound
    // (never a false Match). Contract for the TOFU path — no file means
    // "first time we see this host", not "trust nothing".
    #[test]
    fn empty_known_hosts_returns_not_found() {
        let r = check_known_host_in("", "example.com", 22, "ssh-ed25519", "K");
        assert!(matches!(r, KnownHostResult::NotFound));
    }

    // SDTEST-585 — build_known_host_line format contract. This is the
    // exact string appended to `known_hosts` on TOFU; a rename here
    // silently breaks compat with `ssh` reading the file back.
    #[test]
    fn build_line_uses_bare_hostname_for_port_22() {
        let l = build_known_host_line("example.com", 22, "ssh-ed25519", "KEY_B64");
        assert_eq!(l, "example.com ssh-ed25519 KEY_B64\n");
    }

    #[test]
    fn build_line_brackets_hostname_for_non_default_port() {
        let l = build_known_host_line("example.com", 2222, "ssh-rsa", "KEY_B64");
        assert_eq!(l, "[example.com]:2222 ssh-rsa KEY_B64\n");
    }

    // SDTEST-585bis — `add_known_host_to` appends without overwriting.
    // Load-bearing "trust never silently vanishes" property.
    #[test]
    fn add_known_host_to_appends_never_overwrites() {
        let tmp = std::env::temp_dir().join(format!(
            "shelldeck-kh-append-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
        ));
        let path = tmp.join("known_hosts");
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(&path, "existing.com ssh-ed25519 EXISTING_KEY\n").unwrap();

        add_known_host_to(&path, "new.example.com", 22, "ssh-ed25519", "NEW_KEY")
            .expect("append succeeds");
        // Second write to prove append is idempotent-safe (no truncation).
        add_known_host_to(&path, "another.example.com", 2222, "ssh-rsa", "K2")
            .expect("second append succeeds");

        let after = std::fs::read_to_string(&path).unwrap();
        assert!(
            after.contains("existing.com ssh-ed25519 EXISTING_KEY"),
            "prior entry lost: {after:?}",
        );
        assert!(
            after.contains("new.example.com ssh-ed25519 NEW_KEY"),
            "new entry missing: {after:?}",
        );
        assert!(
            after.contains("[another.example.com]:2222 ssh-rsa K2"),
            "bracketed non-22 entry missing: {after:?}",
        );

        std::fs::remove_dir_all(&tmp).ok();
    }

    // `add_known_host_to` creates the parent directory (~/.ssh) if it's
    // missing — a fresh install's first connection must not silently
    // fail because the .ssh dir doesn't exist yet.
    #[test]
    fn add_known_host_to_creates_parent_directory() {
        let tmp = std::env::temp_dir().join(format!(
            "shelldeck-kh-mkdir-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
        ));
        let path = tmp.join(".ssh").join("known_hosts");
        // Parent dir does NOT exist yet.
        assert!(!path.parent().unwrap().exists());

        add_known_host_to(&path, "first.example.com", 22, "ssh-ed25519", "K")
            .expect("must create parent + file");

        let after = std::fs::read_to_string(&path).unwrap();
        assert!(after.contains("first.example.com"));

        std::fs::remove_dir_all(&tmp).ok();
    }
}
