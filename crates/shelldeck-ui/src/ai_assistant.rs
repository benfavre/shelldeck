use adabraka_ui::components::confirm_dialog::Dialog as UiDialog;
use adabraka_ui::components::empty_state::{EmptyState, EmptyStateSize};
use adabraka_ui::components::icon_button::IconButton;
use adabraka_ui::components::icon_source::IconSource;
use adabraka_ui::components::input::{Input, InputSize};
use adabraka_ui::components::input_state::InputState;
use adabraka_ui::prelude::{
    scrollable_vertical, use_theme, Badge, BadgeVariant, Button, ButtonSize, ButtonVariant,
    Composer, Markdown, Spinner, SpinnerSize, SpinnerVariant,
};
use gpui::prelude::*;
use gpui::*;
use shelldeck_core::ai::{
    ai_line_diff, clippy_prompt as build_clippy_prompt, AiBackend, AiCapability, AiChatRole,
    AiContext, AiConversation, AiConversationStore, AiDiffLine, AiSurface, AiTask, AiTaskStatus,
    ClippyContext, ClippyContextSource, ClippyOperation as CoreClippyOperation, ClippyProposal,
};
use uuid::Uuid;

use crate::ai_workflow::capability_result_is_markdown;
use crate::icons::{ai_provider_inline, lucide_icon, lucide_path};
use crate::monolith::{animated_loading_text, animated_monolith, MonolithMotion};
use crate::scale::px;
use crate::t;
use crate::theme::ShellDeckColors;

#[derive(Debug, Clone)]
pub enum AiAssistantEvent {
    /// The Sheet host draws no chrome of its own any more, so its close control
    /// lives in the view and asks the host to dismiss.
    Close,
    /// The composer's provider chip is a picker. The view does not own `ai.*`
    /// config, so it asks the host to persist the choice.
    SelectBackend(AiBackend),
    /// Rail toolbox: open Settings. The Dock is a separate window and cannot
    /// reach the settings surface on its own, so the host does it.
    OpenSettings,
    /// Rail toolbox: bring the main ShellDeck window forward.
    OpenMainWindow,
    /// Rail toolbox: the command palette lives in its own window, opened by the
    /// app root — the Dock has no visible way in otherwise.
    OpenPalette,
    Submit {
        request_id: u64,
        conversation_id: Uuid,
        prompt: String,
        latest_user_message: String,
        context: AiContext,
    },
    ResumeTask(Uuid),
    OpenTaskTarget(Uuid),
    StopTask(Uuid),
    DeleteTask(Uuid),
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

/// Which window is rendering the view. The two hosts are not the same display:
/// the Sheet is an 780px overlay inside the main window, the Dock a 480px
/// chromeless OS window at the screen edge. Layout decisions that fit one are
/// wrong in the other, so the view has to know.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum AiHost {
    /// `Sheet` in the main window — history is a column, the thread gets a
    /// bounded reading measure, and there is no rail.
    #[default]
    Sheet,
    /// The screen-edge Dock window — history swaps the panel and the rail is
    /// the only navigation.
    Dock,
}

/// The Sheet itself is 780 absolute pixels wide, but its conversation column
/// loses the scaled 240px history rail when that rail is visible. Bubble
/// shaping needs that effective width up front because GPUI cannot recover a
/// wrapped height from an oversized definite child after flex layout clips it.
fn sheet_message_reading_width(
    viewport_width: Pixels,
    history_open: bool,
    history_width: Pixels,
    thread_cap: Pixels,
) -> Pixels {
    let visible_sheet_width = viewport_width.min(gpui::px(780.0));
    let conversation_width = if history_open {
        (visible_sheet_width - history_width).max(gpui::px(0.0))
    } else {
        visible_sheet_width
    };
    conversation_width.min(thread_cap)
}

/// What the panel is showing. The rail selects one of these; there is no
/// second navigation surface. `History` swaps the panel rather than splitting
/// it — at 424px (the dock minus its rail) a second column would leave the
/// conversation under 200px.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum AiActivity {
    #[default]
    Chat,
    Clippy,
    Tasks,
    History,
}

struct AiRailEntry {
    activity: AiActivity,
    icon: &'static str,
    label: String,
    badge: Option<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AiQuickActionMode {
    Submit,
    Prefill,
}

struct AiQuickAction {
    label: String,
    prompt_key: &'static str,
    icon: &'static str,
    mode: AiQuickActionMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ClippyOperationKind {
    Rewrite,
    Translate,
    Shorten,
    Summarize,
    Explain,
    DraftReply,
    Custom,
}

impl ClippyOperationKind {
    fn all() -> &'static [Self] {
        &[
            Self::Rewrite,
            Self::Translate,
            Self::Shorten,
            Self::Summarize,
            Self::Explain,
            Self::DraftReply,
            Self::Custom,
        ]
    }

    fn key(self) -> &'static str {
        match self {
            Self::Rewrite => "rewrite",
            Self::Translate => "translate",
            Self::Shorten => "shorten",
            Self::Summarize => "summarize",
            Self::Explain => "explain",
            Self::DraftReply => "draft_reply",
            Self::Custom => "custom",
        }
    }

    fn icon(self) -> &'static str {
        match self {
            Self::Rewrite => "sparkles",
            Self::Translate => "globe",
            Self::Shorten => "minimize-2",
            Self::Summarize => "scroll-text",
            Self::Explain => "circle-help",
            Self::DraftReply => "reply",
            Self::Custom => "settings",
        }
    }

    fn label(self) -> String {
        match self {
            Self::Rewrite => t!("ai.clippy.operation.rewrite").to_string(),
            Self::Translate => t!("ai.clippy.operation.translate").to_string(),
            Self::Shorten => t!("ai.clippy.operation.shorten").to_string(),
            Self::Summarize => t!("ai.clippy.operation.summarize").to_string(),
            Self::Explain => t!("ai.clippy.operation.explain").to_string(),
            Self::DraftReply => t!("ai.clippy.operation.draft_reply").to_string(),
            Self::Custom => t!("ai.clippy.operation.custom").to_string(),
        }
    }
}

fn should_auto_import_clippy(enabled: bool, current_source: &str) -> bool {
    enabled && current_source.trim().is_empty()
}

/// Whether replacing the view's context with `next` is a real context switch
/// (which must invalidate the request gate and reset transient state) rather
/// than a host re-preparing the same surface (which must preserve any
/// in-flight request). Pure so the reopen contract is unit-testable.
fn context_switch_resets(current: &AiContext, next: &AiContext) -> bool {
    current.surface != next.surface || current.title != next.title
}

fn validated_clippy_result(text: String) -> Result<String, String> {
    let proposal = ClippyProposal {
        result: text.trim().to_string(),
        explanation: None,
        warnings: Vec::new(),
    };
    proposal.validate().map_err(|error| error.to_string())?;
    Ok(proposal.result)
}

pub struct AiAssistantView {
    prompt_state: Entity<InputState>,
    context: AiContext,
    available: bool,
    clippy_available: bool,
    loading: bool,
    error: Option<String>,
    request_gate: AiRequestGate,
    backend: AiBackend,
    model: String,
    conversations: Vec<AiConversation>,
    active_conversation: Option<Uuid>,
    message_scroll: ScrollHandle,
    history_scroll: ScrollHandle,
    task_scroll: ScrollHandle,
    history_open: bool,
    host: AiHost,
    backend_menu_open: bool,
    notice: Option<String>,
    account_label: Option<String>,
    account_detail: Option<String>,
    account_menu_open: bool,
    /// The context chip is removable: dropping it sends the question without
    /// the current screen attached. Reset whenever the host sets a new context.
    context_dropped: bool,
    active_tab: AiActivity,
    show_archived: bool,
    pending_delete: Option<Uuid>,
    tasks: Vec<AiTask>,
    clippy_source_state: Entity<InputState>,
    clippy_instruction_state: Entity<InputState>,
    clippy_result: Option<String>,
    clippy_error: Option<String>,
    clippy_operation: ClippyOperationKind,
    clippy_pending_request: Option<u64>,
    clippy_auto_import_clipboard: bool,
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
            available: true,
            // Clippy is opt-in (`ai.surfaces.clippy`): stay unavailable until a
            // host pushes the real capability via `set_clippy_available`.
            clippy_available: false,
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
            task_scroll: ScrollHandle::new(),
            history_open: true,
            host: AiHost::default(),
            backend_menu_open: false,
            notice: None,
            account_label: None,
            account_detail: None,
            account_menu_open: false,
            context_dropped: false,
            active_tab: AiActivity::Chat,
            show_archived: false,
            pending_delete: None,
            tasks: Vec::new(),
            clippy_source_state: cx.new(|cx| InputState::new(cx).multi_line(true)),
            clippy_instruction_state: cx.new(InputState::new),
            clippy_result: None,
            clippy_error: None,
            clippy_operation: ClippyOperationKind::Rewrite,
            clippy_pending_request: None,
            clippy_auto_import_clipboard: false,
        }
    }

    /// Who the assistant is working for. The Dock is a chromeless window with
    /// no titlebar account chip, so without this it never says which account or
    /// site its context belongs to.
    pub fn set_account_label(
        &mut self,
        label: Option<String>,
        detail: Option<String>,
        cx: &mut Context<Self>,
    ) {
        if self.account_label != label || self.account_detail != detail {
            self.account_label = label;
            self.account_detail = detail;
            if self.account_label.is_none() {
                self.account_menu_open = false;
            }
            cx.notify();
        }
    }

    /// Set once by the host that builds the view.
    pub fn set_host(&mut self, host: AiHost, cx: &mut Context<Self>) {
        self.host = host;
        cx.notify();
    }

    pub fn set_tasks(&mut self, tasks: Vec<AiTask>, cx: &mut Context<Self>) {
        self.tasks = tasks;
        cx.notify();
    }

    /// Kept as the hosts' entry point (the Dock opens without history).
    /// `history_open` is Sheet-only state — it drives the side column and the
    /// header toggle. The Dock never reads it (its rail renders on
    /// `active_tab`), so a Dock host maps the call onto its activity instead
    /// of writing dead state.
    pub fn set_history_open(&mut self, open: bool, cx: &mut Context<Self>) {
        match self.host {
            AiHost::Sheet => self.history_open = open,
            AiHost::Dock => {
                self.active_tab = if open {
                    AiActivity::History
                } else {
                    AiActivity::Chat
                };
            }
        }
        cx.notify();
    }

    pub fn show_tasks(&mut self, cx: &mut Context<Self>) {
        self.active_tab = AiActivity::Tasks;
        cx.notify();
    }

    /// Title of the thread on screen, used by the host-specific header.
    pub fn active_title(&self) -> String {
        self.active_conversation()
            .map(AiConversation::display_title)
            .unwrap_or_else(|| t!("ai.history.new").to_string())
    }

    /// Reload the durable conversation list before a separately hosted
    /// assistant surface becomes visible. The main sheet and companion Dock
    /// use distinct view entities so their focus/request gates cannot interfere,
    /// while this refresh keeps their persisted history coherent.
    pub fn reload_conversations(&mut self, cx: &mut Context<Self>) {
        match AiConversationStore::load() {
            Ok(conversations) => {
                self.active_conversation = conversations
                    .iter()
                    .rev()
                    .find(|conversation| !conversation.archived)
                    .map(|conversation| conversation.id);
                self.conversations = conversations;
                self.show_archived = false;
                self.pending_delete = None;
            }
            Err(error) => tracing::warn!("Failed to reload AI conversations: {error}"),
        }
        cx.notify();
    }

    pub fn set_backend(&mut self, backend: AiBackend, model: String, cx: &mut Context<Self>) {
        self.backend = backend;
        self.model = model;
        cx.notify();
    }

    pub fn set_context(&mut self, context: AiContext, cx: &mut Context<Self>) {
        // Re-preparing a host over the *same* context must not kill an
        // in-flight request. The Dock removes its window on focus loss while
        // the controller (and any pending completion) survives; reopening runs
        // `prepare()` → `set_context()`, and unconditionally invalidating here
        // meant the reply landed on a dead gate — the user saw their own
        // message with no answer, no spinner, and no error, forever. Only a
        // real context switch discards outstanding work.
        let context_changed = context_switch_resets(&self.context, &context);
        self.context = context;
        if context_changed {
            // A genuine context switch re-arms the removable chip: the user
            // dropped the *previous* screen, not this one.
            self.context_dropped = false;
            self.request_gate.invalidate();
            self.loading = false;
            self.error = None;
            // A pending Clippy id must not outlive the gate that issued it.
            self.clippy_pending_request = None;
            if self
                .active_conversation()
                .is_some_and(|conversation| !conversation.messages.is_empty())
            {
                self.active_conversation = None;
            }
        }
        cx.notify();
    }

    pub fn set_available(&mut self, available: bool, cx: &mut Context<Self>) {
        let was_available = self.available;
        self.available = available;
        if !available {
            self.request_gate.invalidate();
            self.loading = false;
            self.error = Some(t!("ai.dock.unavailable").to_string());
        } else if !was_available {
            self.error = None;
        }
        cx.notify();
    }

    pub fn set_clippy_available(&mut self, available: bool, cx: &mut Context<Self>) {
        self.clippy_available = available;
        if !available && self.clippy_pending_request.is_some() {
            self.request_gate.invalidate();
            self.loading = false;
            self.clippy_pending_request = None;
            self.clippy_error = Some(t!("ai.dock.unavailable").to_string());
        }
        cx.notify();
    }

    pub fn show_clippy(&mut self, cx: &mut Context<Self>) {
        self.active_tab = AiActivity::Clippy;
        if should_auto_import_clippy(
            self.clippy_auto_import_clipboard,
            self.clippy_source_state.read(cx).content(),
        ) {
            self.load_clippy_clipboard(cx);
        }
        cx.notify();
    }

    pub fn set_clippy_auto_import_clipboard(&mut self, enabled: bool, cx: &mut Context<Self>) {
        if self.clippy_auto_import_clipboard != enabled {
            self.clippy_auto_import_clipboard = enabled;
            cx.notify();
        }
    }

    pub fn focus_composer(&self, window: &mut Window, cx: &App) {
        if self.available {
            self.prompt_state.read(cx).focus_handle(cx).focus(window);
        }
    }

    pub fn has_open_dialog(&self) -> bool {
        self.pending_delete.is_some() || self.backend_menu_open || self.account_menu_open
    }

    /// Providers offered by the composer picker, mirroring the Settings list.
    fn backend_choices() -> [(AiBackend, &'static str); 5] {
        [
            (AiBackend::ClaudeCli, "Claude Code CLI"),
            (AiBackend::CodexCli, "Codex CLI"),
            (AiBackend::AiderCli, "Aider CLI"),
            (AiBackend::OpenAi, "OpenAI API"),
            (AiBackend::Anthropic, "Anthropic API"),
        ]
    }

    /// A passing note shown above the composer — used by the affordances that
    /// are drawn but not wired yet. Deliberately not `error`: it is neither red
    /// nor a failure, and it clears as soon as the user sends anything.
    fn set_notice(&mut self, message: String, cx: &mut Context<Self>) {
        self.notice = Some(message);
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
    ) -> bool {
        if !self.request_gate.accepts(request_id) {
            return false;
        }
        self.loading = false;
        if self.clippy_pending_request == Some(request_id) {
            self.clippy_pending_request = None;
            match result {
                Ok(text) => match validated_clippy_result(text) {
                    Ok(result) => {
                        self.clippy_result = Some(result);
                        self.clippy_error = None;
                    }
                    Err(error) => {
                        self.clippy_result = None;
                        self.clippy_error = Some(error);
                    }
                },
                Err(error) => self.clippy_error = Some(error),
            }
            cx.notify();
            // The returned bool gates routed `AiAssistantAction` application
            // in both hosts (`Workspace::handle_ai_assistant_event`,
            // `AiCompanionController`). A Clippy turn transforms untrusted
            // clipboard content and must never apply actions — return false
            // so an Action that ever slipped through for a Clippy request is
            // dropped here, not one crate away.
            return false;
        }
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
        true
    }

    fn import_clippy_clipboard(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.load_clippy_clipboard(cx) {
            return;
        }
        self.clippy_source_state
            .read(cx)
            .focus_handle(cx)
            .focus(window);
    }

    fn load_clippy_clipboard(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(item) = cx.read_from_clipboard() else {
            self.clippy_error = Some(t!("ai.clippy.error.no_clipboard").to_string());
            cx.notify();
            return false;
        };
        let Some(text) = item.text() else {
            self.clippy_error = Some(t!("ai.clippy.error.no_clipboard_text").to_string());
            cx.notify();
            return false;
        };
        self.clippy_source_state
            .update(cx, |state, cx| state.replace_content(text, cx));
        self.clippy_error = None;
        cx.notify();
        true
    }

    fn clippy_request(&self, cx: &App) -> Result<(String, AiContext), String> {
        let source = self
            .clippy_source_state
            .read(cx)
            .content()
            .trim()
            .to_string();
        if source.is_empty() {
            return Err(t!("ai.clippy.error.blank").to_string());
        }
        let instruction = self
            .clippy_instruction_state
            .read(cx)
            .content()
            .trim()
            .to_string();
        let operation = match self.clippy_operation {
            ClippyOperationKind::Rewrite => CoreClippyOperation::Rewrite,
            ClippyOperationKind::Translate => CoreClippyOperation::Translate {
                language: if instruction.is_empty() {
                    t!("ai.clippy.default_translation_language").to_string()
                } else {
                    instruction
                },
            },
            ClippyOperationKind::Shorten => CoreClippyOperation::Shorten,
            ClippyOperationKind::Summarize => CoreClippyOperation::Summarize,
            ClippyOperationKind::Explain => CoreClippyOperation::Explain,
            ClippyOperationKind::DraftReply => CoreClippyOperation::DraftReply,
            ClippyOperationKind::Custom => CoreClippyOperation::Custom { instruction },
        };
        let context = ClippyContext {
            source: ClippyContextSource::Clipboard,
            text: source,
            application: None,
            window_title: None,
            focused_role: None,
            screenshot: None,
            selection: None,
        };
        let prompt =
            build_clippy_prompt(&operation, &context).map_err(|error| error.to_string())?;
        let ai_context = context
            .to_ai_context(&operation)
            .map_err(|error| error.to_string())?;
        Ok((prompt, ai_context))
    }

    fn run_clippy(&mut self, cx: &mut Context<Self>) {
        if self.loading || !self.clippy_available {
            return;
        }
        let (prompt, context) = match self.clippy_request(cx) {
            Ok(request) => request,
            Err(error) => {
                self.clippy_error = Some(error);
                cx.notify();
                return;
            }
        };
        let request_id = self.request_gate.begin();
        self.loading = true;
        self.clippy_pending_request = Some(request_id);
        self.clippy_result = None;
        self.clippy_error = None;
        cx.emit(AiAssistantEvent::Submit {
            request_id,
            conversation_id: Uuid::nil(),
            prompt,
            // Clippy submits untrusted clipboard content, not a conversational
            // instruction: the empty message keeps the action router out of
            // the turn entirely (see `complete_assistant_turn`).
            latest_user_message: String::new(),
            context,
        });
        cx.notify();
    }

    fn cancel_clippy(&mut self, cx: &mut Context<Self>) {
        self.request_gate.invalidate();
        self.loading = false;
        self.clippy_pending_request = None;
        self.clippy_error = None;
        cx.notify();
    }

    fn edit_clippy_result(&mut self, cx: &mut Context<Self>) {
        if let Some(result) = self.clippy_result.clone() {
            self.clippy_source_state
                .update(cx, |state, cx| state.replace_content(result, cx));
            self.clippy_result = None;
            cx.notify();
        }
    }

    fn submit(&mut self, cx: &mut Context<Self>) {
        if self.loading || !self.available {
            return;
        }
        // The notice was about an affordance the user just tried; sending
        // something means they moved on.
        self.notice = None;
        let prompt = self.prompt_state.read(cx).content().trim().to_string();
        if prompt.is_empty() {
            return;
        }
        self.submit_prompt(prompt, cx);
    }

    fn submit_prompt(&mut self, prompt: String, cx: &mut Context<Self>) {
        if !self.loading {
            let conversation_id = self.ensure_active_conversation();
            let latest_user_message = prompt.clone();
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
                latest_user_message,
                // Dropping the chip is not cosmetic: the turn really goes out
                // without the screen's context attached.
                context: if self.context_dropped {
                    AiContext::new(
                        AiSurface::Global,
                        t!("ai.context.global").to_string(),
                        serde_json::json!({}),
                    )
                } else {
                    self.context.clone()
                },
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
            // Selecting an already-active thread is navigation, not a data
            // mutation. Persist only when this click genuinely restores an
            // archived conversation; serializing and renaming the complete
            // store on every history click used to block the GPUI thread.
            let restored = conversation.archived;
            conversation.archived = false;
            self.active_conversation = Some(id);
            if restored {
                self.persist_conversations();
            }
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

    fn quick_actions(surface: AiSurface) -> Vec<AiQuickAction> {
        let keys: &[(&str, &str, &str, AiQuickActionMode)] = match surface {
            AiSurface::Support => &[
                (
                    "ai.quick.support_reply",
                    "ai.prompt.support_reply",
                    "reply",
                    AiQuickActionMode::Submit,
                ),
                (
                    "ai.quick.summarize",
                    "ai.prompt.support_summary",
                    "scroll-text",
                    AiQuickActionMode::Submit,
                ),
                (
                    "ai.quick.triage",
                    "ai.prompt.support_triage_chat",
                    "flag",
                    AiQuickActionMode::Submit,
                ),
            ],
            AiSurface::Issue => &[
                (
                    "ai.quick.issue_draft",
                    "ai.prefill.issue_draft",
                    "plus",
                    AiQuickActionMode::Prefill,
                ),
                (
                    "ai.quick.tags",
                    "ai.prompt.issue_tags",
                    "tag",
                    AiQuickActionMode::Submit,
                ),
                (
                    "ai.quick.priority",
                    "ai.prompt.issue_priority",
                    "flag",
                    AiQuickActionMode::Submit,
                ),
            ],
            AiSurface::Script => &[
                (
                    "ai.quick.generate",
                    "ai.prefill.script_generate",
                    "sparkles",
                    AiQuickActionMode::Prefill,
                ),
                (
                    "ai.quick.explain",
                    "ai.prompt.script_explain",
                    "info",
                    AiQuickActionMode::Submit,
                ),
                (
                    "ai.quick.convert",
                    "ai.prefill.script_convert",
                    "arrow-left-right",
                    AiQuickActionMode::Prefill,
                ),
                (
                    "ai.quick.review",
                    "ai.prompt.script_review",
                    "shield-check",
                    AiQuickActionMode::Submit,
                ),
                (
                    "ai.quick.naming",
                    "ai.prompt.naming",
                    "pencil",
                    AiQuickActionMode::Submit,
                ),
            ],
            AiSurface::Terminal => &[
                (
                    "ai.quick.command",
                    "ai.prefill.terminal_command",
                    "terminal",
                    AiQuickActionMode::Prefill,
                ),
                (
                    "ai.quick.error",
                    "ai.prompt.terminal_error",
                    "circle-alert",
                    AiQuickActionMode::Submit,
                ),
                (
                    "ai.quick.issue_draft",
                    "ai.prefill.terminal_issue",
                    "plus",
                    AiQuickActionMode::Prefill,
                ),
            ],
            AiSurface::Jean => &[(
                "ai.quick.jean",
                "ai.prompt.jean",
                "send",
                AiQuickActionMode::Submit,
            )],
            AiSurface::Naming => &[(
                "ai.quick.naming",
                "ai.prompt.naming",
                "pencil",
                AiQuickActionMode::Submit,
            )],
            AiSurface::Recent => &[(
                "ai.quick.summarize",
                "ai.prompt.recent",
                "activity",
                AiQuickActionMode::Submit,
            )],
            AiSurface::Global | AiSurface::Clippy => &[],
        };
        keys.iter()
            .map(|(label, prompt, icon, mode)| AiQuickAction {
                label: t!(*label).to_string(),
                prompt_key: prompt,
                icon,
                mode: *mode,
            })
            .collect()
    }

    fn prefill_prompt(&mut self, prompt: String, window: &mut Window, cx: &mut Context<Self>) {
        if self.loading || !self.available {
            return;
        }
        self.prompt_state
            .update(cx, |state, cx| state.replace_content(prompt, cx));
        self.prompt_state.read(cx).focus_handle(cx).focus(window);
        cx.notify();
    }

    fn render_history(
        &self,
        as_column: bool,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        // The text element itself needs a definite pixel width for GPUI to
        // shape an ellipsis. A flex-only width is measured as max-content and
        // then hard-clipped by the row after the two fixed 36px buttons.
        let panel_width = if as_column {
            px(240.0).to_pixels(window.rem_size())
        } else {
            window.viewport_size().width - gpui::px(56.0)
        };
        // list padding 16 + row padding 16 + two 4px gaps, all scaled; then
        // the two adabraka Sm icon buttons, which stay absolute 36px each.
        let history_text_width =
            (panel_width - px(40.0).to_pixels(window.rem_size()) - gpui::px(72.0))
                .max(gpui::px(0.0));
        let mut list = div().flex().flex_col().gap(px(3.0)).p(px(8.0));
        let mut conversations = self
            .conversations
            .iter()
            .filter(|conversation| conversation.archived == self.show_archived)
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
            let display_title = conversation.display_title();
            let history_meta = format!("{} · {updated}", conversation.context_title);
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
                            .w(history_text_width)
                            .flex_shrink_0()
                            .min_w_0()
                            .overflow_hidden()
                            .cursor_pointer()
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.select_conversation(id, cx);
                            }))
                            .child(
                                div()
                                    .w(history_text_width)
                                    .min_w_0()
                                    .overflow_hidden()
                                    .whitespace_nowrap()
                                    .text_ellipsis()
                                    .text_size(px(12.0))
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(ShellDeckColors::text_primary())
                                    .child(display_title),
                            )
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(4.0))
                                    .w(history_text_width)
                                    .min_w_0()
                                    .overflow_hidden()
                                    .whitespace_nowrap()
                                    .text_ellipsis()
                                    .text_size(px(10.0))
                                    .text_color(ShellDeckColors::text_muted())
                                    .child(history_meta),
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

        // One chrome row, matching the conversation header's 44px: the rail
        // already names this panel, so the title does not need its own row and
        // the Actives / Archivées filter rides along instead of costing a second.
        let header = div()
            .flex()
            .items_center()
            .gap(px(6.0))
            .h(px(44.0))
            .px(px(12.0))
            .flex_shrink_0()
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
            )
            .child(div().flex_1().min_w(px(0.0)))
            .child(
                Button::new("ai-new-conversation", "")
                    .variant(ButtonVariant::Ghost)
                    .size(ButtonSize::Sm)
                    .tooltip(t!("ai.history.new").to_string())
                    .icon(IconSource::from("plus"))
                    .on_click(cx.listener(|this, _, _, cx| this.new_conversation(cx))),
            );

        // This used to be a 250px left column with a sidebar tint and a right
        // border. The rail now selects it as the whole panel, so it must fill —
        // otherwise it renders as a narrow strip with dead space beside it.
        // In the Sheet it is a 240px column beside the conversation; in the
        // Dock the rail selected it, so it *is* the panel and must fill.
        let mut shell = div().flex().flex_col().h_full().min_h(px(0.0));
        if as_column {
            shell = shell
                .w(px(240.0))
                .flex_shrink_0()
                .bg(ShellDeckColors::bg_sidebar())
                .border_r_1()
                .border_color(ShellDeckColors::border());
        } else {
            shell = shell.w_full().min_w(px(0.0));
        }
        shell
            .child(header)
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

    /// The activity rail: the panel's only navigation surface.
    ///
    /// Same contract as the Dev sidebar rail in `.agents/chrome.md` — it lists
    /// *activities*, not destinations: an entry earns a slot only because
    /// selecting it changes what the panel shows. "Nouveau" sits above the
    /// separator because it is an action, not a place; it never stays selected.
    fn render_rail(&self, active_tasks: usize, cx: &mut Context<Self>) -> AnyElement {
        let entries = [
            AiRailEntry {
                activity: AiActivity::Chat,
                icon: "messages-square",
                label: t!("ai.rail.chat").to_string(),
                badge: None,
            },
            AiRailEntry {
                activity: AiActivity::Clippy,
                // `paperclip` is not in the bundled Lucide subset
                // (.agents/icons.md) and silently painted nothing;
                // `clipboard-paste` ships and matches the clipboard job.
                icon: "clipboard-paste",
                label: t!("ai.clippy.tab").to_string(),
                badge: None,
            },
            AiRailEntry {
                activity: AiActivity::Tasks,
                icon: "list-checks",
                label: t!("ai.rail.tasks").to_string(),
                badge: (active_tasks > 0).then_some(active_tasks),
            },
            AiRailEntry {
                activity: AiActivity::History,
                icon: "clock",
                label: t!("ai.rail.history").to_string(),
                badge: None,
            },
        ];

        // The rail and its glyphs are absolute pixels on purpose, exactly like
        // the Dev sidebar rail: a rem-sized 17px glyph would outgrow a fixed
        // 56px rail at 2x and collide with its edges (.agents/spacing.md).
        let mut rail = div()
            .flex()
            .flex_col()
            .items_center()
            .flex_shrink_0()
            .w(gpui::px(56.0))
            .h_full()
            .gap(gpui::px(2.0))
            .pt(gpui::px(8.0))
            .pb(gpui::px(10.0))
            .bg(ShellDeckColors::bg_sidebar())
            .border_l_1()
            .border_color(ShellDeckColors::border());

        // "Nouveau" — outlined, not a filled tile: it is an action.
        rail = rail
            .child(
                div()
                    .id("ai-rail-new")
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(gpui::px(3.0))
                    .w_full()
                    .py(gpui::px(6.0))
                    .cursor_pointer()
                    .text_size(gpui::px(9.5))
                    .text_color(ShellDeckColors::text_muted())
                    .hover(|style| style.text_color(ShellDeckColors::text_primary()))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_center()
                            .w(gpui::px(30.0))
                            .h(gpui::px(26.0))
                            .rounded(gpui::px(7.0))
                            .border_1()
                            .border_color(ShellDeckColors::border())
                            .child(
                                // GPUI skips painting an svg whose own
                                // `style.text.color` is None — inheriting from
                                // the parent div is not enough. `icons.rs`
                                // documents this; the colour must sit here.
                                svg()
                                    .path(lucide_path("pencil"))
                                    .size(gpui::px(16.0))
                                    .text_color(ShellDeckColors::text_primary()),
                            ),
                    )
                    .child(t!("ai.rail.new").to_string())
                    .on_click(cx.listener(|this, _, _, cx| this.new_conversation(cx))),
            )
            .child(
                div()
                    .w(gpui::px(24.0))
                    .h(gpui::px(1.0))
                    .my(gpui::px(7.0))
                    .bg(ShellDeckColors::border()),
            );

        for entry in entries {
            let active = self.active_tab == entry.activity;
            let activity = entry.activity;
            let mut glyph = div()
                .relative()
                .flex()
                .items_center()
                .justify_center()
                .w(gpui::px(30.0))
                .h(gpui::px(26.0))
                .rounded(gpui::px(7.0))
                .child(
                    // Same rule as above: the colour goes on the svg node.
                    svg()
                        .path(lucide_path(entry.icon))
                        .size(gpui::px(17.0))
                        .text_color(if active {
                            ShellDeckColors::text_primary()
                        } else {
                            ShellDeckColors::text_muted()
                        }),
                );
            if active {
                // Neutral fill, not the accent: the accent is already spoken for
                // by the task badge and the send button, and three accents in a
                // 480px panel stop reading as a hierarchy.
                glyph = glyph.bg(ShellDeckColors::selected_bg());
            }
            if let Some(count) = entry.badge {
                glyph = glyph.child(
                    div()
                        .absolute()
                        .top(gpui::px(-2.0))
                        .right(gpui::px(-1.0))
                        .flex()
                        .items_center()
                        .justify_center()
                        .min_w(gpui::px(14.0))
                        .h(gpui::px(14.0))
                        .px(gpui::px(3.0))
                        .rounded_full()
                        .bg(ShellDeckColors::primary())
                        .text_size(gpui::px(9.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(gpui::white())
                        .child(count.to_string()),
                );
            }
            rail = rail.child(
                div()
                    .id(SharedString::from(format!("ai-rail-{:?}", entry.activity)))
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(gpui::px(3.0))
                    .w_full()
                    .py(gpui::px(6.0))
                    .cursor_pointer()
                    .text_size(gpui::px(9.5))
                    .text_color(if active {
                        ShellDeckColors::text_primary()
                    } else {
                        ShellDeckColors::text_muted()
                    })
                    .when(active, |el| el.font_weight(FontWeight::MEDIUM))
                    .hover(|style| style.text_color(ShellDeckColors::text_primary()))
                    .child(glyph)
                    .child(entry.label)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        // `history_open` is Sheet-only state; the rail (Dock)
                        // navigates purely on `active_tab`.
                        this.active_tab = activity;
                        cx.notify();
                    })),
            );
        }

        // The toolbox, pinned to the bottom. Icons only and tighter than the
        // activities above: these are tools you fetch, not places you inhabit.
        // It exists because the Dock is a chromeless window — it has neither
        // titlebar nor menu bar, so settings and help have nowhere else to live.
        rail = rail
            .child(div().flex_1().min_h(gpui::px(0.0)))
            .child(
                div()
                    .id("ai-rail-open-app")
                    .flex()
                    .items_center()
                    .justify_center()
                    .w(gpui::px(30.0))
                    .h(gpui::px(30.0))
                    .rounded(gpui::px(7.0))
                    .cursor_pointer()
                    .hover(|style| style.bg(ShellDeckColors::hover_bg()))
                    .child(
                        svg()
                            .path(lucide_path("external-link"))
                            .size(gpui::px(16.0))
                            .text_color(ShellDeckColors::text_muted()),
                    )
                    .on_click(cx.listener(|_, _, _, cx| {
                        cx.emit(AiAssistantEvent::OpenMainWindow);
                    })),
            )
            .child(
                div()
                    .id("ai-rail-palette")
                    .flex()
                    .items_center()
                    .justify_center()
                    .w(gpui::px(30.0))
                    .h(gpui::px(30.0))
                    .rounded(gpui::px(7.0))
                    .cursor_pointer()
                    .hover(|style| style.bg(ShellDeckColors::hover_bg()))
                    .child(
                        svg()
                            .path(lucide_path("search"))
                            .size(gpui::px(16.0))
                            .text_color(ShellDeckColors::text_muted()),
                    )
                    .on_click(cx.listener(|_, _, _, cx| {
                        cx.emit(AiAssistantEvent::OpenPalette);
                    })),
            )
            .child(
                div()
                    .id("ai-rail-help")
                    .flex()
                    .items_center()
                    .justify_center()
                    .w(gpui::px(30.0))
                    .h(gpui::px(30.0))
                    .rounded(gpui::px(7.0))
                    .cursor_pointer()
                    .hover(|style| style.bg(ShellDeckColors::hover_bg()))
                    .child(
                        svg()
                            .path(lucide_path("circle-help"))
                            .size(gpui::px(16.0))
                            .text_color(ShellDeckColors::text_muted()),
                    )
                    .on_click(|_, _, _| {
                        // Same destination as the Aide ▸ Documentation menu
                        // entry, which the Dock window has no menu bar to reach.
                        let _ = shelldeck_core::config::cloud_account::open_in_browser(
                            "https://shelldeck.1clic.pro",
                        );
                    }),
            )
            .child(
                div()
                    .id("ai-rail-settings")
                    .flex()
                    .items_center()
                    .justify_center()
                    .w(gpui::px(30.0))
                    .h(gpui::px(30.0))
                    .rounded(gpui::px(7.0))
                    .cursor_pointer()
                    .hover(|style| style.bg(ShellDeckColors::hover_bg()))
                    .child(
                        svg()
                            .path(lucide_path("settings"))
                            .size(gpui::px(16.0))
                            .text_color(ShellDeckColors::text_muted()),
                    )
                    .on_click(cx.listener(|_, _, _, cx| {
                        cx.emit(AiAssistantEvent::OpenSettings);
                    })),
            );

        if let Some(label) = self.account_label.clone() {
            let initial = label
                .chars()
                .next()
                .map(|c| c.to_uppercase().to_string())
                .unwrap_or_else(|| "?".to_string());
            rail = rail.child(
                div()
                    .id("ai-rail-account")
                    .relative()
                    .flex()
                    .items_center()
                    .justify_center()
                    .w(gpui::px(26.0))
                    .h(gpui::px(26.0))
                    .mt(gpui::px(7.0))
                    .cursor_pointer()
                    .rounded_full()
                    .bg(ShellDeckColors::primary())
                    .text_size(gpui::px(11.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(gpui::white())
                    .child(initial)
                    .child(
                        div()
                            .absolute()
                            .right(gpui::px(-1.0))
                            .bottom(gpui::px(-1.0))
                            .w(gpui::px(8.0))
                            .h(gpui::px(8.0))
                            .rounded_full()
                            .bg(ShellDeckColors::success()),
                    )
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.account_menu_open = !this.account_menu_open;
                        cx.notify();
                    })),
            );
        }

        rail.into_any_element()
    }

    fn render_messages(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        // The metadata line names the model, not the role — the role is already
        // carried by the position and the absence of a block.
        let meta_model = if self.model.trim().is_empty() {
            self.backend.display_name().to_string()
        } else {
            self.model.trim().to_string()
        };
        // At 780px a full-width line of prose runs past 110 characters, so the
        // Sheet gets a bounded, centred reading measure. The Dock is already
        // narrow enough to read straight.
        let mut thread = div()
            .flex()
            .flex_col()
            .w_full()
            .min_w(px(0.0))
            .gap(px(20.0))
            .px(px(18.0))
            .py(px(16.0));
        if self.host == AiHost::Sheet {
            thread = thread.max_w(px(600.0)).mx_auto();
        }
        // GPUI cannot derive wrapped height from `width: auto` plus a
        // percentage max-width in one measurement pass. Long/structured user
        // messages therefore receive the prototype's definite 88% width;
        // short one-line messages receive an explicit compact width.
        let reading_width = match self.host {
            AiHost::Dock => window.viewport_size().width - gpui::px(56.0),
            AiHost::Sheet => sheet_message_reading_width(
                window.viewport_size().width,
                self.history_open,
                px(240.0).to_pixels(window.rem_size()),
                px(600.0).to_pixels(window.rem_size()),
            ),
        };
        let user_bubble_width =
            (reading_width - px(36.0).to_pixels(window.rem_size())).max(gpui::px(0.0)) * 0.88;
        if let Some(conversation) = self.active_conversation() {
            for message in &conversation.messages {
                let is_user = message.role == AiChatRole::User;
                let message_id = message.id;
                let content = message.content.clone();

                if is_user {
                    // The user's turn is the only one that carries a block, and
                    // the only one aligned right — position is what replaces the
                    // "Vous" label. The block must NOT be `w_full`: an unbounded
                    // flex child fills the row and silently cancels
                    // `justify_end`, which is exactly why the previous version
                    // never aligned anything despite asking for it.
                    let markdown = Markdown::new(content.clone())
                        .base_font_size(px(12.5).to_pixels(window.rem_size()))
                        // Conversation blocks follow the prototype's compact
                        // rhythm and leave no document-style tail margin.
                        .compact()
                        .min_w_0()
                        .whitespace_normal();
                    let structured = content.lines().any(|line| {
                        let line = line.trim_start();
                        line.starts_with("# ")
                            || line.starts_with("## ")
                            || line.starts_with("> ")
                            || line.starts_with("- ")
                            || line.starts_with("* ")
                            || line.starts_with("```")
                            || line.starts_with('|')
                    });
                    let longest_line = content
                        .lines()
                        .map(|line| line.chars().count())
                        .max()
                        .unwrap_or(0) as f32;
                    // A definite width is mandatory for correct wrapped height
                    // in GPUI. Estimate only the compact one-line width; any
                    // overflow still wraps safely because layout now knows the
                    // width before shaping. Structured Markdown uses the full
                    // cap so lists/tables/code never collapse to min-content.
                    let compact_text_width = if longest_line > 0.0 {
                        (longest_line * 6.5).max(18.0)
                    } else {
                        0.0
                    };
                    let compact_width = px(compact_text_width + 24.0)
                        .to_pixels(window.rem_size())
                        .min(user_bubble_width);
                    let bubble_width = if structured {
                        user_bubble_width
                    } else {
                        compact_width
                    };
                    let bubble = div()
                        .id(SharedString::from(format!("ai-message-{message_id}")))
                        .flex()
                        .flex_col()
                        .ml_auto()
                        .w(bubble_width)
                        .min_w(px(0.0))
                        .overflow_hidden()
                        .px(px(12.0))
                        .py(px(9.0))
                        .rounded(px(12.0))
                        .rounded_br(px(3.0))
                        .bg(ShellDeckColors::bg_surface())
                        .text_color(ShellDeckColors::text_primary())
                        .child(markdown);
                    thread = thread.child(bubble);
                } else {
                    // The assistant answers in prose on the surface: no frame, no
                    // fill, no role label. Its metadata sits underneath, quiet —
                    // not as a permanent button in a card header.
                    let markdown = Markdown::new(content.clone())
                        .base_font_size(px(12.5).to_pixels(window.rem_size()))
                        .compact()
                        .w_full()
                        .min_w_0()
                        .whitespace_normal();
                    thread = thread.child(
                        div()
                            .id(SharedString::from(format!("ai-message-{message_id}")))
                            .flex()
                            .flex_col()
                            .w_full()
                            .min_w(px(0.0))
                            .text_color(ShellDeckColors::text_primary())
                            .child(markdown)
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(8.0))
                                    .mt(px(7.0))
                                    .text_size(px(10.0))
                                    .text_color(ShellDeckColors::text_muted())
                                    .child(
                                        // adabraka sizes its buttons in absolute
                                        // pixels, so `IconButton::size` takes
                                        // `gpui::px` — the rem-based `px` shadow
                                        // in this file would not type-check.
                                        IconButton::new(IconSource::from("copy"))
                                            .variant(ButtonVariant::Ghost)
                                            .size(gpui::px(20.0))
                                            .icon_size(gpui::px(12.0))
                                            .on_click(move |_, _, cx| {
                                                cx.write_to_clipboard(ClipboardItem::new_string(
                                                    content.clone(),
                                                ));
                                            }),
                                    )
                                    .child(meta_model.clone()),
                            ),
                    );
                }
            }
        }
        if self.loading {
            // The thinking indicator is not a message either: same prose column,
            // no frame around it.
            thread = thread.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .text_size(px(12.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(animated_monolith(
                        "ai-assistant-thinking",
                        32.0,
                        MonolithMotion::Thinking,
                    ))
                    .child(animated_loading_text(
                        "ai-assistant-thinking-text",
                        t!("ai.assistant.generating").to_string(),
                    )),
            );
        }
        let _ = cx;
        thread.into_any_element()
    }

    fn capability_label(capability: AiCapability) -> String {
        let key = match capability {
            AiCapability::Naming => "ai.tasks.capability.naming",
            AiCapability::JeanDispatch => "ai.tasks.capability.jean_dispatch",
            AiCapability::FleetDispatch => "ai.tasks.capability.fleet_dispatch",
            AiCapability::SupportReply => "ai.tasks.capability.support_reply",
            AiCapability::SupportSummary => "ai.tasks.capability.support_summary",
            AiCapability::SupportTriage => "ai.tasks.capability.support_triage",
            AiCapability::IssueReply => "ai.tasks.capability.issue_reply",
            AiCapability::IssueSummary => "ai.tasks.capability.issue_summary",
            AiCapability::IssueTriage => "ai.tasks.capability.issue_triage",
            AiCapability::IssueCompose => "ai.tasks.capability.issue_compose",
            AiCapability::ScriptGenerate => "ai.tasks.capability.script_generate",
            AiCapability::ScriptExplain => "ai.tasks.capability.script_explain",
            AiCapability::ScriptReview => "ai.tasks.capability.script_review",
            AiCapability::ScriptFix => "ai.tasks.capability.script_fix",
            AiCapability::TerminalCommand => "ai.tasks.capability.terminal_command",
            AiCapability::TerminalDiagnose => "ai.tasks.capability.terminal_diagnose",
            AiCapability::ClippyTransform => "ai.tasks.capability.clippy_transform",
            AiCapability::ClippyExplain => "ai.tasks.capability.clippy_explain",
            AiCapability::ClippyReplaceSelection => "ai.tasks.capability.clippy_replace",
        };
        t!(key).to_string()
    }

    fn status_label(status: AiTaskStatus) -> String {
        let key = match status {
            AiTaskStatus::Generating => "ai.tasks.status.generating",
            AiTaskStatus::Ready => "ai.tasks.status.ready",
            AiTaskStatus::Pending => "ai.tasks.status.pending",
            AiTaskStatus::AwaitingConfirmation => "ai.tasks.status.awaiting_confirmation",
            AiTaskStatus::Executing => "ai.tasks.status.executing",
            AiTaskStatus::Applied => "ai.tasks.status.applied",
            AiTaskStatus::Succeeded => "ai.tasks.status.succeeded",
            AiTaskStatus::Failed => "ai.tasks.status.failed",
            AiTaskStatus::Cancelled => "ai.tasks.status.cancelled",
        };
        t!(key).to_string()
    }

    fn status_color(status: AiTaskStatus) -> Hsla {
        match status {
            AiTaskStatus::Ready | AiTaskStatus::Pending | AiTaskStatus::AwaitingConfirmation => {
                ShellDeckColors::warning()
            }
            AiTaskStatus::Generating | AiTaskStatus::Executing => ShellDeckColors::primary(),
            AiTaskStatus::Applied | AiTaskStatus::Succeeded => ShellDeckColors::success(),
            AiTaskStatus::Failed => ShellDeckColors::error(),
            AiTaskStatus::Cancelled => ShellDeckColors::text_muted(),
        }
    }

    fn render_task(&self, task: &AiTask, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let task_id = task.id;
        let target = if task.target_label.trim().is_empty() {
            task.target_id.clone()
        } else {
            task.target_label.clone()
        };
        let status_color = Self::status_color(task.status);
        let can_resume = task.status == AiTaskStatus::Failed
            || (matches!(task.status, AiTaskStatus::Ready | AiTaskStatus::Pending)
                && !task.result.trim().is_empty());
        let can_stop = task.status == AiTaskStatus::Executing
            && matches!(
                task.capability,
                AiCapability::TerminalCommand
                    | AiCapability::ScriptGenerate
                    | AiCapability::ScriptFix
            );
        let can_delete = !task.status.is_active();
        let can_open_target = match task.surface {
            AiSurface::Support
            | AiSurface::Issue
            | AiSurface::Script
            | AiSurface::Terminal
            | AiSurface::Jean => true,
            AiSurface::Naming => task.target_kind.as_deref() == Some("naming_terminal"),
            AiSurface::Recent | AiSurface::Global | AiSurface::Clippy => false,
        };
        let status_detail = task
            .status_message
            .as_ref()
            .filter(|message| !message.trim().is_empty())
            .cloned();
        let result_detail =
            (!task.result.trim().is_empty()).then(|| task.result.trim().to_string());
        let detail_is_markdown =
            status_detail.is_none() && capability_result_is_markdown(task.capability);
        let detail = status_detail.or(result_detail);

        div()
            .id(SharedString::from(format!("ai-task-card-{task_id}")))
            .flex()
            .flex_col()
            .gap(px(9.0))
            .min_w_0()
            .p(px(12.0))
            .border_b_1()
            .border_color(ShellDeckColors::border())
            .child(
                div()
                    .flex()
                    .items_start()
                    .justify_between()
                    .gap(px(10.0))
                    .min_w_0()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(3.0))
                            .flex_1()
                            .min_w_0()
                            .child(
                                div()
                                    .truncate()
                                    .text_size(px(13.0))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(ShellDeckColors::text_primary())
                                    .child(Self::capability_label(task.capability)),
                            )
                            .child(
                                div()
                                    .truncate()
                                    .text_size(px(11.0))
                                    .text_color(ShellDeckColors::text_muted())
                                    .child(target),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.0))
                            .flex_shrink_0()
                            .when(task.status.is_active(), |row| {
                                row.child(
                                    Spinner::new()
                                        .size(SpinnerSize::Xs)
                                        .variant(SpinnerVariant::Primary),
                                )
                            })
                            .child(
                                Badge::new(Self::status_label(task.status))
                                    .variant(BadgeVariant::Outline)
                                    .border_color(status_color)
                                    .text_color(status_color),
                            ),
                    ),
            )
            .when_some(detail, |row, detail| {
                if detail_is_markdown {
                    row.child(
                        div()
                            .w_full()
                            .min_w_0()
                            .max_h(px(96.0))
                            .overflow_hidden()
                            .text_color(ShellDeckColors::text_muted())
                            .child(
                                Markdown::new(detail)
                                    .base_font_size(px(11.0).to_pixels(window.rem_size()))
                                    .w_full()
                                    .min_w_0()
                                    .whitespace_normal(),
                            ),
                    )
                } else {
                    row.child(
                        div()
                            .line_clamp(3)
                            .overflow_hidden()
                            .text_size(px(11.0))
                            .text_color(ShellDeckColors::text_muted())
                            .child(detail),
                    )
                }
            })
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(px(8.0))
                    .child(
                        div()
                            .text_size(px(10.0))
                            .text_color(ShellDeckColors::text_muted())
                            .child(crate::i18n::rel_time(
                                task.updated_at.timestamp_millis() as f64
                            )),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.0))
                            .when(can_open_target, |actions| {
                                actions.child(
                                    Button::new(
                                        SharedString::from(format!("ai-task-target-{task_id}")),
                                        "",
                                    )
                                    .variant(ButtonVariant::Ghost)
                                    .size(ButtonSize::Sm)
                                    .tooltip(t!("ai.tasks.open_target").to_string())
                                    .icon(IconSource::from("external-link"))
                                    .on_click(cx.listener(
                                        move |_this, _, _, cx| {
                                            cx.emit(AiAssistantEvent::OpenTaskTarget(task_id));
                                        },
                                    )),
                                )
                            })
                            .when(can_resume, |actions| {
                                actions.child(
                                    Button::new(
                                        SharedString::from(format!("ai-task-resume-{task_id}")),
                                        t!("ai.tasks.resume").to_string(),
                                    )
                                    .variant(ButtonVariant::Secondary)
                                    .size(ButtonSize::Sm)
                                    .icon(IconSource::from("rotate-ccw"))
                                    .on_click(cx.listener(
                                        move |_this, _, _, cx| {
                                            cx.emit(AiAssistantEvent::ResumeTask(task_id));
                                        },
                                    )),
                                )
                            })
                            .when(can_stop, |actions| {
                                actions.child(
                                    Button::new(
                                        SharedString::from(format!("ai-task-stop-{task_id}")),
                                        t!("ai.tasks.stop").to_string(),
                                    )
                                    .variant(ButtonVariant::Destructive)
                                    .size(ButtonSize::Sm)
                                    .icon(IconSource::from("square"))
                                    .on_click(cx.listener(
                                        move |_this, _, _, cx| {
                                            cx.emit(AiAssistantEvent::StopTask(task_id));
                                        },
                                    )),
                                )
                            })
                            .when(can_delete, |actions| {
                                actions.child(
                                    Button::new(
                                        SharedString::from(format!("ai-task-delete-{task_id}")),
                                        "",
                                    )
                                    .variant(ButtonVariant::Ghost)
                                    .size(ButtonSize::Sm)
                                    .tooltip(t!("ai.tasks.delete").to_string())
                                    .icon(IconSource::from("trash-2"))
                                    .on_click(cx.listener(
                                        move |_this, _, _, cx| {
                                            cx.emit(AiAssistantEvent::DeleteTask(task_id));
                                        },
                                    )),
                                )
                            }),
                    ),
            )
            .into_any_element()
    }

    fn render_tasks(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let mut tasks = self.tasks.clone();
        tasks.sort_by_key(|task| {
            (
                !(task.status.is_active()
                    || matches!(task.status, AiTaskStatus::Ready | AiTaskStatus::Pending)),
                std::cmp::Reverse(task.updated_at),
            )
        });
        if tasks.is_empty() {
            return div()
                .flex()
                .items_center()
                .justify_center()
                .size_full()
                .child(
                    EmptyState::new("ai-tasks-empty", t!("ai.tasks.empty_title").to_string())
                        .icon(IconSource::from("list-checks"))
                        .description(t!("ai.tasks.empty_description").to_string())
                        .size(EmptyStateSize::Sm),
                )
                .into_any_element();
        }

        let mut list = div().flex().flex_col();
        for task in &tasks {
            list = list.child(self.render_task(task, window, cx));
        }
        div()
            .id("ai-task-scroll")
            .flex_1()
            .w_full()
            .min_w_0()
            .min_h(px(0.0))
            .overflow_y_scroll()
            .track_scroll(&self.task_scroll)
            .child(list)
            .into_any_element()
    }

    fn render_clippy(&self, cx: &mut Context<Self>) -> AnyElement {
        let source = self.clippy_source_state.read(cx).content().to_string();
        let source_len = source.chars().count();
        let source_input = Input::new(&self.clippy_source_state)
            .size(InputSize::Sm)
            .multi_line(true)
            .min_rows(6)
            .max_rows(12)
            .placeholder(t!("ai.clippy.source_placeholder").to_string())
            .disabled(self.loading || !self.clippy_available);
        let instruction_input = Input::new(&self.clippy_instruction_state)
            .size(InputSize::Sm)
            .placeholder(t!("ai.clippy.instruction_placeholder").to_string())
            .disabled(self.loading || !self.clippy_available);

        let mut operations = div().flex().flex_wrap().gap(px(7.0));
        for operation in ClippyOperationKind::all() {
            let operation = *operation;
            operations = operations.child(
                Button::new(
                    SharedString::from(format!("clippy-op-{}", operation.key())),
                    operation.label(),
                )
                .variant(if self.clippy_operation == operation {
                    ButtonVariant::Secondary
                } else {
                    ButtonVariant::Outline
                })
                .size(ButtonSize::Sm)
                .icon(IconSource::from(operation.icon()))
                .disabled(self.loading || !self.clippy_available)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.clippy_operation = operation;
                    cx.notify();
                })),
            );
        }

        let header = div()
            .flex()
            .items_start()
            .justify_between()
            .gap(px(12.0))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .gap(px(3.0))
                    .child(
                        div()
                            .text_size(px(16.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(ShellDeckColors::text_primary())
                            .child(t!("ai.clippy.title").to_string()),
                    )
                    .child(
                        div()
                            .text_size(px(12.0))
                            .text_color(ShellDeckColors::text_muted())
                            .child(t!("ai.clippy.description").to_string()),
                    ),
            )
            .child(
                Button::new("clippy-import", t!("ai.clippy.use_clipboard").to_string())
                    .variant(ButtonVariant::Secondary)
                    .size(ButtonSize::Sm)
                    .icon(IconSource::from("clipboard-paste"))
                    .disabled(self.loading || !self.clippy_available)
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.import_clippy_clipboard(window, cx);
                    })),
            );

        let mut result_card = div().flex().flex_col().gap(px(8.0));
        if self.loading && self.clippy_pending_request.is_some() {
            result_card = result_card.child(
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
                    .child(t!("ai.clippy.generating").to_string()),
            );
        }
        if let Some(error) = &self.clippy_error {
            result_card = result_card.child(
                div()
                    .rounded(px(7.0))
                    .px(px(10.0))
                    .py(px(8.0))
                    .bg(ShellDeckColors::error().opacity(0.10))
                    .text_size(px(12.0))
                    .text_color(ShellDeckColors::error())
                    .child(error.clone()),
            );
        }
        if let Some(result) = &self.clippy_result {
            let result_for_copy = result.clone();
            let diff = ai_line_diff(&source, result);
            let diff_rows = diff.into_iter().map(|line| {
                let (prefix, text, color) = match line {
                    AiDiffLine::Added(text) => ("+ ", text, ShellDeckColors::success()),
                    AiDiffLine::Removed(text) => ("- ", text, ShellDeckColors::error()),
                    AiDiffLine::Context(text) => ("  ", text, ShellDeckColors::text_muted()),
                };
                div()
                    .font_family("monospace")
                    .text_size(px(11.0))
                    .text_color(color)
                    .child(format!("{prefix}{text}"))
            });
            result_card = result_card
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap(px(8.0))
                        .child(
                            div()
                                .text_size(px(12.0))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(ShellDeckColors::text_primary())
                                .child(t!("ai.clippy.result").to_string()),
                        )
                        .child(
                            div()
                                .flex()
                                .gap(px(6.0))
                                .child(
                                    Button::new("clippy-copy", t!("ai.clippy.copy").to_string())
                                        .variant(ButtonVariant::Secondary)
                                        .size(ButtonSize::Sm)
                                        .icon(IconSource::from("copy"))
                                        .on_click(move |_, _, cx| {
                                            cx.write_to_clipboard(ClipboardItem::new_string(
                                                result_for_copy.clone(),
                                            ));
                                        }),
                                )
                                .child(
                                    Button::new("clippy-edit", t!("ai.clippy.edit").to_string())
                                        .variant(ButtonVariant::Ghost)
                                        .size(ButtonSize::Sm)
                                        .icon(IconSource::from("pencil"))
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.edit_clippy_result(cx)
                                        })),
                                )
                                .child(
                                    Button::new(
                                        "clippy-regenerate",
                                        t!("ai.clippy.regenerate").to_string(),
                                    )
                                    .variant(ButtonVariant::Ghost)
                                    .size(ButtonSize::Sm)
                                    .icon(IconSource::from("rotate-ccw"))
                                    .on_click(cx.listener(|this, _, _, cx| this.run_clippy(cx))),
                                ),
                        ),
                )
                .child(
                    div()
                        .rounded(px(7.0))
                        .border_1()
                        .border_color(ShellDeckColors::border())
                        .bg(ShellDeckColors::bg_surface())
                        .p(px(10.0))
                        .text_size(px(12.0))
                        .text_color(ShellDeckColors::text_primary())
                        .child(result.clone()),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(2.0))
                        .rounded(px(7.0))
                        .border_1()
                        .border_color(ShellDeckColors::border())
                        .bg(ShellDeckColors::bg_primary())
                        .p(px(10.0))
                        .child(
                            div()
                                .mb(px(4.0))
                                .text_size(px(11.0))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(ShellDeckColors::text_muted())
                                .child(t!("ai.clippy.diff").to_string()),
                        )
                        .children(diff_rows),
                );
        }

        let content = div()
            .flex()
            .flex_col()
            .gap(px(14.0))
            .p(px(16.0))
            .size_full()
            .child(header)
            .child(operations)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(6.0))
                    .child(
                        div()
                            .text_size(px(11.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(ShellDeckColors::text_muted())
                            .child(t!("ai.clippy.source").to_string()),
                    )
                    .child(source_input)
                    .child(
                        div()
                            .text_size(px(10.0))
                            .text_color(ShellDeckColors::text_muted())
                            .child(t!("ai.clippy.source_count", count = source_len).to_string()),
                    ),
            )
            .child(instruction_input)
            .child(
                div()
                    .flex()
                    .gap(px(8.0))
                    .child(
                        Button::new("clippy-generate", t!("ai.clippy.generate").to_string())
                            .variant(ButtonVariant::Ai)
                            .size(ButtonSize::Sm)
                            .icon(IconSource::from("sparkles"))
                            .disabled(self.loading || !self.clippy_available)
                            .on_click(cx.listener(|this, _, _, cx| this.run_clippy(cx))),
                    )
                    .when(
                        self.loading && self.clippy_pending_request.is_some(),
                        |row| {
                            row.child(
                                Button::new("clippy-cancel", t!("ai.clippy.cancel").to_string())
                                    .variant(ButtonVariant::Ghost)
                                    .size(ButtonSize::Sm)
                                    .icon(IconSource::from("x"))
                                    .on_click(cx.listener(|this, _, _, cx| this.cancel_clippy(cx))),
                            )
                        },
                    ),
            )
            .child(result_card);
        scrollable_vertical(content).into_any_element()
    }
}

impl Render for AiAssistantView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // One closure for the round button and for Enter — `Composer` takes a
        // single handler precisely so the two cannot drift apart.
        let submit = {
            let entity = cx.entity();
            move |cx: &mut App| {
                entity.update(cx, |this, cx| this.submit(cx));
            }
        };

        // The quick actions become a two-column tile grid instead of a wrapping
        // button row: at 424px (the dock minus its rail) a wrapped row of
        // adabraka buttons packs unevenly, while a grid keeps the block square.
        let disabled = self.loading || !self.available;
        let mut tiles = div().flex().flex_col().w_full().min_w(px(0.0)).gap(px(8.0));
        let actions = Self::quick_actions(self.context.surface);
        for (row_index, pair) in actions.chunks(2).enumerate() {
            let mut row = div().flex().w_full().min_w(px(0.0)).gap(px(8.0));
            for (column_index, action) in pair.iter().enumerate() {
                let index = row_index * 2 + column_index;
                let quick_prompt = t!(action.prompt_key).to_string();
                let mode = action.mode;
                row = row.child(
                    div()
                        .id(("ai-quick", index))
                        .flex()
                        .flex_1()
                        .min_w(px(0.0))
                        .items_center()
                        .gap(px(9.0))
                        .px(px(12.0))
                        .py(px(11.0))
                        .rounded(px(9.0))
                        .bg(ShellDeckColors::bg_surface())
                        .text_size(px(12.5))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(ShellDeckColors::text_primary())
                        .when(!disabled, |tile| {
                            tile.cursor_pointer()
                                .hover(|style| style.bg(ShellDeckColors::selected_bg()))
                        })
                        .when(disabled, |tile| tile.opacity(0.5))
                        .child(div().flex_shrink_0().child(lucide_icon(
                            action.icon,
                            15.0,
                            ShellDeckColors::primary(),
                        )))
                        .child(div().flex_1().min_w(px(0.0)).child(action.label.clone()))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            if this.loading || !this.available {
                                return;
                            }
                            match mode {
                                AiQuickActionMode::Submit => {
                                    this.submit_prompt(quick_prompt.clone(), cx);
                                }
                                AiQuickActionMode::Prefill => {
                                    this.prefill_prompt(quick_prompt.clone(), window, cx);
                                }
                            }
                        })),
                );
            }
            // An odd action count would leave the last tile double-width; a
            // spacer keeps the grid honest.
            if pair.len() == 1 {
                row = row.child(div().flex_1().min_w(px(0.0)));
            }
            tiles = tiles.child(row);
        }

        let model = if self.model.trim().is_empty() {
            self.backend.default_model().to_string()
        } else {
            self.model.clone()
        };

        let active_tasks = self
            .tasks
            .iter()
            .filter(|task| {
                task.status.is_active()
                    || matches!(task.status, AiTaskStatus::Ready | AiTaskStatus::Pending)
            })
            .count();
        let active_title = self.active_title();
        // The single chrome row. In the Dock it belongs to the 424px content
        // column, not to the 480px window: the 56px rail is its full-height
        // sibling. In the Sheet the adabraka `Sheet` draws no header of its own
        // (no title, no close button => `has_header` is false), so this row is
        // the whole chrome and carries the close.
        let in_sheet = self.host == AiHost::Sheet;
        let mut conversation_header = div()
            .flex()
            .items_center()
            .gap(px(6.0))
            .h(px(44.0))
            .px(px(10.0))
            .flex_shrink_0()
            .border_b_1()
            .border_color(ShellDeckColors::border());
        if in_sheet {
            // This opaque row reaches the native top-right corner. Like the
            // User-home hero artwork, it must own the curve itself because a
            // GPUI parent clip alone can leave a rectangular paint remnant.
            conversation_header = conversation_header
                .rounded_tr(use_theme().tokens.radius_xl)
                .overflow_hidden()
                .bg(ShellDeckColors::bg_primary());
            conversation_header = conversation_header.child(
                Button::new("ai-toggle-history", "")
                    .variant(ButtonVariant::Ghost)
                    .size(ButtonSize::Sm)
                    .tooltip(if self.history_open {
                        t!("ai.history.hide").to_string()
                    } else {
                        t!("ai.history.show").to_string()
                    })
                    // `panel-left` is not in the bundled Lucide subset
                    // (.agents/icons.md): reuse the two slugs the previous
                    // toggle already shipped with rather than add a file.
                    .icon(IconSource::from(if self.history_open {
                        "chevron-left"
                    } else {
                        "clock"
                    }))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.history_open = !this.history_open;
                        cx.notify();
                    })),
            );
        }
        conversation_header = conversation_header.child(
            div().flex_1().min_w_0().overflow_hidden().child(
                div()
                    .w_full()
                    .min_w_0()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .text_size(px(13.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(ShellDeckColors::text_primary())
                    .child(active_title),
            ),
        );
        if in_sheet {
            // Without a rail, the Sheet needs its own way into — and back out
            // of — the Clippy activity. Same pill pattern as the tasks pill
            // below (.agents/ui-components.md harmonization). Hidden when the
            // surface is disallowed (`ai.surfaces.clippy`), except while
            // already showing Clippy so the route back to Chat never vanishes.
            if self.clippy_available || self.active_tab == AiActivity::Clippy {
                let showing_clippy = self.active_tab == AiActivity::Clippy;
                conversation_header = conversation_header.child(
                    div()
                        .id("ai-clippy-pill")
                        .flex()
                        .flex_shrink_0()
                        .items_center()
                        .gap(px(5.0))
                        .h(px(22.0))
                        .px(px(8.0))
                        .rounded_full()
                        .cursor_pointer()
                        .bg(if showing_clippy {
                            ShellDeckColors::selected_bg()
                        } else {
                            ShellDeckColors::bg_surface()
                        })
                        .text_size(px(11.0))
                        .text_color(ShellDeckColors::text_muted())
                        .child(lucide_icon(
                            "clipboard-paste",
                            12.0,
                            ShellDeckColors::text_muted(),
                        ))
                        .child(t!("ai.clippy.tab").to_string())
                        .on_click(cx.listener(|this, _, _, cx| {
                            if this.active_tab == AiActivity::Clippy {
                                this.active_tab = AiActivity::Chat;
                                cx.notify();
                            } else {
                                this.show_clippy(cx);
                            }
                        })),
                );
            }
            // Without a rail, the Sheet needs its own way into the task list.
            if active_tasks > 0 || self.active_tab == AiActivity::Tasks {
                let showing_tasks = self.active_tab == AiActivity::Tasks;
                conversation_header = conversation_header.child(
                    div()
                        .id("ai-tasks-pill")
                        .flex()
                        .flex_shrink_0()
                        .items_center()
                        .gap(px(5.0))
                        .h(px(22.0))
                        .px(px(8.0))
                        .rounded_full()
                        .cursor_pointer()
                        .bg(if showing_tasks {
                            ShellDeckColors::selected_bg()
                        } else {
                            ShellDeckColors::bg_surface()
                        })
                        .text_size(px(11.0))
                        .text_color(ShellDeckColors::text_muted())
                        .child(lucide_icon(
                            "list-checks",
                            12.0,
                            ShellDeckColors::text_muted(),
                        ))
                        .child(t!("ai.rail.tasks").to_string())
                        .when(active_tasks > 0, |pill| {
                            pill.child(
                                div()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(ShellDeckColors::text_primary())
                                    .child(active_tasks.to_string()),
                            )
                        })
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.active_tab = if this.active_tab == AiActivity::Tasks {
                                AiActivity::Chat
                            } else {
                                AiActivity::Tasks
                            };
                            cx.notify();
                        })),
                );
            }
            conversation_header = conversation_header
                .child(
                    div()
                        .w(px(1.0))
                        .h(px(18.0))
                        .flex_shrink_0()
                        .bg(ShellDeckColors::border()),
                )
                .child(
                    Button::new("ai-sheet-close", "")
                        .variant(ButtonVariant::Ghost)
                        .size(ButtonSize::Sm)
                        .tooltip(t!("ai.dock.hide").to_string())
                        .icon(IconSource::from("x"))
                        .on_click(cx.listener(|_, _, _, cx| cx.emit(AiAssistantEvent::Close))),
                );
        } else {
            // The prototype leaves only the thread title and hide action in
            // the Dock header. "Ouvrir ShellDeck" lives once in the rail's
            // bottom toolbox; duplicating it here shifts the rail down and
            // breaks the two-column hierarchy.
            conversation_header = conversation_header
                // GPUI does not propagate rounded clipping through opaque
                // children, so the surface touching the window corner owns it.
                .rounded_tl(use_theme().tokens.radius_xl)
                .bg(ShellDeckColors::bg_primary())
                .child(
                    Button::new("ai-dock-hide", "")
                        .variant(ButtonVariant::Ghost)
                        .size(ButtonSize::Sm)
                        .tooltip(t!("ai.dock.hide").to_string())
                        .icon(IconSource::from("x"))
                        .on_click(|_, window, _| window.remove_window()),
                );
        }

        let has_messages = self
            .active_conversation()
            .is_some_and(|conversation| !conversation.messages.is_empty());
        // Recent threads are real data, so the "start from here" list shows
        // them rather than invented sample prompts.
        let recent: Vec<(Uuid, String)> = self
            .conversations
            .iter()
            .rev()
            .filter(|conversation| !conversation.archived && !conversation.messages.is_empty())
            .take(3)
            .map(|conversation| (conversation.id, conversation.display_title()))
            .collect();

        let conversation_body = if has_messages {
            self.render_messages(window, cx)
        } else {
            let mut hero = div()
                .flex()
                .flex_col()
                .w_full()
                .min_w(px(0.0))
                .when(self.host == AiHost::Sheet, |el| {
                    el.max_w(px(600.0)).mx_auto()
                })
                .gap(px(22.0))
                .px(px(18.0))
                .pt(px(28.0))
                .pb(px(16.0))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(6.0))
                        .child(
                            // Two SHORT lines carry the hero. The long
                            // "only uses the current screen" sentence belongs
                            // underneath at body size, not set in 21px display.
                            div()
                                .flex()
                                .flex_col()
                                .child(
                                    div()
                                        .text_size(px(21.0))
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(ShellDeckColors::text_primary())
                                        .child(t!("ai.assistant.hero.greeting").to_string()),
                                )
                                .child(
                                    div()
                                        .text_size(px(21.0))
                                        .text_color(ShellDeckColors::text_muted())
                                        .child(t!("ai.assistant.hero.title").to_string()),
                                ),
                        )
                        .child(
                            div()
                                .text_size(px(11.5))
                                .text_color(ShellDeckColors::text_muted())
                                .child(t!("ai.assistant.welcome_description").to_string()),
                        ),
                )
                .child(tiles);

            if !recent.is_empty() {
                let mut list = div().flex().flex_col().w_full().min_w(px(0.0)).gap(px(6.0));
                for (index, (id, title)) in recent.into_iter().enumerate() {
                    list = list.child(
                        div()
                            .id(("ai-recent", index))
                            .w_full()
                            .min_w(px(0.0))
                            .px(px(11.0))
                            .py(px(9.0))
                            .rounded(px(8.0))
                            .border_1()
                            .border_color(ShellDeckColors::border())
                            .cursor_pointer()
                            .flex()
                            .items_center()
                            .hover(|style| style.bg(ShellDeckColors::bg_surface()))
                            .child(
                                // `truncate()` alone collapses to the ellipsis:
                                // a nowrap child needs an explicit share of the
                                // row *and* permission to shrink, per
                                // `.agents/overflow.md`.
                                div()
                                    .flex_1()
                                    .min_w(px(0.0))
                                    .truncate()
                                    .text_size(px(12.5))
                                    .text_color(ShellDeckColors::text_primary())
                                    .child(title),
                            )
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.active_conversation = Some(id);
                                cx.notify();
                            })),
                    );
                }
                hero = hero.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(7.0))
                        .child(
                            div()
                                .text_size(px(11.0))
                                .text_color(ShellDeckColors::text_muted())
                                .child(t!("ai.assistant.hero.recent").to_string()),
                        )
                        .child(list),
                );
            }
            hero.into_any_element()
        };

        // No top border and no `bg_surface` band: the composer is an object
        // sitting on the panel, not a docked strip. The frame it draws is the
        // only frame — see `Composer` (SDPATCH-028).
        let mut composer = div()
            .flex()
            .flex_col()
            .flex_shrink_0()
            .gap(px(8.0))
            .px(px(12.0))
            .pt(px(8.0))
            .pb(px(12.0))
            // Opaque: without it the scrolled thread shows through the padding
            // around the frame and reads as if it ran under the composer.
            .bg(ShellDeckColors::bg_primary());
        // The Dock composer directly owns its exposed bottom-left corner.
        // In a Sheet, the panel/body already own and clip the bottom-right
        // curve. Giving the full-width composer a second `rounded_br` cuts a
        // larger inner arc and reveals the dim backdrop as a grey triangle.
        if self.host == AiHost::Dock {
            composer = composer.rounded_bl(use_theme().tokens.radius_xl);
        }
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
        if let Some(notice) = self.notice.clone() {
            composer = composer.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .px(px(10.0))
                    .py(px(8.0))
                    .rounded(px(6.0))
                    .bg(ShellDeckColors::primary().opacity(0.10))
                    .text_size(px(12.0))
                    .text_color(ShellDeckColors::text_primary())
                    .child(lucide_icon("clock", 13.0, ShellDeckColors::primary()))
                    .child(div().flex_1().min_w(px(0.0)).child(notice))
                    .child(
                        Button::new("ai-notice-dismiss", "")
                            .variant(ButtonVariant::Ghost)
                            .size(ButtonSize::Sm)
                            .icon(IconSource::from("x"))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.notice = None;
                                cx.notify();
                            })),
                    ),
            );
        }
        if self.host == AiHost::Sheet {
            composer = composer.max_w(px(624.0)).mx_auto().w_full();
        }
        let mut assistant_composer = Composer::new("ai-composer", &self.prompt_state)
            .placeholder(t!("ai.assistant.placeholder").to_string())
            .min_rows(2)
            .max_rows(8)
            .disabled(self.loading || !self.available)
            // The context left the header: it now rides with the message,
            // where it is visible next to what it will be sent with.
            // Placeholders, on purpose: the affordances are drawn now so the
            // footer has its final shape, but attachments and targeting are
            // not implemented yet. Their handlers only explain that future
            // availability; they never perform the unavailable operation.
            .action(
                Button::new("ai-composer-attach", "")
                    .variant(ButtonVariant::Ghost)
                    .size(ButtonSize::Sm)
                    .icon(IconSource::from("plus"))
                    .tooltip(t!("ai.composer.attach_soon").to_string())
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.set_notice(t!("ai.composer.attach_soon").to_string(), cx);
                    })),
            )
            .action(
                Button::new("ai-composer-target", "")
                    .variant(ButtonVariant::Ghost)
                    .size(ButtonSize::Sm)
                    .icon(IconSource::from("at-sign"))
                    .tooltip(t!("ai.composer.target_soon").to_string())
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.set_notice(t!("ai.composer.target_soon").to_string(), cx);
                    })),
            )
            // The model moves out of the header and into the footer, where
            // every reference app puts it.
            // The chip is a picker: Settings already lets you switch
            // provider, so the surface that uses it should too.
            .option(
                div()
                    .id("ai-backend-picker")
                    .flex()
                    .items_center()
                    .gap(px(3.0))
                    .rounded(px(6.0))
                    .cursor_pointer()
                    .hover(|style| style.bg(ShellDeckColors::hover_bg()))
                    .child(ai_provider_inline(self.backend, &model))
                    .child(
                        svg()
                            .path(lucide_path("chevron-down"))
                            .size(gpui::px(11.0))
                            .mr(gpui::px(4.0))
                            .text_color(ShellDeckColors::text_muted()),
                    )
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.backend_menu_open = !this.backend_menu_open;
                        cx.notify();
                    })),
            )
            .on_commit(submit)
            // Pushed to the right edge under the frame, as in the mockup:
            // the left of that row is reserved for surface-specific
            // settings (the execution mode), so the keyboard hints sit
            // opposite them and stay opposite even when that side is empty.
            .footnote(
                div()
                    .flex()
                    .w_full()
                    .items_center()
                    .justify_end()
                    .gap(px(9.0))
                    .child(t!("ai.assistant.hint.send").to_string())
                    .child(t!("ai.assistant.hint.newline").to_string()),
            );
        if !self.context_dropped {
            assistant_composer = assistant_composer
                // Removable, as the mockup asks: you can see what leaves with
                // the question, and take it off. Dropping it does not clear the
                // host's context — it sends the turn without it, and the chip
                // comes back as soon as the host sets a new one.
                .context(
                    div()
                        .id("ai-context-chip")
                        .flex()
                        .items_center()
                        .gap(px(5.0))
                        .max_w_full()
                        .min_w(px(0.0))
                        .h(px(21.0))
                        .pl(px(7.0))
                        .pr(px(3.0))
                        .rounded(px(6.0))
                        .bg(ShellDeckColors::bg_surface())
                        .text_size(px(10.5))
                        .text_color(ShellDeckColors::text_muted())
                        .child(lucide_icon("sparkles", 11.0, ShellDeckColors::text_muted()))
                        .child(div().truncate().child(self.context.title.clone()))
                        .child(
                            div()
                                .id("ai-context-drop")
                                .flex()
                                .items_center()
                                .justify_center()
                                .flex_shrink_0()
                                .size(px(16.0))
                                .rounded(px(4.0))
                                .cursor_pointer()
                                .hover(|style| style.bg(ShellDeckColors::hover_bg()))
                                .child(
                                    svg()
                                        .path(lucide_path("x"))
                                        .size(px(10.0))
                                        .text_color(ShellDeckColors::text_muted()),
                                )
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.context_dropped = true;
                                    cx.notify();
                                })),
                        ),
                );
        }
        composer = composer.child(assistant_composer);

        let chat = div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .min_h(px(0.0))
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

        // The two hosts are two different displays, so the assembly diverges
        // here — this is the only place it does.
        let mut sheet_header = None;
        let body = match self.host {
            // Dock: the rail is the navigation, so the activity owns the whole
            // panel. History replaces the conversation instead of splitting it;
            // at 424px a second column would leave under 200px of thread.
            AiHost::Dock => {
                let panel = match self.active_tab {
                    AiActivity::Chat => chat.into_any_element(),
                    AiActivity::Clippy => self.render_clippy(cx),
                    AiActivity::Tasks => self.render_tasks(window, cx),
                    AiActivity::History => self.render_history(false, window, cx),
                };
                div()
                    .flex()
                    .w_full()
                    .flex_1()
                    .min_h(px(0.0))
                    .min_w_0()
                    .overflow_hidden()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w_0()
                            .min_h(px(0.0))
                            .overflow_hidden()
                            .child(conversation_header)
                            .child(panel),
                    )
                    .child(self.render_rail(active_tasks, cx))
            }
            // Sheet: no rail. 780px has room for the history column beside the
            // conversation, and the header carries the toggle and the tasks.
            AiHost::Sheet => {
                // The Sheet has no rail, so its header spans the full split.
                sheet_header = Some(conversation_header);
                let main = match self.active_tab {
                    AiActivity::Tasks => self.render_tasks(window, cx),
                    AiActivity::Clippy => self.render_clippy(cx),
                    _ => chat.into_any_element(),
                };
                let mut split = div()
                    .flex()
                    .w_full()
                    .flex_1()
                    .min_h(px(0.0))
                    .min_w_0()
                    .overflow_hidden();
                if self.history_open {
                    split = split.child(self.render_history(true, window, cx));
                }
                split.child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w_0()
                        .min_h(px(0.0))
                        .overflow_hidden()
                        .child(main),
                )
            }
        };

        let mut root = div()
            .flex()
            .flex_col()
            .w_full()
            .flex_1()
            .h_full()
            .min_h(px(0.0))
            .min_w_0()
            .overflow_hidden()
            .children(sheet_header)
            .child(body);

        if self.account_menu_open {
            if let Some(label) = self.account_label.clone() {
                let detail = self.account_detail.clone();
                let panel = div()
                    .id("ai-account-panel")
                    // Anchored to the rail's bottom-right corner, above the
                    // avatar that opened it.
                    .absolute()
                    .bottom(px(52.0))
                    .right(px(62.0))
                    .w(px(226.0))
                    .p(px(4.0))
                    .flex()
                    .flex_col()
                    .gap(px(1.0))
                    .bg(ShellDeckColors::bg_surface())
                    .border_1()
                    .border_color(ShellDeckColors::border())
                    .rounded(px(10.0))
                    .shadow(
                        vec![BoxShadow {
                            color: hsla(0.0, 0.0, 0.0, 0.45),
                            offset: point(gpui::px(0.0), gpui::px(4.0)),
                            blur_radius: gpui::px(20.0),
                            spread_radius: gpui::px(0.0),
                            inset: false,
                        }]
                        .into(),
                    )
                    .on_mouse_down(MouseButton::Left, |_e, _window, cx: &mut App| {
                        cx.stop_propagation();
                    })
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(2.0))
                            .px(px(9.0))
                            .py(px(8.0))
                            .child(
                                div()
                                    .truncate()
                                    .text_size(px(12.5))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(ShellDeckColors::text_primary())
                                    .child(label),
                            )
                            .children(detail.map(|detail| {
                                div()
                                    .truncate()
                                    .text_size(px(11.0))
                                    .text_color(ShellDeckColors::text_muted())
                                    .child(detail)
                            })),
                    )
                    .child(
                        div()
                            .h(px(1.0))
                            .mx(px(6.0))
                            .my(px(3.0))
                            .bg(ShellDeckColors::border()),
                    )
                    .child(
                        div()
                            .id("ai-account-settings")
                            .flex()
                            .items_center()
                            .gap(px(8.0))
                            .px(px(9.0))
                            .py(px(7.0))
                            .rounded(px(7.0))
                            .cursor_pointer()
                            .text_size(px(12.0))
                            .text_color(ShellDeckColors::text_primary())
                            .hover(|style| style.bg(ShellDeckColors::hover_bg()))
                            .child(lucide_icon("settings", 13.0, ShellDeckColors::text_muted()))
                            .child(t!("settings.title").to_string())
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.account_menu_open = false;
                                cx.emit(AiAssistantEvent::OpenSettings);
                            })),
                    );
                root = root.child(
                    div()
                        .id("ai-account-backdrop")
                        .absolute()
                        .top_0()
                        .left_0()
                        .right_0()
                        .bottom_0()
                        .occlude()
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|this, _e, _window, cx| {
                                this.account_menu_open = false;
                                cx.notify();
                            }),
                        )
                        .child(panel),
                );
            }
        }

        if self.backend_menu_open {
            let current = self.backend;
            let mut menu = div()
                .id("ai-backend-menu")
                .absolute()
                .bottom(px(78.0))
                .right(px(16.0))
                .w(px(208.0))
                .p(px(4.0))
                .flex()
                .flex_col()
                .gap(px(1.0))
                .bg(ShellDeckColors::bg_surface())
                .border_1()
                .border_color(ShellDeckColors::border())
                .rounded(px(10.0))
                .shadow(
                    vec![BoxShadow {
                        color: hsla(0.0, 0.0, 0.0, 0.45),
                        // BoxShadow fields are typed `Pixels` — never rems.
                        offset: point(gpui::px(0.0), gpui::px(4.0)),
                        blur_radius: gpui::px(20.0),
                        spread_radius: gpui::px(0.0),
                        inset: false,
                    }]
                    .into(),
                )
                .on_mouse_down(MouseButton::Left, |_e, _window, cx: &mut App| {
                    cx.stop_propagation();
                });
            for (index, (backend, label)) in Self::backend_choices().into_iter().enumerate() {
                let selected = backend == current;
                menu = menu.child(
                    div()
                        .id(("ai-backend-choice", index))
                        .flex()
                        .items_center()
                        .gap(px(8.0))
                        .px(px(9.0))
                        .py(px(7.0))
                        .rounded(px(7.0))
                        .cursor_pointer()
                        .text_size(px(12.0))
                        .text_color(ShellDeckColors::text_primary())
                        .when(selected, |el| el.bg(ShellDeckColors::selected_bg()))
                        .hover(|style| style.bg(ShellDeckColors::hover_bg()))
                        .child(div().flex_1().min_w(px(0.0)).truncate().child(label))
                        .when(selected, |el| {
                            el.child(lucide_icon("check", 13.0, ShellDeckColors::primary()))
                        })
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.backend_menu_open = false;
                            cx.emit(AiAssistantEvent::SelectBackend(backend));
                            cx.notify();
                        })),
                );
            }
            root = root.child(
                // Dismiss layer: a click anywhere else closes the menu.
                div()
                    .id("ai-backend-menu-backdrop")
                    .absolute()
                    .top_0()
                    .left_0()
                    .right_0()
                    .bottom_0()
                    .occlude()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _e, _window, cx| {
                            this.backend_menu_open = false;
                            cx.notify();
                        }),
                    )
                    .child(menu),
            );
        }

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

#[cfg(test)]
mod tests {
    use super::{
        context_switch_resets, sheet_message_reading_width, should_auto_import_clippy,
        validated_clippy_result, AiAssistantView, AiQuickActionMode, AiRequestGate,
    };
    use shelldeck_core::ai::{AiContext, AiSurface, CLIPPY_MAX_RESULT_CHARS};

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

    // SDTEST-1578 — the Dock removes its window on focus loss while the
    // controller and any in-flight request survive; reopening re-prepares the
    // same Global context. That re-preparation must NOT invalidate the gate
    // (the pending reply would land on a dead gate and vanish without spinner
    // or error), while a genuine surface/title switch still must.
    #[test]
    fn reopening_the_same_context_preserves_the_in_flight_request() {
        let global = AiContext::new(
            AiSurface::Global,
            "Global".to_string(),
            serde_json::json!({}),
        );
        // Same surface + title, different payload: a reopen, not a switch.
        let global_reopened = AiContext::new(
            AiSurface::Global,
            "Global".to_string(),
            serde_json::json!({ "active_view": "Dashboard" }),
        );
        let terminal = AiContext::new(
            AiSurface::Terminal,
            "Terminal".to_string(),
            serde_json::json!({}),
        );

        let mut gate = AiRequestGate::default();
        let in_flight = gate.begin();

        // Reopen: `set_context` keeps the gate → the reply is still accepted.
        assert!(!context_switch_resets(&global, &global_reopened));
        assert!(gate.accepts(in_flight));

        // Real switch: `set_context` invalidates → the stale reply is dropped.
        assert!(context_switch_resets(&global_reopened, &terminal));
        gate.invalidate();
        assert!(!gate.accepts(in_flight));
    }

    // SDTEST-1487
    #[test]
    fn automatic_clipboard_import_requires_opt_in_and_an_empty_draft() {
        assert!(!should_auto_import_clippy(false, ""));
        assert!(should_auto_import_clippy(true, "  \n"));
        assert!(!should_auto_import_clippy(true, "keep my draft"));
    }

    // SDTEST-1488
    #[test]
    fn backend_result_must_satisfy_clippy_proposal_bounds_before_display() {
        assert_eq!(
            validated_clippy_result("  reviewed result  ".to_string()).unwrap(),
            "reviewed result"
        );
        assert!(validated_clippy_result("  ".to_string()).is_err());
        assert!(validated_clippy_result("x".repeat(CLIPPY_MAX_RESULT_CHARS + 1)).is_err());
    }

    // SDTEST-1600 — the 240px history column used to leave the message
    // renderer believing it still owned the Sheet's 600px reading measure.
    // Definite user bubbles then extended underneath history and lost their
    // leading text to the split's overflow clip.
    #[test]
    fn sheet_message_width_accounts_for_the_visible_history_column() {
        assert_eq!(
            sheet_message_reading_width(gpui::px(1210.0), false, gpui::px(240.0), gpui::px(600.0)),
            gpui::px(600.0)
        );
        assert_eq!(
            sheet_message_reading_width(gpui::px(1210.0), true, gpui::px(240.0), gpui::px(600.0)),
            gpui::px(540.0)
        );
        assert_eq!(
            sheet_message_reading_width(gpui::px(600.0), true, gpui::px(240.0), gpui::px(600.0)),
            gpui::px(360.0)
        );
    }

    // SDTEST-1429
    #[test]
    fn quick_actions_distinguish_immediate_submit_from_composer_prefill() {
        // The two modes used to be told apart by their adabraka button variant.
        // The empty screen now renders them as tiles, so the visual difference
        // is gone — but the behavioural one is the point of this test and is
        // unchanged: `Submit` sends straight away, `Prefill` only fills the
        // composer. The assertions below pin that mapping per surface.
        let script = AiAssistantView::quick_actions(AiSurface::Script);
        assert_eq!(script[0].mode, AiQuickActionMode::Prefill);
        assert_eq!(script[0].prompt_key, "ai.prefill.script_generate");
        assert_eq!(script[1].mode, AiQuickActionMode::Submit);
        assert_eq!(script[2].mode, AiQuickActionMode::Prefill);
        assert_eq!(script[2].prompt_key, "ai.prefill.script_convert");
        assert!(script[3..]
            .iter()
            .all(|action| action.mode == AiQuickActionMode::Submit));

        let terminal = AiAssistantView::quick_actions(AiSurface::Terminal);
        assert_eq!(terminal[0].mode, AiQuickActionMode::Prefill);
        assert_eq!(terminal[0].prompt_key, "ai.prefill.terminal_command");
        assert_eq!(terminal[1].mode, AiQuickActionMode::Submit);
        assert_eq!(terminal[2].mode, AiQuickActionMode::Prefill);
        assert_eq!(terminal[2].prompt_key, "ai.prefill.terminal_issue");

        let issue = AiAssistantView::quick_actions(AiSurface::Issue);
        assert_eq!(issue[0].mode, AiQuickActionMode::Prefill);
        assert_eq!(issue[0].prompt_key, "ai.prefill.issue_draft");
        assert!(issue[1..]
            .iter()
            .all(|action| action.mode == AiQuickActionMode::Submit));

        let support = AiAssistantView::quick_actions(AiSurface::Support);
        assert_eq!(support[2].prompt_key, "ai.prompt.support_triage_chat");

        for surface in [
            AiSurface::Support,
            AiSurface::Jean,
            AiSurface::Naming,
            AiSurface::Recent,
            AiSurface::Global,
        ] {
            assert!(AiAssistantView::quick_actions(surface)
                .iter()
                .all(|action| action.mode == AiQuickActionMode::Submit));
        }
    }
}
