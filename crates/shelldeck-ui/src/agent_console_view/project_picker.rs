//! Project navigation uses retained sessions, never edits a bound execution context.
use super::*;

fn compact_label(value: &str, limit: usize) -> String {
    if value.chars().count() <= limit {
        value.to_owned()
    } else {
        format!(
            "{}…",
            value
                .chars()
                .take(limit.saturating_sub(1))
                .collect::<String>()
        )
    }
}

fn picker_target(target: &AgentTarget) -> String {
    match target {
        AgentTarget::Local => target_label(target),
        AgentTarget::Ssh { .. } => {
            format!("{} · {}", t!("agents.project.ssh"), target_label(target))
        }
    }
}

fn project_checkout_label(project: &str, checkout: &str) -> String {
    if project == checkout {
        project.to_owned()
    } else {
        format!("{project} / {checkout}")
    }
}

impl AgentConsoleView {
    pub(super) fn render_project_picker(&self, cx: &mut Context<Self>) -> AnyElement {
        let context = self.displayed_context(cx);
        let missing = match &context.target {
            AgentTarget::Ssh { connection_id, .. }
                if !self
                    .connections
                    .iter()
                    .any(|connection| connection.id == *connection_id) =>
            {
                Some(*connection_id)
            }
            _ => None,
        };
        let worktree = self.project_groups.iter().find_map(|project| {
            project
                .worktrees
                .iter()
                .find(|worktree| {
                    same_target_authority(&context.target, &worktree.target)
                        && context.workdir == worktree.path
                })
                .map(|worktree| project_checkout_label(&project.label, &worktree.label))
        });
        let label = worktree.unwrap_or_else(|| {
            PathBuf::from(&context.workdir)
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or(&context.workdir)
                .to_owned()
        });
        let target = picker_target(&context.target);
        let parent = cx.entity();
        let picker = Popover::new("agent-project-picker")
            .anchor(Corner::BottomLeft)
            .trigger(
                Button::new("agent-project-trigger", compact_label(&label, 36))
                    .variant(ButtonVariant::Ghost)
                    .size(ButtonSize::Sm)
                    .icon(IconSource::from("folder"))
                    .max_w_full()
                    .overflow_hidden()
                    .tooltip(format!(
                        "{} · {}\n{}",
                        t!("agents.project.label"),
                        target,
                        context.workdir
                    )),
            )
            .content(move |window, cx| {
                let parent = parent.clone();
                cx.new(move |content_cx| {
                    let search = content_cx.new(InputState::new);
                    content_cx.observe(&parent, |_, _, cx| cx.notify()).detach();
                    // Popover owns the search lifetime: background session updates
                    // must not replace a focused search field or its query.
                    PopoverContent::new(window, content_cx, move |window, cx| {
                        let query = search.read(cx).content().trim().to_lowercase();
                        let view = parent.read(cx);
                        let selected = view.sessions.selected_id();
                        let mut matches = 0;
                        let mut rows = div()
                            .id("agent-project-results")
                            .flex()
                            .flex_col()
                            .min_h_0()
                            .max_h(px(320.0))
                            .overflow_y_scroll()
                            .gap(px(3.0));
                        for project in &view.project_groups {
                            for worktree in &project.worktrees {
                                let label = format!(
                                    "{} · {}",
                                    project_checkout_label(&project.label, &worktree.label),
                                    picker_target(&worktree.target)
                                );
                                let search_text = format!(
                                    "{} {} {}",
                                    label,
                                    worktree.path,
                                    worktree.branch.as_deref().unwrap_or_default()
                                );
                                if !search_text.to_lowercase().contains(&query) {
                                    continue;
                                }
                                matches += 1;
                                let option = worktree.clone();
                                let choose = parent.clone();
                                let dismiss = cx.entity().downgrade();
                                rows = rows.child(
                                    Button::new(
                                        ("agent-project-choice", stable_string_key(&worktree.id)),
                                        compact_label(&label, 48),
                                    )
                                    .variant(ButtonVariant::Ghost)
                                    .size(ButtonSize::Sm)
                                    .w_full()
                                    .min_w_0()
                                    .justify_start()
                                    .icon(IconSource::from(
                                        if matches!(worktree.target, AgentTarget::Local) {
                                            "folder"
                                        } else {
                                            "server"
                                        },
                                    ))
                                    .overflow_hidden()
                                    .tooltip(search_text)
                                    .on_click(
                                        move |_, _, cx| {
                                            choose.update(cx, |view, cx| {
                                                // Revalidate the live catalog; a stale popup must
                                                // not create a session for a removed/changed checkout.
                                                let current = view
                                                    .project_groups
                                                    .iter()
                                                    .flat_map(|p| &p.worktrees)
                                                    .find(|w| w.id == option.id && **w == option)
                                                    .cloned();
                                                if let Some(current) = current {
                                                    view.select_worktree_session(&current, cx);
                                                }
                                            });
                                            let _ =
                                                dismiss.update(cx, |_, cx| cx.emit(DismissEvent));
                                        },
                                    ),
                                );
                            }
                        }
                        let mut recent = view.sessions.library_sessions(false);
                        recent.sort_by_key(|session| std::cmp::Reverse(session.updated_at_ms));
                        let recent = recent
                            .into_iter()
                            .filter(|session| {
                                format!(
                                    "{} {} {}",
                                    session.name,
                                    session.context.workdir,
                                    picker_target(&session.context.target)
                                )
                                .to_lowercase()
                                .contains(&query)
                            })
                            .collect::<Vec<_>>();
                        if !recent.is_empty() {
                            rows = rows.child(
                                div()
                                    .pt(px(8.0))
                                    .text_size(px(10.0))
                                    .text_color(ShellDeckColors::text_muted())
                                    .child(t!("agents.project.recent").to_string()),
                            );
                        }
                        for session in recent {
                            matches += 1;
                            let id = session.id;
                            let label = format!(
                                "{} · {} · {}",
                                PathBuf::from(&session.context.workdir)
                                    .file_name()
                                    .and_then(|name| name.to_str())
                                    .unwrap_or(&session.context.workdir),
                                session.name,
                                picker_target(&session.context.target)
                            );
                            let choose = parent.clone();
                            let dismiss = cx.entity().downgrade();
                            rows = rows.child(
                                Button::new(
                                    ("agent-project-session", stable_string_key(&id.to_string())),
                                    compact_label(&label, 48),
                                )
                                .variant(if selected == Some(id) {
                                    ButtonVariant::Secondary
                                } else {
                                    ButtonVariant::Ghost
                                })
                                .size(ButtonSize::Sm)
                                .w_full()
                                .min_w_0()
                                .justify_start()
                                .icon(IconSource::from(
                                    if matches!(session.context.target, AgentTarget::Local) {
                                        "folder"
                                    } else {
                                        "server"
                                    },
                                ))
                                .overflow_hidden()
                                .tooltip(format!("{}\n{}", label, session.context.workdir))
                                .on_click(move |_, _, cx| {
                                    choose.update(cx, |view, cx| {
                                        if view
                                            .sessions
                                            .get(id)
                                            .is_some_and(|session| !session.archived)
                                        {
                                            view.select_session(id, cx);
                                        }
                                    });
                                    let _ = dismiss.update(cx, |_, cx| cx.emit(DismissEvent));
                                }),
                            );
                        }
                        if matches == 0 {
                            rows = rows.child(
                                div()
                                    .p(px(8.0))
                                    .text_size(px(12.0))
                                    .text_color(ShellDeckColors::text_muted())
                                    .child(t!("agents.project.no_matches").to_string()),
                            );
                        }
                        let notify = cx.entity().downgrade();
                        let launch = parent.clone();
                        let dismiss = cx.entity().downgrade();
                        div()
                            .w(px(360.0))
                            .max_w_full()
                            .max_h(gpui::px(
                                (window.viewport_size().height.to_f64() as f32 - 80.0).max(120.0),
                            ))
                            .overflow_hidden()
                            .flex()
                            .flex_col()
                            .min_w_0()
                            .gap(px(8.0))
                            .child(
                                Input::new(&search)
                                    .size(InputSize::Sm)
                                    .placeholder(t!("agents.project.search").to_string())
                                    .on_change(move |_, cx| {
                                        let _ = notify.update(cx, |_, cx| cx.notify());
                                    }),
                            )
                            .child(rows)
                            .child(
                                Button::new(
                                    "agent-project-create",
                                    t!("agents.project.create").to_string(),
                                )
                                .variant(ButtonVariant::Ghost)
                                .size(ButtonSize::Sm)
                                .w_full()
                                .justify_start()
                                .icon(IconSource::from("plus"))
                                .on_click(move |_, _, cx| {
                                    launch.update(cx, |_, cx| {
                                        cx.emit(AgentConsoleEvent::OpenProjectSetup)
                                    });
                                    let _ = dismiss.update(cx, |_, cx| cx.emit(DismissEvent));
                                }),
                            )
                            .into_any_element()
                    })
                })
            });
        let row = div()
            .w_full()
            .min_w_0()
            .flex()
            .items_center()
            .gap(px(6.0))
            .child(
                div()
                    .min_w_0()
                    .max_w(px(300.0))
                    .overflow_hidden()
                    .child(picker),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(px(10.5))
                    .text_color(ShellDeckColors::text_muted())
                    .child(target),
            )
            .into_any_element();
        let mut result = div()
            .w_full()
            .min_w_0()
            .flex()
            .flex_col()
            .gap(px(6.0))
            .child(row);
        if let Some(connection_id) = missing {
            result =
                result.child(self.render_connection_repair_picker(&context, connection_id, cx));
        }
        result.into_any_element()
    }
    fn render_connection_repair_picker(
        &self,
        context: &AgentExecutionContext,
        missing: Uuid,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let parent = cx.entity();
        let expected_context = context.clone();
        let expected_session = self.sessions.selected_id();
        let locked = self.context_controls_locked();
        let picker = Popover::new("agent-reconnect-ssh")
            .anchor(Corner::BottomLeft)
            .trigger(Button::new("agent-reconnect-trigger", t!("agents.project.reconnect").to_string())
                .variant(ButtonVariant::Outline).size(ButtonSize::Sm)
                .icon(IconSource::from("server")).disabled(locked)
                .tooltip(if locked { t!("agents.project.reconnect_blocked").to_string() }
                    else { context.workdir.clone() }))
            .content(move |window, cx| {
                let parent = parent.clone();
                let expected_context = expected_context.clone();
                cx.new(move |content_cx| {
                    let search = content_cx.new(InputState::new);
                    content_cx.observe(&parent, |_, _, cx| cx.notify()).detach();
                    PopoverContent::new(window, content_cx, move |window, cx| {
                        let query = search.read(cx).content().trim().to_lowercase();
                        let view = parent.read(cx);
                        let current = view.sessions.selected_id() == expected_session
                            && view.displayed_context(cx) == expected_context
                            && !view.context_controls_locked();
                        let mut checkouts: Vec<_> = view.project_groups.iter().flat_map(|project| {
                            project.worktrees.iter().filter_map(|tree| {
                                (tree.path == expected_context.workdir
                                    && matches!(&tree.target, AgentTarget::Ssh {connection_id, ..} if *connection_id == missing))
                                    .then(|| uuid::Uuid::parse_str(&tree.id).ok().map(|id| (
                                        Some(shelldeck_core::config::workspace_catalog::CatalogCheckoutId::from_uuid(id)),
                                        project.label.clone(),
                                    ))).flatten()
                            })
                        }).collect();
                        if checkouts.is_empty() { checkouts.push((None, String::new())); }
                        let mut rows = div().id("agent-reconnect-options").flex().flex_col()
                            .min_h_0().max_h(px(280.0)).overflow_y_scroll().gap(px(4.0));
                        let mut matches = 0;
                        for (checkout, project_label) in &checkouts {
                            for connection in &view.connections {
                                let label = if checkouts.len() > 1 {
                                    format!("{project_label} · {} — {}", connection.label, connection.host)
                                } else { format!("{} — {}", connection.label, connection.host) };
                                if !label.to_lowercase().contains(&query) { continue; }
                                matches += 1;
                                let checkout = *checkout;
                                let connection_id = connection.id;
                                let choose = parent.clone();
                                let expected_context = expected_context.clone();
                                let dismiss = cx.entity().downgrade();
                                rows = rows.child(Button::new(
                                    ("agent-reconnect-option", stable_string_key(&format!("{checkout:?}:{connection_id}"))), label,
                                ).variant(ButtonVariant::Ghost).size(ButtonSize::Sm).w_full()
                                    .justify_start().overflow_hidden().disabled(!current)
                                    .on_click(move |_, _, cx| {
                                        choose.update(cx, |view, cx| {
                                            if view.sessions.selected_id() != expected_session
                                                || view.displayed_context(cx) != expected_context
                                                || view.context_controls_locked()
                                                || !view.connections.iter().any(|item| item.id == connection_id) { return; }
                                            if let Some(checkout_id) = checkout {
                                                cx.emit(AgentConsoleEvent::ReconnectProject {
                                                    checkout_id, expected_connection: missing, connection_id,
                                                });
                                            } else {
                                                // A conversation without a catalog project still needs
                                                // an explicit target choice, never a Local fallback.
                                                view.selected_connection = Some(connection_id);
                                                view.retain_draft_context(cx);
                                                view.apply_selected_context(cx);
                                                view.schedule_library_save(cx);
                                            }
                                        });
                                        let _ = dismiss.update(cx, |_, cx| cx.emit(DismissEvent));
                                    }));
                            }
                        }
                        if view.connections.is_empty() {
                            rows = rows.child(t!("agents.project.no_connections").to_string());
                        } else if matches == 0 {
                            rows = rows.child(t!("agents.project.no_connections_match").to_string());
                        }
                        let notify = cx.entity().downgrade();
                        div().w(px(360.0)).max_w_full()
                            .max_h(gpui::px((window.viewport_size().height.to_f64() as f32 - 80.0).max(120.0)))
                            .flex().flex_col().min_w_0().overflow_hidden().gap(px(8.0))
                            .child(div().text_size(px(12.0)).whitespace_normal().child(t!("agents.project.reconnect_hint").to_string()))
                            .child(Input::new(&search).size(InputSize::Sm)
                                .placeholder(t!("agents.target.search").to_string())
                                .on_change(move |_, cx| { let _ = notify.update(cx, |_, cx| cx.notify()); }))
                            .child(rows).into_any_element()
                    })
                })
            });
        div()
            .w_full()
            .min_w_0()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(8.0))
            .child(picker)
            .child(
                div()
                    .min_w_0()
                    .flex_1()
                    .truncate()
                    .text_size(px(11.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(context.workdir.clone()),
            )
            .into_any_element()
    }
}
