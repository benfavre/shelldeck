use super::*;

use shelldeck_core::config::platform::{
    stable_client_id, ActionResult, Attachment, ControlClaimResult, PlatformConnection,
    PlatformFollowUpResult, PlatformRefresh, PlatformSnapshot, ResourceCoordinate,
    RetainedSessionUpdate,
};
use shelldeck_core::config::platform_review::{
    PlatformReviewLoad, PlatformReviewTarget, PlatformReviewUnavailable,
};

enum PlatformActionResult {
    Attached(Attachment),
    Detached(ResourceCoordinate, Option<Attachment>),
    ControlClaimed(ControlClaimResult),
    ControlReleased(ResourceCoordinate),
    Executed(ActionResult),
    FollowedUp(PlatformFollowUpResult),
}

enum PlatformLoadResult {
    Snapshot {
        snapshot: PlatformSnapshot,
        review: Option<AttributedPlatformReview>,
    },
    Refresh {
        refresh: PlatformRefresh,
        retained: Vec<RetainedSessionUpdate>,
        reconciled: Vec<PlatformFollowUpResult>,
        review: Option<AttributedPlatformReview>,
    },
}

struct AttributedPlatformReview {
    target: PlatformReviewTarget,
    load: PlatformReviewLoad,
}

fn load_review(
    connection: &PlatformConnection,
    target: Option<PlatformReviewTarget>,
) -> Option<AttributedPlatformReview> {
    target.map(|target| {
        let load = connection
            .review(&target)
            .unwrap_or_else(|error| unavailable_review(&error));
        AttributedPlatformReview { target, load }
    })
}

/// Admit a remote observation only when it is still attributed to the exact
/// active catalog mapping that requested it. A switched or now-unmapped
/// workspace clears the strip instead of inheriting foreign review state.
fn review_for_active_target(
    active: Option<&PlatformReviewTarget>,
    attributed: Option<AttributedPlatformReview>,
) -> Option<PlatformReviewLoad> {
    attributed
        .and_then(|attributed| (Some(&attributed.target) == active).then_some(attributed.load))
}

fn platform_connection_is_current(
    current: Option<&PlatformConnection>,
    captured: &PlatformConnection,
) -> bool {
    current == Some(captured)
}

fn review_for_active_context(
    current_connection: Option<&PlatformConnection>,
    captured_connection: &PlatformConnection,
    active_target: Option<&PlatformReviewTarget>,
    attributed: Option<AttributedPlatformReview>,
) -> Option<PlatformReviewLoad> {
    platform_connection_is_current(current_connection, captured_connection)
        .then(|| review_for_active_target(active_target, attributed))
        .flatten()
}

fn unavailable_review(error: &shelldeck_core::error::ShellDeckError) -> PlatformReviewLoad {
    PlatformReviewLoad::Unavailable(PlatformReviewUnavailable {
        category: "transport_error".to_owned(),
        explanation: error.to_string(),
    })
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
        let request_connection = connection.clone();
        self.fleet_refresh_in_flight = true;
        let request_epoch = self.fleet_request_epoch;
        self.fleet_view.update(cx, |view, cx| {
            view.set_loading(true);
            cx.notify();
        });
        let previous = self.fleet_snapshot.clone();
        let attachments = self.fleet_view.update(cx, |view, _cx| view.attachments());
        let retained_reads = self
            .fleet_view
            .update(cx, |view, _cx| view.retained_reads());
        let pending_follow_ups = self
            .fleet_view
            .update(cx, |view, _cx| view.pending_follow_ups());
        let review_target = self.workspace_hub.read(cx).active_platform_review_target();
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    if let Some(previous) = previous.as_ref() {
                        let refresh = connection.refresh(previous, &attachments)?;
                        let retained = connection.refresh_retained_sessions(&retained_reads)?;
                        let reconciled = pending_follow_ups
                            .into_iter()
                            .map(|follow_up| connection.reconcile_follow_up(follow_up))
                            .collect();
                        let review = load_review(&connection, review_target);
                        Ok(PlatformLoadResult::Refresh {
                            refresh,
                            retained,
                            reconciled,
                            review,
                        })
                    } else {
                        let snapshot = connection.snapshot()?;
                        let review = load_review(&connection, review_target);
                        Ok(PlatformLoadResult::Snapshot { snapshot, review })
                    }
                })
                .await;
            let _ = this.update(cx, |workspace, cx| {
                if workspace.fleet_request_epoch != request_epoch {
                    return;
                }
                workspace.fleet_refresh_in_flight = false;
                let current_connection = workspace.platform_connection();
                if !platform_connection_is_current(current_connection.as_ref(), &request_connection)
                {
                    // Endpoint or credential replacement invalidates every
                    // observation from the captured connection, not only the
                    // review strip. Never relabel one Platform origin as
                    // another when project/workspace IDs happen to match.
                    workspace.fleet_snapshot = None;
                    workspace.fleet_view.update(cx, |view, cx| {
                        view.reset();
                        cx.notify();
                    });
                    return;
                }
                let active_review_target = workspace
                    .workspace_hub
                    .read(cx)
                    .active_platform_review_target();
                match result {
                    Ok(PlatformLoadResult::Snapshot { snapshot, review }) => {
                        let review = review_for_active_context(
                            current_connection.as_ref(),
                            &request_connection,
                            active_review_target.as_ref(),
                            review,
                        );
                        workspace.fleet_snapshot = Some(snapshot.clone());
                        workspace.fleet_view.update(cx, |view, cx| {
                            view.set_snapshot(snapshot);
                            view.set_review(review);
                            cx.notify();
                        });
                        workspace.focus_pending_fleet_session(cx);
                    }
                    Ok(PlatformLoadResult::Refresh {
                        refresh,
                        retained,
                        reconciled,
                        review,
                    }) => {
                        let review = review_for_active_context(
                            current_connection.as_ref(),
                            &request_connection,
                            active_review_target.as_ref(),
                            review,
                        );
                        workspace.fleet_snapshot = Some(refresh.snapshot.clone());
                        workspace.fleet_view.update(cx, |view, cx| {
                            view.apply_refresh(refresh);
                            for result in reconciled {
                                view.set_follow_up_result(result, cx);
                            }
                            view.apply_retained_updates(retained);
                            view.set_review(review);
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
        let follow_up_already_started = matches!(event, FleetViewEvent::FollowUp(_));
        let started = self.fleet_view.update(cx, |view, cx| {
            let started = follow_up_already_started || view.begin_operation();
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
                        FleetViewEvent::FollowUp(follow_up) => {
                            Ok(PlatformActionResult::FollowedUp(
                                connection.follow_up_reconciled(follow_up),
                            ))
                        }
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
                            view.set_attached(attachment, cx);
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
                    Ok(PlatformActionResult::FollowedUp(result)) => {
                        if let PlatformFollowUpResult::Receipt { receipt, .. } = &result {
                            if let Some(snapshot) = workspace.fleet_snapshot.as_mut() {
                                snapshot.view.apply_receipt(receipt.clone());
                                snapshot.resources = snapshot.view.resources().cloned().collect();
                            }
                        }
                        workspace.fleet_view.update(cx, |view, cx| {
                            view.set_follow_up_result(result, cx);
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

#[cfg(test)]
mod tests {
    use super::{
        platform_connection_is_current, review_for_active_context, review_for_active_target,
        AttributedPlatformReview,
    };
    use shelldeck_core::config::platform::PlatformConnection;
    use shelldeck_core::config::platform_review::{
        PlatformReviewLoad, PlatformReviewTarget, PlatformReviewUnavailable,
    };
    use shelldeck_core::config::workspace_catalog::{
        PlatformContextRef, PlatformMappingReconciliation, PlatformV2Mapping,
    };

    fn target(project: &str, workspace: &str) -> PlatformReviewTarget {
        PlatformReviewTarget::from_exact_mapping(&PlatformV2Mapping {
            reconciliation_revision: 1,
            project: PlatformContextRef {
                id: project.to_owned(),
                revision: 1,
            },
            checkout: PlatformContextRef {
                id: "checkout-1".to_owned(),
                revision: 1,
            },
            user_workspace: PlatformContextRef {
                id: workspace.to_owned(),
                revision: 1,
            },
            reconciliation: PlatformMappingReconciliation::Exact {
                reconciled_at_millis: 1,
            },
        })
        .unwrap()
    }

    fn refused(target: PlatformReviewTarget) -> AttributedPlatformReview {
        AttributedPlatformReview {
            target,
            load: PlatformReviewLoad::Unavailable(PlatformReviewUnavailable {
                category: "not_available".to_owned(),
                explanation: "review projection is not available".to_owned(),
            }),
        }
    }

    // SDTEST-1776
    #[test]
    fn review_apply_rechecks_target_and_preserves_same_target_refusal() {
        let requested = target("project-1", "wc_user_1");
        let switched = target("project-1", "wc_user_2");

        let admitted = review_for_active_target(Some(&requested), Some(refused(requested.clone())));
        assert!(matches!(admitted, Some(PlatformReviewLoad::Unavailable(_))));

        assert!(
            review_for_active_target(Some(&switched), Some(refused(requested.clone()))).is_none()
        );
        assert!(review_for_active_target(None, Some(refused(requested))).is_none());
    }

    // SDTEST-1777
    #[test]
    fn review_apply_rejects_changed_endpoint_or_credential_without_debugging_tokens() {
        let captured = PlatformConnection::new_at_endpoint(
            "https://manage-one.example/api/manage/automonique/platform",
            "captured-secret-token",
        )
        .unwrap();
        let same = captured.clone();
        let changed_endpoint = PlatformConnection::new_at_endpoint(
            "https://manage-two.example/api/manage/automonique/platform",
            "captured-secret-token",
        )
        .unwrap();
        let changed_credential = PlatformConnection::new_at_endpoint(
            "https://manage-one.example/api/manage/automonique/platform",
            "replacement-secret-token",
        )
        .unwrap();

        assert!(platform_connection_is_current(Some(&same), &captured));
        assert!(!platform_connection_is_current(
            Some(&changed_endpoint),
            &captured
        ));
        assert!(!platform_connection_is_current(
            Some(&changed_credential),
            &captured
        ));
        assert!(!platform_connection_is_current(None, &captured));

        let exact_target = target("project-1", "wc_user_1");
        assert!(review_for_active_context(
            Some(&changed_endpoint),
            &captured,
            Some(&exact_target),
            Some(refused(exact_target.clone())),
        )
        .is_none());
        assert!(review_for_active_context(
            Some(&changed_credential),
            &captured,
            Some(&exact_target),
            Some(refused(exact_target.clone())),
        )
        .is_none());

        let debug = format!("{captured:?} {changed_credential:?}");
        assert!(debug.contains("<redacted>"));
        assert!(!debug.contains("captured-secret-token"));
        assert!(!debug.contains("replacement-secret-token"));
    }
}
