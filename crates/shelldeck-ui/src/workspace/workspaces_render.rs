use super::*;

impl WorkspaceHubView {
    fn render_header(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let entity = cx.entity();
        let has_checkout = self
            .catalog
            .projects()
            .any(|project| project.checkouts().len() > 0);
        div()
            .flex()
            .items_center()
            .justify_between()
            .px(px(18.0))
            .py(px(12.0))
            .border_b_1()
            .border_color(ShellDeckColors::border())
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(3.0))
                    .child(
                        div()
                            .text_size(px(16.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(ShellDeckColors::text_primary())
                            .child(t!("workspaces.title").to_string()),
                    )
                    .child(
                        div()
                            .text_size(px(11.0))
                            .text_color(ShellDeckColors::text_muted())
                            .child(t!("workspaces.subtitle").to_string()),
                    ),
            )
            .child(
                div()
                    .flex()
                    .gap(px(6.0))
                    .child(
                        Button::new(
                            "workspace-onboarding-open",
                            t!("workspaces.onboarding.add").to_string(),
                        )
                        .size(ButtonSize::Sm)
                        .variant(ButtonVariant::Outline)
                        .on_click({
                            let entity = entity.clone();
                            move |_, _, cx| {
                                entity.update(cx, |this, cx| {
                                    this.onboarding_open = true;
                                    this.error = None;
                                    cx.notify();
                                });
                            }
                        }),
                    )
                    .child(
                        Button::new("workspace-launcher-open", t!("workspaces.new").to_string())
                            .size(ButtonSize::Sm)
                            .variant(ButtonVariant::Default)
                            .disabled(!has_checkout)
                            .on_click(move |_, _, cx| {
                                entity.update(cx, |this, cx| {
                                    this.launcher_open = true;
                                    this.error = None;
                                    cx.notify();
                                });
                            }),
                    ),
            )
    }

    fn render_catalog(&self, _cx: &mut Context<Self>) -> impl IntoElement {
        let mut body = div().flex().flex_col().gap(px(8.0)).p(px(10.0));
        if self.catalog.projects().len() == 0 {
            body = body.child(
                Card::new().content(
                    div()
                        .flex()
                        .flex_col()
                        .items_center()
                        .gap(px(7.0))
                        .p(px(16.0))
                        .text_size(px(11.0))
                        .text_color(ShellDeckColors::text_muted())
                        .child(lucide_icon(
                            "folder-git-2",
                            22.0,
                            ShellDeckColors::text_muted(),
                        ))
                        .child(t!("workspaces.catalog.empty").to_string()),
                ),
            );
        }
        for project in self.catalog.projects() {
            let mut project_view = div().flex().flex_col().gap(px(5.0)).child(
                div()
                    .px(px(4.0))
                    .text_size(px(11.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(ShellDeckColors::text_primary())
                    .child(project.name().to_owned()),
            );
            for checkout in project.checkouts() {
                let item = checkout_presentation(project, checkout, &self.connections);
                project_view = project_view.child(
                    Card::new().content(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(4.0))
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .justify_between()
                                    .child(
                                        div()
                                            .text_size(px(11.0))
                                            .font_weight(FontWeight::MEDIUM)
                                            .text_color(ShellDeckColors::text_primary())
                                            .child(item.checkout),
                                    )
                                    .child(
                                        Badge::new(t!(item.host_kind).to_string())
                                            .variant(BadgeVariant::Outline),
                                    ),
                            )
                            .child(
                                div()
                                    .text_size(px(10.0))
                                    .text_color(ShellDeckColors::text_muted())
                                    .child(format!("{} · {}", item.host, item.repository)),
                            ),
                    ),
                );
            }
            body = body.child(project_view);
        }
        scrollable_vertical(body)
    }

    fn render_onboarding(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let entity = cx.entity();
        let local_entity = entity.clone();
        let ssh_entity = entity.clone();
        let submit_entity = entity.clone();
        let close_entity = entity.clone();
        let mut connections = div().flex().flex_wrap().gap(px(5.0));
        if self.onboarding_ssh {
            for (id, label) in &self.connections {
                let id = *id;
                let selected = self.onboarding_connection == Some(id);
                let select_entity = entity.clone();
                connections = connections.child(
                    Button::new(("workspace-host", id.as_u128() as u64), label.clone())
                        .size(ButtonSize::Sm)
                        .variant(if selected {
                            ButtonVariant::Secondary
                        } else {
                            ButtonVariant::Outline
                        })
                        .on_click(move |_, _, cx| {
                            select_entity.update(cx, |this, cx| {
                                this.onboarding_connection = Some(id);
                                cx.notify();
                            });
                        }),
                );
            }
        }
        Card::new().content(
            div()
                .flex()
                .flex_col()
                .gap(px(8.0))
                .child(t!("workspaces.onboarding.title").to_string())
                .child(
                    Input::new(&self.onboarding_project)
                        .placeholder(t!("workspaces.onboarding.project").to_string()),
                )
                .child(
                    Input::new(&self.onboarding_checkout)
                        .placeholder(t!("workspaces.onboarding.checkout").to_string()),
                )
                .child(
                    Input::new(&self.onboarding_repository)
                        .placeholder(t!("workspaces.onboarding.repository").to_string()),
                )
                .child(
                    div()
                        .flex()
                        .gap(px(5.0))
                        .child(
                            Button::new(
                                "onboard-local",
                                t!("workspaces.authority.local").to_string(),
                            )
                            .size(ButtonSize::Sm)
                            .variant(if self.onboarding_ssh {
                                ButtonVariant::Outline
                            } else {
                                ButtonVariant::Secondary
                            })
                            .on_click(move |_, _, cx| {
                                local_entity.update(cx, |this, cx| {
                                    this.onboarding_ssh = false;
                                    cx.notify();
                                })
                            }),
                        )
                        .child(
                            Button::new("onboard-ssh", t!("workspaces.authority.ssh").to_string())
                                .size(ButtonSize::Sm)
                                .variant(if self.onboarding_ssh {
                                    ButtonVariant::Secondary
                                } else {
                                    ButtonVariant::Outline
                                })
                                .disabled(self.connections.is_empty())
                                .on_click(move |_, _, cx| {
                                    ssh_entity.update(cx, |this, cx| {
                                        this.onboarding_ssh = true;
                                        cx.notify();
                                    })
                                }),
                        ),
                )
                .child(connections)
                .child(
                    Input::new(&self.onboarding_root).placeholder(if self.onboarding_ssh {
                        t!("workspaces.onboarding.remote_root").to_string()
                    } else {
                        t!("workspaces.onboarding.local_root").to_string()
                    }),
                )
                .child(
                    div()
                        .flex()
                        .justify_end()
                        .gap(px(5.0))
                        .child(
                            Button::new(
                                "onboard-close",
                                t!("workspaces.launcher.close").to_string(),
                            )
                            .size(ButtonSize::Sm)
                            .variant(ButtonVariant::Ghost)
                            .on_click(move |_, _, cx| {
                                close_entity.update(cx, |this, cx| {
                                    this.onboarding_open = false;
                                    cx.notify();
                                })
                            }),
                        )
                        .child(
                            Button::new(
                                "onboard-save",
                                t!("workspaces.onboarding.save").to_string(),
                            )
                            .size(ButtonSize::Sm)
                            .variant(ButtonVariant::Default)
                            .on_click(move |_, _, cx| {
                                submit_entity.update(cx, |this, cx| this.submit_onboarding(cx))
                            }),
                        ),
                ),
        )
    }

    fn render_workspace_card(
        &self,
        presentation: WorkspaceCardPresentation,
        active: bool,
        creation: Option<&BackgroundWorkspaceCreateState>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let entity = cx.entity();
        let id = presentation.id;
        let select_entity = entity.clone();
        let archive_entity = entity.clone();
        let mut badges = div().flex().items_center().flex_wrap().gap(px(5.0));
        badges = badges
            .child(
                Badge::new(format!(
                    "{} · {}",
                    t!("workspaces.authority.terminal_filesystem"),
                    t!(presentation.host_kind)
                ))
                .variant(BadgeVariant::Outline),
            )
            .child(
                Badge::new(if presentation.provider_bound {
                    t!("workspaces.authority.provider_only").to_string()
                } else {
                    t!("workspaces.authority.provider_unbound").to_string()
                })
                .variant(BadgeVariant::Secondary),
            );
        if presentation.unread > 0 {
            badges = badges.child(
                Badge::new(t!("workspaces.card.unread", count = presentation.unread).to_string())
                    .variant(BadgeVariant::Secondary),
            );
        }
        if presentation.attention > 0 {
            badges = badges.child(
                Badge::new(
                    t!("workspaces.card.attention", count = presentation.attention).to_string(),
                )
                .variant(BadgeVariant::Destructive),
            );
        }
        let mut content = div()
            .flex()
            .flex_col()
            .gap(px(7.0))
            .child(
                div()
                    .flex()
                    .items_start()
                    .justify_between()
                    .gap(px(8.0))
                    .child(
                        div()
                            .min_w_0()
                            .child(
                                div()
                                    .truncate()
                                    .text_size(px(12.0))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(ShellDeckColors::text_primary())
                                    .child(presentation.name.clone()),
                            )
                            .child(
                                div()
                                    .truncate()
                                    .text_size(px(10.0))
                                    .text_color(ShellDeckColors::text_muted())
                                    .child(format!(
                                        "{} · {} · {} · {}",
                                        presentation.project,
                                        presentation.host,
                                        presentation.repository,
                                        presentation.checkout
                                    )),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(4.0))
                            .child(
                                Badge::new(
                                    t!(
                                        "workspaces.freshness.git",
                                        state = presentation
                                            .git_freshness
                                            .map(freshness_label)
                                            .unwrap_or_else(
                                                || t!("workspaces.freshness.unknown").to_string()
                                            )
                                            .as_str()
                                    )
                                    .to_string(),
                                )
                                .variant(
                                    presentation
                                        .git_freshness
                                        .map(freshness_variant)
                                        .unwrap_or(BadgeVariant::Outline),
                                ),
                            )
                            .child(
                                Badge::new(
                                    t!(
                                        "workspaces.freshness.provider",
                                        state = presentation
                                            .provider_freshness
                                            .map(freshness_label)
                                            .unwrap_or_else(
                                                || t!("workspaces.freshness.unknown").to_string()
                                            )
                                            .as_str()
                                    )
                                    .to_string(),
                                )
                                .variant(
                                    presentation
                                        .provider_freshness
                                        .map(freshness_variant)
                                        .unwrap_or(BadgeVariant::Outline),
                                ),
                            ),
                    ),
            )
            .child(badges);
        let dirty = presentation.dirty;
        content = content.child(
            div()
                .flex()
                .items_center()
                .gap(px(8.0))
                .text_size(px(10.0))
                .text_color(ShellDeckColors::text_muted())
                .child(
                    presentation
                        .branch
                        .clone()
                        .unwrap_or_else(|| t!("workspaces.card.branch_unknown").to_string()),
                )
                .child(if presentation.git_observed {
                    t!(
                        "workspaces.card.dirty",
                        staged = dirty.staged,
                        modified = dirty.modified,
                        untracked = dirty.untracked,
                        conflicted = dirty.conflicted
                    )
                    .to_string()
                } else if presentation.git_unavailable {
                    t!("workspaces.card.git_unavailable").to_string()
                } else {
                    t!("workspaces.card.awaiting_observation").to_string()
                })
                .when(presentation.provider_observed, |row| {
                    row.child(agent_label(presentation.agent))
                }),
        );
        if let Some(external) = presentation.external {
            content = content.child(authority_row(
                "circle-dot",
                t!("workspaces.card.external").to_string(),
                external,
            ));
        }
        if let Some(orchestration) = presentation.orchestration {
            content = content.child(authority_row(
                "bot",
                t!("workspaces.card.orchestration").to_string(),
                orchestration,
            ));
        }
        if let Some(state) = creation {
            content = content.child(render_creation_state(state));
        }
        content = content.child(
            div()
                .flex()
                .items_center()
                .gap(px(6.0))
                .child(
                    Button::new(
                        ("workspace-open", id.as_uuid().as_u128() as u64),
                        if active {
                            t!("workspaces.card.active").to_string()
                        } else {
                            t!("workspaces.card.open").to_string()
                        },
                    )
                    .size(ButtonSize::Sm)
                    .variant(if active {
                        ButtonVariant::Secondary
                    } else {
                        ButtonVariant::Default
                    })
                    .disabled(presentation.archived)
                    .on_click(move |_, _, cx| {
                        select_entity.update(cx, |this, cx| this.switch_to(id, cx));
                    }),
                )
                .child(
                    Button::new(
                        ("workspace-lifecycle", id.as_uuid().as_u128() as u64),
                        if presentation.archived {
                            t!("workspaces.card.resume").to_string()
                        } else {
                            t!("workspaces.card.archive").to_string()
                        },
                    )
                    .size(ButtonSize::Sm)
                    .variant(ButtonVariant::Ghost)
                    .on_click(move |_, _, cx| {
                        archive_entity.update(cx, |this, cx| {
                            this.archive_or_resume(id, cx);
                        });
                    }),
                ),
        );
        if matches!(
            creation,
            Some(
                BackgroundWorkspaceCreateState::Running { .. }
                    | BackgroundWorkspaceCreateState::Cancelling { .. }
            )
        ) {
            let cancel_entity = entity.clone();
            content = content.child(
                Button::new(
                    ("workspace-cancel", id.as_uuid().as_u128() as u64),
                    t!("workspaces.create.cancel").to_string(),
                )
                .size(ButtonSize::Sm)
                .variant(ButtonVariant::Destructive)
                .on_click(move |_, _, cx| {
                    cancel_entity.update(cx, |this, cx| this.request_cancel(id, cx));
                }),
            );
        }
        if creation.is_some_and(creation_retryable) {
            let retry_entity = entity;
            content = content.child(
                Button::new(
                    ("workspace-retry", id.as_uuid().as_u128() as u64),
                    t!("workspaces.create.retry").to_string(),
                )
                .size(ButtonSize::Sm)
                .variant(ButtonVariant::Outline)
                .on_click(move |_, _, cx| {
                    retry_entity.update(cx, |this, cx| this.retry_create(id, cx));
                }),
            );
        }
        Card::new()
            .content(content)
            .border_color(if active {
                ShellDeckColors::primary()
            } else {
                ShellDeckColors::border()
            })
            .into_any_element()
    }

    fn render_launcher(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let entity = cx.entity();
        let mut intake_row = div().flex().items_center().flex_wrap().gap(px(6.0));
        for (intake_index, intake) in [
            LauncherIntakeKind::Manual,
            LauncherIntakeKind::Issue,
            LauncherIntakeKind::PullRequest,
            LauncherIntakeKind::Task,
        ]
        .into_iter()
        .enumerate()
        {
            let target = entity.clone();
            intake_row = intake_row.child(
                Button::new(("workspace-intake", intake_index), intake.label())
                    .size(ButtonSize::Sm)
                    .variant(if self.launcher.intake == intake {
                        ButtonVariant::Default
                    } else {
                        ButtonVariant::Outline
                    })
                    .on_click(move |_, _, cx| {
                        target.update(cx, |this, cx| this.set_intake(intake, cx));
                    }),
            );
        }
        let mut form = div()
            .flex()
            .flex_col()
            .gap(px(9.0))
            .child(intake_row)
            .child({
                let mut modes = div().flex().flex_wrap().gap(px(6.0));
                for (index, mode, label) in [
                    (
                        0usize,
                        WorkspaceLaunchMode::ExistingFolder,
                        t!("workspaces.launcher.mode_existing").to_string(),
                    ),
                    (
                        1,
                        WorkspaceLaunchMode::GitWorktree,
                        t!("workspaces.launcher.mode_worktree").to_string(),
                    ),
                    (
                        2,
                        WorkspaceLaunchMode::Ssh,
                        t!("workspaces.launcher.mode_ssh").to_string(),
                    ),
                ] {
                    let target = entity.clone();
                    modes = modes.child(
                        Button::new(("workspace-launch-mode", index), label)
                            .size(ButtonSize::Sm)
                            .disabled(mode != WorkspaceLaunchMode::ExistingFolder)
                            .variant(if self.launcher.mode == mode {
                                ButtonVariant::Secondary
                            } else {
                                ButtonVariant::Outline
                            })
                            .on_click(move |_, _, cx| {
                                target.update(cx, |this, cx| this.set_launch_mode(mode, cx))
                            }),
                    );
                }
                modes
            })
            .child(self.checkout_select.clone())
            .child(
                Input::new(&self.name_state)
                    .size(InputSize::Sm)
                    .variant(InputVariant::Outline)
                    .placeholder(t!("workspaces.launcher.name").to_string()),
            );
        if self.launcher.intake != LauncherIntakeKind::Manual {
            form = form
                .child(
                    div()
                        .flex()
                        .gap(px(8.0))
                        .child(
                            Input::new(&self.provider_state)
                                .size(InputSize::Sm)
                                .placeholder(t!("workspaces.launcher.provider").to_string()),
                        )
                        .child(
                            Input::new(&self.repository_state)
                                .size(InputSize::Sm)
                                .placeholder(t!("workspaces.launcher.repository").to_string()),
                        ),
                )
                .child(
                    Input::new(&self.key_state)
                        .size(InputSize::Sm)
                        .placeholder(t!("workspaces.launcher.key").to_string()),
                )
                .child(
                    Input::new(&self.title_state)
                        .size(InputSize::Sm)
                        .placeholder(t!("workspaces.launcher.external_title").to_string()),
                )
                .child(
                    Input::new(&self.url_state)
                        .size(InputSize::Sm)
                        .placeholder(t!("workspaces.launcher.url").to_string()),
                );
        }
        let close_entity = entity.clone();
        let submit_entity = entity;
        Card::new().content(
            div()
                .flex()
                .flex_col()
                .gap(px(10.0))
                .child(
                    div()
                        .text_size(px(12.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(ShellDeckColors::text_primary())
                        .child(t!("workspaces.launcher.title").to_string()),
                )
                .child(form)
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_end()
                        .gap(px(6.0))
                        .child(
                            Button::new(
                                "workspace-launcher-close",
                                t!("workspaces.launcher.close").to_string(),
                            )
                            .size(ButtonSize::Sm)
                            .variant(ButtonVariant::Ghost)
                            .on_click(move |_, _, cx| {
                                close_entity.update(cx, |this, cx| {
                                    this.launcher_open = false;
                                    cx.notify();
                                });
                            }),
                        )
                        .child(
                            Button::new(
                                "workspace-launcher-submit",
                                t!("workspaces.launcher.create").to_string(),
                            )
                            .size(ButtonSize::Sm)
                            .variant(ButtonVariant::Default)
                            .on_click(move |_, _, cx| {
                                submit_entity.update(cx, |this, cx| this.submit_launcher(cx));
                            }),
                        ),
                )
                .child(
                    div()
                        .text_size(px(10.0))
                        .text_color(ShellDeckColors::text_muted())
                        .child(t!("workspaces.launcher.effect_notice").to_string()),
                ),
        )
    }
}

impl Render for WorkspaceHubView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut workspace_cards = div().flex().flex_col().gap(px(8.0));
        let default_card = WorkspaceCardState::default();
        for workspace in self.catalog.workspaces() {
            let card = self
                .navigation
                .workspace(workspace.id())
                .map(|retained| &retained.card)
                .unwrap_or(&default_card);
            if let Some(presentation) = workspace_card_presentation(
                &self.catalog,
                workspace,
                card,
                &self.connections,
                self.cards.sources.get(&workspace.id()),
            ) {
                workspace_cards = workspace_cards.child(self.render_workspace_card(
                    presentation,
                    self.navigation.active() == Some(workspace.id()),
                    self.creation.state(workspace.id()),
                    cx,
                ));
            }
        }
        if self.catalog.workspaces().len() == 0 {
            workspace_cards = workspace_cards.child(
                div()
                    .p(px(16.0))
                    .text_size(px(11.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(t!("workspaces.cards.empty").to_string()),
            );
        }

        let mut center = div().flex().flex_col().min_w_0().flex_1();
        let mut top = div().flex().flex_col().gap(px(8.0)).p(px(12.0));
        if let Some(error) = self.load_error.as_ref().or(self.error.as_ref()) {
            top = top.child(
                Alert::new()
                    .variant(AlertVariant::Error)
                    .description(error.clone()),
            );
        }
        if self.launcher_open {
            top = top.child(self.render_launcher(cx));
        }
        if self.onboarding_open {
            top = top.child(self.render_onboarding(cx));
        }
        top = top.child(workspace_cards);
        center = center.child(scrollable_vertical(top));

        let active_surface = self
            .navigation
            .active()
            .and_then(|id| self.retained.get(&id).cloned());
        let surface = div()
            .flex()
            .flex_col()
            .min_w(px(280.0))
            .flex_1()
            .border_l_1()
            .border_color(ShellDeckColors::border())
            .child(
                div()
                    .px(px(14.0))
                    .py(px(9.0))
                    .border_b_1()
                    .border_color(ShellDeckColors::border())
                    .text_size(px(11.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(ShellDeckColors::text_primary())
                    .child(t!("workspaces.surface.title").to_string()),
            )
            .children(active_surface)
            .when(self.navigation.active().is_none(), |view| {
                view.child(
                    div()
                        .flex()
                        .flex_col()
                        .items_center()
                        .justify_center()
                        .gap(px(8.0))
                        .flex_1()
                        .text_size(px(11.0))
                        .text_color(ShellDeckColors::text_muted())
                        .child(lucide_icon(
                            "mouse-pointer-2",
                            22.0,
                            ShellDeckColors::text_muted(),
                        ))
                        .child(t!("workspaces.surface.select").to_string()),
                )
            });

        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(ShellDeckColors::bg_primary())
            .child(self.render_header(cx))
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h(px(0.0))
                    .child(
                        div()
                            .w(px(250.0))
                            .flex_shrink_0()
                            .border_r_1()
                            .border_color(ShellDeckColors::border())
                            .child(self.render_catalog(cx)),
                    )
                    .child(center)
                    .child(surface),
            )
    }
}
