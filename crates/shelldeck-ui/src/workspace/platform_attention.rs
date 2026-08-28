use super::*;

use shelldeck_core::config::platform::PlatformAttentionLoad;
use shelldeck_core::config::platform_attention::{
    resolve_platform_attention_activation, AttentionApplyOutcome, AttentionError,
    AttentionLocalKey, AttentionUnavailableReason, NotificationReservation,
    PlatformAttentionActivation, PlatformAttentionBoard, PlatformAttentionDestination,
};
use shelldeck_core::config::workspace_catalog::CatalogWorkspaceId;

fn apply_attention_read_with_resync(
    board: &mut PlatformAttentionBoard,
    resync: &mut BTreeSet<(
        CatalogWorkspaceId,
        shelldeck_core::config::platform_attention::AttentionSource,
    )>,
    workspace: CatalogWorkspaceId,
    source: &shelldeck_core::config::platform_attention::AttentionSource,
    read: shelldeck_core::config::platform::AttentionReadResult,
) -> Result<AttentionApplyOutcome, AttentionError> {
    let initial = board.retained_snapshot(source).is_none();
    let explicit_resync = resync.contains(&(workspace, source.clone()));
    let outcome = if initial || explicit_resync {
        board.apply_authenticated_baseline_read(source, read)
    } else {
        let outcome = board.apply_read(source, read);
        if matches!(outcome, Err(AttentionError::InvalidSuccessor)) {
            let _ = board.mark_unavailable(source, AttentionUnavailableReason::InventoryIncomplete);
            resync.insert((workspace, source.clone()));
        }
        outcome
    };
    if matches!(
        outcome,
        Ok(AttentionApplyOutcome::Inserted
            | AttentionApplyOutcome::Replaced
            | AttentionApplyOutcome::ExactReplay
            | AttentionApplyOutcome::AvailabilityRestored)
    ) {
        resync.remove(&(workspace, source.clone()));
    }
    outcome
}

fn mark_attention_boards_unavailable(
    boards: &mut BTreeMap<CatalogWorkspaceId, PlatformAttentionBoard>,
) {
    for board in boards.values_mut() {
        for source in board.inventory().sources().to_vec() {
            let _ = board.mark_unavailable(&source, AttentionUnavailableReason::Transport);
        }
    }
}

fn retire_attention_board_state(
    boards: &mut BTreeMap<CatalogWorkspaceId, PlatformAttentionBoard>,
    resync: &mut BTreeSet<(
        CatalogWorkspaceId,
        shelldeck_core::config::platform_attention::AttentionSource,
    )>,
    retired_sources: &mut BTreeSet<shelldeck_core::config::platform_attention::AttentionSource>,
    workspace: CatalogWorkspaceId,
) -> bool {
    let Some(board) = boards.remove(&workspace) else {
        return false;
    };
    retired_sources.extend(board.inventory().sources().iter().cloned());
    resync.retain(|(id, _)| *id != workspace);
    true
}

const fn platform_attention_destination_view(
    destination: &PlatformAttentionDestination,
) -> ActiveView {
    match destination {
        PlatformAttentionDestination::FleetSession { .. } => ActiveView::Fleet,
        PlatformAttentionDestination::WorkspaceAttention { .. }
        | PlatformAttentionDestination::RetainedProviderPane { .. } => ActiveView::Workspaces,
    }
}

fn platform_attention_surface_verified(
    settings_open: bool,
    active_view: ActiveView,
    required_view: ActiveView,
) -> bool {
    !settings_open && active_view == required_view
}

fn attention_context_requires_retirement(
    board: &PlatformAttentionBoard,
    target: Option<&shelldeck_core::config::platform_attention::PlatformAttentionTarget>,
) -> bool {
    target != Some(board.inventory().target())
}

fn record_attention_read_after_visible_open(
    store: Option<&mut shelldeck_core::config::platform_attention::AttentionLocalStateStore>,
    local: AttentionLocalKey,
    opened: bool,
) -> bool {
    if !opened {
        return false;
    }
    store.is_some_and(|store| store.record_read(local).is_ok())
}

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
        if self
            .platform_attention_boards
            .get(&workspace)
            .is_some_and(|board| {
                attention_context_requires_retirement(board, Some(load.inventory.target()))
            })
        {
            self.retire_platform_attention_board(workspace);
        }
        if !self.flush_retired_platform_attention_sources() {
            self.sync_platform_attention_presentations(cx);
            return;
        }
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
            let outcome = apply_attention_read_with_resync(
                board,
                &mut self.platform_attention_resync,
                workspace,
                &source,
                read,
            );
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

    fn retire_platform_attention_board(&mut self, workspace: CatalogWorkspaceId) {
        retire_attention_board_state(
            &mut self.platform_attention_boards,
            &mut self.platform_attention_resync,
            &mut self.platform_attention_retired_sources,
            workspace,
        );
    }

    fn flush_retired_platform_attention_sources(&mut self) -> bool {
        let Some(store) = self.platform_attention_local.as_mut() else {
            // With no local store there is no overlay state to inherit.
            self.platform_attention_retired_sources.clear();
            return true;
        };
        for source in self.platform_attention_retired_sources.clone() {
            match store.remove_source(&source) {
                Ok(_) => {
                    self.platform_attention_retired_sources.remove(&source);
                }
                Err(error) => {
                    tracing::warn!(%error, "Platform attention retired-source custody not persisted");
                }
            }
        }
        self.platform_attention_retired_sources.is_empty()
    }

    /// Retire a board immediately when the active local workspace loses its
    /// exact mapping, or when that mapping now names a different Platform
    /// target. A later valid load creates a fresh board without requiring a
    /// process restart.
    pub(super) fn reconcile_platform_attention_context(
        &mut self,
        context: Option<(
            CatalogWorkspaceId,
            Option<shelldeck_core::config::platform_attention::PlatformAttentionTarget>,
        )>,
        cx: &mut Context<Self>,
    ) {
        let Some((workspace, target)) = context else {
            return;
        };
        let retire = self
            .platform_attention_boards
            .get(&workspace)
            .is_some_and(|board| attention_context_requires_retirement(board, target.as_ref()));
        if retire {
            self.retire_platform_attention_board(workspace);
            let _ = self.flush_retired_platform_attention_sources();
            self.sync_platform_attention_presentations(cx);
        }
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

    pub(super) fn mark_all_platform_attention_unavailable(&mut self, cx: &mut Context<Self>) {
        mark_attention_boards_unavailable(&mut self.platform_attention_boards);
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
        if !self.enter_dev_mode(cx) {
            return false;
        }
        let required_view = platform_attention_destination_view(&destination);
        let opened = match destination {
            PlatformAttentionDestination::WorkspaceAttention { workspace } => {
                self.active_view = required_view;
                self.on_active_view_changed(cx);
                let opened = self.workspace_hub.update(cx, |hub, cx| {
                    hub.open_platform_attention_surface(workspace, cx)
                });
                opened
                    && platform_attention_surface_verified(
                        self.settings_open,
                        self.active_view,
                        required_view,
                    )
            }
            PlatformAttentionDestination::FleetSession {
                workspace: _,
                session,
            } => {
                self.open_fleet(cx)
                    && self
                        .fleet_view
                        .update(cx, |view, _| view.open_session_exact(&session))
                    && platform_attention_surface_verified(
                        self.settings_open,
                        self.active_view,
                        required_view,
                    )
            }
            PlatformAttentionDestination::RetainedProviderPane {
                workspace,
                session,
                focus,
            } => {
                self.active_view = required_view;
                self.on_active_view_changed(cx);
                let opened = self.workspace_hub.update(cx, |hub, cx| {
                    hub.open_retained_provider_pane(workspace, &session, focus, cx)
                });
                opened
                    && platform_attention_surface_verified(
                        self.settings_open,
                        self.active_view,
                        required_view,
                    )
            }
        };
        if !record_attention_read_after_visible_open(
            self.platform_attention_local.as_mut(),
            local,
            opened,
        ) {
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

#[cfg(test)]
mod tests {
    use super::{
        apply_attention_read_with_resync, attention_context_requires_retirement,
        mark_attention_boards_unavailable, platform_attention_destination_view,
        platform_attention_surface_verified, record_attention_read_after_visible_open,
        retire_attention_board_state,
    };
    use crate::workspace::ActiveView;
    use crate::{
        ai_assistant::AiAssistantView,
        workspace::{Workspace, WorkspaceAiBindings},
    };
    use automonique_protocol::platform_v2::{
        CheckoutId, WorkContextAttributes, WorkContextIdentity, WorkContextLabel,
        WorkContextLifecycle, WorkContextRecord, WorkContextRelation, WorkContextRelationKind,
    };
    use automonique_protocol::platform_v2_attention::{
        AttentionItem, AttentionItemId, AttentionItemReason, AttentionItemState, AttentionSourceId,
        AttentionSourceKind, AttentionSourceSnapshot,
    };
    use automonique_protocol::primitives::Revision;
    use gpui::AppContext;
    use shelldeck_core::config::platform::{
        AttentionReadResult, ResourceAuthority, ResourceCoordinate, ResourceId, ResourceKind,
    };
    use shelldeck_core::config::platform_attention::{
        AttentionApplyOutcome, AttentionError, AttentionItemKey, AttentionLocalKey,
        AttentionLocalStateStore, AttentionSourceInventory, AttentionSourceStatus,
        AttentionUnavailableReason, PlatformAttentionBoard, PlatformAttentionDestination,
        PlatformAttentionTarget, ReviewAttentionPresence,
    };
    use shelldeck_core::config::workspace_catalog::CatalogWorkspaceId;
    use shelldeck_core::workspace_navigation::{PaneId, WorkspaceFocus, WorkspaceTabId};
    use shelldeck_core::{
        ai::{AiConfig, AiContext, AiSurface, ClippyConfig},
        config::{
            app_config::AppConfig,
            cloud_account::{AccountInfo, AppMode},
            store::ConnectionStore,
        },
    };
    use std::collections::{BTreeMap, BTreeSet};
    use std::{cell::RefCell, rc::Rc};
    use uuid::Uuid;

    fn relation(kind: WorkContextRelationKind, target: WorkContextIdentity) -> WorkContextRelation {
        WorkContextRelation::new(kind, target).unwrap()
    }

    fn inventory(project: &str, workspace: &str) -> AttentionSourceInventory {
        let project_id = automonique_protocol::platform_v2::ProjectId::new(project).unwrap();
        let workspace_id =
            automonique_protocol::platform_v2::UserWorkspaceId::new(workspace).unwrap();
        let record = WorkContextRecord::new(
            WorkContextIdentity::UserWorkspace(workspace_id.clone()),
            Revision::FIRST,
            WorkContextLifecycle::Active,
            WorkContextLabel::new("workspace").unwrap(),
            WorkContextAttributes::EMPTY,
            vec![
                relation(
                    WorkContextRelationKind::UserWorkspaceProject,
                    WorkContextIdentity::Project(project_id.clone()),
                ),
                relation(
                    WorkContextRelationKind::UserWorkspaceCheckout,
                    WorkContextIdentity::Checkout(CheckoutId::new("checkout-1").unwrap()),
                ),
            ],
        )
        .unwrap();
        AttentionSourceInventory::from_authoritative_records(
            PlatformAttentionTarget {
                project: project_id,
                user_workspace: workspace_id,
            },
            &[record],
            ReviewAttentionPresence::Absent,
        )
        .unwrap()
    }

    fn source(workspace: &str) -> shelldeck_core::config::platform_attention::AttentionSource {
        shelldeck_core::config::platform_attention::AttentionSource::new(
            AttentionSourceKind::Orchestration,
            AttentionSourceId::new(workspace).unwrap(),
        )
    }

    fn snapshot(
        project: &str,
        workspace: &str,
        revision: u64,
        previous: Option<u64>,
    ) -> AttentionSourceSnapshot {
        AttentionSourceSnapshot::new(
            source(workspace),
            automonique_protocol::platform_v2::ProjectId::new(project).unwrap(),
            automonique_protocol::platform_v2::UserWorkspaceId::new(workspace).unwrap(),
            Revision::new(revision).unwrap(),
            previous.map(|value| Revision::new(value).unwrap()),
            revision * 100,
            vec![AttentionItem::new(
                AttentionItemId::new("item-1").unwrap(),
                Revision::new(revision).unwrap(),
                revision * 100,
                AttentionItemState::Blocked,
                AttentionItemReason::ExternalBlocker,
                true,
                Vec::new(),
                None,
            )
            .unwrap()],
        )
        .unwrap()
    }

    // SDTEST-1826
    #[test]
    fn workspace_attention_gap_hides_the_source_until_the_next_poll_baseline() {
        let workspace = CatalogWorkspaceId::from_uuid(Uuid::from_u128(1826));
        let source = source("workspace-1");
        let mut board = PlatformAttentionBoard::new(inventory("project-1", "workspace-1"));
        let mut resync = BTreeSet::new();
        assert_eq!(
            apply_attention_read_with_resync(
                &mut board,
                &mut resync,
                workspace,
                &source,
                AttentionReadResult::Snapshot(Box::new(snapshot(
                    "project-1",
                    "workspace-1",
                    1,
                    None,
                ))),
            )
            .unwrap(),
            AttentionApplyOutcome::Inserted
        );
        assert!(matches!(
            apply_attention_read_with_resync(
                &mut board,
                &mut resync,
                workspace,
                &source,
                AttentionReadResult::Snapshot(Box::new(snapshot(
                    "project-1",
                    "workspace-1",
                    3,
                    Some(2),
                ))),
            ),
            Err(AttentionError::InvalidSuccessor)
        ));
        assert!(board.visible_items().next().is_none());
        assert!(resync.contains(&(workspace, source.clone())));

        assert_eq!(
            apply_attention_read_with_resync(
                &mut board,
                &mut resync,
                workspace,
                &source,
                AttentionReadResult::Snapshot(Box::new(snapshot(
                    "project-1",
                    "workspace-1",
                    3,
                    Some(2),
                ))),
            )
            .unwrap(),
            AttentionApplyOutcome::Replaced
        );
        assert!(!resync.contains(&(workspace, source)));
        assert_eq!(board.visible_items().count(), 1);
    }

    // SDTEST-1827
    #[test]
    fn whole_poll_failure_hides_every_board_and_mapping_replacement_recovers_cleanly() {
        let workspace = CatalogWorkspaceId::from_uuid(Uuid::from_u128(1827));
        let source_one = source("workspace-1");
        let mut board = PlatformAttentionBoard::new(inventory("project-1", "workspace-1"));
        board
            .apply_authenticated_baseline_read(
                &source_one,
                AttentionReadResult::Snapshot(Box::new(snapshot(
                    "project-1",
                    "workspace-1",
                    1,
                    None,
                ))),
            )
            .unwrap();
        let mut boards = BTreeMap::from([(workspace, board)]);
        assert!(!attention_context_requires_retirement(
            &boards[&workspace],
            Some(boards[&workspace].inventory().target()),
        ));
        assert!(attention_context_requires_retirement(
            &boards[&workspace],
            None,
        ));
        let remapped_target = inventory("project-2", "workspace-2");
        assert!(attention_context_requires_retirement(
            &boards[&workspace],
            Some(remapped_target.target()),
        ));
        mark_attention_boards_unavailable(&mut boards);
        assert!(boards[&workspace].visible_items().next().is_none());
        assert_eq!(
            boards[&workspace].status(&source_one),
            Some(&AttentionSourceStatus::Unavailable(
                AttentionUnavailableReason::Transport
            ))
        );

        let mut resync = BTreeSet::from([(workspace, source_one.clone())]);
        let mut retired = BTreeSet::new();
        assert!(retire_attention_board_state(
            &mut boards,
            &mut resync,
            &mut retired,
            workspace,
        ));
        assert!(!boards.contains_key(&workspace));
        assert!(resync.is_empty());
        assert!(retired.contains(&source_one));

        let replacement = PlatformAttentionBoard::new(inventory("project-2", "workspace-2"));
        boards.insert(workspace, replacement);
        assert_eq!(
            boards[&workspace].inventory().target(),
            &PlatformAttentionTarget {
                project: automonique_protocol::platform_v2::ProjectId::new("project-2").unwrap(),
                user_workspace: automonique_protocol::platform_v2::UserWorkspaceId::new(
                    "workspace-2"
                )
                .unwrap(),
            }
        );
    }

    // SDTEST-1828
    #[test]
    fn activation_routes_each_destination_to_a_visible_surface_before_read_custody() {
        let workspace = CatalogWorkspaceId::from_uuid(Uuid::from_u128(1828));
        let session = ResourceCoordinate::new(
            ResourceAuthority::Automonique,
            ResourceKind::Session,
            ResourceId::new("session-1").unwrap(),
        );
        let focus = WorkspaceFocus {
            pane_id: PaneId::from_uuid(Uuid::from_u128(1)),
            tab_id: WorkspaceTabId::from_uuid(Uuid::from_u128(2)),
        };
        for destination in [
            PlatformAttentionDestination::WorkspaceAttention { workspace },
            PlatformAttentionDestination::RetainedProviderPane {
                workspace,
                session: session.clone(),
                focus,
            },
        ] {
            assert_eq!(
                platform_attention_destination_view(&destination),
                ActiveView::Workspaces
            );
        }
        assert_eq!(
            platform_attention_destination_view(&PlatformAttentionDestination::FleetSession {
                workspace,
                session,
            }),
            ActiveView::Fleet
        );
        assert!(platform_attention_surface_verified(
            false,
            ActiveView::Workspaces,
            ActiveView::Workspaces,
        ));
        assert!(!platform_attention_surface_verified(
            true,
            ActiveView::Workspaces,
            ActiveView::Workspaces,
        ));
        assert!(!platform_attention_surface_verified(
            false,
            ActiveView::Fleet,
            ActiveView::Workspaces,
        ));

        let temp = tempfile::tempdir().unwrap();
        let mut store = AttentionLocalStateStore::open(
            temp.path().join("private").join("state").join("local.json"),
            8,
        )
        .unwrap();
        let local = AttentionLocalKey::new(
            AttentionItemKey::new(
                source("workspace-1"),
                AttentionItemId::new("item-1").unwrap(),
            ),
            Revision::FIRST,
        );
        assert!(!record_attention_read_after_visible_open(
            Some(&mut store),
            local.clone(),
            false,
        ));
        assert!(!store.state().is_read(&local));
        assert!(record_attention_read_after_visible_open(
            Some(&mut store),
            local.clone(),
            true,
        ));
        assert!(store.state().is_read(&local));
    }

    // SDTEST-1829
    #[test]
    fn attention_mode_admission_closes_settings_and_stages_dev_mode() {
        let mut cx = gpui::TestAppContext::single();
        let workspace = cx.update(|cx| {
            let assistant = cx.new(|cx| {
                AiAssistantView::new(
                    AiContext::new(AiSurface::Global, "test", serde_json::json!({})),
                    cx,
                )
            });
            let mut config = AppConfig::default();
            config.general.ui_font_family = crate::settings::normalize_ui_font_family("Inter", cx);
            config.account = Some(AccountInfo {
                email: "operator@example.test".into(),
                name: "Operator".into(),
                is_superadmin: true,
                is_admin: true,
                is_inklura_support: true,
                roles: vec!["superadmin".into()],
            });
            config.cloud_sync.enabled = true;
            config.cloud_sync.token = "fixture-sensitive-token".into();
            config.cloud_sync.base_url = "https://manage.example.test".into();
            config.cloud_sync.mode = AppMode::User;
            cx.new(|cx| {
                Workspace::new(
                    cx,
                    config,
                    Vec::new(),
                    ConnectionStore::default(),
                    WorkspaceAiBindings {
                        assistant,
                        tasks: Vec::new(),
                        config: Rc::new(RefCell::new(AiConfig::default())),
                        clippy_config: Rc::new(RefCell::new(ClippyConfig::default())),
                    },
                )
            })
        });
        workspace.update(&mut cx, |workspace, cx| {
            workspace.settings_open = true;
            assert_eq!(workspace.effective_mode(), AppMode::User);
            assert!(workspace.enter_dev_mode(cx));
            assert!(!workspace.settings_open);
            assert!(workspace.mode_transition.is_some());
        });
    }
}
