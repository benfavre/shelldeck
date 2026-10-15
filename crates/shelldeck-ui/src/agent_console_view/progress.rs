//! Provider-advertised progress, context usage and exact ACP decisions.
use super::*;
use shelldeck_core::agent_runtime::{AgentApprovalOptionKind, AgentPlanStepStatus};

impl AgentConsoleView {
    pub(super) fn render_run_progress(
        &self,
        session: &AgentSession,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mut column = div().flex().flex_col().min_w(px(0.0)).gap(px(8.0));
        if let Some(plan) = &session.plan {
            let mut card = div()
                .flex()
                .flex_col()
                .gap(px(6.0))
                .p(px(12.0))
                .border_1()
                .border_color(ShellDeckColors::border())
                .rounded(px(8.0));
            card = card.child(
                div()
                    .text_size(px(11.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(t!("agents.progress.plan").to_string()),
            );
            for step in &plan.steps {
                let (icon, color) = match step.status {
                    AgentPlanStepStatus::Completed => ("check", ShellDeckColors::success()),
                    AgentPlanStepStatus::InProgress => ("refresh-cw", ShellDeckColors::primary()),
                    AgentPlanStepStatus::Pending => ("clock", ShellDeckColors::text_muted()),
                };
                card = card.child(
                    div()
                        .flex()
                        .items_start()
                        .gap(px(8.0))
                        .child(lucide_icon(icon, 13.0, color))
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.0))
                                .text_size(px(11.0))
                                .child(step.text.clone()),
                        ),
                );
            }
            column = column.child(card);
        }
        let mut info = Vec::new();
        if let Some(metadata) = &session.metadata {
            if let Some(model) = &metadata.model {
                info.push(model.clone());
            }
        }
        if let Some(usage) = &session.usage {
            if let Some(input) = usage.input_tokens {
                info.push(t!("agents.progress.input_tokens", count = input).to_string());
            }
            if let Some(output) = usage.output_tokens {
                info.push(t!("agents.progress.output_tokens", count = output).to_string());
            }
            if let (Some(used), Some(size)) = (usage.context_used, usage.context_size) {
                info.push(t!("agents.progress.context", used = used, size = size).to_string());
            }
            if let (Some(cost), Some(currency)) = (&usage.cost_amount, &usage.cost_currency) {
                info.push(format!(
                    "{} {} {}",
                    cost,
                    currency,
                    t!(if usage.cumulative {
                        "agents.progress.session_estimate"
                    } else {
                        "agents.progress.turn_estimate"
                    })
                ));
            }
        }
        if !info.is_empty() {
            column = column.child(
                div()
                    .text_size(px(10.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(info.join(" · ")),
            );
        }
        if let Some(question) = &session.question {
            let mut card = div()
                .p(px(12.0))
                .border_1()
                .border_color(ShellDeckColors::border())
                .rounded(px(8.0))
                .flex()
                .flex_col()
                .gap(px(8.0))
                .child(div().text_size(px(12.0)).child(question.prompt.clone()));
            for (index, option) in question.options.iter().enumerate() {
                let answer = option.label.clone();
                let live_question = question.response_supported.then(|| question.id.clone());
                let run_id = self.session_ui.get(&session.id).and_then(|ui| ui.run_id);
                card = card.child(
                    Button::new(("agent-question", index), option.label.clone())
                        .variant(ButtonVariant::Outline)
                        .size(ButtonSize::Sm)
                        .tooltip(option.description.clone())
                        .disabled(
                            run_id.is_some_and(|run_id| {
                                self.pending_live_inputs.contains_key(&run_id)
                            }),
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if let (Some(question_id), Some(run_id)) = (&live_question, run_id) {
                                if !this.reserve_live_input(
                                    run_id,
                                    PendingLiveInput::question(None, question_id.clone()),
                                    cx,
                                ) {
                                    return;
                                }
                                cx.emit(AgentConsoleEvent::QuestionAnswer {
                                    run_id,
                                    question_id: question_id.clone(),
                                    answer: answer.clone(),
                                });
                                return;
                            }
                            if let Some(id) = this.sessions.selected_id() {
                                if let Some(ui) = this.session_ui.get(&id) {
                                    ui.prompt.update(cx, |state, cx| {
                                        state.replace_content(answer.clone(), cx)
                                    });
                                }
                            }
                            cx.notify();
                        })),
                );
            }
            card = card.child(
                div()
                    .text_size(px(10.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(
                        t!(if question.response_supported {
                            "agents.live.answer_hint"
                        } else {
                            "agents.progress.question_followup"
                        })
                        .to_string(),
                    ),
            );
            column = column.child(card);
        }
        if let (Some(approval), Some(run_id)) = (
            &session.approval,
            self.session_ui.get(&session.id).and_then(|ui| ui.run_id),
        ) {
            let card = div()
                .p(px(12.0))
                .border_1()
                .border_color(ShellDeckColors::warning())
                .rounded(px(8.0))
                .flex()
                .flex_col()
                .gap(px(8.0))
                .child(
                    div()
                        .text_size(px(11.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(t!("agents.progress.permission").to_string()),
                )
                .child(div().text_size(px(12.0)).child(approval.title.clone()));
            let mut choices = div().flex().flex_wrap().gap(px(8.0));
            for (index, option) in approval.options.iter().enumerate() {
                let request_id = approval.id.clone();
                let option_id = option.id.clone();
                let variant = if matches!(option.kind, AgentApprovalOptionKind::AllowOnce) {
                    ButtonVariant::Default
                } else {
                    ButtonVariant::Outline
                };
                choices = choices.child(
                    Button::new(("agent-permission", index), option.label.clone())
                        .variant(variant)
                        .size(ButtonSize::Sm)
                        .on_click(cx.listener(move |_, _, _, cx| {
                            cx.emit(AgentConsoleEvent::ApprovalDecision {
                                run_id,
                                request_id: request_id.clone(),
                                option_id: Some(option_id.clone()),
                            });
                            cx.notify();
                        })),
                );
            }
            let request_id = approval.id.clone();
            choices = choices.child(
                Button::new(
                    "agent-permission-cancel",
                    t!("agents.progress.cancel_permission").to_string(),
                )
                .variant(ButtonVariant::Ghost)
                .size(ButtonSize::Sm)
                .on_click(cx.listener(move |_, _, _, cx| {
                    cx.emit(AgentConsoleEvent::ApprovalDecision {
                        run_id,
                        request_id: request_id.clone(),
                        option_id: None,
                    });
                    cx.notify();
                })),
            );
            column = column.child(card.child(choices));
        }
        for subagent in &session.subagents {
            column = column.child(
                div()
                    .flex()
                    .items_start()
                    .gap(px(8.0))
                    .child(lucide_icon(
                        "bot",
                        13.0,
                        status_trace_color(subagent.status),
                    ))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .flex()
                            .flex_col()
                            .text_size(px(11.0))
                            .child(subagent.title.clone())
                            .child(subagent.summary.clone().unwrap_or_default()),
                    ),
            );
        }
        for (index, source) in session.sources.iter().enumerate() {
            let uri = source.uri.clone();
            let mut button = Button::new(("agent-source", index), source.title.clone())
                .variant(ButtonVariant::Ghost)
                .size(ButtonSize::Sm);
            if let Ok(url) = url::Url::parse(&uri) {
                if matches!(url.scheme(), "http" | "https")
                    && url.username().is_empty()
                    && url.password().is_none()
                {
                    button = button.on_click(move |_, _, cx| cx.open_url(&uri));
                }
            }
            column = column.child(button);
        }
        for (index, artifact) in session.artifacts.iter().enumerate() {
            let uri = artifact.uri.clone();
            column = column.child(
                Button::new(
                    ("agent-artifact", index),
                    format!("{} · {}", artifact.title, t!("agents.progress.copy_link")),
                )
                .variant(ButtonVariant::Outline)
                .size(ButtonSize::Sm)
                .on_click(move |_, _, cx| {
                    cx.write_to_clipboard(ClipboardItem::new_string(uri.clone()))
                }),
            );
        }
        column.into_any_element()
    }
}

fn status_trace_color(status: AgentTraceStatus) -> Hsla {
    match status {
        AgentTraceStatus::Succeeded => ShellDeckColors::success(),
        AgentTraceStatus::Failed => ShellDeckColors::error(),
        AgentTraceStatus::Running => ShellDeckColors::primary(),
        _ => ShellDeckColors::text_muted(),
    }
}
