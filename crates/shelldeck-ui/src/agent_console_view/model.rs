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

    fn begin(&mut self, scope: &str) -> Option<Uuid> {
        if self.loading && self.scope.as_deref() == Some(scope) {
            return None;
        }
        if self.scope.as_deref() != Some(scope) {
            self.catalog = None;
        }
        let id = Uuid::new_v4();
        self.scope = Some(scope.to_owned());
        self.request_id = Some(id);
        self.loading = true;
        self.error = None;
        Some(id)
    }

    fn finish(&mut self, id: Uuid, result: Result<CodexModelCatalog, String>) {
        if self.request_id != Some(id) {
            return;
        }
        self.loading = false;
        match result {
            Ok(catalog) => {
                self.catalog = Some(catalog);
                self.error = None;
            }
            Err(error) => self.error = Some(error),
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
        if context.provider != AgentProvider::Codex || context.target != AgentTarget::Local {
            return;
        }
        let scope = context.workdir;
        let Some(request_id) = self.model_discovery.begin(&scope) else {
            return;
        };
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
                view.model_discovery.finish(request_id, result);
                cx.notify();
            });
        })
        .detach();
    }

    fn choose_discovered_model(
        &mut self,
        session_id: Option<Uuid>,
        expected_context: &AgentExecutionContext,
        value: String,
        advertised: Option<&shelldeck_core::agent_codex::model_discovery::CodexDiscoveredModel>,
        cx: &mut Context<Self>,
    ) {
        if self.sessions.selected_id() != session_id
            || self.context_controls_locked()
            || self.displayed_context(cx) != *expected_context
        {
            return;
        }
        self.model_state
            .update(cx, |state, cx| state.replace_content(value, cx));
        if let Some(id) = session_id {
            if self.composer_ui.effort.get(&id).is_some_and(|effort| {
                advertised.is_none_or(|model| {
                    !model
                        .supported_reasoning_efforts
                        .iter()
                        .any(|option| option.executable_effort() == Some(*effort))
                })
            }) {
                self.composer_ui.effort.remove(&id);
            }
        }
        self.schedule_library_save(cx);
        cx.notify();
    }

    pub(super) fn render_model_popover(&self, cx: &mut Context<Self>) -> AnyElement {
        let configured = self.model_state.read(cx).content().trim().to_string();
        let context = self.displayed_context(cx);
        let catalog = (context.provider == AgentProvider::Codex
            && context.target == AgentTarget::Local
            && self.model_discovery.scope.as_deref() == Some(context.workdir.as_str()))
        .then_some(self.model_discovery.catalog.as_ref())
        .flatten();
        let display = catalog
            .and_then(|catalog| selected_catalog_model(&catalog.models, &configured))
            .map(|model| model.display_name.clone())
            .unwrap_or_else(|| {
                if configured.is_empty() {
                    t!("agents.model.auto").to_string()
                } else {
                    configured
                }
            });
        let effort = self
            .sessions
            .selected_id()
            .and_then(|id| self.composer_ui.effort.get(&id).copied());
        let supports_effort = context.provider.capabilities().reasoning_effort;
        let label = if supports_effort && effort.is_some() {
            format!(
                "{} · {}",
                compact_model_name(&display, 20),
                reasoning_label(effort)
            )
        } else {
            compact_model_name(&display, 20)
        };
        let tooltip = if supports_effort {
            format!(
                "{}: {}\n{}: {}",
                t!("agents.model.label"),
                display,
                t!("agents.model.effort"),
                reasoning_label(effort),
            )
        } else {
            format!("{}: {display}", t!("agents.model.label"))
        };
        let trigger = Button::new("agent-model-trigger", label)
            .variant(ButtonVariant::Ghost)
            .size(ButtonSize::Sm)
            .icon(IconSource::from("chevron-down"))
            .icon_position(adabraka_ui::components::button::IconPosition::End)
            .tooltip(tooltip)
            .h(px(30.0))
            .max_w(px(260.0))
            .min_w_0()
            .px(px(8.0));
        let parent = cx.entity();
        Popover::new("agent-model-popover")
            .anchor(Corner::BottomRight)
            .trigger(trigger)
            .content(move |window, cx| {
                // Opening the picker is an explicit request for available models.
                parent.update(cx, |view, cx| {
                    let context = view.displayed_context(cx);
                    if view.model_discovery.scope.as_deref() != Some(context.workdir.as_str())
                        || (view.model_discovery.catalog.is_none()
                            && view.model_discovery.error.is_none())
                    {
                        view.discover_local_codex_models(cx);
                    }
                });
                let parent = parent.clone();
                cx.new(move |content_cx| {
                    let observed = parent.clone();
                    // Keep search/focus stable while discovery and the parent repaint.
                    let search = content_cx.new(InputState::new);
                    content_cx.observe(&parent, |_, _, cx| cx.notify()).detach();
                    content_cx.observe(&search, |_, _, cx| cx.notify()).detach();
                    PopoverContent::new(window, content_cx, move |window, cx| {
                        let view = observed.read(cx);
                        let session_id = view.sessions.selected_id();
                        let locked = view.context_controls_locked();
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
                        let viewport = window.viewport_size();
                        let mut content = div()
                            .id("agent-model-options")
                            .w(px(320.0))
                            .max_w(gpui::px((viewport.width.to_f64() as f32 - 48.0).max(160.0)))
                            .max_h(gpui::px(
                                (viewport.height.to_f64() as f32 - 100.0).clamp(100.0, 480.0),
                            ))
                            .min_h_0()
                            .flex()
                            .flex_col()
                            .gap(px(10.0))
                            .overflow_y_scroll()
                            .child(
                                div()
                                    .text_size(px(13.0))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(t!("agents.model.label").to_string()),
                            );
                        if locked {
                            content = content.child(
                                div()
                                    .text_size(px(11.0))
                                    .text_color(ShellDeckColors::text_muted())
                                    .child(t!("agents.model.locked").to_string()),
                            );
                        }
                        let automatic_parent = observed.clone();
                        let automatic_context = context.clone();
                        content = content.child(
                            Button::new(
                                "agent-model-automatic",
                                t!("agents.model.auto").to_string(),
                            )
                            .variant(ButtonVariant::Ghost)
                            .size(ButtonSize::Sm)
                            .selected(configured.is_empty())
                            .disabled(locked)
                            .icon(IconSource::from(if configured.is_empty() {
                                "check"
                            } else {
                                "sparkles"
                            }))
                            .w_full()
                            .justify_start()
                            .on_click(move |_, _, cx| {
                                automatic_parent.update(cx, |view, cx| {
                                    view.choose_discovered_model(
                                        session_id,
                                        &automatic_context,
                                        String::new(),
                                        None,
                                        cx,
                                    );
                                });
                            }),
                        );
                        if local_codex {
                            let refresh_parent = observed.clone();
                            let refresh_label = t!(if loading {
                                "agents.model.discovering"
                            } else {
                                "agents.model.discover"
                            })
                            .to_string();
                            let refresh = Button::new(
                                "agent-discover-models",
                                if catalog.is_none() {
                                    refresh_label.clone()
                                } else {
                                    String::new()
                                },
                            )
                            .variant(ButtonVariant::Ghost)
                            .size(ButtonSize::Sm)
                            .icon(IconSource::from("refresh-cw"))
                            .h(px(28.0))
                            .tooltip(refresh_label)
                            .disabled(loading)
                            .on_click(move |_, _, cx| {
                                refresh_parent
                                    .update(cx, |view, cx| view.discover_local_codex_models(cx));
                            });
                            content = if catalog.is_none() {
                                content.child(refresh.w_full())
                            } else {
                                content.child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .justify_between()
                                        .gap(px(8.0))
                                        .child(
                                            div()
                                                .text_size(px(11.0))
                                                .font_weight(FontWeight::SEMIBOLD)
                                                .child(t!("agents.model.available").to_string()),
                                        )
                                        .child(refresh.w(px(28.0))),
                                )
                            };
                            if let Some(error) = error {
                                content = content.child(
                                    div()
                                        .text_size(px(11.0))
                                        .text_color(ShellDeckColors::warning())
                                        .child(
                                            t!("agents.model.discovery_failed", error = error)
                                                .to_string(),
                                        ),
                                );
                            }
                        }
                        if let Some(catalog) = &catalog {
                            if catalog.models.is_empty() {
                                content = content.child(
                                    div()
                                        .text_size(px(11.0))
                                        .text_color(ShellDeckColors::text_muted())
                                        .child(t!("agents.model.discovery_empty").to_string()),
                                );
                            } else {
                                content = content.child(
                                    Input::new(&search)
                                        .size(InputSize::Sm)
                                        .placeholder(t!("agents.model.search").to_string()),
                                );
                                let query = search.read(cx).content().trim().to_lowercase();
                                let mut matches = 0;
                                for (index, model) in catalog.models.iter().enumerate() {
                                    if !model_matches_search(model, &query) {
                                        continue;
                                    }
                                    matches += 1;
                                    let model = model.clone();
                                    let select_parent = observed.clone();
                                    let selection_context = context.clone();
                                    let selected = model.model == configured;
                                    let tooltip = format!("{}\n{}", model.model, model.description);
                                    content = content.child(
                                        Button::new(
                                            ("agent-model-choice", index),
                                            compact_model_name(&model.display_name, 34),
                                        )
                                        .variant(ButtonVariant::Ghost)
                                        .size(ButtonSize::Sm)
                                        .w_full()
                                        .justify_start()
                                        .selected(selected)
                                        .disabled(locked)
                                        .icon(IconSource::from(if selected {
                                            "check"
                                        } else {
                                            "sparkles"
                                        }))
                                        .tooltip(tooltip)
                                        .on_click(
                                            move |_, _, cx| {
                                                select_parent.update(cx, |view, cx| {
                                                    view.choose_discovered_model(
                                                        session_id,
                                                        &selection_context,
                                                        model.model.clone(),
                                                        Some(&model),
                                                        cx,
                                                    );
                                                });
                                            },
                                        ),
                                    );
                                }
                                if matches == 0 {
                                    content = content.child(
                                        div()
                                            .text_size(px(11.0))
                                            .text_color(ShellDeckColors::text_muted())
                                            .child(t!("agents.model.no_matches").to_string()),
                                    );
                                }
                            }
                            if catalog.truncated {
                                content = content.child(
                                    div()
                                        .text_size(px(11.0))
                                        .text_color(ShellDeckColors::text_muted())
                                        .child(t!("agents.model.discovery_truncated").to_string()),
                                );
                            }
                        }
                        if supports_effort {
                            let selected_model = catalog.as_ref().and_then(|catalog| {
                                selected_catalog_model(&catalog.models, &configured)
                            });
                            let automatic = selected_model
                                .and_then(|model| model.default_reasoning_effort.as_deref())
                                .map(|effort| {
                                    format!("{} ({effort})", t!("agents.model.effort_auto"))
                                })
                                .unwrap_or_else(|| reasoning_label(None));
                            let efforts =
                                available_reasoning_efforts(selected_model, current_effort)
                                    .into_iter()
                                    .map(|effort| {
                                        (
                                            effort,
                                            if effort.is_none() {
                                                automatic.clone()
                                            } else {
                                                reasoning_label(effort)
                                            },
                                        )
                                    });
                            if selected_model.is_none() {
                                content = content.child(
                                    div()
                                        .text_size(px(11.0))
                                        .text_color(ShellDeckColors::text_muted())
                                        .child(t!("agents.model.effort_unverified").to_string()),
                                );
                            }
                            let mut choices = div().flex().flex_wrap().gap(px(4.0));
                            for (index, (effort, label)) in efforts.enumerate() {
                                let effort_parent = observed.clone();
                                let effort_context = context.clone();
                                choices = choices.child(
                                    Button::new(("agent-reasoning-choice", index), label)
                                        .variant(ButtonVariant::Ghost)
                                        .size(ButtonSize::Sm)
                                        .h(px(28.0))
                                        .px(px(8.0))
                                        .selected(effort == current_effort)
                                        .disabled(locked)
                                        .on_click(move |_, _, cx| {
                                            effort_parent.update(cx, |view, cx| {
                                                if view.sessions.selected_id() != session_id
                                                    || view.context_controls_locked()
                                                    || view.displayed_context(cx) != effort_context
                                                {
                                                    return;
                                                }
                                                if let Some(id) = session_id {
                                                    if let Some(effort) = effort {
                                                        view.composer_ui.effort.insert(id, effort);
                                                    } else {
                                                        view.composer_ui.effort.remove(&id);
                                                    }
                                                }
                                                view.schedule_library_save(cx);
                                                cx.notify();
                                            });
                                        }),
                                );
                            }
                            content = content
                                .child(
                                    div()
                                        .text_size(px(11.0))
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .child(t!("agents.model.effort").to_string()),
                                )
                                .child(choices);
                        }
                        let notify_parent = observed.clone();
                        let input_id = model_state.entity_id();
                        content = content
                            .child(
                                div()
                                    .text_size(px(11.0))
                                    .text_color(ShellDeckColors::text_muted())
                                    .child(t!("agents.model.custom").to_string()),
                            )
                            .child(
                                Input::new(&model_state)
                                    .size(InputSize::Sm)
                                    .disabled(locked)
                                    .placeholder(t!("agents.model.auto").to_string())
                                    .on_change(move |value, cx| {
                                        notify_parent.update(cx, |view, cx| {
                                            if view.sessions.selected_id() != session_id
                                                || view.context_controls_locked()
                                                || view.model_state.entity_id() != input_id
                                                || view.model_state.read(cx).content()
                                                    != value.as_ref()
                                            {
                                                return;
                                            }
                                            let context = view.displayed_context(cx);
                                            let supported = (context.provider
                                                == AgentProvider::Codex
                                                && context.target == AgentTarget::Local
                                                && view.model_discovery.scope.as_deref()
                                                    == Some(context.workdir.as_str()))
                                            .then_some(view.model_discovery.catalog.as_ref())
                                            .flatten()
                                            .and_then(|catalog| {
                                                selected_catalog_model(
                                                    &catalog.models,
                                                    value.trim(),
                                                )
                                            });
                                            if let Some(id) = session_id {
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
                                    }),
                            )
                            .child(
                                div()
                                    .text_size(px(11.0))
                                    .text_color(ShellDeckColors::text_muted())
                                    .child(t!("agents.model.hint").to_string()),
                            );
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

fn reasoning_label(effort: Option<AgentReasoningEffort>) -> String {
    t!(match effort {
        None => "agents.model.effort_auto",
        Some(AgentReasoningEffort::Minimal) => "agents.model.effort_minimal",
        Some(AgentReasoningEffort::Low) => "agents.model.effort_low",
        Some(AgentReasoningEffort::Medium) => "agents.model.effort_medium",
        Some(AgentReasoningEffort::High) => "agents.model.effort_high",
        Some(AgentReasoningEffort::Xhigh) => "agents.model.effort_xhigh",
        Some(AgentReasoningEffort::Max) => "agents.model.effort_max",
        Some(AgentReasoningEffort::Ultra) => "agents.model.effort_ultra",
    })
    .to_string()
}

fn compact_model_name(value: &str, max_characters: usize) -> String {
    let mut characters = value.chars();
    let shortened: String = characters.by_ref().take(max_characters).collect();
    if characters.next().is_some() {
        format!("{shortened}…")
    } else {
        shortened
    }
}

fn model_matches_search(
    model: &shelldeck_core::agent_codex::model_discovery::CodexDiscoveredModel,
    query: &str,
) -> bool {
    query.is_empty()
        || [&model.model, &model.display_name, &model.description]
            .iter()
            .any(|value| value.to_lowercase().contains(query))
}

/// The selected model's advertised options are authoritative. An existing
/// explicit choice remains visible when discovery is unavailable, but no
/// speculative capability list is offered for Automatic or a custom model.
fn available_reasoning_efforts(
    model: Option<&shelldeck_core::agent_codex::model_discovery::CodexDiscoveredModel>,
    current: Option<AgentReasoningEffort>,
) -> Vec<Option<AgentReasoningEffort>> {
    let mut choices = vec![None];
    if let Some(model) = model {
        for option in &model.supported_reasoning_efforts {
            if let Some(effort) = option.executable_effort() {
                if !choices.contains(&Some(effort)) {
                    choices.push(Some(effort));
                }
            }
        }
    } else if let Some(current) = current {
        choices.push(Some(current));
    }
    choices
}

#[cfg(test)]
mod tests {
    use super::*;
    use shelldeck_core::agent_codex::model_discovery::{
        CodexDiscoveredModel, CodexReasoningOption,
    };
    use std::prelude::v1::test;

    // SDTEST-2826 / SDUC-564
    #[test]
    fn model_picker_preserves_known_choices_and_rejects_obsolete_discovery() {
        let model = CodexDiscoveredModel {
            id: "real-model".into(),
            model: "real-model".into(),
            display_name: "Real model".into(),
            description: String::new(),
            is_default: false,
            default_reasoning_effort: Some("low".into()),
            supported_reasoning_efforts: vec![
                CodexReasoningOption {
                    effort: "low".into(),
                    description: String::new(),
                },
                CodexReasoningOption {
                    effort: "future-value".into(),
                    description: String::new(),
                },
            ],
            input_modalities: vec!["text".into()],
        };
        assert_eq!(available_reasoning_efforts(None, None), vec![None]);
        assert_eq!(
            available_reasoning_efforts(Some(&model), None),
            vec![None, Some(AgentReasoningEffort::Low)]
        );
        assert_eq!(
            available_reasoning_efforts(None, Some(AgentReasoningEffort::High)),
            vec![None, Some(AgentReasoningEffort::High)]
        );
        let mut state = AgentModelDiscoveryUi::new();
        let first = state.begin("/first").unwrap();
        assert!(state.begin("/first").is_none());
        let catalog = CodexModelCatalog {
            models: vec![model],
            truncated: false,
        };
        state.finish(first, Ok(catalog.clone()));
        let refresh = state.begin("/first").unwrap();
        state.finish(refresh, Err("offline".into()));
        assert_eq!(state.catalog.as_ref(), Some(&catalog));
        let old = state.begin("/first").unwrap();
        let current = state.begin("/second").unwrap();
        state.finish(old, Ok(catalog.clone()));
        assert!(state.loading);
        assert!(state.catalog.is_none());
        state.finish(current, Ok(catalog));
        assert!(!state.loading);
        assert!(state.error.is_none());
        assert_eq!(state.scope.as_deref(), Some("/second"));
    }
}
