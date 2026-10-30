//! Read-only, bounded discovery of native CLI conversations on an SSH target.
//! Discovery never executes an agent or reads credential files. Selecting one
//! still requires a separately validated, exact-target resume binding.
use crate::agent_runtime::AgentProvider;
use crate::{Result, ShellDeckError};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

pub const MAX_REMOTE_SESSION_INVENTORY_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_REMOTE_SESSIONS: usize = 600;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RemoteAgentSession {
    pub provider: AgentProvider,
    pub session_id: String,
    pub workdir: String,
    pub title: String,
    pub updated_at_ms: u64,
    pub model: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RemoteAgentSessionInventory {
    pub sessions: Vec<RemoteAgentSession>,
    pub warnings: Vec<String>,
    pub truncated: bool,
    pub next_cursor: Option<String>,
}

/// POSIX command for the existing authenticated SSH exec channel. The static
/// quoted heredoc has no user-controlled substitutions and writes no files.
/// Callers also impose an SSH timeout and bound captured output.
pub fn discovery_command() -> String {
    format!(
        "python3 - <<'SHELLDECK_REMOTE_SESSIONS_PY'\n{}\nSHELLDECK_REMOTE_SESSIONS_PY",
        include_str!("agent_remote_discovery.py")
    )
}

/// Continue a bounded metadata scan without silently losing older sessions.
/// The opaque cursor contains only URL-safe base64 data, never shell syntax.
pub fn discovery_page_command(cursor: Option<&str>) -> Result<String> {
    let Some(cursor) = cursor else {
        return Ok(discovery_command());
    };
    if !valid_cursor(cursor) {
        return Err(invalid());
    }
    Ok(format!(
        "python3 - '{cursor}' <<'SHELLDECK_REMOTE_SESSIONS_PY'\n{}\nSHELLDECK_REMOTE_SESSIONS_PY",
        include_str!("agent_remote_discovery.py")
    ))
}

fn valid_cursor(cursor: &str) -> bool {
    !cursor.is_empty()
        && cursor.len() <= 2048
        && cursor
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"_=-".contains(&c))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireError {
    version: u8,
    error: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireInventory {
    version: u8,
    sessions: Vec<RemoteAgentSession>,
    warnings: Vec<String>,
    truncated: bool,
    #[serde(default)]
    next_cursor: Option<String>,
}

fn invalid() -> ShellDeckError {
    ShellDeckError::Config("Invalid remote session inventory; refresh the server and retry".into())
}

/// Validate the untrusted remote response before it can supply a resume ID or
/// workspace path. Do not salvage malformed identities by trimming them.
pub fn parse_inventory(output: &str) -> Result<RemoteAgentSessionInventory> {
    if output.len() > MAX_REMOTE_SESSION_INVENTORY_BYTES {
        return Err(invalid());
    }
    if let Ok(error) = serde_json::from_str::<WireError>(output) {
        if error.version == 1
            && !error.error.is_empty()
            && error.error.len() <= 512
            && !error.error.chars().any(char::is_control)
        {
            return Err(ShellDeckError::Config(error.error));
        }
        return Err(invalid());
    }
    let wire: WireInventory = serde_json::from_str(output).map_err(|_| invalid())?;
    if wire
        .next_cursor
        .as_deref()
        .is_some_and(|cursor| !valid_cursor(cursor))
        || (wire.next_cursor.is_some() && !wire.truncated)
    {
        return Err(invalid());
    }
    if wire.version != 1 || wire.sessions.len() > MAX_REMOTE_SESSIONS || wire.warnings.len() > 32 {
        return Err(invalid());
    }
    let mut keys = HashSet::new();
    for row in &wire.sessions {
        if !matches!(
            row.provider,
            AgentProvider::Claude | AgentProvider::Codex | AgentProvider::Jcode
        ) || row.session_id.is_empty()
            || row.session_id.len() > 256
            || row.session_id.starts_with('-')
            || !row
                .session_id
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"_-.".contains(&c))
            || !row.workdir.starts_with('/')
            || row.workdir.split('/').any(|part| part == "..")
            || row.workdir.len() > 4096
            || row.workdir.chars().any(char::is_control)
            || row.title.is_empty()
            || row.title.len() > 256
            || row.title.chars().any(char::is_control)
            || row.model.as_ref().is_some_and(|model| {
                model.is_empty() || model.len() > 256 || model.chars().any(char::is_control)
            })
            || !keys.insert((row.provider.binary(), row.session_id.as_str()))
        {
            return Err(invalid());
        }
    }
    if wire
        .warnings
        .iter()
        .any(|warning| warning.len() > 512 || warning.chars().any(char::is_control))
    {
        return Err(invalid());
    }
    let mut sessions = wire.sessions;
    sessions.sort_by(|a, b| {
        b.updated_at_ms
            .cmp(&a.updated_at_ms)
            .then_with(|| a.session_id.cmp(&b.session_id))
    });
    Ok(RemoteAgentSessionInventory {
        sessions,
        warnings: wire.warnings,
        truncated: wire.truncated,
        next_cursor: wire.next_cursor,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn row(provider: &str, id: &str, updated: u64) -> serde_json::Value {
        json!({"provider":provider,"session_id":id,"workdir":"/home/test/work space","title":"Investigate reconnect","updated_at_ms":updated,"model":null})
    }

    // SDTEST-2823 / SDUC-564
    #[test]
    fn discovery_pages_validate_opaque_cursor_and_surface_refresh_errors() {
        assert_eq!(discovery_page_command(None).unwrap(), discovery_command());
        assert!(discovery_page_command(Some("eyJvZmZzZXQiOjIwMH0="))
            .unwrap()
            .starts_with("python3 - 'eyJvZmZzZXQiOjIwMH0='"));
        for cursor in ["", "'; touch /tmp/unsafe; '", "a\nb", "a/b"] {
            assert!(discovery_page_command(Some(cursor)).is_err());
        }
        let value = json!({"version":1,"sessions":[],"warnings":[],"truncated":true,"next_cursor":"opaque_cursor="});
        assert_eq!(
            parse_inventory(&value.to_string())
                .unwrap()
                .next_cursor
                .as_deref(),
            Some("opaque_cursor=")
        );
        let mut invalid_value = value.clone();
        invalid_value["next_cursor"] = json!("not shell safe!");
        assert!(parse_inventory(&invalid_value.to_string()).is_err());
        invalid_value = value;
        invalid_value["truncated"] = json!(false);
        assert!(parse_inventory(&invalid_value.to_string()).is_err());
        assert!(parse_inventory(
            r#"{"version":1,"error":"Remote session list changed. Refresh the server."}"#
        )
        .unwrap_err()
        .to_string()
        .contains("Refresh the server"));
    }

    // SDTEST-2806 / SDUC-562
    #[test]
    fn remote_inventory_preserves_provider_identity_and_partial_discovery() {
        let wire = json!({"version":1,"sessions":[row("claude","same-id",1),row("codex","same-id",3),row("jcode","session_fox_1_abcd",2)],"warnings":["Jcode: scan limit reached"],"truncated":true});
        let inventory = parse_inventory(&wire.to_string()).unwrap();
        assert_eq!(inventory.sessions[0].provider, AgentProvider::Codex);
        assert_eq!(inventory.sessions[2].provider, AgentProvider::Claude);
        assert_eq!(inventory.sessions[1].session_id, "session_fox_1_abcd");
        assert!(inventory.truncated);
        assert_eq!(inventory.warnings.len(), 1);
        assert_eq!(inventory.sessions[0].workdir, "/home/test/work space");
    }

    // SDTEST-2807 / SDUC-562
    #[test]
    fn remote_inventory_rejects_unsafe_resume_authority_and_oversized_output() {
        let good = row("claude", "uuid-id", 1);
        for (key, value) in [
            ("provider", json!("automonique")),
            ("session_id", json!("--resume")),
            ("session_id", json!("id\ncommand")),
            ("session_id", json!("id;command")),
            ("workdir", json!("relative")),
            ("workdir", json!("/tmp/../other")),
            ("workdir", json!("/tmp/\0secret")),
            ("title", json!("x".repeat(257))),
            ("model", json!("bad\nmodel")),
        ] {
            let mut bad = good.clone();
            bad[key] = value;
            assert!(
                parse_inventory(
                    &json!({"version":1,"sessions":[bad],"warnings":[],"truncated":false})
                        .to_string()
                )
                .is_err(),
                "accepted {key}"
            );
        }
        assert!(parse_inventory(
            &json!({"version":1,"sessions":[good.clone(),good],"warnings":[],"truncated":false})
                .to_string()
        )
        .is_err());
        assert!(parse_inventory(&" ".repeat(MAX_REMOTE_SESSION_INVENTORY_BYTES + 1)).is_err());
        assert!(parse_inventory("{\"version\":1}").is_err());
    }
}
