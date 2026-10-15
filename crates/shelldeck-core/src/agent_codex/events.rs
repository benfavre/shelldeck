//! Bounded presentation projection; raw protocol objects never reach the UI.
use super::*;
use crate::agent_session::{AgentTraceKind, AgentTraceStatus, AgentTraceUpdate};

pub(super) struct TextEvents {
    parser: AgentStreamParser,
}
impl TextEvents {
    pub(super) fn new() -> Self {
        Self {
            parser: AgentStreamParser::new(AgentProvider::Codex),
        }
    }
    pub(super) fn finish(&mut self) -> Vec<AgentStreamEvent> {
        self.parser.finish()
    }
    pub(super) fn notification(&mut self, method: &str, params: &Value) -> Vec<AgentStreamEvent> {
        match method {
            "item/agentMessage/delta" => {
                let Ok(id) = valid_id(&params["itemId"]) else {
                    return vec![];
                };
                let Some(delta) = params["delta"].as_str() else {
                    return vec![];
                };
                // Shared parser buffers split credential tokens and deduplicates
                // completed snapshots without reserializing cumulative text.
                self.parser.codex_text_delta(id, delta)
            }
            "item/started" | "item/completed" => {
                let item = &params["item"];
                if item["type"].as_str() == Some("agentMessage") {
                    if method != "item/completed" {
                        return vec![];
                    }
                    let Ok(id) = valid_id(&item["id"]) else {
                        return vec![];
                    };
                    return self
                        .parser
                        .codex_text_complete(id, item["text"].as_str().unwrap_or_default());
                }
                item_events(item, method == "item/completed")
            }
            "turn/plan/updated" => vec![AgentStreamEvent::Plan(
                AgentPlanUpdate {
                    steps: params["plan"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .take(MAX_AGENT_PLAN_STEPS)
                        .map(|step| AgentPlanStep {
                            text: safe(step["step"].as_str().unwrap_or_default()),
                            status: match step["status"].as_str() {
                                Some("completed") => AgentPlanStepStatus::Completed,
                                Some("inProgress") => AgentPlanStepStatus::InProgress,
                                _ => AgentPlanStepStatus::Pending,
                            },
                        })
                        .collect(),
                }
                .bounded(),
            )],
            "thread/tokenUsage/updated" => {
                let usage = &params["tokenUsage"];
                let total = &usage["total"];
                vec![AgentStreamEvent::Usage(
                    AgentUsageUpdate {
                        input_tokens: total["inputTokens"].as_u64(),
                        output_tokens: total["outputTokens"].as_u64(),
                        cached_input_tokens: total["cachedInputTokens"].as_u64(),
                        cache_write_input_tokens: total["cacheWriteInputTokens"].as_u64(),
                        reasoning_output_tokens: total["reasoningOutputTokens"].as_u64(),
                        context_used: usage["last"]["totalTokens"].as_u64(),
                        context_size: usage["modelContextWindow"].as_u64(),
                        cumulative: true,
                        ..Default::default()
                    }
                    .bounded(),
                )]
            }
            "error" => {
                let message = safe(
                    params
                        .pointer("/error/message")
                        .and_then(Value::as_str)
                        .unwrap_or("Codex reported an error"),
                );
                vec![if params["willRetry"].as_bool() == Some(true) {
                    AgentStreamEvent::Activity(message)
                } else {
                    AgentStreamEvent::Error(message)
                }]
            }
            "thread/compacted" => vec![AgentStreamEvent::Activity(
                "Codex compacted conversation context".into(),
            )],
            _ => vec![],
        }
    }
}

fn status(item: &Value, completed: bool) -> AgentTraceStatus {
    match item["status"].as_str() {
        Some("failed" | "declined" | "errored") => AgentTraceStatus::Failed,
        Some("completed") => AgentTraceStatus::Succeeded,
        Some("inProgress") => AgentTraceStatus::Running,
        _ if completed => AgentTraceStatus::Succeeded,
        _ => AgentTraceStatus::Running,
    }
}
fn trace(id: Option<String>, detail: AgentTraceKind) -> AgentStreamEvent {
    AgentStreamEvent::Trace(AgentTraceUpdate {
        correlation_id: id,
        detail,
    })
}

fn field(text: &str) -> String {
    let text = safe(text);
    let mut end = text.len().min(MAX_AGENT_TRACE_FIELD_BYTES);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_string()
}

/// Only offer one-shot file approval when every proposed path and patch fits
/// the review card without redaction or truncation. The full unbounded payload
/// is never retained; incomplete previews cannot authorize a mutation.
pub(super) fn file_change_approval_preview(item: &Value) -> Option<String> {
    let changes = item["changes"]
        .as_array()
        .filter(|changes| !changes.is_empty())?;
    let mut preview = String::new();
    for change in changes {
        let path = change["path"].as_str()?;
        let diff = change["diff"].as_str()?;
        if preview
            .len()
            .saturating_add(path.len())
            .saturating_add(diff.len())
            .saturating_add(16)
            > MAX_AGENT_TRACE_FIELD_BYTES
        {
            return None;
        }
        preview.push_str("File: ");
        preview.push_str(path);
        preview.push('\n');
        preview.push_str(diff);
        preview.push('\n');
    }
    if safe(&preview) != preview {
        return None;
    }
    Some(preview)
}

fn item_events(item: &Value, completed: bool) -> Vec<AgentStreamEvent> {
    let kind = item["type"].as_str().unwrap_or_default();
    let id = valid_id(&item["id"]).ok();
    let state = status(item, completed);
    match kind {
        "commandExecution" => vec![trace(
            id,
            AgentTraceKind::Command {
                command: field(item["command"].as_str().unwrap_or("Command")),
                status: state,
                exit_code: item["exitCode"].as_i64().and_then(|n| n.try_into().ok()),
                summary: item["aggregatedOutput"].as_str().map(field),
            },
        )],
        "fileChange" => item["changes"]
            .as_array()
            .into_iter()
            .flatten()
            .take(128)
            .enumerate()
            .map(|(i, change)| {
                let preview = change["diff"].as_str().map(field);
                let (additions, deletions) = preview.as_deref().unwrap_or_default().lines().fold(
                    (0u32, 0u32),
                    |(a, d), line| {
                        (
                            a + u32::from(line.starts_with('+') && !line.starts_with("+++")),
                            d + u32::from(line.starts_with('-') && !line.starts_with("---")),
                        )
                    },
                );
                trace(
                    id.as_ref().map(|id| format!("{id}:{i}")),
                    AgentTraceKind::Diff {
                        path: field(change["path"].as_str().unwrap_or_default()),
                        status: state,
                        additions,
                        deletions,
                        preview,
                    },
                )
            })
            .collect(),
        "mcpToolCall" | "dynamicToolCall" => {
            let name = match (item["server"].as_str(), item["tool"].as_str()) {
                (Some(server), Some(tool)) => format!("{server}/{tool}"),
                (_, Some(tool)) => tool.into(),
                _ => "Tool".into(),
            };
            let mut events = vec![trace(
                id,
                AgentTraceKind::Tool {
                    name: field(&name),
                    status: state,
                    summary: item
                        .pointer("/error/message")
                        .and_then(Value::as_str)
                        .map(field),
                },
            )];
            // Explicit resource links only; do not turn arbitrary tool output
            // into trusted navigation or expose raw arguments/results.
            for content in item
                .pointer("/result/content")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .take(32)
            {
                if content["type"].as_str() == Some("resource_link") {
                    if let Some(uri) = content["uri"]
                        .as_str()
                        .filter(|uri| uri.starts_with("https://") || uri.starts_with("http://"))
                    {
                        events.push(AgentStreamEvent::Source(
                            AgentSource {
                                id: uuid::Uuid::new_v4().to_string(),
                                title: safe(content["name"].as_str().unwrap_or("Source")),
                                uri: safe(uri),
                            }
                            .bounded(),
                        ));
                    }
                }
            }
            events
        }
        "collabAgentToolCall" => {
            let parent = item["senderThreadId"].as_str().map(safe);
            item["receiverThreadIds"]
                .as_array()
                .into_iter()
                .flatten()
                .take(32)
                .filter_map(|receiver| {
                    let thread = receiver.as_str()?;
                    let summary = item["agentsStates"]
                        .get(thread)
                        .and_then(|s| s["message"].as_str())
                        .map(safe);
                    Some(AgentStreamEvent::Subagent(
                        AgentSubagentUpdate {
                            id: safe(thread),
                            parent_id: parent.clone(),
                            title: safe(item["tool"].as_str().unwrap_or("Delegated task")),
                            status: state,
                            summary,
                        }
                        .bounded(),
                    ))
                })
                .collect()
        }
        "subAgentActivity" => vec![AgentStreamEvent::Subagent(
            AgentSubagentUpdate {
                id: safe(item["agentThreadId"].as_str().unwrap_or_default()),
                parent_id: None,
                title: safe(item["agentPath"].as_str().unwrap_or("Delegated task")),
                status: state,
                summary: None,
            }
            .bounded(),
        )],
        "webSearch" => vec![trace(
            id,
            AgentTraceKind::Tool {
                name: "Web search".into(),
                status: state,
                summary: item["query"].as_str().map(field),
            },
        )],
        "imageView" => vec![trace(
            id,
            AgentTraceKind::FileRead {
                path: field(item["path"].as_str().unwrap_or_default()),
                status: state,
                line_start: None,
                line_end: None,
            },
        )],
        "enteredReviewMode" | "exitedReviewMode" => vec![trace(
            id,
            AgentTraceKind::Tool {
                name: "Code review".into(),
                status: state,
                summary: item["review"].as_str().map(field),
            },
        )],
        "contextCompaction" => vec![AgentStreamEvent::Activity(
            "Codex compacted conversation context".into(),
        )],
        // User messages, internal reasoning, hook instructions, raw function
        // outputs and future item kinds do not leak into conversation prose.
        _ => vec![],
    }
}
