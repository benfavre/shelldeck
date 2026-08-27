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
    CatalogCheckoutId, CatalogWorkspaceId, CheckoutHost, ExternalWorkItem, ExternalWorkItemKind,
    ProjectCatalog, ProjectCheckout, ProjectRecord, UserWorkspaceLifecycle, UserWorkspaceRecord,
    WorkspaceLaunchIntake, WorkspaceLaunchRequest,
};
use shelldeck_core::workspace_navigation::{
    BackgroundWorkspaceCreateState, CreationOperationId, GitDirtyState, PaneNode,
    TerminalBindingId, WorkspaceAgentState, WorkspaceCardState, WorkspaceCreateConflict,
    WorkspaceCreateEvent, WorkspaceCreateFailure, WorkspaceCreateFailureKind, WorkspaceCreatePhase,
    WorkspaceCreationReducer, WorkspaceFreshness, WorkspaceNavigationAction,
    WorkspaceNavigationState, WorkspaceSurfaceState, WorkspaceTabContent,
};
use std::collections::{BTreeMap, HashMap};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

type ExecutorFuture<T> = Pin<Box<dyn Future<Output = T> + Send + 'static>>;

/// Requête déjà validée, livrée à l'adaptateur local ou SSH futur.
#[derive(Clone, Debug)]
pub(super) struct WorkspaceExecutionRequest {
    pub workspace: CatalogWorkspaceId,
    pub checkout: CatalogCheckoutId,
    pub operation: CreationOperationId,
    pub catalog_revision: u64,
}

/// Frontière non bloquante du futur adaptateur d'effets.
///
/// L'implémentation de production reste volontairement indisponible dans ce
/// lot. Un adaptateur devra faire le travail hors du thread GPUI et retourner
/// des événements typés du réducteur, jamais des mutations directes de la vue.
pub(super) trait WorkspaceLaunchExecutor: Send + Sync {
    fn available(&self) -> bool;
    fn launch(
        &self,
        request: WorkspaceExecutionRequest,
    ) -> ExecutorFuture<Result<Vec<WorkspaceCreateEvent>, WorkspaceCreateFailure>>;
    fn cancel(
        &self,
        request: WorkspaceExecutionRequest,
    ) -> ExecutorFuture<Result<WorkspaceCreateEvent, WorkspaceCreateFailure>>;
}

#[derive(Default)]
struct UnavailableWorkspaceLaunchExecutor;

impl WorkspaceLaunchExecutor for UnavailableWorkspaceLaunchExecutor {
    fn available(&self) -> bool {
        false
    }

    fn launch(
        &self,
        request: WorkspaceExecutionRequest,
    ) -> ExecutorFuture<Result<Vec<WorkspaceCreateEvent>, WorkspaceCreateFailure>> {
        let _authorized_checkout = request.checkout;
        Box::pin(async {
            Err(WorkspaceCreateFailure {
                kind: WorkspaceCreateFailureKind::RuntimeUnavailable,
                message: t!("workspaces.launcher.executor_unavailable").to_string(),
                retryable: false,
            })
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

#[cfg(not(test))]
fn persist_catalog(catalog: &mut ProjectCatalog) -> Result<(), String> {
    catalog.save().map_err(|error| error.to_string())
}

#[cfg(test)]
fn persist_catalog(_catalog: &mut ProjectCatalog) -> Result<(), String> {
    Ok(())
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

/// Entité GPUI stable pour un terminal retenu. La vue terminal native reste la
/// même quand le workspace est masqué ; l'adaptateur d'effets futur y attachera
/// le PTY local ou SSH autorisé.
struct RetainedTerminalEntity {
    binding: TerminalBindingId,
    terminal_view: Entity<TerminalView>,
    draft: String,
    scrollback_offset_lines: usize,
    follow_output: bool,
}

impl Render for RetainedTerminalEntity {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id(("retained-terminal", self.binding.as_uuid().as_u128() as u64))
            .flex()
            .flex_col()
            .min_h(px(180.0))
            .child(self.terminal_view.clone())
    }
}

/// Surface complète conservée même lorsqu'elle n'est pas visible.
struct RetainedWorkspaceSurface {
    workspace: CatalogWorkspaceId,
    snapshot: WorkspaceSurfaceState,
    terminals: BTreeMap<TerminalBindingId, Entity<RetainedTerminalEntity>>,
}

impl RetainedWorkspaceSurface {
    fn new(
        workspace: CatalogWorkspaceId,
        snapshot: WorkspaceSurfaceState,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut this = Self {
            workspace,
            snapshot: WorkspaceSurfaceState::default(),
            terminals: BTreeMap::new(),
        };
        this.apply_snapshot(snapshot, cx);
        this
    }

    fn apply_snapshot(&mut self, snapshot: WorkspaceSurfaceState, cx: &mut Context<Self>) {
        let mut live = BTreeMap::new();
        for tab in surface_tabs(&snapshot) {
            if let WorkspaceTabContent::Terminal(terminal) = &tab.content {
                let id = terminal.binding.id;
                let entity = self.terminals.remove(&id).unwrap_or_else(|| {
                    cx.new(|terminal_cx| RetainedTerminalEntity {
                        binding: id,
                        terminal_view: terminal_cx.new(TerminalView::new),
                        draft: terminal.draft.clone(),
                        scrollback_offset_lines: terminal.viewport.scrollback_offset_lines,
                        follow_output: terminal.viewport.follow_output,
                    })
                });
                entity.update(cx, |retained, _| {
                    retained.draft = terminal.draft.clone();
                    retained.scrollback_offset_lines = terminal.viewport.scrollback_offset_lines;
                    retained.follow_output = terminal.viewport.follow_output;
                });
                live.insert(id, entity);
            }
        }
        self.terminals = live;
        self.snapshot = snapshot;
        cx.notify();
    }

    #[cfg(test)]
    fn terminal_entity(&self, id: TerminalBindingId) -> Option<Entity<RetainedTerminalEntity>> {
        self.terminals.get(&id).cloned()
    }
}

impl Render for RetainedWorkspaceSurface {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut content = div()
            .id((
                "retained-workspace-surface",
                self.workspace.as_uuid().as_u128() as u64,
            ))
            .flex()
            .flex_col()
            .gap(px(8.0))
            .p(px(14.0));
        if let Some(root) = self.snapshot.root.as_ref() {
            content = content.child(render_pane_node(root, &self.terminals, cx));
        } else {
            content = content.child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap(px(8.0))
                    .h(px(180.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(lucide_icon(
                        "layout-dashboard",
                        24.0,
                        ShellDeckColors::text_muted(),
                    ))
                    .child(t!("workspaces.surface.empty").to_string()),
            );
        }
        content
    }
}

fn surface_tabs(
    surface: &WorkspaceSurfaceState,
) -> Vec<&shelldeck_core::workspace_navigation::WorkspaceTab> {
    fn collect<'a>(
        node: &'a PaneNode,
        tabs: &mut Vec<&'a shelldeck_core::workspace_navigation::WorkspaceTab>,
    ) {
        match node {
            PaneNode::Leaf(leaf) => tabs.extend(&leaf.tabs),
            PaneNode::Split { first, second, .. } => {
                collect(first, tabs);
                collect(second, tabs);
            }
        }
    }
    let mut tabs = Vec::new();
    if let Some(root) = surface.root.as_ref() {
        collect(root, &mut tabs);
    }
    tabs
}

fn render_pane_node(
    node: &PaneNode,
    terminals: &BTreeMap<TerminalBindingId, Entity<RetainedTerminalEntity>>,
    cx: &mut Context<RetainedWorkspaceSurface>,
) -> AnyElement {
    match node {
        PaneNode::Split {
            axis,
            first,
            second,
            ..
        } => {
            let mut split = div().flex().gap(px(8.0)).w_full();
            if matches!(
                axis,
                shelldeck_core::workspace_navigation::SplitAxis::Vertical
            ) {
                split = split.flex_col();
            }
            split
                .child(render_pane_node(first, terminals, cx))
                .child(render_pane_node(second, terminals, cx))
                .into_any_element()
        }
        PaneNode::Leaf(leaf) => {
            let mut pane = Card::new().content(
                div().flex().flex_col().gap(px(8.0)).child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.0))
                        .children(leaf.tabs.iter().map(|tab| {
                            let variant = if leaf.active_tab == Some(tab.id) {
                                BadgeVariant::Default
                            } else {
                                BadgeVariant::Outline
                            };
                            Badge::new(tab.title.clone()).variant(variant)
                        })),
                ),
            );
            if let Some(active) = leaf
                .active_tab
                .and_then(|id| leaf.tabs.iter().find(|tab| tab.id == id))
            {
                pane = pane.content(render_tab_content(active, terminals, cx));
            }
            pane.min_w(px(220.0)).flex_1().into_any_element()
        }
    }
}

fn render_tab_content(
    tab: &shelldeck_core::workspace_navigation::WorkspaceTab,
    terminals: &BTreeMap<TerminalBindingId, Entity<RetainedTerminalEntity>>,
    _cx: &mut Context<RetainedWorkspaceSurface>,
) -> AnyElement {
    match &tab.content {
        WorkspaceTabContent::Terminal(terminal) => terminals
            .get(&terminal.binding.id)
            .cloned()
            .map(IntoElement::into_any_element)
            .unwrap_or_else(|| div().into_any_element()),
        WorkspaceTabContent::Editor {
            relative_path,
            draft,
            cursor_line,
            cursor_column,
            ..
        } => div()
            .flex()
            .flex_col()
            .gap(px(5.0))
            .text_size(px(11.0))
            .text_color(ShellDeckColors::text_muted())
            .child(relative_path.as_str().to_owned())
            .child(
                t!(
                    "workspaces.surface.editor_state",
                    line = cursor_line + 1,
                    column = cursor_column + 1,
                    bytes = draft.len()
                )
                .to_string(),
            )
            .into_any_element(),
        WorkspaceTabContent::Files { relative_root, .. } => div()
            .text_size(px(11.0))
            .text_color(ShellDeckColors::text_muted())
            .child(t!("workspaces.surface.files", path = relative_root.as_str()).to_string())
            .into_any_element(),
        WorkspaceTabContent::Browser { location } => div()
            .text_size(px(11.0))
            .text_color(ShellDeckColors::text_muted())
            .child(location.clone())
            .into_any_element(),
        WorkspaceTabContent::ProviderSession(binding) => div()
            .flex()
            .items_center()
            .gap(px(6.0))
            .child(
                Badge::new(t!("workspaces.authority.provider_only").to_string())
                    .variant(BadgeVariant::Outline),
            )
            .child(
                div()
                    .text_size(px(11.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(binding.session_id.clone()),
            )
            .into_any_element(),
    }
}

pub(super) struct WorkspaceHubView {
    catalog: ProjectCatalog,
    navigation: WorkspaceNavigationState,
    creation: WorkspaceCreationReducer,
    retained: BTreeMap<CatalogWorkspaceId, Entity<RetainedWorkspaceSurface>>,
    connections: HashMap<Uuid, String>,
    executor: Arc<dyn WorkspaceLaunchExecutor>,
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

impl WorkspaceHubView {
    pub(super) fn new(
        catalog: Result<ProjectCatalog, String>,
        connections: &[(Uuid, String)],
        cx: &mut Context<Self>,
    ) -> Self {
        Self::new_with_executor(
            catalog,
            connections,
            Arc::new(UnavailableWorkspaceLaunchExecutor),
            cx,
        )
    }

    fn new_with_executor(
        catalog: Result<ProjectCatalog, String>,
        connections: &[(Uuid, String)],
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
            retained.insert(
                workspace.id(),
                cx.new(|cx| RetainedWorkspaceSurface::new(workspace.id(), surface, cx)),
            );
        }
        Self {
            catalog,
            navigation,
            creation: WorkspaceCreationReducer::default(),
            retained,
            connections,
            executor,
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
        match self
            .navigation
            .reduce(&self.catalog, WorkspaceNavigationAction::SwitchTo(id))
        {
            Ok(()) => self.error = None,
            Err(error) => self.error = Some(error.to_string()),
        }
        cx.notify();
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

    fn submit_launcher(&mut self, cx: &mut Context<Self>) {
        self.sync_launcher(cx);
        if !self.executor.available() {
            self.error = Some(t!("workspaces.launcher.executor_unavailable").to_string());
            cx.notify();
            return;
        }
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
        let Some(project) = self.catalog.projects().find(|project| {
            project
                .checkouts()
                .any(|checkout| checkout.id() == checkout_id)
        }) else {
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
        let workspace = CatalogWorkspaceId::new();
        if let Err(error) = self.catalog.create_workspace(WorkspaceLaunchRequest {
            id: workspace,
            project_id: project.id(),
            checkout_id,
            name: self.launcher.name.clone(),
            intake,
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
        self.retained.insert(
            workspace,
            cx.new(|cx| RetainedWorkspaceSurface::new(workspace, surface, cx)),
        );
        let request = WorkspaceExecutionRequest {
            workspace,
            checkout: checkout_id,
            operation,
            catalog_revision: self.catalog.revision(),
        };
        self.pending_requests.insert(workspace, request.clone());
        self.launcher_open = false;
        self.switch_to(workspace, cx);

        let executor = self.executor.clone();
        let task = cx.background_executor().spawn(executor.launch(request));
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.apply_executor_result(workspace, result, cx);
            });
        })
        .detach();
    }

    fn apply_executor_result(
        &mut self,
        workspace: CatalogWorkspaceId,
        result: Result<Vec<WorkspaceCreateEvent>, WorkspaceCreateFailure>,
        cx: &mut Context<Self>,
    ) {
        let Some(request) = self.pending_requests.get(&workspace).cloned() else {
            return;
        };
        let events = match result {
            Ok(events) => events,
            Err(failure) => vec![WorkspaceCreateEvent::Failed {
                workspace,
                operation: request.operation,
                failure,
            }],
        };
        for event in events {
            if let Err(error) = self.creation.reduce(request.catalog_revision, event) {
                self.error = Some(error.to_string());
                break;
            }
        }
        if matches!(
            self.creation.state(workspace),
            Some(BackgroundWorkspaceCreateState::Completed { .. })
        ) {
            if let Err(error) = persist_catalog(&mut self.catalog) {
                self.error = Some(error);
            }
            self.pending_requests.remove(&workspace);
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
                this.apply_executor_result(workspace, Ok(vec![event]), cx);
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
        let task = cx.background_executor().spawn(executor.launch(request));
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.apply_executor_result(workspace, result, cx);
            });
        })
        .detach();
        cx.notify();
    }

    fn archive_or_resume(&mut self, workspace: CatalogWorkspaceId, cx: &mut Context<Self>) {
        let before = self.catalog.clone();
        let archived = self
            .catalog
            .workspace(workspace)
            .is_ok_and(|item| item.lifecycle() == UserWorkspaceLifecycle::Archived);
        let result = if archived {
            self.catalog.resume_workspace(workspace)
        } else {
            self.catalog.archive_workspace(workspace)
        };
        if let Err(error) = result {
            self.error = Some(error.to_string());
        } else {
            if let Err(error) = persist_catalog(&mut self.catalog) {
                self.catalog = before;
                self.error = Some(error);
                cx.notify();
                return;
            }
            self.navigation.reconcile_catalog(&self.catalog);
            if archived {
                self.switch_to(workspace, cx);
            }
        }
        cx.notify();
    }

    #[cfg(test)]
    fn capture_surface(
        &mut self,
        workspace: CatalogWorkspaceId,
        surface: WorkspaceSurfaceState,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        self.navigation
            .reduce(
                &self.catalog,
                WorkspaceNavigationAction::UpdateSurface {
                    id: workspace,
                    surface: surface.clone(),
                },
            )
            .map_err(|error| error.to_string())?;
        self.retained
            .get(&workspace)
            .ok_or_else(|| "workspace entity missing".to_string())?
            .update(cx, |retained, cx| retained.apply_snapshot(surface, cx));
        Ok(())
    }

    fn render_header(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let entity = cx.entity();
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
                Button::new("workspace-launcher-open", t!("workspaces.new").to_string())
                    .size(ButtonSize::Sm)
                    .variant(ButtonVariant::Default)
                    .on_click(move |_, _, cx| {
                        entity.update(cx, |this, cx| {
                            this.launcher_open = true;
                            this.error = None;
                            cx.notify();
                        });
                    }),
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
                            .disabled(!self.executor.available())
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
        workspace_card_presentation, LauncherIntakeKind, WorkspaceHubView, WorkspaceLauncherDraft,
    };
    use gpui::{AppContext, TestAppContext};
    use shelldeck_core::config::workspace_catalog::{
        CatalogCheckoutId, CatalogProjectId, CatalogWorkspaceId, CheckoutHost, ExternalWorkItem,
        ExternalWorkItemKind, PlatformContextRef, PlatformMappingReconciliation, PlatformV2Mapping,
        ProjectCatalog, ProjectCheckout, ProjectRecord, RepositoryIdentity, WorkspaceLaunchIntake,
        WorkspaceLaunchRequest, WorkspaceRelativePath,
    };
    use shelldeck_core::workspace_navigation::{
        GitDirtyState, PaneId, PaneLeaf, PaneNode, SplitAxis, TerminalAuthority, TerminalBinding,
        TerminalBindingId, TerminalSurface, TerminalViewport, WorkspaceAgentState,
        WorkspaceCardState, WorkspaceFocus, WorkspaceFreshness, WorkspaceSurfaceState,
        WorkspaceTab, WorkspaceTabContent, WorkspaceTabId,
    };
    use std::collections::HashMap;
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

    fn terminal_surface(
        authority: TerminalAuthority,
        binding: TerminalBindingId,
        draft: &str,
        scrollback: usize,
        rich: bool,
    ) -> WorkspaceSurfaceState {
        let pane = PaneId::from_uuid(Uuid::from_u128(binding.as_uuid().as_u128() + 100));
        let tab = WorkspaceTabId::from_uuid(Uuid::from_u128(binding.as_uuid().as_u128() + 200));
        let checkout = authority_checkout(&authority);
        let terminal_tab = WorkspaceTab {
            id: tab,
            title: "Terminal".into(),
            content: WorkspaceTabContent::Terminal(TerminalSurface {
                binding: TerminalBinding {
                    id: binding,
                    authority,
                },
                viewport: TerminalViewport {
                    scrollback_offset_lines: scrollback,
                    follow_output: false,
                },
                draft: draft.into(),
            }),
        };
        let first = PaneNode::Leaf(PaneLeaf {
            id: pane,
            tabs: if rich {
                vec![
                    terminal_tab,
                    WorkspaceTab {
                        id: WorkspaceTabId::from_uuid(Uuid::from_u128(301)),
                        title: "main.rs".into(),
                        content: WorkspaceTabContent::Editor {
                            checkout_id: checkout,
                            relative_path: WorkspaceRelativePath::new("src/main.rs").unwrap(),
                            draft: "fn main() { /* brouillon */ }".into(),
                            cursor_line: 12,
                            cursor_column: 7,
                        },
                    },
                ]
            } else {
                vec![terminal_tab]
            },
            active_tab: Some(tab),
        });
        let root = if rich {
            PaneNode::Split {
                axis: SplitAxis::Horizontal,
                ratio_basis_points: 6200,
                first: Box::new(first),
                second: Box::new(PaneNode::Leaf(PaneLeaf {
                    id: PaneId::from_uuid(Uuid::from_u128(302)),
                    tabs: vec![
                        WorkspaceTab {
                            id: WorkspaceTabId::from_uuid(Uuid::from_u128(303)),
                            title: "Fichiers".into(),
                            content: WorkspaceTabContent::Files {
                                checkout_id: checkout,
                                relative_root: WorkspaceRelativePath::new("src").unwrap(),
                            },
                        },
                        WorkspaceTab {
                            id: WorkspaceTabId::from_uuid(Uuid::from_u128(304)),
                            title: "Aperçu".into(),
                            content: WorkspaceTabContent::Browser {
                                location: "http://127.0.0.1:3000".into(),
                            },
                        },
                    ],
                    active_tab: Some(WorkspaceTabId::from_uuid(Uuid::from_u128(304))),
                })),
            }
        } else {
            first
        };
        WorkspaceSurfaceState {
            root: Some(root),
            focus: Some(WorkspaceFocus {
                pane_id: pane,
                tab_id: tab,
            }),
        }
    }

    fn authority_checkout(authority: &TerminalAuthority) -> CatalogCheckoutId {
        match authority {
            TerminalAuthority::Local { checkout_id }
            | TerminalAuthority::Ssh { checkout_id, .. } => *checkout_id,
        }
    }

    // SDTEST-1736 — SDUC-490
    #[test]
    fn keyed_gpui_workspace_entity_retention_preserves_hidden_terminal_state() {
        let mut cx = TestAppContext::single();
        let (catalog, workspace_a, workspace_b, checkout, ssh_checkout, ssh_connection) =
            fixture_catalog();
        let hub = cx.update(|cx| cx.new(|cx| WorkspaceHubView::new(Ok(catalog), &[], cx)));
        let terminal_a = TerminalBindingId::from_uuid(Uuid::from_u128(10));
        let terminal_b = TerminalBindingId::from_uuid(Uuid::from_u128(11));

        let surface_a = terminal_surface(
            TerminalAuthority::Local {
                checkout_id: checkout,
            },
            terminal_a,
            "git status",
            42,
            true,
        );
        hub.update(&mut cx, |hub, cx| {
            hub.capture_surface(workspace_a, surface_a.clone(), cx)
                .unwrap();
            hub.capture_surface(
                workspace_b,
                terminal_surface(
                    TerminalAuthority::Ssh {
                        checkout_id: ssh_checkout,
                        connection_id: ssh_connection,
                    },
                    terminal_b,
                    "cargo test",
                    7,
                    false,
                ),
                cx,
            )
            .unwrap();
            hub.switch_to(workspace_a, cx);
        });

        let (workspace_entity_before, terminal_entity_before, native_terminal_before) = hub
            .read_with(&cx, |hub, cx| {
                let surface = hub.retained.get(&workspace_a).unwrap().clone();
                let terminal = surface.read(cx).terminal_entity(terminal_a).unwrap();
                (
                    surface.entity_id(),
                    terminal.entity_id(),
                    terminal.read(cx).terminal_view.entity_id(),
                )
            });

        hub.update(&mut cx, |hub, cx| {
            hub.switch_to(workspace_b, cx);
            hub.switch_to(workspace_a, cx);
        });

        hub.read_with(&cx, |hub, cx| {
            let surface = hub.retained.get(&workspace_a).unwrap();
            let terminal = surface.read(cx).terminal_entity(terminal_a).unwrap();
            assert_eq!(surface.entity_id(), workspace_entity_before);
            assert_eq!(terminal.entity_id(), terminal_entity_before);
            let terminal = terminal.read(cx);
            assert_eq!(terminal.terminal_view.entity_id(), native_terminal_before);
            assert_eq!(terminal.binding, terminal_a);
            assert_eq!(terminal.draft, "git status");
            assert_eq!(terminal.scrollback_offset_lines, 42);
            assert!(!terminal.follow_output);
            assert_eq!(surface.read(cx).snapshot, surface_a);
        });

        hub.update(&mut cx, |hub, cx| {
            hub.archive_or_resume(workspace_a, cx);
            assert_eq!(hub.navigation.active(), None);
            hub.archive_or_resume(workspace_a, cx);
            assert_eq!(hub.navigation.active(), Some(workspace_a));
        });
        hub.read_with(&cx, |hub, _cx| {
            assert_eq!(
                hub.retained.get(&workspace_a).unwrap().entity_id(),
                workspace_entity_before
            );
        });
    }

    // SDTEST-1738 — SDUC-489, SDUC-490
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
}
