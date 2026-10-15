//! Explicit branch delivery, separate from local file review. All subprocess
//! work runs on the background executor; remote reads only occur on a click.
use super::*;
use adabraka_ui::overlays::dialog::{Dialog as DeliveryDialog, DialogSize};
use shelldeck_core::agent_delivery::{
    AgentDeliveryBackend, DeliveryIntent, DeliveryPlan, DeliveryStatus, GitHubDeliveryStatus,
};
use std::sync::Arc;

pub(super) struct AgentDeliveryUi {
    sessions: HashMap<Uuid, DeliverySessionUi>,
    dialog: Option<Entity<DeliveryDialog>>,
    pending: Option<PendingDelivery>,
}

impl AgentDeliveryUi {
    pub(super) fn new(_cx: &mut Context<AgentConsoleView>) -> Self {
        Self {
            sessions: HashMap::new(),
            dialog: None,
            pending: None,
        }
    }
}

#[derive(Default)]
struct DeliverySessionUi {
    context: Option<AgentExecutionContext>,
    generation: u64,
    loading: bool,
    status: Option<Arc<DeliveryStatus>>,
    github: Option<GitHubDeliveryStatus>,
    error: Option<String>,
    notice: Option<String>,
    title: Option<Entity<InputState>>,
    body: Option<Entity<InputState>>,
    base: Option<Entity<InputState>>,
}
struct PendingDelivery {
    session_id: Uuid,
    context: AgentExecutionContext,
    plan: DeliveryPlan,
}

impl AgentConsoleView {
    fn delivery_context_matches(
        &self,
        session_id: Uuid,
        context: &AgentExecutionContext,
        cx: &Context<Self>,
    ) -> bool {
        self.sessions.selected_id() == Some(session_id) && self.displayed_context(cx) == *context
    }
    fn refresh_delivery(&mut self, session_id: Uuid, network: bool, cx: &mut Context<Self>) {
        if self.sessions.selected_id() != Some(session_id) {
            return;
        }
        let context = self.displayed_context(cx);
        if !matches!(context.target, AgentTarget::Local) {
            return;
        }
        let state = self.delivery.sessions.entry(session_id).or_default();
        if state.loading {
            return;
        }
        state.loading = true;
        state.context = Some(context.clone());
        state.generation = state.generation.wrapping_add(1);
        state.error = None;
        state.notice = None;
        let generation = state.generation;
        let directory = PathBuf::from(&context.workdir);
        let task = cx.background_executor().spawn(async move {
            let backend = AgentDeliveryBackend::default();
            let status = backend
                .status(&directory)
                .map_err(|error| error.to_string())?;
            let github = if network {
                Some(
                    backend
                        .github_status(&status)
                        .map_err(|error| error.to_string())?,
                )
            } else {
                None
            };
            Ok::<_, String>((status, github))
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                let Some(state) = this.delivery.sessions.get_mut(&session_id) else {
                    return;
                };
                if state.generation != generation || state.context.as_ref() != Some(&context) {
                    return;
                }
                state.loading = false;
                match result {
                    Ok((status, github)) => {
                        state.status = Some(Arc::new(status));
                        state.github = github;
                        state.error = None;
                    }
                    Err(error) => {
                        state.error = Some(error);
                        state.status = None;
                        state.github = None;
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    fn prepare_delivery(
        &mut self,
        session_id: Uuid,
        intent: DeliveryIntent,
        cx: &mut Context<Self>,
    ) {
        if self
            .sessions
            .get(session_id)
            .is_none_or(|session| session.status.is_active())
        {
            return;
        }
        let context = self.displayed_context(cx);
        let Some(state) = self.delivery.sessions.get_mut(&session_id) else {
            return;
        };
        if state.loading || state.context.as_ref() != Some(&context) {
            return;
        }
        let Some(status) = state.status.clone() else {
            return;
        };
        state.loading = true;
        state.error = None;
        state.notice = None;
        state.generation = state.generation.wrapping_add(1);
        let generation = state.generation;
        self.delivery.dialog = None;
        self.delivery.pending = None;
        let task = cx.background_executor().spawn(async move {
            let backend = AgentDeliveryBackend::default();
            match intent {
                DeliveryIntent::Push => backend.prepare_push(&status),
                DeliveryIntent::CreatePullRequest { title, body, base } => {
                    backend.prepare_pull_request(&status, title, body, base)
                }
            }
            .map_err(|error| error.to_string())
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                let matches_context = this.delivery_context_matches(session_id, &context, cx);
                let Some(state) = this.delivery.sessions.get_mut(&session_id) else {
                    return;
                };
                if state.generation != generation {
                    return;
                }
                state.loading = false;
                match result {
                    Ok(plan) if matches_context => {
                        this.present_delivery_plan(session_id, context.clone(), plan, cx)
                    }
                    Ok(_) => state.error = Some(t!("agents.delivery.context_changed").to_string()),
                    Err(error) => state.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    fn close_delivery_dialog(&mut self, cx: &mut Context<Self>) {
        self.delivery.dialog = None;
        self.delivery.pending = None;
        cx.notify();
    }
    fn present_pull_request_form(&mut self, session_id: Uuid, cx: &mut Context<Self>) {
        let context = self.displayed_context(cx);
        let Some(state) = self.delivery.sessions.get_mut(&session_id) else {
            return;
        };
        if state.loading || state.context.as_ref() != Some(&context) {
            return;
        }
        let (Some(title), Some(body), Some(base)) =
            (state.title.clone(), state.body.clone(), state.base.clone())
        else {
            return;
        };
        let parent = cx.entity();
        let close_parent = parent.clone();
        let cancel_parent = parent.clone();
        self.delivery.pending = None;
        self.delivery.dialog = Some(cx.new(move |dialog_cx| {
            DeliveryDialog::new(dialog_cx)
                .title(t!("agents.delivery.form_title").to_string())
                .description(t!("agents.delivery.form_description").to_string())
                .size(DialogSize::Lg)
                .on_close(move |_, cx| {
                    close_parent.update(cx, |this, cx| this.close_delivery_dialog(cx));
                })
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(10.0))
                        .child(delivery_field(
                            t!("agents.delivery.title_label").to_string(),
                            Input::new(&title)
                                .placeholder(t!("agents.delivery.title_placeholder").to_string()),
                        ))
                        .child(delivery_field(
                            t!("agents.delivery.base_label").to_string(),
                            Input::new(&base).placeholder("main"),
                        ))
                        .child(delivery_field(
                            t!("agents.delivery.body_label").to_string(),
                            Input::new(&body)
                                .h(px(170.0))
                                .placeholder(t!("agents.delivery.body_placeholder").to_string()),
                        )),
                )
                .footer(
                    div()
                        .flex()
                        .justify_end()
                        .gap(px(8.0))
                        .child(
                            Button::new(
                                "agent-pr-form-cancel",
                                t!("agents.review.cancel").to_string(),
                            )
                            .variant(ButtonVariant::Ghost)
                            .on_click(move |_, _, cx| {
                                cancel_parent.update(cx, |this, cx| this.close_delivery_dialog(cx));
                            }),
                        )
                        .child(
                            Button::new(
                                "agent-pr-form-preview",
                                t!("agents.delivery.preview").to_string(),
                            )
                            .on_click(move |_, _, cx| {
                                parent.update(cx, |this, cx| {
                                    if !this.delivery_context_matches(session_id, &context, cx) {
                                        this.close_delivery_dialog(cx);
                                        return;
                                    }
                                    let intent = DeliveryIntent::CreatePullRequest {
                                        title: title.read(cx).content().to_owned(),
                                        body: body.read(cx).content().to_owned(),
                                        base: base.read(cx).content().to_owned(),
                                    };
                                    this.prepare_delivery(session_id, intent, cx);
                                });
                            }),
                        ),
                )
        }));
        cx.notify();
    }
    fn present_delivery_plan(
        &mut self,
        session_id: Uuid,
        context: AgentExecutionContext,
        plan: DeliveryPlan,
        cx: &mut Context<Self>,
    ) {
        let status = plan.status();
        let branch = status.snapshot.branch.clone().unwrap_or_default();
        let head = status.snapshot.head.clone().unwrap_or_default();
        let origin = status.remote.clone().unwrap_or_default();
        let actor = plan.actor().map(str::to_owned);
        let directory = status.snapshot.root.display().to_string();
        let preview = plan.preview().join("\n");
        let intent = plan.intent().clone();
        let is_push = matches!(intent, DeliveryIntent::Push);
        let parent = cx.entity();
        let close_parent = parent.clone();
        let cancel_parent = parent.clone();
        self.delivery.pending = Some(PendingDelivery {
            session_id,
            context,
            plan,
        });
        self.delivery.dialog = Some(cx.new(move |dialog_cx| {
            let mut content = div()
                .flex()
                .flex_col()
                .min_w(px(0.0))
                .gap(px(8.0))
                .child(directory)
                .child(t!("agents.delivery.branch", branch = branch).to_string())
                .child(t!("agents.delivery.head", head = head).to_string())
                .child(t!("agents.delivery.origin", origin = origin).to_string());
            if let Some(actor) = actor {
                content = content.child(t!("agents.delivery.actor", actor = actor).to_string());
            }
            if let DeliveryIntent::CreatePullRequest { title, body, base } = intent {
                content = content
                    .child(format!("{}: {title}", t!("agents.delivery.title_label")))
                    .child(format!("{}: {base}", t!("agents.delivery.base_label")));
                let mut body_preview = div().flex().flex_col().text_size(px(11.0)).gap(px(2.0));
                for line in body.lines() {
                    body_preview = body_preview.child(div().min_h(px(14.0)).child(line.to_owned()));
                }
                content = content.child(body_preview);
            }
            content = content.child(
                div()
                    .font_family("JetBrains Mono")
                    .text_size(px(10.0))
                    .child(preview),
            );
            DeliveryDialog::new(dialog_cx)
                .title(t!("agents.delivery.confirm_title").to_string())
                .description(t!("agents.delivery.confirm_description").to_string())
                .size(DialogSize::Lg)
                .on_close(move |_, cx| {
                    close_parent.update(cx, |this, cx| this.close_delivery_dialog(cx));
                })
                .child(content)
                .footer(
                    div()
                        .flex()
                        .justify_end()
                        .gap(px(8.0))
                        .child(
                            Button::new(
                                "agent-delivery-confirm-cancel",
                                t!("agents.review.cancel").to_string(),
                            )
                            .variant(ButtonVariant::Ghost)
                            .on_click(move |_, _, cx| {
                                cancel_parent.update(cx, |this, cx| this.close_delivery_dialog(cx));
                            }),
                        )
                        .child(
                            Button::new(
                                "agent-delivery-confirm",
                                t!(if is_push {
                                    "agents.delivery.confirm_push"
                                } else {
                                    "agents.delivery.confirm_pr"
                                })
                                .to_string(),
                            )
                            .on_click(move |_, _, cx| {
                                parent.update(cx, |this, cx| this.execute_delivery(cx));
                            }),
                        ),
                )
        }));
        cx.notify();
    }
    fn execute_delivery(&mut self, cx: &mut Context<Self>) {
        let Some(pending) = self.delivery.pending.take() else {
            return;
        };
        self.delivery.dialog = None;
        let session_id = pending.session_id;
        let context = pending.context;
        let ready = self.delivery_context_matches(session_id, &context, cx)
            && self
                .sessions
                .get(session_id)
                .is_some_and(|session| !session.status.is_active());
        let state = self.delivery.sessions.entry(session_id).or_default();
        if !ready || state.loading {
            state.error = Some(t!("agents.delivery.context_changed").to_string());
            cx.notify();
            return;
        }
        state.loading = true;
        state.error = None;
        state.generation = state.generation.wrapping_add(1);
        let generation = state.generation;
        let directory = pending.plan.status().snapshot.root.clone();
        let task = cx.background_executor().spawn(async move {
            let backend = AgentDeliveryBackend::default();
            let outcome = backend
                .execute(&pending.plan, pending.plan.confirm())
                .map_err(|error| error.to_string())?;
            let status = backend.status(&directory).map_err(|error| {
                format!("{}: {error}", t!("agents.delivery.refresh_after_success"))
            })?;
            Ok::<_, String>((outcome, status))
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                let Some(state) = this.delivery.sessions.get_mut(&session_id) else {
                    return;
                };
                if state.generation != generation {
                    return;
                }
                state.loading = false;
                state.github = None;
                match result {
                    Ok((outcome, status)) => {
                        state.notice = Some(t!("agents.delivery.completed").to_string());
                        if let Some(pr) = outcome.pull_request {
                            state.github = Some(GitHubDeliveryStatus {
                                actor: pr.author.login.clone(),
                                actor_permitted: true,
                                pull_request: Some(pr),
                                checks: Vec::new(),
                                mergeable: None,
                                merge_state: None,
                                head_matches: true,
                            });
                        }
                        state.status = Some(Arc::new(status));
                    }
                    Err(error) => {
                        state.error = Some(error);
                        state.status = None;
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    pub(super) fn render_delivery_dialog(&self) -> Option<AnyElement> {
        self.delivery
            .dialog
            .clone()
            .map(IntoElement::into_any_element)
    }
    pub(super) fn render_delivery_controls(
        &mut self,
        session: &AgentSession,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let session_id = session.id;
        let context = self.displayed_context(cx);
        let state = self.delivery.sessions.entry(session_id).or_default();
        if state.title.is_none() {
            state.title = Some(input_state(cx, String::new()));
        }
        if state.body.is_none() {
            state.body = Some(cx.new(|cx| InputState::new(cx).multi_line(true)));
        }
        if state.base.is_none() {
            state.base = Some(input_state(cx, "main".into()));
        }
        let loading = state.loading;
        let matches = state.context.as_ref() == Some(&context);
        let status = if matches { state.status.clone() } else { None };
        let github = if matches { state.github.clone() } else { None };
        let error = if matches { state.error.clone() } else { None };
        let notice = if matches { state.notice.clone() } else { None };
        let local = matches!(context.target, AgentTarget::Local);
        let disabled = loading || session.status.is_active();
        let mut controls = div()
            .flex()
            .flex_col()
            .min_w(px(0.0))
            .gap(px(7.0))
            .px(px(10.0))
            .py(px(8.0))
            .border_b_1()
            .border_color(ShellDeckColors::border())
            .text_size(px(11.0))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap(px(6.0))
                    .child(
                        div()
                            .flex_1()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(t!("agents.delivery.title").to_string()),
                    )
                    .child(
                        Button::new(
                            "agent-delivery-refresh",
                            t!("agents.delivery.refresh").to_string(),
                        )
                        .size(ButtonSize::Sm)
                        .variant(ButtonVariant::Ghost)
                        .disabled(loading || !local)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.refresh_delivery(session_id, false, cx)
                        })),
                    ),
            );
        if !local {
            return controls
                .child(t!("agents.delivery.remote").to_string())
                .into_any_element();
        }
        if loading {
            controls = controls.child(t!("agents.delivery.loading").to_string());
        }
        if let Some(error) = error {
            controls = controls.child(div().text_color(ShellDeckColors::error()).child(error));
        }
        if let Some(notice) = notice {
            controls = controls.child(div().text_color(ShellDeckColors::success()).child(notice));
        }
        if let Some(status) = &status {
            let snapshot = &status.snapshot;
            let branch = snapshot
                .branch
                .clone()
                .unwrap_or_else(|| t!("agents.delivery.detached").to_string());
            let head = snapshot
                .head
                .clone()
                .unwrap_or_else(|| t!("agents.delivery.unborn").to_string());
            controls = controls
                .child(t!("agents.delivery.branch", branch = branch).to_string())
                .child(
                    div().text_color(ShellDeckColors::text_muted()).child(
                        t!(
                            "agents.delivery.head",
                            head = head.chars().take(12).collect::<String>()
                        )
                        .to_string(),
                    ),
                )
                .child(
                    div().text_color(ShellDeckColors::text_muted()).child(
                        t!(
                            "agents.delivery.origin",
                            origin = status
                                .remote
                                .clone()
                                .unwrap_or_else(|| t!("agents.delivery.no_origin").to_string())
                        )
                        .to_string(),
                    ),
                )
                .child(t!("agents.delivery.dirty", count = snapshot.status.len()).to_string());
            if let Some(upstream) = &status.upstream {
                controls = controls.child(
                    t!(
                        "agents.delivery.upstream",
                        upstream = upstream,
                        ahead = status.ahead.unwrap_or(0),
                        behind = status.behind.unwrap_or(0)
                    )
                    .to_string(),
                );
            } else {
                controls = controls.child(
                    div()
                        .text_color(ShellDeckColors::text_muted())
                        .child(t!("agents.delivery.no_upstream").to_string()),
                );
            }
            let can_deliver = snapshot.can_mutate()
                && snapshot.branch.is_some()
                && snapshot.head.is_some()
                && status.remote.is_some();
            let existing_pr = github
                .as_ref()
                .is_some_and(|github| github.pull_request.is_some());
            controls = controls
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap(px(6.0))
                        .child(
                            Button::new(
                                "agent-delivery-push",
                                t!("agents.delivery.push").to_string(),
                            )
                            .size(ButtonSize::Sm)
                            .variant(ButtonVariant::Secondary)
                            .disabled(disabled || !can_deliver)
                            .on_click(cx.listener(
                                move |this, _, _, cx| {
                                    this.prepare_delivery(session_id, DeliveryIntent::Push, cx)
                                },
                            )),
                        )
                        .child(
                            Button::new(
                                "agent-delivery-create-pr",
                                t!("agents.delivery.create_pr").to_string(),
                            )
                            .size(ButtonSize::Sm)
                            .variant(ButtonVariant::Secondary)
                            .disabled(
                                disabled || !can_deliver || status.github.is_none() || existing_pr,
                            )
                            .on_click(cx.listener(
                                move |this, _, _, cx| {
                                    this.present_pull_request_form(session_id, cx)
                                },
                            )),
                        ),
                )
                .child(
                    Button::new(
                        "agent-delivery-github",
                        t!("agents.delivery.github").to_string(),
                    )
                    .size(ButtonSize::Sm)
                    .variant(ButtonVariant::Ghost)
                    .disabled(loading || status.github.is_none())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.refresh_delivery(session_id, true, cx)
                    })),
                );
        } else if !loading {
            controls = controls.child(
                div()
                    .text_color(ShellDeckColors::text_muted())
                    .child(t!("agents.delivery.load_hint").to_string()),
            );
        }
        if let Some(github) = github {
            controls =
                controls.child(t!("agents.delivery.actor", actor = github.actor).to_string());
            if !github.actor_permitted {
                controls = controls.child(
                    div()
                        .text_color(ShellDeckColors::warning())
                        .child(t!("agents.delivery.actor_denied").to_string()),
                );
            }
            if let Some(pr) = github.pull_request {
                let url = pr.url.clone();
                controls = controls.child(
                    Button::new(
                        "agent-delivery-open-pr",
                        format!("#{} {}", pr.number, pr.title),
                    )
                    .size(ButtonSize::Sm)
                    .variant(ButtonVariant::Link)
                    .on_click(move |_, _, cx| cx.open_url(&url)),
                );
                if !github.head_matches {
                    controls = controls.child(
                        div()
                            .text_color(ShellDeckColors::warning())
                            .child(t!("agents.delivery.stale_pr").to_string()),
                    );
                }
                if let (Some(mergeable), Some(state)) = (github.mergeable, github.merge_state) {
                    controls = controls.child(
                        t!(
                            "agents.delivery.readiness",
                            mergeable = mergeable,
                            state = state
                        )
                        .to_string(),
                    );
                }
                if github.checks.is_empty() {
                    controls = controls.child(
                        div()
                            .text_color(ShellDeckColors::text_muted())
                            .child(t!("agents.delivery.checks_empty").to_string()),
                    );
                }
                for (index, check) in github.checks.into_iter().enumerate() {
                    let label = format!("{} · {}", check.name, check.state);
                    if let Some(url) = check.url {
                        controls = controls.child(
                            Button::new(("agent-delivery-check", index), label)
                                .size(ButtonSize::Sm)
                                .variant(ButtonVariant::Ghost)
                                .on_click(move |_, _, cx| cx.open_url(&url)),
                        );
                    } else {
                        controls = controls.child(label);
                    }
                }
            } else {
                controls = controls.child(t!("agents.delivery.no_pr").to_string());
            }
        }
        controls = controls.child(
            Button::new(
                "agent-delivery-worktrees",
                t!("agents.delivery.workspace").to_string(),
            )
            .size(ButtonSize::Sm)
            .variant(ButtonVariant::Ghost)
            .disabled(session.status.is_active())
            .on_click(
                cx.listener(move |_, _, _, cx| cx.emit(AgentConsoleEvent::OpenWorkspaceLauncher)),
            ),
        );
        controls.into_any_element()
    }
}
fn delivery_field(label: String, input: Input) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(5.0))
        .child(div().text_size(px(11.0)).child(label))
        .child(input)
}
