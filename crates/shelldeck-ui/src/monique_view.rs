//! Native Monique console. Network work stays in `Workspace`; this view only
//! renders the typed Automonique snapshot and emits explicit user actions.

use adabraka_ui::components::input::{Input, InputSize, InputState};
use adabraka_ui::prelude::Markdown;
use gpui::prelude::*;
use gpui::*;
use shelldeck_core::config::monique::{
    MoniqueChatAction, MoniqueChatHistory, MoniqueChatMessage, MoniqueChatResponse,
    MoniqueProcesses, MoniqueStatus,
};

use crate::scale::px;
use crate::t;
use crate::theme::ShellDeckColors;

#[derive(Debug, Clone)]
pub enum MoniqueViewEvent {
    Refresh,
    Send(String),
    ResolveAction { action_id: String, approved: bool },
    NewChat,
}

impl EventEmitter<MoniqueViewEvent> for MoniqueView {}

pub struct MoniqueView {
    status: Option<MoniqueStatus>,
    processes: Option<MoniqueProcesses>,
    messages: Vec<MoniqueChatMessage>,
    pending_actions: Vec<MoniqueChatAction>,
    composer: Entity<InputState>,
    loading: bool,
    error: Option<String>,
    scroll: ScrollHandle,
}

impl MoniqueView {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            status: None,
            processes: None,
            messages: Vec::new(),
            pending_actions: Vec::new(),
            composer: cx.new(InputState::new),
            loading: false,
            error: None,
            scroll: ScrollHandle::new(),
        }
    }

    pub fn set_loading(&mut self, loading: bool) {
        self.loading = loading;
        if loading {
            self.error = None;
        }
    }

    pub fn set_error(&mut self, error: impl Into<String>) {
        self.error = Some(error.into());
        self.loading = false;
    }

    pub fn set_snapshot(
        &mut self,
        status: MoniqueStatus,
        processes: MoniqueProcesses,
        history: MoniqueChatHistory,
    ) {
        self.status = Some(status);
        self.processes = Some(processes);
        self.messages = history.messages;
        self.pending_actions = history.pending_actions;
        self.loading = false;
        self.error = None;
    }

    pub fn apply_response(&mut self, response: MoniqueChatResponse) {
        if !response.answer.trim().is_empty() {
            self.messages.push(MoniqueChatMessage {
                role: "assistant".to_string(),
                content: response.answer,
                created_at_ms: chrono::Utc::now().timestamp_millis(),
            });
        }
        if let Some(action) = response.action {
            self.pending_actions.retain(|item| item.id != action.id);
            self.pending_actions.push(action);
        }
        self.loading = false;
        self.error = None;
    }

    pub fn clear_chat(&mut self) {
        self.messages.clear();
        self.pending_actions.clear();
        self.loading = false;
        self.error = None;
    }

    pub fn pending_action_count(&self) -> usize {
        self.pending_actions.len()
    }

    fn submit(&mut self, cx: &mut Context<Self>) {
        if self.loading {
            return;
        }
        let message = self.composer.read(cx).content().trim().to_string();
        if message.is_empty() {
            return;
        }
        self.composer.update(cx, |state, cx| state.reset(cx));
        self.messages.push(MoniqueChatMessage {
            role: "user".to_string(),
            content: message.clone(),
            created_at_ms: chrono::Utc::now().timestamp_millis(),
        });
        self.loading = true;
        cx.emit(MoniqueViewEvent::Send(message));
        cx.notify();
    }

    fn button(
        id: impl Into<ElementId>,
        label: impl Into<SharedString>,
        cx: &mut Context<Self>,
        handler: impl Fn(&mut Self, &mut Context<Self>) + 'static,
    ) -> Stateful<Div> {
        div()
            .id(id)
            .px(px(10.0))
            .py(px(6.0))
            .rounded(px(7.0))
            .border_1()
            .border_color(ShellDeckColors::border())
            .bg(ShellDeckColors::bg_primary())
            .text_size(px(12.0))
            .text_color(ShellDeckColors::text_primary())
            .cursor_pointer()
            .hover(|style| style.bg(ShellDeckColors::hover_bg()))
            .child(label.into())
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| handler(this, cx)))
    }

    fn metric(label: String, value: String) -> impl IntoElement {
        div()
            .min_w(px(104.0))
            .flex_1()
            .flex()
            .flex_col()
            .gap(px(3.0))
            .p(px(10.0))
            .rounded(px(8.0))
            .border_1()
            .border_color(ShellDeckColors::border())
            .bg(ShellDeckColors::bg_surface())
            .child(
                div()
                    .text_size(px(10.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(label),
            )
            .child(
                div()
                    .text_size(px(17.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(ShellDeckColors::text_primary())
                    .child(value),
            )
    }

    fn render_header(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let ready = self.status.as_ref().is_some_and(MoniqueStatus::ready);
        let generation = self
            .status
            .as_ref()
            .and_then(|status| status.generation)
            .map_or_else(|| "—".to_string(), |value| value.to_string());
        div()
            .flex()
            .items_center()
            .gap(px(10.0))
            .px(px(14.0))
            .py(px(10.0))
            .border_b_1()
            .border_color(ShellDeckColors::border())
            .child(div().size(px(8.0)).rounded_full().bg(if ready {
                ShellDeckColors::success()
            } else {
                ShellDeckColors::warning()
            }))
            .child(
                div()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .text_size(px(14.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(ShellDeckColors::text_primary())
                            .child(t!("monique.title").to_string()),
                    )
                    .child(
                        div()
                            .text_size(px(11.0))
                            .text_color(ShellDeckColors::text_muted())
                            .child(format!(
                                "{} · {} {generation}",
                                if ready {
                                    t!("monique.status.ready")
                                } else {
                                    t!("monique.status.check")
                                },
                                t!("monique.generation")
                            )),
                    ),
            )
            .child(Self::button(
                "monique-new-chat",
                t!("monique.new_chat").to_string(),
                cx,
                |_this, cx| cx.emit(MoniqueViewEvent::NewChat),
            ))
            .child(Self::button(
                "monique-refresh",
                t!("monique.refresh").to_string(),
                cx,
                |_this, cx| cx.emit(MoniqueViewEvent::Refresh),
            ))
    }

    fn render_metrics(&self) -> impl IntoElement {
        let status = self.status.as_ref();
        let stats = self.processes.as_ref().map(|snapshot| &snapshot.stats);
        div()
            .flex()
            .flex_wrap()
            .gap(px(8.0))
            .px(px(14.0))
            .pt(px(12.0))
            .child(Self::metric(
                t!("monique.metric.running").to_string(),
                status
                    .and_then(|value| value.running)
                    .unwrap_or(0)
                    .to_string(),
            ))
            .child(Self::metric(
                t!("monique.metric.queued").to_string(),
                stats.map_or(0, |value| value.queued).to_string(),
            ))
            .child(Self::metric(
                t!("monique.metric.completed").to_string(),
                stats.map_or(0, |value| value.completed).to_string(),
            ))
            .child(Self::metric(
                t!("monique.metric.reconciliation").to_string(),
                status
                    .and_then(|value| value.reconciliation_pending)
                    .unwrap_or(0)
                    .to_string(),
            ))
    }

    fn render_action(action: &MoniqueChatAction, cx: &mut Context<Self>) -> impl IntoElement {
        let approve_id = action.id.clone();
        let reject_id = action.id.clone();
        div()
            .flex()
            .flex_col()
            .gap(px(6.0))
            .p(px(10.0))
            .rounded(px(8.0))
            .border_1()
            .border_color(ShellDeckColors::warning())
            .bg(ShellDeckColors::warning().opacity(0.08))
            .child(
                div()
                    .text_size(px(12.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(ShellDeckColors::text_primary())
                    .child(action.title.clone()),
            )
            .child(
                div()
                    .text_size(px(12.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(action.detail.clone()),
            )
            .when(!action.impact.is_empty(), |card| {
                card.child(
                    div()
                        .text_size(px(11.0))
                        .text_color(ShellDeckColors::warning())
                        .child(action.impact.clone()),
                )
            })
            .child(
                div()
                    .flex()
                    .gap(px(6.0))
                    .child(Self::button(
                        ElementId::from(SharedString::from(format!(
                            "monique-approve-{approve_id}"
                        ))),
                        t!("monique.action.approve").to_string(),
                        cx,
                        move |_this, cx| {
                            cx.emit(MoniqueViewEvent::ResolveAction {
                                action_id: approve_id.clone(),
                                approved: true,
                            })
                        },
                    ))
                    .child(Self::button(
                        ElementId::from(SharedString::from(format!("monique-reject-{reject_id}"))),
                        t!("monique.action.reject").to_string(),
                        cx,
                        move |_this, cx| {
                            cx.emit(MoniqueViewEvent::ResolveAction {
                                action_id: reject_id.clone(),
                                approved: false,
                            })
                        },
                    )),
            )
    }

    fn render_message(message: &MoniqueChatMessage) -> impl IntoElement {
        let assistant = message.role == "assistant";
        div()
            .flex()
            .flex_col()
            .gap(px(4.0))
            .p(px(10.0))
            .rounded(px(9.0))
            .border_1()
            .border_color(ShellDeckColors::border())
            .bg(if assistant {
                ShellDeckColors::bg_surface()
            } else {
                ShellDeckColors::selected_bg()
            })
            .child(
                div()
                    .text_size(px(10.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(ShellDeckColors::text_muted())
                    .child(if assistant {
                        "MONIQUE".to_string()
                    } else {
                        t!("monique.you").to_string()
                    }),
            )
            .child(
                Markdown::new(message.content.clone())
                    .base_font_size(gpui::px(13.0))
                    .compact()
                    .w_full()
                    .min_w(px(0.0)),
            )
    }

    fn render_conversation(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut list = div()
            .id("monique-conversation")
            .track_scroll(&self.scroll)
            .flex_1()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap(px(8.0))
            .p(px(14.0));
        if self.messages.is_empty() {
            list = list.child(
                div()
                    .py(px(24.0))
                    .text_size(px(13.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(t!("monique.empty").to_string()),
            );
        } else {
            for message in &self.messages {
                list = list.child(Self::render_message(message));
            }
        }
        for action in &self.pending_actions {
            list = list.child(Self::render_action(action, cx));
        }
        if self.loading {
            list = list.child(
                div()
                    .py(px(8.0))
                    .text_size(px(12.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(t!("monique.thinking").to_string()),
            );
        }
        list
    }

    fn render_composer(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let entity = cx.entity();
        div()
            .flex()
            .items_center()
            .gap(px(8.0))
            .px(px(14.0))
            .py(px(10.0))
            .border_t_1()
            .border_color(ShellDeckColors::border())
            .child(
                div().flex_1().child(
                    Input::new(&self.composer)
                        .size(InputSize::Sm)
                        .placeholder(t!("monique.placeholder").to_string())
                        .on_enter(move |_value, cx| {
                            entity.update(cx, |this, cx| this.submit(cx));
                        }),
                ),
            )
            .child(Self::button(
                "monique-send",
                t!("monique.send").to_string(),
                cx,
                |this, cx| this.submit(cx),
            ))
    }
}

impl Render for MoniqueView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut root = div()
            .id("monique-view-root")
            .size_full()
            .flex()
            .flex_col()
            .bg(ShellDeckColors::bg_primary())
            .child(self.render_header(cx))
            .child(self.render_metrics())
            .child(self.render_conversation(cx))
            .child(self.render_composer(cx));
        if let Some(error) = &self.error {
            root = root.child(
                div()
                    .absolute()
                    .bottom(px(64.0))
                    .left(px(14.0))
                    .right(px(14.0))
                    .p(px(9.0))
                    .rounded(px(8.0))
                    .bg(ShellDeckColors::error())
                    .text_size(px(12.0))
                    .text_color(white())
                    .child(error.clone()),
            );
        }
        root
    }
}
