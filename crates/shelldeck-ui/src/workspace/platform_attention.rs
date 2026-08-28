use super::*;

use shelldeck_core::config::platform::PlatformAttentionLoad;
use shelldeck_core::config::platform_attention::{
    resolve_platform_attention_activation, AttentionApplyOutcome, AttentionError,
    AttentionLocalKey, AttentionRetirement, AttentionUnavailableReason, NotificationReservation,
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
    workspace: CatalogWorkspaceId,
) -> bool {
    if boards.remove(&workspace).is_none() {
        return false;
    }
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
    mode: AppMode,
    transitioning: bool,
    settings_open: bool,
    active_view: ActiveView,
    required_view: ActiveView,
) -> bool {
    mode == AppMode::Dev && !transitioning && !settings_open && active_view == required_view
}

fn attention_context_requires_retirement(
    board: &PlatformAttentionBoard,
    target: Option<&shelldeck_core::config::platform_attention::PlatformAttentionTarget>,
) -> bool {
    target != Some(board.inventory().target())
}

fn attention_context_retirements(
    boards: &BTreeMap<CatalogWorkspaceId, PlatformAttentionBoard>,
    context: Option<&(
        CatalogWorkspaceId,
        Option<shelldeck_core::config::platform_attention::PlatformAttentionTarget>,
    )>,
) -> Vec<CatalogWorkspaceId> {
    match context {
        None => boards.keys().copied().collect(),
        Some((workspace, target)) => boards
            .get(workspace)
            .filter(|board| attention_context_requires_retirement(board, target.as_ref()))
            .map(|_| vec![*workspace])
            .unwrap_or_default(),
    }
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
    const MAX_PENDING_PLATFORM_ATTENTION_ACTIVATIONS: usize = 64;

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
            && !self.retire_platform_attention_board(workspace)
        {
            let _ = self.flush_retired_platform_attention_sources();
            self.sync_platform_attention_presentations(cx);
            return;
        }
        let removed_sources = self
            .platform_attention_boards
            .get(&workspace)
            .map(|board| {
                board
                    .inventory()
                    .sources()
                    .iter()
                    .filter(|source| !load.inventory.contains(source))
                    .cloned()
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        if let Some(board) = self.platform_attention_boards.get(&workspace) {
            self.platform_attention_retirements_pending.extend(
                removed_sources.iter().cloned().map(|source| {
                    AttentionRetirement::new(board.inventory().target().clone(), source)
                }),
            );
        }
        if !self.flush_retired_platform_attention_sources() {
            if let Some(board) = self.platform_attention_boards.get_mut(&workspace) {
                for source in removed_sources {
                    let _ = board
                        .mark_unavailable(&source, AttentionUnavailableReason::InventoryIncomplete);
                }
            }
            self.sync_platform_attention_presentations(cx);
            return;
        }
        let board = self
            .platform_attention_boards
            .entry(workspace)
            .or_insert_with(|| PlatformAttentionBoard::new(load.inventory.clone()));
        if let Err(error) = board.replace_inventory(load.inventory) {
            tracing::warn!(%error, %workspace, "Platform attention inventory rejected");
            return;
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
                                board.inventory().target().clone(),
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
            if let Err(error) = store.reconcile_source(board.inventory().target(), &source, current)
            {
                tracing::warn!(%error, "Platform attention custody reconciliation failed");
                continue;
            }
            for item in board
                .visible_items()
                .filter(|item| item.key().source() == &source)
            {
                let local_key = AttentionLocalKey::new(
                    board.inventory().target().clone(),
                    item.key().clone(),
                    item.value().revision(),
                );
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
        self.platform_attention_pending_activations.clear();
        self.platform_attention_visible_confirmations.clear();
        for workspace in self
            .platform_attention_boards
            .keys()
            .copied()
            .collect::<Vec<_>>()
        {
            let _ = self.retire_platform_attention_board(workspace);
        }
        let _ = self.flush_retired_platform_attention_sources();
        self.sync_platform_attention_presentations(cx);
    }

    fn retire_platform_attention_board(&mut self, workspace: CatalogWorkspaceId) -> bool {
        if let Some(board) = self.platform_attention_boards.get(&workspace) {
            let target = board.inventory().target().clone();
            self.platform_attention_retirements_pending.extend(
                board
                    .inventory()
                    .sources()
                    .iter()
                    .cloned()
                    .map(|source| AttentionRetirement::new(target.clone(), source)),
            );
            let mut fenced = true;
            if let Some(store) = self.platform_attention_local.as_mut() {
                for retirement in self.platform_attention_retirements_pending.clone() {
                    if let Err(error) = store.begin_retirement(retirement) {
                        fenced = false;
                        tracing::warn!(%error, "Platform attention retirement fence not persisted");
                    }
                }
            }
            if !fenced {
                if let Some(board) = self.platform_attention_boards.get_mut(&workspace) {
                    for source in board.inventory().sources().to_vec() {
                        let _ = board.mark_unavailable(
                            &source,
                            AttentionUnavailableReason::InventoryIncomplete,
                        );
                    }
                }
                return false;
            }
        }
        retire_attention_board_state(
            &mut self.platform_attention_boards,
            &mut self.platform_attention_resync,
            workspace,
        )
    }

    fn flush_retired_platform_attention_sources(&mut self) -> bool {
        let Some(store) = self.platform_attention_local.as_mut() else {
            // With no local store there is no overlay state to inherit.
            self.platform_attention_retirements_pending.clear();
            return true;
        };
        for retirement in self.platform_attention_retirements_pending.clone() {
            match store.begin_retirement(retirement.clone()) {
                Ok(_) => {
                    self.platform_attention_retirements_pending
                        .remove(&retirement);
                }
                Err(error) => {
                    tracing::warn!(%error, "Platform attention retirement fence not persisted");
                }
            }
        }
        let durable = store.pending_retirements().clone();
        for retirement in durable {
            if let Err(error) = store.finish_retirement(&retirement) {
                tracing::warn!(%error, "Platform attention retired-source custody not removed");
            }
        }
        self.platform_attention_retirements_pending.is_empty()
            && store.pending_retirements().is_empty()
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
        let retire =
            attention_context_retirements(&self.platform_attention_boards, context.as_ref());
        if !retire.is_empty() {
            for workspace in retire {
                let _ = self.retire_platform_attention_board(workspace);
            }
            self.sync_platform_attention_presentations(cx);
        }
        let _ = self.flush_retired_platform_attention_sources();
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
                    let key = AttentionLocalKey::new(
                        board.inventory().target().clone(),
                        item.key().clone(),
                        item.value().revision(),
                    );
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

    fn resolve_current_platform_attention(
        &mut self,
        activation: PlatformAttentionActivation,
        cx: &mut Context<Self>,
    ) -> Option<(PlatformAttentionDestination, AttentionLocalKey)> {
        let board = self.platform_attention_boards.get(&activation.workspace)?;
        let snapshot = self.fleet_snapshot.as_ref()?;
        let destination = self.workspace_hub.update(cx, |hub, _| {
            resolve_platform_attention_activation(
                activation,
                board,
                hub.catalog(),
                hub.navigation(),
                &snapshot.sessions,
            )
            .ok()
        })?;
        let local = board.item_by_ui_id(activation.item).map(|item| {
            AttentionLocalKey::new(
                board.inventory().target().clone(),
                item.key().clone(),
                activation.item_revision,
            )
        })?;
        Some((destination, local))
    }

    /// Admit an exact same-process activation. Opening and read custody are
    /// deliberately separate UI turns: a real Dev transition must finish,
    /// then the destination is opened, then current authority and rendered
    /// visibility are checked again before the tuple becomes read.
    pub fn activate_platform_attention(
        &mut self,
        activation: PlatformAttentionActivation,
        cx: &mut Context<Self>,
    ) -> bool {
        if self
            .resolve_current_platform_attention(activation, cx)
            .is_none()
            || !self.can_access_mode(AppMode::Dev)
        {
            return false;
        }
        if self
            .platform_attention_pending_activations
            .iter()
            .chain(
                self.platform_attention_visible_confirmations
                    .iter()
                    .map(|(activation, _, _)| activation),
            )
            .any(|pending| *pending == activation)
        {
            return true;
        }
        if self.platform_attention_pending_activations.len()
            + self.platform_attention_visible_confirmations.len()
            == Self::MAX_PENDING_PLATFORM_ATTENTION_ACTIVATIONS
        {
            return false;
        }
        self.platform_attention_pending_activations
            .push_back(activation);
        self.drive_platform_attention_activations(cx);
        true
    }

    pub(super) fn drive_platform_attention_activations(&mut self, cx: &mut Context<Self>) {
        if self.platform_attention_pending_activations.is_empty()
            || !self.platform_attention_visible_confirmations.is_empty()
        {
            return;
        }
        if !self.can_access_mode(AppMode::Dev) {
            self.platform_attention_pending_activations.clear();
            return;
        }
        if self.mode_transition.is_some() {
            self.settings_open = false;
            return;
        }
        if self.effective_mode() != AppMode::Dev {
            let _ = self.enter_dev_mode(cx);
            return;
        }
        self.settings_open = false;
        let Some(activation) = self.platform_attention_pending_activations.pop_front() else {
            return;
        };
        let Some((destination, _)) = self.resolve_current_platform_attention(activation, cx) else {
            self.drive_platform_attention_activations(cx);
            return;
        };
        let required_view = platform_attention_destination_view(&destination);
        let opened = match &destination {
            PlatformAttentionDestination::WorkspaceAttention { workspace } => {
                self.active_view = required_view;
                self.on_active_view_changed(cx);
                let opened = self.workspace_hub.update(cx, |hub, cx| {
                    hub.open_platform_attention_surface(*workspace, cx)
                });
                opened
                    && platform_attention_surface_verified(
                        self.effective_mode(),
                        self.mode_transition.is_some(),
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
                        .update(cx, |view, _| view.open_session_exact(session))
                    && platform_attention_surface_verified(
                        self.effective_mode(),
                        self.mode_transition.is_some(),
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
                    hub.open_retained_provider_pane(*workspace, session, *focus, cx)
                });
                opened
                    && platform_attention_surface_verified(
                        self.effective_mode(),
                        self.mode_transition.is_some(),
                        self.settings_open,
                        self.active_view,
                        required_view,
                    )
            }
        };
        if !opened {
            self.drive_platform_attention_activations(cx);
            return;
        }
        self.platform_attention_visible_confirmations
            .push_back((activation, destination, 0));
        self.schedule_platform_attention_visibility_confirmation(cx);
    }

    fn schedule_platform_attention_visibility_confirmation(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(16))
                .await;
            let _ = this.update(cx, |workspace, cx| {
                workspace.confirm_visible_platform_attention_activation(cx);
            });
        })
        .detach();
    }

    fn confirm_visible_platform_attention_activation(&mut self, cx: &mut Context<Self>) {
        let Some((activation, opened_destination, attempt)) =
            self.platform_attention_visible_confirmations.pop_front()
        else {
            return;
        };
        let Some((current_destination, local)) =
            self.resolve_current_platform_attention(activation, cx)
        else {
            self.drive_platform_attention_activations(cx);
            return;
        };
        let required_view = platform_attention_destination_view(&current_destination);
        let common_visible = current_destination == opened_destination
            && platform_attention_surface_verified(
                self.effective_mode(),
                self.mode_transition.is_some(),
                self.settings_open,
                self.active_view,
                required_view,
            );
        let exact_visible = common_visible
            && match &current_destination {
                PlatformAttentionDestination::WorkspaceAttention { workspace } => self
                    .workspace_hub
                    .read(cx)
                    .platform_attention_surface_is_visible(*workspace),
                PlatformAttentionDestination::FleetSession { session, .. } => {
                    self.fleet_view.read(cx).session_is_open_exact(session)
                }
                PlatformAttentionDestination::RetainedProviderPane {
                    workspace,
                    session,
                    focus,
                } => self
                    .workspace_hub
                    .read(cx)
                    .retained_provider_pane_is_visible(*workspace, session, *focus, cx),
            };
        if !exact_visible && attempt < 60 {
            self.platform_attention_visible_confirmations.push_front((
                activation,
                opened_destination,
                attempt + 1,
            ));
            self.schedule_platform_attention_visibility_confirmation(cx);
            return;
        }
        if record_attention_read_after_visible_open(
            self.platform_attention_local.as_mut(),
            local,
            exact_visible,
        ) {
            self.sync_platform_attention_presentations(cx);
        }
        self.drive_platform_attention_activations(cx);
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
        attention_context_retirements, mark_attention_boards_unavailable,
        platform_attention_destination_view, platform_attention_surface_verified,
        record_attention_read_after_visible_open, retire_attention_board_state,
    };
    use crate::workspace::workspaces::WorkspaceHubView;
    use crate::workspace::ActiveView;
    use crate::{
        ai_assistant::AiAssistantView,
        terminal_view::TerminalView,
        workspace::{Workspace, WorkspaceAiBindings},
    };
    use automonique_protocol::platform::{Capabilities, CursorTopic, PlatformCursor};
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
        AttentionReadResult, PlatformSnapshot, PlatformView, ResourceAuthority, ResourceCoordinate,
        ResourceId, ResourceKind,
    };
    use shelldeck_core::config::platform_attention::{
        AttentionApplyOutcome, AttentionError, AttentionItemKey, AttentionLocalKey,
        AttentionLocalStateStore, AttentionRetirement, AttentionSourceInventory,
        AttentionSourceStatus, AttentionUnavailableReason, PlatformAttentionActivation,
        PlatformAttentionBoard, PlatformAttentionDestination, PlatformAttentionTarget,
        ReviewAttentionPresence,
    };
    use shelldeck_core::config::workspace_catalog::{
        CatalogCheckoutId, CatalogProjectId, CatalogWorkspaceId, CheckoutHost, PlatformContextRef,
        PlatformMappingReconciliation, PlatformV2Mapping, ProjectCatalog, ProjectCheckout,
        ProjectRecord, RepositoryIdentity, WorkspaceLaunchIntake, WorkspaceLaunchRequest,
    };
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

    fn exact_catalog() -> (ProjectCatalog, CatalogWorkspaceId) {
        let project = CatalogProjectId::from_uuid(Uuid::from_u128(18_290));
        let checkout = CatalogCheckoutId::from_uuid(Uuid::from_u128(18_291));
        let workspace = CatalogWorkspaceId::from_uuid(Uuid::from_u128(18_292));
        let mut record = ProjectRecord::new(project, "Attention");
        record.add_checkout(ProjectCheckout::new(
            checkout,
            "local",
            CheckoutHost::Local {
                device_label: "test".into(),
                root: std::env::current_dir().unwrap(),
            },
            RepositoryIdentity {
                slug: "test/attention".into(),
                canonical_url: None,
            },
        ));
        let mut catalog = ProjectCatalog::default();
        catalog.insert_project(record).unwrap();
        catalog
            .create_workspace(WorkspaceLaunchRequest {
                id: workspace,
                project_id: project,
                checkout_id: checkout,
                name: "Attention".into(),
                intake: WorkspaceLaunchIntake::Manual,
            })
            .unwrap();
        catalog
            .set_platform_mapping(
                workspace,
                None,
                PlatformV2Mapping {
                    reconciliation_revision: 1,
                    project: PlatformContextRef {
                        id: "project-1".into(),
                        revision: 1,
                    },
                    checkout: PlatformContextRef {
                        id: "checkout-1".into(),
                        revision: 1,
                    },
                    user_workspace: PlatformContextRef {
                        id: "workspace-1".into(),
                        revision: 1,
                    },
                    reconciliation: PlatformMappingReconciliation::Exact {
                        reconciled_at_millis: 1,
                    },
                },
            )
            .unwrap();
        (catalog, workspace)
    }

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

    fn workspace_activation_fixture(
        cx: &mut gpui::TestAppContext,
        mode: AppMode,
    ) -> (
        gpui::Entity<Workspace>,
        PlatformAttentionActivation,
        AttentionLocalKey,
        tempfile::TempDir,
    ) {
        let (catalog, catalog_workspace) = exact_catalog();
        let temp = tempfile::tempdir().unwrap();
        let local_path = temp.path().join("private").join("local.json");
        let entity = cx.update(|cx| {
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
            config.cloud_sync.mode = mode;
            let workspace = cx.new(|cx| {
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
            });
            let terminal = cx.new(TerminalView::new);
            let hub = cx.new(|cx| WorkspaceHubView::new(Ok(catalog), &[], terminal, cx));
            workspace.update(cx, |workspace, _| {
                workspace.workspace_hub = hub;
                workspace.platform_attention_local =
                    Some(AttentionLocalStateStore::open(local_path, 8).unwrap());
                let source = source("workspace-1");
                let mut board = PlatformAttentionBoard::new(inventory("project-1", "workspace-1"));
                board
                    .apply_authenticated_baseline_read(
                        &source,
                        AttentionReadResult::Snapshot(Box::new(snapshot(
                            "project-1",
                            "workspace-1",
                            1,
                            None,
                        ))),
                    )
                    .unwrap();
                workspace
                    .platform_attention_boards
                    .insert(catalog_workspace, board);
                let cursor = PlatformCursor {
                    authority: ResourceAuthority::Automonique,
                    topic: CursorTopic::new("resources").unwrap(),
                    sequence: Revision::FIRST,
                };
                workspace.fleet_snapshot = Some(PlatformSnapshot {
                    capabilities: Capabilities::platform_v1(),
                    resources: Vec::new(),
                    cursor: cursor.clone(),
                    sessions: Vec::new(),
                    sessions_cursor: cursor,
                    view: PlatformView::default(),
                });
                // Keep this deterministic fixture on its injected authority
                // snapshot; the mode poll must not issue a real HTTP refresh.
                workspace.fleet_refresh_in_flight = true;
            });
            workspace
        });
        cx.executor().allow_parking();
        cx.run_until_parked();
        cx.executor().forbid_parking();
        let (activation, key) = entity.read_with(cx, |workspace, _| {
            let board = &workspace.platform_attention_boards[&catalog_workspace];
            let item = board.visible_items().next().unwrap();
            (
                PlatformAttentionActivation {
                    workspace: catalog_workspace,
                    item: item.ui_id(),
                    item_revision: item.value().revision(),
                },
                AttentionLocalKey::new(
                    board.inventory().target().clone(),
                    item.key().clone(),
                    item.value().revision(),
                ),
            )
        });
        (entity, activation, key, temp)
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
        assert!(retire_attention_board_state(
            &mut boards,
            &mut resync,
            workspace,
        ));
        assert!(!boards.contains_key(&workspace));
        assert!(resync.is_empty());

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
            AppMode::Dev,
            false,
            false,
            ActiveView::Workspaces,
            ActiveView::Workspaces,
        ));
        assert!(!platform_attention_surface_verified(
            AppMode::Dev,
            false,
            true,
            ActiveView::Workspaces,
            ActiveView::Workspaces,
        ));
        assert!(!platform_attention_surface_verified(
            AppMode::Dev,
            false,
            false,
            ActiveView::Fleet,
            ActiveView::Workspaces,
        ));
        assert!(!platform_attention_surface_verified(
            AppMode::User,
            false,
            false,
            ActiveView::Workspaces,
            ActiveView::Workspaces,
        ));
        assert!(!platform_attention_surface_verified(
            AppMode::Dev,
            true,
            false,
            ActiveView::Workspaces,
            ActiveView::Workspaces,
        ));

        let temp = tempfile::tempdir().unwrap();
        let mut store = AttentionLocalStateStore::open(
            temp.path().join("private").join("state").join("local.json"),
            8,
        )
        .unwrap();
        let local = AttentionLocalKey::new(
            inventory("project-1", "workspace-1").target().clone(),
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

    // SDTEST-1831
    #[test]
    fn workspace_context_removal_and_restart_flush_durable_retirement_custody() {
        let mut cx = gpui::TestAppContext::single();
        let workspace_entity = cx.update(|cx| {
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
            config.cloud_sync.mode = AppMode::Dev;
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
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("private").join("local.json");
        let catalog_workspace = CatalogWorkspaceId::from_uuid(Uuid::from_u128(1831));
        let source = source("workspace-1");
        let key = AttentionLocalKey::new(
            inventory("project-1", "workspace-1").target().clone(),
            AttentionItemKey::new(source.clone(), AttentionItemId::new("item-1").unwrap()),
            Revision::FIRST,
        );
        let mut board = PlatformAttentionBoard::new(inventory("project-1", "workspace-1"));
        board
            .apply_authenticated_baseline_read(
                &source,
                AttentionReadResult::Snapshot(Box::new(snapshot(
                    "project-1",
                    "workspace-1",
                    1,
                    None,
                ))),
            )
            .unwrap();
        let mut store = AttentionLocalStateStore::open(path.clone(), 8).unwrap();
        store.record_read(key.clone()).unwrap();
        workspace_entity.update(&mut cx, |workspace, _cx| {
            workspace.platform_attention_local = Some(store);
            workspace
                .platform_attention_boards
                .insert(catalog_workspace, board);
        });
        let displaced = path.with_extension("before-fence");
        std::fs::rename(&path, &displaced).unwrap();
        std::fs::create_dir(&path).unwrap();
        workspace_entity.update(&mut cx, |workspace, cx| {
            workspace.reconcile_platform_attention_context(None, cx);
            let retained = workspace
                .platform_attention_boards
                .get(&catalog_workspace)
                .expect("failed fence persistence keeps the board as an unavailable retry fence");
            assert!(retained.visible_items().next().is_none());
            assert!(!workspace.platform_attention_retirements_pending.is_empty());
            let store = workspace.platform_attention_local.as_ref().unwrap();
            assert!(store.state().is_read(&key));
        });
        std::fs::remove_dir(&path).unwrap();
        std::fs::rename(&displaced, &path).unwrap();
        workspace_entity.update(&mut cx, |workspace, cx| {
            workspace.reconcile_platform_attention_context(None, cx);
            assert!(workspace.platform_attention_boards.is_empty());
            assert!(!workspace
                .platform_attention_local
                .as_ref()
                .unwrap()
                .state()
                .is_read(&key));
        });

        let mut crashed = AttentionLocalStateStore::open(path.clone(), 8).unwrap();
        crashed.record_read(key.clone()).unwrap();
        let retirement = AttentionRetirement::new(
            inventory("project-1", "workspace-1").target().clone(),
            source,
        );
        crashed.begin_retirement(retirement).unwrap();
        drop(crashed);
        workspace_entity.update(&mut cx, |workspace, cx| {
            workspace.platform_attention_local =
                Some(AttentionLocalStateStore::open(path.clone(), 8).unwrap());
            workspace.reconcile_platform_attention_context(None, cx);
            let store = workspace.platform_attention_local.as_ref().unwrap();
            assert!(!store.state().is_read(&key));
            assert!(store.pending_retirements().is_empty());
        });
    }

    // SDTEST-1832
    #[test]
    fn inactive_workspace_revisit_preserves_distinct_board_and_overlay_custody() {
        let mut boards = BTreeMap::new();
        let workspace_one = CatalogWorkspaceId::from_uuid(Uuid::from_u128(18_321));
        let workspace_two = CatalogWorkspaceId::from_uuid(Uuid::from_u128(18_322));
        let board_one = PlatformAttentionBoard::new(inventory("project-1", "workspace-1"));
        let board_two = PlatformAttentionBoard::new(inventory("project-2", "workspace-2"));
        let target_one = board_one.inventory().target().clone();
        let target_two = board_two.inventory().target().clone();
        boards.insert(workspace_one, board_one);
        boards.insert(workspace_two, board_two);

        let context_one = (workspace_one, Some(target_one));
        assert!(attention_context_retirements(&boards, Some(&context_one)).is_empty());
        assert!(boards.contains_key(&workspace_two));
        let context_two = (workspace_two, Some(target_two));
        assert!(attention_context_retirements(&boards, Some(&context_two)).is_empty());
        assert_eq!(
            boards.len(),
            2,
            "switching active workspaces retires neither board"
        );
        assert_eq!(attention_context_retirements(&boards, None).len(), 2);
    }

    // SDTEST-1833
    #[test]
    fn real_workspace_waits_for_dev_visibility_across_modes_settings_and_inflight_transition() {
        fn advance_all(cx: &gpui::TestAppContext) {
            for _ in 0..8 {
                cx.executor()
                    .advance_clock(std::time::Duration::from_secs(5));
                while cx.executor().tick() {}
            }
        }

        for initial_mode in [AppMode::User, AppMode::Support] {
            let mut cx = gpui::TestAppContext::single();
            let (workspace, activation, key, _temp) =
                workspace_activation_fixture(&mut cx, initial_mode);
            workspace.update(&mut cx, |workspace, cx| {
                assert!(workspace.activate_platform_attention(activation, cx));
                assert!(!workspace
                    .platform_attention_local
                    .as_ref()
                    .unwrap()
                    .state()
                    .is_read(&key));
                assert!(workspace.mode_transition.is_some());
            });
            advance_all(&cx);
            workspace.read_with(&cx, |workspace, cx| {
                assert_eq!(workspace.effective_mode(), AppMode::Dev);
                assert!(workspace.mode_transition.is_none());
                assert_eq!(workspace.active_view, ActiveView::Workspaces);
                assert!(
                    workspace.platform_attention_local.as_ref().unwrap().state().is_read(&key),
                    "mode={:?} view={:?} pending={} confirmations={} settings={} board={} snapshot={} navigation={:?}",
                    workspace.effective_mode(),
                    workspace.active_view,
                    workspace.platform_attention_pending_activations.len(),
                    workspace.platform_attention_visible_confirmations.len(),
                    workspace.settings_open,
                    workspace.platform_attention_boards.contains_key(&activation.workspace),
                    workspace.fleet_snapshot.is_some(),
                    workspace.workspace_hub.read(cx).navigation().active(),
                );
            });
        }

        let mut cx = gpui::TestAppContext::single();
        let (workspace, activation, key, _temp) =
            workspace_activation_fixture(&mut cx, AppMode::Dev);
        workspace.update(&mut cx, |workspace, cx| {
            workspace.settings_open = true;
            assert!(workspace.activate_platform_attention(activation, cx));
            assert!(!workspace.settings_open);
            assert!(!workspace
                .platform_attention_local
                .as_ref()
                .unwrap()
                .state()
                .is_read(&key));
        });
        cx.refresh().unwrap();
        workspace.update(&mut cx, |workspace, cx| {
            assert_eq!(workspace.platform_attention_visible_confirmations.len(), 1);
            workspace.confirm_visible_platform_attention_activation(cx);
        });
        assert!(workspace.read_with(&cx, |workspace, _| workspace
            .platform_attention_local
            .as_ref()
            .unwrap()
            .state()
            .is_read(&key)));

        let mut cx = gpui::TestAppContext::single();
        let (workspace, activation, key, _temp) =
            workspace_activation_fixture(&mut cx, AppMode::Support);
        workspace.update(&mut cx, |workspace, cx| {
            workspace.set_mode(AppMode::User, cx);
            assert!(workspace.mode_transition.is_some());
            assert!(workspace.activate_platform_attention(activation, cx));
            assert!(!workspace
                .platform_attention_local
                .as_ref()
                .unwrap()
                .state()
                .is_read(&key));
        });
        advance_all(&cx);
        workspace.read_with(&cx, |workspace, _| {
            assert_eq!(workspace.effective_mode(), AppMode::Dev);
            assert!(workspace.mode_transition.is_none());
            assert!(workspace
                .platform_attention_local
                .as_ref()
                .unwrap()
                .state()
                .is_read(&key));
        });
    }

    // SDTEST-1834
    #[test]
    fn authority_change_between_open_and_visible_confirmation_stays_unread() {
        let mut cx = gpui::TestAppContext::single();
        let (workspace, activation, key, _temp) =
            workspace_activation_fixture(&mut cx, AppMode::Dev);
        workspace.update(&mut cx, |workspace, cx| {
            assert!(workspace.activate_platform_attention(activation, cx));
            assert_eq!(workspace.platform_attention_visible_confirmations.len(), 1);
            let board = workspace
                .platform_attention_boards
                .get_mut(&activation.workspace)
                .unwrap();
            let source = board.inventory().sources()[0].clone();
            board
                .mark_unavailable(&source, AttentionUnavailableReason::Transport)
                .unwrap();
            workspace.confirm_visible_platform_attention_activation(cx);
            assert!(!workspace
                .platform_attention_local
                .as_ref()
                .unwrap()
                .state()
                .is_read(&key));
            assert!(workspace
                .platform_attention_visible_confirmations
                .is_empty());
        });
    }
}
