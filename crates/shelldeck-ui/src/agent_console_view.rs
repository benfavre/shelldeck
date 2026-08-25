//! First-class local/SSH coding-agent console for Dev mode.
//!
//! The view owns presentation and draft state only. Workspace owns process and
//! SSH lifetimes, feeds normalized stream events back, and enforces stop.

use adabraka_ui::components::confirm_dialog::Dialog as UiDialog;
use adabraka_ui::components::icon_source::IconSource;
use adabraka_ui::components::input::{Input, InputSize, InputVariant};
use adabraka_ui::components::input_state::InputState;
use adabraka_ui::components::select::{Select, SelectOption};
use adabraka_ui::overlays::popover::PopoverContent;
use adabraka_ui::prelude::{
    Badge, BadgeVariant, Button, ButtonSize, ButtonVariant, Composer, Markdown, Popover,
};
use gpui::prelude::*;
use gpui::*;
use shelldeck_core::agent_runtime::{
    AgentAccessMode, AgentProvider, AgentRunRequest, AgentStreamEvent, AgentTarget,
    MAX_AGENT_MODEL_BYTES, MAX_AGENT_PROMPT_BYTES, MAX_AGENT_WORKDIR_BYTES,
};
use uuid::Uuid;

use crate::follow_scroll::{follow_latest_if_at_end, pin_to_latest};
use crate::icons::lucide_icon;
use crate::scale::px;
use crate::t;
use crate::theme::ShellDeckColors;

const MAX_TRANSCRIPT_BYTES: usize = 2 * 1024 * 1024;
const MAX_ACTIVITY_ITEMS: usize = 250;
const MAX_POPOVER_ACTIVITY_ITEMS: usize = 12;
const MAX_ACTIVITY_BYTES: usize = 4 * 1024;

fn push_activity(activity: &mut Vec<String>, item: String) {
    if activity.last() == Some(&item) {
        return;
    }
    activity.push(item);
    let overflow = activity.len().saturating_sub(MAX_ACTIVITY_ITEMS);
    if overflow > 0 {
        activity.drain(..overflow);
    }
}

fn popover_activity(activity: &[String]) -> &[String] {
    let start = activity.len().saturating_sub(MAX_POPOVER_ACTIVITY_ITEMS);
    &activity[start..]
}

#[derive(Debug, Clone)]
pub enum AgentConsoleEvent {
    Run(AgentRunRequest),
    Stop(Uuid),
}

impl EventEmitter<AgentConsoleEvent> for AgentConsoleView {}

#[derive(Debug, Clone)]
pub struct AgentConnectionOption {
    pub id: Uuid,
    pub label: String,
    pub host: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AgentSessionContext {
    provider: AgentProvider,
    target: AgentTarget,
    access: AgentAccessMode,
    workdir: String,
    model: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AgentRunDisposition {
    Run,
    Confirm,
}

fn run_disposition(access: AgentAccessMode) -> AgentRunDisposition {
    match access {
        AgentAccessMode::ReadOnly => AgentRunDisposition::Run,
        AgentAccessMode::WorkspaceWrite | AgentAccessMode::FullAccess => {
            AgentRunDisposition::Confirm
        }
    }
}

impl From<&AgentRunRequest> for AgentSessionContext {
    fn from(request: &AgentRunRequest) -> Self {
        Self {
            provider: request.provider,
            target: request.target.clone(),
            access: request.access,
            workdir: request.workdir.clone(),
            model: request.model.clone(),
        }
    }
}

fn matching_resume_token(
    context: Option<&AgentSessionContext>,
    token: Option<&String>,
    request: &AgentRunRequest,
) -> Option<String> {
    (context == Some(&AgentSessionContext::from(request)))
        .then(|| token.cloned())
        .flatten()
}

fn target_label(target: &AgentTarget) -> String {
    match target {
        AgentTarget::Local => t!("agents.target.local").to_string(),
        AgentTarget::Ssh {
            connection_label, ..
        } => connection_label.clone(),
    }
}

pub struct AgentConsoleView {
    provider: AgentProvider,
    access: AgentAccessMode,
    connections: Vec<AgentConnectionOption>,
    selected_connection: Option<Uuid>,
    provider_select: Entity<Select<AgentProvider>>,
    access_select: Entity<Select<AgentAccessMode>>,
    target_select: Entity<Select<Option<Uuid>>>,
    workdir_state: Entity<InputState>,
    model_state: Entity<InputState>,
    prompt_state: Entity<InputState>,
    running: Option<AgentRunRequest>,
    session_context: Option<AgentSessionContext>,
    session_token: Option<String>,
    run_received_delta: bool,
    pending_confirm: Option<AgentRunRequest>,
    transcript: String,
    activity: Vec<String>,
    error: Option<String>,
    scroll: ScrollHandle,
}

fn input_state(cx: &mut Context<AgentConsoleView>, initial: String) -> Entity<InputState> {
    cx.new(|cx| {
        let mut state = InputState::new(cx);
        state.content = initial.into();
        state
    })
}

impl AgentConsoleView {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let parent = cx.entity();
        let provider_select = cx.new({
            let parent = parent.clone();
            move |select_cx| {
                Select::new(select_cx)
                    .options(vec![
                        SelectOption::new(AgentProvider::Claude, "Claude Code")
                            .with_icon("icons/simple/claudecode.svg"),
                        SelectOption::new(AgentProvider::Codex, "Codex")
                            .with_icon("icons/simple/openai.svg"),
                        SelectOption::new(AgentProvider::Jcode, "Jcode (auto)")
                            .with_icon("icons/lucide/sparkles.svg"),
                        SelectOption::new(AgentProvider::DeepSeek, "DeepSeek (Jcode)")
                            .with_icon("icons/lucide/bot.svg"),
                    ])
                    .selected_index(Some(0))
                    .context_label(t!("agents.provider").to_string())
                    .on_change(move |provider, _window, cx| {
                        parent.update(cx, |this, cx| {
                            this.provider = *provider;
                            cx.notify();
                        });
                    })
            }
        });
        let access_select = cx.new({
            let parent = parent.clone();
            move |select_cx| {
                Select::new(select_cx)
                    .options(vec![
                        SelectOption::new(
                            AgentAccessMode::ReadOnly,
                            t!("agents.access.read_only").to_string(),
                        )
                        .with_icon("icons/lucide/lock.svg"),
                        SelectOption::new(
                            AgentAccessMode::WorkspaceWrite,
                            t!("agents.access.workspace_write").to_string(),
                        )
                        .with_icon("icons/lucide/pencil.svg"),
                        SelectOption::new(
                            AgentAccessMode::FullAccess,
                            t!("agents.access.full").to_string(),
                        )
                        .with_icon("icons/lucide/shield.svg"),
                    ])
                    .selected_index(Some(0))
                    .context_label(t!("agents.access.label").to_string())
                    .on_change(move |access, _window, cx| {
                        parent.update(cx, |this, cx| {
                            this.access = *access;
                            cx.notify();
                        });
                    })
            }
        });
        let target_select = Self::build_target_select(&[], None, parent, cx);
        let default_workdir = std::env::current_dir()
            .ok()
            .map(|path| path.display().to_string())
            .filter(|path| !path.is_empty())
            .unwrap_or_else(|| "/tmp".to_string());
        Self {
            provider: AgentProvider::Claude,
            access: AgentAccessMode::ReadOnly,
            connections: Vec::new(),
            selected_connection: None,
            provider_select,
            access_select,
            target_select,
            workdir_state: input_state(cx, default_workdir),
            model_state: input_state(cx, String::new()),
            prompt_state: cx.new(|cx| InputState::new(cx).multi_line(true)),
            running: None,
            session_context: None,
            session_token: None,
            run_received_delta: false,
            pending_confirm: None,
            transcript: String::new(),
            activity: Vec::new(),
            error: None,
            scroll: ScrollHandle::new(),
        }
    }

    fn build_target_select(
        connections: &[AgentConnectionOption],
        selected: Option<Uuid>,
        parent: Entity<Self>,
        cx: &mut Context<Self>,
    ) -> Entity<Select<Option<Uuid>>> {
        let mut options = vec![
            SelectOption::new(None, t!("agents.target.local").to_string())
                .with_icon("icons/lucide/cpu.svg"),
        ];
        options.extend(connections.iter().map(|connection| {
            SelectOption::new(
                Some(connection.id),
                format!("{} — {}", connection.label, connection.host),
            )
            .with_icon("icons/lucide/server.svg")
        }));
        let selected_index = selected
            .and_then(|id| options.iter().position(|option| option.value == Some(id)))
            .or(Some(0));
        cx.new(move |select_cx| {
            Select::new(select_cx)
                .options(options)
                .selected_index(selected_index)
                .searchable(true)
                .search_placeholder(t!("agents.target.search").to_string())
                .context_label(t!("agents.target.label").to_string())
                .on_change(move |connection_id, _window, cx| {
                    parent.update(cx, |this, cx| {
                        this.selected_connection = *connection_id;
                        cx.notify();
                    });
                })
        })
    }

    pub fn set_connections(
        &mut self,
        connections: Vec<AgentConnectionOption>,
        cx: &mut Context<Self>,
    ) {
        if self
            .selected_connection
            .is_some_and(|id| !connections.iter().any(|connection| connection.id == id))
        {
            self.selected_connection = None;
        }
        self.connections = connections;
        self.target_select =
            Self::build_target_select(&self.connections, self.selected_connection, cx.entity(), cx);
        cx.notify();
    }

    pub fn begin_run(&mut self, request: AgentRunRequest, cx: &mut Context<Self>) {
        pin_to_latest(&self.scroll);
        self.prompt_state.update(cx, |state, cx| state.reset(cx));
        let continuing = request.resume_session.is_some();
        if !continuing {
            self.transcript.clear();
            self.activity.clear();
            self.session_token = None;
        }
        if !self.transcript.is_empty() {
            self.transcript.push_str("\n\n---\n\n");
        }
        self.transcript.push_str(&format!(
            "**{}**\n\n{}\n\n**{}**\n\n",
            t!("agents.role.you"),
            markdown_quote(&request.prompt),
            t!("agents.role.agent")
        ));
        retain_recent_utf8(&mut self.transcript, MAX_TRANSCRIPT_BYTES);
        self.session_context = Some(AgentSessionContext::from(&request));
        self.running = Some(request);
        self.run_received_delta = false;
        self.error = None;
        cx.notify();
    }

    pub fn reject_run(&mut self, error: String, cx: &mut Context<Self>) {
        self.error = Some(bounded_utf8(error, MAX_ACTIVITY_BYTES));
        cx.notify();
    }

    pub fn push_stream_event(&mut self, event: AgentStreamEvent, cx: &mut Context<Self>) {
        follow_latest_if_at_end(&self.scroll);
        match event {
            AgentStreamEvent::Text(text) => {
                let streamed_jcode = self.run_received_delta
                    && self.running.as_ref().is_some_and(|run| {
                        matches!(run.provider, AgentProvider::Jcode | AgentProvider::DeepSeek)
                    });
                if streamed_jcode {
                    return;
                }
                if !self.transcript.is_empty() && !self.transcript.ends_with('\n') {
                    self.transcript.push('\n');
                }
                self.transcript.push_str(&text);
                retain_recent_utf8(&mut self.transcript, MAX_TRANSCRIPT_BYTES);
            }
            AgentStreamEvent::TextDelta(delta) => {
                self.run_received_delta = true;
                self.transcript.push_str(&delta);
                retain_recent_utf8(&mut self.transcript, MAX_TRANSCRIPT_BYTES);
            }
            AgentStreamEvent::Session(session) => {
                self.session_token = Some(session);
            }
            AgentStreamEvent::Ready => {
                let provider = self
                    .running
                    .as_ref()
                    .map(|run| run.provider.display_name())
                    .unwrap_or_else(|| self.provider.display_name());
                push_activity(
                    &mut self.activity,
                    t!("agents.status.ready", provider = provider).to_string(),
                );
            }
            AgentStreamEvent::Activity(activity) => {
                let activity = bounded_utf8(activity, MAX_ACTIVITY_BYTES);
                push_activity(&mut self.activity, activity);
            }
            AgentStreamEvent::Error(error) => {
                self.error = Some(bounded_utf8(error, MAX_ACTIVITY_BYTES))
            }
        }
        cx.notify();
    }

    pub fn finish_run(
        &mut self,
        run_id: Uuid,
        exit_code: Option<i32>,
        error: Option<String>,
        cx: &mut Context<Self>,
    ) {
        if self.running.as_ref().is_none_or(|run| run.id != run_id) {
            return;
        }
        let run = self.running.take();
        if let Some(error) = error {
            self.error = Some(bounded_utf8(error, MAX_ACTIVITY_BYTES));
        }
        push_activity(
            &mut self.activity,
            match exit_code {
                Some(0) => t!("agents.status.completed").to_string(),
                Some(code) => t!("agents.status.exit", code = code).to_string(),
                None => t!("agents.status.stopped").to_string(),
            },
        );
        if exit_code == Some(0) && self.session_token.is_none() {
            if let Some(run) = &run {
                if run.provider == AgentProvider::Claude && run.resume_session.is_none() {
                    self.session_token = Some(run.id.to_string());
                }
            }
        }
        if exit_code != Some(0) {
            self.session_context = None;
            self.session_token = None;
        }
        follow_latest_if_at_end(&self.scroll);
        cx.notify();
    }

    fn request_from_draft(&self, cx: &Context<Self>) -> Result<AgentRunRequest, String> {
        let prompt = self.prompt_state.read(cx).content().trim().to_string();
        let workdir = self.workdir_state.read(cx).content().trim().to_string();
        let model = self.model_state.read(cx).content().trim().to_string();
        if prompt.is_empty() {
            return Err(t!("agents.error.prompt_empty").to_string());
        }
        if prompt.len() > MAX_AGENT_PROMPT_BYTES {
            return Err(t!("agents.error.prompt_too_long").to_string());
        }
        if workdir.is_empty() || !std::path::Path::new(&workdir).is_absolute() {
            return Err(t!("agents.error.workdir_absolute").to_string());
        }
        if workdir.len() > MAX_AGENT_WORKDIR_BYTES {
            return Err(t!("agents.error.workdir_too_long").to_string());
        }
        if model.len() > MAX_AGENT_MODEL_BYTES {
            return Err(t!("agents.error.model_too_long").to_string());
        }
        let target = match self.selected_connection {
            None => AgentTarget::Local,
            Some(connection_id) => {
                let connection = self
                    .connections
                    .iter()
                    .find(|connection| connection.id == connection_id)
                    .ok_or_else(|| t!("agents.error.target_missing").to_string())?;
                AgentTarget::Ssh {
                    connection_id,
                    connection_label: connection.label.clone(),
                }
            }
        };
        let request = AgentRunRequest::new(
            self.provider,
            target,
            self.access,
            workdir,
            (!model.is_empty()).then_some(model),
            prompt,
        );
        let resume = matching_resume_token(
            self.session_context.as_ref(),
            self.session_token.as_ref(),
            &request,
        );
        let request = request.with_resume_session(resume);
        request.validate().map_err(|error| error.to_string())?;
        Ok(request)
    }

    fn submit(&mut self, cx: &mut Context<Self>) {
        if self.running.is_some() {
            return;
        }
        match self.request_from_draft(cx) {
            Ok(request) => match run_disposition(request.access) {
                AgentRunDisposition::Run => cx.emit(AgentConsoleEvent::Run(request)),
                AgentRunDisposition::Confirm => {
                    self.pending_confirm = Some(request);
                    cx.notify();
                }
            },
            Err(error) => {
                self.error = Some(error);
                cx.notify();
            }
        }
    }

    fn confirm_pending_run(&mut self, cx: &mut Context<Self>) {
        let Some(request) = self.pending_confirm.take() else {
            return;
        };
        cx.emit(AgentConsoleEvent::Run(request));
        cx.notify();
    }

    fn new_session(&mut self, cx: &mut Context<Self>) {
        if self.running.is_some() {
            return;
        }
        self.session_context = None;
        self.session_token = None;
        self.transcript.clear();
        self.activity.clear();
        self.error = None;
        cx.notify();
    }

    fn render_confirmation(&self, request: AgentRunRequest, cx: &mut Context<Self>) -> AnyElement {
        let target = target_label(&request.target);
        let access = match request.access {
            AgentAccessMode::ReadOnly => t!("agents.access.read_only").to_string(),
            AgentAccessMode::WorkspaceWrite => t!("agents.access.workspace_write").to_string(),
            AgentAccessMode::FullAccess => t!("agents.access.full").to_string(),
        };
        let destructive = request.access == AgentAccessMode::FullAccess;
        UiDialog::new()
            .width(gpui::px(520.0))
            .header(
                div()
                    .flex()
                    .items_center()
                    .gap(px(9.0))
                    .px(px(16.0))
                    .py(px(14.0))
                    .border_b_1()
                    .border_color(ShellDeckColors::border())
                    .child(lucide_icon(
                        "shield-check",
                        17.0,
                        if destructive {
                            ShellDeckColors::error()
                        } else {
                            ShellDeckColors::warning()
                        },
                    ))
                    .child(
                        div()
                            .text_size(px(15.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(t!("agents.confirm.title").to_string()),
                    ),
            )
            .content(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(10.0))
                    .px(px(16.0))
                    .py(px(14.0))
                    .text_size(px(12.0))
                    .child(t!("agents.confirm.description").to_string())
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .gap(px(6.0))
                            .child(
                                Badge::new(request.provider.display_name())
                                    .variant(BadgeVariant::Outline),
                            )
                            .child(Badge::new(target).variant(BadgeVariant::Outline))
                            .child(Badge::new(access).variant(BadgeVariant::Outline)),
                    )
                    .child(
                        div()
                            .rounded(px(7.0))
                            .bg(ShellDeckColors::bg_surface())
                            .overflow_hidden()
                            .px(px(10.0))
                            .py(px(8.0))
                            .font_family("JetBrains Mono")
                            .text_size(px(11.0))
                            .line_clamp(3)
                            .child(request.workdir.clone()),
                    ),
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
                        Button::new("agents-confirm-cancel", t!("scripts.cancel").to_string())
                            .variant(ButtonVariant::Ghost)
                            .size(ButtonSize::Sm)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.pending_confirm = None;
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("agents-confirm-run", t!("agents.run").to_string())
                            .variant(if destructive {
                                ButtonVariant::Destructive
                            } else {
                                ButtonVariant::Default
                            })
                            .size(ButtonSize::Sm)
                            .icon(IconSource::from("play"))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.confirm_pending_run(cx);
                            })),
                    ),
            )
            .on_backdrop_click({
                let parent = cx.entity();
                move |_, cx| {
                    parent.update(cx, |this, cx| {
                        this.pending_confirm = None;
                        cx.notify();
                    });
                }
            })
            .into_any_element()
    }

    fn render_activity_popover(&self) -> AnyElement {
        let entries = popover_activity(&self.activity).to_vec();
        let entry_count = self.activity.len();
        let title = t!("agents.activity.title").to_string();
        let trigger = div()
            .id("agent-activity-trigger")
            .flex()
            .items_center()
            .justify_center()
            .size(px(28.0))
            .flex_shrink_0()
            .rounded_full()
            .border_1()
            .border_color(ShellDeckColors::border())
            .cursor_pointer()
            .text_color(ShellDeckColors::text_muted())
            .hover(|style| style.bg(ShellDeckColors::hover_bg()))
            .child(lucide_icon("activity", 13.0, ShellDeckColors::text_muted()));

        Popover::new("agent-activity-popover")
            .anchor(Corner::TopRight)
            .trigger(trigger)
            .content(move |window, cx| {
                let entries = entries.clone();
                let title = title.clone();
                cx.new(move |content_cx| {
                    PopoverContent::new(window, content_cx, move |_window, _cx| {
                        div()
                            .w(px(280.0))
                            .flex()
                            .flex_col()
                            .gap(px(8.0))
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(7.0))
                                    .child(lucide_icon(
                                        "activity",
                                        13.0,
                                        ShellDeckColors::text_muted(),
                                    ))
                                    .child(
                                        div()
                                            .flex_1()
                                            .text_size(px(12.0))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .child(title.clone()),
                                    )
                                    .child(
                                        Badge::new(entry_count.to_string())
                                            .variant(BadgeVariant::Secondary),
                                    ),
                            )
                            .child(
                                div()
                                    .id("agent-activity-list")
                                    .flex()
                                    .flex_col()
                                    .max_h(px(280.0))
                                    .overflow_y_scroll()
                                    .children(entries.iter().enumerate().map(
                                        |(index, activity)| {
                                            div()
                                                .id(("agent-activity-row", index))
                                                .flex()
                                                .items_start()
                                                .gap(px(8.0))
                                                .py(px(5.0))
                                                .text_size(px(11.0))
                                                .text_color(ShellDeckColors::text_muted())
                                                .child(
                                                    div()
                                                        .mt(px(5.0))
                                                        .size(px(4.0))
                                                        .flex_shrink_0()
                                                        .rounded_full()
                                                        .bg(ShellDeckColors::text_muted()),
                                                )
                                                .child(
                                                    div()
                                                        .min_w(px(0.0))
                                                        .whitespace_normal()
                                                        .child(activity.clone()),
                                                )
                                        },
                                    )),
                            )
                            .into_any_element()
                    })
                })
            })
            .into_any_element()
    }

    fn render_model_popover(&self, cx: &mut Context<Self>) -> AnyElement {
        let configured = self.model_state.read(cx).content().trim().to_string();
        let display = if configured.is_empty() {
            t!("agents.model.auto").to_string()
        } else {
            configured
        };
        let trigger = div()
            .id("agent-model-trigger")
            .h(px(26.0))
            .max_w(px(180.0))
            .flex()
            .items_center()
            .gap(px(5.0))
            .px(px(6.0))
            .rounded(px(7.0))
            .cursor_pointer()
            .text_size(px(10.5))
            .text_color(ShellDeckColors::text_muted())
            .hover(|style| style.bg(ShellDeckColors::hover_bg()))
            .child(
                div()
                    .min_w(px(0.0))
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .child(display),
            )
            .child(lucide_icon(
                "chevron-down",
                11.0,
                ShellDeckColors::text_muted(),
            ));
        let model_state = self.model_state.clone();
        let parent = cx.entity();
        let title = t!("agents.model.label").to_string();
        let hint = t!("agents.model.hint").to_string();
        let placeholder = t!("agents.model.auto").to_string();

        Popover::new("agent-model-popover")
            .anchor(Corner::BottomRight)
            .trigger(trigger)
            .content(move |window, cx| {
                let model_state = model_state.clone();
                let parent = parent.clone();
                let title = title.clone();
                let hint = hint.clone();
                let placeholder = placeholder.clone();
                cx.new(move |content_cx| {
                    PopoverContent::new(window, content_cx, move |_window, _cx| {
                        let notify_parent = parent.clone();
                        div()
                            .w(px(260.0))
                            .flex()
                            .flex_col()
                            .gap(px(7.0))
                            .child(
                                div()
                                    .text_size(px(11.0))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(title.clone()),
                            )
                            .child(
                                Input::new(&model_state)
                                    .size(InputSize::Sm)
                                    .placeholder(placeholder.clone())
                                    .on_change(move |_value, cx| {
                                        notify_parent.update(cx, |_this, cx| cx.notify());
                                    }),
                            )
                            .child(
                                div()
                                    .text_size(px(10.0))
                                    .line_height(relative(1.35))
                                    .text_color(ShellDeckColors::text_muted())
                                    .child(hint.clone()),
                            )
                            .into_any_element()
                    })
                })
            })
            .into_any_element()
    }
}

impl Render for AgentConsoleView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let running = self.running.clone();
        let is_running = running.is_some();
        let submit = {
            let parent = cx.entity();
            move |cx: &mut App| {
                parent.update(cx, |this, cx| this.submit(cx));
            }
        };

        let controls_layout =
            agent_controls_layout(window.viewport_size().width, window.rem_size());
        let provider = context_select_cell(self.provider_select.clone());
        let target = context_select_cell(self.target_select.clone());
        let access = context_select_cell(self.access_select.clone());
        let select_rows = match controls_layout {
            AgentControlsLayout::Wide => div()
                .flex()
                .w_full()
                .child(
                    provider
                        .border_r_1()
                        .border_color(ShellDeckColors::border()),
                )
                .child(target.border_r_1().border_color(ShellDeckColors::border()))
                .child(access),
            AgentControlsLayout::Compact => div()
                .flex()
                .flex_col()
                .w_full()
                .child(
                    div()
                        .flex()
                        .w_full()
                        .child(
                            provider
                                .border_r_1()
                                .border_color(ShellDeckColors::border()),
                        )
                        .child(target),
                )
                .child(access.border_t_1().border_color(ShellDeckColors::border())),
            AgentControlsLayout::Narrow => div()
                .flex()
                .flex_col()
                .w_full()
                .child(provider)
                .child(target.border_t_1().border_color(ShellDeckColors::border()))
                .child(access.border_t_1().border_color(ShellDeckColors::border())),
        };
        let workdir_focused = self
            .workdir_state
            .read(cx)
            .focus_handle(cx)
            .is_focused(window);
        let workdir = div()
            .flex()
            .items_center()
            .gap(px(8.0))
            .min_w(px(0.0))
            .h(px(42.0))
            .px(px(11.0))
            .border_t_1()
            .border_color(ShellDeckColors::border())
            .child(lucide_icon("folder", 14.0, ShellDeckColors::text_muted()))
            .when(controls_layout != AgentControlsLayout::Narrow, |row| {
                row.child(
                    div()
                        .flex_shrink_0()
                        .text_size(px(9.0))
                        .text_color(ShellDeckColors::text_muted())
                        .child(t!("agents.workdir_short").to_string()),
                )
            })
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .h(px(32.0))
                    .overflow_hidden()
                    .border_1()
                    .border_color(if workdir_focused {
                        ShellDeckColors::primary()
                    } else {
                        gpui::transparent_black()
                    })
                    .rounded(px(7.0))
                    .when(workdir_focused, |field| {
                        field.bg(ShellDeckColors::selected_bg())
                    })
                    .child(
                        Input::new(&self.workdir_state)
                            .variant(InputVariant::Bare)
                            .size(InputSize::Sm)
                            .placeholder("/srv/project"),
                    ),
            );
        let controls = div()
            .flex()
            .flex_col()
            .gap(px(7.0))
            .px(px(18.0))
            .py(px(12.0))
            .border_b_1()
            .border_color(ShellDeckColors::border())
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(
                        div()
                            .text_size(px(11.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(t!("agents.context.title").to_string()),
                    )
                    .when(controls_layout != AgentControlsLayout::Narrow, |row| {
                        row.child(
                            div()
                                .text_size(px(10.5))
                                .text_color(ShellDeckColors::text_muted())
                                .child(t!("agents.context.hint").to_string()),
                        )
                    }),
            )
            .child(
                div()
                    .w_full()
                    .min_w(px(0.0))
                    .overflow_hidden()
                    .border_1()
                    .border_color(ShellDeckColors::border())
                    .rounded(px(10.0))
                    .child(select_rows)
                    .child(workdir),
            );

        let mut output = div()
            .id("agent-console-output")
            .flex()
            .flex_col()
            .flex_1()
            .min_h(px(0.0))
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
            .px(px(22.0))
            .py(px(18.0));
        if self.transcript.is_empty() && self.activity.is_empty() && self.error.is_none() {
            let narrow_empty = controls_layout == AgentControlsLayout::Narrow;
            let empty_gap = if narrow_empty { 5.0 } else { 10.0 };
            let empty_icon_size = if narrow_empty { 22.0 } else { 30.0 };
            let empty_title_size = if narrow_empty { 14.0 } else { 15.0 };
            let mut empty_state = div()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .flex_1()
                .gap(px(empty_gap))
                .text_color(ShellDeckColors::text_muted())
                .child(lucide_icon(
                    "bot",
                    empty_icon_size,
                    ShellDeckColors::primary(),
                ))
                .child(
                    div()
                        .text_size(px(empty_title_size))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(ShellDeckColors::text_primary())
                        .child(t!("agents.empty.title").to_string()),
                );
            if !narrow_empty {
                empty_state = empty_state.child(
                    div()
                        .w(px(520.0))
                        .max_w_full()
                        .min_w(px(0.0))
                        .overflow_hidden()
                        .text_size(px(12.0))
                        .line_height(relative(1.35))
                        .text_align(TextAlign::Center)
                        .whitespace_normal()
                        .child(t!("agents.empty.description").to_string()),
                );
            }
            output = output.child(empty_state);
        } else {
            if let Some(run) = running.as_ref() {
                output = output.child(
                    div()
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .gap(px(6.0))
                        .mb(px(10.0))
                        .child(
                            Badge::new(run.provider.display_name()).variant(BadgeVariant::Outline),
                        )
                        .child(Badge::new(target_label(&run.target)).variant(BadgeVariant::Outline))
                        .child(Badge::new(run.workdir.clone()).variant(BadgeVariant::Secondary)),
                );
            }
            if !self.transcript.is_empty() {
                output = output.child(
                    div()
                        .w_full()
                        .max_w(px(860.0))
                        .mx_auto()
                        .min_w(px(0.0))
                        .overflow_hidden()
                        .py(px(8.0))
                        .text_color(ShellDeckColors::text_primary())
                        .child(
                            Markdown::new(self.transcript.clone())
                                .base_font_size(px(13.0).to_pixels(window.rem_size()))
                                .compact()
                                .min_w_0()
                                .whitespace_normal(),
                        ),
                );
            }
        }
        if let Some(error) = &self.error {
            output = output.child(
                div()
                    .flex()
                    .items_start()
                    .gap(px(8.0))
                    .mt(px(10.0))
                    .px(px(10.0))
                    .py(px(8.0))
                    .overflow_hidden()
                    .rounded(px(7.0))
                    .bg(ShellDeckColors::error().opacity(0.10))
                    .text_size(px(12.0))
                    .text_color(ShellDeckColors::error())
                    .child(lucide_icon(
                        "triangle-alert",
                        14.0,
                        ShellDeckColors::error(),
                    ))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .line_clamp(4)
                            .child(error.clone()),
                    ),
            );
        }

        let mut composer = Composer::new("agent-console-composer", &self.prompt_state)
            .placeholder(t!("agents.prompt.placeholder").to_string())
            .min_rows(2)
            .max_rows(8)
            .disabled(is_running)
            .on_commit(submit)
            .option(self.render_model_popover(cx));
        if let Some(run) = running {
            let run_id = run.id;
            composer = composer.without_commit().option(
                Button::new("agent-console-stop", t!("agents.stop").to_string())
                    .variant(ButtonVariant::Destructive)
                    .size(ButtonSize::Sm)
                    .icon(IconSource::from("square"))
                    .on_click(cx.listener(move |_this, _, _, cx| {
                        cx.emit(AgentConsoleEvent::Stop(run_id));
                    })),
            );
        }

        let mut header = div()
            .flex()
            .items_center()
            .gap(px(9.0))
            .px(px(18.0))
            .h(px(48.0))
            .border_b_1()
            .border_color(ShellDeckColors::border())
            .child(lucide_icon("bot", 17.0, ShellDeckColors::primary()))
            .child(
                div()
                    .text_size(px(15.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(t!("agents.title").to_string()),
            )
            .child(Badge::new(t!("agents.scope").to_string()).variant(BadgeVariant::Outline));
        if self.session_token.is_some() {
            header = header.child(
                Badge::new(t!("agents.session.active").to_string())
                    .variant(BadgeVariant::Secondary),
            );
        }
        header = header.child(div().flex_1());
        if !self.activity.is_empty() {
            header = header.child(self.render_activity_popover());
        }
        header = header.child(
            Button::new(
                "agent-console-new-session",
                t!("agents.session.new").to_string(),
            )
            .variant(ButtonVariant::Ghost)
            .size(ButtonSize::Sm)
            .disabled(is_running)
            .icon(IconSource::from("plus"))
            .on_click(cx.listener(|this, _, _, cx| this.new_session(cx))),
        );

        let mut root = div()
            .relative()
            .flex()
            .flex_col()
            .size_full()
            .min_h(px(0.0))
            .bg(ShellDeckColors::bg_primary())
            .child(header)
            .child(controls)
            .child(output)
            .child(
                div()
                    .flex_shrink_0()
                    .w_full()
                    .px(px(18.0))
                    .pt(px(8.0))
                    .pb(px(14.0))
                    .child(
                        div()
                            .w_full()
                            .min_w(px(0.0))
                            .max_w(px(900.0))
                            .mx_auto()
                            .child(composer),
                    ),
            );
        if let Some(request) = self.pending_confirm.clone() {
            root = root.child(self.render_confirmation(request, cx));
        }
        root
    }
}

fn context_select_cell<T: Clone + 'static>(select: Entity<Select<T>>) -> Div {
    div()
        .flex()
        .flex_1()
        .min_w(px(0.0))
        .overflow_hidden()
        .child(select)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AgentControlsLayout {
    Wide,
    Compact,
    Narrow,
}

fn agent_controls_layout(viewport_width: Pixels, rem_size: Pixels) -> AgentControlsLayout {
    if viewport_width >= px(960.0).to_pixels(rem_size) {
        AgentControlsLayout::Wide
    } else if viewport_width >= px(720.0).to_pixels(rem_size) {
        AgentControlsLayout::Compact
    } else {
        AgentControlsLayout::Narrow
    }
}

fn bounded_utf8(mut value: String, max_bytes: usize) -> String {
    if value.len() <= max_bytes {
        return value;
    }
    let mut end = max_bytes;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value.truncate(end);
    value
}

fn markdown_quote(value: &str) -> String {
    value
        .lines()
        .map(|line| format!("> {line}"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn retain_recent_utf8(value: &mut String, max_bytes: usize) {
    if value.len() <= max_bytes {
        return;
    }
    let mut start = value.len() - max_bytes;
    while !value.is_char_boundary(start) {
        start += 1;
    }
    value.drain(..start);
}

#[cfg(test)]
mod tests {
    use super::{
        agent_controls_layout, matching_resume_token, popover_activity, push_activity,
        run_disposition, AgentControlsLayout, AgentRunDisposition, AgentSessionContext,
    };
    use shelldeck_core::agent_runtime::{
        AgentAccessMode, AgentProvider, AgentRunRequest, AgentTarget,
    };

    // SDTEST-1678 — SDUC-475
    #[test]
    fn sdtest_1678_session_resume_requires_the_exact_execution_context() {
        let request = AgentRunRequest::new(
            AgentProvider::Codex,
            AgentTarget::Local,
            AgentAccessMode::ReadOnly,
            "/srv/project",
            Some("gpt-5.6".to_string()),
            "inspect",
        );
        let context = AgentSessionContext::from(&request);
        let token = "thread-123".to_string();
        assert_eq!(
            matching_resume_token(Some(&context), Some(&token), &request),
            Some(token.clone())
        );

        let changed_access = AgentRunRequest {
            access: AgentAccessMode::WorkspaceWrite,
            ..request.clone()
        };
        assert_eq!(
            matching_resume_token(Some(&context), Some(&token), &changed_access),
            None
        );
        let changed_target = AgentRunRequest {
            target: AgentTarget::Ssh {
                connection_id: uuid::Uuid::new_v4(),
                connection_label: "prod".to_string(),
            },
            ..request
        };
        assert_eq!(
            matching_resume_token(Some(&context), Some(&token), &changed_target),
            None
        );
    }

    // SDTEST-1679 — SDUC-475
    #[test]
    fn sdtest_1679_mutating_access_always_requires_confirmation() {
        assert_eq!(
            run_disposition(AgentAccessMode::ReadOnly),
            AgentRunDisposition::Run
        );
        for access in [AgentAccessMode::WorkspaceWrite, AgentAccessMode::FullAccess] {
            assert_eq!(run_disposition(access), AgentRunDisposition::Confirm);
        }
    }

    // SDTEST-1705 — SDUC-475
    #[test]
    fn sdtest_1705_agent_context_panel_uses_scale_aware_explicit_rows() {
        assert_eq!(
            agent_controls_layout(gpui::px(959.0), gpui::px(16.0)),
            AgentControlsLayout::Compact
        );
        assert_eq!(
            agent_controls_layout(gpui::px(960.0), gpui::px(16.0)),
            AgentControlsLayout::Wide
        );
        assert_eq!(
            agent_controls_layout(gpui::px(719.0), gpui::px(16.0)),
            AgentControlsLayout::Narrow
        );
        assert_eq!(
            agent_controls_layout(gpui::px(1_919.0), gpui::px(32.0)),
            AgentControlsLayout::Compact
        );
        assert_eq!(
            agent_controls_layout(gpui::px(1_920.0), gpui::px(32.0)),
            AgentControlsLayout::Wide
        );
    }

    // SDTEST-1707 — SDUC-475
    #[test]
    fn sdtest_1707_agent_activity_deduplicates_and_keeps_a_bounded_popover_tail() {
        let mut activity = Vec::new();
        for _ in 0..8 {
            push_activity(&mut activity, "Claude Code est prêt".to_string());
        }
        assert_eq!(activity, ["Claude Code est prêt"]);

        for index in 1..=14 {
            push_activity(&mut activity, format!("étape {index}"));
        }
        assert_eq!(
            popover_activity(&activity),
            [
                "étape 3",
                "étape 4",
                "étape 5",
                "étape 6",
                "étape 7",
                "étape 8",
                "étape 9",
                "étape 10",
                "étape 11",
                "étape 12",
                "étape 13",
                "étape 14",
            ]
        );
    }
}
