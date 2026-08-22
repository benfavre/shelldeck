use super::*;

#[cfg(test)]
fn runtime_loop_requested(_runtime_enabled: bool, _credentials_available: bool) -> bool {
    false
}

impl Workspace {
    // --- Fleet projection client ---

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
                .spawn(async move { monique_fleet::get_fleet(&base, &token) })
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
        let enabled = self.app_config.monique_runtime.enabled;
        let my_id = self
            .runtime_instance
            .as_ref()
            .map(|i| i.id.clone())
            .or_else(|| self.app_config.monique_runtime.instance_id.clone());
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
                self.app_config.monique_runtime.executor.self_report_label()
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
            FleetViewEvent::RejectJob(id) => self.reject_fleet_job(id, cx),
        }
    }

    /// Keep the retired runtime task stopped even when old config enabled it.
    pub fn sync_runtime_loop(&mut self, _cx: &mut Context<Self>) {
        self.app_config.monique_runtime.enabled = false;
        self._runtime_loop = None;
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
                let _ = monique_fleet::update_job(
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
                t!("toast.monique.login_required_fleet").to_string(),
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

    // SDTEST-272 — ShellDeck remains client-only even with stale runtime config.
    #[test]
    fn runtime_loop_is_never_requested_by_the_client() {
        assert!(!runtime_loop_requested(false, false));
        assert!(!runtime_loop_requested(false, true));
        assert!(!runtime_loop_requested(true, false));
        assert!(!runtime_loop_requested(true, true));
    }
}
