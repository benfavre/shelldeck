//! Persistent project, checkout, and user-workspace catalog.
//!
//! A launcher selects a checkout already present in this catalog. It never
//! accepts an arbitrary path or SSH credential, so UI intake cannot widen the
//! terminal/filesystem authority established by ShellDeck configuration.

use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fmt;
use std::path::{Path, PathBuf};
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

uuid_id!(ProjectId);
uuid_id!(CheckoutId);
uuid_id!(UserWorkspaceId);

const CATALOG_SCHEMA_VERSION: u16 = 1;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RepositoryIdentity {
    /// Stable, display-safe repository identity such as `owner/repository`.
    pub slug: String,
    /// Optional canonical URL. Authentication remains in the git/SSH owner.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub canonical_url: Option<String>,
}

/// ShellDeck-owned host authority for one checkout.
///
/// An SSH checkout stores only an existing connection ID. Passwords, private
/// key paths, and provider-session grants deliberately have no representation
/// here.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CheckoutHost {
    Local { device_label: String },
    Ssh { connection_id: Uuid },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ProjectCheckout {
    pub id: CheckoutId,
    pub label: String,
    pub host: CheckoutHost,
    pub root: PathBuf,
    pub repository: RepositoryIdentity,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ProjectRecord {
    pub id: ProjectId,
    pub name: String,
    #[serde(default)]
    pub checkouts: Vec<ProjectCheckout>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExternalWorkItemKind {
    Issue,
    PullRequest,
    Task,
}

/// A task owned by an external tracker. This is not an agent/runtime run.
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

/// An internal Automonique orchestration identity.
///
/// It can be displayed beside an [`ExternalWorkItem`], but it carries no
/// checkout, terminal, SSH, or filesystem authority.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct OrchestrationRunRef {
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
    pub id: UserWorkspaceId,
    pub project_id: ProjectId,
    pub checkout_id: CheckoutId,
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
    pub id: UserWorkspaceId,
    pub project_id: ProjectId,
    pub checkout_id: CheckoutId,
    pub name: String,
    pub lifecycle: UserWorkspaceLifecycle,
    /// External tracker identity, if the workspace was launched from one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub linked_work_item: Option<ExternalWorkItem>,
    /// Internal run identity, intentionally separate from the external task.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub orchestration_run: Option<OrchestrationRunRef>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ProjectCatalog {
    schema_version: u16,
    #[serde(default)]
    pub projects: Vec<ProjectRecord>,
    #[serde(default)]
    pub workspaces: Vec<UserWorkspaceRecord>,
}

impl Default for ProjectCatalog {
    fn default() -> Self {
        Self {
            schema_version: CATALOG_SCHEMA_VERSION,
            projects: Vec::new(),
            workspaces: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WorkspaceCatalogError {
    UnsupportedSchema(u16),
    DuplicateProject(ProjectId),
    DuplicateCheckout(CheckoutId),
    DuplicateWorkspace(UserWorkspaceId),
    UnknownProject(ProjectId),
    UnknownCheckout(CheckoutId),
    CheckoutOutsideProject {
        project: ProjectId,
        checkout: CheckoutId,
    },
    UnknownWorkspace(UserWorkspaceId),
    BlankName,
}

impl fmt::Display for WorkspaceCatalogError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedSchema(version) => {
                write!(formatter, "unsupported project catalog schema {version}")
            }
            Self::DuplicateProject(id) => write!(formatter, "duplicate project {id}"),
            Self::DuplicateCheckout(id) => write!(formatter, "duplicate checkout {id}"),
            Self::DuplicateWorkspace(id) => write!(formatter, "duplicate workspace {id}"),
            Self::UnknownProject(id) => write!(formatter, "unknown project {id}"),
            Self::UnknownCheckout(id) => write!(formatter, "unknown checkout {id}"),
            Self::CheckoutOutsideProject { project, checkout } => {
                write!(
                    formatter,
                    "checkout {checkout} does not belong to project {project}"
                )
            }
            Self::UnknownWorkspace(id) => write!(formatter, "unknown workspace {id}"),
            Self::BlankName => formatter.write_str("workspace name cannot be blank"),
        }
    }
}

impl std::error::Error for WorkspaceCatalogError {}

impl ProjectCatalog {
    fn catalog_path() -> PathBuf {
        super::app_config::AppConfig::config_dir().join("project-catalog.json")
    }

    pub fn save(&self) -> Result<()> {
        self.save_to(&Self::catalog_path())
    }

    pub fn load() -> Result<Self> {
        Self::load_from(&Self::catalog_path())
    }

    pub(crate) fn save_to(&self, path: &Path) -> Result<()> {
        self.validate().map_err(catalog_serialization_error)?;
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }
        let payload = serde_json::to_vec_pretty(self).map_err(|error| {
            ShellDeckError::Serialization(format!("failed to serialize project catalog: {error}"))
        })?;
        crate::util::atomic_write(path, &payload)?;
        Ok(())
    }

    pub(crate) fn load_from(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let payload = std::fs::read(path)?;
        let catalog: Self = serde_json::from_slice(&payload).map_err(|error| {
            ShellDeckError::Serialization(format!("failed to parse project catalog: {error}"))
        })?;
        catalog.validate().map_err(catalog_serialization_error)?;
        Ok(catalog)
    }

    /// Validates all references before a persistent catalog becomes visible.
    pub fn validate(&self) -> std::result::Result<(), WorkspaceCatalogError> {
        if self.schema_version != CATALOG_SCHEMA_VERSION {
            return Err(WorkspaceCatalogError::UnsupportedSchema(
                self.schema_version,
            ));
        }

        let mut projects = BTreeSet::new();
        let mut checkouts = BTreeSet::new();
        for project in &self.projects {
            if !projects.insert(project.id) {
                return Err(WorkspaceCatalogError::DuplicateProject(project.id));
            }
            for checkout in &project.checkouts {
                if !checkouts.insert(checkout.id) {
                    return Err(WorkspaceCatalogError::DuplicateCheckout(checkout.id));
                }
            }
        }

        let mut workspaces = BTreeSet::new();
        for workspace in &self.workspaces {
            if !workspaces.insert(workspace.id) {
                return Err(WorkspaceCatalogError::DuplicateWorkspace(workspace.id));
            }
            self.checkout_in_project(workspace.project_id, workspace.checkout_id)?;
        }
        Ok(())
    }

    /// Resolves the checkout only when it belongs to the requested project.
    /// Launchers use this instead of accepting an arbitrary filesystem path.
    pub fn checkout_in_project(
        &self,
        project_id: ProjectId,
        checkout_id: CheckoutId,
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

    /// Creates manual and task-prefilled workspaces through one validated path.
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
            WorkspaceLaunchIntake::Prefilled(item) => Some(item),
        };
        self.workspaces.push(UserWorkspaceRecord {
            id: request.id,
            project_id: request.project_id,
            checkout_id: request.checkout_id,
            name: request.name.trim().to_string(),
            lifecycle: UserWorkspaceLifecycle::Active,
            linked_work_item,
            orchestration_run: None,
        });
        Ok(self
            .workspaces
            .last()
            .expect("workspace was inserted immediately before lookup"))
    }

    pub fn archive_workspace(
        &mut self,
        id: UserWorkspaceId,
    ) -> std::result::Result<(), WorkspaceCatalogError> {
        self.workspace_mut(id)?.lifecycle = UserWorkspaceLifecycle::Archived;
        Ok(())
    }

    pub fn resume_workspace(
        &mut self,
        id: UserWorkspaceId,
    ) -> std::result::Result<(), WorkspaceCatalogError> {
        self.workspace_mut(id)?.lifecycle = UserWorkspaceLifecycle::Active;
        Ok(())
    }

    pub fn bind_orchestration_run(
        &mut self,
        id: UserWorkspaceId,
        run: Option<OrchestrationRunRef>,
    ) -> std::result::Result<(), WorkspaceCatalogError> {
        self.workspace_mut(id)?.orchestration_run = run;
        Ok(())
    }

    fn workspace_mut(
        &mut self,
        id: UserWorkspaceId,
    ) -> std::result::Result<&mut UserWorkspaceRecord, WorkspaceCatalogError> {
        self.workspaces
            .iter_mut()
            .find(|workspace| workspace.id == id)
            .ok_or(WorkspaceCatalogError::UnknownWorkspace(id))
    }
}

fn catalog_serialization_error(error: WorkspaceCatalogError) -> ShellDeckError {
    ShellDeckError::Serialization(format!("invalid project catalog: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

    fn id(value: u128) -> Uuid {
        Uuid::from_u128(value)
    }

    fn temp_path() -> PathBuf {
        std::env::temp_dir()
            .join(format!(
                "shelldeck-project-catalog-{}-{}",
                std::process::id(),
                COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            ))
            .join("project-catalog.json")
    }

    fn catalog_with_local_and_ssh() -> ProjectCatalog {
        ProjectCatalog {
            projects: vec![ProjectRecord {
                id: ProjectId::from_uuid(id(1)),
                name: "ShellDeck".into(),
                checkouts: vec![
                    ProjectCheckout {
                        id: CheckoutId::from_uuid(id(2)),
                        label: "Local".into(),
                        host: CheckoutHost::Local {
                            device_label: "This device".into(),
                        },
                        root: PathBuf::from("projects").join("shelldeck"),
                        repository: RepositoryIdentity {
                            slug: "benfavre/shelldeck".into(),
                            canonical_url: Some("https://github.com/benfavre/shelldeck".into()),
                        },
                    },
                    ProjectCheckout {
                        id: CheckoutId::from_uuid(id(3)),
                        label: "Build host".into(),
                        host: CheckoutHost::Ssh {
                            connection_id: id(4),
                        },
                        root: PathBuf::from("workspaces").join("shelldeck"),
                        repository: RepositoryIdentity {
                            slug: "benfavre/shelldeck".into(),
                            canonical_url: None,
                        },
                    },
                ],
            }],
            ..ProjectCatalog::default()
        }
    }

    // SDTEST-1729
    #[test]
    fn project_catalog_round_trip_groups_portable_local_and_ssh_checkouts() {
        let path = temp_path();
        let catalog = catalog_with_local_and_ssh();

        catalog.save_to(&path).expect("save catalog");
        let restored = ProjectCatalog::load_from(&path).expect("load catalog");

        assert_eq!(restored, catalog);
        let hosts: Vec<_> = restored.projects[0]
            .checkouts
            .iter()
            .map(|checkout| &checkout.host)
            .collect();
        assert!(matches!(hosts[0], CheckoutHost::Local { .. }));
        assert_eq!(
            hosts[1],
            &CheckoutHost::Ssh {
                connection_id: id(4)
            }
        );
        let serialized = std::fs::read_to_string(&path).expect("read catalog");
        assert!(!serialized.contains("password"));
        assert!(!serialized.contains("private_key"));

        std::fs::remove_dir_all(path.parent().expect("catalog parent")).ok();
    }

    // SDTEST-1730
    #[test]
    fn one_launcher_validates_manual_and_prefilled_lifecycle_and_keeps_run_distinct() {
        let mut catalog = catalog_with_local_and_ssh();
        let project = ProjectId::from_uuid(id(1));
        let manual = UserWorkspaceId::from_uuid(id(10));
        let issue = UserWorkspaceId::from_uuid(id(11));

        catalog
            .create_workspace(WorkspaceLaunchRequest {
                id: manual,
                project_id: project,
                checkout_id: CheckoutId::from_uuid(id(2)),
                name: "  manual investigation  ".into(),
                intake: WorkspaceLaunchIntake::Manual,
            })
            .expect("manual launch");
        for (offset, kind, key) in [
            (0, ExternalWorkItemKind::Issue, "127"),
            (1, ExternalWorkItemKind::PullRequest, "129"),
            (2, ExternalWorkItemKind::Task, "release-check"),
        ] {
            catalog
                .create_workspace(WorkspaceLaunchRequest {
                    id: UserWorkspaceId::from_uuid(id(11 + offset)),
                    project_id: project,
                    checkout_id: CheckoutId::from_uuid(id(3)),
                    name: format!("Work item {key}"),
                    intake: WorkspaceLaunchIntake::Prefilled(ExternalWorkItem {
                        provider: "github".into(),
                        repository: "benfavre/shelldeck".into(),
                        kind,
                        key: key.into(),
                        title: Some("Workspace navigation".into()),
                        url: None,
                    }),
                })
                .expect("prefilled launch");
        }

        catalog.archive_workspace(issue).expect("archive");
        catalog.resume_workspace(issue).expect("resume");
        catalog
            .bind_orchestration_run(
                issue,
                Some(OrchestrationRunRef {
                    runtime: "automonique".into(),
                    run_id: "run-opaque".into(),
                    session_id: Some("session-opaque".into()),
                }),
            )
            .expect("bind run");

        assert_eq!(catalog.workspaces[0].name, "manual investigation");
        assert!(catalog.workspaces[0].linked_work_item.is_none());
        let linked = catalog.workspaces[1]
            .linked_work_item
            .as_ref()
            .expect("external issue remains linked");
        assert_eq!(linked.key, "127");
        assert_eq!(
            catalog.workspaces[1].lifecycle,
            UserWorkspaceLifecycle::Active
        );
        let run = catalog.workspaces[1]
            .orchestration_run
            .as_ref()
            .expect("internal run remains separately bound");
        assert_eq!(run.run_id, "run-opaque");
        let restored: ProjectCatalog = serde_json::from_value(
            serde_json::to_value(&catalog).expect("serialize durable launcher records"),
        )
        .expect("restore durable launcher records");
        assert_eq!(
            restored.workspaces[1..]
                .iter()
                .map(|workspace| workspace.linked_work_item.as_ref().unwrap().kind)
                .collect::<Vec<_>>(),
            vec![
                ExternalWorkItemKind::Issue,
                ExternalWorkItemKind::PullRequest,
                ExternalWorkItemKind::Task,
            ]
        );

        let outside = catalog.create_workspace(WorkspaceLaunchRequest {
            id: UserWorkspaceId::from_uuid(id(20)),
            project_id: project,
            checkout_id: CheckoutId::from_uuid(id(99)),
            name: "untrusted path".into(),
            intake: WorkspaceLaunchIntake::Manual,
        });
        assert_eq!(
            outside,
            Err(WorkspaceCatalogError::UnknownCheckout(
                CheckoutId::from_uuid(id(99))
            ))
        );
    }
}
