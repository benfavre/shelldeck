//! Source-atomic native consumption of Platform v2 attention.
//!
//! This module deliberately stops before navigation. It retains the complete
//! authoritative source, project, user-workspace, item, revision, and optional
//! authority-qualified Platform session coordinates. It never manufactures a
//! pane, tab, terminal, path, or other client-local target.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;
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
use super::workspace_catalog::PlatformV2Mapping;

/// Same per-workspace ceiling as the authoritative hosted cockpit.
pub const MAX_ATTENTION_SOURCES_PER_WORKSPACE: usize = 64;
/// A client inventory is accepted only as one bounded, duplicate-free value.
pub const MAX_ATTENTION_INVENTORY_RECORDS: usize = 512;
/// Maximum locally retained read/notification tuples.
pub const MAX_ATTENTION_LOCAL_ENTRIES: usize = 4_096;
const MAX_ATTENTION_LOCAL_FILE_BYTES: u64 = 1024 * 1024;
const ATTENTION_LOCAL_SCHEMA: u16 = 1;

/// Namespace reserved for ShellDeck's presentation-only attention UUIDs.
///
/// The UUID is never sent to Automonique and is never an authority coordinate.
/// Its name bytes are the length-delimited raw `(source kind, source id, item
/// id)` tuple, so delimiter-bearing opaque identifiers cannot alias each other.
const ATTENTION_UI_NAMESPACE: Uuid = Uuid::from_bytes([
    0xa6, 0x61, 0x14, 0x3f, 0x23, 0x88, 0x5a, 0x25, 0xa4, 0xca, 0x53, 0x42, 0x6e, 0x92, 0x7d, 0x31,
]);

#[derive(Clone, Debug, Eq, PartialEq)]
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
                if !sources.insert(AttentionSource::new(
                    AttentionSourceKind::ProviderSession,
                    id,
                )) {
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlatformAttentionBoard {
    inventory: AttentionSourceInventory,
    slots: BTreeMap<AttentionSource, AttentionSourceSlot>,
    ui_index: BTreeMap<AttentionUiItemId, AttentionItemKey>,
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

    pub fn replace_source(
        &mut self,
        snapshot: AttentionSourceSnapshot,
    ) -> Result<AttentionApplyOutcome, AttentionError> {
        self.replace_source_with(snapshot, AttentionUiItemId::from_authoritative_key)
    }

    fn replace_source_with(
        &mut self,
        snapshot: AttentionSourceSnapshot,
        projector: impl Fn(&AttentionItemKey) -> AttentionUiItemId,
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
            current
                .snapshot
                .validate_successor(&snapshot)
                .map_err(|_| AttentionError::InvalidSuccessor)?;
        } else if snapshot.revision() != Revision::FIRST {
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
    item: AttentionItemKey,
    item_revision: Revision,
}

impl AttentionLocalKey {
    #[must_use]
    pub const fn new(item: AttentionItemKey, item_revision: Revision) -> Self {
        Self {
            item,
            item_revision,
        }
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
}

impl AttentionLocalState {
    pub fn with_capacity(capacity: usize) -> Result<Self, AttentionLocalStateError> {
        if capacity == 0 || capacity > MAX_ATTENTION_LOCAL_ENTRIES {
            return Err(AttentionLocalStateError::CapacityInvalid);
        }
        Ok(Self {
            capacity,
            entries: BTreeMap::new(),
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
        self.update(key, |flags| &mut flags.read)
    }

    fn mark_notified(&mut self, key: AttentionLocalKey) -> Result<bool, AttentionLocalStateError> {
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
        source: &AttentionSource,
        current: &BTreeSet<AttentionLocalKey>,
    ) -> bool {
        let before = self.entries.len();
        self.entries
            .retain(|key, _| key.item().source() != source || current.contains(key));
        before != self.entries.len()
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
            AppConfig::config_dir().join("platform-attention-local-v1.json"),
            MAX_ATTENTION_LOCAL_ENTRIES,
        )
    }

    pub fn open(path: PathBuf, capacity: usize) -> Result<Self, AttentionLocalStateError> {
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
        if self.state.is_notified(&key) {
            return Ok(NotificationReservation::AlreadyReserved);
        }
        self.transact(|state| state.mark_notified(key))?;
        Ok(NotificationReservation::Reserved)
    }

    /// Prune superseded/removed revisions only after accepting a complete
    /// authoritative source replacement. Refusal/unavailability must not call
    /// this method because neither means an empty source.
    pub fn reconcile_source(
        &mut self,
        source: &AttentionSource,
        current: BTreeSet<AttentionLocalKey>,
    ) -> Result<bool, AttentionLocalStateError> {
        if current.len() > MAX_ATTENTION_ITEMS
            || current.iter().any(|key| key.item().source() != source)
        {
            return Err(AttentionLocalStateError::ReconciliationInvalid);
        }
        self.transact(|state| Ok(state.retain_source_keys(source, &current)))
    }

    pub fn remove_source(
        &mut self,
        source: &AttentionSource,
    ) -> Result<bool, AttentionLocalStateError> {
        self.reconcile_source(source, BTreeSet::new())
    }

    fn transact<T>(
        &mut self,
        update: impl FnOnce(&mut AttentionLocalState) -> Result<T, AttentionLocalStateError>,
    ) -> Result<T, AttentionLocalStateError> {
        let mut candidate = self.state.clone();
        let outcome = update(&mut candidate)?;
        if candidate != self.state {
            persist_local_state(&self.path, &candidate)?;
            self.state = candidate;
        }
        Ok(outcome)
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct AttentionLocalDocument {
    schema: u16,
    entries: Vec<AttentionLocalEntry>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct AttentionLocalEntry {
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
    if !path.exists() {
        return Ok(state);
    }
    let metadata = std::fs::symlink_metadata(path)?;
    if !metadata.file_type().is_file() || metadata.len() > MAX_ATTENTION_LOCAL_FILE_BYTES {
        return Err(AttentionLocalStateError::DocumentInvalid);
    }
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take(MAX_ATTENTION_LOCAL_FILE_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_ATTENTION_LOCAL_FILE_BYTES {
        return Err(AttentionLocalStateError::DocumentInvalid);
    }
    let document: AttentionLocalDocument = serde_json::from_slice(&bytes)?;
    if document.schema != ATTENTION_LOCAL_SCHEMA || document.entries.len() > capacity {
        return Err(AttentionLocalStateError::DocumentInvalid);
    }
    for entry in document.entries {
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
    Ok(state)
}

fn persist_local_state(
    path: &Path,
    state: &AttentionLocalState,
) -> Result<(), AttentionLocalStateError> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    let entries = state
        .entries
        .iter()
        .map(|(key, flags)| AttentionLocalEntry {
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
    };
    let bytes = serde_json::to_vec_pretty(&document)?;
    crate::util::atomic_write(path, &bytes)?;
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
    #[error("same attention revision has different content")]
    ConflictingReplay,
    #[error("attention UI identity collides with another raw source/item tuple")]
    UiIdentityCollision,
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
    #[error("attention local-state I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("attention local-state serialization failed: {0}")]
    Serialization(#[from] serde_json::Error),
}

#[cfg(test)]
mod tests {
    use super::*;
    use automonique_protocol::platform::{
        ResourceAuthority, ResourceCoordinate, ResourceId, ResourceKind,
    };
    use automonique_protocol::platform_v2::{
        AttemptWorkspaceId, CheckoutId, V1SessionRef, WorkContextAttributes, WorkContextLabel,
        WorkContextLifecycle, WorkContextRelation, WorkSessionId,
    };
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

    fn provider_item(id: &str, revision: u64) -> AttentionItem {
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
                    ResourceId::new("platform-session-1").unwrap(),
                ))
                .unwrap(),
            ),
        )
        .unwrap()
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
            AttentionItemKey::new(
                source(AttentionSourceKind::Orchestration, source_id),
                AttentionItemId::new(item_id.to_owned()).unwrap(),
            ),
            Revision::new(revision).unwrap(),
        )
    }

    fn temp_path(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "shelldeck-platform-attention-{}-{}",
            std::process::id(),
            TEMP_NONCE.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join(name)
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
            board.replace_source(discontinuous),
            Err(AttentionError::InvalidSuccessor)
        ));
        assert_eq!(board, before, "rejected replacement changes nothing");

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
            Revision::FIRST
        );

        let empty = snapshot(orchestration.clone(), 2, Some(1), Vec::new());
        assert_eq!(
            board.replace_source(empty).unwrap(),
            AttentionApplyOutcome::Replaced
        );
        assert_eq!(board.visible_items().count(), 0);

        let new_incarnation = snapshot(
            orchestration.clone(),
            3,
            Some(2),
            vec![item("new-incarnation", 3, true)],
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
                    4,
                    Some(3),
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
            Revision::new(3).unwrap()
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
            .reconcile_source(next_revision.item().source(), current)
            .unwrap();
        assert!(!restarted.state().is_read(&first));
        assert!(restarted.state().is_notified(&next_revision));
        let before_wrong_source = std::fs::read(&path).unwrap();
        assert!(matches!(
            restarted.reconcile_source(
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
        std::fs::write(
            &invalid_path,
            br#"{"schema":1,"entries":[],"invented":true}"#,
        )
        .unwrap();
        assert!(matches!(
            AttentionLocalStateStore::open(invalid_path, 2),
            Err(AttentionLocalStateError::Serialization(_))
        ));

        std::fs::remove_dir_all(path.parent().unwrap()).ok();
        std::fs::remove_dir_all(failing_path.parent().unwrap()).ok();
    }
}
