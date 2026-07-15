use adabraka_ui::components::input::{Input, InputSize};
use adabraka_ui::components::input_state::InputState;
use adabraka_ui::prelude::{scrollable_vertical, Button, ButtonVariant};
use gpui::prelude::*;
use gpui::*;
use shelldeck_core::ai::{AiContext, AiSurface};

use crate::scale::px;
use crate::t;
use crate::theme::ShellDeckColors;

#[derive(Debug, Clone)]
pub enum AiAssistantEvent {
    Submit { prompt: String, context: AiContext },
}

impl EventEmitter<AiAssistantEvent> for AiAssistantView {}

pub struct AiAssistantView {
    prompt_state: Entity<InputState>,
    context: AiContext,
    loading: bool,
    result: String,
    error: Option<String>,
}

impl AiAssistantView {
    pub fn new(context: AiContext, cx: &mut Context<Self>) -> Self {
        Self {
            prompt_state: cx.new(|cx| InputState::new(cx).multi_line(true)),
            context,
            loading: false,
            result: String::new(),
            error: None,
        }
    }

    pub fn set_context(&mut self, context: AiContext, cx: &mut Context<Self>) {
        self.context = context;
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

    pub fn set_result(&mut self, result: Result<String, String>, cx: &mut Context<Self>) {
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

    fn submit(&self, cx: &mut Context<Self>) {
        if self.loading {
            return;
        }
        let prompt = self.prompt_state.read(cx).content().trim().to_string();
        if prompt.is_empty() {
            return;
        }
        cx.emit(AiAssistantEvent::Submit {
            prompt,
            context: self.context.clone(),
        });
    }

    fn submit_prompt(&self, prompt: String, cx: &mut Context<Self>) {
        if !self.loading {
            cx.emit(AiAssistantEvent::Submit {
                prompt,
                context: self.context.clone(),
            });
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
            ],
            AiSurface::Terminal => &[
                ("ai.quick.command", "ai.prompt.terminal_command"),
                ("ai.quick.error", "ai.prompt.terminal_error"),
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
                    .text_size(px(12.0))
                    .text_color(ShellDeckColors::text_muted())
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
                        .child(self.result.clone()),
                )
                .child(
                    div().flex().justify_end().child(
                        Button::new("ai-copy-draft", t!("ai.assistant.copy").to_string())
                            .variant(ButtonVariant::Outline)
                            .on_click(move |_event, _window, cx| {
                                cx.write_to_clipboard(ClipboardItem::new_string(result.clone()));
                            }),
                    ),
                );
        }

        let mut quick_actions = div().flex().flex_wrap().gap(px(6.0));
        for (index, (label, quick_prompt)) in Self::quick_actions(self.context.surface)
            .into_iter()
            .enumerate()
        {
            quick_actions = quick_actions.child(
                Button::new(("ai-quick", index), label)
                    .variant(ButtonVariant::Outline)
                    .disabled(self.loading)
                    .on_click(cx.listener(move |this, _, _window, cx| {
                        this.submit_prompt(quick_prompt.clone(), cx);
                    })),
            );
        }

        div()
            .flex()
            .flex_col()
            .size_full()
            .gap(px(12.0))
            .child(
                div()
                    .text_size(px(12.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(self.context.title.clone()),
            )
            .child(quick_actions)
            .child(prompt)
            .child(
                div().flex().justify_end().child(
                    Button::new("ai-submit", t!("ai.assistant.submit").to_string())
                        .variant(ButtonVariant::Default)
                        .disabled(self.loading)
                        .on_click(cx.listener(|this, _, _window, cx| this.submit(cx))),
                ),
            )
            .child(scrollable_vertical(output))
    }
}
