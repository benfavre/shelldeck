use std::collections::HashSet;
use std::path::{Path, PathBuf};

const MAX_DEPTH: usize = 20;
const MAX_VISIBLE_ENTRIES: usize = 5_000;
const MAX_SCANNED_ENTRIES: usize = 20_000;

pub struct FileBrowserListing {
    pub entries: Vec<FileEntry>,
    pub truncated: bool,
}

#[derive(Default)]
struct ListingBudget {
    scanned: usize,
    truncated: bool,
}

/// A single entry in the file browser tree.
#[derive(Debug, Clone)]
pub struct FileEntry {
    pub path: PathBuf,
    pub name: String,
    pub is_dir: bool,
    pub is_expanded: bool,
    pub depth: usize,
}

/// File browser panel state. Manages a tree of directories/files.
pub struct FileBrowserPanel {
    root: PathBuf,
    expanded_dirs: HashSet<PathBuf>,
}

impl Default for FileBrowserPanel {
    fn default() -> Self {
        Self::new()
    }
}

impl FileBrowserPanel {
    pub fn new() -> Self {
        let root = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let mut expanded = HashSet::new();
        expanded.insert(root.clone());
        Self {
            root,
            expanded_dirs: expanded,
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn set_root(&mut self, root: PathBuf) {
        self.expanded_dirs.clear();
        self.expanded_dirs.insert(root.clone());
        self.root = root;
    }

    /// Reveal a document inside this tree without changing its root.
    pub fn reveal_path(&mut self, path: &Path) {
        if !path.starts_with(&self.root) {
            return;
        }
        for parent in path.ancestors().skip(1) {
            if !parent.starts_with(&self.root) {
                break;
            }
            self.expanded_dirs.insert(parent.to_path_buf());
        }
    }

    pub fn toggle_dir(&mut self, path: &Path) {
        if self.expanded_dirs.contains(path) {
            self.expanded_dirs.remove(path);
        } else {
            self.expanded_dirs.insert(path.to_path_buf());
        }
    }

    /// Returns a flat list of visible entries for rendering.
    pub fn visible_entries(&self) -> Vec<FileEntry> {
        self.visible_entries_with_hidden(false)
    }

    /// Show hidden project entries on demand; Git internals and build output
    /// remain excluded even when hidden files are visible.
    pub fn visible_entries_with_hidden(&self, show_hidden: bool) -> Vec<FileEntry> {
        self.visible_listing_with_hidden(show_hidden).entries
    }

    pub fn visible_listing_with_hidden(&self, show_hidden: bool) -> FileBrowserListing {
        let mut entries = Vec::new();
        let mut budget = ListingBudget::default();
        self.collect_entries(&self.root, 0, show_hidden, &mut entries, &mut budget);
        FileBrowserListing {
            entries,
            truncated: budget.truncated,
        }
    }

    fn collect_entries(
        &self,
        dir: &Path,
        depth: usize,
        show_hidden: bool,
        entries: &mut Vec<FileEntry>,
        budget: &mut ListingBudget,
    ) {
        if entries.len() >= MAX_VISIBLE_ENTRIES {
            budget.truncated = true;
            return;
        }
        if depth > MAX_DEPTH {
            return;
        }
        let expanded = self.expanded_dirs.contains(dir);

        // Add the directory itself (except root at depth 0)
        if depth > 0 {
            entries.push(FileEntry {
                path: dir.to_path_buf(),
                name: dir
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("?")
                    .to_string(),
                is_dir: true,
                is_expanded: expanded,
                depth: depth - 1,
            });
        }

        if !expanded {
            return;
        }
        if budget.scanned >= MAX_SCANNED_ENTRIES {
            budget.truncated = true;
            return;
        }

        // Read directory entries
        let mut dirs = Vec::new();
        let mut files = Vec::new();

        if let Ok(read_dir) = std::fs::read_dir(dir) {
            for entry in read_dir.flatten() {
                if budget.scanned >= MAX_SCANNED_ENTRIES {
                    budget.truncated = true;
                    break;
                }
                budget.scanned += 1;
                let path = entry.path();
                let name = entry.file_name().to_str().unwrap_or("?").to_string();

                // Directory symlinks can leave the project or form cycles.
                // The host separately validates each file before opening it.
                if entry.file_type().is_ok_and(|kind| kind.is_symlink()) && path.is_dir() {
                    continue;
                }

                // Skip hidden files/dirs
                if name == ".git" || (!show_hidden && name.starts_with('.')) {
                    continue;
                }
                // Skip common non-useful directories
                if path.is_dir()
                    && matches!(
                        name.as_str(),
                        "target" | "node_modules" | "__pycache__" | ".git" | "dist" | "build"
                    )
                {
                    continue;
                }

                if path.is_dir() {
                    dirs.push((name, path));
                } else {
                    files.push((name, path));
                }
            }
        }

        // Sort alphabetically
        dirs.sort_by_key(|a| a.0.to_lowercase());
        files.sort_by_key(|a| a.0.to_lowercase());

        // Directories first
        for (_name, path) in dirs {
            if entries.len() >= MAX_VISIBLE_ENTRIES {
                budget.truncated = true;
                break;
            }
            self.collect_entries(&path, depth + 1, show_hidden, entries, budget);
        }

        // Then files
        let entry_depth = if depth == 0 { 0 } else { depth };
        for (name, path) in files {
            if entries.len() >= MAX_VISIBLE_ENTRIES {
                budget.truncated = true;
                break;
            }
            entries.push(FileEntry {
                path,
                name,
                is_dir: false,
                is_expanded: false,
                depth: entry_depth,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // SDTEST-2498 · SDUC-475
    #[::core::prelude::v1::test]
    fn sdtest_2498_hidden_project_files_remain_separate_from_git_and_build_output() {
        let root = tempfile::tempdir().unwrap();
        for directory in [".config", ".git", "target", "node_modules", "src"] {
            std::fs::create_dir(root.path().join(directory)).unwrap();
        }
        std::fs::write(root.path().join(".env"), "secret").unwrap();
        std::fs::write(root.path().join(".config/config.toml"), "enabled=true").unwrap();
        std::fs::write(root.path().join("src/main.rs"), "fn main() {}").unwrap();
        let mut browser = FileBrowserPanel::new();
        browser.set_root(root.path().into());
        browser.toggle_dir(&root.path().join(".config"));
        browser.toggle_dir(&root.path().join("src"));
        let names = |entries: Vec<FileEntry>| {
            entries
                .into_iter()
                .map(|entry| entry.name)
                .collect::<Vec<_>>()
        };
        assert_eq!(names(browser.visible_entries()), ["src", "main.rs"]);

        assert_eq!(
            names(browser.visible_entries_with_hidden(true)),
            [".config", "config.toml", "src", "main.rs", ".env"]
        );
        assert_eq!(names(browser.visible_entries()), ["src", "main.rs"]);

        let large = root.path().join("large");
        std::fs::create_dir(&large).unwrap();
        for index in 0..=MAX_VISIBLE_ENTRIES {
            std::fs::write(large.join(format!("file-{index:05}.rs")), "").unwrap();
        }
        browser.set_root(large);
        let listing = browser.visible_listing_with_hidden(false);
        assert!(listing.truncated);
        assert_eq!(listing.entries.len(), MAX_VISIBLE_ENTRIES);
        assert_eq!(listing.entries.first().unwrap().name, "file-00000.rs");
    }
}
