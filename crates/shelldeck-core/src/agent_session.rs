//! Durable, provider-neutral state for the Dev Agents cockpit.
//!
//! Process handles remain in the UI/runtime layer. This module owns the
//! bounded state that may be rendered, retained while panes switch, and
//! serialized without persisting provider resume identifiers.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::agent_runtime::{
    AgentAccessMode, AgentApprovalRequest, AgentArtifact, AgentPlanUpdate, AgentProvider,
    AgentQuestionRequest, AgentRunMetadata, AgentRunRequest, AgentSource, AgentStreamEvent,
    AgentSubagentUpdate, AgentTarget, AgentUsageUpdate,
};
use crate::{Result, ShellDeckError};

#[path = "agent_session_storage.rs"]
pub mod storage;

pub const DEFAULT_MAX_CONCURRENT_AGENT_SESSIONS: usize = 4;
pub const MAX_CONCURRENT_AGENT_SESSIONS: usize = 8;
pub const MAX_AGENT_SESSIONS: usize = 64;
pub const MAX_AGENT_SESSION_NAME_BYTES: usize = 160;
pub const MAX_AGENT_MESSAGES_PER_SESSION: usize = 256;
pub const MAX_AGENT_TRACE_EVENTS_PER_SESSION: usize = 512;
pub const MAX_AGENT_MESSAGE_BYTES: usize = 128 * 1024;
pub const MAX_AGENT_TRACE_STRING_BYTES: usize = 8 * 1024;
pub const MAX_AGENT_TRACE_CORRELATION_BYTES: usize = 256;
pub const MAX_AGENT_TURNS_PER_SESSION: usize = 256;
pub const MAX_AGENT_BOUNDARIES_PER_SESSION: usize = 128;

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
        if self.workdir.contains(['\0', '\r', '\n'])
            || self
                .model
                .as_ref()
                .is_some_and(|model| model.chars().any(char::is_control))
        {
            return Err(ShellDeckError::Config(
                "invalid control character in agent context".into(),
            ));
        }
        if let AgentTarget::Ssh {
            connection_id,
            connection_label,
        } = &mut self.target
        {
            if connection_id.is_nil() || connection_label.chars().any(char::is_control) {
                return Err(ShellDeckError::Config("invalid SSH agent target".into()));
            }
            *connection_label =
                truncate_utf8(connection_label.trim(), MAX_AGENT_SESSION_NAME_BYTES);
        }
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<Uuid>,
    pub role: AgentMessageRole,
    /// Original author survives later provider changes in the same session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<AgentProvider>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<Uuid>,
    pub at_ms: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub correlation_id: Option<String>,
    pub detail: AgentTraceKind,
}

/// A turn is the application run, never a provider's opaque conversation id.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentTurn {
    pub id: Uuid,
    pub context: AgentExecutionContext,
    pub started_at_ms: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at_ms: Option<i64>,
    pub status: AgentSessionStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentHistoryBoundaryReason {
    ContextChanged,
    ApplicationRestart,
    ConversationNotResumed,
    ForkedConversation,
}

impl AgentHistoryBoundaryReason {
    pub fn label(self) -> &'static str {
        match self {
            Self::ContextChanged => {
                "Execution context changed; previous messages are reference history"
            }
            Self::ApplicationRestart => {
                "Application restarted; a new provider conversation will start"
            }
            Self::ConversationNotResumed => {
                "New provider conversation; previous messages were not sent"
            }
            Self::ForkedConversation => {
                "Conversation copied; previous messages are reference history"
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentHistoryBoundary {
    pub id: Uuid,
    pub sequence: u64,
    pub at_ms: i64,
    pub reason: AgentHistoryBoundaryReason,
    pub context: AgentExecutionContext,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentSession {
    pub id: Uuid,
    pub name: String,
    #[serde(default)]
    pub pinned: bool,
    #[serde(default)]
    pub archived: bool,
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
    #[serde(default)]
    pub turns: Vec<AgentTurn>,
    #[serde(default)]
    pub boundaries: Vec<AgentHistoryBoundary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan: Option<AgentPlanUpdate>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<AgentUsageUpdate>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<AgentRunMetadata>,
    /// Requests carry live routing authority and cannot survive restart.
    #[serde(skip)]
    pub question: Option<AgentQuestionRequest>,
    #[serde(skip)]
    pub approval: Option<AgentApprovalRequest>,
    #[serde(default)]
    pub sources: Vec<AgentSource>,
    #[serde(default)]
    pub artifacts: Vec<AgentArtifact>,
    #[serde(default)]
    pub subagents: Vec<AgentSubagentUpdate>,
    #[serde(default = "initial_sequence")]
    next_sequence: u64,
    /// A provider conversation id is scoped to this live application process.
    /// It is deliberately excluded from durable session state.
    #[serde(skip)]
    provider_session_id: Option<String>,
    #[serde(skip)]
    provider_session_context: Option<AgentExecutionContext>,
    #[serde(skip)]
    current_turn_id: Option<Uuid>,
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
            pinned: false,
            archived: false,
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
            turns: Vec::new(),
            boundaries: Vec::new(),
            plan: None,
            usage: None,
            metadata: None,
            question: None,
            approval: None,
            sources: Vec::new(),
            artifacts: Vec::new(),
            subagents: Vec::new(),
            next_sequence: initial_sequence(),
            provider_session_id: None,
            provider_session_context: None,
            current_turn_id: None,
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
        if !self.messages.is_empty() || !self.trace.is_empty() {
            self.push_boundary(AgentHistoryBoundaryReason::ContextChanged, now_ms);
        }
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
        if self.archived {
            return Err(ShellDeckError::Config(
                "restore the archived agent session before running it".into(),
            ));
        }
        if request.resume_session.is_none()
            && !self.messages.is_empty()
            && self.messages.last().is_some_and(|message| {
                self.boundaries
                    .last()
                    .is_none_or(|boundary| message.sequence > boundary.sequence)
            })
        {
            self.push_boundary(AgentHistoryBoundaryReason::ConversationNotResumed, now_ms);
        }
        self.current_turn_id = Some(request.id);
        self.plan = None;
        self.usage = None;
        self.question = None;
        self.approval = None;
        self.subagents.clear();
        push_bounded(
            &mut self.turns,
            AgentTurn {
                id: request.id,
                context: self.context.clone(),
                started_at_ms: now_ms,
                finished_at_ms: None,
                status: AgentSessionStatus::Starting,
            },
            MAX_AGENT_TURNS_PER_SESSION,
        );
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
            if let Some(turn) = self.current_turn_mut() {
                turn.status = AgentSessionStatus::Stopping;
            }
            self.updated_at_ms = now_ms;
            true
        } else {
            false
        }
    }

    /// Reconcile a catalog-admitted checkout path before this exact run starts.
    /// This narrow admission operation cannot change provider, target, access,
    /// or model, and cannot alter an already running or different turn.
    pub fn admit_starting_workdir(
        &mut self,
        request: &mut AgentRunRequest,
        admitted_workdir: &str,
        now_ms: i64,
    ) -> Result<bool> {
        if self.status != AgentSessionStatus::Starting
            || self.current_turn_id != Some(request.id)
            || !self.turns.last().is_some_and(|turn| {
                turn.id == request.id
                    && turn.status == AgentSessionStatus::Starting
                    && turn.context == self.context
            })
            || AgentExecutionContext::from_request(request) != self.context
        {
            return Err(ShellDeckError::Config(
                "agent checkout admission does not match the starting run".into(),
            ));
        }
        let mut admitted = request.clone();
        admitted.workdir = admitted_workdir.trim().to_string();
        admitted.validate()?;
        let context = AgentExecutionContext::from_request(&admitted);
        let changed = context.workdir != self.context.workdir;
        if changed {
            admitted.resume_session = None;
            self.provider_session_id = None;
            self.provider_session_context = None;
            self.context.workdir = context.workdir.clone();
            self.turns
                .last_mut()
                .expect("starting turn checked above")
                .context
                .workdir = context.workdir;
            self.push_boundary(AgentHistoryBoundaryReason::ContextChanged, now_ms);
            self.updated_at_ms = now_ms;
        }
        *request = admitted;
        Ok(changed)
    }

    pub fn apply_stream_event(&mut self, event: AgentStreamEvent, now_ms: i64, visible: bool) {
        let mut user_visible = true;
        match event {
            AgentStreamEvent::UserInputAccepted { text } => self.push_message(
                AgentMessageRole::User,
                sanitize_retained_output(&text, MAX_AGENT_MESSAGE_BYTES),
                now_ms,
            ),
            AgentStreamEvent::UserInputAcknowledged { .. }
            | AgentStreamEvent::UserInputRejected { .. }
            | AgentStreamEvent::QuestionAnswerRejected { .. } => user_visible = false,
            AgentStreamEvent::InteractionResolved { id } => {
                if self.question.as_ref().is_some_and(|q| q.id == id) {
                    self.question = None;
                }
                if self.approval.as_ref().is_some_and(|q| q.id == id) {
                    self.approval = None;
                }
                user_visible = false;
            }
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
                if let Some(turn) = self.current_turn_mut() {
                    turn.status = AgentSessionStatus::Running;
                }
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
            AgentStreamEvent::Plan(plan) => self.plan = Some(plan.bounded()),
            AgentStreamEvent::Usage(usage) => {
                self.usage = Some(usage.bounded());
                user_visible = false;
            }
            AgentStreamEvent::Metadata(metadata) => {
                self.metadata = Some(metadata.bounded());
                user_visible = false;
            }
            AgentStreamEvent::Question(question) => {
                self.question = Some(question.bounded());
                self.attention = AgentSessionAttention::NeedsAttention;
            }
            AgentStreamEvent::Approval(approval) => {
                self.approval = Some(approval.bounded());
                self.attention = AgentSessionAttention::NeedsAttention;
            }
            AgentStreamEvent::Source(source) => {
                let source = source.bounded();
                if let Some(existing) = self
                    .sources
                    .iter_mut()
                    .find(|existing| existing.id == source.id)
                {
                    *existing = source;
                } else {
                    push_bounded(&mut self.sources, source, 128);
                }
            }
            AgentStreamEvent::Artifact(artifact) => {
                let artifact = artifact.bounded();
                if let Some(existing) = self
                    .artifacts
                    .iter_mut()
                    .find(|existing| existing.id == artifact.id)
                {
                    *existing = artifact;
                } else {
                    push_bounded(&mut self.artifacts, artifact, 128);
                }
            }
            AgentStreamEvent::Subagent(subagent) => {
                let subagent = subagent.bounded();
                if let Some(existing) = self
                    .subagents
                    .iter_mut()
                    .find(|existing| existing.id == subagent.id)
                {
                    *existing = subagent;
                } else {
                    push_bounded(&mut self.subagents, subagent, 64);
                }
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
        self.question = None;
        self.approval = None;
        if let Some(turn) = self.current_turn_mut() {
            turn.status = status;
            turn.finished_at_ms = Some(now_ms);
        }
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
                turn_id: self.current_turn_id,
                role,
                provider: (role == AgentMessageRole::Agent).then_some(self.context.provider),
                text: truncate_utf8(&text, MAX_AGENT_MESSAGE_BYTES),
                at_ms: now_ms,
            },
            MAX_AGENT_MESSAGES_PER_SESSION,
        );
    }

    fn push_text_delta(&mut self, delta: String, now_ms: i64) {
        if let Some(message) = self.messages.last_mut().filter(|message| {
            message.role == AgentMessageRole::Agent && message.turn_id == self.current_turn_id
        }) {
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
            self.trace.iter_mut().rev().find(|event| {
                event.turn_id == self.current_turn_id && event.correlation_id.as_deref() == Some(id)
            })
        }) {
            existing.detail = detail;
            return false;
        }
        if matches!(&detail, AgentTraceKind::Activity { label } if self.trace.last().is_some_and(|last| last.turn_id == self.current_turn_id && matches!(&last.detail, AgentTraceKind::Activity { label: previous } if previous == label)))
        {
            return false;
        }
        let sequence = self.allocate_sequence();
        push_bounded(
            &mut self.trace,
            AgentTraceEvent {
                id: Uuid::new_v4(),
                sequence,
                turn_id: self.current_turn_id,
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
        let Some(event) = self.trace.iter_mut().rev().find(|event| {
            event.turn_id == self.current_turn_id
                && event.correlation_id.as_deref() == Some(correlation_id.as_str())
        }) else {
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
            .chain(self.boundaries.last().map(|boundary| boundary.sequence))
            .max()
            .unwrap_or(0);
        let sequence = self.next_sequence.max(retained_max.saturating_add(1));
        self.next_sequence = sequence.saturating_add(1);
        sequence
    }

    pub fn current_turn_id(&self) -> Option<Uuid> {
        self.current_turn_id
    }

    fn current_turn_mut(&mut self) -> Option<&mut AgentTurn> {
        let id = self.current_turn_id?;
        self.turns.iter_mut().rev().find(|turn| turn.id == id)
    }

    fn push_boundary(&mut self, reason: AgentHistoryBoundaryReason, now_ms: i64) {
        let sequence = self.allocate_sequence();
        push_bounded(
            &mut self.boundaries,
            AgentHistoryBoundary {
                id: Uuid::new_v4(),
                sequence,
                at_ms: now_ms,
                reason,
                context: self.context.clone(),
            },
            MAX_AGENT_BOUNDARIES_PER_SESSION,
        );
    }

    /// Export reference history without live provider credentials or handles.
    pub fn export_json(&self) -> Result<String> {
        serde_json::to_string_pretty(self)
            .map_err(|error| ShellDeckError::Serialization(error.to_string()))
    }

    pub fn export_markdown(&self) -> String {
        let mut output = format!(
            "# {}\n\nProvider: {:?} · Target: {} · Directory: {}\n\n",
            self.name,
            self.context.provider,
            self.context.target.label(),
            self.context.workdir
        );
        let mut entries = self
            .messages
            .iter()
            .map(|message| (message.sequence, Some(message), None))
            .chain(
                self.boundaries
                    .iter()
                    .map(|boundary| (boundary.sequence, None, Some(boundary))),
            )
            .collect::<Vec<_>>();
        entries.sort_by_key(|entry| entry.0);
        for (_, message, boundary) in entries {
            if let Some(boundary) = boundary {
                output.push_str(&format!("> {}\n\n", boundary.reason.label()));
            }
            if let Some(message) = message {
                let role = match message.role {
                    AgentMessageRole::User => "You",
                    AgentMessageRole::Agent => "Agent",
                    AgentMessageRole::Error => "Error",
                };
                output.push_str(&format!("## {role}\n\n{}\n\n", message.text));
            }
        }
        output
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

    /// Stable library ordering: pinned first, then most recently active.
    pub fn library_sessions(&self, include_archived: bool) -> Vec<&AgentSession> {
        let mut sessions = self
            .sessions
            .iter()
            .filter(|session| include_archived || !session.archived)
            .collect::<Vec<_>>();
        sessions.sort_by_key(|session| (!session.pinned, std::cmp::Reverse(session.updated_at_ms)));
        sessions
    }

    pub fn search(&self, query: &str, include_archived: bool) -> Vec<&AgentSession> {
        let terms = query
            .split_whitespace()
            .map(str::to_lowercase)
            .collect::<Vec<_>>();
        self.library_sessions(include_archived)
            .into_iter()
            .filter(|session| {
                let haystack = format!(
                    "{}\n{}\n{}\n{:?}\n{}",
                    session.name,
                    session.context.workdir,
                    session.context.target.label(),
                    session.context.provider,
                    session
                        .messages
                        .iter()
                        .map(|message| message.text.as_str())
                        .collect::<Vec<_>>()
                        .join("\n")
                )
                .to_lowercase();
                terms.iter().all(|term| haystack.contains(term))
            })
            .collect()
    }

    pub fn rename(&mut self, id: Uuid, name: impl Into<String>, now_ms: i64) -> Result<()> {
        self.get_mut(id)
            .ok_or_else(|| ShellDeckError::NotFound("agent session".into()))?
            .rename(name, now_ms)
    }

    pub fn set_pinned(&mut self, id: Uuid, pinned: bool, now_ms: i64) -> Result<()> {
        let session = self
            .get_mut(id)
            .ok_or_else(|| ShellDeckError::NotFound("agent session".into()))?;
        session.pinned = pinned;
        session.updated_at_ms = now_ms;
        Ok(())
    }

    pub fn archive(&mut self, id: Uuid, archived: bool, now_ms: i64) -> Result<()> {
        let session = self
            .get_mut(id)
            .ok_or_else(|| ShellDeckError::NotFound("agent session".into()))?;
        if archived && session.status.is_active() {
            return Err(ShellDeckError::Config(
                "stop the agent session before archiving it".into(),
            ));
        }
        session.archived = archived;
        session.updated_at_ms = now_ms;
        if archived && self.selected_id == Some(id) {
            self.selected_id = self
                .library_sessions(false)
                .first()
                .map(|session| session.id);
        }
        Ok(())
    }

    /// Copy bounded transcript for reference. A copy never inherits execution
    /// authority, active processes, unread state, or provider resume tokens.
    pub fn duplicate(&mut self, id: Uuid, name: impl Into<String>, now_ms: i64) -> Result<Uuid> {
        if self.sessions.len() >= MAX_AGENT_SESSIONS {
            return Err(ShellDeckError::Config(format!(
                "at most {MAX_AGENT_SESSIONS} agent sessions may be retained"
            )));
        }
        let source = self
            .get(id)
            .ok_or_else(|| ShellDeckError::NotFound("agent session".into()))?;
        let mut copy = AgentSession::new(name, source.context.clone(), now_ms)?;
        let mut turn_ids = std::collections::HashMap::new();
        copy.turns = source
            .turns
            .iter()
            .rev()
            .take(MAX_AGENT_TURNS_PER_SESSION)
            .rev()
            .map(|turn| {
                let mut turn = turn.clone();
                let new_id = Uuid::new_v4();
                turn_ids.insert(turn.id, new_id);
                turn.id = new_id;
                if turn.status.is_active() {
                    turn.status = AgentSessionStatus::Cancelled;
                    turn.finished_at_ms = Some(now_ms);
                }
                turn
            })
            .collect();
        copy.messages = source
            .messages
            .iter()
            .rev()
            .take(MAX_AGENT_MESSAGES_PER_SESSION)
            .rev()
            .map(|message| {
                let mut message = message.clone();
                message.id = Uuid::new_v4();
                message.turn_id = message.turn_id.and_then(|id| turn_ids.get(&id).copied());
                message.text = truncate_utf8(&message.text, MAX_AGENT_MESSAGE_BYTES);
                message
            })
            .collect();
        copy.trace = source
            .trace
            .iter()
            .rev()
            .take(MAX_AGENT_TRACE_EVENTS_PER_SESSION)
            .rev()
            .map(|trace| {
                let mut trace = trace.clone();
                trace.id = Uuid::new_v4();
                trace.turn_id = trace.turn_id.and_then(|id| turn_ids.get(&id).copied());
                trace.detail = bounded_trace(trace.detail);
                trace
            })
            .collect();
        copy.boundaries = source
            .boundaries
            .iter()
            .rev()
            .take(MAX_AGENT_BOUNDARIES_PER_SESSION.saturating_sub(1))
            .rev()
            .map(|boundary| {
                let mut boundary = boundary.clone();
                boundary.id = Uuid::new_v4();
                boundary
            })
            .collect();
        copy.push_boundary(AgentHistoryBoundaryReason::ForkedConversation, now_ms);
        let new_id = copy.id;
        self.sessions.push(copy);
        self.select(new_id)?;
        Ok(new_id)
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

    /// Merge a disk library after asynchronous startup without replacing work
    /// the user has already started in this process. Current identities win.
    pub fn merge_restored(&mut self, restored: AgentSessionCollection) {
        let current = self
            .sessions
            .iter()
            .map(|session| session.id)
            .collect::<std::collections::HashSet<_>>();
        for session in restored.sessions {
            if self.sessions.len() >= MAX_AGENT_SESSIONS {
                break;
            }
            if !current.contains(&session.id) {
                self.sessions.push(session);
            }
        }
        if self.selected_id.is_none() {
            self.selected_id = restored.selected_id.filter(|id| self.get(*id).is_some());
        }
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
            session.provider_session_id = None;
            session.provider_session_context = None;
            session.current_turn_id = None;
            session.question = None;
            session.approval = None;
            for subagent in &mut session.subagents {
                if matches!(
                    subagent.status,
                    AgentTraceStatus::Pending | AgentTraceStatus::Running
                ) {
                    subagent.status = AgentTraceStatus::Cancelled;
                }
            }
            if session.status.is_active() {
                session.status = AgentSessionStatus::Failed;
                session.updated_at_ms = now_ms;
                session.finished_at_ms = Some(now_ms);
                session.attention = AgentSessionAttention::NeedsAttention;
                session.unread_count = session.unread_count.saturating_add(1);
                for turn in &mut session.turns {
                    if turn.status.is_active() {
                        turn.status = AgentSessionStatus::Failed;
                        turn.finished_at_ms = Some(now_ms);
                    }
                }
                session.push_message(AgentMessageRole::Error, "ShellDeck restarted while this run was active. The run is interrupted; check its target before retrying.".into(), now_ms);
            }
            if !session.messages.is_empty()
                && session.boundaries.last().is_none_or(|boundary| {
                    boundary.reason != AgentHistoryBoundaryReason::ApplicationRestart
                        || session
                            .messages
                            .last()
                            .is_some_and(|message| message.sequence > boundary.sequence)
                })
            {
                session.push_boundary(AgentHistoryBoundaryReason::ApplicationRestart, now_ms);
            }
        }
        self.surface_visible = false;
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
    if name.chars().any(char::is_control) {
        return Err(ShellDeckError::Config(
            "agent session name must be a single line".into(),
        ));
    }
    Ok(name.to_string())
}

const fn initial_sequence() -> u64 {
    1
}

fn push_bounded<T>(items: &mut Vec<T>, item: T, limit: usize) {
    if items.len() >= limit {
        items.drain(..=items.len() - limit);
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

    // SDTEST-2473 — SDUC-550, SDUC-551
    #[test]
    fn sdtest_2473_checkout_admission_reconciles_exact_starting_turn_and_drops_resume() {
        let context = AgentExecutionContext {
            target: AgentTarget::Ssh {
                connection_id: Uuid::new_v4(),
                connection_label: "remote checkout".into(),
            },
            workdir: "/home/desktop/repository".into(),
            model: Some("selected-model".into()),
            ..context(AgentProvider::Claude)
        };
        let mut session = AgentSession::new("Mapped checkout", context.clone(), 1).unwrap();
        let first = session.begin_run("first turn", 2).unwrap();
        session.apply_stream_event(
            AgentStreamEvent::Session("old-path-session".into()),
            3,
            true,
        );
        session
            .finish(AgentSessionStatus::Completed, 4, true)
            .unwrap();
        let mut request = session.begin_run("second turn", 5).unwrap();
        assert_eq!(request.resume_session.as_deref(), Some("old-path-session"));
        let before = request.clone();
        assert!(session
            .admit_starting_workdir(&mut request, "/srv/remote/repository", 6)
            .unwrap());

        let admitted = AgentExecutionContext {
            workdir: "/srv/remote/repository".into(),
            ..context
        };
        assert_eq!(session.context, admitted);
        assert_eq!(session.turns.last().unwrap().context, admitted);
        assert_eq!(session.turns.last().unwrap().id, request.id);
        assert_eq!(session.turns[0].id, first.id);
        assert_eq!(session.turns[0].context.workdir, "/home/desktop/repository");
        assert_eq!(
            request,
            AgentRunRequest {
                workdir: admitted.workdir,
                resume_session: None,
                ..before
            }
        );
        assert_eq!(session.provider_session_id(), None);
        assert_eq!(session.run_request("follow-up").resume_session, None);
        assert_eq!(
            session.boundaries.last().unwrap().reason,
            AgentHistoryBoundaryReason::ContextChanged
        );
    }

    // SDTEST-2474 — SDUC-550, SDUC-551
    #[test]
    fn sdtest_2474_checkout_admission_rejects_stale_runs_authority_changes_and_invalid_paths() {
        let mut session = AgentSession::new(
            "Admission fence",
            AgentExecutionContext {
                target: AgentTarget::Ssh {
                    connection_id: Uuid::new_v4(),
                    connection_label: "host".into(),
                },
                workdir: "/original".into(),
                ..context(AgentProvider::Claude)
            },
            1,
        )
        .unwrap();
        let request = session.begin_run("inspect", 2).unwrap();
        let mut candidates = Vec::new();
        let mut changed = request.clone();
        changed.id = Uuid::new_v4();
        candidates.push(changed);
        let mut changed = request.clone();
        changed.access = AgentAccessMode::FullAccess;
        candidates.push(changed);
        let mut changed = request.clone();
        changed.provider = AgentProvider::Codex;
        candidates.push(changed);
        let mut changed = request.clone();
        changed.target = AgentTarget::Local;
        candidates.push(changed);
        let mut changed = request.clone();
        changed.model = Some("different-model".into());
        candidates.push(changed);
        let before = session.clone();
        for mut candidate in candidates {
            let candidate_before = candidate.clone();
            assert!(session
                .admit_starting_workdir(&mut candidate, "/admitted", 3)
                .is_err());
            assert_eq!(session, before);
            assert_eq!(candidate, candidate_before);
        }
        let mut candidate = request.clone();
        assert!(session
            .admit_starting_workdir(&mut candidate, "relative/path", 3)
            .is_err());
        assert_eq!(session, before);
        session.apply_stream_event(AgentStreamEvent::Ready, 4, true);
        let running = session.clone();
        assert!(session
            .admit_starting_workdir(&mut candidate, "/admitted", 5)
            .is_err());
        assert_eq!(session, running);
    }

    // SDTEST-2475 — SDUC-550, SDUC-551
    #[test]
    fn sdtest_2475_unchanged_checkout_admission_keeps_matching_resume_authority() {
        let mut session =
            AgentSession::new("Unchanged checkout", context(AgentProvider::Claude), 1).unwrap();
        session.begin_run("first", 2).unwrap();
        session.apply_stream_event(
            AgentStreamEvent::Session("matching-session".into()),
            3,
            true,
        );
        session
            .finish(AgentSessionStatus::Completed, 4, true)
            .unwrap();
        let mut request = session.begin_run("continue", 5).unwrap();
        let before = session.clone();
        assert!(!session
            .admit_starting_workdir(&mut request, TEST_WORKDIR, 6)
            .unwrap());
        assert_eq!(session, before);
        assert_eq!(request.resume_session.as_deref(), Some("matching-session"));
    }

    // SDTEST-2204 — SDUC-550
    #[test]
    fn sdtest_2204_library_search_archive_and_fork_preserve_history_without_live_authority() {
        let mut sessions = AgentSessionCollection::default();
        let original = sessions
            .create("Parser investigation", context(AgentProvider::Claude), 1)
            .unwrap();
        let request = sessions
            .begin_run(original, "Investigate café rendering", 2)
            .unwrap();
        sessions
            .apply_stream_event(
                original,
                AgentStreamEvent::Session("do-not-copy-this".into()),
                3,
            )
            .unwrap();
        sessions
            .apply_stream_event(
                original,
                AgentStreamEvent::Text("Check fallback shaping".into()),
                4,
            )
            .unwrap();
        assert!(sessions.archive(original, true, 5).is_err());
        let copy = sessions
            .duplicate(original, "Independent investigation", 6)
            .unwrap();
        let fork = sessions.get(copy).unwrap();
        assert_eq!(fork.status, AgentSessionStatus::Idle);
        assert_eq!(fork.provider_session_id(), None);
        assert_eq!(fork.messages[1].text, "Check fallback shaping");
        assert_ne!(
            fork.messages[0].id,
            sessions.get(original).unwrap().messages[0].id
        );
        assert_ne!(fork.messages[0].turn_id, Some(request.id));
        assert!(!fork.export_json().unwrap().contains("do-not-copy-this"));
        assert!(fork.export_markdown().contains("reference history"));
        assert!(fork.turns.iter().all(|turn| !turn.status.is_active()));
        sessions.set_pinned(copy, true, 7).unwrap();
        sessions.rename(copy, "Pinned follow-up", 8).unwrap();
        assert_eq!(sessions.search("café fallback", false).len(), 2);
        assert_eq!(sessions.library_sessions(false)[0].id, copy);
        sessions.archive(copy, true, 9).unwrap();
        assert_eq!(sessions.search("café fallback", false).len(), 1);
        assert_eq!(sessions.search("café fallback", true).len(), 2);
        assert_ne!(sessions.selected_id(), Some(copy));
        sessions.archive(copy, false, 10).unwrap();
        assert_eq!(sessions.library_sessions(false)[0].name, "Pinned follow-up");
    }

    // SDTEST-2205 — SDUC-550
    #[test]
    fn sdtest_2205_context_switches_never_resume_or_replay_history_and_tool_ids_are_turn_scoped() {
        let mut session = AgentSession::new("Scope", context(AgentProvider::Claude), 1).unwrap();
        let first = session.begin_run("Inspect local secret", 2).unwrap();
        session.apply_stream_event(
            AgentStreamEvent::Session("local-only-conversation".into()),
            3,
            true,
        );
        let trace = || {
            AgentStreamEvent::Trace(AgentTraceUpdate {
                correlation_id: Some("reused-tool-id".into()),
                detail: AgentTraceKind::Activity {
                    label: "Inspecting".into(),
                },
            })
        };
        session.apply_stream_event(trace(), 4, true);
        session.apply_stream_event(AgentStreamEvent::TextDelta("Local answer".into()), 5, true);
        session
            .finish(AgentSessionStatus::Completed, 6, true)
            .unwrap();
        let same = session.run_request("continue");
        assert_eq!(
            same.resume_session.as_deref(),
            Some("local-only-conversation")
        );
        let mut remote = context(AgentProvider::Claude);
        remote.target = AgentTarget::Ssh {
            connection_id: Uuid::new_v4(),
            connection_label: "Different host".into(),
        };
        remote.workdir = "/srv/remote".into();
        session.set_context(remote, 7).unwrap();
        let second = session.begin_run("Inspect remote", 8).unwrap();
        assert_eq!(second.resume_session, None);
        assert_eq!(second.prompt, "Inspect remote");
        assert!(!second.prompt.contains("secret"));
        session.apply_stream_event(trace(), 9, true);
        session.apply_stream_event(
            AgentStreamEvent::TextDelta("Remote answer".into()),
            10,
            true,
        );
        assert_eq!(session.messages.len(), 4);
        assert_eq!(session.messages[1].text, "Local answer");
        assert_eq!(session.messages[3].text, "Remote answer");
        assert_eq!(session.trace.len(), 2);
        assert_eq!(session.trace[0].turn_id, Some(first.id));
        assert_eq!(session.trace[1].turn_id, Some(second.id));
        assert_eq!(
            session.boundaries[0].reason,
            AgentHistoryBoundaryReason::ContextChanged
        );
    }

    // SDTEST-1941 — earlier replies retain their author across provider changes
    // and durable reload; older files without author metadata remain readable.
    #[test]
    fn provider_switch_preserves_reply_authorship_and_legacy_history() {
        let mut session =
            AgentSession::new("Providers", context(AgentProvider::Claude), 10).unwrap();
        session.begin_run("first", 11).unwrap();
        session.apply_stream_event(AgentStreamEvent::TextDelta("Claude".into()), 12, true);
        session.apply_stream_event(AgentStreamEvent::TextDelta(" reply".into()), 13, true);
        session
            .finish(AgentSessionStatus::Completed, 14, true)
            .unwrap();
        session
            .set_context(context(AgentProvider::Codex), 15)
            .unwrap();
        session.begin_run("second", 16).unwrap();
        session.apply_stream_event(AgentStreamEvent::Text("Codex reply".into()), 17, true);
        session
            .finish(AgentSessionStatus::Completed, 18, true)
            .unwrap();
        let history = serde_json::to_value(&session).unwrap();
        let restored: AgentSession = serde_json::from_value(history.clone()).unwrap();
        let replies = restored
            .messages
            .iter()
            .filter(|m| m.role == AgentMessageRole::Agent)
            .map(|m| (m.provider, m.text.as_str()))
            .collect::<Vec<_>>();
        assert_eq!(
            replies,
            [
                (Some(AgentProvider::Claude), "Claude reply"),
                (Some(AgentProvider::Codex), "Codex reply")
            ]
        );
        let mut legacy = history;
        for message in legacy["messages"].as_array_mut().unwrap() {
            message.as_object_mut().unwrap().remove("provider");
        }
        let legacy: AgentSession = serde_json::from_value(legacy).unwrap();
        assert_eq!(legacy.messages.len(), 4);
        assert_eq!(legacy.messages[1].text, "Claude reply");
        assert_eq!(legacy.messages[3].text, "Codex reply");
        assert!(legacy.messages.iter().all(|m| m.provider.is_none()));
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
