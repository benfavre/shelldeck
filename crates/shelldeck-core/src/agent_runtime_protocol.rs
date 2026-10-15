//! Bounded presentation contracts and honest capabilities for agent transports.
use super::*;

pub const MAX_AGENT_PLAN_STEPS: usize = 64;
pub const MAX_AGENT_CHOICES: usize = 16;

/// User-selected file data uses the same credential redactor as contextual AI.
pub fn bounded_agent_context_text(text: &str) -> String {
    let text: String = text
        .chars()
        .take(crate::ai::AI_ATTACHMENT_TEXT_MAX_CHARS)
        .collect();
    let value = serde_json::from_str::<Value>(&text).unwrap_or_else(|_| Value::String(text));
    let redacted = crate::ai::redact_sensitive(&value);
    let redacted = redacted
        .as_str()
        .map(str::to_string)
        .unwrap_or_else(|| redacted.to_string());
    safe_provider_text(&redacted, MAX_AGENT_STREAM_TEXT_BYTES)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AgentProviderCapabilities {
    pub incremental_text: bool,
    pub resume: bool,
    pub model_override: bool,
    pub reasoning_effort: bool,
    pub image_attachments: bool,
    pub plans: bool,
    pub usage: bool,
    pub interactive_approvals: bool,
    pub interactive_questions: bool,
    pub live_steering: bool,
}

impl AgentProvider {
    /// Capabilities of ShellDeck's actual transport, not of the vendor's other
    /// products. ACP runtime capabilities must still be negotiated by the host.
    pub const fn capabilities(self) -> AgentProviderCapabilities {
        let cli = AgentProviderCapabilities {
            incremental_text: true,
            resume: true,
            model_override: true,
            reasoning_effort: false,
            image_attachments: false,
            plans: false,
            usage: false,
            interactive_approvals: false,
            interactive_questions: false,
            live_steering: false,
        };
        match self {
            Self::Claude => AgentProviderCapabilities {
                plans: true,
                usage: true,
                ..cli
            },
            Self::Codex => AgentProviderCapabilities {
                plans: true,
                usage: true,
                reasoning_effort: true,
                image_attachments: true,
                ..cli
            },
            Self::Automonique => AgentProviderCapabilities {
                model_override: false,
                plans: true,
                usage: true,
                interactive_approvals: true,
                ..cli
            },
            Self::Jcode | Self::DeepSeek => cli,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentPlanStepStatus {
    Pending,
    InProgress,
    Completed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentPlanStep {
    pub text: String,
    pub status: AgentPlanStepStatus,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentPlanUpdate {
    pub steps: Vec<AgentPlanStep>,
}
impl AgentPlanUpdate {
    pub fn bounded(mut self) -> Self {
        self.steps.truncate(MAX_AGENT_PLAN_STEPS);
        for step in &mut self.steps {
            step.text = safe_provider_text(&step.text, MAX_AGENT_TRACE_FIELD_BYTES);
        }
        self
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentUsageUpdate {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub cached_input_tokens: Option<u64>,
    pub cache_write_input_tokens: Option<u64>,
    pub reasoning_output_tokens: Option<u64>,
    pub context_used: Option<u64>,
    pub context_size: Option<u64>,
    /// Decimal string avoids floating-point equality/serialization surprises.
    pub cost_amount: Option<String>,
    pub cost_currency: Option<String>,
    /// ACP and Claude costs are cumulative session estimates, not turn charges.
    pub cumulative: bool,
}
impl AgentUsageUpdate {
    pub fn bounded(mut self) -> Self {
        self.cost_amount = self.cost_amount.map(|s| safe_provider_text(&s, 64));
        self.cost_currency = self.cost_currency.map(|s| safe_provider_text(&s, 16));
        self
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentRunMetadata {
    pub model: Option<String>,
    pub provider_version: Option<String>,
    pub available_tools: Vec<String>,
    pub available_skills: Vec<String>,
}
impl AgentRunMetadata {
    pub fn bounded(mut self) -> Self {
        self.model = self
            .model
            .map(|s| safe_provider_text(&s, MAX_AGENT_MODEL_BYTES));
        self.provider_version = self
            .provider_version
            .map(|s| safe_provider_text(&s, MAX_AGENT_MODEL_BYTES));
        self.available_tools.truncate(128);
        self.available_skills.truncate(128);
        for s in self
            .available_tools
            .iter_mut()
            .chain(self.available_skills.iter_mut())
        {
            *s = safe_provider_text(s, MAX_AGENT_MODEL_BYTES);
        }
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentQuestionOption {
    pub label: String,
    pub description: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentQuestionRequest {
    pub id: String,
    pub prompt: String,
    pub options: Vec<AgentQuestionOption>,
    pub multiple: bool,
    /// False for observed one-shot CLI questions: answers require a follow-up.
    pub response_supported: bool,
}
impl AgentQuestionRequest {
    pub fn bounded(mut self) -> Self {
        self.id = safe_provider_text(&self.id, MAX_AGENT_SESSION_BYTES);
        self.prompt = safe_provider_text(&self.prompt, MAX_AGENT_TRACE_FIELD_BYTES);
        self.options.truncate(MAX_AGENT_CHOICES);
        for o in &mut self.options {
            o.label = safe_provider_text(&o.label, MAX_AGENT_ACTIVITY_BYTES);
            o.description = safe_provider_text(&o.description, MAX_AGENT_ACTIVITY_BYTES);
        }
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentApprovalOptionKind {
    AllowOnce,
    AllowAlways,
    RejectOnce,
    RejectAlways,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentApprovalOption {
    pub id: String,
    pub label: String,
    pub kind: AgentApprovalOptionKind,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentApprovalRequest {
    pub id: String,
    pub session_id: String,
    pub tool_call_id: String,
    pub title: String,
    pub options: Vec<AgentApprovalOption>,
}
impl AgentApprovalRequest {
    pub fn bounded(mut self) -> Self {
        // IDs are routing authority; never truncate/redact them into another ID.
        self.title = safe_provider_text(&self.title, MAX_AGENT_TRACE_FIELD_BYTES);
        self.options.truncate(MAX_AGENT_CHOICES);
        for option in &mut self.options {
            option.label = safe_provider_text(&option.label, MAX_AGENT_ACTIVITY_BYTES);
        }
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentSource {
    pub id: String,
    pub title: String,
    pub uri: String,
}
impl AgentSource {
    pub fn bounded(mut self) -> Self {
        self.id = safe_provider_text(&self.id, MAX_AGENT_SESSION_BYTES);
        self.title = safe_provider_text(&self.title, MAX_AGENT_ACTIVITY_BYTES);
        self.uri = safe_provider_text(&self.uri, MAX_AGENT_WORKDIR_BYTES);
        self
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentArtifact {
    pub id: String,
    pub title: String,
    pub uri: String,
    pub mime_type: Option<String>,
}
impl AgentArtifact {
    pub fn bounded(mut self) -> Self {
        self.id = safe_provider_text(&self.id, MAX_AGENT_SESSION_BYTES);
        self.title = safe_provider_text(&self.title, MAX_AGENT_ACTIVITY_BYTES);
        self.uri = safe_provider_text(&self.uri, MAX_AGENT_WORKDIR_BYTES);
        self.mime_type = self.mime_type.map(|s| safe_provider_text(&s, 128));
        self
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentSubagentUpdate {
    pub id: String,
    pub parent_id: Option<String>,
    pub title: String,
    pub status: AgentTraceStatus,
    pub summary: Option<String>,
}
impl AgentSubagentUpdate {
    pub fn bounded(mut self) -> Self {
        self.id = safe_provider_text(&self.id, MAX_AGENT_SESSION_BYTES);
        self.parent_id = self
            .parent_id
            .map(|s| safe_provider_text(&s, MAX_AGENT_SESSION_BYTES));
        self.title = safe_provider_text(&self.title, MAX_AGENT_ACTIVITY_BYTES);
        self.summary = self
            .summary
            .map(|s| safe_provider_text(&s, MAX_AGENT_TRACE_FIELD_BYTES));
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentReasoningEffort {
    Minimal,
    Low,
    Medium,
    High,
    Xhigh,
    Max,
    Ultra,
}
impl AgentReasoningEffort {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Minimal => "minimal",
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Xhigh => "xhigh",
            Self::Max => "max",
            Self::Ultra => "ultra",
        }
    }
}

/// Optional transport inputs avoid changing the durable execution-context wire
/// format. Image paths refer to the selected target, never to the local host
/// implicitly when the run targets SSH. Models may reject unsupported effort.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AgentInputOptions {
    pub reasoning_effort: Option<AgentReasoningEffort>,
    pub image_paths: Vec<PathBuf>,
}

impl AgentCommandSpec {
    pub fn for_request_with_options(
        request: &AgentRunRequest,
        options: &AgentInputOptions,
    ) -> Result<Self> {
        let mut spec = Self::for_request(request)?;
        if options.reasoning_effort.is_some() && !request.provider.capabilities().reasoning_effort {
            return Err(ShellDeckError::Config(
                "Selected agent transport does not support reasoning effort".into(),
            ));
        }
        if options.image_paths.len() > 8
            || (!options.image_paths.is_empty()
                && !request.provider.capabilities().image_attachments)
        {
            return Err(ShellDeckError::Config(
                "Selected agent transport does not support these image attachments".into(),
            ));
        }
        let mut extra = Vec::new();
        if let Some(effort) = options.reasoning_effort {
            extra.extend([
                "-c".to_string(),
                format!("model_reasoning_effort=\"{}\"", effort.as_str()),
            ]);
        }
        for path in &options.image_paths {
            let path = path
                .to_str()
                .filter(|s| !s.contains(['\0', '\n', '\r']) && s.len() <= MAX_AGENT_WORKDIR_BYTES)
                .ok_or_else(|| ShellDeckError::Config("Invalid image attachment path".into()))?;
            let absolute = match request.target {
                AgentTarget::Local => Path::new(path).is_absolute(),
                AgentTarget::Ssh { .. } => path.starts_with('/'),
            };
            if !absolute {
                return Err(ShellDeckError::Config(
                    "Image attachment must have an absolute path on the selected target".into(),
                ));
            }
            extra.extend(["--image".to_string(), path.to_string()]);
        }
        // Options precede the final positional prompt/session arguments.
        let offset = if request.provider == AgentProvider::Codex && request.resume_session.is_some()
        {
            2
        } else {
            1
        };
        spec.args.splice(offset..offset, extra);
        Ok(spec)
    }
}

/// Interactive ACP broker for a host that actually renders permission choices.
/// Replies use the original offered identifier; no textual "approve" command
/// and no provider-wide grants are synthesized. Read-only always cancels.
pub struct AcpInteractivePermissionBroker {
    access: AgentAccessMode,
    events: std::sync::mpsc::Sender<AgentStreamEvent>,
    timeout: std::time::Duration,
    pending: parking_lot::Mutex<std::collections::HashMap<String, std::sync::Arc<PendingApproval>>>,
}

struct PendingApproval {
    offered: Vec<PermissionOptionId>,
    response: parking_lot::Mutex<Option<Option<PermissionOptionId>>>,
    changed: parking_lot::Condvar,
}

impl AcpInteractivePermissionBroker {
    pub fn new(
        access: AgentAccessMode,
        events: std::sync::mpsc::Sender<AgentStreamEvent>,
        timeout: std::time::Duration,
    ) -> Self {
        Self {
            access,
            events,
            timeout: timeout.min(std::time::Duration::from_secs(180)),
            pending: parking_lot::Mutex::new(std::collections::HashMap::new()),
        }
    }

    pub fn respond(&self, request_id: &str, option_id: Option<&str>) -> Result<()> {
        let pending = self
            .pending
            .lock()
            .get(request_id)
            .cloned()
            .ok_or_else(|| {
                ShellDeckError::Config("Permission request expired or was cancelled".into())
            })?;
        let selected = match option_id {
            None => None,
            Some(id) => Some(
                pending
                    .offered
                    .iter()
                    .find(|offered| offered.0.as_ref() == id)
                    .cloned()
                    .ok_or_else(|| {
                        ShellDeckError::Config(
                            "Permission choice was not offered by the agent".into(),
                        )
                    })?,
            ),
        };
        let mut response = pending.response.lock();
        if response.is_some() {
            return Err(ShellDeckError::Config(
                "Permission request already answered".into(),
            ));
        }
        *response = Some(selected);
        pending.changed.notify_all();
        Ok(())
    }

    /// Call before stopping a run so the synchronous protocol callback wakes.
    pub fn cancel_pending(&self) {
        for pending in self.pending.lock().values() {
            *pending.response.lock() = Some(None);
            pending.changed.notify_all();
        }
    }
}

impl crate::acp::AcpPermissionBroker for AcpInteractivePermissionBroker {
    fn select(&self, request: &RequestPermissionRequest) -> Option<PermissionOptionId> {
        if self.access == AgentAccessMode::ReadOnly {
            return None;
        }
        let valid_id =
            |id: &str| !id.is_empty() && id.len() <= MAX_AGENT_SESSION_BYTES && !id.contains('\0');
        if !valid_id(&request.session_id.0) || !valid_id(&request.tool_call.tool_call_id.0) {
            return None;
        }
        let id = Uuid::new_v4().to_string();
        let options: Vec<_> = request
            .options
            .iter()
            .take(MAX_AGENT_CHOICES)
            .filter(|o| valid_id(&o.option_id.0))
            .filter_map(|option| {
                let kind = match option.kind {
                    PermissionOptionKind::AllowOnce => AgentApprovalOptionKind::AllowOnce,
                    PermissionOptionKind::AllowAlways => return None,
                    PermissionOptionKind::RejectOnce => AgentApprovalOptionKind::RejectOnce,
                    PermissionOptionKind::RejectAlways => return None,
                    _ => return None,
                };
                Some(AgentApprovalOption {
                    id: option.option_id.0.to_string(),
                    label: option.name.clone(),
                    kind,
                })
            })
            .collect();
        let pending = std::sync::Arc::new(PendingApproval {
            offered: options
                .iter()
                .map(|o| PermissionOptionId::from(o.id.clone()))
                .collect(),
            response: parking_lot::Mutex::new(None),
            changed: parking_lot::Condvar::new(),
        });
        if pending.offered.is_empty() {
            return None;
        }
        self.pending.lock().insert(id.clone(), pending.clone());
        let event = AgentApprovalRequest {
            id: id.clone(),
            session_id: request.session_id.0.to_string(),
            tool_call_id: request.tool_call.tool_call_id.0.to_string(),
            title: request
                .tool_call
                .fields
                .title
                .clone()
                .unwrap_or_else(|| "Agent tool permission".into()),
            options,
        }
        .bounded();
        if self.events.send(AgentStreamEvent::Approval(event)).is_err() {
            self.pending.lock().remove(&id);
            return None;
        }
        let mut response = pending.response.lock();
        let deadline = std::time::Instant::now() + self.timeout;
        while response.is_none() {
            if pending
                .changed
                .wait_until(&mut response, deadline)
                .timed_out()
            {
                break;
            }
        }
        let selected = response.take().flatten();
        drop(response);
        self.pending.lock().remove(&id);
        selected
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::acp::AcpPermissionBroker as _;

    // SDTEST-2233 — SDUC-551
    #[test]
    fn sdtest_2233_acp_approval_requires_an_exact_offered_choice_and_cancels_readonly() {
        use agent_client_protocol::schema::v1::{
            PermissionOption, ToolCallUpdate, ToolCallUpdateFields,
        };
        let (sender, receiver) = std::sync::mpsc::channel();
        let broker = std::sync::Arc::new(AcpInteractivePermissionBroker::new(
            AgentAccessMode::WorkspaceWrite,
            sender.clone(),
            std::time::Duration::from_secs(2),
        ));
        let request = RequestPermissionRequest::new(
            "session",
            ToolCallUpdate::new("tool", ToolCallUpdateFields::new().title("Edit file")),
            vec![PermissionOption::new(
                "exact/choice",
                "Allow once",
                PermissionOptionKind::AllowOnce,
            )],
        );
        let worker = {
            let broker = broker.clone();
            let request = request.clone();
            std::thread::spawn(move || broker.select(&request))
        };
        let AgentStreamEvent::Approval(approval) = receiver
            .recv_timeout(std::time::Duration::from_secs(1))
            .unwrap()
        else {
            panic!("expected permission request");
        };
        assert!(broker.respond(&approval.id, Some("approve")).is_err());
        broker.respond(&approval.id, Some("exact/choice")).unwrap();
        assert_eq!(worker.join().unwrap().unwrap().0.as_ref(), "exact/choice");
        assert!(AcpInteractivePermissionBroker::new(
            AgentAccessMode::ReadOnly,
            sender,
            std::time::Duration::ZERO
        )
        .select(&request)
        .is_none());
        assert!(receiver.try_recv().is_err());
    }

    // SDTEST-2234 — SDUC-551
    #[test]
    fn sdtest_2234_input_options_keep_target_paths_and_resume_flags_separate() {
        let mut request = AgentRunRequest::new(
            AgentProvider::Codex,
            AgentTarget::Ssh {
                connection_id: Uuid::new_v4(),
                connection_label: "remote".into(),
            },
            AgentAccessMode::ReadOnly,
            "/work",
            None,
            "inspect",
        )
        .with_resume_session(Some("session".into()));
        let options = AgentInputOptions {
            reasoning_effort: Some(AgentReasoningEffort::High),
            image_paths: vec![PathBuf::from("/work/screen shot.png")],
        };
        let spec = AgentCommandSpec::for_request_with_options(&request, &options).unwrap();
        assert_eq!(&spec.args[..2], &["exec", "resume"]);
        assert!(spec
            .args
            .windows(2)
            .any(|pair| pair == ["--image", "/work/screen shot.png"]));
        assert!(spec
            .remote_shell_command()
            .contains("'/work/screen shot.png'"));
        request.provider = AgentProvider::Claude;
        assert!(AgentCommandSpec::for_request_with_options(&request, &options).is_err());
    }
}
