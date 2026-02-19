use std::collections::HashSet;
use std::path::{Path, PathBuf};

const MAX_DEPTH: usize = 20;

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

    pub fn set_root(&mut self, root: PathBuf) {
        self.expanded_dirs.clear();
        self.expanded_dirs.insert(root.clone());
        self.root = root;
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
        let mut entries = Vec::new();
        self.collect_entries(&self.root, 0, &mut entries);
        entries
    }

    fn collect_entries(&self, dir: &Path, depth: usize, entries: &mut Vec<FileEntry>) {
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

        // Read directory entries
        let mut dirs = Vec::new();
        let mut files = Vec::new();

        if let Ok(read_dir) = std::fs::read_dir(dir) {
            for entry in read_dir.flatten() {
                let path = entry.path();
                let name = entry
                    .file_name()
                    .to_str()
                    .unwrap_or("?")
                    .to_string();

                // Skip hidden files/dirs
                if name.starts_with('.') {
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
        dirs.sort_by(|a, b| a.0.to_lowercase().cmp(&b.0.to_lowercase()));
        files.sort_by(|a, b| a.0.to_lowercase().cmp(&b.0.to_lowercase()));

        // Directories first
        for (_name, path) in dirs {
            self.collect_entries(&path, depth + 1, entries);
        }

        // Then files
        let entry_depth = if depth == 0 { 0 } else { depth };
        for (name, path) in files {
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
