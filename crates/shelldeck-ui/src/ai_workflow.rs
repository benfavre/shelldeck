use adabraka_ui::components::icon_source::IconSource;
use adabraka_ui::components::input::{Input, InputSize};
use adabraka_ui::components::input_state::InputState;
use adabraka_ui::prelude::{
    scrollable_vertical, Button, ButtonSize, ButtonVariant, Spinner, SpinnerSize, SpinnerVariant,
};
use gpui::prelude::*;
use gpui::*;
use shelldeck_core::ai::{AiBackend, AiCapability, AiDraft, AiSurface};

use crate::icons::{ai_provider_badge, lucide_icon};
use crate::scale::px;
use crate::t;
use crate::theme::ShellDeckColors;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AiWorkflowTarget {
    SupportReply { ticket_id: String },
    SupportSummary { ticket_id: String },
    SupportTriage { ticket_id: String },
    ScriptGenerate { script_id: String },
    ScriptExplain { script_id: String },
    ScriptReview { script_id: String },
    TerminalCommand { session_id: String },
    TerminalDiagnose { session_id: String },
}

impl AiWorkflowTarget {
    pub fn capability(&self) -> AiCapability {
        match self {
            Self::SupportReply { .. } => AiCapability::SupportReply,
            Self::SupportSummary { .. } => AiCapability::SupportSummary,
            Self::SupportTriage { .. } => AiCapability::SupportTriage,
            Self::ScriptGenerate { .. } => AiCapability::ScriptGenerate,
            Self::ScriptExplain { .. } => AiCapability::ScriptExplain,
            Self::ScriptReview { .. } => AiCapability::ScriptReview,
            Self::TerminalCommand { .. } => AiCapability::TerminalCommand,
            Self::TerminalDiagnose { .. } => AiCapability::TerminalDiagnose,
        }
    }

    pub fn target_id(&self) -> &str {
        match self {
            Self::SupportReply { ticket_id }
            | Self::SupportSummary { ticket_id }
            | Self::SupportTriage { ticket_id } => ticket_id,
            Self::ScriptGenerate { script_id }
            | Self::ScriptExplain { script_id }
            | Self::ScriptReview { script_id } => script_id,
            Self::TerminalCommand { session_id } | Self::TerminalDiagnose { session_id } => {
                session_id
            }
        }
    }

    pub fn surface(&self) -> AiSurface {
        match self {
            Self::SupportReply { .. }
            | Self::SupportSummary { .. }
            | Self::SupportTriage { .. } => AiSurface::Support,
            Self::ScriptGenerate { .. }
            | Self::ScriptExplain { .. }
            | Self::ScriptReview { .. } => AiSurface::Script,
            Self::TerminalCommand { .. } | Self::TerminalDiagnose { .. } => AiSurface::Terminal,
        }
    }

    fn requires_instructions(&self) -> bool {
        matches!(
            self,
            Self::ScriptGenerate { .. } | Self::TerminalCommand { .. }
        )
    }

    fn result_is_read_only(&self) -> bool {
        matches!(
            self,
            Self::SupportSummary { .. }
                | Self::SupportTriage { .. }
                | Self::ScriptExplain { .. }
                | Self::ScriptReview { .. }
                | Self::TerminalDiagnose { .. }
        )
    }
}

#[derive(Debug, Clone)]
pub enum AiWorkflowEvent {
    Generate {
        request_id: u64,
        target: AiWorkflowTarget,
        instructions: String,
    },
    Accept {
        target: AiWorkflowTarget,
        result: String,
    },
    Pending {
        target: AiWorkflowTarget,
        instructions: String,
        result: String,
    },
    Cancel,
}

impl EventEmitter<AiWorkflowEvent> for AiWorkflowView {}

pub struct AiWorkflowView {
    target: AiWorkflowTarget,
    instructions_state: Entity<InputState>,
    result_state: Entity<InputState>,
    loading: bool,
    error: Option<String>,
    request_epoch: u64,
    backend: AiBackend,
    model: String,
    restored: bool,
}

impl AiWorkflowView {
    pub fn new(
        target: AiWorkflowTarget,
        backend: AiBackend,
        model: String,
        pending: Option<AiDraft>,
        cx: &mut Context<Self>,
    ) -> Self {
        let pending_instructions = pending
            .as_ref()
            .map(|draft| draft.instructions.clone())
            .unwrap_or_default();
        let pending_result = pending
            .as_ref()
            .map(|draft| draft.result.clone())
            .unwrap_or_default();
        let instructions_state = cx.new(move |cx| {
            let mut state = InputState::new(cx).multi_line(true);
            if !pending_instructions.is_empty() {
                state.replace_content(pending_instructions, cx);
            }
            state
        });
        let result_state = cx.new(move |cx| {
            let mut state = InputState::new(cx).multi_line(true);
            if !pending_result.is_empty() {
                state.replace_content(pending_result, cx);
            }
            state
        });
        Self {
            target,
            instructions_state,
            result_state,
            loading: false,
            error: None,
            request_epoch: 0,
            backend,
            model,
            restored: pending.is_some(),
        }
    }

    pub fn generate(&mut self, cx: &mut Context<Self>) {
        if self.loading {
            return;
        }
        let instructions = self
            .instructions_state
            .read(cx)
            .content()
            .trim()
            .to_string();
        if self.target.requires_instructions() && instructions.is_empty() {
            self.error = Some(t!("ai.workflow.instructions_required").to_string());
            cx.notify();
            return;
        }
        self.request_epoch = self.request_epoch.wrapping_add(1);
        self.loading = true;
        self.error = None;
        self.restored = false;
        cx.emit(AiWorkflowEvent::Generate {
            request_id: self.request_epoch,
            target: self.target.clone(),
            instructions,
        });
        cx.notify();
    }

    pub fn set_result(
        &mut self,
        request_id: u64,
        result: Result<String, String>,
        cx: &mut Context<Self>,
    ) {
        if request_id != self.request_epoch {
            return;
        }
        self.loading = false;
        match result {
            Ok(text) => {
                self.result_state
                    .update(cx, |state, cx| state.replace_content(text, cx));
                self.error = None;
            }
            Err(error) => self.error = Some(error),
        }
        cx.notify();
    }

    fn accept(&mut self, cx: &mut Context<Self>) {
        let result = self.result_state.read(cx).content().trim().to_string();
        if !result.is_empty() {
            cx.emit(AiWorkflowEvent::Accept {
                target: self.target.clone(),
                result,
            });
        }
    }

    fn put_pending(&mut self, cx: &mut Context<Self>) {
        let result = self.result_state.read(cx).content().trim().to_string();
        if result.is_empty() {
            return;
        }
        cx.emit(AiWorkflowEvent::Pending {
            target: self.target.clone(),
            instructions: self
                .instructions_state
                .read(cx)
                .content()
                .trim()
                .to_string(),
            result,
        });
    }
}

impl Render for AiWorkflowView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let model = if self.model.trim().is_empty() {
            self.backend.default_model().to_string()
        } else {
            self.model.clone()
        };
        let instructions_placeholder = match self.target {
            AiWorkflowTarget::SupportReply { .. } => t!("ai.workflow.support_guidance").to_string(),
            AiWorkflowTarget::SupportSummary { .. } => {
                t!("ai.workflow.support_summary_guidance").to_string()
            }
            AiWorkflowTarget::SupportTriage { .. } => {
                t!("ai.workflow.support_triage_guidance").to_string()
            }
            AiWorkflowTarget::ScriptGenerate { .. } => {
                t!("ai.workflow.script_instructions").to_string()
            }
            AiWorkflowTarget::ScriptExplain { .. } => {
                t!("ai.workflow.script_explain_guidance").to_string()
            }
            AiWorkflowTarget::ScriptReview { .. } => {
                t!("ai.workflow.script_review_guidance").to_string()
            }
            AiWorkflowTarget::TerminalCommand { .. } => {
                t!("ai.workflow.terminal_command_instructions").to_string()
            }
            AiWorkflowTarget::TerminalDiagnose { .. } => {
                t!("ai.workflow.terminal_diagnose_guidance").to_string()
            }
        };
        let has_result = !self.result_state.read(cx).content().trim().is_empty();

        let mut body = div()
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
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(ShellDeckColors::text_primary())
                                            .child(t!("ai.identity.label").to_string()),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(11.0))
                                            .text_color(ShellDeckColors::text_muted())
                                            .child(t!("ai.identity.draft_mode").to_string()),
                                    ),
                            ),
                    )
                    .child(ai_provider_badge(self.backend, &model)),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .w_full()
                    .gap(px(6.0))
                    .child(
                        div()
                            .text_size(px(11.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(ShellDeckColors::text_muted())
                            .child(match self.target {
                                AiWorkflowTarget::SupportReply { .. } => {
                                    t!("ai.workflow.guidance_label").to_string()
                                }
                                AiWorkflowTarget::SupportSummary { .. }
                                | AiWorkflowTarget::SupportTriage { .. }
                                | AiWorkflowTarget::ScriptExplain { .. }
                                | AiWorkflowTarget::ScriptReview { .. }
                                | AiWorkflowTarget::TerminalDiagnose { .. } => {
                                    t!("ai.workflow.adjust_label").to_string()
                                }
                                AiWorkflowTarget::ScriptGenerate { .. } => {
                                    t!("ai.workflow.instructions_label").to_string()
                                }
                                AiWorkflowTarget::TerminalCommand { .. } => {
                                    t!("ai.workflow.terminal_command_label").to_string()
                                }
                            }),
                    )
                    .child(
                        div().w_full().min_w(px(0.0)).child(
                            Input::new(&self.instructions_state)
                                .w_full()
                                .size(InputSize::Sm)
                                .multi_line(true)
                                .min_rows(3)
                                .max_rows(6)
                                .placeholder(instructions_placeholder)
                                .disabled(self.loading),
                        ),
                    ),
            );

        if self.restored {
            body = body.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .text_size(px(11.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(lucide_icon("clock", 12.0, ShellDeckColors::text_muted()))
                    .child(t!("ai.workflow.restored").to_string()),
            );
        }

        if self.loading {
            body = body.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .py(px(12.0))
                    .text_size(px(12.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(
                        Spinner::new()
                            .size(SpinnerSize::Xs)
                            .variant(SpinnerVariant::Primary),
                    )
                    .child(t!("ai.assistant.generating").to_string()),
            );
        } else {
            if let Some(error) = &self.error {
                body = body.child(
                    div()
                        .p(px(10.0))
                        .rounded(px(6.0))
                        .bg(ShellDeckColors::error().opacity(0.10))
                        .text_size(px(12.0))
                        .text_color(ShellDeckColors::error())
                        .child(error.clone()),
                );
            }
            if has_result {
                body = body.child(
                    div()
                        .text_size(px(11.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(ShellDeckColors::text_muted())
                        .child(if self.target.result_is_read_only() {
                            t!("ai.workflow.analysis").to_string()
                        } else {
                            t!("ai.assistant.draft").to_string()
                        }),
                );
                if self.target.result_is_read_only() {
                    let result = self.result_state.read(cx).content().to_string();
                    let mut content = div()
                        .flex()
                        .flex_col()
                        .gap(px(4.0))
                        .p(px(12.0))
                        .text_size(px(12.0))
                        .text_color(ShellDeckColors::text_primary());
                    for line in result.split('\n') {
                        let display: SharedString = if line.is_empty() {
                            " ".into()
                        } else {
                            line.to_string().into()
                        };
                        content = content.child(div().max_w(px(680.0)).child(display));
                    }
                    body = body.child(
                        div()
                            .w_full()
                            .h(px(280.0))
                            .min_h(px(0.0))
                            .overflow_hidden()
                            .rounded(px(6.0))
                            .border_1()
                            .border_color(ShellDeckColors::border())
                            .bg(ShellDeckColors::bg_primary())
                            .child(scrollable_vertical(content)),
                    );
                } else {
                    body = body.child(
                        div().w_full().min_w(px(0.0)).child(
                            Input::new(&self.result_state)
                                .w_full()
                                .size(InputSize::Sm)
                                .multi_line(true)
                                .min_rows(9)
                                .max_rows(14),
                        ),
                    );
                }
            }
        }

        let cancel = Button::new("ai-workflow-cancel", t!("scripts.cancel").to_string())
            .variant(ButtonVariant::Ghost)
            .size(ButtonSize::Sm)
            .icon(IconSource::from("x"))
            .on_click(cx.listener(|_, _, _, cx| cx.emit(AiWorkflowEvent::Cancel)));
        let action_group = div()
            .flex()
            .items_center()
            .justify_end()
            .gap(px(8.0))
            .when(has_result, |actions| {
                actions
                    .child(
                        Button::new("ai-workflow-pending", t!("ai.workflow.pending").to_string())
                            .variant(ButtonVariant::Outline)
                            .size(ButtonSize::Sm)
                            .icon(IconSource::from("clock"))
                            .on_click(cx.listener(|this, _, _, cx| this.put_pending(cx))),
                    )
                    .child(
                        Button::new(
                            "ai-workflow-regenerate",
                            t!("ai.workflow.regenerate").to_string(),
                        )
                        .variant(ButtonVariant::Ai)
                        .size(ButtonSize::Sm)
                        .icon(IconSource::from("sparkles"))
                        .on_click(cx.listener(|this, _, _, cx| this.generate(cx))),
                    )
                    .child(
                        Button::new(
                            "ai-workflow-accept",
                            match self.target {
                                AiWorkflowTarget::TerminalCommand { .. } => {
                                    t!("ai.workflow.insert").to_string()
                                }
                                AiWorkflowTarget::SupportSummary { .. }
                                | AiWorkflowTarget::SupportTriage { .. }
                                | AiWorkflowTarget::ScriptExplain { .. }
                                | AiWorkflowTarget::ScriptReview { .. }
                                | AiWorkflowTarget::TerminalDiagnose { .. } => {
                                    t!("ai.workflow.copy").to_string()
                                }
                                _ => t!("ai.workflow.accept").to_string(),
                            },
                        )
                        .variant(ButtonVariant::Default)
                        .size(ButtonSize::Sm)
                        .icon(IconSource::from("check"))
                        .on_click(cx.listener(|this, _, _, cx| this.accept(cx))),
                    )
            })
            .when(!has_result, |actions| {
                actions.child(
                    Button::new(
                        "ai-workflow-generate",
                        t!("ai.assistant.submit").to_string(),
                    )
                    .variant(ButtonVariant::Ai)
                    .size(ButtonSize::Sm)
                    .disabled(self.loading)
                    .icon(IconSource::from("sparkles"))
                    .on_click(cx.listener(|this, _, _, cx| this.generate(cx))),
                )
            });

        div()
            .flex()
            .flex_col()
            .w_full()
            .flex_1()
            .min_h(px(0.0))
            .overflow_hidden()
            .child(scrollable_vertical(body))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(px(12.0))
                    .px(px(16.0))
                    .py(px(12.0))
                    .border_t_1()
                    .border_color(ShellDeckColors::primary().opacity(0.22))
                    .bg(ShellDeckColors::primary().opacity(0.035))
                    .child(cancel)
                    .child(action_group),
            )
    }
}
