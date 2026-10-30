use super::*;
use shelldeck_core::agent_session::storage::AgentConsolePreferences;

/// Navigation reflects what needs the person's attention, not just process state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum NavigatorState {
    Quiet,
    Starting,
    Working,
    Stopping,
    Stopped,
    Done,
    Failed,
    NeedsAttention,
    Input,
    Approval,
}

impl NavigatorState {
    fn for_session(session: &AgentSession) -> Self {
        if session.archived {
            return Self::Quiet;
        }
        if session.approval.is_some() {
            return Self::Approval;
        }
        if session.question.is_some() {
            return Self::Input;
        }
        if session.status == AgentSessionStatus::Failed {
            return Self::Failed;
        }
        if session.attention == AgentSessionAttention::NeedsAttention {
            return Self::NeedsAttention;
        }
        match session.status {
            AgentSessionStatus::Starting => Self::Starting,
            AgentSessionStatus::Running => Self::Working,
            AgentSessionStatus::Stopping => Self::Stopping,
            AgentSessionStatus::Cancelled => Self::Stopped,
            AgentSessionStatus::Completed
                if session.unread_count > 0
                    || session.attention == AgentSessionAttention::Unread =>
            {
                Self::Done
            }
            _ => Self::Quiet,
        }
    }

    fn priority(self) -> u8 {
        match self {
            Self::Approval | Self::Input | Self::NeedsAttention => 4,
            Self::Failed => 3,
            Self::Done => 2,
            Self::Starting | Self::Working | Self::Stopping => 1,
            Self::Quiet | Self::Stopped => 0,
        }
    }

    fn label(self) -> Option<String> {
        Some(match self {
            Self::Quiet => return None,
            Self::Approval => t!("agents.navigator.awaiting_approval").to_string(),
            Self::Input => t!("agents.navigator.awaiting_input").to_string(),
            Self::NeedsAttention => t!("agents.navigator.needs_attention").to_string(),
            Self::Starting => session_status_label(AgentSessionStatus::Starting),
            Self::Working => session_status_label(AgentSessionStatus::Running),
            Self::Stopping => session_status_label(AgentSessionStatus::Stopping),
            Self::Stopped => session_status_label(AgentSessionStatus::Cancelled),
            Self::Done => session_status_label(AgentSessionStatus::Completed),
            Self::Failed => session_status_label(AgentSessionStatus::Failed),
        })
    }

    fn color(self) -> Hsla {
        match self {
            Self::Approval | Self::Input | Self::NeedsAttention => ShellDeckColors::warning(),
            Self::Failed => ShellDeckColors::error(),
            Self::Done => ShellDeckColors::success(),
            Self::Starting | Self::Working | Self::Stopping => ShellDeckColors::primary(),
            Self::Quiet | Self::Stopped => ShellDeckColors::text_muted(),
        }
    }
}

fn project_session_summary(
    project: &AgentProjectGroup,
    groups: &[AgentProjectGroup],
    sessions: &[AgentSession],
    include_archived: bool,
    selected: Option<Uuid>,
) -> (usize, NavigatorState) {
    session_group_summary(sessions.iter().filter(|session| {
        (include_archived || !session.archived)
            && !super::navigation_groups::navigator_pin(session)
            && Some(session.id) != selected
            && navigator_worktree(session, groups).is_some_and(|owner| {
                project
                    .worktrees
                    .iter()
                    .any(|worktree| worktree.id == owner.id)
            })
    }))
}

pub(super) fn session_group_summary<'a>(
    sessions: impl Iterator<Item = &'a AgentSession>,
) -> (usize, NavigatorState) {
    let mut count = 0;
    let mut attention = NavigatorState::Quiet;
    for session in sessions {
        count += 1;
        let state = NavigatorState::for_session(session);
        if state.priority() > attention.priority() {
            attention = state;
        }
    }
    (count, attention)
}

// Like the sidebar and remote directory, mount tooltips only on hover through
// GPUI. The shared Tooltip's invisible deferred overlay occludes adjacent menus.
struct NavigatorTooltip(String);
impl Render for NavigatorTooltip {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .p(px(8.0))
            .max_w(px(420.0))
            .rounded(px(5.0))
            .bg(ShellDeckColors::bg_surface())
            .text_color(ShellDeckColors::text_primary())
            .text_size(px(11.0))
            .child(self.0.clone())
    }
}

pub(super) fn collapsed_session_summary(
    group_id: &str,
    count: usize,
    attention: NavigatorState,
) -> impl IntoElement {
    let mut hint = t!("agents.navigator.hidden_sessions", count = count).to_string();
    if let Some(label) = attention.label() {
        hint.push_str(&format!(" · {label}"));
    }
    div()
        .id(SharedString::from(format!(
            "agent-group-summary-{group_id}"
        )))
        .debug_selector(|| format!("agent-group-summary-{group_id}"))
        .tooltip(move |_, cx| cx.new(|_| NavigatorTooltip(hint.clone())).into())
        .flex()
        .items_center()
        .gap(px(5.0))
        .flex_shrink_0()
        .when(attention.priority() > 0, |summary| {
            summary.child(div().size(px(6.0)).rounded_full().bg(attention.color()))
        })
        .child(Badge::new(count.to_string()).variant(BadgeVariant::Secondary))
}

pub(super) fn navigator_worktree<'a>(
    session: &AgentSession,
    groups: &'a [AgentProjectGroup],
) -> Option<&'a AgentWorktreeOption> {
    groups
        .iter()
        .flat_map(|project| &project.worktrees)
        .filter(|worktree| session_worktree_matches(session, worktree))
        .max_by(|left, right| {
            left.path
                .trim_end_matches('/')
                .len()
                .cmp(&right.path.trim_end_matches('/').len())
                .then_with(|| right.id.cmp(&left.id))
        })
}

fn navigator_query_matches(query: &str, text: &str) -> bool {
    if query.trim().is_empty() {
        return true;
    }
    let text = text.to_lowercase();
    query
        .to_lowercase()
        .split_whitespace()
        .all(|term| text.contains(term))
}

fn worktree_search_text(project: &str, worktree: &AgentWorktreeOption) -> String {
    format!(
        "{} {} {} {} {}",
        project,
        worktree.label,
        worktree.path,
        worktree.branch.as_deref().unwrap_or_default(),
        worktree.target.label()
    )
}

pub(super) fn navigator_session_matches_query(
    session: &AgentSession,
    groups: &[AgentProjectGroup],
    query: &str,
) -> bool {
    if query.trim().is_empty() {
        return true;
    }
    let scope = navigator_worktree(session, groups)
        .and_then(|worktree| {
            groups
                .iter()
                .find(|project| {
                    project
                        .worktrees
                        .iter()
                        .any(|entry| entry.id == worktree.id)
                })
                .map(|project| worktree_search_text(&project.label, worktree))
        })
        .unwrap_or_default();
    let metadata = format!(
        "{} {} {} {} {}",
        scope,
        session.name,
        session.context.workdir,
        session.context.target.label(),
        session.context.provider.display_name(),
    )
    .to_lowercase();
    let mut missing = query
        .split_whitespace()
        .map(str::to_lowercase)
        .filter(|term| !metadata.contains(term))
        .collect::<Vec<_>>();
    // Avoid copying entire retained transcripts on each streaming repaint.
    // Only search message bodies when project/session metadata was insufficient.
    for message in &session.messages {
        if missing.is_empty() {
            break;
        }
        let body = message.text.to_lowercase();
        missing.retain(|term| !body.contains(term));
    }
    missing.is_empty()
}

pub(super) fn project_group_collapsed(
    preferences: &AgentConsolePreferences,
    project_id: &str,
    query: &str,
) -> bool {
    query.trim().is_empty()
        && preferences
            .collapsed_project_ids
            .iter()
            .any(|id| id == project_id)
}

pub(super) fn toggle_project_group(
    preferences: &mut AgentConsolePreferences,
    project_id: &str,
    query: &str,
) -> bool {
    query.trim().is_empty() && preferences.toggle_project_collapsed(project_id)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ConversationAction {
    Rename,
    TogglePin,
    Fork,
    Copy,
    ExportMarkdown,
    ExportJson,
    ToggleArchive,
}

impl ConversationAction {
    fn label(self, session: &AgentSession) -> String {
        t!(match self {
            Self::Rename => "agents.library.rename",
            Self::TogglePin if session.pinned => "agents.library.unpin",
            Self::TogglePin => "agents.library.pin",
            Self::Fork if session.remote_history.is_some() => "agents.library.fork_local",
            Self::Fork => "agents.library.fork",
            Self::Copy if session.remote_history.is_some() => "agents.library.copy_local_chat",
            Self::Copy => "agents.library.copy_chat",
            Self::ExportMarkdown if session.remote_history.is_some() =>
                "agents.library.export_local_md",
            Self::ExportMarkdown => "agents.library.export_md",
            Self::ExportJson if session.remote_history.is_some() =>
                "agents.library.export_local_json",
            Self::ExportJson => "agents.library.export_json",
            Self::ToggleArchive if session.archived => "agents.library.restore",
            Self::ToggleArchive => "agents.library.archive",
        })
        .to_string()
    }

    fn empty_remote_transcript(self, session: &AgentSession) -> bool {
        session.remote_history.is_some()
            && session.messages.is_empty()
            && session.trace.is_empty()
            && matches!(
                self,
                Self::Fork | Self::Copy | Self::ExportMarkdown | Self::ExportJson
            )
    }

    fn enabled(self, session: &AgentSession) -> bool {
        !(self.empty_remote_transcript(session)
            || self == Self::ToggleArchive && session.status.is_active())
    }

    fn icon(self, session: &AgentSession) -> &'static str {
        match self {
            Self::Rename => "pencil",
            Self::TogglePin => "pin",
            Self::Fork | Self::Copy => "copy",
            Self::ExportMarkdown | Self::ExportJson => "download",
            Self::ToggleArchive if session.archived => "archive-restore",
            Self::ToggleArchive => "archive",
        }
    }
}

impl AgentConsoleView {
    fn perform_conversation_action(
        &mut self,
        id: Uuid,
        action: ConversationAction,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.sessions.get(id) else {
            return;
        };
        // Read current state, not the state captured when the menu was opened.
        if !action.enabled(session) {
            return;
        }
        match action {
            ConversationAction::Rename => {
                self.library.rename_state = input_state(cx, session.name.clone());
                self.library.rename_session = Some(id);
                self.library.error = None;
            }
            ConversationAction::TogglePin => {
                let pinned = session.pinned;
                if let Err(error) = self.sessions.set_pinned(id, !pinned, now_ms()) {
                    self.library.error = Some(error.to_string());
                }
                self.schedule_library_save(cx);
            }
            ConversationAction::Fork => self.fork_library_session(id, cx),
            ConversationAction::Copy => {
                cx.write_to_clipboard(ClipboardItem::new_string(session.export_markdown()));
                self.library.notice = Some(t!("agents.library.copied").to_string());
            }
            ConversationAction::ExportMarkdown => self.export_library_session(id, false, cx),
            ConversationAction::ExportJson => self.export_library_session(id, true, cx),
            ConversationAction::ToggleArchive => {
                // Revalidate after opening the menu: a run may have started meanwhile.
                if session.status.is_active() {
                    return;
                }
                self.archive_library_session(id, !session.archived, cx);
            }
        }
        cx.notify();
    }

    fn conversation_actions_menu(
        &self,
        id: Uuid,
        trigger: AnyElement,
        mouse_button: MouseButton,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let parent = cx.entity();
        Popover::new(SharedString::from(format!(
            "agent-conversation-menu-{id}-{mouse_button:?}"
        )))
        .anchor(Corner::TopRight)
        .mouse_button(mouse_button)
        .trigger(trigger)
        .content(move |window, cx| {
            let parent = parent.clone();
            // A dismissed rename is cancelled when the next menu is opened.
            parent.update(cx, |view, _| view.library.rename_session = None);
            cx.new(move |content_cx| {
                content_cx.observe(&parent, |_, _, cx| cx.notify()).detach();
                PopoverContent::new(window, content_cx, move |_, cx| {
                    let view = parent.read(cx);
                    let Some(session) = view.sessions.get(id) else {
                        return div().into_any_element();
                    };
                    let mut content = div()
                        .id("agent-conversation-actions-scroll")
                        .w(px(250.0))
                        .max_h(px(360.0))
                        .overflow_y_scroll()
                        .flex()
                        .flex_col()
                        .gap(px(3.0));
                    if view.library.rename_session == Some(id) {
                        let enter_parent = parent.clone();
                        let popup = cx.weak_entity();
                        content = content.child(
                            Input::new(&view.library.rename_state)
                                .w_full()
                                .size(InputSize::Sm)
                                .placeholder(t!("agents.library.rename").to_string())
                                .on_enter(move |_, cx| {
                                    let done = enter_parent.update(cx, |view, cx| {
                                        if view.library.rename_session != Some(id) {
                                            return true;
                                        }
                                        view.rename_library_session(cx);
                                        view.library.rename_session.is_none()
                                    });
                                    if done {
                                        let _ = popup.update(cx, |_, cx| cx.emit(DismissEvent));
                                    }
                                }),
                        );
                        if let Some(error) = &view.library.error {
                            content = content.child(
                                div()
                                    .text_size(px(11.0))
                                    .text_color(ShellDeckColors::error())
                                    .child(error.clone()),
                            );
                        }
                        let save_parent = parent.clone();
                        let cancel_parent = parent.clone();
                        return content
                            .child(
                                div()
                                    .flex()
                                    .gap(px(5.0))
                                    .child(
                                        Button::new(
                                            "agent-row-save-name",
                                            t!("agents.library.save").to_string(),
                                        )
                                        .size(ButtonSize::Sm)
                                        .on_click(
                                            cx.listener(move |_, _, _, cx| {
                                                let done = save_parent.update(cx, |view, cx| {
                                                    if view.library.rename_session != Some(id) {
                                                        return true;
                                                    }
                                                    view.rename_library_session(cx);
                                                    view.library.rename_session.is_none()
                                                });
                                                if done {
                                                    cx.emit(DismissEvent);
                                                }
                                            }),
                                        ),
                                    )
                                    .child(
                                        Button::new(
                                            "agent-row-cancel-name",
                                            t!("agents.library.cancel").to_string(),
                                        )
                                        .variant(ButtonVariant::Ghost)
                                        .size(ButtonSize::Sm)
                                        .on_click(
                                            cx.listener(move |_, _, _, cx| {
                                                cancel_parent.update(cx, |view, cx| {
                                                    if view.library.rename_session == Some(id) {
                                                        view.library.rename_session = None;
                                                    }
                                                    cx.notify();
                                                });
                                                cx.emit(DismissEvent);
                                            }),
                                        ),
                                    ),
                            )
                            .into_any_element();
                    }
                    content = content.child(
                        div()
                            .pb(px(5.0))
                            .text_size(px(12.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .truncate()
                            .child(session.name.clone()),
                    );
                    if session.remote_history.is_some() {
                        content = content.child(
                            div()
                                .pb(px(5.0))
                                .text_size(px(11.0))
                                .whitespace_normal()
                                .text_color(ShellDeckColors::text_muted())
                                .child(t!("agents.library.local_export_notice").to_string()),
                        );
                    }
                    for (index, action) in [
                        ConversationAction::Rename,
                        ConversationAction::TogglePin,
                        ConversationAction::Fork,
                        ConversationAction::Copy,
                        ConversationAction::ExportMarkdown,
                        ConversationAction::ExportJson,
                        ConversationAction::ToggleArchive,
                    ]
                    .into_iter()
                    .enumerate()
                    {
                        let parent = parent.clone();
                        content = content.child(
                            Button::new(("agent-row-action", index), action.label(session))
                                .debug_selector(|| format!("agent-nav-action-{id}-{action:?}"))
                                .variant(ButtonVariant::Ghost)
                                .size(ButtonSize::Sm)
                                .w_full()
                                .justify_start()
                                .icon(IconSource::from(action.icon(session)))
                                .disabled(!action.enabled(session))
                                .tooltip(if action.empty_remote_transcript(session) {
                                    t!("agents.library.local_transcript_empty").to_string()
                                } else {
                                    action.label(session)
                                })
                                .on_click(cx.listener(move |_, _, window, cx| {
                                    cx.stop_propagation();
                                    parent.update(cx, |view, cx| {
                                        view.perform_conversation_action(id, action, cx);
                                        if action == ConversationAction::Rename {
                                            window.focus(
                                                &view
                                                    .library
                                                    .rename_state
                                                    .read(cx)
                                                    .focus_handle(cx),
                                            );
                                        }
                                    });
                                    if action != ConversationAction::Rename {
                                        cx.emit(DismissEvent);
                                    }
                                })),
                        );
                    }
                    content.into_any_element()
                })
            })
        })
    }

    fn render_archive_undo(&self, cx: &Context<Self>) -> AnyElement {
        let Some(session) = self
            .library
            .undo_archive
            .and_then(|id| self.sessions.get(id))
            .filter(|session| session.archived)
        else {
            return div().into_any_element();
        };
        div()
            .flex()
            .flex_col()
            .flex_shrink_0()
            .gap(px(4.0))
            .p(px(8.0))
            .border_t_1()
            .border_color(ShellDeckColors::border())
            .child(
                div().text_size(px(11.0)).truncate().child(
                    t!(
                        "agents.library.archived_notice",
                        name = session.name.clone()
                    )
                    .to_string(),
                ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(4.0))
                    .child(
                        Button::new(
                            "agent-navigator-undo-archive",
                            t!("agents.library.undo_archive").to_string(),
                        )
                        .variant(ButtonVariant::Secondary)
                        .size(ButtonSize::Sm)
                        .on_click(cx.listener(|this, _, _, cx| this.undo_library_archive(cx))),
                    )
                    .child(
                        Button::new("agent-navigator-dismiss-archive", "")
                            .variant(ButtonVariant::Ghost)
                            .size(ButtonSize::Icon)
                            .icon(IconSource::from("x"))
                            .tooltip(t!("agents.library.dismiss").to_string())
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.library.undo_archive = None;
                                cx.notify();
                            })),
                    ),
            )
            .into_any_element()
    }

    fn new_worktree_session(&mut self, expected: &AgentWorktreeOption, cx: &mut Context<Self>) {
        let current = self
            .project_groups
            .iter()
            .flat_map(|project| &project.worktrees)
            .find(|worktree| *worktree == expected)
            .cloned();
        if let Some(worktree) = current {
            let context = context_for_worktree(&self.displayed_context(cx), &worktree);
            let _ = self.create_session(t!("agents.session.new").to_string(), context, cx);
        }
    }

    fn new_worktree_button(&self, worktree: &AgentWorktreeOption, cx: &Context<Self>) -> Button {
        let worktree = worktree.clone();
        Button::new(
            SharedString::from(format!("agent-worktree-new-{}", worktree.id)),
            "",
        )
        .variant(ButtonVariant::Ghost)
        .size(ButtonSize::Icon)
        .icon(IconSource::from("plus"))
        .w(px(24.0))
        .h(px(24.0))
        .flex_shrink_0()
        .tooltip(format!("{} · {}", t!("agents.session.new"), worktree.label))
        .on_click(cx.listener(move |this, _, _, cx| {
            cx.stop_propagation();
            this.new_worktree_session(&worktree, cx);
        }))
    }
    pub(crate) fn navigator_visible(&self) -> bool {
        if self.tools.navigator_wide {
            self.tools.navigator_visible
        } else {
            self.tools.navigator_overlay
        }
    }

    pub(crate) fn toggle_navigator(&mut self, cx: &mut Context<Self>) {
        if self.tools.navigator_wide {
            self.tools.navigator_visible = !self.tools.navigator_visible;
            self.schedule_library_save(cx);
        } else {
            self.tools.navigator_overlay = !self.tools.navigator_overlay;
        }
        cx.emit(AgentConsoleEvent::NavigatorChanged);
        cx.notify();
    }

    pub(super) fn close_navigator_overlay(&mut self, cx: &mut Context<Self>) {
        if self.tools.navigator_overlay {
            self.tools.navigator_overlay = false;
            cx.emit(AgentConsoleEvent::NavigatorChanged);
            cx.notify();
        }
    }

    pub(super) fn update_navigator_layout(&mut self, wide: bool, cx: &mut Context<Self>) {
        if self.tools.navigator_wide != wide {
            self.tools.navigator_wide = wide;
            self.tools.navigator_overlay = false;
            cx.emit(AgentConsoleEvent::NavigatorChanged);
        }
    }

    pub(super) fn render_navigator_overlay(&self, cx: &mut Context<Self>) -> AnyElement {
        // The same navigator floats above narrow content instead of squeezing
        // the composer. The backdrop dismisses it without changing the session.
        div()
            .absolute()
            .inset_0()
            .flex()
            .occlude()
            .bg(ShellDeckColors::backdrop())
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.close_navigator_overlay(cx);
                }),
            )
            .child(
                div()
                    .h_full()
                    .max_w_full()
                    .occlude()
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .child(self.render_navigator(cx)),
            )
            .into_any_element()
    }

    pub(super) fn render_navigator(&self, cx: &mut Context<Self>) -> AnyElement {
        let parent = cx.entity();
        let running_count = self.sessions.active_count();
        let unread_count = self
            .sessions
            .sessions()
            .iter()
            .map(|session| u64::from(session.unread_count))
            .sum::<u64>();
        let mut body = div()
            .flex()
            .flex_col()
            .gap(px(4.0))
            .px(px(6.0))
            .py(px(7.0))
            .child(self.render_pinned_conversations(cx));
        let query = self.navigator_search.read(cx).content().to_string();
        for project in &self.project_groups {
            let visible_worktrees = project
                .worktrees
                .iter()
                .filter(|worktree| {
                    navigator_query_matches(&query, &worktree_search_text(&project.label, worktree))
                        || self
                            .sessions
                            .library_sessions(self.library.show_archived())
                            .iter()
                            .any(|session| {
                                navigator_worktree(session, &self.project_groups)
                                    .is_some_and(|owner| owner.id == worktree.id)
                                    && navigator_session_matches_query(
                                        session,
                                        &self.project_groups,
                                        &query,
                                    )
                            })
                })
                .collect::<Vec<_>>();
            if visible_worktrees.is_empty() {
                continue;
            }
            let collapsed = project_group_collapsed(&self.library.preferences, &project.id, &query);
            let project_id = project.id.clone();
            let mut header = div()
                .flex()
                .items_center()
                .min_w_0()
                .gap(px(3.0))
                .pt(px(3.0))
                .child(
                    Button::new(
                        SharedString::from(format!("agent-project-collapse-{}", project.id)),
                        "",
                    )
                    .debug_selector(|| format!("agent-project-collapse-{}", project.id))
                    .variant(ButtonVariant::Ghost)
                    .size(ButtonSize::Icon)
                    .icon(IconSource::from(if collapsed {
                        "chevron-right"
                    } else {
                        "chevron-down"
                    }))
                    .w(px(24.0))
                    .h(px(24.0))
                    .flex_shrink_0()
                    .disabled(!query.trim().is_empty())
                    .tooltip(
                        t!(if collapsed {
                            "agents.navigator.expand_project"
                        } else {
                            "agents.navigator.collapse_project"
                        })
                        .to_string(),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if !this
                            .project_groups
                            .iter()
                            .any(|group| group.id == project_id)
                        {
                            return;
                        }
                        let query = this.navigator_search.read(cx).content().to_string();
                        if toggle_project_group(&mut this.library.preferences, &project_id, &query)
                        {
                            this.schedule_library_save(cx);
                            cx.notify();
                        }
                    })),
                );
            if let [worktree] = project.worktrees.as_slice() {
                // Name navigation remains separate from the collapse control.
                let worktree_option = worktree.clone();
                header = header.child(
                    div().flex_1().min_w_0().overflow_hidden().child(
                        Button::new(
                            SharedString::from(format!("agent-project-{}", project.id)),
                            project.label.clone(),
                        )
                        .variant(ButtonVariant::Ghost)
                        .size(ButtonSize::Sm)
                        .w_full()
                        .justify_start()
                        .overflow_hidden()
                        .text_ellipsis()
                        .icon(IconSource::from("folder"))
                        .tooltip(worktree.path.clone())
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.select_worktree_session(&worktree_option, cx);
                        })),
                    ),
                );
                header = header.child(self.new_worktree_button(worktree, cx));
            } else {
                header = header
                    .child(lucide_icon("folder", 12.0, ShellDeckColors::text_muted()))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .text_ellipsis()
                            .text_size(px(12.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(project.label.clone()),
                    );
            }
            if collapsed {
                let (count, attention) = project_session_summary(
                    project,
                    &self.project_groups,
                    self.sessions.sessions(),
                    self.library.show_archived(),
                    self.navigator_selected_session(),
                );
                if count > 0 {
                    header = header.child(collapsed_session_summary(&project.id, count, attention));
                }
            }
            body = body.child(header);
            if collapsed {
                let selected = self.navigator_selected_session();
                body = body.child(self.render_session_rows_matching(
                    |session| {
                        Some(session.id) == selected
                            && !super::navigation_groups::navigator_pin(session)
                            && navigator_worktree(session, &self.project_groups).is_some_and(
                                |owner| {
                                    project
                                        .worktrees
                                        .iter()
                                        .any(|worktree| worktree.id == owner.id)
                                },
                            )
                    },
                    cx,
                ));
                continue;
            }
            // A single primary checkout sharing the project name stays one row.
            if let [worktree] = project.worktrees.as_slice() {
                if worktree.label == project.label && worktree.branch.is_none() {
                    body = body.child(self.render_project_session_rows(worktree, cx));
                    continue;
                }
            }
            for worktree in visible_worktrees {
                let worktree_option = worktree.clone();
                let worktree_selected = self
                    .navigator_selected_session()
                    .and_then(|id| self.sessions.get(id))
                    .is_some_and(|session| session_worktree_matches(session, worktree));
                body = body.child(
                    div()
                        .id(("agent-worktree", stable_string_key(&worktree.id)))
                        .flex()
                        .items_center()
                        .gap(px(5.0))
                        .px(px(11.0))
                        .py(px(3.0))
                        .rounded(px(4.0))
                        .cursor_pointer()
                        .when(worktree_selected, |row| {
                            row.bg(ShellDeckColors::selected_bg())
                        })
                        .when(!worktree_selected, |row| {
                            row.hover(|style| style.bg(ShellDeckColors::hover_bg()))
                        })
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.select_worktree_session(&worktree_option, cx);
                        }))
                        .text_size(px(11.0))
                        .text_color(ShellDeckColors::text_muted())
                        .child(lucide_icon(
                            "git-branch",
                            10.0,
                            ShellDeckColors::text_muted(),
                        ))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .child(worktree.label.clone()),
                        )
                        .when_some(worktree.branch.clone(), |row, branch| {
                            row.child(
                                div()
                                    .flex_1()
                                    .min_w(px(0.0))
                                    .overflow_hidden()
                                    .text_ellipsis()
                                    .font_family("JetBrains Mono")
                                    .text_size(px(10.0))
                                    .child(branch),
                            )
                        })
                        .when(worktree.is_primary, |row| {
                            row.child(
                                div()
                                    .size(px(5.0))
                                    .rounded_full()
                                    .bg(ShellDeckColors::success()),
                            )
                        })
                        .child(self.new_worktree_button(worktree, cx)),
                );
                body = body.child(self.render_project_session_rows(worktree, cx));
            }
        }
        body = body.child(self.render_uncatalogued_folders(cx));
        body = body.child(self.render_ssh_session_rows(cx));
        body = body.child(self.render_remote_session_rows(cx));
        div()
            .w(px(f32::from(self.library.preferences.navigator_width)))
            .h_full()
            .flex()
            .flex_col()
            .flex_shrink_0()
            .bg(ShellDeckColors::bg_sidebar())
            .border_r_1()
            .border_color(ShellDeckColors::border())
            .child(
                div()
                    .h(px(42.0))
                    .flex()
                    .items_center()
                    .gap(px(7.0))
                    .px(px(10.0))
                    .border_b_1()
                    .border_color(ShellDeckColors::border())
                    .child(lucide_icon("grid-2x2", 14.0, ShellDeckColors::primary()))
                    .child(
                        div()
                            .flex_1()
                            .text_size(px(11.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(t!("agents.title").to_string()),
                    )
                    .when(running_count > 0, |row| {
                        row.child(
                            Badge::new(running_count.to_string()).variant(BadgeVariant::Secondary),
                        )
                    })
                    .child(
                        Button::new("agent-nav-add-project", "")
                            .variant(ButtonVariant::Ghost)
                            .size(ButtonSize::Icon)
                            .icon(IconSource::from("folder"))
                            .w(px(26.0))
                            .h(px(26.0))
                            .tooltip(t!("agents.project.create").to_string())
                            .on_click(cx.listener(|_, _, _, cx| {
                                cx.emit(AgentConsoleEvent::OpenProjectSetup)
                            })),
                    )
                    .child(
                        Button::new("agent-nav-new", "")
                            .variant(ButtonVariant::Ghost)
                            .size(ButtonSize::Icon)
                            .icon(IconSource::from("plus"))
                            .w(px(26.0))
                            .h(px(26.0))
                            .tooltip(t!("agents.session.new").to_string())
                            .on_click(cx.listener(|this, _, _, cx| this.new_session(cx))),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .px(px(8.0))
                    .py(px(6.0))
                    .border_b_1()
                    .border_color(ShellDeckColors::border())
                    .child(lucide_icon("search", 11.0, ShellDeckColors::text_muted()))
                    .child(
                        div().flex().flex_col().flex_1().min_w(px(0.0)).child(
                            Input::new(&self.navigator_search)
                                .debug_selector("agent-nav-search")
                                .variant(InputVariant::Bare)
                                .size(InputSize::Sm)
                                .w_full()
                                .placeholder(t!("agents.navigator.search").to_string())
                                .on_change(move |_value, cx| {
                                    parent.update(cx, |_this, cx| cx.notify());
                                }),
                        ),
                    )
                    .child(
                        Button::new("agent-library-archived-filter", "")
                            .variant(if self.library.show_archived() {
                                ButtonVariant::Secondary
                            } else {
                                ButtonVariant::Ghost
                            })
                            .size(ButtonSize::Icon)
                            .icon(IconSource::from("archive"))
                            .tooltip(t!("agents.library.show_archived").to_string())
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.library.preferences.show_archived =
                                    !this.library.preferences.show_archived;
                                this.schedule_library_save(cx);
                                cx.notify();
                            })),
                    ),
            )
            .child(
                div()
                    .id("agent-navigator-scroll")
                    .debug_selector(|| "agent-navigator-scroll".into())
                    .flex_1()
                    .min_h(px(0.0))
                    .overflow_y_scroll()
                    .child(body),
            )
            .child(self.render_archive_undo(cx))
            .child(
                div()
                    .h(px(30.0))
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .px(px(10.0))
                    .border_t_1()
                    .border_color(ShellDeckColors::border())
                    .text_size(px(9.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(lucide_icon("activity", 11.0, ShellDeckColors::warning()))
                    .child(format!(
                        "{running_count}/{DEFAULT_MAX_CONCURRENT_AGENT_SESSIONS}"
                    ))
                    .child(t!("workspaces.agent.running").to_string())
                    .child(div().flex_1())
                    .when(unread_count > 0, |row| {
                        row.child(lucide_icon("activity", 10.0, ShellDeckColors::primary()))
                            .child(unread_count.to_string())
                            .child(t!("workspaces.attention.unread").to_string())
                    }),
            )
            .into_any_element()
    }

    pub(super) fn render_session_rows(
        &self,
        worktree: Option<&AgentWorktreeOption>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        self.render_session_rows_matching(
            |session| {
                worktree.is_none_or(|worktree| {
                    navigator_worktree(session, &self.project_groups)
                        .is_some_and(|owner| owner.id == worktree.id)
                })
            },
            cx,
        )
    }

    pub(super) fn render_session_rows_matching(
        &self,
        include: impl Fn(&AgentSession) -> bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let query = self
            .navigator_search
            .read(cx)
            .content()
            .trim()
            .to_lowercase();
        let mut list = div().flex().flex_col().gap(px(2.0));
        for session in self
            .sessions
            .library_sessions(self.library.show_archived())
            .into_iter()
            .filter(|session| {
                include(session)
                    && navigator_session_matches_query(session, &self.project_groups, &query)
            })
        {
            let id = session.id;
            let selected = self.navigator_selected_session() == Some(id);
            let state = NavigatorState::for_session(session);
            let hint = format!(
                "{}\n{} · {}\n{}",
                session.name,
                session.context.provider.display_name(),
                target_label(&session.context.target),
                session.context.workdir
            );
            let row = div()
                .id(("agent-nav-session", uuid_key(id)))
                .debug_selector(|| format!("agent-nav-session-{id}"))
                .flex()
                .items_center()
                .gap(px(7.0))
                .h(px(38.0))
                .w_full()
                .min_w_0()
                .cursor_pointer()
                .tooltip(move |_, cx| cx.new(|_| NavigatorTooltip(hint.clone())).into())
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.select_session(id, cx);
                }))
                .child(div().size(px(6.0)).rounded_full().bg(state.color()))
                .child(lucide_icon(
                    if session.archived {
                        "archive"
                    } else {
                        provider_icon(session.context.provider)
                    },
                    12.0,
                    ShellDeckColors::text_muted(),
                ))
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.0))
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .text_size(px(12.0))
                        .font_weight(if selected {
                            FontWeight::SEMIBOLD
                        } else {
                            FontWeight::NORMAL
                        })
                        .child(session.name.clone()),
                )
                .when(self.has_unsent_draft(id, cx), |row| {
                    row.child(
                        div()
                            .id(("agent-draft-tooltip", uuid_key(id)))
                            .flex_shrink_0()
                            .tooltip(|_, cx| {
                                cx.new(|_| {
                                    NavigatorTooltip(
                                        t!("agents.navigator.unsent_draft").to_string(),
                                    )
                                })
                                .into()
                            })
                            .child(lucide_icon("pencil", 12.0, ShellDeckColors::text_muted())),
                    )
                })
                .when_some(state.label(), |row, label| {
                    row.child(
                        div()
                            .flex_shrink_0()
                            .text_size(px(10.0))
                            .text_color(state.color())
                            .child(label),
                    )
                })
                .when(session.unread_count > 0, |row| {
                    row.child(
                        Badge::new(session.unread_count.to_string())
                            .variant(BadgeVariant::Secondary),
                    )
                });
            // Independent sibling trigger regions share one action menu without
            // selecting the conversation when its ellipsis is opened.
            let actions = self.conversation_actions_menu(
                id,
                Button::new(("agent-conversation-actions", uuid_key(id)), "")
                    .debug_selector(|| format!("agent-nav-menu-{id}"))
                    .variant(ButtonVariant::Ghost)
                    .size(ButtonSize::Icon)
                    .w(px(22.0))
                    .h(px(22.0))
                    .flex_shrink_0()
                    .icon(IconSource::from("ellipsis"))
                    .tooltip(t!("agents.library.actions").to_string())
                    .into_any_element(),
                MouseButton::Left,
                cx,
            );
            list = list.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(7.0))
                    .h(px(38.0))
                    .px(px(8.0))
                    .min_w_0()
                    .rounded(px(5.0))
                    .when(selected, |row| row.bg(ShellDeckColors::selected_bg()))
                    .when(!selected, |row| {
                        row.hover(|style| style.bg(ShellDeckColors::hover_bg()))
                    })
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(self.conversation_actions_menu(
                                id,
                                row.into_any_element(),
                                MouseButton::Right,
                                cx,
                            )),
                    )
                    .child(div().flex_shrink_0().child(actions)),
            );
        }
        list.into_any_element()
    }

    pub(super) fn render_tabs(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut tabs = div()
            .id("agent-session-tabs")
            .flex()
            .flex_1()
            .min_w(px(0.0))
            .h_full()
            .overflow_x_scroll();
        for session in self.sessions.library_sessions(false) {
            let id = session.id;
            let selected = self.sessions.selected_id() == Some(id);
            let removable = !session.status.is_active();
            let mut tab = div()
                .id(("agent-session-tab", uuid_key(id)))
                .h_full()
                .max_w(px(220.0))
                .flex()
                .items_center()
                .gap(px(7.0))
                .px(px(11.0))
                .border_r_1()
                .border_color(ShellDeckColors::border())
                .cursor_pointer()
                .when(selected, |tab| {
                    tab.bg(ShellDeckColors::bg_primary())
                        .border_b_2()
                        .border_color(ShellDeckColors::primary())
                })
                .when(!selected, |tab| {
                    tab.hover(|style| style.bg(ShellDeckColors::hover_bg()))
                })
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.select_session(id, cx);
                }))
                .child(
                    div()
                        .size(px(6.0))
                        .rounded_full()
                        .bg(status_color(session.status)),
                )
                .child(lucide_icon(
                    provider_icon(session.context.provider),
                    12.0,
                    ShellDeckColors::text_muted(),
                ))
                .child(
                    div()
                        .min_w(px(0.0))
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .text_size(px(12.0))
                        .child(session.name.clone()),
                )
                .when(session.attention != AgentSessionAttention::None, |tab| {
                    tab.child(
                        div()
                            .size(px(5.0))
                            .rounded_full()
                            .bg(attention_color(session.attention)),
                    )
                });
            if removable {
                tab = tab.child(
                    div()
                        .id(("agent-close-tab", uuid_key(id)))
                        .size(px(18.0))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(4.0))
                        .hover(|style| style.bg(ShellDeckColors::hover_bg()))
                        .on_click(cx.listener(move |this, _event: &ClickEvent, _, cx| {
                            cx.stop_propagation();
                            this.archive_library_session(id, true, cx);
                        }))
                        .child(lucide_icon("x", 10.0, ShellDeckColors::text_muted())),
                );
            }
            tabs = tabs.child(tab);
        }
        tabs.into_any_element()
    }
}

#[cfg(test)]
mod collapse_tests {
    use super::{project_group_collapsed, toggle_project_group};
    use shelldeck_core::agent_session::storage::AgentConsolePreferences;

    // SDTEST-2745 — search reveals groups without changing saved collapse;
    // independent project IDs keep independent choices across label changes.
    #[test]
    fn project_group_search_reveals_without_overwriting_saved_collapse() {
        let mut preferences = AgentConsolePreferences::default();
        assert!(toggle_project_group(
            &mut preferences,
            "stable-project-a",
            ""
        ));
        assert!(project_group_collapsed(
            &preferences,
            "stable-project-a",
            ""
        ));
        assert!(!project_group_collapsed(
            &preferences,
            "stable-project-b",
            ""
        ));
        let saved = preferences.clone();
        assert!(!project_group_collapsed(
            &preferences,
            "stable-project-a",
            "matching conversation"
        ));
        assert!(!toggle_project_group(
            &mut preferences,
            "stable-project-a",
            "matching conversation"
        ));
        assert_eq!(preferences, saved);
        assert!(project_group_collapsed(
            &preferences,
            "stable-project-a",
            "  "
        ));
        assert!(toggle_project_group(
            &mut preferences,
            "stable-project-b",
            ""
        ));
        assert!(toggle_project_group(
            &mut preferences,
            "stable-project-a",
            ""
        ));
        assert!(!project_group_collapsed(
            &preferences,
            "stable-project-a",
            ""
        ));
        assert!(project_group_collapsed(
            &preferences,
            "stable-project-b",
            ""
        ));
        assert!(!toggle_project_group(&mut preferences, "", ""));
    }
}

#[cfg(test)]
mod journey_tests {
    use super::*;

    fn checkout(id: &str, path: &str, connection_id: Uuid) -> AgentWorktreeOption {
        AgentWorktreeOption {
            id: id.into(),
            label: id.into(),
            path: path.into(),
            target: AgentTarget::Ssh {
                connection_id,
                connection_label: "Production".into(),
            },
            branch: None,
            is_primary: true,
        }
    }
    fn conversation(worktree: &AgentWorktreeOption, name: &str, time: i64) -> AgentSession {
        AgentSession::new(
            name,
            AgentExecutionContext {
                provider: AgentProvider::Codex,
                target: worktree.target.clone(),
                access: AgentAccessMode::ReadOnly,
                workdir: worktree.path.clone(),
                model: None,
            },
            time,
        )
        .unwrap()
    }

    // SDTEST-2819 — SDUC-564: nested projects own each conversation once, and
    // searching across project/server/message fields reveals the right branch.
    #[::core::prelude::v1::test]
    fn sdtest_2819_nested_project_search_preserves_scope_and_server_identity() {
        let server = Uuid::new_v4();
        let parent = checkout("Infrastructure", "/srv/repo", server);
        let child = checkout("Frontend", "/srv/repo/site", server);
        let other = checkout("Other server", "/srv/repo/site", Uuid::new_v4());
        let groups = vec![
            AgentProjectGroup {
                id: "parent".into(),
                label: "Infrastructure".into(),
                worktrees: vec![parent.clone()],
            },
            AgentProjectGroup {
                id: "child".into(),
                label: "Customer sites".into(),
                worktrees: vec![child.clone()],
            },
            AgentProjectGroup {
                id: "other".into(),
                label: "Other host".into(),
                worktrees: vec![other.clone()],
            },
        ];
        let mut thread = conversation(&child, "Repair deployment", 1);
        thread.context.workdir = "/srv/repo/site/assets".into();
        thread.apply_stream_event(
            AgentStreamEvent::Text("DNS propagation checked".into()),
            2,
            false,
        );
        assert_eq!(navigator_worktree(&thread, &groups).unwrap().id, child.id);
        assert!(navigator_session_matches_query(
            &thread,
            &groups,
            "CUSTOMER dns Production"
        ));
        assert!(!navigator_session_matches_query(
            &thread,
            &groups,
            "Infrastructure"
        ));
        assert!(!navigator_session_matches_query(
            &thread,
            &groups,
            "Other host"
        ));
        let another_server = conversation(&other, "Repair deployment", 3);
        assert_eq!(
            navigator_worktree(&another_server, &groups).unwrap().id,
            other.id
        );
        thread.context.workdir = "/srv/repository".into();
        assert!(navigator_worktree(&thread, &groups).is_none());
        assert!(navigator_query_matches(
            "customer sites",
            &worktree_search_text("Customer sites", &child)
        ));
    }

    // SDTEST-2820 — SDUC-564: opening a project resumes its newest exact-root
    // conversation rather than an archived thread or an unrelated child folder.
    #[::core::prelude::v1::test]
    fn sdtest_2820_project_open_prefers_recent_exact_unarchived_conversation() {
        let root = checkout("Root", "/srv/project", Uuid::new_v4());
        let old = conversation(&root, "Old", 1);
        let recent = conversation(&root, "Recent", 2);
        let mut archived = conversation(&root, "Archived", 4);
        archived.archived = true;
        let mut child = conversation(&root, "Child", 5);
        child.context.workdir = "/srv/project/child".into();
        assert_eq!(
            worktree_bound_session(
                &[archived.clone(), child.clone(), old, recent.clone()],
                &root
            ),
            Some(recent.id)
        );
        assert_eq!(
            worktree_bound_session(&[archived.clone(), child.clone()], &root),
            Some(child.id)
        );
        assert_eq!(worktree_bound_session(&[archived], &root), None);
    }

    // SDTEST-2821 — SDUC-564: create a thread directly in a project without
    // losing another draft or trusting a stale catalog action; archive/restore
    // also works for the last visible conversation without deleting its draft.
    #[::core::prelude::v1::test]
    fn sdtest_2821_project_new_session_and_last_archive_keep_drafts() {
        let mut app = gpui::TestAppContext::single();
        let view = app.new(AgentConsoleView::new);
        view.update(&mut app, |view, cx| {
            view.library = AgentLibraryUi::new(cx);
            let initial = view.sessions.selected_id().unwrap();
            view.session_ui[&initial]
                .prompt
                .update(cx, |input, _| input.content = "Keep my draft".into());
            view.model_state
                .update(cx, |input, _| input.content = "saved-draft-model".into());
            view.archive_library_session(initial, true, cx);
            assert!(view.sessions.get(initial).unwrap().archived);
            assert_eq!(
                view.sessions.get(initial).unwrap().context.model.as_deref(),
                Some("saved-draft-model")
            );
            assert!(view.sessions.selected_id().is_some_and(|id| id != initial));
            view.archive_library_session(initial, false, cx);
            assert!(!view.sessions.get(initial).unwrap().archived);
            assert_eq!(
                view.session_ui[&initial].prompt.read(cx).content(),
                "Keep my draft"
            );
            let worktree = checkout("Project", "/srv/project", Uuid::new_v4());
            view.project_groups = vec![AgentProjectGroup {
                id: "project".into(),
                label: "Project".into(),
                worktrees: vec![worktree.clone()],
            }];
            let before = view.sessions.sessions().len();
            view.new_worktree_session(&worktree, cx);
            let first = view.sessions.selected_id().unwrap();
            view.new_worktree_session(&worktree, cx);
            assert_ne!(view.sessions.selected_id(), Some(first));
            assert_eq!(view.sessions.sessions().len(), before + 2);
            assert_eq!(
                view.sessions.selected().unwrap().context.workdir,
                worktree.path
            );
            assert_eq!(
                view.sessions.selected().unwrap().context.target,
                worktree.target
            );
            assert!(view.foreground_runs.is_empty());
            let mut stale = worktree;
            stale.path = "/srv/removed".into();
            view.new_worktree_session(&stale, cx);
            assert_eq!(view.sessions.sessions().len(), before + 2);
            assert_eq!(
                view.session_ui[&initial].prompt.read(cx).content(),
                "Keep my draft"
            );
        });
    }
    // SDTEST-2831 — SDUC-564: background approvals remain visible after reading,
    // completed read conversations become quiet, and collapsing a nested project
    // counts only its own conversations without hiding the most urgent state.
    #[::core::prelude::v1::test]
    fn sdtest_2831_navigator_attention_survives_read_and_project_collapse() {
        use shelldeck_core::agent_runtime::{AgentApprovalRequest, AgentQuestionRequest};
        let server = Uuid::new_v4();
        let root = checkout("Root", "/srv/project", server);
        let child = checkout("Child", "/srv/project/child", server);
        let groups = vec![
            AgentProjectGroup {
                id: "root".into(),
                label: "Root".into(),
                worktrees: vec![root.clone()],
            },
            AgentProjectGroup {
                id: "child".into(),
                label: "Child".into(),
                worktrees: vec![child.clone()],
            },
        ];
        let mut working = conversation(&root, "Working", 1);
        working.status = AgentSessionStatus::Running;
        let mut pending = conversation(&root, "Approval", 2);
        pending.status = AgentSessionStatus::Running;
        pending.apply_stream_event(
            AgentStreamEvent::Approval(AgentApprovalRequest {
                id: "approval".into(),
                session_id: "native".into(),
                tool_call_id: "tool".into(),
                title: "Apply the edit?".into(),
                options: vec![],
            }),
            3,
            false,
        );
        pending.mark_read();
        assert_eq!(
            NavigatorState::for_session(&pending),
            NavigatorState::Approval
        );
        let mut completed = conversation(&root, "Completed", 4);
        completed
            .finish(AgentSessionStatus::Completed, 5, false)
            .unwrap();
        assert_eq!(
            NavigatorState::for_session(&completed),
            NavigatorState::Done
        );
        completed.mark_read();
        assert_eq!(
            NavigatorState::for_session(&completed),
            NavigatorState::Quiet
        );
        let mut nested = conversation(&child, "Nested failure", 6);
        nested.finish(AgentSessionStatus::Failed, 7, false).unwrap();
        let mut archived = conversation(&root, "Archived failure", 8);
        archived
            .finish(AgentSessionStatus::Failed, 9, false)
            .unwrap();
        archived.archived = true;
        let mut sessions = vec![working, pending, completed, nested, archived];
        assert_eq!(
            project_session_summary(&groups[0], &groups, &sessions, false, None),
            (3, NavigatorState::Approval)
        );
        assert_eq!(
            project_session_summary(&groups[1], &groups, &sessions, false, None),
            (1, NavigatorState::Failed)
        );
        assert_eq!(
            project_session_summary(&groups[0], &groups, &sessions, true, None),
            (4, NavigatorState::Approval)
        );
        // Native imports outside the catalog use the same attention summary.
        let mut imported = sessions[1].clone();
        imported.context.workdir = "/srv/uncatalogued".into();
        sessions.push(imported);
        assert_eq!(
            session_group_summary(sessions.iter().filter(|session| {
                navigator_worktree(session, &groups).is_none() && !session.archived
            })),
            (1, NavigatorState::Approval)
        );
        assert_eq!(
            project_session_summary(&groups[0], &groups, &sessions, false, None),
            (3, NavigatorState::Approval)
        );
        sessions[1].approval = None;
        sessions[1].apply_stream_event(
            AgentStreamEvent::Question(AgentQuestionRequest {
                id: "question".into(),
                prompt: "Which environment?".into(),
                options: vec![],
                multiple: false,
                response_supported: false,
            }),
            10,
            false,
        );
        sessions[1].mark_read();
        assert_eq!(
            project_session_summary(&groups[0], &groups, &sessions, false, None),
            (3, NavigatorState::Input)
        );
        sessions[1]
            .finish(AgentSessionStatus::Completed, 11, true)
            .unwrap();
        sessions[1].acknowledge_attention();
        assert_eq!(
            project_session_summary(&groups[0], &groups, &sessions, true, None),
            (4, NavigatorState::Working)
        );
        assert_eq!(
            NavigatorState::for_session(&sessions[4]),
            NavigatorState::Quiet
        );
    }
    // SDTEST-2833 — SDUC-564: background row actions target their own UUID,
    // without replacing the selected conversation's unsent model or draft.
    #[::core::prelude::v1::test]
    fn sdtest_2833_background_row_actions_preserve_selected_draft() {
        let mut app = gpui::TestAppContext::single();
        let view = app.new(AgentConsoleView::new);
        view.update(&mut app, |view, cx| {
            view.library = AgentLibraryUi::new(cx);
            let selected = view.sessions.selected_id().unwrap();
            let context = view.sessions.selected().unwrap().context.clone();
            let other = view.create_session("Other", context, cx).unwrap();
            view.select_session(selected, cx);
            view.session_ui[&selected]
                .prompt
                .update(cx, |input, _| input.content = "Keep this draft".into());
            view.model_state
                .update(cx, |input, _| input.content = "unsent-model".into());
            view.perform_conversation_action(other, ConversationAction::TogglePin, cx);
            assert!(view.sessions.get(other).unwrap().pinned);
            view.perform_conversation_action(other, ConversationAction::Rename, cx);
            assert_eq!(view.library.rename_session, Some(other));
            view.library
                .rename_state
                .update(cx, |input, _| input.content = "Renamed background".into());
            view.rename_library_session(cx);
            assert_eq!(view.sessions.get(other).unwrap().name, "Renamed background");
            assert_eq!(view.sessions.selected_id(), Some(selected));
            assert_eq!(
                view.session_ui[&selected].prompt.read(cx).content(),
                "Keep this draft"
            );
            assert_eq!(view.model_state.read(cx).content(), "unsent-model");
            view.perform_conversation_action(other, ConversationAction::ToggleArchive, cx);
            assert!(view.sessions.get(other).unwrap().archived);
            assert_eq!(view.library.undo_archive, Some(other));
            assert_eq!(view.model_state.read(cx).content(), "unsent-model");
            view.undo_library_archive(cx);
            assert!(!view.sessions.get(other).unwrap().archived);
            assert_eq!(view.sessions.selected_id(), Some(selected));
            assert_eq!(view.model_state.read(cx).content(), "unsent-model");
            assert_eq!(view.library.undo_archive, None);
            view.sessions.get_mut(other).unwrap().remote_history =
                Some(shelldeck_core::agent_session::RemoteHistoryReference {
                    connection_id: Uuid::new_v4(),
                    provider: AgentProvider::Codex,
                    session_id: Uuid::new_v4().to_string(),
                    workdir: "/srv/project".into(),
                    model: None,
                    snapshot: None,
                });
            let count = view.sessions.sessions().len();
            for action in [
                ConversationAction::Fork,
                ConversationAction::Copy,
                ConversationAction::ExportMarkdown,
                ConversationAction::ExportJson,
            ] {
                assert!(!action.enabled(view.sessions.get(other).unwrap()));
                view.perform_conversation_action(other, action, cx);
            }
            assert_eq!(view.sessions.sessions().len(), count);
            assert_eq!(view.sessions.selected_id(), Some(selected));
            view.sessions.get_mut(other).unwrap().apply_stream_event(
                AgentStreamEvent::Text("A local turn".into()),
                20,
                true,
            );
            assert!(ConversationAction::Copy.enabled(view.sessions.get(other).unwrap()));
        });
    }

    // SDTEST-2834 — SDUC-564: stale menu actions cannot archive a newly active
    // run; Undo restores the latest archived conversation with its draft intact.
    #[::core::prelude::v1::test]
    fn sdtest_2834_archive_menu_revalidates_run_and_undo_keeps_last_draft() {
        let mut app = gpui::TestAppContext::single();
        let view = app.new(AgentConsoleView::new);
        view.update(&mut app, |view, cx| {
            view.library = AgentLibraryUi::new(cx);
            let id = view.sessions.selected_id().unwrap();
            view.session_ui[&id]
                .prompt
                .update(cx, |input, _| input.content = "A recoverable draft".into());
            view.sessions.get_mut(id).unwrap().status = AgentSessionStatus::Running;
            view.perform_conversation_action(id, ConversationAction::ToggleArchive, cx);
            assert!(!view.sessions.get(id).unwrap().archived);
            assert_eq!(view.sessions.selected_id(), Some(id));
            assert_eq!(view.library.undo_archive, None);
            view.sessions.get_mut(id).unwrap().status = AgentSessionStatus::Completed;
            view.perform_conversation_action(id, ConversationAction::ToggleArchive, cx);
            assert!(view.sessions.get(id).unwrap().archived);
            let replacement = view.sessions.selected_id().unwrap();
            assert_ne!(replacement, id);
            view.undo_library_archive(cx);
            assert!(!view.sessions.get(id).unwrap().archived);
            assert_eq!(view.sessions.selected_id(), Some(replacement));
            assert_eq!(
                view.session_ui[&id].prompt.read(cx).content(),
                "A recoverable draft"
            );
            view.undo_library_archive(cx);
            assert_eq!(view.sessions.selected_id(), Some(replacement));
        });
    }

    // SDTEST-2835 — SDUC-564: a background conversation copy opens with that
    // conversation's context while retaining the previous unsent context.
    #[::core::prelude::v1::test]
    fn sdtest_2835_row_fork_restores_source_context_and_retains_previous_draft() {
        let mut app = gpui::TestAppContext::single();
        let view = app.new(AgentConsoleView::new);
        view.update(&mut app, |view, cx| {
            view.library = AgentLibraryUi::new(cx);
            let previous = view.sessions.selected_id().unwrap();
            let mut context = view.sessions.selected().unwrap().context.clone();
            context.provider = AgentProvider::Claude;
            context.model = Some("source-model".into());
            let source = view
                .create_session("Copy source", context.clone(), cx)
                .unwrap();
            view.select_session(previous, cx);
            view.session_ui[&previous]
                .prompt
                .update(cx, |input, _| input.content = "Previous draft".into());
            view.model_state.update(cx, |input, _| {
                input.content = "previous-unsent-model".into()
            });
            view.perform_conversation_action(source, ConversationAction::Fork, cx);
            let copy = view.sessions.selected_id().unwrap();
            assert_ne!(copy, source);
            assert_ne!(copy, previous);
            assert_eq!(view.sessions.get(copy).unwrap().context, context);
            assert_eq!(view.displayed_context(cx), context);
            assert_eq!(
                view.sessions
                    .get(previous)
                    .unwrap()
                    .context
                    .model
                    .as_deref(),
                Some("previous-unsent-model")
            );
            assert_eq!(
                view.session_ui[&previous].prompt.read(cx).content(),
                "Previous draft"
            );
            assert!(view.foreground_runs.is_empty());
        });
    }
}
