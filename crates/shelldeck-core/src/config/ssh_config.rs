use crate::error::{Result, ShellDeckError};
use crate::models::{Connection, ConnectionSource, ConnectionStatus};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use uuid::Uuid;

/// Parse the default ~/.ssh/config file.
pub fn parse_ssh_config() -> Result<Vec<Connection>> {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
    let config_path = PathBuf::from(home).join(".ssh").join("config");
    if !config_path.exists() {
        return Ok(Vec::new());
    }
    parse_ssh_config_file(&config_path)
}

/// Parse a specific SSH config file into Connection structs.
pub fn parse_ssh_config_file(path: &Path) -> Result<Vec<Connection>> {
    let content = std::fs::read_to_string(path).map_err(|e| {
        ShellDeckError::SshConfigParse(format!("Failed to read {}: {}", path.display(), e))
    })?;

    // Use the ssh2_config crate for basic field extraction
    let mut reader = std::io::BufReader::new(content.as_bytes());
    let ssh_config = ssh2_config::SshConfig::default()
        .parse(&mut reader, ssh2_config::ParseRule::ALLOW_UNKNOWN_FIELDS)
        .map_err(|e| ShellDeckError::SshConfigParse(format!("Parse error: {}", e)))?;

    // Also do a manual parse for fields not exposed by ssh2_config
    // (ProxyJump, ForwardAgent, LocalForward, RemoteForward)
    let extra_fields = parse_extra_fields(&content);

    let mut connections = Vec::new();

    for host in ssh_config.get_hosts() {
        // Skip hosts with wildcard-only patterns
        let aliases: Vec<String> = host
            .pattern
            .iter()
            .filter(|clause| !clause.negated)
            .map(|clause| clause.pattern.clone())
            .filter(|p| !is_wildcard_only(p))
            .collect();

        if aliases.is_empty() {
            continue;
        }

        let alias = aliases[0].clone();
        let params = &host.params;

        let hostname = params.host_name.clone().unwrap_or_else(|| alias.clone());
        let user = params
            .user
            .clone()
            .unwrap_or_else(|| whoami().unwrap_or_else(|| "root".to_string()));
        let port = params.port.unwrap_or(22);
        let identity_file = params
            .identity_file
            .as_ref()
            .and_then(|files| files.first().cloned())
            .map(|p| expand_tilde(&p));

        // Get extra fields from manual parse
        let extras = extra_fields.get(&alias);
        let proxy_jump = extras.and_then(|e| e.proxy_jump.clone());
        let forward_agent = extras.map(|e| e.forward_agent).unwrap_or(false);

        let conn = Connection {
            id: Uuid::new_v4(),
            alias,
            hostname,
            port,
            user,
            identity_file,
            proxy_jump,
            group: None,
            tags: Vec::new(),
            auto_forwards: Vec::new(),
            auto_scripts: Vec::new(),
            source: ConnectionSource::SshConfig,
            forward_agent,
            status: ConnectionStatus::Disconnected,
        };

        connections.push(conn);
    }

    Ok(connections)
}

/// Returns true if the pattern is wildcard-only (e.g., "*", "?", "*.*")
fn is_wildcard_only(pattern: &str) -> bool {
    pattern.chars().all(|c| c == '*' || c == '?' || c == '.')
}

/// Expand ~ in paths to the user's home directory.
fn expand_tilde(path: &Path) -> PathBuf {
    let s = path.to_string_lossy();
    if s.starts_with("~/") {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
        PathBuf::from(home).join(&s[2..])
    } else {
        path.to_path_buf()
    }
}

/// Get the current username.
fn whoami() -> Option<String> {
    std::env::var("USER")
        .or_else(|_| std::env::var("LOGNAME"))
        .ok()
}

/// Extra fields per host that ssh2_config doesn't expose.
#[derive(Debug, Default)]
struct ExtraHostFields {
    proxy_jump: Option<String>,
    forward_agent: bool,
    local_forwards: Vec<(String, u16, String, u16)>, // (local_host, local_port, remote_host, remote_port)
    remote_forwards: Vec<(String, u16, String, u16)>,
}

/// Manually parse SSH config for fields not covered by ssh2_config.
fn parse_extra_fields(content: &str) -> HashMap<String, ExtraHostFields> {
    let mut result: HashMap<String, ExtraHostFields> = HashMap::new();
    let mut current_hosts: Vec<String> = Vec::new();

    for line in content.lines() {
        let trimmed = line.trim();

        // Skip empty lines and comments
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        // Detect Host blocks
        if let Some(rest) = strip_keyword(trimmed, "Host") {
            current_hosts = rest
                .split_whitespace()
                .filter(|h| !is_wildcard_only(h))
                .map(|h| h.to_string())
                .collect();
            for host in &current_hosts {
                result.entry(host.clone()).or_default();
            }
            continue;
        }

        // Skip Match blocks
        if strip_keyword(trimmed, "Match").is_some() {
            current_hosts.clear();
            continue;
        }

        if current_hosts.is_empty() {
            continue;
        }

        // ProxyJump
        if let Some(value) = strip_keyword(trimmed, "ProxyJump") {
            for host in &current_hosts {
                if let Some(fields) = result.get_mut(host) {
                    fields.proxy_jump = Some(value.to_string());
                }
            }
            continue;
        }

        // ForwardAgent
        if let Some(value) = strip_keyword(trimmed, "ForwardAgent") {
            let enabled = value.eq_ignore_ascii_case("yes");
            for host in &current_hosts {
                if let Some(fields) = result.get_mut(host) {
                    fields.forward_agent = enabled;
                }
            }
            continue;
        }

        // LocalForward: LocalForward [bind_address:]port host:hostport
        if let Some(value) = strip_keyword(trimmed, "LocalForward") {
            if let Some(fwd) = parse_forward_directive(value) {
                for host in &current_hosts {
                    if let Some(fields) = result.get_mut(host) {
                        fields.local_forwards.push(fwd.clone());
                    }
                }
            }
            continue;
        }

        // RemoteForward: RemoteForward [bind_address:]port host:hostport
        if let Some(value) = strip_keyword(trimmed, "RemoteForward") {
            if let Some(fwd) = parse_forward_directive(value) {
                for host in &current_hosts {
                    if let Some(fields) = result.get_mut(host) {
                        fields.remote_forwards.push(fwd.clone());
                    }
                }
            }
            continue;
        }
    }

    result
}

/// Strip a keyword (case-insensitive) from a line and return the value part.
fn strip_keyword<'a>(line: &'a str, keyword: &str) -> Option<&'a str> {
    // Handle both "Keyword value" and "Keyword=value"
    let lower = line.to_lowercase();
    let kw_lower = keyword.to_lowercase();

    if lower.starts_with(&kw_lower) {
        let rest = &line[keyword.len()..];
        if rest.starts_with('=') {
            Some(rest[1..].trim())
        } else if rest.starts_with(' ') || rest.starts_with('\t') {
            Some(rest.trim())
        } else {
            None
        }
    } else {
        None
    }
}

/// Parse a forward directive value like "8080 localhost:80" or "127.0.0.1:8080 localhost:80".
fn parse_forward_directive(value: &str) -> Option<(String, u16, String, u16)> {
    let parts: Vec<&str> = value.split_whitespace().collect();
    if parts.len() != 2 {
        return None;
    }

    let (local_host, local_port) = parse_host_port(parts[0])?;
    let (remote_host, remote_port) = parse_host_port(parts[1])?;

    Some((local_host, local_port, remote_host, remote_port))
}

/// Parse "host:port" or just "port" (defaulting host to "127.0.0.1").
fn parse_host_port(s: &str) -> Option<(String, u16)> {
    if let Some(idx) = s.rfind(':') {
        let host = &s[..idx];
        let port: u16 = s[idx + 1..].parse().ok()?;
        Some((host.to_string(), port))
    } else {
        let port: u16 = s.parse().ok()?;
        Some(("127.0.0.1".to_string(), port))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_wildcard_only() {
        assert!(is_wildcard_only("*"));
        assert!(is_wildcard_only("*.*"));
        assert!(is_wildcard_only("?"));
        assert!(!is_wildcard_only("myhost"));
        assert!(!is_wildcard_only("*.example.com"));
    }

    #[test]
    fn test_parse_host_port() {
        assert_eq!(
            parse_host_port("8080"),
            Some(("127.0.0.1".to_string(), 8080))
        );
        assert_eq!(
            parse_host_port("localhost:80"),
            Some(("localhost".to_string(), 80))
        );
        assert_eq!(
            parse_host_port("192.168.1.1:3306"),
            Some(("192.168.1.1".to_string(), 3306))
        );
        assert_eq!(parse_host_port("not_a_port"), None);
    }

    #[test]
    fn test_strip_keyword() {
        assert_eq!(strip_keyword("Host myserver", "Host"), Some("myserver"));
        assert_eq!(strip_keyword("HostName 1.2.3.4", "HostName"), Some("1.2.3.4"));
        assert_eq!(strip_keyword("Port 22", "Port"), Some("22"));
        assert_eq!(strip_keyword("ProxyJump bastion", "ProxyJump"), Some("bastion"));
        assert_eq!(strip_keyword("ForwardAgent yes", "ForwardAgent"), Some("yes"));
        assert_eq!(strip_keyword("Host=myserver", "Host"), Some("myserver"));
        assert_eq!(strip_keyword("Something else", "Host"), None);
    }

    #[test]
    fn test_parse_forward_directive() {
        assert_eq!(
            parse_forward_directive("8080 localhost:80"),
            Some(("127.0.0.1".to_string(), 8080, "localhost".to_string(), 80))
        );
        assert_eq!(
            parse_forward_directive("127.0.0.1:3306 db.internal:3306"),
            Some(("127.0.0.1".to_string(), 3306, "db.internal".to_string(), 3306))
        );
    }

    #[test]
    fn test_parse_extra_fields() {
        let config = r#"
Host bastion
    HostName bastion.example.com
    User admin
    ForwardAgent yes

Host webserver
    HostName 10.0.0.5
    User deploy
    ProxyJump bastion
    LocalForward 8080 localhost:80
    RemoteForward 9222 127.0.0.1:9222
"#;
        let extras = parse_extra_fields(config);

        let bastion = extras.get("bastion").unwrap();
        assert!(bastion.forward_agent);
        assert!(bastion.proxy_jump.is_none());

        let web = extras.get("webserver").unwrap();
        assert_eq!(web.proxy_jump.as_deref(), Some("bastion"));
        assert!(!web.forward_agent);
        assert_eq!(web.local_forwards.len(), 1);
        assert_eq!(web.remote_forwards.len(), 1);
    }

    #[test]
    fn test_expand_tilde() {
        let path = PathBuf::from("~/.ssh/id_rsa");
        let expanded = expand_tilde(&path);
        assert!(!expanded.to_string_lossy().starts_with("~"));

        let abs = PathBuf::from("/etc/ssh/id_rsa");
        assert_eq!(expand_tilde(&abs), abs);
    }
}
