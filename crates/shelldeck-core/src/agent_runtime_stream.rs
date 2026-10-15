//! Stateful JSONL normalization. Final snapshots never repeat streamed text.
use super::*;
use std::collections::VecDeque;

/// Keep one parser per run/stdout stream. The stateless compatibility parser
/// cannot correlate partial messages with completed provider snapshots.
pub struct AgentStreamParser {
    provider: AgentProvider,
    claude_message: String,
    claude_index: u64,
    active: Option<StreamingText>,
    seen: VecDeque<(String, String)>,
    streamed: VecDeque<(String, String)>,
}

struct StreamingText {
    id: String,
    raw: String,
    pending: String,
    started: bool,
}

impl AgentStreamParser {
    pub fn new(provider: AgentProvider) -> Self {
        Self {
            provider,
            claude_message: String::new(),
            claude_index: 0,
            active: None,
            seen: VecDeque::new(),
            streamed: VecDeque::new(),
        }
    }

    pub fn parse_line(&mut self, line: &str) -> Vec<AgentStreamEvent> {
        let trimmed = line.trim();
        if trimmed.len() > MAX_AGENT_STREAM_LINE_BYTES {
            return parse_stream_line(self.provider, line);
        }
        let Ok(value) = serde_json::from_str::<Value>(trimmed) else {
            return parse_stream_line(self.provider, line);
        };
        let mut events = match self.provider {
            AgentProvider::Claude => self.claude(&value),
            AgentProvider::Codex => self.codex(&value),
            _ => parse_stream_line(self.provider, line),
        };
        events.extend(metadata_events(self.provider, &value));
        events
    }

    /// Flush a final word after EOF/cancellation without losing a cut stream.
    pub fn finish(&mut self) -> Vec<AgentStreamEvent> {
        self.flush_active(true)
    }

    /// App-server already supplies a delta rather than a CLI snapshot. Keep
    /// the same split-token redaction without repeatedly serializing the
    /// complete accumulated message through JSON.
    pub(crate) fn codex_text_delta(&mut self, id: String, text: &str) -> Vec<AgentStreamEvent> {
        self.delta(id, text)
    }

    pub(crate) fn codex_text_complete(&mut self, id: String, text: &str) -> Vec<AgentStreamEvent> {
        self.complete(id, text)
    }

    fn flush_active(&mut self, finish: bool) -> Vec<AgentStreamEvent> {
        let Some(active) = &mut self.active else {
            return Vec::new();
        };
        let mut end = if finish {
            active.pending.len()
        } else {
            active
                .pending
                .char_indices()
                .rev()
                .find(|(_, c)| c.is_whitespace())
                .map(|(i, c)| i + c.len_utf8())
                .unwrap_or(0)
        };
        if !finish
            && active.pending[..end]
                .split_whitespace()
                .last()
                .is_some_and(|word| word.eq_ignore_ascii_case("bearer"))
        {
            let prefix = active.pending[..end].trim_end();
            end = prefix
                .rfind(char::is_whitespace)
                .map(|i| i + 1)
                .unwrap_or(0);
        }
        if end == 0 {
            return Vec::new();
        }
        let text = safe_visible_text(&active.pending[..end]);
        active.pending.drain(..end);
        if text.is_empty() {
            return Vec::new();
        }
        let event = if active.started {
            AgentStreamEvent::TextDelta(text)
        } else {
            active.started = true;
            AgentStreamEvent::Text(text)
        };
        vec![event]
    }

    fn delta(&mut self, id: String, delta: &str) -> Vec<AgentStreamEvent> {
        let mut events = Vec::new();
        if self.active.as_ref().is_none_or(|active| active.id != id) {
            events.extend(self.finish_text());
            self.active = Some(StreamingText {
                id,
                raw: String::new(),
                pending: String::new(),
                started: false,
            });
        }
        let active = self
            .active
            .as_mut()
            .expect("active stream just initialized");
        let available = MAX_AGENT_STREAM_TEXT_BYTES.saturating_sub(active.raw.len());
        let mut end = available.min(delta.len());
        while !delta.is_char_boundary(end) {
            end -= 1;
        }
        active.raw.push_str(&delta[..end]);
        active.pending.push_str(&delta[..end]);
        events.extend(self.flush_active(false));
        events
    }

    fn finish_text(&mut self) -> Vec<AgentStreamEvent> {
        let events = self.flush_active(true);
        if let Some(active) = self.active.take() {
            self.streamed.push_back((active.id, active.raw));
            while self.streamed.len() > 64 {
                self.streamed.pop_front();
            }
        }
        events
    }

    fn complete(&mut self, id: String, text: &str) -> Vec<AgentStreamEvent> {
        let mut events = Vec::new();
        if self
            .seen
            .iter()
            .any(|(seen_id, seen_text)| seen_id == &id && seen_text == text)
        {
            return events;
        }
        self.seen
            .push_back((id.clone(), truncate_utf8(text, MAX_AGENT_STREAM_TEXT_BYTES)));
        while self.seen.len() > 64 {
            self.seen.pop_front();
        }
        if self.active.as_ref().is_some_and(|active| active.id == id) {
            let raw = self.active.as_ref().unwrap().raw.clone();
            if let Some(rest) = text.strip_prefix(&raw) {
                events.extend(self.delta(id.clone(), rest));
            }
            events.extend(self.finish_text());
            return events;
        }
        if self
            .streamed
            .iter()
            .any(|(stream_id, raw)| stream_id == &id && raw == text)
        {
            return events;
        }
        events.extend(self.finish_text());
        events.push(AgentStreamEvent::Text(safe_visible_text(text)));
        events
    }

    fn claude(&mut self, value: &Value) -> Vec<AgentStreamEvent> {
        if value.get("type").and_then(Value::as_str) == Some("stream_event") {
            let mut events = session_events(value);
            let event = &value["event"];
            match event["type"].as_str() {
                Some("message_start") => {
                    events.extend(self.finish_text());
                    self.claude_message = event
                        .pointer("/message/id")
                        .and_then(Value::as_str)
                        .unwrap_or("stream")
                        .to_string();
                }
                Some("content_block_start") => {
                    self.claude_index = event["index"].as_u64().unwrap_or(0);
                    if event.pointer("/content_block/type").and_then(Value::as_str) == Some("text")
                    {
                        if let Some(text) =
                            event.pointer("/content_block/text").and_then(Value::as_str)
                        {
                            events.extend(self.delta(
                                format!("{}:{}", self.claude_message, self.claude_index),
                                text,
                            ));
                        }
                    }
                }
                Some("content_block_delta")
                    if event.pointer("/delta/type").and_then(Value::as_str)
                        == Some("text_delta") =>
                {
                    let index = event["index"].as_u64().unwrap_or(self.claude_index);
                    if let Some(text) = event.pointer("/delta/text").and_then(Value::as_str) {
                        events.extend(self.delta(format!("{}:{index}", self.claude_message), text));
                    }
                }
                Some("content_block_stop") | Some("message_stop") => {
                    events.extend(self.finish_text())
                }
                _ => {}
            }
            return events;
        }
        let mut events = parse_claude_event(value);
        if value["type"].as_str() == Some("assistant")
            && value["isApiErrorMessage"].as_bool() != Some(true)
        {
            events.retain(|event| !matches!(event, AgentStreamEvent::Text(_)));
            // Complete subagent messages carry a parent tool identity. Their
            // text belongs to delegated activity, not the parent response.
            if let Some(parent) = value["parent_tool_use_id"].as_str() {
                let summary = value.pointer("/message/content").and_then(text_from_value);
                events.push(AgentStreamEvent::Subagent(
                    AgentSubagentUpdate {
                        id: parent.to_string(),
                        parent_id: None,
                        title: "Delegated task".into(),
                        status: AgentTraceStatus::Running,
                        summary,
                    }
                    .bounded(),
                ));
            } else if let Some(content) =
                value.pointer("/message/content").and_then(Value::as_array)
            {
                let message_id = value
                    .pointer("/message/id")
                    .and_then(Value::as_str)
                    .unwrap_or(&self.claude_message)
                    .to_string();
                for (index, block) in content
                    .iter()
                    .enumerate()
                    .filter(|(_, b)| b["type"].as_str() == Some("text"))
                {
                    if let Some(text) = block["text"].as_str() {
                        // CLI versions emit either one complete block at a
                        // time or a complete message. Match the actual block.
                        let matching = self
                            .active
                            .as_ref()
                            .filter(|a| {
                                a.id.starts_with(&format!("{message_id}:"))
                                    && text.starts_with(&a.raw)
                            })
                            .map(|a| a.id.clone())
                            .or_else(|| {
                                self.streamed
                                    .iter()
                                    .rev()
                                    .find(|(id, raw)| {
                                        id.starts_with(&format!("{message_id}:")) && raw == text
                                    })
                                    .map(|(id, _)| id.clone())
                            })
                            .unwrap_or_else(|| format!("{message_id}:{index}"));
                        events.extend(self.complete(matching, text));
                    }
                }
            }
            events.extend(claude_semantic_tools(value));
        }
        if value["type"].as_str() == Some("result") {
            events.extend(self.finish_text());
        }
        events
    }

    fn codex(&mut self, value: &Value) -> Vec<AgentStreamEvent> {
        let kind = value["type"].as_str().unwrap_or_default();
        let item = &value["item"];
        if matches!(kind, "item.started" | "item.updated" | "item.completed")
            && item["type"].as_str() == Some("agent_message")
        {
            let id = item["id"].as_str().unwrap_or("codex-message").to_string();
            let text = item["text"].as_str().unwrap_or_default();
            if kind == "item.completed" {
                return self.complete(id, text);
            }
            let previous = self
                .active
                .as_ref()
                .filter(|a| a.id == id)
                .map(|a| a.raw.as_str())
                .unwrap_or("");
            return match text.strip_prefix(previous) {
                Some(delta) => self.delta(id, delta),
                None => vec![AgentStreamEvent::Activity(
                    "Provider revised an in-progress message".into(),
                )],
            };
        }
        let mut events = if kind == "item.updated" {
            session_events(value)
        } else {
            parse_codex_event(value)
        };
        if matches!(kind, "item.started" | "item.updated" | "item.completed") {
            if item["type"].as_str() == Some("todo_list") {
                events.retain(|e| !matches!(e, AgentStreamEvent::Trace(_)));
                events.push(AgentStreamEvent::Plan(
                    AgentPlanUpdate {
                        steps: item["items"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .take(MAX_AGENT_PLAN_STEPS)
                            .map(|step| AgentPlanStep {
                                text: step["text"].as_str().unwrap_or_default().into(),
                                status: if step["completed"].as_bool() == Some(true) {
                                    AgentPlanStepStatus::Completed
                                } else {
                                    AgentPlanStepStatus::Pending
                                },
                            })
                            .collect(),
                    }
                    .bounded(),
                ));
            } else if kind == "item.updated" {
                events.extend(codex_trace_events(item, AgentTraceStatus::Running));
            }
            if item["type"].as_str() == Some("mcp_tool_call") {
                events.extend(resource_events(
                    item.pointer("/result/content")
                        .and_then(Value::as_array)
                        .map(Vec::as_slice)
                        .unwrap_or(&[]),
                ));
            }
        }
        if matches!(kind, "turn.completed" | "turn.failed" | "error") {
            events.extend(self.finish_text());
        }
        events
    }
}

fn metadata_events(provider: AgentProvider, value: &Value) -> Vec<AgentStreamEvent> {
    let mut events = Vec::new();
    let kind = value["type"].as_str().unwrap_or_default();
    if provider == AgentProvider::Claude
        && kind == "system"
        && value["subtype"].as_str() == Some("init")
    {
        events.push(AgentStreamEvent::Metadata(
            AgentRunMetadata {
                model: value_string(value, &["model"]),
                provider_version: value_string(value, &["claude_code_version"]),
                available_tools: value["tools"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect(),
                available_skills: value["skills"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect(),
            }
            .bounded(),
        ));
    }
    let usage = match (provider, kind) {
        (AgentProvider::Codex, "turn.completed") | (AgentProvider::Claude, "result") => {
            value.get("usage")
        }
        _ => None,
    };
    if let Some(usage) = usage {
        events.push(AgentStreamEvent::Usage(
            AgentUsageUpdate {
                input_tokens: usage["input_tokens"].as_u64(),
                output_tokens: usage["output_tokens"].as_u64(),
                cached_input_tokens: usage
                    .get("cached_input_tokens")
                    .or_else(|| usage.get("cache_read_input_tokens"))
                    .and_then(Value::as_u64),
                cache_write_input_tokens: usage
                    .get("cache_write_input_tokens")
                    .or_else(|| usage.get("cache_creation_input_tokens"))
                    .and_then(Value::as_u64),
                reasoning_output_tokens: usage["reasoning_output_tokens"].as_u64(),
                cost_amount: value["total_cost_usd"]
                    .as_f64()
                    .filter(|v| v.is_finite() && *v >= 0.0)
                    .map(|v| v.to_string()),
                cost_currency: value
                    .get("total_cost_usd")
                    .and_then(Value::as_f64)
                    .map(|_| "USD".into()),
                cumulative: provider == AgentProvider::Claude,
                ..Default::default()
            }
            .bounded(),
        ));
    }
    events
}

fn claude_semantic_tools(value: &Value) -> Vec<AgentStreamEvent> {
    let mut events = Vec::new();
    for block in value
        .pointer("/message/content")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|b| b["type"].as_str() == Some("tool_use"))
    {
        let input = &block["input"];
        match block["name"].as_str() {
            Some("TodoWrite") => events.push(AgentStreamEvent::Plan(
                AgentPlanUpdate {
                    steps: input["todos"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .take(MAX_AGENT_PLAN_STEPS)
                        .map(|step| AgentPlanStep {
                            text: step["content"].as_str().unwrap_or_default().into(),
                            status: match step["status"].as_str() {
                                Some("completed") => AgentPlanStepStatus::Completed,
                                Some("in_progress") => AgentPlanStepStatus::InProgress,
                                _ => AgentPlanStepStatus::Pending,
                            },
                        })
                        .collect(),
                }
                .bounded(),
            )),
            Some("AskUserQuestion") => {
                for (index, question) in input["questions"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .take(MAX_AGENT_CHOICES)
                    .enumerate()
                {
                    events.push(AgentStreamEvent::Question(
                        AgentQuestionRequest {
                            id: format!("{}:{index}", block["id"].as_str().unwrap_or("question")),
                            prompt: question["question"].as_str().unwrap_or_default().into(),
                            options: question["options"]
                                .as_array()
                                .into_iter()
                                .flatten()
                                .take(MAX_AGENT_CHOICES)
                                .map(|o| AgentQuestionOption {
                                    label: o["label"].as_str().unwrap_or_default().into(),
                                    description: o["description"]
                                        .as_str()
                                        .unwrap_or_default()
                                        .into(),
                                })
                                .collect(),
                            multiple: question["multiSelect"].as_bool().unwrap_or(false),
                            response_supported: false,
                        }
                        .bounded(),
                    ));
                }
            }
            Some("Task" | "Agent") => events.push(AgentStreamEvent::Subagent(
                AgentSubagentUpdate {
                    id: block["id"].as_str().unwrap_or("task").into(),
                    parent_id: value["parent_tool_use_id"].as_str().map(str::to_string),
                    title: input["description"]
                        .as_str()
                        .unwrap_or("Delegated task")
                        .into(),
                    status: AgentTraceStatus::Running,
                    summary: None,
                }
                .bounded(),
            )),
            _ => {}
        }
    }
    events
}

pub(super) fn resource_events(content: &[Value]) -> Vec<AgentStreamEvent> {
    content
        .iter()
        .take(64)
        .filter(|item| item["type"].as_str() == Some("resource_link"))
        .filter_map(|item| {
            let uri = item["uri"].as_str()?;
            let title = item["title"]
                .as_str()
                .or_else(|| item["name"].as_str())
                .unwrap_or(uri);
            Some(AgentStreamEvent::Artifact(
                AgentArtifact {
                    id: uri.into(),
                    title: title.into(),
                    uri: uri.into(),
                    mime_type: item["mimeType"].as_str().map(str::to_string),
                }
                .bounded(),
            ))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn text(events: &[AgentStreamEvent]) -> String {
        events
            .iter()
            .filter_map(|e| match e {
                AgentStreamEvent::Text(t) | AgentStreamEvent::TextDelta(t) => Some(t.as_str()),
                _ => None,
            })
            .collect()
    }

    // SDTEST-2230 — SDUC-551
    #[test]
    fn sdtest_2230_claude_partial_blocks_do_not_repeat_completed_snapshots() {
        let mut parser = AgentStreamParser::new(AgentProvider::Claude);
        let fixture = [
            r#"{"type":"stream_event","event":{"type":"message_start","message":{"id":"m1"}}}"#,
            r#"{"type":"stream_event","event":{"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}}"#,
            r#"{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hello "}}}"#,
            r#"{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"world"}}}"#,
            r#"{"type":"assistant","message":{"id":"m1","content":[{"type":"text","text":"Hello world"}]}}"#,
            r#"{"type":"stream_event","event":{"type":"content_block_stop","index":0}}"#,
            r#"{"type":"result","result":"Hello world","usage":{"input_tokens":20,"output_tokens":2},"total_cost_usd":0.01}"#,
        ];
        let events = fixture
            .iter()
            .flat_map(|line| parser.parse_line(line))
            .collect::<Vec<_>>();
        assert_eq!(text(&events), "Hello world");
        assert!(matches!(
            events.last(),
            Some(AgentStreamEvent::Usage(AgentUsageUpdate {
                input_tokens: Some(20),
                cumulative: true,
                ..
            }))
        ));
    }

    // SDTEST-2231 — SDUC-551
    #[test]
    fn sdtest_2231_codex_snapshots_stream_once_and_update_plans_usage() {
        let mut parser = AgentStreamParser::new(AgentProvider::Codex);
        let fixture = [
            r#"{"type":"item.started","item":{"id":"a","type":"agent_message","text":"Checking "}}"#,
            r#"{"type":"item.updated","item":{"id":"a","type":"agent_message","text":"Checking files"}}"#,
            r#"{"type":"item.completed","item":{"id":"a","type":"agent_message","text":"Checking files"}}"#,
            r#"{"type":"item.updated","item":{"id":"p","type":"todo_list","items":[{"text":"Inspect","completed":true},{"text":"Fix","completed":false}]}}"#,
            r#"{"type":"turn.completed","usage":{"input_tokens":19,"cached_input_tokens":7,"output_tokens":3}}"#,
        ];
        let events = fixture
            .iter()
            .flat_map(|line| parser.parse_line(line))
            .collect::<Vec<_>>();
        assert_eq!(text(&events), "Checking files");
        assert!(events.iter().any(|e| matches!(e, AgentStreamEvent::Plan(p) if p.steps.len() == 2 && p.steps[0].status == AgentPlanStepStatus::Completed)));
        assert!(events.iter().any(|e| matches!(e, AgentStreamEvent::Usage(u) if u.cached_input_tokens == Some(7) && !u.cumulative)));
    }

    // SDTEST-2232 — SDUC-551
    #[test]
    fn sdtest_2232_streamed_credentials_are_redacted_across_chunk_boundaries() {
        let mut parser = AgentStreamParser::new(AgentProvider::Claude);
        let events = [r#"{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"sk-secr"}}}"#,
            r#"{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"et123 "}}}"#].iter().flat_map(|line| parser.parse_line(line)).collect::<Vec<_>>();
        assert_eq!(text(&events), "[redacted] ");
    }
}
