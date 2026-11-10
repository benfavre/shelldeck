//! Desktop-native, read-only CLI discovery. No Python, shell, provider or credential reads.
use crate::agent_remote_sessions::{RemoteAgentSession, RemoteAgentSessionInventory};
use crate::agent_runtime::AgentProvider;
use crate::{Result, ShellDeckError};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    fs,
    io::{BufRead, BufReader, Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    time::{Duration, Instant, UNIX_EPOCH},
};

#[path = "agent_local_history.rs"]
mod history;
pub use history::read_history;
const CANDIDATES: usize = 20_000;
const PAGE: usize = 200;
const LINE_LIMIT: usize = 2 * 1024 * 1024;

#[derive(Clone, Debug)]
pub struct LocalSessionRoots {
    pub claude: PathBuf,
    pub codex: PathBuf,
}
impl LocalSessionRoots {
    pub fn from_environment() -> Result<Self> {
        let home = directories::BaseDirs::new()
            .ok_or_else(|| error("The local home directory is unavailable."))?
            .home_dir()
            .to_path_buf();
        let root = |key: &str, fallback: &str| {
            std::env::var_os(key)
                .filter(|v| !v.is_empty())
                .map(PathBuf::from)
                .unwrap_or_else(|| home.join(fallback))
        };
        let roots = Self {
            claude: root("CLAUDE_CONFIG_DIR", ".claude"),
            codex: root("CODEX_HOME", ".codex"),
        };
        if !roots.claude.is_absolute() || !roots.codex.is_absolute() {
            return Err(error(
                "CLI configuration directories must be absolute paths.",
            ));
        }
        Ok(roots)
    }
    fn fingerprint(&self) -> String {
        format!(
            "{:x}",
            Sha256::digest(
                serde_json::to_vec(&(self.claude.to_string_lossy(), self.codex.to_string_lossy()))
                    .unwrap()
            )
        )
    }
}
pub(super) fn error(message: &str) -> ShellDeckError {
    ShellDeckError::Config(message.into())
}
pub(super) fn clean(value: &str, limit: usize) -> String {
    let text = value.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut result = String::new();
    for c in text.chars().filter(|c| !c.is_control()) {
        if result.len() + c.len_utf8() > limit {
            break;
        }
        result.push(c);
    }
    result
}
pub(super) fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 256
        && !id.starts_with(['-', '.'])
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b))
}
pub(super) fn valid_workdir(cwd: &str) -> bool {
    Path::new(cwd).is_absolute()
        && cwd.len() <= 4096
        && !cwd.chars().any(char::is_control)
        && !Path::new(cwd)
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
}
pub(super) fn millis(value: &Value) -> Option<i64> {
    value
        .as_str()
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|t| t.timestamp_millis())
}
pub(super) fn content_text(value: &Value) -> String {
    if let Some(text) = value.as_str() {
        return text.into();
    }
    value
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter(|v| {
                    matches!(
                        v["type"].as_str(),
                        Some("text" | "input_text" | "output_text" | "summary_text")
                    )
                })
                .filter_map(|v| v["text"].as_str())
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default()
}
#[derive(Clone)]
struct Candidate {
    provider: AgentProvider,
    path: PathBuf,
    key: (u64, String),
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Cursor {
    version: u8,
    source: String,
    before: (u64, String),
}
fn candidates(
    roots: &LocalSessionRoots,
    deadline: Instant,
    warnings: &mut Vec<String>,
) -> Vec<Candidate> {
    let mut result = Vec::new();
    let mut visited = 0;
    for (provider, root, depth) in [
        (AgentProvider::Claude, roots.claude.join("projects"), 1),
        (AgentProvider::Codex, roots.codex.join("sessions"), 4),
    ] {
        let mut pending = vec![(root, 0)];
        while let Some((folder, level)) = pending.pop() {
            if visited >= CANDIDATES || Instant::now() >= deadline {
                warnings.push("Local scan limit reached; showing a partial inventory.".into());
                return result;
            }
            let Ok(meta) = fs::symlink_metadata(&folder) else {
                continue;
            };
            if meta.file_type().is_symlink() {
                warnings.push(format!(
                    "{}: symbolic-link session directories are not scanned.",
                    provider.display_name()
                ));
                continue;
            }
            let entries = match fs::read_dir(folder) {
                Ok(entries) => entries,
                Err(_) => {
                    warnings.push(format!(
                        "{}: some session directories could not be read.",
                        provider.display_name()
                    ));
                    continue;
                }
            };
            for entry in entries {
                visited += 1;
                if visited > CANDIDATES || Instant::now() >= deadline {
                    warnings.push("Local scan limit reached; showing a partial inventory.".into());
                    return result;
                }
                let Ok(entry) = entry else {
                    continue;
                };
                let path = entry.path();
                let Ok(kind) = entry.file_type() else {
                    continue;
                };
                let name = entry.file_name();
                let name = name.to_string_lossy();
                if kind.is_dir() && level < depth && name != "subagents" && !name.starts_with('.') {
                    pending.push((path, level + 1));
                } else if kind.is_file()
                    && path.extension().is_some_and(|ext| ext == "jsonl")
                    && !name.starts_with("agent-")
                {
                    let Ok(meta) = entry.metadata() else {
                        continue;
                    };
                    let time = meta
                        .modified()
                        .ok()
                        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                        .map_or(0, |t| t.as_micros().min(u64::MAX as u128) as u64);
                    let identity = format!("{:?}:{}", provider, path.to_string_lossy());
                    result.push(Candidate {
                        provider,
                        path,
                        key: (time, format!("{:x}", Sha256::digest(identity.as_bytes()))),
                    });
                }
            }
        }
    }
    result
}
/// A bounded JSONL reader shared with passive transcript paging. Never follows a file symlink.
pub(super) fn records(
    path: &Path,
    limit: u64,
    deadline: Instant,
    mut consume: impl FnMut(Value) -> Result<bool>,
) -> Result<u64> {
    if !fs::symlink_metadata(path)
        .map_err(|_| error("The local history could not be read."))?
        .file_type()
        .is_file()
    {
        return Err(error("The local history is not a regular file."));
    }
    let file = fs::File::open(path).map_err(|_| error("The local history could not be read."))?;
    let mut reader = BufReader::new(file).take(limit + 1);
    let mut line = Vec::new();
    let mut total = 0;
    let line_limit = if limit <= LINE_LIMIT as u64 {
        LINE_LIMIT as u64
    } else {
        128 * 1024 * 1024
    };
    loop {
        if Instant::now() >= deadline {
            return Err(error(
                "Local history read timed out; retry loading history.",
            ));
        }
        line.clear();
        let count = (&mut reader)
            .take(line_limit + 1)
            .read_until(b'\n', &mut line)
            .map_err(|_| error("The local history could not be read."))?;
        if count == 0 {
            break;
        }
        total += count as u64;
        if count as u64 > line_limit || total > limit {
            return Err(error("Local history exceeds the bounded reader limit."));
        }
        if line.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        let value = match serde_json::from_slice::<Value>(&line) {
            Ok(value) if value.is_object() => value,
            // An agent may currently be appending its final JSON record.
            Err(_) if !line.ends_with(b"\n") => break,
            _ => return Err(error("The local history contains an invalid record.")),
        };
        if !consume(value)? {
            break;
        }
    }
    Ok(total)
}
fn metadata(
    candidate: &Candidate,
    deadline: Instant,
    limit: u64,
) -> Result<(Option<RemoteAgentSession>, u64)> {
    let mut id = String::new();
    let mut cwd = String::new();
    let mut title = String::new();
    let mut model = None;
    let mut sidechain = false;
    let consumed = records(
        &candidate.path,
        limit.min(2 * 1024 * 1024),
        deadline,
        |row| {
            let kind = row["type"].as_str().unwrap_or_default();
            if candidate.provider == AgentProvider::Claude {
                if row["isSidechain"].as_bool() == Some(true) {
                    sidechain = true;
                    return Ok(false);
                }
                if id.is_empty() {
                    id = row["sessionId"].as_str().unwrap_or_default().into();
                }
                if cwd.is_empty() {
                    cwd = row["cwd"].as_str().unwrap_or_default().into();
                }
                if kind == "user" && title.is_empty() {
                    title = content_text(&row["message"]["content"]);
                }
                if kind == "custom-title" {
                    title = row["customTitle"].as_str().unwrap_or(&title).into();
                }
                if let Some(value) = row["message"]["model"].as_str() {
                    model = Some(clean(value, 256));
                }
            } else {
                let payload = &row["payload"];
                if kind == "session_meta" {
                    if payload["source"].get("subagent").is_some() {
                        sidechain = true;
                        return Ok(false);
                    }
                    id = payload["id"]
                        .as_str()
                        .or_else(|| payload["session_id"].as_str())
                        .unwrap_or_default()
                        .into();
                    cwd = payload["cwd"].as_str().unwrap_or_default().into();
                }
                if kind == "turn_context" {
                    model = payload["model"].as_str().map(|s| clean(s, 256));
                }
                if title.is_empty() && kind == "event_msg" && payload["type"] == "user_message" {
                    title = content_text(&payload["message"]);
                }
                if title.is_empty() && kind == "response_item" && payload["role"] == "user" {
                    title = content_text(&payload["content"]);
                }
            }
            Ok(id.is_empty() || cwd.is_empty() || title.is_empty())
        },
    )?;
    if sidechain {
        return Ok((None, consumed));
    }
    if !valid_id(&id) || !valid_workdir(&cwd) {
        return Err(error(
            "Some local histories lack valid session or directory metadata.",
        ));
    }
    let title = clean(&title, 240);
    Ok((
        Some(RemoteAgentSession {
            provider: candidate.provider,
            session_id: id.clone(),
            workdir: cwd,
            title: if title.is_empty() { id } else { title },
            updated_at_ms: candidate.key.0 / 1000,
            model: model.filter(|s| !s.is_empty()),
        }),
        consumed,
    ))
}
fn codex_titles(roots: &LocalSessionRoots) -> std::collections::HashMap<String, String> {
    let mut titles = std::collections::HashMap::new();
    let path = roots.codex.join("session_index.jsonl");
    if !fs::symlink_metadata(&path).is_ok_and(|meta| meta.file_type().is_file()) {
        return titles;
    }
    let Ok(mut file) = fs::File::open(path) else {
        return titles;
    };
    let Ok(meta) = file.metadata() else {
        return titles;
    };
    let offset = meta.len().saturating_sub(2 * 1024 * 1024);
    if file.seek(SeekFrom::Start(offset)).is_err() {
        return titles;
    }
    let mut reader = BufReader::new(file).take(2 * 1024 * 1024);
    let mut line = String::new();
    if offset > 0 {
        let _ = reader.read_line(&mut line);
    }
    loop {
        line.clear();
        if !matches!(reader.read_line(&mut line), Ok(n) if n>0) {
            break;
        }
        if let Ok(row) = serde_json::from_str::<Value>(&line) {
            if let (Some(id), Some(title)) = (row["id"].as_str(), row["thread_name"].as_str()) {
                let title = clean(title, 240);
                if valid_id(id) && !title.is_empty() {
                    titles.insert(id.into(), title);
                }
            }
        }
    }
    titles
}
/// Read one metadata page. A source-bound keyset cursor survives new files and appends.
pub fn discover(
    roots: &LocalSessionRoots,
    cursor: Option<&str>,
) -> Result<RemoteAgentSessionInventory> {
    let deadline = Instant::now() + Duration::from_secs(12);
    let before = cursor
        .map(|cursor| -> Result<_> {
            if cursor.len() > 2048 {
                return Err(error(
                    "Invalid local inventory cursor; refresh this computer.",
                ));
            }
            let data = URL_SAFE_NO_PAD
                .decode(cursor)
                .map_err(|_| error("Invalid local inventory cursor; refresh this computer."))?;
            let cursor: Cursor = serde_json::from_slice(&data)
                .map_err(|_| error("Invalid local inventory cursor; refresh this computer."))?;
            if cursor.version != 1 || cursor.source != roots.fingerprint() {
                return Err(error(
                    "Local session directories changed; refresh this computer.",
                ));
            }
            Ok(cursor.before)
        })
        .transpose()?;
    let mut inventory = RemoteAgentSessionInventory::default();
    let mut files = candidates(roots, deadline, &mut inventory.warnings);
    files.sort_by(|a, b| b.key.cmp(&a.key));
    if let Some(before) = before {
        files.retain(|c| c.key < before);
    }
    let mut seen = HashSet::new();
    let titles = codex_titles(roots);
    let mut budget = 32 * 1024 * 1024u64;
    for (index, candidate) in files.iter().enumerate() {
        match metadata(candidate, deadline, budget) {
            Ok((session, consumed)) => {
                budget = budget.saturating_sub(consumed);
                if let Some(mut session) = session {
                    if session.provider == AgentProvider::Codex {
                        if let Some(title) = titles.get(&session.session_id) {
                            session.title = title.clone();
                        }
                    }
                    if seen.insert((session.provider.binary(), session.session_id.clone())) {
                        inventory.sessions.push(session);
                    }
                }
            }
            Err(_) => {
                budget = budget.saturating_sub(2 * 1024 * 1024);
                let warning = "Some local histories could not be read completely; refresh after the CLI finishes writing.".to_string();
                if !inventory.warnings.contains(&warning) {
                    inventory.warnings.push(warning);
                }
            }
        }
        if inventory.sessions.len() >= PAGE || budget == 0 || Instant::now() >= deadline {
            if index + 1 < files.len() {
                inventory.next_cursor = Some(
                    URL_SAFE_NO_PAD.encode(
                        serde_json::to_vec(&Cursor {
                            version: 1,
                            source: roots.fingerprint(),
                            before: candidate.key.clone(),
                        })
                        .unwrap(),
                    ),
                );
            }
            break;
        }
    }
    inventory.truncated = inventory.next_cursor.is_some() || !inventory.warnings.is_empty();
    inventory.warnings.truncate(16);
    Ok(inventory)
}
/// Locate only the selected exact ID under the configured session roots.
pub(super) fn locate(roots: &LocalSessionRoots, session: &RemoteAgentSession) -> Result<PathBuf> {
    if !valid_id(&session.session_id)
        || !valid_workdir(&session.workdir)
        || !matches!(
            session.provider,
            AgentProvider::Claude | AgentProvider::Codex
        )
    {
        return Err(error("Invalid local conversation identity."));
    }
    let deadline = Instant::now() + Duration::from_secs(12);
    let mut warnings = Vec::new();
    let mut matches = candidates(roots, deadline, &mut warnings)
        .into_iter()
        .filter(|c| {
            c.provider == session.provider
                && c.path
                    .file_stem()
                    .is_some_and(|s| s.to_string_lossy().ends_with(&session.session_id))
        });
    let candidate = matches
        .next()
        .ok_or_else(|| error("The local conversation was not found. Refresh this computer."))?;
    if matches.next().is_some() {
        return Err(error(
            "Multiple local histories have this identity; select an unambiguous conversation.",
        ));
    }
    Ok(candidate.path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{agent_remote_history::*, agent_session::*};
    use serde_json::json;
    use std::io::Write;
    fn fixture() -> (tempfile::TempDir, LocalSessionRoots) {
        let dir = tempfile::tempdir().unwrap();
        let roots = LocalSessionRoots {
            claude: dir.path().join("claude"),
            codex: dir.path().join("codex"),
        };
        fs::create_dir_all(roots.claude.join("projects/project")).unwrap();
        fs::create_dir_all(roots.codex.join("sessions/2026/11/10")).unwrap();
        (dir, roots)
    }
    fn write(path: &Path, rows: &[Value]) {
        let mut file = fs::File::create(path).unwrap();
        for row in rows {
            writeln!(file, "{row}").unwrap();
        }
    }
    // SDTEST-2950 / SDUC-572 — desktop metadata, keyset paging, scope and omissions.
    #[test]
    fn sdtest_2950_local_discovery_pages_both_providers_without_processes() {
        let (dir, roots) = fixture();
        let cwd = dir.path().to_string_lossy();
        for n in 0..205 {
            let id = format!("native-{n:04}");
            write(
                &roots.claude.join(format!("projects/project/{id}.jsonl")),
                &[
                    json!({"type":"user","sessionId":id,"cwd":cwd,"message":{"content":"A real saved task"}}),
                ],
            );
        }
        write(
            &roots
                .codex
                .join("sessions/2026/11/10/rollout-thread-one.jsonl"),
            &[
                json!({"type":"session_meta","payload":{"id":"thread-one","cwd":cwd}}),
                json!({"type":"event_msg","payload":{"type":"user_message","message":"Fix a bug"}}),
            ],
        );
        fs::create_dir_all(roots.claude.join("projects/project/subagents")).unwrap();
        write(
            &roots
                .claude
                .join("projects/project/subagents/agent-hidden.jsonl"),
            &[
                json!({"type":"user","sessionId":"hidden","cwd":cwd,"message":{"content":"Not a user thread"}}),
            ],
        );
        write(
            &roots.claude.join("projects/project/broken.jsonl"),
            &[json!({"sessionId":"../../escape","cwd":cwd})],
        );
        write(
            &roots.codex.join("session_index.jsonl"),
            &[json!({"id":"thread-one","thread_name":"Renamed Codex task"})],
        );
        let first = discover(&roots, None).unwrap();
        assert_eq!(first.sessions.len(), 200);
        assert!(first.truncated);
        assert!(first.next_cursor.is_some());
        let next = discover(&roots, first.next_cursor.as_deref()).unwrap();
        let all = first
            .sessions
            .iter()
            .chain(&next.sessions)
            .collect::<Vec<_>>();
        assert_eq!(all.len(), 206);
        assert_eq!(
            all.iter()
                .map(|s| (&s.session_id, s.provider.binary()))
                .collect::<HashSet<_>>()
                .len(),
            206
        );
        assert!(all
            .iter()
            .any(|s| s.provider == AgentProvider::Codex && s.title == "Renamed Codex task"));
        assert!(next.next_cursor.is_none());
        assert!(!first.warnings.is_empty() || !next.warnings.is_empty());
        let foreign = LocalSessionRoots {
            codex: dir.path().join("other"),
            ..roots.clone()
        };
        assert!(discover(&foreign, first.next_cursor.as_deref()).is_err());
        assert!(discover(&roots, Some("not/base64")).is_err());
    }
    // SDTEST-2951 / SDUC-572 — real JSONL parsing, duplicate suppression and immutable pages.
    #[test]
    fn sdtest_2951_local_history_paging_rewrite_and_native_reconnect() {
        let (dir, roots) = fixture();
        let cwd = dir.path().to_str().unwrap();
        for provider in [AgentProvider::Claude, AgentProvider::Codex] {
            let id = provider.binary();
            let path = if provider == AgentProvider::Claude {
                roots.claude.join(format!("projects/project/{id}.jsonl"))
            } else {
                roots
                    .codex
                    .join(format!("sessions/2026/11/10/rollout-{id}.jsonl"))
            };
            let mut rows = Vec::new();
            if provider == AgentProvider::Codex {
                rows.push(json!({"type":"session_meta","payload":{"id":id,"cwd":cwd}}));
                rows.push(json!({"type":"turn_context","payload":{"turn_id":"turn-a","model":"actual-model"}}));
            }
            if provider == AgentProvider::Codex {
                rows.push(json!({"type":"response_item","payload":{"type":"message","role":"developer","content":[{"type":"input_text","text":"Internal provider instructions are not a conversation turn"}]}}));
            }
            for n in 0..90 {
                let message = format!("Retained message {n}");
                if provider == AgentProvider::Claude {
                    rows.push(json!({"type":"user","sessionId":id,"cwd":cwd,"message":{"role":"user","content":message}}));
                } else {
                    rows.push(json!({"type":"event_msg","payload":{"type":"user_message","message":message}}));
                    rows.push(json!({"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":message}]}}));
                }
            }
            write(&path, &rows);
            let native = discover(&roots, None)
                .unwrap()
                .sessions
                .into_iter()
                .find(|s| s.provider == provider)
                .unwrap();
            let latest =
                read_history(&roots, &native, &RemoteHistoryQuery::default(), true).unwrap();
            assert_eq!(
                (latest.total_entries, latest.start, latest.end),
                (90, 26, 90)
            );
            assert_eq!(latest.entries[0].text, "Retained message 26");
            let query = RemoteHistoryQuery {
                snapshot: Some(latest.snapshot.clone()),
                position: RemoteHistoryPosition::Before(26),
            };
            let older = read_history(&roots, &native, &query, false).unwrap();
            assert_eq!((older.start, older.end), (0, 26));
            let mut file = fs::OpenOptions::new().append(true).open(&path).unwrap();
            if provider == AgentProvider::Claude {
                writeln!(file,"{}",json!({"type":"assistant","sessionId":id,"cwd":cwd,"message":{"role":"assistant","content":"New external turn"}})).unwrap();
            } else {
                writeln!(file,"{}",json!({"type":"response_item","payload":{"type":"message","role":"assistant","content":"New external turn"}})).unwrap();
            }
            assert_eq!(read_history(&roots, &native, &query, false).unwrap(), older);
            let mut sessions = AgentSessionCollection::default();
            let context = AgentExecutionContext {
                provider,
                target: crate::agent_runtime::AgentTarget::Local,
                workdir: cwd.into(),
                access: crate::agent_runtime::AgentAccessMode::WorkspaceWrite,
                model: latest.model.clone(),
            };
            let selected = sessions
                .reconnect_remote("Local CLI", context.clone(), id, 1)
                .unwrap();
            assert_eq!(
                sessions
                    .get(selected)
                    .unwrap()
                    .run_request("continue")
                    .resume_session
                    .as_deref(),
                Some(id)
            );
            assert_eq!(
                sessions.get(selected).unwrap().context.access,
                crate::agent_runtime::AgentAccessMode::ReadOnly
            );
            let restored: AgentSession = serde_json::from_value(
                serde_json::to_value(sessions.get(selected).unwrap()).unwrap(),
            )
            .unwrap();
            assert!(restored
                .remote_history
                .as_ref()
                .unwrap()
                .connection_id
                .is_none());
            assert!(restored.run_request("continue").resume_session.is_none());
            assert_eq!(
                sessions
                    .reconnect_remote("Same native", context, id, 2)
                    .unwrap(),
                selected
            );
            rows[if provider == AgentProvider::Claude {
                0
            } else {
                3
            }] = if provider == AgentProvider::Claude {
                json!({"type":"user","sessionId":id,"cwd":cwd,"message":{"content":"Rewritten"}})
            } else {
                json!({"type":"event_msg","payload":{"type":"user_message","message":"Rewritten"}})
            };
            write(&path, &rows);
            assert!(read_history(&roots, &native, &query, false).is_err());
            let mut foreign = native;
            foreign.workdir = dir.path().join("other").to_string_lossy().into_owned();
            assert!(read_history(&roots, &foreign, &RemoteHistoryQuery::default(), false).is_err());
        }
    }
}

#[cfg(test)]
mod large_history_tests {
    use super::*;
    use crate::agent_remote_history::{RemoteHistoryKind, RemoteHistoryQuery};
    use serde_json::json;
    use std::io::Write;
    // SDTEST-2953 / SDUC-572 — large saved image records must not leak blobs or break UTF-8 paging.
    #[test]
    fn sdtest_2953_local_large_attachment_history_is_passive_and_chunked() {
        let directory = tempfile::tempdir().unwrap();
        let roots = LocalSessionRoots {
            claude: directory.path().join("claude"),
            codex: directory.path().join("codex"),
        };
        let folder = roots.claude.join("projects/project");
        fs::create_dir_all(&folder).unwrap();
        let path = folder.join("native-image.jsonl");
        let cwd = directory.path().to_str().unwrap();
        let original = "é🌍".repeat(5000);
        let blob = "a".repeat(3 * 1024 * 1024);
        let mut file = fs::File::create(&path).unwrap();
        writeln!(file,"{}",json!({"type":"user","sessionId":"native-image","cwd":cwd,"message":{"role":"user","content":"Inspect this screenshot"}})).unwrap();
        writeln!(file,"{}",json!({"type":"assistant","sessionId":"native-image","cwd":cwd,"message":{"role":"assistant","model":"actual-model","content":[{"type":"image","source":{"type":"base64","data":blob}},{"type":"text","text":original}]}})).unwrap();
        drop(file);
        let native = discover(&roots, None).unwrap().sessions.remove(0);
        let page = read_history(&roots, &native, &RemoteHistoryQuery::default(), false).unwrap();
        assert_eq!(page.model.as_deref(), Some("actual-model"));
        assert!(page
            .entries
            .iter()
            .any(|e| e.kind == RemoteHistoryKind::Activity && e.text == "Saved attachment: image"));
        let chunks = page
            .entries
            .iter()
            .filter(|e| e.kind == RemoteHistoryKind::Assistant)
            .collect::<Vec<_>>();
        assert_eq!(chunks.len(), 2);
        assert!(chunks
            .iter()
            .all(|e| e.text.len() <= 16 * 1024 && e.parts == 2));
        assert_eq!(
            chunks.iter().map(|e| e.text.as_str()).collect::<String>(),
            original
        );
        assert!(!page
            .entries
            .iter()
            .any(|e| e.text.contains(&"a".repeat(100))));
        fs::copy(&path, folder.join("copy-native-image.jsonl")).unwrap();
        assert!(
            read_history(&roots, &native, &RemoteHistoryQuery::default(), false).is_err(),
            "ambiguous source must not grant a resume binding"
        );
    }
}
