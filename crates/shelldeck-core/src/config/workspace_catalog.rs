//! Persistent ShellDeck project, checkout, and user-workspace catalog.
//!
//! Catalog identities are deliberately local. Platform v2 identities are
//! stored only in an explicit, revisioned reconciliation mapping, so a local
//! record can never be mistaken for an authoritative Automonique context.

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::collections::BTreeSet;
use std::fmt;
use std::path::{Component, Path, PathBuf};
use uuid::Uuid;

use crate::error::{Result, ShellDeckError};

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

uuid_id!(CatalogProjectId);
uuid_id!(CatalogCheckoutId);
uuid_id!(CatalogWorkspaceId);

const CATALOG_SCHEMA_VERSION: u16 = 3;
const MAX_PLATFORM_ID_BYTES: usize = 256;
static CATALOG_SAVE_LOCK: parking_lot::Mutex<()> = parking_lot::Mutex::new(());

/// Slash-separated path interpreted by the selected checkout host.
/// Backslashes, absolute roots, prefixes, `.` and `..` are refused.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct WorkspaceRelativePath(String);

impl WorkspaceRelativePath {
    pub fn new(value: impl Into<String>) -> std::result::Result<Self, WorkspaceCatalogError> {
        let value = value.into();
        if value.is_empty() {
            return Ok(Self(value));
        }
        if value.starts_with('/')
            || value.contains('\\')
            || value.contains('\0')
            || value
                .split('/')
                .any(|part| part.is_empty() || part == "." || part == "..")
            || value.as_bytes().get(1) == Some(&b':')
        {
            return Err(WorkspaceCatalogError::InvalidRelativePath(value));
        }
        Ok(Self(value))
    }
    #[must_use]
    pub const fn as_str(&self) -> &str {
        self.0.as_str()
    }
    #[must_use]
    pub fn to_local_path(&self) -> PathBuf {
        self.0.split('/').filter(|part| !part.is_empty()).collect()
    }
}
impl Serialize for WorkspaceRelativePath {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.0)
    }
}
impl<'de> Deserialize<'de> for WorkspaceRelativePath {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::new(String::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

/// Absolute POSIX path interpreted by the remote SSH host, never by the client OS.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct RemotePosixPath(String);
impl RemotePosixPath {
    pub fn new(value: impl Into<String>) -> std::result::Result<Self, WorkspaceCatalogError> {
        let value = value.into();
        if !value.starts_with('/')
            || value.contains('\\')
            || value.contains('\0')
            || value
                .split('/')
                .skip(1)
                .any(|part| part == "." || part == "..")
        {
            return Err(WorkspaceCatalogError::InvalidRemotePath(value));
        }
        Ok(Self(value))
    }
    #[must_use]
    pub const fn as_str(&self) -> &str {
        self.0.as_str()
    }
}
impl Serialize for RemotePosixPath {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.0)
    }
}
impl<'de> Deserialize<'de> for RemotePosixPath {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::new(String::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RepositoryIdentity {
    pub slug: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub canonical_url: Option<String>,
}

/// ShellDeck-owned host and path authority for one checkout.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CheckoutHost {
    Local {
        device_label: String,
        root: PathBuf,
    },
    Ssh {
        connection_id: Uuid,
        root: RemotePosixPath,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ProjectCheckout {
    id: CatalogCheckoutId,
    label: String,
    host: CheckoutHost,
    repository: RepositoryIdentity,
}
impl ProjectCheckout {
    pub fn new(
        id: CatalogCheckoutId,
        label: impl Into<String>,
        host: CheckoutHost,
        repository: RepositoryIdentity,
    ) -> Self {
        Self {
            id,
            label: label.into(),
            host,
            repository,
        }
    }
    #[must_use]
    pub const fn id(&self) -> CatalogCheckoutId {
        self.id
    }
    #[must_use]
    pub const fn label(&self) -> &str {
        self.label.as_str()
    }
    #[must_use]
    pub const fn host(&self) -> &CheckoutHost {
        &self.host
    }
    #[must_use]
    pub const fn repository(&self) -> &RepositoryIdentity {
        &self.repository
    }

    /// Resolve an existing local entry beneath this checkout and return only
    /// its canonical, containment-checked path.
    ///
    /// Callers must open the returned path rather than reconstructing it from
    /// the untrusted relative input. This rejects symlink escapes before the
    /// open. Executors operating on an adversarially mutable filesystem should
    /// additionally use handle-relative/no-follow APIs to close rename races.
    pub fn resolve_existing_local_path(
        &self,
        relative: &WorkspaceRelativePath,
    ) -> std::result::Result<AuthorizedLocalPath, WorkspaceCatalogError> {
        let CheckoutHost::Local { root, .. } = &self.host else {
            return Err(WorkspaceCatalogError::ExpectedLocalCheckout);
        };
        let canonical_root = std::fs::canonicalize(root)
            .map_err(|_| WorkspaceCatalogError::LocalPathUnavailable(root.clone()))?;
        let candidate = root.join(relative.to_local_path());
        let canonical_candidate = std::fs::canonicalize(&candidate)
            .map_err(|_| WorkspaceCatalogError::LocalPathUnavailable(candidate.clone()))?;
        if !canonical_candidate.starts_with(&canonical_root) {
            return Err(WorkspaceCatalogError::LocalPathEscapesCheckout(
                canonical_candidate,
            ));
        }
        Ok(AuthorizedLocalPath(canonical_candidate))
    }

    /// Delegate a remote operation to an SSH adapter whose contract guarantees
    /// server-side beneath/no-follow resolution. ShellDeck core intentionally
    /// never constructs or returns a joined remote path.
    pub fn execute_remote_beneath<E: SshBeneathExecutor>(
        &self,
        relative: &WorkspaceRelativePath,
        request: E::Request,
        executor: &E,
    ) -> std::result::Result<E::Output, E::Error>
    where
        E::Error: From<WorkspaceCatalogError>,
    {
        let CheckoutHost::Ssh {
            connection_id,
            root,
        } = &self.host
        else {
            return Err(WorkspaceCatalogError::ExpectedSshCheckout.into());
        };
        executor.execute_beneath(
            RemoteBeneathAuthority {
                connection_id: *connection_id,
                root,
                relative,
            },
            request,
        )
    }
}

/// Canonical local path admitted beneath a catalog checkout.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuthorizedLocalPath(PathBuf);
impl AuthorizedLocalPath {
    #[must_use]
    pub fn as_path(&self) -> &Path {
        &self.0
    }
}

/// Unjoined SSH path authority supplied only to a beneath/no-follow adapter.
pub struct RemoteBeneathAuthority<'a> {
    connection_id: Uuid,
    root: &'a RemotePosixPath,
    relative: &'a WorkspaceRelativePath,
}
impl RemoteBeneathAuthority<'_> {
    #[must_use]
    pub const fn connection_id(&self) -> Uuid {
        self.connection_id
    }
    #[must_use]
    pub const fn root(&self) -> &RemotePosixPath {
        self.root
    }
    #[must_use]
    pub const fn relative(&self) -> &WorkspaceRelativePath {
        self.relative
    }
}

/// SSH executor boundary for path operations below an authorized remote root.
/// Implementations must use a server-side no-follow/beneath primitive and must
/// not interpolate `root` plus `relative` into a shell command.
pub trait SshBeneathExecutor {
    type Request;
    type Output;
    type Error;

    fn execute_beneath(
        &self,
        authority: RemoteBeneathAuthority<'_>,
        request: Self::Request,
    ) -> std::result::Result<Self::Output, Self::Error>;
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ProjectRecord {
    id: CatalogProjectId,
    name: String,
    #[serde(default)]
    checkouts: Vec<ProjectCheckout>,
}
impl ProjectRecord {
    pub fn new(id: CatalogProjectId, name: impl Into<String>) -> Self {
        Self {
            id,
            name: name.into(),
            checkouts: Vec::new(),
        }
    }
    pub fn add_checkout(&mut self, checkout: ProjectCheckout) {
        self.checkouts.push(checkout);
    }
    #[must_use]
    pub const fn id(&self) -> CatalogProjectId {
        self.id
    }
    #[must_use]
    pub const fn name(&self) -> &str {
        self.name.as_str()
    }
    pub fn checkouts(&self) -> impl ExactSizeIterator<Item = &ProjectCheckout> {
        self.checkouts.iter()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExternalWorkItemKind {
    Issue,
    PullRequest,
    Task,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ExternalWorkItem {
    pub provider: String,
    pub repository: String,
    pub kind: ExternalWorkItemKind,
    pub key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PlatformContextRef {
    pub id: String,
    pub revision: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum PlatformMappingReconciliation {
    Pending,
    Exact { reconciled_at_millis: u64 },
    Diverged { observed_at_millis: u64 },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PlatformV2Mapping {
    /// Monotonic revision of the complete reconciliation observation. This is
    /// distinct from the three Platform context revisions because identities
    /// can change during an explicitly fenced reconciliation.
    #[serde(default = "initial_reconciliation_revision")]
    pub reconciliation_revision: u64,
    pub project: PlatformContextRef,
    pub checkout: PlatformContextRef,
    pub user_workspace: PlatformContextRef,
    pub reconciliation: PlatformMappingReconciliation,
}
impl PlatformV2Mapping {
    #[must_use]
    pub const fn is_exact(&self) -> bool {
        matches!(
            self.reconciliation,
            PlatformMappingReconciliation::Exact { .. }
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct OrchestrationRunRef {
    pub runtime: String,
    pub run_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    pub platform_user_workspace_id: String,
}

/// Schema-v1 run metadata retained for display/migration only.
///
/// It deliberately has no Platform workspace identity and therefore can never
/// authorize a provider session. A successful v2 binding replaces it.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct LegacyOrchestrationRunRef {
    pub runtime: String,
    pub run_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WorkspaceLaunchIntake {
    Manual,
    Prefilled(ExternalWorkItem),
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkspaceLaunchRequest {
    pub id: CatalogWorkspaceId,
    pub project_id: CatalogProjectId,
    pub checkout_id: CatalogCheckoutId,
    pub name: String,
    pub intake: WorkspaceLaunchIntake,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UserWorkspaceLifecycle {
    Active,
    Archived,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct UserWorkspaceRecord {
    id: CatalogWorkspaceId,
    project_id: CatalogProjectId,
    checkout_id: CatalogCheckoutId,
    name: String,
    lifecycle: UserWorkspaceLifecycle,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    linked_work_item: Option<ExternalWorkItem>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    platform_mapping: Option<PlatformV2Mapping>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    orchestration_run: Option<OrchestrationRunRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    legacy_orchestration_run: Option<LegacyOrchestrationRunRef>,
}
impl UserWorkspaceRecord {
    #[must_use]
    pub const fn id(&self) -> CatalogWorkspaceId {
        self.id
    }
    #[must_use]
    pub const fn project_id(&self) -> CatalogProjectId {
        self.project_id
    }
    #[must_use]
    pub const fn checkout_id(&self) -> CatalogCheckoutId {
        self.checkout_id
    }
    #[must_use]
    pub const fn name(&self) -> &str {
        self.name.as_str()
    }
    #[must_use]
    pub const fn lifecycle(&self) -> UserWorkspaceLifecycle {
        self.lifecycle
    }
    #[must_use]
    pub const fn linked_work_item(&self) -> Option<&ExternalWorkItem> {
        self.linked_work_item.as_ref()
    }
    #[must_use]
    pub const fn platform_mapping(&self) -> Option<&PlatformV2Mapping> {
        self.platform_mapping.as_ref()
    }
    #[must_use]
    pub const fn orchestration_run(&self) -> Option<&OrchestrationRunRef> {
        self.orchestration_run.as_ref()
    }
    #[must_use]
    pub const fn legacy_orchestration_run(&self) -> Option<&LegacyOrchestrationRunRef> {
        self.legacy_orchestration_run.as_ref()
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
struct CatalogDiskV3 {
    schema_version: u16,
    revision: u64,
    #[serde(default)]
    projects: Vec<ProjectRecord>,
    #[serde(default)]
    workspaces: Vec<UserWorkspaceRecord>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ProjectCatalog {
    revision: u64,
    persisted_revision: u64,
    projects: Vec<ProjectRecord>,
    workspaces: Vec<UserWorkspaceRecord>,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WorkspaceCatalogError {
    UnsupportedSchema(u16),
    RevisionConflict {
        expected: u64,
        actual: u64,
    },
    RevisionExhausted,
    DuplicateProject(CatalogProjectId),
    DuplicateCheckout(CatalogCheckoutId),
    DuplicateWorkspace(CatalogWorkspaceId),
    UnknownProject(CatalogProjectId),
    UnknownCheckout(CatalogCheckoutId),
    CheckoutOutsideProject {
        project: CatalogProjectId,
        checkout: CatalogCheckoutId,
    },
    UnknownWorkspace(CatalogWorkspaceId),
    BlankName,
    InvalidLocalRoot(PathBuf),
    InvalidRemotePath(String),
    InvalidRelativePath(String),
    InvalidPlatformMapping,
    PlatformMappingRevisionConflict {
        expected: Option<u64>,
        actual: Option<u64>,
    },
    InvalidPlatformMappingTransition,
    StalePlatformMapping,
    DuplicatePlatformWorkspace(String),
    PlatformMappingNotExact(CatalogWorkspaceId),
    ExpectedLocalCheckout,
    ExpectedSshCheckout,
    LocalPathUnavailable(PathBuf),
    LocalPathEscapesCheckout(PathBuf),
}
impl fmt::Display for WorkspaceCatalogError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedSchema(v) => write!(f, "unsupported project catalog schema {v}"),
            Self::RevisionConflict { expected, actual } => write!(
                f,
                "catalog revision conflict: expected {expected}, found {actual}"
            ),
            Self::RevisionExhausted => f.write_str("catalog revision exhausted"),
            Self::DuplicateProject(id) => write!(f, "duplicate project {id}"),
            Self::DuplicateCheckout(id) => write!(f, "duplicate checkout {id}"),
            Self::DuplicateWorkspace(id) => write!(f, "duplicate workspace {id}"),
            Self::UnknownProject(id) => write!(f, "unknown project {id}"),
            Self::UnknownCheckout(id) => write!(f, "unknown checkout {id}"),
            Self::CheckoutOutsideProject { project, checkout } => write!(
                f,
                "checkout {checkout} does not belong to project {project}"
            ),
            Self::UnknownWorkspace(id) => write!(f, "unknown workspace {id}"),
            Self::BlankName => f.write_str("required catalog text cannot be blank"),
            Self::InvalidLocalRoot(path) => {
                write!(f, "local checkout root is not absolute: {}", path.display())
            }
            Self::InvalidRemotePath(path) => write!(f, "invalid remote POSIX path: {path}"),
            Self::InvalidRelativePath(path) => write!(f, "invalid workspace-relative path: {path}"),
            Self::InvalidPlatformMapping => f.write_str("invalid Platform v2 mapping"),
            Self::PlatformMappingRevisionConflict { expected, actual } => write!(
                f,
                "Platform mapping revision conflict: expected {expected:?}, found {actual:?}"
            ),
            Self::InvalidPlatformMappingTransition => {
                f.write_str("invalid Platform mapping reconciliation transition")
            }
            Self::StalePlatformMapping => {
                f.write_str("stale Platform v2 mapping cannot replace newer evidence")
            }
            Self::DuplicatePlatformWorkspace(id) => {
                write!(
                    f,
                    "Platform v2 workspace {id} maps to multiple local records"
                )
            }
            Self::PlatformMappingNotExact(id) => {
                write!(f, "workspace {id} has no exact Platform v2 mapping")
            }
            Self::ExpectedLocalCheckout => f.write_str("checkout is not local"),
            Self::ExpectedSshCheckout => f.write_str("checkout is not SSH"),
            Self::LocalPathUnavailable(path) => {
                write!(f, "checkout path is unavailable: {}", path.display())
            }
            Self::LocalPathEscapesCheckout(path) => write!(
                f,
                "resolved path escapes the authorized checkout: {}",
                path.display()
            ),
        }
    }
}
impl std::error::Error for WorkspaceCatalogError {}

impl ProjectCatalog {
    fn catalog_path() -> PathBuf {
        super::app_config::AppConfig::config_dir().join("project-catalog.json")
    }
    #[must_use]
    pub const fn revision(&self) -> u64 {
        self.revision
    }
    pub fn projects(&self) -> impl ExactSizeIterator<Item = &ProjectRecord> {
        self.projects.iter()
    }
    pub fn workspaces(&self) -> impl ExactSizeIterator<Item = &UserWorkspaceRecord> {
        self.workspaces.iter()
    }
    pub fn save(&mut self) -> Result<()> {
        self.save_to(&Self::catalog_path())
    }
    pub fn load() -> Result<Self> {
        Self::load_from(&Self::catalog_path())
    }

    pub(crate) fn save_to(&mut self, path: &Path) -> Result<()> {
        // Keep the process mutex as a cheap thread-level admission gate, then
        // take a real OS file lock so revision check + replace is one
        // transaction even during simultaneous launches or recovery tools.
        let _save_guard = CATALOG_SAVE_LOCK.lock();
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            std::fs::create_dir_all(parent)?;
        }
        let _file_lock = CatalogFileLock::acquire(path)?;
        self.validate().map_err(catalog_serialization_error)?;
        let actual = disk_revision(path)?;
        if actual != self.persisted_revision {
            return Err(catalog_serialization_error(
                WorkspaceCatalogError::RevisionConflict {
                    expected: self.persisted_revision,
                    actual,
                },
            ));
        }
        let disk = CatalogDiskV3 {
            schema_version: CATALOG_SCHEMA_VERSION,
            revision: self.revision,
            projects: self.projects.clone(),
            workspaces: self.workspaces.clone(),
        };
        let payload = serde_json::to_vec_pretty(&disk).map_err(|error| {
            ShellDeckError::Serialization(format!("failed to serialize project catalog: {error}"))
        })?;
        crate::util::atomic_write(path, &payload)?;
        self.persisted_revision = self.revision;
        Ok(())
    }

    pub(crate) fn load_from(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let value: serde_json::Value =
            serde_json::from_slice(&std::fs::read(path)?).map_err(parse_error)?;
        let schema = schema_version(&value)?;
        let catalog = match schema {
            1 => migrate_v1(value)?,
            2 | CATALOG_SCHEMA_VERSION => {
                // Schema 2 has the same envelope; mappings missing the new
                // complete-reconciliation revision receive the explicit
                // baseline revision one during migration to schema 3.
                let disk: CatalogDiskV3 = serde_json::from_value(value).map_err(parse_error)?;
                Self {
                    revision: disk.revision,
                    persisted_revision: disk.revision,
                    projects: disk.projects,
                    workspaces: disk.workspaces,
                }
            }
            other => {
                return Err(catalog_serialization_error(
                    WorkspaceCatalogError::UnsupportedSchema(other),
                ))
            }
        };
        catalog.validate().map_err(catalog_serialization_error)?;
        Ok(catalog)
    }

    pub fn insert_project(
        &mut self,
        project: ProjectRecord,
    ) -> std::result::Result<(), WorkspaceCatalogError> {
        if self.projects.iter().any(|item| item.id == project.id) {
            return Err(WorkspaceCatalogError::DuplicateProject(project.id));
        }
        let next_revision = self.next_revision()?;
        let mut candidate = self.clone();
        candidate.projects.push(project);
        candidate.validate()?;
        candidate.revision = next_revision;
        *self = candidate;
        Ok(())
    }

    pub fn checkout_in_project(
        &self,
        project_id: CatalogProjectId,
        checkout_id: CatalogCheckoutId,
    ) -> std::result::Result<&ProjectCheckout, WorkspaceCatalogError> {
        let project = self
            .projects
            .iter()
            .find(|project| project.id == project_id)
            .ok_or(WorkspaceCatalogError::UnknownProject(project_id))?;
        if let Some(checkout) = project
            .checkouts
            .iter()
            .find(|checkout| checkout.id == checkout_id)
        {
            return Ok(checkout);
        }
        if self
            .projects
            .iter()
            .flat_map(|project| &project.checkouts)
            .any(|checkout| checkout.id == checkout_id)
        {
            Err(WorkspaceCatalogError::CheckoutOutsideProject {
                project: project_id,
                checkout: checkout_id,
            })
        } else {
            Err(WorkspaceCatalogError::UnknownCheckout(checkout_id))
        }
    }

    pub fn workspace(
        &self,
        id: CatalogWorkspaceId,
    ) -> std::result::Result<&UserWorkspaceRecord, WorkspaceCatalogError> {
        self.workspaces
            .iter()
            .find(|workspace| workspace.id == id)
            .ok_or(WorkspaceCatalogError::UnknownWorkspace(id))
    }

    pub fn create_workspace(
        &mut self,
        request: WorkspaceLaunchRequest,
    ) -> std::result::Result<&UserWorkspaceRecord, WorkspaceCatalogError> {
        if request.name.trim().is_empty() {
            return Err(WorkspaceCatalogError::BlankName);
        }
        if self
            .workspaces
            .iter()
            .any(|workspace| workspace.id == request.id)
        {
            return Err(WorkspaceCatalogError::DuplicateWorkspace(request.id));
        }
        self.checkout_in_project(request.project_id, request.checkout_id)?;
        let linked_work_item = match request.intake {
            WorkspaceLaunchIntake::Manual => None,
            WorkspaceLaunchIntake::Prefilled(item) => {
                validate_work_item(&item)?;
                Some(item)
            }
        };
        let next_revision = self.next_revision()?;
        self.workspaces.push(UserWorkspaceRecord {
            id: request.id,
            project_id: request.project_id,
            checkout_id: request.checkout_id,
            name: request.name.trim().to_owned(),
            lifecycle: UserWorkspaceLifecycle::Active,
            linked_work_item,
            platform_mapping: None,
            orchestration_run: None,
            legacy_orchestration_run: None,
        });
        self.revision = next_revision;
        Ok(self.workspaces.last().expect("workspace inserted"))
    }

    pub fn archive_workspace(
        &mut self,
        id: CatalogWorkspaceId,
    ) -> std::result::Result<(), WorkspaceCatalogError> {
        if self.workspace(id)?.lifecycle == UserWorkspaceLifecycle::Archived {
            return Ok(());
        }
        let next_revision = self.next_revision()?;
        self.workspace_mut(id)?.lifecycle = UserWorkspaceLifecycle::Archived;
        self.revision = next_revision;
        Ok(())
    }
    pub fn resume_workspace(
        &mut self,
        id: CatalogWorkspaceId,
    ) -> std::result::Result<(), WorkspaceCatalogError> {
        if self.workspace(id)?.lifecycle == UserWorkspaceLifecycle::Active {
            return Ok(());
        }
        let next_revision = self.next_revision()?;
        self.workspace_mut(id)?.lifecycle = UserWorkspaceLifecycle::Active;
        self.revision = next_revision;
        Ok(())
    }
    pub fn set_platform_mapping(
        &mut self,
        id: CatalogWorkspaceId,
        expected_prior_revision: Option<u64>,
        mapping: PlatformV2Mapping,
    ) -> std::result::Result<(), WorkspaceCatalogError> {
        validate_mapping(&mapping)?;
        let actual_prior_revision = self
            .workspace(id)?
            .platform_mapping
            .as_ref()
            .map(|mapping| mapping.reconciliation_revision);
        if expected_prior_revision != actual_prior_revision {
            return Err(WorkspaceCatalogError::PlatformMappingRevisionConflict {
                expected: expected_prior_revision,
                actual: actual_prior_revision,
            });
        }
        if let Some(existing) = self.workspace(id)?.platform_mapping.as_ref() {
            if existing == &mapping {
                return Ok(());
            }
            if mapping.reconciliation_revision <= existing.reconciliation_revision {
                return Err(WorkspaceCatalogError::StalePlatformMapping);
            }
            if !mapping_transition_allowed(existing, &mapping) {
                return Err(WorkspaceCatalogError::InvalidPlatformMappingTransition);
            }
            for (old, new) in [
                (&existing.project, &mapping.project),
                (&existing.checkout, &mapping.checkout),
                (&existing.user_workspace, &mapping.user_workspace),
            ] {
                if old.id == new.id && new.revision < old.revision {
                    return Err(WorkspaceCatalogError::StalePlatformMapping);
                }
            }
        }
        if mapping.is_exact()
            && self.workspaces.iter().any(|workspace| {
                workspace.id != id
                    && workspace.platform_mapping.as_ref().is_some_and(|existing| {
                        existing.is_exact()
                            && existing.user_workspace.id == mapping.user_workspace.id
                    })
            })
        {
            return Err(WorkspaceCatalogError::DuplicatePlatformWorkspace(
                mapping.user_workspace.id,
            ));
        }
        let next_revision = self.next_revision()?;
        let workspace = self.workspace_mut(id)?;
        if !mapping.is_exact()
            || workspace
                .orchestration_run
                .as_ref()
                .is_some_and(|run| run.platform_user_workspace_id != mapping.user_workspace.id)
        {
            workspace.orchestration_run = None;
        }
        workspace.platform_mapping = Some(mapping);
        self.revision = next_revision;
        Ok(())
    }
    pub fn bind_orchestration_run(
        &mut self,
        id: CatalogWorkspaceId,
        run: Option<OrchestrationRunRef>,
    ) -> std::result::Result<(), WorkspaceCatalogError> {
        if let Some(run) = run.as_ref() {
            if run.runtime.trim().is_empty()
                || run.run_id.trim().is_empty()
                || run
                    .session_id
                    .as_ref()
                    .is_some_and(|id| id.trim().is_empty())
            {
                return Err(WorkspaceCatalogError::BlankName);
            }
            let mapping = self
                .workspace(id)?
                .platform_mapping
                .as_ref()
                .filter(|mapping| mapping.is_exact())
                .ok_or(WorkspaceCatalogError::PlatformMappingNotExact(id))?;
            if run.platform_user_workspace_id != mapping.user_workspace.id {
                return Err(WorkspaceCatalogError::InvalidPlatformMapping);
            }
        }
        let next_revision = self.next_revision()?;
        let workspace = self.workspace_mut(id)?;
        workspace.orchestration_run = run;
        if workspace.orchestration_run.is_some() {
            workspace.legacy_orchestration_run = None;
        }
        self.revision = next_revision;
        Ok(())
    }

    pub fn validate(&self) -> std::result::Result<(), WorkspaceCatalogError> {
        let mut projects = BTreeSet::new();
        let mut checkouts = BTreeSet::new();
        for project in &self.projects {
            if project.name.trim().is_empty() {
                return Err(WorkspaceCatalogError::BlankName);
            }
            if !projects.insert(project.id) {
                return Err(WorkspaceCatalogError::DuplicateProject(project.id));
            }
            for checkout in &project.checkouts {
                if checkout.label.trim().is_empty() || checkout.repository.slug.trim().is_empty() {
                    return Err(WorkspaceCatalogError::BlankName);
                }
                if !checkouts.insert(checkout.id) {
                    return Err(WorkspaceCatalogError::DuplicateCheckout(checkout.id));
                }
                if let CheckoutHost::Local { root, .. } = &checkout.host {
                    if !root.is_absolute()
                        || root
                            .components()
                            .any(|component| matches!(component, Component::ParentDir))
                    {
                        return Err(WorkspaceCatalogError::InvalidLocalRoot(root.clone()));
                    }
                }
            }
        }
        let mut workspaces = BTreeSet::new();
        let mut exact_platform_workspaces = BTreeSet::new();
        for workspace in &self.workspaces {
            if workspace.name.trim().is_empty() {
                return Err(WorkspaceCatalogError::BlankName);
            }
            if !workspaces.insert(workspace.id) {
                return Err(WorkspaceCatalogError::DuplicateWorkspace(workspace.id));
            }
            self.checkout_in_project(workspace.project_id, workspace.checkout_id)?;
            if let Some(item) = workspace.linked_work_item.as_ref() {
                if item.provider.trim().is_empty()
                    || item.repository.trim().is_empty()
                    || item.key.trim().is_empty()
                {
                    return Err(WorkspaceCatalogError::BlankName);
                }
            }
            if let Some(mapping) = workspace.platform_mapping.as_ref() {
                validate_mapping(mapping)?;
                if mapping.is_exact()
                    && !exact_platform_workspaces.insert(mapping.user_workspace.id.as_str())
                {
                    return Err(WorkspaceCatalogError::DuplicatePlatformWorkspace(
                        mapping.user_workspace.id.clone(),
                    ));
                }
            }
            if let Some(run) = workspace.orchestration_run.as_ref() {
                let mapping = workspace
                    .platform_mapping
                    .as_ref()
                    .filter(|mapping| mapping.is_exact())
                    .ok_or(WorkspaceCatalogError::PlatformMappingNotExact(workspace.id))?;
                if run.platform_user_workspace_id != mapping.user_workspace.id {
                    return Err(WorkspaceCatalogError::InvalidPlatformMapping);
                }
            }
            if let Some(run) = workspace.legacy_orchestration_run.as_ref() {
                if run.runtime.trim().is_empty()
                    || run.run_id.trim().is_empty()
                    || run
                        .session_id
                        .as_ref()
                        .is_some_and(|session| session.trim().is_empty())
                {
                    return Err(WorkspaceCatalogError::BlankName);
                }
            }
        }
        Ok(())
    }

    fn workspace_mut(
        &mut self,
        id: CatalogWorkspaceId,
    ) -> std::result::Result<&mut UserWorkspaceRecord, WorkspaceCatalogError> {
        self.workspaces
            .iter_mut()
            .find(|workspace| workspace.id == id)
            .ok_or(WorkspaceCatalogError::UnknownWorkspace(id))
    }
    fn next_revision(&self) -> std::result::Result<u64, WorkspaceCatalogError> {
        self.revision
            .checked_add(1)
            .ok_or(WorkspaceCatalogError::RevisionExhausted)
    }
}

struct CatalogFileLock(std::fs::File);
impl CatalogFileLock {
    fn acquire(catalog_path: &Path) -> Result<Self> {
        let file = open_catalog_lock(catalog_path)?;
        fs2::FileExt::lock_exclusive(&file)?;
        Ok(Self(file))
    }

    #[cfg(test)]
    fn try_acquire(catalog_path: &Path) -> Result<Option<Self>> {
        let file = open_catalog_lock(catalog_path)?;
        match fs2::FileExt::try_lock_exclusive(&file) {
            Ok(()) => Ok(Some(Self(file))),
            Err(error) if is_lock_contention(&error) => Ok(None),
            Err(error) => Err(error.into()),
        }
    }
}
impl Drop for CatalogFileLock {
    fn drop(&mut self) {
        let _ = fs2::FileExt::unlock(&self.0);
    }
}

#[cfg(test)]
fn is_lock_contention(error: &std::io::Error) -> bool {
    if error.kind() == std::io::ErrorKind::WouldBlock {
        return true;
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::Foundation::ERROR_LOCK_VIOLATION;

        return error.raw_os_error() == Some(ERROR_LOCK_VIOLATION as i32);
    }
    #[cfg(not(windows))]
    false
}

fn open_catalog_lock(catalog_path: &Path) -> Result<std::fs::File> {
    let mut lock_name = catalog_path
        .file_name()
        .unwrap_or_else(|| std::ffi::OsStr::new("project-catalog.json"))
        .to_os_string();
    lock_name.push(".lock");
    let lock_path = catalog_path.with_file_name(lock_name);
    Ok(std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(lock_path)?)
}

fn validate_mapping(mapping: &PlatformV2Mapping) -> std::result::Result<(), WorkspaceCatalogError> {
    if mapping.reconciliation_revision == 0 {
        return Err(WorkspaceCatalogError::InvalidPlatformMapping);
    }
    for context in [&mapping.project, &mapping.checkout, &mapping.user_workspace] {
        if context.id.trim().is_empty()
            || context.id.len() > MAX_PLATFORM_ID_BYTES
            || context.revision == 0
        {
            return Err(WorkspaceCatalogError::InvalidPlatformMapping);
        }
    }
    Ok(())
}

const fn initial_reconciliation_revision() -> u64 {
    1
}

fn mapping_transition_allowed(existing: &PlatformV2Mapping, next: &PlatformV2Mapping) -> bool {
    let same_identities = existing.project.id == next.project.id
        && existing.checkout.id == next.checkout.id
        && existing.user_workspace.id == next.user_workspace.id;
    match (&existing.reconciliation, &next.reconciliation) {
        (PlatformMappingReconciliation::Pending, _) => true,
        (PlatformMappingReconciliation::Exact { .. }, PlatformMappingReconciliation::Pending)
        | (
            PlatformMappingReconciliation::Diverged { .. },
            PlatformMappingReconciliation::Pending,
        ) => false,
        (
            PlatformMappingReconciliation::Exact { .. },
            PlatformMappingReconciliation::Exact { .. },
        ) => same_identities,
        (
            PlatformMappingReconciliation::Exact { .. },
            PlatformMappingReconciliation::Diverged { .. },
        )
        | (PlatformMappingReconciliation::Diverged { .. }, _) => true,
    }
}

fn validate_work_item(item: &ExternalWorkItem) -> std::result::Result<(), WorkspaceCatalogError> {
    if item.provider.trim().is_empty()
        || item.repository.trim().is_empty()
        || item.key.trim().is_empty()
    {
        return Err(WorkspaceCatalogError::BlankName);
    }
    Ok(())
}

fn disk_revision(path: &Path) -> Result<u64> {
    if !path.exists() {
        return Ok(0);
    }
    let value: serde_json::Value =
        serde_json::from_slice(&std::fs::read(path)?).map_err(parse_error)?;
    let schema = schema_version(&value)?;
    match schema {
        1 => Ok(0),
        2 | CATALOG_SCHEMA_VERSION => value
            .get("revision")
            .and_then(serde_json::Value::as_u64)
            .ok_or_else(|| parse_error("missing revision")),
        other => Err(catalog_serialization_error(
            WorkspaceCatalogError::UnsupportedSchema(other),
        )),
    }
}

fn schema_version(value: &serde_json::Value) -> Result<u16> {
    let raw = value
        .get("schema_version")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| parse_error("missing schema_version"))?;
    u16::try_from(raw).map_err(|_| parse_error(format!("schema_version {raw} is out of range")))
}

#[derive(Deserialize)]
struct CatalogDiskV1 {
    #[allow(dead_code)]
    schema_version: u16,
    #[serde(default)]
    projects: Vec<ProjectRecordV1>,
    #[serde(default)]
    workspaces: Vec<UserWorkspaceRecordV1>,
}
#[derive(Deserialize)]
struct ProjectRecordV1 {
    id: Uuid,
    name: String,
    #[serde(default)]
    checkouts: Vec<ProjectCheckoutV1>,
}
#[derive(Deserialize)]
struct ProjectCheckoutV1 {
    id: Uuid,
    label: String,
    host: CheckoutHostV1,
    root: PathBuf,
    repository: RepositoryIdentity,
}
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum CheckoutHostV1 {
    Local { device_label: String },
    Ssh { connection_id: Uuid },
}
#[derive(Deserialize)]
struct UserWorkspaceRecordV1 {
    id: Uuid,
    project_id: Uuid,
    checkout_id: Uuid,
    name: String,
    lifecycle: UserWorkspaceLifecycle,
    #[serde(default)]
    linked_work_item: Option<ExternalWorkItem>,
    #[serde(default)]
    orchestration_run: Option<LegacyOrchestrationRunRef>,
}

fn migrate_v1(value: serde_json::Value) -> Result<ProjectCatalog> {
    let old: CatalogDiskV1 = serde_json::from_value(value).map_err(parse_error)?;
    let mut projects = Vec::with_capacity(old.projects.len());
    for project in old.projects {
        let mut current = ProjectRecord::new(CatalogProjectId::from_uuid(project.id), project.name);
        for checkout in project.checkouts {
            let host = match checkout.host {
                CheckoutHostV1::Local { device_label } => CheckoutHost::Local {
                    device_label,
                    root: checkout.root,
                },
                CheckoutHostV1::Ssh { connection_id } => CheckoutHost::Ssh {
                    connection_id,
                    root: RemotePosixPath::new(
                        checkout
                            .root
                            .to_str()
                            .ok_or_else(|| parse_error("v1 SSH root is not UTF-8"))?
                            .to_owned(),
                    )
                    .map_err(catalog_serialization_error)?,
                },
            };
            current.add_checkout(ProjectCheckout::new(
                CatalogCheckoutId::from_uuid(checkout.id),
                checkout.label,
                host,
                checkout.repository,
            ));
        }
        projects.push(current);
    }
    let workspaces = old
        .workspaces
        .into_iter()
        .map(|workspace| UserWorkspaceRecord {
            id: CatalogWorkspaceId::from_uuid(workspace.id),
            project_id: CatalogProjectId::from_uuid(workspace.project_id),
            checkout_id: CatalogCheckoutId::from_uuid(workspace.checkout_id),
            name: workspace.name,
            lifecycle: workspace.lifecycle,
            linked_work_item: workspace.linked_work_item,
            platform_mapping: None,
            orchestration_run: None,
            legacy_orchestration_run: workspace.orchestration_run,
        })
        .collect();
    Ok(ProjectCatalog {
        revision: 1,
        persisted_revision: 0,
        projects,
        workspaces,
    })
}

fn catalog_serialization_error(error: WorkspaceCatalogError) -> ShellDeckError {
    ShellDeckError::Serialization(format!("invalid project catalog: {error}"))
}
fn parse_error(error: impl fmt::Display) -> ShellDeckError {
    ShellDeckError::Serialization(format!("failed to parse project catalog: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    fn id(value: u128) -> Uuid {
        Uuid::from_u128(value)
    }
    fn temp_path() -> PathBuf {
        std::env::temp_dir()
            .join(format!(
                "shelldeck-project-catalog-{}-{}",
                std::process::id(),
                COUNTER.fetch_add(1, Ordering::Relaxed)
            ))
            .join("project-catalog.json")
    }
    fn local_root() -> PathBuf {
        std::env::temp_dir().join("shelldeck").join("checkout")
    }

    fn catalog_with_local_and_ssh() -> ProjectCatalog {
        let mut project = ProjectRecord::new(CatalogProjectId::from_uuid(id(1)), "ShellDeck");
        project.add_checkout(ProjectCheckout::new(
            CatalogCheckoutId::from_uuid(id(2)),
            "Local",
            CheckoutHost::Local {
                device_label: "This device".into(),
                root: local_root(),
            },
            RepositoryIdentity {
                slug: "benfavre/shelldeck".into(),
                canonical_url: None,
            },
        ));
        project.add_checkout(ProjectCheckout::new(
            CatalogCheckoutId::from_uuid(id(3)),
            "Build host",
            CheckoutHost::Ssh {
                connection_id: id(4),
                root: RemotePosixPath::new("/srv/workspaces/shelldeck").unwrap(),
            },
            RepositoryIdentity {
                slug: "benfavre/shelldeck".into(),
                canonical_url: None,
            },
        ));
        let mut catalog = ProjectCatalog::default();
        catalog.insert_project(project).unwrap();
        catalog
    }

    // SDTEST-1729
    #[test]
    fn catalog_v1_fixture_migrates_and_v3_cas_refuses_a_stale_writer() {
        let path = temp_path();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let fixture = format!(
            r#"{{
          "schema_version": 1,
          "projects": [{{"id":"{}","name":"ShellDeck","checkouts":[
            {{"id":"{}","label":"Local","host":{{"kind":"local","device_label":"This device"}},"root":{},"repository":{{"slug":"benfavre/shelldeck"}}}},
            {{"id":"{}","label":"SSH","host":{{"kind":"ssh","connection_id":"{}"}},"root":"/srv/shelldeck","repository":{{"slug":"benfavre/shelldeck"}}}}
          ]}}], "workspaces": [{{
            "id":"{}","project_id":"{}","checkout_id":"{}","name":"Legacy run",
            "lifecycle":"active","orchestration_run":{{"runtime":"automonique","run_id":"legacy-run","session_id":"legacy-session"}}
          }}]
        }}"#,
            id(1),
            id(2),
            serde_json::to_string(&local_root()).unwrap(),
            id(3),
            id(4),
            id(10),
            id(1),
            id(2)
        );
        std::fs::write(&path, fixture).unwrap();
        let migrated = ProjectCatalog::load_from(&path).expect("migrate v1 fixture");
        assert_eq!(migrated.revision(), 1);
        assert!(
            matches!(migrated.checkout_in_project(CatalogProjectId::from_uuid(id(1)), CatalogCheckoutId::from_uuid(id(3))).unwrap().host(), CheckoutHost::Ssh { root, .. } if root.as_str() == "/srv/shelldeck")
        );
        let legacy = migrated
            .workspace(CatalogWorkspaceId::from_uuid(id(10)))
            .unwrap()
            .legacy_orchestration_run()
            .expect("v1 run retained as non-authoritative display data");
        assert_eq!(legacy.run_id, "legacy-run");
        assert!(migrated
            .workspace(CatalogWorkspaceId::from_uuid(id(10)))
            .unwrap()
            .orchestration_run()
            .is_none());
        let stale = migrated.clone();
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(3));
        let writers: Vec<_> = [migrated, stale]
            .into_iter()
            .map(|mut catalog| {
                let path = path.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    catalog.save_to(&path)
                })
            })
            .collect();
        barrier.wait();
        let results: Vec<_> = writers
            .into_iter()
            .map(|writer| writer.join().expect("catalog writer"))
            .collect();
        assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
        assert_eq!(
            results
                .iter()
                .filter(|result| result
                    .as_ref()
                    .is_err_and(|error| error.to_string().contains("revision conflict")))
                .count(),
            1
        );
        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }

    // SDTEST-1740
    #[test]
    fn catalog_file_lock_excludes_an_independent_file_handle() {
        let path = temp_path();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let owner = CatalogFileLock::acquire(&path).expect("first lock");
        assert!(CatalogFileLock::try_acquire(&path)
            .expect("try competing lock")
            .is_none());
        drop(owner);
        assert!(CatalogFileLock::try_acquire(&path)
            .expect("lock after release")
            .is_some());
        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }

    // SDTEST-1739
    #[test]
    fn future_schema_and_non_portable_paths_are_rejected() {
        let path = temp_path();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, r#"{"schema_version":99,"revision":1}"#).unwrap();
        assert!(ProjectCatalog::load_from(&path)
            .unwrap_err()
            .to_string()
            .contains("unsupported"));
        std::fs::write(&path, r#"{"schema_version":65537,"revision":1}"#).unwrap();
        assert!(ProjectCatalog::load_from(&path)
            .unwrap_err()
            .to_string()
            .contains("out of range"));
        std::fs::write(
            &path,
            r#"{"schema_version":2,"revision":5,"projects":[],"workspaces":[]}"#,
        )
        .unwrap();
        let mut schema_two = ProjectCatalog::load_from(&path).expect("load schema 2");
        schema_two.save_to(&path).expect("migrate schema 2 to 3");
        let migrated: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(migrated["schema_version"], CATALOG_SCHEMA_VERSION);
        assert!(RemotePosixPath::new(r"C:\\workspaces").is_err());
        assert!(RemotePosixPath::new("/srv/../secret").is_err());
        assert!(WorkspaceRelativePath::new("../../secret").is_err());
        assert!(WorkspaceRelativePath::new(r"C:\\secret").is_err());
        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }

    // SDTEST-1741
    #[test]
    fn executor_path_boundary_contains_local_paths_and_delegates_remote_components() {
        let temp = temp_path();
        let base = temp.parent().unwrap();
        let root = base.join("checkout");
        let outside = base.join("outside");
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(root.join("src/lib.rs"), "safe").unwrap();
        std::fs::write(outside.join("secret"), "outside").unwrap();
        let local = ProjectCheckout::new(
            CatalogCheckoutId::from_uuid(id(20)),
            "Local",
            CheckoutHost::Local {
                device_label: "This device".into(),
                root: root.clone(),
            },
            RepositoryIdentity {
                slug: "benfavre/shelldeck".into(),
                canonical_url: None,
            },
        );
        let admitted = local
            .resolve_existing_local_path(&WorkspaceRelativePath::new("src/lib.rs").unwrap())
            .unwrap();
        assert_eq!(
            admitted.as_path(),
            std::fs::canonicalize(root.join("src/lib.rs")).unwrap()
        );

        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&outside, root.join("escape")).unwrap();
            assert!(matches!(
                local.resolve_existing_local_path(
                    &WorkspaceRelativePath::new("escape/secret").unwrap()
                ),
                Err(WorkspaceCatalogError::LocalPathEscapesCheckout(_))
            ));
        }

        struct RecordingSsh;
        impl SshBeneathExecutor for RecordingSsh {
            type Request = &'static str;
            type Output = (Uuid, String, String, &'static str);
            type Error = WorkspaceCatalogError;

            fn execute_beneath(
                &self,
                authority: RemoteBeneathAuthority<'_>,
                request: Self::Request,
            ) -> std::result::Result<Self::Output, Self::Error> {
                Ok((
                    authority.connection_id(),
                    authority.root().as_str().to_owned(),
                    authority.relative().as_str().to_owned(),
                    request,
                ))
            }
        }
        let ssh = ProjectCheckout::new(
            CatalogCheckoutId::from_uuid(id(21)),
            "SSH",
            CheckoutHost::Ssh {
                connection_id: id(22),
                root: RemotePosixPath::new("/srv/shelldeck").unwrap(),
            },
            RepositoryIdentity {
                slug: "benfavre/shelldeck".into(),
                canonical_url: None,
            },
        );
        assert_eq!(
            ssh.execute_remote_beneath(
                &WorkspaceRelativePath::new("src/lib.rs").unwrap(),
                "read",
                &RecordingSsh,
            )
            .unwrap(),
            (id(22), "/srv/shelldeck".into(), "src/lib.rs".into(), "read")
        );
        assert!(matches!(
            ssh.resolve_existing_local_path(&WorkspaceRelativePath::new("src/lib.rs").unwrap()),
            Err(WorkspaceCatalogError::ExpectedLocalCheckout)
        ));
        std::fs::remove_dir_all(base).ok();
    }

    // SDTEST-1730
    #[test]
    fn session_binding_requires_the_exact_authoritative_workspace_mapping() {
        let mut catalog = catalog_with_local_and_ssh();
        let workspace = CatalogWorkspaceId::from_uuid(id(10));
        catalog
            .create_workspace(WorkspaceLaunchRequest {
                id: workspace,
                project_id: CatalogProjectId::from_uuid(id(1)),
                checkout_id: CatalogCheckoutId::from_uuid(id(2)),
                name: "Issue 127".into(),
                intake: WorkspaceLaunchIntake::Prefilled(ExternalWorkItem {
                    provider: "github".into(),
                    repository: "benfavre/shelldeck".into(),
                    kind: ExternalWorkItemKind::Issue,
                    key: "127".into(),
                    title: None,
                    url: None,
                }),
            })
            .unwrap();
        let run = OrchestrationRunRef {
            runtime: "automonique".into(),
            run_id: "run-1".into(),
            session_id: Some("session-1".into()),
            platform_user_workspace_id: "workspace-v2".into(),
        };
        assert_eq!(
            catalog.bind_orchestration_run(workspace, Some(run.clone())),
            Err(WorkspaceCatalogError::PlatformMappingNotExact(workspace))
        );
        catalog
            .set_platform_mapping(
                workspace,
                None,
                PlatformV2Mapping {
                    reconciliation_revision: 1,
                    project: PlatformContextRef {
                        id: "project-v2".into(),
                        revision: 2,
                    },
                    checkout: PlatformContextRef {
                        id: "checkout-v2".into(),
                        revision: 3,
                    },
                    user_workspace: PlatformContextRef {
                        id: "workspace-v2".into(),
                        revision: 4,
                    },
                    reconciliation: PlatformMappingReconciliation::Pending,
                },
            )
            .unwrap();
        assert_eq!(
            catalog.bind_orchestration_run(workspace, Some(run.clone())),
            Err(WorkspaceCatalogError::PlatformMappingNotExact(workspace))
        );
        catalog
            .set_platform_mapping(
                workspace,
                Some(1),
                PlatformV2Mapping {
                    reconciliation_revision: 2,
                    project: PlatformContextRef {
                        id: "project-v2".into(),
                        revision: 2,
                    },
                    checkout: PlatformContextRef {
                        id: "checkout-v2".into(),
                        revision: 3,
                    },
                    user_workspace: PlatformContextRef {
                        id: "workspace-v2".into(),
                        revision: 4,
                    },
                    reconciliation: PlatformMappingReconciliation::Exact {
                        reconciled_at_millis: 10,
                    },
                },
            )
            .unwrap();
        assert_eq!(
            catalog.set_platform_mapping(
                workspace,
                Some(2),
                PlatformV2Mapping {
                    reconciliation_revision: 3,
                    project: PlatformContextRef {
                        id: "project-v2".into(),
                        revision: 2,
                    },
                    checkout: PlatformContextRef {
                        id: "checkout-v2".into(),
                        revision: 3,
                    },
                    user_workspace: PlatformContextRef {
                        id: "workspace-v2".into(),
                        revision: 4,
                    },
                    reconciliation: PlatformMappingReconciliation::Pending,
                }
            ),
            Err(WorkspaceCatalogError::InvalidPlatformMappingTransition)
        );
        assert_eq!(
            catalog.set_platform_mapping(
                workspace,
                Some(2),
                PlatformV2Mapping {
                    reconciliation_revision: 3,
                    project: PlatformContextRef {
                        id: "different-project".into(),
                        revision: 1,
                    },
                    checkout: PlatformContextRef {
                        id: "different-checkout".into(),
                        revision: 1,
                    },
                    user_workspace: PlatformContextRef {
                        id: "different-workspace".into(),
                        revision: 1,
                    },
                    reconciliation: PlatformMappingReconciliation::Exact {
                        reconciled_at_millis: 11,
                    },
                }
            ),
            Err(WorkspaceCatalogError::InvalidPlatformMappingTransition)
        );
        assert_eq!(
            catalog.set_platform_mapping(
                workspace,
                Some(1),
                PlatformV2Mapping {
                    reconciliation_revision: 3,
                    project: PlatformContextRef {
                        id: "project-v2".into(),
                        revision: 2,
                    },
                    checkout: PlatformContextRef {
                        id: "checkout-v2".into(),
                        revision: 3,
                    },
                    user_workspace: PlatformContextRef {
                        id: "workspace-v2".into(),
                        revision: 4,
                    },
                    reconciliation: PlatformMappingReconciliation::Exact {
                        reconciled_at_millis: 11,
                    },
                }
            ),
            Err(WorkspaceCatalogError::PlatformMappingRevisionConflict {
                expected: Some(1),
                actual: Some(2),
            })
        );
        assert_eq!(
            catalog.set_platform_mapping(
                workspace,
                Some(2),
                PlatformV2Mapping {
                    reconciliation_revision: 3,
                    project: PlatformContextRef {
                        id: "project-v2".into(),
                        revision: 1,
                    },
                    checkout: PlatformContextRef {
                        id: "checkout-v2".into(),
                        revision: 3,
                    },
                    user_workspace: PlatformContextRef {
                        id: "workspace-v2".into(),
                        revision: 4,
                    },
                    reconciliation: PlatformMappingReconciliation::Exact {
                        reconciled_at_millis: 9,
                    },
                }
            ),
            Err(WorkspaceCatalogError::StalePlatformMapping)
        );
        let mut wrong_run = run.clone();
        wrong_run.platform_user_workspace_id = "other-workspace".into();
        assert_eq!(
            catalog.bind_orchestration_run(workspace, Some(wrong_run)),
            Err(WorkspaceCatalogError::InvalidPlatformMapping)
        );
        catalog
            .bind_orchestration_run(workspace, Some(run))
            .expect("exact mapping authorizes binding");
        assert_eq!(
            catalog
                .workspace(workspace)
                .unwrap()
                .linked_work_item()
                .unwrap()
                .key,
            "127"
        );

        let duplicate = CatalogWorkspaceId::from_uuid(id(11));
        catalog
            .create_workspace(WorkspaceLaunchRequest {
                id: duplicate,
                project_id: CatalogProjectId::from_uuid(id(1)),
                checkout_id: CatalogCheckoutId::from_uuid(id(2)),
                name: "Duplicate mapping".into(),
                intake: WorkspaceLaunchIntake::Manual,
            })
            .unwrap();
        assert!(matches!(
            catalog.set_platform_mapping(
                duplicate,
                None,
                PlatformV2Mapping {
                    reconciliation_revision: 1,
                    project: PlatformContextRef {
                        id: "project-v2".into(),
                        revision: 2,
                    },
                    checkout: PlatformContextRef {
                        id: "checkout-v2".into(),
                        revision: 3,
                    },
                    user_workspace: PlatformContextRef {
                        id: "workspace-v2".into(),
                        revision: 4,
                    },
                    reconciliation: PlatformMappingReconciliation::Exact {
                        reconciled_at_millis: 11,
                    },
                }
            ),
            Err(WorkspaceCatalogError::DuplicatePlatformWorkspace(_))
        ));
    }
}
