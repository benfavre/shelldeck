//! Folder-first project setup. Browse only edits the draft; submission owns effects.
use super::*;
use adabraka_ui::components::input::InputSize;

impl WorkspaceHubView {
    fn browse_onboarding_folder(&self, window: &mut Window, cx: &mut Context<Self>) {
        if self.onboarding_ssh || !self.onboarding_open {
            return;
        }
        let input = self.onboarding_root.clone();
        let original = input.read(cx).content().to_string();
        let current = PathBuf::from(original.trim());
        let starting_directory = current.is_dir().then_some(current);
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some(t!("workspaces.setup.browse_prompt").to_string().into()),
            starting_directory,
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = receiver.await else {
                return;
            };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            let _ = this.update(cx, |view, cx| {
                if !view.onboarding_open
                    || view.onboarding_ssh
                    || view.onboarding_root.entity_id() != input.entity_id()
                    || view.onboarding_root.read(cx).content() != original.as_str()
                {
                    return;
                }
                let Some(path) = path.to_str() else {
                    view.error = Some(t!("workspaces.setup.path_encoding").to_string());
                    cx.notify();
                    return;
                };
                view.onboarding_root
                    .update(cx, |state, cx| state.replace_content(path.to_owned(), cx));
                view.error = None;
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn render_onboarding(&mut self, cx: &mut Context<Self>) -> AnyElement {
        if self.onboarding_connection_select.is_none() {
            let mut connections = self
                .connections
                .iter()
                .map(|(id, label)| (*id, label.clone()))
                .collect::<Vec<_>>();
            connections.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.cmp(&b.0)));
            let selected = connections
                .iter()
                .position(|(id, _)| Some(*id) == self.onboarding_connection);
            let parent = cx.entity();
            self.onboarding_connection_select = Some(cx.new(move |select_cx| {
                Select::new(select_cx)
                    .options(
                        connections
                            .into_iter()
                            .map(|(id, label)| SelectOption::new(id, label))
                            .collect(),
                    )
                    .selected_index(selected)
                    .searchable(true)
                    .placeholder(t!("workspaces.setup.connection").to_string())
                    .search_placeholder(t!("workspaces.setup.connection_search").to_string())
                    .on_change(move |id, _, cx| {
                        parent.update(cx, |view, cx| {
                            if view.onboarding_open
                                && view.onboarding_ssh
                                && view.connections.contains_key(id)
                                && view.onboarding_connection != Some(*id)
                            {
                                view.cancel_remote_folder_picker(cx);
                                view.onboarding_browse_authority = None;
                                view.onboarding_root.update(cx, |state, cx| {
                                    state.replace_content(String::new(), cx)
                                });
                                view.onboarding_connection = Some(*id);
                                view.error = None;
                                cx.notify();
                            }
                        });
                    })
            }));
        }
        let mut location = div().flex().flex_col().gap(px(8.0)).child(
            div()
                .flex()
                .items_center()
                .flex_wrap()
                .gap(px(6.0))
                .child(
                    Button::new(
                        "project-setup-local",
                        t!("workspaces.authority.local").to_string(),
                    )
                    .variant(if self.onboarding_ssh {
                        ButtonVariant::Ghost
                    } else {
                        ButtonVariant::Secondary
                    })
                    .size(ButtonSize::Sm)
                    .icon(IconSource::from("terminal"))
                    .on_click(cx.listener(|view, _, _, cx| {
                        if view.onboarding_ssh {
                            view.cancel_remote_folder_picker(cx);
                            view.onboarding_browse_authority = None;
                            view.onboarding_ssh = false;
                            view.onboarding_root
                                .update(cx, |state, cx| state.replace_content(String::new(), cx));
                            view.error = None;
                            cx.notify();
                        }
                    })),
                )
                .child(
                    Button::new(
                        "project-setup-ssh",
                        t!("workspaces.authority.ssh").to_string(),
                    )
                    .variant(if self.onboarding_ssh {
                        ButtonVariant::Secondary
                    } else {
                        ButtonVariant::Ghost
                    })
                    .size(ButtonSize::Sm)
                    .icon(IconSource::from("server"))
                    .disabled(self.connections.is_empty())
                    .on_click(cx.listener(|view, _, _, cx| {
                        if !view.onboarding_ssh && !view.connections.is_empty() {
                            view.cancel_remote_folder_picker(cx);
                            view.onboarding_browse_authority = None;
                            view.onboarding_ssh = true;
                            view.onboarding_root
                                .update(cx, |state, cx| state.replace_content(String::new(), cx));
                            view.error = None;
                            cx.notify();
                        }
                    })),
                ),
        );
        if self.onboarding_ssh {
            location = location.child(self.onboarding_connection_select.as_ref().unwrap().clone());
        }
        let mut folder = div().flex().items_center().min_w_0().gap(px(8.0)).child(
            div().flex_1().min_w_0().child(
                Input::new(&self.onboarding_root)
                    .disabled(self.remote_folder_picker.is_some())
                    .size(InputSize::Md)
                    .placeholder(
                        t!(if self.onboarding_ssh {
                            "workspaces.setup.remote_folder"
                        } else {
                            "workspaces.setup.local_folder"
                        })
                        .to_string(),
                    ),
            ),
        );
        if self.onboarding_ssh {
            folder = folder.child(
                Button::new(
                    "project-setup-browse-remote",
                    t!("workspaces.setup.connect_browse").to_string(),
                )
                .variant(ButtonVariant::Outline)
                .size(ButtonSize::Sm)
                .disabled(
                    self.onboarding_connection.is_none() || self.remote_folder_picker.is_some(),
                )
                .on_click(cx.listener(|view, _, _, cx| view.request_remote_folder_picker(cx))),
            );
        } else {
            folder = folder.child(
                Button::new(
                    "project-setup-browse",
                    t!("workspaces.setup.browse").to_string(),
                )
                .variant(ButtonVariant::Outline)
                .size(ButtonSize::Sm)
                .on_click(
                    cx.listener(|view, _, window, cx| view.browse_onboarding_folder(window, cx)),
                ),
            );
        }
        if let Some(picker) = self.remote_folder_picker.as_ref() {
            return div()
                .w_full()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(16.0))
                .child(location)
                .child(picker.view.clone())
                .into_any_element();
        }
        let mut form = div()
            .w_full()
            .max_w(px(600.0))
            .mx_auto()
            .min_w_0()
            .flex()
            .flex_col()
            .gap(px(16.0))
            .child(
                div().flex().flex_col().gap(px(6.0)).child(
                    div()
                        .text_size(px(20.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(t!("workspaces.setup.title").to_string()),
                ),
            )
            .child(location)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(6.0))
                    .child(setup_label("workspaces.setup.folder"))
                    .child(folder)
                    .child(
                        div()
                            .text_size(px(11.0))
                            .text_color(ShellDeckColors::text_muted())
                            .child(
                                t!(if self.onboarding_ssh {
                                    "workspaces.setup.remote_hint"
                                } else {
                                    "workspaces.setup.folder_hint"
                                })
                                .to_string(),
                            ),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(6.0))
                    .child(setup_label("workspaces.setup.name"))
                    .child(
                        Input::new(&self.onboarding_project)
                            .size(InputSize::Md)
                            .placeholder(t!("workspaces.setup.name_placeholder").to_string()),
                    ),
            )
            .child(
                Button::new(
                    "project-setup-advanced",
                    t!("workspaces.setup.advanced").to_string(),
                )
                .variant(ButtonVariant::Ghost)
                .size(ButtonSize::Sm)
                .justify_start()
                .icon(IconSource::from(if self.onboarding_advanced {
                    "chevron-up"
                } else {
                    "chevron-down"
                }))
                .on_click(cx.listener(|view, _, _, cx| {
                    view.onboarding_advanced = !view.onboarding_advanced;
                    cx.notify();
                })),
            );
        if self.onboarding_advanced {
            form = form.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(10.0))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(6.0))
                            .child(setup_label("workspaces.setup.checkout"))
                            .child(
                                Input::new(&self.onboarding_checkout)
                                    .size(InputSize::Sm)
                                    .placeholder(
                                        t!("workspaces.setup.checkout_placeholder").to_string(),
                                    ),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(6.0))
                            .child(setup_label("workspaces.setup.repository"))
                            .child(
                                Input::new(&self.onboarding_repository)
                                    .size(InputSize::Sm)
                                    .placeholder(
                                        t!("workspaces.setup.repository_placeholder").to_string(),
                                    ),
                            ),
                    ),
            );
        }
        form.child(
            div()
                .flex()
                .items_center()
                .justify_end()
                .flex_wrap()
                .gap(px(8.0))
                .child(
                    Button::new(
                        "project-setup-cancel",
                        t!("workspaces.setup.cancel").to_string(),
                    )
                    .variant(ButtonVariant::Ghost)
                    .size(ButtonSize::Sm)
                    .on_click(cx.listener(|view, _, _, cx| {
                        view.dismiss_forms(cx);
                    })),
                )
                .child(
                    Button::new(
                        "project-setup-open",
                        t!("workspaces.setup.open").to_string(),
                    )
                    .variant(ButtonVariant::Default)
                    .size(ButtonSize::Sm)
                    .on_click(cx.listener(|view, _, _, cx| view.submit_onboarding(cx))),
                ),
        )
        .into_any_element()
    }
}

fn setup_label(key: &str) -> AnyElement {
    div()
        .text_size(px(12.0))
        .font_weight(FontWeight::MEDIUM)
        .child(t!(key).to_string())
        .into_any_element()
}
