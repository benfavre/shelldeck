//! Project navigation actions. Removing metadata never deletes a checkout.
use super::navigator::navigator_worktree;
use super::*;

impl AgentConsoleView {
    pub(super) fn set_navigator_groups_collapsed(
        &mut self,
        collapsed: bool,
        cx: &mut Context<Self>,
    ) {
        if collapsed {
            for key in self.navigator_group_keys() {
                if !self
                    .library
                    .preferences
                    .collapsed_project_ids
                    .contains(&key)
                {
                    self.library.preferences.toggle_project_collapsed(&key);
                }
            }
        } else {
            self.library.preferences.collapsed_project_ids.clear();
        }
        self.library.preferences.ssh_sessions_collapsed = collapsed;
        self.library.preferences.remote_sessions_expanded = !collapsed;
        self.schedule_library_save(cx);
        cx.notify();
    }

    pub(super) fn navigator_actions_button(&self, cx: &Context<Self>) -> AnyElement {
        let parent = cx.entity();
        Popover::new("agent-navigator-actions")
            .anchor(Corner::TopRight)
            .trigger(
                Button::new("agent-navigator-actions-trigger", "")
                    .debug_selector(|| "agent-navigator-actions-trigger".into())
                    .variant(ButtonVariant::Ghost)
                    .size(ButtonSize::Icon)
                    .icon("ellipsis")
                    .w(px(28.0))
                    .h(px(28.0))
                    .tooltip(t!("agents.navigator.actions").to_string()),
            )
            .content(move |window, cx| {
                let parent = parent.clone();
                cx.new(move |cx| {
                    PopoverContent::new(window, cx, move |_, cx| {
                        let searching = !parent
                            .read(cx)
                            .navigator_search
                            .read(cx)
                            .content()
                            .trim()
                            .is_empty();
                        let mut menu = div().w(px(220.0)).flex().flex_col().gap(px(4.0));
                        let setup_parent = parent.clone();
                        menu = menu.child(
                            Button::new(
                                "agent-nav-add-project",
                                t!("agents.project.create").to_string(),
                            )
                            .debug_selector(|| "agent-nav-add-project".into())
                            .variant(ButtonVariant::Ghost)
                            .size(ButtonSize::Sm)
                            .icon("folder")
                            .w_full()
                            .justify_start()
                            .on_click(cx.listener(
                                move |_, _, _, cx| {
                                    setup_parent.update(cx, |_, cx| {
                                        cx.emit(AgentConsoleEvent::OpenProjectSetup)
                                    });
                                    cx.emit(DismissEvent);
                                },
                            )),
                        );
                        for collapsed in [true, false] {
                            let parent = parent.clone();
                            let id = if collapsed {
                                "agent-collapse-all"
                            } else {
                                "agent-expand-all"
                            };
                            menu = menu.child(
                                Button::new(
                                    id,
                                    t!(if collapsed {
                                        "agents.navigator.collapse_all"
                                    } else {
                                        "agents.navigator.expand_all"
                                    })
                                    .to_string(),
                                )
                                .debug_selector(move || id.to_owned())
                                .variant(ButtonVariant::Ghost)
                                .size(ButtonSize::Sm)
                                .icon(if collapsed {
                                    "chevron-right"
                                } else {
                                    "chevron-down"
                                })
                                .w_full()
                                .justify_start()
                                .disabled(searching)
                                .on_click(cx.listener(
                                    move |_, _, _, cx| {
                                        parent.update(cx, |view, cx| {
                                            view.set_navigator_groups_collapsed(collapsed, cx)
                                        });
                                        cx.emit(DismissEvent);
                                    },
                                )),
                            );
                        }
                        let archived = parent.read(cx).library.show_archived();
                        let archive_parent = parent.clone();
                        menu.child(
                            Button::new(
                                "agent-library-archived-filter",
                                t!(if archived {
                                    "agents.navigator.show_active"
                                } else {
                                    "agents.library.show_archived"
                                })
                                .to_string(),
                            )
                            .debug_selector(|| "agent-library-archived-filter".into())
                            .variant(ButtonVariant::Ghost)
                            .size(ButtonSize::Sm)
                            .icon(if archived {
                                "archive-restore"
                            } else {
                                "archive"
                            })
                            .w_full()
                            .justify_start()
                            .on_click(cx.listener(
                                move |_, _, _, cx| {
                                    archive_parent.update(cx, |view, cx| {
                                        view.library.preferences.show_archived =
                                            !view.library.preferences.show_archived;
                                        view.schedule_library_save(cx);
                                        cx.notify();
                                    });
                                    cx.emit(DismissEvent);
                                },
                            )),
                        )
                        .into_any_element()
                    })
                })
            })
            .into_any_element()
    }

    pub(crate) fn can_remove_navigator_project(&self, id: &str) -> bool {
        self.project_groups.iter().any(|group| group.id == id)
            && !self
                .library
                .remove_project
                .as_ref()
                .is_some_and(|expected| {
                    expected.id == id && !self.project_groups.contains(expected)
                })
            && !self.pending_confirm.as_ref().is_some_and(|pending| {
                self.sessions
                    .get(pending.session_id)
                    .is_some_and(|session| self.session_in_project(session, id))
            })
            && !self
                .sessions
                .sessions()
                .iter()
                .any(|session| session.status.is_active() && self.session_in_project(session, id))
    }

    fn session_in_project(&self, session: &AgentSession, id: &str) -> bool {
        navigator_worktree(session, &self.project_groups).is_some_and(|owner| {
            self.project_groups.iter().any(|group| {
                group.id == id && group.worktrees.iter().any(|tree| tree.id == owner.id)
            })
        })
    }

    pub fn complete_navigator_project_removal(
        &mut self,
        id: &str,
        result: Result<(), String>,
        cx: &mut Context<Self>,
    ) {
        if self
            .library
            .remove_project
            .as_ref()
            .is_none_or(|group| group.id != id)
        {
            return;
        }
        if let Err(error) = result {
            self.library.error = Some(error);
            cx.notify();
            return;
        }
        self.retain_draft_context(cx);
        let ids: Vec<_> = self
            .sessions
            .sessions()
            .iter()
            .filter(|session| self.session_in_project(session, id))
            .map(|session| session.id)
            .collect();
        for session in ids {
            // Dispatch revalidates activity before removing the catalog entry.
            let _ = self.sessions.archive(session, true, now_ms());
        }
        self.project_groups.retain(|group| group.id != id);
        self.library
            .preferences
            .collapsed_project_ids
            .retain(|key| key != id);
        self.library.remove_project = None;
        self.library.search_open = false;
        self.navigator_search
            .update(cx, |state, cx| state.replace_content(String::new(), cx));
        self.library.error = None;
        self.library.notice = Some(t!("agents.project.removed").to_string());
        if self.sessions.selected_id().is_none() {
            let context = AgentExecutionContext {
                provider: self.provider,
                access: AgentAccessMode::ReadOnly,
                target: AgentTarget::Local,
                workdir: self.default_local_workdir.clone(),
                model: None,
            };
            let _ = self.create_session(t!("agents.session.new").to_string(), context, cx);
        } else {
            self.apply_selected_context(cx);
        }
        self.schedule_library_save(cx);
        cx.notify();
    }

    pub(super) fn project_actions_button(
        &self,
        id: &str,
        hover_group: SharedString,
        cx: &Context<Self>,
    ) -> AnyElement {
        let parent = cx.entity();
        let id = id.to_owned();
        let selector = format!("agent-project-menu-{id}");
        Popover::new(SharedString::from(selector.clone()))
            .anchor(Corner::TopRight)
            .trigger(
                // Hide only the trigger; the open menu must survive leaving the header.
                div()
                    .flex_shrink_0()
                    .opacity(0.0)
                    .group_hover(hover_group, |style| style.opacity(1.0))
                    .child(
                        Button::new(SharedString::from(selector.clone()), "")
                            .debug_selector(move || selector.clone())
                            .variant(ButtonVariant::Ghost)
                            .hover_background(gpui::transparent_black())
                            .size(ButtonSize::Icon)
                            .icon(IconSource::from("ellipsis"))
                            .w(px(24.0))
                            .h(px(24.0))
                            .flex_shrink_0()
                            .tooltip(t!("agents.project.actions").to_string()),
                    ),
            )
            .content(move |window, cx| {
                let parent = parent.clone();
                let id = id.clone();
                cx.new(move |cx| {
                    cx.observe(&parent, |_, _, cx| cx.notify()).detach();
                    PopoverContent::new(window, cx, move |_, cx| {
                        let view = parent.read(cx);
                        let Some(project) = view.project_groups.iter().find(|group| group.id == id)
                        else {
                            return div().into_any_element();
                        };
                        let mut menu = div().w(px(250.0)).flex().flex_col().gap(px(4.0)).child(
                            div()
                                .px(px(8.0))
                                .py(px(5.0))
                                .text_size(px(12.0))
                                .font_weight(FontWeight::SEMIBOLD)
                                .truncate()
                                .child(project.label.clone()),
                        );
                        let rename_parent = parent.clone();
                        let rename_id = id.clone();
                        let name = project.label.clone();
                        menu = menu.child(
                            Button::new(
                                "agent-project-rename",
                                t!("agents.project.rename").to_string(),
                            )
                            .debug_selector(|| "agent-project-rename".into())
                            .variant(ButtonVariant::Ghost)
                            .size(ButtonSize::Sm)
                            .w_full()
                            .justify_start()
                            .icon("pencil")
                            .on_click(cx.listener(
                                move |_, _, window, cx| {
                                    rename_parent.update(cx, |view, cx| {
                                        view.open_project_rename(
                                            rename_id.clone(),
                                            name.clone(),
                                            window,
                                            cx,
                                        );
                                    });
                                    cx.emit(DismissEvent);
                                },
                            )),
                        );
                        if let [tree] = project.worktrees.as_slice() {
                            menu = menu.child(
                                div()
                                    .px(px(8.0))
                                    .py(px(4.0))
                                    .min_w_0()
                                    .text_size(px(11.0))
                                    .text_color(ShellDeckColors::text_muted())
                                    .overflow_hidden()
                                    .text_ellipsis()
                                    .child(tree.path.clone()),
                            );
                            if let Some(branch) = &tree.branch {
                                menu = menu.child(
                                    div()
                                        .px(px(8.0))
                                        .min_w_0()
                                        .text_size(px(11.0))
                                        .text_color(ShellDeckColors::text_muted())
                                        .overflow_hidden()
                                        .text_ellipsis()
                                        .child(branch.clone()),
                                );
                            }
                        }
                        if let Some(tree) = project.worktrees.first() {
                            let tree_new = tree.clone();
                            let path = tree.path.clone();
                            let action_parent = parent.clone();
                            menu = menu
                                .child(
                                    Button::new(
                                        "agent-project-action-new",
                                        t!("agents.session.new").to_string(),
                                    )
                                    .debug_selector(|| "agent-project-action-new".into())
                                    .variant(ButtonVariant::Ghost)
                                    .size(ButtonSize::Sm)
                                    .w_full()
                                    .justify_start()
                                    .icon("plus")
                                    .on_click(cx.listener(
                                        move |_, _, _, cx| {
                                            action_parent.update(cx, |view, cx| {
                                                view.new_worktree_session(&tree_new, cx)
                                            });
                                            cx.emit(DismissEvent);
                                        },
                                    )),
                                )
                                .child(
                                    Button::new(
                                        "agent-project-copy-path",
                                        t!("agents.project.copy_path").to_string(),
                                    )
                                    .debug_selector(|| "agent-project-copy-path".into())
                                    .variant(ButtonVariant::Ghost)
                                    .size(ButtonSize::Sm)
                                    .w_full()
                                    .justify_start()
                                    .icon("copy")
                                    .on_click(cx.listener(
                                        move |_, _, _, cx| {
                                            cx.write_to_clipboard(ClipboardItem::new_string(
                                                path.clone(),
                                            ));
                                            cx.emit(DismissEvent);
                                        },
                                    )),
                                );
                        }
                        let removable = view.can_remove_navigator_project(&id);
                        let project = project.clone();
                        let action_parent = parent.clone();
                        menu.child(
                            Button::new(
                                "agent-project-remove",
                                t!("workspaces.projects.remove").to_string(),
                            )
                            .debug_selector(|| "agent-project-remove".into())
                            .variant(ButtonVariant::Ghost)
                            .size(ButtonSize::Sm)
                            .w_full()
                            .justify_start()
                            .icon("trash-2")
                            .text_color(ShellDeckColors::error())
                            .disabled(!removable)
                            .tooltip(
                                t!(if removable {
                                    "agents.project.remove_hint"
                                } else {
                                    "agents.project.running"
                                })
                                .to_string(),
                            )
                            .on_click(cx.listener(
                                move |_, _, _, cx| {
                                    action_parent.update(cx, |view, cx| {
                                        if view.can_remove_navigator_project(&project.id) {
                                            view.library.remove_project = Some(project.clone());
                                            view.library.error = None;
                                            cx.notify();
                                        }
                                    });
                                    cx.emit(DismissEvent);
                                },
                            )),
                        )
                        .into_any_element()
                    })
                })
            })
            .into_any_element()
    }

    pub(super) fn submit_project_rename(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.library.rename_project.clone() else {
            return;
        };
        let name = self
            .library
            .project_name
            .read(cx)
            .content()
            .trim()
            .to_owned();
        if name.is_empty() {
            self.library.error = Some(t!("agents.project.name_required").to_string());
            cx.notify();
            return;
        }
        cx.emit(AgentConsoleEvent::RenameProject { id, name });
    }

    pub fn complete_navigator_project_rename(
        &mut self,
        id: &str,
        result: Result<(), String>,
        cx: &mut Context<Self>,
    ) {
        if self.library.rename_project.as_deref() != Some(id) {
            return;
        }
        match result {
            Ok(()) => {
                self.cancel_project_rename(cx);
            }
            Err(error) => self.library.error = Some(error),
        }
        cx.notify();
    }

    fn open_project_rename(
        &mut self,
        id: String,
        name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.library.rename_project = Some(id);
        self.library.project_name = input_state(cx, name);
        self.library.error = None;
        let parent = cx.weak_entity();
        let close_parent = parent.clone();
        let observed_parent = cx.entity();
        self.library.project_rename_dialog = Some(cx.new(|cx| {
            cx.observe(&observed_parent, |_, _, cx| cx.notify())
                .detach();
            adabraka_ui::overlays::dialog::Dialog::new(cx)
                .title(t!("agents.project.rename").to_string())
                .size(adabraka_ui::overlays::dialog::DialogSize::Sm)
                .body(move |_, cx| {
                    parent
                        .update(cx, |view, cx| view.render_project_rename_body(cx))
                        .unwrap_or_else(|_| div().into_any_element())
                })
                .on_close(move |_, cx| {
                    let _ = close_parent.update(cx, |view, cx| view.cancel_project_rename(cx));
                })
        }));
        let parent = cx.weak_entity();
        window.on_next_frame(move |window, _| {
            // Dialog takes initial focus on its first render; focus its field afterward.
            window.on_next_frame(move |window, cx| {
                let _ = parent.update(cx, |view, cx| {
                    if view.library.rename_project.is_some() {
                        window.focus(&view.library.project_name.read(cx).focus_handle(cx));
                    }
                });
            });
        });
        cx.notify();
    }

    fn cancel_project_rename(&mut self, cx: &mut Context<Self>) {
        self.library.rename_project = None;
        self.library.project_rename_dialog = None;
        self.library.error = None;
        self.request_surface_focus(cx);
    }

    pub(super) fn render_project_rename(&self, _cx: &Context<Self>) -> Option<AnyElement> {
        self.library.rename_project.as_ref()?;
        self.library
            .project_rename_dialog
            .clone()
            .map(|dialog| dialog.into_any_element())
    }

    fn render_project_rename_body(&self, cx: &Context<Self>) -> AnyElement {
        let enter_parent = cx.weak_entity();
        div()
            .w_full()
            .p(px(24.0))
            .flex()
            .flex_col()
            .min_h(px(0.0))
            .gap(px(12.0))
            .capture_action(cx.listener(|view, _: &crate::overlay::InputEscape, _, cx| {
                view.cancel_project_rename(cx);
                cx.stop_propagation();
            }))
            .child(
                Input::new(&self.library.project_name)
                    .debug_selector("agent-project-name")
                    .w_full()
                    .placeholder(t!("agents.project.name").to_string())
                    .on_enter(move |_, cx| {
                        let _ = enter_parent.update(cx, |view, cx| view.submit_project_rename(cx));
                    }),
            )
            .when_some(self.library.error.clone(), |body, error| {
                body.child(
                    div()
                        .w_full()
                        .min_w(px(0.0))
                        .text_size(px(12.0))
                        .line_clamp(4)
                        .text_color(ShellDeckColors::error())
                        .child(error),
                )
            })
            .child(
                div()
                    .w_full()
                    .flex()
                    .justify_end()
                    .gap(px(8.0))
                    .child(
                        Button::new(
                            "agent-project-rename-cancel",
                            t!("scripts.cancel").to_string(),
                        )
                        .debug_selector(|| "agent-project-rename-cancel".into())
                        .variant(ButtonVariant::Ghost)
                        .size(ButtonSize::Sm)
                        .on_click(cx.listener(|view, _, _, cx| view.cancel_project_rename(cx))),
                    )
                    .child(
                        Button::new(
                            "agent-project-rename-save",
                            t!("agents.library.save").to_string(),
                        )
                        .debug_selector(|| "agent-project-rename-save".into())
                        .size(ButtonSize::Sm)
                        .on_click(cx.listener(|view, _, _, cx| view.submit_project_rename(cx))),
                    ),
            )
            .into_any_element()
    }

    pub(super) fn render_project_removal(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let project = self.library.remove_project.as_ref()?;
        let id = project.id.clone();
        let removable = self.can_remove_navigator_project(&id);
        Some(
            UiDialog::new()
                .width(gpui::px(440.0))
                .header(
                    div()
                        .p(px(16.0))
                        .text_size(px(15.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .line_clamp(2)
                        .child(
                            t!("agents.project.remove_title", name = project.label.clone())
                                .to_string(),
                        ),
                )
                .content(
                    div()
                        .px(px(16.0))
                        .pb(px(16.0))
                        .flex()
                        .flex_col()
                        .gap(px(10.0))
                        .text_size(px(12.0))
                        .child(t!("agents.project.remove_hint").to_string())
                        .when_some(self.library.error.clone(), |body, error| {
                            body.child(div().text_color(ShellDeckColors::error()).child(error))
                        })
                        .when(!removable, |body| {
                            body.child(
                                div()
                                    .text_color(ShellDeckColors::warning())
                                    .child(t!("agents.project.running").to_string()),
                            )
                        }),
                )
                .footer(
                    div()
                        .p(px(12.0))
                        .flex()
                        .justify_end()
                        .gap(px(8.0))
                        .child(
                            Button::new(
                                "agent-project-remove-cancel",
                                t!("scripts.cancel").to_string(),
                            )
                            .debug_selector(|| "agent-project-remove-cancel".into())
                            .variant(ButtonVariant::Ghost)
                            .size(ButtonSize::Sm)
                            .on_click(cx.listener(|view, _, _, cx| {
                                view.library.remove_project = None;
                                cx.notify();
                            })),
                        )
                        .child(
                            Button::new(
                                "agent-project-remove-confirm",
                                t!("workspaces.projects.remove").to_string(),
                            )
                            .debug_selector(|| "agent-project-remove-confirm".into())
                            .variant(ButtonVariant::Destructive)
                            .size(ButtonSize::Sm)
                            .disabled(!removable)
                            .on_click(cx.listener(
                                move |view, _, _, cx| {
                                    if view.can_remove_navigator_project(&id) {
                                        cx.emit(AgentConsoleEvent::RemoveProject(id.clone()));
                                    }
                                },
                            )),
                        ),
                )
                .on_backdrop_click({
                    let parent = cx.entity();
                    move |_, cx| {
                        parent.update(cx, |view, cx| {
                            view.library.remove_project = None;
                            cx.notify();
                        });
                    }
                })
                .into_any_element(),
        )
    }
}
