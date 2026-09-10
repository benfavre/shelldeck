use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;

/// Upper bound for one file diff handed to the UI. Larger diffs are cut at a
/// line boundary, so a generated or vendored file cannot stall the renderer.
pub const MAX_DIFF_BYTES: usize = 64 * 1024;

/// Untracked files above this size are not read to count their lines.
const MAX_UNTRACKED_READ_BYTES: u64 = 1024 * 1024;

/// Git repository status information.
#[derive(Debug, Clone, Default)]
pub struct GitStatus {
    /// Current branch name (e.g., "main").
    pub branch: Option<String>,
    /// Number of files added/modified (unstaged + staged).
    pub modified: usize,
    /// Number of staged files.
    pub staged: usize,
    /// Number of untracked files.
    pub untracked: usize,
}

impl GitStatus {
    /// Format for status bar display, e.g., "main +3 ~1"
    pub fn display(&self) -> Option<String> {
        let branch = self.branch.as_ref()?;
        let mut parts = vec![branch.clone()];
        if self.staged > 0 {
            parts.push(format!("+{}", self.staged));
        }
        if self.modified > 0 {
            parts.push(format!("~{}", self.modified));
        }
        if self.untracked > 0 {
            parts.push(format!("?{}", self.untracked));
        }
        Some(parts.join(" "))
    }
}

/// Check if a directory is inside a git repository.
pub fn is_git_repo(dir: &Path) -> bool {
    Command::new("git")
        .args(["rev-parse", "--is-inside-work-tree"])
        .current_dir(dir)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Get git status for a directory. Returns None if not a git repo.
pub fn get_git_status(dir: &Path) -> Option<GitStatus> {
    let output = Command::new("git")
        .args(["status", "--porcelain=v1", "--branch"])
        .current_dir(dir)
        .output()
        .ok()
        .filter(|output| output.status.success())?;

    Some(parse_status(&String::from_utf8_lossy(&output.stdout)))
}

fn parse_status(output: &str) -> GitStatus {
    let mut branch = None;

    let mut modified = 0;
    let mut staged = 0;
    let mut untracked = 0;

    for line in output.lines() {
        if let Some(header) = line.strip_prefix("## ") {
            let name = header
                .strip_prefix("No commits yet on ")
                .or_else(|| header.strip_prefix("Initial commit on "))
                .unwrap_or(header)
                .split("...")
                .next()
                .unwrap_or_default()
                .split_whitespace()
                .next()
                .unwrap_or_default();
            if !name.is_empty() {
                branch = Some(name.to_string());
            }
            continue;
        }
        if line.len() < 2 {
            continue;
        }
        let index = line.as_bytes()[0];
        let worktree = line.as_bytes()[1];

        if line.starts_with("??") {
            untracked += 1;
        } else {
            if index != b' ' && index != b'?' {
                staged += 1;
            }
            if worktree != b' ' && worktree != b'?' {
                modified += 1;
            }
        }
    }

    GitStatus {
        branch,
        modified,
        staged,
        untracked,
    }
}

/// State of one side (index or working tree) of a porcelain entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitChange {
    Unmodified,
    Modified,
    Added,
    Deleted,
    Renamed,
    Copied,
    TypeChanged,
    Unmerged,
    Untracked,
}

impl GitChange {
    fn from_code(code: u8) -> Self {
        match code {
            b'M' => Self::Modified,
            b'A' => Self::Added,
            b'D' => Self::Deleted,
            b'R' => Self::Renamed,
            b'C' => Self::Copied,
            b'T' => Self::TypeChanged,
            b'U' => Self::Unmerged,
            b'?' => Self::Untracked,
            _ => Self::Unmodified,
        }
    }
}

/// One path reported by `git status --porcelain=v1 -z`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitFileEntry {
    pub path: String,
    /// Source path of a staged rename or copy.
    pub original_path: Option<String>,
    pub index: GitChange,
    pub worktree: GitChange,
}

impl GitFileEntry {
    pub fn is_untracked(&self) -> bool {
        self.index == GitChange::Untracked
    }

    /// Unmerged paths, including both-added and both-deleted conflicts.
    pub fn is_conflicted(&self) -> bool {
        self.index == GitChange::Unmerged
            || self.worktree == GitChange::Unmerged
            || (self.index == GitChange::Added && self.worktree == GitChange::Added)
            || (self.index == GitChange::Deleted && self.worktree == GitChange::Deleted)
    }

    pub fn has_staged_change(&self) -> bool {
        !self.is_conflicted() && !matches!(self.index, GitChange::Unmodified | GitChange::Untracked)
    }

    pub fn has_unstaged_change(&self) -> bool {
        self.is_untracked() || (!self.is_conflicted() && self.worktree != GitChange::Unmodified)
    }

    /// A file the repository does not know yet, whether staged or not.
    pub fn is_new(&self) -> bool {
        matches!(self.index, GitChange::Untracked | GitChange::Added)
    }

    /// One-letter badge for the staged (index) or unstaged (working tree) row.
    pub fn letter(&self, staged: bool) -> &'static str {
        if self.is_conflicted() {
            return "U";
        }
        let change = if staged {
            self.index
        } else if self.is_untracked() {
            GitChange::Added
        } else {
            self.worktree
        };
        match change {
            GitChange::Added | GitChange::Untracked => "A",
            GitChange::Deleted => "D",
            GitChange::Renamed => "R",
            GitChange::Copied => "C",
            GitChange::TypeChanged => "T",
            GitChange::Unmerged => "U",
            GitChange::Modified | GitChange::Unmodified => "M",
        }
    }
}

/// Branch, tracking and per-file state of a working tree.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GitWorkingTree {
    pub branch: Option<String>,
    pub upstream: Option<String>,
    pub ahead: u32,
    pub behind: u32,
    pub files: Vec<GitFileEntry>,
}

/// Bounded stderr of a failed git invocation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitCommandError(pub String);

impl std::fmt::Display for GitCommandError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for GitCommandError {}

fn run_git(dir: &Path, args: &[&str]) -> Result<String, GitCommandError> {
    let output = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .map_err(|error| GitCommandError(format!("git: {error}")))?;
    if output.status.success() {
        return Ok(String::from_utf8_lossy(&output.stdout).into_owned());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    let message: String = stderr.trim().chars().take(400).collect();
    if message.is_empty() {
        Err(GitCommandError(format!(
            "git {} failed",
            args.first().copied().unwrap_or_default()
        )))
    } else {
        Err(GitCommandError(message))
    }
}

/// Per-file working tree state, or `None` outside a repository.
pub fn read_working_tree(dir: &Path) -> Option<GitWorkingTree> {
    run_git(
        dir,
        &[
            "status",
            "--porcelain=v1",
            "--branch",
            "-z",
            "--untracked-files=all",
        ],
    )
    .ok()
    .map(|output| parse_porcelain_z(&output))
}

fn parse_porcelain_z(output: &str) -> GitWorkingTree {
    let mut tree = GitWorkingTree::default();
    let mut records = output.split('\0').filter(|record| !record.is_empty());
    while let Some(record) = records.next() {
        if let Some(header) = record.strip_prefix("## ") {
            parse_branch_header(header, &mut tree);
            continue;
        }
        // `XY path`: the two status codes and the separating space are ASCII.
        if record.len() < 4 || !record.is_char_boundary(3) {
            continue;
        }
        let codes = record.as_bytes();
        let (x, y) = (codes[0], codes[1]);
        // A staged rename or copy carries its source as the next record.
        let original_path = matches!(x, b'R' | b'C')
            .then(|| records.next().map(str::to_string))
            .flatten();
        if x == b'!' {
            continue;
        }
        tree.files.push(GitFileEntry {
            path: record[3..].to_string(),
            original_path,
            index: GitChange::from_code(x),
            worktree: GitChange::from_code(y),
        });
    }
    tree
}

fn parse_branch_header(header: &str, tree: &mut GitWorkingTree) {
    let header = header
        .strip_prefix("No commits yet on ")
        .or_else(|| header.strip_prefix("Initial commit on "))
        .unwrap_or(header);
    let (names, tracking) = match header.find(" [") {
        Some(start) if header.ends_with(']') => {
            (&header[..start], Some(&header[start + 2..header.len() - 1]))
        }
        _ => (header, None),
    };
    let mut parts = names.splitn(2, "...");
    let branch = parts
        .next()
        .unwrap_or_default()
        .split_whitespace()
        .next()
        .unwrap_or_default();
    if !branch.is_empty() {
        tree.branch = Some(branch.to_string());
    }
    tree.upstream = parts
        .next()
        .map(str::trim)
        .filter(|upstream| !upstream.is_empty())
        .map(str::to_string);
    for item in tracking.unwrap_or_default().split(", ") {
        if let Some(count) = item.strip_prefix("ahead ") {
            tree.ahead = count.parse().unwrap_or(0);
        } else if let Some(count) = item.strip_prefix("behind ") {
            tree.behind = count.parse().unwrap_or(0);
        }
    }
}

/// Added and deleted line counts per path, for the index (`staged`) or the
/// working tree. Binary files count as zero.
pub fn diff_line_counts(dir: &Path, staged: bool) -> BTreeMap<String, (u32, u32)> {
    let mut args = vec!["diff", "--numstat", "-z", "--no-color", "--no-ext-diff"];
    if staged {
        args.push("--cached");
    }
    run_git(dir, &args)
        .map(|output| parse_numstat_z(&output))
        .unwrap_or_default()
}

fn parse_numstat_z(output: &str) -> BTreeMap<String, (u32, u32)> {
    let mut stats = BTreeMap::new();
    let mut fields = output.split('\0');
    while let Some(record) = fields.next() {
        if record.is_empty() {
            continue;
        }
        let mut columns = record.splitn(3, '\t');
        let (Some(added), Some(deleted), Some(path)) =
            (columns.next(), columns.next(), columns.next())
        else {
            continue;
        };
        // A rename leaves the path column empty and sends `old\0new\0`.
        let path = if path.is_empty() {
            let _source = fields.next();
            fields.next().unwrap_or_default()
        } else {
            path
        };
        if path.is_empty() {
            continue;
        }
        stats.insert(
            path.to_string(),
            (added.parse().unwrap_or(0), deleted.parse().unwrap_or(0)),
        );
    }
    stats
}

/// Line count of an untracked file, which `git diff` does not report.
pub fn untracked_line_count(dir: &Path, path: &str) -> Option<u32> {
    let full = dir.join(path);
    let metadata = std::fs::metadata(&full).ok()?;
    if !metadata.is_file() || metadata.len() > MAX_UNTRACKED_READ_BYTES {
        return None;
    }
    let bytes = std::fs::read(&full).ok()?;
    if bytes.contains(&0) {
        return Some(0);
    }
    let newlines = bytes.iter().filter(|byte| **byte == b'\n').count();
    let unterminated = usize::from(!bytes.is_empty() && !bytes.ends_with(b"\n"));
    Some((newlines + unterminated).min(u32::MAX as usize) as u32)
}

/// Unified diff of one path, bounded to `MAX_DIFF_BYTES`. An untracked file is
/// shown as a whole-file addition read directly, without invoking git.
pub fn file_diff(
    dir: &Path,
    path: &str,
    staged: bool,
    untracked: bool,
) -> Result<String, GitCommandError> {
    let text = if untracked {
        let bytes = std::fs::read(dir.join(path))
            .map_err(|error| GitCommandError(format!("{path}: {error}")))?;
        let mut diff = format!("--- /dev/null\n+++ b/{path}\n");
        if bytes.contains(&0) {
            diff.push_str("Binary file\n");
        } else {
            for line in String::from_utf8_lossy(&bytes).lines() {
                diff.push('+');
                diff.push_str(line);
                diff.push('\n');
                if diff.len() > MAX_DIFF_BYTES {
                    break;
                }
            }
        }
        diff
    } else {
        let mut args = vec!["diff", "--no-color", "--no-ext-diff"];
        if staged {
            args.push("--cached");
        }
        args.extend(["--", path]);
        run_git(dir, &args)?
    };
    Ok(truncate_diff(text))
}

fn truncate_diff(mut text: String) -> String {
    if text.len() <= MAX_DIFF_BYTES {
        return text;
    }
    let mut cut = MAX_DIFF_BYTES;
    while !text.is_char_boundary(cut) {
        cut -= 1;
    }
    if let Some(newline) = text[..cut].rfind('\n') {
        cut = newline + 1;
    }
    text.truncate(cut);
    text.push_str("…\n");
    text
}

/// Stage the given paths.
pub fn stage_paths(dir: &Path, paths: &[String]) -> Result<(), GitCommandError> {
    if paths.is_empty() {
        return Ok(());
    }
    let mut args = vec!["add", "--"];
    args.extend(paths.iter().map(String::as_str));
    run_git(dir, &args).map(|_| ())
}

/// Stage every change in the working tree, including deletions and new files.
pub fn stage_all(dir: &Path) -> Result<(), GitCommandError> {
    run_git(dir, &["add", "--all"]).map(|_| ())
}

/// Remove paths from the index without touching the working tree. Before the
/// first commit there is no HEAD to restore from, so the paths are dropped
/// from the index instead.
pub fn unstage_paths(dir: &Path, paths: &[String]) -> Result<(), GitCommandError> {
    if paths.is_empty() {
        return Ok(());
    }
    let has_head = run_git(dir, &["rev-parse", "--verify", "--quiet", "HEAD"]).is_ok();
    let mut args = if has_head {
        vec!["restore", "--staged", "--"]
    } else {
        vec!["rm", "--cached", "--quiet", "-r", "--"]
    };
    args.extend(paths.iter().map(String::as_str));
    run_git(dir, &args).map(|_| ())
}

/// Commit what is staged, keeping the repository's hooks and signing
/// configuration, and return the abbreviated commit id.
pub fn commit_staged(dir: &Path, message: &str) -> Result<String, GitCommandError> {
    let message = message.trim();
    if message.is_empty() {
        return Err(GitCommandError("empty commit message".to_string()));
    }
    run_git(dir, &["commit", "--quiet", "--message", message])?;
    run_git(dir, &["rev-parse", "--short", "HEAD"]).map(|sha| sha.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    // SDTEST-1387
    #[test]
    fn porcelain_branch_status_parses_in_one_pass() {
        let status = parse_status(
            "## feature/perf...origin/feature/perf [ahead 2]\n M modified.rs\nM  staged.rs\nMM both.rs\n?? new.rs\n",
        );

        assert_eq!(status.branch.as_deref(), Some("feature/perf"));
        assert_eq!(status.modified, 2);
        assert_eq!(status.staged, 2);
        assert_eq!(status.untracked, 1);

        let unborn = parse_status("## No commits yet on main\n?? README.md\n");
        assert_eq!(unborn.branch.as_deref(), Some("main"));
        assert_eq!(unborn.untracked, 1);

        let detached = parse_status("## HEAD (no branch)\n");
        assert_eq!(detached.branch.as_deref(), Some("HEAD"));
    }

    // SDTEST-1926
    #[test]
    fn sdtest_1926_porcelain_and_numstat_parsers_keep_exact_paths_and_sides() {
        let tree = parse_porcelain_z(
            "## feature/x...origin/feature/x [ahead 2, behind 1]\x00M  staged.rs\x00 M unstaged.rs\x00MM both.rs\x00R  new name.rs\x00old name.rs\x00?? dir/new.txt\x00UU conflict.rs\x00",
        );
        assert_eq!(tree.branch.as_deref(), Some("feature/x"));
        assert_eq!(tree.upstream.as_deref(), Some("origin/feature/x"));
        assert_eq!((tree.ahead, tree.behind), (2, 1));
        assert_eq!(tree.files.len(), 6);

        let staged = &tree.files[0];
        assert!(staged.has_staged_change() && !staged.has_unstaged_change());
        let unstaged = &tree.files[1];
        assert!(!unstaged.has_staged_change() && unstaged.has_unstaged_change());
        let both = &tree.files[2];
        assert!(both.has_staged_change() && both.has_unstaged_change());
        let renamed = &tree.files[3];
        assert_eq!(renamed.path, "new name.rs");
        assert_eq!(renamed.original_path.as_deref(), Some("old name.rs"));
        assert_eq!(renamed.letter(true), "R");
        let untracked = &tree.files[4];
        assert_eq!(untracked.path, "dir/new.txt");
        assert!(untracked.is_untracked() && untracked.is_new());
        assert!(!untracked.has_staged_change() && untracked.has_unstaged_change());
        assert_eq!(untracked.letter(false), "A");
        let conflict = &tree.files[5];
        assert!(conflict.is_conflicted());
        assert!(!conflict.has_staged_change() && !conflict.has_unstaged_change());
        assert_eq!(conflict.letter(false), "U");

        let unborn = parse_porcelain_z("## No commits yet on main\x00A  first.rs\x00");
        assert_eq!(unborn.branch.as_deref(), Some("main"));
        assert!(unborn.files[0].is_new() && unborn.files[0].has_staged_change());
        let gone = parse_porcelain_z("## topic...origin/topic [gone]\x00");
        assert_eq!(gone.upstream.as_deref(), Some("origin/topic"));
        assert_eq!((gone.ahead, gone.behind), (0, 0));

        let stats = parse_numstat_z(
            "12\t4\tsrc/lib.rs\x00-\t-\tlogo.png\x003\t0\t\x00old.rs\x00new.rs\x00",
        );
        assert_eq!(stats.get("src/lib.rs"), Some(&(12, 4)));
        assert_eq!(stats.get("logo.png"), Some(&(0, 0)));
        assert_eq!(stats.get("new.rs"), Some(&(3, 0)));
        assert!(!stats.contains_key("old.rs"));
    }

    // SDTEST-1927
    #[test]
    fn sdtest_1927_stage_unstage_diff_and_commit_act_on_the_real_index() {
        let dir = std::env::temp_dir().join(format!(
            "shelldeck-git-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).expect("create temp repository");
        let git = |args: &[&str]| {
            let output = Command::new("git")
                .args(args)
                .current_dir(&dir)
                .output()
                .expect("git available for git integration test");
            assert!(
                output.status.success(),
                "git {args:?}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        };
        let hooks = dir.join(".no-hooks");
        git(&["init", "--quiet"]);
        git(&["config", "user.email", "test@example.test"]);
        git(&["config", "user.name", "ShellDeck Test"]);
        git(&["config", "commit.gpgsign", "false"]);
        git(&["config", "core.autocrlf", "false"]);
        git(&["config", "core.hooksPath", hooks.to_str().unwrap()]);
        let entry = |path: &str| {
            read_working_tree(&dir)
                .expect("inside a repository")
                .files
                .into_iter()
                .find(|file| file.path == path)
                .expect("path listed")
        };

        // Before the first commit, unstaging has no HEAD to restore from.
        std::fs::write(dir.join("tracked.txt"), "one\n").unwrap();
        stage_paths(&dir, &["tracked.txt".to_string()]).unwrap();
        assert!(entry("tracked.txt").has_staged_change());
        unstage_paths(&dir, &["tracked.txt".to_string()]).unwrap();
        assert!(entry("tracked.txt").is_untracked());
        stage_all(&dir).unwrap();
        assert!(!commit_staged(&dir, "first").unwrap().is_empty());
        assert!(commit_staged(&dir, "   ").is_err());

        std::fs::write(dir.join("tracked.txt"), "one\ntwo\n").unwrap();
        std::fs::write(dir.join("new.txt"), "fresh\nlines\n").unwrap();
        assert!(entry("tracked.txt").has_unstaged_change());
        assert!(entry("new.txt").is_untracked());
        assert_eq!(untracked_line_count(&dir, "new.txt"), Some(2));
        assert_eq!(
            diff_line_counts(&dir, false).get("tracked.txt"),
            Some(&(1, 0))
        );
        assert!(file_diff(&dir, "tracked.txt", false, false)
            .unwrap()
            .contains("+two"));
        assert!(file_diff(&dir, "new.txt", false, true)
            .unwrap()
            .contains("+fresh"));

        stage_paths(&dir, &["tracked.txt".to_string()]).unwrap();
        assert_eq!(
            diff_line_counts(&dir, true).get("tracked.txt"),
            Some(&(1, 0))
        );
        assert!(file_diff(&dir, "tracked.txt", true, false)
            .unwrap()
            .contains("+two"));
        unstage_paths(&dir, &["tracked.txt".to_string()]).unwrap();
        assert!(diff_line_counts(&dir, true).is_empty());
        assert!(entry("tracked.txt").has_unstaged_change());

        std::fs::remove_dir_all(&dir).ok();
    }
}
