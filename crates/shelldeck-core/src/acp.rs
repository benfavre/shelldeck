//! Stable ACP v1 client host for ShellDeck.
//!
//! ShellDeck owns the client side of the Agent Client Protocol. It starts an
//! explicitly configured agent process, negotiates capabilities, creates or
//! reloads a session, forwards streaming updates, and routes every permission
//! request through an injected user-decision broker. It deliberately
//! advertises no client-side filesystem or terminal capability: those effects
//! remain behind ShellDeck's typed confirmation paths and Automonique's
//! canonical authority.

use std::path::PathBuf;
use std::sync::Arc;

use agent_client_protocol::schema::v1::{
    AgentCapabilities, AuthMethodId, AuthenticateRequest, CancelNotification, ClientCapabilities,
    ContentBlock, Implementation, InitializeRequest, LoadSessionRequest, McpServer,
    NewSessionRequest, PermissionOptionId, PromptRequest, PromptResponse, RequestPermissionOutcome,
    RequestPermissionRequest, RequestPermissionResponse, SelectedPermissionOutcome, SessionId,
    SessionNotification,
};
use agent_client_protocol::schema::ProtocolVersion;
use agent_client_protocol::{AcpAgent, AcpAgentConfig, Agent, ConnectionTo};
use parking_lot::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::error::{Result, ShellDeckError};

/// Stable SDK release used by both ShellDeck and Automonique.
pub const ACP_SDK_VERSION: &str = "2.0.0";

pub type AcpCancelSender = futures::channel::oneshot::Sender<()>;
pub type AcpCancelReceiver = futures::channel::oneshot::Receiver<()>;

pub fn cancel_channel() -> (AcpCancelSender, AcpCancelReceiver) {
    futures::channel::oneshot::channel()
}

/// Explicit child-process launch configuration. No shell is involved.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AcpLaunch {
    pub program: PathBuf,
    pub arguments: Vec<String>,
}

impl AcpLaunch {
    #[must_use]
    pub fn automonique(program: impl Into<PathBuf>) -> Self {
        Self {
            program: program.into(),
            arguments: vec!["acp".to_string()],
        }
    }

    #[must_use]
    pub fn new(program: impl Into<PathBuf>, arguments: Vec<String>) -> Self {
        Self {
            program: program.into(),
            arguments,
        }
    }
}

/// One new or resumed ACP turn.
#[derive(Clone, Debug, PartialEq)]
pub struct AcpTurnRequest {
    pub cwd: PathBuf,
    pub session_id: Option<SessionId>,
    pub prompt: Vec<ContentBlock>,
    pub additional_directories: Vec<PathBuf>,
    pub mcp_servers: Vec<McpServer>,
    pub authentication_method: Option<AuthMethodId>,
}

impl AcpTurnRequest {
    #[must_use]
    pub fn new(cwd: impl Into<PathBuf>, prompt: Vec<ContentBlock>) -> Self {
        Self {
            cwd: cwd.into(),
            session_id: None,
            prompt,
            additional_directories: Vec::new(),
            mcp_servers: Vec::new(),
            authentication_method: None,
        }
    }

    #[must_use]
    pub fn text(cwd: impl Into<PathBuf>, prompt: impl Into<String>) -> Self {
        Self::new(
            cwd,
            vec![ContentBlock::Text(
                agent_client_protocol::schema::v1::TextContent::new(prompt),
            )],
        )
    }

    #[must_use]
    pub fn resume(mut self, session_id: impl Into<SessionId>) -> Self {
        self.session_id = Some(session_id.into());
        self
    }

    #[must_use]
    pub fn authenticate_with(mut self, method: impl Into<AuthMethodId>) -> Self {
        self.authentication_method = Some(method.into());
        self
    }
}

/// Result of a complete prompt turn, including every ordered update received.
#[derive(Clone, Debug, PartialEq)]
pub struct AcpTurnResult {
    pub agent_info: Option<Implementation>,
    pub agent_capabilities: AgentCapabilities,
    pub session_id: SessionId,
    pub updates: Vec<SessionNotification>,
    pub response: PromptResponse,
}

/// Synchronous bridge to ShellDeck's explicit permission UI/policy.
pub trait AcpPermissionBroker: Send + Sync + 'static {
    /// Return one option offered by the agent, or `None` to cancel the request.
    fn select(&self, request: &RequestPermissionRequest) -> Option<PermissionOptionId>;
}

/// Safe default used when no interactive permission surface is connected.
#[derive(Default)]
pub struct CancelPermissions;

impl AcpPermissionBroker for CancelPermissions {
    fn select(&self, _request: &RequestPermissionRequest) -> Option<PermissionOptionId> {
        None
    }
}

/// Stable ACP client backed by the official Rust SDK.
pub struct AcpClient {
    launch: AcpLaunch,
    permissions: Arc<dyn AcpPermissionBroker>,
    updates: Arc<dyn Fn(&SessionNotification) + Send + Sync>,
}

impl AcpClient {
    #[must_use]
    pub fn new(launch: AcpLaunch) -> Self {
        Self {
            launch,
            permissions: Arc::new(CancelPermissions),
            updates: Arc::new(|_| {}),
        }
    }

    #[must_use]
    pub fn with_permissions(mut self, permissions: Arc<dyn AcpPermissionBroker>) -> Self {
        self.permissions = permissions;
        self
    }

    /// Observe each ordered ACP update as it arrives. The callback must remain
    /// lightweight; provider I/O continues on the ACP connection task.
    #[must_use]
    pub fn with_updates(
        mut self,
        updates: Arc<dyn Fn(&SessionNotification) + Send + Sync>,
    ) -> Self {
        self.updates = updates;
        self
    }

    /// Route ACP updates directly into the provider-neutral coding-agent
    /// timeline while preserving protocol order.
    #[must_use]
    pub fn with_agent_events(
        self,
        events: std::sync::mpsc::Sender<crate::agent_runtime::AgentStreamEvent>,
    ) -> Self {
        let ready = Arc::new(AtomicBool::new(false));
        self.with_updates(Arc::new(move |notification| {
            if !ready.swap(true, Ordering::AcqRel) {
                let _ = events.send(crate::agent_runtime::AgentStreamEvent::Ready);
            }
            for event in crate::agent_runtime::acp_notification_events(notification) {
                let _ = events.send(event);
            }
        }))
    }

    /// Run one complete turn. The process group is torn down by the SDK when
    /// the connection ends, including wrapper subprocesses on Unix.
    pub fn prompt(&self, request: AcpTurnRequest) -> Result<AcpTurnResult> {
        self.prompt_inner(request, None)?
            .ok_or_else(|| ShellDeckError::Connection("ACP turn was unexpectedly cancelled".into()))
    }

    /// Run one turn with protocol-level cancellation. Dropping the connection
    /// after `session/cancel` also activates the SDK's child process-group
    /// guard, so Stop cannot leave an Automonique wrapper or tool orphaned.
    pub fn prompt_cancellable(
        &self,
        request: AcpTurnRequest,
        cancel: AcpCancelReceiver,
    ) -> Result<Option<AcpTurnResult>> {
        self.prompt_inner(request, Some(cancel))
    }

    fn prompt_inner(
        &self,
        request: AcpTurnRequest,
        mut cancel: Option<AcpCancelReceiver>,
    ) -> Result<Option<AcpTurnResult>> {
        validate_request(&request)?;

        let updates = Arc::new(Mutex::new(Vec::new()));
        let result = Arc::new(Mutex::new(None));
        let updates_handler = Arc::clone(&updates);
        let live_updates = Arc::clone(&self.updates);
        let result_handler = Arc::clone(&result);
        let permissions = Arc::clone(&self.permissions);
        let cancelled = Arc::new(AtomicBool::new(false));
        let cancelled_handler = Arc::clone(&cancelled);
        let agent = AcpAgent::new(
            AcpAgentConfig::new(&self.launch.program).args(self.launch.arguments.clone()),
        );

        futures::executor::block_on(
            agent_client_protocol::Client
                .builder()
                .name("shelldeck")
                .on_receive_notification(
                    async move |notification: SessionNotification, _connection| {
                        live_updates(&notification);
                        updates_handler.lock().push(notification);
                        Ok(())
                    },
                    agent_client_protocol::on_receive_notification!(),
                )
                .on_receive_request(
                    async move |request: RequestPermissionRequest, responder, _connection| {
                        let selected = permissions.select(&request).filter(|selected| {
                            request
                                .options
                                .iter()
                                .any(|option| option.option_id.0.as_ref() == selected.0.as_ref())
                        });
                        let outcome = selected.map_or(RequestPermissionOutcome::Cancelled, |id| {
                            RequestPermissionOutcome::Selected(SelectedPermissionOutcome::new(id))
                        });
                        responder.respond(RequestPermissionResponse::new(outcome))
                    },
                    agent_client_protocol::on_receive_request!(),
                )
                .connect_with(agent, move |connection: ConnectionTo<Agent>| async move {
                    let initialized = connection
                        .send_request(
                            InitializeRequest::new(ProtocolVersion::V1)
                                .client_info(Implementation::new("shelldeck", crate::VERSION))
                                .client_capabilities(ClientCapabilities::new()),
                        )
                        .block_task()
                        .await?;
                    if initialized.protocol_version != ProtocolVersion::V1 {
                        return Err(agent_client_protocol::Error::invalid_request()
                            .data("unsupported_protocol_version"));
                    }
                    if !initialized.auth_methods.is_empty() {
                        let method = request.authentication_method.ok_or_else(|| {
                            agent_client_protocol::Error::invalid_request()
                                .data("agent_authentication_required")
                        })?;
                        if !initialized
                            .auth_methods
                            .iter()
                            .any(|offered| offered.id().0.as_ref() == method.0.as_ref())
                        {
                            return Err(agent_client_protocol::Error::invalid_params()
                                .data("authentication_method_not_offered"));
                        }
                        connection
                            .send_request(AuthenticateRequest::new(method))
                            .block_task()
                            .await?;
                    }

                    let session_id = if let Some(session_id) = request.session_id {
                        if !initialized.agent_capabilities.load_session {
                            return Err(agent_client_protocol::Error::invalid_request()
                                .data("agent_does_not_support_session_load"));
                        }
                        connection
                            .send_request(
                                LoadSessionRequest::new(session_id.clone(), request.cwd)
                                    .additional_directories(request.additional_directories)
                                    .mcp_servers(request.mcp_servers),
                            )
                            .block_task()
                            .await?;
                        session_id
                    } else {
                        connection
                            .send_request(
                                NewSessionRequest::new(request.cwd)
                                    .additional_directories(request.additional_directories)
                                    .mcp_servers(request.mcp_servers),
                            )
                            .block_task()
                            .await?
                            .session_id
                    };
                    let prompt = connection
                        .send_request(PromptRequest::new(session_id.clone(), request.prompt))
                        .block_task();
                    let response = if let Some(cancel) = cancel.as_mut() {
                        use futures::future::{select, Either};
                        use futures::FutureExt as _;

                        let prompt = prompt.fuse();
                        let stop = cancel.fuse();
                        futures::pin_mut!(prompt, stop);
                        match select(prompt, stop).await {
                            Either::Left((response, _)) => response?,
                            Either::Right((_, _)) => {
                                cancelled_handler.store(true, Ordering::Release);
                                connection
                                    .send_notification(CancelNotification::new(session_id))?;
                                return Ok(());
                            }
                        }
                    } else {
                        prompt.await?
                    };
                    *result_handler.lock() = Some((initialized, session_id, response));
                    Ok(())
                }),
        )
        .map_err(acp_error)?;

        let completed = result.lock().take();
        if completed.is_none() && cancelled.load(Ordering::Acquire) {
            return Ok(None);
        }
        let (initialized, session_id, response) = completed
            .ok_or_else(|| ShellDeckError::Connection("ACP turn ended without a result".into()))?;
        let updates = std::mem::take(&mut *updates.lock());
        Ok(Some(AcpTurnResult {
            agent_info: initialized.agent_info,
            agent_capabilities: initialized.agent_capabilities,
            session_id,
            updates,
            response,
        }))
    }
}

fn validate_request(request: &AcpTurnRequest) -> Result<()> {
    if !request.cwd.is_absolute() {
        return Err(ShellDeckError::Config(
            "ACP working directory must be absolute".into(),
        ));
    }
    if request
        .additional_directories
        .iter()
        .any(|path| !path.is_absolute())
    {
        return Err(ShellDeckError::Config(
            "ACP additional directories must be absolute".into(),
        ));
    }
    if request.prompt.is_empty() {
        return Err(ShellDeckError::Config("ACP prompt cannot be empty".into()));
    }
    Ok(())
}

fn acp_error(error: agent_client_protocol::Error) -> ShellDeckError {
    ShellDeckError::Connection(format!("ACP protocol error: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_client_protocol::schema::v1::{
        PermissionOption, PermissionOptionKind, TextContent, ToolCallUpdate, ToolCallUpdateFields,
    };

    struct UnknownChoice;

    impl AcpPermissionBroker for UnknownChoice {
        fn select(&self, _request: &RequestPermissionRequest) -> Option<PermissionOptionId> {
            Some(PermissionOptionId::new("not-offered"))
        }
    }

    // SDTEST-1693 / SDUC-480 — unadvertised permission choices never cross
    // the ACP trust boundary. The end-to-end transport is covered in the
    // Automonique ACP crate so this test pins ShellDeck's local broker rule.
    #[test]
    fn permission_broker_output_must_match_an_offered_option() {
        let request = RequestPermissionRequest::new(
            "session-1",
            ToolCallUpdate::new("tool-1", ToolCallUpdateFields::new()),
            vec![PermissionOption::new(
                "allow-once",
                "Allow once",
                PermissionOptionKind::AllowOnce,
            )],
        );
        let selected = UnknownChoice.select(&request).filter(|selected| {
            request
                .options
                .iter()
                .any(|option| option.option_id.0.as_ref() == selected.0.as_ref())
        });
        assert!(selected.is_none());
    }

    // SDTEST-1694 / SDUC-480
    #[test]
    fn request_validation_rejects_relative_workspace_and_empty_prompt() {
        let empty = AcpTurnRequest::new("/workspace", Vec::new());
        assert!(validate_request(&empty).is_err());
        let relative = AcpTurnRequest::new(
            "workspace",
            vec![ContentBlock::Text(TextContent::new("hello"))],
        );
        assert!(validate_request(&relative).is_err());
    }
}
