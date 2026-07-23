use gpui::{AppContext, AsyncApp, Context, Entity, EventEmitter, Subscription};
use shelldeck_core::ai::{
    configured_cli_available, create_client, AiConfig, AiContext, AiSurface, AiTask, AiTaskStatus,
    AiTaskStore,
};
use std::cell::RefCell;
use std::rc::Rc;
use uuid::Uuid;

use crate::ai_assistant::{AiAssistantEvent, AiAssistantView};
use crate::t;

#[derive(Debug, Clone)]
pub enum AiCompanionEvent {
    ResumeTask(Uuid),
    OpenTaskTarget(Uuid),
    StopTask(Uuid),
    DeleteTask(Uuid),
}

/// Owns the standalone companion assistant independently from `Workspace`.
///
/// Global chat completions can therefore run while the main application
/// surface and its pollers remain uninitialized. Task actions that need a
/// terminal, ticket, or script surface are emitted to the application root.
pub struct AiCompanionController {
    assistant: Entity<AiAssistantView>,
    config: Rc<RefCell<AiConfig>>,
    tasks: Vec<AiTask>,
    _assistant_sub: Subscription,
}

impl EventEmitter<AiCompanionEvent> for AiCompanionController {}

impl AiCompanionController {
    pub fn new(config: AiConfig, cx: &mut Context<Self>) -> Self {
        let mut tasks = AiTaskStore::load().unwrap_or_else(|error| {
            tracing::warn!("Failed to load AI tasks for companion: {error}");
            Vec::new()
        });
        let mut recovered = false;
        for task in &mut tasks {
            if task.status.is_active() {
                task.set_status(AiTaskStatus::Cancelled, None);
                recovered = true;
            }
        }
        if recovered {
            let _ = AiTaskStore::save(&tasks);
        }

        let assistant = cx.new(|cx| {
            let mut view = AiAssistantView::new(
                AiContext::new(
                    AiSurface::Global,
                    t!("ai.context.global").to_string(),
                    serde_json::json!({}),
                ),
                cx,
            );
            view.set_history_open(false, cx);
            view.set_tasks(tasks.clone(), cx);
            view
        });
        let config = Rc::new(RefCell::new(config));
        let assistant_sub =
            cx.subscribe(
                &assistant,
                |this, view, event: &AiAssistantEvent, cx| match event.clone() {
                    AiAssistantEvent::Submit {
                        request_id,
                        conversation_id,
                        prompt,
                        context,
                    } => {
                        let config = this.config.borrow().clone();
                        let source = view.clone();
                        cx.spawn(async move |_this, cx: &mut AsyncApp| {
                            let result = cx
                                .background_executor()
                                .spawn(async move {
                                    let client = create_client(&config)?;
                                    client
                                        .complete(&prompt, context)
                                        .map(|response| response.text)
                                })
                                .await
                                .map_err(|error| error.to_string());
                            let _ = cx.update(|cx| {
                                source.update(cx, |assistant, cx| {
                                    assistant.set_result(request_id, conversation_id, result, cx);
                                });
                            });
                        })
                        .detach();
                    }
                    AiAssistantEvent::ResumeTask(id) => {
                        cx.emit(AiCompanionEvent::ResumeTask(id));
                    }
                    AiAssistantEvent::OpenTaskTarget(id) => {
                        cx.emit(AiCompanionEvent::OpenTaskTarget(id));
                    }
                    AiAssistantEvent::StopTask(id) => {
                        cx.emit(AiCompanionEvent::StopTask(id));
                    }
                    AiAssistantEvent::DeleteTask(id) => {
                        cx.emit(AiCompanionEvent::DeleteTask(id));
                    }
                },
            );

        Self {
            assistant,
            config,
            tasks,
            _assistant_sub: assistant_sub,
        }
    }

    pub fn assistant(&self) -> Entity<AiAssistantView> {
        self.assistant.clone()
    }

    pub fn shared_config(&self) -> Rc<RefCell<AiConfig>> {
        self.config.clone()
    }

    pub fn tasks(&self) -> Vec<AiTask> {
        self.tasks.clone()
    }

    pub fn prepare(&mut self, cx: &mut Context<Self>) -> Entity<AiAssistantView> {
        let config = self.config.borrow().clone();
        let available = config.is_configured()
            && (!config.backend.is_cli() || configured_cli_available(&config))
            && config.allows(AiSurface::Global);
        self.assistant.update(cx, |assistant, cx| {
            assistant.reload_conversations(cx);
            assistant.set_backend(config.backend, config.model, cx);
            assistant.set_context(
                AiContext::new(
                    AiSurface::Global,
                    t!("ai.context.global").to_string(),
                    serde_json::json!({}),
                ),
                cx,
            );
            assistant.set_available(available, cx);
        });
        self.assistant.clone()
    }

    /// Refresh a hidden Dock without invalidating an in-flight request.
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        let config = self.config.borrow().clone();
        let available = config.is_configured()
            && (!config.backend.is_cli() || configured_cli_available(&config))
            && config.allows(AiSurface::Global);
        self.assistant.update(cx, |assistant, cx| {
            assistant.reload_conversations(cx);
            assistant.set_backend(config.backend, config.model, cx);
            assistant.set_available(available, cx);
        });
    }
}
