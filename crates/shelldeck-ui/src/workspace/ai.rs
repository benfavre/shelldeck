use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IssueTargetResolution<'a> {
    Found(&'a str),
    Ambiguous,
    NotFound,
}

fn resolve_issue_target<'a>(
    issues: &'a [Issue],
    issue_id: Option<&str>,
    query: &str,
) -> IssueTargetResolution<'a> {
    if let Some(issue_id) = issue_id.map(str::trim).filter(|id| !id.is_empty()) {
        if let Some(issue) = issues
            .iter()
            .find(|issue| issue.id.eq_ignore_ascii_case(issue_id))
        {
            return IssueTargetResolution::Found(&issue.id);
        }
    }

    let query = query.trim();
    if query.is_empty() {
        return IssueTargetResolution::NotFound;
    }
    if let Some(issue) = issues
        .iter()
        .find(|issue| issue.id.eq_ignore_ascii_case(query))
    {
        return IssueTargetResolution::Found(&issue.id);
    }

    let exact_titles = issues
        .iter()
        .filter(|issue| issue.title.trim().eq_ignore_ascii_case(query))
        .collect::<Vec<_>>();
    match exact_titles.as_slice() {
        [issue] => return IssueTargetResolution::Found(&issue.id),
        [_, ..] => return IssueTargetResolution::Ambiguous,
        [] => {}
    }

    let query = query.to_lowercase();
    let partial = issues
        .iter()
        .filter(|issue| {
            issue.title.to_lowercase().contains(&query) || issue.id.to_lowercase().contains(&query)
        })
        .collect::<Vec<_>>();
    match partial.as_slice() {
        [issue] => IssueTargetResolution::Found(&issue.id),
        [_, ..] => IssueTargetResolution::Ambiguous,
        [] => IssueTargetResolution::NotFound,
    }
}

impl Workspace {
    pub(super) fn update_ai_api_key(
        &mut self,
        backend: shelldeck_core::ai::AiBackend,
        value: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let Some(provider) = backend.provider_key() else {
            return;
        };
        self.ai_sheet = None;
        self.refresh_command_palette(cx);
        let provider = provider.to_string();
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let deleting = value.is_none();
            let result = cx
                .background_executor()
                .spawn(async move {
                    match value {
                        None => shelldeck_core::config::keychain::delete_ai_api_key(&provider),
                        Some(value) => shelldeck_core::config::keychain::store_ai_api_key(
                            &provider,
                            value.trim(),
                        ),
                    }
                })
                .await;
            let _ = this.update(cx, |ws, cx| match result {
                Ok(()) => {
                    ws.settings.update(cx, |settings, cx| {
                        settings.reset_ai_connection_state(cx);
                    });
                    ws.show_toast(
                        if deleting {
                            t!("toast.ai.key_deleted").to_string()
                        } else {
                            t!("toast.ai.key_saved").to_string()
                        },
                        ToastLevel::Info,
                        cx,
                    );
                }
                Err(error) => {
                    let message = t!("toast.ai.key_failed", error = error.to_string()).to_string();
                    ws.settings.update(cx, |settings, cx| {
                        settings.set_ai_connection_result(Err(message.clone()), cx);
                    });
                    ws.show_toast(message, ToastLevel::Error, cx);
                }
            });
        })
        .detach();
    }

    pub(super) fn test_ai_connection(&mut self, config: AppConfig, cx: &mut Context<Self>) {
        let ai_config = config.ai;
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let tested_config = ai_config.clone();
            let result = cx
                .background_executor()
                .spawn(async move { test_connection(&tested_config) })
                .await
                .map(|_| ())
                .map_err(|error| error.to_string());

            let _ = this.update(cx, |workspace, cx| {
                if workspace.app_config.ai != ai_config {
                    workspace.settings.update(cx, |settings, cx| {
                        settings.reset_ai_connection_state(cx);
                    });
                    return;
                }
                workspace.settings.update(cx, |settings, cx| {
                    settings.set_ai_connection_result(result.clone(), cx);
                });
                workspace.refresh_command_palette(cx);
                workspace.show_toast(
                    match &result {
                        Ok(()) => t!("toast.ai.test_ok").to_string(),
                        Err(error) => t!("toast.ai.test_failed", error = error.clone()).to_string(),
                    },
                    if result.is_ok() {
                        ToastLevel::Success
                    } else {
                        ToastLevel::Error
                    },
                    cx,
                );
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn open_ai_assistant(&mut self, cx: &mut Context<Self>) {
        let context = self.current_ai_context(cx);
        if !self.ai_backend_available() || !self.app_config.ai.allows(context.surface) {
            return;
        }
        self.open_ai_assistant_with_context(context, cx);
    }

    pub fn companion_ui_font_family(&self) -> Option<String> {
        self.resolved_ui_font_family.clone()
    }

    /// Keep a single conversation editor active when the standalone Dock is
    /// opened alongside an already-initialized main workspace.
    pub fn close_ai_sheet_for_companion(&mut self, cx: &mut Context<Self>) {
        self.ai_sheet = None;
        cx.notify();
    }

    pub(super) fn current_ai_context(&self, cx: &App) -> AiContext {
        if self.effective_mode() == AppMode::Support {
            let support = self.support.read(cx);
            return AiContext::new(
                support.ai_surface(),
                t!("ai.context.support").to_string(),
                self.ai_context_data_with_hosts(support.ai_context_data()),
            );
        }
        if self.effective_mode() == AppMode::User && self.user_home_tab == UserHomeTab::Requests {
            return AiContext::new(
                AiSurface::Issue,
                t!("ai.context.issue").to_string(),
                self.ai_context_data_with_hosts(
                    serde_json::to_value(&self.issue_detail)
                        .unwrap_or_else(|_| serde_json::json!({ "issue": null })),
                ),
            );
        }
        match self.active_view {
            ActiveView::Terminal => AiContext::new(
                AiSurface::Terminal,
                t!("ai.context.terminal").to_string(),
                self.ai_context_data_with_hosts(self.terminal.read(cx).ai_context_data()),
            ),
            ActiveView::Scripts => AiContext::new(
                AiSurface::Script,
                t!("ai.context.script").to_string(),
                self.script_ai_context_data(cx),
            ),
            ActiveView::JeanConsole | ActiveView::Fleet => AiContext::new(
                AiSurface::Jean,
                t!("ai.context.jean").to_string(),
                self.ai_context_data_with_hosts(
                    serde_json::to_value(&self.jean_state)
                        .unwrap_or_else(|_| serde_json::json!({ "jean": null })),
                ),
            ),
            ActiveView::Recent | ActiveView::Dashboard => AiContext::new(
                AiSurface::Recent,
                t!("ai.context.recent").to_string(),
                self.ai_context_data_with_hosts(
                    serde_json::to_value(self.recent_activity.iter().take(50).collect::<Vec<_>>())
                        .unwrap_or_else(|_| serde_json::json!([])),
                ),
            ),
            ActiveView::Sites | ActiveView::PortForwards => AiContext::new(
                AiSurface::Naming,
                t!("ai.context.naming").to_string(),
                serde_json::json!({
                    "connections": self.ai_hosts_context_data(),
                    "tunnels": self.store.port_forwards.iter().map(|forward| serde_json::json!({
                        "label": forward.label,
                        "direction": format!("{:?}", forward.direction),
                        "local": format!("{}:{}", forward.local_host, forward.local_port),
                        "remote": format!("{}:{}", forward.remote_host, forward.remote_port),
                    })).collect::<Vec<_>>(),
                }),
            ),
            _ => AiContext::new(
                AiSurface::Global,
                t!("ai.context.global").to_string(),
                self.ai_context_data_with_hosts(serde_json::json!({
                    "active_view": format!("{:?}", self.active_view),
                    "active_site": self.app_config.cloud_sync.active_site_label,
                    "connections": self.connections.len(),
                    "active_tunnels": self.active_tunnels.len(),
                    "active_scripts": self.active_scripts.len(),
                })),
            ),
        }
    }

    pub(super) fn ai_hosts_context_data(&self) -> serde_json::Value {
        host_context(&self.connections)
    }

    pub(super) fn ai_context_data_with_hosts(&self, data: serde_json::Value) -> serde_json::Value {
        serde_json::json!({
            "screen": data,
            "hosts": self.ai_hosts_context_data(),
        })
    }

    pub(super) fn script_ai_context_data(&self, cx: &App) -> serde_json::Value {
        serde_json::json!({
            "script": self.scripts.read(cx).ai_context_data(),
            "hosts": self.ai_hosts_context_data(),
        })
    }

    pub(super) fn ai_backend_available(&self) -> bool {
        self.app_config.ai.is_configured()
            && (!self.app_config.ai.backend.is_cli()
                || configured_cli_available(&self.app_config.ai))
    }

    pub(super) fn ai_available_for_current_surface(&self, cx: &App) -> bool {
        self.signed_in()
            && self.ai_backend_available()
            && self
                .app_config
                .ai
                .allows(self.current_ai_context(cx).surface)
    }

    pub(super) fn sync_ai_affordances(&mut self, cx: &mut Context<Self>) {
        let backend_ready = self.ai_backend_available();
        self.support.update(cx, |view, cx| {
            view.set_ai_reply_enabled(
                backend_ready && self.app_config.ai.allows(AiSurface::Support),
                cx,
            );
            view.set_ai_issue_enabled(
                backend_ready && self.app_config.ai.allows(AiSurface::Issue),
                cx,
            );
        });
        self.scripts.update(cx, |view, cx| {
            view.set_ai_generation_enabled(
                backend_ready && self.app_config.ai.allows(AiSurface::Script),
                cx,
            );
        });
        if let Some(form) = self.script_form.as_ref() {
            form.update(cx, |form, cx| {
                form.set_ai_enabled(
                    backend_ready && self.app_config.ai.allows(AiSurface::Script),
                    cx,
                );
                form.set_ai_naming_enabled(
                    backend_ready && self.app_config.ai.allows(AiSurface::Naming),
                    cx,
                );
            });
        }
        self.terminal.update(cx, |view, cx| {
            view.set_ai_actions_enabled(
                backend_ready && self.app_config.ai.allows(AiSurface::Terminal),
                cx,
            );
            view.set_ai_naming_enabled(
                backend_ready && self.app_config.ai.allows(AiSurface::Naming),
                cx,
            );
        });
        self.recent.update(cx, |view, cx| {
            view.set_ai_enabled(backend_ready && self.app_config.ai.allows(AiSurface::Recent));
            cx.notify();
        });
        // The mention directory depends on the mode and the active site, both
        // of which move with the config this method re-reads.
        self.refresh_mention_directory(cx);
    }

    pub(super) fn close_ai_workflow(&mut self, cx: &mut Context<Self>) {
        self.ai_workflow_sheet = None;
        self.ai_workflow = None;
        self._ai_workflow_sub = None;
        cx.notify();
    }

    pub(super) fn sync_ai_tasks(&mut self, cx: &mut Context<Self>) {
        if let Err(error) = AiTaskStore::save(&self.ai_tasks) {
            tracing::warn!("Failed to save AI tasks: {error}");
        }
        let tasks = self.ai_tasks.clone();
        self.ai_assistant
            .update(cx, |view, cx| view.set_tasks(tasks.clone(), cx));
        self.ai_dock_assistant
            .update(cx, |view, cx| view.set_tasks(tasks, cx));
        self.publish_tray_state(cx);
        cx.notify();
    }

    pub(super) fn begin_ai_workflow_task(
        &mut self,
        target: &AiWorkflowTarget,
        instructions: String,
        cx: &mut Context<Self>,
    ) -> Uuid {
        let context_title = self.ai_workflow_context(target, cx).title;
        let model = if self.app_config.ai.model.trim().is_empty() {
            self.app_config.ai.backend.default_model().to_string()
        } else {
            self.app_config.ai.model.clone()
        };
        if let Some(task) = self.ai_tasks.iter_mut().rev().find(|task| {
            task.capability == target.capability()
                && task.target_id == target.target_id()
                && (!task.status.is_finished() || task.status == AiTaskStatus::Failed)
        }) {
            task.instructions = instructions;
            task.result.clear();
            task.target_kind = Some(target.storage_kind().to_string());
            task.target_label = context_title;
            task.model = model;
            task.set_status(AiTaskStatus::Generating, None);
            let id = task.id;
            self.sync_ai_tasks(cx);
            return id;
        }

        let mut task = AiTask::new(
            target.capability(),
            target.surface(),
            target.target_id(),
            self.app_config.ai.backend,
            instructions,
            String::new(),
        );
        task.target_kind = Some(target.storage_kind().to_string());
        task.target_label = context_title;
        task.model = model;
        task.set_status(AiTaskStatus::Generating, None);
        let id = task.id;
        self.ai_tasks.push(task);
        self.sync_ai_tasks(cx);
        id
    }

    pub(super) fn finish_ai_workflow_task(
        &mut self,
        task_id: Uuid,
        result: &Result<String, String>,
        cx: &mut Context<Self>,
    ) {
        let completed_in_background = self.ai_workflow.is_none();
        if let Some(task) = self.ai_tasks.iter_mut().find(|task| task.id == task_id) {
            match result {
                Ok(output) => {
                    task.result = output.clone();
                    task.set_status(AiTaskStatus::Ready, None);
                }
                Err(error) => task.set_status(AiTaskStatus::Failed, Some(error.clone())),
            }
            self.sync_ai_tasks(cx);
            if completed_in_background {
                self.show_toast(
                    if result.is_ok() {
                        t!("toast.ai.task_ready").to_string()
                    } else {
                        t!("toast.ai.task_failed").to_string()
                    },
                    if result.is_ok() {
                        ToastLevel::Success
                    } else {
                        ToastLevel::Error
                    },
                    cx,
                );
                if !self.window_active && self.app_config.tray.notify_ai_tasks {
                    self.emit_tray_notification(TrayNotification::AiTaskDone {
                        success: result.is_ok(),
                    });
                }
            }
        }
    }

    pub(super) fn set_ai_workflow_task_status(
        &mut self,
        target: &AiWorkflowTarget,
        status: AiTaskStatus,
        cx: &mut Context<Self>,
    ) {
        if let Some(task) = self.ai_tasks.iter_mut().rev().find(|task| {
            task.capability == target.capability() && task.target_id == target.target_id()
        }) {
            task.set_status(status, None);
            self.sync_ai_tasks(cx);
        }
    }

    pub(super) fn set_ai_action_task_status(
        &mut self,
        plan: &AiActionPlan,
        status: AiTaskStatus,
        message: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let task_index = self.ai_tasks.iter().rposition(|task| {
            task.id == plan.id
                || (task.capability == plan.capability
                    && task.target_id == plan.target_id
                    && !task.status.is_finished())
        });
        if let Some(index) = task_index {
            let task = &mut self.ai_tasks[index];
            task.id = plan.id;
            task.target_label = plan.target_label.clone();
            task.model = plan.model.clone();
            task.set_status(status, message);
        } else {
            let mut task = AiTask::from_action(plan, status);
            task.status_message = message;
            self.ai_tasks.push(task);
        }
        self.sync_ai_tasks(cx);
    }

    /// Revalidated immediately before execution as well as at staging time —
    /// see `.agents/ai.md`. A plan staged in Dev must not survive a switch back
    /// to User.
    fn ai_action_allowed(&self, plan: &AiActionPlan) -> bool {
        super::ai_routing::ai_action_permitted(&self.ai_action_context(), &plan.payload)
    }

    pub(super) fn stage_ai_action(&mut self, mut plan: AiActionPlan, cx: &mut Context<Self>) {
        let level = self.app_config.ai.policies.level_for(plan.capability);
        plan.autonomy = level;
        let route = super::ai_routing::route_ai_action(
            &self.ai_action_context(),
            &plan.payload,
            level,
            plan.risk,
        );
        match route {
            super::ai_routing::AiActionRoute::Refused => {}
            super::ai_routing::AiActionRoute::Draft => {
                self.set_ai_action_task_status(&plan, AiTaskStatus::Ready, None, cx);
                self.show_toast(
                    t!("toast.ai.policy_preparation_only").to_string(),
                    ToastLevel::Info,
                    cx,
                );
            }
            super::ai_routing::AiActionRoute::AwaitConfirmation => {
                self.set_ai_action_task_status(&plan, AiTaskStatus::AwaitingConfirmation, None, cx);
                self.ai_action_confirmation = Some(plan);
                cx.notify();
            }
            // Still routed through `confirm_ai_action`, which revalidates the
            // target: the automatic path is a skipped dialog, not a shortcut
            // around the execution gate.
            super::ai_routing::AiActionRoute::ExecuteWithoutDialog => {
                self.set_ai_action_task_status(&plan, AiTaskStatus::AwaitingConfirmation, None, cx);
                self.ai_action_confirmation = Some(plan);
                self.confirm_ai_action(cx);
            }
        }
    }

    pub(super) fn open_ai_workflow(&mut self, target: AiWorkflowTarget, cx: &mut Context<Self>) {
        self.open_ai_workflow_with_instructions(target, None, false, cx);
    }

    fn open_ai_workflow_with_instructions(
        &mut self,
        target: AiWorkflowTarget,
        initial_instructions: Option<String>,
        generate_immediately: bool,
        cx: &mut Context<Self>,
    ) {
        let surface = target.surface();
        let role_allowed = match &target {
            AiWorkflowTarget::SupportReply { .. }
            | AiWorkflowTarget::SupportSummary { .. }
            | AiWorkflowTarget::SupportTriage { .. }
            | AiWorkflowTarget::IssueTriage { .. } => self.can_access_mode(AppMode::Support),
            AiWorkflowTarget::EntityNaming { kind, .. } => {
                matches!(kind, AiNamingKind::Issue) || self.can_access_mode(AppMode::Dev)
            }
            AiWorkflowTarget::ScriptGenerate { .. }
            | AiWorkflowTarget::ScriptExplain { .. }
            | AiWorkflowTarget::ScriptReview { .. }
            | AiWorkflowTarget::ScriptFix { .. }
            | AiWorkflowTarget::TerminalCommand { .. }
            | AiWorkflowTarget::TerminalDiagnose { .. } => self.can_access_mode(AppMode::Dev),
            AiWorkflowTarget::IssueReply { .. } | AiWorkflowTarget::IssueSummary { .. } => true,
        };
        if !self.signed_in()
            || !role_allowed
            || !self.ai_backend_available()
            || !self.app_config.ai.allows(surface)
        {
            return;
        }
        let pending = if initial_instructions.is_some() {
            None
        } else {
            self.ai_tasks
                .iter()
                .rev()
                .find(|draft| {
                    draft.capability == target.capability()
                        && draft.target_id == target.target_id()
                        && matches!(draft.status, AiTaskStatus::Ready | AiTaskStatus::Pending)
                })
                .cloned()
        };
        let should_generate = generate_immediately
            || (matches!(
                &target,
                AiWorkflowTarget::EntityNaming { .. }
                    | AiWorkflowTarget::SupportReply { .. }
                    | AiWorkflowTarget::SupportSummary { .. }
                    | AiWorkflowTarget::SupportTriage { .. }
                    | AiWorkflowTarget::IssueReply { .. }
                    | AiWorkflowTarget::IssueSummary { .. }
                    | AiWorkflowTarget::IssueTriage { .. }
                    | AiWorkflowTarget::ScriptExplain { .. }
                    | AiWorkflowTarget::ScriptReview { .. }
                    | AiWorkflowTarget::ScriptFix { .. }
                    | AiWorkflowTarget::TerminalDiagnose { .. }
            ) && pending.is_none());
        let title = match &target {
            AiWorkflowTarget::EntityNaming { .. } => t!("ai.naming.title").to_string(),
            AiWorkflowTarget::SupportReply { .. } => t!("ai.workflow.support_title").to_string(),
            AiWorkflowTarget::SupportSummary { .. } => {
                t!("ai.workflow.support_summary_title").to_string()
            }
            AiWorkflowTarget::SupportTriage { .. } => {
                t!("ai.workflow.support_triage_title").to_string()
            }
            AiWorkflowTarget::IssueReply { .. } => t!("ai.workflow.issue_reply_title").to_string(),
            AiWorkflowTarget::IssueSummary { .. } => {
                t!("ai.workflow.issue_summary_title").to_string()
            }
            AiWorkflowTarget::IssueTriage { .. } => {
                t!("ai.workflow.issue_triage_title").to_string()
            }
            AiWorkflowTarget::ScriptGenerate { .. } => t!("ai.workflow.script_title").to_string(),
            AiWorkflowTarget::ScriptExplain { .. } => {
                t!("ai.workflow.script_explain_title").to_string()
            }
            AiWorkflowTarget::ScriptReview { .. } => {
                t!("ai.workflow.script_review_title").to_string()
            }
            AiWorkflowTarget::ScriptFix { .. } => t!("ai.workflow.script_fix_title").to_string(),
            AiWorkflowTarget::TerminalCommand { .. } => {
                t!("ai.workflow.terminal_command_title").to_string()
            }
            AiWorkflowTarget::TerminalDiagnose { .. } => {
                t!("ai.workflow.terminal_diagnose_title").to_string()
            }
        };
        let comparison_original = match &target {
            AiWorkflowTarget::ScriptGenerate { script_id }
            | AiWorkflowTarget::ScriptFix { script_id } => Uuid::parse_str(script_id)
                .ok()
                .and_then(|id| self.scripts.read(cx).script_body(id)),
            _ => None,
        };
        let issue_triage_current = match &target {
            AiWorkflowTarget::SupportTriage { .. } => {
                self.support.read(cx).selected_ticket_triage_state()
            }
            AiWorkflowTarget::IssueTriage { issue_id } => self
                .issue_detail
                .as_ref()
                .filter(|issue| &issue.id == issue_id)
                .map(|issue| (issue.priority.clone(), issue.assignee.clone())),
            _ => None,
        };
        let action_policy = self.app_config.ai.policies.level_for(target.capability());
        let workflow = cx.new(|cx| {
            AiWorkflowView::new(
                AiWorkflowInit {
                    target,
                    backend: self.app_config.ai.backend,
                    model: self.app_config.ai.model.clone(),
                    pending,
                    initial_instructions,
                    comparison_original,
                    issue_triage_current,
                    action_policy,
                },
                cx,
            )
        });
        let subscription = cx.subscribe(&workflow, |this, _view, event: &AiWorkflowEvent, cx| {
            this.handle_ai_workflow_event(event.clone(), cx);
        });
        let sheet_workflow = workflow.clone();
        let workspace = cx.entity().downgrade();
        self.ai_workflow_sheet = Some(cx.new(move |sheet_cx| {
            Sheet::new(sheet_cx)
                .size(SheetSize::Assistant)
                .variant(SheetVariant::Assistant)
                .title(title)
                .description(t!("ai.workflow.description").to_string())
                .dynamic_content(move || sheet_workflow.clone())
                .on_close(move |_window, cx| {
                    if let Some(workspace) = workspace.upgrade() {
                        workspace.update(cx, |this, cx| this.close_ai_workflow(cx));
                    }
                })
        }));
        self.ai_workflow = Some(workflow.clone());
        self._ai_workflow_sub = Some(subscription);
        if should_generate {
            workflow.update(cx, |view, cx| view.generate(cx));
        }
        cx.notify();
    }

    pub(super) fn ai_workflow_context(&self, target: &AiWorkflowTarget, cx: &App) -> AiContext {
        match target {
            AiWorkflowTarget::EntityNaming { kind, .. } => {
                let entity = match kind {
                    AiNamingKind::Script => self
                        .script_form
                        .as_ref()
                        .map(|form| form.read(cx).ai_context_data(cx))
                        .unwrap_or_else(|| self.script_ai_context_data(cx)),
                    AiNamingKind::Terminal => self.terminal.read(cx).ai_context_data(),
                    AiNamingKind::Tunnel => self
                        .port_forward_form
                        .as_ref()
                        .map(|form| form.read(cx).ai_context_data(cx))
                        .unwrap_or_else(|| serde_json::json!({ "tunnel": null })),
                    AiNamingKind::Issue => serde_json::json!({
                        "request": {
                            "current_title": self.issue_title_state.read(cx).content().to_string(),
                            "description": self.issue_body_state.read(cx).content().to_string(),
                            "priority": self.issue_new_priority,
                        }
                    }),
                };
                AiContext::new(
                    AiSurface::Naming,
                    t!("ai.context.entity_naming").to_string(),
                    self.ai_context_data_with_hosts(entity),
                )
            }
            AiWorkflowTarget::SupportReply { .. } | AiWorkflowTarget::SupportSummary { .. } => {
                AiContext::new(
                    AiSurface::Support,
                    t!("ai.context.support").to_string(),
                    self.ai_context_data_with_hosts(self.support.read(cx).ai_context_data()),
                )
            }
            AiWorkflowTarget::SupportTriage { .. } => AiContext::new(
                AiSurface::Support,
                t!("ai.context.support").to_string(),
                self.ai_context_data_with_hosts(
                    self.support.read(cx).support_triage_context_data(),
                ),
            ),
            AiWorkflowTarget::IssueReply { .. } | AiWorkflowTarget::IssueSummary { .. } => {
                AiContext::new(
                    AiSurface::Issue,
                    t!("ai.context.issue").to_string(),
                    self.ai_context_data_with_hosts(self.support.read(cx).ai_context_data()),
                )
            }
            AiWorkflowTarget::IssueTriage { .. } => AiContext::new(
                AiSurface::Issue,
                t!("ai.context.issue").to_string(),
                self.ai_context_data_with_hosts(self.support.read(cx).issue_triage_context_data()),
            ),
            AiWorkflowTarget::ScriptGenerate { .. }
            | AiWorkflowTarget::ScriptExplain { .. }
            | AiWorkflowTarget::ScriptReview { .. } => AiContext::new(
                AiSurface::Script,
                t!("ai.context.script").to_string(),
                self.script_ai_context_data(cx),
            ),
            AiWorkflowTarget::ScriptFix { .. } => AiContext::new(
                AiSurface::Script,
                t!("ai.context.script_fix").to_string(),
                serde_json::json!({
                    "script": self.scripts.read(cx).ai_fix_context_data(),
                    "hosts": self.ai_hosts_context_data(),
                }),
            ),
            AiWorkflowTarget::TerminalCommand { .. }
            | AiWorkflowTarget::TerminalDiagnose { .. } => AiContext::new(
                AiSurface::Terminal,
                t!("ai.context.terminal").to_string(),
                self.ai_context_data_with_hosts(self.terminal.read(cx).ai_context_data()),
            ),
        }
    }

    pub(super) fn prepare_ai_action(
        &mut self,
        target: AiWorkflowTarget,
        result: String,
        cx: &mut Context<Self>,
    ) {
        let model = if self.app_config.ai.model.trim().is_empty() {
            self.app_config.ai.backend.default_model().to_string()
        } else {
            self.app_config.ai.model.clone()
        };
        let plan = match &target {
            AiWorkflowTarget::TerminalCommand { session_id } => {
                if crate::terminal_view::validate_ai_command(&result).is_err() {
                    self.show_toast(
                        t!("toast.ai.command_invalid").to_string(),
                        ToastLevel::Error,
                        cx,
                    );
                    return;
                }
                let context = self.terminal.read(cx).ai_context_data();
                if context
                    .get("session_id")
                    .and_then(serde_json::Value::as_str)
                    != Some(session_id.as_str())
                {
                    self.show_toast(
                        t!("toast.ai.action_target_changed").to_string(),
                        ToastLevel::Error,
                        cx,
                    );
                    return;
                }
                let label = context
                    .get("title")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("Terminal");
                AiActionPlan::new(AiActionPlanSpec {
                    capability: target.capability(),
                    kind: AiActionKind::TerminalCommand,
                    risk: AiActionRisk::High,
                    target_id: session_id.clone(),
                    target_label: label.to_string(),
                    backend: self.app_config.ai.backend,
                    model,
                    timeout_secs: 1_800,
                    payload: AiActionPayload::TerminalCommand { command: result },
                })
            }
            AiWorkflowTarget::ScriptGenerate { script_id }
            | AiWorkflowTarget::ScriptFix { script_id } => {
                let Some(script) = Uuid::parse_str(script_id).ok().and_then(|id| {
                    self.scripts
                        .read(cx)
                        .scripts
                        .iter()
                        .find(|script| script.id == id)
                        .cloned()
                }) else {
                    self.show_toast(
                        t!("toast.ai.action_target_changed").to_string(),
                        ToastLevel::Error,
                        cx,
                    );
                    return;
                };
                AiActionPlan::new(AiActionPlanSpec {
                    capability: target.capability(),
                    kind: AiActionKind::ScriptExecution,
                    risk: AiActionRisk::High,
                    target_id: script.id.to_string(),
                    target_label: script.name,
                    backend: self.app_config.ai.backend,
                    model,
                    timeout_secs: 1_800,
                    payload: AiActionPayload::ScriptExecution {
                        body: shelldeck_core::ai::clean_generated_script_body(&result),
                    },
                })
            }
            AiWorkflowTarget::SupportReply { ticket_id } => {
                let Some((selected_id, label)) = self.support.read(cx).selected_ticket_identity()
                else {
                    return;
                };
                if &selected_id != ticket_id {
                    self.show_toast(
                        t!("toast.ai.action_target_changed").to_string(),
                        ToastLevel::Error,
                        cx,
                    );
                    return;
                }
                AiActionPlan::new(AiActionPlanSpec {
                    capability: target.capability(),
                    kind: AiActionKind::SupportSend,
                    risk: AiActionRisk::Moderate,
                    target_id: ticket_id.clone(),
                    target_label: label,
                    backend: self.app_config.ai.backend,
                    model,
                    timeout_secs: 30,
                    payload: AiActionPayload::SupportSend { body: result },
                })
            }
            _ => return,
        };
        match plan {
            Ok(plan) => self.stage_ai_action(plan, cx),
            Err(error) => self.show_toast(error.to_string(), ToastLevel::Error, cx),
        }
    }

    pub(super) fn prepare_ai_diagnostic_step(
        &mut self,
        target: AiWorkflowTarget,
        command: String,
        cx: &mut Context<Self>,
    ) {
        let AiWorkflowTarget::TerminalDiagnose { session_id } = &target else {
            return;
        };
        if validate_diagnostic_command(&command).is_err() {
            self.show_toast(
                t!("toast.ai.command_invalid").to_string(),
                ToastLevel::Error,
                cx,
            );
            return;
        }
        let context = self.terminal.read(cx).ai_context_data();
        if context
            .get("session_id")
            .and_then(serde_json::Value::as_str)
            != Some(session_id.as_str())
        {
            self.show_toast(
                t!("toast.ai.action_target_changed").to_string(),
                ToastLevel::Error,
                cx,
            );
            return;
        }
        let label = context
            .get("title")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("Terminal");
        let model = if self.app_config.ai.model.trim().is_empty() {
            self.app_config.ai.backend.default_model().to_string()
        } else {
            self.app_config.ai.model.clone()
        };
        match AiActionPlan::new(AiActionPlanSpec {
            capability: target.capability(),
            kind: AiActionKind::TerminalCommand,
            risk: AiActionRisk::High,
            target_id: session_id.clone(),
            target_label: label.to_string(),
            backend: self.app_config.ai.backend,
            model,
            timeout_secs: 1_800,
            payload: AiActionPayload::TerminalCommand { command },
        }) {
            Ok(plan) => self.stage_ai_action(plan, cx),
            Err(error) => self.show_toast(error.to_string(), ToastLevel::Error, cx),
        }
    }

    pub(super) fn start_ai_diagnostic_sequence(
        &mut self,
        target: AiWorkflowTarget,
        plan: shelldeck_core::ai::AiDiagnosticPlan,
        cx: &mut Context<Self>,
    ) {
        let AiWorkflowTarget::TerminalDiagnose { session_id } = &target else {
            return;
        };
        let Ok(session_id) = Uuid::parse_str(session_id) else {
            return;
        };
        if plan
            .steps
            .iter()
            .any(|step| validate_diagnostic_command(&step.command).is_err())
        {
            self.show_toast(
                t!("toast.ai.command_invalid").to_string(),
                ToastLevel::Error,
                cx,
            );
            return;
        }
        let remaining = plan
            .steps
            .into_iter()
            .map(|step| step.command)
            .collect::<VecDeque<_>>();
        self.ai_diagnostic_sequences
            .insert(session_id, AiDiagnosticSequence { target, remaining });
        self.prepare_next_ai_diagnostic_step(session_id, String::new(), cx);
    }

    pub(super) fn prepare_next_ai_diagnostic_step(
        &mut self,
        session_id: Uuid,
        _previous_output: String,
        cx: &mut Context<Self>,
    ) {
        let next = self
            .ai_diagnostic_sequences
            .get_mut(&session_id)
            .and_then(|sequence| {
                sequence
                    .remaining
                    .pop_front()
                    .map(|command| (sequence.target.clone(), command))
            });
        if let Some((target, command)) = next {
            self.prepare_ai_diagnostic_step(target, command, cx);
            return;
        }
        if let Some(sequence) = self.ai_diagnostic_sequences.remove(&session_id) {
            self.set_ai_workflow_task_status(&sequence.target, AiTaskStatus::Succeeded, cx);
            self.show_toast(
                t!("toast.ai.diagnostic_completed").to_string(),
                ToastLevel::Success,
                cx,
            );
        }
    }

    pub(super) fn prepare_jean_dispatch(&mut self, prompt: String, cx: &mut Context<Self>) {
        if self.effective_jean_config().is_none() || prompt.trim().is_empty() {
            return;
        }
        let model = if self.app_config.ai.model.trim().is_empty() {
            self.app_config.ai.backend.default_model().to_string()
        } else {
            self.app_config.ai.model.clone()
        };
        let (target_id, target_label) = self
            .support
            .read(cx)
            .selected_ticket_identity()
            .unwrap_or_else(|| ("jean".to_string(), "JeanClaude".to_string()));
        match AiActionPlan::new(AiActionPlanSpec {
            capability: shelldeck_core::ai::AiCapability::JeanDispatch,
            kind: AiActionKind::JeanDispatch,
            risk: AiActionRisk::Moderate,
            target_id,
            target_label,
            backend: self.app_config.ai.backend,
            model,
            timeout_secs: 30,
            payload: AiActionPayload::JeanDispatch { prompt },
        }) {
            Ok(plan) => self.stage_ai_action(plan, cx),
            Err(error) => self.show_toast(error.to_string(), ToastLevel::Error, cx),
        }
    }

    pub(super) fn prepare_fleet_dispatch(
        &mut self,
        issue_id: String,
        instance_id: String,
        cx: &mut Context<Self>,
    ) {
        if !self.issues_staff
            || !self
                .issues_instances
                .iter()
                .any(|instance| instance.id == instance_id)
        {
            self.show_toast(
                t!("toast.ai.action_target_changed").to_string(),
                ToastLevel::Error,
                cx,
            );
            return;
        }
        let Some(issue) = self
            .issues_list
            .iter()
            .find(|issue| issue.id == issue_id)
            .cloned()
        else {
            return;
        };
        let model = if self.app_config.ai.model.trim().is_empty() {
            self.app_config.ai.backend.default_model().to_string()
        } else {
            self.app_config.ai.model.clone()
        };
        match AiActionPlan::new(AiActionPlanSpec {
            capability: shelldeck_core::ai::AiCapability::FleetDispatch,
            kind: AiActionKind::FleetDispatch,
            risk: AiActionRisk::High,
            target_id: issue.id.clone(),
            target_label: issue.title,
            backend: self.app_config.ai.backend,
            model,
            timeout_secs: 30,
            payload: AiActionPayload::FleetDispatch {
                issue_id: issue.id,
                instance_id,
            },
        }) {
            Ok(plan) => self.stage_ai_action(plan, cx),
            Err(error) => self.show_toast(error.to_string(), ToastLevel::Error, cx),
        }
    }

    pub(super) fn audit_ai_action(
        &mut self,
        plan: &AiActionPlan,
        status: &str,
        cx: &mut Context<Self>,
    ) {
        let task_status = match status {
            "confirmed" | "automatic" | "started" | "submitted" => Some(AiTaskStatus::Executing),
            "succeeded" => Some(AiTaskStatus::Succeeded),
            "failed" | "timed_out" => Some(AiTaskStatus::Failed),
            "cancelled" | "target_changed" | "target_closed" => Some(AiTaskStatus::Cancelled),
            _ => None,
        };
        if let Some(task_status) = task_status {
            self.set_ai_action_task_status(plan, task_status, None, cx);
        }
        if !self.window_active
            && self.app_config.tray.notify_ai_tasks
            && matches!(status, "succeeded" | "failed" | "timed_out")
        {
            self.emit_tray_notification(TrayNotification::AiTaskDone {
                success: status == "succeeded",
            });
        }
        let kind = match plan.kind {
            AiActionKind::TerminalCommand => ActivityKind::Terminal,
            AiActionKind::ScriptExecution => ActivityKind::Script,
            AiActionKind::SupportSend => ActivityKind::Support,
            AiActionKind::JeanDispatch => ActivityKind::Jean,
            AiActionKind::FleetDispatch => ActivityKind::Fleet,
            AiActionKind::ClippyReplaceSelection => ActivityKind::Script,
        };
        let actor = self
            .app_config
            .account
            .as_ref()
            .map(|account| account.email.as_str())
            .filter(|email| !email.trim().is_empty())
            .unwrap_or("local-session");
        self.add_activity_entry(
            ActivityEntry::new(
                kind,
                t!(
                    "activity.ai.action",
                    status = status,
                    target = plan.target_label.as_str()
                )
                .to_string(),
            )
            .with_target(plan.target_id.clone(), plan.target_label.clone())
            .with_detail(format!("actor={actor} {}", plan.audit_detail(status))),
            cx,
        );
    }

    pub(super) fn cancel_ai_action_confirmation(&mut self, cx: &mut Context<Self>) {
        if let Some(plan) = self.ai_action_confirmation.take() {
            if plan.capability == shelldeck_core::ai::AiCapability::TerminalDiagnose {
                if let Ok(session_id) = Uuid::parse_str(&plan.target_id) {
                    self.ai_diagnostic_sequences.remove(&session_id);
                }
            }
            let resumable = self
                .ai_tasks
                .iter()
                .find(|task| task.id == plan.id)
                .is_some_and(|task| !task.result.trim().is_empty());
            self.set_ai_action_task_status(
                &plan,
                if resumable {
                    AiTaskStatus::Ready
                } else {
                    AiTaskStatus::Cancelled
                },
                None,
                cx,
            );
        }
        cx.notify();
    }

    pub(super) fn track_ai_script_run(
        &mut self,
        script_id: Uuid,
        plan: AiActionPlan,
        cx: &mut Context<Self>,
    ) {
        self.audit_ai_action(&plan, "started", cx);
        let action_id = plan.id;
        let timeout = std::time::Duration::from_secs(plan.timeout_secs);
        self.ai_script_runs.insert(script_id, plan);
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            cx.background_executor().timer(timeout).await;
            let _ = this.update(cx, |workspace, cx| {
                let outcome = super::ai_routing::ai_timeout_outcome(
                    workspace.ai_script_runs.get(&script_id).map(|plan| plan.id),
                    action_id,
                    workspace.scripts.read(cx).running_script_id == Some(script_id),
                );
                if outcome == super::ai_routing::AiTimeoutOutcome::Ignore {
                    return;
                }
                if let Some(plan) = workspace.ai_script_runs.remove(&script_id) {
                    workspace.audit_ai_action(&plan, "timed_out", cx);
                }
                workspace.handle_script_event(&ScriptEvent::StopScript, cx);
                workspace.show_toast(
                    t!("toast.ai.action_timed_out").to_string(),
                    ToastLevel::Warning,
                    cx,
                );
            });
        })
        .detach();
    }

    pub(super) fn track_ai_terminal_run(
        &mut self,
        session_id: Uuid,
        plan: AiActionPlan,
        cx: &mut Context<Self>,
    ) {
        let action_id = plan.id;
        let timeout = std::time::Duration::from_secs(plan.timeout_secs);
        self.ai_terminal_runs.insert(session_id, plan);
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            cx.background_executor().timer(timeout).await;
            let _ = this.update(cx, |workspace, cx| {
                // The terminal path has no liveness check of its own:
                // `stop_ai_command` is a no-op when nothing is running.
                let outcome = super::ai_routing::ai_timeout_outcome(
                    workspace
                        .ai_terminal_runs
                        .get(&session_id)
                        .map(|plan| plan.id),
                    action_id,
                    true,
                );
                if outcome == super::ai_routing::AiTimeoutOutcome::Ignore {
                    return;
                }
                let _ = workspace
                    .terminal
                    .update(cx, |terminal, cx| terminal.stop_ai_command(session_id, cx));
                workspace.ai_diagnostic_sequences.remove(&session_id);
                if let Some(plan) = workspace.ai_terminal_runs.remove(&session_id) {
                    workspace.audit_ai_action(&plan, "timed_out", cx);
                }
                workspace.show_toast(
                    t!("toast.ai.action_timed_out").to_string(),
                    ToastLevel::Warning,
                    cx,
                );
            });
        })
        .detach();
    }

    pub(super) fn finish_ai_script_run(
        &mut self,
        script_id: Uuid,
        status: &str,
        cx: &mut Context<Self>,
    ) {
        if let Some(plan) = self.ai_script_runs.remove(&script_id) {
            self.audit_ai_action(&plan, status, cx);
        }
    }

    pub(super) fn confirm_ai_action(&mut self, cx: &mut Context<Self>) {
        let Some(plan) = self.ai_action_confirmation.take() else {
            return;
        };
        if !self.ai_action_allowed(&plan) {
            return;
        }
        self.audit_ai_action(
            &plan,
            if plan.autonomy == shelldeck_core::ai::AiAutonomyLevel::Automatic {
                "automatic"
            } else {
                "confirmed"
            },
            cx,
        );
        let payload = plan.payload.clone();
        match payload {
            AiActionPayload::TerminalCommand { command } => {
                let executed = Uuid::parse_str(&plan.target_id)
                    .ok()
                    .is_some_and(|session_id| {
                        self.terminal
                            .update(cx, |terminal, cx| {
                                terminal.execute_ai_command(session_id, &command, cx)
                            })
                            .is_ok()
                    });
                if executed {
                    self.audit_ai_action(&plan, "submitted", cx);
                    if let Ok(session_id) = Uuid::parse_str(&plan.target_id) {
                        self.track_ai_terminal_run(session_id, plan.clone(), cx);
                    }
                    self.close_ai_workflow(cx);
                    self.show_toast(
                        t!("toast.ai.action_submitted").to_string(),
                        ToastLevel::Success,
                        cx,
                    );
                } else {
                    if plan.capability == shelldeck_core::ai::AiCapability::TerminalDiagnose {
                        if let Ok(session_id) = Uuid::parse_str(&plan.target_id) {
                            self.ai_diagnostic_sequences.remove(&session_id);
                        }
                    }
                    self.audit_ai_action(&plan, "target_changed", cx);
                    self.show_toast(
                        t!("toast.ai.action_target_changed").to_string(),
                        ToastLevel::Error,
                        cx,
                    );
                }
            }
            AiActionPayload::ScriptExecution { body } => {
                let Some(mut script) = Uuid::parse_str(&plan.target_id).ok().and_then(|id| {
                    self.scripts
                        .read(cx)
                        .scripts
                        .iter()
                        .find(|script| script.id == id)
                        .cloned()
                }) else {
                    self.audit_ai_action(&plan, "target_changed", cx);
                    self.show_toast(
                        t!("toast.ai.action_target_changed").to_string(),
                        ToastLevel::Error,
                        cx,
                    );
                    return;
                };
                script.body = body;
                let script_id = script.id;
                self.close_ai_workflow(cx);
                self.handle_script_event(&ScriptEvent::RunScript(script), cx);
                if self.scripts.read(cx).running_script_id == Some(script_id) {
                    self.track_ai_script_run(script_id, plan, cx);
                }
            }
            AiActionPayload::SupportSend { body } => {
                let selected = self.support.read(cx).selected_ticket_identity();
                if selected.as_ref().map(|(id, _)| id.as_str()) != Some(plan.target_id.as_str()) {
                    self.audit_ai_action(&plan, "target_changed", cx);
                    self.show_toast(
                        t!("toast.ai.action_target_changed").to_string(),
                        ToastLevel::Error,
                        cx,
                    );
                    return;
                }
                self.close_ai_workflow(cx);
                self.execute_ai_support_send(plan, body, cx);
            }
            AiActionPayload::JeanDispatch { prompt } => {
                self.execute_ai_jean_dispatch(plan, prompt, cx);
            }
            AiActionPayload::FleetDispatch {
                issue_id,
                instance_id,
            } => {
                self.execute_ai_fleet_dispatch(plan, issue_id, instance_id, cx);
            }
            AiActionPayload::ClippyReplaceSelection { .. } => {
                self.audit_ai_action(&plan, "unsupported", cx);
                self.show_toast(
                    t!("toast.ai.action_clippy_replace_unsupported").to_string(),
                    ToastLevel::Warning,
                    cx,
                );
            }
        }
        cx.notify();
    }

    pub(super) fn execute_ai_support_send(
        &mut self,
        plan: AiActionPlan,
        body: String,
        cx: &mut Context<Self>,
    ) {
        if !self.can_access_mode(AppMode::Support) {
            return;
        }
        let base = self.account_base_url();
        let token = self.app_config.cloud_sync.token.clone();
        let ticket_id = plan.target_id.clone();
        self.support.update(cx, |view, cx| {
            view.set_loading(true);
            cx.notify();
        });
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(
                    async move { manage_support::support_reply(&base, &token, &ticket_id, &body) },
                )
                .await;
            let _ = this.update(cx, |workspace, cx| match result {
                Ok(ticket) => {
                    workspace.support.update(cx, |view, cx| {
                        view.set_detail(ticket, cx);
                        cx.notify();
                    });
                    workspace.audit_ai_action(&plan, "succeeded", cx);
                    workspace.refresh_support(cx);
                    workspace.show_toast(
                        t!("toast.ai.action_succeeded").to_string(),
                        ToastLevel::Success,
                        cx,
                    );
                }
                Err(error) => {
                    workspace.audit_ai_action(&plan, "failed", cx);
                    let message = cloud_account::user_message(&error);
                    workspace.support.update(cx, |view, cx| {
                        view.set_error(message.clone());
                        cx.notify();
                    });
                    workspace.show_toast(message, ToastLevel::Error, cx);
                }
            });
        })
        .detach();
    }

    pub(super) fn execute_ai_jean_dispatch(
        &mut self,
        plan: AiActionPlan,
        prompt: String,
        cx: &mut Context<Self>,
    ) {
        let Some(config) = self.effective_jean_config() else {
            self.audit_ai_action(&plan, "target_changed", cx);
            return;
        };
        self.audit_ai_action(&plan, "started", cx);
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move { jeanclaude::say(&config, &prompt) })
                .await;
            let _ = this.update(cx, |workspace, cx| {
                match result {
                    Ok(_) => {
                        workspace.audit_ai_action(&plan, "succeeded", cx);
                        workspace.show_toast(
                            t!("toast.jean.sent").to_string(),
                            ToastLevel::Success,
                            cx,
                        );
                    }
                    Err(error) => {
                        workspace.audit_ai_action(&plan, "failed", cx);
                        workspace.show_toast(
                            t!(
                                "toast.jean.error",
                                error = cloud_account::user_message(&error)
                            )
                            .to_string(),
                            ToastLevel::Error,
                            cx,
                        );
                    }
                }
                workspace.refresh_jean_state(cx);
            });
        })
        .detach();
    }

    pub(super) fn execute_ai_fleet_dispatch(
        &mut self,
        plan: AiActionPlan,
        issue_id: String,
        instance_id: String,
        cx: &mut Context<Self>,
    ) {
        if !self.issues_staff
            || !self
                .issues_instances
                .iter()
                .any(|instance| instance.id == instance_id)
            || !self.issues_list.iter().any(|issue| issue.id == issue_id)
        {
            self.audit_ai_action(&plan, "target_changed", cx);
            self.show_toast(
                t!("toast.ai.action_target_changed").to_string(),
                ToastLevel::Error,
                cx,
            );
            return;
        }
        let Some((base, token)) = self.fleet_base_token() else {
            return;
        };
        self.audit_ai_action(&plan, "started", cx);
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(
                    async move { issues::dispatch_issue(&base, &token, &issue_id, &instance_id) },
                )
                .await;
            let _ = this.update(cx, |workspace, cx| match result {
                Ok(issue) => {
                    workspace.audit_ai_action(&plan, "succeeded", cx);
                    workspace.upsert_issue_in_list(issue.clone());
                    workspace.issue_detail = Some(issue);
                    workspace.push_issues_to_support(cx);
                    workspace.show_toast(
                        t!("toast.ai.action_succeeded").to_string(),
                        ToastLevel::Success,
                        cx,
                    );
                }
                Err(error) => {
                    workspace.audit_ai_action(&plan, "failed", cx);
                    workspace.show_toast(
                        t!(
                            "toast.issue.staff_failed",
                            error = cloud_account::user_message(&error)
                        )
                        .to_string(),
                        ToastLevel::Error,
                        cx,
                    );
                }
            });
        })
        .detach();
    }

    pub(super) fn handle_ai_workflow_event(
        &mut self,
        event: AiWorkflowEvent,
        cx: &mut Context<Self>,
    ) {
        if !self.signed_in() {
            return;
        }
        match event {
            AiWorkflowEvent::Generate {
                request_id,
                target,
                instructions,
            } => {
                let base = match &target {
                    AiWorkflowTarget::EntityNaming { .. } => {
                        t!("ai.prompt.entity_name").to_string()
                    }
                    AiWorkflowTarget::SupportReply { .. } => {
                        t!("ai.prompt.support_reply").to_string()
                    }
                    AiWorkflowTarget::SupportSummary { .. } => {
                        t!("ai.prompt.support_summary").to_string()
                    }
                    AiWorkflowTarget::SupportTriage { .. } => {
                        t!("ai.prompt.support_triage").to_string()
                    }
                    AiWorkflowTarget::IssueReply { .. } => t!("ai.prompt.issue_reply").to_string(),
                    AiWorkflowTarget::IssueSummary { .. } => {
                        t!("ai.prompt.issue_summary").to_string()
                    }
                    AiWorkflowTarget::IssueTriage { .. } => {
                        t!("ai.prompt.issue_triage").to_string()
                    }
                    AiWorkflowTarget::ScriptGenerate { .. } => {
                        t!("ai.prompt.script_generate").to_string()
                    }
                    AiWorkflowTarget::ScriptExplain { .. } => {
                        t!("ai.prompt.script_explain").to_string()
                    }
                    AiWorkflowTarget::ScriptReview { .. } => {
                        t!("ai.prompt.script_review").to_string()
                    }
                    AiWorkflowTarget::ScriptFix { .. } => t!("ai.prompt.script_fix").to_string(),
                    AiWorkflowTarget::TerminalCommand { .. } => {
                        t!("ai.prompt.terminal_command_strict").to_string()
                    }
                    AiWorkflowTarget::TerminalDiagnose { .. } => {
                        t!("ai.prompt.terminal_diagnostic_plan").to_string()
                    }
                };
                let prompt = if instructions.trim().is_empty() {
                    base
                } else {
                    format!(
                        "{base}\n\n{}:\n{}",
                        t!("ai.workflow.additional_instructions"),
                        instructions.trim()
                    )
                };
                let context = self.ai_workflow_context(&target, cx);
                let config = self.app_config.ai.clone();
                let structured_issue_triage = matches!(
                    &target,
                    AiWorkflowTarget::SupportTriage { .. } | AiWorkflowTarget::IssueTriage { .. }
                );
                let structured_name = matches!(&target, AiWorkflowTarget::EntityNaming { .. });
                let structured_diagnostic =
                    matches!(&target, AiWorkflowTarget::TerminalDiagnose { .. });
                let issue_triage_repair =
                    t!("ai.prompt.issue_triage_repair", error = "__TRIAGE_ERROR__").to_string();
                let diagnostic_repair = t!(
                    "ai.prompt.terminal_diagnostic_repair",
                    error = "__DIAGNOSTIC_ERROR__"
                )
                .to_string();
                let workflow = self.ai_workflow.as_ref().map(|entity| entity.downgrade());
                let task_id = self.begin_ai_workflow_task(&target, instructions, cx);
                let automatic_support_triage =
                    matches!(&target, AiWorkflowTarget::SupportTriage { .. })
                        && self.app_config.ai.policies.support_triage
                            == shelldeck_core::ai::AiAutonomyLevel::Automatic;
                let generated_target = target.clone();
                cx.spawn(async move |this, cx: &mut AsyncApp| {
                    let result = cx
                        .background_executor()
                        .spawn(async move {
                            let client = create_client(&config)?;
                            let response = client.complete(&prompt, context.clone())?;
                            if structured_name {
                                parse_generated_name(&response.text)?;
                                return Ok::<String, shelldeck_core::ShellDeckError>(response.text);
                            }
                            if structured_diagnostic {
                                return match parse_diagnostic_plan(&response.text) {
                                    Ok(_) => {
                                        Ok::<String, shelldeck_core::ShellDeckError>(response.text)
                                    }
                                    Err(first_error) => {
                                        let repair = diagnostic_repair.replace(
                                            "__DIAGNOSTIC_ERROR__",
                                            &first_error.to_string(),
                                        );
                                        let repaired = client.complete(
                                            &format!("{prompt}\n\n{repair}"),
                                            context.clone(),
                                        )?;
                                        parse_diagnostic_plan(&repaired.text)?;
                                        Ok::<String, shelldeck_core::ShellDeckError>(repaired.text)
                                    }
                                };
                            }
                            if !structured_issue_triage {
                                return Ok::<String, shelldeck_core::ShellDeckError>(response.text);
                            }
                            match parse_issue_triage_proposal(&response.text) {
                                Ok(_) => {
                                    Ok::<String, shelldeck_core::ShellDeckError>(response.text)
                                }
                                Err(first_error) => {
                                    let repair = issue_triage_repair
                                        .replace("__TRIAGE_ERROR__", &first_error.to_string());
                                    let repaired = client
                                        .complete(&format!("{prompt}\n\n{repair}"), context)?;
                                    parse_issue_triage_proposal(&repaired.text)?;
                                    Ok::<String, shelldeck_core::ShellDeckError>(repaired.text)
                                }
                            }
                        })
                        .await
                        .map_err(|error| error.to_string());
                    let _ = this.update(cx, |workspace, cx| {
                        workspace.finish_ai_workflow_task(task_id, &result, cx);
                        if automatic_support_triage {
                            if let (AiWorkflowTarget::SupportTriage { ticket_id }, Ok(raw)) =
                                (&generated_target, &result)
                            {
                                if let Ok(proposal) = parse_issue_triage_proposal(raw) {
                                    workspace.set_ai_workflow_task_status(
                                        &generated_target,
                                        AiTaskStatus::Applied,
                                        cx,
                                    );
                                    workspace.close_ai_workflow(cx);
                                    workspace.apply_support_triage(ticket_id.clone(), proposal, cx);
                                }
                            }
                        }
                    });
                    if let Some(workflow) = workflow.and_then(|workflow| workflow.upgrade()) {
                        let _ = workflow.update(cx, |view, cx| {
                            view.set_result(request_id, result, cx);
                        });
                    }
                })
                .detach();
            }
            AiWorkflowEvent::Accept { target, result } => {
                if let AiWorkflowTarget::EntityNaming { kind, target_id } = &target {
                    let generated = match parse_generated_name(&result) {
                        Ok(generated) => generated,
                        Err(error) => {
                            self.show_toast(
                                t!("toast.ai.name_invalid", error = error.to_string()).to_string(),
                                ToastLevel::Error,
                                cx,
                            );
                            return;
                        }
                    };
                    let open_script_form = self
                        .script_form
                        .as_ref()
                        .map(|form| form.entity_id().to_string());
                    let open_tunnel_form = self
                        .port_forward_form
                        .as_ref()
                        .map(|form| form.entity_id().to_string());
                    let application = super::ai_routing::resolve_naming_application(
                        *kind,
                        target_id,
                        open_script_form.as_deref(),
                        open_tunnel_form.as_deref(),
                        self.user_new_request_sheet_open,
                    );
                    let applied = match application {
                        super::ai_routing::NamingApplication::ScriptForm => {
                            self.script_form.as_ref().is_some_and(|form| {
                                form.update(cx, |form, cx| form.apply_ai_name(generated.name, cx));
                                true
                            })
                        }
                        super::ai_routing::NamingApplication::Terminal(id) => self
                            .terminal
                            .update(cx, |view, cx| view.apply_ai_name(id, generated.name, cx))
                            .is_ok(),
                        super::ai_routing::NamingApplication::TunnelForm => {
                            self.port_forward_form.as_ref().is_some_and(|form| {
                                form.update(cx, |form, cx| form.apply_ai_name(generated.name, cx));
                                true
                            })
                        }
                        super::ai_routing::NamingApplication::IssueDraft => {
                            self.issue_title_state
                                .update(cx, |state, cx| state.replace_content(generated.name, cx));
                            true
                        }
                        super::ai_routing::NamingApplication::TargetLost => false,
                    };
                    if !applied {
                        self.show_toast(
                            t!("toast.ai.name_target_missing").to_string(),
                            ToastLevel::Error,
                            cx,
                        );
                        return;
                    }
                    self.set_ai_workflow_task_status(&target, AiTaskStatus::Applied, cx);
                    self.close_ai_workflow(cx);
                    self.show_toast(
                        t!("toast.ai.name_applied").to_string(),
                        ToastLevel::Success,
                        cx,
                    );
                    return;
                }
                if let AiWorkflowTarget::SupportTriage { ticket_id } = &target {
                    let proposal = match parse_issue_triage_proposal(&result) {
                        Ok(proposal) => proposal,
                        Err(error) => {
                            self.show_toast(
                                t!("toast.ai.triage_invalid", error = error.to_string())
                                    .to_string(),
                                ToastLevel::Error,
                                cx,
                            );
                            return;
                        }
                    };
                    if self.app_config.ai.policies.support_triage
                        == shelldeck_core::ai::AiAutonomyLevel::Preparation
                    {
                        cx.write_to_clipboard(ClipboardItem::new_string(result));
                        self.set_ai_workflow_task_status(&target, AiTaskStatus::Applied, cx);
                        self.close_ai_workflow(cx);
                        self.show_toast(
                            t!("toast.ai.analysis_copied").to_string(),
                            ToastLevel::Success,
                            cx,
                        );
                        return;
                    }
                    self.set_ai_workflow_task_status(&target, AiTaskStatus::Applied, cx);
                    let ticket_id = ticket_id.clone();
                    self.close_ai_workflow(cx);
                    self.apply_support_triage(ticket_id, proposal, cx);
                    return;
                }
                if let AiWorkflowTarget::IssueTriage { issue_id } = &target {
                    let proposal = match parse_issue_triage_proposal(&result) {
                        Ok(proposal) => proposal,
                        Err(error) => {
                            self.show_toast(
                                t!("toast.ai.triage_invalid", error = error.to_string())
                                    .to_string(),
                                ToastLevel::Error,
                                cx,
                            );
                            return;
                        }
                    };
                    self.set_ai_workflow_task_status(&target, AiTaskStatus::Applied, cx);
                    let issue_id = issue_id.clone();
                    self.close_ai_workflow(cx);
                    self.apply_issue_triage(issue_id, proposal, cx);
                    return;
                }
                let copied = matches!(
                    &target,
                    AiWorkflowTarget::SupportSummary { .. }
                        | AiWorkflowTarget::SupportTriage { .. }
                        | AiWorkflowTarget::IssueSummary { .. }
                        | AiWorkflowTarget::ScriptExplain { .. }
                        | AiWorkflowTarget::ScriptReview { .. }
                        | AiWorkflowTarget::TerminalDiagnose { .. }
                );
                match &target {
                    AiWorkflowTarget::EntityNaming { .. } => unreachable!(),
                    AiWorkflowTarget::SupportReply { .. } => {
                        self.support.update(cx, |view, cx| {
                            view.set_composer_draft(result, cx);
                        });
                    }
                    AiWorkflowTarget::IssueReply { .. } => {
                        // Distinct card, not the composer: a suggestion is a
                        // proposal to review, not a keystroke.
                        self.support.update(cx, |view, cx| {
                            view.set_issue_ai_draft(result, cx);
                        });
                    }
                    AiWorkflowTarget::SupportSummary { .. }
                    | AiWorkflowTarget::SupportTriage { .. }
                    | AiWorkflowTarget::IssueSummary { .. }
                    | AiWorkflowTarget::ScriptExplain { .. }
                    | AiWorkflowTarget::ScriptReview { .. } => {
                        cx.write_to_clipboard(ClipboardItem::new_string(result));
                    }
                    AiWorkflowTarget::IssueTriage { .. } => unreachable!(),
                    AiWorkflowTarget::ScriptGenerate { script_id } => {
                        let Some(script_id) = Uuid::parse_str(script_id).ok().filter(|script_id| {
                            self.scripts.read(cx).script_body(*script_id).is_some()
                        }) else {
                            self.show_toast(
                                t!("toast.ai.action_target_changed").to_string(),
                                ToastLevel::Error,
                                cx,
                            );
                            return;
                        };
                        self.active_view = ActiveView::Scripts;
                        self.ai_sheet = None;
                        self.scripts.update(cx, |view, cx| {
                            view.apply_generated_body(script_id, result, cx);
                        });
                    }
                    AiWorkflowTarget::ScriptFix { script_id } => {
                        let Some(script_id) = Uuid::parse_str(script_id).ok().filter(|script_id| {
                            self.scripts.read(cx).script_body(*script_id).is_some()
                        }) else {
                            self.show_toast(
                                t!("toast.ai.action_target_changed").to_string(),
                                ToastLevel::Error,
                                cx,
                            );
                            return;
                        };
                        let body = shelldeck_core::ai::clean_generated_script_body(&result);
                        self.active_view = ActiveView::Scripts;
                        self.ai_sheet = None;
                        self.scripts.update(cx, |view, cx| {
                            view.apply_generated_body(script_id, body, cx);
                        });
                    }
                    AiWorkflowTarget::TerminalCommand { session_id } => {
                        let applied = Uuid::parse_str(session_id).ok().is_some_and(|session_id| {
                            self.terminal
                                .read(cx)
                                .insert_ai_command(session_id, &result)
                                .is_ok()
                        });
                        if !applied {
                            self.show_toast(
                                t!("toast.ai.command_invalid").to_string(),
                                ToastLevel::Error,
                                cx,
                            );
                            return;
                        }
                    }
                    AiWorkflowTarget::TerminalDiagnose { .. } => {
                        let display = match parse_diagnostic_plan(&result) {
                            Ok(plan) => plan.display_text(),
                            Err(error) => {
                                self.show_toast(error.to_string(), ToastLevel::Error, cx);
                                return;
                            }
                        };
                        cx.write_to_clipboard(ClipboardItem::new_string(display));
                    }
                }
                self.set_ai_workflow_task_status(&target, AiTaskStatus::Applied, cx);
                self.close_ai_workflow(cx);
                self.show_toast(
                    if copied {
                        t!("toast.ai.analysis_copied").to_string()
                    } else {
                        t!("toast.ai.draft_applied").to_string()
                    },
                    ToastLevel::Success,
                    cx,
                );
            }
            AiWorkflowEvent::PrepareAction { target, result } => {
                self.prepare_ai_action(target, result, cx);
            }
            AiWorkflowEvent::PrepareDiagnosticStep { target, command } => {
                self.prepare_ai_diagnostic_step(target, command, cx);
            }
            AiWorkflowEvent::PrepareDiagnosticPlan { target, plan } => {
                self.start_ai_diagnostic_sequence(target, plan, cx);
            }
            AiWorkflowEvent::Pending {
                target,
                instructions,
                result,
            } => {
                if let Some(task) = self.ai_tasks.iter_mut().rev().find(|task| {
                    task.capability == target.capability() && task.target_id == target.target_id()
                }) {
                    task.instructions = instructions;
                    task.result = result;
                    task.target_kind = Some(target.storage_kind().to_string());
                    task.set_status(AiTaskStatus::Pending, None);
                } else {
                    let mut task = AiTask::new(
                        target.capability(),
                        target.surface(),
                        target.target_id(),
                        self.app_config.ai.backend,
                        instructions,
                        result,
                    );
                    task.target_kind = Some(target.storage_kind().to_string());
                    self.ai_tasks.push(task);
                }
                self.sync_ai_tasks(cx);
                self.show_toast(
                    t!("toast.ai.draft_pending").to_string(),
                    ToastLevel::Success,
                    cx,
                );
                self.close_ai_workflow(cx);
            }
            AiWorkflowEvent::Cancel => self.close_ai_workflow(cx),
        }
    }

    pub(super) fn open_ai_assistant_with_context(
        &mut self,
        context: AiContext,
        cx: &mut Context<Self>,
    ) {
        if !self.signed_in()
            || !self.ai_backend_available()
            || !self.app_config.ai.allows(context.surface)
        {
            return;
        }
        self.ai_assistant.update(cx, |assistant, cx| {
            assistant.reload_conversations(cx);
            assistant.set_backend(
                self.app_config.ai.backend,
                self.app_config.ai.model.clone(),
                cx,
            );
            assistant.set_context(context, cx);
        });
        self.refresh_mention_directory(cx);
        self.present_ai_sheet(cx);
    }

    /// Open the assistant Sheet directly on the Clippy activity (palette
    /// `OpenClippy`). Gated on the Clippy surface, not the current chat
    /// surface: Clippy transforms clipboard text and is independent of what
    /// the conversation context is about — so the chat context is left as-is
    /// and no in-flight chat request is disturbed.
    pub(super) fn open_ai_clippy(&mut self, cx: &mut Context<Self>) {
        if !self.signed_in()
            || !self.ai_backend_available()
            || !self.app_config.ai.allows(AiSurface::Clippy)
        {
            return;
        }
        let backend = self.app_config.ai.backend;
        let model = self.app_config.ai.model.clone();
        let auto_import = self.app_config.clippy.auto_import_clipboard_on_shortcut;
        self.ai_assistant.update(cx, |assistant, cx| {
            assistant.reload_conversations(cx);
            assistant.set_backend(backend, model, cx);
            // The guard above just validated the Clippy surface.
            assistant.set_clippy_available(true, cx);
            assistant.set_clippy_auto_import_clipboard(auto_import, cx);
            assistant.show_clippy(cx);
        });
        self.refresh_mention_directory(cx);
        self.present_ai_sheet(cx);
    }

    /// Hide any Dock windows and (re)build the main-window assistant Sheet
    /// around the shared `ai_assistant` entity. Callers gate access and
    /// prepare the entity (context / activity) before presenting.
    fn present_ai_sheet(&mut self, cx: &mut Context<Self>) {
        for handle in cx.windows() {
            if let Some(dock) = handle.downcast::<crate::ai_dock::AiDockView>() {
                let _ = dock.update(cx, |_dock, window, _cx| window.hide_window());
            }
        }
        let sheet_assistant = self.ai_assistant.clone();
        let workspace = cx.entity().downgrade();
        self.ai_sheet = Some(cx.new(move |sheet_cx| {
            // No title and no close button => adabraka's `has_header` is
            // false and the Sheet draws no chrome of its own. The single 44px
            // row now lives inside the view, which is what the mockup calls
            // "the host provides one row" — here the host's contribution is to
            // get out of the way.
            Sheet::new(sheet_cx)
                .width(gpui::px(780.0))
                .variant(SheetVariant::Assistant)
                .show_close_button(false)
                .dynamic_content(move || sheet_assistant.clone())
                .on_close(move |_window, cx| {
                    if let Some(workspace) = workspace.upgrade() {
                        workspace.update(cx, |this, cx| {
                            this.ai_sheet = None;
                            cx.notify();
                        });
                    }
                })
        }));
        cx.notify();
    }

    pub(super) fn handle_ai_assistant_event(
        &mut self,
        source: Entity<AiAssistantView>,
        event: AiAssistantEvent,
        cx: &mut Context<Self>,
    ) {
        if !self.signed_in() {
            return;
        }
        match event {
            // Routed through the Settings entity, which owns `ai.*`. Writing
            // `app_config` here would leave the Settings snapshot stale and
            // resurrect the old backend on the next settings save
            // (`.agents/session-state.md`).
            AiAssistantEvent::OpenSettings => self.open_settings(cx),
            // Only the Dock's rail raises these — in the Sheet the main window
            // is already in front and the palette has its own shortcut.
            AiAssistantEvent::OpenMainWindow | AiAssistantEvent::OpenPalette => {}
            AiAssistantEvent::SelectBackend(backend) => {
                self.settings.update(cx, |settings, cx| {
                    settings.set_ai_backend(backend, cx);
                });
            }
            // The Sheet no longer draws its own chrome, so its close button
            // lives in the view and asks us to dismiss.
            AiAssistantEvent::Close => {
                self.ai_sheet = None;
                cx.notify();
            }
            AiAssistantEvent::Submit {
                request_id,
                conversation_id,
                prompt,
                latest_user_message,
                context,
            } => {
                let config = self.app_config.ai.clone();
                let source = source.clone();
                cx.spawn(async move |this, cx: &mut AsyncApp| {
                    let result = cx
                        .background_executor()
                        .spawn(async move {
                            let client = create_client(&config)?;
                            complete_assistant_turn(
                                client.as_ref(),
                                &prompt,
                                &latest_user_message,
                                *context,
                            )
                        })
                        .await
                        .map_err(|error| error.to_string());
                    let _ = this.update(cx, |workspace, cx| match result {
                        Ok(AiAssistantCompletion::Message(message)) => {
                            source.update(cx, |assistant, cx| {
                                assistant.set_result(request_id, conversation_id, Ok(message), cx);
                            });
                        }
                        Ok(AiAssistantCompletion::Action(action)) => {
                            let accepted = source.update(cx, |assistant, cx| {
                                assistant.set_result(
                                    request_id,
                                    conversation_id,
                                    Ok(assistant_action_acknowledgement(&action)),
                                    cx,
                                )
                            });
                            if accepted {
                                workspace.apply_ai_assistant_action(action, cx);
                            }
                        }
                        Err(error) => {
                            source.update(cx, |assistant, cx| {
                                assistant.set_result(request_id, conversation_id, Err(error), cx);
                            });
                        }
                    });
                })
                .detach();
            }
            AiAssistantEvent::ResumeTask(task_id) => {
                let target = self
                    .ai_tasks
                    .iter()
                    .find(|task| task.id == task_id)
                    .and_then(AiWorkflowTarget::from_task);
                if let Some(target) = target {
                    self.ai_sheet = None;
                    self.open_ai_workflow(target, cx);
                } else {
                    self.show_toast(
                        t!("toast.ai.task_unavailable").to_string(),
                        ToastLevel::Warning,
                        cx,
                    );
                }
            }
            AiAssistantEvent::OpenTaskTarget(task_id) => {
                self.open_ai_task_target(task_id, cx);
            }
            AiAssistantEvent::RefreshMentions => {
                self.refresh_mention_directory(cx);
            }
            AiAssistantEvent::StopTask(task_id) => {
                self.stop_ai_task(task_id, cx);
            }
            AiAssistantEvent::DeleteTask(task_id) => {
                let active = self
                    .ai_tasks
                    .iter()
                    .find(|task| task.id == task_id)
                    .is_some_and(|task| task.status.is_active());
                if !active {
                    self.ai_tasks.retain(|task| task.id != task_id);
                    self.sync_ai_tasks(cx);
                }
            }
        }
    }

    pub fn handle_ai_companion_event(&mut self, event: AiCompanionEvent, cx: &mut Context<Self>) {
        if !self.signed_in() {
            return;
        }
        match event {
            AiCompanionEvent::ApplyAction(action) => {
                self.apply_ai_assistant_action(action, cx);
            }
            AiCompanionEvent::OpenSettings => self.open_settings(cx),
            // The app root already raised the main window on its way here.
            AiCompanionEvent::OpenMainWindow | AiCompanionEvent::OpenPalette => {}
            AiCompanionEvent::ResumeTask(task_id) => {
                let target = self
                    .ai_tasks
                    .iter()
                    .find(|task| task.id == task_id)
                    .and_then(AiWorkflowTarget::from_task);
                if let Some(target) = target {
                    self.ai_sheet = None;
                    self.open_ai_workflow(target, cx);
                } else {
                    self.show_toast(
                        t!("toast.ai.task_unavailable").to_string(),
                        ToastLevel::Warning,
                        cx,
                    );
                }
            }
            AiCompanionEvent::OpenTaskTarget(task_id) => {
                self.open_ai_task_target(task_id, cx);
            }
            AiCompanionEvent::StopTask(task_id) => {
                self.stop_ai_task(task_id, cx);
            }
            AiCompanionEvent::DeleteTask(task_id) => {
                let active = self
                    .ai_tasks
                    .iter()
                    .find(|task| task.id == task_id)
                    .is_some_and(|task| task.status.is_active());
                if !active {
                    self.ai_tasks.retain(|task| task.id != task_id);
                    self.sync_ai_tasks(cx);
                }
            }
        }
    }

    fn apply_ai_assistant_action(&mut self, action: AiAssistantAction, cx: &mut Context<Self>) {
        self.ai_sheet = None;
        match action {
            AiAssistantAction::CreateRequest(draft) => self.open_ai_request_draft(draft, cx),
            AiAssistantAction::CreateScript { instructions } => {
                if !self.can_access_mode(AppMode::Dev)
                    || !self.ai_backend_available()
                    || !self.app_config.ai.allows(AiSurface::Script)
                {
                    self.show_toast(
                        t!("toast.ai.surface_unavailable").to_string(),
                        ToastLevel::Warning,
                        cx,
                    );
                    return;
                }
                self.open_ai_script_draft(instructions, cx);
            }
            AiAssistantAction::TerminalCommand { instructions } => {
                if !self.ai_backend_available() || !self.app_config.ai.allows(AiSurface::Terminal) {
                    self.show_toast(
                        t!("toast.ai.surface_unavailable").to_string(),
                        ToastLevel::Warning,
                        cx,
                    );
                    return;
                }
                let session_id = (self.effective_mode() == AppMode::Dev
                    && self.active_view == ActiveView::Terminal)
                    .then(|| self.terminal.read(cx).ai_context_data())
                    .and_then(|data| {
                        data.get("session_id")
                            .and_then(serde_json::Value::as_str)
                            .map(str::to_string)
                    });
                let Some(session_id) = session_id else {
                    self.show_toast(
                        t!("toast.ai.no_active_terminal").to_string(),
                        ToastLevel::Warning,
                        cx,
                    );
                    return;
                };
                self.open_ai_workflow_with_instructions(
                    AiWorkflowTarget::TerminalCommand { session_id },
                    Some(instructions),
                    true,
                    cx,
                );
            }
            AiAssistantAction::SupportReply { instructions } => {
                if !self.ai_backend_available() || !self.app_config.ai.allows(AiSurface::Support) {
                    self.show_toast(
                        t!("toast.ai.surface_unavailable").to_string(),
                        ToastLevel::Warning,
                        cx,
                    );
                    return;
                }
                if self.effective_mode() != AppMode::Support {
                    self.show_toast(
                        t!("toast.ai.no_selected_support_ticket").to_string(),
                        ToastLevel::Warning,
                        cx,
                    );
                    return;
                }
                let Some((ticket_id, _)) = self.support.read(cx).selected_ticket_identity() else {
                    self.show_toast(
                        t!("toast.ai.no_selected_support_ticket").to_string(),
                        ToastLevel::Warning,
                        cx,
                    );
                    return;
                };
                self.open_ai_workflow_with_instructions(
                    AiWorkflowTarget::SupportReply { ticket_id },
                    Some(instructions),
                    true,
                    cx,
                );
            }
            AiAssistantAction::JeanDispatch { prompt } => {
                if self.effective_jean_config().is_none() {
                    self.show_toast(
                        t!("toast.jean.not_configured").to_string(),
                        ToastLevel::Warning,
                        cx,
                    );
                    return;
                }
                self.prepare_jean_dispatch(prompt, cx);
            }
            AiAssistantAction::OpenRequest { issue_id, query } => {
                if !self.signed_in() {
                    self.show_toast(
                        t!("toast.issue.login_required_list").to_string(),
                        ToastLevel::Warning,
                        cx,
                    );
                    return;
                }
                let resolution =
                    resolve_issue_target(&self.issues_list, issue_id.as_deref(), &query);
                let issue_id = match resolution {
                    IssueTargetResolution::Found(id) => id.to_string(),
                    IssueTargetResolution::Ambiguous => {
                        self.show_toast(
                            t!("toast.ai.request_target_ambiguous").to_string(),
                            ToastLevel::Warning,
                            cx,
                        );
                        return;
                    }
                    IssueTargetResolution::NotFound => {
                        self.show_toast(
                            t!("toast.ai.request_target_not_found").to_string(),
                            ToastLevel::Warning,
                            cx,
                        );
                        return;
                    }
                };
                if self.effective_mode() == AppMode::Support {
                    self.support.update(cx, |view, cx| {
                        view.set_section(crate::support_view::SupportSection::Requests);
                        cx.notify();
                    });
                } else {
                    if self.can_switch_mode() {
                        self.set_mode(AppMode::User, cx);
                    }
                    self.user_home_tab = UserHomeTab::Requests;
                }
                self.select_issue(issue_id, cx);
            }
        }
        cx.notify();
    }

    pub(super) fn open_ai_task_target(&mut self, task_id: Uuid, cx: &mut Context<Self>) {
        let Some(task) = self
            .ai_tasks
            .iter()
            .find(|task| task.id == task_id)
            .cloned()
        else {
            return;
        };
        self.ai_sheet = None;
        match task.surface {
            AiSurface::Support => {
                if !self.can_access_mode(AppMode::Support) {
                    return;
                }
                self.set_mode(AppMode::Support, cx);
                self.support.update(cx, |view, cx| {
                    view.set_section(crate::support_view::SupportSection::Tickets);
                    cx.notify();
                });
                self.select_support_ticket(task.target_id, cx);
            }
            AiSurface::Issue => {
                self.open_support_requests(cx);
                self.select_issue(task.target_id, cx);
            }
            AiSurface::Script => {
                if let Ok(script_id) = Uuid::parse_str(&task.target_id) {
                    self.active_view = ActiveView::Scripts;
                    self.scripts.update(cx, |view, cx| {
                        view.selected_script = Some(script_id);
                        cx.notify();
                    });
                }
            }
            AiSurface::Terminal => {
                if let Ok(session_id) = Uuid::parse_str(&task.target_id) {
                    self.active_view = ActiveView::Terminal;
                    self.terminal.update(cx, |view, cx| {
                        view.select_tab(session_id);
                        cx.notify();
                    });
                }
            }
            AiSurface::Jean => {
                self.active_view = ActiveView::JeanConsole;
            }
            AiSurface::Naming => {
                if task.target_kind.as_deref() == Some("naming_terminal") {
                    if let Ok(session_id) = Uuid::parse_str(&task.target_id) {
                        self.active_view = ActiveView::Terminal;
                        self.terminal.update(cx, |view, cx| {
                            view.select_tab(session_id);
                            cx.notify();
                        });
                    }
                }
            }
            AiSurface::Recent | AiSurface::Global | AiSurface::Clippy => {}
        }
        cx.notify();
    }

    pub(super) fn stop_ai_task(&mut self, task_id: Uuid, cx: &mut Context<Self>) {
        if let Some((script_id, _)) = self
            .ai_script_runs
            .iter()
            .find(|(_, plan)| plan.id == task_id)
            .map(|(script_id, plan)| (*script_id, plan.clone()))
        {
            if self.scripts.read(cx).running_script_id == Some(script_id) {
                self.handle_script_event(&ScriptEvent::StopScript, cx);
                return;
            }
        }
        if let Some((session_id, plan)) = self
            .ai_terminal_runs
            .iter()
            .find(|(_, plan)| plan.id == task_id)
            .map(|(session_id, plan)| (*session_id, plan.clone()))
        {
            let stopped = self
                .terminal
                .update(cx, |terminal, cx| terminal.stop_ai_command(session_id, cx))
                .is_ok();
            if stopped {
                self.ai_terminal_runs.remove(&session_id);
                self.audit_ai_action(&plan, "cancelled", cx);
                self.show_toast(
                    t!("toast.ai.command_stopped").to_string(),
                    ToastLevel::Info,
                    cx,
                );
                return;
            }
        }
        self.show_toast(
            t!("toast.ai.task_stop_unavailable").to_string(),
            ToastLevel::Warning,
            cx,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::{resolve_issue_target, IssueTargetResolution};
    use shelldeck_core::config::issues::Issue;

    fn issue(id: &str, title: &str) -> Issue {
        Issue {
            id: id.to_string(),
            title: title.to_string(),
            ..Issue::default()
        }
    }

    // SDTEST-1431
    #[test]
    fn assistant_request_target_resolution_rejects_stale_and_ambiguous_matches() {
        let issues = vec![
            issue("req-42", "Chargement des produits clients"),
            issue("req-43", "Chargement des produits fournisseurs"),
            issue("req-99", "Certificat nginx expiré"),
        ];

        assert_eq!(
            resolve_issue_target(&issues, Some("REQ-42"), "texte ignoré"),
            IssueTargetResolution::Found("req-42")
        );
        assert_eq!(
            resolve_issue_target(&issues, None, "Certificat nginx expiré"),
            IssueTargetResolution::Found("req-99")
        );
        assert_eq!(
            resolve_issue_target(&issues, None, "produits"),
            IssueTargetResolution::Ambiguous
        );
        assert_eq!(
            resolve_issue_target(&issues, Some("req-stale"), "absente"),
            IssueTargetResolution::NotFound
        );
    }
}
