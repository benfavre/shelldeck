use std::path::Path;
use std::process::Command;

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
}
