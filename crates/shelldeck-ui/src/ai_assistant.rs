use adabraka_ui::components::confirm_dialog::Dialog as UiDialog;
use adabraka_ui::components::empty_state::{EmptyState, EmptyStateSize};
use adabraka_ui::components::icon_source::IconSource;
use adabraka_ui::components::input::{Input, InputSize};
use adabraka_ui::components::input_state::InputState;
use adabraka_ui::prelude::{
    Button, ButtonSize, ButtonVariant, Spinner, SpinnerSize, SpinnerVariant,
};
use gpui::prelude::*;
use gpui::*;
use shelldeck_core::ai::{
    AiBackend, AiChatRole, AiContext, AiConversation, AiConversationStore, AiSurface,
};
use uuid::Uuid;

use crate::icons::{ai_provider_badge, lucide_icon};
use crate::scale::px;
use crate::t;
use crate::theme::ShellDeckColors;

#[derive(Debug, Clone)]
pub enum AiAssistantEvent {
    Submit {
        request_id: u64,
        conversation_id: Uuid,
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
    error: Option<String>,
    request_gate: AiRequestGate,
    backend: AiBackend,
    model: String,
    conversations: Vec<AiConversation>,
    active_conversation: Option<Uuid>,
    message_scroll: ScrollHandle,
    history_scroll: ScrollHandle,
    history_open: bool,
    show_archived: bool,
    pending_delete: Option<Uuid>,
}

impl AiAssistantView {
    pub fn new(context: AiContext, cx: &mut Context<Self>) -> Self {
        let conversations = AiConversationStore::load().unwrap_or_else(|error| {
            tracing::warn!("Failed to load AI conversations: {error}");
            Vec::new()
        });
        Self {
            prompt_state: cx.new(|cx| InputState::new(cx).multi_line(true)),
            context,
            loading: false,
            error: None,
            request_gate: AiRequestGate::default(),
            backend: AiBackend::Disabled,
            model: String::new(),
            active_conversation: conversations
                .iter()
                .rev()
                .find(|conversation| !conversation.archived)
                .map(|conversation| conversation.id),
            conversations,
            message_scroll: ScrollHandle::new(),
            history_scroll: ScrollHandle::new(),
            history_open: true,
            show_archived: false,
            pending_delete: None,
        }
    }

    pub fn set_backend(&mut self, backend: AiBackend, model: String, cx: &mut Context<Self>) {
        self.backend = backend;
        self.model = model;
        cx.notify();
    }

    pub fn set_context(&mut self, context: AiContext, cx: &mut Context<Self>) {
        self.request_gate.invalidate();
        let context_changed =
            self.context.surface != context.surface || self.context.title != context.title;
        self.context = context;
        self.loading = false;
        self.error = None;
        if context_changed
            && self
                .active_conversation()
                .is_some_and(|conversation| !conversation.messages.is_empty())
        {
            self.active_conversation = None;
        }
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
        conversation_id: Uuid,
        result: Result<String, String>,
        cx: &mut Context<Self>,
    ) {
        if !self.request_gate.accepts(request_id) {
            return;
        }
        self.loading = false;
        match result {
            Ok(text) => {
                if let Some(conversation) = self
                    .conversations
                    .iter_mut()
                    .find(|conversation| conversation.id == conversation_id)
                {
                    conversation.push(AiChatRole::Assistant, text.clone());
                    self.persist_conversations();
                    self.message_scroll.scroll_to_bottom();
                }
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
            let conversation_id = self.ensure_active_conversation();
            if let Some(conversation) = self
                .conversations
                .iter_mut()
                .find(|conversation| conversation.id == conversation_id)
            {
                conversation.push(AiChatRole::User, prompt);
            }
            self.persist_conversations();
            let prompt = self.conversation_prompt(conversation_id);
            self.prompt_state.update(cx, |state, cx| state.reset(cx));
            let request_id = self.request_gate.begin();
            self.loading = true;
            self.error = None;
            cx.emit(AiAssistantEvent::Submit {
                request_id,
                conversation_id,
                prompt,
                context: self.context.clone(),
            });
            cx.notify();
        }
    }

    fn active_conversation(&self) -> Option<&AiConversation> {
        let id = self.active_conversation?;
        self.conversations
            .iter()
            .find(|conversation| conversation.id == id)
    }

    fn ensure_active_conversation(&mut self) -> Uuid {
        if let Some(id) = self.active_conversation {
            if self
                .conversations
                .iter()
                .any(|conversation| conversation.id == id && !conversation.archived)
            {
                return id;
            }
        }
        let conversation = AiConversation::new(self.context.surface, self.context.title.clone());
        let id = conversation.id;
        self.conversations.push(conversation);
        self.active_conversation = Some(id);
        id
    }

    fn conversation_prompt(&self, conversation_id: Uuid) -> String {
        let Some(conversation) = self
            .conversations
            .iter()
            .find(|conversation| conversation.id == conversation_id)
        else {
            return String::new();
        };
        conversation
            .messages
            .iter()
            .rev()
            .take(12)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .map(|message| {
                let role = match message.role {
                    AiChatRole::User => "User",
                    AiChatRole::Assistant => "Assistant",
                };
                format!("{role}: {}", message.content)
            })
            .collect::<Vec<_>>()
            .join("\n\n")
    }

    fn persist_conversations(&self) {
        if let Err(error) = AiConversationStore::save(&self.conversations) {
            tracing::warn!("Failed to save AI conversations: {error}");
        }
    }

    fn new_conversation(&mut self, cx: &mut Context<Self>) {
        self.request_gate.invalidate();
        self.active_conversation = None;
        self.loading = false;
        self.error = None;
        self.prompt_state.update(cx, |state, cx| state.reset(cx));
        cx.notify();
    }

    fn select_conversation(&mut self, id: Uuid, cx: &mut Context<Self>) {
        if let Some(conversation) = self
            .conversations
            .iter_mut()
            .find(|conversation| conversation.id == id)
        {
            conversation.archived = false;
            self.active_conversation = Some(id);
            self.persist_conversations();
            self.message_scroll.scroll_to_bottom();
            cx.notify();
        }
    }

    fn toggle_archive(&mut self, id: Uuid, cx: &mut Context<Self>) {
        if let Some(conversation) = self
            .conversations
            .iter_mut()
            .find(|conversation| conversation.id == id)
        {
            conversation.archived = !conversation.archived;
            conversation.updated_at = chrono::Utc::now();
            if conversation.archived && self.active_conversation == Some(id) {
                self.active_conversation = None;
            }
            self.persist_conversations();
            cx.notify();
        }
    }

    fn delete_conversation(&mut self, id: Uuid, cx: &mut Context<Self>) {
        self.conversations
            .retain(|conversation| conversation.id != id);
        if self.active_conversation == Some(id) {
            self.active_conversation = None;
        }
        self.pending_delete = None;
        self.persist_conversations();
        cx.notify();
    }

    fn quick_actions(surface: AiSurface) -> Vec<(String, String, &'static str)> {
        let keys: &[(&str, &str, &str)] = match surface {
            AiSurface::Support => &[
                ("ai.quick.support_reply", "ai.prompt.support_reply", "reply"),
                (
                    "ai.quick.summarize",
                    "ai.prompt.support_summary",
                    "scroll-text",
                ),
                ("ai.quick.triage", "ai.prompt.support_triage", "flag"),
            ],
            AiSurface::Issue => &[
                ("ai.quick.issue_draft", "ai.prompt.issue_draft", "plus"),
                ("ai.quick.tags", "ai.prompt.issue_tags", "tag"),
                ("ai.quick.priority", "ai.prompt.issue_priority", "flag"),
            ],
            AiSurface::Script => &[
                ("ai.quick.generate", "ai.prompt.script_generate", "sparkles"),
                ("ai.quick.explain", "ai.prompt.script_explain", "info"),
                (
                    "ai.quick.convert",
                    "ai.prompt.script_convert",
                    "arrow-left-right",
                ),
                ("ai.quick.review", "ai.prompt.script_review", "shield-check"),
                ("ai.quick.naming", "ai.prompt.naming", "pencil"),
            ],
            AiSurface::Terminal => &[
                ("ai.quick.command", "ai.prompt.terminal_command", "terminal"),
                ("ai.quick.error", "ai.prompt.terminal_error", "circle-alert"),
                ("ai.quick.issue_draft", "ai.prompt.terminal_issue", "plus"),
            ],
            AiSurface::Jean => &[("ai.quick.jean", "ai.prompt.jean", "send")],
            AiSurface::Naming => &[("ai.quick.naming", "ai.prompt.naming", "pencil")],
            AiSurface::Recent => &[("ai.quick.summarize", "ai.prompt.recent", "activity")],
            AiSurface::Global => &[],
        };
        keys.iter()
            .map(|(label, prompt, icon)| (t!(*label).to_string(), t!(*prompt).to_string(), *icon))
            .collect()
    }

    fn render_history(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut list = div().flex().flex_col().gap(px(3.0)).p(px(8.0));
        let mut conversations = self
            .conversations
            .iter()
            .filter(|conversation| conversation.archived == self.show_archived)
            .cloned()
            .collect::<Vec<_>>();
        conversations.sort_by_key(|conversation| std::cmp::Reverse(conversation.updated_at));

        if conversations.is_empty() {
            list = list.child(
                div()
                    .px(px(8.0))
                    .py(px(24.0))
                    .text_size(px(12.0))
                    .text_align(TextAlign::Center)
                    .text_color(ShellDeckColors::text_muted())
                    .child(t!("ai.history.empty").to_string()),
            );
        }

        for conversation in conversations {
            let id = conversation.id;
            let archive_id = id;
            let delete_id = id;
            let selected = self.active_conversation == Some(id);
            let updated = crate::i18n::rel_time(conversation.updated_at.timestamp_millis() as f64);
            list = list.child(
                div()
                    .id(SharedString::from(format!("ai-conversation-{id}")))
                    .flex()
                    .items_center()
                    .gap(px(4.0))
                    .min_w_0()
                    .px(px(8.0))
                    .py(px(7.0))
                    .rounded(px(6.0))
                    .when(selected, |row| row.bg(ShellDeckColors::selected_bg()))
                    .hover(|style| style.bg(ShellDeckColors::hover_bg()))
                    .child(
                        div()
                            .id(SharedString::from(format!("ai-conversation-open-{id}")))
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .cursor_pointer()
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.select_conversation(id, cx);
                            }))
                            .child(
                                div()
                                    .truncate()
                                    .text_size(px(12.0))
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(ShellDeckColors::text_primary())
                                    .child(conversation.title),
                            )
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(4.0))
                                    .min_w_0()
                                    .truncate()
                                    .text_size(px(10.0))
                                    .text_color(ShellDeckColors::text_muted())
                                    .child(conversation.context_title)
                                    .child("·")
                                    .child(updated),
                            ),
                    )
                    .child(
                        Button::new(SharedString::from(format!("ai-archive-{archive_id}")), "")
                            .variant(ButtonVariant::Ghost)
                            .size(ButtonSize::Sm)
                            .flex_shrink_0()
                            .tooltip(
                                if conversation.archived {
                                    t!("ai.history.restore")
                                } else {
                                    t!("ai.history.archive")
                                }
                                .to_string(),
                            )
                            .icon(IconSource::from(if conversation.archived {
                                "archive-restore"
                            } else {
                                "archive"
                            }))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.toggle_archive(archive_id, cx);
                            })),
                    )
                    .child(
                        Button::new(SharedString::from(format!("ai-delete-{delete_id}")), "")
                            .variant(ButtonVariant::Ghost)
                            .size(ButtonSize::Sm)
                            .flex_shrink_0()
                            .tooltip(t!("ai.history.delete").to_string())
                            .icon(IconSource::from("trash-2"))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.pending_delete = Some(delete_id);
                                cx.notify();
                            })),
                    ),
            );
        }

        let header = div()
            .flex()
            .items_center()
            .justify_between()
            .gap(px(8.0))
            .h(px(48.0))
            .px(px(12.0))
            .border_b_1()
            .border_color(ShellDeckColors::border())
            .child(
                div()
                    .text_size(px(13.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(ShellDeckColors::text_primary())
                    .child(t!("ai.history.title").to_string()),
            )
            .child(
                Button::new("ai-new-conversation", "")
                    .variant(ButtonVariant::Ghost)
                    .size(ButtonSize::Sm)
                    .tooltip(t!("ai.history.new").to_string())
                    .icon(IconSource::from("plus"))
                    .on_click(cx.listener(|this, _, _, cx| this.new_conversation(cx))),
            );

        let filters = div()
            .flex()
            .items_center()
            .gap(px(6.0))
            .px(px(10.0))
            .py(px(8.0))
            .border_b_1()
            .border_color(ShellDeckColors::border())
            .child(
                Button::new("ai-history-active", t!("ai.history.active").to_string())
                    .variant(if self.show_archived {
                        ButtonVariant::Ghost
                    } else {
                        ButtonVariant::Secondary
                    })
                    .size(ButtonSize::Sm)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.show_archived = false;
                        cx.notify();
                    })),
            )
            .child(
                Button::new("ai-history-archived", t!("ai.history.archived").to_string())
                    .variant(if self.show_archived {
                        ButtonVariant::Secondary
                    } else {
                        ButtonVariant::Ghost
                    })
                    .size(ButtonSize::Sm)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.show_archived = true;
                        cx.notify();
                    })),
            );

        div()
            .flex()
            .flex_col()
            .w(px(250.0))
            .h_full()
            .flex_shrink_0()
            .min_h(px(0.0))
            .bg(ShellDeckColors::bg_sidebar())
            .border_r_1()
            .border_color(ShellDeckColors::border())
            .child(header)
            .child(filters)
            .child(
                div()
                    .id("ai-history-scroll")
                    .flex_1()
                    .min_h(px(0.0))
                    .overflow_y_scroll()
                    .track_scroll(&self.history_scroll)
                    .child(list),
            )
            .into_any_element()
    }

    fn render_messages(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut thread = div()
            .flex()
            .flex_col()
            .gap(px(14.0))
            .px(px(20.0))
            .py(px(18.0));
        if let Some(conversation) = self.active_conversation() {
            for message in &conversation.messages {
                let is_user = message.role == AiChatRole::User;
                let mut row = div().flex().w_full();
                if is_user {
                    row = row.justify_end();
                }
                let lines = message
                    .content
                    .split('\n')
                    .map(|line| {
                        div()
                            .min_h(px(18.0))
                            .child(if line.is_empty() { " " } else { line }.to_string())
                    })
                    .collect::<Vec<_>>();
                let message_id = message.id;
                let content = message.content.clone();
                thread =
                    thread.child(
                        row.child(
                            div()
                                .max_w(px(480.0))
                                .min_w(px(0.0))
                                .overflow_hidden()
                                .px(px(11.0))
                                .py(px(9.0))
                                .rounded(px(8.0))
                                .bg(if is_user {
                                    ShellDeckColors::primary().opacity(0.12)
                                } else {
                                    ShellDeckColors::bg_surface()
                                })
                                .border_1()
                                .border_color(ShellDeckColors::border())
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .justify_between()
                                        .gap(px(10.0))
                                        .mb(px(5.0))
                                        .child(
                                            div()
                                                .flex()
                                                .items_center()
                                                .gap(px(6.0))
                                                .text_size(px(10.0))
                                                .font_weight(FontWeight::SEMIBOLD)
                                                .text_color(ShellDeckColors::text_muted())
                                                .when(!is_user, |label| {
                                                    label.child(lucide_icon(
                                                        "sparkles",
                                                        11.0,
                                                        ShellDeckColors::primary(),
                                                    ))
                                                })
                                                .child(if is_user {
                                                    t!("ai.assistant.you").to_string()
                                                } else {
                                                    t!("ai.assistant.name").to_string()
                                                }),
                                        )
                                        .when(!is_user, |header| {
                                            header.child(
                                                Button::new(
                                                    SharedString::from(format!(
                                                        "ai-copy-message-{message_id}"
                                                    )),
                                                    "",
                                                )
                                                .variant(ButtonVariant::Ghost)
                                                .size(ButtonSize::Sm)
                                                .tooltip(t!("ai.assistant.copy").to_string())
                                                .icon(IconSource::from("copy"))
                                                .on_click(move |_, _, cx| {
                                                    cx.write_to_clipboard(
                                                        ClipboardItem::new_string(content.clone()),
                                                    );
                                                }),
                                            )
                                        }),
                                )
                                .child(
                                    div()
                                        .flex()
                                        .flex_col()
                                        .gap(px(1.0))
                                        .text_size(px(12.0))
                                        .text_color(ShellDeckColors::text_primary())
                                        .children(lines),
                                ),
                        ),
                    );
            }
        }
        if self.loading {
            thread = thread.child(
                div().flex().child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.0))
                        .px(px(11.0))
                        .py(px(9.0))
                        .rounded(px(8.0))
                        .border_1()
                        .border_color(ShellDeckColors::border())
                        .bg(ShellDeckColors::bg_surface())
                        .text_size(px(12.0))
                        .text_color(ShellDeckColors::text_muted())
                        .child(
                            Spinner::new()
                                .size(SpinnerSize::Xs)
                                .variant(SpinnerVariant::Primary),
                        )
                        .child(t!("ai.assistant.generating").to_string()),
                ),
            );
        }
        let _ = cx;
        thread.into_any_element()
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
            .min_rows(3)
            .max_rows(6)
            .placeholder(t!("ai.assistant.placeholder").to_string())
            .disabled(self.loading)
            .on_enter(submit);

        let mut quick_actions = div().flex().flex_wrap().justify_center().gap(px(8.0));
        for (index, (label, quick_prompt, icon)) in Self::quick_actions(self.context.surface)
            .into_iter()
            .enumerate()
        {
            quick_actions = quick_actions.child(
                Button::new(("ai-quick", index), label)
                    .variant(ButtonVariant::Outline)
                    .size(ButtonSize::Sm)
                    .icon(IconSource::from(icon))
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

        let active_title = self
            .active_conversation()
            .map(|conversation| conversation.title.clone())
            .unwrap_or_else(|| t!("ai.history.new").to_string());
        let conversation_header = div()
            .flex()
            .items_center()
            .justify_between()
            .gap(px(12.0))
            .h(px(58.0))
            .px(px(14.0))
            .border_b_1()
            .border_color(ShellDeckColors::border())
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .flex_1()
                    .min_w_0()
                    .child(
                        Button::new("ai-toggle-history", "")
                            .variant(ButtonVariant::Ghost)
                            .size(ButtonSize::Sm)
                            .tooltip(if self.history_open {
                                t!("ai.history.hide").to_string()
                            } else {
                                t!("ai.history.show").to_string()
                            })
                            .icon(IconSource::from(if self.history_open {
                                "chevron-left"
                            } else {
                                "clock"
                            }))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.history_open = !this.history_open;
                                cx.notify();
                            })),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w_0()
                            .gap(px(1.0))
                            .child(
                                div()
                                    .truncate()
                                    .text_size(px(13.0))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(ShellDeckColors::text_primary())
                                    .child(active_title),
                            )
                            .child(
                                div()
                                    .truncate()
                                    .text_size(px(10.0))
                                    .text_color(ShellDeckColors::text_muted())
                                    .child(self.context.title.clone()),
                            ),
                    ),
            )
            .child(ai_provider_badge(self.backend, &model));

        let has_messages = self
            .active_conversation()
            .is_some_and(|conversation| !conversation.messages.is_empty());
        let conversation_body = if has_messages {
            self.render_messages(cx)
        } else {
            div()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap(px(14.0))
                .min_h(px(360.0))
                .px(px(28.0))
                .child(
                    EmptyState::new(
                        "ai-empty-chat",
                        t!("ai.assistant.welcome_title").to_string(),
                    )
                    .icon(IconSource::from("sparkles"))
                    .description(t!("ai.assistant.welcome_description").to_string())
                    .size(EmptyStateSize::Sm),
                )
                .child(quick_actions)
                .into_any_element()
        };

        let mut composer = div()
            .flex()
            .flex_col()
            .gap(px(8.0))
            .px(px(14.0))
            .py(px(12.0))
            .border_t_1()
            .border_color(ShellDeckColors::border())
            .bg(ShellDeckColors::bg_surface());
        if let Some(error) = &self.error {
            composer = composer.child(
                div()
                    .px(px(10.0))
                    .py(px(8.0))
                    .rounded(px(6.0))
                    .bg(ShellDeckColors::error().opacity(0.10))
                    .text_size(px(12.0))
                    .text_color(ShellDeckColors::error())
                    .child(error.clone()),
            );
        }
        composer = composer.child(
            div()
                .flex()
                .items_end()
                .gap(px(8.0))
                .min_w_0()
                .child(div().flex_1().min_w_0().child(prompt))
                .child(
                    Button::new("ai-submit", t!("ai.assistant.send").to_string())
                        .variant(ButtonVariant::Ai)
                        .size(ButtonSize::Sm)
                        .icon(IconSource::from("send"))
                        .disabled(self.loading)
                        .on_click(cx.listener(|this, _, _window, cx| this.submit(cx))),
                ),
        );

        let chat = div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .min_h(px(0.0))
            .child(conversation_header)
            .child(
                div()
                    .id("ai-message-scroll")
                    .flex_1()
                    .min_h(px(0.0))
                    .overflow_y_scroll()
                    .track_scroll(&self.message_scroll)
                    .child(conversation_body),
            )
            .child(composer);

        let mut root = div()
            .flex()
            .w_full()
            .flex_1()
            .h_full()
            .min_h(px(0.0))
            .min_w_0()
            .overflow_hidden();
        if self.history_open {
            root = root.child(self.render_history(cx));
        }
        root = root.child(chat);

        if let Some(delete_id) = self.pending_delete {
            root = root.child(
                UiDialog::new()
                    .width(gpui::px(420.0))
                    .header(
                        div()
                            .px(px(16.0))
                            .py(px(14.0))
                            .text_size(px(14.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(t!("ai.history.delete_title").to_string()),
                    )
                    .content(
                        div()
                            .px(px(16.0))
                            .pb(px(16.0))
                            .text_size(px(12.0))
                            .text_color(ShellDeckColors::text_muted())
                            .child(t!("ai.history.delete_description").to_string()),
                    )
                    .footer(
                        div()
                            .flex()
                            .justify_end()
                            .gap(px(8.0))
                            .p(px(12.0))
                            .border_t_1()
                            .border_color(ShellDeckColors::border())
                            .child(
                                Button::new("ai-delete-cancel", t!("scripts.cancel").to_string())
                                    .variant(ButtonVariant::Ghost)
                                    .size(ButtonSize::Sm)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.pending_delete = None;
                                        cx.notify();
                                    })),
                            )
                            .child(
                                Button::new(
                                    "ai-delete-confirm",
                                    t!("ai.history.delete").to_string(),
                                )
                                .variant(ButtonVariant::Destructive)
                                .size(ButtonSize::Sm)
                                .icon(IconSource::from("trash-2"))
                                .on_click(cx.listener(
                                    move |this, _, _, cx| {
                                        this.delete_conversation(delete_id, cx);
                                    },
                                )),
                            ),
                    )
                    .on_backdrop_click({
                        let entity = cx.entity();
                        move |_, cx| {
                            entity.update(cx, |this, cx| {
                                this.pending_delete = None;
                                cx.notify();
                            });
                        }
                    }),
            );
        }

        root
    }
}
