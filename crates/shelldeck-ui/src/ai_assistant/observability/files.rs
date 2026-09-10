//! Fichiers: the paths an Agent session touched, as a collapsible tree.

use std::collections::{BTreeMap, HashSet};

use gpui::prelude::*;
use gpui::*;
use shelldeck_core::agent_session::AgentSession;
use shelldeck_core::git::GitWorkingTree;

use super::super::AiAssistantView;
use super::{empty_line, observed_files, ObservedFile, MONO};
use crate::icons::lucide_icon;
use crate::scale::px;
use crate::t;
use crate::theme::ShellDeckColors;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum FileAccess {
    Read,
    Modified,
    Created,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum TreeRowKind {
    Directory {
        collapsed: bool,
    },
    File {
        access: FileAccess,
        additions: u32,
        deletions: u32,
        at_ms: i64,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct TreeRow {
    /// Directory path, or file path relative to the working directory.
    pub key: String,
    pub name: String,
    pub depth: usize,
    pub kind: TreeRowKind,
}

/// Path of an observed file relative to the session's working directory.
/// Paths outside it are kept whole instead of being guessed at.
pub(super) fn relative_to_workdir(path: &str, workdir: &str) -> String {
    let base = workdir.trim_end_matches(['/', '\\']);
    path.strip_prefix(base)
        .and_then(|rest| rest.strip_prefix(['/', '\\']))
        .filter(|rest| !rest.is_empty())
        .unwrap_or(path)
        .to_string()
}

#[derive(Default)]
struct TreeNode {
    directories: BTreeMap<String, TreeNode>,
    files: BTreeMap<String, (String, FileAccess, u32, u32, i64)>,
}

/// Display-ordered rows: directories before files at each level, collapsed
/// directories hiding their descendants. Git's own view decides "Créé": a path
/// the repository does not know yet is new even when the trace only shows a
/// write, and a read-only trace never invents a change.
pub(super) fn build_tree_rows(
    files: &BTreeMap<String, ObservedFile>,
    workdir: &str,
    git: Option<&GitWorkingTree>,
    collapsed: &HashSet<String>,
) -> Vec<TreeRow> {
    let (new_paths, changed_paths): (HashSet<&str>, HashSet<&str>) = git
        .map(|tree| {
            let new = tree
                .files
                .iter()
                .filter(|file| file.is_new())
                .map(|file| file.path.as_str())
                .collect();
            let changed = tree
                .files
                .iter()
                .filter(|file| !file.is_new())
                .map(|file| file.path.as_str())
                .collect();
            (new, changed)
        })
        .unwrap_or_default();

    let mut root = TreeNode::default();
    for (path, file) in files {
        let relative = relative_to_workdir(path, workdir);
        let changed =
            file.additions > 0 || file.deletions > 0 || changed_paths.contains(relative.as_str());
        let access = if new_paths.contains(relative.as_str()) {
            FileAccess::Created
        } else if changed || !file.read {
            // A diff with no line counts (a rename, a mode change) is still a
            // write, never a read.
            FileAccess::Modified
        } else {
            FileAccess::Read
        };
        let parts: Vec<&str> = relative
            .split(['/', '\\'])
            .filter(|part| !part.is_empty())
            .collect();
        let Some((name, directories)) = parts.split_last() else {
            continue;
        };
        let mut node = &mut root;
        for directory in directories {
            node = node
                .directories
                .entry((*directory).to_string())
                .or_default();
        }
        node.files.insert(
            (*name).to_string(),
            (
                relative.clone(),
                access,
                file.additions,
                file.deletions,
                file.at_ms,
            ),
        );
    }

    fn walk(
        node: &TreeNode,
        prefix: &str,
        depth: usize,
        collapsed: &HashSet<String>,
        rows: &mut Vec<TreeRow>,
    ) {
        for (name, child) in &node.directories {
            let key = if prefix.is_empty() {
                name.clone()
            } else {
                format!("{prefix}/{name}")
            };
            let is_collapsed = collapsed.contains(&key);
            rows.push(TreeRow {
                key: key.clone(),
                name: name.clone(),
                depth,
                kind: TreeRowKind::Directory {
                    collapsed: is_collapsed,
                },
            });
            if !is_collapsed {
                walk(child, &key, depth + 1, collapsed, rows);
            }
        }
        for (name, (key, access, additions, deletions, at_ms)) in &node.files {
            rows.push(TreeRow {
                key: key.clone(),
                name: name.clone(),
                depth,
                kind: TreeRowKind::File {
                    access: *access,
                    additions: *additions,
                    deletions: *deletions,
                    at_ms: *at_ms,
                },
            });
        }
    }

    let mut rows = Vec::new();
    walk(&root, "", 0, collapsed, &mut rows);
    rows
}

fn access_badge(access: FileAccess) -> (&'static str, Hsla) {
    match access {
        FileAccess::Read => ("L", ShellDeckColors::primary()),
        FileAccess::Modified => ("M", ShellDeckColors::warning()),
        FileAccess::Created => ("A", ShellDeckColors::success()),
    }
}

impl AiAssistantView {
    pub(super) fn render_files(
        &self,
        session: &AgentSession,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let files = observed_files(session);
        let rows = build_tree_rows(
            &files,
            &session.context.workdir,
            self.agent_git.tree_for(&session.context.workdir),
            &self.agent_files_collapsed,
        );
        let legend = [
            (FileAccess::Read, "ai.observability.files_read"),
            (FileAccess::Modified, "ai.observability.files_modified"),
            (FileAccess::Created, "ai.observability.files_created"),
        ];
        let mut legend_row = div()
            .flex()
            .flex_wrap()
            .gap(px(10.0))
            .px(px(6.0))
            .pb(px(6.0))
            .text_size(px(9.0))
            .text_color(ShellDeckColors::text_muted());
        for (access, key) in legend {
            let (_, color) = access_badge(access);
            legend_row = legend_row.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(4.0))
                    .child(div().size(px(6.0)).rounded(px(2.0)).bg(color))
                    .child(t!(key).to_string()),
            );
        }
        let mut root = div().flex().flex_col().gap(px(1.0)).child(legend_row);
        if rows.is_empty() {
            return root
                .child(empty_line(t!("ai.observability.empty_files").to_string()))
                .into_any_element();
        }
        for row in rows {
            root = root.child(self.render_tree_row(row, cx));
        }
        root.into_any_element()
    }

    fn render_tree_row(&self, row: TreeRow, cx: &mut Context<Self>) -> AnyElement {
        let indent = 6.0 + row.depth as f32 * 12.0;
        let base = div()
            .id(SharedString::from(format!("ai-file-{}", row.key)))
            .flex()
            .items_center()
            .gap(px(6.0))
            .min_w(px(0.0))
            .h(px(25.0))
            .pl(px(indent))
            .pr(px(6.0))
            .rounded(px(5.0))
            .hover(|style| style.bg(ShellDeckColors::hover_bg()))
            .text_size(px(10.0));
        match row.kind {
            TreeRowKind::Directory { collapsed } => {
                let key = row.key;
                base.cursor_pointer()
                    .child(lucide_icon(
                        if collapsed {
                            "chevron-right"
                        } else {
                            "chevron-down"
                        },
                        11.0,
                        ShellDeckColors::text_muted(),
                    ))
                    .child(lucide_icon(
                        if collapsed { "folder" } else { "folder-open" },
                        12.0,
                        ShellDeckColors::text_muted(),
                    ))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .truncate()
                            .text_color(ShellDeckColors::text_primary())
                            .child(row.name),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if !this.agent_files_collapsed.remove(&key) {
                            this.agent_files_collapsed.insert(key.clone());
                        }
                        cx.notify();
                    }))
                    .into_any_element()
            }
            TreeRowKind::File {
                access,
                additions,
                deletions,
                at_ms,
            } => {
                let (letter, color) = access_badge(access);
                let key = row.key;
                let mut element = base
                    .child(div().w(px(11.0)).flex_shrink_0())
                    .child(lucide_icon(
                        "file-text",
                        11.0,
                        ShellDeckColors::text_muted(),
                    ))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .truncate()
                            .font_family(MONO)
                            .text_color(ShellDeckColors::text_primary())
                            .child(row.name),
                    )
                    .when(at_ms > 0, |element| {
                        element.child(
                            div()
                                .flex_shrink_0()
                                .font_family(MONO)
                                .text_size(px(8.5))
                                .text_color(ShellDeckColors::text_muted())
                                .child(crate::i18n::rel_time(at_ms as f64)),
                        )
                    })
                    .when(additions > 0 || deletions > 0, |element| {
                        element
                            .child(
                                div()
                                    .flex_shrink_0()
                                    .font_family(MONO)
                                    .text_size(px(9.0))
                                    .text_color(ShellDeckColors::success())
                                    .child(format!("+{additions}")),
                            )
                            .child(
                                div()
                                    .flex_shrink_0()
                                    .font_family(MONO)
                                    .text_size(px(9.0))
                                    .text_color(ShellDeckColors::error())
                                    .child(format!("−{deletions}")),
                            )
                    })
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_center()
                            .flex_shrink_0()
                            .size(px(16.0))
                            .rounded(px(4.0))
                            .bg(color.opacity(0.12))
                            .font_family(MONO)
                            .text_size(px(9.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(color)
                            .child(letter),
                    );
                if access != FileAccess::Read {
                    element =
                        element
                            .cursor_pointer()
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.show_git_diff_for(&key, cx);
                            }));
                }
                element.into_any_element()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::ObservedFile;
    use super::{build_tree_rows, relative_to_workdir, FileAccess, TreeRowKind};
    use shelldeck_core::git::{GitChange, GitFileEntry, GitWorkingTree};
    use std::collections::{BTreeMap, HashSet};

    fn describe(rows: &[super::TreeRow]) -> Vec<(String, usize, String)> {
        rows.iter()
            .map(|row| {
                let kind = match &row.kind {
                    TreeRowKind::Directory { .. } => "dir".to_string(),
                    TreeRowKind::File { access, .. } => format!("{access:?}"),
                };
                (row.name.clone(), row.depth, kind)
            })
            .collect()
    }

    // SDTEST-1928
    #[test]
    fn sdtest_1928_file_tree_nests_directories_and_marks_read_modified_created() {
        let mut files = BTreeMap::new();
        files.insert(
            "/work/app/src/main.rs".to_string(),
            ObservedFile {
                read: true,
                ..Default::default()
            },
        );
        files.insert(
            "src/lib.rs".to_string(),
            ObservedFile {
                additions: 3,
                deletions: 1,
                ..Default::default()
            },
        );
        files.insert(
            "src/new.rs".to_string(),
            ObservedFile {
                additions: 9,
                ..Default::default()
            },
        );
        files.insert(
            "README.md".to_string(),
            ObservedFile {
                read: true,
                ..Default::default()
            },
        );
        let git = GitWorkingTree {
            files: vec![GitFileEntry {
                path: "src/new.rs".to_string(),
                original_path: None,
                index: GitChange::Untracked,
                worktree: GitChange::Untracked,
            }],
            ..Default::default()
        };

        let rows = build_tree_rows(&files, "/work/app", Some(&git), &HashSet::new());
        assert_eq!(
            describe(&rows),
            vec![
                ("src".to_string(), 0, "dir".to_string()),
                (
                    "lib.rs".to_string(),
                    1,
                    format!("{:?}", FileAccess::Modified)
                ),
                ("main.rs".to_string(), 1, format!("{:?}", FileAccess::Read)),
                (
                    "new.rs".to_string(),
                    1,
                    format!("{:?}", FileAccess::Created)
                ),
                (
                    "README.md".to_string(),
                    0,
                    format!("{:?}", FileAccess::Read)
                ),
            ]
        );

        let collapsed: HashSet<String> = ["src".to_string()].into();
        let rows = build_tree_rows(&files, "/work/app/", Some(&git), &collapsed);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].kind, TreeRowKind::Directory { collapsed: true });

        assert_eq!(
            relative_to_workdir("/elsewhere/x.rs", "/work/app"),
            "/elsewhere/x.rs"
        );
    }
}
