use super::*;

fn runtime_loop_requested(runtime_enabled: bool, credentials_available: bool) -> bool {
    runtime_enabled && credentials_available
}

impl Workspace {
    // --- Jean fleet runtime ---

    /// `(base_url, token)` when signed in to Inklura Manage.
    pub(super) fn fleet_base_token(&self) -> Option<(String, String)> {
        if self.signed_in() {
            Some((
                self.account_base_url(),
                self.app_config.cloud_sync.token.clone(),
            ))
        } else {
            None
        }
    }

    pub(super) fn fleet_visible(&self) -> bool {
        !self.settings_open
            && self.fleet_base_token().is_some()
            && self.effective_mode() == AppMode::Dev
            && self.active_view == ActiveView::Fleet
    }

    pub(super) fn update_fleet_availability(&mut self, cx: &mut Context<Self>) {
        let show = self.fleet_base_token().is_some() && self.effective_mode() == AppMode::Dev;
        self.sidebar.update(cx, |s, cx| {
            s.set_fleet_available(show);
            cx.notify();
        });
    }

    /// Tenant to register this machine under: the active site's tenant, else the
    /// first known site's, else empty (the server pins it for non-super-admins).
    pub(super) fn runtime_tenant(&self) -> (String, String) {
        if let Some(active) = self.active_site_info() {
            return (active.tenant_id, active.tenant_name);
        }
        if let Some(dir) = &self.site_directory {
            if let Some(s) = dir.sites.first() {
                return (s.tenant_id.clone(), s.tenant_name.clone());
            }
        }
        (String::new(), String::new())
    }

    pub(super) fn runtime_workdir_model(&self) -> (String, String) {
        let inst = self.runtime_instance.as_ref();
        let workdir = inst
            .map(|i| i.workdir.clone())
            .filter(|s| !s.trim().is_empty())
            .or_else(|| self.app_config.jean_runtime.workdir.clone())
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| {
                // util::home_dir() also honors %USERPROFILE% / the platform
                // lookup, so Windows and macOS don't silently fall back to
                // "." the way the old $HOME-only read did.
                shelldeck_core::util::home_dir()
                    .map(|home| home.display().to_string())
                    .unwrap_or_else(|| ".".to_string())
            });
        let fleet_model = inst.map(|i| i.model.clone()).unwrap_or_default();
        let model = self.app_config.jean_runtime.job_model(&fleet_model);
        (workdir, model)
    }

    pub(super) fn build_register(&self) -> Option<RegisterInstance> {
        let (tenant_id, tenant_name) = self.runtime_tenant();
        let name = self
            .app_config
            .jean_runtime
            .name
            .clone()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(cloud_account::device_name);
        let (workdir, model) = self.runtime_workdir_model();
        let model = (!model.trim().is_empty()).then_some(model);
        Some(RegisterInstance {
            id: self.app_config.jean_runtime.instance_id.clone(),
            name,
            tenant_id,
            tenant_name,
            site_id: self.app_config.cloud_sync.active_site_id.clone(),
            slack_channel: None,
            workdir,
            model,
            // Only set autonomy on the FIRST register (safe default = confirm);
            // later leave it so an admin can flip it to "auto" in the console.
            autonomy: if self.app_config.jean_runtime.instance_id.is_none() {
                Some("confirm".to_string())
            } else {
                None
            },
        })
    }

    pub(super) fn refresh_fleet_view(&mut self, cx: &mut Context<Self>) {
        let Some((base, token)) = self.fleet_base_token() else {
            return;
        };
        self.fleet_view.update(cx, |v, cx| {
            v.set_loading(true);
            cx.notify();
        });
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move { jean_fleet::get_fleet(&base, &token) })
                .await;
            let _ = this.update(cx, |ws, cx| match result {
                Ok(snap) => {
                    ws.fleet_snapshot = Some(snap.clone());
                    ws.fleet_view.update(cx, |v, cx| {
                        v.set_snapshot(snap);
                        cx.notify();
                    });
                    ws.push_runtime_status_to_fleet(cx);
                    ws.focus_pending_fleet_job(cx);
                }
                Err(e) => ws.fleet_view.update(cx, |v, cx| {
                    v.set_error(crate::i18n::api_error_message(&e));
                    cx.notify();
                }),
            });
        })
        .detach();
    }

    pub(super) fn push_runtime_status_to_fleet(&mut self, cx: &mut Context<Self>) {
        let enabled = self.app_config.jean_runtime.enabled;
        let my_id = self
            .runtime_instance
            .as_ref()
            .map(|i| i.id.clone())
            .or_else(|| self.app_config.jean_runtime.instance_id.clone());
        let status = if !enabled {
            "désactivé".to_string()
        } else {
            let base = self
                .runtime_instance
                .as_ref()
                .map(|i| i.status.clone())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "démarrage…".to_string());
            format!(
                "{} · {}",
                base,
                self.app_config.jean_runtime.executor.self_report_label()
            )
        };
        let awaiting = self.runtime_awaiting.clone();
        self.fleet_view.update(cx, |v, cx| {
            v.set_runtime(enabled, my_id, status);
            v.set_awaiting(awaiting);
            cx.notify();
        });
        self.focus_pending_fleet_job(cx);
    }

    pub(super) fn focus_pending_fleet_job(&mut self, cx: &mut Context<Self>) {
        let Some(job_id) = self.pending_fleet_job_focus.clone() else {
            return;
        };
        let opened = self
            .fleet_view
            .update(cx, |view, cx| view.open_job_by_id(&job_id, cx));
        if opened {
            self.pending_fleet_job_focus = None;
        }
    }

    pub(super) fn sync_fleet_view_poll(&mut self, cx: &mut Context<Self>) {
        if self.fleet_visible() {
            self.refresh_fleet_view(cx);
            if self._fleet_view_poll.is_none() {
                let task = cx.spawn(async move |this, cx: &mut AsyncApp| loop {
                    cx.background_executor()
                        .timer(std::time::Duration::from_secs(10))
                        .await;
                    let keep = this
                        .update(cx, |ws, cx| {
                            if ws.fleet_visible() {
                                ws.refresh_fleet_view(cx);
                                true
                            } else {
                                false
                            }
                        })
                        .unwrap_or(false);
                    if !keep {
                        break;
                    }
                });
                self._fleet_view_poll = Some(task);
            }
        } else {
            self._fleet_view_poll = None;
        }
    }

    pub(super) fn handle_fleet_event(&mut self, event: FleetViewEvent, cx: &mut Context<Self>) {
        if !self.can_access_mode(AppMode::Dev) {
            return;
        }
        match event {
            FleetViewEvent::Refresh => self.refresh_fleet_view(cx),
            FleetViewEvent::ToggleRuntime => self.toggle_jean_runtime(cx),
            FleetViewEvent::ApproveJob(id) => self.approve_fleet_job(id, cx),
            FleetViewEvent::RejectJob(id) => self.reject_fleet_job(id, cx),
        }
    }

    /// Enable/disable THIS machine as a fleet runtime. Off by default; enabling
    /// starts the loop. The safety gate is that the loop only *executes* jobs
    /// when enabled AND the instance autonomy is "auto"; "confirm" needs a click.
    pub fn toggle_jean_runtime(&mut self, cx: &mut Context<Self>) {
        if !self.enter_dev_mode(cx) {
            return;
        }
        if self.fleet_base_token().is_none() {
            self.show_toast(
                t!("toast.jean.login_required_runtime").to_string(),
                ToastLevel::Warning,
                cx,
            );
            return;
        }
        let now = !self.app_config.jean_runtime.enabled;
        self.app_config.jean_runtime.enabled = now;
        if let Err(e) = self.app_config.save() {
            tracing::error!("Failed to save jean_runtime: {}", e);
        }
        if now {
            self.show_toast(
                t!("toast.jean.runtime_on").to_string(),
                ToastLevel::Success,
                cx,
            );
        } else {
            // Best-effort offline heartbeat, then clear local state.
            if let (Some((base, token)), Some(inst)) =
                (self.fleet_base_token(), self.runtime_instance.clone())
            {
                cx.background_executor()
                    .spawn(async move {
                        let _ = jean_fleet::heartbeat(
                            &base,
                            &token,
                            &inst.id,
                            "offline",
                            Some("désactivé"),
                            None,
                        );
                    })
                    .detach();
            }
            self.runtime_instance = None;
            self.runtime_awaiting.clear();
            self.runtime_busy = false;
            self.publish_tray_state(cx);
            self.show_toast(
                t!("toast.jean.runtime_off").to_string(),
                ToastLevel::Info,
                cx,
            );
        }
        self.sync_runtime_loop(cx);
        self.push_runtime_status_to_fleet(cx);
        cx.notify();
    }

    /// Start/stop the runtime loop from config + auth state.
    pub fn sync_runtime_loop(&mut self, cx: &mut Context<Self>) {
        let want = runtime_loop_requested(
            self.app_config.jean_runtime.enabled,
            self.fleet_base_token().is_some(),
        );
        if want {
            if self._runtime_loop.is_none() {
                let task = cx.spawn(async move |this, cx: &mut AsyncApp| {
                    loop {
                        let step = this
                            .update(cx, |ws, cx| ws.runtime_loop_step(cx))
                            .ok()
                            .flatten();
                        let Some(step) = step else {
                            break; // disabled / signed out → stop
                        };
                        match step {
                            RuntimeStep::Register(base, token, reg) => {
                                let r = cx
                                    .background_executor()
                                    .spawn(async move { jean_fleet::register(&base, &token, &reg) })
                                    .await;
                                let _ = this.update(cx, |ws, cx| ws.apply_register(r, cx));
                            }
                            RuntimeStep::HeartbeatOnly(base, token, id, version) => {
                                cx.background_executor()
                                    .spawn(async move {
                                        let _ = jean_fleet::heartbeat(
                                            &base,
                                            &token,
                                            &id,
                                            "online",
                                            None,
                                            Some(&version),
                                        );
                                    })
                                    .await;
                            }
                            RuntimeStep::Tick(tc) => {
                                let r = cx
                                    .background_executor()
                                    .spawn(async move {
                                        let exec = tc.runtime_config.job_executor();
                                        let timeout = tc.runtime_config.job_timeout();
                                        jean_fleet::runtime_tick(
                                            &tc.base,
                                            &tc.token,
                                            &tc.instance_id,
                                            &tc.workdir,
                                            &tc.model,
                                            &tc.autonomy,
                                            &tc.version,
                                            &exec,
                                            timeout,
                                        )
                                    })
                                    .await;
                                let _ = this.update(cx, |ws, cx| ws.apply_tick_result(r, cx));
                            }
                        }
                        cx.background_executor()
                            .timer(std::time::Duration::from_secs(15))
                            .await;
                    }
                });
                self._runtime_loop = Some(task);
            }
        } else {
            self._runtime_loop = None;
        }
    }

    /// Decide this loop iteration's action on the UI thread (keeps all the config
    /// reads + gating in one place). `None` = stop the loop.
    pub(super) fn runtime_loop_step(&mut self, _cx: &mut Context<Self>) -> Option<RuntimeStep> {
        if !self.app_config.jean_runtime.enabled {
            return None;
        }
        let (base, token) = self.fleet_base_token()?;
        let version = shelldeck_core::VERSION.to_string();

        if self.runtime_instance.is_none() {
            let reg = self.build_register()?;
            return Some(RuntimeStep::Register(base, token, reg));
        }
        let id = self.runtime_instance.as_ref().unwrap().id.clone();
        // Concurrency 1: while a job runs / awaits confirmation, just heartbeat.
        if self.runtime_busy {
            return Some(RuntimeStep::HeartbeatOnly(base, token, id, version));
        }
        let (workdir, model) = self.runtime_workdir_model();
        let autonomy = self.runtime_instance.as_ref().unwrap().autonomy.clone();
        Some(RuntimeStep::Tick(RuntimeTickCtx {
            base,
            token,
            instance_id: id,
            workdir,
            model,
            autonomy,
            version,
            runtime_config: self.app_config.jean_runtime.clone(),
        }))
    }

    pub(super) fn apply_register(
        &mut self,
        r: shelldeck_core::Result<JeanInstance>,
        cx: &mut Context<Self>,
    ) {
        match r {
            Ok(inst) => {
                self.app_config.jean_runtime.instance_id = Some(inst.id.clone());
                if let Err(e) = self.app_config.save() {
                    tracing::error!("Failed to persist runtime instance id: {}", e);
                }
                self.runtime_instance = Some(inst);
                self.push_runtime_status_to_fleet(cx);
            }
            Err(e) => {
                self.fleet_view.update(cx, |v, cx| {
                    v.set_error(
                        t!(
                            "toast.jean.register_failed",
                            error = crate::i18n::api_error_message(&e)
                        )
                        .to_string(),
                    );
                    cx.notify();
                });
            }
        }
    }

    pub(super) fn apply_tick_result(
        &mut self,
        result: shelldeck_core::Result<jean_fleet::TickResult>,
        cx: &mut Context<Self>,
    ) {
        match result {
            Ok(tick) => {
                if let Some(job) = tick.awaiting_confirm {
                    if !self.runtime_awaiting.iter().any(|j| j.id == job.id) {
                        let job_id = job.id.clone();
                        let prompt = job.prompt.clone();
                        self.runtime_awaiting.push(job);
                        self.add_activity_entry(
                            ActivityEntry::new(
                                ActivityKind::Fleet,
                                t!("activity.fleet.awaiting").to_string(),
                            )
                            .with_target(job_id, t!("activity.fleet.job").to_string())
                            .with_detail(prompt)
                            .with_action(ActivityAction::OpenFleet),
                            cx,
                        );
                        self.publish_tray_state(cx);
                    }
                    self.runtime_busy = true;
                    self.show_toast(
                        t!("toast.jean.ticket_awaiting").to_string(),
                        ToastLevel::Warning,
                        cx,
                    );
                }
                self.push_runtime_status_to_fleet(cx);
            }
            Err(e) => {
                self.fleet_view.update(cx, |v, cx| {
                    v.set_error(crate::i18n::api_error_message(&e));
                    cx.notify();
                });
            }
        }
    }

    /// Approve a confirm-mode job: execute it now (running → done/failed).
    pub(super) fn approve_fleet_job(&mut self, job_id: String, cx: &mut Context<Self>) {
        let Some(job) = self
            .runtime_awaiting
            .iter()
            .find(|j| j.id == job_id)
            .cloned()
        else {
            return;
        };
        let Some((base, token)) = self.fleet_base_token() else {
            return;
        };
        let (workdir, model) = self.runtime_workdir_model();
        let runtime_config = self.app_config.jean_runtime.clone();
        self.runtime_awaiting.retain(|j| j.id != job_id);
        self.publish_tray_state(cx);
        // busy stays true through execution.
        self.push_runtime_status_to_fleet(cx);
        self.add_activity_entry(
            ActivityEntry::new(
                ActivityKind::Fleet,
                t!("activity.fleet.running").to_string(),
            )
            .with_target(job.id.clone(), t!("activity.fleet.job").to_string())
            .with_detail(job.prompt.clone())
            .with_action(ActivityAction::OpenFleet),
            cx,
        );
        self.show_toast(
            t!("toast.jean.ticket_running").to_string(),
            ToastLevel::Info,
            cx,
        );
        let job_id_for_activity = job.id.clone();
        let prompt_for_activity = job.prompt.clone();
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let r = cx
                .background_executor()
                .spawn(async move {
                    let exec = runtime_config.job_executor();
                    let timeout = runtime_config.job_timeout();
                    jean_fleet::execute_job(&base, &token, &job, &workdir, &model, &exec, timeout)
                })
                .await;
            let _ = this.update(cx, |ws, cx| {
                let success = r.is_ok();
                if let Err(e) = r {
                    ws.add_activity_entry(
                        ActivityEntry::new(
                            ActivityKind::Fleet,
                            t!("activity.fleet.failed").to_string(),
                        )
                        .with_target(
                            job_id_for_activity.clone(),
                            t!("activity.fleet.job").to_string(),
                        )
                        .with_detail(prompt_for_activity.clone())
                        .with_action(ActivityAction::OpenFleet),
                        cx,
                    );
                    ws.show_toast(
                        t!(
                            "toast.jean.ticket_failed",
                            error = crate::i18n::api_error_message(&e)
                        )
                        .to_string(),
                        ToastLevel::Error,
                        cx,
                    );
                } else {
                    ws.add_activity_entry(
                        ActivityEntry::new(
                            ActivityKind::Fleet,
                            t!("activity.fleet.done").to_string(),
                        )
                        .with_target(
                            job_id_for_activity.clone(),
                            t!("activity.fleet.job").to_string(),
                        )
                        .with_detail(prompt_for_activity.clone())
                        .with_action(ActivityAction::OpenFleet),
                        cx,
                    );
                    ws.show_toast(
                        t!("toast.jean.ticket_done").to_string(),
                        ToastLevel::Success,
                        cx,
                    );
                }
                // Notify the OS whether the job succeeded — the user
                // may have switched away from the ShellDeck window
                // while the executor was running. Muted from Settings
                // → Général via `AppConfig.tray.notify_fleet_done`.
                if ws.app_config.tray.notify_fleet_done {
                    ws.emit_tray_notification(TrayNotification::FleetJobDone { success });
                }
                ws.runtime_busy = false; // free for the next claim
                ws.push_runtime_status_to_fleet(cx);
                ws.refresh_fleet_view(cx);
            });
        })
        .detach();
    }

    /// Reject a confirm-mode job: mark it cancelled server-side.
    pub(super) fn reject_fleet_job(&mut self, job_id: String, cx: &mut Context<Self>) {
        let Some((base, token)) = self.fleet_base_token() else {
            return;
        };
        let prompt = self
            .runtime_awaiting
            .iter()
            .find(|j| j.id == job_id)
            .map(|j| j.prompt.clone())
            .unwrap_or_default();
        self.runtime_awaiting.retain(|j| j.id != job_id);
        self.runtime_busy = false;
        self.publish_tray_state(cx);
        self.push_runtime_status_to_fleet(cx);
        self.add_activity_entry(
            ActivityEntry::new(
                ActivityKind::Fleet,
                t!("activity.fleet.rejected").to_string(),
            )
            .with_target(job_id.clone(), t!("activity.fleet.job").to_string())
            .with_detail(prompt)
            .with_action(ActivityAction::OpenFleet),
            cx,
        );
        let jid = job_id;
        cx.background_executor()
            .spawn(async move {
                let _ = jean_fleet::update_job(
                    &base,
                    &token,
                    &jid,
                    "cancelled",
                    Some("rejeté depuis ShellDeck"),
                );
            })
            .detach();
        self.refresh_fleet_view(cx);
        cx.notify();
    }

    /// Open the Fleet view (palette / action) in Dev mode.
    pub fn open_fleet(&mut self, cx: &mut Context<Self>) {
        if !self.enter_dev_mode(cx) {
            return;
        }
        if self.fleet_base_token().is_none() {
            self.show_toast(
                t!("toast.jean.login_required_fleet").to_string(),
                ToastLevel::Warning,
                cx,
            );
            return;
        }
        self.active_view = ActiveView::Fleet;
        self.on_active_view_changed(cx);
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::runtime_loop_requested;

    // SDTEST-272 — the Jean runtime must never start implicitly.
    #[test]
    fn runtime_loop_requested_requires_explicit_enablement_and_credentials() {
        assert!(!runtime_loop_requested(false, false));
        assert!(!runtime_loop_requested(false, true));
        assert!(!runtime_loop_requested(true, false));
        assert!(runtime_loop_requested(true, true));
    }
}
