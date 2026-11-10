//! Bounded passive Claude/Codex JSONL history with immutable-prefix paging.
use super::*;
use crate::agent_remote_history::{
    RemoteHistoryEntry, RemoteHistoryKind as Kind, RemoteHistoryPage,
    RemoteHistoryPosition as Position, RemoteHistoryQuery, REMOTE_HISTORY_PAGE_SIZE,
};
use std::collections::{HashMap, VecDeque};
const HISTORY_LIMIT: u64 = 128 * 1024 * 1024;
const CHUNK: usize = 16 * 1024;
const CHANGED: &str = "Saved history changed. Reload history to start a fresh snapshot.";

fn text(value: &Value) -> String {
    value
        .as_str()
        .map(str::to_string)
        .unwrap_or_else(|| value.to_string())
}
fn kind(role: &str) -> Kind {
    match role {
        "user" => Kind::User,
        "assistant" => Kind::Assistant,
        "reasoning" => Kind::Reasoning,
        _ => Kind::Tool,
    }
}
fn blocks(value: &Value, role: &str, emit: &mut impl FnMut(Kind, Option<String>, String)) {
    if let Some(s) = value.as_str() {
        emit(kind(role), None, s.into());
        return;
    }
    let Some(blocks) = value.as_array() else {
        return;
    };
    for block in blocks {
        let name = block["type"].as_str().unwrap_or("unknown");
        match name {
            "text" | "input_text" | "output_text" | "summary_text" => {
                emit(kind(role), None, text(&block["text"]))
            }
            "thinking" | "reasoning" => emit(
                Kind::Reasoning,
                Some("Reasoning".into()),
                text(
                    block
                        .get("thinking")
                        .or_else(|| block.get("text"))
                        .unwrap_or(&Value::Null),
                ),
            ),
            "tool_use" | "function_call" => emit(
                Kind::Tool,
                Some(clean(block["name"].as_str().unwrap_or("Tool call"), 256)),
                text(&block["input"]),
            ),
            "tool_result" => emit(
                Kind::Tool,
                Some(clean(
                    block["tool_use_id"].as_str().unwrap_or("Tool result"),
                    256,
                )),
                text(&block["content"]),
            ),
            "image" | "input_image" | "document" | "audio" | "input_audio" | "file" => emit(
                Kind::Activity,
                Some("Attachment".into()),
                format!(
                    "Saved attachment: {}",
                    clean(block["filename"].as_str().unwrap_or(name), 256)
                ),
            ),
            "redacted_thinking" => emit(
                Kind::Reasoning,
                Some("Reasoning".into()),
                "This reasoning was redacted by the provider.".into(),
            ),
            _ => emit(
                Kind::Activity,
                Some(format!("Saved content ({})", clean(name, 128))),
                text(block),
            ),
        }
    }
}
fn normalize(
    row: &Value,
    provider: AgentProvider,
    skip: bool,
    emit: &mut impl FnMut(Kind, Option<String>, String),
) {
    let typ = row["type"].as_str().unwrap_or_default();
    if provider == AgentProvider::Claude {
        if !matches!(typ, "user" | "assistant") || row["isSidechain"].as_bool() == Some(true) {
            return;
        }
        let message = &row["message"];
        blocks(
            &message["content"],
            message["role"].as_str().unwrap_or(typ),
            emit,
        );
    } else {
        let p = &row["payload"];
        let k = p["type"].as_str().unwrap_or_default();
        if typ == "response_item" {
            match k {
                "message" if matches!(p["role"].as_str(), Some("user" | "assistant" | "tool")) => {
                    blocks(&p["content"], p["role"].as_str().unwrap_or("tool"), emit)
                }
                "reasoning" => blocks(&p["summary"], "reasoning", emit),
                "function_call" | "custom_tool_call" => emit(
                    Kind::Tool,
                    Some(clean(p["name"].as_str().unwrap_or("Tool call"), 256)),
                    text(
                        p.get("arguments")
                            .or_else(|| p.get("input"))
                            .unwrap_or(&Value::Null),
                    ),
                ),
                "function_call_output" | "custom_tool_call_output" => emit(
                    Kind::Tool,
                    Some(clean(p["call_id"].as_str().unwrap_or("Tool result"), 256)),
                    text(&p["output"]),
                ),
                _ => {}
            }
        } else if typ == "event_msg" && !skip {
            match k {
                "user_message" => blocks(&p["message"], "user", emit),
                "agent_message" => blocks(&p["message"], "assistant", emit),
                "agent_reasoning" => blocks(&p["text"], "reasoning", emit),
                "error" | "warning" => emit(Kind::Activity, Some(k.into()), text(&p["message"])),
                _ => {}
            }
        }
    }
}
fn metadata(
    path: &Path,
    session: &RemoteAgentSession,
    deadline: Instant,
) -> Result<(Option<String>, HashSet<usize>)> {
    let mut model = None;
    let mut found = false;
    let mut index = 0;
    let mut scope = None;
    // Keep repeated messages in separate turns. Only matched canonical/event pairs
    // in the same turn (or adjacent legacy records) are duplicates.
    type Pairs = (Vec<usize>, Vec<usize>);
    let mut pairs: HashMap<(Option<String>, String, String), Pairs> = HashMap::new();
    records(path, HISTORY_LIMIT, deadline, |row| {
        let typ = row["type"].as_str().unwrap_or_default();
        let p = &row["payload"];
        if session.provider == AgentProvider::Claude {
            if row["sessionId"]
                .as_str()
                .is_some_and(|id| id != session.session_id)
            {
                return Err(error(
                    "The local history belongs to a different conversation.",
                ));
            }
            if row["sessionId"] == session.session_id && row["cwd"] == session.workdir {
                found = true;
            }
            if typ == "assistant" {
                if let Some(s) = row["message"]["model"].as_str() {
                    model = Some(clean(s, 256));
                }
            }
        } else {
            if typ == "session_meta" {
                if p["id"].as_str().or_else(|| p["session_id"].as_str())
                    != Some(session.session_id.as_str())
                    || p["cwd"] != session.workdir
                    || p["source"].get("subagent").is_some()
                {
                    return Err(error(
                        "The local history identity or directory does not match this conversation.",
                    ));
                }
                found = true;
            }
            if typ == "turn_context" || (typ == "event_msg" && p["type"] == "task_started") {
                scope = p["turn_id"]
                    .as_str()
                    .or_else(|| p["root_turn_id"].as_str())
                    .map(str::to_string);
            }
            if typ == "turn_context" {
                if let Some(s) = p["model"].as_str() {
                    model = Some(clean(s, 256));
                }
            }
            let signature = if typ == "response_item" && p["type"] == "message" {
                Some((
                    false,
                    p["role"].as_str().unwrap_or_default(),
                    content_text(&p["content"]),
                ))
            } else if typ == "event_msg"
                && matches!(
                    p["type"].as_str(),
                    Some("user_message" | "agent_message" | "agent_reasoning")
                )
            {
                let role = match p["type"].as_str() {
                    Some("user_message") => "user",
                    Some("agent_message") => "assistant",
                    _ => "reasoning",
                };
                Some((
                    true,
                    role,
                    content_text(
                        p.get("message")
                            .or_else(|| p.get("text"))
                            .unwrap_or(&Value::Null),
                    ),
                ))
            } else {
                None
            };
            if let Some((event, role, text)) = signature.filter(|(_, _, text)| !text.is_empty()) {
                let key = (
                    scope.clone(),
                    role.to_string(),
                    format!("{:x}", Sha256::digest(text.as_bytes())),
                );
                let pair = pairs.entry(key).or_default();
                if event {
                    pair.0.push(index);
                } else {
                    pair.1.push(index);
                }
            }
            if typ == "event_msg" && p["type"] == "task_complete" {
                scope = None;
            }
        }
        index += 1;
        if index > 250_000 {
            return Err(error("Local history exceeds the bounded record limit."));
        }
        Ok(true)
    })?;
    if !found {
        return Err(error(
            "The local history does not confirm this conversation and directory.",
        ));
    }
    let mut skip = HashSet::new();
    for ((scope, _, _), (mut events, canonical)) in pairs {
        for index in canonical {
            if let Some((position, event)) = events
                .iter()
                .enumerate()
                .min_by_key(|(_, event)| event.abs_diff(index))
                .map(|(p, e)| (p, *e))
            {
                if scope.is_some() || event.abs_diff(index) == 1 {
                    skip.insert(event);
                    events.remove(position);
                }
            }
        }
    }
    Ok((model.filter(|s| !s.is_empty()), skip))
}
/// Read a passive page and revalidate exact identity, without granting resume authority.
pub fn read_history(
    roots: &LocalSessionRoots,
    session: &RemoteAgentSession,
    query: &RemoteHistoryQuery,
    require_workdir: bool,
) -> Result<RemoteHistoryPage> {
    let path = locate(roots, session)?;
    if require_workdir && !Path::new(&session.workdir).is_dir() {
        return Err(error("The local working directory is no longer available."));
    }
    let deadline = Instant::now() + Duration::from_secs(20);
    let before = fs::metadata(&path).map_err(|_| error("The local history could not be read."))?;
    let (model, skip) = metadata(&path, session, deadline)?;
    let pinned = query
        .snapshot
        .as_deref()
        .map(|s| -> Result<(u64, &str)> {
            let mut parts = s.split(':');
            if parts.next() != Some("v1") {
                return Err(error(CHANGED));
            }
            let count = parts
                .next()
                .and_then(|n| n.parse().ok())
                .ok_or_else(|| error(CHANGED))?;
            let digest = parts.next().ok_or_else(|| error(CHANGED))?;
            if digest.len() != 64
                || !digest.bytes().all(|c| c.is_ascii_hexdigit())
                || parts.next().is_some()
            {
                return Err(error(CHANGED));
            }
            Ok((count, digest))
        })
        .transpose()?;
    if let Position::Before(n) | Position::After(n) = query.position {
        if pinned.is_none_or(|(count, _)| n > count) {
            return Err(error("Invalid local history page cursor; reload history."));
        }
    }
    let mut hasher = Sha256::new();
    hasher.update(
        serde_json::to_vec(&(session.provider, &session.session_id, &session.workdir)).unwrap(),
    );
    let mut selected = VecDeque::new();
    let mut total = 0;
    let mut row_index = 0;
    records(&path, HISTORY_LIMIT, deadline, |row| {
        let at = millis(&row["timestamp"]);
        normalize(
            &row,
            session.provider,
            skip.contains(&row_index),
            &mut |kind, title, text| {
                let mut pieces = Vec::new();
                let mut start = 0;
                while start < text.len() {
                    let mut end = (start + CHUNK).min(text.len());
                    while !text.is_char_boundary(end) {
                        end -= 1;
                    }
                    pieces.push(&text[start..end]);
                    start = end;
                }
                if pieces.is_empty() {
                    pieces.push("");
                }
                for (part, piece) in pieces.iter().enumerate() {
                    if pinned.is_some_and(|(count, _)| total >= count) {
                        break;
                    }
                    let entry = RemoteHistoryEntry {
                        index: total,
                        kind,
                        title: title.clone(),
                        text: (*piece).into(),
                        at_ms: at,
                        part: part as u32 + 1,
                        parts: pieces.len() as u32,
                    };
                    hasher.update(serde_json::to_vec(&entry).unwrap());
                    hasher.update(b"\n");
                    let eligible = match query.position {
                        Position::Latest => true,
                        Position::Before(n) => total < n,
                        Position::After(n) => {
                            total >= n && selected.len() < REMOTE_HISTORY_PAGE_SIZE
                        }
                    };
                    if eligible {
                        if selected.len() == REMOTE_HISTORY_PAGE_SIZE {
                            selected.pop_front();
                        }
                        selected.push_back(entry);
                    }
                    total += 1;
                }
            },
        );
        row_index += 1;
        Ok(pinned.is_none_or(|(count, _)| total < count))
    })?;
    let after = fs::metadata(&path).map_err(|_| error("The local history could not be read."))?;
    if before.len() != after.len() || before.modified().ok() != after.modified().ok() {
        return Err(error(
            "History changed while it was being read. Retry loading history.",
        ));
    }
    let digest = format!("{:x}", hasher.finalize());
    if pinned.is_some_and(|(count, expected)| count != total || digest != expected) {
        return Err(error(CHANGED));
    }
    let start = selected.front().map_or_else(
        || match query.position {
            Position::Latest => total,
            Position::Before(n) | Position::After(n) => n.min(total),
        },
        |e| e.index,
    );
    let end = selected.back().map_or(start, |e| e.index + 1);
    Ok(RemoteHistoryPage {
        provider: session.provider,
        session_id: session.session_id.clone(),
        workdir: session.workdir.clone(),
        model,
        snapshot: format!("v1:{total}:{digest}"),
        total_entries: total,
        start,
        end,
        entries: selected.into(),
        warnings: Vec::new(),
    })
}
