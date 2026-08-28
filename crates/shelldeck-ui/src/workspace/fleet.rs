use super::*;

use shelldeck_core::config::platform::{
    stable_client_id, ActionResult, Attachment, ControlClaimResult, PlatformConnection,
    PlatformFollowUpResult, PlatformRefresh, PlatformReviewActionResult, PlatformSnapshot,
    ResourceCoordinate, RetainedSessionUpdate,
};
use shelldeck_core::config::platform_review::{
    PlatformReviewCapabilitiesLoad, PlatformReviewLoad, PlatformReviewTarget,
    PlatformReviewUnavailable,
};

enum PlatformActionResult {
    Attached(Attachment),
    Detached(ResourceCoordinate, Option<Attachment>),
    ControlClaimed(ControlClaimResult),
    ControlReleased(ResourceCoordinate),
    Executed(ActionResult),
    Review(AttributedPlatformReviewActionResult),
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
        review_action: Option<Box<AttributedPlatformReviewActionResult>>,
    },
}

struct AttributedPlatformReview {
    target: PlatformReviewTarget,
    load: PlatformReviewLoad,
    capabilities: PlatformReviewCapabilitiesLoad,
}

struct AttributedPlatformReviewActionResult {
    target: PlatformReviewTarget,
    result: PlatformReviewActionResult,
}

fn load_review(
    connection: &PlatformConnection,
    target: Option<PlatformReviewTarget>,
) -> Option<AttributedPlatformReview> {
    target.map(|target| {
        let load = connection
            .review(&target)
            .unwrap_or_else(|error| unavailable_review(&error));
        let capabilities = connection
            .review_capabilities(&target)
            .unwrap_or_else(|error| unavailable_review_capabilities(&error));
        AttributedPlatformReview {
            target,
            load,
            capabilities,
        }
    })
}

/// Admit a remote observation only when it is still attributed to the exact
/// active catalog mapping that requested it. A switched or now-unmapped
/// workspace clears the strip instead of inheriting foreign review state.
#[cfg(test)]
fn review_for_active_target(
    active: Option<&PlatformReviewTarget>,
    attributed: Option<AttributedPlatformReview>,
) -> Option<PlatformReviewLoad> {
    attributed
        .and_then(|attributed| (Some(&attributed.target) == active).then_some(attributed.load))
}

fn review_and_capabilities_for_active_target(
    active: Option<&PlatformReviewTarget>,
    attributed: Option<AttributedPlatformReview>,
) -> Option<(PlatformReviewLoad, PlatformReviewCapabilitiesLoad)> {
    attributed.and_then(|attributed| {
        (Some(&attributed.target) == active).then_some((attributed.load, attributed.capabilities))
    })
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
) -> Option<(PlatformReviewLoad, PlatformReviewCapabilitiesLoad)> {
    platform_connection_is_current(current_connection, captured_connection)
        .then(|| review_and_capabilities_for_active_target(active_target, attributed))
        .flatten()
}

fn review_action_for_active_context(
    current_connection: Option<&PlatformConnection>,
    captured_connection: &PlatformConnection,
    active_target: Option<&PlatformReviewTarget>,
    attributed: AttributedPlatformReviewActionResult,
) -> Option<PlatformReviewActionResult> {
    (platform_connection_is_current(current_connection, captured_connection)
        && active_target == Some(&attributed.target)
        && attributed.result.preview().target() == &attributed.target)
        .then_some(attributed.result)
}

fn unavailable_review(error: &shelldeck_core::error::ShellDeckError) -> PlatformReviewLoad {
    PlatformReviewLoad::Unavailable(PlatformReviewUnavailable {
        category: "transport_error".to_owned(),
        explanation: error.to_string(),
    })
}

fn unavailable_review_capabilities(
    error: &shelldeck_core::error::ShellDeckError,
) -> PlatformReviewCapabilitiesLoad {
    PlatformReviewCapabilitiesLoad::Unavailable(PlatformReviewUnavailable {
        category: "transport_error".to_owned(),
        explanation: error.to_string(),
    })
}

const FLEET_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_secs(10);

fn fleet_retry_delay(consecutive_failures: u32) -> std::time::Duration {
    let exponent = consecutive_failures.saturating_sub(1).min(2);
    std::time::Duration::from_secs(30 * (1_u64 << exponent))
}

fn fleet_should_log_failure(consecutive_failures: u32) -> bool {
    consecutive_failures == 0
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
        let pending_review_action = self
            .fleet_view
            .update(cx, |view, _cx| view.pending_review_reconciliation());
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
                        let review_action = pending_review_action.map(|preview| {
                            let target = preview.target().clone();
                            Box::new(AttributedPlatformReviewActionResult {
                                target,
                                result: connection.reconcile_review_action(preview),
                            })
                        });
                        let review = load_review(&connection, review_target);
                        Ok(PlatformLoadResult::Refresh {
                            refresh,
                            retained,
                            reconciled,
                            review,
                            review_action,
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
                        let recovered = workspace.fleet_refresh_failures > 0;
                        workspace.fleet_refresh_failures = 0;
                        workspace.fleet_retry_not_before = None;
                        if recovered {
                            tracing::info!("Plateforme de nouveau joignable");
                        }
                        workspace.fleet_snapshot = Some(snapshot.clone());
                        workspace.fleet_view.update(cx, |view, cx| {
                            view.set_snapshot(snapshot);
                            let (review, capabilities) = review
                                .map(|(review, capabilities)| (Some(review), Some(capabilities)))
                                .unwrap_or((None, None));
                            view.set_review(active_review_target.clone(), review, capabilities);
                            cx.notify();
                        });
                        workspace.focus_pending_fleet_session(cx);
                    }
                    Ok(PlatformLoadResult::Refresh {
                        refresh,
                        retained,
                        reconciled,
                        review,
                        review_action,
                    }) => {
                        let review = review_for_active_context(
                            current_connection.as_ref(),
                            &request_connection,
                            active_review_target.as_ref(),
                            review,
                        );
                        let review_action = review_action.and_then(|result| {
                            review_action_for_active_context(
                                current_connection.as_ref(),
                                &request_connection,
                                active_review_target.as_ref(),
                                *result,
                            )
                        });
                        let recovered = workspace.fleet_refresh_failures > 0;
                        workspace.fleet_refresh_failures = 0;
                        workspace.fleet_retry_not_before = None;
                        if recovered {
                            tracing::info!("Plateforme de nouveau joignable");
                        }
                        workspace.fleet_snapshot = Some(refresh.snapshot.clone());
                        workspace.fleet_view.update(cx, |view, cx| {
                            view.apply_refresh(refresh);
                            for result in reconciled {
                                view.set_follow_up_result(result, cx);
                            }
                            view.apply_retained_updates(retained);
                            let (review, capabilities) = review
                                .map(|(review, capabilities)| (Some(review), Some(capabilities)))
                                .unwrap_or((None, None));
                            view.set_review(active_review_target.clone(), review, capabilities);
                            if let Some(result) = review_action {
                                view.set_review_action_result(result, cx);
                            }
                            cx.notify();
                        });
                        workspace.focus_pending_fleet_session(cx);
                    }
                    Err(error) => {
                        let log_failure =
                            fleet_should_log_failure(workspace.fleet_refresh_failures);
                        workspace.fleet_refresh_failures =
                            workspace.fleet_refresh_failures.saturating_add(1);
                        workspace.fleet_retry_not_before = Some(
                            std::time::Instant::now()
                                + fleet_retry_delay(workspace.fleet_refresh_failures),
                        );
                        let message = crate::i18n::platform_error_message(&error, log_failure);
                        workspace.fleet_view.update(cx, |view, cx| {
                            view.set_error(message);
                            cx.notify();
                        });
                    }
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
                    cx.background_executor().timer(FLEET_POLL_INTERVAL).await;
                    let keep = this
                        .update(cx, |workspace, cx| {
                            if workspace.fleet_visible() {
                                let retry_ready =
                                    workspace.fleet_retry_not_before.is_none_or(|not_before| {
                                        std::time::Instant::now() >= not_before
                                    });
                                if retry_ready {
                                    workspace.refresh_fleet_view(cx);
                                }
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
        let operation_already_started = matches!(
            event,
            FleetViewEvent::FollowUp(_) | FleetViewEvent::ExecuteReview(_)
        );
        let started = self.fleet_view.update(cx, |view, cx| {
            let started = operation_already_started || view.begin_operation();
            cx.notify();
            started
        });
        if !started {
            return;
        }
        let request_epoch = self.fleet_request_epoch;
        let request_connection = connection.clone();
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
                        FleetViewEvent::ExecuteReview(preview) => {
                            let target = preview.target().clone();
                            Ok(PlatformActionResult::Review(
                                AttributedPlatformReviewActionResult {
                                    target,
                                    result: connection.execute_review_action(preview),
                                },
                            ))
                        }
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
                    Ok(PlatformActionResult::Review(attributed)) => {
                        let current_connection = workspace.platform_connection();
                        let active_target = workspace
                            .workspace_hub
                            .read(cx)
                            .active_platform_review_target();
                        if let Some(result) = review_action_for_active_context(
                            current_connection.as_ref(),
                            &request_connection,
                            active_target.as_ref(),
                            attributed,
                        ) {
                            workspace.fleet_view.update(cx, |view, cx| {
                                view.set_review_action_result(result, cx);
                                cx.notify();
                            });
                            workspace.refresh_fleet_view(cx);
                        } else {
                            workspace.fleet_view.update(cx, |view, cx| {
                                view.reset_review_action_for_context_change();
                                cx.notify();
                            });
                        }
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
                        view.set_operation_error(crate::i18n::platform_error_message(&error, true));
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
        fleet_retry_delay, fleet_should_log_failure, platform_connection_is_current,
        review_action_for_active_context, review_for_active_context, review_for_active_target,
        AttributedPlatformReview, AttributedPlatformReviewActionResult, FLEET_POLL_INTERVAL,
    };
    use shelldeck_core::config::platform::{PlatformConnection, PlatformReviewActionResult};
    use shelldeck_core::config::platform_review::{
        AttentionState, ConflictState, DeliverySemantic, DeliveryState, DiffChangeKind, DiffSide,
        MergeReadiness, PlatformReviewActionPreview, PlatformReviewCapabilitiesLoad,
        PlatformReviewLoad, PlatformReviewSemantic, PlatformReviewTarget,
        PlatformReviewUnavailable, PreviewKind, PullRequestSemantic, PullRequestState,
        ReviewAnchorSemantic, ReviewAttentionSemantic, ReviewAuthorityKind,
        ReviewAuthoritySemantic, ReviewDecision, ReviewFileSemantic, ReviewFreshnessSemantic,
        ReviewFreshnessState, ReviewHunkSemantic, ReviewPreviewSemantic, ReviewSchemaVersion,
        ReviewStatusSemantic, WorktreeFileState,
    };
    use shelldeck_core::config::workspace_catalog::{
        PlatformContextRef, PlatformMappingReconciliation, PlatformV2Mapping,
    };
    use std::time::Duration;

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
            capabilities: PlatformReviewCapabilitiesLoad::Unavailable(PlatformReviewUnavailable {
                category: "not_available".to_owned(),
                explanation: "review capabilities are not available".to_owned(),
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

    // SDTEST-1782
    #[test]
    fn review_mutation_result_rechecks_connection_target_and_original_action_attribution() {
        let exact_target = target("project-1", "wc_user_1");
        let authority = |kind| ReviewAuthoritySemantic {
            kind,
            id: format!("{}-authority", kind.as_str()),
        };
        let freshness = ReviewFreshnessSemantic {
            state: ReviewFreshnessState::Fresh,
            observed_revision: shelldeck_core::config::platform::Revision::new(8).unwrap(),
            observed_at_ms: 1,
        };
        let semantic = PlatformReviewSemantic {
            schema: ReviewSchemaVersion::V2,
            workspace_kind: exact_target.workspace.kind(),
            workspace_id: exact_target.workspace.id().to_owned(),
            revision: shelldeck_core::config::platform::Revision::new(9).unwrap(),
            attention: ReviewAttentionSemantic {
                state: AttentionState::Idle,
                reason: None,
                source_revision: None,
                unread: 0,
                needs_user_action: false,
            },
            attention_events: Vec::new(),
            review: ReviewStatusSemantic {
                decision: ReviewDecision::Pending,
                authority: authority(ReviewAuthorityKind::Review),
                freshness,
            },
            checks: Vec::new(),
            pull_request: PullRequestSemantic {
                id: None,
                state: PullRequestState::Absent,
                readiness: MergeReadiness::Unknown,
                head_revision: None,
                authority: authority(ReviewAuthorityKind::PullRequest),
                freshness,
            },
            delivery: DeliverySemantic {
                id: None,
                state: DeliveryState::NotDelivered,
                authority: authority(ReviewAuthorityKind::Delivery),
                freshness,
            },
            files: vec![ReviewFileSemantic {
                id: "file-1".to_owned(),
                path: "src/main.rs".to_owned(),
                change: DiffChangeKind::Modified,
                worktree: WorktreeFileState::Unstaged,
                conflict: ConflictState::None,
                preview: ReviewPreviewSemantic {
                    kind: PreviewKind::Text,
                    media_type: Some("text/plain".to_owned()),
                    byte_size: Some(10),
                    width: None,
                    height: None,
                    sanitized: true,
                },
                hunks: vec![ReviewHunkSemantic {
                    id: "hunk-1".to_owned(),
                    old_start: 10,
                    old_lines: 2,
                    new_start: 11,
                    new_lines: 2,
                    preview: "+ exact".to_owned(),
                }],
            }],
            comments: Vec::new(),
            proposals: Vec::new(),
        };
        let preview = PlatformReviewActionPreview::add_comment(
            exact_target.clone(),
            &semantic,
            &ReviewAnchorSemantic {
                file_id: "file-1".to_owned(),
                hunk_id: "hunk-1".to_owned(),
                side: DiffSide::New,
                line: 11,
            },
            "Exact line comment.",
        )
        .unwrap();
        let original_key = preview.idempotency_key().clone();
        let attributed = || AttributedPlatformReviewActionResult {
            target: exact_target.clone(),
            result: PlatformReviewActionResult::ReconciliationPending {
                preview: preview.clone(),
                category: "transport_uncertain".to_owned(),
            },
        };
        let connection = PlatformConnection::new_at_endpoint(
            "https://manage.example/api/manage/automonique/platform",
            "captured-token",
        )
        .unwrap();

        let admitted = review_action_for_active_context(
            Some(&connection),
            &connection,
            Some(&exact_target),
            attributed(),
        )
        .unwrap();
        assert_eq!(admitted.preview().idempotency_key(), &original_key);
        assert!(admitted.requires_lookup());

        let changed_connection = PlatformConnection::new_at_endpoint(
            "https://manage.example/api/manage/automonique/platform",
            "replacement-token",
        )
        .unwrap();
        assert!(review_action_for_active_context(
            Some(&changed_connection),
            &connection,
            Some(&exact_target),
            attributed(),
        )
        .is_none());
        let switched = target("project-1", "wc_user_2");
        assert!(review_action_for_active_context(
            Some(&connection),
            &connection,
            Some(&switched),
            attributed(),
        )
        .is_none());

        let foreign_target = target("project-1", "wc_user_2");
        let foreign_attribution = AttributedPlatformReviewActionResult {
            target: foreign_target,
            result: attributed().result,
        };
        assert!(review_action_for_active_context(
            Some(&connection),
            &connection,
            Some(&exact_target),
            foreign_attribution,
        )
        .is_none());
    }

    // SDTEST-1799 — Platform polling must remain useful while healthy without
    // hammering or repeating diagnostics throughout one outage.
    #[test]
    fn platform_failure_policy_backs_off_and_logs_once_per_outage() {
        assert_eq!(FLEET_POLL_INTERVAL, Duration::from_secs(10));
        assert_eq!(fleet_retry_delay(1), Duration::from_secs(30));
        assert_eq!(fleet_retry_delay(2), Duration::from_secs(60));
        assert_eq!(fleet_retry_delay(3), Duration::from_secs(120));
        assert_eq!(fleet_retry_delay(20), Duration::from_secs(120));

        assert!(fleet_should_log_failure(0));
        assert!(!fleet_should_log_failure(1));
        assert!(!fleet_should_log_failure(20));
    }
}
