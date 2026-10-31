use super::*;
use crate::agent_console_view::{AgentProjectGroup, AgentRemoteFileEntry, AgentWorktreeOption};
use shelldeck_core::agent_runtime::{
    configure_local_process, terminate_local_process, AcpInteractivePermissionBroker,
    AgentCommandSpec, AgentProvider, AgentRunRequest, AgentStreamEvent, AgentStreamFrame,
    AgentStreamFramer, AgentStreamParser, AgentTarget, LocalProcessTree,
    AGENT_STREAM_OVERSIZED_LABEL,
};
use shelldeck_core::agent_session::DEFAULT_MAX_CONCURRENT_AGENT_SESSIONS;
use shelldeck_core::config::workspace_catalog::{
    CatalogCheckoutId, CatalogWorkspaceId, CheckoutHost, ProjectCatalog, RemotePosixPath,
    UserWorkspaceLifecycle, WorkspaceRelativePath,
};
use shelldeck_core::workspace_navigation::AgentSessionBinding;
use shelldeck_ssh::client::SshClient;
use std::io::{Read, Write};
use std::process::{Command, Stdio};

pub(super) struct ActiveAgentRun {
    shutdown: AgentShutdown,
    _thread: std::thread::JoinHandle<()>,
    pub(super) approval_broker: Option<std::sync::Arc<AcpInteractivePermissionBroker>>,
}

enum AgentShutdown {
    Codex(
        parking_lot::Mutex<
            Option<std::sync::mpsc::SyncSender<shelldeck_core::agent_codex::LiveCodexCommand>>,
        >,
        std::sync::Arc<std::sync::atomic::AtomicBool>,
    ),
    Process(tokio::sync::mpsc::Sender<()>),
    Acp(parking_lot::Mutex<Option<shelldeck_core::acp::AcpCancelSender>>),
}

impl ActiveAgentRun {
    pub(super) fn live_codex(
        commands: std::sync::mpsc::SyncSender<shelldeck_core::agent_codex::LiveCodexCommand>,
        cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
        thread: std::thread::JoinHandle<()>,
    ) -> Self {
        Self {
            shutdown: AgentShutdown::Codex(parking_lot::Mutex::new(Some(commands)), cancel),
            _thread: thread,
            approval_broker: None,
        }
    }
    pub(super) fn send_live(
        &self,
        command: shelldeck_core::agent_codex::LiveCodexCommand,
    ) -> Result<(), String> {
        if let AgentShutdown::Codex(sender, _) = &self.shutdown {
            sender
                .lock()
                .as_ref()
                .ok_or("Agent run is stopping")?
                .try_send(command)
                .map_err(|_| "Agent input queue is busy or closed".to_string())
        } else {
            Err("This agent transport does not accept live input".to_string())
        }
    }

    fn stop(&self) {
        if let Some(broker) = &self.approval_broker {
            broker.cancel_pending();
        }
        match &self.shutdown {
            AgentShutdown::Codex(sender, cancel) => {
                cancel.store(true, std::sync::atomic::Ordering::Release);
                if let Some(sender) = sender.lock().take() {
                    let _ =
                        sender.try_send(shelldeck_core::agent_codex::LiveCodexCommand::Interrupt);
                }
            }
            AgentShutdown::Process(sender) => {
                let _ = sender.try_send(());
            }
            AgentShutdown::Acp(sender) => {
                if let Some(sender) = sender.lock().take() {
                    let _ = sender.send(());
                }
            }
        }
    }
}

type AgentDone = (Option<i32>, Option<String>);

/// Keep parallel coding work useful without allowing an accidental prompt
/// fan-out to exhaust the desktop host. Every admitted run still owns an
/// independently stoppable process tree or SSH channel.
fn agent_run_has_capacity(active: usize) -> bool {
    active < DEFAULT_MAX_CONCURRENT_AGENT_SESSIONS
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AutomoniqueAdmissionError {
    SshTarget,
    ModelOverride,
}

fn automonique_agent_admission(request: &AgentRunRequest) -> Result<(), AutomoniqueAdmissionError> {
    if !matches!(request.target, AgentTarget::Local) {
        return Err(AutomoniqueAdmissionError::SshTarget);
    }
    if request
        .model
        .as_ref()
        .is_some_and(|model| !model.trim().is_empty())
    {
        return Err(AutomoniqueAdmissionError::ModelOverride);
    }
    Ok(())
}

impl Workspace {
    pub(super) fn handle_agent_console_event(
        &mut self,
        event: AgentConsoleEvent,
        cx: &mut Context<Self>,
    ) {
        if let AgentConsoleEvent::Stop(run_id) = &event {
            self.stop_agent_run(*run_id, cx);
            return;
        }
        if !self.can_access_mode(AppMode::Dev) {
            self.agent_console.update(cx, |view, cx| {
                view.set_workflows_authorized(false, cx);
                if let AgentConsoleEvent::SaveFile(request) = &event {
                    view.complete_agent_file_save(
                        request,
                        Err(t!("agents.live.authorization_lost").to_string()),
                        cx,
                    );
                }
                if let AgentConsoleEvent::Run(request) = &event {
                    view.reject_run_for(
                        request.id,
                        t!("agents.live.authorization_lost").to_string(),
                        cx,
                    );
                }
                match &event {
                    AgentConsoleEvent::LoadSshHistory(request) => view.complete_native_history(
                        request.clone(),
                        Err(t!("agents.live.authorization_lost").to_string()),
                        cx,
                    ),
                    AgentConsoleEvent::ReconnectSshSession { request_id, .. } => view
                        .complete_ssh_reconnect(
                            *request_id,
                            Err(t!("agents.live.authorization_lost").to_string()),
                            cx,
                        ),
                    AgentConsoleEvent::DiscoverSshSessions {
                        connection_id,
                        request_id,
                        ..
                    } => view.set_ssh_sessions(
                        *connection_id,
                        *request_id,
                        Err(t!("agents.live.authorization_lost").to_string()),
                        cx,
                    ),
                    _ => {}
                }
            });
            return;
        }
        match event {
            AgentConsoleEvent::NavigatorChanged => cx.notify(),
            AgentConsoleEvent::ReconnectProject {
                checkout_id,
                expected_connection,
                connection_id,
            } => {
                self.reconnect_agent_project(checkout_id, expected_connection, connection_id, cx);
            }
            AgentConsoleEvent::RefreshRemoteSessions => self.refresh_remote_sessions(cx),
            AgentConsoleEvent::DiscoverSshSessions {
                connection_id,
                request_id,
                cursor,
            } => self.discover_ssh_agent_sessions(connection_id, request_id, cursor, cx),
            AgentConsoleEvent::ReconnectSshSession {
                connection_id,
                request_id,
                session,
            } => self.reconnect_ssh_agent_session(connection_id, request_id, session, cx),
            AgentConsoleEvent::LoadSshHistory(request) => self.load_ssh_agent_history(request, cx),
            AgentConsoleEvent::OpenPlatformSession(session) => {
                if self.platform_connection().is_some()
                    && self
                        .fleet_view
                        .update(cx, |view, cx| view.select_agent_session(&session, cx))
                {
                    self.agent_console.update(cx, |view, cx| {
                        view.clear_ssh_session_preview();
                        view.set_remote_session_active(true, cx);
                    });
                }
            }
            AgentConsoleEvent::SaveFile(request) => self.save_agent_file(request, cx),
            AgentConsoleEvent::Run(request) => self.prepare_agent_run(request, cx),
            AgentConsoleEvent::Stop(run_id) => self.stop_agent_run(run_id, cx),
            AgentConsoleEvent::OpenLocalFile { session_id, path } => {
                self.agent_console.update(cx, |view, cx| {
                    if view.selected_session_id() == Some(session_id) {
                        view.open_local_agent_preview(path, cx);
                    }
                });
            }
            AgentConsoleEvent::CheckReadiness { check_id, context } => {
                self.check_remote_agent_readiness(check_id, context, cx)
            }
            AgentConsoleEvent::OpenTerminal { context } => self.open_agent_terminal(context, cx),
            AgentConsoleEvent::OpenWorkspaceLauncher => self.open_agent_workspace_launcher(cx),
            AgentConsoleEvent::OpenProjectSetup => self.open_agent_project_setup(cx),
            AgentConsoleEvent::OpenRemoteFile {
                context,
                session_id,
                request_id,
                connection_id,
                path,
            } => self.open_remote_agent_file(
                session_id,
                request_id,
                connection_id,
                context,
                path,
                cx,
            ),
            AgentConsoleEvent::Steer { run_id, text } => self.send_live_codex(
                run_id,
                shelldeck_core::agent_codex::LiveCodexCommand::Steer { text },
                cx,
            ),
            AgentConsoleEvent::QuestionAnswer {
                run_id,
                question_id,
                answer,
            } => self.send_live_codex(
                run_id,
                shelldeck_core::agent_codex::LiveCodexCommand::Answer {
                    question_id,
                    answer,
                },
                cx,
            ),
            AgentConsoleEvent::ApprovalDecision {
                run_id,
                request_id,
                option_id,
            } => {
                if let Some(broker) = self
                    .active_agent_runs
                    .get(&run_id)
                    .and_then(|run| run.approval_broker.as_ref())
                {
                    if let Err(error) = broker.respond(&request_id, option_id.as_deref()) {
                        self.agent_console.update(cx, |view, cx| {
                            view.reject_approval_decision(
                                run_id,
                                &request_id,
                                error.to_string(),
                                cx,
                            )
                        });
                        self.show_toast(error.to_string(), ToastLevel::Warning, cx);
                    } else {
                        self.agent_console.update(cx, |view, cx| {
                            view.push_stream_event_for(
                                run_id,
                                AgentStreamEvent::InteractionResolved {
                                    id: request_id.clone(),
                                },
                                cx,
                            )
                        });
                    }
                } else {
                    self.send_live_codex(
                        run_id,
                        shelldeck_core::agent_codex::LiveCodexCommand::Approval {
                            request_id,
                            option_id,
                        },
                        cx,
                    );
                }
            }
            AgentConsoleEvent::BrowseRemoteDirectory(request) => {
                self.browse_remote_agent_directory(request, cx);
            }
            AgentConsoleEvent::CloseSession(session_id) => {
                self.agent_session_bindings.remove(&session_id);
                self.workspace_hub.update(cx, |hub, cx| {
                    if let Err(error) = hub.remove_agent_session_tabs(session_id, cx) {
                        tracing::warn!(%error, %session_id, "could not remove closed agent session tabs");
                    }
                });
                if self.active_view == ActiveView::Workspaces {
                    let visible_session = self.workspace_hub.read(cx).active_agent_session_id();
                    self.agent_console.update(cx, |view, cx| {
                        if let Some(session_id) = visible_session {
                            view.select_session(session_id, cx);
                        }
                        view.set_surface_visible(visible_session.is_some(), cx);
                    });
                }
            }
        }
    }

    pub(super) fn start_agent_run(&mut self, mut request: AgentRunRequest, cx: &mut Context<Self>) {
        if !self.can_access_mode(AppMode::Dev) {
            self.agent_console.update(cx, |view, cx| {
                view.reject_run_for(
                    request.id,
                    t!("agents.live.authorization_lost").to_string(),
                    cx,
                )
            });
            return;
        }

        if !self.imported_agent_destination_matches(&request) {
            self.reject_agent_run(
                &request,
                t!("agents.ssh_sessions.destination_changed").to_string(),
                ToastLevel::Warning,
                cx,
            );
            return;
        }
        if !agent_run_has_capacity(self.active_agent_runs.len()) {
            let message = t!(
                "agents.error.parallel_limit",
                count = DEFAULT_MAX_CONCURRENT_AGENT_SESSIONS.to_string()
            )
            .to_string();
            self.agent_console.update(cx, |view, cx| {
                view.reject_run_for(request.id, message.clone(), cx)
            });
            self.show_toast(message, ToastLevel::Warning, cx);
            return;
        }
        if request.provider == AgentProvider::Codex && matches!(request.target, AgentTarget::Local)
        {
            self.start_live_codex(request, cx);
            return;
        }
        if request.provider == AgentProvider::Automonique {
            self.start_automonique_agent_run(request, cx);
            return;
        }
        if let Some(workdir) =
            remote_launch_workdir(self.workspace_hub.read(cx).catalog(), &request)
        {
            let admitted = self.agent_console.update(cx, |view, cx| {
                view.admit_starting_run_workdir(&mut request, &workdir, cx)
            });
            if let Err(message) = admitted {
                self.agent_console.update(cx, |view, cx| {
                    view.reject_run_for(request.id, message.clone(), cx)
                });
                self.show_toast(message, ToastLevel::Error, cx);
                return;
            }
        }
        let mut options = self
            .agent_console
            .update(cx, |view, _| view.take_run_options(request.id));
        let remote_browser = match self.agent_console.update(cx, |view, cx| {
            view.attach_browser_tools(&request, &mut options, cx)
        }) {
            Ok(remote) => remote,
            Err(error) => {
                self.reject_agent_run(&request, error, ToastLevel::Error, cx);
                return;
            }
        };
        let mut spec = match AgentCommandSpec::for_request_with_options(&request, &options) {
            Ok(spec) => spec,
            Err(error) => {
                let message = error.to_string();
                self.agent_console.update(cx, |view, cx| {
                    view.reject_run_for(request.id, message.clone(), cx)
                });
                self.show_toast(message, ToastLevel::Error, cx);
                return;
            }
        };
        let connection = match &request.target {
            AgentTarget::Local => None,
            AgentTarget::Ssh { connection_id, .. } => {
                let Some(connection) = self
                    .connections
                    .iter()
                    .find(|connection| connection.id == *connection_id)
                    .cloned()
                else {
                    let message = t!("agents.error.target_missing").to_string();
                    self.agent_console.update(cx, |view, cx| {
                        view.reject_run_for(request.id, message.clone(), cx)
                    });
                    self.show_toast(message, ToastLevel::Error, cx);
                    return;
                };
                Some(connection)
            }
        };
        if connection.is_none() {
            if let Some(program) = resolve_local_agent_program(&spec.program) {
                spec.program = program;
            } else {
                let message = t!(
                    "agents.error.binary_missing",
                    binary = spec.program.as_str()
                )
                .to_string();
                self.agent_console.update(cx, |view, cx| {
                    view.reject_run_for(request.id, message.clone(), cx)
                });
                self.show_toast(message, ToastLevel::Error, cx);
                return;
            }
        }
        if connection.is_none() && !spec.cwd.is_dir() {
            let message = t!(
                "agents.error.workdir_missing",
                workdir = request.workdir.as_str()
            )
            .to_string();
            self.agent_console.update(cx, |view, cx| {
                view.reject_run_for(request.id, message.clone(), cx)
            });
            self.show_toast(message, ToastLevel::Error, cx);
            return;
        }

        let run_id = request.id;
        let provider = request.provider;
        self.agent_console
            .update(cx, |view, cx| view.begin_run(request.clone(), cx));
        if let Err(message) = self.bind_agent_run_to_workspace(&request, cx) {
            self.agent_console.update(cx, |view, cx| {
                view.reject_run_for(request.id, message.clone(), cx)
            });
            self.show_toast(message, ToastLevel::Error, cx);
            return;
        }

        let (shutdown_tx, shutdown_rx) = tokio::sync::mpsc::channel::<()>(1);
        let (event_tx, event_rx) = std::sync::mpsc::channel::<AgentStreamEvent>();
        let (done_tx, done_rx) = std::sync::mpsc::channel::<AgentDone>();
        let thread = match connection {
            Some(connection) => spawn_remote_agent(
                &request,
                spec,
                connection,
                remote_browser,
                shutdown_rx,
                event_tx,
                done_tx,
            ),
            None => spawn_local_agent(run_id, provider, spec, shutdown_rx, event_tx, done_tx),
        };
        let thread = match thread {
            Ok(thread) => thread,
            Err(error) => {
                self.agent_console.update(cx, |view, cx| {
                    view.finish_run(run_id, None, Some(error.to_string()), cx)
                });
                return;
            }
        };
        self.active_agent_runs.insert(
            run_id,
            ActiveAgentRun {
                shutdown: AgentShutdown::Process(shutdown_tx),
                approval_broker: None,
                _thread: thread,
            },
        );

        let console = self.agent_console.downgrade();
        let mut pump = super::agent_stream::AgentEventPump::new(event_rx, done_rx);
        cx.spawn(async move |this, cx: &mut AsyncApp| loop {
            cx.background_executor()
                .timer(super::agent_stream::FRAME_INTERVAL)
                .await;
            let (events, completion) = pump.poll();
            if !events.is_empty() {
                let _ = console.update(cx, |view, cx| {
                    view.push_stream_events_for(run_id, events, cx)
                });
            }
            if let Some((exit_code, error)) = completion {
                let error = error.or_else(|| {
                    (exit_code == Some(127)).then(|| {
                        t!(
                            "agents.error.remote_binary_missing",
                            binary = provider.binary()
                        )
                        .to_string()
                    })
                });
                let _ = this.update(cx, |workspace, cx| {
                    workspace.complete_agent_run(run_id, exit_code, error, cx)
                });
                break;
            }
        })
        .detach();
    }

    fn start_automonique_agent_run(&mut self, request: AgentRunRequest, cx: &mut Context<Self>) {
        if let Err(error) = request.validate() {
            self.reject_agent_run(&request, error.to_string(), ToastLevel::Error, cx);
            return;
        }
        if let Err(error) = automonique_agent_admission(&request) {
            let message = match error {
                AutomoniqueAdmissionError::SshTarget => {
                    t!("agents.error.automonique_ssh_boundary").to_string()
                }
                AutomoniqueAdmissionError::ModelOverride => {
                    t!("agents.error.automonique_model_override").to_string()
                }
            };
            self.reject_agent_run(&request, message, ToastLevel::Warning, cx);
            return;
        }
        let Some(program) = resolve_local_agent_program(request.provider.binary()) else {
            self.reject_agent_run(
                &request,
                t!(
                    "agents.error.binary_missing",
                    binary = request.provider.binary()
                )
                .to_string(),
                ToastLevel::Error,
                cx,
            );
            return;
        };
        if !std::path::Path::new(&request.workdir).is_dir() {
            self.reject_agent_run(
                &request,
                t!(
                    "agents.error.workdir_missing",
                    workdir = request.workdir.as_str()
                )
                .to_string(),
                ToastLevel::Error,
                cx,
            );
            return;
        }

        let run_id = request.id;
        let mut options = self
            .agent_console
            .update(cx, |view, _| view.take_run_options(run_id));
        if let Err(error) = self.agent_console.update(cx, |view, cx| {
            view.attach_browser_tools(&request, &mut options, cx)
        }) {
            self.reject_agent_run(&request, error, ToastLevel::Error, cx);
            return;
        }
        if let Err(error) =
            shelldeck_core::agent_browser_tools::validate_browser_mcp(&request, &options)
        {
            self.reject_agent_run(&request, error.to_string(), ToastLevel::Error, cx);
            return;
        }
        self.agent_console
            .update(cx, |view, cx| view.begin_run(request.clone(), cx));
        if let Err(message) = self.bind_agent_run_to_workspace(&request, cx) {
            self.agent_console.update(cx, |view, cx| {
                view.reject_run_for(request.id, message.clone(), cx)
            });
            self.show_toast(message, ToastLevel::Error, cx);
            return;
        }

        let (cancel_tx, cancel_rx) = shelldeck_core::acp::cancel_channel();
        let (event_tx, event_rx) = std::sync::mpsc::channel::<AgentStreamEvent>();
        let (done_tx, done_rx) = std::sync::mpsc::channel::<AgentDone>();
        let broker = std::sync::Arc::new(AcpInteractivePermissionBroker::new(
            request.access,
            event_tx.clone(),
            std::time::Duration::from_secs(180),
        ));
        let thread = match spawn_local_automonique_agent(
            request,
            program,
            cancel_rx,
            event_tx,
            done_tx,
            broker.clone(),
            options,
        ) {
            Ok(thread) => thread,
            Err(error) => {
                self.agent_console.update(cx, |view, cx| {
                    view.finish_run(run_id, None, Some(error.to_string()), cx)
                });
                return;
            }
        };
        self.active_agent_runs.insert(
            run_id,
            ActiveAgentRun {
                shutdown: AgentShutdown::Acp(parking_lot::Mutex::new(Some(cancel_tx))),
                approval_broker: Some(broker),
                _thread: thread,
            },
        );

        let console = self.agent_console.downgrade();
        let mut pump = super::agent_stream::AgentEventPump::new(event_rx, done_rx);
        cx.spawn(async move |this, cx: &mut AsyncApp| loop {
            cx.background_executor()
                .timer(super::agent_stream::FRAME_INTERVAL)
                .await;
            let (events, completion) = pump.poll();
            if !events.is_empty() {
                let _ = console.update(cx, |view, cx| {
                    view.push_stream_events_for(run_id, events, cx)
                });
            }
            if let Some((exit_code, error)) = completion {
                let _ = this.update(cx, |workspace, cx| {
                    workspace.complete_agent_run(run_id, exit_code, error, cx)
                });
                break;
            }
        })
        .detach();
    }

    fn reject_agent_run(
        &mut self,
        request: &AgentRunRequest,
        message: String,
        level: ToastLevel,
        cx: &mut Context<Self>,
    ) {
        self.agent_console.update(cx, |view, cx| {
            view.reject_run_for(request.id, message.clone(), cx)
        });
        self.show_toast(message, level, cx);
    }

    fn stop_agent_run(&mut self, run_id: Uuid, cx: &mut Context<Self>) {
        self.agent_console
            .update(cx, |view, _| view.retire_browser_run(run_id));
        if let Some(run) = self.active_agent_runs.get(&run_id) {
            run.stop();
        } else {
            self.agent_console
                .update(cx, |view, cx| view.finish_run(run_id, None, None, cx));
        }
    }

    fn browse_remote_agent_directory(
        &mut self,
        request: crate::agent_console_view::AgentRemoteDirectoryRequest,
        cx: &mut Context<Self>,
    ) {
        let crate::agent_console_view::AgentRemoteDirectoryRequest {
            session_id,
            connection_id,
            context,
            root,
            path,
            ..
        } = request.clone();
        let matches = self.agent_console.read(cx).selected_execution_context(cx)
            .is_some_and(|(id, current)| id == session_id && current == context && matches!(current.target, AgentTarget::Ssh { connection_id: selected, .. } if selected == connection_id));
        let connection = matches
            .then(|| {
                self.connections
                    .iter()
                    .find(|c| c.id == connection_id)
                    .cloned()
            })
            .flatten();
        let requested_path = path.clone();
        let task = cx.background_executor().spawn(async move {
            let connection =
                connection.ok_or_else(|| t!("agents.error.target_missing").to_string())?;
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|e| e.to_string())?;
            runtime.block_on(async {
                tokio::time::timeout(std::time::Duration::from_secs(30), async {
                    let session = SshClient::new()
                        .connect(&connection)
                        .await
                        .map_err(|e| e.to_string())?;
                    shelldeck_ssh::agent_files::list(&session, &root, &path)
                        .await
                        .map_err(super::agent_tools::agent_file_error)
                })
                .await
                .map_err(|_| t!("agents.editor.timeout").to_string())?
            })
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                let (root, path, truncated, entries) = match result {
                    Ok(directory) => (
                        directory.root,
                        directory.path,
                        directory.truncated,
                        Ok(directory
                            .entries
                            .into_iter()
                            .map(|entry| AgentRemoteFileEntry {
                                path: entry.path,
                                name: entry.name,
                                is_dir: entry.is_dir,
                            })
                            .collect()),
                    ),
                    Err(error) => (String::new(), requested_path.clone(), false, Err(error)),
                };
                this.agent_console.update(cx, |view, cx| {
                    view.set_remote_directory_listing(
                        crate::agent_console_view::AgentRemoteDirectoryListing {
                            request,
                            root,
                            truncated,
                            resolved_path: path,
                            result: entries,
                        },
                        cx,
                    )
                });
            });
        })
        .detach();
    }

    pub(super) fn stop_all_agent_runs(&mut self) {
        for run in self.active_agent_runs.values() {
            run.stop();
        }
        self.active_agent_runs.clear();
    }

    pub(super) fn refresh_agent_connections(&mut self, cx: &mut Context<Self>) {
        self.workspace_hub.update(cx, |hub, cx| {
            hub.refresh_browse_connections(&self.connections, cx)
        });
        let connections = self
            .connections
            .iter()
            .map(
                |connection| crate::agent_console_view::AgentConnectionOption {
                    id: connection.id,
                    label: connection.display_name().to_string(),
                    host: connection.hostname.clone(),
                },
            )
            .collect();
        self.agent_console
            .update(cx, |view, cx| view.set_connections(connections, cx));
        self.refresh_agent_projects(cx);
    }

    pub(super) fn refresh_agent_projects(&mut self, cx: &mut Context<Self>) {
        let groups = agent_project_groups(self.workspace_hub.read(cx).catalog(), &self.connections);
        self.agent_console
            .update(cx, |view, cx| view.set_project_groups(groups, cx));
    }

    pub(super) fn open_project_agents(
        &mut self,
        checkout: CatalogCheckoutId,
        cx: &mut Context<Self>,
    ) {
        if !self.can_access_mode(AppMode::Dev) || self.effective_mode() != AppMode::Dev {
            return;
        }
        // Resolve the current saved checkout rather than trusting an old UI row.
        let groups = agent_project_groups(self.workspace_hub.read(cx).catalog(), &self.connections);
        let Some(worktree) = groups
            .iter()
            .flat_map(|group| &group.worktrees)
            .find(|worktree| worktree.id == checkout.to_string())
            .cloned()
        else {
            return;
        };
        if matches!(&worktree.target, AgentTarget::Ssh { connection_id, .. }
            if !self.connections.iter().any(|connection| connection.id == *connection_id))
        {
            return;
        }
        self.agent_console.update(cx, |view, cx| {
            view.set_project_groups(groups, cx);
            view.select_worktree_session(&worktree, cx);
        });
        self.activate_dev_section(SidebarSection::Agents, cx);
    }

    pub(super) fn bind_agent_run_to_workspace(
        &mut self,
        request: &AgentRunRequest,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        let Some(session_id) = self.agent_console.read(cx).session_id_for_run(request.id) else {
            return Err(t!("agents.error.session_missing").to_string());
        };
        let candidate = catalog_workspace_for_run(self.workspace_hub.read(cx).catalog(), request);
        if let Some(bound) = self.agent_session_bindings.get(&session_id).copied() {
            if candidate != Some(bound) {
                return Err(t!("agents.error.session_checkout_changed").to_string());
            }
        }
        let Some((workspace, checkout_id)) = candidate else {
            return Ok(());
        };
        let title = self
            .agent_console
            .read(cx)
            .sessions()
            .iter()
            .find(|session| session.id == session_id)
            .map(|session| session.name.clone())
            .unwrap_or_else(|| t!("agents.session.new").to_string());
        let foreground = self.agent_console.read(cx).run_requests_focus(request.id)
            && matches!(
                self.active_view,
                ActiveView::Agents | ActiveView::Workspaces
            );
        if foreground {
            self.workspace_hub.update(cx, |hub, cx| {
                hub.open_or_focus_agent_session(
                    workspace,
                    title,
                    AgentSessionBinding {
                        checkout_id,
                        session_id,
                    },
                    cx,
                )
                .map(|_| ())
            })?;
        }
        self.agent_session_bindings
            .insert(session_id, (workspace, checkout_id));
        self.agent_console.update(cx, |view, cx| {
            view.set_session_context_locked(session_id, true, cx)
        });
        if foreground {
            self.active_view = ActiveView::Workspaces;
            self.on_active_view_changed(cx);
        }
        Ok(())
    }
}

pub(super) fn agent_project_groups(
    catalog: &ProjectCatalog,
    connections: &[Connection],
) -> Vec<AgentProjectGroup> {
    catalog
        .projects()
        .map(|project| AgentProjectGroup {
            id: project.id().to_string(),
            label: project.name().to_string(),
            worktrees: project
                .checkouts()
                .enumerate()
                .map(|(index, checkout)| {
                    let (path, target) = match checkout.host() {
                        CheckoutHost::Local { root, .. } => {
                            (root.display().to_string(), AgentTarget::Local)
                        }
                        CheckoutHost::Ssh {
                            connection_id,
                            root,
                        } => {
                            let connection_label = connections
                                .iter()
                                .find(|connection| connection.id == *connection_id)
                                .map(|connection| connection.display_name().to_string())
                                .unwrap_or_else(|| connection_id.to_string());
                            (
                                root.as_str().to_string(),
                                AgentTarget::Ssh {
                                    connection_id: *connection_id,
                                    connection_label,
                                },
                            )
                        }
                    };
                    AgentWorktreeOption {
                        id: checkout.id().to_string(),
                        label: checkout.label().to_string(),
                        path,
                        target,
                        branch: None,
                        is_primary: index == 0,
                    }
                })
                .collect(),
        })
        .collect()
}

fn catalog_workspace_for_run(
    catalog: &ProjectCatalog,
    request: &AgentRunRequest,
) -> Option<(CatalogWorkspaceId, CatalogCheckoutId)> {
    catalog
        .workspaces()
        .filter(|workspace| workspace.lifecycle() == UserWorkspaceLifecycle::Active)
        .filter_map(|workspace| {
            let checkout = catalog
                .checkout_in_project(workspace.project_id(), workspace.checkout_id())
                .ok()?;
            let authority_len = match (&request.target, checkout.host()) {
                (AgentTarget::Local, CheckoutHost::Local { root, .. }) => {
                    let canonical_root = std::fs::canonicalize(root).ok()?;
                    let canonical_workdir = std::fs::canonicalize(&request.workdir).ok()?;
                    if !canonical_workdir.starts_with(&canonical_root) {
                        return None;
                    }
                    canonical_root.as_os_str().len()
                }
                (
                    AgentTarget::Ssh { connection_id, .. },
                    CheckoutHost::Ssh {
                        connection_id: expected,
                        root,
                    },
                ) if connection_id == expected
                    && RemotePosixPath::new(request.workdir.clone()).is_ok_and(|candidate| {
                        remote_path_contains(root.as_str(), candidate.as_str())
                    }) =>
                {
                    root.as_str().len()
                }
                _ => return None,
            };
            Some((authority_len, workspace.id(), workspace.checkout_id()))
        })
        .max_by_key(|(authority_len, _, _)| *authority_len)
        .map(|(_, workspace, checkout)| (workspace, checkout))
}

fn remote_launch_workdir(
    catalog: &shelldeck_core::config::workspace_catalog::ProjectCatalog,
    request: &AgentRunRequest,
) -> Option<String> {
    if request.resume_session.is_some() {
        return None;
    }
    let AgentTarget::Ssh { connection_id, .. } = &request.target else {
        return None;
    };
    mapped_remote_agent_workdir(catalog, *connection_id, &request.workdir)
}

fn remote_path_contains(root: &str, candidate: &str) -> bool {
    candidate == root
        || candidate
            .strip_prefix(root.trim_end_matches('/'))
            .is_some_and(|suffix| suffix.starts_with('/'))
}

/// Translate an inherited local checkout path to the corresponding SSH
/// checkout in the same catalog project. This is a launch-time safety net for
/// stale sessions and target changes; the cockpit normally applies the remote
/// checkout root before emitting the request.
///
/// Mapping is deliberately conservative. It requires one unambiguous remote
/// checkout for the selected connection and repository, and preserves only a
/// validated relative suffix beneath the longest matching local authority.
fn mapped_remote_agent_workdir(
    catalog: &ProjectCatalog,
    connection_id: Uuid,
    workdir: &str,
) -> Option<String> {
    let local_workdir = std::path::Path::new(workdir);
    catalog
        .projects()
        .filter_map(|project| {
            let source = project
                .checkouts()
                .filter_map(|checkout| match checkout.host() {
                    CheckoutHost::Local { root, .. } => local_workdir
                        .strip_prefix(root)
                        .ok()
                        .map(|relative| (root.as_os_str().len(), checkout, relative)),
                    CheckoutHost::Ssh { .. } => None,
                })
                .max_by_key(|(authority_len, _, _)| *authority_len)?;
            let repository = source.1.repository().slug.as_str();
            let mut targets = project
                .checkouts()
                .filter_map(|checkout| match checkout.host() {
                    CheckoutHost::Ssh {
                        connection_id: candidate,
                        root,
                    } if *candidate == connection_id
                        && checkout.repository().slug == repository =>
                    {
                        Some(root)
                    }
                    _ => None,
                });
            let target = targets.next()?;
            if targets.next().is_some() {
                return None;
            }
            let relative = source
                .2
                .components()
                .map(|component| match component {
                    std::path::Component::Normal(value) => value.to_str(),
                    _ => None,
                })
                .collect::<Option<Vec<_>>>()?
                .join("/");
            let relative = WorkspaceRelativePath::new(relative).ok()?;
            let mapped = if relative.as_str().is_empty() {
                target.as_str().to_string()
            } else {
                format!(
                    "{}/{}",
                    target.as_str().trim_end_matches('/'),
                    relative.as_str()
                )
            };
            RemotePosixPath::new(mapped.clone()).ok()?;
            Some((source.0, mapped))
        })
        .max_by_key(|(authority_len, _)| *authority_len)
        .map(|(_, mapped)| mapped)
}

pub(crate) fn resolve_local_agent_program(program: &str) -> Option<String> {
    if shelldeck_core::util::executable_on_path(program) {
        return Some(program.to_string());
    }
    let home = shelldeck_core::util::home_dir()?;
    let mut directories = vec![home.join(".local/bin"), home.join("bin")];
    #[cfg(unix)]
    directories.extend(["/usr/local/bin".into(), "/opt/homebrew/bin".into()]);
    #[cfg(windows)]
    let extensions = ["", ".exe", ".cmd", ".bat"];
    #[cfg(not(windows))]
    let extensions = [""];

    for directory in directories {
        for extension in extensions {
            let candidate = directory.join(format!("{program}{extension}"));
            let Ok(metadata) = candidate.metadata() else {
                continue;
            };
            if !metadata.is_file() {
                continue;
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt as _;
                if metadata.permissions().mode() & 0o111 == 0 {
                    continue;
                }
            }
            return Some(candidate.display().to_string());
        }
    }
    None
}

fn send_stream_frames(
    frames: Vec<AgentStreamFrame>,
    parser: &mut AgentStreamParser,
    parse_json: bool,
    event_tx: &std::sync::mpsc::Sender<AgentStreamEvent>,
) {
    for frame in frames {
        match frame {
            AgentStreamFrame::Line(line) if parse_json => {
                for event in parser.parse_line(&line) {
                    let _ = event_tx.send(event);
                }
            }
            AgentStreamFrame::Line(line) => {
                if !line.trim().is_empty() {
                    let _ = event_tx.send(AgentStreamEvent::Activity(line));
                }
            }
            AgentStreamFrame::Oversized => {
                let _ = event_tx.send(AgentStreamEvent::Activity(
                    AGENT_STREAM_OVERSIZED_LABEL.to_string(),
                ));
            }
        }
    }
}

fn forward_local_stream(
    mut reader: impl Read,
    provider: shelldeck_core::agent_runtime::AgentProvider,
    parse_json: bool,
    event_tx: &std::sync::mpsc::Sender<AgentStreamEvent>,
) {
    let mut framer = AgentStreamFramer::default();
    let mut parser = AgentStreamParser::new(provider);
    let mut chunk = [0_u8; 8 * 1024];
    loop {
        match reader.read(&mut chunk) {
            Ok(0) => break,
            Ok(read) => send_stream_frames(
                framer.push(&chunk[..read]),
                &mut parser,
                parse_json,
                event_tx,
            ),
            Err(_) => break,
        }
    }
    send_stream_frames(framer.finish(), &mut parser, parse_json, event_tx);
    if parse_json {
        for event in parser.finish() {
            let _ = event_tx.send(event);
        }
    }
}

fn spawn_local_automonique_agent(
    request: AgentRunRequest,
    program: String,
    cancel_rx: shelldeck_core::acp::AcpCancelReceiver,
    event_tx: std::sync::mpsc::Sender<AgentStreamEvent>,
    done_tx: std::sync::mpsc::Sender<AgentDone>,
    permissions: std::sync::Arc<AcpInteractivePermissionBroker>,
    options: shelldeck_core::agent_runtime::AgentInputOptions,
) -> std::io::Result<std::thread::JoinHandle<()>> {
    let run_id = request.id;
    std::thread::Builder::new()
        .name(format!("agent-acp-{run_id}"))
        .spawn(move || {
            let mut turn =
                shelldeck_core::acp::AcpTurnRequest::text(&request.workdir, request.prompt);
            turn.mcp_servers =
                shelldeck_core::agent_browser_tools::acp_browser_servers(&options.mcp_servers);
            if let Some(session) = request.resume_session {
                turn = turn.resume(session);
            }
            let result = shelldeck_core::acp::AcpClient::new(
                shelldeck_core::acp::AcpLaunch::automonique(program),
            )
            .with_permissions(permissions)
            .with_agent_events(event_tx.clone())
            .prompt_cancellable(turn, cancel_rx);
            match result {
                Ok(Some(result)) => {
                    let _ = event_tx.send(AgentStreamEvent::Session(result.session_id.to_string()));
                    let _ = done_tx.send((Some(0), None));
                }
                Ok(None) => {
                    let _ = done_tx.send((None, None));
                }
                Err(error) => {
                    let _ = done_tx.send((None, Some(error.to_string())));
                }
            }
        })
}

fn spawn_local_agent(
    run_id: Uuid,
    provider: shelldeck_core::agent_runtime::AgentProvider,
    spec: AgentCommandSpec,
    mut shutdown_rx: tokio::sync::mpsc::Receiver<()>,
    event_tx: std::sync::mpsc::Sender<AgentStreamEvent>,
    done_tx: std::sync::mpsc::Sender<AgentDone>,
) -> std::io::Result<std::thread::JoinHandle<()>> {
    std::thread::Builder::new()
        .name(format!("agent-local-{run_id}"))
        .spawn(move || {
            let mut command = Command::new(&spec.program);
            command
                .args(&spec.args)
                .current_dir(&spec.cwd)
                .stdin(if spec.stdin.is_some() {
                    Stdio::piped()
                } else {
                    Stdio::null()
                })
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
            for key in &spec.remove_env {
                command.env_remove(key);
            }
            configure_local_process(&mut command);
            let mut child = match command.spawn() {
                Ok(child) => child,
                Err(error) => {
                    let _ = done_tx.send((None, Some(error.to_string())));
                    return;
                }
            };
            let mut process_tree = match LocalProcessTree::capture(child.id()) {
                Ok(process_tree) => process_tree,
                Err(error) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    let _ = done_tx.send((None, Some(error.to_string())));
                    return;
                }
            };
            if let (Some(input), Some(mut stdin)) = (spec.stdin, child.stdin.take()) {
                if let Err(error) = stdin.write_all(&input) {
                    let _ = terminate_local_process(&mut child, &mut process_tree);
                    let _ = done_tx.send((None, Some(error.to_string())));
                    return;
                }
            }

            let stdout = child.stdout.take();
            let stderr = child.stderr.take();
            let stdout_tx = event_tx.clone();
            let stdout_thread = std::thread::spawn(move || {
                if let Some(stdout) = stdout {
                    forward_local_stream(stdout, provider, true, &stdout_tx);
                }
            });
            let stderr_thread = std::thread::spawn(move || {
                if let Some(stderr) = stderr {
                    forward_local_stream(stderr, provider, false, &event_tx);
                }
            });

            loop {
                match child.try_wait() {
                    Ok(Some(status)) => {
                        process_tree.terminate();
                        let _ = stdout_thread.join();
                        let _ = stderr_thread.join();
                        let _ = done_tx.send((status.code(), None));
                        return;
                    }
                    Ok(None) => {}
                    Err(error) => {
                        let _ = terminate_local_process(&mut child, &mut process_tree);
                        let _ = done_tx.send((None, Some(error.to_string())));
                        return;
                    }
                }
                match shutdown_rx.try_recv() {
                    Ok(()) | Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                        let _ = terminate_local_process(&mut child, &mut process_tree);
                        let _ = stdout_thread.join();
                        let _ = stderr_thread.join();
                        let _ = done_tx.send((None, None));
                        return;
                    }
                    Err(tokio::sync::mpsc::error::TryRecvError::Empty) => {}
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
        })
}

fn spawn_remote_agent(
    request: &AgentRunRequest,
    spec: AgentCommandSpec,
    connection: Connection,
    mut browser: Option<shelldeck_ssh::browser_bridge::BrowserBridgeConfig>,
    shutdown_rx: tokio::sync::mpsc::Receiver<()>,
    event_tx: std::sync::mpsc::Sender<AgentStreamEvent>,
    done_tx: std::sync::mpsc::Sender<AgentDone>,
) -> std::io::Result<std::thread::JoinHandle<()>> {
    let (run_id, provider) = (request.id, request.provider);
    std::thread::Builder::new()
        .name(format!("agent-remote-{run_id}"))
        .spawn(move || {
            let runtime = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(runtime) => runtime,
                Err(error) => {
                    let _ = done_tx.send((None, Some(error.to_string())));
                    return;
                }
            };
            runtime.block_on(async move {
                let mut shutdown_rx = shutdown_rx;
                let client = SshClient::new();
                let connect = client.connect(&connection);
                let mut session = match tokio::select! {
                    biased;
                    _ = shutdown_rx.recv() => {
                        let _ = done_tx.send((None, None));
                        return;
                    }
                    _ = async { match &mut browser { Some(browser) => browser.revoked().await, None => std::future::pending::<()>().await } } => {
                        let _ = done_tx.send((None, Some(t!("agents.live_browser.remote_expired").to_string())));
                        return;
                    }
                    result = connect => result,
                } {
                    Ok(session) => session,
                    Err(error) => {
                        let _ = done_tx.send((None, Some(error.to_string())));
                        return;
                    }
                };
                let (output_tx, mut output_rx) = tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();
                let forward = tokio::spawn(async move {
                    let mut framer = AgentStreamFramer::default();
                    let mut parser = AgentStreamParser::new(provider);
                    while let Some(bytes) = output_rx.recv().await {
                        send_stream_frames(framer.push(&bytes), &mut parser, true, &event_tx);
                    }
                    send_stream_frames(framer.finish(), &mut parser, true, &event_tx);
                    for event in parser.finish() {
                        let _ = event_tx.send(event);
                    }
                });
                let result = match browser {
                    Some(browser) => shelldeck_ssh::browser_bridge::run(
                        &mut session, run_id, provider, &spec, browser, output_tx, shutdown_rx,
                    ).await,
                    None => session.exec_cancellable(&spec.remote_shell_command(), output_tx, shutdown_rx).await,
                };
                let _ = forward.await;
                match result {
                    Ok(exit_code) => {
                        let _ = done_tx.send((exit_code.map(|code| code as i32), None));
                    }
                    Err(error) => {
                        let _ = done_tx.send((None, Some(error.to_string())));
                    }
                }
            });
        })
}

#[cfg(test)]
mod tests {
    use super::{
        agent_run_has_capacity, automonique_agent_admission, forward_local_stream,
        mapped_remote_agent_workdir, spawn_local_agent, AgentCommandSpec, AgentStreamEvent,
        AgentStreamFrame, AgentStreamFramer, AGENT_STREAM_OVERSIZED_LABEL,
        DEFAULT_MAX_CONCURRENT_AGENT_SESSIONS,
    };
    use shelldeck_core::agent_runtime::{
        AgentAccessMode, AgentProvider, AgentRunRequest, AgentTarget,
    };
    use shelldeck_core::config::workspace_catalog::{
        CatalogCheckoutId, CatalogProjectId, CheckoutHost, ProjectCatalog, ProjectCheckout,
        ProjectRecord, RemotePosixPath, RepositoryIdentity,
    };
    use std::time::Duration;
    use uuid::Uuid;

    // SDTEST-1907 — SDUC-475, SDUC-480
    #[test]
    fn sdtest_1907_automonique_never_falls_back_from_ssh_to_local_acp() {
        let mut request = AgentRunRequest::new(
            AgentProvider::Automonique,
            AgentTarget::Local,
            AgentAccessMode::ReadOnly,
            std::env::temp_dir().display().to_string(),
            None,
            "inspect",
        );
        assert!(automonique_agent_admission(&request).is_ok());

        request.target = AgentTarget::Ssh {
            connection_id: Uuid::new_v4(),
            connection_label: "remote".to_string(),
        };
        assert!(automonique_agent_admission(&request).is_err());

        request.target = AgentTarget::Local;
        request.model = Some("provider-owned".to_string());
        assert!(automonique_agent_admission(&request).is_err());
    }

    // SDTEST-1676 — SDUC-475
    #[cfg(unix)]
    #[test]
    fn sdtest_1676_local_runner_streams_and_stops_the_process_group() {
        let spec = AgentCommandSpec {
            program: "sh".to_string(),
            args: vec![
                "-c".to_string(),
                "printf '%s\\n' '{\"type\":\"content.delta\",\"delta\":\"ready\"}'; sleep 30"
                    .to_string(),
            ],
            cwd: std::env::temp_dir(),
            stdin: None,
            remove_env: Vec::new(),
        };
        let (shutdown_tx, shutdown_rx) = tokio::sync::mpsc::channel(1);
        let (event_tx, event_rx) = std::sync::mpsc::channel();
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        let thread = spawn_local_agent(
            Uuid::new_v4(),
            shelldeck_core::agent_runtime::AgentProvider::DeepSeek,
            spec,
            shutdown_rx,
            event_tx,
            done_tx,
        )
        .unwrap();

        assert_eq!(
            event_rx.recv_timeout(Duration::from_secs(2)).unwrap(),
            AgentStreamEvent::TextDelta("ready".to_string())
        );
        shutdown_tx.try_send(()).unwrap();
        assert_eq!(
            done_rx.recv_timeout(Duration::from_secs(3)).unwrap(),
            (None, None)
        );
        thread.join().unwrap();
    }

    // SDTEST-1681 — SDUC-475
    #[test]
    fn sdtest_1681_remote_stream_reassembles_split_utf8_and_json_lines() {
        let payload = "{\"type\":\"content.delta\",\"delta\":\"terminé ✅\"}\r\n".as_bytes();
        let split = payload
            .windows(2)
            .position(|pair| pair[0] == 0xc3 && pair[1] == 0xa9)
            .unwrap()
            + 1;
        let mut framer = AgentStreamFramer::default();
        assert!(framer.push(&payload[..split]).is_empty());
        assert_eq!(
            framer.push(&payload[split..]),
            vec![AgentStreamFrame::Line(
                "{\"type\":\"content.delta\",\"delta\":\"terminé ✅\"}".to_string()
            )]
        );
        assert_eq!(framer.buffered_bytes(), 0);
    }

    // SDTEST-1905 — SDUC-475
    #[test]
    fn sdtest_1905_remote_launch_maps_an_inherited_local_checkout_path() {
        let connection_id = Uuid::new_v4();
        let other_connection = Uuid::new_v4();
        let repository = || RepositoryIdentity {
            slug: "inklura/infra".to_string(),
            canonical_url: None,
        };
        let mut project = ProjectRecord::new(CatalogProjectId::new(), "Infra");
        project.add_checkout(ProjectCheckout::new(
            CatalogCheckoutId::new(),
            "Laptop",
            CheckoutHost::Local {
                device_label: "pc1".to_string(),
                root: "/home/pc1/dev/infra".into(),
            },
            repository(),
        ));
        project.add_checkout(ProjectCheckout::new(
            CatalogCheckoutId::new(),
            "Server",
            CheckoutHost::Ssh {
                connection_id,
                root: RemotePosixPath::new("/home/infra-sj278/infra").unwrap(),
            },
            repository(),
        ));
        let mut catalog = ProjectCatalog::default();
        catalog.insert_project(project).unwrap();
        let request = AgentRunRequest::new(
            AgentProvider::Claude,
            AgentTarget::Ssh {
                connection_id,
                connection_label: "Server".into(),
            },
            shelldeck_core::agent_runtime::AgentAccessMode::ReadOnly,
            "/home/pc1/dev/infra",
            None,
            "continue",
        );
        assert!(super::remote_launch_workdir(&catalog, &request).is_some());
        assert!(
            super::remote_launch_workdir(
                &catalog,
                &request.with_resume_session(Some("existing-remote-id".into()))
            )
            .is_none(),
            "a native resume must keep its verified remote directory"
        );

        assert_eq!(
            mapped_remote_agent_workdir(&catalog, connection_id, "/home/pc1/dev/infra/crates/api")
                .as_deref(),
            Some("/home/infra-sj278/infra/crates/api")
        );
        assert_eq!(
            mapped_remote_agent_workdir(&catalog, connection_id, "/home/pc1/dev/infra").as_deref(),
            Some("/home/infra-sj278/infra")
        );
        assert!(
            mapped_remote_agent_workdir(&catalog, other_connection, "/home/pc1/dev/infra")
                .is_none()
        );
        assert!(mapped_remote_agent_workdir(
            &catalog,
            connection_id,
            "/home/pc1/dev/infrastructure"
        )
        .is_none());
        assert!(
            mapped_remote_agent_workdir(&catalog, connection_id, "/srv/explicit-remote-path")
                .is_none()
        );
    }

    // SDTEST-1897 — SDUC-499
    #[test]
    fn sdtest_1897_local_stream_discards_newline_free_oversize_then_recovers() {
        use shelldeck_core::agent_runtime::MAX_AGENT_STREAM_LINE_BYTES;

        let mut payload = vec![b'x'; MAX_AGENT_STREAM_LINE_BYTES + 17];
        payload.extend_from_slice(b"\n{\"type\":\"content.delta\",\"delta\":\"recovered\"}\n");
        let (event_tx, event_rx) = std::sync::mpsc::channel();
        forward_local_stream(
            std::io::Cursor::new(payload),
            shelldeck_core::agent_runtime::AgentProvider::Jcode,
            true,
            &event_tx,
        );
        drop(event_tx);
        assert_eq!(
            event_rx.into_iter().collect::<Vec<_>>(),
            vec![
                AgentStreamEvent::Activity(AGENT_STREAM_OVERSIZED_LABEL.to_string()),
                AgentStreamEvent::TextDelta("recovered".to_string()),
            ]
        );
    }

    // SDTEST-1883 — SDUC-475
    #[test]
    fn sdtest_1883_parallel_agent_runs_are_bounded_without_a_singleton_gate() {
        assert!(agent_run_has_capacity(0));
        assert!(agent_run_has_capacity(1));
        assert!(agent_run_has_capacity(
            DEFAULT_MAX_CONCURRENT_AGENT_SESSIONS - 1
        ));
        assert!(!agent_run_has_capacity(
            DEFAULT_MAX_CONCURRENT_AGENT_SESSIONS
        ));
        assert!(!agent_run_has_capacity(
            DEFAULT_MAX_CONCURRENT_AGENT_SESSIONS + 1
        ));
    }
}
