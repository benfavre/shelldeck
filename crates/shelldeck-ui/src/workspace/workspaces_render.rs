use super::*;

impl WorkspaceHubView {
    fn render_header(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let entity = cx.entity();
        let has_checkout = self
            .catalog
            .projects()
            .any(|project| project.checkouts().len() > 0);
        let mut title = t!("workspaces.title").to_string();
        let mut subtitle = t!("workspaces.subtitle").to_string();
        if self.retained_surface_visible() {
            if let Some(workspace) = self
                .navigation
                .active()
                .and_then(|id| self.catalog.workspace(id).ok())
            {
                title = workspace.name().to_owned();
                if let Ok(checkout) = self
                    .catalog
                    .checkout_in_project(workspace.project_id(), workspace.checkout_id())
                {
                    subtitle = match checkout.host() {
                        CheckoutHost::Local { root, .. } => root.to_string_lossy().into_owned(),
                        CheckoutHost::Ssh {
                            connection_id,
                            root,
                        } => format!(
                            "{} · {}",
                            self.connections
                                .get(connection_id)
                                .cloned()
                                .unwrap_or_else(|| t!("workspaces.authority.ssh").to_string()),
                            root.as_str()
                        ),
                    };
                }
            }
        }
        let mut actions = div().flex().items_center().flex_wrap().gap(px(8.0));
        if self.navigation.active().is_some() {
            actions = actions.child(
                Button::new(
                    "workspace-overview",
                    t!(
                        if self.overview_open && !self.onboarding_open && !self.launcher_open {
                            "workspaces.hub.back_workspace"
                        } else {
                            "workspaces.hub.overview"
                        }
                    )
                    .to_string(),
                )
                .size(ButtonSize::Sm)
                .variant(ButtonVariant::Ghost)
                .on_click({
                    let entity = entity.clone();
                    move |_, _, cx| {
                        entity.update(cx, |this, cx| {
                            this.toggle_overview(cx);
                        });
                    }
                }),
            );
        }
        // The empty state owns its single primary action.
        if has_checkout {
            actions = actions
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
                            entity.update(cx, |this, cx| this.show_onboarding(cx));
                        }
                    }),
                )
                .child(
                    Button::new("workspace-launcher-open", t!("workspaces.new").to_string())
                        .size(ButtonSize::Sm)
                        .on_click(move |_, _, cx| {
                            entity.update(cx, |this, cx| this.show_launcher(cx));
                        }),
                );
        }
        div()
            .flex()
            .items_center()
            .justify_between()
            .flex_wrap()
            .gap(px(12.0))
            .px(px(20.0))
            .py(px(14.0))
            .flex_shrink_0()
            .border_b_1()
            .border_color(ShellDeckColors::border())
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(4.0))
                    .flex_1()
                    // Keep the heading readable; long translated actions wrap below it.
                    .min_w(px(180.0))
                    .overflow_hidden()
                    .child(
                        div()
                            .text_size(px(17.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .truncate()
                            .child(title),
                    )
                    .child(
                        div()
                            .text_size(px(12.0))
                            .text_color(ShellDeckColors::text_muted())
                            .truncate()
                            .child(subtitle),
                    ),
            )
            .child(actions)
    }

    fn render_empty(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let entity = cx.entity();
        div()
            .flex()
            .flex_col()
            .items_center()
            .gap(px(16.0))
            .py(px(48.0))
            .px(px(16.0))
            .child(lucide_icon("folder", 32.0, ShellDeckColors::text_muted()))
            .child(
                div()
                    .text_size(px(23.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_center()
                    .child(t!("workspaces.hub.empty_title").to_string()),
            )
            .child(
                div()
                    .max_w(px(420.0))
                    .text_size(px(13.0))
                    .text_center()
                    .text_color(ShellDeckColors::text_muted())
                    .child(t!("workspaces.hub.empty_body").to_string()),
            )
            .child(
                Button::new(
                    "workspace-empty-add",
                    t!("workspaces.onboarding.add").to_string(),
                )
                .on_click(move |_, _, cx| entity.update(cx, |this, cx| this.show_onboarding(cx))),
            )
    }

    fn render_catalog(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut body = div().flex().flex_col().gap(px(8.0)).w_full().min_w_0();
        if self.catalog.projects().len() > 0 {
            body = body.child(
                div()
                    .text_size(px(12.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(ShellDeckColors::text_muted())
                    .child(t!("workspaces.hub.projects").to_string()),
            );
        }
        for project in self.catalog.projects() {
            for checkout in project.checkouts() {
                let item = checkout_presentation(project, checkout, &self.connections);
                let root = match checkout.host() {
                    CheckoutHost::Local { root, .. } => root.to_string_lossy().into_owned(),
                    CheckoutHost::Ssh { root, .. } => root.as_str().to_owned(),
                };
                let checkout_id = checkout.id();
                let entity = cx.entity();
                body = body.child(
                    Button::new(
                        ("workspace-project", checkout_id.as_uuid().as_u128() as u64),
                        format!("{} · {}", item.project, item.host),
                    )
                    .variant(ButtonVariant::Ghost)
                    .icon("folder")
                    .w_full()
                    .justify_start()
                    .overflow_hidden()
                    .tooltip(root.clone())
                    .on_click(move |_, _, cx| {
                        entity.update(cx, |this, cx| {
                            this.open_project_agents(checkout_id, cx);
                        })
                    }),
                );
                body = body.child(
                    div()
                        .w_full()
                        .pl(px(34.0))
                        .pb(px(6.0))
                        .min_w_0()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_size(px(11.0))
                        .text_color(ShellDeckColors::text_muted())
                        .child(root),
                );
            }
            let project_id = project.id();
            let entity = cx.entity();
            let removable = self.can_remove_project(project_id);
            body = body.child(
                div().flex().justify_end().child(
                    Button::new(
                        (
                            "workspace-project-remove",
                            project_id.as_uuid().as_u128() as u64,
                        ),
                        t!("workspaces.projects.remove").to_string(),
                    )
                    .variant(ButtonVariant::Ghost)
                    .size(ButtonSize::Sm)
                    .disabled(!removable)
                    .tooltip(if removable {
                        t!("workspaces.projects.remove_keeps_files").to_string()
                    } else {
                        t!("workspaces.projects.remove_blocked").to_string()
                    })
                    .on_click(move |_, _, cx| {
                        entity.update(cx, |this, cx| this.remove_project(project_id, cx))
                    }),
                ),
            );
        }
        body
    }

    fn render_workspace_navigation(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut body = div().flex().flex_col().gap(px(8.0)).p(px(12.0)).min_w_0();
        for workspace in self.catalog.workspaces() {
            if workspace.lifecycle() == UserWorkspaceLifecycle::Archived {
                continue;
            }
            let id = workspace.id();
            let entity = cx.entity();
            let active = self.navigation.active() == Some(id);
            body = body.child(
                Button::new(
                    ("workspace-nav", id.as_uuid().as_u128() as u64),
                    workspace.name().to_owned(),
                )
                .icon("folder")
                .variant(if active {
                    ButtonVariant::Secondary
                } else {
                    ButtonVariant::Ghost
                })
                .w_full()
                .justify_start()
                .overflow_hidden()
                .on_click(move |_, _, cx| entity.update(cx, |this, cx| this.switch_to(id, cx))),
            );
        }
        body
    }

    fn render_workspace_card(
        &self,
        presentation: WorkspaceCardPresentation,
        active: bool,
        creation: Option<&BackgroundWorkspaceCreateState>,
        attention_items: Vec<AttentionItemPresentation>,
        platform_attention_items: Vec<PlatformAttentionPresentation>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let entity = cx.entity();
        let id = presentation.id;
        let select_entity = entity.clone();
        let archive_entity = entity.clone();
        let mut badges = div().flex().items_center().flex_wrap().gap(px(5.0));
        badges = badges.child(
            Badge::new(format!(
                "{} · {}",
                t!("workspaces.authority.terminal_filesystem"),
                t!(presentation.host_kind)
            ))
            .variant(BadgeVariant::Outline),
        );
        if presentation.provider_bound {
            badges = badges.child(
                Badge::new(t!("workspaces.authority.provider_only").to_string())
                    .variant(BadgeVariant::Secondary),
            );
        }
        let mut metadata = vec![presentation.host.clone()];
        for value in [
            &presentation.project,
            &presentation.checkout,
            &presentation.repository,
        ] {
            if value != &presentation.name
                && value != &t!("workspaces.hub.folder").to_string()
                && !metadata.contains(value)
            {
                metadata.push(value.clone());
            }
        }
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
                    .flex_wrap()
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
                                    .child(metadata.join(" · ")),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .flex_wrap()
                            .gap(px(6.0))
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
                            .when(presentation.provider_bound, |row| {
                                row.child(
                                    Badge::new(
                                        t!(
                                            "workspaces.freshness.provider",
                                            state = presentation
                                                .provider_freshness
                                                .map(freshness_label)
                                                .unwrap_or_else(|| t!(
                                                    "workspaces.freshness.unknown"
                                                )
                                                .to_string())
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
                                )
                            }),
                    ),
            )
            .child(badges);
        let dirty = presentation.dirty;
        content = content.child(
            div()
                .flex()
                .items_center()
                .flex_wrap()
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
        if !attention_items.is_empty() || !platform_attention_items.is_empty() {
            content = content.child(
                div()
                    .pt(px(3.0))
                    .text_size(px(10.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(ShellDeckColors::text_primary())
                    .child(t!("workspaces.attention.title").to_string()),
            );
        }
        for item in attention_items {
            let attention_entity = entity.clone();
            let workspace = item.workspace;
            let attention_id = item.id;
            let revision = item.revision;
            let path = if item.agent_path.is_empty() {
                t!("workspaces.attention.agent_root").to_string()
            } else {
                item.agent_path.join(" › ")
            };
            content = content.child(
                Card::new().content(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(5.0))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(5.0))
                                .child(
                                    Badge::new(attention_state_label(item.state))
                                        .variant(attention_state_variant(item.state)),
                                )
                                .child(
                                    Badge::new(if item.unread {
                                        t!("workspaces.attention.unread").to_string()
                                    } else {
                                        t!("workspaces.attention.read").to_string()
                                    })
                                    .variant(if item.unread {
                                        BadgeVariant::Destructive
                                    } else {
                                        BadgeVariant::Outline
                                    }),
                                )
                                .child(
                                    div()
                                        .min_w_0()
                                        .truncate()
                                        .text_size(px(10.0))
                                        .text_color(ShellDeckColors::text_muted())
                                        .child(
                                            t!("workspaces.attention.agent_path", path = path)
                                                .to_string(),
                                        ),
                                ),
                        )
                        .child(
                            Button::new(
                                SharedString::from(format!(
                                    "workspace-attention-{}-{revision}",
                                    attention_id.as_uuid()
                                )),
                                item.title,
                            )
                            .size(ButtonSize::Sm)
                            .variant(ButtonVariant::Ghost)
                            .on_click(move |_, _, cx| {
                                attention_entity.update(cx, |this, cx| {
                                    if let Err(error) = this.open_attention_item(
                                        workspace,
                                        attention_id,
                                        revision,
                                        cx,
                                    ) {
                                        this.error = Some(attention_error_label(error));
                                        cx.notify();
                                    }
                                });
                            }),
                        ),
                ),
            );
        }
        for item in platform_attention_items {
            let activation = item.activation;
            let attention_entity = entity.clone();
            let path = if item.nested_agent_path.is_empty() {
                t!("workspaces.attention.agent_root").to_string()
            } else {
                item.nested_agent_path.join(" › ")
            };
            content = content.child(
                Card::new().content(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(5.0))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(5.0))
                                .child(
                                    Badge::new(platform_attention_state_label(item.state))
                                        .variant(BadgeVariant::Outline),
                                )
                                .child(
                                    Badge::new(if item.unread {
                                        t!("workspaces.attention.unread").to_string()
                                    } else {
                                        t!("workspaces.attention.read").to_string()
                                    })
                                    .variant(if item.unread {
                                        BadgeVariant::Destructive
                                    } else {
                                        BadgeVariant::Outline
                                    }),
                                )
                                .child(
                                    div()
                                        .min_w_0()
                                        .truncate()
                                        .text_size(px(10.0))
                                        .text_color(ShellDeckColors::text_muted())
                                        .child(path),
                                ),
                        )
                        .child(
                            Button::new(
                                SharedString::from(format!(
                                    "platform-attention-{}-{}",
                                    activation.item.uuid(),
                                    activation.item_revision.get()
                                )),
                                attention_reason_label(item.reason),
                            )
                            .size(ButtonSize::Sm)
                            .variant(ButtonVariant::Ghost)
                            .on_click(move |_, _, cx| {
                                attention_entity.update(cx, |_this, cx| {
                                    cx.emit(WorkspaceHubEvent::OpenPlatformAttention(activation));
                                });
                            }),
                        ),
                ),
            );
        }
        if let Some(state) = creation
            .filter(|state| !matches!(state, BackgroundWorkspaceCreateState::Completed { .. }))
        {
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
        let selected_is_ssh = self.launcher.checkout.is_some_and(|selected| {
            self.catalog.projects().any(|project| {
                project.checkouts().any(|checkout| {
                    checkout.id() == selected && matches!(checkout.host(), CheckoutHost::Ssh { .. })
                })
            })
        });
        let mut form = div()
            .flex()
            .flex_col()
            .gap(px(16.0))
            .min_w_0()
            .child(
                div()
                    .text_size(px(22.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(t!("workspaces.launcher.title").to_string()),
            )
            .child(
                div()
                    .text_size(px(13.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(t!("workspaces.hub.launch_body").to_string()),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(8.0))
                    .child(
                        div()
                            .text_size(px(12.0))
                            .child(t!("workspaces.hub.project_folder").to_string()),
                    )
                    .child(self.checkout_select.clone()),
            )
            .child(
                Input::new(&self.name_state)
                    .variant(InputVariant::Outline)
                    .placeholder(t!("workspaces.launcher.name").to_string()),
            )
            .child(
                Button::new(
                    "workspace-launcher-advanced",
                    t!("workspaces.hub.advanced").to_string(),
                )
                .variant(ButtonVariant::Ghost)
                .icon(if self.launcher_advanced {
                    "chevron-down"
                } else {
                    "chevron-right"
                })
                .justify_start()
                .on_click({
                    let entity = entity.clone();
                    move |_, _, cx| {
                        entity.update(cx, |this, cx| {
                            this.launcher_advanced = !this.launcher_advanced;
                            cx.notify();
                        });
                    }
                }),
            );
        if self.launcher_advanced {
            if !selected_is_ssh {
                let mut modes = div().flex().flex_wrap().gap(px(8.0));
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
                ] {
                    let entity = entity.clone();
                    modes = modes.child(
                        Button::new(("workspace-launch-mode", index), label)
                            .size(ButtonSize::Sm)
                            .variant(if self.launcher.mode == mode {
                                ButtonVariant::Secondary
                            } else {
                                ButtonVariant::Outline
                            })
                            .on_click(move |_, _, cx| {
                                entity.update(cx, |this, cx| this.set_launch_mode(mode, cx))
                            }),
                    );
                }
                form = form.child(modes);
                if self.launcher.mode == WorkspaceLaunchMode::GitWorktree {
                    form = form
                        .child(
                            Input::new(&self.branch_state)
                                .placeholder(t!("workspaces.launcher.branch").to_string()),
                        )
                        .child(
                            Input::new(&self.start_point_state)
                                .placeholder(t!("workspaces.launcher.start_point").to_string()),
                        );
                }
            }
            form = form.child(
                div()
                    .text_size(px(12.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(t!("workspaces.hub.link_task").to_string()),
            );
            let mut intakes = div().flex().flex_wrap().gap(px(8.0));
            for (index, intake) in [
                LauncherIntakeKind::Manual,
                LauncherIntakeKind::Issue,
                LauncherIntakeKind::PullRequest,
                LauncherIntakeKind::Task,
            ]
            .into_iter()
            .enumerate()
            {
                let entity = entity.clone();
                intakes = intakes.child(
                    Button::new(("workspace-intake", index), intake.label())
                        .size(ButtonSize::Sm)
                        .variant(if self.launcher.intake == intake {
                            ButtonVariant::Secondary
                        } else {
                            ButtonVariant::Outline
                        })
                        .on_click(move |_, _, cx| {
                            entity.update(cx, |this, cx| this.set_intake(intake, cx))
                        }),
                );
            }
            form = form.child(intakes);
            if self.launcher.intake != LauncherIntakeKind::Manual {
                form = form
                    .child(
                        Input::new(&self.provider_state)
                            .placeholder(t!("workspaces.launcher.provider").to_string()),
                    )
                    .child(
                        Input::new(&self.repository_state)
                            .placeholder(t!("workspaces.launcher.repository").to_string()),
                    )
                    .child(
                        Input::new(&self.key_state)
                            .placeholder(t!("workspaces.launcher.key").to_string()),
                    )
                    .child(
                        Input::new(&self.title_state)
                            .placeholder(t!("workspaces.launcher.external_title").to_string()),
                    )
                    .child(
                        Input::new(&self.url_state)
                            .placeholder(t!("workspaces.launcher.url").to_string()),
                    );
            }
        } else if self.launcher.mode == WorkspaceLaunchMode::GitWorktree
            || self.launcher.intake != LauncherIntakeKind::Manual
        {
            form = form.child(
                div()
                    .text_size(px(12.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(format!(
                        "{} · {}",
                        if selected_is_ssh {
                            t!("workspaces.launcher.mode_ssh")
                        } else if self.launcher.mode == WorkspaceLaunchMode::GitWorktree {
                            t!("workspaces.launcher.mode_worktree")
                        } else {
                            t!("workspaces.launcher.mode_existing")
                        },
                        self.launcher.intake.label()
                    )),
            );
        }
        form.child(
            div()
                .flex()
                .justify_end()
                .gap(px(8.0))
                .child(
                    Button::new(
                        "workspace-launcher-close",
                        t!("workspaces.launcher.close").to_string(),
                    )
                    .variant(ButtonVariant::Ghost)
                    .on_click({
                        let entity = entity.clone();
                        move |_, _, cx| {
                            entity.update(cx, |this, cx| {
                                this.dismiss_forms(cx);
                            });
                        }
                    }),
                )
                .child(
                    Button::new(
                        "workspace-launcher-submit",
                        t!("workspaces.launcher.create").to_string(),
                    )
                    .disabled(self.launcher.checkout.is_none() || self.recovery_pending)
                    .on_click(move |_, _, cx| {
                        entity.update(cx, |this, cx| this.submit_launcher(cx))
                    }),
                ),
        )
    }
}

impl Render for WorkspaceHubView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
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
                let attention_items = self.attention_items(workspace.id());
                let platform_attention_items = self
                    .platform_attention
                    .get(&workspace.id())
                    .cloned()
                    .unwrap_or_default();
                workspace_cards = workspace_cards.child(self.render_workspace_card(
                    presentation,
                    self.navigation.active() == Some(workspace.id()),
                    self.creation.state(workspace.id()),
                    attention_items,
                    platform_attention_items,
                    cx,
                ));
            }
        }
        for (workspace, request) in &self.pending_requests {
            if self.catalog.workspace(*workspace).is_ok() {
                continue;
            }
            let Some(state) = self.creation.state(*workspace) else {
                continue;
            };
            let id = *workspace;
            let entity = cx.entity();
            let mut actions = div().flex().flex_wrap().gap(px(6.0));
            if matches!(
                state,
                BackgroundWorkspaceCreateState::Running { .. }
                    | BackgroundWorkspaceCreateState::Cancelling { .. }
            ) {
                let cancel_entity = entity.clone();
                actions = actions.child(
                    Button::new(
                        ("workspace-pending-cancel", id.as_uuid().as_u128() as u64),
                        t!("workspaces.create.cancel").to_string(),
                    )
                    .size(ButtonSize::Sm)
                    .disabled(matches!(
                        state,
                        BackgroundWorkspaceCreateState::Cancelling { .. }
                    ))
                    .on_click(move |_, _, cx| {
                        cancel_entity.update(cx, |this, cx| this.request_cancel(id, cx));
                    }),
                );
            } else if creation_retryable(state) {
                let entity = entity.clone();
                actions = actions.child(
                    Button::new(
                        ("workspace-pending-retry", id.as_uuid().as_u128() as u64),
                        t!("workspaces.create.retry").to_string(),
                    )
                    .size(ButtonSize::Sm)
                    .on_click(move |_, _, cx| {
                        entity.update(cx, |this, cx| this.retry_create(id, cx));
                    }),
                );
            }
            if matches!(
                state,
                BackgroundWorkspaceCreateState::Failed { .. }
                    | BackgroundWorkspaceCreateState::Conflict { .. }
                    | BackgroundWorkspaceCreateState::Cancelled { .. }
            ) {
                let operation = request.operation;
                actions = actions.child(
                    Button::new(
                        ("workspace-pending-remove", id.as_uuid().as_u128() as u64),
                        t!("workspaces.create.remove").to_string(),
                    )
                    .size(ButtonSize::Sm)
                    .variant(ButtonVariant::Ghost)
                    .on_click(move |_, _, cx| {
                        entity.update(cx, |this, cx| this.dismiss_create(id, operation, cx));
                    }),
                );
            }
            workspace_cards = workspace_cards.child(
                Card::new().content(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(6.0))
                        .child(request.name.clone())
                        .child(render_creation_state(state))
                        .child(actions),
                ),
            );
        }
        let active_surface = self
            .navigation
            .active()
            .and_then(|id| self.retained.get(&id).cloned());
        let editing = self.launcher_open || self.onboarding_open;
        let show_surface = self.retained_surface_visible();
        let mut body = div().flex().flex_1().min_h(px(0.0)).min_w_0();
        if show_surface {
            // At smaller widths the overview button replaces the list; retained entities stay alive.
            if window.viewport_size().width >= px(1100.0).to_pixels(window.rem_size()) {
                body = body.child(
                    div()
                        .w(px(240.0))
                        .flex_shrink_0()
                        .min_h(px(0.0))
                        .border_r_1()
                        .border_color(ShellDeckColors::border())
                        .child(scrollable_vertical(
                            self.render_workspace_navigation(cx).into_any_element(),
                        )),
                );
            }
            body = body.child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .min_h(px(0.0))
                    .children(active_surface),
            );
        } else {
            let mut content = div()
                .flex()
                .flex_col()
                .gap(px(24.0))
                .w_full()
                .max_w(px(if editing { 580.0 } else { 880.0 }))
                .min_w_0();
            if self.onboarding_open {
                content = content.child(self.render_onboarding(cx));
            } else if self.launcher_open {
                content = content.child(self.render_launcher(cx));
            } else if self.catalog.projects().len() == 0 {
                content = content.child(self.render_empty(cx));
            } else {
                content = content.child(self.render_catalog(cx));
                if self.catalog.workspaces().len() > 0 || !self.pending_requests.is_empty() {
                    content = content
                        .child(
                            div()
                                .text_size(px(13.0))
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(t!("workspaces.title").to_string()),
                        )
                        .child(workspace_cards);
                } else {
                    content = content.child(
                        div()
                            .text_size(px(13.0))
                            .text_color(ShellDeckColors::text_muted())
                            .child(t!("workspaces.cards.empty").to_string()),
                    );
                }
            }
            body = body.child(scrollable_vertical(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .w_full()
                    .min_w_0()
                    .p(px(24.0))
                    .child(content),
            ));
        }
        let mut root = div()
            .flex()
            .flex_col()
            .size_full()
            .min_w_0()
            .min_h(px(0.0))
            .bg(ShellDeckColors::bg_primary())
            .text_color(ShellDeckColors::text_primary())
            .child(self.render_header(cx));
        if let Some(error) = self.load_error.as_ref().or(self.error.as_ref()) {
            root = root.child(
                div().p(px(12.0)).flex_shrink_0().child(
                    Alert::new()
                        .variant(AlertVariant::Error)
                        .description(error.clone()),
                ),
            );
        }
        root.child(body)
    }
}
