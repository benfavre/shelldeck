use super::*;
use crate::agent_console_view::{AgentProjectGroup, AgentRemoteFileEntry, AgentWorktreeOption};
use shelldeck_core::agent_runtime::{
    configure_local_process, parse_stream_line, terminate_local_process, AcpAgentPermissionBroker,
    AgentCommandSpec, AgentProvider, AgentRunRequest, AgentStreamEvent, AgentStreamFrame,
    AgentStreamFramer, AgentTarget, LocalProcessTree, AGENT_STREAM_OVERSIZED_LABEL,
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
}

enum AgentShutdown {
    Process(tokio::sync::mpsc::Sender<()>),
    Acp(parking_lot::Mutex<Option<shelldeck_core::acp::AcpCancelSender>>),
}

impl ActiveAgentRun {
    fn stop(&self) {
        match &self.shutdown {
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
        if !self.can_access_mode(AppMode::Dev) {
            return;
        }
        match event {
            AgentConsoleEvent::Run(request) => self.start_agent_run(request, cx),
            AgentConsoleEvent::Stop(run_id) => self.stop_agent_run(run_id, cx),
            AgentConsoleEvent::OpenLocalFile { session_id, path } => {
                let path = authorized_local_agent_file(
                    self.agent_console.read(cx).sessions(),
                    session_id,
                    &path,
                );
                if let Some(path) = path {
                    self.file_editor.update(cx, |editor, cx| {
                        editor.open_file(path, cx);
                    });
                    self.set_active_view(ActiveView::FileEditor);
                }
            }
            AgentConsoleEvent::BrowseRemoteDirectory {
                session_id,
                connection_id,
                path,
            } => self.browse_remote_agent_directory(session_id, connection_id, path, cx),
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

    fn start_agent_run(&mut self, mut request: AgentRunRequest, cx: &mut Context<Self>) {
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
        if request.provider == AgentProvider::Automonique {
            self.start_automonique_agent_run(request, cx);
            return;
        }
        if let AgentTarget::Ssh { connection_id, .. } = &request.target {
            if let Some(workdir) = mapped_remote_agent_workdir(
                self.workspace_hub.read(cx).catalog(),
                *connection_id,
                &request.workdir,
            ) {
                request.workdir = workdir;
            }
        }
        let mut spec = match AgentCommandSpec::for_request(&request) {
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
                run_id,
                provider,
                spec,
                connection,
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
                _thread: thread,
            },
        );

        let console = self.agent_console.downgrade();
        cx.spawn(async move |this, cx: &mut AsyncApp| loop {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(50))
                .await;
            let mut events = Vec::new();
            while let Ok(event) = event_rx.try_recv() {
                events.push(event);
            }
            if !events.is_empty() {
                let _ = console.update(cx, |view, cx| {
                    for event in events {
                        view.push_stream_event_for(run_id, event, cx);
                    }
                });
            }
            match done_rx.try_recv() {
                Ok((exit_code, error)) => {
                    let error = error.or_else(|| {
                        (exit_code == Some(127)).then(|| {
                            t!(
                                "agents.error.remote_binary_missing",
                                binary = provider.binary()
                            )
                            .to_string()
                        })
                    });
                    let _ = console.update(cx, |view, cx| {
                        view.finish_run(run_id, exit_code, error.clone(), cx)
                    });
                    let _ = this.update(cx, |workspace, cx| {
                        workspace.active_agent_runs.remove(&run_id);
                        let level = if exit_code == Some(0) && error.is_none() {
                            ToastLevel::Success
                        } else if exit_code.is_none() && error.is_none() {
                            ToastLevel::Info
                        } else {
                            ToastLevel::Error
                        };
                        let message = if let Some(error) = error {
                            t!("agents.toast.failed", error = error).to_string()
                        } else if exit_code == Some(0) {
                            t!("agents.toast.completed").to_string()
                        } else if exit_code.is_none() {
                            t!("agents.toast.stopped").to_string()
                        } else {
                            t!(
                                "agents.toast.exit",
                                code = exit_code.unwrap_or(-1).to_string()
                            )
                            .to_string()
                        };
                        workspace.show_toast(message, level, cx);
                    });
                    break;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    let _ = console.update(cx, |view, cx| {
                        view.finish_run(
                            run_id,
                            None,
                            Some(t!("agents.error.runtime_lost").to_string()),
                            cx,
                        )
                    });
                    let _ = this.update(cx, |workspace, _| {
                        workspace.active_agent_runs.remove(&run_id);
                    });
                    break;
                }
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
        let thread = match spawn_local_automonique_agent(
            run_id, request, program, cancel_rx, event_tx, done_tx,
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
                _thread: thread,
            },
        );

        let console = self.agent_console.downgrade();
        cx.spawn(async move |this, cx: &mut AsyncApp| loop {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(50))
                .await;
            let mut events = Vec::new();
            while let Ok(event) = event_rx.try_recv() {
                events.push(event);
            }
            if !events.is_empty() {
                let _ = console.update(cx, |view, cx| {
                    for event in events {
                        view.push_stream_event_for(run_id, event, cx);
                    }
                });
            }
            match done_rx.try_recv() {
                Ok((exit_code, error)) => {
                    let _ = console.update(cx, |view, cx| {
                        view.finish_run(run_id, exit_code, error.clone(), cx)
                    });
                    let _ = this.update(cx, |workspace, cx| {
                        workspace.active_agent_runs.remove(&run_id);
                        let (message, level) = if let Some(error) = error {
                            (
                                t!("agents.toast.failed", error = error).to_string(),
                                ToastLevel::Error,
                            )
                        } else if exit_code == Some(0) {
                            (
                                t!("agents.toast.completed").to_string(),
                                ToastLevel::Success,
                            )
                        } else {
                            (t!("agents.toast.stopped").to_string(), ToastLevel::Info)
                        };
                        workspace.show_toast(message, level, cx);
                    });
                    break;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    let _ = console.update(cx, |view, cx| {
                        view.finish_run(
                            run_id,
                            None,
                            Some(t!("agents.error.runtime_lost").to_string()),
                            cx,
                        )
                    });
                    let _ = this.update(cx, |workspace, _| {
                        workspace.active_agent_runs.remove(&run_id);
                    });
                    break;
                }
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

    fn stop_agent_run(&mut self, run_id: Uuid, _cx: &mut Context<Self>) {
        if let Some(run) = self.active_agent_runs.get(&run_id) {
            run.stop();
        }
    }

    fn browse_remote_agent_directory(
        &mut self,
        session_id: Uuid,
        connection_id: Uuid,
        path: String,
        cx: &mut Context<Self>,
    ) {
        let selected_context_matches =
            self.agent_console
                .read(cx)
                .sessions()
                .iter()
                .any(|session| {
                    session.id == session_id
                        && matches!(
                            session.context.target,
                            AgentTarget::Ssh {
                                connection_id: selected,
                                ..
                            } if selected == connection_id
                        )
                });
        let Some(connection) = selected_context_matches
            .then_some({
                self.connections
                    .iter()
                    .find(|connection| connection.id == connection_id)
                    .cloned()
            })
            .flatten()
        else {
            self.agent_console.update(cx, |view, cx| {
                view.set_remote_directory_listing(
                    session_id,
                    connection_id,
                    path.clone(),
                    path,
                    Err(t!("agents.error.target_missing").to_string()),
                    cx,
                )
            });
            return;
        };
        if path.len() > shelldeck_core::agent_runtime::MAX_AGENT_WORKDIR_BYTES
            || (path != shelldeck_core::agent_runtime::AGENT_REMOTE_HOME_WORKDIR
                && RemotePosixPath::new(path.clone()).is_err())
        {
            self.agent_console.update(cx, |view, cx| {
                view.set_remote_directory_listing(
                    session_id,
                    connection_id,
                    path.clone(),
                    path,
                    Err(t!("agents.error.workdir_absolute").to_string()),
                    cx,
                )
            });
            return;
        }

        let requested_path = path.clone();
        let (result_tx, result_rx) = std::sync::mpsc::channel();
        let spawn_result = std::thread::Builder::new()
            .name(format!("agent-remote-files-{session_id}"))
            .spawn(move || {
                let runtime = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(runtime) => runtime,
                    Err(error) => {
                        let _ = result_tx.send(Err(error.to_string()));
                        return;
                    }
                };
                let result = runtime.block_on(async move {
                    let session = SshClient::new()
                        .connect(&connection)
                        .await
                        .map_err(|error| error.to_string())?;
                    let canonical_path = if requested_path
                        == shelldeck_core::agent_runtime::AGENT_REMOTE_HOME_WORKDIR
                    {
                        let home = session
                            .exec("printf '%s\\n' \"$HOME\"")
                            .await
                            .map_err(|error| error.to_string())?;
                        if !home.success() {
                            return Err("remote home directory is unavailable".to_string());
                        }
                        parse_remote_home(&home.stdout)?
                    } else {
                        requested_path
                    };
                    let command = shelldeck_core::models::discovery::ls_command(&canonical_path);
                    let fallback =
                        shelldeck_core::models::discovery::ls_command_fallback(&canonical_path);
                    let primary = session
                        .exec(&command)
                        .await
                        .map_err(|error| error.to_string())?;
                    let entries = if primary.success() && !primary.stdout.is_empty() {
                        shelldeck_core::models::discovery::parse_stat_output(
                            &String::from_utf8_lossy(&primary.stdout),
                            &canonical_path,
                        )
                    } else {
                        let fallback = session
                            .exec(&fallback)
                            .await
                            .map_err(|error| error.to_string())?;
                        if !fallback.success() {
                            return Err("remote directory is unavailable".to_string());
                        }
                        shelldeck_core::models::discovery::parse_ls_output(
                            &String::from_utf8_lossy(&fallback.stdout),
                            &canonical_path,
                        )
                    };
                    Ok((canonical_path, remote_agent_file_entries(entries)))
                });
                let _ = result_tx.send(result);
            });
        if let Err(error) = spawn_result {
            self.agent_console.update(cx, |view, cx| {
                view.set_remote_directory_listing(
                    session_id,
                    connection_id,
                    path.clone(),
                    path,
                    Err(error.to_string()),
                    cx,
                )
            });
            return;
        }

        let console = self.agent_console.downgrade();
        cx.spawn(async move |_this, cx: &mut AsyncApp| loop {
            match result_rx.try_recv() {
                Ok(Ok((canonical_path, entries))) => {
                    let _ = console.update(cx, |view, cx| {
                        view.set_remote_directory_listing(
                            session_id,
                            connection_id,
                            path.clone(),
                            canonical_path,
                            Ok(entries),
                            cx,
                        )
                    });
                    break;
                }
                Ok(Err(error)) => {
                    let _ = console.update(cx, |view, cx| {
                        view.set_remote_directory_listing(
                            session_id,
                            connection_id,
                            path.clone(),
                            path,
                            Err(error),
                            cx,
                        )
                    });
                    break;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    cx.background_executor()
                        .timer(std::time::Duration::from_millis(50))
                        .await;
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    let _ = console.update(cx, |view, cx| {
                        view.set_remote_directory_listing(
                            session_id,
                            connection_id,
                            path.clone(),
                            path,
                            Err(t!("agents.error.runtime_lost").to_string()),
                            cx,
                        )
                    });
                    break;
                }
            }
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

    fn bind_agent_run_to_workspace(
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
        self.agent_session_bindings
            .insert(session_id, (workspace, checkout_id));
        self.agent_console.update(cx, |view, cx| {
            view.set_session_context_locked(session_id, true, cx)
        });
        self.active_view = ActiveView::Workspaces;
        self.on_active_view_changed(cx);
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

fn remote_agent_file_entries(
    entries: Vec<shelldeck_core::models::server_sync::FileEntry>,
) -> Vec<AgentRemoteFileEntry> {
    entries
        .into_iter()
        .map(|entry| AgentRemoteFileEntry {
            path: entry.path,
            name: entry.name,
            is_dir: entry.is_dir,
        })
        .collect()
}

fn authorized_local_agent_file(
    sessions: &[shelldeck_core::agent_session::AgentSession],
    session_id: Uuid,
    requested: &std::path::Path,
) -> Option<std::path::PathBuf> {
    let session = sessions.iter().find(|session| {
        session.id == session_id && matches!(session.context.target, AgentTarget::Local)
    })?;
    let root = std::fs::canonicalize(&session.context.workdir).ok()?;
    let requested = std::fs::canonicalize(requested).ok()?;
    (requested.is_file() && requested.starts_with(root)).then_some(requested)
}

fn parse_remote_home(stdout: &[u8]) -> Result<String, String> {
    let home = std::str::from_utf8(stdout)
        .map_err(|_| "remote home directory is not UTF-8".to_string())?;
    let home = home.strip_suffix('\n').unwrap_or(home);
    let home = home.strip_suffix('\r').unwrap_or(home);
    if home.is_empty()
        || home.len() > shelldeck_core::agent_runtime::MAX_AGENT_WORKDIR_BYTES
        || home.contains(['\n', '\r'])
        || RemotePosixPath::new(home.to_string()).is_err()
    {
        return Err("remote home directory is invalid".to_string());
    }
    Ok(home.to_string())
}

fn resolve_local_agent_program(program: &str) -> Option<String> {
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
    provider: shelldeck_core::agent_runtime::AgentProvider,
    parse_json: bool,
    event_tx: &std::sync::mpsc::Sender<AgentStreamEvent>,
) {
    for frame in frames {
        match frame {
            AgentStreamFrame::Line(line) if parse_json => {
                for event in parse_stream_line(provider, &line) {
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
    let mut chunk = [0_u8; 8 * 1024];
    loop {
        match reader.read(&mut chunk) {
            Ok(0) => break,
            Ok(read) => {
                send_stream_frames(framer.push(&chunk[..read]), provider, parse_json, event_tx)
            }
            Err(_) => break,
        }
    }
    send_stream_frames(framer.finish(), provider, parse_json, event_tx);
}

fn spawn_local_automonique_agent(
    run_id: Uuid,
    request: AgentRunRequest,
    program: String,
    cancel_rx: shelldeck_core::acp::AcpCancelReceiver,
    event_tx: std::sync::mpsc::Sender<AgentStreamEvent>,
    done_tx: std::sync::mpsc::Sender<AgentDone>,
) -> std::io::Result<std::thread::JoinHandle<()>> {
    std::thread::Builder::new()
        .name(format!("agent-acp-{run_id}"))
        .spawn(move || {
            use std::sync::Arc;

            let permissions = Arc::new(AcpAgentPermissionBroker::new(request.access));
            let mut turn =
                shelldeck_core::acp::AcpTurnRequest::text(&request.workdir, request.prompt);
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
    run_id: Uuid,
    provider: shelldeck_core::agent_runtime::AgentProvider,
    spec: AgentCommandSpec,
    connection: Connection,
    shutdown_rx: tokio::sync::mpsc::Receiver<()>,
    event_tx: std::sync::mpsc::Sender<AgentStreamEvent>,
    done_tx: std::sync::mpsc::Sender<AgentDone>,
) -> std::io::Result<std::thread::JoinHandle<()>> {
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
                let session = match tokio::select! {
                    _ = shutdown_rx.recv() => {
                        let _ = done_tx.send((None, None));
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
                    while let Some(bytes) = output_rx.recv().await {
                        send_stream_frames(framer.push(&bytes), provider, true, &event_tx);
                    }
                    send_stream_frames(framer.finish(), provider, true, &event_tx);
                });
                let result = session
                    .exec_cancellable(&spec.remote_shell_command(), output_tx, shutdown_rx)
                    .await;
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
        agent_run_has_capacity, authorized_local_agent_file, automonique_agent_admission,
        forward_local_stream, mapped_remote_agent_workdir, parse_remote_home, spawn_local_agent,
        AgentCommandSpec, AgentStreamEvent, AgentStreamFrame, AgentStreamFramer,
        AGENT_STREAM_OVERSIZED_LABEL, DEFAULT_MAX_CONCURRENT_AGENT_SESSIONS,
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

    // SDTEST-1908 — SDUC-475
    #[test]
    fn sdtest_1908_remote_home_probe_accepts_one_absolute_path_only() {
        assert_eq!(
            parse_remote_home(b"/home/remote user\n").unwrap(),
            "/home/remote user"
        );
        assert_eq!(parse_remote_home(b"/root\r\n").unwrap(), "/root");
        assert!(parse_remote_home(b"relative/home\n").is_err());
        assert!(parse_remote_home(b"/home/first\n/home/second\n").is_err());
        assert!(parse_remote_home(b"/home/remote\0escape\n").is_err());
        assert!(parse_remote_home(&[0xff, b'\n']).is_err());
    }

    // SDTEST-1909 — SDUC-499
    #[test]
    fn sdtest_1909_agent_file_open_stays_inside_the_selected_local_session_root() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let admitted = root.path().join("src.rs");
        let refused = outside.path().join("secret.txt");
        std::fs::write(&admitted, "safe").unwrap();
        std::fs::write(&refused, "outside").unwrap();
        let context = shelldeck_core::agent_session::AgentExecutionContext {
            provider: AgentProvider::Claude,
            target: AgentTarget::Local,
            access: AgentAccessMode::ReadOnly,
            workdir: root.path().display().to_string(),
            model: None,
        };
        let session = shelldeck_core::agent_session::AgentSession::new("local", context, 1)
            .expect("valid local session");
        assert_eq!(
            authorized_local_agent_file(std::slice::from_ref(&session), session.id, &admitted),
            Some(std::fs::canonicalize(&admitted).unwrap())
        );
        assert!(
            authorized_local_agent_file(std::slice::from_ref(&session), session.id, &refused)
                .is_none()
        );
        assert!(authorized_local_agent_file(
            std::slice::from_ref(&session),
            Uuid::new_v4(),
            &admitted
        )
        .is_none());

        #[cfg(unix)]
        {
            let escape = root.path().join("escape.txt");
            std::os::unix::fs::symlink(&refused, &escape).unwrap();
            assert!(authorized_local_agent_file(
                std::slice::from_ref(&session),
                session.id,
                &escape
            )
            .is_none());
        }
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
