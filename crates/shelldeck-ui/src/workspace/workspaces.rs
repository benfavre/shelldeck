//! Catalogue de travail retenu et lanceur de workspaces.
//!
//! Ce module ne possède aucune exécution git/SSH. Il projette uniquement le
//! catalogue local autorisé, conserve les entités GPUI par workspace et remet
//! les demandes de création à une frontière d'exécuteur injectée.

use super::*;
use adabraka_ui::components::input::InputVariant;
use adabraka_ui::display::card::Card;
use adabraka_ui::prelude::{Alert, AlertVariant};
use shelldeck_core::config::workspace_catalog::{
    CatalogCheckoutId, CatalogProjectId, CatalogWorkspaceId, CheckoutHost, ExternalWorkItem,
    ExternalWorkItemKind, ProjectCatalog, ProjectCheckout, ProjectRecord, RemotePosixPath,
    RepositoryIdentity, UserWorkspaceLifecycle, UserWorkspaceRecord, WorkspaceLaunchIntake,
    WorkspaceLaunchRequest,
};
use shelldeck_core::workspace_navigation::{
    BackgroundWorkspaceCreateState, CreationOperationId, GitDirtyState, PaneId, PaneLeaf,
    TerminalAuthority, TerminalBinding, TerminalBindingId, TerminalSurface, TerminalViewport,
    WorkspaceAgentState, WorkspaceCardState, WorkspaceCreateConflict, WorkspaceCreateEvent,
    WorkspaceCreateFailure, WorkspaceCreateFailureKind, WorkspaceCreatePhase,
    WorkspaceCreateProgress, WorkspaceCreationReducer, WorkspaceFreshness,
    WorkspaceNavigationAction, WorkspaceNavigationState, WorkspaceSurfaceState, WorkspaceTab,
    WorkspaceTabContent, WorkspaceTabId,
};
use shelldeck_terminal::session::SessionState;
use std::collections::{BTreeMap, HashMap};
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::mpsc;

type ExecutorFuture<T> = Pin<Box<dyn Future<Output = T> + Send + 'static>>;

/// Autorité immuable résolue avant de quitter le catalogue.
#[derive(Clone, Debug)]
pub(super) enum AuthorizedLaunchHost {
    Local {
        canonical_root: PathBuf,
    },
    Ssh {
        connection_id: Uuid,
        remote_root: String,
    },
}

/// Requête riche et immuable livrée à l'adaptateur d'effets.
#[derive(Clone, Debug)]
pub(super) struct WorkspaceExecutionRequest {
    pub workspace: CatalogWorkspaceId,
    pub checkout: CatalogCheckoutId,
    pub operation: CreationOperationId,
    pub catalog_revision: u64,
    pub name: String,
    pub intake: WorkspaceLaunchIntake,
    pub host: AuthorizedLaunchHost,
}

/// Frontière non bloquante du futur adaptateur d'effets.
///
/// L'implémentation de production reste volontairement indisponible dans ce
/// lot. Un adaptateur devra faire le travail hors du thread GPUI et retourner
/// des événements typés du réducteur, jamais des mutations directes de la vue.
pub(super) trait WorkspaceLaunchExecutor: Send + Sync {
    fn launch(
        &self,
        request: WorkspaceExecutionRequest,
        events: mpsc::UnboundedSender<WorkspaceCreateEvent>,
    ) -> ExecutorFuture<Result<(), WorkspaceCreateFailure>>;
    fn cancel(
        &self,
        request: WorkspaceExecutionRequest,
    ) -> ExecutorFuture<Result<WorkspaceCreateEvent, WorkspaceCreateFailure>>;
}

#[derive(Default)]
struct ExistingFolderWorkspaceExecutor;

impl WorkspaceLaunchExecutor for ExistingFolderWorkspaceExecutor {
    fn launch(
        &self,
        request: WorkspaceExecutionRequest,
        events: mpsc::UnboundedSender<WorkspaceCreateEvent>,
    ) -> ExecutorFuture<Result<(), WorkspaceCreateFailure>> {
        Box::pin(async move {
            tracing::debug!(
                checkout = %request.checkout,
                intake = ?request.intake,
                revision = request.catalog_revision,
                "launching authorized workspace attach"
            );
            if let AuthorizedLaunchHost::Ssh {
                connection_id,
                remote_root,
            } = &request.host
            {
                tracing::debug!(%connection_id, remote_root, "SSH attach adapter unavailable");
            }
            let AuthorizedLaunchHost::Local { canonical_root } = &request.host else {
                return Err(WorkspaceCreateFailure {
                    kind: WorkspaceCreateFailureKind::RuntimeUnavailable,
                    message: t!("workspaces.launcher.ssh_executor_unavailable").to_string(),
                    retryable: false,
                });
            };
            if !canonical_root.is_dir() {
                return Err(WorkspaceCreateFailure {
                    kind: WorkspaceCreateFailureKind::Filesystem,
                    message: t!("workspaces.launcher.folder_unavailable").to_string(),
                    retryable: true,
                });
            }
            let phases = [
                WorkspaceCreatePhase::ResolvingHost,
                WorkspaceCreatePhase::PreparingCheckout,
                WorkspaceCreatePhase::CreatingWorkspace,
                WorkspaceCreatePhase::BindingRuntime,
            ];
            for (index, phase) in phases.into_iter().enumerate() {
                events
                    .send(WorkspaceCreateEvent::Progress {
                        workspace: request.workspace,
                        operation: request.operation,
                        progress: WorkspaceCreateProgress {
                            phase,
                            completed_steps: (index + 1) as u32,
                            total_steps: phases.len() as u32,
                            detail: request.name.clone(),
                        },
                    })
                    .map_err(|_| WorkspaceCreateFailure {
                        kind: WorkspaceCreateFailureKind::Unknown,
                        message: "workspace progress receiver closed".into(),
                        retryable: true,
                    })?;
                tokio::task::yield_now().await;
            }
            events
                .send(WorkspaceCreateEvent::Completed {
                    workspace: request.workspace,
                    operation: request.operation,
                })
                .map_err(|_| WorkspaceCreateFailure {
                    kind: WorkspaceCreateFailureKind::Unknown,
                    message: "workspace progress receiver closed".into(),
                    retryable: true,
                })?;
            Ok(())
        })
    }

    fn cancel(
        &self,
        request: WorkspaceExecutionRequest,
    ) -> ExecutorFuture<Result<WorkspaceCreateEvent, WorkspaceCreateFailure>> {
        Box::pin(async move {
            Ok(WorkspaceCreateEvent::Cancelled {
                workspace: request.workspace,
                operation: request.operation,
            })
        })
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum LauncherIntakeKind {
    #[default]
    Manual,
    Issue,
    PullRequest,
    Task,
}

impl LauncherIntakeKind {
    fn external_kind(self) -> Option<ExternalWorkItemKind> {
        match self {
            Self::Manual => None,
            Self::Issue => Some(ExternalWorkItemKind::Issue),
            Self::PullRequest => Some(ExternalWorkItemKind::PullRequest),
            Self::Task => Some(ExternalWorkItemKind::Task),
        }
    }

    fn label(self) -> String {
        match self {
            Self::Manual => t!("workspaces.launcher.intake.manual"),
            Self::Issue => t!("workspaces.launcher.intake.issue"),
            Self::PullRequest => t!("workspaces.launcher.intake.pull_request"),
            Self::Task => t!("workspaces.launcher.intake.task"),
        }
        .to_string()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct WorkspaceLauncherDraft {
    intake: LauncherIntakeKind,
    checkout: Option<CatalogCheckoutId>,
    name: String,
    provider: String,
    repository: String,
    key: String,
    title: String,
    url: String,
}

impl Default for WorkspaceLauncherDraft {
    fn default() -> Self {
        Self {
            intake: LauncherIntakeKind::Manual,
            checkout: None,
            name: String::new(),
            provider: String::new(),
            repository: String::new(),
            key: String::new(),
            title: String::new(),
            url: String::new(),
        }
    }
}

impl WorkspaceLauncherDraft {
    fn launch_intake(&self) -> Result<WorkspaceLaunchIntake, String> {
        let Some(kind) = self.intake.external_kind() else {
            return Ok(WorkspaceLaunchIntake::Manual);
        };
        if self.provider.trim().is_empty()
            || self.repository.trim().is_empty()
            || self.key.trim().is_empty()
        {
            return Err(t!("workspaces.launcher.error.external_required").to_string());
        }
        Ok(WorkspaceLaunchIntake::Prefilled(ExternalWorkItem {
            provider: self.provider.trim().to_owned(),
            repository: self.repository.trim().to_owned(),
            kind,
            key: self.key.trim().to_owned(),
            title: non_empty(&self.title),
            url: non_empty(&self.url),
        }))
    }
}

fn non_empty(value: &str) -> Option<String> {
    (!value.trim().is_empty()).then(|| value.trim().to_owned())
}

fn mutate_and_persist<T>(
    catalog: &mut ProjectCatalog,
    mutate: impl FnOnce(&mut ProjectCatalog) -> Result<T, String>,
    persist: impl FnOnce(&mut ProjectCatalog) -> Result<(), String>,
) -> Result<T, String> {
    let before = catalog.clone();
    let value = mutate(catalog)?;
    if let Err(error) = persist(catalog) {
        *catalog = before;
        return Err(error);
    }
    Ok(value)
}

fn mutate_and_save<T>(
    catalog: &mut ProjectCatalog,
    mutate: impl FnOnce(&mut ProjectCatalog) -> Result<T, String>,
) -> Result<T, String> {
    mutate_and_persist(catalog, mutate, |catalog| {
        catalog.save().map_err(|error| error.to_string())
    })
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct CatalogCheckoutPresentation {
    project: String,
    checkout: String,
    repository: String,
    host: String,
    host_kind: &'static str,
}

fn checkout_presentation(
    project: &ProjectRecord,
    checkout: &ProjectCheckout,
    connections: &HashMap<Uuid, String>,
) -> CatalogCheckoutPresentation {
    let (host, host_kind) = match checkout.host() {
        CheckoutHost::Local { device_label, .. } => {
            (device_label.clone(), "workspaces.authority.local")
        }
        CheckoutHost::Ssh { connection_id, .. } => (
            connections
                .get(connection_id)
                .cloned()
                .unwrap_or_else(|| t!("workspaces.authority.ssh_unknown").to_string()),
            "workspaces.authority.ssh",
        ),
    };
    CatalogCheckoutPresentation {
        project: project.name().to_owned(),
        checkout: checkout.label().to_owned(),
        repository: checkout.repository().slug.clone(),
        host,
        host_kind,
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct WorkspaceCardPresentation {
    id: CatalogWorkspaceId,
    name: String,
    project: String,
    checkout: String,
    repository: String,
    host: String,
    host_kind: &'static str,
    branch: Option<String>,
    dirty: GitDirtyState,
    external: Option<String>,
    orchestration: Option<String>,
    agent: WorkspaceAgentState,
    unread: usize,
    attention: usize,
    freshness: WorkspaceFreshness,
    observed: bool,
    archived: bool,
    provider_bound: bool,
}

fn workspace_card_presentation(
    catalog: &ProjectCatalog,
    workspace: &UserWorkspaceRecord,
    card: &WorkspaceCardState,
    connections: &HashMap<Uuid, String>,
) -> Option<WorkspaceCardPresentation> {
    let project = catalog
        .projects()
        .find(|project| project.id() == workspace.project_id())?;
    let checkout = catalog
        .checkout_in_project(workspace.project_id(), workspace.checkout_id())
        .ok()?;
    let checkout = checkout_presentation(project, checkout, connections);
    let external = workspace.linked_work_item().map(|item| {
        let kind = match item.kind {
            ExternalWorkItemKind::Issue => t!("workspaces.card.issue"),
            ExternalWorkItemKind::PullRequest => t!("workspaces.card.pull_request"),
            ExternalWorkItemKind::Task => t!("workspaces.card.task"),
        };
        format!("{kind} {} · {}", item.key, item.repository)
    });
    let orchestration = workspace
        .orchestration_run()
        .map(|run| format!("{} · {}", run.runtime, run.run_id))
        .or_else(|| {
            workspace
                .legacy_orchestration_run()
                .map(|run| format!("{} · {}", run.runtime, run.run_id))
        });
    Some(WorkspaceCardPresentation {
        id: workspace.id(),
        name: workspace.name().to_owned(),
        project: checkout.project,
        checkout: checkout.checkout,
        repository: checkout.repository,
        host: checkout.host,
        host_kind: checkout.host_kind,
        branch: card.branch.clone(),
        dirty: card.dirty,
        external,
        orchestration,
        agent: card.agent,
        unread: card.unread,
        attention: card.attention,
        freshness: card.freshness,
        observed: card.observed_at_millis > 0,
        archived: workspace.lifecycle() == UserWorkspaceLifecycle::Archived,
        provider_bound: workspace.orchestration_run().is_some(),
    })
}

/// Propriétaire stable d'un vrai terminal natif. Masquer la surface ne ferme
/// ni le PTY ni ses splits; la capture ne contient que l'état réapplicable.
struct RetainedWorkspaceSurface {
    workspace: CatalogWorkspaceId,
    terminal: Entity<TerminalView>,
    native_snapshot: Option<crate::terminal_view::TerminalWorkspaceSnapshot>,
}

impl RetainedWorkspaceSurface {
    fn new(workspace: CatalogWorkspaceId, terminal: Entity<TerminalView>) -> Self {
        Self {
            workspace,
            terminal,
            native_snapshot: None,
        }
    }

    fn capture(&mut self, cx: &mut Context<Self>) {
        self.native_snapshot = Some(self.terminal.read(cx).capture_workspace_snapshot());
    }

    fn apply(&mut self, cx: &mut Context<Self>) {
        if let Some(snapshot) = self.native_snapshot.as_ref() {
            self.terminal.update(cx, |terminal, _| {
                terminal.apply_workspace_snapshot(snapshot)
            });
        }
    }
}

impl Render for RetainedWorkspaceSurface {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id((
                "retained-workspace",
                self.workspace.as_uuid().as_u128() as u64,
            ))
            .flex()
            .flex_col()
            .size_full()
            .child(
                Alert::info()
                    .description(t!("workspaces.surface.native_terminal_only").to_string()),
            )
            .child(self.terminal.clone())
    }
}

fn terminal_surface(
    catalog: &ProjectCatalog,
    workspace_id: CatalogWorkspaceId,
    terminal: &TerminalView,
) -> WorkspaceSurfaceState {
    let Ok(workspace) = catalog.workspace(workspace_id) else {
        return WorkspaceSurfaceState::default();
    };
    let Ok(checkout) = catalog.checkout_in_project(workspace.project_id(), workspace.checkout_id())
    else {
        return WorkspaceSurfaceState::default();
    };
    let tabs = terminal
        .tabs
        .iter()
        .filter_map(|tab| {
            let authority = match checkout.host() {
                CheckoutHost::Local { .. } if tab.connection_id.is_none() => {
                    TerminalAuthority::Local {
                        checkout_id: workspace.checkout_id(),
                    }
                }
                CheckoutHost::Ssh { connection_id, .. }
                    if tab.connection_id == Some(*connection_id) =>
                {
                    TerminalAuthority::Ssh {
                        checkout_id: workspace.checkout_id(),
                        connection_id: *connection_id,
                    }
                }
                _ => return None,
            };
            Some(WorkspaceTab {
                id: WorkspaceTabId::from_uuid(tab.id),
                title: tab.title.clone(),
                content: WorkspaceTabContent::Terminal(TerminalSurface {
                    binding: TerminalBinding {
                        id: TerminalBindingId::from_uuid(tab.id),
                        authority,
                    },
                    viewport: TerminalViewport::default(),
                    draft: String::new(),
                }),
            })
        })
        .collect::<Vec<_>>();
    let active_tab = terminal
        .tabs
        .get(terminal.active_tab_index())
        .map(|tab| WorkspaceTabId::from_uuid(tab.id))
        .filter(|active| tabs.iter().any(|tab| tab.id == *active));
    let pane_id = PaneId::from_uuid(workspace_id.as_uuid());
    WorkspaceSurfaceState {
        root: Some(shelldeck_core::workspace_navigation::PaneNode::Leaf(
            PaneLeaf {
                id: pane_id,
                tabs,
                active_tab,
            },
        )),
        focus: active_tab
            .map(|tab_id| shelldeck_core::workspace_navigation::WorkspaceFocus { pane_id, tab_id }),
    }
}

pub(super) struct WorkspaceHubView {
    catalog: ProjectCatalog,
    navigation: WorkspaceNavigationState,
    creation: WorkspaceCreationReducer,
    retained: BTreeMap<CatalogWorkspaceId, Entity<RetainedWorkspaceSurface>>,
    unclaimed_terminal: Option<Entity<TerminalView>>,
    connections: HashMap<Uuid, String>,
    executor: Arc<dyn WorkspaceLaunchExecutor>,
    onboarding_open: bool,
    onboarding_ssh: bool,
    onboarding_connection: Option<Uuid>,
    onboarding_project: Entity<InputState>,
    onboarding_checkout: Entity<InputState>,
    onboarding_repository: Entity<InputState>,
    onboarding_root: Entity<InputState>,
    launcher_open: bool,
    launcher: WorkspaceLauncherDraft,
    name_state: Entity<InputState>,
    provider_state: Entity<InputState>,
    repository_state: Entity<InputState>,
    key_state: Entity<InputState>,
    title_state: Entity<InputState>,
    url_state: Entity<InputState>,
    checkout_select: Entity<Select<CatalogCheckoutId>>,
    error: Option<String>,
    load_error: Option<String>,
    pending_requests: BTreeMap<CatalogWorkspaceId, WorkspaceExecutionRequest>,
}

pub(super) enum WorkspaceHubEvent {
    ActiveTerminal(Entity<TerminalView>),
}

impl EventEmitter<WorkspaceHubEvent> for WorkspaceHubView {}

impl WorkspaceHubView {
    pub(super) fn new(
        catalog: Result<ProjectCatalog, String>,
        connections: &[(Uuid, String)],
        initial_terminal: Entity<TerminalView>,
        cx: &mut Context<Self>,
    ) -> Self {
        Self::new_with_executor(
            catalog,
            connections,
            initial_terminal,
            Arc::new(ExistingFolderWorkspaceExecutor),
            cx,
        )
    }

    fn new_with_executor(
        catalog: Result<ProjectCatalog, String>,
        connections: &[(Uuid, String)],
        initial_terminal: Entity<TerminalView>,
        executor: Arc<dyn WorkspaceLaunchExecutor>,
        cx: &mut Context<Self>,
    ) -> Self {
        let (catalog, load_error) = match catalog {
            Ok(catalog) => (catalog, None),
            Err(error) => (ProjectCatalog::default(), Some(error)),
        };
        let connections: HashMap<_, _> = connections.iter().cloned().collect();
        let parent = cx.entity();
        let checkouts = checkout_options(&catalog, &connections);
        let selected = checkouts.first().map(|(id, _)| *id);
        let checkout_select = cx.new({
            let parent = parent.clone();
            move |select_cx| {
                Select::new(select_cx)
                    .options(
                        checkouts
                            .into_iter()
                            .map(|(id, label)| SelectOption::new(id, label))
                            .collect(),
                    )
                    .selected_index(selected.map(|_| 0))
                    .placeholder(t!("workspaces.launcher.checkout_placeholder").to_string())
                    .searchable(true)
                    .search_placeholder(t!("workspaces.launcher.checkout_search").to_string())
                    .disabled(selected.is_none())
                    .on_change(move |checkout, _window, cx| {
                        parent.update(cx, |this, cx| {
                            this.launcher.checkout = Some(*checkout);
                            cx.notify();
                        });
                    })
            }
        });
        let mut navigation = WorkspaceNavigationState::default();
        let mut retained = BTreeMap::new();
        let mut initial_terminal = Some(initial_terminal);
        for workspace in catalog.workspaces() {
            let surface = WorkspaceSurfaceState::default();
            let _ = navigation.reduce(
                &catalog,
                WorkspaceNavigationAction::Retain {
                    id: workspace.id(),
                    surface: surface.clone(),
                    card: WorkspaceCardState::default(),
                },
            );
            let terminal = if workspace.lifecycle() == UserWorkspaceLifecycle::Active {
                initial_terminal
                    .take()
                    .unwrap_or_else(|| cx.new(TerminalView::new))
            } else {
                cx.new(TerminalView::new)
            };
            retained.insert(
                workspace.id(),
                cx.new(move |_| RetainedWorkspaceSurface::new(workspace.id(), terminal)),
            );
        }
        let onboarding_open = catalog.projects().len() == 0;
        let onboarding_connection = connections.keys().next().copied();
        Self {
            catalog,
            navigation,
            creation: WorkspaceCreationReducer::default(),
            retained,
            unclaimed_terminal: initial_terminal,
            connections,
            executor,
            onboarding_open,
            onboarding_ssh: false,
            onboarding_connection,
            onboarding_project: cx.new(InputState::new),
            onboarding_checkout: cx.new(InputState::new),
            onboarding_repository: cx.new(InputState::new),
            onboarding_root: cx.new(InputState::new),
            launcher_open: false,
            launcher: WorkspaceLauncherDraft {
                checkout: selected,
                ..WorkspaceLauncherDraft::default()
            },
            name_state: cx.new(InputState::new),
            provider_state: cx.new(InputState::new),
            repository_state: cx.new(InputState::new),
            key_state: cx.new(InputState::new),
            title_state: cx.new(InputState::new),
            url_state: cx.new(InputState::new),
            checkout_select,
            error: None,
            load_error,
            pending_requests: BTreeMap::new(),
        }
    }

    fn switch_to(&mut self, id: CatalogWorkspaceId, cx: &mut Context<Self>) {
        if let Some(outgoing) = self.navigation.active() {
            if let Some(surface) = self.retained.get(&outgoing) {
                let state = {
                    let retained = surface.read(cx);
                    terminal_surface(&self.catalog, outgoing, retained.terminal.read(cx))
                };
                surface.update(cx, |surface, cx| surface.capture(cx));
                if let Err(error) = self.navigation.reduce(
                    &self.catalog,
                    WorkspaceNavigationAction::UpdateSurface {
                        id: outgoing,
                        surface: state,
                    },
                ) {
                    self.error = Some(error.to_string());
                    cx.notify();
                    return;
                }
            }
        }
        match self
            .navigation
            .reduce(&self.catalog, WorkspaceNavigationAction::SwitchTo(id))
        {
            Ok(()) => {
                self.error = None;
                if let Some(surface) = self.retained.get(&id) {
                    surface.update(cx, |surface, cx| surface.apply(cx));
                    cx.emit(WorkspaceHubEvent::ActiveTerminal(
                        surface.read(cx).terminal.clone(),
                    ));
                }
                self.observe_local_card(id, cx);
            }
            Err(error) => self.error = Some(error.to_string()),
        }
        cx.notify();
    }

    fn observe_local_card(&self, workspace: CatalogWorkspaceId, cx: &mut Context<Self>) {
        let Some((root, terminal)) = self.catalog.workspace(workspace).ok().and_then(|record| {
            let checkout = self
                .catalog
                .checkout_in_project(record.project_id(), record.checkout_id())
                .ok()?;
            let CheckoutHost::Local { root, .. } = checkout.host() else {
                return None;
            };
            Some((
                root.clone(),
                self.retained.get(&workspace)?.read(cx).terminal.clone(),
            ))
        }) else {
            return;
        };
        let task = cx
            .background_executor()
            .spawn(async move { shelldeck_core::git::get_git_status(&root) });
        cx.spawn(async move |this, cx| {
            let git = task.await;
            let _ = this.update(cx, |this, cx| {
                let running = terminal
                    .read(cx)
                    .tabs
                    .iter()
                    .any(|tab| matches!(tab.state, SessionState::Running));
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64;
                let card = WorkspaceCardState {
                    branch: git.as_ref().and_then(|status| status.branch.clone()),
                    dirty: git.map_or_else(GitDirtyState::default, |status| GitDirtyState {
                        staged: status.staged,
                        modified: status.modified,
                        untracked: status.untracked,
                        conflicted: 0,
                    }),
                    agent: if running {
                        WorkspaceAgentState::Running
                    } else {
                        WorkspaceAgentState::Idle
                    },
                    unread: 0,
                    attention: 0,
                    freshness: WorkspaceFreshness::Fresh,
                    source_revision: now,
                    observed_at_millis: now,
                };
                if let Err(error) = this.navigation.reduce(
                    &this.catalog,
                    WorkspaceNavigationAction::UpdateCard {
                        id: workspace,
                        card,
                    },
                ) {
                    this.error = Some(error.to_string());
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn set_intake(&mut self, intake: LauncherIntakeKind, cx: &mut Context<Self>) {
        self.launcher.intake = intake;
        self.error = None;
        cx.notify();
    }

    fn sync_launcher(&mut self, cx: &Context<Self>) {
        self.launcher.name = self.name_state.read(cx).content().trim().to_owned();
        self.launcher.provider = self.provider_state.read(cx).content().trim().to_owned();
        self.launcher.repository = self.repository_state.read(cx).content().trim().to_owned();
        self.launcher.key = self.key_state.read(cx).content().trim().to_owned();
        self.launcher.title = self.title_state.read(cx).content().trim().to_owned();
        self.launcher.url = self.url_state.read(cx).content().trim().to_owned();
    }

    fn submit_onboarding(&mut self, cx: &mut Context<Self>) {
        let project_name = self.onboarding_project.read(cx).content().trim().to_owned();
        let checkout_name = self
            .onboarding_checkout
            .read(cx)
            .content()
            .trim()
            .to_owned();
        let repository = self
            .onboarding_repository
            .read(cx)
            .content()
            .trim()
            .to_owned();
        let root = self.onboarding_root.read(cx).content().trim().to_owned();
        if [
            project_name.as_str(),
            checkout_name.as_str(),
            repository.as_str(),
            root.as_str(),
        ]
        .iter()
        .any(|value| value.is_empty())
        {
            self.error = Some(t!("workspaces.onboarding.required").to_string());
            cx.notify();
            return;
        }
        let host = if self.onboarding_ssh {
            let Some(connection_id) = self
                .onboarding_connection
                .filter(|id| self.connections.contains_key(id))
            else {
                self.error = Some(t!("workspaces.onboarding.connection_required").to_string());
                cx.notify();
                return;
            };
            match RemotePosixPath::new(root) {
                Ok(root) => CheckoutHost::Ssh {
                    connection_id,
                    root,
                },
                Err(error) => {
                    self.error = Some(error.to_string());
                    cx.notify();
                    return;
                }
            }
        } else {
            let entered = PathBuf::from(root);
            let canonical = match std::fs::canonicalize(&entered) {
                Ok(path) if path.is_dir() => path,
                _ => {
                    self.error = Some(t!("workspaces.launcher.folder_unavailable").to_string());
                    cx.notify();
                    return;
                }
            };
            CheckoutHost::Local {
                device_label: t!("workspaces.onboarding.this_device").to_string(),
                root: canonical,
            }
        };
        let checkout_id = CatalogCheckoutId::new();
        let checkout = ProjectCheckout::new(
            checkout_id,
            checkout_name,
            host,
            RepositoryIdentity {
                slug: repository,
                canonical_url: None,
            },
        );
        let existing_project = self
            .catalog
            .projects()
            .find(|project| project.name() == project_name)
            .map(ProjectRecord::id);
        let result = mutate_and_save(&mut self.catalog, |catalog| {
            if let Some(project_id) = existing_project {
                catalog
                    .add_checkout(project_id, checkout)
                    .map_err(|error| error.to_string())
            } else {
                let mut project = ProjectRecord::new(CatalogProjectId::new(), project_name);
                project.add_checkout(checkout);
                catalog
                    .insert_project(project)
                    .map_err(|error| error.to_string())
            }
        });
        match result {
            Ok(()) => {
                self.launcher.checkout = Some(checkout_id);
                self.onboarding_open = false;
                self.error = None;
            }
            Err(error) => self.error = Some(error),
        }
        cx.notify();
    }

    fn submit_launcher(&mut self, cx: &mut Context<Self>) {
        self.sync_launcher(cx);
        let Some(checkout_id) = self.launcher.checkout else {
            self.error = Some(t!("workspaces.launcher.error.checkout_required").to_string());
            cx.notify();
            return;
        };
        if self.launcher.name.is_empty() {
            self.error = Some(t!("workspaces.launcher.error.name_required").to_string());
            cx.notify();
            return;
        }
        let Some(project_id) = self
            .catalog
            .projects()
            .find(|project| {
                project
                    .checkouts()
                    .any(|checkout| checkout.id() == checkout_id)
            })
            .map(ProjectRecord::id)
        else {
            self.error = Some(t!("workspaces.launcher.error.checkout_required").to_string());
            cx.notify();
            return;
        };
        let intake = match self.launcher.launch_intake() {
            Ok(intake) => intake,
            Err(error) => {
                self.error = Some(error);
                cx.notify();
                return;
            }
        };
        let host = match self
            .catalog
            .checkout_in_project(project_id, checkout_id)
            .map(ProjectCheckout::host)
        {
            Ok(CheckoutHost::Local { root, .. }) => match std::fs::canonicalize(root) {
                Ok(canonical_root) if canonical_root.is_dir() => {
                    AuthorizedLaunchHost::Local { canonical_root }
                }
                _ => {
                    self.error = Some(t!("workspaces.launcher.folder_unavailable").to_string());
                    cx.notify();
                    return;
                }
            },
            Ok(CheckoutHost::Ssh {
                connection_id,
                root,
            }) => AuthorizedLaunchHost::Ssh {
                connection_id: *connection_id,
                remote_root: root.as_str().to_owned(),
            },
            Err(error) => {
                self.error = Some(error.to_string());
                cx.notify();
                return;
            }
        };
        let workspace = CatalogWorkspaceId::new();
        let launch_name = self.launcher.name.clone();
        let launch_intake = intake.clone();
        if let Err(error) = mutate_and_save(&mut self.catalog, |catalog| {
            catalog
                .create_workspace(WorkspaceLaunchRequest {
                    id: workspace,
                    project_id,
                    checkout_id,
                    name: launch_name.clone(),
                    intake: launch_intake.clone(),
                })
                .map(|_| ())
                .map_err(|error| error.to_string())
        }) {
            self.error = Some(error.to_string());
            cx.notify();
            return;
        }
        let operation = CreationOperationId::new();
        if let Err(error) = self.creation.reduce(
            self.catalog.revision(),
            WorkspaceCreateEvent::Start {
                workspace,
                operation,
            },
        ) {
            self.error = Some(error.to_string());
            cx.notify();
            return;
        }
        let surface = WorkspaceSurfaceState::default();
        if let Err(error) = self.navigation.reduce(
            &self.catalog,
            WorkspaceNavigationAction::Retain {
                id: workspace,
                surface: surface.clone(),
                card: WorkspaceCardState::default(),
            },
        ) {
            self.error = Some(error.to_string());
            cx.notify();
            return;
        }
        let terminal = self
            .unclaimed_terminal
            .take()
            .unwrap_or_else(|| cx.new(TerminalView::new));
        self.retained.insert(
            workspace,
            cx.new(move |_| RetainedWorkspaceSurface::new(workspace, terminal)),
        );
        let request = WorkspaceExecutionRequest {
            workspace,
            checkout: checkout_id,
            operation,
            catalog_revision: self.catalog.revision(),
            name: self.launcher.name.clone(),
            intake,
            host,
        };
        self.pending_requests.insert(workspace, request.clone());
        self.launcher_open = false;
        self.switch_to(workspace, cx);

        let executor = self.executor.clone();
        let (event_tx, mut event_rx) = mpsc::unbounded_channel();
        let task = cx
            .background_executor()
            .spawn(executor.launch(request, event_tx));
        cx.spawn(async move |this, cx| {
            while let Some(event) = event_rx.recv().await {
                let _ = this.update(cx, |this, cx| {
                    this.apply_executor_event(workspace, event, cx)
                });
            }
            let result = task.await;
            if let Err(failure) = result {
                let _ = this.update(cx, |this, cx| {
                    if let Some(request) = this.pending_requests.get(&workspace) {
                        this.apply_executor_event(
                            workspace,
                            WorkspaceCreateEvent::Failed {
                                workspace,
                                operation: request.operation,
                                failure,
                            },
                            cx,
                        );
                    }
                });
            }
        })
        .detach();
    }

    fn apply_executor_event(
        &mut self,
        workspace: CatalogWorkspaceId,
        event: WorkspaceCreateEvent,
        cx: &mut Context<Self>,
    ) {
        let Some(request) = self.pending_requests.get(&workspace).cloned() else {
            return;
        };
        if let Err(error) = self.creation.reduce(request.catalog_revision, event) {
            self.error = Some(error.to_string());
            cx.notify();
            return;
        }
        if matches!(
            self.creation.state(workspace),
            Some(BackgroundWorkspaceCreateState::Completed { .. })
        ) {
            if let AuthorizedLaunchHost::Local { canonical_root } = &request.host {
                if let Some(surface) = self.retained.get(&workspace) {
                    let terminal = surface.read(cx).terminal.clone();
                    let result = terminal.update(cx, |terminal, cx| {
                        terminal.spawn_local_terminal_at(canonical_root, cx)
                    });
                    if let Err(error) = result {
                        self.error = Some(error);
                    }
                }
            }
            self.pending_requests.remove(&workspace);
            self.observe_local_card(workspace, cx);
        }
        cx.notify();
    }

    fn request_cancel(&mut self, workspace: CatalogWorkspaceId, cx: &mut Context<Self>) {
        let Some(request) = self.pending_requests.get(&workspace).cloned() else {
            return;
        };
        if let Err(error) = self.creation.reduce(
            request.catalog_revision,
            WorkspaceCreateEvent::RequestCancel {
                workspace,
                operation: request.operation,
            },
        ) {
            self.error = Some(error.to_string());
            cx.notify();
            return;
        }
        let executor = self.executor.clone();
        let task = cx
            .background_executor()
            .spawn(executor.cancel(request.clone()));
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                let event = match result {
                    Ok(event) => event,
                    Err(failure) => WorkspaceCreateEvent::Failed {
                        workspace,
                        operation: request.operation,
                        failure,
                    },
                };
                this.apply_executor_event(workspace, event, cx);
            });
        })
        .detach();
        cx.notify();
    }

    fn retry_create(&mut self, workspace: CatalogWorkspaceId, cx: &mut Context<Self>) {
        let Some(prior) = self.pending_requests.get(&workspace).cloned() else {
            return;
        };
        let catalog_revision = match self.creation.state(workspace) {
            Some(BackgroundWorkspaceCreateState::Conflict {
                conflict: WorkspaceCreateConflict::CatalogRevisionChanged { actual, .. },
                ..
            }) => *actual,
            Some(
                BackgroundWorkspaceCreateState::Cancelled {
                    catalog_revision, ..
                }
                | BackgroundWorkspaceCreateState::Conflict {
                    catalog_revision, ..
                }
                | BackgroundWorkspaceCreateState::Failed {
                    catalog_revision, ..
                },
            ) => *catalog_revision,
            _ => return,
        };
        let operation = CreationOperationId::new();
        if let Err(error) = self.creation.reduce(
            catalog_revision,
            WorkspaceCreateEvent::Retry {
                workspace,
                prior_operation: prior.operation,
                operation,
            },
        ) {
            self.error = Some(error.to_string());
            cx.notify();
            return;
        }
        let request = WorkspaceExecutionRequest {
            operation,
            catalog_revision,
            ..prior
        };
        self.pending_requests.insert(workspace, request.clone());
        let executor = self.executor.clone();
        let (event_tx, mut event_rx) = mpsc::unbounded_channel();
        let task = cx
            .background_executor()
            .spawn(executor.launch(request, event_tx));
        cx.spawn(async move |this, cx| {
            while let Some(event) = event_rx.recv().await {
                let _ = this.update(cx, |this, cx| {
                    this.apply_executor_event(workspace, event, cx)
                });
            }
            let result = task.await;
            if let Err(failure) = result {
                let _ = this.update(cx, |this, cx| {
                    this.apply_executor_event(
                        workspace,
                        WorkspaceCreateEvent::Failed {
                            workspace,
                            operation,
                            failure,
                        },
                        cx,
                    )
                });
            }
        })
        .detach();
        cx.notify();
    }

    fn archive_or_resume(&mut self, workspace: CatalogWorkspaceId, cx: &mut Context<Self>) {
        let archived = self
            .catalog
            .workspace(workspace)
            .is_ok_and(|item| item.lifecycle() == UserWorkspaceLifecycle::Archived);
        let result = mutate_and_save(&mut self.catalog, |catalog| {
            if archived {
                catalog.resume_workspace(workspace)
            } else {
                catalog.archive_workspace(workspace)
            }
            .map_err(|error| error.to_string())
        });
        if let Err(error) = result {
            self.error = Some(error.to_string());
        } else {
            self.navigation.reconcile_catalog(&self.catalog);
            if archived {
                self.switch_to(workspace, cx);
            }
        }
        cx.notify();
    }

    fn render_header(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let entity = cx.entity();
        let has_checkout = self
            .catalog
            .projects()
            .any(|project| project.checkouts().len() > 0);
        div()
            .flex()
            .items_center()
            .justify_between()
            .px(px(18.0))
            .py(px(12.0))
            .border_b_1()
            .border_color(ShellDeckColors::border())
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(3.0))
                    .child(
                        div()
                            .text_size(px(16.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(ShellDeckColors::text_primary())
                            .child(t!("workspaces.title").to_string()),
                    )
                    .child(
                        div()
                            .text_size(px(11.0))
                            .text_color(ShellDeckColors::text_muted())
                            .child(t!("workspaces.subtitle").to_string()),
                    ),
            )
            .child(
                div()
                    .flex()
                    .gap(px(6.0))
                    .child(
                        Button::new(
                            "workspace-onboarding-open",
                            t!("workspaces.onboarding.add").to_string(),
                        )
                        .size(ButtonSize::Sm)
                        .variant(ButtonVariant::Outline)
                        .on_click({
                            let entity = entity.clone();
                            move |_, _, cx| {
                                entity.update(cx, |this, cx| {
                                    this.onboarding_open = true;
                                    this.error = None;
                                    cx.notify();
                                });
                            }
                        }),
                    )
                    .child(
                        Button::new("workspace-launcher-open", t!("workspaces.new").to_string())
                            .size(ButtonSize::Sm)
                            .variant(ButtonVariant::Default)
                            .disabled(!has_checkout)
                            .on_click(move |_, _, cx| {
                                entity.update(cx, |this, cx| {
                                    this.launcher_open = true;
                                    this.error = None;
                                    cx.notify();
                                });
                            }),
                    ),
            )
    }

    fn render_catalog(&self, _cx: &mut Context<Self>) -> impl IntoElement {
        let mut body = div().flex().flex_col().gap(px(8.0)).p(px(10.0));
        if self.catalog.projects().len() == 0 {
            body = body.child(
                Card::new().content(
                    div()
                        .flex()
                        .flex_col()
                        .items_center()
                        .gap(px(7.0))
                        .p(px(16.0))
                        .text_size(px(11.0))
                        .text_color(ShellDeckColors::text_muted())
                        .child(lucide_icon(
                            "folder-git-2",
                            22.0,
                            ShellDeckColors::text_muted(),
                        ))
                        .child(t!("workspaces.catalog.empty").to_string()),
                ),
            );
        }
        for project in self.catalog.projects() {
            let mut project_view = div().flex().flex_col().gap(px(5.0)).child(
                div()
                    .px(px(4.0))
                    .text_size(px(11.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(ShellDeckColors::text_primary())
                    .child(project.name().to_owned()),
            );
            for checkout in project.checkouts() {
                let item = checkout_presentation(project, checkout, &self.connections);
                project_view = project_view.child(
                    Card::new().content(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(4.0))
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .justify_between()
                                    .child(
                                        div()
                                            .text_size(px(11.0))
                                            .font_weight(FontWeight::MEDIUM)
                                            .text_color(ShellDeckColors::text_primary())
                                            .child(item.checkout),
                                    )
                                    .child(
                                        Badge::new(t!(item.host_kind).to_string())
                                            .variant(BadgeVariant::Outline),
                                    ),
                            )
                            .child(
                                div()
                                    .text_size(px(10.0))
                                    .text_color(ShellDeckColors::text_muted())
                                    .child(format!("{} · {}", item.host, item.repository)),
                            ),
                    ),
                );
            }
            body = body.child(project_view);
        }
        scrollable_vertical(body)
    }

    fn render_onboarding(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let entity = cx.entity();
        let local_entity = entity.clone();
        let ssh_entity = entity.clone();
        let submit_entity = entity.clone();
        let close_entity = entity.clone();
        let mut connections = div().flex().flex_wrap().gap(px(5.0));
        if self.onboarding_ssh {
            for (id, label) in &self.connections {
                let id = *id;
                let selected = self.onboarding_connection == Some(id);
                let select_entity = entity.clone();
                connections = connections.child(
                    Button::new(("workspace-host", id.as_u128() as u64), label.clone())
                        .size(ButtonSize::Sm)
                        .variant(if selected {
                            ButtonVariant::Secondary
                        } else {
                            ButtonVariant::Outline
                        })
                        .on_click(move |_, _, cx| {
                            select_entity.update(cx, |this, cx| {
                                this.onboarding_connection = Some(id);
                                cx.notify();
                            });
                        }),
                );
            }
        }
        Card::new().content(
            div()
                .flex()
                .flex_col()
                .gap(px(8.0))
                .child(t!("workspaces.onboarding.title").to_string())
                .child(
                    Input::new(&self.onboarding_project)
                        .placeholder(t!("workspaces.onboarding.project").to_string()),
                )
                .child(
                    Input::new(&self.onboarding_checkout)
                        .placeholder(t!("workspaces.onboarding.checkout").to_string()),
                )
                .child(
                    Input::new(&self.onboarding_repository)
                        .placeholder(t!("workspaces.onboarding.repository").to_string()),
                )
                .child(
                    div()
                        .flex()
                        .gap(px(5.0))
                        .child(
                            Button::new(
                                "onboard-local",
                                t!("workspaces.authority.local").to_string(),
                            )
                            .size(ButtonSize::Sm)
                            .variant(if self.onboarding_ssh {
                                ButtonVariant::Outline
                            } else {
                                ButtonVariant::Secondary
                            })
                            .on_click(move |_, _, cx| {
                                local_entity.update(cx, |this, cx| {
                                    this.onboarding_ssh = false;
                                    cx.notify();
                                })
                            }),
                        )
                        .child(
                            Button::new("onboard-ssh", t!("workspaces.authority.ssh").to_string())
                                .size(ButtonSize::Sm)
                                .variant(if self.onboarding_ssh {
                                    ButtonVariant::Secondary
                                } else {
                                    ButtonVariant::Outline
                                })
                                .disabled(self.connections.is_empty())
                                .on_click(move |_, _, cx| {
                                    ssh_entity.update(cx, |this, cx| {
                                        this.onboarding_ssh = true;
                                        cx.notify();
                                    })
                                }),
                        ),
                )
                .child(connections)
                .child(
                    Input::new(&self.onboarding_root).placeholder(if self.onboarding_ssh {
                        t!("workspaces.onboarding.remote_root").to_string()
                    } else {
                        t!("workspaces.onboarding.local_root").to_string()
                    }),
                )
                .child(
                    div()
                        .flex()
                        .justify_end()
                        .gap(px(5.0))
                        .child(
                            Button::new(
                                "onboard-close",
                                t!("workspaces.launcher.close").to_string(),
                            )
                            .size(ButtonSize::Sm)
                            .variant(ButtonVariant::Ghost)
                            .on_click(move |_, _, cx| {
                                close_entity.update(cx, |this, cx| {
                                    this.onboarding_open = false;
                                    cx.notify();
                                })
                            }),
                        )
                        .child(
                            Button::new(
                                "onboard-save",
                                t!("workspaces.onboarding.save").to_string(),
                            )
                            .size(ButtonSize::Sm)
                            .variant(ButtonVariant::Default)
                            .on_click(move |_, _, cx| {
                                submit_entity.update(cx, |this, cx| this.submit_onboarding(cx))
                            }),
                        ),
                ),
        )
    }

    fn render_workspace_card(
        &self,
        presentation: WorkspaceCardPresentation,
        active: bool,
        creation: Option<&BackgroundWorkspaceCreateState>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let entity = cx.entity();
        let id = presentation.id;
        let select_entity = entity.clone();
        let archive_entity = entity.clone();
        let mut badges = div().flex().items_center().flex_wrap().gap(px(5.0));
        badges = badges
            .child(
                Badge::new(format!(
                    "{} · {}",
                    t!("workspaces.authority.terminal_filesystem"),
                    t!(presentation.host_kind)
                ))
                .variant(BadgeVariant::Outline),
            )
            .child(
                Badge::new(if presentation.provider_bound {
                    t!("workspaces.authority.provider_only").to_string()
                } else {
                    t!("workspaces.authority.provider_unbound").to_string()
                })
                .variant(BadgeVariant::Secondary),
            );
        if presentation.unread > 0 {
            badges = badges.child(
                Badge::new(t!("workspaces.card.unread", count = presentation.unread).to_string())
                    .variant(BadgeVariant::Secondary),
            );
        }
        if presentation.attention > 0 {
            badges = badges.child(
                Badge::new(
                    t!("workspaces.card.attention", count = presentation.attention).to_string(),
                )
                .variant(BadgeVariant::Destructive),
            );
        }
        let mut content = div()
            .flex()
            .flex_col()
            .gap(px(7.0))
            .child(
                div()
                    .flex()
                    .items_start()
                    .justify_between()
                    .gap(px(8.0))
                    .child(
                        div()
                            .min_w_0()
                            .child(
                                div()
                                    .truncate()
                                    .text_size(px(12.0))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(ShellDeckColors::text_primary())
                                    .child(presentation.name.clone()),
                            )
                            .child(
                                div()
                                    .truncate()
                                    .text_size(px(10.0))
                                    .text_color(ShellDeckColors::text_muted())
                                    .child(format!(
                                        "{} · {} · {} · {}",
                                        presentation.project,
                                        presentation.host,
                                        presentation.repository,
                                        presentation.checkout
                                    )),
                            ),
                    )
                    .child(
                        Badge::new(if presentation.observed {
                            freshness_label(presentation.freshness)
                        } else {
                            t!("workspaces.freshness.unknown").to_string()
                        })
                        .variant(if presentation.observed {
                            freshness_variant(presentation.freshness)
                        } else {
                            BadgeVariant::Outline
                        }),
                    ),
            )
            .child(badges);
        let dirty = presentation.dirty;
        content = content.child(
            div()
                .flex()
                .items_center()
                .gap(px(8.0))
                .text_size(px(10.0))
                .text_color(ShellDeckColors::text_muted())
                .child(
                    presentation
                        .branch
                        .clone()
                        .unwrap_or_else(|| t!("workspaces.card.branch_unknown").to_string()),
                )
                .child(if presentation.observed {
                    t!(
                        "workspaces.card.dirty",
                        staged = dirty.staged,
                        modified = dirty.modified,
                        untracked = dirty.untracked,
                        conflicted = dirty.conflicted
                    )
                    .to_string()
                } else {
                    t!("workspaces.card.awaiting_observation").to_string()
                })
                .child(agent_label(presentation.agent)),
        );
        if let Some(external) = presentation.external {
            content = content.child(authority_row(
                "circle-dot",
                t!("workspaces.card.external").to_string(),
                external,
            ));
        }
        if let Some(orchestration) = presentation.orchestration {
            content = content.child(authority_row(
                "bot",
                t!("workspaces.card.orchestration").to_string(),
                orchestration,
            ));
        }
        if let Some(state) = creation {
            content = content.child(render_creation_state(state));
        }
        content = content.child(
            div()
                .flex()
                .items_center()
                .gap(px(6.0))
                .child(
                    Button::new(
                        ("workspace-open", id.as_uuid().as_u128() as u64),
                        if active {
                            t!("workspaces.card.active").to_string()
                        } else {
                            t!("workspaces.card.open").to_string()
                        },
                    )
                    .size(ButtonSize::Sm)
                    .variant(if active {
                        ButtonVariant::Secondary
                    } else {
                        ButtonVariant::Default
                    })
                    .disabled(presentation.archived)
                    .on_click(move |_, _, cx| {
                        select_entity.update(cx, |this, cx| this.switch_to(id, cx));
                    }),
                )
                .child(
                    Button::new(
                        ("workspace-lifecycle", id.as_uuid().as_u128() as u64),
                        if presentation.archived {
                            t!("workspaces.card.resume").to_string()
                        } else {
                            t!("workspaces.card.archive").to_string()
                        },
                    )
                    .size(ButtonSize::Sm)
                    .variant(ButtonVariant::Ghost)
                    .on_click(move |_, _, cx| {
                        archive_entity.update(cx, |this, cx| {
                            this.archive_or_resume(id, cx);
                        });
                    }),
                ),
        );
        if matches!(
            creation,
            Some(
                BackgroundWorkspaceCreateState::Running { .. }
                    | BackgroundWorkspaceCreateState::Cancelling { .. }
            )
        ) {
            let cancel_entity = entity.clone();
            content = content.child(
                Button::new(
                    ("workspace-cancel", id.as_uuid().as_u128() as u64),
                    t!("workspaces.create.cancel").to_string(),
                )
                .size(ButtonSize::Sm)
                .variant(ButtonVariant::Destructive)
                .on_click(move |_, _, cx| {
                    cancel_entity.update(cx, |this, cx| this.request_cancel(id, cx));
                }),
            );
        }
        if creation.is_some_and(creation_retryable) {
            let retry_entity = entity;
            content = content.child(
                Button::new(
                    ("workspace-retry", id.as_uuid().as_u128() as u64),
                    t!("workspaces.create.retry").to_string(),
                )
                .size(ButtonSize::Sm)
                .variant(ButtonVariant::Outline)
                .on_click(move |_, _, cx| {
                    retry_entity.update(cx, |this, cx| this.retry_create(id, cx));
                }),
            );
        }
        Card::new()
            .content(content)
            .border_color(if active {
                ShellDeckColors::primary()
            } else {
                ShellDeckColors::border()
            })
            .into_any_element()
    }

    fn render_launcher(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let entity = cx.entity();
        let mut intake_row = div().flex().items_center().flex_wrap().gap(px(6.0));
        for (intake_index, intake) in [
            LauncherIntakeKind::Manual,
            LauncherIntakeKind::Issue,
            LauncherIntakeKind::PullRequest,
            LauncherIntakeKind::Task,
        ]
        .into_iter()
        .enumerate()
        {
            let target = entity.clone();
            intake_row = intake_row.child(
                Button::new(("workspace-intake", intake_index), intake.label())
                    .size(ButtonSize::Sm)
                    .variant(if self.launcher.intake == intake {
                        ButtonVariant::Default
                    } else {
                        ButtonVariant::Outline
                    })
                    .on_click(move |_, _, cx| {
                        target.update(cx, |this, cx| this.set_intake(intake, cx));
                    }),
            );
        }
        let mut form = div()
            .flex()
            .flex_col()
            .gap(px(9.0))
            .child(intake_row)
            .child(self.checkout_select.clone())
            .child(
                Input::new(&self.name_state)
                    .size(InputSize::Sm)
                    .variant(InputVariant::Outline)
                    .placeholder(t!("workspaces.launcher.name").to_string()),
            );
        if self.launcher.intake != LauncherIntakeKind::Manual {
            form = form
                .child(
                    div()
                        .flex()
                        .gap(px(8.0))
                        .child(
                            Input::new(&self.provider_state)
                                .size(InputSize::Sm)
                                .placeholder(t!("workspaces.launcher.provider").to_string()),
                        )
                        .child(
                            Input::new(&self.repository_state)
                                .size(InputSize::Sm)
                                .placeholder(t!("workspaces.launcher.repository").to_string()),
                        ),
                )
                .child(
                    Input::new(&self.key_state)
                        .size(InputSize::Sm)
                        .placeholder(t!("workspaces.launcher.key").to_string()),
                )
                .child(
                    Input::new(&self.title_state)
                        .size(InputSize::Sm)
                        .placeholder(t!("workspaces.launcher.external_title").to_string()),
                )
                .child(
                    Input::new(&self.url_state)
                        .size(InputSize::Sm)
                        .placeholder(t!("workspaces.launcher.url").to_string()),
                );
        }
        let close_entity = entity.clone();
        let submit_entity = entity;
        Card::new().content(
            div()
                .flex()
                .flex_col()
                .gap(px(10.0))
                .child(
                    div()
                        .text_size(px(12.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(ShellDeckColors::text_primary())
                        .child(t!("workspaces.launcher.title").to_string()),
                )
                .child(form)
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_end()
                        .gap(px(6.0))
                        .child(
                            Button::new(
                                "workspace-launcher-close",
                                t!("workspaces.launcher.close").to_string(),
                            )
                            .size(ButtonSize::Sm)
                            .variant(ButtonVariant::Ghost)
                            .on_click(move |_, _, cx| {
                                close_entity.update(cx, |this, cx| {
                                    this.launcher_open = false;
                                    cx.notify();
                                });
                            }),
                        )
                        .child(
                            Button::new(
                                "workspace-launcher-submit",
                                t!("workspaces.launcher.create").to_string(),
                            )
                            .size(ButtonSize::Sm)
                            .variant(ButtonVariant::Default)
                            .on_click(move |_, _, cx| {
                                submit_entity.update(cx, |this, cx| this.submit_launcher(cx));
                            }),
                        ),
                )
                .child(
                    div()
                        .text_size(px(10.0))
                        .text_color(ShellDeckColors::text_muted())
                        .child(t!("workspaces.launcher.effect_notice").to_string()),
                ),
        )
    }
}

impl Render for WorkspaceHubView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut workspace_cards = div().flex().flex_col().gap(px(8.0));
        let default_card = WorkspaceCardState::default();
        for workspace in self.catalog.workspaces() {
            let card = self
                .navigation
                .workspace(workspace.id())
                .map(|retained| &retained.card)
                .unwrap_or(&default_card);
            if let Some(presentation) =
                workspace_card_presentation(&self.catalog, workspace, card, &self.connections)
            {
                workspace_cards = workspace_cards.child(self.render_workspace_card(
                    presentation,
                    self.navigation.active() == Some(workspace.id()),
                    self.creation.state(workspace.id()),
                    cx,
                ));
            }
        }
        if self.catalog.workspaces().len() == 0 {
            workspace_cards = workspace_cards.child(
                div()
                    .p(px(16.0))
                    .text_size(px(11.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(t!("workspaces.cards.empty").to_string()),
            );
        }

        let mut center = div().flex().flex_col().min_w_0().flex_1();
        let mut top = div().flex().flex_col().gap(px(8.0)).p(px(12.0));
        if let Some(error) = self.load_error.as_ref().or(self.error.as_ref()) {
            top = top.child(
                Alert::new()
                    .variant(AlertVariant::Error)
                    .description(error.clone()),
            );
        }
        if self.launcher_open {
            top = top.child(self.render_launcher(cx));
        }
        if self.onboarding_open {
            top = top.child(self.render_onboarding(cx));
        }
        top = top.child(workspace_cards);
        center = center.child(scrollable_vertical(top));

        let active_surface = self
            .navigation
            .active()
            .and_then(|id| self.retained.get(&id).cloned());
        let surface = div()
            .flex()
            .flex_col()
            .min_w(px(280.0))
            .flex_1()
            .border_l_1()
            .border_color(ShellDeckColors::border())
            .child(
                div()
                    .px(px(14.0))
                    .py(px(9.0))
                    .border_b_1()
                    .border_color(ShellDeckColors::border())
                    .text_size(px(11.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(ShellDeckColors::text_primary())
                    .child(t!("workspaces.surface.title").to_string()),
            )
            .children(active_surface)
            .when(self.navigation.active().is_none(), |view| {
                view.child(
                    div()
                        .flex()
                        .flex_col()
                        .items_center()
                        .justify_center()
                        .gap(px(8.0))
                        .flex_1()
                        .text_size(px(11.0))
                        .text_color(ShellDeckColors::text_muted())
                        .child(lucide_icon(
                            "mouse-pointer-2",
                            22.0,
                            ShellDeckColors::text_muted(),
                        ))
                        .child(t!("workspaces.surface.select").to_string()),
                )
            });

        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(ShellDeckColors::bg_primary())
            .child(self.render_header(cx))
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h(px(0.0))
                    .child(
                        div()
                            .w(px(250.0))
                            .flex_shrink_0()
                            .border_r_1()
                            .border_color(ShellDeckColors::border())
                            .child(self.render_catalog(cx)),
                    )
                    .child(center)
                    .child(surface),
            )
    }
}

fn checkout_options(
    catalog: &ProjectCatalog,
    connections: &HashMap<Uuid, String>,
) -> Vec<(CatalogCheckoutId, String)> {
    catalog
        .projects()
        .flat_map(|project| {
            project.checkouts().map(move |checkout| {
                let item = checkout_presentation(project, checkout, connections);
                (
                    checkout.id(),
                    format!("{} · {} · {}", item.project, item.host, item.checkout),
                )
            })
        })
        .collect()
}

fn authority_row(icon: &str, label: String, detail: String) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .gap(px(6.0))
        .text_size(px(10.0))
        .child(lucide_icon(icon, 12.0, ShellDeckColors::text_muted()))
        .child(
            div()
                .text_color(ShellDeckColors::text_muted())
                .child(format!("{label} · {detail}")),
        )
}

fn agent_label(agent: WorkspaceAgentState) -> String {
    match agent {
        WorkspaceAgentState::Idle => t!("workspaces.agent.idle"),
        WorkspaceAgentState::Running => t!("workspaces.agent.running"),
        WorkspaceAgentState::WaitingForInput => t!("workspaces.agent.waiting"),
        WorkspaceAgentState::Failed => t!("workspaces.agent.failed"),
        WorkspaceAgentState::Completed => t!("workspaces.agent.completed"),
    }
    .to_string()
}

fn freshness_label(freshness: WorkspaceFreshness) -> String {
    match freshness {
        WorkspaceFreshness::Fresh => t!("workspaces.freshness.fresh"),
        WorkspaceFreshness::Aging => t!("workspaces.freshness.aging"),
        WorkspaceFreshness::Stale => t!("workspaces.freshness.stale"),
        WorkspaceFreshness::Offline => t!("workspaces.freshness.offline"),
    }
    .to_string()
}

fn freshness_variant(freshness: WorkspaceFreshness) -> BadgeVariant {
    match freshness {
        WorkspaceFreshness::Fresh => BadgeVariant::Default,
        WorkspaceFreshness::Aging => BadgeVariant::Secondary,
        WorkspaceFreshness::Stale => BadgeVariant::Warning,
        WorkspaceFreshness::Offline => BadgeVariant::Outline,
    }
}

fn phase_label(phase: WorkspaceCreatePhase) -> String {
    match phase {
        WorkspaceCreatePhase::Queued => t!("workspaces.create.queued"),
        WorkspaceCreatePhase::ResolvingHost => t!("workspaces.create.resolving_host"),
        WorkspaceCreatePhase::PreparingCheckout => t!("workspaces.create.preparing_checkout"),
        WorkspaceCreatePhase::CreatingWorkspace => t!("workspaces.create.creating"),
        WorkspaceCreatePhase::BindingRuntime => t!("workspaces.create.binding_runtime"),
    }
    .to_string()
}

fn render_creation_state(state: &BackgroundWorkspaceCreateState) -> impl IntoElement {
    let (label, detail) = match state {
        BackgroundWorkspaceCreateState::Running { progress, .. }
        | BackgroundWorkspaceCreateState::Cancelling { progress, .. } => (
            phase_label(progress.phase),
            format!(
                "{} / {} · {}",
                progress.completed_steps, progress.total_steps, progress.detail
            ),
        ),
        BackgroundWorkspaceCreateState::Cancelled { .. } => {
            (t!("workspaces.create.cancelled").to_string(), String::new())
        }
        BackgroundWorkspaceCreateState::Conflict { conflict, .. } => (
            t!("workspaces.create.conflict").to_string(),
            create_conflict_label(conflict),
        ),
        BackgroundWorkspaceCreateState::Failed { failure, .. } => (
            t!("workspaces.create.failed").to_string(),
            failure.message.clone(),
        ),
        BackgroundWorkspaceCreateState::Completed { .. } => {
            (t!("workspaces.create.completed").to_string(), String::new())
        }
    };
    div()
        .flex()
        .items_center()
        .gap(px(6.0))
        .text_size(px(10.0))
        .text_color(ShellDeckColors::text_muted())
        .child(lucide_icon(
            "loader-circle",
            12.0,
            ShellDeckColors::primary(),
        ))
        .child(if detail.is_empty() {
            label
        } else {
            format!("{label} · {detail}")
        })
}

fn creation_retryable(state: &BackgroundWorkspaceCreateState) -> bool {
    matches!(
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
    )
}

fn create_conflict_label(conflict: &WorkspaceCreateConflict) -> String {
    match conflict {
        WorkspaceCreateConflict::CheckoutAlreadyExists { .. } => {
            t!("workspaces.create.conflict_checkout").to_string()
        }
        WorkspaceCreateConflict::WorktreeLocked { .. } => {
            t!("workspaces.create.conflict_worktree").to_string()
        }
        WorkspaceCreateConflict::BranchAlreadyCheckedOut { branch } => {
            t!("workspaces.create.conflict_branch", branch = branch).to_string()
        }
        WorkspaceCreateConflict::HostUnavailable => {
            t!("workspaces.create.conflict_host").to_string()
        }
        WorkspaceCreateConflict::CatalogRevisionChanged { expected, actual } => t!(
            "workspaces.create.conflict_catalog",
            expected = expected,
            actual = actual
        )
        .to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        mutate_and_persist, workspace_card_presentation, AuthorizedLaunchHost,
        ExistingFolderWorkspaceExecutor, LauncherIntakeKind, WorkspaceExecutionRequest,
        WorkspaceHubView, WorkspaceLaunchExecutor, WorkspaceLauncherDraft,
    };
    use crate::terminal_view::TerminalView;
    use gpui::{AppContext, TestAppContext};
    use shelldeck_core::config::workspace_catalog::{
        CatalogCheckoutId, CatalogProjectId, CatalogWorkspaceId, CheckoutHost, ExternalWorkItem,
        ExternalWorkItemKind, PlatformContextRef, PlatformMappingReconciliation, PlatformV2Mapping,
        ProjectCatalog, ProjectCheckout, ProjectRecord, RepositoryIdentity, WorkspaceLaunchIntake,
        WorkspaceLaunchRequest,
    };
    use shelldeck_core::workspace_navigation::{
        CreationOperationId, GitDirtyState, WorkspaceAgentState, WorkspaceCardState,
        WorkspaceCreateEvent, WorkspaceCreatePhase, WorkspaceFreshness,
    };
    use std::collections::HashMap;
    use tokio::sync::mpsc;
    use uuid::Uuid;

    fn fixture_catalog() -> (
        ProjectCatalog,
        CatalogWorkspaceId,
        CatalogWorkspaceId,
        CatalogCheckoutId,
        CatalogCheckoutId,
        Uuid,
    ) {
        let project_id = CatalogProjectId::from_uuid(Uuid::from_u128(1));
        let checkout_id = CatalogCheckoutId::from_uuid(Uuid::from_u128(2));
        let ssh_checkout_id = CatalogCheckoutId::from_uuid(Uuid::from_u128(5));
        let ssh_connection = Uuid::from_u128(50);
        let workspace_a = CatalogWorkspaceId::from_uuid(Uuid::from_u128(3));
        let workspace_b = CatalogWorkspaceId::from_uuid(Uuid::from_u128(4));
        let mut project = ProjectRecord::new(project_id, "ShellDeck");
        project.add_checkout(ProjectCheckout::new(
            checkout_id,
            "principal",
            CheckoutHost::Local {
                device_label: "Machine locale".into(),
                root: std::env::current_dir().unwrap().join("fixture-repo"),
            },
            RepositoryIdentity {
                slug: "inklura/shelldeck".into(),
                canonical_url: None,
            },
        ));
        project.add_checkout(ProjectCheckout::new(
            ssh_checkout_id,
            "distant",
            CheckoutHost::Ssh {
                connection_id: ssh_connection,
                root: shelldeck_core::config::workspace_catalog::RemotePosixPath::new(
                    "/srv/shelldeck",
                )
                .unwrap(),
            },
            RepositoryIdentity {
                slug: "inklura/shelldeck".into(),
                canonical_url: None,
            },
        ));
        let mut catalog = ProjectCatalog::default();
        catalog.insert_project(project).unwrap();
        for (id, name, checkout_id) in [
            (workspace_a, "A", checkout_id),
            (workspace_b, "B", ssh_checkout_id),
        ] {
            catalog
                .create_workspace(WorkspaceLaunchRequest {
                    id,
                    project_id,
                    checkout_id,
                    name: name.into(),
                    intake: WorkspaceLaunchIntake::Manual,
                })
                .unwrap();
        }
        (
            catalog,
            workspace_a,
            workspace_b,
            checkout_id,
            ssh_checkout_id,
            ssh_connection,
        )
    }

    // SDTEST-1736 — SDUC-490 — YELLOW: native terminal identity is proven;
    // editor/files/browser ownership remains explicitly unsupported.
    #[test]
    fn keyed_gpui_workspace_entity_retention_preserves_hidden_terminal_state() {
        let mut cx = TestAppContext::single();
        let (catalog, workspace_a, workspace_b, _checkout, _ssh_checkout, _ssh_connection) =
            fixture_catalog();
        let initial_terminal = cx.update(|cx| cx.new(TerminalView::new));
        let native_terminal_before = initial_terminal.entity_id();
        let hub = cx.update(|cx| {
            cx.new(|cx| WorkspaceHubView::new(Ok(catalog), &[], initial_terminal.clone(), cx))
        });
        let workspace_entity_before = hub.read_with(&cx, |hub, _| {
            hub.retained.get(&workspace_a).unwrap().entity_id()
        });

        hub.update(&mut cx, |hub, cx| {
            hub.switch_to(workspace_a, cx);
            hub.switch_to(workspace_b, cx);
            hub.switch_to(workspace_a, cx);
        });

        hub.read_with(&cx, |hub, cx| {
            let surface = hub.retained.get(&workspace_a).unwrap();
            assert_eq!(surface.entity_id(), workspace_entity_before);
            assert_eq!(
                surface.read(cx).terminal.entity_id(),
                native_terminal_before
            );
            assert!(surface.read(cx).native_snapshot.is_some());
        });
    }

    // SDTEST-1738 — SDUC-489, SDUC-490 — YELLOW: local git/terminal
    // observations use UpdateCard; a live provider observation feed is pending.
    #[test]
    fn workspace_card_keeps_external_and_provider_authorities_distinct() {
        let (mut catalog, workspace_a, _, checkout, _, _) = fixture_catalog();
        let project = catalog.workspace(workspace_a).unwrap().project_id();
        let exact_mapping = PlatformV2Mapping {
            reconciliation_revision: 1,
            project: PlatformContextRef {
                id: "platform-project".into(),
                revision: 1,
            },
            checkout: PlatformContextRef {
                id: "platform-checkout".into(),
                revision: 1,
            },
            user_workspace: PlatformContextRef {
                id: "platform-workspace-a".into(),
                revision: 1,
            },
            reconciliation: PlatformMappingReconciliation::Exact {
                reconciled_at_millis: 1,
            },
        };
        catalog
            .set_platform_mapping(workspace_a, None, exact_mapping)
            .unwrap();
        catalog
            .bind_orchestration_run(
                workspace_a,
                Some(
                    shelldeck_core::config::workspace_catalog::OrchestrationRunRef {
                        runtime: "Automonique".into(),
                        run_id: "run-1".into(),
                        session_id: Some("session-1".into()),
                        platform_user_workspace_id: "platform-workspace-a".into(),
                    },
                ),
            )
            .unwrap();
        let mut task_catalog = ProjectCatalog::default();
        let source_project = catalog
            .projects()
            .find(|item| item.id() == project)
            .unwrap()
            .clone();
        task_catalog.insert_project(source_project).unwrap();
        let task_workspace = CatalogWorkspaceId::from_uuid(Uuid::from_u128(30));
        task_catalog
            .create_workspace(WorkspaceLaunchRequest {
                id: task_workspace,
                project_id: project,
                checkout_id: checkout,
                name: "Issue 127".into(),
                intake: WorkspaceLaunchIntake::Prefilled(ExternalWorkItem {
                    provider: "GitHub".into(),
                    repository: "inklura/shelldeck".into(),
                    kind: ExternalWorkItemKind::Issue,
                    key: "#127".into(),
                    title: Some("Workspace navigation".into()),
                    url: None,
                }),
            })
            .unwrap();

        let card = WorkspaceCardState {
            branch: Some("fix/workspace-navigation-ui-127".into()),
            dirty: GitDirtyState {
                staged: 1,
                modified: 2,
                untracked: 3,
                conflicted: 0,
            },
            agent: WorkspaceAgentState::WaitingForInput,
            unread: 4,
            attention: 2,
            freshness: WorkspaceFreshness::Aging,
            source_revision: 9,
            observed_at_millis: 100,
        };
        let external = workspace_card_presentation(
            &task_catalog,
            task_catalog.workspace(task_workspace).unwrap(),
            &card,
            &HashMap::new(),
        )
        .unwrap();
        assert_eq!(
            external.external.as_deref(),
            Some("Issue #127 · inklura/shelldeck")
        );
        assert_eq!(external.orchestration, None);
        assert!(!external.provider_bound);
        assert_eq!(
            external.branch.as_deref(),
            Some("fix/workspace-navigation-ui-127")
        );
        assert_eq!(external.dirty.modified, 2);
        assert_eq!(external.agent, WorkspaceAgentState::WaitingForInput);
        assert_eq!((external.unread, external.attention), (4, 2));

        let provider = workspace_card_presentation(
            &catalog,
            catalog.workspace(workspace_a).unwrap(),
            &card,
            &HashMap::new(),
        )
        .unwrap();
        assert_eq!(provider.external, None);
        assert_eq!(
            provider.orchestration.as_deref(),
            Some("Automonique · run-1")
        );
        assert!(provider.provider_bound);
    }

    #[test]
    fn launcher_prefills_all_external_kinds_through_one_validated_model() {
        for (intake, expected) in [
            (LauncherIntakeKind::Issue, ExternalWorkItemKind::Issue),
            (
                LauncherIntakeKind::PullRequest,
                ExternalWorkItemKind::PullRequest,
            ),
            (LauncherIntakeKind::Task, ExternalWorkItemKind::Task),
        ] {
            let draft = WorkspaceLauncherDraft {
                intake,
                provider: "GitHub".into(),
                repository: "inklura/shelldeck".into(),
                key: "#127".into(),
                title: "Navigation".into(),
                ..WorkspaceLauncherDraft::default()
            };
            let WorkspaceLaunchIntake::Prefilled(item) = draft.launch_intake().unwrap() else {
                panic!("external intake must stay prefilled");
            };
            assert_eq!(item.kind, expected);
            assert_eq!(item.key, "#127");
        }
    }

    #[test]
    fn catalog_save_failure_rolls_back_the_real_mutation() {
        let (mut catalog, workspace, ..) = fixture_catalog();
        let before = catalog.clone();
        let temp = tempfile::tempdir().unwrap();
        let parent_file = temp.path().join("not-a-directory");
        std::fs::write(&parent_file, b"occupied").unwrap();
        let invalid_target = parent_file.join("catalog.json");
        let result = mutate_and_persist(
            &mut catalog,
            |catalog| {
                catalog
                    .archive_workspace(workspace)
                    .map_err(|error| error.to_string())
            },
            |catalog| {
                catalog
                    .save_to(&invalid_target)
                    .map_err(|error| error.to_string())
            },
        );
        assert!(result.is_err());
        assert_eq!(catalog, before);
    }

    #[test]
    fn explicit_catalog_store_round_trips_an_existing_folder_checkout() {
        let temp = tempfile::tempdir().unwrap();
        let catalog_path = temp.path().join("catalog.json");
        let mut catalog = ProjectCatalog::default();
        let project_id = CatalogProjectId::new();
        let checkout_id = CatalogCheckoutId::new();
        catalog
            .insert_project(ProjectRecord::new(project_id, "ShellDeck"))
            .unwrap();
        catalog
            .add_checkout(
                project_id,
                ProjectCheckout::new(
                    checkout_id,
                    "Issue 127",
                    CheckoutHost::Local {
                        device_label: "Test".into(),
                        root: temp.path().to_path_buf(),
                    },
                    RepositoryIdentity {
                        slug: "inklura/shelldeck".into(),
                        canonical_url: None,
                    },
                ),
            )
            .unwrap();
        catalog.save_to(&catalog_path).unwrap();
        let loaded = ProjectCatalog::load_from(&catalog_path).unwrap();
        assert_eq!(loaded, catalog);
        assert!(loaded.checkout_in_project(project_id, checkout_id).is_ok());
    }

    #[tokio::test]
    async fn existing_folder_executor_streams_intermediate_progress() {
        let temp = tempfile::tempdir().unwrap();
        let workspace = CatalogWorkspaceId::new();
        let operation = CreationOperationId::new();
        let request = WorkspaceExecutionRequest {
            workspace,
            checkout: CatalogCheckoutId::new(),
            operation,
            catalog_revision: 7,
            name: "Attach".into(),
            intake: WorkspaceLaunchIntake::Manual,
            host: AuthorizedLaunchHost::Local {
                canonical_root: temp.path().to_path_buf(),
            },
        };
        let (tx, mut rx) = mpsc::unbounded_channel();
        ExistingFolderWorkspaceExecutor
            .launch(request, tx)
            .await
            .unwrap();
        let mut phases = Vec::new();
        while let Some(event) = rx.recv().await {
            match event {
                WorkspaceCreateEvent::Progress { progress, .. } => phases.push(progress.phase),
                WorkspaceCreateEvent::Completed { workspace: id, .. } => {
                    assert_eq!(id, workspace);
                }
                other => panic!("unexpected event: {other:?}"),
            }
        }
        assert_eq!(
            phases,
            vec![
                WorkspaceCreatePhase::ResolvingHost,
                WorkspaceCreatePhase::PreparingCheckout,
                WorkspaceCreatePhase::CreatingWorkspace,
                WorkspaceCreatePhase::BindingRuntime,
            ]
        );
    }
}
