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

const CATALOG_SCHEMA_VERSION: u16 = 2;
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
    pub const fn host(&self) -> &CheckoutHost {
        &self.host
    }
    #[must_use]
    pub const fn repository(&self) -> &RepositoryIdentity {
        &self.repository
    }
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
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
struct CatalogDiskV2 {
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
    StalePlatformMapping,
    DuplicatePlatformWorkspace(String),
    PlatformMappingNotExact(CatalogWorkspaceId),
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
        // ShellDeck's single-instance boundary supplies the cross-process
        // owner. This lock makes revision check + replace one transaction for
        // background tasks and cloned snapshots inside that process.
        let _save_guard = CATALOG_SAVE_LOCK.lock();
        self.validate().map_err(catalog_serialization_error)?;
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            std::fs::create_dir_all(parent)?;
        }
        let actual = disk_revision(path)?;
        if actual != self.persisted_revision {
            return Err(catalog_serialization_error(
                WorkspaceCatalogError::RevisionConflict {
                    expected: self.persisted_revision,
                    actual,
                },
            ));
        }
        let disk = CatalogDiskV2 {
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
            CATALOG_SCHEMA_VERSION => {
                let disk: CatalogDiskV2 = serde_json::from_value(value).map_err(parse_error)?;
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
        mapping: PlatformV2Mapping,
    ) -> std::result::Result<(), WorkspaceCatalogError> {
        validate_mapping(&mapping)?;
        if let Some(existing) = self.workspace(id)?.platform_mapping.as_ref() {
            for (old, new) in [
                (&existing.project, &mapping.project),
                (&existing.checkout, &mapping.checkout),
                (&existing.user_workspace, &mapping.user_workspace),
            ] {
                if old.id == new.id && new.revision < old.revision {
                    return Err(WorkspaceCatalogError::StalePlatformMapping);
                }
            }
            if mapping_observed_at(&mapping)
                .zip(mapping_observed_at(existing))
                .is_some_and(|(new, old)| new < old)
            {
                return Err(WorkspaceCatalogError::StalePlatformMapping);
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
        self.workspace_mut(id)?.orchestration_run = run;
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

fn validate_mapping(mapping: &PlatformV2Mapping) -> std::result::Result<(), WorkspaceCatalogError> {
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

fn mapping_observed_at(mapping: &PlatformV2Mapping) -> Option<u64> {
    match mapping.reconciliation {
        PlatformMappingReconciliation::Pending => None,
        PlatformMappingReconciliation::Exact {
            reconciled_at_millis,
        } => Some(reconciled_at_millis),
        PlatformMappingReconciliation::Diverged { observed_at_millis } => Some(observed_at_millis),
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
        CATALOG_SCHEMA_VERSION => value
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
    orchestration_run: Option<serde_json::Value>,
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
        .map(|workspace| {
            let _ = workspace.orchestration_run;
            UserWorkspaceRecord {
                id: CatalogWorkspaceId::from_uuid(workspace.id),
                project_id: CatalogProjectId::from_uuid(workspace.project_id),
                checkout_id: CatalogCheckoutId::from_uuid(workspace.checkout_id),
                name: workspace.name,
                lifecycle: workspace.lifecycle,
                linked_work_item: workspace.linked_work_item,
                platform_mapping: None,
                orchestration_run: None,
            }
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
    fn catalog_v1_fixture_migrates_and_v2_cas_refuses_a_stale_writer() {
        let path = temp_path();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let fixture = format!(
            r#"{{
          "schema_version": 1,
          "projects": [{{"id":"{}","name":"ShellDeck","checkouts":[
            {{"id":"{}","label":"Local","host":{{"kind":"local","device_label":"This device"}},"root":{},"repository":{{"slug":"benfavre/shelldeck"}}}},
            {{"id":"{}","label":"SSH","host":{{"kind":"ssh","connection_id":"{}"}},"root":"/srv/shelldeck","repository":{{"slug":"benfavre/shelldeck"}}}}
          ]}}], "workspaces": []
        }}"#,
            id(1),
            id(2),
            serde_json::to_string(&local_root()).unwrap(),
            id(3),
            id(4)
        );
        std::fs::write(&path, fixture).unwrap();
        let migrated = ProjectCatalog::load_from(&path).expect("migrate v1 fixture");
        assert_eq!(migrated.revision(), 1);
        assert!(
            matches!(migrated.checkout_in_project(CatalogProjectId::from_uuid(id(1)), CatalogCheckoutId::from_uuid(id(3))).unwrap().host(), CheckoutHost::Ssh { root, .. } if root.as_str() == "/srv/shelldeck")
        );
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
        assert!(RemotePosixPath::new(r"C:\\workspaces").is_err());
        assert!(RemotePosixPath::new("/srv/../secret").is_err());
        assert!(WorkspaceRelativePath::new("../../secret").is_err());
        assert!(WorkspaceRelativePath::new(r"C:\\secret").is_err());
        std::fs::remove_dir_all(path.parent().unwrap()).ok();
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
                PlatformV2Mapping {
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
                PlatformV2Mapping {
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
                PlatformV2Mapping {
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
                PlatformV2Mapping {
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
