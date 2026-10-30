//! Read-only native transcript retrieval with exact identity and immutable-prefix paging.
use crate::agent_remote_sessions::RemoteAgentSession;
use crate::agent_runtime::AgentProvider;
use crate::{Result, ShellDeckError};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde::{Deserialize, Serialize};

/// Includes worst-case JSON escaping of 64 chunks of 16 KiB each.
pub const MAX_REMOTE_HISTORY_BYTES: usize = 8 * 1024 * 1024;
pub const REMOTE_HISTORY_PAGE_SIZE: usize = 64;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum RemoteHistoryPosition {
    #[default]
    Latest,
    Before(u64),
    After(u64),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RemoteHistoryQuery {
    pub snapshot: Option<String>,
    pub position: RemoteHistoryPosition,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RemoteHistoryKind {
    User,
    Assistant,
    Tool,
    Reasoning,
    Activity,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RemoteHistoryEntry {
    pub index: u64,
    pub kind: RemoteHistoryKind,
    pub title: Option<String>,
    pub text: String,
    pub at_ms: Option<i64>,
    /// One-based part number. Joining consecutive parts reproduces the original text.
    pub part: u32,
    pub parts: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RemoteHistoryPage {
    pub provider: AgentProvider,
    pub session_id: String,
    pub workdir: String,
    pub model: Option<String>,
    pub snapshot: String,
    pub total_entries: u64,
    pub start: u64,
    pub end: u64,
    pub entries: Vec<RemoteHistoryEntry>,
    pub warnings: Vec<String>,
}

fn invalid() -> ShellDeckError {
    ShellDeckError::Config("Invalid remote history response; reload history and retry".into())
}

fn snapshot_total(snapshot: &str) -> Option<u64> {
    let mut parts = snapshot.split(':');
    if parts.next()? != "v1" {
        return None;
    }
    let count = parts.next()?;
    if count.is_empty() || count.len() > 16 || !count.bytes().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let digest = parts.next()?;
    if digest.len() != 64
        || !digest
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
        || parts.next().is_some()
    {
        return None;
    }
    count.parse().ok()
}

fn validate_request(session: &RemoteAgentSession, query: &RemoteHistoryQuery) -> Result<()> {
    if !matches!(
        session.provider,
        AgentProvider::Claude | AgentProvider::Codex | AgentProvider::Jcode
    ) || session.session_id.is_empty()
        || session.session_id.len() > 256
        || session.session_id.starts_with(['-', '.'])
        || !session
            .session_id
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"_.-".contains(&c))
        || !session.workdir.starts_with('/')
        || session.workdir.len() > 4096
        || session.workdir.split('/').any(|c| c == "..")
        || session.workdir.chars().any(char::is_control)
    {
        return Err(invalid());
    }
    let total = query
        .snapshot
        .as_deref()
        .map(|snapshot| snapshot_total(snapshot).ok_or_else(invalid))
        .transpose()?;
    if let RemoteHistoryPosition::Before(boundary) | RemoteHistoryPosition::After(boundary) =
        query.position
    {
        if total.is_none_or(|total| boundary > total) {
            return Err(invalid());
        }
    }
    Ok(())
}

/// Shell-safe command for an already authenticated SSH channel. The request is
/// base64 data, never executable shell syntax. The reader performs no writes.
pub fn history_command(session: &RemoteAgentSession, query: &RemoteHistoryQuery) -> Result<String> {
    validate_request(session, query)?;
    let (position, boundary) = match query.position {
        RemoteHistoryPosition::Latest => ("latest", 0),
        RemoteHistoryPosition::Before(n) => ("before", n),
        RemoteHistoryPosition::After(n) => ("after", n),
    };
    let request = serde_json::json!({"provider":session.provider,"session_id":session.session_id,"workdir":session.workdir,"snapshot":query.snapshot,"position":position,"boundary":boundary});
    let encoded = STANDARD.encode(request.to_string());
    Ok(format!(
        "python3 - '{encoded}' <<'SHELLDECK_NATIVE_HISTORY_PY'\n{}\nSHELLDECK_NATIVE_HISTORY_PY",
        include_str!("agent_remote_history.py")
    ))
}

/// Treat remote output as untrusted: bind the page to the requested native
/// session, snapshot and cursor before it can be shown in a conversation.
pub fn parse_history(
    output: &str,
    session: &RemoteAgentSession,
    query: &RemoteHistoryQuery,
) -> Result<RemoteHistoryPage> {
    validate_request(session, query)?;
    if output.len() > MAX_REMOTE_HISTORY_BYTES {
        return Err(invalid());
    }
    let mut value: serde_json::Value = serde_json::from_str(output).map_err(|_| invalid())?;
    let object = value.as_object_mut().ok_or_else(invalid)?;
    if object.remove("version") != Some(serde_json::json!(1)) {
        return Err(invalid());
    }
    if let Some(error) = object.get("error").and_then(|v| v.as_str()) {
        if object.len() == 1
            && !error.is_empty()
            && error.len() <= 512
            && !error.chars().any(char::is_control)
        {
            return Err(ShellDeckError::Config(error.to_owned()));
        }
        return Err(invalid());
    }
    let page: RemoteHistoryPage = serde_json::from_value(value).map_err(|_| invalid())?;
    if page.provider != session.provider
        || page.session_id != session.session_id
        || page.workdir != session.workdir
        || snapshot_total(&page.snapshot) != Some(page.total_entries)
        || query
            .snapshot
            .as_ref()
            .is_some_and(|snapshot| snapshot != &page.snapshot)
        || page.start > page.end
        || page.end > page.total_entries
        || page.end - page.start != page.entries.len() as u64
        || page.entries.len() > REMOTE_HISTORY_PAGE_SIZE
        || page.model.as_ref().is_some_and(|model| {
            model.is_empty() || model.len() > 256 || model.chars().any(char::is_control)
        })
        || page.warnings.len() > 16
        || page
            .warnings
            .iter()
            .any(|w| w.len() > 512 || w.chars().any(char::is_control))
    {
        return Err(invalid());
    }
    let expected = match query.position {
        RemoteHistoryPosition::Latest => (
            page.total_entries
                .saturating_sub(REMOTE_HISTORY_PAGE_SIZE as u64),
            page.total_entries,
        ),
        RemoteHistoryPosition::Before(n) => (n.saturating_sub(REMOTE_HISTORY_PAGE_SIZE as u64), n),
        RemoteHistoryPosition::After(n) => (
            n,
            n.saturating_add(REMOTE_HISTORY_PAGE_SIZE as u64)
                .min(page.total_entries),
        ),
    };
    if (page.start, page.end) != expected {
        return Err(invalid());
    }
    for (offset, entry) in page.entries.iter().enumerate() {
        if entry.index != page.start + offset as u64
            || entry.text.len() > 16 * 1024
            || entry.part == 0
            || entry.parts == 0
            || entry.part > entry.parts
            || entry.title.as_ref().is_some_and(|title| title.len() > 4096)
        {
            return Err(invalid());
        }
    }
    Ok(page)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn session() -> RemoteAgentSession {
        RemoteAgentSession {
            provider: AgentProvider::Codex,
            session_id: "session-123".into(),
            workdir: "/home/test/a 'quoted' $(folder)".into(),
            title: "Test".into(),
            updated_at_ms: 0,
            model: None,
        }
    }
    fn wire() -> serde_json::Value {
        json!({"version":1,"provider":"codex","session_id":session().session_id,"workdir":session().workdir,"model":"test-model","snapshot":format!("v1:1:{}", "a".repeat(64)),"total_entries":1,"start":0,"end":1,"entries":[{"index":0,"kind":"assistant","title":null,"text":"Hello 🌍","at_ms":null,"part":1,"parts":1}],"warnings":[]})
    }
    // SDTEST-2813 / SDUC-563
    #[test]
    fn history_command_carries_shell_safe_identity_and_parser_validates_page() {
        let query = RemoteHistoryQuery::default();
        let command = history_command(&session(), &query).unwrap();
        assert!(!command.contains(&session().workdir));
        let encoded = command.split('\'').nth(1).unwrap();
        let request: serde_json::Value =
            serde_json::from_slice(&STANDARD.decode(encoded).unwrap()).unwrap();
        assert_eq!(request["workdir"], session().workdir);
        assert_eq!(
            parse_history(&wire().to_string(), &session(), &query)
                .unwrap()
                .entries[0]
                .text,
            "Hello 🌍"
        );
        let unsafe_session = RemoteAgentSession {
            session_id: "../foreign".into(),
            ..session()
        };
        assert!(history_command(&unsafe_session, &query).is_err());
    }
    // SDTEST-2814 / SDUC-563
    #[test]
    fn history_rejects_foreign_snapshots_identity_gaps_and_oversized_messages() {
        for (key, replacement) in [
            ("provider", json!("claude")),
            ("session_id", json!("foreign")),
            ("workdir", json!("/foreign")),
            ("end", json!(0)),
            ("snapshot", json!("v1:2:invalid")),
        ] {
            let mut value = wire();
            value[key] = replacement;
            assert!(parse_history(
                &value.to_string(),
                &session(),
                &RemoteHistoryQuery::default()
            )
            .is_err());
        }
        let query = RemoteHistoryQuery {
            snapshot: Some(format!("v1:1:{}", "b".repeat(64))),
            position: RemoteHistoryPosition::Before(1),
        };
        assert!(parse_history(&wire().to_string(), &session(), &query).is_err());
        let mut value = wire();
        value["entries"][0]["text"] = json!("x".repeat(16 * 1024 + 1));
        assert!(parse_history(
            &value.to_string(),
            &session(),
            &RemoteHistoryQuery::default()
        )
        .is_err());
        assert!(parse_history(
            r#"{"version":1,"error":"Saved history changed. Reload history."}"#,
            &session(),
            &RemoteHistoryQuery::default()
        )
        .unwrap_err()
        .to_string()
        .contains("Reload history"));
    }
}
