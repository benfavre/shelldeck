use adabraka_ui::components::icon_source::IconSource;
use adabraka_ui::components::input::{Input, InputSize};
use adabraka_ui::components::input_state::InputState;
use adabraka_ui::prelude::{
    scrollable_vertical, Badge, BadgeVariant, Button, ButtonSize, ButtonVariant, Spinner,
    SpinnerSize, SpinnerVariant,
};
use gpui::prelude::*;
use gpui::*;
use shelldeck_core::ai::{
    ai_line_diff, parse_issue_triage_proposal, AiBackend, AiCapability, AiDiffLine, AiDraft,
    AiIssueTriageProposal, AiSurface,
};

use crate::icons::{ai_provider_badge, lucide_icon};
use crate::scale::px;
use crate::support_view::{assignee_display, priority_badge};
use crate::t;
use crate::theme::ShellDeckColors;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AiWorkflowTarget {
    SupportReply { ticket_id: String },
    SupportSummary { ticket_id: String },
    SupportTriage { ticket_id: String },
    IssueReply { issue_id: String },
    IssueSummary { issue_id: String },
    IssueTriage { issue_id: String },
    ScriptGenerate { script_id: String },
    ScriptExplain { script_id: String },
    ScriptReview { script_id: String },
    ScriptFix { script_id: String },
    TerminalCommand { session_id: String },
    TerminalDiagnose { session_id: String },
}

impl AiWorkflowTarget {
    pub fn capability(&self) -> AiCapability {
        match self {
            Self::SupportReply { .. } => AiCapability::SupportReply,
            Self::SupportSummary { .. } => AiCapability::SupportSummary,
            Self::SupportTriage { .. } => AiCapability::SupportTriage,
            Self::IssueReply { .. } => AiCapability::IssueReply,
            Self::IssueSummary { .. } => AiCapability::IssueSummary,
            Self::IssueTriage { .. } => AiCapability::IssueTriage,
            Self::ScriptGenerate { .. } => AiCapability::ScriptGenerate,
            Self::ScriptExplain { .. } => AiCapability::ScriptExplain,
            Self::ScriptReview { .. } => AiCapability::ScriptReview,
            Self::ScriptFix { .. } => AiCapability::ScriptFix,
            Self::TerminalCommand { .. } => AiCapability::TerminalCommand,
            Self::TerminalDiagnose { .. } => AiCapability::TerminalDiagnose,
        }
    }

    pub fn target_id(&self) -> &str {
        match self {
            Self::SupportReply { ticket_id }
            | Self::SupportSummary { ticket_id }
            | Self::SupportTriage { ticket_id } => ticket_id,
            Self::IssueReply { issue_id }
            | Self::IssueSummary { issue_id }
            | Self::IssueTriage { issue_id } => issue_id,
            Self::ScriptGenerate { script_id }
            | Self::ScriptExplain { script_id }
            | Self::ScriptReview { script_id }
            | Self::ScriptFix { script_id } => script_id,
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
            Self::IssueReply { .. } | Self::IssueSummary { .. } | Self::IssueTriage { .. } => {
                AiSurface::Issue
            }
            Self::ScriptGenerate { .. }
            | Self::ScriptExplain { .. }
            | Self::ScriptReview { .. }
            | Self::ScriptFix { .. } => AiSurface::Script,
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
                | Self::IssueSummary { .. }
                | Self::IssueTriage { .. }
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

fn render_issue_triage_proposal(
    proposal: &AiIssueTriageProposal,
    current: Option<&(String, String)>,
) -> impl IntoElement {
    let mut changes = div().flex().items_center().gap(px(8.0)).flex_wrap();
    if let Some(priority) = &proposal.priority {
        changes = changes.child(
            div()
                .flex()
                .items_center()
                .gap(px(6.0))
                .child(
                    div()
                        .text_size(px(11.0))
                        .text_color(ShellDeckColors::text_muted())
                        .child(t!("ai.workflow.triage.priority").to_string()),
                )
                .when_some(current, |row, (current_priority, _)| {
                    row.child(priority_badge(current_priority))
                        .child(lucide_icon(
                            "arrow-right",
                            12.0,
                            ShellDeckColors::text_muted(),
                        ))
                })
                .child(priority_badge(priority)),
        );
    }
    if let Some(assignee) = &proposal.assignee {
        let label = if assignee.is_empty() {
            t!("support.assignee.none").to_string()
        } else {
            assignee.clone()
        };
        changes = changes.child(
            div()
                .flex()
                .items_center()
                .gap(px(6.0))
                .child(
                    div()
                        .text_size(px(11.0))
                        .text_color(ShellDeckColors::text_muted())
                        .child(t!("ai.workflow.triage.assignee").to_string()),
                )
                .when_some(current, |row, (_, current_assignee)| {
                    row.child(
                        Badge::new(assignee_display(current_assignee, None))
                            .variant(BadgeVariant::Outline),
                    )
                    .child(lucide_icon(
                        "arrow-right",
                        12.0,
                        ShellDeckColors::text_muted(),
                    ))
                })
                .child(Badge::new(label).variant(BadgeVariant::Outline)),
        );
    }
    if !proposal.has_changes() {
        changes = changes.child(
            div()
                .text_size(px(12.0))
                .text_color(ShellDeckColors::text_muted())
                .child(t!("ai.workflow.triage.no_changes").to_string()),
        );
    }

    let mut actions = div().flex().flex_col().gap(px(5.0));
    for action in &proposal.next_actions {
        actions = actions.child(
            div()
                .flex()
                .items_start()
                .gap(px(7.0))
                .child(lucide_icon(
                    "arrow-right",
                    12.0,
                    ShellDeckColors::text_muted(),
                ))
                .child(
                    div()
                        .min_w(px(0.0))
                        .text_size(px(12.0))
                        .text_color(ShellDeckColors::text_primary())
                        .child(action.clone()),
                ),
        );
    }

    div()
        .flex()
        .flex_col()
        .gap(px(12.0))
        .p(px(12.0))
        .rounded(px(6.0))
        .border_1()
        .border_color(ShellDeckColors::border())
        .bg(ShellDeckColors::bg_primary())
        .child(changes)
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(4.0))
                .child(
                    div()
                        .text_size(px(11.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(ShellDeckColors::text_muted())
                        .child(t!("ai.workflow.triage.rationale").to_string()),
                )
                .child(
                    div()
                        .text_size(px(12.0))
                        .text_color(ShellDeckColors::text_primary())
                        .child(proposal.rationale.clone()),
                ),
        )
        .when(!proposal.next_actions.is_empty(), |content| {
            content.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(5.0))
                    .child(
                        div()
                            .text_size(px(11.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(ShellDeckColors::text_muted())
                            .child(t!("ai.workflow.triage.next_actions").to_string()),
                    )
                    .child(actions),
            )
        })
}

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
    comparison_original: Option<String>,
    issue_triage_current: Option<(String, String)>,
}

impl AiWorkflowView {
    pub fn new(
        target: AiWorkflowTarget,
        backend: AiBackend,
        model: String,
        pending: Option<AiDraft>,
        comparison_original: Option<String>,
        issue_triage_current: Option<(String, String)>,
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
        let instructions_multiline = !target.result_is_read_only();
        let instructions_state = cx.new(move |cx| {
            let mut state = InputState::new(cx).multi_line(instructions_multiline);
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
            comparison_original,
            issue_triage_current,
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
            AiWorkflowTarget::IssueReply { .. } => {
                t!("ai.workflow.issue_reply_guidance").to_string()
            }
            AiWorkflowTarget::IssueSummary { .. } => {
                t!("ai.workflow.issue_summary_guidance").to_string()
            }
            AiWorkflowTarget::IssueTriage { .. } => {
                t!("ai.workflow.issue_triage_guidance").to_string()
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
            AiWorkflowTarget::ScriptFix { .. } => t!("ai.workflow.script_fix_guidance").to_string(),
            AiWorkflowTarget::TerminalCommand { .. } => {
                t!("ai.workflow.terminal_command_instructions").to_string()
            }
            AiWorkflowTarget::TerminalDiagnose { .. } => {
                t!("ai.workflow.terminal_diagnose_guidance").to_string()
            }
        };
        let has_result = !self.result_state.read(cx).content().trim().is_empty();
        let issue_triage_proposal =
            if has_result && matches!(&self.target, AiWorkflowTarget::IssueTriage { .. }) {
                parse_issue_triage_proposal(self.result_state.read(cx).content()).ok()
            } else {
                None
            };
        let triage_accept_disabled = matches!(&self.target, AiWorkflowTarget::IssueTriage { .. })
            && issue_triage_proposal
                .as_ref()
                .is_none_or(|proposal| !proposal.has_changes());

        let instructions_input = if self.target.result_is_read_only() {
            let entity = cx.entity();
            div()
                .flex()
                .items_center()
                .w_full()
                .min_w(px(0.0))
                .gap(px(8.0))
                .child(
                    div()
                        .w_full()
                        .min_w(px(0.0))
                        .flex_initial()
                        .h(px(32.0))
                        .overflow_hidden()
                        .child(
                            Input::new(&self.instructions_state)
                                .w_full()
                                .size(InputSize::Sm)
                                .placeholder(instructions_placeholder)
                                .disabled(self.loading)
                                .on_enter(move |_, cx| {
                                    entity.update(cx, |this, cx| this.generate(cx));
                                }),
                        ),
                )
                .child(
                    Button::new("ai-workflow-adjust", t!("ai.workflow.adjust").to_string())
                        .variant(ButtonVariant::Ai)
                        .size(ButtonSize::Sm)
                        .min_w(px(96.0))
                        .flex_shrink_0()
                        .icon(IconSource::from("sparkles"))
                        .disabled(self.loading)
                        .on_click(cx.listener(|this, _, _, cx| this.generate(cx))),
                )
                .into_any_element()
        } else {
            div()
                .w_full()
                .min_w(px(0.0))
                .child(
                    Input::new(&self.instructions_state)
                        .w_full()
                        .size(InputSize::Sm)
                        .multi_line(true)
                        .min_rows(3)
                        .max_rows(6)
                        .placeholder(instructions_placeholder)
                        .disabled(self.loading),
                )
                .into_any_element()
        };

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
                                AiWorkflowTarget::SupportReply { .. }
                                | AiWorkflowTarget::IssueReply { .. } => {
                                    t!("ai.workflow.guidance_label").to_string()
                                }
                                AiWorkflowTarget::SupportSummary { .. }
                                | AiWorkflowTarget::SupportTriage { .. }
                                | AiWorkflowTarget::IssueSummary { .. }
                                | AiWorkflowTarget::IssueTriage { .. }
                                | AiWorkflowTarget::ScriptExplain { .. }
                                | AiWorkflowTarget::ScriptReview { .. }
                                | AiWorkflowTarget::TerminalDiagnose { .. } => {
                                    t!("ai.workflow.adjust_label").to_string()
                                }
                                AiWorkflowTarget::ScriptGenerate { .. }
                                | AiWorkflowTarget::ScriptFix { .. } => {
                                    t!("ai.workflow.instructions_label").to_string()
                                }
                                AiWorkflowTarget::TerminalCommand { .. } => {
                                    t!("ai.workflow.terminal_command_label").to_string()
                                }
                            }),
                    )
                    .child(instructions_input),
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
                    if let Some(proposal) = issue_triage_proposal.as_ref() {
                        body = body.child(render_issue_triage_proposal(
                            proposal,
                            self.issue_triage_current.as_ref(),
                        ));
                    } else {
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
                    }
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
                if let Some(original) = self.comparison_original.as_deref() {
                    let proposed = self.result_state.read(cx).content().to_string();
                    let mut diff = div()
                        .flex()
                        .flex_col()
                        .font_family("monospace")
                        .text_size(px(11.0));
                    for line in ai_line_diff(original, &proposed) {
                        let (prefix, color, bg, text) = match line {
                            AiDiffLine::Context(text) => (
                                " ",
                                ShellDeckColors::text_muted(),
                                gpui::transparent_black(),
                                text,
                            ),
                            AiDiffLine::Removed(text) => (
                                "-",
                                ShellDeckColors::error(),
                                ShellDeckColors::error().opacity(0.08),
                                text,
                            ),
                            AiDiffLine::Added(text) => (
                                "+",
                                ShellDeckColors::success(),
                                ShellDeckColors::success().opacity(0.08),
                                text,
                            ),
                        };
                        diff = diff.child(
                            div()
                                .flex()
                                .min_w(px(0.0))
                                .gap(px(8.0))
                                .px(px(8.0))
                                .py(px(2.0))
                                .bg(bg)
                                .text_color(color)
                                .child(div().flex_shrink_0().w(px(10.0)).child(prefix))
                                .child(div().min_w(px(0.0)).max_w(px(640.0)).child(
                                    if text.is_empty() {
                                        " ".to_string()
                                    } else {
                                        text
                                    },
                                )),
                        );
                    }
                    body = body
                        .child(
                            div()
                                .text_size(px(11.0))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(ShellDeckColors::text_muted())
                                .child(t!("ai.workflow.diff_preview").to_string()),
                        )
                        .child(
                            div()
                                .w_full()
                                .h(px(220.0))
                                .min_h(px(0.0))
                                .overflow_hidden()
                                .rounded(px(6.0))
                                .border_1()
                                .border_color(ShellDeckColors::border())
                                .bg(ShellDeckColors::bg_primary())
                                .child(scrollable_vertical(diff)),
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
                                AiWorkflowTarget::IssueTriage { .. } => {
                                    t!("ai.workflow.triage.apply").to_string()
                                }
                                AiWorkflowTarget::SupportSummary { .. }
                                | AiWorkflowTarget::SupportTriage { .. }
                                | AiWorkflowTarget::IssueSummary { .. }
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
                        .disabled(triage_accept_disabled)
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
