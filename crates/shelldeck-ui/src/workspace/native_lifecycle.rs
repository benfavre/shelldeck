use crate::t;
use crate::terminal_view::AuthorizedLocalRoot;
use async_process::{Command, Stdio};
use futures_lite::io::AsyncReadExt;
use serde::{Deserialize, Serialize};
use shelldeck_core::config::workspace_catalog::{
    CatalogCheckoutId, CatalogProjectId, CatalogWorkspaceId, ProjectCheckout, WorkspaceLaunchIntake,
};
use shelldeck_core::workspace_navigation::{
    CreationOperationId, WorkspaceCreateConflict, WorkspaceCreateEvent, WorkspaceCreateFailure,
    WorkspaceCreateFailureKind, WorkspaceCreatePhase, WorkspaceCreateProgress,
};
use std::collections::{BTreeMap, HashSet};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::process::ExitStatus;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

pub(super) type ExecutorFuture<T> = Pin<Box<dyn Future<Output = T> + Send + 'static>>;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) enum WorkspaceLaunchMode {
    #[default]
    ExistingFolder,
    GitWorktree,
    Ssh,
}

#[derive(Clone, Debug)]
pub(super) enum AuthorizedLaunchHost {
    LocalExisting {
        authority: AuthorizedLocalRoot,
    },
    LocalWorktree {
        source_authority: AuthorizedLocalRoot,
        target_root: PathBuf,
        branch: String,
        start_point: String,
    },
    #[allow(dead_code)]
    Ssh {
        connection_id: uuid::Uuid,
        remote_root: String,
    },
}

#[derive(Clone, Debug)]
pub(super) struct WorkspaceExecutionRequest {
    pub workspace: CatalogWorkspaceId,
    pub project: CatalogProjectId,
    pub source_checkout: CatalogCheckoutId,
    pub checkout: CatalogCheckoutId,
    pub created_checkout: Option<ProjectCheckout>,
    pub operation: CreationOperationId,
    pub catalog_revision: u64,
    pub name: String,
    pub intake: WorkspaceLaunchIntake,
    pub host: AuthorizedLaunchHost,
    pub mode: WorkspaceLaunchMode,
}

pub(super) struct WorkspaceLaunchReceipt {
    pub authority: AuthorizedLocalRoot,
    cleanup: Option<WorktreeEffect>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct WorktreeEffect {
    source: PathBuf,
    target: PathBuf,
    branch: String,
    oid: String,
    workspace: CatalogWorkspaceId,
    source_repository: FileIdentity,
    target_identity: Option<FileIdentity>,
    reservation: String,
    committed: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct FileIdentity {
    volume: u64,
    file: u64,
}

#[derive(Debug, Deserialize, Serialize)]
struct WorktreeJournal {
    version: u8,
    workspace: String,
    source: PathBuf,
    target: PathBuf,
    branch: String,
    oid: String,
    source_repository: FileIdentity,
    target_identity: Option<FileIdentity>,
    reservation: String,
    committed: bool,
}

impl From<&WorktreeEffect> for WorktreeJournal {
    fn from(effect: &WorktreeEffect) -> Self {
        Self {
            version: 2,
            workspace: effect.workspace.to_string(),
            source: effect.source.clone(),
            target: effect.target.clone(),
            branch: effect.branch.clone(),
            oid: effect.oid.clone(),
            source_repository: effect.source_repository,
            target_identity: effect.target_identity,
            reservation: effect.reservation.clone(),
            committed: effect.committed,
        }
    }
}

impl TryFrom<WorktreeJournal> for WorktreeEffect {
    type Error = WorkspaceCreateFailure;

    fn try_from(journal: WorktreeJournal) -> Result<Self, Self::Error> {
        let workspace = uuid::Uuid::parse_str(&journal.workspace)
            .map(CatalogWorkspaceId::from_uuid)
            .map_err(|_| workspace_fs_failure())?;
        let mut reservation_components = Path::new(&journal.reservation).components();
        let reservation_is_one_normal_component = matches!(
            reservation_components.next(),
            Some(std::path::Component::Normal(value))
                if value == std::ffi::OsStr::new(&journal.reservation)
        ) && reservation_components.next().is_none();
        if journal.version != 2
            || !journal.source.is_absolute()
            || !journal.target.is_absolute()
            || journal.branch.is_empty()
            || journal.branch.len() > 255
            || !matches!(journal.oid.len(), 40 | 64)
            || !journal.oid.bytes().all(|byte| byte.is_ascii_hexdigit())
            || !reservation_is_one_normal_component
            || !journal
                .reservation
                .starts_with(&format!(".reserve-{workspace}-"))
        {
            return Err(workspace_fs_failure());
        }
        Ok(Self {
            source: journal.source,
            target: journal.target,
            branch: journal.branch,
            oid: journal.oid,
            workspace,
            source_repository: journal.source_repository,
            target_identity: journal.target_identity,
            reservation: journal.reservation,
            committed: journal.committed,
        })
    }
}

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

    fn take_receipt(&self, _operation: CreationOperationId) -> Option<WorkspaceLaunchReceipt> {
        None
    }

    fn compensate(
        &self,
        _request: WorkspaceExecutionRequest,
        _receipt: WorkspaceLaunchReceipt,
    ) -> ExecutorFuture<Result<(), WorkspaceCreateFailure>> {
        Box::pin(async { Ok(()) })
    }

    fn acknowledge(
        &self,
        _request: WorkspaceExecutionRequest,
    ) -> ExecutorFuture<Result<(), WorkspaceCreateFailure>> {
        Box::pin(async { Ok(()) })
    }

    fn recover_orphans(
        &self,
        _retained_roots: Vec<PathBuf>,
    ) -> ExecutorFuture<Result<(), WorkspaceCreateFailure>> {
        Box::pin(async { Ok(()) })
    }
}

pub(super) struct LaunchCancellation {
    requested: AtomicBool,
    finished: AtomicBool,
}

impl Default for LaunchCancellation {
    fn default() -> Self {
        Self {
            requested: AtomicBool::new(false),
            finished: AtomicBool::new(false),
        }
    }
}

pub(super) struct NativeWorkspaceExecutor {
    cancellations: Arc<parking_lot::Mutex<BTreeMap<CreationOperationId, Arc<LaunchCancellation>>>>,
    receipts: Arc<parking_lot::Mutex<BTreeMap<CreationOperationId, WorkspaceLaunchReceipt>>>,
    git: Arc<dyn GitWorktreeAdapter>,
}

pub(super) enum NativeLaunchOutcome {
    Ready(WorkspaceLaunchReceipt),
    Cancelled,
    Conflict(WorkspaceCreateConflict),
}

#[cfg(test)]
impl NativeLaunchOutcome {
    pub(super) fn test_ready(path: &Path) -> Self {
        Self::Ready(WorkspaceLaunchReceipt {
            authority: AuthorizedLocalRoot::capture(path).unwrap(),
            cleanup: None,
        })
    }
}

struct LaunchFinishGuard {
    operation: CreationOperationId,
    cancellation: Arc<LaunchCancellation>,
    registry: Arc<parking_lot::Mutex<BTreeMap<CreationOperationId, Arc<LaunchCancellation>>>>,
}

impl Drop for LaunchFinishGuard {
    fn drop(&mut self) {
        self.cancellation.finished.store(true, Ordering::Release);
        self.registry.lock().remove(&self.operation);
    }
}

#[derive(Debug)]
struct ChildOutput {
    status: ExitStatus,
    stdout: Vec<u8>,
    stderr: String,
}

pub(super) trait GitWorktreeAdapter: Send + Sync {
    fn prepare(
        &self,
        request: WorkspaceExecutionRequest,
        cancelled: Arc<LaunchCancellation>,
    ) -> ExecutorFuture<Result<NativeLaunchOutcome, WorkspaceCreateFailure>>;
}

#[derive(Default)]
struct SystemGitWorktreeAdapter;

impl GitWorktreeAdapter for SystemGitWorktreeAdapter {
    fn prepare(
        &self,
        request: WorkspaceExecutionRequest,
        cancelled: Arc<LaunchCancellation>,
    ) -> ExecutorFuture<Result<NativeLaunchOutcome, WorkspaceCreateFailure>> {
        Box::pin(async move { prepare_git_worktree(&request, &cancelled.requested).await })
    }
}

impl Default for NativeWorkspaceExecutor {
    fn default() -> Self {
        Self {
            cancellations: Arc::default(),
            receipts: Arc::default(),
            git: Arc::new(SystemGitWorktreeAdapter),
        }
    }
}

impl NativeWorkspaceExecutor {
    #[cfg(test)]
    pub(super) fn with_git(git: Arc<dyn GitWorktreeAdapter>) -> Self {
        Self {
            cancellations: Arc::default(),
            receipts: Arc::default(),
            git,
        }
    }

    fn cancellation(&self, operation: CreationOperationId) -> Arc<LaunchCancellation> {
        let mut cancellations = self.cancellations.lock();
        cancellations
            .entry(operation)
            .or_insert_with(|| Arc::new(LaunchCancellation::default()))
            .clone()
    }
}

impl WorkspaceLaunchExecutor for NativeWorkspaceExecutor {
    fn launch(
        &self,
        request: WorkspaceExecutionRequest,
        events: mpsc::UnboundedSender<WorkspaceCreateEvent>,
    ) -> ExecutorFuture<Result<(), WorkspaceCreateFailure>> {
        let cancellation = self.cancellation(request.operation);
        let registry = self.cancellations.clone();
        let receipts = self.receipts.clone();
        let git = self.git.clone();
        Box::pin(async move {
            let _finish = LaunchFinishGuard {
                operation: request.operation,
                cancellation: cancellation.clone(),
                registry,
            };
            if request.mode == WorkspaceLaunchMode::Ssh {
                return Err(runtime_unavailable(
                    "workspaces.launcher.ssh_executor_unavailable",
                ));
            }
            let phases = [
                WorkspaceCreatePhase::Queued,
                WorkspaceCreatePhase::ResolvingHost,
                WorkspaceCreatePhase::PreparingCheckout,
                WorkspaceCreatePhase::CreatingWorkspace,
                WorkspaceCreatePhase::BindingRuntime,
            ];
            for phase in phases.into_iter().take(3) {
                if cancellation.requested.load(Ordering::Acquire) {
                    return Ok(());
                }
                send_progress(&events, &request, phase)?;
                futures_lite::future::yield_now().await;
            }
            let outcome = match &request.host {
                AuthorizedLaunchHost::LocalExisting { authority } => {
                    authority.revalidate().map_err(|_| workspace_fs_failure())?;
                    NativeLaunchOutcome::Ready(WorkspaceLaunchReceipt {
                        authority: authority.clone(),
                        cleanup: None,
                    })
                }
                AuthorizedLaunchHost::LocalWorktree { .. } => {
                    match git.prepare(request.clone(), cancellation.clone()).await {
                        Err(error)
                            if cancellation.requested.load(Ordering::Acquire)
                                && error.message == CANCELLED_MESSAGE =>
                        {
                            return Ok(())
                        }
                        result => result?,
                    }
                }
                AuthorizedLaunchHost::Ssh { .. } => unreachable!(),
            };
            let receipt = match outcome {
                NativeLaunchOutcome::Cancelled => return Ok(()),
                NativeLaunchOutcome::Conflict(conflict) => {
                    events
                        .send(WorkspaceCreateEvent::Conflict {
                            workspace: request.workspace,
                            operation: request.operation,
                            conflict,
                        })
                        .map_err(|_| receiver_closed())?;
                    return Ok(());
                }
                NativeLaunchOutcome::Ready(receipt) => receipt,
            };
            for phase in phases.into_iter().skip(3) {
                if cancellation.requested.load(Ordering::Acquire) {
                    compensate_receipt(receipt).await?;
                    return Ok(());
                }
                if let Err(error) = send_progress(&events, &request, phase) {
                    compensate_receipt(receipt).await?;
                    return Err(error);
                }
            }
            if cancellation.requested.load(Ordering::Acquire) {
                compensate_receipt(receipt).await?;
                return Ok(());
            }
            receipts.lock().insert(request.operation, receipt);
            if events
                .send(WorkspaceCreateEvent::Completed {
                    workspace: request.workspace,
                    operation: request.operation,
                })
                .is_err()
            {
                let receipt = { receipts.lock().remove(&request.operation) };
                if let Some(receipt) = receipt {
                    compensate_receipt(receipt).await?;
                }
                return Err(receiver_closed());
            }
            Ok(())
        })
    }

    fn cancel(
        &self,
        request: WorkspaceExecutionRequest,
    ) -> ExecutorFuture<Result<WorkspaceCreateEvent, WorkspaceCreateFailure>> {
        let cancellation = self.cancellations.lock().get(&request.operation).cloned();
        Box::pin(async move {
            if let Some(cancellation) = cancellation {
                cancellation.requested.store(true, Ordering::Release);
                while !cancellation.finished.load(Ordering::Acquire) {
                    async_io::Timer::after(Duration::from_millis(5)).await;
                }
            }
            Ok(WorkspaceCreateEvent::Cancelled {
                workspace: request.workspace,
                operation: request.operation,
            })
        })
    }

    fn take_receipt(&self, operation: CreationOperationId) -> Option<WorkspaceLaunchReceipt> {
        self.receipts.lock().remove(&operation)
    }

    fn compensate(
        &self,
        _request: WorkspaceExecutionRequest,
        receipt: WorkspaceLaunchReceipt,
    ) -> ExecutorFuture<Result<(), WorkspaceCreateFailure>> {
        Box::pin(async move { compensate_receipt(receipt).await })
    }

    fn acknowledge(
        &self,
        request: WorkspaceExecutionRequest,
    ) -> ExecutorFuture<Result<(), WorkspaceCreateFailure>> {
        Box::pin(async move {
            if let AuthorizedLaunchHost::LocalWorktree { target_root, .. } = request.host {
                let private_root = target_root.parent().ok_or_else(workspace_fs_failure)?;
                let authority = ensure_private_root(private_root)?;
                let mut effect =
                    read_intent(&authority, request.workspace)?.ok_or_else(workspace_fs_failure)?;
                if effect.target != target_root {
                    return Err(authorization_failure(
                        "workspaces.launcher.target_authority_changed",
                    ));
                }
                effect.committed = true;
                write_intent(&authority, &effect, false)?;
            }
            Ok(())
        })
    }

    fn recover_orphans(
        &self,
        retained_roots: Vec<PathBuf>,
    ) -> ExecutorFuture<Result<(), WorkspaceCreateFailure>> {
        Box::pin(async move { recover_owned_intents(retained_roots).await })
    }
}

fn send_progress(
    events: &mpsc::UnboundedSender<WorkspaceCreateEvent>,
    request: &WorkspaceExecutionRequest,
    phase: WorkspaceCreatePhase,
) -> Result<(), WorkspaceCreateFailure> {
    events
        .send(WorkspaceCreateEvent::Progress {
            workspace: request.workspace,
            operation: request.operation,
            progress: WorkspaceCreateProgress {
                phase,
                completed_steps: 1,
                total_steps: 1,
                detail: request.name.clone(),
            },
        })
        .map_err(|_| receiver_closed())
}

async fn prepare_git_worktree(
    request: &WorkspaceExecutionRequest,
    cancelled: &AtomicBool,
) -> Result<NativeLaunchOutcome, WorkspaceCreateFailure> {
    let AuthorizedLaunchHost::LocalWorktree {
        source_authority,
        target_root,
        branch,
        start_point,
    } = &request.host
    else {
        unreachable!()
    };
    source_authority
        .revalidate()
        .map_err(|_| workspace_fs_failure())?;
    let source = source_authority.path();
    let top = git_output(source, ["rev-parse", "--show-toplevel"], cancelled).await?;
    let top = path_from_output(&top.stdout)?;
    if std::fs::canonicalize(top).ok().as_deref() != Some(source) {
        return Err(authorization_failure(
            "workspaces.launcher.git_root_mismatch",
        ));
    }
    validate_branch(source, branch, cancelled).await?;
    let oid = resolve_commit(source, start_point, cancelled).await?;
    let source_repository = repository_identity(source, cancelled).await?;
    let private_root = target_root.parent().ok_or_else(workspace_fs_failure)?;
    let private_authority = ensure_private_root(private_root)?;
    if target_root.parent() != Some(private_root)
        || target_root.file_name().and_then(|name| name.to_str())
            != Some(&request.workspace.to_string())
    {
        return Err(authorization_failure(
            "workspaces.launcher.target_authority_changed",
        ));
    }

    if target_root.exists() {
        let Some(effect) = read_intent(&private_authority, request.workspace)? else {
            return Ok(NativeLaunchOutcome::Conflict(
                WorkspaceCreateConflict::CheckoutAlreadyExists {
                    root: target_root.display().to_string(),
                },
            ));
        };
        if !effect_matches_request(
            &effect,
            request.workspace,
            source,
            target_root,
            branch,
            &oid,
            source_repository,
        ) {
            return Err(authorization_failure(
                "workspaces.launcher.target_authority_changed",
            ));
        }
        if exact_complete_worktree(
            source,
            target_root,
            branch,
            &oid,
            source_repository,
            effect.target_identity,
            cancelled,
        )
        .await?
        {
            let authority =
                AuthorizedLocalRoot::capture(target_root).map_err(|_| workspace_fs_failure())?;
            return Ok(NativeLaunchOutcome::Ready(WorkspaceLaunchReceipt {
                authority,
                cleanup: Some(effect),
            }));
        }
        cleanup_owned_effect(effect).await?;
    }

    let branch_ref = format!("refs/heads/{branch}");
    let branch_status = git_output(
        source,
        [
            "show-ref",
            "--verify",
            "--quiet",
            "--end-of-options",
            &branch_ref,
        ],
        cancelled,
    )
    .await?;
    if branch_status.status.success() {
        return Ok(NativeLaunchOutcome::Conflict(
            WorkspaceCreateConflict::BranchAlreadyExists {
                branch: branch.clone(),
            },
        ));
    }
    if branch_status.status.code() != Some(1) {
        return Err(git_repo_failure());
    }
    if cancelled.load(Ordering::Acquire) {
        return Ok(NativeLaunchOutcome::Cancelled);
    }
    let mut effect = WorktreeEffect {
        source: source.to_path_buf(),
        target: target_root.clone(),
        branch: branch.clone(),
        oid: oid.clone(),
        workspace: request.workspace,
        source_repository,
        target_identity: None,
        reservation: format!(".reserve-{}-{}", request.workspace, uuid::Uuid::new_v4()),
        committed: false,
    };
    write_intent(&private_authority, &effect, true)?;
    let reservation_path = private_authority.command_path().join(&effect.reservation);
    if let Err(error) = std::fs::create_dir(&reservation_path) {
        let _ = remove_intent(&private_authority, request.workspace);
        tracing::warn!(%error, "could not reserve private worktree target");
        return Err(workspace_fs_failure());
    }
    effect.target_identity = Some(file_identity(&reservation_path)?);
    if let Err(error) = write_intent(&private_authority, &effect, false) {
        let _ = remove_owned_target(&private_authority, &effect);
        let _ = remove_intent(&private_authority, request.workspace);
        return Err(error);
    }
    let (command_private_root, _inherited_private_root) = private_authority
        .inherited_argument_path()
        .map_err(|_| workspace_fs_failure())?;
    let command_target = command_private_root.join(request.workspace.to_string());
    if let Err(error) = std::fs::rename(&reservation_path, &command_target) {
        cleanup_owned_effect(effect).await?;
        tracing::warn!(%error, "could not publish private worktree target reservation");
        return Err(workspace_fs_failure());
    }
    sync_directory(&private_authority.command_path())?;
    if private_authority.revalidate().is_err() {
        let _ = remove_owned_target(&private_authority, &effect);
        let _ = remove_intent(&private_authority, request.workspace);
        return Err(authorization_failure(
            "workspaces.launcher.target_authority_changed",
        ));
    }
    let output = git_output_owned(
        source,
        vec![
            "worktree".into(),
            "add".into(),
            "--no-track".into(),
            "-b".into(),
            branch.clone().into(),
            "--".into(),
            command_target.as_os_str().to_owned(),
            oid.clone().into(),
        ],
        cancelled,
    )
    .await;
    let output = match output {
        Ok(output) => output,
        Err(error) => {
            cleanup_owned_effect(effect).await?;
            return Err(error);
        }
    };
    if cancelled.load(Ordering::Acquire) {
        cleanup_owned_effect(effect).await?;
        return Ok(NativeLaunchOutcome::Cancelled);
    }
    if !output.status.success() {
        tracing::warn!(stderr = %bounded_error(&output.stderr), "git worktree creation failed");
        cleanup_owned_effect(effect).await?;
        return Err(filesystem_failure(
            "workspaces.launcher.worktree_failed",
            true,
        ));
    }
    let exact = match exact_complete_worktree(
        source,
        target_root,
        branch,
        &oid,
        source_repository,
        effect.target_identity,
        cancelled,
    )
    .await
    {
        Ok(exact) => exact,
        Err(error) => {
            cleanup_owned_effect(effect).await?;
            return Err(error);
        }
    };
    if !exact {
        cleanup_owned_effect(effect).await?;
        return Err(filesystem_failure(
            "workspaces.launcher.worktree_failed",
            true,
        ));
    }
    let authority = match AuthorizedLocalRoot::capture(target_root) {
        Ok(authority) => authority,
        Err(_) => {
            cleanup_owned_effect(effect).await?;
            return Err(workspace_fs_failure());
        }
    };
    Ok(NativeLaunchOutcome::Ready(WorkspaceLaunchReceipt {
        authority,
        cleanup: Some(effect),
    }))
}

async fn validate_branch(
    source: &Path,
    branch: &str,
    cancelled: &AtomicBool,
) -> Result<(), WorkspaceCreateFailure> {
    if branch.is_empty() || branch.len() > 255 || branch.starts_with('-') {
        return Err(invalid_branch_failure());
    }
    let output = git_output(source, ["check-ref-format", "--branch", branch], cancelled).await?;
    if output.status.success() {
        Ok(())
    } else {
        Err(invalid_branch_failure())
    }
}

async fn resolve_commit(
    source: &Path,
    start_point: &str,
    cancelled: &AtomicBool,
) -> Result<String, WorkspaceCreateFailure> {
    if start_point.is_empty()
        || start_point.len() > 512
        || start_point.starts_with('-')
        || start_point.chars().any(char::is_control)
    {
        return Err(authorization_failure(
            "workspaces.launcher.invalid_start_point",
        ));
    }
    let commitish = format!("{start_point}^{{commit}}");
    let output = git_output(
        source,
        [
            "rev-parse",
            "--verify",
            "--quiet",
            "--end-of-options",
            &commitish,
        ],
        cancelled,
    )
    .await?;
    if !output.status.success() {
        return Err(filesystem_failure(
            "workspaces.launcher.start_point_missing",
            true,
        ));
    }
    let oid = String::from_utf8(output.stdout)
        .map_err(|_| git_repo_failure())?
        .trim()
        .to_ascii_lowercase();
    if !matches!(oid.len(), 40 | 64) || !oid.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(git_repo_failure());
    }
    Ok(oid)
}

async fn exact_complete_worktree(
    source: &Path,
    target: &Path,
    branch: &str,
    oid: &str,
    source_repository: FileIdentity,
    target_identity: Option<FileIdentity>,
    cancelled: &AtomicBool,
) -> Result<bool, WorkspaceCreateFailure> {
    let Ok(authority) = AuthorizedLocalRoot::capture(target) else {
        return Ok(false);
    };
    if target_identity.is_none() || file_identity(target).ok() != target_identity {
        return Ok(false);
    }
    let list = git_output(source, ["worktree", "list", "--porcelain", "-z"], cancelled).await?;
    if !list.status.success() {
        return Err(git_repo_failure());
    }
    let expected_branch = format!("refs/heads/{branch}");
    let mut registered = false;
    let mut path = None;
    let mut observed_branch = None;
    let mut unsafe_record = false;
    for field in list.stdout.split(|byte| *byte == 0) {
        let field = String::from_utf8_lossy(field);
        if field.is_empty() {
            if path.as_deref() == Some(authority.path())
                && observed_branch.as_deref() == Some(expected_branch.as_str())
                && !unsafe_record
            {
                registered = true;
                break;
            }
            path = None;
            observed_branch = None;
            unsafe_record = false;
        } else if let Some(value) = field.strip_prefix("worktree ") {
            let candidate = PathBuf::from(value);
            path = (candidate == authority.path()).then_some(candidate);
        } else if let Some(value) = field.strip_prefix("branch ") {
            observed_branch = Some(value.to_owned());
        } else if field == "detached"
            || field.starts_with("locked")
            || field.starts_with("prunable")
        {
            unsafe_record = true;
        }
    }
    if !registered {
        return Ok(false);
    }
    let head = git_output(target, ["rev-parse", "--verify", "HEAD"], cancelled).await?;
    let head = String::from_utf8(head.stdout).map_err(|_| git_repo_failure())?;
    if !head.trim().eq_ignore_ascii_case(oid) {
        return Ok(false);
    }
    let symbolic = git_output(target, ["symbolic-ref", "-q", "HEAD"], cancelled).await?;
    if !symbolic.status.success() || symbolic.stdout != format!("{expected_branch}\n").as_bytes() {
        return Ok(false);
    }
    let branch_oid = git_output(
        target,
        [
            "rev-parse",
            "--verify",
            "--end-of-options",
            &expected_branch,
        ],
        cancelled,
    )
    .await?;
    if !String::from_utf8_lossy(&branch_oid.stdout)
        .trim()
        .eq_ignore_ascii_case(oid)
    {
        return Ok(false);
    }
    let source_common = git_output(
        source,
        ["rev-parse", "--path-format=absolute", "--git-common-dir"],
        cancelled,
    )
    .await?;
    let target_common = git_output(
        target,
        ["rev-parse", "--path-format=absolute", "--git-common-dir"],
        cancelled,
    )
    .await?;
    let source_common = std::fs::canonicalize(path_from_output(&source_common.stdout)?)
        .map_err(|_| git_repo_failure())?;
    let target_common = std::fs::canonicalize(path_from_output(&target_common.stdout)?)
        .map_err(|_| git_repo_failure())?;
    if source_common != target_common
        || file_identity(&source_common).ok() != Some(source_repository)
        || file_identity(&target_common).ok() != Some(source_repository)
    {
        return Ok(false);
    }
    let status = git_output(
        target,
        [
            "status",
            "--porcelain=v2",
            "-z",
            "--untracked-files=all",
            "--ignore-submodules=none",
        ],
        cancelled,
    )
    .await?;
    authority.revalidate().map_err(|_| workspace_fs_failure())?;
    Ok(status.status.success()
        && status.stdout.is_empty()
        && file_identity(target).ok() == target_identity)
}

async fn repository_identity(
    source: &Path,
    cancelled: &AtomicBool,
) -> Result<FileIdentity, WorkspaceCreateFailure> {
    let common = git_output(
        source,
        ["rev-parse", "--path-format=absolute", "--git-common-dir"],
        cancelled,
    )
    .await?;
    if !common.status.success() {
        return Err(git_repo_failure());
    }
    let common =
        std::fs::canonicalize(path_from_output(&common.stdout)?).map_err(|_| git_repo_failure())?;
    file_identity(&common)
}

async fn compensate_receipt(receipt: WorkspaceLaunchReceipt) -> Result<(), WorkspaceCreateFailure> {
    if let Some(effect) = receipt.cleanup {
        cleanup_owned_effect(effect).await?;
    }
    Ok(())
}

async fn cleanup_owned_effect(effect: WorktreeEffect) -> Result<(), WorkspaceCreateFailure> {
    let private_root = effect.target.parent().ok_or_else(workspace_fs_failure)?;
    let private_authority = ensure_private_root(private_root)?;
    if effect.target.file_name().and_then(|value| value.to_str())
        != Some(&effect.workspace.to_string())
    {
        return Err(authorization_failure(
            "workspaces.launcher.target_authority_changed",
        ));
    }
    if read_intent(&private_authority, effect.workspace)?.as_ref() != Some(&effect) {
        return Err(authorization_failure(
            "workspaces.launcher.target_authority_changed",
        ));
    }
    let never_cancelled = AtomicBool::new(false);
    if repository_identity(&effect.source, &never_cancelled).await? != effect.source_repository {
        return Err(authorization_failure(
            "workspaces.launcher.target_authority_changed",
        ));
    }
    let registered = worktree_registered(
        &effect.source,
        &effect.target,
        &effect.branch,
        &never_cancelled,
    )
    .await
    .unwrap_or(false);
    if registered {
        if !exact_complete_worktree(
            &effect.source,
            &effect.target,
            &effect.branch,
            &effect.oid,
            effect.source_repository,
            effect.target_identity,
            &never_cancelled,
        )
        .await?
        {
            return Err(authorization_failure(
                "workspaces.launcher.target_authority_changed",
            ));
        }
        let (command_target, _target_authority, _inherited_target) =
            bound_worktree_argument(&effect)?;
        let removed = git_output_owned(
            &effect.source,
            vec![
                "worktree".into(),
                "remove".into(),
                "--force".into(),
                "--".into(),
                command_target.into_os_string(),
            ],
            &never_cancelled,
        )
        .await;
        if !removed?.status.success()
            || worktree_registered(
                &effect.source,
                &effect.target,
                &effect.branch,
                &never_cancelled,
            )
            .await?
        {
            return Err(workspace_fs_failure());
        }
    } else if owned_target_is_nonempty(&private_authority, &effect)? {
        return Err(authorization_failure(
            "workspaces.launcher.target_authority_changed",
        ));
    }
    let branch_ref = format!("refs/heads/{}", effect.branch);
    let branch_removed = git_output(
        &effect.source,
        ["update-ref", "-d", &branch_ref, &effect.oid],
        &never_cancelled,
    )
    .await?;
    if !branch_removed.status.success() {
        return Err(git_repo_failure());
    }
    remove_owned_target(&private_authority, &effect)?;
    remove_intent(&private_authority, effect.workspace)?;
    Ok(())
}

async fn worktree_registered(
    source: &Path,
    target: &Path,
    branch: &str,
    cancelled: &AtomicBool,
) -> Result<bool, WorkspaceCreateFailure> {
    let output = git_output(source, ["worktree", "list", "--porcelain", "-z"], cancelled).await?;
    if !output.status.success() {
        return Err(git_repo_failure());
    }
    let expected_branch = format!("refs/heads/{branch}");
    let mut path = None;
    let mut observed_branch = None;
    for field in output.stdout.split(|byte| *byte == 0) {
        let field = String::from_utf8_lossy(field);
        if field.is_empty() {
            if path.as_deref() == Some(target)
                && observed_branch.as_deref() == Some(expected_branch.as_str())
            {
                return Ok(true);
            }
            path = None;
            observed_branch = None;
        } else if let Some(value) = field.strip_prefix("worktree ") {
            let candidate = PathBuf::from(value);
            path = (candidate == target).then_some(candidate);
        } else if let Some(value) = field.strip_prefix("branch ") {
            observed_branch = Some(value.to_owned());
        }
    }
    Ok(path.as_deref() == Some(target)
        && observed_branch.as_deref() == Some(expected_branch.as_str()))
}

async fn run_command(
    mut command: Command,
    cancelled: &AtomicBool,
) -> Result<ChildOutput, WorkspaceCreateFailure> {
    command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let mut child = command.spawn().map_err(|_| git_runtime_failure())?;
    let mut process_tree =
        match shelldeck_core::agent_runtime::LocalProcessTree::capture(child.id()) {
            Ok(process_tree) => process_tree,
            Err(_) => {
                let _ = child.kill();
                let _ = child.status().await;
                return Err(git_runtime_failure());
            }
        };
    let stdout = child.stdout.take().ok_or_else(git_runtime_failure)?;
    let stderr = child.stderr.take().ok_or_else(git_runtime_failure)?;
    let stdout_task = smol::spawn(drain_bounded(stdout, 64 * 1024));
    let stderr_task = smol::spawn(drain_bounded(stderr, 16 * 1024));
    let started = Instant::now();
    let mut timed_out = false;
    let mut was_cancelled = false;
    let status = loop {
        if cancelled.load(Ordering::Acquire) || started.elapsed() >= Duration::from_secs(120) {
            was_cancelled = cancelled.load(Ordering::Acquire);
            timed_out = !was_cancelled;
            process_tree.terminate();
            let _ = child.kill();
        }
        if let Some(status) = child.try_status().map_err(|_| git_runtime_failure())? {
            break status;
        }
        async_io::Timer::after(Duration::from_millis(5)).await;
    };
    let drained = futures_lite::future::race(
        async move { Some((stdout_task.await, stderr_task.await)) },
        async {
            async_io::Timer::after(Duration::from_secs(1)).await;
            None
        },
    )
    .await;
    let Some((stdout, stderr)) = drained else {
        process_tree.terminate();
        let _ = child.kill();
        return Err(filesystem_failure(
            "workspaces.launcher.worktree_timeout",
            true,
        ));
    };
    let stderr = String::from_utf8_lossy(&stderr).into_owned();
    process_tree.disarm();
    if was_cancelled {
        return Err(WorkspaceCreateFailure {
            kind: WorkspaceCreateFailureKind::Unknown,
            message: CANCELLED_MESSAGE.into(),
            retryable: true,
        });
    }
    if timed_out {
        return Err(filesystem_failure(
            "workspaces.launcher.worktree_timeout",
            true,
        ));
    }
    Ok(ChildOutput {
        status,
        stdout,
        stderr,
    })
}

async fn drain_bounded<R: futures_lite::io::AsyncRead + Unpin>(
    mut reader: R,
    limit: usize,
) -> Vec<u8> {
    let mut retained = Vec::with_capacity(limit.min(4096));
    let mut buffer = [0_u8; 8192];
    loop {
        match reader.read(&mut buffer).await {
            Ok(0) | Err(_) => break,
            Ok(read) => {
                let keep = read.min(limit.saturating_sub(retained.len()));
                retained.extend_from_slice(&buffer[..keep]);
            }
        }
    }
    retained
}

async fn git_output<const N: usize>(
    root: &Path,
    args: [&str; N],
    cancelled: &AtomicBool,
) -> Result<ChildOutput, WorkspaceCreateFailure> {
    git_output_owned(root, args.into_iter().map(Into::into).collect(), cancelled).await
}

async fn git_output_owned(
    root: &Path,
    args: Vec<std::ffi::OsString>,
    cancelled: &AtomicBool,
) -> Result<ChildOutput, WorkspaceCreateFailure> {
    let authority = AuthorizedLocalRoot::capture(root)
        .map_err(|_| authorization_failure("workspaces.launcher.target_authority_changed"))?;
    let mut command = std::process::Command::new("git");
    command
        .args(args)
        .current_dir(authority.command_path())
        .stdin(Stdio::null())
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_COMMON_DIR")
        .env_remove("GIT_INDEX_FILE");
    shelldeck_core::agent_runtime::configure_local_process(&mut command);
    let output = run_command(Command::from(command), cancelled).await;
    authority
        .revalidate()
        .map_err(|_| authorization_failure("workspaces.launcher.target_authority_changed"))?;
    output
}

fn ensure_private_root(root: &Path) -> Result<AuthorizedLocalRoot, WorkspaceCreateFailure> {
    if !root.is_absolute() {
        return Err(authorization_failure(
            "workspaces.launcher.target_authority_changed",
        ));
    }
    #[cfg(unix)]
    secure_create_directory_chain(root)?;
    #[cfg(not(unix))]
    {
        let mut cursor = PathBuf::new();
        for component in root.components() {
            cursor.push(component.as_os_str());
            if cursor.as_os_str().is_empty() {
                continue;
            }
            match std::fs::symlink_metadata(&cursor) {
                Ok(metadata) => {
                    if metadata.file_type().is_symlink()
                        || is_windows_reparse(&metadata)
                        || !metadata.is_dir()
                    {
                        return Err(authorization_failure(
                            "workspaces.launcher.target_authority_changed",
                        ));
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    std::fs::create_dir(&cursor).map_err(|_| workspace_fs_failure())?;
                }
                Err(_) => return Err(workspace_fs_failure()),
            }
        }
    }
    let authority = AuthorizedLocalRoot::capture(root)
        .map_err(|_| authorization_failure("workspaces.launcher.target_authority_changed"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(
            authority.command_path(),
            std::fs::Permissions::from_mode(0o700),
        )
        .map_err(|_| workspace_fs_failure())?;
    }
    let intents = authority.command_path().join(".intents");
    match std::fs::create_dir(&intents) {
        Ok(()) => sync_directory(&authority.command_path())?,
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(_) => return Err(workspace_fs_failure()),
    }
    let metadata = std::fs::symlink_metadata(&intents).map_err(|_| workspace_fs_failure())?;
    if metadata.file_type().is_symlink() || is_windows_reparse(&metadata) || !metadata.is_dir() {
        return Err(authorization_failure(
            "workspaces.launcher.target_authority_changed",
        ));
    }
    authority
        .revalidate()
        .map_err(|_| authorization_failure("workspaces.launcher.target_authority_changed"))?;
    Ok(authority)
}

fn write_intent(
    private_root: &AuthorizedLocalRoot,
    effect: &WorktreeEffect,
    create_new: bool,
) -> Result<(), WorkspaceCreateFailure> {
    private_root
        .revalidate()
        .map_err(|_| authorization_failure("workspaces.launcher.target_authority_changed"))?;
    let final_path = intent_path(private_root, effect.workspace);
    let payload = serde_json::to_vec_pretty(&WorktreeJournal::from(effect))
        .map_err(|_| workspace_fs_failure())?;
    if create_new {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&final_path)
            .map_err(|_| authorization_failure("workspaces.launcher.target_authority_changed"))?;
        use std::io::Write as _;
        if file
            .write_all(&payload)
            .and_then(|_| file.sync_all())
            .is_err()
        {
            drop(file);
            let _ = std::fs::remove_file(&final_path);
            let _ = sync_directory(final_path.parent().ok_or_else(workspace_fs_failure)?);
            return Err(workspace_fs_failure());
        }
        sync_directory(final_path.parent().ok_or_else(workspace_fs_failure)?)?;
    } else {
        validate_regular_nofollow(&final_path)?;
        shelldeck_core::util::atomic_write(&final_path, &payload)
            .map_err(|_| workspace_fs_failure())?;
    }
    Ok(())
}

fn read_intent(
    private_root: &AuthorizedLocalRoot,
    workspace: CatalogWorkspaceId,
) -> Result<Option<WorktreeEffect>, WorkspaceCreateFailure> {
    let path = intent_path(private_root, workspace);
    let metadata = match std::fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(workspace_fs_failure()),
    };
    if metadata.file_type().is_symlink() || is_windows_reparse(&metadata) || !metadata.is_file() {
        return Err(authorization_failure(
            "workspaces.launcher.target_authority_changed",
        ));
    }
    let file = open_regular_nofollow(&path)?;
    let journal: WorktreeJournal =
        serde_json::from_reader(file).map_err(|_| workspace_fs_failure())?;
    let effect = WorktreeEffect::try_from(journal)?;
    if effect.workspace != workspace {
        return Err(authorization_failure(
            "workspaces.launcher.target_authority_changed",
        ));
    }
    Ok(Some(effect))
}

fn intent_path(private_root: &AuthorizedLocalRoot, workspace: CatalogWorkspaceId) -> PathBuf {
    private_root
        .command_path()
        .join(".intents")
        .join(format!("{workspace}.json"))
}

fn remove_intent(
    private_root: &AuthorizedLocalRoot,
    workspace: CatalogWorkspaceId,
) -> Result<(), WorkspaceCreateFailure> {
    let path = intent_path(private_root, workspace);
    if path.exists() {
        validate_regular_nofollow(&path)?;
    }
    match std::fs::remove_file(path) {
        Ok(()) => sync_directory(&private_root.command_path().join(".intents")),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err(workspace_fs_failure()),
    }
}

fn remove_owned_target(
    private_root: &AuthorizedLocalRoot,
    effect: &WorktreeEffect,
) -> Result<(), WorkspaceCreateFailure> {
    if effect.target.parent() != Some(private_root.path()) {
        return Err(authorization_failure(
            "workspaces.launcher.target_authority_changed",
        ));
    }
    let Some(expected) = effect.target_identity else {
        return Ok(());
    };
    let target = private_root
        .command_path()
        .join(effect.workspace.to_string());
    let reservation = private_root.command_path().join(&effect.reservation);
    let candidate = [target, reservation]
        .into_iter()
        .find(|candidate| file_identity(candidate).ok() == Some(expected));
    let Some(candidate) = candidate else {
        if [
            private_root
                .command_path()
                .join(effect.workspace.to_string()),
            private_root.command_path().join(&effect.reservation),
        ]
        .into_iter()
        .any(|path| path.exists())
        {
            return Err(authorization_failure(
                "workspaces.launcher.target_authority_changed",
            ));
        }
        return Ok(());
    };
    #[cfg(not(unix))]
    {
        let _ = (candidate, expected);
        // Preserve the exact candidate and journal when the platform cannot
        // provide handle-relative recursive deletion.
        return Err(authorization_failure(
            "workspaces.launcher.target_authority_changed",
        ));
    }
    #[cfg(unix)]
    let quarantine = private_root
        .command_path()
        .join(format!(".cleanup-{}", uuid::Uuid::new_v4()));
    #[cfg(unix)]
    std::fs::rename(&candidate, &quarantine).map_err(|_| workspace_fs_failure())?;
    #[cfg(unix)]
    sync_directory(&private_root.command_path())?;
    #[cfg(unix)]
    remove_verified_quarantine(private_root, &quarantine, expected)
}

#[cfg(unix)]
fn remove_verified_quarantine(
    private_root: &AuthorizedLocalRoot,
    quarantine: &Path,
    expected: FileIdentity,
) -> Result<(), WorkspaceCreateFailure> {
    let authority = AuthorizedLocalRoot::capture(quarantine)
        .map_err(|_| authorization_failure("workspaces.launcher.target_authority_changed"))?;
    if authority.unix_identity() != (expected.volume, expected.file) {
        return Err(authorization_failure(
            "workspaces.launcher.target_authority_changed",
        ));
    }
    // Delete children through the already-open directory authority. The
    // standard library's Unix remove_dir_all is descriptor-relative and
    // symlink-race hardened; spelling each child beneath the stable fd keeps a
    // later exchange of the quarantine leaf out of the traversal.
    let entries = std::fs::read_dir(authority.command_path())
        .map_err(|_| workspace_fs_failure())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| workspace_fs_failure())?;
    for entry in entries {
        let child = authority.command_path().join(entry.file_name());
        let metadata = std::fs::symlink_metadata(&child).map_err(|_| workspace_fs_failure())?;
        if metadata.is_dir() && !metadata.file_type().is_symlink() {
            std::fs::remove_dir_all(&child).map_err(|_| workspace_fs_failure())?;
        } else {
            std::fs::remove_file(&child).map_err(|_| workspace_fs_failure())?;
        }
    }
    authority
        .revalidate()
        .map_err(|_| authorization_failure("workspaces.launcher.target_authority_changed"))?;
    // Only an empty directory can be removed here. An exchange after the
    // revalidation therefore cannot recursively destroy replacement data.
    std::fs::remove_dir(quarantine).map_err(|_| workspace_fs_failure())?;
    sync_directory(&private_root.command_path())
}

fn bound_worktree_argument(
    effect: &WorktreeEffect,
) -> Result<(PathBuf, AuthorizedLocalRoot, Option<std::fs::File>), WorkspaceCreateFailure> {
    let _expected = effect
        .target_identity
        .ok_or_else(|| authorization_failure("workspaces.launcher.target_authority_changed"))?;
    let authority = AuthorizedLocalRoot::capture(&effect.target)
        .map_err(|_| authorization_failure("workspaces.launcher.target_authority_changed"))?;
    authority
        .revalidate()
        .map_err(|_| authorization_failure("workspaces.launcher.target_authority_changed"))?;
    #[cfg(unix)]
    {
        if authority.unix_identity() != (_expected.volume, _expected.file) {
            return Err(authorization_failure(
                "workspaces.launcher.target_authority_changed",
            ));
        }
        let (path, inherited) = authority
            .inherited_argument_path()
            .map_err(|_| workspace_fs_failure())?;
        Ok((path, authority, inherited))
    }
    #[cfg(not(unix))]
    {
        // Git accepts only a pathname and would reopen the leaf after this
        // check. Windows exposes no portable handle-relative spelling that Git
        // can consume, so destructive registered-worktree cleanup fails closed.
        let _ = authority;
        Err(authorization_failure(
            "workspaces.launcher.target_authority_changed",
        ))
    }
}

fn owned_target_is_nonempty(
    private_root: &AuthorizedLocalRoot,
    effect: &WorktreeEffect,
) -> Result<bool, WorkspaceCreateFailure> {
    let Some(expected) = effect.target_identity else {
        return Ok(false);
    };
    let target = private_root
        .command_path()
        .join(effect.workspace.to_string());
    let reservation = private_root.command_path().join(&effect.reservation);
    let candidate = [target, reservation]
        .into_iter()
        .find(|candidate| file_identity(candidate).ok() == Some(expected));
    let Some(candidate) = candidate else {
        return Ok(false);
    };
    let mut entries = std::fs::read_dir(candidate).map_err(|_| workspace_fs_failure())?;
    Ok(entries
        .next()
        .transpose()
        .map_err(|_| workspace_fs_failure())?
        .is_some())
}

async fn recover_owned_intents(retained_roots: Vec<PathBuf>) -> Result<(), WorkspaceCreateFailure> {
    let private_root = shelldeck_core::config::AppConfig::config_dir().join("workspace-checkouts");
    recover_owned_intents_at(&private_root, retained_roots).await
}

async fn recover_owned_intents_at(
    private_root: &Path,
    retained_roots: Vec<PathBuf>,
) -> Result<(), WorkspaceCreateFailure> {
    if !private_root.exists() {
        return Ok(());
    }
    let authority = ensure_private_root(private_root)?;
    let retained: HashSet<_> = retained_roots.into_iter().collect();
    let entries = std::fs::read_dir(authority.command_path().join(".intents"))
        .map_err(|_| workspace_fs_failure())?;
    for entry in entries {
        let entry = entry.map_err(|_| workspace_fs_failure())?;
        let metadata =
            std::fs::symlink_metadata(entry.path()).map_err(|_| workspace_fs_failure())?;
        if !metadata.is_file()
            || metadata.file_type().is_symlink()
            || is_windows_reparse(&metadata)
            || entry.path().extension().and_then(|value| value.to_str()) != Some("json")
        {
            continue;
        }
        let workspace = entry
            .path()
            .file_stem()
            .and_then(|value| value.to_str())
            .and_then(|value| uuid::Uuid::parse_str(value).ok())
            .map(CatalogWorkspaceId::from_uuid)
            .ok_or_else(workspace_fs_failure)?;
        let mut effect = read_intent(&authority, workspace)?.ok_or_else(workspace_fs_failure)?;
        if effect.target.parent() != Some(private_root)
            || effect.target.file_name().and_then(|value| value.to_str())
                != Some(&effect.workspace.to_string())
        {
            return Err(authorization_failure(
                "workspaces.launcher.target_authority_changed",
            ));
        }
        if retained.contains(&effect.target) {
            let never_cancelled = AtomicBool::new(false);
            if !exact_complete_worktree(
                &effect.source,
                &effect.target,
                &effect.branch,
                &effect.oid,
                effect.source_repository,
                effect.target_identity,
                &never_cancelled,
            )
            .await?
            {
                return Err(authorization_failure(
                    "workspaces.launcher.target_authority_changed",
                ));
            }
            if !effect.committed {
                effect.committed = true;
                write_intent(&authority, &effect, false)?;
            }
        } else {
            cleanup_owned_effect(effect).await?;
        }
    }
    Ok(())
}

fn effect_matches_request(
    effect: &WorktreeEffect,
    workspace: CatalogWorkspaceId,
    source: &Path,
    target: &Path,
    branch: &str,
    oid: &str,
    source_repository: FileIdentity,
) -> bool {
    effect.workspace == workspace
        && effect.source == source
        && effect.target == target
        && effect.branch == branch
        && effect.oid.eq_ignore_ascii_case(oid)
        && effect.source_repository == source_repository
}

fn file_identity(path: &Path) -> Result<FileIdentity, WorkspaceCreateFailure> {
    #[cfg(unix)]
    {
        use rustix::fs::{fstat, open, Mode, OFlags};
        let descriptor = open(
            path,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(|_| workspace_fs_failure())?;
        let stat = fstat(&descriptor).map_err(|_| workspace_fs_failure())?;
        Ok(FileIdentity {
            volume: stat.st_dev as u64,
            file: stat.st_ino as u64,
        })
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt as _;
        use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
        use windows_sys::Win32::Storage::FileSystem::{
            CreateFileW, GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
            FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_BACKUP_SEMANTICS,
            FILE_FLAG_OPEN_REPARSE_POINT, FILE_READ_ATTRIBUTES, FILE_SHARE_READ, FILE_SHARE_WRITE,
            OPEN_EXISTING,
        };

        let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        // SAFETY: `wide` is a live NUL-terminated UTF-16 path. Opening the
        // reparse point itself lets us reject it instead of following it.
        let handle = unsafe {
            CreateFileW(
                wide.as_ptr(),
                FILE_READ_ATTRIBUTES,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                std::ptr::null(),
                OPEN_EXISTING,
                FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
                std::ptr::null_mut(),
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            return Err(workspace_fs_failure());
        }
        let mut information = BY_HANDLE_FILE_INFORMATION::default();
        // SAFETY: `handle` is valid and `information` is writable for the
        // duration of the call. It is closed on every branch below.
        let read = unsafe { GetFileInformationByHandle(handle, &mut information) };
        unsafe { CloseHandle(handle) };
        if read == 0
            || information.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY == 0
            || information.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0
        {
            return Err(authorization_failure(
                "workspaces.launcher.target_authority_changed",
            ));
        }
        return Ok(FileIdentity {
            volume: information.dwVolumeSerialNumber as u64,
            file: ((information.nFileIndexHigh as u64) << 32) | information.nFileIndexLow as u64,
        });
    }
    #[cfg(not(any(unix, windows)))]
    {
        let handle = same_file::Handle::from_path(path).map_err(|_| workspace_fs_failure())?;
        let encoded = format!("{handle:?}");
        use std::hash::{Hash as _, Hasher as _};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        encoded.hash(&mut hasher);
        Ok(FileIdentity {
            volume: 0,
            file: hasher.finish(),
        })
    }
}

fn validate_regular_nofollow(path: &Path) -> Result<(), WorkspaceCreateFailure> {
    let metadata = std::fs::symlink_metadata(path).map_err(|_| workspace_fs_failure())?;
    if metadata.file_type().is_symlink() || is_windows_reparse(&metadata) || !metadata.is_file() {
        return Err(authorization_failure(
            "workspaces.launcher.target_authority_changed",
        ));
    }
    Ok(())
}

fn open_regular_nofollow(path: &Path) -> Result<std::fs::File, WorkspaceCreateFailure> {
    validate_regular_nofollow(path)?;
    #[cfg(unix)]
    {
        use rustix::fs::{open, Mode, OFlags};
        let descriptor = open(
            path,
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(|_| workspace_fs_failure())?;
        let metadata = rustix::fs::fstat(&descriptor).map_err(|_| workspace_fs_failure())?;
        if rustix::fs::FileType::from_raw_mode(metadata.st_mode)
            != rustix::fs::FileType::RegularFile
        {
            return Err(authorization_failure(
                "workspaces.launcher.target_authority_changed",
            ));
        }
        Ok(std::fs::File::from(descriptor))
    }
    #[cfg(not(unix))]
    std::fs::File::open(path).map_err(|_| workspace_fs_failure())
}

fn sync_directory(_path: &Path) -> Result<(), WorkspaceCreateFailure> {
    #[cfg(unix)]
    {
        let directory = std::fs::File::open(_path).map_err(|_| workspace_fs_failure())?;
        directory.sync_all().map_err(|_| workspace_fs_failure())?;
    }
    Ok(())
}

#[cfg(unix)]
fn secure_create_directory_chain(path: &Path) -> Result<(), WorkspaceCreateFailure> {
    use rustix::fs::{mkdirat, open, openat, Mode, OFlags};
    use std::path::Component;

    let flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
    let mut directory = open("/", flags, Mode::empty()).map_err(|_| workspace_fs_failure())?;
    for component in path.components() {
        match component {
            Component::RootDir => {}
            Component::Normal(name) => {
                match mkdirat(&directory, name, Mode::from_raw_mode(0o700)) {
                    Ok(()) | Err(rustix::io::Errno::EXIST) => {}
                    Err(_) => return Err(workspace_fs_failure()),
                }
                directory = openat(&directory, name, flags, Mode::empty()).map_err(|_| {
                    authorization_failure("workspaces.launcher.target_authority_changed")
                })?;
            }
            _ => {
                return Err(authorization_failure(
                    "workspaces.launcher.target_authority_changed",
                ))
            }
        }
    }
    Ok(())
}

fn path_from_output(bytes: &[u8]) -> Result<PathBuf, WorkspaceCreateFailure> {
    let value = String::from_utf8(bytes.to_vec()).map_err(|_| git_repo_failure())?;
    Ok(PathBuf::from(value.trim()))
}

fn receiver_closed() -> WorkspaceCreateFailure {
    WorkspaceCreateFailure {
        kind: WorkspaceCreateFailureKind::Unknown,
        message: "workspace progress receiver closed".into(),
        retryable: true,
    }
}

fn workspace_fs_failure() -> WorkspaceCreateFailure {
    filesystem_failure("workspaces.launcher.folder_unavailable", true)
}

fn git_runtime_failure() -> WorkspaceCreateFailure {
    runtime_unavailable("workspaces.launcher.git_unavailable")
}

fn git_repo_failure() -> WorkspaceCreateFailure {
    filesystem_failure("workspaces.launcher.git_repository_required", false)
}

fn invalid_branch_failure() -> WorkspaceCreateFailure {
    authorization_failure("workspaces.launcher.invalid_branch")
}

fn authorization_failure(key: &str) -> WorkspaceCreateFailure {
    WorkspaceCreateFailure {
        kind: WorkspaceCreateFailureKind::Authorization,
        message: t!(key).to_string(),
        retryable: false,
    }
}

fn filesystem_failure(key: &str, retryable: bool) -> WorkspaceCreateFailure {
    WorkspaceCreateFailure {
        kind: WorkspaceCreateFailureKind::Filesystem,
        message: t!(key).to_string(),
        retryable,
    }
}

fn runtime_unavailable(key: &str) -> WorkspaceCreateFailure {
    WorkspaceCreateFailure {
        kind: WorkspaceCreateFailureKind::RuntimeUnavailable,
        message: t!(key).to_string(),
        retryable: true,
    }
}

fn bounded_error(value: &str) -> String {
    value.chars().take(512).collect()
}

const CANCELLED_MESSAGE: &str = "workspace operation cancelled";

#[cfg(windows)]
fn is_windows_reparse(metadata: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    metadata.file_attributes() & 0x400 != 0
}

#[cfg(not(windows))]
fn is_windows_reparse(_metadata: &std::fs::Metadata) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command as StdCommand;

    fn git(root: &Path, args: &[&str]) -> String {
        let output = StdCommand::new("git")
            .args(args)
            .current_dir(root)
            .output()
            .expect("git available for lifecycle integration test");
        assert!(
            output.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap().trim().to_owned()
    }

    fn repository(root: &Path) -> (String, String) {
        std::fs::create_dir(root).unwrap();
        git(root, &["init", "-b", "main"]);
        git(root, &["config", "user.name", "ShellDeck Test"]);
        git(root, &["config", "user.email", "shelldeck@example.invalid"]);
        std::fs::write(root.join("tracked.txt"), "one\n").unwrap();
        git(root, &["add", "tracked.txt"]);
        git(root, &["commit", "-m", "one"]);
        let first = git(root, &["rev-parse", "HEAD"]);
        std::fs::write(root.join("tracked.txt"), "two\n").unwrap();
        git(root, &["commit", "-am", "two"]);
        let second = git(root, &["rev-parse", "HEAD"]);
        (first, second)
    }

    fn worktree_request(
        workspace: CatalogWorkspaceId,
        source: PathBuf,
        target: PathBuf,
        start_point: String,
    ) -> WorkspaceExecutionRequest {
        WorkspaceExecutionRequest {
            workspace,
            project: CatalogProjectId::new(),
            source_checkout: CatalogCheckoutId::new(),
            checkout: CatalogCheckoutId::new(),
            created_checkout: None,
            operation: CreationOperationId::new(),
            catalog_revision: 1,
            name: "Lifecycle test".into(),
            intake: WorkspaceLaunchIntake::Manual,
            host: AuthorizedLaunchHost::LocalWorktree {
                source_authority: AuthorizedLocalRoot::capture(&source).unwrap(),
                target_root: target,
                branch: format!("fix/test-{workspace}"),
                start_point,
            },
            mode: WorkspaceLaunchMode::GitWorktree,
        }
    }

    // SDTEST-1763 — SDUC-491
    #[tokio::test]
    async fn sdtest_1763_durable_journal_restarts_only_an_exact_clean_worktree() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        let (first, _) = repository(&source);
        let private = temp.path().join("private");
        let workspace = CatalogWorkspaceId::new();
        let target = private.join(workspace.to_string());
        let request = worktree_request(
            workspace,
            std::fs::canonicalize(&source).unwrap(),
            target.clone(),
            first,
        );
        let cancelled = AtomicBool::new(false);
        let NativeLaunchOutcome::Ready(receipt) =
            prepare_git_worktree(&request, &cancelled).await.unwrap()
        else {
            panic!("worktree was not prepared")
        };
        drop(receipt);

        recover_owned_intents_at(&private, vec![target.clone()])
            .await
            .unwrap();
        let authority = ensure_private_root(&private).unwrap();
        let effect = read_intent(&authority, workspace).unwrap().unwrap();
        assert!(
            effect.committed,
            "restart must durably adopt the catalogued effect"
        );

        std::fs::write(target.join("untracked.txt"), "user data\n").unwrap();
        assert!(recover_owned_intents_at(&private, vec![target.clone()])
            .await
            .is_err());
        assert!(target.join("untracked.txt").exists());
        assert!(intent_path(&authority, workspace).exists());

        std::fs::remove_file(target.join("untracked.txt")).unwrap();
        cleanup_owned_effect(effect).await.unwrap();
        assert!(!target.exists());
    }

    // SDTEST-1764 — SDUC-491
    #[tokio::test]
    async fn sdtest_1764_adoption_refuses_detached_wrong_oid_wrong_repo_and_dirty_targets() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        let (first, second) = repository(&source);
        let private = temp.path().join("private");
        let workspace = CatalogWorkspaceId::new();
        let target = private.join(workspace.to_string());
        let request = worktree_request(
            workspace,
            std::fs::canonicalize(&source).unwrap(),
            target.clone(),
            first.clone(),
        );
        let cancelled = AtomicBool::new(false);
        let NativeLaunchOutcome::Ready(receipt) =
            prepare_git_worktree(&request, &cancelled).await.unwrap()
        else {
            panic!("worktree was not prepared")
        };
        let effect = receipt.cleanup.unwrap();

        git(&target, &["checkout", "--detach", &first]);
        assert!(!exact_complete_worktree(
            &source,
            &target,
            &effect.branch,
            &effect.oid,
            effect.source_repository,
            effect.target_identity,
            &cancelled,
        )
        .await
        .unwrap());

        git(
            &target,
            &[
                "symbolic-ref",
                "HEAD",
                &format!("refs/heads/{}", effect.branch),
            ],
        );
        git(&target, &["reset", "--hard", &second]);
        assert!(!exact_complete_worktree(
            &source,
            &target,
            &effect.branch,
            &effect.oid,
            effect.source_repository,
            effect.target_identity,
            &cancelled,
        )
        .await
        .unwrap());

        git(&target, &["reset", "--hard", &first]);
        std::fs::write(target.join("dirty.txt"), "dirty\n").unwrap();
        assert!(!exact_complete_worktree(
            &source,
            &target,
            &effect.branch,
            &effect.oid,
            effect.source_repository,
            effect.target_identity,
            &cancelled,
        )
        .await
        .unwrap());
        std::fs::remove_file(target.join("dirty.txt")).unwrap();

        git(
            &source,
            &[
                "worktree",
                "remove",
                "--force",
                "--",
                target.to_str().unwrap(),
            ],
        );
        std::fs::create_dir(&target).unwrap();
        git(&target, &["init", "-b", "main"]);
        assert!(!exact_complete_worktree(
            &source,
            &target,
            &effect.branch,
            &effect.oid,
            effect.source_repository,
            effect.target_identity,
            &cancelled,
        )
        .await
        .unwrap());
        assert!(cleanup_owned_effect(effect).await.is_err());
        assert!(
            target.exists(),
            "a substituted repository must never be deleted"
        );
    }

    // SDTEST-1765 — SDUC-491
    #[cfg(unix)]
    #[test]
    fn sdtest_1765_private_root_and_target_authority_never_follow_symlinks() {
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().unwrap();
        let actual = temp.path().join("actual");
        let alias = temp.path().join("alias");
        std::fs::create_dir(&actual).unwrap();
        symlink(&actual, &alias).unwrap();
        assert!(ensure_private_root(&alias).is_err());
        assert!(AuthorizedLocalRoot::capture(&alias).is_err());

        let private = temp.path().join("private");
        let authority = ensure_private_root(&private).unwrap();
        let moved = temp.path().join("moved");
        std::fs::rename(&private, &moved).unwrap();
        let attacker = temp.path().join("attacker");
        std::fs::create_dir(&attacker).unwrap();
        symlink(&attacker, &private).unwrap();
        let protected_child = authority.command_path().join("protected");
        std::fs::create_dir(&protected_child).unwrap();
        assert!(moved.join("protected").is_dir());
        assert!(!attacker.join("protected").exists());
        assert!(authority.revalidate().is_err());
    }

    // SDTEST-1770 — SDUC-491
    #[cfg(unix)]
    #[tokio::test]
    async fn sdtest_1770_cleanup_binds_leaf_and_rechecks_quarantine_identity() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        let (first, _) = repository(&source);
        let private = temp.path().join("private");
        let workspace = CatalogWorkspaceId::new();
        let target = private.join(workspace.to_string());
        let request = worktree_request(
            workspace,
            std::fs::canonicalize(&source).unwrap(),
            target.clone(),
            first,
        );
        let outcome = prepare_git_worktree(&request, &AtomicBool::new(false))
            .await
            .unwrap();
        let NativeLaunchOutcome::Ready(receipt) = outcome else {
            panic!("expected a ready worktree");
        };
        let effect = receipt.cleanup.unwrap();
        let (bound, _authority, _inherited) = bound_worktree_argument(&effect).unwrap();
        let moved = private.join("moved-owned-worktree");
        std::fs::rename(&target, &moved).unwrap();
        std::fs::create_dir(&target).unwrap();
        assert_eq!(std::fs::canonicalize(bound).unwrap(), moved);

        let private_authority = ensure_private_root(&private).unwrap();
        let owned = private.join("owned");
        std::fs::create_dir(&owned).unwrap();
        let expected = file_identity(&owned).unwrap();
        let quarantine = private.join(".cleanup-substituted");
        std::fs::create_dir(&quarantine).unwrap();
        std::fs::write(quarantine.join("must-survive"), "unowned").unwrap();
        assert!(remove_verified_quarantine(&private_authority, &quarantine, expected).is_err());
        assert!(quarantine.join("must-survive").is_file());
    }

    struct GatedGitAdapter {
        release: Arc<tokio::sync::Semaphore>,
        target: PathBuf,
    }

    impl GitWorktreeAdapter for GatedGitAdapter {
        fn prepare(
            &self,
            _request: WorkspaceExecutionRequest,
            _cancelled: Arc<LaunchCancellation>,
        ) -> ExecutorFuture<Result<NativeLaunchOutcome, WorkspaceCreateFailure>> {
            let release = self.release.clone();
            let target = self.target.clone();
            Box::pin(async move {
                let permit = release.acquire().await.unwrap();
                permit.forget();
                std::fs::create_dir_all(&target).unwrap();
                Ok(NativeLaunchOutcome::test_ready(&target))
            })
        }
    }

    // SDTEST-1766 — SDUC-491
    #[tokio::test]
    async fn sdtest_1766_closed_ui_receiver_cannot_publish_or_leak_a_ready_receipt() {
        let temp = tempfile::tempdir().unwrap();
        let source = std::fs::canonicalize(temp.path()).unwrap();
        let target = source.join("target");
        let workspace = CatalogWorkspaceId::new();
        let request = worktree_request(workspace, source, target.clone(), "HEAD".into());
        let operation = request.operation;
        let release = Arc::new(tokio::sync::Semaphore::new(0));
        let executor = NativeWorkspaceExecutor::with_git(Arc::new(GatedGitAdapter {
            release: release.clone(),
            target,
        }));
        let (tx, mut rx) = mpsc::unbounded_channel();
        let launch = tokio::spawn(executor.launch(request, tx));
        for _ in 0..3 {
            assert!(matches!(
                rx.recv().await,
                Some(WorkspaceCreateEvent::Progress { .. })
            ));
        }
        drop(rx);
        release.add_permits(1);
        let error = launch.await.unwrap().unwrap_err();
        assert_eq!(error.message, receiver_closed().message);
        assert!(executor.take_receipt(operation).is_none());
    }

    // SDTEST-1767 — SDUC-491
    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn sdtest_1767_cancellation_terminates_and_reaps_the_entire_process_group() {
        let temp = tempfile::tempdir().unwrap();
        let pid_path = temp.path().join("grandchild.pid");
        let mut command = StdCommand::new("sh");
        command
            .args([
                "-c",
                "sleep 30 & child=$!; printf '%s' \"$child\" > \"$1\"; wait",
                "shelldeck-cancel-test",
            ])
            .arg(&pid_path)
            .stdin(Stdio::null());
        shelldeck_core::agent_runtime::configure_local_process(&mut command);
        let cancelled = AtomicBool::new(false);
        let cancel_when_ready = async {
            for _ in 0..400 {
                if pid_path.exists() {
                    cancelled.store(true, Ordering::Release);
                    return;
                }
                async_io::Timer::after(Duration::from_millis(5)).await;
            }
            panic!("process tree did not publish its grandchild pid")
        };
        let (result, ()) = futures_lite::future::zip(
            run_command(Command::from(command), &cancelled),
            cancel_when_ready,
        )
        .await;
        assert_eq!(result.unwrap_err().message, CANCELLED_MESSAGE);
        let grandchild = std::fs::read_to_string(&pid_path).unwrap();
        let grandchild = grandchild.trim().parse::<u32>().unwrap();
        let proc_entry = PathBuf::from(format!("/proc/{grandchild}"));
        for _ in 0..400 {
            if !proc_entry.exists() {
                return;
            }
            async_io::Timer::after(Duration::from_millis(5)).await;
        }
        panic!("cancelled grandchild {grandchild} remained alive or unreaped");
    }

    // SDTEST-1768 — SDUC-491
    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn sdtest_1768_descendant_pipe_holder_cannot_deadlock_command_completion() {
        let temp = tempfile::tempdir().unwrap();
        let pid_path = temp.path().join("grandchild.pid");
        let mut command = StdCommand::new("sh");
        command
            .args([
                "-c",
                "sleep 30 & child=$!; printf '%s' \"$child\" > \"$1\"; exit 0",
                "shelldeck-drain-test",
            ])
            .arg(&pid_path)
            .stdin(Stdio::null());
        shelldeck_core::agent_runtime::configure_local_process(&mut command);
        let started = Instant::now();
        let error = run_command(Command::from(command), &AtomicBool::new(false))
            .await
            .unwrap_err();
        assert_eq!(error.message, t!("workspaces.launcher.worktree_timeout"));
        assert!(started.elapsed() < Duration::from_secs(5));
        let grandchild = std::fs::read_to_string(&pid_path)
            .unwrap()
            .trim()
            .parse::<u32>()
            .unwrap();
        let proc_entry = PathBuf::from(format!("/proc/{grandchild}"));
        for _ in 0..400 {
            if !proc_entry.exists() {
                return;
            }
            async_io::Timer::after(Duration::from_millis(5)).await;
        }
        panic!("pipe-holding grandchild {grandchild} remained alive or unreaped");
    }
}
