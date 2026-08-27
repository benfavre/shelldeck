//! Pure, keyed workspace navigation and background-creation state.
//!
//! UI entities keep the terminal processes and grids alive. This reducer keeps
//! their stable bindings in a surface snapshot per user workspace, so switching
//! changes visibility without destroying a hidden terminal or conflating it
//! with Automonique provider-session authority.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::PathBuf;
use uuid::Uuid;

use crate::config::workspace_catalog::{CheckoutId, UserWorkspaceId};

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
        checkout_id: CheckoutId,
    },
    Ssh {
        checkout_id: CheckoutId,
        connection_id: Uuid,
    },
}

/// Stable identity of a live ShellDeck terminal entity.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TerminalBinding {
    pub id: TerminalBindingId,
    pub authority: TerminalAuthority,
}

/// Opaque presentation binding to a provider session.
///
/// This type intentionally cannot be converted into [`TerminalAuthority`]. A
/// platform session never authorizes a local path, SSH connection, or terminal.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ProviderSessionBinding {
    pub authority: String,
    pub session_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct TerminalViewport {
    /// Lines above the live bottom edge. The live grid owns the actual buffer.
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
        checkout_id: CheckoutId,
        relative_path: PathBuf,
        #[serde(default)]
        draft: String,
        cursor_line: usize,
        cursor_column: usize,
    },
    Files {
        checkout_id: CheckoutId,
        relative_root: PathBuf,
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
        /// First-child size in basis points. Must be strictly inside 0..10000.
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
}

impl fmt::Display for SurfaceValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidSplitRatio(ratio) => write!(formatter, "invalid pane split ratio {ratio}"),
            Self::DuplicatePane(id) => write!(formatter, "duplicate pane {id}"),
            Self::DuplicateTab(id) => write!(formatter, "duplicate tab {id}"),
            Self::DuplicateTerminalBinding(id) => {
                write!(formatter, "duplicate terminal binding {id}")
            }
            Self::MissingActiveTab(pane) => write!(formatter, "pane {pane} has no active tab"),
            Self::UnknownActiveTab { pane, tab } => {
                write!(formatter, "pane {pane} has unknown active tab {tab}")
            }
            Self::FocusWithoutSurface => formatter.write_str("focus exists without a pane surface"),
            Self::UnknownFocusedPane(pane) => write!(formatter, "unknown focused pane {pane}"),
            Self::UnknownFocusedTab { pane, tab } => {
                write!(formatter, "pane {pane} has no focused tab {tab}")
            }
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
                    });
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

/// Presentation facts for project/workspace cards. This contains no authority.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct WorkspaceCardState {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    pub dirty: GitDirtyState,
    pub agent: WorkspaceAgentState,
    pub unread: usize,
    pub attention: usize,
    pub freshness: WorkspaceFreshness,
    pub observed_at_millis: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RetainedWorkspaceLifecycle {
    Active,
    Archived,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RetainedWorkspaceState {
    pub lifecycle: RetainedWorkspaceLifecycle,
    pub surface: WorkspaceSurfaceState,
    pub card: WorkspaceCardState,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct WorkspaceNavigationState {
    workspaces: BTreeMap<UserWorkspaceId, RetainedWorkspaceState>,
    active: Option<UserWorkspaceId>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WorkspaceNavigationAction {
    Retain {
        id: UserWorkspaceId,
        surface: WorkspaceSurfaceState,
        card: WorkspaceCardState,
    },
    SwitchTo(UserWorkspaceId),
    UpdateSurface {
        id: UserWorkspaceId,
        surface: WorkspaceSurfaceState,
    },
    UpdateCard {
        id: UserWorkspaceId,
        card: WorkspaceCardState,
    },
    Archive(UserWorkspaceId),
    Resume(UserWorkspaceId),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum NavigationError {
    DuplicateWorkspace(UserWorkspaceId),
    UnknownWorkspace(UserWorkspaceId),
    ArchivedWorkspace(UserWorkspaceId),
    InvalidSurface(SurfaceValidationError),
}

impl fmt::Display for NavigationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateWorkspace(id) => write!(formatter, "workspace {id} is already retained"),
            Self::UnknownWorkspace(id) => write!(formatter, "workspace {id} is not retained"),
            Self::ArchivedWorkspace(id) => write!(formatter, "workspace {id} is archived"),
            Self::InvalidSurface(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for NavigationError {}

impl WorkspaceNavigationState {
    #[must_use]
    pub const fn active(&self) -> Option<UserWorkspaceId> {
        self.active
    }

    #[must_use]
    pub fn workspace(&self, id: UserWorkspaceId) -> Option<&RetainedWorkspaceState> {
        self.workspaces.get(&id)
    }

    pub fn workspaces(
        &self,
    ) -> impl ExactSizeIterator<Item = (UserWorkspaceId, &RetainedWorkspaceState)> {
        self.workspaces
            .iter()
            .map(|(id, workspace)| (*id, workspace))
    }

    pub fn reduce(&mut self, action: WorkspaceNavigationAction) -> Result<(), NavigationError> {
        match action {
            WorkspaceNavigationAction::Retain { id, surface, card } => {
                surface
                    .validate()
                    .map_err(NavigationError::InvalidSurface)?;
                if self.workspaces.contains_key(&id) {
                    return Err(NavigationError::DuplicateWorkspace(id));
                }
                self.workspaces.insert(
                    id,
                    RetainedWorkspaceState {
                        lifecycle: RetainedWorkspaceLifecycle::Active,
                        surface,
                        card,
                    },
                );
                self.active.get_or_insert(id);
            }
            WorkspaceNavigationAction::SwitchTo(id) => {
                let workspace = self
                    .workspaces
                    .get(&id)
                    .ok_or(NavigationError::UnknownWorkspace(id))?;
                if workspace.lifecycle == RetainedWorkspaceLifecycle::Archived {
                    return Err(NavigationError::ArchivedWorkspace(id));
                }
                self.active = Some(id);
            }
            WorkspaceNavigationAction::UpdateSurface { id, surface } => {
                surface
                    .validate()
                    .map_err(NavigationError::InvalidSurface)?;
                self.workspace_mut(id)?.surface = surface;
            }
            WorkspaceNavigationAction::UpdateCard { id, card } => {
                self.workspace_mut(id)?.card = card;
            }
            WorkspaceNavigationAction::Archive(id) => {
                self.workspace_mut(id)?.lifecycle = RetainedWorkspaceLifecycle::Archived;
                if self.active == Some(id) {
                    self.active = None;
                }
            }
            WorkspaceNavigationAction::Resume(id) => {
                self.workspace_mut(id)?.lifecycle = RetainedWorkspaceLifecycle::Active;
            }
        }
        Ok(())
    }

    fn workspace_mut(
        &mut self,
        id: UserWorkspaceId,
    ) -> Result<&mut RetainedWorkspaceState, NavigationError> {
        self.workspaces
            .get_mut(&id)
            .ok_or(NavigationError::UnknownWorkspace(id))
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
    CheckoutAlreadyExists { root: PathBuf },
    WorktreeLocked { root: PathBuf },
    BranchAlreadyCheckedOut { branch: String },
    HostUnavailable,
    CatalogRevisionChanged,
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
        progress: WorkspaceCreateProgress,
    },
    Cancelling {
        operation: CreationOperationId,
        progress: WorkspaceCreateProgress,
    },
    Cancelled {
        operation: CreationOperationId,
    },
    Conflict {
        operation: CreationOperationId,
        conflict: WorkspaceCreateConflict,
    },
    Failed {
        operation: CreationOperationId,
        failure: WorkspaceCreateFailure,
    },
    Completed {
        operation: CreationOperationId,
    },
}

impl BackgroundWorkspaceCreateState {
    fn operation(&self) -> CreationOperationId {
        match self {
            Self::Running { operation, .. }
            | Self::Cancelling { operation, .. }
            | Self::Cancelled { operation }
            | Self::Conflict { operation, .. }
            | Self::Failed { operation, .. }
            | Self::Completed { operation } => *operation,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WorkspaceCreateEvent {
    Start {
        workspace: UserWorkspaceId,
        operation: CreationOperationId,
    },
    Progress {
        workspace: UserWorkspaceId,
        operation: CreationOperationId,
        progress: WorkspaceCreateProgress,
    },
    RequestCancel {
        workspace: UserWorkspaceId,
        operation: CreationOperationId,
    },
    Cancelled {
        workspace: UserWorkspaceId,
        operation: CreationOperationId,
    },
    Conflict {
        workspace: UserWorkspaceId,
        operation: CreationOperationId,
        conflict: WorkspaceCreateConflict,
    },
    Failed {
        workspace: UserWorkspaceId,
        operation: CreationOperationId,
        failure: WorkspaceCreateFailure,
    },
    Completed {
        workspace: UserWorkspaceId,
        operation: CreationOperationId,
    },
    Retry {
        workspace: UserWorkspaceId,
        prior_operation: CreationOperationId,
        operation: CreationOperationId,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WorkspaceCreateError {
    AlreadyRunning(UserWorkspaceId),
    ReusedOperation(CreationOperationId),
    UnknownWorkspaceJob(UserWorkspaceId),
    StaleOperation {
        expected: CreationOperationId,
        received: CreationOperationId,
    },
    InvalidTransition,
    InvalidProgress,
    ProgressRegressed,
}

impl fmt::Display for WorkspaceCreateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AlreadyRunning(workspace) => {
                write!(
                    formatter,
                    "workspace {workspace} already has a running create"
                )
            }
            Self::ReusedOperation(operation) => {
                write!(formatter, "create operation {operation} was already used")
            }
            Self::UnknownWorkspaceJob(workspace) => {
                write!(formatter, "workspace {workspace} has no create job")
            }
            Self::StaleOperation { expected, received } => write!(
                formatter,
                "stale create operation {received}; current operation is {expected}"
            ),
            Self::InvalidTransition => formatter.write_str("invalid workspace-create transition"),
            Self::InvalidProgress => formatter.write_str("invalid workspace-create progress"),
            Self::ProgressRegressed => {
                formatter.write_str("workspace-create progress cannot move backwards")
            }
        }
    }
}

impl std::error::Error for WorkspaceCreateError {}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct WorkspaceCreationReducer {
    jobs: BTreeMap<UserWorkspaceId, BackgroundWorkspaceCreateState>,
    seen_operations: BTreeSet<CreationOperationId>,
}

impl WorkspaceCreationReducer {
    #[must_use]
    pub fn state(&self, workspace: UserWorkspaceId) -> Option<&BackgroundWorkspaceCreateState> {
        self.jobs.get(&workspace)
    }

    pub fn reduce(&mut self, event: WorkspaceCreateEvent) -> Result<(), WorkspaceCreateError> {
        match event {
            WorkspaceCreateEvent::Start {
                workspace,
                operation,
            } => {
                if matches!(
                    self.jobs.get(&workspace),
                    Some(BackgroundWorkspaceCreateState::Running { .. })
                        | Some(BackgroundWorkspaceCreateState::Cancelling { .. })
                ) {
                    return Err(WorkspaceCreateError::AlreadyRunning(workspace));
                }
                if !self.seen_operations.insert(operation) {
                    return Err(WorkspaceCreateError::ReusedOperation(operation));
                }
                self.jobs.insert(
                    workspace,
                    BackgroundWorkspaceCreateState::Running {
                        operation,
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
                let state = self.matching_state_mut(workspace, operation)?;
                let BackgroundWorkspaceCreateState::Running {
                    progress: previous, ..
                } = state
                else {
                    return Err(WorkspaceCreateError::InvalidTransition);
                };
                if progress.phase < previous.phase
                    || (progress.phase == previous.phase
                        && progress.completed_steps < previous.completed_steps)
                {
                    return Err(WorkspaceCreateError::ProgressRegressed);
                }
                *previous = progress;
            }
            WorkspaceCreateEvent::RequestCancel {
                workspace,
                operation,
            } => {
                let state = self.matching_state_mut(workspace, operation)?;
                let BackgroundWorkspaceCreateState::Running { progress, .. } = state else {
                    return Err(WorkspaceCreateError::InvalidTransition);
                };
                let progress = progress.clone();
                *state = BackgroundWorkspaceCreateState::Cancelling {
                    operation,
                    progress,
                };
            }
            WorkspaceCreateEvent::Cancelled {
                workspace,
                operation,
            } => {
                let state = self.matching_state_mut(workspace, operation)?;
                if !matches!(state, BackgroundWorkspaceCreateState::Cancelling { .. }) {
                    return Err(WorkspaceCreateError::InvalidTransition);
                }
                *state = BackgroundWorkspaceCreateState::Cancelled { operation };
            }
            WorkspaceCreateEvent::Conflict {
                workspace,
                operation,
                conflict,
            } => {
                let state = self.matching_state_mut(workspace, operation)?;
                if !matches!(
                    state,
                    BackgroundWorkspaceCreateState::Running { .. }
                        | BackgroundWorkspaceCreateState::Cancelling { .. }
                ) {
                    return Err(WorkspaceCreateError::InvalidTransition);
                }
                *state = BackgroundWorkspaceCreateState::Conflict {
                    operation,
                    conflict,
                };
            }
            WorkspaceCreateEvent::Failed {
                workspace,
                operation,
                failure,
            } => {
                let state = self.matching_state_mut(workspace, operation)?;
                if !matches!(
                    state,
                    BackgroundWorkspaceCreateState::Running { .. }
                        | BackgroundWorkspaceCreateState::Cancelling { .. }
                ) {
                    return Err(WorkspaceCreateError::InvalidTransition);
                }
                *state = BackgroundWorkspaceCreateState::Failed { operation, failure };
            }
            WorkspaceCreateEvent::Completed {
                workspace,
                operation,
            } => {
                let state = self.matching_state_mut(workspace, operation)?;
                if !matches!(state, BackgroundWorkspaceCreateState::Running { .. }) {
                    return Err(WorkspaceCreateError::InvalidTransition);
                }
                *state = BackgroundWorkspaceCreateState::Completed { operation };
            }
            WorkspaceCreateEvent::Retry {
                workspace,
                prior_operation,
                operation,
            } => {
                {
                    let state = self.matching_state_mut(workspace, prior_operation)?;
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
                let state = self.matching_state_mut(workspace, prior_operation)?;
                *state = BackgroundWorkspaceCreateState::Running {
                    operation,
                    progress: WorkspaceCreateProgress::default(),
                };
            }
        }
        Ok(())
    }

    fn matching_state_mut(
        &mut self,
        workspace: UserWorkspaceId,
        operation: CreationOperationId,
    ) -> Result<&mut BackgroundWorkspaceCreateState, WorkspaceCreateError> {
        let state = self
            .jobs
            .get_mut(&workspace)
            .ok_or(WorkspaceCreateError::UnknownWorkspaceJob(workspace))?;
        let expected = state.operation();
        if expected != operation {
            return Err(WorkspaceCreateError::StaleOperation {
                expected,
                received: operation,
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

    fn uuid(value: u128) -> Uuid {
        Uuid::from_u128(value)
    }

    fn workspace(value: u128) -> UserWorkspaceId {
        UserWorkspaceId::from_uuid(uuid(value))
    }

    fn terminal_tab(
        tab: u128,
        binding: u128,
        checkout: u128,
        scrollback: usize,
        draft: &str,
    ) -> WorkspaceTab {
        WorkspaceTab {
            id: WorkspaceTabId::from_uuid(uuid(tab)),
            title: "Terminal".into(),
            content: WorkspaceTabContent::Terminal(TerminalSurface {
                binding: TerminalBinding {
                    id: TerminalBindingId::from_uuid(uuid(binding)),
                    authority: TerminalAuthority::Local {
                        checkout_id: CheckoutId::from_uuid(uuid(checkout)),
                    },
                },
                viewport: TerminalViewport {
                    scrollback_offset_lines: scrollback,
                    follow_output: false,
                },
                draft: draft.into(),
            }),
        }
    }

    fn one_pane_surface(
        pane: u128,
        tab: u128,
        binding: u128,
        checkout: u128,
        scrollback: usize,
        draft: &str,
    ) -> WorkspaceSurfaceState {
        let pane_id = PaneId::from_uuid(uuid(pane));
        let tab = terminal_tab(tab, binding, checkout, scrollback, draft);
        let tab_id = tab.id;
        WorkspaceSurfaceState {
            root: Some(PaneNode::Leaf(PaneLeaf {
                id: pane_id,
                tabs: vec![tab],
                active_tab: Some(tab_id),
            })),
            focus: Some(WorkspaceFocus { pane_id, tab_id }),
        }
    }

    // SDTEST-1731
    #[test]
    fn keyed_switch_restores_exact_layout_focus_scrollback_drafts_and_hidden_terminals() {
        let first_id = workspace(1);
        let second_id = workspace(2);
        let first_surface = one_pane_surface(10, 11, 12, 13, 240, "cargo test");
        let second_surface = one_pane_surface(20, 21, 22, 23, 9, "git status");
        let mut state = WorkspaceNavigationState::default();

        state
            .reduce(WorkspaceNavigationAction::Retain {
                id: first_id,
                surface: first_surface.clone(),
                card: WorkspaceCardState::default(),
            })
            .expect("retain first");
        state
            .reduce(WorkspaceNavigationAction::Retain {
                id: second_id,
                surface: second_surface.clone(),
                card: WorkspaceCardState::default(),
            })
            .expect("retain second");
        state
            .reduce(WorkspaceNavigationAction::SwitchTo(second_id))
            .expect("switch second");
        state
            .reduce(WorkspaceNavigationAction::SwitchTo(first_id))
            .expect("switch first");

        assert_eq!(state.active(), Some(first_id));
        assert_eq!(state.workspaces().len(), 2);
        assert_eq!(state.workspace(first_id).unwrap().surface, first_surface);
        assert_eq!(state.workspace(second_id).unwrap().surface, second_surface);

        state
            .reduce(WorkspaceNavigationAction::Archive(first_id))
            .expect("archive");
        assert_eq!(state.active(), None);
        assert_eq!(state.workspace(first_id).unwrap().surface, first_surface);
        state
            .reduce(WorkspaceNavigationAction::Resume(first_id))
            .expect("resume");
        state
            .reduce(WorkspaceNavigationAction::SwitchTo(first_id))
            .expect("switch resumed");
        assert_eq!(state.workspace(first_id).unwrap().surface, first_surface);
    }

    // SDTEST-1732
    #[test]
    fn surface_validation_rejects_ambiguous_focus_tabs_splits_and_terminal_identity_reuse() {
        let shared_binding = TerminalBindingId::from_uuid(uuid(90));
        let first_tab = terminal_tab(31, 90, 33, 0, "");
        let mut second_tab = terminal_tab(41, 91, 43, 0, "");
        if let WorkspaceTabContent::Terminal(terminal) = &mut second_tab.content {
            terminal.binding.id = shared_binding;
        }
        let invalid = WorkspaceSurfaceState {
            root: Some(PaneNode::Split {
                axis: SplitAxis::Horizontal,
                ratio_basis_points: 5_000,
                first: Box::new(PaneNode::Leaf(PaneLeaf {
                    id: PaneId::from_uuid(uuid(30)),
                    active_tab: Some(first_tab.id),
                    tabs: vec![first_tab],
                })),
                second: Box::new(PaneNode::Leaf(PaneLeaf {
                    id: PaneId::from_uuid(uuid(40)),
                    active_tab: Some(second_tab.id),
                    tabs: vec![second_tab],
                })),
            }),
            focus: None,
        };
        assert_eq!(
            invalid.validate(),
            Err(SurfaceValidationError::DuplicateTerminalBinding(
                shared_binding
            ))
        );

        let bad_ratio = WorkspaceSurfaceState {
            root: Some(PaneNode::Split {
                axis: SplitAxis::Vertical,
                ratio_basis_points: 10_000,
                first: Box::new(PaneNode::Leaf(PaneLeaf {
                    id: PaneId::from_uuid(uuid(50)),
                    tabs: Vec::new(),
                    active_tab: None,
                })),
                second: Box::new(PaneNode::Leaf(PaneLeaf {
                    id: PaneId::from_uuid(uuid(51)),
                    tabs: Vec::new(),
                    active_tab: None,
                })),
            }),
            focus: None,
        };
        assert_eq!(
            bad_ratio.validate(),
            Err(SurfaceValidationError::InvalidSplitRatio(10_000))
        );

        let bad_focus = WorkspaceSurfaceState {
            root: None,
            focus: Some(WorkspaceFocus {
                pane_id: PaneId::from_uuid(uuid(60)),
                tab_id: WorkspaceTabId::from_uuid(uuid(61)),
            }),
        };
        assert_eq!(
            bad_focus.validate(),
            Err(SurfaceValidationError::FocusWithoutSurface)
        );
    }

    // SDTEST-1733
    #[test]
    fn background_creation_progress_cancel_conflict_and_retry_ignore_stale_operations() {
        let workspace = workspace(70);
        let first = CreationOperationId::from_uuid(uuid(71));
        let retry = CreationOperationId::from_uuid(uuid(72));
        let mut reducer = WorkspaceCreationReducer::default();

        reducer
            .reduce(WorkspaceCreateEvent::Start {
                workspace,
                operation: first,
            })
            .expect("start");
        reducer
            .reduce(WorkspaceCreateEvent::Progress {
                workspace,
                operation: first,
                progress: WorkspaceCreateProgress {
                    phase: WorkspaceCreatePhase::PreparingCheckout,
                    completed_steps: 2,
                    total_steps: 5,
                    detail: "Preparing worktree".into(),
                },
            })
            .expect("progress");
        reducer
            .reduce(WorkspaceCreateEvent::RequestCancel {
                workspace,
                operation: first,
            })
            .expect("request cancel");
        reducer
            .reduce(WorkspaceCreateEvent::Conflict {
                workspace,
                operation: first,
                conflict: WorkspaceCreateConflict::WorktreeLocked {
                    root: PathBuf::from("workspaces").join("issue-127"),
                },
            })
            .expect("typed conflict while cancelling");
        assert_eq!(
            reducer.reduce(WorkspaceCreateEvent::Retry {
                workspace,
                prior_operation: first,
                operation: first,
            }),
            Err(WorkspaceCreateError::ReusedOperation(first))
        );
        reducer
            .reduce(WorkspaceCreateEvent::Retry {
                workspace,
                prior_operation: first,
                operation: retry,
            })
            .expect("retry");

        let stale = reducer.reduce(WorkspaceCreateEvent::Completed {
            workspace,
            operation: first,
        });
        assert_eq!(
            stale,
            Err(WorkspaceCreateError::StaleOperation {
                expected: retry,
                received: first,
            })
        );
        assert!(matches!(
            reducer.state(workspace),
            Some(BackgroundWorkspaceCreateState::Running {
                operation,
                progress: WorkspaceCreateProgress {
                    phase: WorkspaceCreatePhase::Queued,
                    ..
                }
            }) if *operation == retry
        ));
    }

    // SDTEST-1734
    #[test]
    fn local_ssh_and_provider_bindings_share_a_surface_without_sharing_authority() {
        let local = WorkspaceTab {
            id: WorkspaceTabId::from_uuid(uuid(81)),
            title: "Local".into(),
            content: WorkspaceTabContent::Terminal(TerminalSurface {
                binding: TerminalBinding {
                    id: TerminalBindingId::from_uuid(uuid(82)),
                    authority: TerminalAuthority::Local {
                        checkout_id: CheckoutId::from_uuid(uuid(83)),
                    },
                },
                viewport: TerminalViewport::default(),
                draft: String::new(),
            }),
        };
        let ssh = WorkspaceTab {
            id: WorkspaceTabId::from_uuid(uuid(84)),
            title: "SSH".into(),
            content: WorkspaceTabContent::Terminal(TerminalSurface {
                binding: TerminalBinding {
                    id: TerminalBindingId::from_uuid(uuid(85)),
                    authority: TerminalAuthority::Ssh {
                        checkout_id: CheckoutId::from_uuid(uuid(86)),
                        connection_id: uuid(87),
                    },
                },
                viewport: TerminalViewport::default(),
                draft: String::new(),
            }),
        };
        let provider = WorkspaceTab {
            id: WorkspaceTabId::from_uuid(uuid(88)),
            title: "Agent".into(),
            content: WorkspaceTabContent::ProviderSession(ProviderSessionBinding {
                authority: "tenant/project/workspace".into(),
                session_id: "provider-session".into(),
                run_id: Some("provider-run".into()),
            }),
        };
        let pane = PaneId::from_uuid(uuid(89));
        let surface = WorkspaceSurfaceState {
            root: Some(PaneNode::Leaf(PaneLeaf {
                id: pane,
                active_tab: Some(local.id),
                tabs: vec![local.clone(), ssh.clone(), provider.clone()],
            })),
            focus: Some(WorkspaceFocus {
                pane_id: pane,
                tab_id: provider.id,
            }),
        };

        surface.validate().expect("mixed surface is valid");
        let PaneNode::Leaf(leaf) = surface.root.unwrap() else {
            panic!("one leaf expected");
        };
        assert_eq!(leaf.tabs[0].content, local.content);
        assert_eq!(leaf.tabs[1].content, ssh.content);
        assert_eq!(leaf.tabs[2].content, provider.content);
    }
}
