use super::*;
use shelldeck_core::agent_runtime::AgentTarget;
use shelldeck_core::agent_session::AgentExecutionContext;

impl Workspace {
    pub(super) fn complete_agent_run(
        &mut self,
        run_id: Uuid,
        exit_code: Option<i32>,
        error: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let session = self
            .agent_console
            .read(cx)
            .session_id_for_run(run_id)
            .and_then(|id| {
                self.agent_console
                    .read(cx)
                    .sessions()
                    .iter()
                    .find(|s| s.id == id)
                    .cloned()
            });
        if let Some(run) = self.active_agent_runs.get(&run_id) {
            if let Some(broker) = &run.approval_broker {
                broker.cancel_pending();
            }
        }
        let Some(session) = session else {
            self.active_agent_runs.remove(&run_id);
            return;
        };
        if shelldeck_core::agent_review::requires_turn_snapshot(
            &session.context.target,
            session.context.access,
        ) {
            let root = std::path::PathBuf::from(&session.context.workdir);
            let task = cx.background_executor().spawn(async move {
                let started = std::time::Instant::now();
                let result = shelldeck_core::agent_review::AgentReviewBackend::default()
                    .observe(&root)
                    .map_err(|e| e.to_string());
                tracing::debug!(
                    elapsed_ms = started.elapsed().as_millis() as u64,
                    "agent post-run repository snapshot finished"
                );
                result
            });
            cx.spawn(async move |this, cx| {
                let after = task.await;
                let _ = this.update(cx, |this, cx| {
                    if this.agent_console.read(cx).session_id_for_run(run_id) != Some(session.id) {
                        return;
                    }
                    this.agent_console.update(cx, |view, cx| {
                        view.set_review_after_run(session.id, after, cx)
                    });
                    this.finish_agent_run_ui(run_id, exit_code, error, cx);
                });
            })
            .detach();
        } else {
            self.finish_agent_run_ui(run_id, exit_code, error, cx);
        }
    }

    fn finish_agent_run_ui(
        &mut self,
        run_id: Uuid,
        exit_code: Option<i32>,
        error: Option<String>,
        cx: &mut Context<Self>,
    ) {
        if self
            .agent_console
            .read(cx)
            .session_id_for_run(run_id)
            .is_none()
        {
            self.active_agent_runs.remove(&run_id);
            return;
        }
        self.active_agent_runs.remove(&run_id);
        self.agent_console.update(cx, |view, cx| {
            view.finish_run(run_id, exit_code, error.clone(), cx)
        });
        let success = exit_code == Some(0) && error.is_none();
        let stopped = exit_code.is_none() && error.is_none();
        let message = if let Some(error) = error {
            t!("agents.toast.failed", error = error).to_string()
        } else if success {
            t!("agents.toast.completed").to_string()
        } else if stopped {
            t!("agents.toast.stopped").to_string()
        } else {
            t!(
                "agents.toast.exit",
                code = exit_code.unwrap_or(-1).to_string()
            )
            .to_string()
        };
        self.show_toast(
            message,
            if success {
                crate::toast::ToastLevel::Success
            } else if stopped {
                crate::toast::ToastLevel::Info
            } else {
                crate::toast::ToastLevel::Error
            },
            cx,
        );
        if self.app_config.general.show_notifications {
            self.emit_tray_notification(TrayNotification::AiTaskDone { success });
        }
    }

    pub(super) fn prepare_agent_run(
        &mut self,
        request: shelldeck_core::agent_runtime::AgentRunRequest,
        cx: &mut Context<Self>,
    ) {
        if !shelldeck_core::agent_review::requires_turn_snapshot(&request.target, request.access) {
            self.start_agent_run(request, cx);
            return;
        }
        let Some(session_id) = self.agent_console.read(cx).session_id_for_run(request.id) else {
            return;
        };
        let root = std::path::PathBuf::from(&request.workdir);
        let task = cx.background_executor().spawn(async move {
            let started = std::time::Instant::now();
            let result = shelldeck_core::agent_review::AgentReviewBackend::default()
                .observe(&root)
                .map_err(|e| e.to_string());
            tracing::debug!(
                elapsed_ms = started.elapsed().as_millis() as u64,
                "agent pre-run repository snapshot finished"
            );
            result
        });
        cx.spawn(async move |this, cx| {
            let snapshot = task.await;
            let _ = this.update(cx, |this, cx| {
                let can_start = this.agent_console.read(cx).is_run_starting(request.id);
                if !can_start {
                    return;
                }
                this.agent_console.update(cx, |view, cx| {
                    view.set_review_before_run(session_id, snapshot, cx)
                });
                this.start_agent_run(request, cx);
            });
        })
        .detach();
    }

    pub(super) fn open_agent_terminal(
        &mut self,
        context: AgentExecutionContext,
        cx: &mut Context<Self>,
    ) {
        match context.target {
            AgentTarget::Local => {
                let (rows, cols) = self.terminal.read(cx).grid_size();
                let shell = self.app_config.terminal.default_shell.clone();
                let result = shelldeck_terminal::session::TerminalSession::spawn_local_at(
                    shell.as_deref().filter(|s| !s.is_empty()),
                    rows.max(1),
                    cols.max(1),
                    std::path::Path::new(&context.workdir),
                );
                match result {
                    Ok(session) => self.terminal.update(cx, |terminal, cx| {
                        terminal.add_session(session);
                        terminal.ensure_refresh_running(cx);
                        cx.notify();
                    }),
                    Err(error) => {
                        self.show_toast(error.to_string(), crate::toast::ToastLevel::Error, cx);
                        return;
                    }
                }
            }
            AgentTarget::Ssh { connection_id, .. } => {
                let Some(connection) = self
                    .connections
                    .iter()
                    .find(|c| c.id == connection_id)
                    .cloned()
                else {
                    return;
                };
                if context.workdir != "~"
                    && shelldeck_core::config::workspace_catalog::RemotePosixPath::new(
                        context.workdir.clone(),
                    )
                    .is_err()
                {
                    return;
                }
                let previous = self.terminal.read(cx).active_session().map(|s| s.id);
                self.connect_ssh_at(connection, Some(context.workdir), cx);
                let terminal = self.terminal.read(cx);
                let current = terminal.active_session().map(|s| s.id);
                if current.is_none()
                    || current == previous
                    || !terminal.tabs.iter().any(|tab| {
                        Some(tab.id) == current && tab.connection_id == Some(connection_id)
                    })
                {
                    return;
                }
            }
        }
        let terminal = self.terminal.clone();
        self.agent_console
            .update(cx, |view, cx| view.show_agent_terminal(terminal, cx));
    }

    pub(super) fn open_agent_workspace_launcher(&mut self, cx: &mut Context<Self>) {
        self.workspace_hub
            .update(cx, |hub, cx| hub.show_launcher(cx));
        self.activate_dev_section(SidebarSection::Workspaces, cx);
    }

    pub(super) fn open_agent_project_setup(&mut self, cx: &mut Context<Self>) {
        self.workspace_hub
            .update(cx, |hub, cx| hub.show_agent_project_setup(cx));
        self.activate_dev_section(SidebarSection::Workspaces, cx);
    }

    pub(super) fn open_remote_agent_file(
        &mut self,
        session_id: Uuid,
        request_id: Uuid,
        connection_id: Uuid,
        captured_context: AgentExecutionContext,
        path: String,
        cx: &mut Context<Self>,
    ) {
        let context = self.agent_console.read(cx).selected_execution_context(cx)
            .filter(|(id, context)| *id == session_id && *context == captured_context && matches!(context.target, AgentTarget::Ssh { connection_id: selected, .. } if selected == connection_id))
            .map(|(_, context)| context);
        let connection = self
            .connections
            .iter()
            .find(|c| c.id == connection_id)
            .cloned();
        let task = cx.background_executor().spawn(async move {
            let context = context.ok_or_else(|| t!("agents.editor.context_changed").to_string())?;
            let connection =
                connection.ok_or_else(|| t!("agents.error.target_missing").to_string())?;
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|e| e.to_string())?;
            runtime.block_on(async {
                tokio::time::timeout(std::time::Duration::from_secs(30), async {
                    let session = shelldeck_ssh::client::SshClient::new()
                        .connect(&connection)
                        .await
                        .map_err(|e| e.to_string())?;
                    shelldeck_ssh::agent_files::read(&session, &context.workdir, &path)
                        .await
                        .map_err(agent_file_error)
                })
                .await
                .map_err(|_| t!("agents.editor.timeout").to_string())?
            })
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.agent_console.update(cx, |view, cx| {
                    view.set_remote_file_document(session_id, request_id, result, cx)
                });
            });
        })
        .detach();
    }

    pub(super) fn save_agent_file(
        &mut self,
        request: crate::agent_console_view::AgentFileSaveRequest,
        cx: &mut Context<Self>,
    ) {
        use crate::agent_console_view::AgentFileOrigin;
        let matches = self
            .agent_console
            .read(cx)
            .selected_execution_context(cx)
            .is_some_and(|(id, context)| id == request.session_id && context == request.context);
        if !matches {
            self.agent_console.update(cx, |view, cx| {
                view.complete_agent_file_save(
                    &request,
                    Err(t!("agents.editor.context_changed").to_string()),
                    cx,
                )
            });
            return;
        }
        let connection = match &request.origin {
            AgentFileOrigin::Remote { connection_id, .. } => self
                .connections
                .iter()
                .find(|c| c.id == *connection_id)
                .cloned(),
            _ => None,
        };
        let operation = request.clone();
        let task = cx.background_executor().spawn(async move {
            match operation.origin {
                origin @ AgentFileOrigin::Local {..} => crate::agent_console_view::save_local_document(&operation.context.workdir, origin, operation.text),
                AgentFileOrigin::Remote {root, path, revision, connection_id} => {
                    if !matches!(operation.context.target, AgentTarget::Ssh {connection_id: id, ..} if id == connection_id) { return Err(t!("agents.editor.context_changed").to_string()); }
                    let connection = connection.ok_or_else(|| t!("agents.error.target_missing").to_string())?;
                    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().map_err(|e| e.to_string())?;
                    runtime.block_on(async {
                        tokio::time::timeout(std::time::Duration::from_secs(30), async {
                            let session = shelldeck_ssh::client::SshClient::new().connect(&connection).await.map_err(|e| e.to_string())?;
                            let document = shelldeck_ssh::agent_files::save(&session, &root, &path, &revision, &operation.text).await.map_err(agent_file_error)?;
                            Ok(AgentFileOrigin::Remote {root: document.root, path: document.path, revision: document.revision, connection_id})
                        }).await.map_err(|_| t!("agents.editor.timeout").to_string())?
                    })
                }
            }
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.agent_console.update(cx, |view, cx| {
                    view.complete_agent_file_save(&request, result, cx)
                });
            });
        })
        .detach();
    }
}

pub(super) fn agent_file_error(error: shelldeck_ssh::agent_files::AgentFileError) -> String {
    if error.is_conflict() {
        return t!("agents.editor.conflict").to_string();
    }
    match error {
        shelldeck_ssh::agent_files::AgentFileError::UnsupportedHost => {
            t!("agents.editor.remote_requirements").to_string()
        }
        error => error.to_string(),
    }
}

struct SshReadinessExecutor<'a> {
    runtime: &'a tokio::runtime::Runtime,
    session: &'a shelldeck_ssh::session::SshSession,
}
impl shelldeck_core::agent_workflows::ReadinessExecutor for SshReadinessExecutor<'_> {
    fn execute(
        &self,
        spec: &shelldeck_core::agent_workflows::ReadinessCommand,
        timeout: std::time::Duration,
    ) -> Result<shelldeck_core::agent_workflows::ReadinessOutput, String> {
        let allowed = matches!(
            spec.program.as_str(),
            "claude" | "codex" | "jcode" | "automonique"
        ) && matches!(
            spec.args
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>()
                .as_slice(),
            ["--version"] | ["auth", "status"] | ["login", "status"]
        );
        if !allowed {
            return Err("Readiness command is not allowed".to_string());
        }
        let command = std::iter::once(spec.program.as_str())
            .chain(spec.args.iter().map(String::as_str))
            .map(shelldeck_core::util::shell_escape)
            .collect::<Vec<_>>()
            .join(" ");
        let command=format!("({command} 2>&1; status=$?; printf '\\nSD_AGENT_READY:%s\\n' \"$status\") | head -c 16385");
        self.runtime.block_on(async {
            let output = tokio::time::timeout(timeout, self.session.exec(&command))
                .await
                .map_err(|_| "Remote readiness check timed out".to_string())?
                .map_err(|_| "Remote readiness check failed".to_string())?;
            if output.stdout.len() > 16384 {
                return Err("Remote readiness output exceeded its limit".to_string());
            }
            let text = String::from_utf8_lossy(&output.stdout);
            let (body, status) = text
                .rsplit_once("\nSD_AGENT_READY:")
                .ok_or("Remote readiness result was incomplete".to_string())?;
            let code = status
                .trim()
                .parse::<u32>()
                .map_err(|_| "Invalid remote readiness result".to_string())?;
            Ok(shelldeck_core::agent_workflows::ReadinessOutput {
                success: code == 0,
                output: body.to_string(),
            })
        })
    }
}
impl Workspace {
    pub(super) fn check_remote_agent_readiness(
        &mut self,
        check_id: Uuid,
        context: shelldeck_core::agent_session::AgentExecutionContext,
        cx: &mut Context<Self>,
    ) {
        let connection =
            if let shelldeck_core::agent_runtime::AgentTarget::Ssh { connection_id, .. } =
                &context.target
            {
                self.connections
                    .iter()
                    .find(|c| c.id == *connection_id)
                    .cloned()
            } else {
                None
            };
        let Some(connection) = connection else {
            self.agent_console.update(cx, |view, cx| {
                view.set_agent_readiness_result(
                    check_id,
                    Err(t!("agents.error.target_missing").to_string()),
                    cx,
                )
            });
            return;
        };
        let task = cx.background_executor().spawn(async move {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|_| "Readiness worker unavailable".to_string())?;
            let session = runtime.block_on(async {
                tokio::time::timeout(
                    std::time::Duration::from_secs(10),
                    shelldeck_ssh::client::SshClient::new().connect(&connection),
                )
                .await
                .map_err(|_| "SSH connection timed out".to_string())?
                .map_err(|_| "SSH connection failed".to_string())
            })?;
            shelldeck_core::agent_workflows::check_provider_readiness(
                &context,
                &SshReadinessExecutor {
                    runtime: &runtime,
                    session: &session,
                },
            )
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.agent_console.update(cx, |view, cx| {
                    view.set_agent_readiness_result(check_id, result, cx)
                });
            });
        })
        .detach();
    }
}
