//! Catalogue de travail retenu et lanceur de workspaces.
//!
//! Ce module ne possède aucune exécution git/SSH. Il projette uniquement le
//! catalogue local autorisé, conserve les entités GPUI par workspace et remet
//! les demandes de création à une frontière d'exécuteur injectée.

use super::*;
use adabraka_ui::components::input::InputVariant;
use adabraka_ui::display::card::Card;
use adabraka_ui::prelude::{Alert, AlertVariant};
use shelldeck_core::config::platform_review::PlatformReviewTarget;
use shelldeck_core::config::workspace_catalog::{
    CatalogCheckoutId, CatalogProjectId, CatalogWorkspaceId, CheckoutHost, ExternalWorkItem,
    ExternalWorkItemKind, ProjectCatalog, ProjectCheckout, ProjectRecord, RemotePosixPath,
    RepositoryIdentity, UserWorkspaceLifecycle, UserWorkspaceRecord, WorkspaceLaunchIntake,
    WorkspaceLaunchRequest,
};
use shelldeck_core::models::connection::Connection;
use shelldeck_core::workspace_navigation::{
    BackgroundWorkspaceCreateState, CreationOperationId, GitDirtyState, PaneId, PaneLeaf,
    TerminalAuthority, TerminalBinding, TerminalBindingId, TerminalSurface, TerminalViewport,
    WorkspaceAgentState, WorkspaceCardState, WorkspaceCreateConflict, WorkspaceCreateEvent,
    WorkspaceCreateFailure, WorkspaceCreateFailureKind, WorkspaceCreatePhase,
    WorkspaceCreationReducer, WorkspaceFreshness, WorkspaceNavigationAction,
    WorkspaceNavigationState, WorkspaceSurfaceState, WorkspaceTab, WorkspaceTabContent,
    WorkspaceTabId,
};
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::mpsc;

#[path = "native_lifecycle.rs"]
mod native_lifecycle;
use native_lifecycle::{
    AuthorizedLaunchHost, NativeWorkspaceExecutor, WorkspaceExecutionRequest,
    WorkspaceLaunchExecutor, WorkspaceLaunchMode,
};
#[cfg(test)]
use native_lifecycle::{GitWorktreeAdapter, NativeLaunchOutcome};

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
    mode: WorkspaceLaunchMode,
    intake: LauncherIntakeKind,
    checkout: Option<CatalogCheckoutId>,
    name: String,
    provider: String,
    repository: String,
    key: String,
    title: String,
    url: String,
    branch: String,
    start_point: String,
}

impl Default for WorkspaceLauncherDraft {
    fn default() -> Self {
        Self {
            mode: WorkspaceLaunchMode::ExistingFolder,
            intake: LauncherIntakeKind::Manual,
            checkout: None,
            name: String::new(),
            provider: String::new(),
            repository: String::new(),
            key: String::new(),
            title: String::new(),
            url: String::new(),
            branch: String::new(),
            start_point: "HEAD".into(),
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
    let value = match mutate(catalog) {
        Ok(value) => value,
        Err(error) => {
            *catalog = before;
            return Err(error);
        }
    };
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
    git_observed: bool,
    git_unavailable: bool,
    git_freshness: Option<WorkspaceFreshness>,
    provider_observed: bool,
    provider_freshness: Option<WorkspaceFreshness>,
    archived: bool,
    provider_bound: bool,
}

#[derive(Clone, Debug)]
struct GitCardSource {
    branch: Option<String>,
    staged: usize,
    modified: usize,
    untracked: usize,
    observed_at: u64,
}

#[derive(Clone, Debug)]
pub(super) struct ProviderCardObservation {
    pub agent: WorkspaceAgentState,
    pub unread: usize,
    pub attention: usize,
    pub freshness: WorkspaceFreshness,
    pub observed_at: u64,
}

#[derive(Clone, Debug, Default)]
struct WorkspaceCardSources {
    git: Option<GitCardSource>,
    git_unavailable: bool,
    provider: Option<ProviderCardObservation>,
    terminal_tabs: usize,
}

#[derive(Default)]
struct WorkspaceCardAggregator {
    sources: BTreeMap<CatalogWorkspaceId, WorkspaceCardSources>,
    revision: u64,
}

impl WorkspaceCardAggregator {
    fn observe_git(
        &mut self,
        workspace: CatalogWorkspaceId,
        status: Option<shelldeck_core::git::GitStatus>,
        observed_at: u64,
    ) {
        let source = self.sources.entry(workspace).or_default();
        source.git_unavailable = status.is_none();
        if let Some(status) = status {
            source.git = Some(GitCardSource {
                branch: status.branch,
                staged: status.staged,
                modified: status.modified,
                untracked: status.untracked,
                observed_at,
            });
        }
    }

    fn observe_terminal(&mut self, workspace: CatalogWorkspaceId, tabs: usize) {
        self.sources.entry(workspace).or_default().terminal_tabs = tabs;
    }

    #[allow(dead_code)]
    fn observe_provider(
        &mut self,
        workspace: CatalogWorkspaceId,
        observation: ProviderCardObservation,
    ) {
        self.sources.entry(workspace).or_default().provider = Some(observation);
    }

    fn aggregate(
        &mut self,
        workspace: CatalogWorkspaceId,
        previous: &WorkspaceCardState,
    ) -> WorkspaceCardState {
        self.revision = self.revision.saturating_add(1).max(1);
        let source = self.sources.entry(workspace).or_default();
        let mut card = previous.clone();
        if let Some(git) = &source.git {
            card.branch.clone_from(&git.branch);
            card.dirty.staged = git.staged;
            card.dirty.modified = git.modified;
            card.dirty.untracked = git.untracked;
            // `git status` does not currently report conflicts: preserve the
            // last authoritative value rather than inventing zero.
        }
        if let Some(provider) = &source.provider {
            card.agent = provider.agent;
            card.unread = provider.unread;
            card.attention = provider.attention;
            card.freshness = provider.freshness;
        } else if source.git_unavailable {
            card.freshness = WorkspaceFreshness::Stale;
        }
        card.source_revision = self.revision;
        card.observed_at_millis = match (&source.git, &source.provider) {
            (Some(git), Some(provider)) if !source.git_unavailable => {
                git.observed_at.min(provider.observed_at)
            }
            _ => 0,
        };
        card
    }
}

fn workspace_card_presentation(
    catalog: &ProjectCatalog,
    workspace: &UserWorkspaceRecord,
    card: &WorkspaceCardState,
    connections: &HashMap<Uuid, String>,
    sources: Option<&WorkspaceCardSources>,
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
        branch: sources
            .filter(|source| source.git.is_some() && !source.git_unavailable)
            .and_then(|_| card.branch.clone()),
        dirty: card.dirty,
        external,
        orchestration,
        agent: card.agent,
        unread: card.unread,
        attention: card.attention,
        git_observed: sources.is_some_and(|source| source.git.is_some() && !source.git_unavailable),
        git_unavailable: sources.is_some_and(|source| source.git_unavailable),
        git_freshness: sources.and_then(|source| {
            if source.git_unavailable {
                Some(WorkspaceFreshness::Stale)
            } else {
                source.git.as_ref().map(|_| WorkspaceFreshness::Fresh)
            }
        }),
        provider_observed: sources.is_some_and(|source| source.provider.is_some()),
        provider_freshness: sources
            .and_then(|source| source.provider.as_ref())
            .map(|provider| provider.freshness),
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

#[derive(Clone)]
pub(super) struct WorkspaceTerminalConfig {
    pub theme: TerminalTheme,
    pub font_size: f32,
    pub font_family: String,
    pub default_shell: Option<String>,
    pub cursor_style: String,
    pub cursor_blink: bool,
    pub scrollback_lines: usize,
    pub sidebar_width: f32,
    pub menu_bar_visible: bool,
}

pub(super) fn apply_terminal_config(
    terminal: &Entity<TerminalView>,
    config: &WorkspaceTerminalConfig,
    cx: &mut App,
) {
    terminal.update(cx, |terminal, _| {
        terminal.set_terminal_theme(&config.theme);
        terminal.set_font_size(config.font_size);
        terminal.set_font_family(config.font_family.clone());
        terminal.set_default_shell(config.default_shell.clone());
        terminal.set_cursor_style(&config.cursor_style);
        terminal.set_cursor_blink(config.cursor_blink);
        terminal.set_scrollback_lines(config.scrollback_lines);
        terminal.set_sidebar_width(config.sidebar_width);
        terminal.set_menu_bar_visible(config.menu_bar_visible);
    });
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
    cards: WorkspaceCardAggregator,
    creation: WorkspaceCreationReducer,
    retained: BTreeMap<CatalogWorkspaceId, Entity<RetainedWorkspaceSurface>>,
    unclaimed_terminal: Option<Entity<TerminalView>>,
    terminal_config: Option<WorkspaceTerminalConfig>,
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
    branch_state: Entity<InputState>,
    start_point_state: Entity<InputState>,
    checkout_select: Entity<Select<CatalogCheckoutId>>,
    error: Option<String>,
    load_error: Option<String>,
    recovery_pending: bool,
    pending_requests: BTreeMap<CatalogWorkspaceId, WorkspaceExecutionRequest>,
}

pub(super) enum WorkspaceHubEvent {
    ActiveTerminal(Entity<TerminalView>),
}

impl EventEmitter<WorkspaceHubEvent> for WorkspaceHubView {}

impl WorkspaceHubView {
    /// Resolve the active workspace through the catalog's exact reconciliation.
    /// Provider session bindings are deliberately not considered authority.
    pub(super) fn active_platform_review_target(&self) -> Option<PlatformReviewTarget> {
        let active = self.navigation.active()?;
        let mapping = self.catalog.workspace(active).ok()?.platform_mapping()?;
        PlatformReviewTarget::from_exact_mapping(mapping).ok()
    }

    pub(super) fn configure_terminals(
        &mut self,
        config: WorkspaceTerminalConfig,
        cx: &mut Context<Self>,
    ) {
        for surface in self.retained.values() {
            let terminal = surface.read(cx).terminal.clone();
            apply_terminal_config(&terminal, &config, cx);
        }
        if let Some(terminal) = self.unclaimed_terminal.as_ref() {
            apply_terminal_config(terminal, &config, cx);
        }
        self.terminal_config = Some(config);
    }

    pub(super) fn new(
        catalog: Result<ProjectCatalog, String>,
        connections: &[Connection],
        initial_terminal: Entity<TerminalView>,
        cx: &mut Context<Self>,
    ) -> Self {
        let labels = connections
            .iter()
            .map(|connection| (connection.id, connection.display_name().to_string()))
            .collect::<Vec<_>>();
        Self::new_with_executor(
            catalog,
            &labels,
            initial_terminal,
            Arc::new(NativeWorkspaceExecutor::with_connections(
                connections.to_vec(),
            )),
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
            if let Ok(checkout) =
                catalog.checkout_in_project(workspace.project_id(), workspace.checkout_id())
            {
                if let CheckoutHost::Local { root, .. } = checkout.host() {
                    if let Err(error) =
                        terminal.update(cx, |terminal, _| terminal.set_default_cwd(root))
                    {
                        tracing::warn!(%error, workspace = %workspace.id(), "workspace terminal cwd unavailable");
                    }
                }
            }
            retained.insert(
                workspace.id(),
                cx.new(move |_| RetainedWorkspaceSurface::new(workspace.id(), terminal)),
            );
        }
        let onboarding_open = catalog.projects().len() == 0;
        let onboarding_connection = connections.keys().next().copied();
        let retained_roots = catalog
            .projects()
            .flat_map(ProjectRecord::checkouts)
            .filter_map(|checkout| match checkout.host() {
                CheckoutHost::Local { root, .. } => Some(root.clone()),
                CheckoutHost::Ssh { .. } => None,
            })
            .collect();
        let recovery = cx
            .background_executor()
            .spawn(executor.clone().recover_orphans(retained_roots));
        cx.spawn(async move |this, cx| {
            let result = recovery.await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(()) => this.recovery_pending = false,
                    Err(failure) => {
                        this.error = Some(failure.message);
                        // Keep the gate closed: an unvalidated retained
                        // worktree must never become terminal authority.
                    }
                }
                cx.notify();
            });
        })
        .detach();
        Self {
            catalog,
            navigation,
            cards: WorkspaceCardAggregator::default(),
            creation: WorkspaceCreationReducer::default(),
            retained,
            unclaimed_terminal: initial_terminal,
            terminal_config: None,
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
            branch_state: cx.new(InputState::new),
            start_point_state: cx.new(InputState::new),
            checkout_select,
            error: None,
            load_error,
            recovery_pending: true,
            pending_requests: BTreeMap::new(),
        }
    }

    fn switch_to(&mut self, id: CatalogWorkspaceId, cx: &mut Context<Self>) {
        if self.recovery_pending {
            self.error = Some(t!("workspaces.launcher.folder_unavailable").to_string());
            cx.notify();
            return;
        }
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

    fn observe_local_card(&mut self, workspace: CatalogWorkspaceId, cx: &mut Context<Self>) {
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
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64;
                this.cards.observe_git(workspace, git, now);
                this.cards
                    .observe_terminal(workspace, terminal.read(cx).tab_count());
                let previous = this
                    .navigation
                    .workspace(workspace)
                    .map(|state| state.card.clone())
                    .unwrap_or_default();
                let card = this.cards.aggregate(workspace, &previous);
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

    /// Production ingress for the Fleet/Platform observer. Kept separate from
    /// Git polling so a missing provider refresh cannot erase prior evidence.
    #[allow(dead_code)]
    pub(super) fn observe_provider_card(
        &mut self,
        workspace: CatalogWorkspaceId,
        observation: ProviderCardObservation,
        cx: &mut Context<Self>,
    ) {
        self.cards.observe_provider(workspace, observation);
        let previous = self
            .navigation
            .workspace(workspace)
            .map(|state| state.card.clone())
            .unwrap_or_default();
        let card = self.cards.aggregate(workspace, &previous);
        if let Err(error) = self.navigation.reduce(
            &self.catalog,
            WorkspaceNavigationAction::UpdateCard {
                id: workspace,
                card,
            },
        ) {
            self.error = Some(error.to_string());
        }
        cx.notify();
    }

    fn set_intake(&mut self, intake: LauncherIntakeKind, cx: &mut Context<Self>) {
        self.launcher.intake = intake;
        self.error = None;
        cx.notify();
    }

    fn set_launch_mode(&mut self, mode: WorkspaceLaunchMode, cx: &mut Context<Self>) {
        self.launcher.mode = mode;
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
        self.launcher.branch = self.branch_state.read(cx).content().trim().to_owned();
        self.launcher.start_point = self.start_point_state.read(cx).content().trim().to_owned();
        if self.launcher.start_point.is_empty() {
            self.launcher.start_point = "HEAD".into();
        }
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
                self.refresh_checkout_select(Some(checkout_id), cx);
                self.onboarding_open = false;
                self.error = None;
            }
            Err(error) => self.error = Some(error),
        }
        cx.notify();
    }

    fn refresh_checkout_select(
        &mut self,
        selected: Option<CatalogCheckoutId>,
        cx: &mut Context<Self>,
    ) {
        let options = checkout_options(&self.catalog, &self.connections);
        let selected = selected
            .filter(|id| options.iter().any(|(candidate, _)| candidate == id))
            .or_else(|| options.first().map(|(id, _)| *id));
        let selected_index = selected.and_then(|selected| {
            options
                .iter()
                .position(|(candidate, _)| *candidate == selected)
        });
        let parent = cx.entity();
        self.checkout_select = cx.new(move |select_cx| {
            Select::new(select_cx)
                .options(
                    options
                        .into_iter()
                        .map(|(id, label)| SelectOption::new(id, label))
                        .collect(),
                )
                .selected_index(selected_index)
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
        });
        self.launcher.checkout = selected;
    }

    fn submit_launcher(&mut self, cx: &mut Context<Self>) {
        if self.recovery_pending {
            self.error = Some(t!("workspaces.launcher.folder_unavailable").to_string());
            cx.notify();
            return;
        }
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
        let selected_checkout = match self
            .catalog
            .checkout_in_project(project_id, checkout_id)
            .cloned()
        {
            Ok(checkout) => checkout,
            Err(error) => {
                self.error = Some(error.to_string());
                cx.notify();
                return;
            }
        };
        let mode = if matches!(selected_checkout.host(), CheckoutHost::Ssh { .. }) {
            WorkspaceLaunchMode::Ssh
        } else {
            self.launcher.mode
        };
        let workspace = CatalogWorkspaceId::new();
        let (host, workspace_checkout, created_checkout) = match (mode, selected_checkout.host()) {
            (WorkspaceLaunchMode::ExistingFolder, CheckoutHost::Local { root, .. }) => {
                match crate::terminal_view::AuthorizedLocalRoot::capture(root) {
                    Ok(authority) => (
                        AuthorizedLaunchHost::LocalExisting { authority },
                        checkout_id,
                        None,
                    ),
                    _ => {
                        self.error = Some(t!("workspaces.launcher.folder_unavailable").to_string());
                        cx.notify();
                        return;
                    }
                }
            }
            (WorkspaceLaunchMode::GitWorktree, CheckoutHost::Local { root, device_label }) => {
                if self.launcher.branch.is_empty() || self.launcher.start_point.is_empty() {
                    self.error =
                        Some(t!("workspaces.launcher.worktree_fields_required").to_string());
                    cx.notify();
                    return;
                }
                let source_authority =
                    match crate::terminal_view::AuthorizedLocalRoot::capture(root) {
                        Ok(source_authority) => source_authority,
                        _ => {
                            self.error =
                                Some(t!("workspaces.launcher.folder_unavailable").to_string());
                            cx.notify();
                            return;
                        }
                    };
                let owned_parent =
                    shelldeck_core::config::AppConfig::config_dir().join("workspace-checkouts");
                let target_root = owned_parent.join(workspace.to_string());
                let generated_checkout = CatalogCheckoutId::new();
                let checkout = ProjectCheckout::new(
                    generated_checkout,
                    self.launcher.name.clone(),
                    CheckoutHost::Local {
                        device_label: device_label.clone(),
                        root: target_root.clone(),
                    },
                    selected_checkout.repository().clone(),
                );
                (
                    AuthorizedLaunchHost::LocalWorktree {
                        source_authority,
                        target_root,
                        branch: self.launcher.branch.clone(),
                        start_point: self.launcher.start_point.clone(),
                    },
                    generated_checkout,
                    Some(checkout),
                )
            }
            (
                WorkspaceLaunchMode::Ssh,
                CheckoutHost::Ssh {
                    connection_id,
                    root,
                },
            ) => (
                AuthorizedLaunchHost::Ssh {
                    connection_id: *connection_id,
                    remote_root: root.as_str().to_string(),
                },
                checkout_id,
                None,
            ),
            _ => unreachable!("launch mode must match the selected checkout host"),
        };
        let operation = CreationOperationId::new();
        let catalog_revision = self.catalog.revision();
        if let Err(error) = self.creation.reduce(
            catalog_revision,
            WorkspaceCreateEvent::Start {
                workspace,
                operation,
            },
        ) {
            self.error = Some(error.to_string());
            cx.notify();
            return;
        }
        let request = WorkspaceExecutionRequest {
            workspace,
            project: project_id,
            source_checkout: checkout_id,
            checkout: workspace_checkout,
            created_checkout,
            operation,
            catalog_revision,
            name: self.launcher.name.clone(),
            intake,
            host,
            mode,
        };
        self.pending_requests.insert(workspace, request.clone());
        self.launcher_open = false;

        let executor = self.executor.clone();
        let (event_tx, mut event_rx) = mpsc::unbounded_channel();
        let task = cx
            .background_executor()
            .spawn(executor.launch(request.clone(), event_tx));
        cx.spawn(async move |this, cx| {
            while let Some(event) = event_rx.recv().await {
                let _ = this.update(cx, |this, cx| {
                    this.apply_executor_event(workspace, event, cx)
                });
            }
            let result = task.await;
            if let Err(failure) = result {
                let _ = this.update(cx, |this, cx| {
                    if this
                        .pending_requests
                        .get(&workspace)
                        .is_some_and(|pending| pending.operation == request.operation)
                    {
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
        let (event_workspace, event_operation) = create_event_coordinates(&event);
        if event_workspace != workspace
            || event_workspace != request.workspace
            || event_operation != request.operation
        {
            return;
        }
        let event = if let WorkspaceCreateEvent::Completed {
            workspace: event_workspace,
            operation: event_operation,
        } = event
        {
            let attachable = completion_can_attach(
                workspace,
                event_workspace,
                event_operation,
                &request,
                self.creation.state(event_workspace),
            );
            if !attachable {
                // Les événements retardés, prématurés ou reçus pendant une
                // annulation ne doivent jamais atteindre le PTY. Un reçu natif
                // reste toutefois une preuve d'effet et doit être compensé;
                // le reducer conserve son état canonique actuel.
                if let Some(receipt) = self.executor.take_receipt(event_operation) {
                    self.compensate_rejected_receipt(request, receipt, cx);
                }
                return;
            }
            if self.catalog.revision() != request.catalog_revision {
                let event = WorkspaceCreateEvent::Conflict {
                    workspace: event_workspace,
                    operation: event_operation,
                    conflict: WorkspaceCreateConflict::CatalogRevisionChanged {
                        expected: request.catalog_revision,
                        actual: self.catalog.revision(),
                    },
                };
                let Some(receipt) = self.executor.take_receipt(event_operation) else {
                    return;
                };
                self.compensate_then_apply(request, receipt, event, cx);
                return;
            } else {
                let Some(receipt) = self.executor.take_receipt(event_operation) else {
                    return;
                };
                let (authority, cleanup) = match receipt {
                    native_lifecycle::WorkspaceLaunchReceipt::Local { authority, cleanup } => {
                        (authority, cleanup)
                    }
                    native_lifecycle::WorkspaceLaunchReceipt::Ssh(prepared) => {
                        self.resume_ssh_then_complete(request, prepared, cx);
                        return;
                    }
                };
                let receipt =
                    native_lifecycle::WorkspaceLaunchReceipt::Local { authority, cleanup };
                let native_lifecycle::WorkspaceLaunchReceipt::Local { authority, .. } = &receipt
                else {
                    unreachable!()
                };
                if authority.revalidate().is_err() {
                    let event = WorkspaceCreateEvent::Failed {
                        workspace: event_workspace,
                        operation: event_operation,
                        failure: WorkspaceCreateFailure {
                            kind: WorkspaceCreateFailureKind::Authorization,
                            message: t!("workspaces.launcher.authority_changed").to_string(),
                            retryable: false,
                        },
                    };
                    self.compensate_then_apply(request, receipt, event, cx);
                    return;
                }
                let using_unclaimed_terminal = self.unclaimed_terminal.is_some();
                let terminal = self
                    .unclaimed_terminal
                    .clone()
                    .unwrap_or_else(|| cx.new(TerminalView::new));
                if let Some(config) = self.terminal_config.as_ref() {
                    apply_terminal_config(&terminal, config, cx);
                }
                let prepared = terminal.update(cx, |terminal, _cx| {
                    terminal.prepare_authorized_local_terminal(authority)
                });
                let prepared = match prepared {
                    Ok(prepared) => prepared,
                    Err(message) => {
                        let event = WorkspaceCreateEvent::Failed {
                            workspace: event_workspace,
                            operation: event_operation,
                            failure: WorkspaceCreateFailure {
                                kind: WorkspaceCreateFailureKind::Filesystem,
                                message,
                                retryable: true,
                            },
                        };
                        self.compensate_then_apply(request, receipt, event, cx);
                        return;
                    }
                };
                let created_checkout = request.created_checkout.clone();
                let launch_name = request.name.clone();
                let launch_intake = request.intake.clone();
                let commit = mutate_and_save(&mut self.catalog, |catalog| {
                    if let Some(checkout) = created_checkout {
                        catalog
                            .add_checkout(request.project, checkout)
                            .map_err(|error| error.to_string())?;
                    }
                    catalog
                        .create_workspace(WorkspaceLaunchRequest {
                            id: request.workspace,
                            project_id: request.project,
                            checkout_id: request.checkout,
                            name: launch_name,
                            intake: launch_intake,
                        })
                        .map(|_| ())
                        .map_err(|error| error.to_string())
                });
                if let Err(message) = commit {
                    drop(prepared);
                    let event = WorkspaceCreateEvent::Failed {
                        workspace: event_workspace,
                        operation: event_operation,
                        failure: WorkspaceCreateFailure {
                            kind: WorkspaceCreateFailureKind::Filesystem,
                            message,
                            retryable: true,
                        },
                    };
                    self.compensate_then_apply(request, receipt, event, cx);
                    return;
                }
                if using_unclaimed_terminal {
                    self.unclaimed_terminal.take();
                }
                terminal.update(cx, |terminal, cx| {
                    terminal.commit_prepared_local_terminal(prepared, cx);
                });
                let surface = WorkspaceSurfaceState::default();
                if let Err(error) = self.navigation.reduce(
                    &self.catalog,
                    WorkspaceNavigationAction::Retain {
                        id: workspace,
                        surface,
                        card: WorkspaceCardState::default(),
                    },
                ) {
                    self.error = Some(error.to_string());
                }
                self.retained.insert(
                    workspace,
                    cx.new(move |_| RetainedWorkspaceSurface::new(workspace, terminal)),
                );
                let executor = self.executor.clone();
                let acknowledge = cx
                    .background_executor()
                    .spawn(executor.acknowledge(request.clone()));
                cx.spawn(async move |_this, _cx| {
                    if let Err(error) = acknowledge.await {
                        tracing::warn!(message = %error.message, "workspace journal acknowledgement deferred to recovery");
                    }
                })
                .detach();
                WorkspaceCreateEvent::Completed {
                    workspace: event_workspace,
                    operation: event_operation,
                }
            }
        } else {
            event
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
            self.pending_requests.remove(&workspace);
            self.switch_to(workspace, cx);
            self.observe_local_card(workspace, cx);
        }
        cx.notify();
    }

    fn resume_ssh_then_complete(
        &mut self,
        request: WorkspaceExecutionRequest,
        prepared: native_lifecycle::PreparedSshWorkspace,
        cx: &mut Context<Self>,
    ) {
        let workspace = request.workspace;
        let operation = request.operation;
        let task = cx
            .background_executor()
            .spawn(native_lifecycle::prepare_ssh_terminal(prepared));
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                let still_attachable = this
                    .pending_requests
                    .get(&workspace)
                    .is_some_and(|pending| pending.operation == operation)
                    && completion_can_attach(
                        workspace,
                        workspace,
                        operation,
                        &request,
                        this.creation.state(workspace),
                    );
                if !still_attachable {
                    if let Ok(prepared) = &result {
                        prepared.shutdown();
                    }
                    drop(result);
                    return;
                }
                if this.catalog.revision() != request.catalog_revision {
                    if let Ok(prepared) = &result {
                        prepared.shutdown();
                    }
                    drop(result);
                    this.apply_executor_event(
                        workspace,
                        WorkspaceCreateEvent::Conflict {
                            workspace,
                            operation,
                            conflict: WorkspaceCreateConflict::CatalogRevisionChanged {
                                expected: request.catalog_revision,
                                actual: this.catalog.revision(),
                            },
                        },
                        cx,
                    );
                    return;
                }
                let prepared = match result {
                    Ok(prepared) => prepared,
                    Err(failure) => {
                        this.apply_executor_event(
                            workspace,
                            WorkspaceCreateEvent::Failed {
                                workspace,
                                operation,
                                failure,
                            },
                            cx,
                        );
                        return;
                    }
                };
                this.commit_prepared_ssh(request, prepared, cx);
            });
        })
        .detach();
    }

    fn commit_prepared_ssh(
        &mut self,
        request: WorkspaceExecutionRequest,
        prepared: native_lifecycle::PreparedSshTerminal,
        cx: &mut Context<Self>,
    ) {
        let workspace = request.workspace;
        let operation = request.operation;
        let using_unclaimed_terminal = self.unclaimed_terminal.is_some();
        let terminal = self
            .unclaimed_terminal
            .clone()
            .unwrap_or_else(|| cx.new(TerminalView::new));
        if let Some(config) = self.terminal_config.as_ref() {
            apply_terminal_config(&terminal, config, cx);
        }
        let created_checkout = request.created_checkout.clone();
        let launch_name = request.name.clone();
        let launch_intake = request.intake.clone();
        let commit = mutate_and_save(&mut self.catalog, |catalog| {
            if let Some(checkout) = created_checkout {
                catalog
                    .add_checkout(request.project, checkout)
                    .map_err(|error| error.to_string())?;
            }
            catalog
                .create_workspace(WorkspaceLaunchRequest {
                    id: workspace,
                    project_id: request.project,
                    checkout_id: request.checkout,
                    name: launch_name,
                    intake: launch_intake,
                })
                .map(|_| ())
                .map_err(|error| error.to_string())
        });
        if let Err(message) = commit {
            prepared.shutdown();
            drop(prepared);
            self.apply_executor_event(
                workspace,
                WorkspaceCreateEvent::Failed {
                    workspace,
                    operation,
                    failure: WorkspaceCreateFailure {
                        kind: WorkspaceCreateFailureKind::Filesystem,
                        message,
                        retryable: true,
                    },
                },
                cx,
            );
            return;
        }
        if using_unclaimed_terminal {
            self.unclaimed_terminal.take();
        }
        terminal.update(cx, |terminal, cx| {
            terminal.add_session_with_connection(prepared.session, Some(prepared.connection_id));
            terminal.ensure_refresh_running(cx);
            cx.notify();
        });
        let surface = WorkspaceSurfaceState::default();
        if let Err(error) = self.navigation.reduce(
            &self.catalog,
            WorkspaceNavigationAction::Retain {
                id: workspace,
                surface,
                card: WorkspaceCardState::default(),
            },
        ) {
            self.error = Some(error.to_string());
        }
        self.retained.insert(
            workspace,
            cx.new(move |_| RetainedWorkspaceSurface::new(workspace, terminal)),
        );
        if let Err(error) = self.creation.reduce(
            request.catalog_revision,
            WorkspaceCreateEvent::Completed {
                workspace,
                operation,
            },
        ) {
            self.error = Some(error.to_string());
            cx.notify();
            return;
        }
        self.pending_requests.remove(&workspace);
        self.switch_to(workspace, cx);
        cx.notify();
    }

    fn compensate_then_apply(
        &mut self,
        request: WorkspaceExecutionRequest,
        receipt: native_lifecycle::WorkspaceLaunchReceipt,
        event: WorkspaceCreateEvent,
        cx: &mut Context<Self>,
    ) {
        let workspace = request.workspace;
        let operation = request.operation;
        let revision = request.catalog_revision;
        let executor = self.executor.clone();
        let task = cx
            .background_executor()
            .spawn(executor.compensate(request, receipt));
        cx.spawn(async move |this, cx| {
            let cleanup = task.await;
            let _ = this.update(cx, |this, cx| {
                if this
                    .pending_requests
                    .get(&workspace)
                    .is_none_or(|pending| pending.operation != operation)
                {
                    return;
                }
                let event = match cleanup {
                    Ok(()) => event,
                    Err(failure) => WorkspaceCreateEvent::Failed {
                        workspace,
                        operation,
                        failure,
                    },
                };
                if let Err(error) = this.creation.reduce(revision, event) {
                    this.error = Some(error.to_string());
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn compensate_rejected_receipt(
        &mut self,
        request: WorkspaceExecutionRequest,
        receipt: native_lifecycle::WorkspaceLaunchReceipt,
        cx: &mut Context<Self>,
    ) {
        let executor = self.executor.clone();
        let task = cx
            .background_executor()
            .spawn(executor.compensate(request, receipt));
        cx.spawn(async move |this, cx| {
            if let Err(failure) = task.await {
                let _ = this.update(cx, |this, cx| {
                    this.error = Some(failure.message);
                    cx.notify();
                });
            }
        })
        .detach();
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
        if !request_matches_catalog(&self.catalog, &prior) {
            self.error = Some(t!("workspaces.launcher.authority_changed").to_string());
            cx.notify();
            return;
        }
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
        let was_active = self.navigation.active() == Some(workspace);
        if !archived
            && matches!(
                self.creation.state(workspace),
                Some(
                    BackgroundWorkspaceCreateState::Running { .. }
                        | BackgroundWorkspaceCreateState::Cancelling { .. }
                )
            )
        {
            self.error = Some(t!("workspaces.lifecycle.cancel_before_archive").to_string());
            cx.notify();
            return;
        }
        if archived {
            let root = self.catalog.workspace(workspace).ok().and_then(|record| {
                self.catalog
                    .checkout_in_project(record.project_id(), record.checkout_id())
                    .ok()
            });
            let Some(CheckoutHost::Local { root, .. }) = root.map(ProjectCheckout::host) else {
                self.error = Some(t!("workspaces.launcher.ssh_executor_unavailable").to_string());
                cx.notify();
                return;
            };
            let Ok(canonical) = std::fs::canonicalize(root) else {
                self.error = Some(t!("workspaces.launcher.folder_unavailable").to_string());
                cx.notify();
                return;
            };
            if canonical != *root || !canonical.is_dir() {
                self.error = Some(t!("workspaces.launcher.authority_changed").to_string());
                cx.notify();
                return;
            }
            let Ok(authority) = crate::terminal_view::AuthorizedLocalRoot::capture(&canonical)
            else {
                self.error = Some(t!("workspaces.launcher.authority_changed").to_string());
                cx.notify();
                return;
            };
            if let Some(surface) = self.retained.get(&workspace) {
                let terminal = surface.read(cx).terminal.clone();
                terminal.update(cx, |terminal, _| {
                    terminal.install_authorized_default_cwd(&authority)
                });
            }
        }
        if !archived && was_active {
            if let Some(surface) = self.retained.get(&workspace) {
                let state = {
                    let retained = surface.read(cx);
                    terminal_surface(&self.catalog, workspace, retained.terminal.read(cx))
                };
                surface.update(cx, |surface, cx| surface.capture(cx));
                if let Err(error) = self.navigation.reduce(
                    &self.catalog,
                    WorkspaceNavigationAction::UpdateSurface {
                        id: workspace,
                        surface: state,
                    },
                ) {
                    self.error = Some(error.to_string());
                    cx.notify();
                    return;
                }
            }
        }
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
            } else if was_active {
                let fallback = self
                    .unclaimed_terminal
                    .get_or_insert_with(|| cx.new(TerminalView::new))
                    .clone();
                if let Some(config) = self.terminal_config.as_ref() {
                    apply_terminal_config(&fallback, config, cx);
                }
                cx.emit(WorkspaceHubEvent::ActiveTerminal(fallback));
            }
        }
        cx.notify();
    }
}

fn request_matches_catalog(catalog: &ProjectCatalog, request: &WorkspaceExecutionRequest) -> bool {
    let Ok(checkout) = catalog.checkout_in_project(request.project, request.source_checkout) else {
        return false;
    };
    match (&request.host, checkout.host()) {
        (AuthorizedLaunchHost::LocalExisting { authority }, CheckoutHost::Local { root, .. }) => {
            root == authority.path() && authority.revalidate().is_ok()
        }
        (
            AuthorizedLaunchHost::LocalWorktree {
                source_authority, ..
            },
            CheckoutHost::Local { root, .. },
        ) => root == source_authority.path() && source_authority.revalidate().is_ok(),
        (
            AuthorizedLaunchHost::Ssh {
                connection_id,
                remote_root,
            },
            CheckoutHost::Ssh {
                connection_id: catalog_connection,
                root,
            },
        ) => connection_id == catalog_connection && remote_root == root.as_str(),
        _ => false,
    }
}

fn create_event_coordinates(
    event: &WorkspaceCreateEvent,
) -> (CatalogWorkspaceId, CreationOperationId) {
    match event {
        WorkspaceCreateEvent::Start {
            workspace,
            operation,
        }
        | WorkspaceCreateEvent::Progress {
            workspace,
            operation,
            ..
        }
        | WorkspaceCreateEvent::RequestCancel {
            workspace,
            operation,
        }
        | WorkspaceCreateEvent::Cancelled {
            workspace,
            operation,
        }
        | WorkspaceCreateEvent::Conflict {
            workspace,
            operation,
            ..
        }
        | WorkspaceCreateEvent::Failed {
            workspace,
            operation,
            ..
        }
        | WorkspaceCreateEvent::Completed {
            workspace,
            operation,
        } => (*workspace, *operation),
        WorkspaceCreateEvent::Retry {
            workspace,
            operation,
            ..
        } => (*workspace, *operation),
    }
}

/// Barrière pure exécutée avant tout accès au terminal ou au système de
/// fichiers. Les coordonnées du canal, de l'événement, de la requête et du
/// reducer doivent désigner exactement la même opération arrivée au dernier
/// jalon. Un événement rejeté ne peut donc pas être « corrigé » vers une
/// requête plus récente.
fn completion_can_attach(
    delivery_workspace: CatalogWorkspaceId,
    event_workspace: CatalogWorkspaceId,
    event_operation: CreationOperationId,
    request: &WorkspaceExecutionRequest,
    state: Option<&BackgroundWorkspaceCreateState>,
) -> bool {
    delivery_workspace == event_workspace
        && request.workspace == event_workspace
        && request.operation == event_operation
        && matches!(
            state,
            Some(BackgroundWorkspaceCreateState::Running {
                operation,
                catalog_revision,
                progress,
            }) if *operation == event_operation
                && *catalog_revision == request.catalog_revision
                && progress.phase == WorkspaceCreatePhase::BindingRuntime
                && progress.total_steps > 0
                && progress.completed_steps == progress.total_steps
        )
}

#[path = "workspaces_render.rs"]
mod hub_render;
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
        WorkspaceCreateConflict::BranchAlreadyExists { branch } => {
            t!("workspaces.create.conflict_branch_exists", branch = branch).to_string()
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
#[path = "workspaces_tests.rs"]
mod tests_file;
