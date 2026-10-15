use super::*;
use shelldeck_core::agent_codex::model_discovery::{
    discover_models, CodexModelCatalog, CodexModelInputError,
};
use shelldeck_core::agent_runtime::AgentReasoningEffort;

pub(super) struct AgentModelDiscoveryUi {
    scope: Option<String>,
    request_id: Option<Uuid>,
    catalog: Option<CodexModelCatalog>,
    loading: bool,
    error: Option<String>,
}

impl AgentModelDiscoveryUi {
    pub(super) fn new() -> Self {
        Self {
            scope: None,
            request_id: None,
            catalog: None,
            loading: false,
            error: None,
        }
    }
}

impl AgentConsoleView {
    pub(super) fn validate_discovered_model_input(
        &self,
        context: &AgentExecutionContext,
        options: &shelldeck_core::agent_runtime::AgentInputOptions,
    ) -> Result<(), String> {
        if context.provider != AgentProvider::Codex
            || context.target != AgentTarget::Local
            || self.model_discovery.scope.as_deref() != Some(context.workdir.as_str())
        {
            return Ok(());
        }
        let Some(model) = self.model_discovery.catalog.as_ref().and_then(|catalog| {
            selected_catalog_model(
                &catalog.models,
                context.model.as_deref().unwrap_or_default(),
            )
        }) else {
            // An omitted model follows Codex configuration; isDefault is only
            // a catalog recommendation, not evidence of that resolved value.
            return Ok(());
        };
        model.validate_input(options).map_err(|error| match error {
            CodexModelInputError::ImagesUnsupported => t!(
                "agents.model.images_unsupported",
                model = model.display_name.clone()
            )
            .to_string(),
            CodexModelInputError::ReasoningEffortUnsupported => t!(
                "agents.model.effort_unsupported",
                model = model.display_name.clone()
            )
            .to_string(),
        })
    }

    fn discover_local_codex_models(&mut self, cx: &mut Context<Self>) {
        let context = self.displayed_context(cx);
        if context.provider != AgentProvider::Codex
            || context.target != AgentTarget::Local
            || self.model_discovery.loading
        {
            return;
        }
        let request_id = Uuid::new_v4();
        let scope = context.workdir;
        self.model_discovery.scope = Some(scope.clone());
        self.model_discovery.request_id = Some(request_id);
        self.model_discovery.loading = true;
        self.model_discovery.error = None;
        self.model_discovery.catalog = None;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let program = crate::workspace::resolve_local_agent_program("codex")
                        .ok_or_else(|| {
                            t!("agents.error.binary_missing", binary = "codex").to_string()
                        })?;
                    discover_models(
                        PathBuf::from(program).as_path(),
                        PathBuf::from(scope).as_path(),
                        std::time::Duration::from_secs(10),
                    )
                    .map_err(|error| error.to_string())
                })
                .await;
            let _ = this.update(cx, |view, cx| {
                if view.model_discovery.request_id != Some(request_id) {
                    return;
                }
                view.model_discovery.loading = false;
                match result {
                    Ok(catalog) => view.model_discovery.catalog = Some(catalog),
                    Err(error) => view.model_discovery.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn render_model_popover(&self, cx: &mut Context<Self>) -> AnyElement {
        let configured = self.model_state.read(cx).content().trim().to_string();
        let display = if configured.is_empty() {
            t!("agents.model.auto").to_string()
        } else {
            configured
        };
        let trigger = div()
            .id("agent-model-trigger")
            .h(px(26.0))
            .max_w(px(180.0))
            .flex()
            .items_center()
            .gap(px(6.0))
            .px(px(6.0))
            .rounded(px(6.0))
            .cursor_pointer()
            .text_size(px(10.5))
            .text_color(ShellDeckColors::text_muted())
            .hover(|style| style.bg(ShellDeckColors::hover_bg()))
            .child(
                div()
                    .min_w(px(0.0))
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .child(display),
            )
            .child(lucide_icon(
                "chevron-down",
                11.0,
                ShellDeckColors::text_muted(),
            ));
        let parent = cx.entity();
        Popover::new("agent-model-popover")
            .anchor(Corner::BottomRight)
            .trigger(trigger)
            .content(move |window, cx| {
                let parent = parent.clone();
                cx.new(move |content_cx| {
                    let observed = parent.clone();
                    content_cx.observe(&parent, |_, _, cx| cx.notify()).detach();
                    PopoverContent::new(window, content_cx, move |_window, cx| {
                        let view = observed.read(cx);
                        let session_id = view.sessions.selected_id();
                        let locked = session_id.is_some_and(|id| {
                            view.context_locks.contains(&id)
                                || view
                                    .sessions
                                    .get(id)
                                    .is_some_and(|session| session.status.is_active())
                        });
                        let context = view.displayed_context(cx);
                        let local_codex = context.provider == AgentProvider::Codex
                            && context.target == AgentTarget::Local;
                        let configured = view.model_state.read(cx).content().trim().to_string();
                        let model_state = view.model_state.clone();
                        let scope_matches = local_codex
                            && view.model_discovery.scope.as_deref()
                                == Some(context.workdir.as_str());
                        let catalog = scope_matches
                            .then(|| view.model_discovery.catalog.clone())
                            .flatten();
                        let loading = scope_matches && view.model_discovery.loading;
                        let error = scope_matches
                            .then(|| view.model_discovery.error.clone())
                            .flatten();
                        let current_effort =
                            session_id.and_then(|id| view.composer_ui.effort.get(&id).copied());
                        let supports_effort = context.provider.capabilities().reasoning_effort;
                        let mut content = div()
                            .w(px(300.0))
                            .max_h(px(480.0))
                            .flex()
                            .flex_col()
                            .gap(px(8.0));
                        content = content.child(
                            div()
                                .text_size(px(11.0))
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(t!("agents.model.label").to_string()),
                        );
                        if local_codex {
                            let refresh_parent = observed.clone();
                            content = content.child(
                                Button::new(
                                    "agent-discover-models",
                                    t!(if loading {
                                        "agents.model.discovering"
                                    } else {
                                        "agents.model.discover"
                                    })
                                    .to_string(),
                                )
                                .variant(ButtonVariant::Outline)
                                .size(ButtonSize::Sm)
                                .disabled(loading)
                                .on_click(move |_, _, cx| {
                                    refresh_parent
                                        .update(cx, |view, cx| view.discover_local_codex_models(cx))
                                }),
                            );
                            if let Some(error) = error {
                                content = content.child(
                                    div()
                                        .text_size(px(10.0))
                                        .text_color(ShellDeckColors::warning())
                                        .child(
                                            t!("agents.model.discovery_failed", error = error)
                                                .to_string(),
                                        ),
                                );
                            }
                        }
                        if let Some(catalog) = &catalog {
                            if !catalog.models.is_empty() {
                                let mut options = vec![SelectOption::new(
                                    String::new(),
                                    t!("agents.model.auto").to_string(),
                                )];
                                options.extend(catalog.models.iter().map(|model| {
                                    SelectOption::new(
                                        model.model.clone(),
                                        model.display_name.clone(),
                                    )
                                }));
                                let selected =
                                    options.iter().position(|option| option.value == configured);
                                let select_parent = observed.clone();
                                let selectable_models = catalog.models.clone();
                                let select = cx.new(move |cx| {
                                    Select::new(cx)
                                        .options(options)
                                        .selected_index(selected)
                                        .searchable(true)
                                        .search_placeholder(t!("agents.model.search").to_string())
                                        .disabled(locked)
                                        .on_change(move |value: &String, _, cx| {
                                            select_parent.update(cx, |view, cx| {
                                                if view.sessions.selected_id() != session_id
                                                    || view
                                                        .sessions
                                                        .selected()
                                                        .is_some_and(|s| s.status.is_active())
                                                    || session_id.is_some_and(|id| {
                                                        view.context_locks.contains(&id)
                                                    })
                                                {
                                                    return;
                                                }
                                                view.model_state.update(cx, |state, cx| {
                                                    state.replace_content(value.clone(), cx)
                                                });
                                                if let Some(id) = session_id {
                                                    let supported = selected_catalog_model(
                                                        &selectable_models,
                                                        value,
                                                    );
                                                    if view.composer_ui.effort.get(&id).is_some_and(
                                                        |effort| {
                                                            supported.is_none_or(|model| {
                                                                !model
                                                                    .supported_reasoning_efforts
                                                                    .iter()
                                                                    .any(|option| {
                                                                        option.executable_effort()
                                                                            == Some(*effort)
                                                                    })
                                                            })
                                                        },
                                                    ) {
                                                        view.composer_ui.effort.remove(&id);
                                                    }
                                                }
                                                cx.notify();
                                            });
                                        })
                                });
                                content = content.child(select);
                            } else {
                                content = content.child(
                                    div()
                                        .text_size(px(10.0))
                                        .text_color(ShellDeckColors::text_muted())
                                        .child(t!("agents.model.discovery_empty").to_string()),
                                );
                            }
                            if catalog.truncated {
                                content = content.child(
                                    div()
                                        .text_size(px(10.0))
                                        .text_color(ShellDeckColors::text_muted())
                                        .child(t!("agents.model.discovery_truncated").to_string()),
                                );
                            }
                        }
                        let notify_parent = observed.clone();
                        content = content
                            .child(
                                Input::new(&model_state)
                                    .size(InputSize::Sm)
                                    .disabled(locked)
                                    .placeholder(t!("agents.model.auto").to_string())
                                    .on_change(move |_, cx| {
                                        notify_parent.update(cx, |view, cx| {
                                            if let Some(id) = session_id {
                                                view.composer_ui.effort.remove(&id);
                                            }
                                            cx.notify();
                                        });
                                    }),
                            )
                            .child(
                                div()
                                    .text_size(px(10.0))
                                    .text_color(ShellDeckColors::text_muted())
                                    .child(t!("agents.model.hint").to_string()),
                            );
                        if supports_effort {
                            let selected_model = catalog.as_ref().and_then(|catalog| {
                                selected_catalog_model(&catalog.models, &configured)
                            });
                            let automatic = selected_model
                                .and_then(|model| model.default_reasoning_effort.as_deref())
                                .map(|effort| {
                                    format!("{} ({effort})", t!("agents.model.effort_auto"))
                                })
                                .unwrap_or_else(|| t!("agents.model.effort_auto").to_string());
                            let mut options = vec![SelectOption::new(None, automatic)];
                            if let Some(model) = selected_model {
                                options.extend(
                                    model
                                        .supported_reasoning_efforts
                                        .iter()
                                        .filter_map(|option| {
                                            option.executable_effort().map(|effort| {
                                                SelectOption::new(
                                                    Some(effort),
                                                    option.effort.clone(),
                                                )
                                            })
                                        }),
                                );
                            } else {
                                options.extend([
                                    SelectOption::new(
                                        Some(AgentReasoningEffort::Low),
                                        t!("agents.model.effort_low").to_string(),
                                    ),
                                    SelectOption::new(
                                        Some(AgentReasoningEffort::Medium),
                                        t!("agents.model.effort_medium").to_string(),
                                    ),
                                    SelectOption::new(
                                        Some(AgentReasoningEffort::High),
                                        t!("agents.model.effort_high").to_string(),
                                    ),
                                    SelectOption::new(
                                        Some(AgentReasoningEffort::Xhigh),
                                        t!("agents.model.effort_xhigh").to_string(),
                                    ),
                                ]);
                            }
                            let selected = options
                                .iter()
                                .position(|option| option.value == current_effort);
                            let effort_parent = observed.clone();
                            let select = cx.new(move |cx| {
                                Select::new(cx)
                                    .options(options)
                                    .selected_index(selected)
                                    .disabled(locked)
                                    .on_change(
                                        move |value: &Option<AgentReasoningEffort>, _, cx| {
                                            effort_parent.update(cx, |view, cx| {
                                                if view.sessions.selected_id() != session_id
                                                    || view
                                                        .sessions
                                                        .selected()
                                                        .is_some_and(|s| s.status.is_active())
                                                    || session_id.is_some_and(|id| {
                                                        view.context_locks.contains(&id)
                                                    })
                                                {
                                                    return;
                                                }
                                                if let Some(id) = session_id {
                                                    match value {
                                                        Some(effort) => {
                                                            view.composer_ui
                                                                .effort
                                                                .insert(id, *effort);
                                                        }
                                                        None => {
                                                            view.composer_ui.effort.remove(&id);
                                                        }
                                                    }
                                                }
                                                cx.notify();
                                            });
                                        },
                                    )
                            });
                            content = content
                                .child(
                                    div()
                                        .text_size(px(11.0))
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .child(t!("agents.model.effort").to_string()),
                                )
                                .child(select)
                                .child(
                                    div()
                                        .text_size(px(10.0))
                                        .text_color(ShellDeckColors::text_muted())
                                        .child(t!("agents.model.effort_hint").to_string()),
                                );
                        }
                        content.into_any_element()
                    })
                })
            })
            .into_any_element()
    }
}

fn selected_catalog_model<'a>(
    models: &'a [shelldeck_core::agent_codex::model_discovery::CodexDiscoveredModel],
    configured: &str,
) -> Option<&'a shelldeck_core::agent_codex::model_discovery::CodexDiscoveredModel> {
    (!configured.is_empty())
        .then(|| models.iter().find(|model| model.model == configured))
        .flatten()
}
