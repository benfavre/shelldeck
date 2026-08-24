use super::*;

use shelldeck_core::config::platform::{
    stable_client_id, ActionResult, Attachment, ControlClaimResult, PlatformConnection,
    PlatformRefresh, PlatformSnapshot, ResourceCoordinate,
};

enum PlatformActionResult {
    Attached(Attachment),
    Detached(ResourceCoordinate, Option<Attachment>),
    ControlClaimed(ControlClaimResult),
    ControlReleased(ResourceCoordinate),
    Executed(ActionResult),
}

enum PlatformLoadResult {
    Snapshot(PlatformSnapshot),
    Refresh(PlatformRefresh),
}

impl Workspace {
    /// Signed-in AI Operations origin and bearer used by non-platform Manage APIs.
    pub(super) fn manage_base_token(&self) -> Option<(String, String)> {
        self.signed_in().then(|| {
            (
                self.account_base_url(),
                self.app_config.cloud_sync.token.clone(),
            )
        })
    }

    pub(super) fn platform_connection(&self) -> Option<PlatformConnection> {
        if !self.signed_in() {
            return None;
        }
        let manage_origin = self
            .site_directory
            .as_ref()
            .map(|directory| directory.manage_origin.trim())
            .filter(|origin| !origin.is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| self.account_base_url());
        let endpoint = format!(
            "{}/api/manage/automonique/platform",
            manage_origin.trim_end_matches('/')
        );
        PlatformConnection::new_at_endpoint(&endpoint, &self.app_config.cloud_sync.token).ok()
    }

    pub(super) fn fleet_visible(&self) -> bool {
        !self.settings_open
            && self.platform_connection().is_some()
            && self.effective_mode() == AppMode::Dev
            && self.active_view == ActiveView::Fleet
    }

    pub(super) fn update_fleet_availability(&mut self, cx: &mut Context<Self>) {
        let show = self.platform_connection().is_some() && self.effective_mode() == AppMode::Dev;
        self.sidebar.update(cx, |sidebar, cx| {
            sidebar.set_fleet_available(show);
            cx.notify();
        });
    }

    pub(super) fn refresh_fleet_view(&mut self, cx: &mut Context<Self>) {
        if self.fleet_refresh_in_flight {
            return;
        }
        if !self.fleet_view.update(cx, |view, _cx| view.can_refresh()) {
            return;
        }
        let Some(connection) = self.platform_connection() else {
            return;
        };
        self.fleet_refresh_in_flight = true;
        let request_epoch = self.fleet_request_epoch;
        self.fleet_view.update(cx, |view, cx| {
            view.set_loading(true);
            cx.notify();
        });
        let previous = self.fleet_snapshot.clone();
        let attachments = self.fleet_view.update(cx, |view, _cx| view.attachments());
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    if let Some(previous) = previous.as_ref() {
                        connection
                            .refresh(previous, &attachments)
                            .map(PlatformLoadResult::Refresh)
                    } else {
                        connection.snapshot().map(PlatformLoadResult::Snapshot)
                    }
                })
                .await;
            let _ = this.update(cx, |workspace, cx| {
                if workspace.fleet_request_epoch != request_epoch {
                    return;
                }
                workspace.fleet_refresh_in_flight = false;
                if workspace.platform_connection().is_none() {
                    return;
                }
                match result {
                    Ok(PlatformLoadResult::Snapshot(snapshot)) => {
                        workspace.fleet_snapshot = Some(snapshot.clone());
                        workspace.fleet_view.update(cx, |view, cx| {
                            view.set_snapshot(snapshot);
                            cx.notify();
                        });
                        workspace.focus_pending_fleet_session(cx);
                    }
                    Ok(PlatformLoadResult::Refresh(refresh)) => {
                        workspace.fleet_snapshot = Some(refresh.snapshot.clone());
                        workspace.fleet_view.update(cx, |view, cx| {
                            view.apply_refresh(refresh);
                            cx.notify();
                        });
                        workspace.focus_pending_fleet_session(cx);
                    }
                    Err(error) => workspace.fleet_view.update(cx, |view, cx| {
                        view.set_error(crate::i18n::api_error_message(&error));
                        cx.notify();
                    }),
                }
            });
        })
        .detach();
    }

    pub(super) fn focus_pending_fleet_session(&mut self, cx: &mut Context<Self>) {
        let Some(session_id) = self.pending_fleet_session_focus.clone() else {
            return;
        };
        let opened = self
            .fleet_view
            .update(cx, |view, _cx| view.open_session_by_id(&session_id));
        if opened {
            self.pending_fleet_session_focus = None;
        }
    }

    pub(super) fn sync_fleet_view_poll(&mut self, cx: &mut Context<Self>) {
        if self.fleet_visible() {
            self.refresh_fleet_view(cx);
            if self._fleet_view_poll.is_none() {
                self._fleet_view_poll = Some(cx.spawn(async move |this, cx: &mut AsyncApp| loop {
                    cx.background_executor()
                        .timer(std::time::Duration::from_secs(2))
                        .await;
                    let keep = this
                        .update(cx, |workspace, cx| {
                            if workspace.fleet_visible() {
                                workspace.refresh_fleet_view(cx);
                                true
                            } else {
                                false
                            }
                        })
                        .unwrap_or(false);
                    if !keep {
                        break;
                    }
                }));
            }
        } else {
            self._fleet_view_poll = None;
        }
    }

    pub(super) fn handle_fleet_event(&mut self, event: FleetViewEvent, cx: &mut Context<Self>) {
        if !self.can_access_mode(AppMode::Dev) {
            return;
        }
        if matches!(event, FleetViewEvent::Refresh) {
            self.refresh_fleet_view(cx);
            return;
        }
        let Some(connection) = self.platform_connection() else {
            return;
        };
        let Ok(client) = stable_client_id(&shelldeck_core::util::hostname()) else {
            return;
        };
        let detached_attachment = match &event {
            FleetViewEvent::Detach(session) => self
                .fleet_view
                .update(cx, |view, _cx| view.attachment(session)),
            _ => None,
        };
        let started = self.fleet_view.update(cx, |view, cx| {
            let started = view.begin_operation();
            cx.notify();
            started
        });
        if !started {
            return;
        }
        let request_epoch = self.fleet_request_epoch;
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    match event {
                        FleetViewEvent::Refresh => unreachable!(),
                        FleetViewEvent::Attach(session) => connection
                            .attach(session, client)
                            .map(PlatformActionResult::Attached),
                        FleetViewEvent::Detach(session) => connection
                            .detach(session.clone(), client)
                            .map(|()| PlatformActionResult::Detached(session, detached_attachment)),
                        FleetViewEvent::ClaimControl(session) => connection
                            .claim_control(session, client)
                            .map(PlatformActionResult::ControlClaimed),
                        FleetViewEvent::ReleaseControl(session, lease) => connection
                            .release_control(session.clone(), client, lease.id)
                            .map(|()| PlatformActionResult::ControlReleased(session)),
                        FleetViewEvent::Execute(preview) => connection
                            .execute_reconciled(preview)
                            .map(PlatformActionResult::Executed),
                    }
                })
                .await;
            let _ = this.update(cx, |workspace, cx| {
                if workspace.fleet_request_epoch != request_epoch || !workspace.signed_in() {
                    return;
                }
                match result {
                    Ok(PlatformActionResult::Attached(attachment)) => {
                        if let Some(snapshot) = workspace.fleet_snapshot.as_mut() {
                            snapshot.view.track_attachment(&attachment);
                        }
                        workspace.fleet_view.update(cx, |view, cx| {
                            view.set_attached(attachment);
                            view.set_loading(false);
                            cx.notify();
                        });
                    }
                    Ok(PlatformActionResult::Detached(session, attachment)) => {
                        if let Some(attachment) = attachment {
                            if let Some(snapshot) = workspace.fleet_snapshot.as_mut() {
                                snapshot.view.forget_attachment(&attachment);
                            }
                        }
                        workspace.fleet_view.update(cx, |view, cx| {
                            view.set_detached(&session);
                            view.set_loading(false);
                            cx.notify();
                        });
                    }
                    Ok(PlatformActionResult::ControlClaimed(result)) => {
                        workspace.fleet_view.update(cx, |view, cx| {
                            view.set_control_claim_result(result);
                            view.set_loading(false);
                            cx.notify();
                        });
                    }
                    Ok(PlatformActionResult::ControlReleased(session)) => {
                        workspace.fleet_view.update(cx, |view, cx| {
                            view.set_control_released(&session);
                            view.set_loading(false);
                            cx.notify();
                        });
                    }
                    Ok(PlatformActionResult::Executed(result)) => {
                        if let ActionResult::Receipt(receipt) = &result {
                            if let Some(snapshot) = workspace.fleet_snapshot.as_mut() {
                                snapshot.view.apply_receipt(receipt.clone());
                                snapshot.resources = snapshot.view.resources().cloned().collect();
                            }
                        }
                        workspace.fleet_view.update(cx, |view, cx| {
                            view.set_action_result(result);
                            cx.notify();
                        });
                    }
                    Err(error) => workspace.fleet_view.update(cx, |view, cx| {
                        view.set_operation_error(crate::i18n::api_error_message(&error));
                        cx.notify();
                    }),
                }
            });
        })
        .detach();
    }

    pub fn open_fleet(&mut self, cx: &mut Context<Self>) {
        if !self.enter_dev_mode(cx) {
            return;
        }
        if self.platform_connection().is_none() {
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
