//! Branch-aware local delivery. Remote writes require an explicit, immutable
//! preview and are fenced against changes to the repository and destination.
//! No stage, checkout, commit, force-push or implicit gh push is performed here.

use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::{Duration, Instant};

use serde::Deserialize;
use uuid::Uuid;

use crate::agent_review::{
    AgentReviewBackend, DeliveryAction, FileImage, RepositorySnapshot, ReviewCommand,
    ReviewCommandOutput, ReviewError, ReviewExecutor, ReviewLimits, ReviewResult,
    SystemReviewExecutor,
};

const MAX_BODY_BYTES: usize = 128 * 1024;
const PLAN_LIFETIME: Duration = Duration::from_secs(300);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitHubRepository {
    pub host: String,
    pub owner: String,
    pub name: String,
}

impl GitHubRepository {
    pub fn cli_name(&self) -> String {
        format!("{}/{}/{}", self.host, self.owner, self.name)
    }

    /// ShellDeck's contributor policy applies only to this repository. Other
    /// projects use the authenticated actor shown in their own delivery preview.
    pub fn permits_actor(&self, actor: &str) -> bool {
        if self.host.eq_ignore_ascii_case("github.com")
            && self.owner.eq_ignore_ascii_case("benfavre")
            && self.name.eq_ignore_ascii_case("shelldeck")
        {
            matches!(actor, "pedrokarim" | "benfavre")
        } else {
            true
        }
    }
}

/// Recognize canonical GitHub remotes without guessing repository ownership
/// from unrelated URLs, custom remote helpers or URLs containing credentials.
pub fn github_repository(remote: &str) -> Option<GitHubRepository> {
    let (host, path) = if let Some((host, path)) = remote
        .strip_prefix("git@")
        .and_then(|value| value.split_once(':'))
        .filter(|(host, _)| host.eq_ignore_ascii_case("github.com"))
    {
        (host.to_ascii_lowercase(), path.to_owned())
    } else {
        let url = url::Url::parse(remote).ok()?;
        if !matches!(url.scheme(), "https" | "ssh")
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
            || url.port().is_some()
            || (url.scheme() == "https" && !url.username().is_empty())
            || (url.scheme() == "ssh" && url.username() != "git")
            || url.host_str()? != "github.com"
        {
            return None;
        }
        (
            url.host_str()?.to_owned(),
            url.path().trim_start_matches('/').to_owned(),
        )
    };
    let mut components = path.split('/');
    let owner = components.next()?;
    let name = components.next()?;
    let name = name.strip_suffix(".git").unwrap_or(name);
    if components.next().is_some() || !safe_component(owner) || !safe_component(name) {
        return None;
    }
    Some(GitHubRepository {
        host,
        owner: owner.into(),
        name: name.into(),
    })
}

fn safe_component(value: &str) -> bool {
    !value.is_empty()
        && !value.starts_with('.')
        && value.len() <= 100
        && value
            .bytes()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, b'-' | b'_' | b'.'))
}

#[derive(Clone, Debug)]
pub struct DeliveryStatus {
    /// Preserve the session's path as well as its canonical destination. A
    /// symlink retarget must not silently publish from the previous worktree.
    pub directory: PathBuf,
    pub canonical_directory: PathBuf,
    pub snapshot: Arc<RepositorySnapshot>,
    pub remote: Option<String>,
    pub push_remote: Option<String>,
    remote_configuration: Vec<u8>,
    pub github: Option<GitHubRepository>,
    pub upstream: Option<String>,
    pub ahead: Option<u64>,
    pub behind: Option<u64>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct PullRequestAuthor {
    pub login: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DeliveryPullRequest {
    pub number: u64,
    pub url: String,
    pub title: String,
    pub author: PullRequestAuthor,
    pub head_ref_name: String,
    pub base_ref_name: String,
    pub head_ref_oid: String,
    pub is_draft: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeliveryCheck {
    pub name: String,
    pub state: String,
    pub url: Option<String>,
}

#[derive(Clone, Debug)]
pub struct GitHubDeliveryStatus {
    pub actor: String,
    pub actor_permitted: bool,
    pub pull_request: Option<DeliveryPullRequest>,
    pub checks: Vec<DeliveryCheck>,
    pub mergeable: Option<String>,
    pub merge_state: Option<String>,
    pub head_matches: bool,
}

#[derive(Clone, Debug)]
pub enum DeliveryIntent {
    Push,
    CreatePullRequest {
        title: String,
        body: String,
        base: String,
    },
}

/// Fields are private so a renderer cannot silently change a confirmed effect.
#[derive(Clone)]
pub struct DeliveryPlan {
    id: Uuid,
    status: DeliveryStatus,
    intent: DeliveryIntent,
    actor: Option<String>,
    body: Option<Arc<PreparedBody>>,
    index: Option<Vec<u8>>,
    created: Instant,
    consumed: Arc<AtomicBool>,
    preview: Vec<String>,
}

impl DeliveryPlan {
    pub fn preview(&self) -> &[String] {
        &self.preview
    }
    pub fn intent(&self) -> &DeliveryIntent {
        &self.intent
    }
    pub fn status(&self) -> &DeliveryStatus {
        &self.status
    }
    pub fn actor(&self) -> Option<&str> {
        self.actor.as_deref()
    }
    /// Call only from the concrete user confirmation callback.
    pub fn confirm(&self) -> DeliveryConfirmation {
        DeliveryConfirmation { id: self.id }
    }
}

pub struct DeliveryConfirmation {
    id: Uuid,
}

#[derive(Clone, Debug)]
pub struct DeliveryOutcome {
    pub output: String,
    pub pull_request: Option<DeliveryPullRequest>,
}

struct PreparedBody {
    directory: PathBuf,
    path: PathBuf,
    bytes: Vec<u8>,
}
impl PreparedBody {
    fn new(body: &str) -> ReviewResult<Self> {
        if body.len() > MAX_BODY_BYTES || body.contains('\0') {
            return refused("Pull request body exceeds its limit or contains NUL");
        }
        let directory = std::env::temp_dir().join(format!("shelldeck-pr-{}", Uuid::new_v4()));
        fs::create_dir(&directory)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))?;
        }
        let path = directory.join("body.md");
        let result = (|| {
            let mut options = OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut file = options.open(&path)?;
            file.write_all(body.as_bytes())?;
            file.sync_all()?;
            Ok(Self {
                directory: directory.clone(),
                path,
                bytes: body.as_bytes().to_vec(),
            })
        })();
        if result.is_err() {
            let _ = fs::remove_dir_all(directory);
        }
        result
    }
    fn validate(&self) -> ReviewResult<()> {
        if fs::symlink_metadata(&self.path)?.file_type().is_symlink()
            || read_optional(&self.path, MAX_BODY_BYTES)? != Some(self.bytes.clone())
        {
            return refused("Prepared pull request body changed; review and confirm again");
        }
        Ok(())
    }
}
impl Drop for PreparedBody {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
        let _ = fs::remove_dir(&self.directory);
    }
}

struct BorrowedExecutor<'a, E>(&'a E);
impl<E: ReviewExecutor> ReviewExecutor for BorrowedExecutor<'_, E> {
    fn run(&self, command: &ReviewCommand) -> ReviewResult<ReviewCommandOutput> {
        self.0.run(command)
    }
}

pub struct AgentDeliveryBackend<E = SystemReviewExecutor> {
    executor: E,
    limits: ReviewLimits,
}
impl Default for AgentDeliveryBackend<SystemReviewExecutor> {
    fn default() -> Self {
        Self::new(SystemReviewExecutor, ReviewLimits::default())
    }
}
impl<E: ReviewExecutor> AgentDeliveryBackend<E> {
    pub fn new(executor: E, limits: ReviewLimits) -> Self {
        Self { executor, limits }
    }
    fn review(&self) -> AgentReviewBackend<BorrowedExecutor<'_, E>> {
        AgentReviewBackend::new(BorrowedExecutor(&self.executor), self.limits.clone())
    }
    fn command(&self, root: &Path, program: &str, args: &[&str]) -> ReviewCommand {
        ReviewCommand {
            program: program.into(),
            args: if program == "git" {
                ["--no-optional-locks", "-c", "core.fsmonitor=false"]
                    .into_iter()
                    .chain(args.iter().copied())
                    .map(str::to_owned)
                    .collect()
            } else {
                args.iter().map(|s| (*s).into()).collect()
            },
            cwd: root.to_owned(),
            stdin: Vec::new(),
            env: BTreeMap::from([
                ("GH_PROMPT_DISABLED".into(), "1".into()),
                ("GH_PAGER".into(), "cat".into()),
            ]),
            max_output_bytes: self.limits.max_command_bytes.min(1024 * 1024),
            timeout: self.limits.command_timeout,
        }
    }
    fn output(&self, command: &ReviewCommand) -> ReviewResult<ReviewCommandOutput> {
        let output = self.executor.run(command)?;
        if output.stdout.len() > command.max_output_bytes || output.stderr.len() > 64 * 1024 {
            return refused("Delivery command output exceeded its limit");
        }
        Ok(output)
    }
    fn run(&self, command: &ReviewCommand) -> ReviewResult<Vec<u8>> {
        let result = self.output(command)?;
        if result.exit_code != 0 {
            return Err(ReviewError::Command {
                program: command.program.clone(),
                detail: String::from_utf8_lossy(&result.stderr)
                    .chars()
                    .take(2048)
                    .collect(),
            });
        }
        Ok(result.stdout)
    }
    fn git(&self, root: &Path, args: &[&str]) -> ReviewResult<Vec<u8>> {
        self.run(&self.command(root, "git", args))
    }
    fn gh(&self, root: &Path, args: &[&str]) -> ReviewResult<Vec<u8>> {
        self.run(&self.command(root, "gh", args))
    }
    fn optional_git(&self, root: &Path, args: &[&str]) -> ReviewResult<Option<String>> {
        let result = self.output(&self.command(root, "git", args))?;
        match result.exit_code {
            0 => Ok(Some(single_line(&result.stdout)?)),
            1 | 128 => Ok(None),
            2 if args.first() == Some(&"remote") => Ok(None),
            _ => refused("Cannot read Git delivery configuration"),
        }
    }
    /// Local-only read. GitHub checks are a separate, explicit network action.
    pub fn status(&self, directory: &Path) -> ReviewResult<DeliveryStatus> {
        let requested = if directory.is_absolute() {
            directory.to_owned()
        } else {
            std::env::current_dir()?.join(directory)
        };
        let canonical = fs::canonicalize(&requested)?;
        let snapshot = self.review().observe(&requested)?;
        if fs::canonicalize(&requested)? != canonical {
            return refused("Working directory changed while being inspected");
        }
        self.status_from_snapshot(snapshot, requested, canonical)
    }
    fn status_from_snapshot(
        &self,
        snapshot: RepositorySnapshot,
        directory: PathBuf,
        canonical_directory: PathBuf,
    ) -> ReviewResult<DeliveryStatus> {
        let root = &snapshot.root;
        let remote_configuration = self.remote_configuration(root)?;
        // get-url applies configured insteadOf and pushurl rules. Reading all
        // destinations refuses an ambiguous multi-remote/fan-out effect.
        let remote = self.optional_git(root, &["remote", "get-url", "--all", "origin"])?;
        let push_remote = if remote.is_some() {
            self.optional_git(root, &["remote", "get-url", "--push", "--all", "origin"])?
        } else {
            None
        };
        for remote in remote.iter().chain(push_remote.iter()) {
            validate_remote(remote)?;
        }
        if self.remote_configuration(root)? != remote_configuration {
            return refused("Remote configuration changed while being inspected");
        }
        let github = remote.as_deref().and_then(github_repository);
        let upstream = if snapshot.branch.is_some() {
            self.optional_git(
                root,
                &[
                    "rev-parse",
                    "--abbrev-ref",
                    "--symbolic-full-name",
                    "@{upstream}",
                ],
            )?
        } else {
            None
        };
        let (ahead, behind) = if upstream.is_some() {
            let counts = self.git(
                root,
                &["rev-list", "--left-right", "--count", "HEAD...@{upstream}"],
            )?;
            parse_counts(&counts)?
        } else {
            (None, None)
        };
        Ok(DeliveryStatus {
            directory,
            canonical_directory,
            snapshot: Arc::new(snapshot),
            remote,
            push_remote,
            remote_configuration,
            github,
            upstream,
            ahead,
            behind,
        })
    }
    fn remote_configuration(&self, root: &Path) -> ReviewResult<Vec<u8>> {
        let output = self.output(&self.command(
            root,
            "git",
            &[
                "config",
                "--null",
                "--get-regexp",
                r"^(remote\.origin\.|url\.)",
            ],
        ))?;
        match output.exit_code {
            0 => Ok(output.stdout),
            1 => Ok(Vec::new()),
            _ => refused("Cannot inspect effective remote configuration"),
        }
    }
    fn actor(&self, status: &DeliveryStatus, enforce: bool) -> ReviewResult<String> {
        let repo = status.github.as_ref().ok_or_else(|| {
            ReviewError::Refused("GitHub delivery requires a canonical github.com origin".into())
        })?;
        self.gh(
            &status.snapshot.root,
            &["auth", "status", "--hostname", &repo.host],
        )?;
        let actor = single_line(&self.gh(
            &status.snapshot.root,
            &["api", "--hostname", &repo.host, "user", "--jq", ".login"],
        )?)?;
        if !safe_component(&actor) {
            return refused("GitHub returned an invalid authenticated actor");
        }
        if enforce && !repo.permits_actor(&actor) {
            return refused(format!(
                "Authenticated GitHub actor {actor} is not permitted for this repository; switch accounts before confirming delivery"
            ));
        }
        Ok(actor)
    }
    fn open_prs(&self, status: &DeliveryStatus) -> ReviewResult<Vec<DeliveryPullRequest>> {
        let repo = status
            .github
            .as_ref()
            .ok_or_else(|| ReviewError::Refused("Not a supported GitHub origin".into()))?
            .cli_name();
        let branch = named_branch(&status.snapshot)?;
        let bytes = self.gh(
            &status.snapshot.root,
            &[
                "pr",
                "list",
                "--repo",
                &repo,
                "--head",
                branch,
                "--state",
                "open",
                "--limit",
                "20",
                "--json",
                "number,url,title,author,headRefName,baseRefName,headRefOid,isDraft",
            ],
        )?;
        let prs: Vec<DeliveryPullRequest> = parse_json(&bytes)?;
        for pr in &prs {
            self.validate_pr(status, pr)?;
        }
        Ok(prs)
    }
    /// Explicit user-triggered network read; it does not poll or mutate GitHub.
    pub fn github_status(&self, status: &DeliveryStatus) -> ReviewResult<GitHubDeliveryStatus> {
        self.revalidate(status, None)?;
        let actor = self.actor(status, false)?;
        let mut prs = self.open_prs(status)?;
        if prs.len() > 1 {
            return refused(
                "Multiple open pull requests match this branch; choose the PR in GitHub",
            );
        }
        let mut result = GitHubDeliveryStatus {
            actor_permitted: status
                .github
                .as_ref()
                .is_some_and(|repo| repo.permits_actor(&actor)),
            actor,
            pull_request: prs.pop(),
            checks: Vec::new(),
            mergeable: None,
            merge_state: None,
            head_matches: false,
        };
        if let Some(pr) = &result.pull_request {
            let repo = status
                .github
                .as_ref()
                .expect("validated repository")
                .cli_name();
            let bytes = self.gh(
                &status.snapshot.root,
                &[
                    "pr",
                    "view",
                    &pr.number.to_string(),
                    "--repo",
                    &repo,
                    "--json",
                    "statusCheckRollup,mergeable,mergeStateStatus,headRefOid",
                ],
            )?;
            let value: serde_json::Value = parse_json(&bytes)?;
            result.head_matches =
                value.get("headRefOid").and_then(|v| v.as_str()) == status.snapshot.head.as_deref();
            result.mergeable = value
                .get("mergeable")
                .and_then(|v| v.as_str())
                .map(str::to_owned);
            result.merge_state = value
                .get("mergeStateStatus")
                .and_then(|v| v.as_str())
                .map(str::to_owned);
            result.checks = parse_checks(&value)?;
        }
        Ok(result)
    }
    pub fn prepare_push(&self, status: &DeliveryStatus) -> ReviewResult<DeliveryPlan> {
        self.prepare(status, DeliveryIntent::Push)
    }
    pub fn prepare_pull_request(
        &self,
        status: &DeliveryStatus,
        title: String,
        body: String,
        base: String,
    ) -> ReviewResult<DeliveryPlan> {
        self.prepare(
            status,
            DeliveryIntent::CreatePullRequest { title, body, base },
        )
    }
    fn prepare(
        &self,
        status: &DeliveryStatus,
        intent: DeliveryIntent,
    ) -> ReviewResult<DeliveryPlan> {
        validate_snapshot(&status.snapshot)?;
        let branch = named_branch(&status.snapshot)?;
        self.git(
            &status.snapshot.root,
            &["check-ref-format", "--branch", branch],
        )?;
        let remote = status
            .remote
            .as_deref()
            .ok_or_else(|| ReviewError::Refused("No origin remote is configured".into()))?;
        validate_remote(remote)?;
        if status.push_remote != status.remote {
            return refused(
                "Origin has a different push destination. Use a worktree with one explicit fetch/push repository before publishing from this surface",
            );
        }
        let actor = if status.github.is_some() {
            Some(self.actor(status, true)?)
        } else {
            None
        };
        let body = match &intent {
            DeliveryIntent::Push => None,
            DeliveryIntent::CreatePullRequest { title, body, base } => {
                if title.trim().is_empty()
                    || title.len() > 1024
                    || title.chars().any(char::is_control)
                    || base.is_empty()
                    || base.len() > 255
                    || base.starts_with('-')
                    || base.chars().any(char::is_control)
                    || base == branch
                {
                    return refused("Supply a title and a different, valid base branch");
                }
                self.git(
                    &status.snapshot.root,
                    &["check-ref-format", "--branch", base],
                )?;
                if !self.open_prs(status)?.is_empty() {
                    return refused(
                        "This branch already has an open pull request; refresh GitHub status",
                    );
                }
                self.validate_published_head(status)?;
                Some(Arc::new(PreparedBody::new(body)?))
            }
        };
        self.revalidate(status, None)?;
        let index = read_optional(
            &status.snapshot.git_dir.join("index"),
            self.limits.max_command_bytes,
        )?;
        let mut plan = DeliveryPlan {
            id: Uuid::new_v4(),
            status: status.clone(),
            intent,
            actor,
            body,
            index,
            created: Instant::now(),
            consumed: Arc::new(AtomicBool::new(false)),
            preview: Vec::new(),
        };
        let commands = self.effect_commands(&plan)?;
        plan.preview = commands.iter().map(display_command).collect();
        Ok(plan)
    }
    fn effect_commands(&self, plan: &DeliveryPlan) -> ReviewResult<Vec<ReviewCommand>> {
        match &plan.intent {
            DeliveryIntent::Push => {
                let mut commands = self
                    .review()
                    .delivery_commands(&plan.status.snapshot, DeliveryAction::Push)?;
                let command = commands.last_mut().expect("push command");
                let branch = named_branch(&plan.status.snapshot)?;
                let head =
                    plan.status.snapshot.head.as_deref().ok_or_else(|| {
                        ReviewError::Refused("Cannot push an unborn branch".into())
                    })?;
                // Capture both the immutable commit and destination. A concurrent
                // ref/config update cannot redirect the reviewed network effect.
                let len = command.args.len();
                command.args[len - 2] = plan.status.remote.clone().expect("validated remote");
                command.args[len - 1] = format!("{head}:refs/heads/{branch}");
                Ok(commands)
            }
            DeliveryIntent::CreatePullRequest { title, base, .. } => {
                let body_file = plan.body.as_ref().expect("prepared body").path.clone();
                let mut commands = self.review().delivery_commands(
                    &plan.status.snapshot,
                    DeliveryAction::CreatePullRequest {
                        title: title.clone(),
                        body_file,
                        base: base.clone(),
                    },
                )?;
                // Authentication is performed explicitly immediately before the
                // mutation, rather than silently executing the preview's reads.
                let mut effect = commands.pop().expect("pull request command");
                effect.args.extend([
                    "--repo".into(),
                    plan.status.github.as_ref().expect("GitHub PR").cli_name(),
                ]);
                effect.env.insert("GH_PROMPT_DISABLED".into(), "1".into());
                Ok(vec![effect])
            }
        }
    }
    fn revalidate(
        &self,
        status: &DeliveryStatus,
        index: Option<&Option<Vec<u8>>>,
    ) -> ReviewResult<()> {
        if fs::canonicalize(&status.directory)? != status.canonical_directory {
            return refused("Working directory changed; refresh and confirm delivery again");
        }
        let current = self.status(&status.directory)?;
        let expected = &status.snapshot;
        let actual = &current.snapshot;
        if !actual.same_repository_state(expected)
            || !actual.complete
            || current.remote != status.remote
            || current.push_remote != status.push_remote
            || current.remote_configuration != status.remote_configuration
        {
            return refused(
                "Repository, branch, HEAD, origin or working files changed; refresh and confirm delivery again",
            );
        }
        if let Some(index) = index {
            if &read_optional(
                &expected.git_dir.join("index"),
                self.limits.max_command_bytes,
            )? != index
            {
                return refused("Git index changed; refresh and confirm delivery again");
            }
        }
        Ok(())
    }
    fn validate_published_head(&self, status: &DeliveryStatus) -> ReviewResult<()> {
        let branch = named_branch(&status.snapshot)?;
        let reference = format!("refs/heads/{branch}");
        let bytes = self.git(
            &status.snapshot.root,
            &[
                "ls-remote",
                "--refs",
                "--",
                status.remote.as_deref().expect("validated remote"),
                &reference,
            ],
        )?;
        let text = std::str::from_utf8(&bytes)
            .map_err(|_| ReviewError::Refused("Invalid remote reference response".into()))?;
        let expected = format!(
            "{}\t{}",
            status.snapshot.head.as_deref().unwrap_or_default(),
            reference
        );
        if text.trim_end_matches(['\r', '\n']) != expected {
            return refused(
                "The origin branch does not match the reviewed commit; push it explicitly, then refresh before creating the PR",
            );
        }
        Ok(())
    }
    fn validate_pr(&self, status: &DeliveryStatus, pr: &DeliveryPullRequest) -> ReviewResult<()> {
        let repo = status
            .github
            .as_ref()
            .ok_or_else(|| ReviewError::Refused("Unknown GitHub repository".into()))?;
        let expected = format!(
            "https://{}/{}/{}/pull/{}",
            repo.host, repo.owner, repo.name, pr.number
        );
        if pr.number == 0
            || !pr.url.eq_ignore_ascii_case(&expected)
            || pr.head_ref_name != named_branch(&status.snapshot)?
            || !safe_component(&pr.author.login)
        {
            return refused(
                "GitHub returned a pull request outside the reviewed repository/branch",
            );
        }
        Ok(())
    }
    /// Only the UI's concrete confirmation callback should reach this method.
    /// All test workflows inject ReviewExecutor and never invoke live writes.
    pub fn execute(
        &self,
        plan: &DeliveryPlan,
        confirmation: DeliveryConfirmation,
    ) -> ReviewResult<DeliveryOutcome> {
        if confirmation.id != plan.id || plan.created.elapsed() > PLAN_LIFETIME {
            return refused("Delivery confirmation is invalid or expired; review again");
        }
        if plan.consumed.swap(true, Ordering::AcqRel) {
            return refused("This delivery preview has already been used; refresh before retrying");
        }
        self.revalidate(&plan.status, Some(&plan.index))?;
        if let Some(body) = &plan.body {
            body.validate()?;
        }
        if plan.status.github.is_some() {
            let actor = self.actor(&plan.status, true)?;
            if Some(&actor) != plan.actor.as_ref() {
                return refused("Authenticated GitHub actor changed; review and confirm again");
            }
        }
        if matches!(plan.intent, DeliveryIntent::CreatePullRequest { .. }) {
            if !self.open_prs(&plan.status)?.is_empty() {
                return refused("An open PR now exists for this branch; no duplicate was created");
            }
            self.validate_published_head(&plan.status)?;
        }
        // Revalidate after network reads too; a user's checkout or edits may
        // have changed while authentication/readiness was being inspected.
        self.revalidate(&plan.status, Some(&plan.index))?;
        let commands = self.effect_commands(plan)?;
        let mut output = String::new();
        for command in commands {
            let bytes = self.run(&command).map_err(|error| ReviewError::Refused(format!("Delivery may have reached the remote: {error}. Refresh GitHub status before retrying.")))?;
            output.push_str(&String::from_utf8_lossy(&bytes));
        }
        let pull_request = if matches!(plan.intent, DeliveryIntent::CreatePullRequest { .. }) {
            let url = single_line(output.as_bytes()).map_err(|_| ReviewError::Refused("GitHub may have created the PR but did not return a verifiable URL; refresh GitHub status before retrying".into()))?;
            let repository = plan.status.github.as_ref().expect("GitHub PR");
            let prefix = format!(
                "https://{}/{}/{}/pull/",
                repository.host, repository.owner, repository.name
            );
            if !url
                .get(..prefix.len())
                .is_some_and(|value| value.eq_ignore_ascii_case(&prefix))
                || !url.get(prefix.len()..).is_some_and(|number| {
                    !number.is_empty() && number.bytes().all(|ch| ch.is_ascii_digit())
                })
            {
                return refused(
                    "GitHub may have created the PR but returned an unexpected URL; refresh status before retrying",
                );
            }
            let repo = repository.cli_name();
            let bytes = self.gh(&plan.status.snapshot.root, &["pr", "view", &url, "--repo", &repo, "--json", "number,url,title,author,headRefName,baseRefName,headRefOid,isDraft"])
                .map_err(|error| ReviewError::Refused(format!("PR creation returned {url}, but author verification failed: {error}. Refresh status before retrying")))?;
            let pr: DeliveryPullRequest = parse_json(&bytes)?;
            self.validate_pr(&plan.status, &pr)?;
            if Some(&pr.author.login) != plan.actor.as_ref() {
                return refused(format!(
                    "PR {} was created but its author {} differs from the confirmed actor; inspect it before continuing",
                    pr.url, pr.author.login
                ));
            }
            if let DeliveryIntent::CreatePullRequest { base, .. } = &plan.intent {
                if &pr.base_ref_name != base {
                    return refused(format!(
                        "PR {} was created against an unexpected base; inspect it before continuing",
                        pr.url
                    ));
                }
            }
            if pr.head_ref_oid != plan.status.snapshot.head.as_deref().unwrap_or_default() {
                return refused(format!(
                    "PR {} was created but its branch moved; inspect the PR before continuing",
                    pr.url
                ));
            }
            Some(pr)
        } else {
            None
        };
        Ok(DeliveryOutcome {
            output,
            pull_request,
        })
    }
}

fn validate_snapshot(snapshot: &RepositorySnapshot) -> ReviewResult<()> {
    if !snapshot.can_mutate()
        || snapshot
            .files
            .values()
            .any(|file| matches!(file, FileImage::Refused(_)))
    {
        return refused("Delivery needs a complete, conflict-free review snapshot");
    }
    let head = snapshot.head.as_deref().unwrap_or_default();
    if !matches!(head.len(), 40 | 64) || !head.bytes().all(|ch| ch.is_ascii_hexdigit()) {
        return refused("Delivery requires a resolved commit");
    }
    Ok(())
}
fn named_branch(snapshot: &RepositorySnapshot) -> ReviewResult<&str> {
    snapshot
        .branch
        .as_deref()
        .filter(|branch| {
            !branch.is_empty() && !branch.starts_with('-') && !branch.chars().any(char::is_control)
        })
        .ok_or_else(|| ReviewError::Refused("Delivery requires a named branch".into()))
}
fn validate_remote(remote: &str) -> ReviewResult<()> {
    if remote.chars().any(char::is_control) {
        return refused("Unsafe origin remote");
    }
    let github_host = url::Url::parse(remote).ok().is_some_and(|url| {
        url.host_str()
            .is_some_and(|host| host.eq_ignore_ascii_case("github.com"))
    }) || remote
        .strip_prefix("git@")
        .and_then(|value| value.split_once(':'))
        .is_some_and(|(host, _)| host.eq_ignore_ascii_case("github.com"));
    if github_host && github_repository(remote).is_none() {
        return refused(
            "Use a canonical GitHub SSH or HTTPS origin before delivery so the repository and actor policy can be verified",
        );
    }
    if remote.starts_with("git@")
        && remote.contains(':')
        && !remote.contains(char::is_whitespace)
        && !remote.contains("::")
    {
        return Ok(());
    }
    if let Ok(url) = url::Url::parse(remote) {
        if matches!(url.scheme(), "https" | "ssh")
            && url.host_str().is_some()
            && url.password().is_none()
            && (url.scheme() != "https" || url.username().is_empty())
            && url.query().is_none()
            && url.fragment().is_none()
        {
            return Ok(());
        }
    }
    refused(
        "Delivery accepts credential-free HTTPS or SSH origins; custom remote helpers are not executed",
    )
}
fn single_line(bytes: &[u8]) -> ReviewResult<String> {
    let text = std::str::from_utf8(bytes)
        .map_err(|_| ReviewError::Refused("Command returned invalid UTF-8".into()))?
        .trim_end_matches(['\r', '\n']);
    if text.is_empty() || text.chars().any(char::is_control) {
        return refused("Command returned an invalid single-line value");
    }
    Ok(text.into())
}
fn parse_counts(bytes: &[u8]) -> ReviewResult<(Option<u64>, Option<u64>)> {
    let text = std::str::from_utf8(bytes)
        .map_err(|_| ReviewError::Refused("Invalid upstream counts".into()))?;
    let mut fields = text.split_whitespace();
    let ahead = fields.next().and_then(|v| v.parse().ok());
    let behind = fields.next().and_then(|v| v.parse().ok());
    if ahead.is_none() || behind.is_none() || fields.next().is_some() {
        return refused("Invalid upstream counts");
    }
    Ok((ahead, behind))
}
fn parse_json<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> ReviewResult<T> {
    serde_json::from_slice(bytes)
        .map_err(|error| ReviewError::Refused(format!("Invalid GitHub response: {error}")))
}
fn parse_checks(value: &serde_json::Value) -> ReviewResult<Vec<DeliveryCheck>> {
    let Some(values) = value.get("statusCheckRollup").and_then(|v| v.as_array()) else {
        return Ok(Vec::new());
    };
    if values.len() > 500 {
        return refused("Too many GitHub checks for this view");
    }
    Ok(values
        .iter()
        .map(|check| {
            let text = |key: &str| check.get(key).and_then(|v| v.as_str());
            let state = text("conclusion")
                .filter(|value| !value.is_empty())
                .or_else(|| text("status"))
                .or_else(|| text("state"))
                .unwrap_or("UNKNOWN");
            let url = text("detailsUrl")
                .or_else(|| text("targetUrl"))
                .and_then(|value| {
                    let url = url::Url::parse(value).ok()?;
                    (url.scheme() == "https"
                        && url.username().is_empty()
                        && url.password().is_none())
                    .then(|| value.to_owned())
                });
            DeliveryCheck {
                name: text("name")
                    .or_else(|| text("context"))
                    .unwrap_or("Check")
                    .chars()
                    .take(200)
                    .collect(),
                state: state.chars().take(100).collect(),
                url,
            }
        })
        .collect())
}
fn display_command(command: &ReviewCommand) -> String {
    std::iter::once(command.program.as_str())
        .chain(command.args.iter().map(String::as_str))
        .map(|arg| format!("'{}'", arg.replace('\'', "'\\''")))
        .collect::<Vec<_>>()
        .join(" ")
}
fn read_optional(path: &Path, max: usize) -> ReviewResult<Option<Vec<u8>>> {
    let before = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    if !before.is_file() || before.file_type().is_symlink() || before.len() > max as u64 {
        return refused("Delivery fence is not a safe regular file or exceeds its byte limit");
    }
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        // Never block on a FIFO swapped into place after metadata inspection,
        // or follow a symlink while validating the index/prepared body.
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let mut file = options.open(path)?;
    if !same_metadata(&before, &file.metadata()?) {
        return refused("Delivery fence changed while being opened");
    }
    let mut bytes = Vec::new();
    Read::by_ref(&mut file)
        .take(max.saturating_add(1) as u64)
        .read_to_end(&mut bytes)?;
    if bytes.len() > max || !same_metadata(&before, &fs::symlink_metadata(path)?) {
        return refused("Delivery fence changed or exceeded its byte limit");
    }
    Ok(Some(bytes))
}
fn same_metadata(first: &fs::Metadata, second: &fs::Metadata) -> bool {
    if !second.is_file()
        || second.file_type().is_symlink()
        || first.len() != second.len()
        || first.modified().ok() != second.modified().ok()
    {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if first.dev() != second.dev() || first.ino() != second.ino() {
            return false;
        }
    }
    true
}

fn refused<T>(reason: impl Into<String>) -> ReviewResult<T> {
    Err(ReviewError::Refused(reason.into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use parking_lot::Mutex;
    use serde_json::json;

    #[derive(Clone)]
    struct FakeExecutor {
        root: PathBuf,
        state: Arc<Mutex<FakeState>>,
    }
    struct FakeState {
        head: String,
        branch: String,
        remote: String,
        push_remote: Option<String>,
        rewrite: String,
        actor: String,
        pr_author: String,
        published: Option<String>,
        calls: Vec<ReviewCommand>,
        bodies: Vec<String>,
        prs: Vec<serde_json::Value>,
        rollup: serde_json::Value,
        dirty: bool,
        mutate_on_auth: bool,
    }
    impl FakeState {
        fn pr(&self) -> serde_json::Value {
            json!({ "number": 41, "url": "https://github.com/benfavre/shelldeck/pull/41", "title": "Fix delivery",
                "author": {"login": self.pr_author}, "headRefName": self.branch, "baseRefName": "main", "headRefOid": self.head, "isDraft": false })
        }
    }
    impl ReviewExecutor for FakeExecutor {
        fn run(&self, command: &ReviewCommand) -> ReviewResult<ReviewCommandOutput> {
            assert!(command.timeout <= Duration::from_secs(10));
            assert!(command.max_output_bytes <= 4 * 1024 * 1024);
            assert_eq!(command.cwd, self.root);
            let mut state = self.state.lock();
            state.calls.push(command.clone());
            let args = &command.args;
            let has = |needle: &str| args.iter().any(|arg| arg == needle);
            let ok = |text: Vec<u8>| {
                Ok(ReviewCommandOutput {
                    exit_code: 0,
                    stdout: text,
                    stderr: vec![],
                })
            };
            if command.program == "git" {
                if has("rev-parse") {
                    if has("--show-toplevel") {
                        return ok(format!("{}\n", self.root.display()).into_bytes());
                    }
                    if has("--absolute-git-dir") || has("--git-common-dir") {
                        return ok(format!("{}\n", self.root.join(".git").display()).into_bytes());
                    }
                    if has("--verify") {
                        return ok(format!("{}\n", state.head).into_bytes());
                    }
                    if has("@{upstream}") {
                        return ok(format!("origin/{}\n", state.branch).into_bytes());
                    }
                }
                if has("symbolic-ref") {
                    return ok(format!("refs/heads/{}\n", state.branch).into_bytes());
                }
                if has("status") {
                    return ok(if state.dirty {
                        b" M file.txt\0".to_vec()
                    } else {
                        vec![]
                    });
                }
                if has("ls-files") {
                    return ok(if has("--stage") {
                        format!("100644 {} 0\tfile.txt\0", "a".repeat(40)).into_bytes()
                    } else {
                        vec![]
                    });
                }
                if has("diff") {
                    return ok(if !state.dirty || has("--cached") {
                        vec![]
                    } else if has("--numstat") {
                        b"1\t1\tfile.txt\0".to_vec()
                    } else {
                        b"diff --git a/file.txt b/file.txt\n@@ -1 +1 @@\n-old\n+local dirty\n"
                            .to_vec()
                    });
                }
                if has("remote") {
                    return ok(format!(
                        "{}\n",
                        if has("--push") {
                            state.push_remote.as_deref().unwrap_or(&state.remote)
                        } else {
                            &state.remote
                        }
                    )
                    .into_bytes());
                }
                if has("config") {
                    return ok(format!(
                        "remote.origin.url\n{}\0url.example.insteadOf\n{}\0",
                        state.remote, state.rewrite
                    )
                    .into_bytes());
                }
                if has("rev-list") {
                    return ok(b"2\t1\n".to_vec());
                }
                if has("check-ref-format") {
                    return ok(b"fix/delivery\n".to_vec());
                }
                if has("ls-remote") {
                    return ok(state
                        .published
                        .as_ref()
                        .map(|head| format!("{}\trefs/heads/{}\n", head, state.branch).into_bytes())
                        .unwrap_or_default());
                }
                if has("push") {
                    return ok(b"Pushed reviewed commit\n".to_vec());
                }
            }
            if command.program == "gh" {
                if has("auth") {
                    if state.mutate_on_auth {
                        fs::write(
                            self.root.join("file.txt"),
                            b"user edited during authentication\n",
                        )?;
                    }
                    return ok(vec![]);
                }
                if has("api") {
                    return ok(format!("{}\n", state.actor).into_bytes());
                }
                if has("list") {
                    return ok(serde_json::to_vec(&state.prs).unwrap());
                }
                if has("create") {
                    let index = args
                        .iter()
                        .position(|arg| arg == "--body-file")
                        .expect("prepared file");
                    state.bodies.push(fs::read_to_string(&args[index + 1])?);
                    return ok(b"https://github.com/benfavre/shelldeck/pull/41\n".to_vec());
                }
                if has("view") {
                    let response = if args
                        .last()
                        .is_some_and(|arg| arg.contains("statusCheckRollup"))
                    {
                        state.rollup.clone()
                    } else {
                        state.pr()
                    };
                    return ok(serde_json::to_vec(&response).unwrap());
                }
            }
            panic!(
                "Unexpected fake command: {} {:?}",
                command.program, command.args
            );
        }
    }
    struct Fixture {
        executor: FakeExecutor,
    }
    impl Fixture {
        fn new() -> Self {
            let root =
                std::env::temp_dir().join(format!("shelldeck-delivery-test-{}", Uuid::new_v4()));
            fs::create_dir_all(root.join(".git")).unwrap();
            fs::write(root.join("file.txt"), b"local dirty\n").unwrap();
            fs::write(root.join(".git/index"), b"original index").unwrap();
            let head = "a".repeat(40);
            let state = FakeState {
                head: head.clone(),
                branch: "fix/delivery".into(),
                remote: "https://github.com/benfavre/shelldeck.git".into(),
                push_remote: None,
                rewrite: String::new(),
                actor: "pedrokarim".into(),
                pr_author: "pedrokarim".into(),
                published: Some(head.clone()),
                calls: vec![],
                bodies: vec![],
                prs: vec![],
                rollup: json!({"headRefOid":head,"mergeable":"MERGEABLE","mergeStateStatus":"CLEAN","statusCheckRollup":[]}),
                dirty: true,
                mutate_on_auth: false,
            };
            Self {
                executor: FakeExecutor {
                    root: fs::canonicalize(root).unwrap(),
                    state: Arc::new(Mutex::new(state)),
                },
            }
        }
        fn backend(&self) -> AgentDeliveryBackend<FakeExecutor> {
            AgentDeliveryBackend::new(self.executor.clone(), ReviewLimits::default())
        }
        fn status(&self) -> DeliveryStatus {
            self.backend().status(&self.executor.root).unwrap()
        }
        fn effects(&self) -> Vec<ReviewCommand> {
            self.executor
                .state
                .lock()
                .calls
                .iter()
                .filter(|command| {
                    command.args.iter().any(|arg| arg == "push")
                        || (command.program == "gh"
                            && command.args.iter().any(|arg| arg == "create"))
                })
                .cloned()
                .collect()
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.executor.root);
        }
    }

    // SDTEST-2440 — SDUC-556
    #[test]
    fn sdtest_2440_local_delivery_status_does_not_contact_github_or_write() {
        let fixture = Fixture::new();
        let status = fixture.status();
        assert_eq!((status.ahead, status.behind), (Some(2), Some(1)));
        assert_eq!(status.snapshot.status[0].path, "file.txt");
        assert!(fixture
            .executor
            .state
            .lock()
            .calls
            .iter()
            .all(|command| command.program == "git"));
        assert!(fixture.effects().is_empty());
    }

    // SDTEST-2441 — SDUC-556
    #[test]
    fn sdtest_2441_push_preview_is_read_only_and_confirmation_uses_exact_commit_destination() {
        let fixture = Fixture::new();
        let status = fixture.status();
        let plan = fixture.backend().prepare_push(&status).unwrap();
        assert!(fixture.effects().is_empty());
        assert!(plan
            .preview()
            .join("\n")
            .contains(&format!("{}:refs/heads/fix/delivery", "a".repeat(40))));
        fixture.backend().execute(&plan, plan.confirm()).unwrap();
        let commands = fixture.effects();
        assert_eq!(commands.len(), 1);
        assert_eq!(
            commands[0].args.last().unwrap(),
            &format!("{}:refs/heads/fix/delivery", "a".repeat(40))
        );
        assert!(commands[0]
            .args
            .iter()
            .any(|arg| arg == "https://github.com/benfavre/shelldeck.git"));
        assert!(!commands[0].args.iter().any(|arg| arg == "--force"));
        assert_eq!(
            fs::read(fixture.executor.root.join("file.txt")).unwrap(),
            b"local dirty\n"
        );
        assert_eq!(
            fs::read(fixture.executor.root.join(".git/index")).unwrap(),
            b"original index"
        );
    }

    // SDTEST-2442 — SDUC-556
    #[test]
    fn sdtest_2442_stale_branch_head_origin_index_and_working_file_each_refuse_effect() {
        for change in ["branch", "head", "origin", "index", "file"] {
            let fixture = Fixture::new();
            let plan = fixture.backend().prepare_push(&fixture.status()).unwrap();
            match change {
                "branch" => fixture.executor.state.lock().branch = "fix/other".into(),
                "head" => fixture.executor.state.lock().head = "b".repeat(40),
                "origin" => {
                    fixture.executor.state.lock().remote =
                        "https://github.com/other/shelldeck.git".into()
                }
                "index" => fs::write(
                    fixture.executor.root.join(".git/index"),
                    b"new staged state",
                )
                .unwrap(),
                "file" => {
                    fs::write(fixture.executor.root.join("file.txt"), b"new user work").unwrap()
                }
                _ => unreachable!(),
            }
            assert!(
                fixture.backend().execute(&plan, plan.confirm()).is_err(),
                "{change}"
            );
            assert!(fixture.effects().is_empty(), "{change}");
        }
    }

    // SDTEST-2443 — SDUC-556
    #[test]
    fn sdtest_2443_repository_actor_policy_refuses_untrusted_and_changed_accounts() {
        let fixture = Fixture::new();
        fixture.executor.state.lock().actor = "unexpected-bot".into();
        assert!(fixture.backend().prepare_push(&fixture.status()).is_err());
        fixture.executor.state.lock().actor = "pedrokarim".into();
        let plan = fixture.backend().prepare_push(&fixture.status()).unwrap();
        fixture.executor.state.lock().actor = "benfavre".into();
        assert!(fixture.backend().execute(&plan, plan.confirm()).is_err());
        assert!(fixture.effects().is_empty());
        let generic = github_repository("https://github.com/example/project.git").unwrap();
        assert!(generic.permits_actor("someone-else"));
    }

    // SDTEST-2444 — SDUC-556
    #[test]
    fn sdtest_2444_pr_body_preserves_newlines_and_explicit_head_repo_and_author_are_verified() {
        let fixture = Fixture::new();
        let body = "A concrete change.\n\nValidation:\n- unit tests\n- literal `$(echo nope)`\n";
        let plan = fixture
            .backend()
            .prepare_pull_request(
                &fixture.status(),
                "Fix delivery".into(),
                body.into(),
                "main".into(),
            )
            .unwrap();
        assert!(fixture.effects().is_empty());
        let body_path = plan.body.as_ref().unwrap().path.clone();
        let outcome = fixture.backend().execute(&plan, plan.confirm()).unwrap();
        assert_eq!(outcome.pull_request.unwrap().author.login, "pedrokarim");
        let state = fixture.executor.state.lock();
        assert_eq!(state.bodies, [body]);
        let effect = state
            .calls
            .iter()
            .find(|call| call.program == "gh" && call.args.iter().any(|arg| arg == "create"))
            .unwrap();
        let value_after = |key: &str| {
            effect.args[effect.args.iter().position(|arg| arg == key).unwrap() + 1].as_str()
        };
        assert_eq!(value_after("--head"), "fix/delivery");
        assert_eq!(value_after("--repo"), "github.com/benfavre/shelldeck");
        assert_eq!(value_after("--base"), "main");
        assert!(!effect.args.iter().any(|arg| arg == "--body"));
        drop(state);
        drop(plan);
        assert!(!body_path.exists());
    }

    // SDTEST-2445 — SDUC-556
    #[test]
    fn sdtest_2445_duplicate_pr_and_unpublished_head_are_refused_before_create() {
        let fixture = Fixture::new();
        fixture.executor.state.lock().published = None;
        assert!(fixture
            .backend()
            .prepare_pull_request(
                &fixture.status(),
                "Fix".into(),
                "Body".into(),
                "main".into()
            )
            .is_err());
        fixture.executor.state.lock().published = Some("a".repeat(40));
        let plan = fixture
            .backend()
            .prepare_pull_request(
                &fixture.status(),
                "Fix".into(),
                "Body".into(),
                "main".into(),
            )
            .unwrap();
        {
            let mut state = fixture.executor.state.lock();
            let pr = state.pr();
            state.prs.push(pr);
        }
        assert!(fixture.backend().execute(&plan, plan.confirm()).is_err());
        assert!(fixture.effects().is_empty());
    }

    // SDTEST-2446 — SDUC-556
    #[test]
    fn sdtest_2446_tampered_body_and_replayed_confirmation_cannot_create_or_push_twice() {
        let fixture = Fixture::new();
        let plan = fixture
            .backend()
            .prepare_pull_request(
                &fixture.status(),
                "Fix".into(),
                "Reviewed body".into(),
                "main".into(),
            )
            .unwrap();
        fs::write(&plan.body.as_ref().unwrap().path, "unreviewed body").unwrap();
        assert!(fixture.backend().execute(&plan, plan.confirm()).is_err());
        assert!(fixture.effects().is_empty());
        let push = fixture.backend().prepare_push(&fixture.status()).unwrap();
        fixture.backend().execute(&push, push.confirm()).unwrap();
        assert!(fixture.backend().execute(&push, push.confirm()).is_err());
        assert_eq!(fixture.effects().len(), 1);
    }

    // SDTEST-2447 — SDUC-556
    #[test]
    fn sdtest_2447_checks_readiness_and_untrusted_links_are_parsed_without_effects() {
        let fixture = Fixture::new();
        {
            let mut state = fixture.executor.state.lock();
            let pr = state.pr();
            state.prs.push(pr);
            state.rollup = json!({"headRefOid":"b".repeat(40),"mergeable":"MERGEABLE","mergeStateStatus":"BLOCKED","statusCheckRollup":[
                {"name":"tests","status":"COMPLETED","conclusion":"SUCCESS","detailsUrl":"https://github.com/benfavre/shelldeck/actions/runs/1"},
                {"context":"legacy","state":"PENDING","targetUrl":"javascript:alert(1)"}
            ]});
        }
        let read = fixture.backend().github_status(&fixture.status()).unwrap();
        assert!(!read.head_matches);
        assert_eq!(read.merge_state.as_deref(), Some("BLOCKED"));
        assert_eq!(read.checks[0].state, "SUCCESS");
        assert_eq!(read.checks[1].state, "PENDING");
        assert!(read.checks[1].url.is_none());
        assert!(fixture.effects().is_empty());
    }

    // SDTEST-2448 — SDUC-556
    #[test]
    fn sdtest_2448_created_pr_author_mismatch_is_reported_after_one_effect_without_retry() {
        let fixture = Fixture::new();
        fixture.executor.state.lock().pr_author = "unexpected-bot".into();
        let plan = fixture
            .backend()
            .prepare_pull_request(
                &fixture.status(),
                "Fix".into(),
                "Body".into(),
                "main".into(),
            )
            .unwrap();
        let error = fixture
            .backend()
            .execute(&plan, plan.confirm())
            .unwrap_err()
            .to_string();
        assert!(error.contains("was created") && error.contains("unexpected-bot"));
        assert!(fixture.backend().execute(&plan, plan.confirm()).is_err());
        assert_eq!(fixture.effects().len(), 1);
    }

    // SDTEST-2449 — SDUC-556
    #[test]
    fn sdtest_2449_changes_during_authentication_are_caught_before_effect() {
        let fixture = Fixture::new();
        let plan = fixture.backend().prepare_push(&fixture.status()).unwrap();
        fixture.executor.state.lock().mutate_on_auth = true;
        assert!(fixture.backend().execute(&plan, plan.confirm()).is_err());
        assert!(fixture.effects().is_empty());
        assert_eq!(
            fs::read(fixture.executor.root.join("file.txt")).unwrap(),
            b"user edited during authentication\n"
        );
    }

    // SDTEST-2450 — SDUC-556
    #[test]
    fn sdtest_2450_origin_parser_rejects_credentials_encoded_paths_and_remote_helpers() {
        for remote in [
            "https://github.com/benfavre/shelldeck.git",
            "git@github.com:benfavre/shelldeck.git",
            "ssh://git@github.com/benfavre/shelldeck.git",
        ] {
            let repo = github_repository(remote).unwrap();
            assert_eq!(repo.cli_name(), "github.com/benfavre/shelldeck");
            assert!(!repo.permits_actor("unexpected-bot"));
        }
        for remote in [
            "https://token@github.com/benfavre/shelldeck",
            "https://github.com/benfavre/%73helldeck",
            "https://evil.example/benfavre/shelldeck",
            "ext::malicious",
            "https://github.com/benfavre/shelldeck?x=1",
            "https://github.com/benfavre/shelldeck/extra",
        ] {
            assert!(github_repository(remote).is_none(), "{remote}");
        }
    }

    // SDTEST-2451 — SDUC-556
    #[test]
    fn sdtest_2451_index_change_before_preview_invalidates_original_snapshot() {
        let fixture = Fixture::new();
        let status = fixture.status();
        fs::write(
            fixture.executor.root.join(".git/index"),
            b"different staged binary object",
        )
        .unwrap();
        assert!(fixture.backend().prepare_push(&status).is_err());
        assert!(fixture.effects().is_empty());
    }

    // SDTEST-2452 — SDUC-556
    #[test]
    fn sdtest_2452_wrong_or_expired_confirmation_never_publishes() {
        let fixture = Fixture::new();
        let first = fixture.backend().prepare_push(&fixture.status()).unwrap();
        let mut second = fixture.backend().prepare_push(&fixture.status()).unwrap();
        assert!(fixture.backend().execute(&first, second.confirm()).is_err());
        second.created = Instant::now() - PLAN_LIFETIME - Duration::from_secs(1);
        assert!(fixture
            .backend()
            .execute(&second, second.confirm())
            .is_err());
        assert!(fixture.effects().is_empty());
    }

    // SDTEST-2453 — SDUC-556; Unix symlink semantics, compiled only on Unix.
    #[cfg(unix)]
    #[test]
    fn sdtest_2453_retargeted_session_workdir_refuses_previous_repository_delivery() {
        use std::os::unix::fs::symlink;
        let first = Fixture::new();
        let second = Fixture::new();
        let link = first.executor.root.with_extension("workdir-link");
        symlink(&first.executor.root, &link).unwrap();
        let status = first.backend().status(&link).unwrap();
        let plan = first.backend().prepare_push(&status).unwrap();
        fs::remove_file(&link).unwrap();
        symlink(&second.executor.root, &link).unwrap();
        assert!(first.backend().execute(&plan, plan.confirm()).is_err());
        assert!(first.effects().is_empty());
        fs::remove_file(link).unwrap();
    }

    // SDTEST-2454 — SDUC-556
    #[test]
    fn sdtest_2454_remote_branch_moving_after_preview_refuses_pull_request_creation() {
        let fixture = Fixture::new();
        let plan = fixture
            .backend()
            .prepare_pull_request(
                &fixture.status(),
                "Fix".into(),
                "Body".into(),
                "main".into(),
            )
            .unwrap();
        fixture.executor.state.lock().published = Some("b".repeat(40));
        assert!(fixture.backend().execute(&plan, plan.confirm()).is_err());
        assert!(fixture.effects().is_empty());
    }

    // SDTEST-2455 — SDUC-556
    #[test]
    fn sdtest_2455_incomplete_unborn_detached_and_unsafe_origins_are_not_delivery_candidates() {
        let fixture = Fixture::new();
        for mode in ["incomplete", "unborn", "detached"] {
            let mut status = fixture.status();
            let snapshot = Arc::make_mut(&mut status.snapshot);
            match mode {
                "incomplete" => snapshot.complete = false,
                "unborn" => snapshot.head = None,
                "detached" => snapshot.branch = None,
                _ => unreachable!(),
            }
            assert!(fixture.backend().prepare_push(&status).is_err(), "{mode}");
        }
        for remote in [
            "ext::sh -c dangerous",
            "https://token@github.com/benfavre/shelldeck",
            "https://github.com/benfavre/%73helldeck",
        ] {
            assert!(validate_remote(remote).is_err(), "{remote}");
        }
        assert!(fixture.effects().is_empty());
    }

    // SDTEST-2456 — SDUC-556
    #[test]
    fn sdtest_2456_distinct_push_urls_and_changed_url_rewrites_never_redirect_delivery() {
        let fixture = Fixture::new();
        fixture.executor.state.lock().push_remote =
            Some("https://github.com/other/shelldeck.git".into());
        assert!(fixture.backend().prepare_push(&fixture.status()).is_err());
        fixture.executor.state.lock().push_remote = None;
        let plan = fixture.backend().prepare_push(&fixture.status()).unwrap();
        fixture.executor.state.lock().rewrite = "https://github.com/".into();
        assert!(fixture.backend().execute(&plan, plan.confirm()).is_err());
        assert!(fixture.effects().is_empty());
    }
}
