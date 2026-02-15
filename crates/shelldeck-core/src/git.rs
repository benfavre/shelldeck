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
    if !is_git_repo(dir) {
        return None;
    }

    let branch = Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .current_dir(dir)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string());

    let status_output = Command::new("git")
        .args(["status", "--porcelain=v1"])
        .current_dir(dir)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .unwrap_or_default();

    let mut modified = 0;
    let mut staged = 0;
    let mut untracked = 0;

    for line in status_output.lines() {
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

    Some(GitStatus {
        branch,
        modified,
        staged,
        untracked,
    })
}
