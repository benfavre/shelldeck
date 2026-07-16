use adabraka_ui::components::input::{Input, InputSize};
use adabraka_ui::components::input_state::InputState;
use adabraka_ui::prelude::{
    scrollable_vertical, Button, ButtonSize, ButtonVariant, Spinner, SpinnerSize, SpinnerVariant,
};
use gpui::prelude::*;
use gpui::*;
use shelldeck_core::ai::{AiBackend, AiContext, AiSurface};

use crate::icons::{ai_provider_badge, lucide_icon};
use crate::scale::px;
use crate::t;
use crate::theme::ShellDeckColors;

#[derive(Debug, Clone)]
pub enum AiAssistantEvent {
    Submit {
        request_id: u64,
        prompt: String,
        context: AiContext,
    },
}

impl EventEmitter<AiAssistantEvent> for AiAssistantView {}

#[derive(Default)]
struct AiRequestGate {
    epoch: u64,
}

impl AiRequestGate {
    fn invalidate(&mut self) {
        self.epoch = self.epoch.wrapping_add(1);
    }

    fn begin(&mut self) -> u64 {
        self.invalidate();
        self.epoch
    }

    fn accepts(&self, request_id: u64) -> bool {
        request_id == self.epoch
    }
}

pub struct AiAssistantView {
    prompt_state: Entity<InputState>,
    context: AiContext,
    loading: bool,
    result: String,
    error: Option<String>,
    request_gate: AiRequestGate,
    backend: AiBackend,
    model: String,
}

impl AiAssistantView {
    pub fn new(context: AiContext, cx: &mut Context<Self>) -> Self {
        Self {
            prompt_state: cx.new(|cx| InputState::new(cx).multi_line(true)),
            context,
            loading: false,
            result: String::new(),
            error: None,
            request_gate: AiRequestGate::default(),
            backend: AiBackend::Disabled,
            model: String::new(),
        }
    }

    pub fn set_backend(&mut self, backend: AiBackend, model: String, cx: &mut Context<Self>) {
        self.backend = backend;
        self.model = model;
        cx.notify();
    }

    pub fn set_context(&mut self, context: AiContext, cx: &mut Context<Self>) {
        self.request_gate.invalidate();
        self.context = context;
        self.loading = false;
        self.result.clear();
        self.error = None;
        cx.notify();
    }

    pub fn set_loading(&mut self, loading: bool, cx: &mut Context<Self>) {
        self.loading = loading;
        if loading {
            self.error = None;
        }
        cx.notify();
    }

    pub fn set_result(
        &mut self,
        request_id: u64,
        result: Result<String, String>,
        cx: &mut Context<Self>,
    ) {
        if !self.request_gate.accepts(request_id) {
            return;
        }
        self.loading = false;
        match result {
            Ok(text) => {
                self.result = text;
                self.error = None;
            }
            Err(error) => self.error = Some(error),
        }
        cx.notify();
    }

    fn submit(&mut self, cx: &mut Context<Self>) {
        if self.loading {
            return;
        }
        let prompt = self.prompt_state.read(cx).content().trim().to_string();
        if prompt.is_empty() {
            return;
        }
        self.submit_prompt(prompt, cx);
    }

    fn submit_prompt(&mut self, prompt: String, cx: &mut Context<Self>) {
        if !self.loading {
            let request_id = self.request_gate.begin();
            self.loading = true;
            self.error = None;
            cx.emit(AiAssistantEvent::Submit {
                request_id,
                prompt,
                context: self.context.clone(),
            });
            cx.notify();
        }
    }

    fn quick_actions(surface: AiSurface) -> Vec<(String, String)> {
        let keys: &[(&str, &str)] = match surface {
            AiSurface::Support => &[
                ("ai.quick.support_reply", "ai.prompt.support_reply"),
                ("ai.quick.summarize", "ai.prompt.support_summary"),
                ("ai.quick.triage", "ai.prompt.support_triage"),
            ],
            AiSurface::Issue => &[
                ("ai.quick.issue_draft", "ai.prompt.issue_draft"),
                ("ai.quick.tags", "ai.prompt.issue_tags"),
                ("ai.quick.priority", "ai.prompt.issue_priority"),
            ],
            AiSurface::Script => &[
                ("ai.quick.generate", "ai.prompt.script_generate"),
                ("ai.quick.explain", "ai.prompt.script_explain"),
                ("ai.quick.convert", "ai.prompt.script_convert"),
                ("ai.quick.review", "ai.prompt.script_review"),
                ("ai.quick.naming", "ai.prompt.naming"),
            ],
            AiSurface::Terminal => &[
                ("ai.quick.command", "ai.prompt.terminal_command"),
                ("ai.quick.error", "ai.prompt.terminal_error"),
                ("ai.quick.issue_draft", "ai.prompt.terminal_issue"),
            ],
            AiSurface::Jean => &[("ai.quick.jean", "ai.prompt.jean")],
            AiSurface::Naming => &[("ai.quick.naming", "ai.prompt.naming")],
            AiSurface::Recent => &[("ai.quick.summarize", "ai.prompt.recent")],
            AiSurface::Global => &[],
        };
        keys.iter()
            .map(|(label, prompt)| (t!(*label).to_string(), t!(*prompt).to_string()))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::AiRequestGate;

    // SDTEST-1341
    #[test]
    fn stale_ai_response_is_rejected_after_context_invalidation() {
        let mut gate = AiRequestGate::default();
        let old_request = gate.begin();
        assert!(gate.accepts(old_request));

        gate.invalidate();
        assert!(!gate.accepts(old_request));

        let current_request = gate.begin();
        assert!(gate.accepts(current_request));
    }
}

impl Render for AiAssistantView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let submit = {
            let entity = cx.entity();
            move |_value: SharedString, cx: &mut App| {
                entity.update(cx, |this, cx| this.submit(cx));
            }
        };
        let prompt = Input::new(&self.prompt_state)
            .size(InputSize::Sm)
            .multi_line(true)
            .min_rows(5)
            .placeholder(t!("ai.assistant.placeholder").to_string())
            .disabled(self.loading)
            .on_enter(submit);

        let mut output = div().flex().flex_col().gap(px(8.0));
        if self.loading {
            output = output.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .text_size(px(12.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(
                        Spinner::new()
                            .size(SpinnerSize::Xs)
                            .variant(SpinnerVariant::Primary),
                    )
                    .child(t!("ai.assistant.generating").to_string()),
            );
        } else if let Some(error) = &self.error {
            output = output.child(
                div()
                    .p(px(10.0))
                    .rounded(px(6.0))
                    .bg(ShellDeckColors::error().opacity(0.10))
                    .text_size(px(12.0))
                    .text_color(ShellDeckColors::error())
                    .child(error.clone()),
            );
        } else if !self.result.is_empty() {
            let result = self.result.clone();
            let result_lines = self
                .result
                .split('\n')
                .map(|line| {
                    div()
                        .min_h(px(18.0))
                        .child(if line.is_empty() { " " } else { line }.to_string())
                })
                .collect::<Vec<_>>();
            output = output
                .child(
                    div()
                        .text_size(px(11.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(ShellDeckColors::text_muted())
                        .child(t!("ai.assistant.draft").to_string()),
                )
                .child(
                    div()
                        .p(px(10.0))
                        .rounded(px(6.0))
                        .border_1()
                        .border_color(ShellDeckColors::border())
                        .bg(ShellDeckColors::bg_primary())
                        .text_size(px(13.0))
                        .text_color(ShellDeckColors::text_primary())
                        .whitespace_normal()
                        .children(result_lines),
                )
                .child(
                    div().flex().justify_end().child(
                        Button::new("ai-copy-draft", t!("ai.assistant.copy").to_string())
                            .variant(ButtonVariant::Outline)
                            .size(ButtonSize::Sm)
                            .on_click(move |_event, _window, cx| {
                                cx.write_to_clipboard(ClipboardItem::new_string(result.clone()));
                            }),
                    ),
                );
        }

        let mut quick_actions = div().flex().flex_wrap().gap(px(8.0));
        for (index, (label, quick_prompt)) in Self::quick_actions(self.context.surface)
            .into_iter()
            .enumerate()
        {
            quick_actions = quick_actions.child(
                Button::new(("ai-quick", index), label)
                    .variant(ButtonVariant::Outline)
                    .size(ButtonSize::Sm)
                    .disabled(self.loading)
                    .on_click(cx.listener(move |this, _, _window, cx| {
                        this.submit_prompt(quick_prompt.clone(), cx);
                    })),
            );
        }

        let model = if self.model.trim().is_empty() {
            self.backend.default_model().to_string()
        } else {
            self.model.clone()
        };

        let body = div()
            .id("ai-assistant-body")
            .flex()
            .flex_col()
            .w_full()
            .px(px(16.0))
            .py(px(14.0))
            .gap(px(12.0))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(px(8.0))
                    .p(px(10.0))
                    .rounded(px(6.0))
                    .border_1()
                    .border_color(ShellDeckColors::primary().opacity(0.35))
                    .bg(ShellDeckColors::primary().opacity(0.08))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .flex_1()
                            .min_w_0()
                            .gap(px(9.0))
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .size(px(30.0))
                                    .rounded(px(6.0))
                                    .bg(ShellDeckColors::primary().opacity(0.16))
                                    .child(lucide_icon(
                                        "sparkles",
                                        15.0,
                                        ShellDeckColors::primary(),
                                    )),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .min_w_0()
                                    .gap(px(1.0))
                                    .child(
                                        div()
                                            .text_size(px(12.0))
                                            .text_color(ShellDeckColors::text_primary())
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .child(t!("ai.identity.label").to_string()),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(11.0))
                                            .text_color(ShellDeckColors::text_muted())
                                            .child(self.context.title.clone()),
                                    ),
                            ),
                    )
                    .child(ai_provider_badge(self.backend, &model)),
            )
            .child(quick_actions)
            .child(prompt)
            .child(
                div().flex().justify_end().child(
                    Button::new("ai-submit", t!("ai.assistant.submit").to_string())
                        .variant(ButtonVariant::Ai)
                        .size(ButtonSize::Sm)
                        .icon(adabraka_ui::components::icon_source::IconSource::from(
                            "sparkles",
                        ))
                        .disabled(self.loading)
                        .on_click(cx.listener(|this, _, _window, cx| this.submit(cx))),
                ),
            )
            .child(output);

        div()
            .flex()
            .flex_col()
            .w_full()
            .flex_1()
            .min_h(px(0.0))
            .overflow_hidden()
            .child(scrollable_vertical(body))
    }
}
