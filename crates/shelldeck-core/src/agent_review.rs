//! Bounded local Git review and explicitly confirmed, file-level operations.
//!
//! Observations never write to Git. A mutation is previewed against the actual
//! HEAD, whole index, and exact worktree bytes, then revalidated under Git locks.
//! There is deliberately no hunk staging API. Before/after observations alone
//! establish change, not authorship: undo additionally requires exact edit
//! evidence supplied by the agent adapter. Unknown or oversized data refuses
//! writes rather than being interpreted as an empty file or a clean checkout.

/// Only local runs with write access need eager before/after attribution.
/// Read-only chat can inspect Changes on demand without delaying the provider.
pub fn requires_turn_snapshot(
    target: &crate::agent_runtime::AgentTarget,
    access: crate::agent_runtime::AgentAccessMode,
) -> bool {
    matches!(target, crate::agent_runtime::AgentTarget::Local)
        && access != crate::agent_runtime::AgentAccessMode::ReadOnly
}

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, Write};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use fs2::FileExt;
use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
pub enum ReviewError {
    #[error("{0}")]
    Refused(String),
    #[error("Review I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("{program} failed: {detail}")]
    Command { program: String, detail: String },
}

pub type ReviewResult<T> = Result<T, ReviewError>;

#[derive(Clone, Debug)]
pub struct ReviewLimits {
    pub max_command_bytes: usize,
    pub max_file_bytes: usize,
    pub max_snapshot_bytes: usize,
    pub max_files: usize,
    pub command_timeout: Duration,
}

impl Default for ReviewLimits {
    fn default() -> Self {
        Self {
            max_command_bytes: 4 * 1024 * 1024,
            max_file_bytes: 8 * 1024 * 1024,
            max_snapshot_bytes: 128 * 1024 * 1024,
            max_files: 20_000,
            command_timeout: Duration::from_secs(10),
        }
    }
}

/// No shell is involved. `stdin` remains separate from the displayable argv.
#[derive(Clone, Debug)]
pub struct ReviewCommand {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    pub stdin: Vec<u8>,
    pub env: BTreeMap<String, String>,
    pub max_output_bytes: usize,
    pub timeout: Duration,
}

#[derive(Clone, Debug)]
pub struct ReviewCommandOutput {
    pub exit_code: i32,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

/// Unit tests substitute this boundary; they never execute real Git or gh.
pub trait ReviewExecutor: Send + Sync {
    fn run(&self, command: &ReviewCommand) -> ReviewResult<ReviewCommandOutput>;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct SystemReviewExecutor;

impl ReviewExecutor for SystemReviewExecutor {
    fn run(&self, command: &ReviewCommand) -> ReviewResult<ReviewCommandOutput> {
        if !matches!(command.program.as_str(), "git" | "gh") {
            return refuse("Only the local git and gh executables are supported");
        }
        let mut process = Command::new(&command.program);
        // A terminal's inherited Git overrides must not redirect this review
        // to another checkout/index/object store.
        for name in [
            "GIT_DIR",
            "GIT_WORK_TREE",
            "GIT_INDEX_FILE",
            "GIT_COMMON_DIR",
            "GIT_OBJECT_DIRECTORY",
            "GIT_ALTERNATE_OBJECT_DIRECTORIES",
            "GIT_NAMESPACE",
            "GIT_CONFIG_PARAMETERS",
            "GIT_CONFIG_COUNT",
        ] {
            process.env_remove(name);
        }
        let mut child = process
            .args(&command.args)
            .current_dir(&command.cwd)
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_OPTIONAL_LOCKS", "0")
            .env("GIT_PAGER", "cat")
            .envs(&command.env)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        let stdout = child.stdout.take().expect("piped stdout");
        let stderr = child.stderr.take().expect("piped stderr");
        let mut stdin = child.stdin.take().expect("piped stdin");
        let input = command.stdin.clone();
        let cap = command.max_output_bytes;
        let (out_tx, out_rx) = std::sync::mpsc::sync_channel(1);
        let (err_tx, err_rx) = std::sync::mpsc::sync_channel(1);
        thread::spawn(move || {
            let _ = out_tx.send(read_capped(stdout, cap));
        });
        thread::spawn(move || {
            let _ = err_tx.send(read_capped(stderr, 64 * 1024));
        });
        thread::spawn(move || {
            let _ = stdin.write_all(&input);
        });
        let start = Instant::now();
        let (status, timed_out) = loop {
            if let Some(status) = child.try_wait()? {
                break (status, false);
            }
            if start.elapsed() >= command.timeout {
                let _ = child.kill();
                break (child.wait()?, true);
            }
            thread::sleep(Duration::from_millis(10));
        };
        if timed_out {
            return refuse("Git review command exceeded its time limit");
        }
        // A descendant may inherit the pipes after Git exits. Do not wait for
        // such a process indefinitely; all retained data remains capped.
        let out = out_rx
            .recv_timeout(Duration::from_millis(250))
            .map_err(|_| {
                ReviewError::Refused("Git output pipe did not close within its deadline".into())
            })??;
        let err = err_rx
            .recv_timeout(Duration::from_millis(250))
            .map_err(|_| {
                ReviewError::Refused("Git error pipe did not close within its deadline".into())
            })??;
        if out.1 || err.1 {
            return refuse(
                "Git review output exceeded its byte limit; no incomplete result was used",
            );
        }
        Ok(ReviewCommandOutput {
            exit_code: status.code().unwrap_or(-1),
            stdout: out.0,
            stderr: err.0,
        })
    }
}

fn read_capped(mut reader: impl Read, cap: usize) -> std::io::Result<(Vec<u8>, bool)> {
    let mut bytes = Vec::with_capacity(cap.min(8192));
    reader
        .by_ref()
        .take(cap.saturating_add(1) as u64)
        .read_to_end(&mut bytes)?;
    let exceeded = bytes.len() > cap;
    bytes.truncate(cap);
    Ok((bytes, exceeded))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StatusEntry {
    pub path: String,
    pub original_path: Option<String>,
    pub index: char,
    pub worktree: char,
}

impl StatusEntry {
    pub fn is_untracked(&self) -> bool {
        self.index == '?' && self.worktree == '?'
    }
    pub fn is_conflicted(&self) -> bool {
        matches!(
            (self.index, self.worktree),
            ('D', 'D')
                | ('A', 'U')
                | ('U', 'D')
                | ('U', 'A')
                | ('D', 'U')
                | ('A', 'A')
                | ('U', 'U')
        )
    }
    pub fn is_staged(&self) -> bool {
        !matches!(self.index, ' ' | '?' | '!')
    }
    pub fn is_unstaged(&self) -> bool {
        !matches!(self.worktree, ' ' | '?' | '!')
    }
}

/// Git porcelain v1 `-z` names are unquoted and may contain tabs or newlines.
/// For renames/copies Git reports the destination first, then the source.
pub fn parse_status_z(bytes: &[u8]) -> ReviewResult<Vec<StatusEntry>> {
    let mut records = bytes.split(|byte| *byte == 0);
    let mut entries = Vec::new();
    while let Some(record) = records.next() {
        if record.is_empty() {
            continue;
        }
        if record.len() < 4
            || record[2] != b' '
            || !valid_status(record[0])
            || !valid_status(record[1])
        {
            return refuse("Malformed NUL-delimited Git status");
        }
        let path = utf8_path(&record[3..])?;
        let original_path = if matches!(record[0], b'R' | b'C') || matches!(record[1], b'R' | b'C')
        {
            Some(utf8_path(records.next().ok_or_else(|| {
                ReviewError::Refused("Git rename is missing its original path".into())
            })?)?)
        } else {
            None
        };
        entries.push(StatusEntry {
            path,
            original_path,
            index: record[0] as char,
            worktree: record[1] as char,
        });
    }
    Ok(entries)
}

fn valid_status(value: u8) -> bool {
    b" MADRCU?!T".contains(&value)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct LineCounts {
    /// None is unknown/binary, never zero.
    pub additions: Option<u64>,
    pub deletions: Option<u64>,
}

pub fn parse_numstat_z(bytes: &[u8]) -> ReviewResult<BTreeMap<String, LineCounts>> {
    let mut result = BTreeMap::new();
    let mut records = bytes.split(|byte| *byte == 0);
    while let Some(record) = records.next() {
        if record.is_empty() {
            continue;
        }
        let mut fields = record.splitn(3, |byte| *byte == b'\t');
        let added = fields.next().unwrap_or_default();
        let removed = fields
            .next()
            .ok_or_else(|| ReviewError::Refused("Malformed Git numstat".into()))?;
        let name = fields
            .next()
            .ok_or_else(|| ReviewError::Refused("Git numstat is missing a path".into()))?;
        let counts = if added == b"-" && removed == b"-" {
            LineCounts::default()
        } else {
            LineCounts {
                additions: Some(parse_u64(added)?),
                deletions: Some(parse_u64(removed)?),
            }
        };
        let path = if name.is_empty() {
            let old = records.next().ok_or_else(|| {
                ReviewError::Refused("Git numstat rename is missing a source".into())
            })?;
            let _ = utf8_path(old)?;
            utf8_path(records.next().ok_or_else(|| {
                ReviewError::Refused("Git numstat rename is missing a destination".into())
            })?)?
        } else {
            utf8_path(name)?
        };
        if result.insert(path, counts).is_some() {
            return refuse("Git numstat contains a duplicate path");
        }
    }
    Ok(result)
}

fn parse_u64(bytes: &[u8]) -> ReviewResult<u64> {
    std::str::from_utf8(bytes)
        .ok()
        .and_then(|text| text.parse().ok())
        .ok_or_else(|| ReviewError::Refused("Invalid Git line count".into()))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FileImage {
    Missing,
    Regular {
        bytes: Vec<u8>,
        mode: u32,
        permission_bits: Option<u32>,
        readonly: bool,
    },
    Refused(String),
}

impl FileImage {
    fn exact(&self) -> bool {
        !matches!(self, Self::Refused(_))
    }
    fn size(&self) -> usize {
        match self {
            Self::Regular { bytes, .. } => bytes.len(),
            _ => 0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReviewSection {
    Staged,
    Unstaged,
    Untracked,
}

#[derive(Clone, Debug)]
pub struct FileDiff {
    pub path: String,
    pub original_path: Option<String>,
    pub section: ReviewSection,
    pub counts: LineCounts,
    pub unified: String,
    pub conflicted: bool,
}

#[derive(Clone, Debug)]
pub struct RepositorySnapshot {
    pub id: Uuid,
    pub root: PathBuf,
    pub git_dir: PathBuf,
    pub common_dir: PathBuf,
    /// None means an unborn branch, not an unknown HEAD.
    pub head: Option<String>,
    pub head_ref: Option<String>,
    pub branch: Option<String>,
    pub status: Vec<StatusEntry>,
    pub diffs: Vec<FileDiff>,
    pub files: BTreeMap<String, FileImage>,
    pub complete: bool,
    pub refusals: Vec<String>,
    index: Option<Vec<u8>>,
    index_modes: BTreeMap<String, u32>,
}

impl RepositorySnapshot {
    pub fn same_repository_state(&self, other: &Self) -> bool {
        self.root == other.root
            && self.git_dir == other.git_dir
            && self.common_dir == other.common_dir
            && self.head == other.head
            && self.head_ref == other.head_ref
            && self.branch == other.branch
            && self.status == other.status
            && self.files == other.files
            && self.index == other.index
            && self.index_modes == other.index_modes
            && self.complete == other.complete
            && self.refusals == other.refusals
    }

    pub fn staged_count(&self) -> usize {
        self.status.iter().filter(|entry| entry.is_staged()).count()
    }
    pub fn can_mutate(&self) -> bool {
        self.complete && !self.status.iter().any(StatusEntry::is_conflicted)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ChangeAttribution {
    /// Exact agent edit evidence spans this before/after pair.
    ProvenAgentEdit,
    /// Before/after alone cannot distinguish agent work from user edits.
    ChangedDuringRun,
}

#[derive(Clone, Debug)]
pub struct OwnedFileEdit {
    pub path: String,
    pub before: FileImage,
    pub after: FileImage,
}

#[derive(Clone, Debug)]
pub struct TurnFileChange {
    pub path: String,
    pub before: FileImage,
    pub after: FileImage,
    pub baseline_dirty: bool,
    pub attribution: ChangeAttribution,
}

#[derive(Clone, Debug)]
pub struct ReviewTurn {
    pub id: Uuid,
    pub before: Arc<RepositorySnapshot>,
    pub after: Arc<RepositorySnapshot>,
    pub changes: Vec<TurnFileChange>,
}

#[derive(Clone, Debug)]
pub enum ReviewAction {
    StageFiles {
        paths: Vec<String>,
    },
    UnstageFiles {
        paths: Vec<String>,
    },
    /// Commits exactly the reviewed index; commit-tree deliberately runs no hooks.
    Commit {
        message: String,
    },
}

#[derive(Clone, Debug)]
enum PlannedAction {
    Git(ReviewAction),
    Undo {
        path: String,
        before: FileImage,
        after: FileImage,
    },
}

/// The private fence cannot be changed after the concrete preview is presented.
#[derive(Clone, Debug)]
pub struct ReviewPlan {
    pub id: Uuid,
    pub summary: String,
    pub paths: Vec<String>,
    snapshot: RepositorySnapshot,
    action: PlannedAction,
}

#[derive(Clone, Copy, Debug)]
pub struct ReviewConfirmation {
    plan_id: Uuid,
}

impl ReviewPlan {
    /// Call only from an explicit user confirmation of this exact plan.
    pub fn confirm(&self) -> ReviewConfirmation {
        ReviewConfirmation { plan_id: self.id }
    }
}

#[derive(Clone, Debug)]
pub struct MutationOutcome {
    pub paths: Vec<String>,
    pub commit: Option<String>,
}

pub struct AgentReviewBackend<E = SystemReviewExecutor> {
    executor: E,
    limits: ReviewLimits,
}

impl Default for AgentReviewBackend<SystemReviewExecutor> {
    fn default() -> Self {
        Self::new(SystemReviewExecutor, ReviewLimits::default())
    }
}

impl<E: ReviewExecutor> AgentReviewBackend<E> {
    pub fn new(executor: E, limits: ReviewLimits) -> Self {
        Self { executor, limits }
    }

    fn command(&self, cwd: &Path, args: &[&str]) -> ReviewCommand {
        ReviewCommand {
            program: "git".into(),
            // Disable external diff drivers and fsmonitor hooks on reads.
            args: [
                "--no-optional-locks",
                "--literal-pathspecs",
                "-c",
                "core.fsmonitor=false",
            ]
            .into_iter()
            .chain(args.iter().copied())
            .map(str::to_owned)
            .collect(),
            cwd: cwd.to_owned(),
            stdin: Vec::new(),
            env: BTreeMap::new(),
            max_output_bytes: self.limits.max_command_bytes,
            timeout: self.limits.command_timeout,
        }
    }

    fn run(&self, command: ReviewCommand) -> ReviewResult<Vec<u8>> {
        let result = self.executor.run(&command)?;
        if result.stdout.len() > command.max_output_bytes || result.stderr.len() > 64 * 1024 {
            return refuse("Git output exceeded the review limit");
        }
        if result.exit_code != 0 {
            return Err(ReviewError::Command {
                program: command.program,
                detail: String::from_utf8_lossy(&result.stderr)
                    .chars()
                    .take(2048)
                    .collect(),
            });
        }
        Ok(result.stdout)
    }

    fn git(&self, cwd: &Path, args: &[&str]) -> ReviewResult<Vec<u8>> {
        self.run(self.command(cwd, args))
    }

    fn read_identity(&self, root: &Path) -> ReviewResult<(Option<String>, Option<String>)> {
        let symbolic = self
            .executor
            .run(&self.command(root, &["symbolic-ref", "--quiet", "HEAD"]))?;
        let head_ref = match symbolic.exit_code {
            0 => {
                let name = output_text(&symbolic.stdout)?;
                if !name.starts_with("refs/heads/") || !valid_relative_path(&name) {
                    return refuse("Unsafe HEAD reference");
                }
                Some(name)
            }
            1 => None,
            _ => return refuse("Cannot determine the current HEAD reference"),
        };
        let resolved = self
            .executor
            .run(&self.command(root, &["rev-parse", "--verify", "HEAD"]))?;
        let head = if resolved.exit_code == 0 {
            Some(object_id(&resolved.stdout)?)
        } else if head_ref.is_some() && matches!(resolved.exit_code, 1 | 128) {
            // Distinguish an unborn ref from an unreadable/corrupt existing ref.
            let exists = self.executor.run(&self.command(
                root,
                &[
                    "show-ref",
                    "--verify",
                    "--quiet",
                    head_ref.as_deref().unwrap_or_default(),
                ],
            ))?;
            if exists.exit_code != 1 {
                return refuse("HEAD could not be resolved safely");
            }
            None
        } else {
            return refuse("HEAD could not be resolved safely");
        };
        Ok((head, head_ref))
    }

    /// Captures actual Git state and exact bounded worktree bytes. No writes.
    pub fn observe(&self, directory: &Path) -> ReviewResult<RepositorySnapshot> {
        let requested = fs::canonicalize(directory)?;
        let root = fs::canonicalize(PathBuf::from(output_text(
            &self.git(&requested, &["rev-parse", "--show-toplevel"])?,
        )?))?;
        let git_dir = fs::canonicalize(PathBuf::from(output_text(
            &self.git(&root, &["rev-parse", "--absolute-git-dir"])?,
        )?))?;
        let common = output_text(&self.git(
            &root,
            &["rev-parse", "--path-format=absolute", "--git-common-dir"],
        )?)?;
        let common_dir = fs::canonicalize(common)?;
        let (head, head_ref) = self.read_identity(&root)?;
        let index = bounded_optional_file(&git_dir.join("index"), self.limits.max_command_bytes)?;
        let status = parse_status_z(&self.git(
            &root,
            &["status", "--porcelain=v1", "-z", "--untracked-files=all"],
        )?)?;
        if status.len() > self.limits.max_files {
            return refuse("Too many changed files for a bounded review");
        }
        let tracked = self.git(&root, &["ls-files", "--stage", "-z"])?;
        let index_modes = parse_index_modes(&tracked)?;
        let untracked = self.git(&root, &["ls-files", "--others", "--exclude-standard", "-z"])?;
        let mut names = index_modes.keys().cloned().collect::<BTreeSet<_>>();
        for name in untracked
            .split(|byte| *byte == 0)
            .filter(|name| !name.is_empty())
        {
            names.insert(utf8_path(name)?);
        }
        for entry in &status {
            names.insert(entry.path.clone());
            if let Some(original) = &entry.original_path {
                names.insert(original.clone());
            }
        }
        let mut files = BTreeMap::new();
        let mut used = 0usize;
        let mut refusals = Vec::new();
        let mut complete = true;
        for (ordinal, path) in names.iter().enumerate() {
            if ordinal >= self.limits.max_files || used >= self.limits.max_snapshot_bytes {
                complete = false;
                refusals.push("Snapshot file/byte limit reached; turn attribution and mutations are unavailable".into());
                break;
            }
            let file = read_image(
                &root,
                path,
                self.limits
                    .max_file_bytes
                    .min(self.limits.max_snapshot_bytes.saturating_sub(used)),
                index_modes.get(path).copied(),
            )?;
            if let FileImage::Refused(reason) = &file {
                refusals.push(format!("{path}: {reason}"));
            }
            used = used.saturating_add(file.size());
            files.insert(path.clone(), file);
        }
        let staged_counts = parse_numstat_z(&self.git(
            &root,
            &[
                "diff",
                "--cached",
                "--numstat",
                "-z",
                "--no-renames",
                "--no-ext-diff",
                "--no-textconv",
            ],
        )?)?;
        let unstaged_counts = parse_numstat_z(&self.git(
            &root,
            &[
                "diff",
                "--numstat",
                "-z",
                "--no-renames",
                "--no-ext-diff",
                "--no-textconv",
            ],
        )?)?;
        let mut diffs = Vec::new();
        let mut diff_bytes = 0usize;
        for entry in &status {
            if entry.is_untracked() {
                let (counts, unified) = untracked_diff(&entry.path, files.get(&entry.path));
                diff_bytes = diff_bytes.saturating_add(unified.len());
                if diff_bytes > self.limits.max_command_bytes {
                    return refuse("Combined diff exceeds the bounded review limit");
                }
                diffs.push(FileDiff {
                    path: entry.path.clone(),
                    original_path: None,
                    section: ReviewSection::Untracked,
                    counts,
                    unified,
                    conflicted: false,
                });
                continue;
            }
            for (active, section, counts) in [
                (entry.is_staged(), ReviewSection::Staged, &staged_counts),
                (
                    entry.is_unstaged(),
                    ReviewSection::Unstaged,
                    &unstaged_counts,
                ),
            ] {
                if !active {
                    continue;
                }
                let mut args = vec![
                    "diff",
                    "--no-ext-diff",
                    "--no-textconv",
                    "--no-color",
                    "--find-renames",
                    "--unified=3",
                ];
                if section == ReviewSection::Staged {
                    args.push("--cached");
                }
                args.extend(["--", &entry.path]);
                if let Some(original) = &entry.original_path {
                    args.push(original);
                }
                let bytes = self.git(&root, &args)?;
                diff_bytes = diff_bytes.saturating_add(bytes.len());
                if diff_bytes > self.limits.max_command_bytes {
                    return refuse("Combined diff exceeds the bounded review limit");
                }
                let unified = String::from_utf8_lossy(&bytes).into_owned();
                let mut line_counts = counts.get(&entry.path).copied().unwrap_or_default();
                if let Some(original) = &entry.original_path {
                    if let Some(old) = counts.get(original) {
                        line_counts.additions = line_counts
                            .additions
                            .zip(old.additions)
                            .map(|(a, b)| a.saturating_add(b));
                        line_counts.deletions = line_counts
                            .deletions
                            .zip(old.deletions)
                            .map(|(a, b)| a.saturating_add(b));
                    }
                }
                diffs.push(FileDiff {
                    path: entry.path.clone(),
                    original_path: entry.original_path.clone(),
                    section,
                    counts: line_counts,
                    unified,
                    conflicted: entry.is_conflicted(),
                });
            }
        }
        // A read racing another Git writer is not a valid mutation fence.
        let final_identity = self.read_identity(&root)?;
        if final_identity != (head.clone(), head_ref.clone())
            || bounded_optional_file(&git_dir.join("index"), self.limits.max_command_bytes)?
                != index
        {
            return refuse("Repository changed while being read; refresh the review");
        }
        Ok(RepositorySnapshot {
            id: Uuid::new_v4(),
            root,
            git_dir,
            common_dir,
            branch: head_ref
                .as_deref()
                .and_then(|name| name.strip_prefix("refs/heads/"))
                .map(str::to_owned),
            head,
            head_ref,
            status,
            diffs,
            files,
            complete,
            refusals,
            index,
            index_modes,
        })
    }

    pub fn finish_turn(&self, before: RepositorySnapshot) -> ReviewResult<ReviewTurn> {
        self.finish_turn_with_evidence(before, &[])
    }

    pub fn finish_turn_with_evidence(
        &self,
        before: RepositorySnapshot,
        evidence: &[OwnedFileEdit],
    ) -> ReviewResult<ReviewTurn> {
        let after = self.observe(&before.root)?;
        attribute_turn(before, after, evidence)
    }

    pub fn plan(
        &self,
        snapshot: &RepositorySnapshot,
        action: ReviewAction,
    ) -> ReviewResult<ReviewPlan> {
        if !snapshot.can_mutate() {
            return refuse("Incomplete or conflicted repositories cannot be mutated from review");
        }
        let (paths, summary) = match &action {
            ReviewAction::StageFiles { paths } | ReviewAction::UnstageFiles { paths } => {
                if paths.is_empty() || paths.len() > 256 {
                    return refuse("Select between 1 and 256 exact file paths");
                }
                let unique = paths.iter().collect::<BTreeSet<_>>();
                if unique.len() != paths.len() {
                    return refuse("Duplicate review paths");
                }
                for path in paths {
                    validate_mutable_path(snapshot, path)?;
                    let entry = snapshot.status.iter().find(|entry| {
                        &entry.path == path || entry.original_path.as_ref() == Some(path)
                    });
                    if entry.is_none() {
                        return refuse("Selected path is absent from the reviewed changes");
                    }
                    if let Some(entry) = entry {
                        if let Some(original) = &entry.original_path {
                            if !paths.contains(original) || !paths.contains(&entry.path) {
                                return refuse(
                                    "A rename must explicitly include both its original and destination paths",
                                );
                            }
                        }
                    }
                }
                let verb = if matches!(&action, ReviewAction::StageFiles { .. }) {
                    "Stage exact bytes (without content filters) of"
                } else {
                    "Unstage"
                };
                (
                    paths.clone(),
                    format!(
                        "{verb} {} reviewed file(s) in {}",
                        paths.len(),
                        snapshot.root.display()
                    ),
                )
            }
            ReviewAction::Commit { message } => {
                if message.trim().is_empty() || message.len() > 64 * 1024 || message.contains('\0')
                {
                    return refuse("A bounded nonempty commit message is required");
                }
                if snapshot.head_ref.is_none() {
                    return refuse(
                        "Create commits on a named branch; detached HEAD is not supported",
                    );
                }
                if snapshot.staged_count() == 0 {
                    return refuse("There are no staged changes to commit");
                }
                for path in snapshot.files.keys() {
                    validate_mutable_path(snapshot, path)?;
                }
                (
                    snapshot
                        .status
                        .iter()
                        .filter(|entry| entry.is_staged())
                        .map(|entry| entry.path.clone())
                        .collect(),
                    format!(
                        "Commit the exact reviewed index on {} in {}. Git hooks are not run.\n\n{}",
                        snapshot.branch.as_deref().unwrap_or_default(),
                        snapshot.root.display(),
                        message
                    ),
                )
            }
        };
        Ok(ReviewPlan {
            id: Uuid::new_v4(),
            summary,
            paths,
            snapshot: snapshot.clone(),
            action: PlannedAction::Git(action),
        })
    }

    /// Undo is intentionally one file per confirmation. The baseline may be
    /// dirty; its exact bytes are restored, rather than `git restore`/HEAD.
    pub fn plan_undo(
        &self,
        current: &RepositorySnapshot,
        turn: &ReviewTurn,
        path: &str,
    ) -> ReviewResult<ReviewPlan> {
        if !current.can_mutate()
            || current.root != turn.after.root
            || turn.before.head != turn.after.head
            || turn.before.index != turn.after.index
        {
            return refuse(
                "Undo requires the same complete repository, HEAD, and unchanged turn index",
            );
        }
        if current.head != turn.after.head
            || current.index != turn.after.index
            || current.head_ref != turn.after.head_ref
        {
            return refuse("Repository changed since this turn; undo is stale");
        }
        let change = turn
            .changes
            .iter()
            .find(|change| change.path == path)
            .ok_or_else(|| {
                ReviewError::Refused("This file was not changed in the selected turn".into())
            })?;
        if change.attribution != ChangeAttribution::ProvenAgentEdit {
            return refuse(
                "Undo is unavailable: exact agent-owned before/after edit evidence was not supplied",
            );
        }
        validate_mutable_path(current, path)?;
        if current
            .files
            .get(path)
            .cloned()
            .unwrap_or(FileImage::Missing)
            != change.after
        {
            return refuse(
                "File has changed since the agent edit; unrelated work will not be discarded",
            );
        }
        Ok(ReviewPlan {
            id: Uuid::new_v4(),
            summary: format!(
                "Restore {path} to its exact pre-turn bytes in {}. Any pre-existing edits are preserved.",
                current.root.display()
            ),
            paths: vec![path.to_owned()],
            snapshot: current.clone(),
            action: PlannedAction::Undo {
                path: path.to_owned(),
                before: change.before.clone(),
                after: change.after.clone(),
            },
        })
    }

    pub fn execute(
        &self,
        plan: &ReviewPlan,
        confirmation: ReviewConfirmation,
    ) -> ReviewResult<MutationOutcome> {
        if confirmation.plan_id != plan.id {
            return refuse("Confirmation belongs to a different review plan");
        }
        let mut locks = RepositoryLocks::acquire(
            &plan.snapshot,
            !matches!(
                &plan.action,
                PlannedAction::Git(ReviewAction::Commit { .. })
            ),
        )?;
        self.validate_fence(&plan.snapshot, &plan.paths)?;
        let result = match &plan.action {
            PlannedAction::Undo {
                path,
                before,
                after,
            } => {
                restore_exact_file(
                    &plan.snapshot.root,
                    path,
                    before,
                    after,
                    self.limits.max_file_bytes,
                )?;
                MutationOutcome {
                    paths: vec![path.clone()],
                    commit: None,
                }
            }
            PlannedAction::Git(action) => {
                let temporary = TemporaryIndex::new(&plan.snapshot)?;
                match action {
                    ReviewAction::StageFiles { paths } => {
                        for path in paths {
                            match plan.snapshot.files.get(path).unwrap_or(&FileImage::Missing) {
                                FileImage::Missing => {
                                    self.run(temporary.command(self.command(
                                        &plan.snapshot.root,
                                        &["update-index", "--force-remove", "--", path],
                                    )))?;
                                }
                                FileImage::Regular { bytes, mode, .. } => {
                                    let mut hash = self.command(
                                        &plan.snapshot.root,
                                        &["hash-object", "-w", "--stdin", "--no-filters"],
                                    );
                                    hash.stdin = bytes.clone();
                                    let oid = object_id(&self.run(hash)?)?;
                                    self.run(temporary.command(self.command(
                                        &plan.snapshot.root,
                                        &[
                                            "update-index",
                                            "--add",
                                            "--cacheinfo",
                                            &format!("{mode:o}"),
                                            &oid,
                                            path,
                                        ],
                                    )))?;
                                }
                                FileImage::Refused(_) => {
                                    return refuse("Unsafe file cannot be staged");
                                }
                            }
                        }
                        self.validate_fence(&plan.snapshot, paths)?;
                        locks.install_index(&temporary.path, self.limits.max_command_bytes)?;
                        MutationOutcome {
                            paths: paths.clone(),
                            commit: None,
                        }
                    }
                    ReviewAction::UnstageFiles { paths } => {
                        for path in paths {
                            if let Some(head) = &plan.snapshot.head {
                                self.run(temporary.command(self.command(
                                    &plan.snapshot.root,
                                    &["reset", "--quiet", head, "--", path],
                                )))?;
                            } else {
                                self.run(temporary.command(self.command(
                                    &plan.snapshot.root,
                                    &["update-index", "--force-remove", "--", path],
                                )))?;
                            }
                        }
                        self.validate_fence(&plan.snapshot, paths)?;
                        locks.install_index(&temporary.path, self.limits.max_command_bytes)?;
                        MutationOutcome {
                            paths: paths.clone(),
                            commit: None,
                        }
                    }
                    ReviewAction::Commit { message } => {
                        self.validate_fence(
                            &plan.snapshot,
                            &plan.snapshot.files.keys().cloned().collect::<Vec<_>>(),
                        )?;
                        let tree = object_id(&self.run(
                            temporary.command(self.command(&plan.snapshot.root, &["write-tree"])),
                        )?)?;
                        let mut args = vec!["commit-tree", &tree];
                        if let Some(parent) = &plan.snapshot.head {
                            args.extend(["-p", parent]);
                        }
                        let mut command = self.command(&plan.snapshot.root, &args);
                        command.stdin = message.as_bytes().to_vec();
                        command.stdin.push(b'\n');
                        let commit = object_id(&self.run(command)?)?;
                        self.validate_fence(
                            &plan.snapshot,
                            &plan.snapshot.files.keys().cloned().collect::<Vec<_>>(),
                        )?;
                        // update-ref compares the original object atomically. HEAD.lock
                        // and index.lock remain held; checkout/index writers refuse.
                        self.git(
                            &plan.snapshot.root,
                            &[
                                "update-ref",
                                "-m",
                                "ShellDeck confirmed review commit",
                                plan.snapshot.head_ref.as_deref().unwrap_or_default(),
                                &commit,
                                plan.snapshot.head.as_deref().unwrap_or(""),
                            ],
                        )?;
                        MutationOutcome {
                            paths: plan.paths.clone(),
                            commit: Some(commit),
                        }
                    }
                }
            }
        };
        Ok(result)
    }

    fn validate_fence(&self, snapshot: &RepositorySnapshot, paths: &[String]) -> ReviewResult<()> {
        if fs::canonicalize(&snapshot.root)? != snapshot.root
            || fs::canonicalize(&snapshot.git_dir)? != snapshot.git_dir
            || fs::canonicalize(&snapshot.common_dir)? != snapshot.common_dir
        {
            return refuse("Repository location changed since confirmation");
        }
        if self.read_identity(&snapshot.root)? != (snapshot.head.clone(), snapshot.head_ref.clone())
        {
            return refuse("HEAD changed since the reviewed plan; refresh and confirm again");
        }
        if bounded_optional_file(
            &snapshot.git_dir.join("index"),
            self.limits.max_command_bytes,
        )? != snapshot.index
        {
            return refuse("Git index changed since the reviewed plan; refresh and confirm again");
        }
        for path in paths {
            validate_mutable_path(snapshot, path)?;
            let actual = read_image(
                &snapshot.root,
                path,
                self.limits.max_file_bytes,
                snapshot.index_modes.get(path).copied(),
            )?;
            if actual
                != snapshot
                    .files
                    .get(path)
                    .cloned()
                    .unwrap_or(FileImage::Missing)
            {
                return refuse(format!(
                    "{path} changed since the reviewed plan; unrelated work was preserved"
                ));
            }
        }
        Ok(())
    }

    /// Explicit delivery command previews. This method never contacts a remote.
    /// UI must separately confirm these commands and, for gh mutations, verify
    /// `gh auth status` and `gh api user --jq .login` per repository policy.
    pub fn delivery_commands(
        &self,
        snapshot: &RepositorySnapshot,
        action: DeliveryAction,
    ) -> ReviewResult<Vec<ReviewCommand>> {
        let branch = snapshot
            .branch
            .as_deref()
            .ok_or_else(|| ReviewError::Refused("Delivery requires a named branch".into()))?;
        match action {
            DeliveryAction::Push => Ok(vec![self.command(
                &snapshot.root,
                &["push", "--", "origin", &format!("HEAD:refs/heads/{branch}")],
            )]),
            DeliveryAction::CreatePullRequest {
                title,
                body_file,
                base,
            } => {
                if title.trim().is_empty()
                    || title.len() > 1024
                    || title.contains('\0')
                    || base.is_empty()
                    || base.starts_with('-')
                    || base.contains('\0')
                {
                    return refuse("Invalid pull request title/base");
                }
                let body = fs::canonicalize(body_file)?;
                if !body.is_file() {
                    return refuse("Pull request body must be a prepared regular file");
                }
                let body = body
                    .to_str()
                    .ok_or_else(|| ReviewError::Refused("Non-UTF-8 body path".into()))?;
                let gh = |args: &[&str]| ReviewCommand {
                    program: "gh".into(),
                    args: args.iter().map(|arg| (*arg).to_owned()).collect(),
                    cwd: snapshot.root.clone(),
                    stdin: vec![],
                    env: BTreeMap::new(),
                    max_output_bytes: self.limits.max_command_bytes,
                    timeout: self.limits.command_timeout,
                };
                Ok(vec![
                    gh(&["auth", "status"]),
                    gh(&["api", "user", "--jq", ".login"]),
                    gh(&[
                        "pr",
                        "create",
                        "--title",
                        &title,
                        "--body-file",
                        body,
                        "--base",
                        &base,
                        "--head",
                        branch,
                    ]),
                ])
            }
        }
    }
}

/// Pure attribution of two already captured snapshots. Neither observes nor writes.
pub fn attribute_turn(
    before: RepositorySnapshot,
    after: RepositorySnapshot,
    evidence: &[OwnedFileEdit],
) -> ReviewResult<ReviewTurn> {
    attribute_shared_turn(Arc::new(before), Arc::new(after), evidence)
}

/// Shared snapshots avoid copying entire bounded worktrees into every UI card.
pub fn attribute_shared_turn(
    before: Arc<RepositorySnapshot>,
    after: Arc<RepositorySnapshot>,
    evidence: &[OwnedFileEdit],
) -> ReviewResult<ReviewTurn> {
    if !before.complete
        || !after.complete
        || before.root != after.root
        || before.git_dir != after.git_dir
    {
        return refuse("A complete before/after snapshot is required for turn attribution");
    }
    let paths = before
        .files
        .keys()
        .chain(after.files.keys())
        .cloned()
        .collect::<BTreeSet<_>>();
    let mut changes = Vec::new();
    for path in paths {
        let old = before
            .files
            .get(&path)
            .cloned()
            .unwrap_or(FileImage::Missing);
        let new = after
            .files
            .get(&path)
            .cloned()
            .unwrap_or(FileImage::Missing);
        if old == new {
            continue;
        }
        let owned = old.exact()
            && new.exact()
            && evidence
                .iter()
                .any(|edit| edit.path == path && edit.before == old && edit.after == new);
        changes.push(TurnFileChange {
            baseline_dirty: before
                .status
                .iter()
                .any(|entry| entry.path == path || entry.original_path.as_ref() == Some(&path)),
            path,
            before: old,
            after: new,
            attribution: if owned {
                ChangeAttribution::ProvenAgentEdit
            } else {
                ChangeAttribution::ChangedDuringRun
            },
        });
    }
    Ok(ReviewTurn {
        id: Uuid::new_v4(),
        before,
        after,
        changes,
    })
}

#[derive(Clone, Debug)]
pub enum DeliveryAction {
    Push,
    CreatePullRequest {
        title: String,
        body_file: PathBuf,
        base: String,
    },
}

fn refuse<T>(reason: impl Into<String>) -> ReviewResult<T> {
    Err(ReviewError::Refused(reason.into()))
}

fn output_text(bytes: &[u8]) -> ReviewResult<String> {
    let text = std::str::from_utf8(bytes)
        .map_err(|_| ReviewError::Refused("Git returned a non-UTF-8 identity/path".into()))?;
    let text = text
        .strip_suffix('\n')
        .unwrap_or(text)
        .strip_suffix('\r')
        .unwrap_or_else(|| text.strip_suffix('\n').unwrap_or(text));
    if text.is_empty() || text.contains('\0') {
        return refuse("Git returned an empty or invalid identity/path");
    }
    Ok(text.to_owned())
}

fn object_id(bytes: &[u8]) -> ReviewResult<String> {
    let text = output_text(bytes)?;
    if !matches!(text.len(), 40 | 64) || !text.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return refuse("Git returned an invalid object id");
    }
    Ok(text)
}

fn utf8_path(bytes: &[u8]) -> ReviewResult<String> {
    let text = std::str::from_utf8(bytes).map_err(|_| {
        ReviewError::Refused("Non-UTF-8 Git paths cannot be reviewed safely".into())
    })?;
    if !valid_relative_path(text) {
        return refuse("Git reported an unsafe relative path");
    }
    Ok(text.to_owned())
}

fn valid_relative_path(path: &str) -> bool {
    !path.is_empty()
        && path.len() <= 4096
        && !path.contains(['\0', '\\'])
        && !path.split('/').any(|part| {
            part.is_empty() || part == "." || part == ".." || part.eq_ignore_ascii_case(".git")
        })
        && Path::new(path)
            .components()
            .all(|part| matches!(part, Component::Normal(_)))
}

fn parse_index_modes(bytes: &[u8]) -> ReviewResult<BTreeMap<String, u32>> {
    let mut result = BTreeMap::new();
    for record in bytes
        .split(|byte| *byte == 0)
        .filter(|record| !record.is_empty())
    {
        let tab = record
            .iter()
            .position(|byte| *byte == b'\t')
            .ok_or_else(|| ReviewError::Refused("Invalid Git index entry".into()))?;
        let fields = std::str::from_utf8(&record[..tab])
            .map_err(|_| ReviewError::Refused("Invalid Git index entry".into()))?
            .split(' ')
            .collect::<Vec<_>>();
        if fields.len() != 3 {
            return refuse("Malformed Git index entry");
        }
        let mode = u32::from_str_radix(fields[0], 8)
            .map_err(|_| ReviewError::Refused("Invalid Git file mode".into()))?;
        if !matches!(fields[2], "0" | "1" | "2" | "3") {
            return refuse("Invalid Git index stage");
        }
        result.insert(utf8_path(&record[tab + 1..])?, mode);
    }
    Ok(result)
}

fn checked_path(root: &Path, relative: &str) -> ReviewResult<PathBuf> {
    if !valid_relative_path(relative) {
        return refuse("Unsafe repository-relative path");
    }
    let mut current = root.to_owned();
    let parts = relative.split('/').collect::<Vec<_>>();
    for (ordinal, part) in parts.iter().enumerate() {
        current.push(part);
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return refuse(
                    "Symlink paths and symlink ancestors are not supported for review mutations",
                );
            }
            Ok(metadata) if ordinal + 1 < parts.len() && !metadata.is_dir() => {
                return refuse("A parent path is not a regular directory");
            }
            Ok(_) => (),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
            Err(error) => return Err(error.into()),
        }
    }
    Ok(current)
}

fn read_image(
    root: &Path,
    relative: &str,
    limit: usize,
    index_mode: Option<u32>,
) -> ReviewResult<FileImage> {
    read_image_from_file(root, relative, limit, index_mode, None)
}

fn read_image_from_file(
    root: &Path,
    relative: &str,
    limit: usize,
    index_mode: Option<u32>,
    opened_file: Option<&mut File>,
) -> ReviewResult<FileImage> {
    let path = match checked_path(root, relative) {
        Ok(path) => path,
        Err(ReviewError::Refused(reason)) => return Ok(FileImage::Refused(reason)),
        Err(error) => return Err(error),
    };
    let metadata = match fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(FileImage::Missing);
        }
        Err(error) => return Err(error.into()),
    };
    if !metadata.is_file() {
        return Ok(FileImage::Refused(
            "Submodules, directories, and special files are not mutable review files".into(),
        ));
    }
    if metadata.len() > limit as u64 {
        return Ok(FileImage::Refused(
            "File exceeds the exact snapshot byte limit".into(),
        ));
    }
    let mut newly_opened;
    let file = match opened_file {
        Some(file) => file,
        None => {
            newly_opened = File::open(&path)?;
            &mut newly_opened
        }
    };
    if !same_file_metadata(&metadata, &file.metadata()?)
        || fs::symlink_metadata(&path)?.file_type().is_symlink()
    {
        return Ok(FileImage::Refused(
            "File identity changed while being read".into(),
        ));
    }
    file.rewind()?;
    let (bytes, exceeded) = read_capped(&mut *file, limit)?;
    if exceeded
        || !same_file_metadata(&metadata, &file.metadata()?)
        || !same_file_metadata(&metadata, &fs::symlink_metadata(&path)?)
    {
        return Ok(FileImage::Refused(
            "File changed or exceeded its limit while being read".into(),
        ));
    }
    Ok(FileImage::Regular {
        bytes,
        mode: regular_mode(&metadata, index_mode),
        permission_bits: permission_bits(&metadata),
        readonly: metadata.permissions().readonly(),
    })
}

fn permission_bits(metadata: &fs::Metadata) -> Option<u32> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        Some(metadata.permissions().mode() & 0o7777)
    }
    #[cfg(not(unix))]
    {
        let _ = metadata;
        None
    }
}

fn same_file_metadata(left: &fs::Metadata, right: &fs::Metadata) -> bool {
    if left.len() != right.len()
        || left.modified().ok() != right.modified().ok()
        || left.file_type() != right.file_type()
    {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        left.dev() == right.dev() && left.ino() == right.ino() && left.mode() == right.mode()
    }
    #[cfg(not(unix))]
    {
        left.permissions().readonly() == right.permissions().readonly()
    }
}

fn regular_mode(metadata: &fs::Metadata, index_mode: Option<u32>) -> u32 {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = index_mode;
        if metadata.permissions().mode() & 0o111 != 0 {
            0o100755
        } else {
            0o100644
        }
    }
    #[cfg(not(unix))]
    {
        let _ = metadata;
        if index_mode == Some(0o100755) {
            0o100755
        } else {
            0o100644
        }
    }
}

fn bounded_optional_file(path: &Path, limit: usize) -> ReviewResult<Option<Vec<u8>>> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > limit as u64 {
        return refuse("Git index/body is unsafe or exceeds its byte limit");
    }
    let file = File::open(path)?;
    if !same_file_metadata(&metadata, &file.metadata()?) {
        return refuse("Index identity changed while opening it");
    }
    let (bytes, exceeded) = read_capped(file, limit)?;
    if exceeded || !same_file_metadata(&metadata, &fs::symlink_metadata(path)?) {
        return refuse("Index changed while being read");
    }
    Ok(Some(bytes))
}

fn untracked_diff(path: &str, image: Option<&FileImage>) -> (LineCounts, String) {
    let Some(FileImage::Regular { bytes, .. }) = image else {
        return (
            LineCounts::default(),
            "Exact preview unavailable (unsafe, missing, or oversized file).".into(),
        );
    };
    let Ok(text) = std::str::from_utf8(bytes) else {
        return (
            LineCounts::default(),
            "Binary file (line counts unknown).".into(),
        );
    };
    if bytes.contains(&0) {
        return (
            LineCounts::default(),
            "Binary file (line counts unknown).".into(),
        );
    }
    let lines = text.lines().count() as u64;
    let mut diff = format!("--- /dev/null\n+++ b/{path}\n@@ -0,0 +1,{lines} @@\n");
    for line in text.lines() {
        diff.push('+');
        diff.push_str(line);
        diff.push('\n');
    }
    if !text.is_empty() && !text.ends_with('\n') {
        diff.push_str("\\ No newline at end of file\n");
    }
    (
        LineCounts {
            additions: Some(lines),
            deletions: Some(0),
        },
        diff,
    )
}

fn validate_mutable_path(snapshot: &RepositorySnapshot, path: &str) -> ReviewResult<()> {
    if !valid_relative_path(path) {
        return refuse("Invalid review path");
    }
    match snapshot.files.get(path) {
        Some(FileImage::Refused(reason)) => refuse(format!("{path}: {reason}")),
        Some(_) => Ok(()),
        None => refuse("Path has no exact snapshot; refresh the review"),
    }
}

struct LockFile {
    path: PathBuf,
    file: Option<File>,
}

impl LockFile {
    fn create(path: PathBuf) -> ReviewResult<Self> {
        let mut options = OpenOptions::new();
        options.write(true).read(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let file = options.open(&path).map_err(|error| {
            ReviewError::Refused(format!(
                "Cannot acquire {}: {error}. Another Git operation may be running.",
                path.display()
            ))
        })?;
        Ok(Self {
            path,
            file: Some(file),
        })
    }
}

impl Drop for LockFile {
    fn drop(&mut self) {
        self.file.take();
        let _ = fs::remove_file(&self.path);
    }
}

struct RepositoryLocks {
    index: LockFile,
    _head: LockFile,
    _branch: Option<LockFile>,
}

impl RepositoryLocks {
    fn acquire(snapshot: &RepositorySnapshot, lock_branch: bool) -> ReviewResult<Self> {
        // Canonical roots and a no-symlink branch ancestry are checked before
        // creating lockfiles. Existing locks are never removed or stolen.
        if fs::canonicalize(&snapshot.git_dir)? != snapshot.git_dir
            || fs::canonicalize(&snapshot.common_dir)? != snapshot.common_dir
        {
            return refuse("Git directory identity changed");
        }
        let head = LockFile::create(snapshot.git_dir.join("HEAD.lock"))?;
        let branch = if lock_branch {
            if let Some(reference) = &snapshot.head_ref {
                let reference_path = checked_path(&snapshot.common_dir, reference)?;
                let parent = reference_path
                    .parent()
                    .ok_or_else(|| ReviewError::Refused("Reference has no parent".into()))?;
                if !parent.is_dir() {
                    return refuse(
                        "Reference parent is missing; refresh after creating the branch",
                    );
                }
                Some(LockFile::create(reference_path.with_file_name(format!(
                        "{}.lock",
                        reference_path
                            .file_name()
                            .and_then(|part| part.to_str())
                            .unwrap_or_default()
                    )))?)
            } else {
                None
            }
        } else {
            None
        };
        let index = LockFile::create(snapshot.git_dir.join("index.lock"))?;
        Ok(Self {
            index,
            _head: head,
            _branch: branch,
        })
    }

    fn install_index(&mut self, source: &Path, limit: usize) -> ReviewResult<()> {
        let bytes = bounded_optional_file(source, limit)?
            .ok_or_else(|| ReviewError::Refused("Prepared index is missing".into()))?;
        let file =
            self.index.file.as_mut().ok_or_else(|| {
                ReviewError::Refused("Index lock has already been consumed".into())
            })?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        self.index.file.take();
        let destination = self.index.path.with_file_name("index");
        fs::rename(&self.index.path, destination)?;
        Ok(())
    }
}

struct TemporaryIndex {
    path: PathBuf,
}

impl TemporaryIndex {
    fn new(snapshot: &RepositorySnapshot) -> ReviewResult<Self> {
        let path = snapshot
            .git_dir
            .join(format!("shelldeck-review-{}.index", Uuid::new_v4()));
        if let Some(bytes) = &snapshot.index {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)?;
            file.write_all(bytes)?;
            file.sync_all()?;
        }
        Ok(Self { path })
    }
    fn command(&self, mut command: ReviewCommand) -> ReviewCommand {
        command.env.insert(
            "GIT_INDEX_FILE".into(),
            self.path.to_string_lossy().into_owned(),
        );
        command
    }
}

impl Drop for TemporaryIndex {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
        let mut lock = self.path.as_os_str().to_os_string();
        lock.push(".lock");
        let _ = fs::remove_file(PathBuf::from(lock));
    }
}

fn restore_exact_file(
    root: &Path,
    relative: &str,
    before: &FileImage,
    after: &FileImage,
    limit: usize,
) -> ReviewResult<()> {
    let path = checked_path(root, relative)?;
    let after_mode = match after {
        FileImage::Regular { mode, .. } => Some(*mode),
        _ => None,
    };
    let mut locked_file = match after {
        FileImage::Regular { .. } => {
            let file = OpenOptions::new().read(true).write(true).open(&path)?;
            file.try_lock_exclusive().map_err(|error| {
                ReviewError::Refused(format!("File is busy; undo refused: {error}"))
            })?;
            Some(file)
        }
        FileImage::Missing => None,
        FileImage::Refused(_) => return refuse("Unknown after bytes cannot be undone"),
    };
    // Windows byte-range locks also reject reads through another handle in
    // this process. Revalidate through the handle that owns the exclusive
    // lock, while still checking that its identity matches the current path.
    if read_image_from_file(root, relative, limit, after_mode, locked_file.as_mut())? != *after {
        return refuse("File changed before undo; unrelated edits were preserved");
    }
    if let Some(file) = &locked_file {
        if !same_file_metadata(&file.metadata()?, &fs::symlink_metadata(&path)?) {
            return refuse("File identity changed before undo");
        }
    }
    match before {
        FileImage::Missing => {
            fs::remove_file(&path)?;
        }
        FileImage::Regular {
            bytes,
            permission_bits,
            readonly,
            ..
        } => {
            let parent = path
                .parent()
                .ok_or_else(|| ReviewError::Refused("File parent is missing".into()))?;
            if !parent.is_dir() {
                return refuse("Undo will not create missing parent directories");
            }
            let temporary = parent.join(format!(".shelldeck-undo-{}", Uuid::new_v4()));
            let mut cleanup = LockFile::create(temporary.clone())?;
            let output = cleanup.file.as_mut().expect("new file");
            output.write_all(bytes)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = readonly;
                let bits = permission_bits.ok_or_else(|| {
                    ReviewError::Refused("Exact baseline permissions are unavailable".into())
                })?;
                if bits & 0o7000 != 0 {
                    return refuse("Undo refuses files with set-id or sticky permission bits");
                }
                output.set_permissions(fs::Permissions::from_mode(bits))?;
            }
            #[cfg(not(unix))]
            {
                let _ = permission_bits;
                let mut permissions = output.metadata()?.permissions();
                permissions.set_readonly(*readonly);
                output.set_permissions(permissions)?;
            }
            output.sync_all()?;
            if read_image_from_file(root, relative, limit, after_mode, locked_file.as_mut())?
                != *after
            {
                return refuse("File changed while preparing undo; unrelated edits were preserved");
            }
            // Missing-file restoration uses hard_link's create-new semantics:
            // a newly appeared file cannot be replaced by this undo.
            if matches!(after, FileImage::Missing) {
                fs::hard_link(&temporary, &path)?;
            } else {
                fs::rename(&temporary, &path)?;
            }
            cleanup.file.take();
        }
        FileImage::Refused(_) => return refuse("Unknown baseline bytes cannot be restored"),
    }
    if let Some(file) = locked_file.take() {
        let _ = FileExt::unlock(&file);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use parking_lot::Mutex;
    use std::sync::Arc;

    struct Fixture {
        root: PathBuf,
    }
    impl Fixture {
        fn new() -> Self {
            let root =
                std::env::temp_dir().join(format!("shelldeck-review-test-{}", Uuid::new_v4()));
            fs::create_dir_all(root.join(".git/refs/heads")).unwrap();
            fs::write(root.join(".git/HEAD"), b"ref: refs/heads/main\n").unwrap();
            fs::write(root.join(".git/index"), b"original-index").unwrap();
            fs::write(root.join("file.rs"), b"user baseline\n").unwrap();
            Self { root }
        }
        fn backend(&self) -> (AgentReviewBackend<FakeExecutor>, Arc<Mutex<FakeState>>) {
            let state = Arc::new(Mutex::new(FakeState {
                root: self.root.clone(),
                head: Some("a".repeat(40)),
                reference: Some("refs/heads/main".into()),
                status: b" M file.rs\0".to_vec(),
                tracked: vec![("file.rs".into(), 0o100644)],
                untracked: vec![],
                cached_numstat: vec![],
                worktree_numstat: b"1\t1\tfile.rs\0".to_vec(),
                calls: vec![],
                fail_update_ref: false,
            }));
            (
                AgentReviewBackend::new(FakeExecutor(state.clone()), ReviewLimits::default()),
                state,
            )
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    struct FakeState {
        root: PathBuf,
        head: Option<String>,
        reference: Option<String>,
        status: Vec<u8>,
        tracked: Vec<(String, u32)>,
        untracked: Vec<String>,
        cached_numstat: Vec<u8>,
        worktree_numstat: Vec<u8>,
        calls: Vec<ReviewCommand>,
        fail_update_ref: bool,
    }
    struct FakeExecutor(Arc<Mutex<FakeState>>);
    impl ReviewExecutor for FakeExecutor {
        fn run(&self, command: &ReviewCommand) -> ReviewResult<ReviewCommandOutput> {
            let mut state = self.0.lock();
            state.calls.push(command.clone());
            let args = command.args[4..]
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>();
            let mut code = 0;
            let result = match args.as_slice() {
                ["rev-parse", "--show-toplevel"] => {
                    format!("{}\n", state.root.display()).into_bytes()
                }
                ["rev-parse", "--absolute-git-dir"]
                | ["rev-parse", "--path-format=absolute", "--git-common-dir"] => {
                    format!("{}\n", state.root.join(".git").display()).into_bytes()
                }
                ["symbolic-ref", "--quiet", "HEAD"] => match &state.reference {
                    Some(value) => format!("{value}\n").into_bytes(),
                    None => {
                        code = 1;
                        vec![]
                    }
                },
                ["rev-parse", "--verify", "HEAD"] => match &state.head {
                    Some(value) => format!("{value}\n").into_bytes(),
                    None => {
                        code = 128;
                        vec![]
                    }
                },
                ["show-ref", "--verify", "--quiet", _] => {
                    code = if state.head.is_some() { 0 } else { 1 };
                    vec![]
                }
                ["status", "--porcelain=v1", "-z", "--untracked-files=all"] => state.status.clone(),
                ["ls-files", "--stage", "-z"] => state
                    .tracked
                    .iter()
                    .flat_map(|(path, mode)| {
                        format!("{mode:o} {} 0\t{path}\0", "a".repeat(40)).into_bytes()
                    })
                    .collect(),
                ["ls-files", "--others", "--exclude-standard", "-z"] => state
                    .untracked
                    .iter()
                    .flat_map(|path| format!("{path}\0").into_bytes())
                    .collect(),
                args if args.first() == Some(&"diff") && args.contains(&"--numstat") => {
                    if args.contains(&"--cached") {
                        state.cached_numstat.clone()
                    } else {
                        state.worktree_numstat.clone()
                    }
                }
                args if args.first() == Some(&"diff") => {
                    b"--- a/file.rs\n+++ b/file.rs\n@@ -1 +1 @@\n-old\n+new\n".to_vec()
                }
                ["hash-object", "-w", "--stdin", "--no-filters"] => {
                    format!("{}\n", "b".repeat(40)).into_bytes()
                }
                args if args.first() == Some(&"update-index") || args.first() == Some(&"reset") => {
                    let alternate = command
                        .env
                        .get("GIT_INDEX_FILE")
                        .expect("mutations must use a private index");
                    assert_ne!(Path::new(alternate), state.root.join(".git/index"));
                    assert!(
                        state.root.join(".git/index.lock").is_file(),
                        "canonical index must remain locked"
                    );
                    fs::write(alternate, b"prepared-index").unwrap();
                    vec![]
                }
                ["write-tree"] => format!("{}\n", "d".repeat(40)).into_bytes(),
                args if args.first() == Some(&"commit-tree") => {
                    format!("{}\n", "c".repeat(40)).into_bytes()
                }
                ["update-ref", "-m", _, reference, new, old] => {
                    assert_eq!(Some(*reference), state.reference.as_deref());
                    assert_eq!(*old, state.head.as_deref().unwrap_or(""));
                    if state.fail_update_ref {
                        code = 128;
                    } else {
                        state.head = Some((*new).into());
                    }
                    vec![]
                }
                _ => panic!("Unexpected fake command: {:?}", command.args),
            };
            Ok(ReviewCommandOutput {
                exit_code: code,
                stdout: result,
                stderr: if code == 0 {
                    vec![]
                } else {
                    b"fake command refused".to_vec()
                },
            })
        }
    }

    // SDTEST-2260 — SDUC-552
    #[test]
    fn sdtest_2260_status_preserves_literal_names_and_rename_direction() {
        let entries = parse_status_z(
            b"R  new name\n.rs\0old\tname.rs\0?? --flag\0UU conflict.rs\0 D deleted.rs\0",
        )
        .unwrap();
        assert_eq!(entries[0].path, "new name\n.rs");
        assert_eq!(entries[0].original_path.as_deref(), Some("old\tname.rs"));
        assert!(entries[0].is_staged());
        assert!(entries[1].is_untracked());
        assert!(entries[2].is_conflicted());
        assert!(entries[3].is_unstaged());
        assert!(parse_status_z(b"R  renamed\0").is_err());
        assert!(parse_status_z(b"?? ../outside\0").is_err());
        assert!(parse_status_z(b"?? bad\xff\0").is_err());
    }

    // SDTEST-2261 — SDUC-552
    #[test]
    fn sdtest_2261_numstat_distinguishes_binary_unknown_zero_and_renames() {
        let counts =
            parse_numstat_z(b"0\t0\tempty\0-\t-\timage.png\x0012\t3\t\0old\0new\0").unwrap();
        assert_eq!(
            counts["empty"],
            LineCounts {
                additions: Some(0),
                deletions: Some(0)
            }
        );
        assert_eq!(counts["image.png"], LineCounts::default());
        assert_eq!(
            counts["new"],
            LineCounts {
                additions: Some(12),
                deletions: Some(3)
            }
        );
        assert!(parse_numstat_z(b"-\t1\tfile\0").is_err());
        assert!(parse_numstat_z(b"1\t2\t\0old\0").is_err());
    }

    // SDTEST-2262 — SDUC-552
    #[test]
    fn sdtest_2262_observation_covers_actual_sections_and_binary_counts() {
        let fixture = Fixture::new();
        let (backend, state) = fixture.backend();
        fs::write(fixture.root.join("new file"), b"one\ntwo").unwrap();
        fs::write(fixture.root.join("image.bin"), [0u8, 1, 2]).unwrap();
        {
            let mut state = state.lock();
            state.status = b"MM file.rs\0 D gone.rs\0?? new file\0?? image.bin\0".to_vec();
            state.tracked.push(("gone.rs".into(), 0o100644));
            state.untracked = vec!["new file".into(), "image.bin".into()];
            state.cached_numstat = b"2\t1\tfile.rs\0".to_vec();
            state.worktree_numstat = b"3\t2\tfile.rs\x000\t5\tgone.rs\0".to_vec();
        }
        let snapshot = backend.observe(&fixture.root).unwrap();
        assert_eq!(snapshot.branch.as_deref(), Some("main"));
        assert_eq!(snapshot.staged_count(), 1);
        assert_eq!(snapshot.diffs.len(), 5);
        assert!(snapshot
            .diffs
            .iter()
            .any(|diff| diff.path == "gone.rs" && diff.counts.deletions == Some(5)));
        let new = snapshot
            .diffs
            .iter()
            .find(|diff| diff.path == "new file")
            .unwrap();
        assert_eq!(new.counts.additions, Some(2));
        assert!(new.unified.contains("\\ No newline at end of file"));
        assert_eq!(
            snapshot
                .diffs
                .iter()
                .find(|diff| diff.path == "image.bin")
                .unwrap()
                .counts
                .additions,
            None
        );
        assert_eq!(
            fs::read(fixture.root.join(".git/index")).unwrap(),
            b"original-index"
        );
        assert!(state
            .lock()
            .calls
            .iter()
            .all(|call| !call.args.iter().any(|arg| matches!(
                arg.as_str(),
                "add" | "update-index" | "reset" | "hash-object"
            ))));
    }

    // SDTEST-2263 — SDUC-552
    #[test]
    fn sdtest_2263_confirmed_stage_uses_reviewed_bytes_private_index_and_locks() {
        let fixture = Fixture::new();
        let (backend, state) = fixture.backend();
        let snapshot = backend.observe(&fixture.root).unwrap();
        let plan = backend
            .plan(
                &snapshot,
                ReviewAction::StageFiles {
                    paths: vec!["file.rs".into()],
                },
            )
            .unwrap();
        let result = backend.execute(&plan, plan.confirm()).unwrap();
        assert_eq!(result.paths, ["file.rs"]);
        assert_eq!(
            fs::read(fixture.root.join(".git/index")).unwrap(),
            b"prepared-index"
        );
        let calls = &state.lock().calls;
        let hash = calls
            .iter()
            .find(|call| call.args.iter().any(|arg| arg == "hash-object"))
            .unwrap();
        assert_eq!(hash.stdin, b"user baseline\n");
        let update = calls
            .iter()
            .find(|call| call.args.iter().any(|arg| arg == "update-index"))
            .unwrap();
        assert!(update.args.iter().any(|arg| arg == &"b".repeat(40)));
        assert!(!fixture.root.join(".git/index.lock").exists());
        assert!(!fixture.root.join(".git/HEAD.lock").exists());
        assert!(!fixture.root.join(".git/refs/heads/main.lock").exists());
    }

    // SDTEST-2264 — SDUC-552
    #[test]
    fn sdtest_2264_stale_content_index_head_and_wrong_confirmation_refuse_before_writes() {
        for stale in ["file", "index", "head", "confirmation"] {
            let fixture = Fixture::new();
            let (backend, state) = fixture.backend();
            let snapshot = backend.observe(&fixture.root).unwrap();
            let plan = backend
                .plan(
                    &snapshot,
                    ReviewAction::StageFiles {
                        paths: vec!["file.rs".into()],
                    },
                )
                .unwrap();
            match stale {
                "file" => fs::write(fixture.root.join("file.rs"), b"new user work").unwrap(),
                "index" => fs::write(fixture.root.join(".git/index"), b"other staging").unwrap(),
                "head" => state.lock().head = Some("e".repeat(40)),
                _ => (),
            }
            let confirmation = if stale == "confirmation" {
                ReviewConfirmation {
                    plan_id: Uuid::new_v4(),
                }
            } else {
                plan.confirm()
            };
            assert!(
                backend.execute(&plan, confirmation).is_err(),
                "must reject stale {stale}"
            );
            assert!(!state
                .lock()
                .calls
                .iter()
                .any(|call| call.args.iter().any(|arg| arg == "hash-object")));
            assert!(!fixture.root.join(".git/index.lock").exists());
        }
    }

    // SDTEST-2265 — SDUC-552
    #[test]
    fn sdtest_2265_dirty_baseline_undo_requires_exact_evidence_and_preserves_baseline() {
        let fixture = Fixture::new();
        let (backend, _) = fixture.backend();
        let before = backend.observe(&fixture.root).unwrap();
        let baseline = before.files["file.rs"].clone();
        fs::write(fixture.root.join("file.rs"), b"user baseline\nagent edit\n").unwrap();
        let agent_after = read_image(&fixture.root, "file.rs", 1024, None).unwrap();
        let turn = backend
            .finish_turn_with_evidence(
                before,
                &[OwnedFileEdit {
                    path: "file.rs".into(),
                    before: baseline,
                    after: agent_after,
                }],
            )
            .unwrap();
        assert!(turn.changes[0].baseline_dirty);
        assert_eq!(
            turn.changes[0].attribution,
            ChangeAttribution::ProvenAgentEdit
        );
        let plan = backend.plan_undo(&turn.after, &turn, "file.rs").unwrap();
        backend.execute(&plan, plan.confirm()).unwrap();
        assert_eq!(
            fs::read(fixture.root.join("file.rs")).unwrap(),
            b"user baseline\n"
        );
        assert_eq!(
            fs::read(fixture.root.join(".git/index")).unwrap(),
            b"original-index"
        );
    }

    // SDTEST-2266 — SDUC-552
    #[test]
    fn sdtest_2266_unproven_or_later_user_edits_are_never_discarded() {
        let fixture = Fixture::new();
        let (backend, _) = fixture.backend();
        let before = backend.observe(&fixture.root).unwrap();
        fs::write(fixture.root.join("file.rs"), b"some edit\n").unwrap();
        let unproven = backend.finish_turn(before.clone()).unwrap();
        assert!(backend
            .plan_undo(&unproven.after, &unproven, "file.rs")
            .is_err());
        let turn = backend
            .finish_turn_with_evidence(
                before.clone(),
                &[OwnedFileEdit {
                    path: "file.rs".into(),
                    before: before.files["file.rs"].clone(),
                    after: unproven.after.files["file.rs"].clone(),
                }],
            )
            .unwrap();
        let plan = backend.plan_undo(&turn.after, &turn, "file.rs").unwrap();
        fs::write(fixture.root.join("file.rs"), b"user added later\n").unwrap();
        assert!(backend.execute(&plan, plan.confirm()).is_err());
        assert_eq!(
            fs::read(fixture.root.join("file.rs")).unwrap(),
            b"user added later\n"
        );
    }

    // SDTEST-2267 — SDUC-552
    #[test]
    fn sdtest_2267_commit_uses_reviewed_tree_and_atomic_expected_ref_without_hooks() {
        let fixture = Fixture::new();
        let (backend, state) = fixture.backend();
        state.lock().status = b"M  file.rs\0".to_vec();
        state.lock().cached_numstat = b"1\t1\tfile.rs\0".to_vec();
        let snapshot = backend.observe(&fixture.root).unwrap();
        let plan = backend
            .plan(
                &snapshot,
                ReviewAction::Commit {
                    message: "Preserve literal $(text)\n\nBody".into(),
                },
            )
            .unwrap();
        let outcome = backend.execute(&plan, plan.confirm()).unwrap();
        assert_eq!(outcome.commit, Some("c".repeat(40)));
        let state = state.lock();
        let commit = state
            .calls
            .iter()
            .find(|call| call.args.iter().any(|arg| arg == "commit-tree"))
            .unwrap();
        assert_eq!(commit.stdin, b"Preserve literal $(text)\n\nBody\n");
        assert!(commit.args.iter().any(|arg| arg == &"a".repeat(40)));
        assert!(!state
            .calls
            .iter()
            .any(|call| call.args.iter().any(|arg| arg == "commit")));
        assert_eq!(state.head, Some("c".repeat(40)));
        assert_eq!(
            fs::read(fixture.root.join(".git/index")).unwrap(),
            b"original-index"
        );
    }

    // SDTEST-2268 — SDUC-552
    #[test]
    fn sdtest_2268_existing_git_lock_is_never_stolen_or_removed() {
        let fixture = Fixture::new();
        let (backend, state) = fixture.backend();
        let snapshot = backend.observe(&fixture.root).unwrap();
        let plan = backend
            .plan(
                &snapshot,
                ReviewAction::StageFiles {
                    paths: vec!["file.rs".into()],
                },
            )
            .unwrap();
        fs::write(fixture.root.join(".git/index.lock"), b"other Git process").unwrap();
        assert!(backend.execute(&plan, plan.confirm()).is_err());
        assert_eq!(
            fs::read(fixture.root.join(".git/index.lock")).unwrap(),
            b"other Git process"
        );
        assert!(!state
            .lock()
            .calls
            .iter()
            .any(|call| call.args.iter().any(|arg| arg == "hash-object")));
        assert!(!fixture.root.join(".git/HEAD.lock").exists());
    }

    // SDTEST-2269 — SDUC-552
    #[test]
    fn sdtest_2269_snapshot_budget_and_conflicts_disable_unsafe_mutations() {
        let fixture = Fixture::new();
        let (_, state) = fixture.backend();
        let limits = ReviewLimits {
            max_file_bytes: 2,
            ..ReviewLimits::default()
        };
        let backend = AgentReviewBackend::new(FakeExecutor(state.clone()), limits);
        let snapshot = backend.observe(&fixture.root).unwrap();
        assert!(matches!(snapshot.files["file.rs"], FileImage::Refused(_)));
        assert!(backend
            .plan(
                &snapshot,
                ReviewAction::StageFiles {
                    paths: vec!["file.rs".into()]
                }
            )
            .is_err());
        state.lock().status = b"UU file.rs\0".to_vec();
        let snapshot = backend.observe(&fixture.root).unwrap();
        assert!(!snapshot.can_mutate());
        assert!(snapshot.diffs.iter().all(|diff| diff.conflicted));
    }

    // SDTEST-2494 — SDUC-552: native app assets must not disable every Git action.
    #[test]
    fn sdtest_2494_large_tracked_assets_remain_complete_and_detect_intervening_edits() {
        let fixture = Fixture::new();
        let mut asset = vec![0xab; 6 * 1024 * 1024];
        fs::write(fixture.root.join("asset.bin"), &asset).unwrap();
        let (backend, state) = fixture.backend();
        {
            let mut state = state.lock();
            state.tracked.push(("asset.bin".into(), 0o100644));
            state.status = b" M file.rs\0 M asset.bin\0".to_vec();
            state.worktree_numstat = b"1\t1\tfile.rs\0-\t-\tasset.bin\0".to_vec();
        }
        let snapshot = backend.observe(&fixture.root).unwrap();
        assert!(snapshot.can_mutate(), "{:?}", snapshot.refusals);
        let plan = backend
            .plan(
                &snapshot,
                ReviewAction::StageFiles {
                    paths: vec!["asset.bin".into()],
                },
            )
            .unwrap();
        asset[0] = 0xcd;
        fs::write(fixture.root.join("asset.bin"), &asset).unwrap();
        assert!(backend.execute(&plan, plan.confirm()).is_err());
        assert_eq!(
            fs::read(fixture.root.join(".git/index")).unwrap(),
            b"original-index"
        );
        assert!(!state
            .lock()
            .calls
            .iter()
            .any(|call| call.args.iter().any(|arg| arg == "update-index")));
    }

    // SDTEST-2270 — SDUC-552; Unix no-follow/execute bits, Windows compile coverage.
    #[cfg(unix)]
    #[test]
    fn sdtest_2270_symlink_ancestor_never_reads_or_mutates_outside_checkout() {
        use std::os::unix::fs::symlink;
        let fixture = Fixture::new();
        let (backend, state) = fixture.backend();
        fs::create_dir(fixture.root.join("outside")).unwrap();
        fs::write(fixture.root.join("outside/secret"), b"do not touch").unwrap();
        symlink(fixture.root.join("outside"), fixture.root.join("link")).unwrap();
        state.lock().tracked = vec![("link/secret".into(), 0o100644)];
        state.lock().status = b" M link/secret\0".to_vec();
        let snapshot = backend.observe(&fixture.root).unwrap();
        assert!(matches!(
            snapshot.files["link/secret"],
            FileImage::Refused(_)
        ));
        assert!(backend
            .plan(
                &snapshot,
                ReviewAction::StageFiles {
                    paths: vec!["link/secret".into()]
                }
            )
            .is_err());
        assert_eq!(
            fs::read(fixture.root.join("outside/secret")).unwrap(),
            b"do not touch"
        );
    }

    // SDTEST-2271 — SDUC-552
    #[test]
    fn sdtest_2271_unstage_prepares_only_exact_named_paths_on_private_index() {
        let fixture = Fixture::new();
        let (backend, state) = fixture.backend();
        state.lock().status = b"M  file.rs\0".to_vec();
        let snapshot = backend.observe(&fixture.root).unwrap();
        let plan = backend
            .plan(
                &snapshot,
                ReviewAction::UnstageFiles {
                    paths: vec!["file.rs".into()],
                },
            )
            .unwrap();
        backend.execute(&plan, plan.confirm()).unwrap();
        let state = state.lock();
        let reset = state
            .calls
            .iter()
            .find(|call| call.args.iter().any(|arg| arg == "reset"))
            .unwrap();
        assert_eq!(
            &reset.args[4..],
            ["reset", "--quiet", &"a".repeat(40), "--", "file.rs"]
        );
        assert_eq!(
            fs::read(fixture.root.join("file.rs")).unwrap(),
            b"user baseline\n"
        );
        assert_eq!(
            fs::read(fixture.root.join(".git/index")).unwrap(),
            b"prepared-index"
        );
    }

    // SDTEST-2272 — SDUC-552
    #[test]
    fn sdtest_2272_commit_compare_and_swap_failure_leaves_reviewed_index_untouched() {
        let fixture = Fixture::new();
        let (backend, state) = fixture.backend();
        state.lock().status = b"M  file.rs\0".to_vec();
        state.lock().fail_update_ref = true;
        let snapshot = backend.observe(&fixture.root).unwrap();
        let plan = backend
            .plan(
                &snapshot,
                ReviewAction::Commit {
                    message: "Cannot move someone else's branch".into(),
                },
            )
            .unwrap();
        assert!(backend.execute(&plan, plan.confirm()).is_err());
        assert_eq!(state.lock().head, Some("a".repeat(40)));
        assert_eq!(
            fs::read(fixture.root.join(".git/index")).unwrap(),
            b"original-index"
        );
        assert!(!fixture.root.join(".git/index.lock").exists());
    }
}

#[cfg(test)]
mod chat_snapshot_tests {
    use super::requires_turn_snapshot;
    use crate::agent_runtime::{AgentAccessMode, AgentTarget};

    // SDTEST-2792 — read-only chat bypasses Git; edit attribution stays exact.
    #[test]
    fn chat_snapshot_policy_preserves_local_mutation_boundaries() {
        for access in [
            AgentAccessMode::ReadOnly,
            AgentAccessMode::WorkspaceWrite,
            AgentAccessMode::FullAccess,
        ] {
            assert_eq!(
                requires_turn_snapshot(&AgentTarget::Local, access),
                access != AgentAccessMode::ReadOnly
            );
            assert!(!requires_turn_snapshot(
                &AgentTarget::Ssh {
                    connection_id: uuid::Uuid::new_v4(),
                    connection_label: "test".into()
                },
                access
            ));
        }
    }
}
