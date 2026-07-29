use super::*;

impl FileEditorView {
    // -----------------------------------------------------------------------
    // Find in files
    // -----------------------------------------------------------------------

    pub fn toggle_find_in_files(&mut self) {
        self.fif_visible = !self.fif_visible;
        if self.fif_visible {
            self.search_visible = false;
            self.goto_line_visible = false;
            self.context_menu_visible = false;
        }
    }

    /// Run the find-in-files query over the file-browser root. The actual file
    /// walk runs on a background thread (it does blocking disk I/O) and the
    /// results are applied back on the UI thread, so a large tree can never
    /// freeze or crash the editor.
    pub fn run_find_in_files(&mut self, cx: &mut Context<Self>) {
        self.fif_results.clear();
        self.fif_selected = 0;
        let query = self.fif_query.trim().to_string();
        if query.is_empty() {
            return;
        }
        let case_sensitive = self.search_case_sensitive;
        let needle = if case_sensitive {
            query
        } else {
            query.to_lowercase()
        };
        let root = self.file_browser.root().to_path_buf();
        self.fif_searching = true;

        let search = cx.background_executor().spawn(async move {
            let mut results = Vec::new();
            let mut files_scanned = 0usize;
            FileEditorView::fif_walk(
                &root,
                &needle,
                case_sensitive,
                &mut results,
                &mut files_scanned,
                0,
            );
            results
        });

        cx.spawn(async move |this, cx| {
            let results = search.await;
            let _ = this.update(cx, |this, cx| {
                this.fif_searching = false;
                if this.fif_visible {
                    this.fif_results = results;
                    this.fif_selected = 0;
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn fif_walk(
        dir: &std::path::Path,
        needle: &str,
        case_sensitive: bool,
        out: &mut Vec<FifMatch>,
        files_scanned: &mut usize,
        depth: usize,
    ) {
        const MAX_RESULTS: usize = 1000;
        const MAX_FILES: usize = 8000;
        const MAX_FILE_SIZE: u64 = 1_048_576; // 1 MB
        const MAX_DEPTH: usize = 32;
        if out.len() >= MAX_RESULTS || *files_scanned >= MAX_FILES || depth > MAX_DEPTH {
            return;
        }
        let Ok(read_dir) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in read_dir.flatten() {
            if out.len() >= MAX_RESULTS || *files_scanned >= MAX_FILES {
                return;
            }
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with('.') {
                continue;
            }
            // Use the dir-entry file type (does NOT follow symlinks) so symlink
            // cycles can't cause infinite recursion / a stack overflow.
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if file_type.is_symlink() {
                continue;
            }
            let path = entry.path();
            if file_type.is_dir() {
                if matches!(
                    name.as_str(),
                    "target" | "node_modules" | "__pycache__" | ".git" | "dist" | "build"
                ) {
                    continue;
                }
                Self::fif_walk(&path, needle, case_sensitive, out, files_scanned, depth + 1);
            } else if file_type.is_file() {
                if let Ok(meta) = entry.metadata() {
                    if meta.len() > MAX_FILE_SIZE {
                        continue;
                    }
                }
                *files_scanned += 1;
                let Ok(content) = std::fs::read_to_string(&path) else {
                    continue; // non-UTF-8 / unreadable → skip (binary)
                };
                for (li, line) in content.lines().enumerate() {
                    let matched = if case_sensitive {
                        line.contains(needle)
                    } else {
                        line.to_lowercase().contains(needle)
                    };
                    if matched {
                        out.push(FifMatch {
                            path: path.clone(),
                            line: li + 1,
                            preview: line.trim_start().chars().take(160).collect(),
                        });
                        if out.len() >= MAX_RESULTS {
                            return;
                        }
                    }
                }
            }
        }
    }

    /// Open the file for the given result and jump to its line.
    pub fn open_fif_result(&mut self, idx: usize, cx: &mut Context<Self>) {
        let Some(m) = self.fif_results.get(idx).cloned() else {
            return;
        };
        self.open_file(m.path, cx);
        if let Some(tab) = self.active_tab_mut() {
            if let Some(buf) = tab.buffer_mut() {
                buf.goto_line(m.line);
            }
        }
        self.ensure_cursor_visible();
        self.fif_visible = false;
        cx.notify();
    }
}
