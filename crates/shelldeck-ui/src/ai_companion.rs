use gpui::{AppContext, AsyncApp, Context, Entity, EventEmitter, Subscription};
use shelldeck_core::ai::{
    complete_assistant_turn, configured_cli_available, create_client, AiAssistantAction,
    AiAssistantCompletion, AiConfig, AiContext, AiSurface, AiTask, AiTaskStatus, AiTaskStore,
    ClippyConfig,
};
use std::cell::RefCell;
use std::rc::Rc;
use uuid::Uuid;

use crate::ai_assistant::{AiAssistantEvent, AiAssistantView};
use crate::t;

#[derive(Debug, Clone)]
pub enum AiCompanionEvent {
    ApplyAction(AiAssistantAction),
    /// Raised from the Dock's rail toolbox; the app root must show the main
    /// window before the workspace can honour it.
    OpenSettings,
    /// Bring the main window forward and close the Dock.
    OpenMainWindow,
    /// The command palette is its own window, owned by the app root.
    OpenPalette,
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
    clippy_config: Rc<RefCell<ClippyConfig>>,
    tasks: Vec<AiTask>,
    _assistant_sub: Subscription,
}

impl EventEmitter<AiCompanionEvent> for AiCompanionController {}

impl AiCompanionController {
    pub fn new(config: AiConfig, clippy_config: ClippyConfig, cx: &mut Context<Self>) -> Self {
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
            view.set_host(crate::ai_assistant::AiHost::Dock, cx);
            view.set_history_open(false, cx);
            view.set_tasks(tasks.clone(), cx);
            view
        });
        let config = Rc::new(RefCell::new(config));
        let clippy_config = Rc::new(RefCell::new(clippy_config));
        let assistant_sub =
            cx.subscribe(
                &assistant,
                |this, view, event: &AiAssistantEvent, cx| match event.clone() {
                    // The Dock has its own close control in `AiDockView`; the
                    // in-view one only exists for the Sheet.
                    AiAssistantEvent::Close => {}
                    // The mention directory is built from Workspace state. The
                    // Workspace subscribes to this same entity for exactly this
                    // event; when it does not exist yet (standalone Dock before
                    // the main window), there is nothing to rebuild and the
                    // picker correctly reports that mentions are unavailable.
                    AiAssistantEvent::RefreshMentions => {}
                    // The Dock cannot open Settings itself — it is a separate
                    // window with no workspace. Forwarded to the app root,
                    // which raises the main window and routes it.
                    AiAssistantEvent::OpenSettings => {
                        cx.emit(AiCompanionEvent::OpenSettings);
                    }
                    AiAssistantEvent::OpenMainWindow => {
                        cx.emit(AiCompanionEvent::OpenMainWindow);
                    }
                    AiAssistantEvent::OpenPalette => {
                        cx.emit(AiCompanionEvent::OpenPalette);
                    }
                    // Applied to the shared `AiConfig` the Workspace also
                    // holds, so the change is live in both hosts immediately.
                    // Durable persistence is the Workspace's job via Settings.
                    AiAssistantEvent::SelectBackend(backend) => {
                        {
                            let mut config = this.config.borrow_mut();
                            config.backend = backend;
                            config.model.clear();
                        }
                        this.refresh(cx);
                    }
                    AiAssistantEvent::Submit {
                        request_id,
                        conversation_id,
                        prompt,
                        latest_user_message,
                        context,
                    } => {
                        let config = this.config.borrow().clone();
                        let source = view.clone();
                        cx.spawn(async move |this, cx: &mut AsyncApp| {
                            let result = cx
                                .background_executor()
                                .spawn(async move {
                                    let client = create_client(&config)?;
                                    complete_assistant_turn(
                                        client.as_ref(),
                                        &prompt,
                                        &latest_user_message,
                                        *context,
                                    )
                                })
                                .await
                                .map_err(|error| error.to_string());
                            let _ = this.update(cx, |_controller, cx| match result {
                                Ok(AiAssistantCompletion::Message(message)) => {
                                    source.update(cx, |assistant, cx| {
                                        assistant.set_result(
                                            request_id,
                                            conversation_id,
                                            Ok(message),
                                            cx,
                                        );
                                    });
                                }
                                Ok(AiAssistantCompletion::Action(action)) => {
                                    let accepted = source.update(cx, |assistant, cx| {
                                        assistant.set_result(
                                            request_id,
                                            conversation_id,
                                            Ok(assistant_action_acknowledgement(&action)),
                                            cx,
                                        )
                                    });
                                    if accepted {
                                        cx.emit(AiCompanionEvent::ApplyAction(action));
                                    }
                                }
                                Err(error) => {
                                    source.update(cx, |assistant, cx| {
                                        assistant.set_result(
                                            request_id,
                                            conversation_id,
                                            Err(error),
                                            cx,
                                        );
                                    });
                                }
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
            clippy_config,
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

    pub fn shared_clippy_config(&self) -> Rc<RefCell<ClippyConfig>> {
        self.clippy_config.clone()
    }

    pub fn tasks(&self) -> Vec<AiTask> {
        self.tasks.clone()
    }

    pub fn prepare(&mut self, cx: &mut Context<Self>) -> Entity<AiAssistantView> {
        let config = self.config.borrow().clone();
        let backend_available = config.is_configured()
            && (!config.backend.is_cli() || configured_cli_available(&config));
        let available = backend_available && config.allows(AiSurface::Global);
        let clippy_available = backend_available && config.allows(AiSurface::Clippy);
        let clippy_config = self.clippy_config.borrow().clone();
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
            assistant.set_clippy_available(clippy_available, cx);
            assistant.set_clippy_auto_import_clipboard(
                clippy_config.auto_import_clipboard_on_shortcut,
                cx,
            );
        });
        self.assistant.clone()
    }

    pub fn prepare_clippy(&mut self, cx: &mut Context<Self>) -> Entity<AiAssistantView> {
        let assistant = self.prepare(cx);
        assistant.update(cx, |assistant, cx| assistant.show_clippy(cx));
        assistant
    }

    pub fn prepare_task_center(&mut self, cx: &mut Context<Self>) -> Entity<AiAssistantView> {
        let assistant = self.prepare(cx);
        assistant.update(cx, |assistant, cx| assistant.show_tasks(cx));
        assistant
    }

    /// Refresh a hidden Dock without invalidating an in-flight request.
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        let config = self.config.borrow().clone();
        let backend_available = config.is_configured()
            && (!config.backend.is_cli() || configured_cli_available(&config));
        let available = backend_available && config.allows(AiSurface::Global);
        let clippy_available = backend_available && config.allows(AiSurface::Clippy);
        let clippy_config = self.clippy_config.borrow().clone();
        self.assistant.update(cx, |assistant, cx| {
            assistant.reload_conversations(cx);
            assistant.set_backend(config.backend, config.model, cx);
            assistant.set_available(available, cx);
            assistant.set_clippy_available(clippy_available, cx);
            assistant.set_clippy_auto_import_clipboard(
                clippy_config.auto_import_clipboard_on_shortcut,
                cx,
            );
        });
    }
}

pub(crate) fn assistant_action_acknowledgement(action: &AiAssistantAction) -> String {
    match action {
        AiAssistantAction::CreateRequest(_) => t!("ai.assistant.request_draft_ready").to_string(),
        AiAssistantAction::CreateScript { .. } => t!("ai.assistant.script_draft_ready").to_string(),
        AiAssistantAction::TerminalCommand { .. } => {
            t!("ai.assistant.terminal_command_ready").to_string()
        }
        AiAssistantAction::SupportReply { .. } => {
            t!("ai.assistant.support_reply_ready").to_string()
        }
        AiAssistantAction::MoniqueDispatch { .. } => {
            t!("ai.assistant.monique_dispatch_ready").to_string()
        }
        AiAssistantAction::OpenRequest { .. } => t!("ai.assistant.request_opening").to_string(),
    }
}
