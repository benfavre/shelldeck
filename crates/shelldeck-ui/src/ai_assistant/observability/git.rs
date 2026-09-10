//! Local Git control for the observed session's working directory.
//!
//! Staging is a direct, reversible index change started from a file row. A
//! commit needs its own confirmation showing the message and the staged files,
//! and nothing in the Assistant ever pushes (SDUC-499).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use adabraka_ui::components::checkbox::Checkbox;
use adabraka_ui::components::confirm_dialog::Dialog as UiDialog;
use adabraka_ui::components::icon_source::IconSource;
use adabraka_ui::components::input::{Input, InputSize};
use adabraka_ui::components::input_state::InputState;
use adabraka_ui::prelude::{
    Button, ButtonSize, ButtonVariant, Spinner, SpinnerSize, SpinnerVariant,
};
use gpui::prelude::*;
use gpui::*;
use shelldeck_core::agent_runtime::AgentTarget;
use shelldeck_core::agent_session::AgentSession;
use shelldeck_core::git::{self as core_git, GitCommandError, GitFileEntry, GitWorkingTree};

use super::super::{AiActivity, AiAssistantView};
use super::{empty_line, MONO};
use crate::icons::lucide_icon;
use crate::scale::px;
use crate::t;
use crate::theme::ShellDeckColors;

/// Minimum spacing between two automatic reads of the same working tree.
const REFRESH_INTERVAL: Duration = Duration::from_secs(2);
/// Untracked files whose lines are counted on each read.
const MAX_UNTRACKED_COUNTED: usize = 200;
/// Diff lines painted for one file; the core already bounds the bytes.
const MAX_DIFF_LINES: usize = 400;
/// Staged paths listed in the commit confirmation before an ellipsis.
const MAX_COMMIT_PATHS_LISTED: usize = 8;

#[derive(Debug, Clone, PartialEq, Eq)]
struct GitSelection {
    path: String,
    staged: bool,
    untracked: bool,
}

#[derive(Default)]
struct GitSnapshot {
    tree: Option<GitWorkingTree>,
    staged: BTreeMap<String, (u32, u32)>,
    unstaged: BTreeMap<String, (u32, u32)>,
}

/// Git panel state for the observed session. The snapshot always belongs to
/// `workdir`; switching sessions resets everything else with it.
pub(in crate::ai_assistant) struct AgentGitPanel {
    workdir: Option<String>,
    snapshot: GitSnapshot,
    loaded: bool,
    last_refresh: Option<Instant>,
    busy: bool,
    error: Option<String>,
    notice: Option<String>,
    selection: Option<GitSelection>,
    diff: Option<(GitSelection, String)>,
    commit_open: bool,
    commit_message: Entity<InputState>,
    /// The commit message draft in flight, if any. A result for any other
    /// request, or after the dialog closed, is discarded.
    commit_draft_request: Option<u64>,
    commit_draft_seq: u64,
}

impl AgentGitPanel {
    pub(in crate::ai_assistant) fn new(cx: &mut App) -> Self {
        Self {
            workdir: None,
            snapshot: GitSnapshot::default(),
            loaded: false,
            last_refresh: None,
            busy: false,
            error: None,
            notice: None,
            selection: None,
            diff: None,
            commit_open: false,
            commit_message: commit_message_state(cx),
            commit_draft_request: None,
            commit_draft_seq: 0,
        }
    }

    fn reset_for(&mut self, workdir: Option<String>) {
        self.workdir = workdir;
        self.snapshot = GitSnapshot::default();
        self.loaded = false;
        self.last_refresh = None;
        self.error = None;
        self.notice = None;
        self.selection = None;
        self.diff = None;
        self.commit_open = false;
        self.commit_draft_request = None;
    }

    /// Working tree of `workdir`, when the last read targeted that directory.
    pub(super) fn tree_for(&self, workdir: &str) -> Option<&GitWorkingTree> {
        if self.workdir.as_deref() == Some(workdir) {
            self.snapshot.tree.as_ref()
        } else {
            None
        }
    }

    /// Paths with any staged, unstaged or conflicting change.
    pub(super) fn changed_count(&self) -> usize {
        self.snapshot
            .tree
            .as_ref()
            .map_or(0, |tree| tree.files.len())
    }

    /// Added and deleted lines across the index and the working tree.
    pub(super) fn line_totals(&self) -> (u32, u32) {
        self.snapshot
            .staged
            .values()
            .chain(self.snapshot.unstaged.values())
            .fold((0, 0), |(added, deleted), (add, del)| {
                (added.saturating_add(*add), deleted.saturating_add(*del))
            })
    }

    fn staged_files(&self) -> Vec<&GitFileEntry> {
        self.snapshot
            .tree
            .as_ref()
            .map(|tree| {
                tree.files
                    .iter()
                    .filter(|file| file.has_staged_change())
                    .collect()
            })
            .unwrap_or_default()
    }
}

enum GitOperation {
    Stage(Vec<String>),
    Unstage(Vec<String>),
    StageAll,
    Commit(String),
}

impl GitOperation {
    fn run(self, dir: &Path) -> Result<Option<String>, GitCommandError> {
        match self {
            Self::Stage(paths) => core_git::stage_paths(dir, &paths).map(|()| None),
            Self::Unstage(paths) => core_git::unstage_paths(dir, &paths).map(|()| None),
            Self::StageAll => core_git::stage_all(dir).map(|()| None),
            Self::Commit(message) => core_git::commit_staged(dir, &message).map(Some),
        }
    }
}

fn read_snapshot(dir: &Path) -> GitSnapshot {
    let Some(tree) = core_git::read_working_tree(dir) else {
        return GitSnapshot::default();
    };
    let staged = core_git::diff_line_counts(dir, true);
    let mut unstaged = core_git::diff_line_counts(dir, false);
    for file in tree
        .files
        .iter()
        .filter(|file| file.is_untracked())
        .take(MAX_UNTRACKED_COUNTED)
    {
        if let Some(lines) = core_git::untracked_line_count(dir, &file.path) {
            unstaged.insert(file.path.clone(), (lines, 0));
        }
    }
    GitSnapshot {
        tree: Some(tree),
        staged,
        unstaged,
    }
}

fn split_path(path: &str) -> (&str, &str) {
    match path.rfind(['/', '\\']) {
        Some(index) => (&path[..index], &path[index + 1..]),
        None => ("", path),
    }
}

fn letter_color(letter: &str) -> Hsla {
    match letter {
        "A" => ShellDeckColors::success(),
        "D" | "U" => ShellDeckColors::error(),
        "M" => ShellDeckColors::warning(),
        _ => ShellDeckColors::primary(),
    }
}

impl AiAssistantView {
    /// Throttled read of the observed session's working tree.
    pub(in crate::ai_assistant) fn refresh_agent_git(&mut self, cx: &mut Context<Self>) {
        self.reload_agent_git(false, cx);
    }

    fn reload_agent_git(&mut self, force: bool, cx: &mut Context<Self>) {
        let local_workdir = self.observed_session(cx).and_then(|session| {
            matches!(session.context.target, AgentTarget::Local).then_some(session.context.workdir)
        });
        let Some(workdir) = local_workdir else {
            // An SSH target or no session never shows local Git data.
            if self.agent_git.workdir.is_some() {
                self.agent_git.reset_for(None);
            }
            return;
        };
        if self.agent_git.workdir.as_deref() != Some(workdir.as_str()) {
            self.agent_git.reset_for(Some(workdir.clone()));
        } else if !force
            && self
                .agent_git
                .last_refresh
                .is_some_and(|at| at.elapsed() < REFRESH_INTERVAL)
        {
            return;
        }
        self.agent_git.last_refresh = Some(Instant::now());
        let selection = self.agent_git.selection.clone();
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let dir = PathBuf::from(&workdir);
            let wanted = selection.clone();
            let (snapshot, diff) = cx
                .background_executor()
                .spawn(async move {
                    let snapshot = read_snapshot(&dir);
                    let diff = wanted.and_then(|selection| {
                        core_git::file_diff(
                            &dir,
                            &selection.path,
                            selection.staged,
                            selection.untracked,
                        )
                        .ok()
                        .map(|text| (selection, text))
                    });
                    (snapshot, diff)
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.agent_git.workdir.as_deref() != Some(workdir.as_str()) {
                    return;
                }
                this.agent_git.snapshot = snapshot;
                this.agent_git.loaded = true;
                let still_listed = this.agent_git.selection.as_ref().is_some_and(|selected| {
                    this.agent_git.snapshot.tree.as_ref().is_some_and(|tree| {
                        tree.files.iter().any(|file| {
                            file.path == selected.path
                                && if selected.staged {
                                    file.has_staged_change()
                                } else {
                                    file.has_unstaged_change()
                                }
                        })
                    })
                });
                if !still_listed {
                    this.agent_git.selection = None;
                    this.agent_git.diff = None;
                } else if this.agent_git.selection == selection {
                    this.agent_git.diff = diff;
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn run_git_operation(&mut self, operation: GitOperation, cx: &mut Context<Self>) {
        let Some(workdir) = self.agent_git.workdir.clone() else {
            return;
        };
        if self.agent_git.busy {
            return;
        }
        self.agent_git.busy = true;
        self.agent_git.error = None;
        self.agent_git.notice = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let dir = PathBuf::from(&workdir);
            let result = cx
                .background_executor()
                .spawn(async move { operation.run(&dir) })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.agent_git.busy = false;
                match result {
                    Ok(Some(sha)) => {
                        this.agent_git.notice =
                            Some(t!("ai.observability.git_commit_done", sha = sha).to_string());
                        this.agent_git.commit_open = false;
                        this.agent_git.commit_draft_request = None;
                        this.agent_git.commit_message = commit_message_state(cx);
                    }
                    Ok(None) => {}
                    // A failed commit keeps its dialog and message open.
                    Err(error) => this.agent_git.error = Some(error.to_string()),
                }
                this.reload_agent_git(true, cx);
                cx.notify();
            });
        })
        .detach();
    }

    fn select_git_file(&mut self, selection: GitSelection, cx: &mut Context<Self>) {
        let Some(workdir) = self.agent_git.workdir.clone() else {
            return;
        };
        self.agent_git.selection = Some(selection.clone());
        self.agent_git.diff = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let dir = PathBuf::from(&workdir);
            let wanted = selection.clone();
            let result = cx
                .background_executor()
                .spawn(async move {
                    core_git::file_diff(&dir, &wanted.path, wanted.staged, wanted.untracked)
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.agent_git.selection.as_ref() != Some(&selection) {
                    return;
                }
                match result {
                    Ok(text) => this.agent_git.diff = Some((selection, text)),
                    Err(error) => this.agent_git.error = Some(error.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Open the Git panel on a file chosen from the Fichiers tree.
    pub(super) fn show_git_diff_for(&mut self, path: &str, cx: &mut Context<Self>) {
        let entry = self
            .agent_git
            .snapshot
            .tree
            .as_ref()
            .and_then(|tree| tree.files.iter().find(|file| file.path == path))
            .cloned();
        self.active_tab = AiActivity::AgentGit;
        self.sync_loading();
        match entry {
            Some(entry) => {
                let staged = entry.has_staged_change() && !entry.has_unstaged_change();
                self.select_git_file(
                    GitSelection {
                        path: entry.path,
                        staged,
                        untracked: entry.index == core_git::GitChange::Untracked,
                    },
                    cx,
                );
            }
            None => {
                self.reload_agent_git(true, cx);
                cx.notify();
            }
        }
    }

    fn confirm_git_commit(&mut self, cx: &mut Context<Self>) {
        let message = self
            .agent_git
            .commit_message
            .read(cx)
            .content()
            .trim()
            .to_string();
        if message.is_empty() {
            self.agent_git.error = Some(t!("ai.observability.git_commit_empty").to_string());
            cx.notify();
            return;
        }
        // Revalidate right before committing: the observed session may have
        // switched target, started running again or lost its staged files.
        let still_committable = self.observed_session(cx).is_some_and(|session| {
            matches!(session.context.target, AgentTarget::Local)
                && !session.status.is_active()
                && self.agent_git.workdir.as_deref() == Some(session.context.workdir.as_str())
        }) && !self.agent_git.staged_files().is_empty();
        if !still_committable {
            self.agent_git.error = Some(t!("ai.observability.git_commit_blocked").to_string());
            cx.notify();
            return;
        }
        self.run_git_operation(GitOperation::Commit(message), cx);
    }

    pub(super) fn render_git(&self, session: &AgentSession, cx: &mut Context<Self>) -> AnyElement {
        if !matches!(session.context.target, AgentTarget::Local) {
            return empty_line(t!("ai.observability.git_remote").to_string());
        }
        if !self.agent_git.loaded {
            return div()
                .flex()
                .justify_center()
                .py(px(18.0))
                .child(
                    Spinner::new()
                        .size(SpinnerSize::Sm)
                        .variant(SpinnerVariant::Primary),
                )
                .into_any_element();
        }
        let Some(tree) = self.agent_git.tree_for(&session.context.workdir) else {
            return empty_line(t!("ai.observability.git_unknown").to_string());
        };
        let staged: Vec<&GitFileEntry> = tree
            .files
            .iter()
            .filter(|file| file.has_staged_change())
            .collect();
        let changes: Vec<&GitFileEntry> = tree
            .files
            .iter()
            .filter(|file| file.has_unstaged_change() || file.is_conflicted())
            .collect();
        let busy = self.agent_git.busy;
        let session_active = session.status.is_active();

        let mut root = div()
            .flex()
            .flex_col()
            .gap(px(10.0))
            .child(self.render_git_branch_card(tree))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .child(
                        Button::new(
                            "ai-git-stage-all",
                            t!("ai.observability.git_stage_all").to_string(),
                        )
                        .variant(ButtonVariant::Secondary)
                        .size(ButtonSize::Sm)
                        .icon(IconSource::from("plus"))
                        .disabled(busy || !changes.iter().any(|file| !file.is_conflicted()))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.run_git_operation(GitOperation::StageAll, cx);
                        })),
                    )
                    .child(
                        Button::new(
                            "ai-git-refresh",
                            t!("ai.observability.git_refresh").to_string(),
                        )
                        .variant(ButtonVariant::Ghost)
                        .size(ButtonSize::Sm)
                        .icon(IconSource::from("refresh-cw"))
                        .disabled(busy)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.reload_agent_git(true, cx);
                        })),
                    )
                    .when(busy, |row| {
                        row.child(
                            Spinner::new()
                                .size(SpinnerSize::Xs)
                                .variant(SpinnerVariant::Primary),
                        )
                    }),
            );
        if let Some(error) = self.agent_git.error.clone() {
            root = root.child(status_line("circle-alert", ShellDeckColors::error(), error));
        }
        if let Some(notice) = self.agent_git.notice.clone() {
            root = root.child(status_line(
                "circle-check",
                ShellDeckColors::success(),
                notice,
            ));
        }

        if tree.files.is_empty() {
            root = root.child(empty_line(t!("ai.observability.git_clean").to_string()));
        } else {
            if !staged.is_empty() {
                root = root.child(section_label(
                    t!("ai.observability.git_staged").to_string(),
                    staged.len(),
                ));
                for entry in &staged {
                    root = root.child(self.render_git_file_row(entry, true, busy, cx));
                }
            }
            if !changes.is_empty() {
                root = root.child(section_label(
                    t!("ai.observability.git_changes").to_string(),
                    changes.len(),
                ));
                for entry in &changes {
                    root = root.child(self.render_git_file_row(entry, false, busy, cx));
                }
            }
            root = match self.render_git_diff() {
                Some(diff) => root.child(diff),
                None => root.child(empty_line(
                    t!("ai.observability.git_select_hint").to_string(),
                )),
            };
        }

        let can_commit = !staged.is_empty() && !busy && !session_active;
        root.child(
            div()
                .flex()
                .flex_col()
                .gap(px(7.0))
                .p(px(9.0))
                .rounded(px(8.0))
                .bg(ShellDeckColors::warning().opacity(0.10))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.0))
                        .text_size(px(10.5))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(lucide_icon(
                            "shield-check",
                            12.0,
                            ShellDeckColors::warning(),
                        ))
                        .child(t!("ai.observability.git_commit_heading").to_string()),
                )
                .child(
                    div()
                        .text_size(px(10.0))
                        .text_color(ShellDeckColors::text_muted())
                        .child(if session_active {
                            t!("ai.observability.git_commit_running_agent").to_string()
                        } else {
                            t!("ai.observability.git_no_push").to_string()
                        }),
                )
                .child(
                    div().flex().child(
                        Button::new(
                            "ai-git-prepare-commit",
                            t!("ai.observability.git_prepare_commit").to_string(),
                        )
                        .variant(ButtonVariant::Default)
                        .size(ButtonSize::Sm)
                        .icon(IconSource::from("git-commit-horizontal"))
                        .disabled(!can_commit)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.agent_git.commit_open = true;
                            this.agent_git.error = None;
                            cx.notify();
                        })),
                    ),
                ),
        )
        .into_any_element()
    }

    fn render_git_branch_card(&self, tree: &GitWorkingTree) -> AnyElement {
        let (additions, deletions) = self.agent_git.line_totals();
        let branch = tree
            .branch
            .clone()
            .unwrap_or_else(|| t!("ai.observability.git_unknown").to_string());
        div()
            .flex()
            .flex_col()
            .gap(px(6.0))
            .p(px(10.0))
            .rounded(px(8.0))
            .border_1()
            .border_color(ShellDeckColors::border())
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .min_w(px(0.0))
                    .font_family(MONO)
                    .text_size(px(10.5))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(lucide_icon(
                        "git-branch",
                        12.0,
                        ShellDeckColors::text_muted(),
                    ))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .truncate()
                            .text_color(ShellDeckColors::text_primary())
                            .child(branch),
                    )
                    .children(tree.upstream.as_ref().map(|_| {
                        div()
                            .flex_shrink_0()
                            .font_weight(FontWeight::NORMAL)
                            .text_size(px(9.5))
                            .text_color(ShellDeckColors::text_muted())
                            .child(format!("↑ {} ↓ {}", tree.ahead, tree.behind))
                    })),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap(px(8.0))
                    .text_size(px(9.5))
                    .text_color(ShellDeckColors::text_muted())
                    .child(
                        t!("ai.observability.git_file_count", count = tree.files.len()).to_string(),
                    )
                    .child(
                        div()
                            .text_color(ShellDeckColors::success())
                            .child(format!("+{additions}")),
                    )
                    .child(
                        div()
                            .text_color(ShellDeckColors::error())
                            .child(format!("−{deletions}")),
                    )
                    .children(
                        tree.upstream
                            .clone()
                            .map(|upstream| div().min_w(px(0.0)).truncate().child(upstream)),
                    ),
            )
            .into_any_element()
    }

    fn render_git_file_row(
        &self,
        entry: &GitFileEntry,
        staged: bool,
        busy: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (directory, name) = split_path(&entry.path);
        let letter = entry.letter(staged);
        let counts = if staged {
            self.agent_git.snapshot.staged.get(&entry.path)
        } else {
            self.agent_git.snapshot.unstaged.get(&entry.path)
        }
        .copied();
        let selection = GitSelection {
            path: entry.path.clone(),
            staged,
            untracked: entry.is_untracked(),
        };
        let selected = self.agent_git.selection.as_ref() == Some(&selection);
        let mut paths = vec![entry.path.clone()];
        if staged {
            // Unstaging a rename restores both sides of it.
            paths.extend(entry.original_path.clone());
        }
        let view = cx.entity();
        let checkbox = Checkbox::new(SharedString::from(format!(
            "ai-git-check-{}-{}",
            u8::from(staged),
            entry.path
        )))
        .checked(staged)
        .disabled(busy || entry.is_conflicted())
        .on_click(move |_, _, cx| {
            let operation = if staged {
                GitOperation::Unstage(paths.clone())
            } else {
                GitOperation::Stage(paths.clone())
            };
            view.update(cx, |this, cx| this.run_git_operation(operation, cx));
        });

        div()
            .flex()
            .items_center()
            .gap(px(7.0))
            .min_w(px(0.0))
            .px(px(6.0))
            .py(px(4.0))
            .rounded(px(5.0))
            .when(selected, |row| row.bg(ShellDeckColors::selected_bg()))
            .child(div().flex_shrink_0().child(checkbox))
            .child(
                div()
                    .id(SharedString::from(format!(
                        "ai-git-file-{}-{}",
                        u8::from(staged),
                        entry.path
                    )))
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w(px(0.0))
                    .cursor_pointer()
                    .child(
                        div()
                            .truncate()
                            .font_family(MONO)
                            .text_size(px(10.0))
                            .text_color(ShellDeckColors::text_primary())
                            .child(name.to_string()),
                    )
                    .when(!directory.is_empty(), |column| {
                        column.child(
                            div()
                                .truncate()
                                .font_family(MONO)
                                .text_size(px(8.5))
                                .text_color(ShellDeckColors::text_muted())
                                .child(directory.to_string()),
                        )
                    })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.select_git_file(selection.clone(), cx);
                    })),
            )
            .children(counts.map(|(additions, deletions)| {
                div()
                    .flex()
                    .flex_shrink_0()
                    .gap(px(4.0))
                    .font_family(MONO)
                    .text_size(px(9.0))
                    .child(
                        div()
                            .text_color(ShellDeckColors::success())
                            .child(format!("+{additions}")),
                    )
                    .child(
                        div()
                            .text_color(ShellDeckColors::error())
                            .child(format!("−{deletions}")),
                    )
            }))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .flex_shrink_0()
                    .size(px(16.0))
                    .rounded(px(4.0))
                    .bg(letter_color(letter).opacity(0.12))
                    .font_family(MONO)
                    .text_size(px(9.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(letter_color(letter))
                    .child(letter),
            )
            .into_any_element()
    }

    fn render_git_diff(&self) -> Option<AnyElement> {
        let (selection, text) = self.agent_git.diff.as_ref()?;
        let side = if selection.staged {
            t!("ai.observability.git_diff_staged")
        } else {
            t!("ai.observability.git_diff_unstaged")
        };
        let mut lines = div()
            .flex()
            .flex_col()
            .py(px(4.0))
            .font_family(MONO)
            .text_size(px(9.0));
        for line in text.lines().take(MAX_DIFF_LINES) {
            let (color, background) = if line.starts_with("+++") || line.starts_with("---") {
                (ShellDeckColors::text_muted(), gpui::transparent_black())
            } else if line.starts_with("@@") {
                (
                    ShellDeckColors::primary(),
                    ShellDeckColors::primary().opacity(0.06),
                )
            } else if line.starts_with('+') {
                (
                    ShellDeckColors::success(),
                    ShellDeckColors::success().opacity(0.08),
                )
            } else if line.starts_with('-') {
                (
                    ShellDeckColors::error(),
                    ShellDeckColors::error().opacity(0.08),
                )
            } else {
                (ShellDeckColors::text_muted(), gpui::transparent_black())
            };
            // A row gives the text a definite, shrinkable width. A bare
            // `truncate()` line in a column is shaped at min-content and
            // collapses to a lone ellipsis.
            lines = lines.child(
                div()
                    .flex()
                    .w_full()
                    .min_w(px(0.0))
                    .px(px(8.0))
                    .bg(background)
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .truncate()
                            .text_color(color)
                            .child(if line.is_empty() {
                                " ".to_string()
                            } else {
                                line.to_string()
                            }),
                    ),
            );
        }
        Some(
            div()
                .flex()
                .flex_col()
                .rounded(px(8.0))
                .border_1()
                .border_color(ShellDeckColors::border())
                .bg(ShellDeckColors::bg_primary())
                .overflow_hidden()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.0))
                        .min_w(px(0.0))
                        .px(px(8.0))
                        .h(px(28.0))
                        .border_b_1()
                        .border_color(ShellDeckColors::border())
                        .text_size(px(9.5))
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
                                .child(selection.path.clone()),
                        )
                        .child(
                            div()
                                .flex_shrink_0()
                                .text_color(ShellDeckColors::text_muted())
                                .child(side.to_string()),
                        ),
                )
                .child(lines)
                .into_any_element(),
        )
    }

    /// Explicit request for a commit message draft. The staged patch and the
    /// latest subjects are read off the UI thread, then handed to the host,
    /// which owns the AI configuration.
    fn start_commit_message_draft(&mut self, cx: &mut Context<Self>) {
        const MAX_DRAFT_SUBJECTS: usize = 8;
        let Some(workdir) = self.agent_git.workdir.clone() else {
            return;
        };
        let staged_files: Vec<String> = self
            .agent_git
            .staged_files()
            .iter()
            .map(|entry| entry.path.clone())
            .collect();
        if self.agent_git.busy
            || self.agent_git.commit_draft_request.is_some()
            || staged_files.is_empty()
            || !self.available
        {
            return;
        }
        let branch = self
            .agent_git
            .snapshot
            .tree
            .as_ref()
            .and_then(|tree| tree.branch.clone());
        self.agent_git.commit_draft_seq = self.agent_git.commit_draft_seq.wrapping_add(1);
        let request_id = self.agent_git.commit_draft_seq;
        self.agent_git.commit_draft_request = Some(request_id);
        self.agent_git.error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let dir = PathBuf::from(&workdir);
            let prepared = cx
                .background_executor()
                .spawn(async move {
                    let patch = core_git::staged_patch(&dir)?;
                    let subjects = core_git::recent_commit_subjects(&dir, MAX_DRAFT_SUBJECTS);
                    Ok::<_, GitCommandError>(shelldeck_core::ai::commit_message_context(
                        branch.as_deref(),
                        &staged_files,
                        &subjects,
                        &patch,
                    ))
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                // Closing the dialog abandons the draft before any AI call.
                if this.agent_git.commit_draft_request != Some(request_id) {
                    return;
                }
                match prepared {
                    Ok(context) => cx.emit(super::super::AiAssistantEvent::DraftCommitMessage {
                        request_id,
                        context: Box::new(context),
                    }),
                    Err(error) => {
                        this.set_commit_message_draft(request_id, Err(error.to_string()), cx)
                    }
                }
            });
        })
        .detach();
    }

    /// Host answer to `AiAssistantEvent::DraftCommitMessage`. Only the draft
    /// still awaited fills the message, which stays editable.
    pub fn set_commit_message_draft(
        &mut self,
        request_id: u64,
        result: Result<String, String>,
        cx: &mut Context<Self>,
    ) {
        if self.agent_git.commit_draft_request != Some(request_id) {
            return;
        }
        self.agent_git.commit_draft_request = None;
        match result {
            Ok(message) if self.agent_git.commit_open => {
                self.agent_git.commit_message.update(cx, |state, cx| {
                    state.replace_content(message, cx);
                    cx.notify();
                });
            }
            Ok(_) => {}
            Err(error) => {
                self.agent_git.error = Some(
                    t!("ai.observability.git_commit_generate_failed", error = error).to_string(),
                );
            }
        }
        cx.notify();
    }

    /// Commit confirmation, mounted at the Assistant root like the other
    /// dialogs so it overlays whichever host is showing the panel.
    pub(in crate::ai_assistant) fn render_git_commit_dialog(
        &self,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if !self.agent_git.commit_open {
            return None;
        }
        let staged = self.agent_git.staged_files();
        let busy = self.agent_git.busy;
        let drafting = self.agent_git.commit_draft_request.is_some();
        let mut paths = div()
            .flex()
            .flex_col()
            .gap(px(2.0))
            .font_family(MONO)
            .text_size(px(10.0))
            .text_color(ShellDeckColors::text_muted());
        for entry in staged.iter().take(MAX_COMMIT_PATHS_LISTED) {
            paths = paths.child(div().truncate().child(entry.path.clone()));
        }
        if staged.len() > MAX_COMMIT_PATHS_LISTED {
            paths = paths.child(div().child("…"));
        }
        Some(
            UiDialog::new()
                .width(gpui::px(460.0))
                .header(
                    div()
                        .px(px(16.0))
                        .py(px(14.0))
                        .text_size(px(14.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(t!("ai.observability.git_commit_title").to_string()),
                )
                .content(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(10.0))
                        .px(px(16.0))
                        .pb(px(16.0))
                        .child(
                            Input::new(&self.agent_git.commit_message)
                                .size(InputSize::Sm)
                                .multi_line(true)
                                .min_rows(3)
                                .max_rows(8)
                                .placeholder(
                                    t!("ai.observability.git_commit_placeholder").to_string(),
                                )
                                .disabled(busy || drafting),
                        )
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(8.0))
                                .child(
                                    Button::new(
                                        "ai-git-commit-generate",
                                        if drafting {
                                            t!("ai.observability.git_commit_generating")
                                        } else {
                                            t!("ai.observability.git_commit_generate")
                                        }
                                        .to_string(),
                                    )
                                    .variant(ButtonVariant::Secondary)
                                    .size(ButtonSize::Sm)
                                    .icon(IconSource::from("sparkles"))
                                    .disabled(
                                        busy || drafting || staged.is_empty() || !self.available,
                                    )
                                    .on_click(cx.listener(
                                        |this, _, _, cx| {
                                            this.start_commit_message_draft(cx);
                                        },
                                    )),
                                )
                                .children(drafting.then(|| {
                                    Spinner::new()
                                        .size(SpinnerSize::Xs)
                                        .variant(SpinnerVariant::Primary)
                                })),
                        )
                        .child(
                            div()
                                .text_size(px(10.0))
                                .text_color(ShellDeckColors::text_muted())
                                .child(t!("ai.observability.git_commit_generate_note").to_string()),
                        )
                        .child(
                            div()
                                .text_size(px(11.0))
                                .text_color(ShellDeckColors::text_muted())
                                .child(
                                    t!("ai.observability.git_file_count", count = staged.len())
                                        .to_string(),
                                ),
                        )
                        .child(paths)
                        .children(self.agent_git.error.clone().map(|error| {
                            div()
                                .text_size(px(11.0))
                                .text_color(ShellDeckColors::error())
                                .child(error)
                        }))
                        .child(
                            div()
                                .text_size(px(10.5))
                                .text_color(ShellDeckColors::text_muted())
                                .child(t!("ai.observability.git_no_push").to_string()),
                        ),
                )
                .footer(
                    div()
                        .flex()
                        .justify_end()
                        .gap(px(8.0))
                        .p(px(12.0))
                        .border_t_1()
                        .border_color(ShellDeckColors::border())
                        .child(
                            Button::new("ai-git-commit-cancel", t!("scripts.cancel").to_string())
                                .variant(ButtonVariant::Ghost)
                                .size(ButtonSize::Sm)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.agent_git.commit_open = false;
                                    this.agent_git.commit_draft_request = None;
                                    cx.notify();
                                })),
                        )
                        .child(
                            Button::new(
                                "ai-git-commit-confirm",
                                t!("ai.observability.git_commit_confirm").to_string(),
                            )
                            .variant(ButtonVariant::Default)
                            .size(ButtonSize::Sm)
                            .icon(IconSource::from("git-commit-horizontal"))
                            .disabled(busy || drafting)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.confirm_git_commit(cx);
                            })),
                        ),
                )
                .on_backdrop_click({
                    let entity = cx.entity();
                    move |_, cx| {
                        entity.update(cx, |this, cx| {
                            this.agent_git.commit_open = false;
                            this.agent_git.commit_draft_request = None;
                            cx.notify();
                        });
                    }
                })
                .into_any_element(),
        )
    }
}

/// The commit message field is multi-line: a body may follow the subject.
fn commit_message_state(cx: &mut App) -> Entity<InputState> {
    cx.new(|cx| InputState::new(cx).multi_line(true))
}

fn section_label(label: String, count: usize) -> AnyElement {
    div()
        .flex()
        .items_center()
        .gap(px(6.0))
        .pt(px(2.0))
        .text_size(px(9.0))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(ShellDeckColors::text_muted())
        .child(label.to_uppercase())
        .child(
            div()
                .font_family(MONO)
                .font_weight(FontWeight::NORMAL)
                .child(count.to_string()),
        )
        .into_any_element()
}

fn status_line(icon: &'static str, color: Hsla, message: String) -> AnyElement {
    div()
        .flex()
        .items_start()
        .gap(px(6.0))
        .min_w(px(0.0))
        .text_size(px(10.0))
        .text_color(color)
        .child(div().flex_shrink_0().child(lucide_icon(icon, 12.0, color)))
        .child(div().flex_1().min_w(px(0.0)).child(message))
        .into_any_element()
}
