//! Catalog-authorized, keyed workspace navigation and creation reducers.
//!
//! This module retains presentation bindings only. Live GPUI entities remain
//! owned by the UI, and provider authority remains owned by Platform v2.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use uuid::Uuid;

use crate::config::workspace_catalog::{
    CatalogCheckoutId, CatalogWorkspaceId, CheckoutHost, ProjectCatalog, UserWorkspaceLifecycle,
    WorkspaceCatalogError, WorkspaceRelativePath,
};

const MAX_BINDING_ID_BYTES: usize = 256;

macro_rules! uuid_id {
    ($name:ident) => {
        #[derive(
            Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize,
        )]
        #[serde(transparent)]
        pub struct $name(Uuid);
        impl $name {
            #[must_use]
            pub fn new() -> Self {
                Self(Uuid::new_v4())
            }
            #[must_use]
            pub const fn from_uuid(value: Uuid) -> Self {
                Self(value)
            }
            #[must_use]
            pub const fn as_uuid(self) -> Uuid {
                self.0
            }
        }
        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }
        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(formatter)
            }
        }
    };
}

uuid_id!(PaneId);
uuid_id!(WorkspaceTabId);
uuid_id!(TerminalBindingId);
uuid_id!(CreationOperationId);

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TerminalAuthority {
    Local {
        checkout_id: CatalogCheckoutId,
    },
    Ssh {
        checkout_id: CatalogCheckoutId,
        connection_id: Uuid,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TerminalBinding {
    pub id: TerminalBindingId,
    pub authority: TerminalAuthority,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ProviderSessionBinding {
    pub platform_user_workspace_id: String,
    pub session_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct TerminalViewport {
    pub scrollback_offset_lines: usize,
    pub follow_output: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TerminalSurface {
    pub binding: TerminalBinding,
    pub viewport: TerminalViewport,
    #[serde(default)]
    pub draft: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WorkspaceTabContent {
    Terminal(TerminalSurface),
    Editor {
        checkout_id: CatalogCheckoutId,
        relative_path: WorkspaceRelativePath,
        #[serde(default)]
        draft: String,
        cursor_line: usize,
        cursor_column: usize,
    },
    Files {
        checkout_id: CatalogCheckoutId,
        relative_root: WorkspaceRelativePath,
    },
    Browser {
        location: String,
    },
    ProviderSession(ProviderSessionBinding),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct WorkspaceTab {
    pub id: WorkspaceTabId,
    pub title: String,
    pub content: WorkspaceTabContent,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PaneLeaf {
    pub id: PaneId,
    #[serde(default)]
    pub tabs: Vec<WorkspaceTab>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_tab: Option<WorkspaceTabId>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SplitAxis {
    Horizontal,
    Vertical,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PaneNode {
    Leaf(PaneLeaf),
    Split {
        axis: SplitAxis,
        ratio_basis_points: u16,
        first: Box<PaneNode>,
        second: Box<PaneNode>,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct WorkspaceFocus {
    pub pane_id: PaneId,
    pub tab_id: WorkspaceTabId,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct WorkspaceSurfaceState {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root: Option<PaneNode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub focus: Option<WorkspaceFocus>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SurfaceValidationError {
    InvalidSplitRatio(u16),
    DuplicatePane(PaneId),
    DuplicateTab(WorkspaceTabId),
    DuplicateTerminalBinding(TerminalBindingId),
    MissingActiveTab(PaneId),
    UnknownActiveTab { pane: PaneId, tab: WorkspaceTabId },
    FocusWithoutSurface,
    UnknownFocusedPane(PaneId),
    UnknownFocusedTab { pane: PaneId, tab: WorkspaceTabId },
    UnknownWorkspace,
    CheckoutAuthorityMismatch(CatalogCheckoutId),
    HostAuthorityMismatch,
    PlatformMappingNotExact,
    PlatformWorkspaceMismatch,
    InvalidProviderBinding,
    InvalidEditorPath,
}
impl fmt::Display for SurfaceValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidSplitRatio(v) => write!(f, "invalid pane split ratio {v}"),
            Self::DuplicatePane(id) => write!(f, "duplicate pane {id}"),
            Self::DuplicateTab(id) => write!(f, "duplicate tab {id}"),
            Self::DuplicateTerminalBinding(id) => write!(f, "duplicate terminal binding {id}"),
            Self::MissingActiveTab(id) => write!(f, "pane {id} has no active tab"),
            Self::UnknownActiveTab { pane, tab } => {
                write!(f, "pane {pane} has unknown active tab {tab}")
            }
            Self::FocusWithoutSurface => f.write_str("focus exists without a pane surface"),
            Self::UnknownFocusedPane(id) => write!(f, "unknown focused pane {id}"),
            Self::UnknownFocusedTab { pane, tab } => {
                write!(f, "pane {pane} has no focused tab {tab}")
            }
            Self::UnknownWorkspace => f.write_str("surface workspace is absent from the catalog"),
            Self::CheckoutAuthorityMismatch(id) => {
                write!(f, "tab checkout {id} is outside the workspace")
            }
            Self::HostAuthorityMismatch => {
                f.write_str("terminal host authority differs from the catalog checkout")
            }
            Self::PlatformMappingNotExact => {
                f.write_str("provider session requires an exact Platform v2 mapping")
            }
            Self::PlatformWorkspaceMismatch => {
                f.write_str("provider session belongs to a different Platform v2 workspace")
            }
            Self::InvalidProviderBinding => f.write_str("provider session binding is invalid"),
            Self::InvalidEditorPath => f.write_str("editor path must name a file below checkout"),
        }
    }
}
impl std::error::Error for SurfaceValidationError {}

impl WorkspaceSurfaceState {
    pub fn validate(&self) -> Result<(), SurfaceValidationError> {
        let Some(root) = self.root.as_ref() else {
            return if self.focus.is_none() {
                Ok(())
            } else {
                Err(SurfaceValidationError::FocusWithoutSurface)
            };
        };
        let mut panes = BTreeMap::<PaneId, BTreeSet<WorkspaceTabId>>::new();
        let mut tabs = BTreeSet::new();
        let mut terminals = BTreeSet::new();
        validate_node(root, &mut panes, &mut tabs, &mut terminals)?;
        if let Some(focus) = self.focus {
            let pane_tabs = panes
                .get(&focus.pane_id)
                .ok_or(SurfaceValidationError::UnknownFocusedPane(focus.pane_id))?;
            if !pane_tabs.contains(&focus.tab_id) {
                return Err(SurfaceValidationError::UnknownFocusedTab {
                    pane: focus.pane_id,
                    tab: focus.tab_id,
                });
            }
        }
        Ok(())
    }

    pub fn validate_for(
        &self,
        catalog: &ProjectCatalog,
        workspace_id: CatalogWorkspaceId,
    ) -> Result<(), SurfaceValidationError> {
        self.validate()?;
        let workspace = catalog
            .workspace(workspace_id)
            .map_err(|_| SurfaceValidationError::UnknownWorkspace)?;
        let checkout = catalog
            .checkout_in_project(workspace.project_id(), workspace.checkout_id())
            .map_err(|_| {
                SurfaceValidationError::CheckoutAuthorityMismatch(workspace.checkout_id())
            })?;
        for tab in self.tabs() {
            match &tab.content {
                WorkspaceTabContent::Terminal(terminal) => {
                    match (&terminal.binding.authority, checkout.host()) {
                        (TerminalAuthority::Local { checkout_id }, CheckoutHost::Local { .. })
                            if *checkout_id == workspace.checkout_id() => {}
                        (
                            TerminalAuthority::Ssh {
                                checkout_id,
                                connection_id,
                            },
                            CheckoutHost::Ssh {
                                connection_id: expected,
                                ..
                            },
                        ) if *checkout_id == workspace.checkout_id()
                            && connection_id == expected => {}
                        (TerminalAuthority::Local { checkout_id }, _)
                        | (TerminalAuthority::Ssh { checkout_id, .. }, _)
                            if *checkout_id != workspace.checkout_id() =>
                        {
                            return Err(SurfaceValidationError::CheckoutAuthorityMismatch(
                                *checkout_id,
                            ))
                        }
                        _ => return Err(SurfaceValidationError::HostAuthorityMismatch),
                    }
                }
                WorkspaceTabContent::Editor {
                    checkout_id,
                    relative_path,
                    ..
                } => {
                    if *checkout_id != workspace.checkout_id() {
                        return Err(SurfaceValidationError::CheckoutAuthorityMismatch(
                            *checkout_id,
                        ));
                    }
                    if relative_path.as_str().is_empty() {
                        return Err(SurfaceValidationError::InvalidEditorPath);
                    }
                }
                WorkspaceTabContent::Files { checkout_id, .. }
                    if *checkout_id != workspace.checkout_id() =>
                {
                    return Err(SurfaceValidationError::CheckoutAuthorityMismatch(
                        *checkout_id,
                    ));
                }
                WorkspaceTabContent::ProviderSession(binding) => {
                    if binding.platform_user_workspace_id.trim().is_empty()
                        || binding.platform_user_workspace_id.len() > MAX_BINDING_ID_BYTES
                        || binding.session_id.trim().is_empty()
                        || binding.session_id.len() > MAX_BINDING_ID_BYTES
                    {
                        return Err(SurfaceValidationError::InvalidProviderBinding);
                    }
                    let mapping = workspace
                        .platform_mapping()
                        .filter(|mapping| mapping.is_exact())
                        .ok_or(SurfaceValidationError::PlatformMappingNotExact)?;
                    if mapping.user_workspace.id != binding.platform_user_workspace_id {
                        return Err(SurfaceValidationError::PlatformWorkspaceMismatch);
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn tabs(&self) -> Vec<&WorkspaceTab> {
        fn collect<'a>(node: &'a PaneNode, tabs: &mut Vec<&'a WorkspaceTab>) {
            match node {
                PaneNode::Leaf(leaf) => tabs.extend(&leaf.tabs),
                PaneNode::Split { first, second, .. } => {
                    collect(first, tabs);
                    collect(second, tabs);
                }
            }
        }
        let mut tabs = Vec::new();
        if let Some(root) = self.root.as_ref() {
            collect(root, &mut tabs);
        }
        tabs
    }

    fn terminal_ids(&self) -> BTreeSet<TerminalBindingId> {
        self.tabs()
            .into_iter()
            .filter_map(|tab| match &tab.content {
                WorkspaceTabContent::Terminal(terminal) => Some(terminal.binding.id),
                _ => None,
            })
            .collect()
    }
}

fn validate_node(
    node: &PaneNode,
    panes: &mut BTreeMap<PaneId, BTreeSet<WorkspaceTabId>>,
    tabs: &mut BTreeSet<WorkspaceTabId>,
    terminals: &mut BTreeSet<TerminalBindingId>,
) -> Result<(), SurfaceValidationError> {
    match node {
        PaneNode::Split {
            ratio_basis_points,
            first,
            second,
            ..
        } => {
            if !(1..10_000).contains(ratio_basis_points) {
                return Err(SurfaceValidationError::InvalidSplitRatio(
                    *ratio_basis_points,
                ));
            }
            validate_node(first, panes, tabs, terminals)?;
            validate_node(second, panes, tabs, terminals)
        }
        PaneNode::Leaf(leaf) => {
            if panes.contains_key(&leaf.id) {
                return Err(SurfaceValidationError::DuplicatePane(leaf.id));
            }
            let mut pane_tabs = BTreeSet::new();
            for tab in &leaf.tabs {
                if !tabs.insert(tab.id) {
                    return Err(SurfaceValidationError::DuplicateTab(tab.id));
                }
                pane_tabs.insert(tab.id);
                if let WorkspaceTabContent::Terminal(terminal) = &tab.content {
                    if !terminals.insert(terminal.binding.id) {
                        return Err(SurfaceValidationError::DuplicateTerminalBinding(
                            terminal.binding.id,
                        ));
                    }
                }
            }
            match (leaf.tabs.is_empty(), leaf.active_tab) {
                (true, None) => {}
                (false, None) => return Err(SurfaceValidationError::MissingActiveTab(leaf.id)),
                (_, Some(active)) if !pane_tabs.contains(&active) => {
                    return Err(SurfaceValidationError::UnknownActiveTab {
                        pane: leaf.id,
                        tab: active,
                    })
                }
                _ => {}
            }
            panes.insert(leaf.id, pane_tabs);
            Ok(())
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct GitDirtyState {
    pub staged: usize,
    pub modified: usize,
    pub untracked: usize,
    pub conflicted: usize,
}
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceAgentState {
    #[default]
    Idle,
    Running,
    WaitingForInput,
    Failed,
    Completed,
}
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceFreshness {
    #[default]
    Fresh,
    Aging,
    Stale,
    Offline,
}

/// Complete presentation aggregate produced by one authoritative workspace-card
/// DTO owner. Individual git/agent/attention pollers must reconcile upstream;
/// they must not publish partial instances with unrelated revision domains.
/// `source_revision` and observation time fence late aggregate projections.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct WorkspaceCardAggregate {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    pub dirty: GitDirtyState,
    pub agent: WorkspaceAgentState,
    pub unread: usize,
    pub attention: usize,
    pub freshness: WorkspaceFreshness,
    pub source_revision: u64,
    pub observed_at_millis: u64,
}

/// Compatibility name for the retained state slot. The concrete DTO is an
/// indivisible [`WorkspaceCardAggregate`], not a bag of independently fenced
/// partial poll results.
pub type WorkspaceCardState = WorkspaceCardAggregate;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RetainedWorkspaceState {
    pub surface: WorkspaceSurfaceState,
    pub card: WorkspaceCardState,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct WorkspaceNavigationState {
    workspaces: BTreeMap<CatalogWorkspaceId, RetainedWorkspaceState>,
    active: Option<CatalogWorkspaceId>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WorkspaceNavigationAction {
    Retain {
        id: CatalogWorkspaceId,
        surface: WorkspaceSurfaceState,
        card: WorkspaceCardState,
    },
    SwitchTo(CatalogWorkspaceId),
    UpdateSurface {
        id: CatalogWorkspaceId,
        surface: WorkspaceSurfaceState,
    },
    UpdateCard {
        id: CatalogWorkspaceId,
        card: WorkspaceCardState,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum NavigationError {
    DuplicateWorkspace(CatalogWorkspaceId),
    UnknownWorkspace(CatalogWorkspaceId),
    ArchivedWorkspace(CatalogWorkspaceId),
    InvalidSurface(SurfaceValidationError),
    TerminalOwnedByWorkspace {
        terminal: TerminalBindingId,
        owner: CatalogWorkspaceId,
    },
    StaleCard,
    ConflictingCardFence,
    Catalog(WorkspaceCatalogError),
}
impl fmt::Display for NavigationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateWorkspace(id) => write!(f, "workspace {id} is already retained"),
            Self::UnknownWorkspace(id) => write!(f, "workspace {id} is not retained"),
            Self::ArchivedWorkspace(id) => write!(f, "workspace {id} is archived"),
            Self::InvalidSurface(error) => error.fmt(f),
            Self::TerminalOwnedByWorkspace { terminal, owner } => write!(
                f,
                "terminal {terminal} is already owned by workspace {owner}"
            ),
            Self::StaleCard => f.write_str("workspace card observation is stale"),
            Self::ConflictingCardFence => {
                f.write_str("workspace card changed without advancing its aggregate fence")
            }
            Self::Catalog(error) => error.fmt(f),
        }
    }
}
impl std::error::Error for NavigationError {}

impl WorkspaceNavigationState {
    #[must_use]
    pub const fn active(&self) -> Option<CatalogWorkspaceId> {
        self.active
    }
    #[must_use]
    pub fn workspace(&self, id: CatalogWorkspaceId) -> Option<&RetainedWorkspaceState> {
        self.workspaces.get(&id)
    }
    pub fn workspaces(
        &self,
    ) -> impl ExactSizeIterator<Item = (CatalogWorkspaceId, &RetainedWorkspaceState)> {
        self.workspaces.iter().map(|(id, value)| (*id, value))
    }

    /// Reconciles visibility after the catalog owner archives or removes a
    /// workspace. Snapshots stay retained; only active visibility follows the
    /// authoritative catalog lifecycle.
    pub fn reconcile_catalog(&mut self, catalog: &ProjectCatalog) {
        if self.active.is_some_and(|id| {
            catalog.workspace(id).map_or(true, |workspace| {
                workspace.lifecycle() == UserWorkspaceLifecycle::Archived
            })
        }) {
            self.active = None;
        }
    }

    pub fn reduce(
        &mut self,
        catalog: &ProjectCatalog,
        action: WorkspaceNavigationAction,
    ) -> Result<(), NavigationError> {
        match action {
            WorkspaceNavigationAction::Retain { id, surface, card } => {
                catalog.workspace(id).map_err(NavigationError::Catalog)?;
                surface
                    .validate_for(catalog, id)
                    .map_err(NavigationError::InvalidSurface)?;
                if self.workspaces.contains_key(&id) {
                    return Err(NavigationError::DuplicateWorkspace(id));
                }
                self.ensure_terminal_ownership(id, &surface)?;
                self.workspaces
                    .insert(id, RetainedWorkspaceState { surface, card });
                if self.active.is_none()
                    && catalog
                        .workspace(id)
                        .map_err(NavigationError::Catalog)?
                        .lifecycle()
                        == UserWorkspaceLifecycle::Active
                {
                    self.active = Some(id);
                }
            }
            WorkspaceNavigationAction::SwitchTo(id) => {
                if !self.workspaces.contains_key(&id) {
                    return Err(NavigationError::UnknownWorkspace(id));
                }
                if catalog
                    .workspace(id)
                    .map_err(NavigationError::Catalog)?
                    .lifecycle()
                    == UserWorkspaceLifecycle::Archived
                {
                    return Err(NavigationError::ArchivedWorkspace(id));
                }
                self.active = Some(id);
            }
            WorkspaceNavigationAction::UpdateSurface { id, surface } => {
                if !self.workspaces.contains_key(&id) {
                    return Err(NavigationError::UnknownWorkspace(id));
                }
                surface
                    .validate_for(catalog, id)
                    .map_err(NavigationError::InvalidSurface)?;
                self.ensure_terminal_ownership(id, &surface)?;
                self.workspaces.get_mut(&id).expect("checked above").surface = surface;
            }
            WorkspaceNavigationAction::UpdateCard { id, card } => {
                let workspace = self
                    .workspaces
                    .get_mut(&id)
                    .ok_or(NavigationError::UnknownWorkspace(id))?;
                let incoming_fence = (card.source_revision, card.observed_at_millis);
                let current_fence = (
                    workspace.card.source_revision,
                    workspace.card.observed_at_millis,
                );
                if incoming_fence < current_fence {
                    return Err(NavigationError::StaleCard);
                }
                if incoming_fence == current_fence {
                    return if card == workspace.card {
                        Ok(())
                    } else {
                        Err(NavigationError::ConflictingCardFence)
                    };
                }
                workspace.card = card;
            }
        }
        Ok(())
    }

    fn ensure_terminal_ownership(
        &self,
        target: CatalogWorkspaceId,
        surface: &WorkspaceSurfaceState,
    ) -> Result<(), NavigationError> {
        let incoming = surface.terminal_ids();
        for (owner, workspace) in &self.workspaces {
            if *owner == target {
                continue;
            }
            if let Some(terminal) = workspace
                .surface
                .terminal_ids()
                .intersection(&incoming)
                .next()
            {
                return Err(NavigationError::TerminalOwnedByWorkspace {
                    terminal: *terminal,
                    owner: *owner,
                });
            }
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum WorkspaceCreatePhase {
    Queued,
    ResolvingHost,
    PreparingCheckout,
    CreatingWorkspace,
    BindingRuntime,
}
impl WorkspaceCreatePhase {
    fn next(self) -> Option<Self> {
        match self {
            Self::Queued => Some(Self::ResolvingHost),
            Self::ResolvingHost => Some(Self::PreparingCheckout),
            Self::PreparingCheckout => Some(Self::CreatingWorkspace),
            Self::CreatingWorkspace => Some(Self::BindingRuntime),
            Self::BindingRuntime => None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkspaceCreateProgress {
    pub phase: WorkspaceCreatePhase,
    pub completed_steps: u32,
    pub total_steps: u32,
    pub detail: String,
}
impl Default for WorkspaceCreateProgress {
    fn default() -> Self {
        Self {
            phase: WorkspaceCreatePhase::Queued,
            completed_steps: 0,
            total_steps: 1,
            detail: String::new(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WorkspaceCreateConflict {
    CheckoutAlreadyExists { root: String },
    WorktreeLocked { root: String },
    BranchAlreadyCheckedOut { branch: String },
    HostUnavailable,
    CatalogRevisionChanged { expected: u64, actual: u64 },
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorkspaceCreateFailureKind {
    Authorization,
    Filesystem,
    Transport,
    RuntimeUnavailable,
    Unknown,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkspaceCreateFailure {
    pub kind: WorkspaceCreateFailureKind,
    pub message: String,
    pub retryable: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BackgroundWorkspaceCreateState {
    Running {
        operation: CreationOperationId,
        catalog_revision: u64,
        progress: WorkspaceCreateProgress,
    },
    Cancelling {
        operation: CreationOperationId,
        catalog_revision: u64,
        progress: WorkspaceCreateProgress,
    },
    Cancelled {
        operation: CreationOperationId,
        catalog_revision: u64,
    },
    Conflict {
        operation: CreationOperationId,
        catalog_revision: u64,
        conflict: WorkspaceCreateConflict,
    },
    Failed {
        operation: CreationOperationId,
        catalog_revision: u64,
        failure: WorkspaceCreateFailure,
    },
    Completed {
        operation: CreationOperationId,
        catalog_revision: u64,
    },
}
impl BackgroundWorkspaceCreateState {
    fn operation(&self) -> CreationOperationId {
        match self {
            Self::Running { operation, .. }
            | Self::Cancelling { operation, .. }
            | Self::Cancelled { operation, .. }
            | Self::Conflict { operation, .. }
            | Self::Failed { operation, .. }
            | Self::Completed { operation, .. } => *operation,
        }
    }
    fn catalog_revision(&self) -> u64 {
        match self {
            Self::Running {
                catalog_revision, ..
            }
            | Self::Cancelling {
                catalog_revision, ..
            }
            | Self::Cancelled {
                catalog_revision, ..
            }
            | Self::Conflict {
                catalog_revision, ..
            }
            | Self::Failed {
                catalog_revision, ..
            }
            | Self::Completed {
                catalog_revision, ..
            } => *catalog_revision,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WorkspaceCreateEvent {
    Start {
        workspace: CatalogWorkspaceId,
        operation: CreationOperationId,
    },
    Progress {
        workspace: CatalogWorkspaceId,
        operation: CreationOperationId,
        progress: WorkspaceCreateProgress,
    },
    RequestCancel {
        workspace: CatalogWorkspaceId,
        operation: CreationOperationId,
    },
    Cancelled {
        workspace: CatalogWorkspaceId,
        operation: CreationOperationId,
    },
    Conflict {
        workspace: CatalogWorkspaceId,
        operation: CreationOperationId,
        conflict: WorkspaceCreateConflict,
    },
    Failed {
        workspace: CatalogWorkspaceId,
        operation: CreationOperationId,
        failure: WorkspaceCreateFailure,
    },
    Completed {
        workspace: CatalogWorkspaceId,
        operation: CreationOperationId,
    },
    Retry {
        workspace: CatalogWorkspaceId,
        prior_operation: CreationOperationId,
        operation: CreationOperationId,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WorkspaceCreateError {
    ExistingJob(CatalogWorkspaceId),
    ReusedOperation(CreationOperationId),
    UnknownWorkspaceJob(CatalogWorkspaceId),
    StaleOperation {
        expected: CreationOperationId,
        received: CreationOperationId,
    },
    CatalogRevisionChanged {
        expected: u64,
        actual: u64,
    },
    InvalidTransition,
    InvalidProgress,
    ProgressRegressed,
}
impl fmt::Display for WorkspaceCreateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ExistingJob(id) => write!(f, "workspace {id} already has a create job"),
            Self::ReusedOperation(id) => write!(f, "create operation {id} was already used"),
            Self::UnknownWorkspaceJob(id) => write!(f, "workspace {id} has no create job"),
            Self::StaleOperation { expected, received } => write!(
                f,
                "stale create operation {received}; current operation is {expected}"
            ),
            Self::CatalogRevisionChanged { expected, actual } => {
                write!(f, "catalog revision changed from {expected} to {actual}")
            }
            Self::InvalidTransition => f.write_str("invalid workspace-create transition"),
            Self::InvalidProgress => f.write_str("invalid workspace-create progress"),
            Self::ProgressRegressed => {
                f.write_str("workspace-create progress cannot move backwards")
            }
        }
    }
}
impl std::error::Error for WorkspaceCreateError {}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct WorkspaceCreationReducer {
    jobs: BTreeMap<CatalogWorkspaceId, BackgroundWorkspaceCreateState>,
    seen_operations: BTreeSet<CreationOperationId>,
}

impl WorkspaceCreationReducer {
    #[must_use]
    pub fn state(&self, workspace: CatalogWorkspaceId) -> Option<&BackgroundWorkspaceCreateState> {
        self.jobs.get(&workspace)
    }

    pub fn reduce(
        &mut self,
        catalog_revision: u64,
        event: WorkspaceCreateEvent,
    ) -> Result<(), WorkspaceCreateError> {
        match event {
            WorkspaceCreateEvent::Start {
                workspace,
                operation,
            } => {
                if self.jobs.contains_key(&workspace) {
                    return Err(WorkspaceCreateError::ExistingJob(workspace));
                }
                if !self.seen_operations.insert(operation) {
                    return Err(WorkspaceCreateError::ReusedOperation(operation));
                }
                self.jobs.insert(
                    workspace,
                    BackgroundWorkspaceCreateState::Running {
                        operation,
                        catalog_revision,
                        progress: WorkspaceCreateProgress::default(),
                    },
                );
            }
            WorkspaceCreateEvent::Progress {
                workspace,
                operation,
                progress,
            } => {
                validate_progress(&progress)?;
                let state =
                    self.matching_state_mut(workspace, operation, catalog_revision, false)?;
                let BackgroundWorkspaceCreateState::Running {
                    progress: previous, ..
                } = state
                else {
                    return Err(WorkspaceCreateError::InvalidTransition);
                };
                if progress.phase == previous.phase {
                    if progress.total_steps != previous.total_steps
                        || progress.completed_steps < previous.completed_steps
                    {
                        return Err(WorkspaceCreateError::ProgressRegressed);
                    }
                } else if previous.phase.next() != Some(progress.phase)
                    || previous.completed_steps != previous.total_steps
                {
                    return Err(WorkspaceCreateError::ProgressRegressed);
                }
                *previous = progress;
            }
            WorkspaceCreateEvent::RequestCancel {
                workspace,
                operation,
            } => {
                let state =
                    self.matching_state_mut(workspace, operation, catalog_revision, true)?;
                let BackgroundWorkspaceCreateState::Running {
                    catalog_revision,
                    progress,
                    ..
                } = state
                else {
                    return Err(WorkspaceCreateError::InvalidTransition);
                };
                let revision = *catalog_revision;
                let progress = progress.clone();
                *state = BackgroundWorkspaceCreateState::Cancelling {
                    operation,
                    catalog_revision: revision,
                    progress,
                };
            }
            WorkspaceCreateEvent::Cancelled {
                workspace,
                operation,
            } => {
                let state =
                    self.matching_state_mut(workspace, operation, catalog_revision, true)?;
                let BackgroundWorkspaceCreateState::Cancelling {
                    catalog_revision, ..
                } = state
                else {
                    return Err(WorkspaceCreateError::InvalidTransition);
                };
                let revision = *catalog_revision;
                *state = BackgroundWorkspaceCreateState::Cancelled {
                    operation,
                    catalog_revision: revision,
                };
            }
            WorkspaceCreateEvent::Conflict {
                workspace,
                operation,
                conflict,
            } => {
                let stored_revision = self
                    .jobs
                    .get(&workspace)
                    .map(BackgroundWorkspaceCreateState::catalog_revision)
                    .ok_or(WorkspaceCreateError::UnknownWorkspaceJob(workspace))?;
                let allow_revision_change = matches!(conflict, WorkspaceCreateConflict::CatalogRevisionChanged { expected, actual } if expected == stored_revision && actual > expected && actual == catalog_revision);
                let state = self.matching_state_mut(
                    workspace,
                    operation,
                    catalog_revision,
                    allow_revision_change,
                )?;
                if !matches!(
                    state,
                    BackgroundWorkspaceCreateState::Running { .. }
                        | BackgroundWorkspaceCreateState::Cancelling { .. }
                ) {
                    return Err(WorkspaceCreateError::InvalidTransition);
                }
                let revision = state.catalog_revision();
                *state = BackgroundWorkspaceCreateState::Conflict {
                    operation,
                    catalog_revision: revision,
                    conflict,
                };
            }
            WorkspaceCreateEvent::Failed {
                workspace,
                operation,
                failure,
            } => {
                let state =
                    self.matching_state_mut(workspace, operation, catalog_revision, false)?;
                if !matches!(
                    state,
                    BackgroundWorkspaceCreateState::Running { .. }
                        | BackgroundWorkspaceCreateState::Cancelling { .. }
                ) {
                    return Err(WorkspaceCreateError::InvalidTransition);
                }
                *state = BackgroundWorkspaceCreateState::Failed {
                    operation,
                    catalog_revision,
                    failure,
                };
            }
            WorkspaceCreateEvent::Completed {
                workspace,
                operation,
            } => {
                let state =
                    self.matching_state_mut(workspace, operation, catalog_revision, false)?;
                let BackgroundWorkspaceCreateState::Running { progress, .. } = state else {
                    return Err(WorkspaceCreateError::InvalidTransition);
                };
                if progress.phase != WorkspaceCreatePhase::BindingRuntime
                    || progress.completed_steps != progress.total_steps
                {
                    return Err(WorkspaceCreateError::InvalidTransition);
                }
                *state = BackgroundWorkspaceCreateState::Completed {
                    operation,
                    catalog_revision,
                };
            }
            WorkspaceCreateEvent::Retry {
                workspace,
                prior_operation,
                operation,
            } => {
                let allow_revision_change = self.jobs.get(&workspace).is_some_and(|state| {
                    matches!(
                        state,
                        BackgroundWorkspaceCreateState::Conflict {
                            catalog_revision: stored,
                            conflict: WorkspaceCreateConflict::CatalogRevisionChanged {
                                expected,
                                actual,
                            },
                            ..
                        } if expected == stored && *actual == catalog_revision
                    )
                });
                {
                    let state = self.matching_state_mut(
                        workspace,
                        prior_operation,
                        catalog_revision,
                        allow_revision_change,
                    )?;
                    if !matches!(
                        state,
                        BackgroundWorkspaceCreateState::Cancelled { .. }
                            | BackgroundWorkspaceCreateState::Conflict { .. }
                            | BackgroundWorkspaceCreateState::Failed {
                                failure: WorkspaceCreateFailure {
                                    retryable: true,
                                    ..
                                },
                                ..
                            }
                    ) {
                        return Err(WorkspaceCreateError::InvalidTransition);
                    }
                }
                if !self.seen_operations.insert(operation) {
                    return Err(WorkspaceCreateError::ReusedOperation(operation));
                }
                let state = self.jobs.get_mut(&workspace).expect("checked above");
                *state = BackgroundWorkspaceCreateState::Running {
                    operation,
                    catalog_revision,
                    progress: WorkspaceCreateProgress::default(),
                };
            }
        }
        Ok(())
    }

    fn matching_state_mut(
        &mut self,
        workspace: CatalogWorkspaceId,
        operation: CreationOperationId,
        catalog_revision: u64,
        allow_revision_change: bool,
    ) -> Result<&mut BackgroundWorkspaceCreateState, WorkspaceCreateError> {
        let state = self
            .jobs
            .get_mut(&workspace)
            .ok_or(WorkspaceCreateError::UnknownWorkspaceJob(workspace))?;
        let expected_operation = state.operation();
        if expected_operation != operation {
            return Err(WorkspaceCreateError::StaleOperation {
                expected: expected_operation,
                received: operation,
            });
        }
        let expected_revision = state.catalog_revision();
        if expected_revision != catalog_revision && !allow_revision_change {
            return Err(WorkspaceCreateError::CatalogRevisionChanged {
                expected: expected_revision,
                actual: catalog_revision,
            });
        }
        Ok(state)
    }
}

fn validate_progress(progress: &WorkspaceCreateProgress) -> Result<(), WorkspaceCreateError> {
    if progress.total_steps == 0 || progress.completed_steps > progress.total_steps {
        return Err(WorkspaceCreateError::InvalidProgress);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::workspace_catalog::{
        CatalogProjectId, CheckoutHost, PlatformContextRef, PlatformMappingReconciliation,
        PlatformV2Mapping, ProjectCheckout, ProjectRecord, RepositoryIdentity,
        WorkspaceLaunchIntake, WorkspaceLaunchRequest,
    };

    fn uuid(value: u128) -> Uuid {
        Uuid::from_u128(value)
    }
    fn workspace(value: u128) -> CatalogWorkspaceId {
        CatalogWorkspaceId::from_uuid(uuid(value))
    }
    fn checkout(value: u128) -> CatalogCheckoutId {
        CatalogCheckoutId::from_uuid(uuid(value))
    }
    fn catalog() -> ProjectCatalog {
        let mut project = ProjectRecord::new(CatalogProjectId::from_uuid(uuid(1)), "ShellDeck");
        project.add_checkout(ProjectCheckout::new(
            checkout(2),
            "Local",
            CheckoutHost::Local {
                device_label: "Local".into(),
                root: std::env::temp_dir().join("shelldeck"),
            },
            RepositoryIdentity {
                slug: "benfavre/shelldeck".into(),
                canonical_url: None,
            },
        ));
        project.add_checkout(ProjectCheckout::new(
            checkout(3),
            "SSH",
            CheckoutHost::Ssh {
                connection_id: uuid(4),
                root: crate::config::workspace_catalog::RemotePosixPath::new("/srv/shelldeck")
                    .unwrap(),
            },
            RepositoryIdentity {
                slug: "benfavre/shelldeck".into(),
                canonical_url: None,
            },
        ));
        let mut catalog = ProjectCatalog::default();
        catalog.insert_project(project).unwrap();
        for (id, checkout_id) in [(10, checkout(2)), (11, checkout(2)), (12, checkout(3))] {
            catalog
                .create_workspace(WorkspaceLaunchRequest {
                    id: workspace(id),
                    project_id: CatalogProjectId::from_uuid(uuid(1)),
                    checkout_id,
                    name: format!("Workspace {id}"),
                    intake: WorkspaceLaunchIntake::Manual,
                })
                .unwrap();
        }
        catalog
    }
    fn terminal_surface(
        binding: u128,
        checkout_id: CatalogCheckoutId,
        ssh: Option<Uuid>,
    ) -> WorkspaceSurfaceState {
        let pane = PaneId::from_uuid(uuid(binding + 100));
        let tab = WorkspaceTabId::from_uuid(uuid(binding + 200));
        let authority = match ssh {
            Some(connection_id) => TerminalAuthority::Ssh {
                checkout_id,
                connection_id,
            },
            None => TerminalAuthority::Local { checkout_id },
        };
        WorkspaceSurfaceState {
            root: Some(PaneNode::Leaf(PaneLeaf {
                id: pane,
                tabs: vec![WorkspaceTab {
                    id: tab,
                    title: "Terminal".into(),
                    content: WorkspaceTabContent::Terminal(TerminalSurface {
                        binding: TerminalBinding {
                            id: TerminalBindingId::from_uuid(uuid(binding)),
                            authority,
                        },
                        viewport: TerminalViewport::default(),
                        draft: "draft".into(),
                    }),
                }],
                active_tab: Some(tab),
            })),
            focus: Some(WorkspaceFocus {
                pane_id: pane,
                tab_id: tab,
            }),
        }
    }

    // SDTEST-1731
    #[test]
    fn navigation_rejects_cross_workspace_terminal_reuse_and_stale_card_observations() {
        let mut catalog = catalog();
        let mut navigation = WorkspaceNavigationState::default();
        let first = terminal_surface(20, checkout(2), None);
        navigation
            .reduce(
                &catalog,
                WorkspaceNavigationAction::Retain {
                    id: workspace(10),
                    surface: first.clone(),
                    card: WorkspaceCardState {
                        source_revision: 5,
                        observed_at_millis: 50,
                        ..Default::default()
                    },
                },
            )
            .unwrap();
        let error = navigation
            .reduce(
                &catalog,
                WorkspaceNavigationAction::Retain {
                    id: workspace(11),
                    surface: first,
                    card: WorkspaceCardState::default(),
                },
            )
            .unwrap_err();
        assert!(
            matches!(error, NavigationError::TerminalOwnedByWorkspace { owner, .. } if owner == workspace(10))
        );
        assert_eq!(
            navigation.reduce(
                &catalog,
                WorkspaceNavigationAction::UpdateCard {
                    id: workspace(10),
                    card: WorkspaceCardState {
                        source_revision: 4,
                        observed_at_millis: 100,
                        ..Default::default()
                    }
                }
            ),
            Err(NavigationError::StaleCard)
        );
        let same_card = navigation.workspace(workspace(10)).unwrap().card.clone();
        navigation
            .reduce(
                &catalog,
                WorkspaceNavigationAction::UpdateCard {
                    id: workspace(10),
                    card: same_card.clone(),
                },
            )
            .expect("identical aggregate replay is idempotent");
        let mut conflicting_card = same_card;
        conflicting_card.unread = 1;
        assert_eq!(
            navigation.reduce(
                &catalog,
                WorkspaceNavigationAction::UpdateCard {
                    id: workspace(10),
                    card: conflicting_card,
                }
            ),
            Err(NavigationError::ConflictingCardFence)
        );
        let second = terminal_surface(21, checkout(2), None);
        navigation
            .reduce(
                &catalog,
                WorkspaceNavigationAction::Retain {
                    id: workspace(11),
                    surface: second,
                    card: WorkspaceCardState::default(),
                },
            )
            .unwrap();
        navigation
            .reduce(&catalog, WorkspaceNavigationAction::SwitchTo(workspace(11)))
            .unwrap();
        navigation
            .reduce(&catalog, WorkspaceNavigationAction::SwitchTo(workspace(10)))
            .unwrap();
        assert_eq!(navigation.active(), Some(workspace(10)));
        assert_eq!(
            navigation
                .workspace(workspace(10))
                .expect("retained first")
                .surface,
            terminal_surface(20, checkout(2), None)
        );
        catalog.archive_workspace(workspace(10)).unwrap();
        navigation.reconcile_catalog(&catalog);
        assert_eq!(navigation.active(), None);
    }

    // SDTEST-1732
    #[test]
    fn catalog_context_rejects_checkout_and_ssh_connection_authority_mismatches() {
        let catalog = catalog();
        assert!(matches!(
            terminal_surface(30, checkout(3), None).validate_for(&catalog, workspace(10)),
            Err(SurfaceValidationError::CheckoutAuthorityMismatch(_))
        ));
        assert_eq!(
            terminal_surface(31, checkout(3), Some(uuid(99))).validate_for(&catalog, workspace(12)),
            Err(SurfaceValidationError::HostAuthorityMismatch)
        );
        terminal_surface(32, checkout(3), Some(uuid(4)))
            .validate_for(&catalog, workspace(12))
            .expect("exact SSH authority");
    }

    // SDTEST-1733
    #[test]
    fn creation_requires_strict_progress_retry_and_catalog_revision_fences() {
        let id = workspace(10);
        let operation = CreationOperationId::from_uuid(uuid(40));
        let retry = CreationOperationId::from_uuid(uuid(41));
        let mut reducer = WorkspaceCreationReducer::default();
        reducer
            .reduce(
                7,
                WorkspaceCreateEvent::Start {
                    workspace: id,
                    operation,
                },
            )
            .unwrap();
        assert_eq!(
            reducer.reduce(
                7,
                WorkspaceCreateEvent::Completed {
                    workspace: id,
                    operation
                }
            ),
            Err(WorkspaceCreateError::InvalidTransition)
        );
        reducer
            .reduce(
                7,
                WorkspaceCreateEvent::Progress {
                    workspace: id,
                    operation,
                    progress: WorkspaceCreateProgress {
                        phase: WorkspaceCreatePhase::Queued,
                        completed_steps: 1,
                        total_steps: 1,
                        detail: "Queued".into(),
                    },
                },
            )
            .unwrap();
        assert_eq!(
            reducer.reduce(
                7,
                WorkspaceCreateEvent::Progress {
                    workspace: id,
                    operation,
                    progress: WorkspaceCreateProgress {
                        phase: WorkspaceCreatePhase::Queued,
                        completed_steps: 1,
                        total_steps: 2,
                        detail: "Regressed fraction".into(),
                    },
                }
            ),
            Err(WorkspaceCreateError::ProgressRegressed)
        );
        assert_eq!(
            reducer.reduce(
                8,
                WorkspaceCreateEvent::Progress {
                    workspace: id,
                    operation,
                    progress: WorkspaceCreateProgress {
                        phase: WorkspaceCreatePhase::Queued,
                        completed_steps: 1,
                        total_steps: 1,
                        detail: String::new()
                    }
                }
            ),
            Err(WorkspaceCreateError::CatalogRevisionChanged {
                expected: 7,
                actual: 8
            })
        );
        reducer
            .reduce(
                8,
                WorkspaceCreateEvent::Conflict {
                    workspace: id,
                    operation,
                    conflict: WorkspaceCreateConflict::CatalogRevisionChanged {
                        expected: 7,
                        actual: 8,
                    },
                },
            )
            .unwrap();
        reducer
            .reduce(
                8,
                WorkspaceCreateEvent::Retry {
                    workspace: id,
                    prior_operation: operation,
                    operation: retry,
                },
            )
            .unwrap();
        assert_eq!(
            reducer.reduce(
                8,
                WorkspaceCreateEvent::Start {
                    workspace: id,
                    operation: CreationOperationId::from_uuid(uuid(42))
                }
            ),
            Err(WorkspaceCreateError::ExistingJob(id))
        );

        let non_retryable_workspace = workspace(11);
        let failed_operation = CreationOperationId::from_uuid(uuid(43));
        reducer
            .reduce(
                8,
                WorkspaceCreateEvent::Start {
                    workspace: non_retryable_workspace,
                    operation: failed_operation,
                },
            )
            .unwrap();
        reducer
            .reduce(
                8,
                WorkspaceCreateEvent::Failed {
                    workspace: non_retryable_workspace,
                    operation: failed_operation,
                    failure: WorkspaceCreateFailure {
                        kind: WorkspaceCreateFailureKind::Authorization,
                        message: "Denied".into(),
                        retryable: false,
                    },
                },
            )
            .unwrap();
        assert_eq!(
            reducer.reduce(
                8,
                WorkspaceCreateEvent::Retry {
                    workspace: non_retryable_workspace,
                    prior_operation: failed_operation,
                    operation: CreationOperationId::from_uuid(uuid(44)),
                }
            ),
            Err(WorkspaceCreateError::InvalidTransition)
        );

        let retryable_workspace = workspace(12);
        let retryable_operation = CreationOperationId::from_uuid(uuid(45));
        reducer
            .reduce(
                8,
                WorkspaceCreateEvent::Start {
                    workspace: retryable_workspace,
                    operation: retryable_operation,
                },
            )
            .unwrap();
        reducer
            .reduce(
                8,
                WorkspaceCreateEvent::Failed {
                    workspace: retryable_workspace,
                    operation: retryable_operation,
                    failure: WorkspaceCreateFailure {
                        kind: WorkspaceCreateFailureKind::Transport,
                        message: "Retry".into(),
                        retryable: true,
                    },
                },
            )
            .unwrap();
        assert_eq!(
            reducer.reduce(
                9,
                WorkspaceCreateEvent::Retry {
                    workspace: retryable_workspace,
                    prior_operation: retryable_operation,
                    operation: CreationOperationId::from_uuid(uuid(46)),
                }
            ),
            Err(WorkspaceCreateError::CatalogRevisionChanged {
                expected: 8,
                actual: 9,
            })
        );
    }

    // SDTEST-1734
    #[test]
    fn provider_session_requires_exact_matching_platform_workspace_mapping() {
        let mut catalog = catalog();
        let id = workspace(10);
        let pane = PaneId::from_uuid(uuid(60));
        let tab = WorkspaceTabId::from_uuid(uuid(61));
        let surface = |platform_id: &str| WorkspaceSurfaceState {
            root: Some(PaneNode::Leaf(PaneLeaf {
                id: pane,
                tabs: vec![WorkspaceTab {
                    id: tab,
                    title: "Agent".into(),
                    content: WorkspaceTabContent::ProviderSession(ProviderSessionBinding {
                        platform_user_workspace_id: platform_id.into(),
                        session_id: "session".into(),
                        run_id: None,
                    }),
                }],
                active_tab: Some(tab),
            })),
            focus: None,
        };
        assert_eq!(
            surface("platform-workspace").validate_for(&catalog, id),
            Err(SurfaceValidationError::PlatformMappingNotExact)
        );
        catalog
            .set_platform_mapping(
                id,
                None,
                PlatformV2Mapping {
                    reconciliation_revision: 1,
                    project: PlatformContextRef {
                        id: "project".into(),
                        revision: 1,
                    },
                    checkout: PlatformContextRef {
                        id: "checkout".into(),
                        revision: 1,
                    },
                    user_workspace: PlatformContextRef {
                        id: "platform-workspace".into(),
                        revision: 1,
                    },
                    reconciliation: PlatformMappingReconciliation::Exact {
                        reconciled_at_millis: 1,
                    },
                },
            )
            .unwrap();
        assert_eq!(
            surface("other").validate_for(&catalog, id),
            Err(SurfaceValidationError::PlatformWorkspaceMismatch)
        );
        surface("platform-workspace")
            .validate_for(&catalog, id)
            .expect("exact provider mapping");
    }
}
