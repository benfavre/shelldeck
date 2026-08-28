use super::*;
use std::fs::{File, OpenOptions};
use std::io::Read;
use std::path::{Path, PathBuf};

pub(super) fn workspace_review_root() -> PathBuf {
    crate::config::app_config::AppConfig::config_dir().join("workspace-review")
}

pub(super) fn workspace_state_path(
    root: &Path,
    workspace: CatalogWorkspaceId,
    file_name: &str,
) -> PathBuf {
    debug_assert!(matches!(file_name, "drafts.json" | "mutation-ledger.json"));
    root.join(workspace.to_string()).join(file_name)
}

pub(super) fn ensure_private_directory(path: &Path) -> Result<(), ReviewDraftError> {
    let root = path.parent().ok_or_else(invalid_private_path)?;
    if let Some(parent) = root.parent() {
        std::fs::create_dir_all(parent)?;
    }
    create_owned_directory(root)?;
    create_owned_directory(path)?;
    Ok(())
}

fn create_owned_directory(path: &Path) -> Result<(), ReviewDraftError> {
    match std::fs::create_dir(path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error.into()),
    }
    validate_directory_shape(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))?;
    }
    validate_owned_directory(path)?;
    Ok(())
}

fn validate_owned_parent(path: &Path) -> std::io::Result<()> {
    let workspace = path.parent().ok_or_else(invalid_private_path)?;
    let root = workspace.parent().ok_or_else(invalid_private_path)?;
    for component in [root, workspace] {
        match std::fs::symlink_metadata(component) {
            Ok(_) => validate_owned_directory(component)?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

fn validate_owned_directory(path: &Path) -> std::io::Result<()> {
    let _metadata = validate_directory_shape(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
        let directory = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(path)?;
        let metadata = directory.metadata()?;
        // SAFETY: `geteuid` has no pointer arguments or caller preconditions.
        let effective_user = unsafe { libc::geteuid() };
        if metadata.uid() != effective_user {
            return Err(invalid_private_path());
        }
        if metadata.permissions().mode() & 0o077 != 0 {
            // Older ShellDeck builds secured only the workspace leaf. Tighten
            // an owned, structurally validated legacy root before reading it.
            directory.set_permissions(std::fs::Permissions::from_mode(0o700))?;
            let secured = directory.metadata()?;
            if secured.uid() != effective_user || secured.permissions().mode() & 0o077 != 0 {
                return Err(invalid_private_path());
            }
        }
    }
    Ok(())
}

fn validate_directory_shape(path: &Path) -> std::io::Result<std::fs::Metadata> {
    let metadata = std::fs::symlink_metadata(path)?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() || is_windows_reparse(&metadata) {
        return Err(invalid_private_path());
    }
    let parent = path.parent().ok_or_else(invalid_private_path)?;
    let name = path.file_name().ok_or_else(invalid_private_path)?;
    let expected = std::fs::canonicalize(parent)?.join(name);
    if std::fs::canonicalize(path)? != expected {
        return Err(invalid_private_path());
    }
    Ok(metadata)
}

#[cfg(windows)]
fn is_windows_reparse(metadata: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT;
    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
const fn is_windows_reparse(_metadata: &std::fs::Metadata) -> bool {
    false
}

fn invalid_private_path() -> std::io::Error {
    std::io::Error::new(
        std::io::ErrorKind::InvalidInput,
        "review persistence path contains a non-canonical or linked component",
    )
}

fn open_no_follow(path: &Path, configure: impl FnOnce(&mut OpenOptions)) -> std::io::Result<File> {
    validate_owned_parent(path)?;
    let mut options = OpenOptions::new();
    configure(&mut options);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        use windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT;
        options.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
    }
    let file = options.open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.file_type().is_symlink() || is_windows_reparse(&metadata) {
        return Err(invalid_private_path());
    }
    Ok(file)
}

fn bounded_descriptor_read_after_open(
    path: &Path,
    maximum: u64,
    after_open: impl FnOnce(),
) -> std::io::Result<Option<Vec<u8>>> {
    let file = match open_no_follow(path, |options| {
        options.read(true);
    }) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    // Keep the validated descriptor for the entire read. Replacing the path
    // cannot redirect this read and growth of this inode is detected by the
    // maximum-plus-one limit below.
    after_open();
    let mut bytes = Vec::new();
    file.take(maximum.saturating_add(1))
        .read_to_end(&mut bytes)?;
    Ok(Some(bytes))
}

fn bounded_descriptor_read(path: &Path, maximum: u64) -> std::io::Result<Option<Vec<u8>>> {
    bounded_descriptor_read_after_open(path, maximum, || {})
}

#[cfg(test)]
pub(super) fn bounded_read_after_open_for_test(
    path: &Path,
    maximum: u64,
    after_open: impl FnOnce(),
) -> std::io::Result<Option<Vec<u8>>> {
    bounded_descriptor_read_after_open(path, maximum, after_open)
}

pub(super) fn bounded_read(path: &Path, maximum: u64) -> Result<Option<Vec<u8>>, ReviewDraftError> {
    let bytes = bounded_descriptor_read(path, maximum)?;
    if bytes
        .as_ref()
        .is_some_and(|bytes| bytes.len() as u64 > maximum)
    {
        return Err(ReviewDraftError::BoundsExceeded("file size"));
    }
    Ok(bytes)
}

pub(super) fn workflow_bounded_read(path: &Path) -> Result<Option<Vec<u8>>, ReviewWorkflowError> {
    let bytes = bounded_descriptor_read(path, MAX_WORKFLOW_FILE_BYTES)
        .map_err(|error| ReviewWorkflowError::Storage(error.to_string()))?;
    if bytes
        .as_ref()
        .is_some_and(|bytes| bytes.len() as u64 > MAX_WORKFLOW_FILE_BYTES)
    {
        return Err(ReviewWorkflowError::BoundsExceeded(
            "mutation ledger file exceeds its bound",
        ));
    }
    Ok(bytes)
}

pub(super) fn open_lock_file(path: &Path) -> std::io::Result<File> {
    open_no_follow(path, |options| {
        options.create(true).truncate(false).read(true).write(true);
    })
}

pub(super) fn secure_atomic_write(path: &Path, payload: &[u8]) -> std::io::Result<()> {
    validate_owned_parent(path)?;
    if let Ok(metadata) = std::fs::symlink_metadata(path) {
        if metadata.file_type().is_symlink() || is_windows_reparse(&metadata) {
            return Err(invalid_private_path());
        }
    }
    crate::util::atomic_write(path, payload)
}

pub(super) fn lock_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(".lock");
    PathBuf::from(name)
}

pub(super) struct ReviewDraftDiskIdentity {
    pub(super) revision: u64,
    pub(super) workspace: Option<CatalogWorkspaceId>,
}

pub(super) fn read_disk_identity(path: &Path) -> Result<ReviewDraftDiskIdentity, ReviewDraftError> {
    match bounded_read(path, MAX_DRAFT_FILE_BYTES)? {
        Some(bytes) => {
            let disk = serde_json::from_slice::<ReviewDraftDisk>(&bytes)?;
            if disk.schema_version != REVIEW_DRAFT_SCHEMA {
                return Err(ReviewDraftError::UnsupportedSchema(disk.schema_version));
            }
            Ok(ReviewDraftDiskIdentity {
                revision: disk.revision,
                workspace: Some(disk.workspace),
            })
        }
        None => Ok(ReviewDraftDiskIdentity {
            revision: 0,
            workspace: None,
        }),
    }
}

pub(super) fn workflow_disk_revision(
    path: &Path,
    workspace: CatalogWorkspaceId,
) -> Result<u64, ReviewWorkflowError> {
    let Some(bytes) = workflow_bounded_read(path)? else {
        return Ok(0);
    };
    let disk: ReviewWorkflowDisk = serde_json::from_slice(&bytes)
        .map_err(|error| ReviewWorkflowError::Storage(error.to_string()))?;
    if disk.schema_version != REVIEW_WORKFLOW_SCHEMA || disk.workspace != workspace {
        return Err(ReviewWorkflowError::Storage(
            "invalid mutation ledger identity or schema".into(),
        ));
    }
    Ok(disk.revision)
}
