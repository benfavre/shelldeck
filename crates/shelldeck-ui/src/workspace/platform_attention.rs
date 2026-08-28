use super::*;

use shelldeck_core::config::platform::PlatformAttentionLoad;
use shelldeck_core::config::platform_attention::{
    resolve_platform_attention_activation, AttentionApplyOutcome, AttentionError,
    AttentionLocalKey, AttentionUnavailableReason, NotificationReservation,
    PlatformAttentionActivation, PlatformAttentionBoard, PlatformAttentionDestination,
};
use shelldeck_core::config::workspace_catalog::CatalogWorkspaceId;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PlatformAttentionPresentation {
    pub activation: PlatformAttentionActivation,
    pub state: shelldeck_core::config::platform_attention::AttentionItemState,
    pub reason: shelldeck_core::config::platform_attention::AttentionItemReason,
    pub unread: bool,
    pub nested_agent_path: Vec<String>,
}

/// Same-process only notification payload. No URI/deep-link or cold-launch
/// contract is encoded in this type.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlatformAttentionNotification {
    pub activation: PlatformAttentionActivation,
    pub summary: String,
    pub body: String,
    pub action_label: String,
}

impl Workspace {
    pub fn set_platform_attention_notifier(
        &mut self,
        notifier: Box<dyn Fn(PlatformAttentionNotification) + Send + Sync>,
    ) {
        self.platform_attention_notifier = Some(notifier);
    }

    pub(super) fn apply_platform_attention_load(
        &mut self,
        workspace: CatalogWorkspaceId,
        load: PlatformAttentionLoad,
        cx: &mut Context<Self>,
    ) {
        let prior_sources = self
            .platform_attention_boards
            .get(&workspace)
            .map(|board| board.inventory().sources().to_vec())
            .unwrap_or_default();
        let board = self
            .platform_attention_boards
            .entry(workspace)
            .or_insert_with(|| PlatformAttentionBoard::new(load.inventory.clone()));
        if let Err(error) = board.replace_inventory(load.inventory) {
            tracing::warn!(%error, %workspace, "Platform attention inventory rejected");
            return;
        }
        if let Some(store) = self.platform_attention_local.as_mut() {
            for removed in prior_sources
                .iter()
                .filter(|source| !board.inventory().contains(source))
            {
                if let Err(error) = store.remove_source(removed) {
                    tracing::warn!(%error, "Platform attention removed-source custody not persisted");
                }
            }
        }
        self.platform_attention_resync
            .retain(|(id, source)| *id != workspace || board.inventory().contains(source));

        let mut notifications = Vec::new();
        for (source, read) in load.reads {
            let initial = board.retained_snapshot(&source).is_none();
            let explicit_resync = self
                .platform_attention_resync
                .contains(&(workspace, source.clone()));
            let outcome = if initial || explicit_resync {
                board.apply_authenticated_baseline_read(&source, read)
            } else {
                let outcome = board.apply_read(&source, read);
                if matches!(outcome, Err(AttentionError::InvalidSuccessor)) {
                    let _ = board
                        .mark_unavailable(&source, AttentionUnavailableReason::InventoryIncomplete);
                    self.platform_attention_resync
                        .insert((workspace, source.clone()));
                }
                outcome
            };
            let accepted = matches!(
                outcome,
                Ok(AttentionApplyOutcome::Inserted
                    | AttentionApplyOutcome::Replaced
                    | AttentionApplyOutcome::ExactReplay
                    | AttentionApplyOutcome::AvailabilityRestored)
            );
            if !accepted {
                if let Err(error) = outcome {
                    tracing::warn!(%error, "Platform attention source replacement rejected");
                }
                continue;
            }
            self.platform_attention_resync
                .remove(&(workspace, source.clone()));

            let current = board
                .retained_snapshot(&source)
                .map(|snapshot| {
                    snapshot
                        .items()
                        .iter()
                        .map(|item| {
                            AttentionLocalKey::new(
                                shelldeck_core::config::platform_attention::AttentionItemKey::new(
                                    source.clone(),
                                    item.id().clone(),
                                ),
                                item.revision(),
                            )
                        })
                        .collect()
                })
                .unwrap_or_default();
            let Some(store) = self.platform_attention_local.as_mut() else {
                continue;
            };
            if let Err(error) = store.reconcile_source(&source, current) {
                tracing::warn!(%error, "Platform attention custody reconciliation failed");
                continue;
            }
            for item in board
                .visible_items()
                .filter(|item| item.key().source() == &source)
            {
                let local_key = AttentionLocalKey::new(item.key().clone(), item.value().revision());
                let eligible = item.value().unread() && !store.state().is_read(&local_key);
                match store.reserve_notification(local_key, eligible) {
                    Ok(NotificationReservation::Reserved) => {
                        notifications.push(PlatformAttentionNotification {
                            activation: PlatformAttentionActivation {
                                workspace,
                                item: item.ui_id(),
                                item_revision: item.value().revision(),
                            },
                            summary: t!("notification.attention.summary").to_string(),
                            body: attention_reason_label(item.value().reason()),
                            action_label: t!("notification.attention.open").to_string(),
                        })
                    }
                    Ok(
                        NotificationReservation::Ineligible
                        | NotificationReservation::AlreadyReserved,
                    ) => {}
                    Err(error) => {
                        tracing::warn!(%error, "Platform attention notification suppressed because reservation failed");
                    }
                }
            }
        }
        self.sync_platform_attention_presentations(cx);
        if let Some(notifier) = self.platform_attention_notifier.as_ref() {
            for notification in notifications {
                notifier(notification);
            }
        }
    }

    pub(super) fn clear_platform_attention(&mut self, cx: &mut Context<Self>) {
        self.platform_attention_boards.clear();
        self.platform_attention_resync.clear();
        self.sync_platform_attention_presentations(cx);
    }

    pub(super) fn mark_platform_attention_unavailable(
        &mut self,
        workspace: CatalogWorkspaceId,
        target: &shelldeck_core::config::platform_attention::PlatformAttentionTarget,
        cx: &mut Context<Self>,
    ) {
        let Some(board) = self.platform_attention_boards.get_mut(&workspace) else {
            return;
        };
        if board.inventory().target() != target {
            return;
        }
        for source in board.inventory().sources().to_vec() {
            let _ = board.mark_unavailable(&source, AttentionUnavailableReason::Transport);
        }
        self.sync_platform_attention_presentations(cx);
    }

    fn sync_platform_attention_presentations(&mut self, cx: &mut Context<Self>) {
        let mut rows = BTreeMap::<CatalogWorkspaceId, Vec<PlatformAttentionPresentation>>::new();
        for (workspace, board) in &self.platform_attention_boards {
            let items = board
                .visible_items()
                .map(|item| {
                    let key = AttentionLocalKey::new(item.key().clone(), item.value().revision());
                    PlatformAttentionPresentation {
                        activation: PlatformAttentionActivation {
                            workspace: *workspace,
                            item: item.ui_id(),
                            item_revision: item.value().revision(),
                        },
                        state: item.value().state(),
                        reason: item.value().reason(),
                        unread: item.value().unread()
                            && self
                                .platform_attention_local
                                .as_ref()
                                .is_none_or(|store| !store.state().is_read(&key)),
                        nested_agent_path: item
                            .value()
                            .nested_agent_path()
                            .iter()
                            .map(|id| id.as_str().to_owned())
                            .collect(),
                    }
                })
                .collect();
            rows.insert(*workspace, items);
        }
        self.workspace_hub.update(cx, |hub, cx| {
            hub.set_platform_attention(rows.clone(), cx);
        });
        self.fleet_view.update(cx, |fleet, cx| {
            fleet.set_platform_attention(rows.into_values().flatten().collect());
            cx.notify();
        });
    }

    /// Re-resolve one same-process activation against current state and mark
    /// its exact revision read only after a destination was successfully
    /// opened.
    pub fn activate_platform_attention(
        &mut self,
        activation: PlatformAttentionActivation,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(board) = self.platform_attention_boards.get(&activation.workspace) else {
            return false;
        };
        let Some(snapshot) = self.fleet_snapshot.as_ref() else {
            return false;
        };
        let destination = self.workspace_hub.update(cx, |hub, _| {
            resolve_platform_attention_activation(
                activation,
                board,
                hub.catalog(),
                hub.navigation(),
                &snapshot.sessions,
            )
        });
        let Ok(destination) = destination else {
            return false;
        };
        let Some(local) = board
            .item_by_ui_id(activation.item)
            .map(|item| AttentionLocalKey::new(item.key().clone(), activation.item_revision))
        else {
            return false;
        };
        let opened = match destination {
            PlatformAttentionDestination::WorkspaceAttention { workspace } => {
                self.active_view = ActiveView::Workspaces;
                self.workspace_hub.update(cx, |hub, cx| {
                    hub.open_platform_attention_surface(workspace, cx)
                })
            }
            PlatformAttentionDestination::FleetSession {
                workspace: _,
                session,
            } => {
                self.open_fleet(cx);
                self.fleet_view
                    .update(cx, |view, _| view.open_session_exact(&session))
            }
            PlatformAttentionDestination::RetainedProviderPane {
                workspace,
                session,
                focus,
            } => self.workspace_hub.update(cx, |hub, cx| {
                hub.open_retained_provider_pane(workspace, &session, focus, cx)
            }),
        };
        if !opened {
            return false;
        }
        let Some(store) = self.platform_attention_local.as_mut() else {
            return false;
        };
        if store.record_read(local).is_err() {
            return false;
        }
        self.sync_platform_attention_presentations(cx);
        true
    }
}

pub(crate) fn attention_reason_label(
    reason: shelldeck_core::config::platform_attention::AttentionItemReason,
) -> String {
    t!(match reason {
        shelldeck_core::config::platform_attention::AttentionItemReason::ReviewRequested =>
            "platform_attention.reason.review_requested",
        shelldeck_core::config::platform_attention::AttentionItemReason::CommentReply =>
            "platform_attention.reason.comment_reply",
        shelldeck_core::config::platform_attention::AttentionItemReason::ApprovalRequired =>
            "platform_attention.reason.approval_required",
        shelldeck_core::config::platform_attention::AttentionItemReason::AgentWorking =>
            "platform_attention.reason.agent_working",
        shelldeck_core::config::platform_attention::AttentionItemReason::CheckRunning =>
            "platform_attention.reason.check_running",
        shelldeck_core::config::platform_attention::AttentionItemReason::DeliveryPending =>
            "platform_attention.reason.delivery_pending",
        shelldeck_core::config::platform_attention::AttentionItemReason::Complete =>
            "platform_attention.reason.complete",
        shelldeck_core::config::platform_attention::AttentionItemReason::Conflict =>
            "platform_attention.reason.conflict",
        shelldeck_core::config::platform_attention::AttentionItemReason::CheckFailed =>
            "platform_attention.reason.check_failed",
        shelldeck_core::config::platform_attention::AttentionItemReason::ExternalBlocker =>
            "platform_attention.reason.external_blocker",
    })
    .to_string()
}

pub(crate) fn platform_attention_state_label(
    state: shelldeck_core::config::platform_attention::AttentionItemState,
) -> String {
    match state {
        shelldeck_core::config::platform_attention::AttentionItemState::NeedsYou => {
            t!("workspaces.attention.needs_you")
        }
        shelldeck_core::config::platform_attention::AttentionItemState::Working => {
            t!("workspaces.attention.working")
        }
        shelldeck_core::config::platform_attention::AttentionItemState::Done => {
            t!("workspaces.attention.done")
        }
        shelldeck_core::config::platform_attention::AttentionItemState::Blocked => {
            t!("workspaces.attention.blocked")
        }
    }
    .to_string()
}
