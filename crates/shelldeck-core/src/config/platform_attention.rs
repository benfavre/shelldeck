//! Source-atomic native consumption of Platform v2 attention.
//!
//! It retains the complete authoritative source, project, user-workspace, item,
//! revision, and authority-qualified Platform session coordinates. Native
//! activation may resolve those coordinates through current client-local
//! catalogues, but never manufactures a pane, tab, terminal, or path.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use automonique_platform_client::platform_v2_client::AttentionReadResult;
use automonique_protocol::platform_v2::{
    ProjectId, UserWorkspaceId, WorkContextIdentity, WorkContextKind, WorkContextRecord,
    WorkContextRelationKind,
};
pub use automonique_protocol::platform_v2_attention::{
    AttentionItem, AttentionItemId, AttentionItemReason, AttentionItemState, AttentionSource,
    AttentionSourceId, AttentionSourceKind, AttentionSourceSnapshot, MAX_ATTENTION_ITEMS,
};
use automonique_protocol::platform_v2_transport::PlatformV2Refusal;
use automonique_protocol::primitives::Revision;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use super::app_config::AppConfig;
use super::platform::{
    FreshnessState, ResourceAuthority, ResourceCoordinate, ResourceKind, SessionRecord,
};
use super::workspace_catalog::PlatformV2Mapping;
use super::workspace_catalog::{CatalogWorkspaceId, ProjectCatalog};
use crate::workspace_navigation::{
    PaneNode, WorkspaceFocus, WorkspaceNavigationState, WorkspaceTabContent,
};
use crate::workspace_review::storage::{
    bounded_descriptor_read, ensure_private_directory_io, lock_path, open_lock_file,
    secure_atomic_write,
};

/// Same per-workspace ceiling as the authoritative hosted cockpit.
pub const MAX_ATTENTION_SOURCES_PER_WORKSPACE: usize = 64;
/// A client inventory is accepted only as one bounded, duplicate-free value.
pub const MAX_ATTENTION_INVENTORY_RECORDS: usize = 512;
/// Maximum locally retained read/notification tuples.
pub const MAX_ATTENTION_LOCAL_ENTRIES: usize = 4_096;
const MAX_ATTENTION_LOCAL_FILE_BYTES: u64 = 1024 * 1024;
const ATTENTION_LOCAL_SCHEMA: u16 = 2;
static ATTENTION_LOCAL_STORE_LOCK: parking_lot::Mutex<()> = parking_lot::Mutex::new(());

/// Namespace reserved for ShellDeck's presentation-only attention UUIDs.
///
/// The UUID is never sent to Automonique and is never an authority coordinate.
/// Its name bytes are the length-delimited raw `(source kind, source id, item
/// id)` tuple, so delimiter-bearing opaque identifiers cannot alias each other.
const ATTENTION_UI_NAMESPACE: Uuid = Uuid::from_bytes([
    0xa6, 0x61, 0x14, 0x3f, 0x23, 0x88, 0x5a, 0x25, 0xa4, 0xca, 0x53, 0x42, 0x6e, 0x92, 0x7d, 0x31,
]);

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct PlatformAttentionTarget {
    pub project: ProjectId,
    pub user_workspace: UserWorkspaceId,
}

impl PlatformAttentionTarget {
    /// Admit attention only from a completely reconciled catalog mapping.
    pub fn from_exact_mapping(mapping: &PlatformV2Mapping) -> Result<Self, AttentionError> {
        if !mapping.is_exact() {
            return Err(AttentionError::MappingNotExact);
        }
        Ok(Self {
            project: ProjectId::new(mapping.project.id.clone())
                .map_err(|_| AttentionError::MappingInvalid)?,
            user_workspace: UserWorkspaceId::new(mapping.user_workspace.id.clone())
                .map_err(|_| AttentionError::MappingInvalid)?,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReviewAttentionPresence {
    /// A typed `GetReview` returned the exact requested review.
    Present,
    /// A typed `GetReview` returned the exact not-found refusal.
    Absent,
}

/// Complete bounded source inventory for one exact project/user-workspace.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AttentionSourceInventory {
    target: PlatformAttentionTarget,
    sources: Vec<AttentionSource>,
    /// Exact Platform session coordinate authoritatively related to each
    /// provider WorkContext source. This binding must survive inventory
    /// projection so an item cannot redirect its source to another otherwise
    /// valid current session.
    provider_sessions: BTreeMap<AttentionSource, ResourceCoordinate>,
}

impl AttentionSourceInventory {
    pub fn from_authoritative_records(
        target: PlatformAttentionTarget,
        records: &[WorkContextRecord],
        review: ReviewAttentionPresence,
    ) -> Result<Self, AttentionError> {
        if records.len() > MAX_ATTENTION_INVENTORY_RECORDS {
            return Err(AttentionError::InventoryTooLarge);
        }
        let mut identities = BTreeSet::new();
        if records
            .iter()
            .any(|record| !identities.insert(record.identity().clone()))
        {
            return Err(AttentionError::InventoryDuplicate);
        }

        let workspace_identity = WorkContextIdentity::UserWorkspace(target.user_workspace.clone());
        let workspace = records
            .iter()
            .filter(|record| record.identity() == &workspace_identity)
            .collect::<Vec<_>>();
        if workspace.len() != 1 {
            return Err(AttentionError::WorkspaceMissingOrAmbiguous);
        }
        let project_identity = WorkContextIdentity::Project(target.project.clone());
        if relation(workspace[0], WorkContextRelationKind::UserWorkspaceProject)
            != Some(&project_identity)
        {
            return Err(AttentionError::WorkspaceProjectMismatch);
        }

        let workspace_source = AttentionSourceId::new(target.user_workspace.as_str().to_owned())
            .map_err(|_| AttentionError::SourceInvalid)?;
        let mut sources = BTreeSet::new();
        let mut provider_sessions = BTreeMap::new();
        if review == ReviewAttentionPresence::Present {
            sources.insert(AttentionSource::new(
                AttentionSourceKind::Review,
                workspace_source.clone(),
            ));
        }
        sources.insert(AttentionSource::new(
            AttentionSourceKind::Orchestration,
            workspace_source,
        ));

        let attempts = records
            .iter()
            .filter(|record| record.kind() == WorkContextKind::AttemptWorkspace)
            .filter(|record| {
                relation(record, WorkContextRelationKind::AttemptUserWorkspace)
                    == Some(&workspace_identity)
            })
            .map(|record| record.identity().clone())
            .collect::<BTreeSet<_>>();
        for session in records
            .iter()
            .filter(|record| record.kind() == WorkContextKind::Session)
        {
            if relation(session, WorkContextRelationKind::SessionAttemptWorkspace)
                .is_some_and(|attempt| attempts.contains(attempt))
            {
                let id = AttentionSourceId::new(session.identity().id().to_owned())
                    .map_err(|_| AttentionError::SourceInvalid)?;
                let source = AttentionSource::new(AttentionSourceKind::ProviderSession, id);
                let coordinate =
                    match relation(session, WorkContextRelationKind::SessionPlatformSession) {
                        Some(WorkContextIdentity::PlatformSession(session))
                            if session.coordinate().authority == ResourceAuthority::Automonique
                                && session.coordinate().kind == ResourceKind::Session =>
                        {
                            session.coordinate().clone()
                        }
                        _ => return Err(AttentionError::ProviderSessionRelationInvalid),
                    };
                if !sources.insert(source.clone())
                    || provider_sessions.insert(source, coordinate).is_some()
                {
                    return Err(AttentionError::SourceDuplicate);
                }
            }
        }
        if sources.len() > MAX_ATTENTION_SOURCES_PER_WORKSPACE {
            return Err(AttentionError::SourceInventoryTooLarge);
        }
        Ok(Self {
            target,
            sources: sources.into_iter().collect(),
            provider_sessions,
        })
    }

    #[must_use]
    pub const fn target(&self) -> &PlatformAttentionTarget {
        &self.target
    }

    #[must_use]
    pub fn sources(&self) -> &[AttentionSource] {
        &self.sources
    }

    #[must_use]
    pub fn contains(&self, source: &AttentionSource) -> bool {
        self.sources.binary_search(source).is_ok()
    }

    #[must_use]
    pub fn provider_session(&self, source: &AttentionSource) -> Option<&ResourceCoordinate> {
        self.provider_sessions.get(source)
    }
}

fn relation(
    record: &WorkContextRecord,
    kind: WorkContextRelationKind,
) -> Option<&WorkContextIdentity> {
    record
        .relations()
        .iter()
        .find(|relation| relation.kind() == kind)
        .map(|relation| relation.target())
}

/// Full raw authority key. The item id has no meaning without its source.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct AttentionItemKey {
    source: AttentionSource,
    item: AttentionItemId,
}

impl AttentionItemKey {
    #[must_use]
    pub const fn new(source: AttentionSource, item: AttentionItemId) -> Self {
        Self { source, item }
    }

    #[must_use]
    pub const fn source(&self) -> &AttentionSource {
        &self.source
    }

    #[must_use]
    pub const fn item(&self) -> &AttentionItemId {
        &self.item
    }
}

/// Presentation-only stable identifier derived from [`AttentionItemKey`].
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct AttentionUiItemId(Uuid);

impl AttentionUiItemId {
    #[must_use]
    pub fn from_authoritative_key(key: &AttentionItemKey) -> Self {
        let mut name = Vec::with_capacity(
            key.source.kind().as_str().len()
                + key.source.id().as_str().len()
                + key.item.as_str().len()
                + 12,
        );
        append_component(&mut name, key.source.kind().as_str());
        append_component(&mut name, key.source.id().as_str());
        append_component(&mut name, key.item.as_str());
        Self(Uuid::new_v5(&ATTENTION_UI_NAMESPACE, &name))
    }

    #[must_use]
    pub const fn uuid(self) -> Uuid {
        self.0
    }
}

fn append_component(output: &mut Vec<u8>, value: &str) {
    let length = u32::try_from(value.len()).expect("protocol field bound fits u32");
    output.extend_from_slice(&length.to_be_bytes());
    output.extend_from_slice(value.as_bytes());
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuthoritativeAttentionItem {
    key: AttentionItemKey,
    ui_id: AttentionUiItemId,
    value: AttentionItem,
}

impl AuthoritativeAttentionItem {
    #[must_use]
    pub const fn key(&self) -> &AttentionItemKey {
        &self.key
    }

    #[must_use]
    pub const fn ui_id(&self) -> AttentionUiItemId {
        self.ui_id
    }

    #[must_use]
    pub const fn value(&self) -> &AttentionItem {
        &self.value
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AttentionUnavailableReason {
    NotObserved,
    Transport,
    Protocol,
    InventoryIncomplete,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AttentionSourceStatus {
    Available,
    Refused { category: String },
    Unavailable(AttentionUnavailableReason),
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct AttentionSourceProjection {
    snapshot: AttentionSourceSnapshot,
    items: BTreeMap<AttentionItemKey, AuthoritativeAttentionItem>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct AttentionSourceSlot {
    status: AttentionSourceStatus,
    /// Retained while unavailable/refused so a later read must still be the
    /// exact successor rather than silently resetting the revision chain.
    projection: Option<AttentionSourceProjection>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AttentionApplyOutcome {
    Inserted,
    Replaced,
    ExactReplay,
    AvailabilityRestored,
    Refused,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AttentionReplacementMode {
    Continuous,
    AuthenticatedCompleteBaseline,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlatformAttentionBoard {
    inventory: AttentionSourceInventory,
    slots: BTreeMap<AttentionSource, AttentionSourceSlot>,
    ui_index: BTreeMap<AttentionUiItemId, AttentionItemKey>,
}

/// Stable same-process activation request. It deliberately carries the
/// authoritative presentation identity and revision, not a previously
/// resolved pane/session. Every click is resolved again against the current
/// authorized catalogues.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PlatformAttentionActivation {
    pub workspace: CatalogWorkspaceId,
    pub item: AttentionUiItemId,
    pub item_revision: Revision,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PlatformAttentionDestination {
    WorkspaceAttention {
        workspace: CatalogWorkspaceId,
    },
    FleetSession {
        workspace: CatalogWorkspaceId,
        session: ResourceCoordinate,
    },
    RetainedProviderPane {
        workspace: CatalogWorkspaceId,
        session: ResourceCoordinate,
        focus: WorkspaceFocus,
    },
}

/// Resolve an activation through the complete current catalogue.
///
/// Provider coordinates must be exact authorized Automonique sessions. A
/// retained provider pane, when present, must also be unique. Review and
/// orchestration items intentionally resolve only to the mapped workspace's
/// attention surface and cannot acquire provider authority.
pub fn resolve_platform_attention_activation(
    activation: PlatformAttentionActivation,
    board: &PlatformAttentionBoard,
    catalog: &ProjectCatalog,
    navigation: &WorkspaceNavigationState,
    sessions: &[SessionRecord],
) -> Result<PlatformAttentionDestination, AttentionActivationError> {
    let item = board
        .item_by_ui_id(activation.item)
        .ok_or(AttentionActivationError::ItemMissing)?;
    if item.value().revision() != activation.item_revision {
        return Err(AttentionActivationError::ItemStale);
    }

    let matches = catalog
        .workspaces()
        .filter(|workspace| {
            workspace.platform_mapping().is_some_and(|mapping| {
                PlatformAttentionTarget::from_exact_mapping(mapping)
                    .is_ok_and(|target| &target == board.inventory().target())
            })
        })
        .map(|workspace| workspace.id())
        .collect::<Vec<_>>();
    if matches.as_slice() != [activation.workspace] {
        return Err(AttentionActivationError::WorkspaceMissingOrAmbiguous);
    }
    let retained = navigation
        .workspace(activation.workspace)
        .ok_or(AttentionActivationError::WorkspaceMissingOrAmbiguous)?;

    let source_kind = item.key().source().kind();
    if source_kind != AttentionSourceKind::ProviderSession {
        if item.value().platform_session().is_some() {
            return Err(AttentionActivationError::CoordinateInvalid);
        }
        return Ok(PlatformAttentionDestination::WorkspaceAttention {
            workspace: activation.workspace,
        });
    }

    let coordinate = item
        .value()
        .platform_session()
        .map(|session| session.coordinate())
        .ok_or(AttentionActivationError::CoordinateInvalid)?;
    if coordinate.authority != ResourceAuthority::Automonique
        || coordinate.kind != ResourceKind::Session
        || board.inventory().provider_session(item.key().source()) != Some(coordinate)
    {
        return Err(AttentionActivationError::CoordinateInvalid);
    }
    let matching_sessions = sessions
        .iter()
        .filter(|record| &record.session.resource == coordinate)
        .collect::<Vec<_>>();
    let [session] = matching_sessions.as_slice() else {
        return Err(AttentionActivationError::SessionMissingOrAmbiguous);
    };
    if session.session.freshness.state != FreshnessState::Fresh {
        return Err(AttentionActivationError::SessionNotFresh);
    }

    let mapping = catalog
        .workspace(activation.workspace)
        .ok()
        .and_then(|workspace| workspace.platform_mapping())
        .filter(|mapping| mapping.is_exact())
        .ok_or(AttentionActivationError::WorkspaceMissingOrAmbiguous)?;
    let mut pane_matches = Vec::new();
    collect_provider_panes(
        retained.surface.root.as_ref(),
        &mapping.user_workspace.id,
        coordinate,
        &mut pane_matches,
    );
    match pane_matches.as_slice() {
        [] => Ok(PlatformAttentionDestination::FleetSession {
            workspace: activation.workspace,
            session: coordinate.clone(),
        }),
        [focus] => Ok(PlatformAttentionDestination::RetainedProviderPane {
            workspace: activation.workspace,
            session: coordinate.clone(),
            focus: *focus,
        }),
        _ => Err(AttentionActivationError::PaneAmbiguous),
    }
}

fn collect_provider_panes(
    node: Option<&PaneNode>,
    user_workspace: &str,
    coordinate: &ResourceCoordinate,
    output: &mut Vec<WorkspaceFocus>,
) {
    let Some(node) = node else { return };
    match node {
        PaneNode::Leaf(leaf) => {
            for tab in &leaf.tabs {
                if matches!(
                    &tab.content,
                    WorkspaceTabContent::ProviderSession(binding)
                        if binding.platform_user_workspace_id == user_workspace
                            && binding.session_id == coordinate.id.as_str()
                ) {
                    output.push(WorkspaceFocus {
                        pane_id: leaf.id,
                        tab_id: tab.id,
                    });
                }
            }
        }
        PaneNode::Split { first, second, .. } => {
            collect_provider_panes(Some(first), user_workspace, coordinate, output);
            collect_provider_panes(Some(second), user_workspace, coordinate, output);
        }
    }
}

impl PlatformAttentionBoard {
    #[must_use]
    pub fn new(inventory: AttentionSourceInventory) -> Self {
        let slots = inventory
            .sources()
            .iter()
            .cloned()
            .map(|source| {
                (
                    source,
                    AttentionSourceSlot {
                        status: AttentionSourceStatus::Unavailable(
                            AttentionUnavailableReason::NotObserved,
                        ),
                        projection: None,
                    },
                )
            })
            .collect();
        Self {
            inventory,
            slots,
            ui_index: BTreeMap::new(),
        }
    }

    #[must_use]
    pub const fn inventory(&self) -> &AttentionSourceInventory {
        &self.inventory
    }

    /// Replace the complete source inventory. Removed sources and all their UI
    /// projections disappear together; retained sources keep revision custody.
    pub fn replace_inventory(
        &mut self,
        inventory: AttentionSourceInventory,
    ) -> Result<(), AttentionError> {
        if inventory.target() != self.inventory.target() {
            return Err(AttentionError::TargetMismatch);
        }
        let mut slots = BTreeMap::new();
        for source in inventory.sources() {
            slots.insert(
                source.clone(),
                self.slots
                    .get(source)
                    .cloned()
                    .unwrap_or(AttentionSourceSlot {
                        status: AttentionSourceStatus::Unavailable(
                            AttentionUnavailableReason::NotObserved,
                        ),
                        projection: None,
                    }),
            );
        }
        let ui_index = build_ui_index(&slots)?;
        self.inventory = inventory;
        self.slots = slots;
        self.ui_index = ui_index;
        Ok(())
    }

    pub fn apply_read(
        &mut self,
        requested_source: &AttentionSource,
        result: AttentionReadResult,
    ) -> Result<AttentionApplyOutcome, AttentionError> {
        match result {
            AttentionReadResult::Snapshot(snapshot) => {
                if snapshot.source() != requested_source {
                    return Err(AttentionError::SourceMismatch);
                }
                self.replace_source(*snapshot)
            }
            AttentionReadResult::Refused(refusal) => {
                self.mark_refused(requested_source, &refusal)?;
                Ok(AttentionApplyOutcome::Refused)
            }
        }
    }

    /// Install a complete snapshot returned directly by an authenticated
    /// Platform attention read as a fresh baseline or an explicit gap resync.
    ///
    /// Unlike [`Self::apply_read`], this operation may begin above revision one
    /// and may bridge a missed predecessor. It still rejects rollback,
    /// conflicting replay, regressing observation/item revisions, wrong
    /// source/target, and presentation identity collisions. Callers must not
    /// pass cached, synthesized, or locally assembled snapshots here.
    pub fn apply_authenticated_baseline_read(
        &mut self,
        requested_source: &AttentionSource,
        result: AttentionReadResult,
    ) -> Result<AttentionApplyOutcome, AttentionError> {
        match result {
            AttentionReadResult::Snapshot(snapshot) => {
                if snapshot.source() != requested_source {
                    return Err(AttentionError::SourceMismatch);
                }
                self.replace_source_with_mode(
                    *snapshot,
                    AttentionUiItemId::from_authoritative_key,
                    AttentionReplacementMode::AuthenticatedCompleteBaseline,
                )
            }
            AttentionReadResult::Refused(refusal) => {
                self.mark_refused(requested_source, &refusal)?;
                Ok(AttentionApplyOutcome::Refused)
            }
        }
    }

    pub fn replace_source(
        &mut self,
        snapshot: AttentionSourceSnapshot,
    ) -> Result<AttentionApplyOutcome, AttentionError> {
        self.replace_source_with_mode(
            snapshot,
            AttentionUiItemId::from_authoritative_key,
            AttentionReplacementMode::Continuous,
        )
    }

    #[cfg(test)]
    fn replace_source_with(
        &mut self,
        snapshot: AttentionSourceSnapshot,
        projector: impl Fn(&AttentionItemKey) -> AttentionUiItemId,
    ) -> Result<AttentionApplyOutcome, AttentionError> {
        self.replace_source_with_mode(snapshot, projector, AttentionReplacementMode::Continuous)
    }

    fn replace_source_with_mode(
        &mut self,
        snapshot: AttentionSourceSnapshot,
        projector: impl Fn(&AttentionItemKey) -> AttentionUiItemId,
        mode: AttentionReplacementMode,
    ) -> Result<AttentionApplyOutcome, AttentionError> {
        let source = snapshot.source().clone();
        let current_slot = self
            .slots
            .get(&source)
            .ok_or(AttentionError::SourceNotInventoried)?;
        if snapshot.project() != &self.inventory.target.project
            || snapshot.user_workspace() != &self.inventory.target.user_workspace
        {
            return Err(AttentionError::TargetMismatch);
        }

        let previous_status = current_slot.status.clone();
        if let Some(current) = &current_slot.projection {
            if snapshot.revision() == current.snapshot.revision() {
                if snapshot != current.snapshot {
                    return Err(AttentionError::ConflictingReplay);
                }
                if previous_status == AttentionSourceStatus::Available {
                    return Ok(AttentionApplyOutcome::ExactReplay);
                }
                let mut slots = self.slots.clone();
                slots
                    .get_mut(&source)
                    .expect("source admission checked above")
                    .status = AttentionSourceStatus::Available;
                let ui_index = build_ui_index(&slots)?;
                self.slots = slots;
                self.ui_index = ui_index;
                return Ok(AttentionApplyOutcome::AvailabilityRestored);
            }
            match mode {
                AttentionReplacementMode::Continuous => current
                    .snapshot
                    .validate_successor(&snapshot)
                    .map_err(|_| AttentionError::InvalidSuccessor)?,
                AttentionReplacementMode::AuthenticatedCompleteBaseline => {
                    validate_baseline_advance(&current.snapshot, &snapshot)?;
                }
            }
        } else if mode == AttentionReplacementMode::Continuous
            && snapshot.revision() != Revision::FIRST
        {
            return Err(AttentionError::InitialRevisionRequired);
        }

        let mut projected = BTreeMap::new();
        for item in snapshot.items() {
            let key = AttentionItemKey::new(source.clone(), item.id().clone());
            let ui_id = projector(&key);
            projected.insert(
                key.clone(),
                AuthoritativeAttentionItem {
                    key,
                    ui_id,
                    value: item.clone(),
                },
            );
        }

        // Build the entire candidate board and collision index before making
        // any visible mutation. One malformed/colliding item changes nothing.
        let mut slots = self.slots.clone();
        let was_present = current_slot.projection.is_some();
        slots.insert(
            source,
            AttentionSourceSlot {
                status: AttentionSourceStatus::Available,
                projection: Some(AttentionSourceProjection {
                    snapshot,
                    items: projected,
                }),
            },
        );
        let ui_index = build_ui_index(&slots)?;
        self.slots = slots;
        self.ui_index = ui_index;
        Ok(if was_present {
            AttentionApplyOutcome::Replaced
        } else {
            AttentionApplyOutcome::Inserted
        })
    }

    /// A refusal hides the old projection but retains its revision chain.
    pub fn mark_refused(
        &mut self,
        source: &AttentionSource,
        refusal: &PlatformV2Refusal,
    ) -> Result<(), AttentionError> {
        let slot = self
            .slots
            .get_mut(source)
            .ok_or(AttentionError::SourceNotInventoried)?;
        slot.status = AttentionSourceStatus::Refused {
            category: refusal.category().as_str().to_owned(),
        };
        self.rebuild_visible_index()?;
        Ok(())
    }

    /// A local transport/protocol failure is not an authoritative empty source.
    pub fn mark_unavailable(
        &mut self,
        source: &AttentionSource,
        reason: AttentionUnavailableReason,
    ) -> Result<(), AttentionError> {
        let slot = self
            .slots
            .get_mut(source)
            .ok_or(AttentionError::SourceNotInventoried)?;
        slot.status = AttentionSourceStatus::Unavailable(reason);
        self.rebuild_visible_index()?;
        Ok(())
    }

    fn rebuild_visible_index(&mut self) -> Result<(), AttentionError> {
        let index = build_ui_index(&self.slots)?;
        self.ui_index = index;
        Ok(())
    }

    #[must_use]
    pub fn status(&self, source: &AttentionSource) -> Option<&AttentionSourceStatus> {
        self.slots.get(source).map(|slot| &slot.status)
    }

    #[must_use]
    pub fn retained_snapshot(&self, source: &AttentionSource) -> Option<&AttentionSourceSnapshot> {
        self.slots
            .get(source)
            .and_then(|slot| slot.projection.as_ref())
            .map(|projection| &projection.snapshot)
    }

    pub fn visible_items(&self) -> impl Iterator<Item = &AuthoritativeAttentionItem> {
        self.slots.values().flat_map(|slot| {
            (slot.status == AttentionSourceStatus::Available)
                .then_some(slot.projection.as_ref())
                .flatten()
                .into_iter()
                .flat_map(|projection| projection.items.values())
        })
    }

    /// Every visible item, newest observation first.
    ///
    /// The authoritative `observed_at_ms` is the only chronology ShellDeck
    /// has: order is never inferred from poll arrival, source order, or when
    /// a row reached this process. Equal observations fall back to the item
    /// revision and then to the authoritative `(source, item)` key, so the
    /// order is total, deterministic, and identical in every process.
    #[must_use]
    pub fn chronology(&self) -> Vec<&AuthoritativeAttentionItem> {
        let mut items = self.visible_items().collect::<Vec<_>>();
        items.sort_by(|left, right| {
            right
                .value()
                .observed_at_ms()
                .cmp(&left.value().observed_at_ms())
                .then_with(|| right.value().revision().cmp(&left.value().revision()))
                .then_with(|| left.key().cmp(right.key()))
        });
        items
    }

    #[must_use]
    pub fn item_by_ui_id(&self, id: AttentionUiItemId) -> Option<&AuthoritativeAttentionItem> {
        let key = self.ui_index.get(&id)?;
        self.slots
            .get(key.source())?
            .projection
            .as_ref()?
            .items
            .get(key)
    }
}

fn validate_baseline_advance(
    current: &AttentionSourceSnapshot,
    next: &AttentionSourceSnapshot,
) -> Result<(), AttentionError> {
    if next.source() != current.source()
        || next.project() != current.project()
        || next.user_workspace() != current.user_workspace()
        || next.revision() <= current.revision()
        || next
            .previous_revision()
            .is_none_or(|previous| previous < current.revision())
        || next.observed_at_ms() < current.observed_at_ms()
    {
        return Err(AttentionError::InvalidBaseline);
    }
    for next_item in next.items() {
        let current_item = current
            .items()
            .iter()
            .find(|item| item.id() == next_item.id());
        if current_item.is_some_and(|current_item| {
            next_item.revision() < current_item.revision()
                || next_item.observed_at_ms() < current_item.observed_at_ms()
                || (next_item.revision() == current_item.revision() && next_item != current_item)
        }) {
            return Err(AttentionError::InvalidBaseline);
        }
    }
    Ok(())
}

fn build_ui_index(
    slots: &BTreeMap<AttentionSource, AttentionSourceSlot>,
) -> Result<BTreeMap<AttentionUiItemId, AttentionItemKey>, AttentionError> {
    let mut index = BTreeMap::new();
    for slot in slots.values() {
        if slot.status != AttentionSourceStatus::Available {
            continue;
        }
        if let Some(projection) = &slot.projection {
            for item in projection.items.values() {
                if index
                    .insert(item.ui_id(), item.key().clone())
                    .is_some_and(|existing| existing != *item.key())
                {
                    return Err(AttentionError::UiIdentityCollision);
                }
            }
        }
    }
    Ok(index)
}

/// Durable local acknowledgement key. Server `unread` remains untouched.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct AttentionLocalKey {
    target: PlatformAttentionTarget,
    item: AttentionItemKey,
    item_revision: Revision,
}

/// Durable fence for local overlay custody belonging to a retired Platform
/// source incarnation. The target is part of the key so a remap can never
/// make an old target's cleanup look like custody for the replacement.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct AttentionRetirement {
    target: PlatformAttentionTarget,
    source: AttentionSource,
}

impl AttentionRetirement {
    #[must_use]
    pub const fn new(target: PlatformAttentionTarget, source: AttentionSource) -> Self {
        Self { target, source }
    }

    #[must_use]
    pub const fn target(&self) -> &PlatformAttentionTarget {
        &self.target
    }

    #[must_use]
    pub const fn source(&self) -> &AttentionSource {
        &self.source
    }
}

impl AttentionLocalKey {
    #[must_use]
    pub const fn new(
        target: PlatformAttentionTarget,
        item: AttentionItemKey,
        item_revision: Revision,
    ) -> Self {
        Self {
            target,
            item,
            item_revision,
        }
    }

    #[must_use]
    pub const fn target(&self) -> &PlatformAttentionTarget {
        &self.target
    }

    #[must_use]
    pub const fn item(&self) -> &AttentionItemKey {
        &self.item
    }

    #[must_use]
    pub const fn item_revision(&self) -> Revision {
        self.item_revision
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct AttentionLocalFlags {
    read: bool,
    notified: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AttentionLocalState {
    capacity: usize,
    entries: BTreeMap<AttentionLocalKey, AttentionLocalFlags>,
    retirements: BTreeSet<AttentionRetirement>,
}

impl AttentionLocalState {
    pub fn with_capacity(capacity: usize) -> Result<Self, AttentionLocalStateError> {
        if capacity == 0 || capacity > MAX_ATTENTION_LOCAL_ENTRIES {
            return Err(AttentionLocalStateError::CapacityInvalid);
        }
        Ok(Self {
            capacity,
            entries: BTreeMap::new(),
            retirements: BTreeSet::new(),
        })
    }

    #[must_use]
    pub fn is_read(&self, key: &AttentionLocalKey) -> bool {
        self.entries.get(key).is_some_and(|flags| flags.read)
    }

    #[must_use]
    pub fn is_notified(&self, key: &AttentionLocalKey) -> bool {
        self.entries.get(key).is_some_and(|flags| flags.notified)
    }

    fn mark_read(&mut self, key: AttentionLocalKey) -> Result<bool, AttentionLocalStateError> {
        if self.retirements.iter().any(|retirement| {
            retirement.target() == key.target() && retirement.source() == key.item().source()
        }) {
            return Err(AttentionLocalStateError::RetirementPending);
        }
        self.update(key, |flags| &mut flags.read)
    }

    fn mark_notified(&mut self, key: AttentionLocalKey) -> Result<bool, AttentionLocalStateError> {
        if self.retirements.iter().any(|retirement| {
            retirement.target() == key.target() && retirement.source() == key.item().source()
        }) {
            return Err(AttentionLocalStateError::RetirementPending);
        }
        self.update(key, |flags| &mut flags.notified)
    }

    fn update(
        &mut self,
        key: AttentionLocalKey,
        select: impl Fn(&mut AttentionLocalFlags) -> &mut bool,
    ) -> Result<bool, AttentionLocalStateError> {
        if !self.entries.contains_key(&key) && self.entries.len() == self.capacity {
            return Err(AttentionLocalStateError::CapacityExceeded);
        }
        let value = select(self.entries.entry(key).or_default());
        if *value {
            return Ok(false);
        }
        *value = true;
        Ok(true)
    }

    fn retain_source_keys(
        &mut self,
        target: &PlatformAttentionTarget,
        source: &AttentionSource,
        current: &BTreeSet<AttentionLocalKey>,
    ) -> bool {
        let before = self.entries.len();
        self.entries.retain(|key, _| {
            key.target() != target || key.item().source() != source || current.contains(key)
        });
        before != self.entries.len()
    }

    fn begin_retirement(
        &mut self,
        retirement: AttentionRetirement,
    ) -> Result<bool, AttentionLocalStateError> {
        if !self.retirements.contains(&retirement) && self.retirements.len() == self.capacity {
            return Err(AttentionLocalStateError::CapacityExceeded);
        }
        Ok(self.retirements.insert(retirement))
    }

    fn finish_retirement(&mut self, retirement: &AttentionRetirement) -> bool {
        let removed_entries =
            self.retain_source_keys(retirement.target(), retirement.source(), &BTreeSet::new());
        self.retirements.remove(retirement) || removed_entries
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NotificationReservation {
    Ineligible,
    AlreadyReserved,
    Reserved,
}

/// Atomic store for local read and at-most-once notification custody.
///
/// Every mutation first writes a complete candidate document and changes the
/// in-memory value only after atomic replacement succeeds. Load, persistence,
/// and capacity errors are returned to the caller; callers must suppress a
/// notification on error instead of evicting an older tuple and risking spam.
#[derive(Clone, Debug)]
pub struct AttentionLocalStateStore {
    path: PathBuf,
    state: AttentionLocalState,
}

impl AttentionLocalStateStore {
    pub fn open_default() -> Result<Self, AttentionLocalStateError> {
        Self::open(
            AppConfig::config_dir()
                .join("platform-attention")
                .join("state")
                .join("local-v1.json"),
            MAX_ATTENTION_LOCAL_ENTRIES,
        )
    }

    pub fn open(path: PathBuf, capacity: usize) -> Result<Self, AttentionLocalStateError> {
        AttentionLocalState::with_capacity(capacity)?;
        let _process_guard = ATTENTION_LOCAL_STORE_LOCK.lock();
        prepare_local_storage(&path)?;
        let lock = open_lock_file(&lock_path(&path))?;
        fs2::FileExt::lock_exclusive(&lock)?;
        let state = load_local_state(&path, capacity)?;
        Ok(Self { path, state })
    }

    #[must_use]
    pub const fn state(&self) -> &AttentionLocalState {
        &self.state
    }

    pub fn record_read(
        &mut self,
        key: AttentionLocalKey,
    ) -> Result<bool, AttentionLocalStateError> {
        self.transact(|state| state.mark_read(key))
    }

    pub fn reserve_notification(
        &mut self,
        key: AttentionLocalKey,
        authoritative_unread: bool,
    ) -> Result<NotificationReservation, AttentionLocalStateError> {
        if !authoritative_unread {
            return Ok(NotificationReservation::Ineligible);
        }
        self.transact(|state| {
            if state.retirements.iter().any(|retirement| {
                retirement.target() == key.target() && retirement.source() == key.item().source()
            }) {
                return Err(AttentionLocalStateError::RetirementPending);
            }
            if state.is_notified(&key) {
                return Ok(NotificationReservation::AlreadyReserved);
            }
            state.mark_notified(key)?;
            Ok(NotificationReservation::Reserved)
        })
    }

    /// Prune superseded/removed revisions only after accepting a complete
    /// authoritative source replacement. Refusal/unavailability must not call
    /// this method because neither means an empty source.
    pub fn reconcile_source(
        &mut self,
        target: &PlatformAttentionTarget,
        source: &AttentionSource,
        current: BTreeSet<AttentionLocalKey>,
    ) -> Result<bool, AttentionLocalStateError> {
        if current.len() > MAX_ATTENTION_ITEMS
            || current
                .iter()
                .any(|key| key.target() != target || key.item().source() != source)
        {
            return Err(AttentionLocalStateError::ReconciliationInvalid);
        }
        self.transact(|state| {
            if state
                .retirements
                .iter()
                .any(|retirement| retirement.target() == target && retirement.source() == source)
            {
                return Err(AttentionLocalStateError::RetirementPending);
            }
            Ok(state.retain_source_keys(target, source, &current))
        })
    }

    pub fn remove_source(
        &mut self,
        target: &PlatformAttentionTarget,
        source: &AttentionSource,
    ) -> Result<bool, AttentionLocalStateError> {
        self.reconcile_source(target, source, BTreeSet::new())
    }

    /// Persist the retirement fence before the corresponding in-memory board
    /// is removed. A crash after this succeeds leaves enough custody for the
    /// next process to finish cleanup before admitting a replacement.
    pub fn begin_retirement(
        &mut self,
        retirement: AttentionRetirement,
    ) -> Result<bool, AttentionLocalStateError> {
        self.transact(|state| state.begin_retirement(retirement))
    }

    /// Atomically remove every overlay tuple for the retired source and its
    /// durable fence. Failure leaves both intact and therefore fail-closed.
    pub fn finish_retirement(
        &mut self,
        retirement: &AttentionRetirement,
    ) -> Result<bool, AttentionLocalStateError> {
        self.transact(|state| Ok(state.finish_retirement(retirement)))
    }

    #[must_use]
    pub fn pending_retirements(&self) -> &BTreeSet<AttentionRetirement> {
        &self.state.retirements
    }

    fn transact<T>(
        &mut self,
        update: impl FnOnce(&mut AttentionLocalState) -> Result<T, AttentionLocalStateError>,
    ) -> Result<T, AttentionLocalStateError> {
        let _process_guard = ATTENTION_LOCAL_STORE_LOCK.lock();
        prepare_local_storage(&self.path)?;
        let lock = open_lock_file(&lock_path(&self.path))?;
        fs2::FileExt::lock_exclusive(&lock)?;
        let disk = load_local_state(&self.path, self.state.capacity)?;
        let mut candidate = disk.clone();
        let outcome = update(&mut candidate)?;
        if candidate != disk {
            persist_local_state(&self.path, &candidate)?;
        }
        self.state = candidate;
        Ok(outcome)
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct AttentionLocalDocument {
    schema: u16,
    entries: Vec<AttentionLocalEntry>,
    retirements: Vec<AttentionLocalRetirement>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct AttentionLocalEntry {
    project_id: String,
    workspace_id: String,
    source_kind: String,
    source_id: String,
    item_id: String,
    item_revision: u64,
    read: bool,
    notified: bool,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct AttentionLocalRetirement {
    project_id: String,
    workspace_id: String,
    source_kind: String,
    source_id: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AttentionLocalDocumentV1 {
    schema: u16,
    entries: Vec<AttentionLocalEntryV1>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(dead_code)] // Parsed only to validate and boundedly discard unscoped v1 custody.
struct AttentionLocalEntryV1 {
    source_kind: String,
    source_id: String,
    item_id: String,
    item_revision: u64,
    read: bool,
    notified: bool,
}

fn load_local_state(
    path: &Path,
    capacity: usize,
) -> Result<AttentionLocalState, AttentionLocalStateError> {
    let mut state = AttentionLocalState::with_capacity(capacity)?;
    let Some(bytes) = bounded_descriptor_read(path, MAX_ATTENTION_LOCAL_FILE_BYTES)? else {
        return Ok(state);
    };
    if bytes.len() as u64 > MAX_ATTENTION_LOCAL_FILE_BYTES {
        return Err(AttentionLocalStateError::DocumentInvalid);
    }
    let schema = serde_json::from_slice::<serde_json::Value>(&bytes)?
        .get("schema")
        .and_then(serde_json::Value::as_u64)
        .ok_or(AttentionLocalStateError::DocumentInvalid)?;
    if schema == 1 {
        let document: AttentionLocalDocumentV1 = serde_json::from_slice(&bytes)?;
        if document.schema != 1 || document.entries.len() > capacity {
            return Err(AttentionLocalStateError::DocumentInvalid);
        }
        // V1 tuples were not target-qualified. They cannot be safely
        // attributed after a remap, so migration deliberately discards them
        // rather than inheriting read/notification custody.
        return Ok(state);
    }
    if schema != u64::from(ATTENTION_LOCAL_SCHEMA) {
        return Err(AttentionLocalStateError::DocumentInvalid);
    }
    let document: AttentionLocalDocument = serde_json::from_slice(&bytes)?;
    let (entries, retirements) = (document.entries, document.retirements);
    if entries.len() > capacity || retirements.len() > capacity {
        return Err(AttentionLocalStateError::DocumentInvalid);
    }
    for entry in entries {
        if !entry.read && !entry.notified {
            return Err(AttentionLocalStateError::DocumentInvalid);
        }
        let source = AttentionSource::new(
            AttentionSourceKind::parse(&entry.source_kind)
                .map_err(|_| AttentionLocalStateError::DocumentInvalid)?,
            AttentionSourceId::new(entry.source_id)
                .map_err(|_| AttentionLocalStateError::DocumentInvalid)?,
        );
        let key = AttentionLocalKey::new(
            PlatformAttentionTarget {
                project: ProjectId::new(entry.project_id)
                    .map_err(|_| AttentionLocalStateError::DocumentInvalid)?,
                user_workspace: UserWorkspaceId::new(entry.workspace_id)
                    .map_err(|_| AttentionLocalStateError::DocumentInvalid)?,
            },
            AttentionItemKey::new(
                source,
                AttentionItemId::new(entry.item_id)
                    .map_err(|_| AttentionLocalStateError::DocumentInvalid)?,
            ),
            Revision::new(entry.item_revision)
                .map_err(|_| AttentionLocalStateError::DocumentInvalid)?,
        );
        if state
            .entries
            .insert(
                key,
                AttentionLocalFlags {
                    read: entry.read,
                    notified: entry.notified,
                },
            )
            .is_some()
        {
            return Err(AttentionLocalStateError::DocumentInvalid);
        }
    }
    for retirement in retirements {
        let retirement = AttentionRetirement::new(
            PlatformAttentionTarget {
                project: ProjectId::new(retirement.project_id)
                    .map_err(|_| AttentionLocalStateError::DocumentInvalid)?,
                user_workspace: UserWorkspaceId::new(retirement.workspace_id)
                    .map_err(|_| AttentionLocalStateError::DocumentInvalid)?,
            },
            AttentionSource::new(
                AttentionSourceKind::parse(&retirement.source_kind)
                    .map_err(|_| AttentionLocalStateError::DocumentInvalid)?,
                AttentionSourceId::new(retirement.source_id)
                    .map_err(|_| AttentionLocalStateError::DocumentInvalid)?,
            ),
        );
        if !state.retirements.insert(retirement) {
            return Err(AttentionLocalStateError::DocumentInvalid);
        }
    }
    Ok(state)
}

fn persist_local_state(
    path: &Path,
    state: &AttentionLocalState,
) -> Result<(), AttentionLocalStateError> {
    let bytes = encode_local_state(state)?;
    secure_atomic_write(path, &bytes)?;
    Ok(())
}

fn encode_local_state(state: &AttentionLocalState) -> Result<Vec<u8>, AttentionLocalStateError> {
    let entries = state
        .entries
        .iter()
        .map(|(key, flags)| AttentionLocalEntry {
            project_id: key.target().project.as_str().to_owned(),
            workspace_id: key.target().user_workspace.as_str().to_owned(),
            source_kind: key.item().source().kind().as_str().to_owned(),
            source_id: key.item().source().id().as_str().to_owned(),
            item_id: key.item().item().as_str().to_owned(),
            item_revision: key.item_revision().get(),
            read: flags.read,
            notified: flags.notified,
        })
        .collect();
    let document = AttentionLocalDocument {
        schema: ATTENTION_LOCAL_SCHEMA,
        entries,
        retirements: state
            .retirements
            .iter()
            .map(|retirement| AttentionLocalRetirement {
                project_id: retirement.target().project.as_str().to_owned(),
                workspace_id: retirement.target().user_workspace.as_str().to_owned(),
                source_kind: retirement.source().kind().as_str().to_owned(),
                source_id: retirement.source().id().as_str().to_owned(),
            })
            .collect(),
    };
    let bytes = serde_json::to_vec_pretty(&document)?;
    if bytes.len() as u64 > MAX_ATTENTION_LOCAL_FILE_BYTES {
        return Err(AttentionLocalStateError::DocumentInvalid);
    }
    Ok(bytes)
}

fn prepare_local_storage(path: &Path) -> Result<(), AttentionLocalStateError> {
    let parent = path.parent().ok_or_else(|| {
        AttentionLocalStateError::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "attention local-state path has no parent",
        ))
    })?;
    ensure_private_directory_io(parent)?;
    Ok(())
}

#[derive(Debug, Error)]
pub enum AttentionError {
    #[error("platform mapping is not exact")]
    MappingNotExact,
    #[error("platform mapping contains an invalid protocol identity")]
    MappingInvalid,
    #[error("attention inventory exceeds its client bound")]
    InventoryTooLarge,
    #[error("attention inventory repeats a work-context identity")]
    InventoryDuplicate,
    #[error("attention workspace is missing or ambiguous")]
    WorkspaceMissingOrAmbiguous,
    #[error("attention workspace does not belong to the exact project")]
    WorkspaceProjectMismatch,
    #[error("attention source identity is invalid")]
    SourceInvalid,
    #[error("attention provider source lacks its exact Platform session relation")]
    ProviderSessionRelationInvalid,
    #[error("attention source is duplicated")]
    SourceDuplicate,
    #[error("attention source inventory exceeds its per-workspace bound")]
    SourceInventoryTooLarge,
    #[error("attention source is not in the authoritative inventory")]
    SourceNotInventoried,
    #[error("attention source does not match the requested source")]
    SourceMismatch,
    #[error("attention snapshot target does not match the board")]
    TargetMismatch,
    #[error("attention source must begin at revision one")]
    InitialRevisionRequired,
    #[error("attention source successor is stale or discontinuous")]
    InvalidSuccessor,
    #[error("attention complete baseline rolls back retained source or item custody")]
    InvalidBaseline,
    #[error("same attention revision has different content")]
    ConflictingReplay,
    #[error("attention UI identity collides with another raw source/item tuple")]
    UiIdentityCollision,
}

#[derive(Debug, Error, Eq, PartialEq)]
pub enum AttentionActivationError {
    #[error("attention item is no longer present")]
    ItemMissing,
    #[error("attention item revision is stale")]
    ItemStale,
    #[error("attention workspace is missing or ambiguous in the current catalog")]
    WorkspaceMissingOrAmbiguous,
    #[error("attention provider coordinate is invalid")]
    CoordinateInvalid,
    #[error("attention provider session is missing or ambiguous")]
    SessionMissingOrAmbiguous,
    #[error("attention provider session is not fresh")]
    SessionNotFresh,
    #[error("attention provider session is retained by multiple panes")]
    PaneAmbiguous,
}

#[derive(Debug, Error)]
pub enum AttentionLocalStateError {
    #[error("attention local-state capacity is invalid")]
    CapacityInvalid,
    #[error("attention local-state capacity is exhausted")]
    CapacityExceeded,
    #[error("attention local-state document is invalid")]
    DocumentInvalid,
    #[error("attention local-state reconciliation is not source-exact")]
    ReconciliationInvalid,
    #[error("attention local-state source retirement is still pending")]
    RetirementPending,
    #[error("attention local-state I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("attention local-state serialization failed: {0}")]
    Serialization(#[from] serde_json::Error),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::workspace_catalog::{
        CatalogCheckoutId, CatalogProjectId, CheckoutHost, PlatformContextRef,
        PlatformMappingReconciliation, ProjectCheckout, ProjectRecord, RepositoryIdentity,
        WorkspaceLaunchIntake, WorkspaceLaunchRequest,
    };
    use crate::workspace_navigation::{
        PaneId, PaneLeaf, ProviderSessionBinding, WorkspaceCardState, WorkspaceNavigationAction,
        WorkspaceSurfaceState, WorkspaceTab, WorkspaceTabContent, WorkspaceTabId,
    };
    use automonique_protocol::platform::{
        Freshness, FreshnessState, ResourceAuthority, ResourceCoordinate, ResourceId, ResourceKind,
        ResourceRecord,
    };
    use automonique_protocol::platform_v2::{
        AttemptWorkspaceId, CheckoutId, V1SessionRef, WorkContextAttributes, WorkContextLabel,
        WorkContextLifecycle, WorkContextRelation, WorkSessionId,
    };
    use automonique_protocol::primitives::EpochMillis;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEMP_NONCE: AtomicU64 = AtomicU64::new(0);

    fn target() -> PlatformAttentionTarget {
        PlatformAttentionTarget {
            project: ProjectId::new("project-1").unwrap(),
            user_workspace: UserWorkspaceId::new("workspace-1").unwrap(),
        }
    }

    fn source(kind: AttentionSourceKind, id: &str) -> AttentionSource {
        AttentionSource::new(kind, AttentionSourceId::new(id.to_owned()).unwrap())
    }

    fn item(id: &str, revision: u64, unread: bool) -> AttentionItem {
        AttentionItem::new(
            AttentionItemId::new(id.to_owned()).unwrap(),
            Revision::new(revision).unwrap(),
            revision * 100,
            AttentionItemState::Blocked,
            AttentionItemReason::ExternalBlocker,
            unread,
            Vec::new(),
            None,
        )
        .unwrap()
    }

    fn provider_item_for(id: &str, revision: u64, session_id: &str) -> AttentionItem {
        AttentionItem::new(
            AttentionItemId::new(id).unwrap(),
            Revision::new(revision).unwrap(),
            revision * 100,
            AttentionItemState::Done,
            AttentionItemReason::Complete,
            true,
            Vec::new(),
            Some(
                V1SessionRef::new(ResourceCoordinate::new(
                    ResourceAuthority::Automonique,
                    ResourceKind::Session,
                    ResourceId::new(session_id).unwrap(),
                ))
                .unwrap(),
            ),
        )
        .unwrap()
    }

    fn provider_item(id: &str, revision: u64) -> AttentionItem {
        provider_item_for(id, revision, "platform-session-1")
    }

    fn snapshot(
        source: AttentionSource,
        revision: u64,
        previous: Option<u64>,
        items: Vec<AttentionItem>,
    ) -> AttentionSourceSnapshot {
        AttentionSourceSnapshot::new(
            source,
            target().project,
            target().user_workspace,
            Revision::new(revision).unwrap(),
            previous.map(|value| Revision::new(value).unwrap()),
            revision * 100,
            items,
        )
        .unwrap()
    }

    fn relation(kind: WorkContextRelationKind, target: WorkContextIdentity) -> WorkContextRelation {
        WorkContextRelation::new(kind, target).unwrap()
    }

    fn record(
        identity: WorkContextIdentity,
        lifecycle: WorkContextLifecycle,
        relations: Vec<WorkContextRelation>,
    ) -> WorkContextRecord {
        WorkContextRecord::new(
            identity,
            Revision::FIRST,
            lifecycle,
            WorkContextLabel::new("record").unwrap(),
            WorkContextAttributes::EMPTY,
            relations,
        )
        .unwrap()
    }

    fn workspace_record(project: &str, workspace: &str) -> WorkContextRecord {
        record(
            WorkContextIdentity::UserWorkspace(UserWorkspaceId::new(workspace).unwrap()),
            WorkContextLifecycle::Active,
            vec![
                relation(
                    WorkContextRelationKind::UserWorkspaceProject,
                    WorkContextIdentity::Project(ProjectId::new(project).unwrap()),
                ),
                relation(
                    WorkContextRelationKind::UserWorkspaceCheckout,
                    WorkContextIdentity::Checkout(CheckoutId::new("checkout-1").unwrap()),
                ),
            ],
        )
    }

    fn attempt_record(id: &str) -> WorkContextRecord {
        record(
            WorkContextIdentity::AttemptWorkspace(AttemptWorkspaceId::new(id).unwrap()),
            WorkContextLifecycle::Running,
            vec![relation(
                WorkContextRelationKind::AttemptUserWorkspace,
                WorkContextIdentity::UserWorkspace(UserWorkspaceId::new("workspace-1").unwrap()),
            )],
        )
    }

    fn session_record(id: &str, attempt: &str) -> WorkContextRecord {
        let platform_session = V1SessionRef::new(ResourceCoordinate::new(
            ResourceAuthority::Automonique,
            ResourceKind::Session,
            ResourceId::new(format!("platform-{id}")).unwrap(),
        ))
        .unwrap();
        record(
            WorkContextIdentity::Session(WorkSessionId::new(id).unwrap()),
            WorkContextLifecycle::Active,
            vec![
                relation(
                    WorkContextRelationKind::SessionAttemptWorkspace,
                    WorkContextIdentity::AttemptWorkspace(
                        AttemptWorkspaceId::new(attempt).unwrap(),
                    ),
                ),
                relation(
                    WorkContextRelationKind::SessionPlatformSession,
                    WorkContextIdentity::PlatformSession(platform_session),
                ),
            ],
        )
    }

    fn inventory(review: ReviewAttentionPresence) -> AttentionSourceInventory {
        AttentionSourceInventory::from_authoritative_records(
            target(),
            &[
                workspace_record("project-1", "workspace-1"),
                attempt_record("attempt-1"),
                session_record("session-1", "attempt-1"),
            ],
            review,
        )
        .unwrap()
    }

    fn local_key(source_id: &str, item_id: &str, revision: u64) -> AttentionLocalKey {
        AttentionLocalKey::new(
            target(),
            AttentionItemKey::new(
                source(AttentionSourceKind::Orchestration, source_id),
                AttentionItemId::new(item_id.to_owned()).unwrap(),
            ),
            Revision::new(revision).unwrap(),
        )
    }

    fn temp_path(name: &str) -> PathBuf {
        let base = std::env::temp_dir().join(format!(
            "shelldeck-platform-attention-{}-{}",
            std::process::id(),
            TEMP_NONCE.fetch_add(1, Ordering::Relaxed)
        ));
        base.join("private").join("state").join(name)
    }

    // SDTEST-1816 — SDUC-493
    #[test]
    fn sdtest_1816_inventory_is_exact_bounded_and_authority_derived() {
        let source_inventory = inventory(ReviewAttentionPresence::Present);
        assert_eq!(source_inventory.sources().len(), 3);
        assert!(source_inventory.contains(&source(AttentionSourceKind::Review, "workspace-1")));
        assert!(
            source_inventory.contains(&source(AttentionSourceKind::Orchestration, "workspace-1"))
        );
        assert!(
            source_inventory.contains(&source(AttentionSourceKind::ProviderSession, "session-1"))
        );

        let without_review = inventory(ReviewAttentionPresence::Absent);
        assert_eq!(without_review.sources().len(), 2);
        assert!(!without_review.contains(&source(AttentionSourceKind::Review, "workspace-1")));

        let wrong_project = AttentionSourceInventory::from_authoritative_records(
            target(),
            &[workspace_record("project-elsewhere", "workspace-1")],
            ReviewAttentionPresence::Absent,
        );
        assert!(matches!(
            wrong_project,
            Err(AttentionError::WorkspaceProjectMismatch)
        ));

        let duplicate = workspace_record("project-1", "workspace-1");
        let ambiguous = AttentionSourceInventory::from_authoritative_records(
            target(),
            &[duplicate.clone(), duplicate],
            ReviewAttentionPresence::Absent,
        );
        assert!(matches!(ambiguous, Err(AttentionError::InventoryDuplicate)));

        let mut records = vec![workspace_record("project-1", "workspace-1")];
        for index in 0..63 {
            let attempt = format!("attempt-{index}");
            records.push(attempt_record(&attempt));
            records.push(session_record(&format!("session-{index}"), &attempt));
        }
        let overflow = AttentionSourceInventory::from_authoritative_records(
            target(),
            &records,
            ReviewAttentionPresence::Present,
        );
        assert!(matches!(
            overflow,
            Err(AttentionError::SourceInventoryTooLarge)
        ));
    }

    // SDTEST-1817 — SDUC-493
    #[test]
    fn sdtest_1817_snapshot_replacement_is_atomic_and_retains_revision_custody() {
        let orchestration = source(AttentionSourceKind::Orchestration, "workspace-1");
        let current_after_restart = snapshot(
            orchestration.clone(),
            7,
            Some(6),
            vec![item("restart-baseline", 7, true)],
        );
        let mut fresh = PlatformAttentionBoard::new(inventory(ReviewAttentionPresence::Present));
        assert!(matches!(
            fresh.apply_read(
                &orchestration,
                AttentionReadResult::Snapshot(Box::new(current_after_restart.clone())),
            ),
            Err(AttentionError::InitialRevisionRequired)
        ));
        assert_eq!(
            fresh
                .apply_authenticated_baseline_read(
                    &orchestration,
                    AttentionReadResult::Snapshot(Box::new(current_after_restart.clone())),
                )
                .unwrap(),
            AttentionApplyOutcome::Inserted
        );
        let mut restarted =
            PlatformAttentionBoard::new(inventory(ReviewAttentionPresence::Present));
        restarted
            .apply_authenticated_baseline_read(
                &orchestration,
                AttentionReadResult::Snapshot(Box::new(current_after_restart)),
            )
            .unwrap();
        assert_eq!(
            restarted
                .retained_snapshot(&orchestration)
                .unwrap()
                .revision(),
            Revision::new(7).unwrap()
        );

        let mut board = PlatformAttentionBoard::new(inventory(ReviewAttentionPresence::Present));
        assert_eq!(
            board
                .replace_source(snapshot(
                    orchestration.clone(),
                    1,
                    None,
                    vec![item("old", 1, true)],
                ))
                .unwrap(),
            AttentionApplyOutcome::Inserted
        );
        let before = board.clone();
        let discontinuous = snapshot(
            orchestration.clone(),
            3,
            Some(2),
            vec![item("old", 2, true)],
        );
        assert!(matches!(
            board.replace_source(discontinuous.clone()),
            Err(AttentionError::InvalidSuccessor)
        ));
        assert_eq!(board, before, "rejected replacement changes nothing");
        assert_eq!(
            board
                .apply_authenticated_baseline_read(
                    &orchestration,
                    AttentionReadResult::Snapshot(Box::new(discontinuous)),
                )
                .unwrap(),
            AttentionApplyOutcome::Replaced
        );
        assert_eq!(
            board.retained_snapshot(&orchestration).unwrap().revision(),
            Revision::new(3).unwrap()
        );
        let before_regression = board.clone();
        let regressing_baseline = snapshot(
            orchestration.clone(),
            4,
            Some(3),
            vec![item("old", 1, true)],
        );
        assert!(matches!(
            board.apply_authenticated_baseline_read(
                &orchestration,
                AttentionReadResult::Snapshot(Box::new(regressing_baseline)),
            ),
            Err(AttentionError::InvalidBaseline)
        ));
        assert_eq!(board, before_regression);
        let forked_baseline = snapshot(
            orchestration.clone(),
            6,
            Some(1),
            vec![item("forked", 6, true)],
        );
        assert!(matches!(
            board.apply_authenticated_baseline_read(
                &orchestration,
                AttentionReadResult::Snapshot(Box::new(forked_baseline)),
            ),
            Err(AttentionError::InvalidBaseline)
        ));
        assert_eq!(board, before_regression);

        let first = board.retained_snapshot(&orchestration).unwrap().clone();
        assert_eq!(
            board.replace_source(first).unwrap(),
            AttentionApplyOutcome::ExactReplay
        );
        board
            .mark_unavailable(&orchestration, AttentionUnavailableReason::Transport)
            .unwrap();
        assert_eq!(board.visible_items().count(), 0);
        assert_eq!(
            board.retained_snapshot(&orchestration).unwrap().revision(),
            Revision::new(3).unwrap()
        );

        let empty = snapshot(orchestration.clone(), 4, Some(3), Vec::new());
        assert_eq!(
            board.replace_source(empty).unwrap(),
            AttentionApplyOutcome::Replaced
        );
        assert_eq!(board.visible_items().count(), 0);

        let new_incarnation = snapshot(
            orchestration.clone(),
            5,
            Some(4),
            vec![item("new-incarnation", 5, true)],
        );
        board.replace_source(new_incarnation).unwrap();
        let visible = board.visible_items().next().unwrap();
        assert_eq!(visible.key().item().as_str(), "new-incarnation");

        let review = source(AttentionSourceKind::Review, "workspace-1");
        let before_mismatched_read = board.clone();
        assert!(matches!(
            board.apply_read(
                &review,
                AttentionReadResult::Snapshot(Box::new(snapshot(
                    orchestration.clone(),
                    6,
                    Some(5),
                    Vec::new(),
                ))),
            ),
            Err(AttentionError::SourceMismatch)
        ));
        assert_eq!(board, before_mismatched_read);

        let refusal = PlatformV2Refusal::new("not_authorized", "refused").unwrap();
        assert_eq!(
            board
                .apply_read(&orchestration, AttentionReadResult::Refused(refusal))
                .unwrap(),
            AttentionApplyOutcome::Refused
        );
        assert_eq!(board.visible_items().count(), 0);
        assert_eq!(
            board.retained_snapshot(&orchestration).unwrap().revision(),
            Revision::new(5).unwrap()
        );
        let replay = board.retained_snapshot(&orchestration).unwrap().clone();
        assert_eq!(
            board.replace_source(replay).unwrap(),
            AttentionApplyOutcome::AvailabilityRestored
        );

        let provider = source(AttentionSourceKind::ProviderSession, "session-1");
        board
            .replace_source(snapshot(
                provider.clone(),
                1,
                None,
                vec![provider_item("provider-item", 1)],
            ))
            .unwrap();
        let provider_ui_id = board
            .visible_items()
            .find(|item| item.key().source() == &provider)
            .unwrap()
            .ui_id();
        let new_inventory = AttentionSourceInventory::from_authoritative_records(
            target(),
            &[workspace_record("project-1", "workspace-1")],
            ReviewAttentionPresence::Absent,
        )
        .unwrap();
        board.replace_inventory(new_inventory).unwrap();
        assert!(board.retained_snapshot(&provider).is_none());
        assert!(board.item_by_ui_id(provider_ui_id).is_none());

        board
            .replace_inventory(inventory(ReviewAttentionPresence::Present))
            .unwrap();
        assert!(matches!(
            board.replace_source(snapshot(
                provider.clone(),
                5,
                Some(4),
                vec![provider_item("provider-current", 5)],
            )),
            Err(AttentionError::InitialRevisionRequired)
        ));
        board
            .apply_authenticated_baseline_read(
                &provider,
                AttentionReadResult::Snapshot(Box::new(snapshot(
                    provider.clone(),
                    5,
                    Some(4),
                    vec![provider_item("provider-current", 5)],
                ))),
            )
            .unwrap();
        assert_eq!(
            board.retained_snapshot(&provider).unwrap().revision(),
            Revision::new(5).unwrap()
        );
    }

    // SDTEST-1818 — SDUC-493
    #[test]
    fn sdtest_1818_ui_identity_is_source_scoped_and_collisions_fail_atomically() {
        let left = AttentionItemKey::new(
            source(AttentionSourceKind::Orchestration, "a:b"),
            AttentionItemId::new("c").unwrap(),
        );
        let right = AttentionItemKey::new(
            source(AttentionSourceKind::Orchestration, "a"),
            AttentionItemId::new("b:c").unwrap(),
        );
        assert_ne!(
            AttentionUiItemId::from_authoritative_key(&left),
            AttentionUiItemId::from_authoritative_key(&right),
            "length prefixes prevent delimiter aliasing"
        );
        assert_eq!(
            AttentionUiItemId::from_authoritative_key(&left),
            AttentionUiItemId::from_authoritative_key(&left),
            "the presentation UUID is deterministic"
        );
        assert_eq!(
            AttentionUiItemId::from_authoritative_key(&left).uuid(),
            Uuid::parse_str("4b8762bc-3aa9-5973-815c-d0e44ba41aa9").unwrap(),
            "the documented v1 namespace and tuple encoding are release-stable"
        );

        let review = source(AttentionSourceKind::Review, "workspace-1");
        let orchestration = source(AttentionSourceKind::Orchestration, "workspace-1");
        let mut board = PlatformAttentionBoard::new(inventory(ReviewAttentionPresence::Present));
        let forced = AttentionUiItemId(Uuid::nil());
        board
            .replace_source_with(
                snapshot(review.clone(), 1, None, vec![item("review-item", 1, true)]),
                |_| forced,
            )
            .unwrap();
        let before = board.clone();
        assert!(matches!(
            board.replace_source_with(
                snapshot(
                    orchestration.clone(),
                    1,
                    None,
                    vec![item("orchestration-item", 1, true)],
                ),
                |_| forced,
            ),
            Err(AttentionError::UiIdentityCollision)
        ));
        assert_eq!(board, before);
        assert!(board.retained_snapshot(&orchestration).is_none());
        assert_eq!(board.item_by_ui_id(forced).unwrap().key().source(), &review);
    }

    // SDTEST-1819 — SDUC-493
    #[test]
    fn sdtest_1819_local_overlay_is_revision_bound_durable_and_fail_closed() {
        let path = temp_path("attention.json");
        let first = local_key("workspace-1", "item-1", 1);
        let next_revision = local_key("workspace-1", "item-1", 2);
        let second = local_key("workspace-1", "item-2", 1);
        let mut store = AttentionLocalStateStore::open(path.clone(), 2).unwrap();
        assert_eq!(
            store.reserve_notification(first.clone(), false).unwrap(),
            NotificationReservation::Ineligible
        );
        assert!(!path.exists(), "ineligible state does not create custody");
        assert!(store.record_read(first.clone()).unwrap());
        assert_eq!(
            store.reserve_notification(first.clone(), true).unwrap(),
            NotificationReservation::Reserved
        );
        assert_eq!(
            store.reserve_notification(first.clone(), true).unwrap(),
            NotificationReservation::AlreadyReserved
        );

        let mut restarted = AttentionLocalStateStore::open(path.clone(), 2).unwrap();
        assert!(restarted.state().is_read(&first));
        assert!(restarted.state().is_notified(&first));
        assert!(!restarted.state().is_read(&next_revision));
        assert!(!restarted.state().is_notified(&next_revision));
        restarted
            .reserve_notification(next_revision.clone(), true)
            .unwrap();
        let durable_before = std::fs::read(&path).unwrap();
        assert!(matches!(
            restarted.record_read(second),
            Err(AttentionLocalStateError::CapacityExceeded)
        ));
        assert_eq!(std::fs::read(&path).unwrap(), durable_before);

        let current = BTreeSet::from([next_revision.clone()]);
        restarted
            .reconcile_source(
                next_revision.target(),
                next_revision.item().source(),
                current,
            )
            .unwrap();
        assert!(!restarted.state().is_read(&first));
        assert!(restarted.state().is_notified(&next_revision));
        let before_wrong_source = std::fs::read(&path).unwrap();
        assert!(matches!(
            restarted.reconcile_source(
                next_revision.target(),
                &source(AttentionSourceKind::Review, "workspace-1"),
                BTreeSet::from([next_revision.clone()]),
            ),
            Err(AttentionLocalStateError::ReconciliationInvalid)
        ));
        assert_eq!(std::fs::read(&path).unwrap(), before_wrong_source);

        let failing_path = temp_path("will-be-directory");
        let mut failing = AttentionLocalStateStore::open(failing_path.clone(), 2).unwrap();
        std::fs::create_dir(&failing_path).unwrap();
        assert!(matches!(
            failing.record_read(first.clone()),
            Err(AttentionLocalStateError::Io(_))
        ));
        assert!(!failing.state().is_read(&first));

        let invalid_path = temp_path("invalid.json");
        prepare_local_storage(&invalid_path).unwrap();
        std::fs::write(
            &invalid_path,
            br#"{"schema":1,"entries":[],"invented":true}"#,
        )
        .unwrap();
        assert!(matches!(
            AttentionLocalStateStore::open(invalid_path, 2),
            Err(AttentionLocalStateError::Serialization(_))
        ));

        let stale_handle_path = temp_path("stale-handles.json");
        let stale_key = local_key("workspace-1", "stale-handle-item", 1);
        let mut first_handle =
            AttentionLocalStateStore::open(stale_handle_path.clone(), 2).unwrap();
        let mut second_handle =
            AttentionLocalStateStore::open(stale_handle_path.clone(), 2).unwrap();
        assert_eq!(
            first_handle
                .reserve_notification(stale_key.clone(), true)
                .unwrap(),
            NotificationReservation::Reserved
        );
        assert_eq!(
            second_handle
                .reserve_notification(stale_key.clone(), true)
                .unwrap(),
            NotificationReservation::AlreadyReserved
        );
        assert!(second_handle.state().is_notified(&stale_key));

        let oversized_path = temp_path("oversized.json");
        prepare_local_storage(&oversized_path).unwrap();
        let oversized_source = source(
            AttentionSourceKind::Orchestration,
            &"s".repeat(automonique_protocol::platform_v2_attention::MAX_ATTENTION_FIELD_BYTES),
        );
        let large_state = |count: usize| {
            let mut state =
                AttentionLocalState::with_capacity(MAX_ATTENTION_LOCAL_ENTRIES).unwrap();
            for index in 0..count {
                let item_id = format!("{index:04}{}", "i".repeat(252));
                state.entries.insert(
                    AttentionLocalKey::new(
                        target(),
                        AttentionItemKey::new(
                            oversized_source.clone(),
                            AttentionItemId::new(item_id).unwrap(),
                        ),
                        Revision::FIRST,
                    ),
                    AttentionLocalFlags {
                        read: true,
                        notified: false,
                    },
                );
            }
            state
        };
        let mut fits = 0;
        let mut exceeds = MAX_ATTENTION_LOCAL_ENTRIES;
        assert!(encode_local_state(&large_state(exceeds)).is_err());
        while fits + 1 < exceeds {
            let candidate = fits + (exceeds - fits) / 2;
            if encode_local_state(&large_state(candidate)).is_ok() {
                fits = candidate;
            } else {
                exceeds = candidate;
            }
        }
        let accepted_large = large_state(fits);
        persist_local_state(&oversized_path, &accepted_large).unwrap();
        let durable_large = std::fs::read(&oversized_path).unwrap();
        let mut size_fenced =
            AttentionLocalStateStore::open(oversized_path.clone(), MAX_ATTENTION_LOCAL_ENTRIES)
                .unwrap();
        let overflow_key = AttentionLocalKey::new(
            target(),
            AttentionItemKey::new(
                oversized_source,
                AttentionItemId::new(format!("{fits:04}{}", "i".repeat(252))).unwrap(),
            ),
            Revision::FIRST,
        );
        assert!(matches!(
            size_fenced.record_read(overflow_key.clone()),
            Err(AttentionLocalStateError::DocumentInvalid)
        ));
        assert!(!size_fenced.state().is_read(&overflow_key));
        assert_eq!(std::fs::read(&oversized_path).unwrap(), durable_large);

        std::fs::remove_dir_all(path.parent().unwrap()).ok();
        std::fs::remove_dir_all(failing_path.parent().unwrap()).ok();
    }

    // SDTEST-1837 — SDUC-493
    #[test]
    fn sdtest_1837_retirement_fence_survives_failed_cleanup_and_restart() {
        let path = temp_path("retirement-restart.json");
        let key = local_key("workspace-1", "retired-item", 1);
        let retirement = AttentionRetirement::new(
            target(),
            source(AttentionSourceKind::Orchestration, "workspace-1"),
        );
        let mut store = AttentionLocalStateStore::open(path.clone(), 4).unwrap();
        assert!(store.record_read(key.clone()).unwrap());
        assert!(store.begin_retirement(retirement.clone()).unwrap());
        assert!(store.pending_retirements().contains(&retirement));
        assert!(matches!(
            store.record_read(key.clone()),
            Err(AttentionLocalStateError::RetirementPending)
        ));
        assert!(matches!(
            store.reserve_notification(key.clone(), true),
            Err(AttentionLocalStateError::RetirementPending)
        ));

        let durable_fenced = std::fs::read(&path).unwrap();
        let displaced = path.with_extension("fenced-backup");
        std::fs::rename(&path, &displaced).unwrap();
        std::fs::create_dir(&path).unwrap();
        assert!(matches!(
            store.finish_retirement(&retirement),
            Err(AttentionLocalStateError::Io(_))
        ));
        assert!(store.state().is_read(&key));
        assert!(store.pending_retirements().contains(&retirement));

        std::fs::remove_dir(&path).unwrap();
        std::fs::rename(&displaced, &path).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), durable_fenced);
        let mut restarted = AttentionLocalStateStore::open(path.clone(), 4).unwrap();
        assert!(restarted.state().is_read(&key));
        assert!(restarted.pending_retirements().contains(&retirement));
        assert!(restarted.finish_retirement(&retirement).unwrap());
        assert!(!restarted.state().is_read(&key));
        assert!(restarted.pending_retirements().is_empty());

        let final_restart = AttentionLocalStateStore::open(path.clone(), 4).unwrap();
        assert!(!final_restart.state().is_read(&key));
        assert!(final_restart.pending_retirements().is_empty());

        let legacy_path = temp_path("unscoped-v1.json");
        prepare_local_storage(&legacy_path).unwrap();
        std::fs::write(
            &legacy_path,
            br#"{
              "schema": 1,
              "entries": [{
                "source_kind": "orchestration",
                "source_id": "workspace-1",
                "item_id": "retired-item",
                "item_revision": 1,
                "read": true,
                "notified": true
              }]
            }"#,
        )
        .unwrap();
        let mut migrated = AttentionLocalStateStore::open(legacy_path.clone(), 4).unwrap();
        assert!(
            !migrated.state().is_read(&key),
            "unscoped v1 custody is discarded"
        );
        migrated.record_read(key.clone()).unwrap();
        let migrated_document = std::fs::read_to_string(&legacy_path).unwrap();
        assert!(migrated_document.contains("\"schema\": 2"));
        assert!(migrated_document.contains("\"project_id\": \"project-1\""));
        assert!(migrated_document.contains("\"workspace_id\": \"workspace-1\""));
        std::fs::remove_dir_all(path.parent().unwrap()).ok();
        std::fs::remove_dir_all(legacy_path.parent().unwrap()).ok();
    }

    // SDTEST-1820 — SDUC-493
    #[test]
    fn sdtest_1820_notification_reservation_is_locked_across_processes() {
        const CHILD_PATH: &str = "SHELLDECK_ATTENTION_CHILD_PATH";
        const CHILD_READY: &str = "SHELLDECK_ATTENTION_CHILD_READY";
        const CHILD_START: &str = "SHELLDECK_ATTENTION_CHILD_START";
        const CHILD_RESULT: &str = "SHELLDECK_ATTENTION_CHILD_RESULT";
        const TEST_NAME: &str = "config::platform_attention::tests::sdtest_1820_notification_reservation_is_locked_across_processes";

        fn wait_for(path: &Path) {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
            while !path.exists() {
                assert!(
                    std::time::Instant::now() < deadline,
                    "timed out waiting for {}",
                    path.display()
                );
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
        }

        if let Some(path) = std::env::var_os(CHILD_PATH) {
            let path = PathBuf::from(path);
            let ready = PathBuf::from(std::env::var_os(CHILD_READY).unwrap());
            let start = PathBuf::from(std::env::var_os(CHILD_START).unwrap());
            let result = PathBuf::from(std::env::var_os(CHILD_RESULT).unwrap());
            let mut store = AttentionLocalStateStore::open(path, 2).unwrap();
            std::fs::write(&ready, b"ready").unwrap();
            wait_for(&start);
            let outcome = store
                .reserve_notification(local_key("workspace-1", "process-race", 1), true)
                .unwrap();
            let value = match outcome {
                NotificationReservation::Reserved => "reserved",
                NotificationReservation::AlreadyReserved => "already",
                NotificationReservation::Ineligible => panic!("unread item became ineligible"),
            };
            std::fs::write(result, value).unwrap();
            return;
        }

        let path = temp_path("process-race.json");
        let base = path
            .parent()
            .and_then(Path::parent)
            .and_then(Path::parent)
            .unwrap()
            .to_path_buf();
        std::fs::create_dir_all(&base).unwrap();
        let start = base.join("start");
        let executable = std::env::current_exe().unwrap();
        let mut children = Vec::new();
        for index in 0..2 {
            let ready = base.join(format!("ready-{index}"));
            let result = base.join(format!("result-{index}"));
            let child = std::process::Command::new(&executable)
                .arg("--exact")
                .arg(TEST_NAME)
                .arg("--nocapture")
                .env(CHILD_PATH, &path)
                .env(CHILD_READY, &ready)
                .env(CHILD_START, &start)
                .env(CHILD_RESULT, &result)
                .spawn()
                .unwrap();
            children.push((child, ready, result));
        }
        for (_, ready, _) in &children {
            wait_for(ready);
        }
        std::fs::write(&start, b"start").unwrap();

        let mut outcomes = Vec::new();
        for (mut child, _, result) in children {
            assert!(child.wait().unwrap().success());
            outcomes.push(std::fs::read_to_string(result).unwrap());
        }
        outcomes.sort();
        assert_eq!(outcomes, vec!["already".to_owned(), "reserved".to_owned()]);

        let restarted = AttentionLocalStateStore::open(path, 2).unwrap();
        assert!(restarted
            .state()
            .is_notified(&local_key("workspace-1", "process-race", 1)));
        std::fs::remove_dir_all(base).ok();
    }

    // SDTEST-1821 — SDUC-493
    #[cfg(unix)]
    #[test]
    fn sdtest_1821_attention_storage_refuses_links_and_reads_one_descriptor() {
        use std::os::unix::fs::symlink;

        let linked_path = temp_path("linked.json");
        prepare_local_storage(&linked_path).unwrap();
        let base = linked_path
            .parent()
            .and_then(Path::parent)
            .and_then(Path::parent)
            .unwrap()
            .to_path_buf();
        let outside = base.join("outside.json");
        std::fs::write(&outside, br#"{"not":"attention custody"}"#).unwrap();

        symlink(&outside, &linked_path).unwrap();
        assert!(AttentionLocalStateStore::open(linked_path.clone(), 2).is_err());
        std::fs::remove_file(&linked_path).unwrap();
        symlink(base.join("missing.json"), &linked_path).unwrap();
        assert!(AttentionLocalStateStore::open(linked_path.clone(), 2).is_err());
        std::fs::remove_file(&linked_path).unwrap();

        let lock_attack_path = temp_path("linked-lock.json");
        let lock_attack_base = lock_attack_path
            .parent()
            .and_then(Path::parent)
            .and_then(Path::parent)
            .unwrap()
            .to_path_buf();
        prepare_local_storage(&lock_attack_path).unwrap();
        symlink(&outside, lock_path(&lock_attack_path)).unwrap();
        assert!(AttentionLocalStateStore::open(lock_attack_path.clone(), 2).is_err());
        std::fs::remove_file(lock_path(&lock_attack_path)).unwrap();

        let key = local_key("workspace-1", "descriptor", 1);
        let mut store = AttentionLocalStateStore::open(linked_path.clone(), 2).unwrap();
        store.record_read(key).unwrap();
        let accepted = std::fs::read(&linked_path).unwrap();
        let moved = base.join("accepted.json");
        let bytes = crate::workspace_review::storage::bounded_descriptor_read_after_open(
            &linked_path,
            MAX_ATTENTION_LOCAL_FILE_BYTES,
            || {
                std::fs::rename(&linked_path, &moved).unwrap();
                symlink(&outside, &linked_path).unwrap();
            },
        )
        .unwrap()
        .unwrap();
        assert_eq!(bytes, accepted);
        std::fs::remove_file(&linked_path).unwrap();
        std::fs::rename(&moved, &linked_path).unwrap();

        let linked_parent_path = temp_path("parent-link.json");
        let private_root = linked_parent_path
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .to_path_buf();
        let parent_base = private_root.parent().unwrap().to_path_buf();
        std::fs::create_dir_all(&parent_base).unwrap();
        let outside_root = parent_base.join("outside-root");
        std::fs::create_dir_all(outside_root.join("state")).unwrap();
        symlink(&outside_root, &private_root).unwrap();
        assert!(AttentionLocalStateStore::open(linked_parent_path, 2).is_err());

        std::fs::remove_dir_all(base).ok();
        std::fs::remove_dir_all(lock_attack_base).ok();
        std::fs::remove_file(&private_root).ok();
        std::fs::remove_dir_all(&parent_base).ok();
    }

    // Windows release runners exercise the same no-follow path using native
    // reparse-point metadata and FILE_FLAG_OPEN_REPARSE_POINT.
    #[cfg(windows)]
    #[test]
    fn sdtest_1821_attention_storage_refuses_windows_reparse_points() {
        let linked_path = temp_path("parent-reparse.json");
        let private_root = linked_path
            .parent()
            .and_then(Path::parent)
            .unwrap()
            .to_path_buf();
        let base = private_root.parent().unwrap().to_path_buf();
        let outside = base.join("outside-root");
        std::fs::create_dir_all(outside.join("state")).unwrap();
        let status = std::process::Command::new("cmd")
            .arg("/C")
            .arg("mklink")
            .arg("/J")
            .arg(&private_root)
            .arg(&outside)
            .status()
            .unwrap();
        assert!(status.success(), "failed to create test junction");
        assert!(AttentionLocalStateStore::open(linked_path, 2).is_err());
        std::fs::remove_dir(private_root).ok();
        std::fs::remove_dir_all(base).ok();
    }

    fn activation_catalog() -> (ProjectCatalog, CatalogWorkspaceId, CatalogCheckoutId) {
        let project = CatalogProjectId::from_uuid(Uuid::from_u128(900));
        let checkout = CatalogCheckoutId::from_uuid(Uuid::from_u128(901));
        let workspace = CatalogWorkspaceId::from_uuid(Uuid::from_u128(902));
        let mut project_record = ProjectRecord::new(project, "Attention project");
        project_record.add_checkout(ProjectCheckout::new(
            checkout,
            "main",
            CheckoutHost::Local {
                device_label: "local".into(),
                root: std::env::temp_dir().join("shelldeck-attention-activation"),
            },
            RepositoryIdentity {
                slug: "bext/shelldeck".into(),
                canonical_url: None,
            },
        ));
        let mut catalog = ProjectCatalog::default();
        catalog.insert_project(project_record).unwrap();
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
        (catalog, workspace, checkout)
    }

    fn provider_surface(count: usize) -> WorkspaceSurfaceState {
        let tabs = (0..count)
            .map(|index| WorkspaceTab {
                id: WorkspaceTabId::from_uuid(Uuid::from_u128(920 + index as u128)),
                title: "Provider".into(),
                content: WorkspaceTabContent::ProviderSession(ProviderSessionBinding {
                    platform_user_workspace_id: "workspace-1".into(),
                    session_id: "platform-session-1".into(),
                    run_id: None,
                }),
            })
            .collect::<Vec<_>>();
        let active_tab = tabs.first().map(|tab| tab.id);
        WorkspaceSurfaceState {
            root: Some(PaneNode::Leaf(PaneLeaf {
                id: PaneId::from_uuid(Uuid::from_u128(919)),
                tabs,
                active_tab,
            })),
            focus: active_tab.map(|tab_id| WorkspaceFocus {
                pane_id: PaneId::from_uuid(Uuid::from_u128(919)),
                tab_id,
            }),
        }
    }

    fn authorized_session_with_freshness(state: FreshnessState) -> SessionRecord {
        SessionRecord {
            session: ResourceRecord {
                resource: ResourceCoordinate::new(
                    ResourceAuthority::Automonique,
                    ResourceKind::Session,
                    ResourceId::new("platform-session-1").unwrap(),
                ),
                freshness: Freshness {
                    state,
                    observed_at: EpochMillis::from_millis(1),
                    revision: Revision::FIRST,
                },
                summary: automonique_protocol::platform::PlatformText::new("session").unwrap(),
            },
            run: None,
            attachable: true,
            controllable: false,
        }
    }

    fn authorized_session() -> SessionRecord {
        authorized_session_with_freshness(FreshnessState::Fresh)
    }

    // SDTEST-1829
    #[test]
    fn sdtest_1829_activation_re_resolves_exact_current_catalogues_and_refuses_ambiguity() {
        let (catalog, workspace, _) = activation_catalog();
        let mut navigation = WorkspaceNavigationState::default();
        navigation
            .reduce(
                &catalog,
                WorkspaceNavigationAction::Retain {
                    id: workspace,
                    surface: provider_surface(0),
                    card: WorkspaceCardState::default(),
                },
            )
            .unwrap();
        let provider = source(AttentionSourceKind::ProviderSession, "session-1");
        let mut board = PlatformAttentionBoard::new(inventory(ReviewAttentionPresence::Present));
        board
            .apply_authenticated_baseline_read(
                &provider,
                AttentionReadResult::Snapshot(Box::new(snapshot(
                    provider.clone(),
                    1,
                    None,
                    vec![provider_item("provider", 1)],
                ))),
            )
            .unwrap();
        let visible = board.visible_items().next().unwrap();
        let activation = PlatformAttentionActivation {
            workspace,
            item: visible.ui_id(),
            item_revision: visible.value().revision(),
        };
        let sessions = vec![authorized_session()];
        assert!(matches!(
            resolve_platform_attention_activation(
                activation,
                &board,
                &catalog,
                &navigation,
                &sessions,
            ),
            Ok(PlatformAttentionDestination::FleetSession { .. })
        ));
        assert_eq!(
            resolve_platform_attention_activation(activation, &board, &catalog, &navigation, &[],),
            Err(AttentionActivationError::SessionMissingOrAmbiguous)
        );
        let duplicate = vec![authorized_session(), authorized_session()];
        assert_eq!(
            resolve_platform_attention_activation(
                activation,
                &board,
                &catalog,
                &navigation,
                &duplicate,
            ),
            Err(AttentionActivationError::SessionMissingOrAmbiguous)
        );
        for state in [FreshnessState::Stale, FreshnessState::Unknown] {
            assert_eq!(
                resolve_platform_attention_activation(
                    activation,
                    &board,
                    &catalog,
                    &navigation,
                    &[authorized_session_with_freshness(state)],
                ),
                Err(AttentionActivationError::SessionNotFresh)
            );
        }

        let mut redirected =
            PlatformAttentionBoard::new(inventory(ReviewAttentionPresence::Present));
        redirected
            .apply_authenticated_baseline_read(
                &provider,
                AttentionReadResult::Snapshot(Box::new(snapshot(
                    provider.clone(),
                    1,
                    None,
                    vec![provider_item_for("provider", 1, "platform-session-foreign")],
                ))),
            )
            .unwrap();
        let redirected_item = redirected.visible_items().next().unwrap();
        let redirected_activation = PlatformAttentionActivation {
            workspace,
            item: redirected_item.ui_id(),
            item_revision: redirected_item.value().revision(),
        };
        let mut foreign = authorized_session();
        foreign.session.resource.id = ResourceId::new("platform-session-foreign").unwrap();
        assert_eq!(
            resolve_platform_attention_activation(
                redirected_activation,
                &redirected,
                &catalog,
                &navigation,
                &[foreign],
            ),
            Err(AttentionActivationError::CoordinateInvalid),
            "an item cannot redirect its provider source to another current session"
        );

        navigation
            .reduce(
                &catalog,
                WorkspaceNavigationAction::UpdateSurface {
                    id: workspace,
                    surface: provider_surface(1),
                },
            )
            .unwrap();
        assert!(matches!(
            resolve_platform_attention_activation(
                activation,
                &board,
                &catalog,
                &navigation,
                &sessions,
            ),
            Ok(PlatformAttentionDestination::RetainedProviderPane { .. })
        ));
        navigation
            .reduce(
                &catalog,
                WorkspaceNavigationAction::UpdateSurface {
                    id: workspace,
                    surface: provider_surface(2),
                },
            )
            .unwrap();
        assert_eq!(
            resolve_platform_attention_activation(
                activation,
                &board,
                &catalog,
                &navigation,
                &sessions,
            ),
            Err(AttentionActivationError::PaneAmbiguous)
        );

        board
            .apply_read(
                &provider,
                AttentionReadResult::Snapshot(Box::new(snapshot(
                    provider.clone(),
                    2,
                    Some(1),
                    vec![provider_item("provider", 2)],
                ))),
            )
            .unwrap();
        assert_eq!(
            resolve_platform_attention_activation(
                activation,
                &board,
                &catalog,
                &navigation,
                &sessions,
            ),
            Err(AttentionActivationError::ItemStale)
        );
    }

    // --- Shared cross-client attention succession corpus -------------------

    const ATTENTION_CORPUS: &[u8] =
        include_bytes!("../../tests/fixtures/platform-v2-attention-conformance-v1.json");

    #[derive(serde::Deserialize)]
    struct CorpusFile {
        schema: String,
        version: String,
        target: CorpusTarget,
        cases: Vec<CorpusCase>,
    }

    #[derive(serde::Deserialize)]
    struct CorpusTarget {
        project: String,
        user_workspace: String,
    }

    #[derive(serde::Deserialize)]
    struct CorpusCase {
        id: String,
        source: CorpusSource,
        reads: Vec<CorpusRead>,
        expected: CorpusExpected,
    }

    #[derive(serde::Deserialize)]
    struct CorpusSource {
        kind: String,
        id: String,
    }

    #[derive(serde::Deserialize)]
    #[serde(tag = "kind", rename_all = "snake_case")]
    enum CorpusRead {
        Snapshot {
            mode: String,
            outcome: String,
            snapshot: CorpusSnapshot,
        },
        Refusal {
            category: String,
        },
        Unavailable {
            reason: String,
        },
    }

    #[derive(serde::Deserialize)]
    struct CorpusSnapshot {
        source: CorpusSource,
        project: String,
        user_workspace: String,
        revision: String,
        previous_revision: Option<String>,
        observed_at_ms: String,
        items: Vec<CorpusItem>,
    }

    #[derive(serde::Deserialize)]
    struct CorpusItem {
        id: String,
        revision: String,
        observed_at_ms: String,
        state: String,
        reason: String,
        unread: bool,
        nested_agent_path: Vec<String>,
        platform_session: Option<CorpusSession>,
    }

    #[derive(serde::Deserialize)]
    struct CorpusSession {
        authority: String,
        kind: String,
        id: String,
    }

    #[derive(serde::Deserialize)]
    struct CorpusExpected {
        available: bool,
        visible_items: Vec<String>,
    }

    fn corpus_decimal(value: &str) -> u64 {
        assert!(
            !value.is_empty()
                && (value == "0"
                    || (!value.starts_with('0') && value.bytes().all(|b| b.is_ascii_digit()))),
            "corpus revision is not a canonical decimal: {value}"
        );
        value
            .parse()
            .expect("corpus revision fits the client fence")
    }

    fn corpus_target() -> PlatformAttentionTarget {
        PlatformAttentionTarget {
            project: ProjectId::new("project-conformance").unwrap(),
            user_workspace: UserWorkspaceId::new("workspace-conformance").unwrap(),
        }
    }

    /// The graph the corpus target implies: one workspace in its project, one
    /// attempt under it, and one session bound to an exact Platform session.
    fn corpus_inventory() -> AttentionSourceInventory {
        let workspace = record(
            WorkContextIdentity::UserWorkspace(
                UserWorkspaceId::new("workspace-conformance").unwrap(),
            ),
            WorkContextLifecycle::Active,
            vec![
                relation(
                    WorkContextRelationKind::UserWorkspaceProject,
                    WorkContextIdentity::Project(ProjectId::new("project-conformance").unwrap()),
                ),
                relation(
                    WorkContextRelationKind::UserWorkspaceCheckout,
                    WorkContextIdentity::Checkout(CheckoutId::new("checkout-conformance").unwrap()),
                ),
            ],
        );
        let attempt = record(
            WorkContextIdentity::AttemptWorkspace(
                AttemptWorkspaceId::new("attempt-conformance").unwrap(),
            ),
            WorkContextLifecycle::Running,
            vec![relation(
                WorkContextRelationKind::AttemptUserWorkspace,
                WorkContextIdentity::UserWorkspace(
                    UserWorkspaceId::new("workspace-conformance").unwrap(),
                ),
            )],
        );
        let platform_session = V1SessionRef::new(ResourceCoordinate::new(
            ResourceAuthority::Automonique,
            ResourceKind::Session,
            ResourceId::new("platform-session-conformance").unwrap(),
        ))
        .unwrap();
        let session = record(
            WorkContextIdentity::Session(WorkSessionId::new("session-conformance").unwrap()),
            WorkContextLifecycle::Active,
            vec![
                relation(
                    WorkContextRelationKind::SessionAttemptWorkspace,
                    WorkContextIdentity::AttemptWorkspace(
                        AttemptWorkspaceId::new("attempt-conformance").unwrap(),
                    ),
                ),
                relation(
                    WorkContextRelationKind::SessionPlatformSession,
                    WorkContextIdentity::PlatformSession(platform_session),
                ),
            ],
        );
        AttentionSourceInventory::from_authoritative_records(
            corpus_target(),
            &[workspace, attempt, session],
            ReviewAttentionPresence::Present,
        )
        .unwrap()
    }

    fn corpus_source(value: &CorpusSource) -> AttentionSource {
        let kind = match value.kind.as_str() {
            "review" => AttentionSourceKind::Review,
            "orchestration" => AttentionSourceKind::Orchestration,
            "provider_session" => AttentionSourceKind::ProviderSession,
            other => panic!("corpus source kind {other} is unknown"),
        };
        AttentionSource::new(kind, AttentionSourceId::new(value.id.clone()).unwrap())
    }

    fn corpus_state(value: &str) -> AttentionItemState {
        match value {
            "needs_you" => AttentionItemState::NeedsYou,
            "working" => AttentionItemState::Working,
            "blocked" => AttentionItemState::Blocked,
            "done" => AttentionItemState::Done,
            other => panic!("corpus item state {other} is unknown"),
        }
    }

    fn corpus_reason(value: &str) -> AttentionItemReason {
        match value {
            "review_requested" => AttentionItemReason::ReviewRequested,
            "comment_reply" => AttentionItemReason::CommentReply,
            "approval_required" => AttentionItemReason::ApprovalRequired,
            "agent_working" => AttentionItemReason::AgentWorking,
            "check_running" => AttentionItemReason::CheckRunning,
            "delivery_pending" => AttentionItemReason::DeliveryPending,
            "complete" => AttentionItemReason::Complete,
            "conflict" => AttentionItemReason::Conflict,
            "check_failed" => AttentionItemReason::CheckFailed,
            "external_blocker" => AttentionItemReason::ExternalBlocker,
            other => panic!("corpus item reason {other} is unknown"),
        }
    }

    fn corpus_snapshot(value: &CorpusSnapshot) -> AttentionSourceSnapshot {
        let items = value
            .items
            .iter()
            .map(|item| {
                let session = item.platform_session.as_ref().map(|coordinate| {
                    assert_eq!(coordinate.authority, "automonique");
                    assert_eq!(coordinate.kind, "session");
                    V1SessionRef::new(ResourceCoordinate::new(
                        ResourceAuthority::Automonique,
                        ResourceKind::Session,
                        ResourceId::new(coordinate.id.clone()).unwrap(),
                    ))
                    .unwrap()
                });
                AttentionItem::new(
                    AttentionItemId::new(item.id.clone()).unwrap(),
                    Revision::new(corpus_decimal(&item.revision)).unwrap(),
                    corpus_decimal(&item.observed_at_ms),
                    corpus_state(&item.state),
                    corpus_reason(&item.reason),
                    item.unread,
                    item.nested_agent_path
                        .iter()
                        .map(|agent| {
                            automonique_protocol::platform_v2_attention::AttentionAgentId::new(
                                agent.clone(),
                            )
                            .unwrap()
                        })
                        .collect(),
                    session,
                )
                .unwrap()
            })
            .collect();
        AttentionSourceSnapshot::new(
            corpus_source(&value.source),
            ProjectId::new(value.project.clone()).unwrap(),
            UserWorkspaceId::new(value.user_workspace.clone()).unwrap(),
            Revision::new(corpus_decimal(&value.revision)).unwrap(),
            value
                .previous_revision
                .as_ref()
                .map(|previous| Revision::new(corpus_decimal(previous)).unwrap()),
            corpus_decimal(&value.observed_at_ms),
            items,
        )
        .unwrap()
    }

    fn corpus_unavailable(reason: &str) -> AttentionUnavailableReason {
        match reason {
            "transport" => AttentionUnavailableReason::Transport,
            "inventory_incomplete" => AttentionUnavailableReason::InventoryIncomplete,
            other => panic!("corpus unavailable reason {other} is unknown"),
        }
    }

    /// Assert this board reached exactly what the corpus records for the read.
    fn assert_corpus_outcome(
        case: &str,
        outcome: &str,
        actual: &Result<AttentionApplyOutcome, AttentionError>,
    ) {
        let matched = match outcome {
            "inserted" => matches!(actual, Ok(AttentionApplyOutcome::Inserted)),
            "replaced" => matches!(actual, Ok(AttentionApplyOutcome::Replaced)),
            "exact_replay" => matches!(actual, Ok(AttentionApplyOutcome::ExactReplay)),
            "availability_restored" => {
                matches!(actual, Ok(AttentionApplyOutcome::AvailabilityRestored))
            }
            "initial_revision_required" => {
                matches!(actual, Err(AttentionError::InitialRevisionRequired))
            }
            "invalid_successor" => matches!(actual, Err(AttentionError::InvalidSuccessor)),
            "conflicting_replay" => matches!(actual, Err(AttentionError::ConflictingReplay)),
            "baseline_invalid" => matches!(actual, Err(AttentionError::InvalidBaseline)),
            other => panic!("corpus outcome {other} is unknown"),
        };
        assert!(
            matched,
            "case {case} expected {outcome}, board reached {actual:?}"
        );
    }

    // SDTEST-1856 — SDUC-493
    #[test]
    fn sdtest_1856_chronology_follows_authoritative_observation_not_source_order() {
        fn observed(id: &str, revision: u64, observed_at_ms: u64) -> AttentionItem {
            AttentionItem::new(
                AttentionItemId::new(id.to_owned()).unwrap(),
                Revision::new(revision).unwrap(),
                observed_at_ms,
                AttentionItemState::Blocked,
                AttentionItemReason::ExternalBlocker,
                true,
                Vec::new(),
                None,
            )
            .unwrap()
        }

        let review = source(AttentionSourceKind::Review, "workspace-1");
        let orchestration = source(AttentionSourceKind::Orchestration, "workspace-1");
        let mut board = PlatformAttentionBoard::new(inventory(ReviewAttentionPresence::Present));
        board
            .replace_source(
                AttentionSourceSnapshot::new(
                    review.clone(),
                    target().project,
                    target().user_workspace,
                    Revision::FIRST,
                    None,
                    900,
                    vec![observed("review-old", 1, 100)],
                )
                .unwrap(),
            )
            .unwrap();
        board
            .apply_authenticated_baseline_read(
                &orchestration,
                AttentionReadResult::Snapshot(Box::new(
                    AttentionSourceSnapshot::new(
                        orchestration.clone(),
                        target().project,
                        target().user_workspace,
                        Revision::new(3).unwrap(),
                        Some(Revision::new(2).unwrap()),
                        900,
                        vec![
                            observed("orchestration-newest", 3, 800),
                            observed("orchestration-tie-high", 3, 400),
                            observed("orchestration-tie-low", 2, 400),
                        ],
                    )
                    .unwrap(),
                )),
            )
            .unwrap();

        // Key order alone would put the review source first: it is the lowest
        // source kind. Chronology must contradict that.
        assert_eq!(
            board
                .visible_items()
                .map(|item| item.key().item().as_str().to_owned())
                .collect::<Vec<_>>(),
            vec![
                "review-old",
                "orchestration-newest",
                "orchestration-tie-high",
                "orchestration-tie-low",
            ]
        );
        assert_eq!(
            board
                .chronology()
                .iter()
                .map(|item| item.key().item().as_str().to_owned())
                .collect::<Vec<_>>(),
            vec![
                "orchestration-newest",
                // Equal observation falls back to the higher item revision.
                "orchestration-tie-high",
                "orchestration-tie-low",
                "review-old",
            ]
        );

        // A hidden source contributes no chronology, and hiding it never
        // reorders what survives.
        board
            .mark_unavailable(&orchestration, AttentionUnavailableReason::Transport)
            .unwrap();
        assert_eq!(
            board
                .chronology()
                .iter()
                .map(|item| item.key().item().as_str().to_owned())
                .collect::<Vec<_>>(),
            vec!["review-old"]
        );
    }

    // SDTEST-1842 — SDUC-493
    #[test]
    fn sdtest_1842_shared_attention_corpus_replays_to_the_recorded_outcomes() {
        let corpus: CorpusFile = serde_json::from_slice(ATTENTION_CORPUS).unwrap();
        assert_eq!(corpus.schema, "automonique.attention-conformance/v1");
        assert_eq!(corpus.version, "1");
        assert_eq!(corpus.target.project, "project-conformance");
        assert_eq!(corpus.target.user_workspace, "workspace-conformance");
        assert!(!corpus.cases.is_empty());

        for case in &corpus.cases {
            let source = corpus_source(&case.source);
            let mut board = PlatformAttentionBoard::new(corpus_inventory());
            assert!(
                board.inventory().contains(&source),
                "case {} names a source the corpus target does not inventory",
                case.id
            );

            for read in &case.reads {
                match read {
                    CorpusRead::Snapshot {
                        mode,
                        outcome,
                        snapshot,
                    } => {
                        let value = corpus_snapshot(snapshot);
                        let result = match mode.as_str() {
                            "continuous" => board.apply_read(
                                &source,
                                AttentionReadResult::Snapshot(Box::new(value)),
                            ),
                            "baseline" => board.apply_authenticated_baseline_read(
                                &source,
                                AttentionReadResult::Snapshot(Box::new(value)),
                            ),
                            other => panic!("corpus read mode {other} is unknown"),
                        };
                        assert_corpus_outcome(&case.id, outcome, &result);
                    }
                    CorpusRead::Refusal { category } => {
                        let refusal = PlatformV2Refusal::new(category.clone(), "corpus").unwrap();
                        board.mark_refused(&source, &refusal).unwrap();
                    }
                    CorpusRead::Unavailable { reason } => {
                        board
                            .mark_unavailable(&source, corpus_unavailable(reason))
                            .unwrap();
                    }
                }
            }

            let available = matches!(
                board.status(&source),
                Some(AttentionSourceStatus::Available)
            );
            assert_eq!(
                available, case.expected.available,
                "case {} reached the wrong availability",
                case.id
            );
            let visible = board
                .visible_items()
                .filter(|item| item.key().source() == &source)
                .map(|item| item.value().id().as_str().to_owned())
                .collect::<Vec<_>>();
            assert_eq!(
                visible, case.expected.visible_items,
                "case {} rendered the wrong items",
                case.id
            );
            // Hiding a source must not discard its revision chain: a client
            // that forgot where it was could only resynchronize by trusting
            // whatever the next read claims.
            let ever_accepted = case.reads.iter().any(|read| {
                matches!(
                    read,
                    CorpusRead::Snapshot { outcome, .. }
                        if outcome == "inserted" || outcome == "replaced"
                )
            });
            assert_eq!(
                board.retained_snapshot(&source).is_some(),
                ever_accepted,
                "case {} disagrees about whether the revision chain survived",
                case.id
            );
        }
    }
}
