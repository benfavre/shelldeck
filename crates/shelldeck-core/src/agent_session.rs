//! Durable, provider-neutral state for the Dev Agents cockpit.
//!
//! Process handles remain in the UI/runtime layer. This module owns the
//! bounded state that may be rendered, retained while panes switch, and
//! serialized without persisting provider resume identifiers.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::agent_runtime::{
    AgentAccessMode, AgentProvider, AgentRunRequest, AgentStreamEvent, AgentTarget,
};
use crate::{Result, ShellDeckError};

pub const DEFAULT_MAX_CONCURRENT_AGENT_SESSIONS: usize = 4;
pub const MAX_CONCURRENT_AGENT_SESSIONS: usize = 8;
pub const MAX_AGENT_SESSIONS: usize = 64;
pub const MAX_AGENT_SESSION_NAME_BYTES: usize = 160;
pub const MAX_AGENT_MESSAGES_PER_SESSION: usize = 256;
pub const MAX_AGENT_TRACE_EVENTS_PER_SESSION: usize = 512;
pub const MAX_AGENT_MESSAGE_BYTES: usize = 128 * 1024;
pub const MAX_AGENT_TRACE_STRING_BYTES: usize = 8 * 1024;
pub const MAX_AGENT_TRACE_CORRELATION_BYTES: usize = 256;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentExecutionContext {
    pub provider: AgentProvider,
    pub target: AgentTarget,
    pub access: AgentAccessMode,
    pub workdir: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

impl AgentExecutionContext {
    pub fn from_request(request: &AgentRunRequest) -> Self {
        Self {
            provider: request.provider,
            target: request.target.clone(),
            access: request.access,
            workdir: request.workdir.trim().to_string(),
            model: request
                .model
                .as_deref()
                .map(str::trim)
                .filter(|model| !model.is_empty())
                .map(str::to_string),
        }
    }

    pub fn run_request(&self, prompt: impl Into<String>) -> AgentRunRequest {
        AgentRunRequest::new(
            self.provider,
            self.target.clone(),
            self.access,
            self.workdir.clone(),
            self.model.clone(),
            prompt,
        )
    }

    fn normalized(mut self) -> Result<Self> {
        self.workdir = self.workdir.trim().to_string();
        self.model = self
            .model
            .take()
            .map(|model| model.trim().to_string())
            .filter(|model| !model.is_empty());
        self.run_request("validate execution context").validate()?;
        Ok(self)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentSessionStatus {
    #[default]
    Idle,
    Starting,
    Running,
    Stopping,
    Completed,
    Failed,
    Cancelled,
}

impl AgentSessionStatus {
    pub fn is_active(self) -> bool {
        matches!(self, Self::Starting | Self::Running | Self::Stopping)
    }

    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Cancelled)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentSessionAttention {
    #[default]
    None,
    Unread,
    NeedsAttention,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentMessageRole {
    User,
    Agent,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentMessage {
    pub id: Uuid,
    #[serde(default)]
    pub sequence: u64,
    pub role: AgentMessageRole,
    pub text: String,
    pub at_ms: i64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentTraceStatus {
    Pending,
    Running,
    Succeeded,
    Failed,
    Cancelled,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AgentTraceKind {
    Command {
        command: String,
        status: AgentTraceStatus,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        exit_code: Option<i32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        summary: Option<String>,
    },
    FileRead {
        path: String,
        #[serde(default)]
        status: AgentTraceStatus,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        line_start: Option<u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        line_end: Option<u32>,
    },
    Diff {
        path: String,
        #[serde(default)]
        status: AgentTraceStatus,
        additions: u32,
        deletions: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        preview: Option<String>,
    },
    Test {
        name: String,
        status: AgentTraceStatus,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        summary: Option<String>,
    },
    Tool {
        name: String,
        status: AgentTraceStatus,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        summary: Option<String>,
    },
    Activity {
        label: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentTraceUpdate {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub correlation_id: Option<String>,
    pub detail: AgentTraceKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentTraceEvent {
    pub id: Uuid,
    #[serde(default)]
    pub sequence: u64,
    pub at_ms: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub correlation_id: Option<String>,
    pub detail: AgentTraceKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentSession {
    pub id: Uuid,
    pub name: String,
    pub context: AgentExecutionContext,
    pub status: AgentSessionStatus,
    pub created_at_ms: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at_ms: Option<i64>,
    pub updated_at_ms: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at_ms: Option<i64>,
    #[serde(default)]
    pub unread_count: u32,
    #[serde(default)]
    pub attention: AgentSessionAttention,
    #[serde(default)]
    pub messages: Vec<AgentMessage>,
    #[serde(default)]
    pub trace: Vec<AgentTraceEvent>,
    #[serde(default = "initial_sequence")]
    next_sequence: u64,
    /// A provider conversation id is scoped to this live application process.
    /// It is deliberately excluded from durable session state.
    #[serde(skip)]
    provider_session_id: Option<String>,
    #[serde(skip)]
    provider_session_context: Option<AgentExecutionContext>,
}

impl AgentSession {
    pub fn new(
        name: impl Into<String>,
        context: AgentExecutionContext,
        now_ms: i64,
    ) -> Result<Self> {
        let name = normalized_name(name.into())?;
        let context = context.normalized()?;
        Ok(Self {
            id: Uuid::new_v4(),
            name,
            context,
            status: AgentSessionStatus::Idle,
            created_at_ms: now_ms,
            started_at_ms: None,
            updated_at_ms: now_ms,
            finished_at_ms: None,
            unread_count: 0,
            attention: AgentSessionAttention::None,
            messages: Vec::new(),
            trace: Vec::new(),
            next_sequence: initial_sequence(),
            provider_session_id: None,
            provider_session_context: None,
        })
    }

    pub fn provider_session_id(&self) -> Option<&str> {
        self.provider_session_id.as_deref()
    }

    pub fn rename(&mut self, name: impl Into<String>, now_ms: i64) -> Result<()> {
        self.name = normalized_name(name.into())?;
        self.updated_at_ms = now_ms;
        Ok(())
    }

    /// Replace the execution context between runs. Resume authority is cleared
    /// whenever any provider, target, access, workdir, or model field changes.
    pub fn set_context(&mut self, context: AgentExecutionContext, now_ms: i64) -> Result<bool> {
        if self.status.is_active() {
            return Err(ShellDeckError::Config(
                "stop the agent session before changing its execution context".to_string(),
            ));
        }
        let context = context.normalized()?;
        if context == self.context {
            return Ok(false);
        }
        self.context = context;
        self.provider_session_id = None;
        self.provider_session_context = None;
        self.updated_at_ms = now_ms;
        Ok(true)
    }

    pub fn run_request(&self, prompt: impl Into<String>) -> AgentRunRequest {
        let resume_session = self
            .provider_session_context
            .as_ref()
            .filter(|context| *context == &self.context)
            .and(self.provider_session_id.clone());
        self.context
            .run_request(prompt)
            .with_resume_session(resume_session)
    }

    pub fn begin_run(&mut self, prompt: impl Into<String>, now_ms: i64) -> Result<AgentRunRequest> {
        if self.status.is_active() {
            return Err(ShellDeckError::Config(
                "agent session is already running".to_string(),
            ));
        }
        let prompt = prompt.into();
        let request = self.run_request(prompt.clone());
        request.validate()?;
        self.status = AgentSessionStatus::Starting;
        self.started_at_ms = Some(now_ms);
        self.updated_at_ms = now_ms;
        self.finished_at_ms = None;
        self.push_message(AgentMessageRole::User, prompt, now_ms);
        Ok(request)
    }

    pub fn request_stop(&mut self, now_ms: i64) -> bool {
        if matches!(
            self.status,
            AgentSessionStatus::Starting | AgentSessionStatus::Running
        ) {
            self.status = AgentSessionStatus::Stopping;
            self.updated_at_ms = now_ms;
            true
        } else {
            false
        }
    }

    pub fn apply_stream_event(&mut self, event: AgentStreamEvent, now_ms: i64, visible: bool) {
        let mut user_visible = true;
        match event {
            AgentStreamEvent::Text(text) => self.push_message(
                AgentMessageRole::Agent,
                sanitize_retained_output(&text, MAX_AGENT_MESSAGE_BYTES),
                now_ms,
            ),
            AgentStreamEvent::TextDelta(delta) => self.push_text_delta(
                sanitize_retained_output(&delta, MAX_AGENT_MESSAGE_BYTES),
                now_ms,
            ),
            AgentStreamEvent::Session(session_id) => {
                self.provider_session_id = Some(session_id);
                self.provider_session_context = Some(self.context.clone());
                user_visible = false;
            }
            AgentStreamEvent::Ready => {
                self.status = AgentSessionStatus::Running;
                user_visible = false;
            }
            AgentStreamEvent::Trace(update) => {
                user_visible = self.upsert_trace(update, now_ms);
            }
            AgentStreamEvent::TraceStatus {
                correlation_id,
                status,
                summary,
            } => {
                user_visible = false;
                self.update_trace_status(&correlation_id, status, summary, now_ms);
            }
            AgentStreamEvent::Activity(label) => {
                self.upsert_trace(
                    AgentTraceUpdate {
                        correlation_id: None,
                        detail: AgentTraceKind::Activity { label },
                    },
                    now_ms,
                );
            }
            AgentStreamEvent::Error(error) => {
                self.push_message(
                    AgentMessageRole::Error,
                    sanitize_retained_output(&error, MAX_AGENT_MESSAGE_BYTES),
                    now_ms,
                );
                self.attention = AgentSessionAttention::NeedsAttention;
            }
        }
        self.updated_at_ms = now_ms;
        if user_visible && !visible {
            self.unread_count = self.unread_count.saturating_add(1);
            if self.attention == AgentSessionAttention::None {
                self.attention = AgentSessionAttention::Unread;
            }
        }
    }

    pub fn finish(&mut self, status: AgentSessionStatus, now_ms: i64, visible: bool) -> Result<()> {
        if !matches!(
            status,
            AgentSessionStatus::Completed
                | AgentSessionStatus::Failed
                | AgentSessionStatus::Cancelled
        ) {
            return Err(ShellDeckError::Config(
                "agent run must finish with a terminal status".to_string(),
            ));
        }
        self.status = status;
        self.updated_at_ms = now_ms;
        self.finished_at_ms = Some(now_ms);
        if !visible {
            self.unread_count = self.unread_count.saturating_add(1);
            if status == AgentSessionStatus::Failed {
                self.attention = AgentSessionAttention::NeedsAttention;
            } else if self.attention == AgentSessionAttention::None {
                self.attention = AgentSessionAttention::Unread;
            }
        } else if status == AgentSessionStatus::Failed {
            self.attention = AgentSessionAttention::NeedsAttention;
        }
        Ok(())
    }

    pub fn mark_read(&mut self) {
        self.unread_count = 0;
        if self.attention == AgentSessionAttention::Unread {
            self.attention = AgentSessionAttention::None;
        }
    }

    pub fn acknowledge_attention(&mut self) {
        self.unread_count = 0;
        self.attention = AgentSessionAttention::None;
    }

    fn push_message(&mut self, role: AgentMessageRole, text: String, now_ms: i64) {
        let sequence = self.allocate_sequence();
        push_bounded(
            &mut self.messages,
            AgentMessage {
                id: Uuid::new_v4(),
                sequence,
                role,
                text: truncate_utf8(&text, MAX_AGENT_MESSAGE_BYTES),
                at_ms: now_ms,
            },
            MAX_AGENT_MESSAGES_PER_SESSION,
        );
    }

    fn push_text_delta(&mut self, delta: String, now_ms: i64) {
        if let Some(message) = self
            .messages
            .last_mut()
            .filter(|message| message.role == AgentMessageRole::Agent)
        {
            let remaining = MAX_AGENT_MESSAGE_BYTES.saturating_sub(message.text.len());
            if remaining > 0 {
                message.text.push_str(&truncate_utf8(&delta, remaining));
            }
        } else {
            self.push_message(AgentMessageRole::Agent, delta, now_ms);
        }
    }

    fn upsert_trace(&mut self, update: AgentTraceUpdate, now_ms: i64) -> bool {
        let detail = bounded_trace(update.detail);
        let correlation_id = update
            .correlation_id
            .map(|id| truncate_utf8(&id, MAX_AGENT_TRACE_CORRELATION_BYTES));
        if let Some(existing) = correlation_id.as_deref().and_then(|id| {
            self.trace
                .iter_mut()
                .rev()
                .find(|event| event.correlation_id.as_deref() == Some(id))
        }) {
            existing.detail = detail;
            return false;
        }
        if matches!(&detail, AgentTraceKind::Activity { label } if self.trace.last().is_some_and(|last| matches!(&last.detail, AgentTraceKind::Activity { label: previous } if previous == label)))
        {
            return false;
        }
        let sequence = self.allocate_sequence();
        push_bounded(
            &mut self.trace,
            AgentTraceEvent {
                id: Uuid::new_v4(),
                sequence,
                at_ms: now_ms,
                correlation_id,
                detail,
            },
            MAX_AGENT_TRACE_EVENTS_PER_SESSION,
        );
        true
    }

    fn update_trace_status(
        &mut self,
        correlation_id: &str,
        status: AgentTraceStatus,
        summary: Option<String>,
        _now_ms: i64,
    ) {
        let correlation_id = truncate_utf8(correlation_id, MAX_AGENT_TRACE_CORRELATION_BYTES);
        let Some(event) = self
            .trace
            .iter_mut()
            .rev()
            .find(|event| event.correlation_id.as_deref() == Some(correlation_id.as_str()))
        else {
            return;
        };
        set_trace_status(&mut event.detail, status);
        if let Some(summary) = summary {
            set_trace_summary(
                &mut event.detail,
                sanitize_retained_output(&summary, MAX_AGENT_TRACE_STRING_BYTES),
            );
        }
    }

    fn allocate_sequence(&mut self) -> u64 {
        let retained_max = self
            .messages
            .last()
            .map(|message| message.sequence)
            .into_iter()
            .chain(self.trace.last().map(|trace| trace.sequence))
            .max()
            .unwrap_or(0);
        let sequence = self.next_sequence.max(retained_max.saturating_add(1));
        self.next_sequence = sequence.saturating_add(1);
        sequence
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentSessionCollection {
    #[serde(default)]
    sessions: Vec<AgentSession>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    selected_id: Option<Uuid>,
    max_concurrent: usize,
    #[serde(skip)]
    surface_visible: bool,
}

impl Default for AgentSessionCollection {
    fn default() -> Self {
        Self::new(DEFAULT_MAX_CONCURRENT_AGENT_SESSIONS)
            .expect("the default agent concurrency limit is valid")
    }
}

impl AgentSessionCollection {
    pub fn new(max_concurrent: usize) -> Result<Self> {
        if !(1..=MAX_CONCURRENT_AGENT_SESSIONS).contains(&max_concurrent) {
            return Err(ShellDeckError::Config(format!(
                "agent concurrency limit must be between 1 and {MAX_CONCURRENT_AGENT_SESSIONS}"
            )));
        }
        Ok(Self {
            sessions: Vec::new(),
            selected_id: None,
            max_concurrent,
            surface_visible: false,
        })
    }

    pub fn sessions(&self) -> &[AgentSession] {
        &self.sessions
    }

    pub fn selected_id(&self) -> Option<Uuid> {
        self.selected_id
    }

    pub fn selected(&self) -> Option<&AgentSession> {
        let id = self.selected_id?;
        self.get(id)
    }

    pub fn get(&self, id: Uuid) -> Option<&AgentSession> {
        self.sessions.iter().find(|session| session.id == id)
    }

    pub fn get_mut(&mut self, id: Uuid) -> Option<&mut AgentSession> {
        self.sessions.iter_mut().find(|session| session.id == id)
    }

    pub fn active_count(&self) -> usize {
        self.sessions
            .iter()
            .filter(|session| session.status.is_active())
            .count()
    }

    pub fn has_capacity(&self) -> bool {
        self.active_count() < self.max_concurrent
    }

    pub fn create(
        &mut self,
        name: impl Into<String>,
        context: AgentExecutionContext,
        now_ms: i64,
    ) -> Result<Uuid> {
        if self.sessions.len() >= MAX_AGENT_SESSIONS {
            return Err(ShellDeckError::Config(format!(
                "at most {MAX_AGENT_SESSIONS} agent sessions may be retained"
            )));
        }
        let session = AgentSession::new(name, context, now_ms)?;
        let id = session.id;
        self.sessions.push(session);
        self.select(id)?;
        Ok(id)
    }

    pub fn select(&mut self, id: Uuid) -> Result<()> {
        if self.get(id).is_none() {
            return Err(ShellDeckError::Config(
                "agent session no longer exists".to_string(),
            ));
        }
        self.selected_id = Some(id);
        if self.surface_visible {
            self.get_mut(id)
                .expect("session existence was checked")
                .mark_read();
        }
        Ok(())
    }

    /// Set whether the selected session is actually presented on screen.
    /// Selection is navigation identity; only visibility consumes unread state.
    pub fn set_surface_visible(&mut self, visible: bool) -> bool {
        if self.surface_visible == visible {
            return false;
        }
        self.surface_visible = visible;
        if visible {
            if let Some(id) = self.selected_id {
                if let Some(session) = self.get_mut(id) {
                    session.mark_read();
                }
            }
        }
        true
    }

    pub fn surface_visible(&self) -> bool {
        self.surface_visible
    }

    pub fn begin_run(
        &mut self,
        id: Uuid,
        prompt: impl Into<String>,
        now_ms: i64,
    ) -> Result<AgentRunRequest> {
        let already_active = self
            .get(id)
            .ok_or_else(|| ShellDeckError::Config("agent session no longer exists".to_string()))?
            .status
            .is_active();
        if !already_active && !self.has_capacity() {
            return Err(ShellDeckError::Config(format!(
                "at most {} agent sessions may run concurrently",
                self.max_concurrent
            )));
        }
        self.get_mut(id)
            .expect("session existence was checked")
            .begin_run(prompt, now_ms)
    }

    pub fn apply_stream_event(
        &mut self,
        id: Uuid,
        event: AgentStreamEvent,
        now_ms: i64,
    ) -> Result<()> {
        let visible = self.surface_visible && self.selected_id == Some(id);
        self.get_mut(id)
            .ok_or_else(|| ShellDeckError::Config("agent session no longer exists".to_string()))?
            .apply_stream_event(event, now_ms, visible);
        Ok(())
    }

    pub fn finish(&mut self, id: Uuid, status: AgentSessionStatus, now_ms: i64) -> Result<()> {
        let visible = self.surface_visible && self.selected_id == Some(id);
        self.get_mut(id)
            .ok_or_else(|| ShellDeckError::Config("agent session no longer exists".to_string()))?
            .finish(status, now_ms, visible)
    }

    pub fn remove(&mut self, id: Uuid) -> Result<AgentSession> {
        let index = self
            .sessions
            .iter()
            .position(|session| session.id == id)
            .ok_or_else(|| ShellDeckError::Config("agent session no longer exists".to_string()))?;
        if self.sessions[index].status.is_active() {
            return Err(ShellDeckError::Config(
                "stop the agent session before removing it".to_string(),
            ));
        }
        let removed = self.sessions.remove(index);
        if self.selected_id == Some(id) {
            self.selected_id = self
                .sessions
                .get(index)
                .or_else(|| self.sessions.last())
                .map(|s| s.id);
            if self.surface_visible {
                if let Some(selected_id) = self.selected_id {
                    if let Some(session) = self.get_mut(selected_id) {
                        session.mark_read();
                    }
                }
            }
        }
        Ok(removed)
    }

    /// An active process cannot survive application restart. This explicit
    /// recovery step makes stale durable state presentable but never claims
    /// that work is still running.
    pub fn recover_interrupted(&mut self, now_ms: i64) {
        for session in &mut self.sessions {
            if session.status.is_active() {
                session.status = AgentSessionStatus::Failed;
                session.updated_at_ms = now_ms;
                session.finished_at_ms = Some(now_ms);
                session.attention = AgentSessionAttention::NeedsAttention;
                session.provider_session_id = None;
                session.provider_session_context = None;
            }
        }
    }
}

fn normalized_name(name: String) -> Result<String> {
    let name = name.trim();
    if name.is_empty() {
        return Err(ShellDeckError::Config(
            "agent session name must not be empty".to_string(),
        ));
    }
    if name.len() > MAX_AGENT_SESSION_NAME_BYTES {
        return Err(ShellDeckError::Config(
            "agent session name is too long".to_string(),
        ));
    }
    Ok(name.to_string())
}

const fn initial_sequence() -> u64 {
    1
}

fn push_bounded<T>(items: &mut Vec<T>, item: T, limit: usize) {
    if items.len() == limit {
        items.remove(0);
    }
    items.push(item);
}

fn bounded_trace(detail: AgentTraceKind) -> AgentTraceKind {
    match detail {
        AgentTraceKind::Command {
            command,
            status,
            exit_code,
            summary,
        } => AgentTraceKind::Command {
            command: sanitize_retained_output(&command, MAX_AGENT_TRACE_STRING_BYTES),
            status,
            exit_code,
            summary: summary
                .map(|text| sanitize_retained_output(&text, MAX_AGENT_TRACE_STRING_BYTES)),
        },
        AgentTraceKind::FileRead {
            path,
            status,
            line_start,
            line_end,
        } => AgentTraceKind::FileRead {
            path: sanitize_retained_output(&path, MAX_AGENT_TRACE_STRING_BYTES),
            status,
            line_start,
            line_end,
        },
        AgentTraceKind::Diff {
            path,
            status,
            additions,
            deletions,
            preview,
        } => AgentTraceKind::Diff {
            path: sanitize_retained_output(&path, MAX_AGENT_TRACE_STRING_BYTES),
            status,
            additions,
            deletions,
            preview: preview
                .map(|text| sanitize_retained_output(&text, MAX_AGENT_TRACE_STRING_BYTES)),
        },
        AgentTraceKind::Test {
            name,
            status,
            summary,
        } => AgentTraceKind::Test {
            name: sanitize_retained_output(&name, MAX_AGENT_TRACE_STRING_BYTES),
            status,
            summary: summary
                .map(|text| sanitize_retained_output(&text, MAX_AGENT_TRACE_STRING_BYTES)),
        },
        AgentTraceKind::Tool {
            name,
            status,
            summary,
        } => AgentTraceKind::Tool {
            name: sanitize_retained_output(&name, MAX_AGENT_TRACE_STRING_BYTES),
            status,
            summary: summary
                .map(|text| sanitize_retained_output(&text, MAX_AGENT_TRACE_STRING_BYTES)),
        },
        AgentTraceKind::Activity { label } => AgentTraceKind::Activity {
            label: sanitize_retained_output(&label, MAX_AGENT_TRACE_STRING_BYTES),
        },
    }
}

fn set_trace_status(detail: &mut AgentTraceKind, new_status: AgentTraceStatus) {
    match detail {
        AgentTraceKind::Command { status, .. }
        | AgentTraceKind::FileRead { status, .. }
        | AgentTraceKind::Diff { status, .. }
        | AgentTraceKind::Test { status, .. }
        | AgentTraceKind::Tool { status, .. } => *status = new_status,
        AgentTraceKind::Activity { .. } => {}
    }
}

fn set_trace_summary(detail: &mut AgentTraceKind, new_summary: String) {
    match detail {
        AgentTraceKind::Command { summary, .. }
        | AgentTraceKind::Test { summary, .. }
        | AgentTraceKind::Tool { summary, .. } => *summary = Some(new_summary),
        AgentTraceKind::FileRead { .. }
        | AgentTraceKind::Diff { .. }
        | AgentTraceKind::Activity { .. } => {}
    }
}

fn truncate_utf8(text: &str, max_bytes: usize) -> String {
    if text.len() <= max_bytes {
        return text.to_string();
    }
    let suffix = "…";
    if max_bytes < suffix.len() {
        return String::new();
    }
    let mut end = max_bytes.saturating_sub(suffix.len()).min(text.len());
    while !text.is_char_boundary(end) {
        end = end.saturating_sub(1);
    }
    format!("{}{suffix}", &text[..end])
}

fn sanitize_retained_output(text: &str, max_bytes: usize) -> String {
    truncate_utf8(&redact_credentials(text), max_bytes)
}

fn redact_credentials(text: &str) -> String {
    let mut redact_next = false;
    text.split_inclusive(char::is_whitespace)
        .map(|part| {
            let trimmed = part.trim_end_matches(char::is_whitespace);
            let suffix = &part[trimmed.len()..];
            if redact_next && !trimmed.is_empty() {
                redact_next = false;
                return format!("[redacted]{suffix}");
            }
            if trimmed.eq_ignore_ascii_case("bearer") {
                redact_next = true;
                return part.to_string();
            }
            if ["sk-", "ghp_", "github_pat_", "xoxb-", "xoxp-"]
                .iter()
                .any(|prefix| trimmed.starts_with(prefix) && trimmed.len() > prefix.len() + 4)
            {
                return format!("[redacted]{suffix}");
            }
            if let Some((key, _)) = trimmed.split_once('=') {
                let key_upper = key.to_ascii_uppercase();
                if [
                    "TOKEN",
                    "SECRET",
                    "PASSWORD",
                    "PASSWD",
                    "API_KEY",
                    "PRIVATE_KEY",
                ]
                .iter()
                .any(|sensitive| key_upper.contains(sensitive))
                {
                    return format!("{key}=[redacted]{suffix}");
                }
            }
            part.to_string()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(not(windows))]
    const TEST_WORKDIR: &str = "/srv/project";
    #[cfg(windows)]
    const TEST_WORKDIR: &str = r"C:\srv\project";

    fn context(provider: AgentProvider) -> AgentExecutionContext {
        AgentExecutionContext {
            provider,
            target: AgentTarget::Local,
            access: AgentAccessMode::WorkspaceWrite,
            workdir: TEST_WORKDIR.to_string(),
            model: None,
        }
    }

    // SDTEST-1879 — SDUC-499
    #[test]
    fn sdtest_1879_collection_runs_bounded_sessions_concurrently_and_tracks_attention() {
        let mut sessions = AgentSessionCollection::new(2).unwrap();
        let first = sessions
            .create("Fix parser", context(AgentProvider::Codex), 10)
            .unwrap();
        let second = sessions
            .create("Add tests", context(AgentProvider::Claude), 11)
            .unwrap();
        let waiting = sessions
            .create("Review", context(AgentProvider::Jcode), 12)
            .unwrap();

        sessions.begin_run(first, "fix it", 20).unwrap();
        sessions.begin_run(second, "test it", 21).unwrap();
        assert_eq!(sessions.active_count(), 2);
        assert!(sessions.begin_run(waiting, "review it", 22).is_err());

        sessions
            .apply_stream_event(first, AgentStreamEvent::Ready, 23)
            .unwrap();
        sessions
            .apply_stream_event(
                first,
                AgentStreamEvent::Trace(AgentTraceUpdate {
                    correlation_id: Some("read-1".to_string()),
                    detail: AgentTraceKind::FileRead {
                        path: "src/lib.rs".to_string(),
                        status: AgentTraceStatus::Running,
                        line_start: Some(1),
                        line_end: Some(20),
                    },
                }),
                24,
            )
            .unwrap();
        sessions
            .apply_stream_event(
                first,
                AgentStreamEvent::TraceStatus {
                    correlation_id: "read-1".to_string(),
                    status: AgentTraceStatus::Succeeded,
                    summary: None,
                },
                25,
            )
            .unwrap();
        sessions
            .apply_stream_event(first, AgentStreamEvent::Text("Implemented".to_string()), 26)
            .unwrap();
        let first_session = sessions.get(first).unwrap();
        assert_eq!(first_session.status, AgentSessionStatus::Running);
        assert_eq!(first_session.unread_count, 2);
        assert_eq!(first_session.attention, AgentSessionAttention::Unread);
        assert_eq!(first_session.trace.len(), 1);
        assert!(matches!(
            first_session.trace[0].detail,
            AgentTraceKind::FileRead {
                status: AgentTraceStatus::Succeeded,
                ..
            }
        ));

        sessions.set_surface_visible(true);
        sessions.select(first).unwrap();
        assert_eq!(sessions.get(first).unwrap().unread_count, 0);
        sessions
            .finish(second, AgentSessionStatus::Failed, 30)
            .unwrap();
        assert_eq!(
            sessions.get(second).unwrap().attention,
            AgentSessionAttention::NeedsAttention
        );
        assert!(sessions.has_capacity());
    }

    // SDTEST-1881 — SDUC-499
    #[test]
    fn sdtest_1881_durable_state_excludes_resume_ids_recovers_runs_and_bounds_streams() {
        let mut sessions = AgentSessionCollection::default();
        let id = sessions
            .create("Bound output", context(AgentProvider::Claude), 10)
            .unwrap();
        sessions.begin_run(id, "inspect", 11).unwrap();
        sessions
            .apply_stream_event(
                id,
                AgentStreamEvent::Session("provider-secret-resume-id".to_string()),
                12,
            )
            .unwrap();
        {
            let session = sessions.get_mut(id).unwrap();
            session.context.model = Some("changed-model".to_string());
            assert_eq!(session.run_request("continue").resume_session, None);
            session.context.model = None;
            assert_eq!(
                session.run_request("continue").resume_session.as_deref(),
                Some("provider-secret-resume-id")
            );
        }
        for _ in 0..3 {
            sessions
                .apply_stream_event(
                    id,
                    AgentStreamEvent::TextDelta("é".repeat(MAX_AGENT_MESSAGE_BYTES)),
                    13,
                )
                .unwrap();
        }
        sessions
            .apply_stream_event(
                id,
                AgentStreamEvent::Trace(AgentTraceUpdate {
                    correlation_id: Some("command-1".to_string()),
                    detail: AgentTraceKind::Command {
                        command: "é".repeat(MAX_AGENT_TRACE_STRING_BYTES),
                        status: AgentTraceStatus::Running,
                        exit_code: None,
                        summary: None,
                    },
                }),
                14,
            )
            .unwrap();
        let session = sessions.get(id).unwrap();
        assert!(session.messages.last().unwrap().text.len() <= MAX_AGENT_MESSAGE_BYTES);
        let AgentTraceKind::Command { command, .. } = &session.trace.last().unwrap().detail else {
            panic!("expected command trace");
        };
        assert!(command.len() <= MAX_AGENT_TRACE_STRING_BYTES);

        let json = serde_json::to_string(&sessions).unwrap();
        assert!(!json.contains("provider-secret-resume-id"));
        let mut restored: AgentSessionCollection = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.get(id).unwrap().provider_session_id(), None);
        restored.recover_interrupted(20);
        assert_eq!(restored.get(id).unwrap().status, AgentSessionStatus::Failed);
        assert_eq!(
            restored.get(id).unwrap().attention,
            AgentSessionAttention::NeedsAttention
        );
    }

    // SDTEST-1895 — SDUC-499
    #[test]
    fn sdtest_1895_equal_timestamps_and_incremental_updates_keep_one_monotonic_timeline() {
        let mut session = AgentSession::new("Sequence", context(AgentProvider::Codex), 10).unwrap();
        session.begin_run("first", 20).unwrap();
        session.apply_stream_event(
            AgentStreamEvent::Trace(AgentTraceUpdate {
                correlation_id: Some("command-1".to_string()),
                detail: AgentTraceKind::Command {
                    command: "cargo check".to_string(),
                    status: AgentTraceStatus::Running,
                    exit_code: None,
                    summary: None,
                },
            }),
            20,
            true,
        );
        session.apply_stream_event(AgentStreamEvent::Text("working".to_string()), 20, true);
        session.apply_stream_event(AgentStreamEvent::TextDelta("…done".to_string()), 30, true);
        session.apply_stream_event(
            AgentStreamEvent::Trace(AgentTraceUpdate {
                correlation_id: Some("command-1".to_string()),
                detail: AgentTraceKind::Command {
                    command: "cargo check".to_string(),
                    status: AgentTraceStatus::Succeeded,
                    exit_code: Some(0),
                    summary: None,
                },
            }),
            40,
            true,
        );

        let mut timeline = session
            .messages
            .iter()
            .map(|message| (message.sequence, "message", message.at_ms))
            .chain(
                session
                    .trace
                    .iter()
                    .map(|trace| (trace.sequence, "trace", trace.at_ms)),
            )
            .collect::<Vec<_>>();
        timeline.sort_by_key(|item| item.0);
        assert_eq!(
            timeline,
            vec![(1, "message", 20), (2, "trace", 20), (3, "message", 20)]
        );
        assert_eq!(session.messages[1].text, "working…done");
        assert!(matches!(
            session.trace[0].detail,
            AgentTraceKind::Command {
                status: AgentTraceStatus::Succeeded,
                exit_code: Some(0),
                ..
            }
        ));
    }

    // SDTEST-1898 — SDUC-499
    #[test]
    fn sdtest_1898_selected_but_hidden_session_accumulates_unread_until_visible() {
        let mut sessions = AgentSessionCollection::default();
        let id = sessions
            .create("Visibility", context(AgentProvider::Claude), 10)
            .unwrap();
        assert_eq!(sessions.selected_id(), Some(id));
        assert!(!sessions.surface_visible());

        sessions
            .apply_stream_event(id, AgentStreamEvent::Text("hidden output".to_string()), 20)
            .unwrap();
        assert_eq!(sessions.get(id).unwrap().unread_count, 1);

        assert!(sessions.set_surface_visible(true));
        assert_eq!(sessions.get(id).unwrap().unread_count, 0);
        sessions
            .apply_stream_event(id, AgentStreamEvent::Text("visible output".to_string()), 30)
            .unwrap();
        assert_eq!(sessions.get(id).unwrap().unread_count, 0);

        assert!(sessions.set_surface_visible(false));
        sessions
            .apply_stream_event(id, AgentStreamEvent::Text("hidden again".to_string()), 40)
            .unwrap();
        assert_eq!(sessions.get(id).unwrap().unread_count, 1);
    }
}
