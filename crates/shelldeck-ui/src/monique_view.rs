//! Native Monique console. Network work stays in `Workspace`; this view only
//! renders the typed Automonique snapshot and emits explicit user actions.

use adabraka_ui::components::input::{Input, InputSize, InputState};
use adabraka_ui::prelude::Markdown;
use gpui::prelude::*;
use gpui::*;
use shelldeck_core::config::monique::{
    MoniqueAgentAccount, MoniqueAgentAccounts, MoniqueAgentAuthAction, MoniqueAgentLoginSession,
    MoniqueChatAction, MoniqueChatHistory, MoniqueChatMessage, MoniqueChatResponse,
    MoniqueProcesses, MoniqueStatus,
};
use std::collections::BTreeMap;

use crate::scale::px;
use crate::t;
use crate::theme::ShellDeckColors;

#[derive(Debug, Clone)]
pub enum MoniqueViewEvent {
    Refresh,
    Send(String),
    ResolveAction { action_id: String, approved: bool },
    NewChat,
    AgentAction(MoniqueAgentAuthAction),
    OpenAuthorization(String),
}

impl EventEmitter<MoniqueViewEvent> for MoniqueView {}

pub struct MoniqueView {
    status: Option<MoniqueStatus>,
    processes: Option<MoniqueProcesses>,
    messages: Vec<MoniqueChatMessage>,
    pending_actions: Vec<MoniqueChatAction>,
    agent_accounts: MoniqueAgentAccounts,
    composer: Entity<InputState>,
    account_label: Entity<InputState>,
    authorization_codes: BTreeMap<String, Entity<InputState>>,
    confirm_logout: Option<String>,
    confirm_remove: Option<String>,
    loading: bool,
    account_loading: bool,
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
            agent_accounts: MoniqueAgentAccounts::default(),
            composer: cx.new(InputState::new),
            account_label: cx.new(InputState::new),
            authorization_codes: BTreeMap::new(),
            confirm_logout: None,
            confirm_remove: None,
            loading: false,
            account_loading: false,
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
        self.account_loading = false;
    }

    pub fn set_account_error(&mut self, error: impl Into<String>) {
        self.error = Some(error.into());
        self.account_loading = false;
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

    pub fn set_agent_accounts(&mut self, accounts: MoniqueAgentAccounts, cx: &mut Context<Self>) {
        self.authorization_codes.retain(|session_id, _| {
            accounts
                .login_sessions
                .iter()
                .any(|session| session.id == *session_id && session.accepts_authorization_code)
        });
        for session in &accounts.login_sessions {
            if session.accepts_authorization_code
                && !self.authorization_codes.contains_key(&session.id)
            {
                self.authorization_codes
                    .insert(session.id.clone(), cx.new(InputState::new));
            }
        }
        self.agent_accounts = accounts;
        self.account_loading = false;
        self.error = None;
    }

    pub fn set_account_loading(&mut self, loading: bool) {
        self.account_loading = loading;
        if loading {
            self.error = None;
        }
    }

    pub fn has_active_login(&self) -> bool {
        self.agent_accounts
            .login_sessions
            .iter()
            .any(MoniqueAgentLoginSession::active)
    }

    pub fn chat_busy(&self) -> bool {
        self.loading
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
        self.account_loading = true;
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

    fn start_agent_login(&mut self, provider: &str, cx: &mut Context<Self>) {
        let entered = self.account_label.read(cx).content().trim().to_string();
        let label = if entered.is_empty() {
            match provider {
                "claude" => t!("monique.accounts.default_claude").to_string(),
                _ => t!("monique.accounts.default_codex").to_string(),
            }
        } else {
            entered
        };
        self.account_label.update(cx, |state, cx| state.reset(cx));
        self.loading = true;
        cx.emit(MoniqueViewEvent::AgentAction(
            MoniqueAgentAuthAction::StartLogin {
                provider: provider.to_string(),
                label,
                account_id: None,
            },
        ));
        cx.notify();
    }

    fn render_agent_account(
        &self,
        account: &MoniqueAgentAccount,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let select_id = account.id.clone();
        let refresh_id = account.id.clone();
        let login_id = account.id.clone();
        let login_provider = account.provider.clone();
        let login_label = account.label.clone();
        let logout_id = account.id.clone();
        let remove_id = account.id.clone();
        let confirming = self.confirm_remove.as_deref() == Some(account.id.as_str());
        let confirming_logout = self.confirm_logout.as_deref() == Some(account.id.as_str());
        let can_select = account.can_select() && !account.worker_selected;
        let state = if account.worker_selected {
            t!("monique.accounts.active").to_string()
        } else {
            account.status.clone()
        };
        let mut actions = div().flex().flex_wrap().gap(px(5.0));
        if can_select {
            actions = actions.child(Self::button(
                ElementId::from(SharedString::from(format!(
                    "monique-account-select-{select_id}"
                ))),
                t!("monique.accounts.use").to_string(),
                cx,
                move |_this, cx| {
                    cx.emit(MoniqueViewEvent::AgentAction(
                        MoniqueAgentAuthAction::Select {
                            account_id: select_id.clone(),
                        },
                    ))
                },
            ));
        }
        actions = actions
            .child(Self::button(
                ElementId::from(SharedString::from(format!(
                    "monique-account-refresh-{refresh_id}"
                ))),
                t!("monique.accounts.verify").to_string(),
                cx,
                move |_this, cx| {
                    cx.emit(MoniqueViewEvent::AgentAction(
                        MoniqueAgentAuthAction::Refresh {
                            account_id: refresh_id.clone(),
                        },
                    ))
                },
            ))
            .child(Self::button(
                ElementId::from(SharedString::from(format!(
                    "monique-account-login-{login_id}"
                ))),
                t!("monique.accounts.sign_in_again").to_string(),
                cx,
                move |_this, cx| {
                    cx.emit(MoniqueViewEvent::AgentAction(
                        MoniqueAgentAuthAction::StartLogin {
                            provider: login_provider.clone(),
                            label: login_label.clone(),
                            account_id: Some(login_id.clone()),
                        },
                    ))
                },
            ))
            .child(Self::button(
                ElementId::from(SharedString::from(format!(
                    "monique-account-logout-{logout_id}"
                ))),
                if confirming_logout {
                    t!("monique.accounts.confirm_sign_out").to_string()
                } else {
                    t!("monique.accounts.sign_out").to_string()
                },
                cx,
                move |this, cx| {
                    if this.confirm_logout.as_deref() == Some(logout_id.as_str()) {
                        this.confirm_logout = None;
                        cx.emit(MoniqueViewEvent::AgentAction(
                            MoniqueAgentAuthAction::Logout {
                                account_id: logout_id.clone(),
                                confirm: true,
                            },
                        ));
                    } else {
                        this.confirm_logout = Some(logout_id.clone());
                        cx.notify();
                    }
                },
            ));
        if !account.worker_selected {
            actions = actions.child(Self::button(
                ElementId::from(SharedString::from(format!(
                    "monique-account-remove-{remove_id}"
                ))),
                if confirming {
                    t!("monique.accounts.confirm_remove").to_string()
                } else {
                    t!("monique.accounts.remove").to_string()
                },
                cx,
                move |this, cx| {
                    if this.confirm_remove.as_deref() == Some(remove_id.as_str()) {
                        this.confirm_remove = None;
                        cx.emit(MoniqueViewEvent::AgentAction(
                            MoniqueAgentAuthAction::Remove {
                                account_id: remove_id.clone(),
                                confirm: true,
                            },
                        ));
                    } else {
                        this.confirm_remove = Some(remove_id.clone());
                        cx.notify();
                    }
                },
            ));
        }
        div()
            .flex()
            .flex_col()
            .gap(px(5.0))
            .p(px(9.0))
            .rounded(px(8.0))
            .border_1()
            .border_color(if account.worker_selected {
                ShellDeckColors::success()
            } else {
                ShellDeckColors::border()
            })
            .bg(ShellDeckColors::bg_surface())
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(
                        div()
                            .flex_1()
                            .text_size(px(12.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(ShellDeckColors::text_primary())
                            .child(account.label.clone()),
                    )
                    .child(
                        div()
                            .text_size(px(10.0))
                            .text_color(if account.can_select() {
                                ShellDeckColors::success()
                            } else {
                                ShellDeckColors::warning()
                            })
                            .child(state),
                    ),
            )
            .child(
                div()
                    .text_size(px(10.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(format!("{} · {}", account.provider_name, account.method)),
            )
            .child(actions)
    }

    fn render_login_session(
        &self,
        session: &MoniqueAgentLoginSession,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let open_url = session.safe_authorization_url().map(str::to_string);
        let cancel_id = session.id.clone();
        let submit_id = session.id.clone();
        let mut actions = div().flex().flex_wrap().items_center().gap(px(6.0));
        if let Some(url) = open_url {
            actions = actions.child(Self::button(
                ElementId::from(SharedString::from(format!(
                    "monique-login-open-{}",
                    session.id
                ))),
                t!("monique.accounts.continue_login").to_string(),
                cx,
                move |_this, cx| cx.emit(MoniqueViewEvent::OpenAuthorization(url.clone())),
            ));
        }
        if let Some(code) = &session.user_code {
            actions = actions.child(
                div()
                    .px(px(8.0))
                    .py(px(5.0))
                    .rounded(px(6.0))
                    .bg(ShellDeckColors::selected_bg())
                    .text_size(px(12.0))
                    .child(code.clone()),
            );
        }
        if session.accepts_authorization_code {
            let entity = cx.entity();
            if let Some(code_state) = self.authorization_codes.get(&session.id).cloned() {
                let code_state_for_submit = code_state.clone();
                actions = actions
                    .child(
                        div().w(px(210.0)).child(
                            Input::new(&code_state)
                                .size(InputSize::Sm)
                                .placeholder(t!("monique.accounts.authorization_code").to_string()),
                        ),
                    )
                    .child(Self::button(
                        ElementId::from(SharedString::from(format!(
                            "monique-login-submit-{submit_id}"
                        ))),
                        t!("monique.accounts.submit_code").to_string(),
                        cx,
                        move |_this, cx| {
                            entity.update(cx, |_this, cx| {
                                let code =
                                    code_state_for_submit.read(cx).content().trim().to_string();
                                if !code.is_empty() {
                                    code_state_for_submit.update(cx, |state, cx| state.reset(cx));
                                    cx.emit(MoniqueViewEvent::AgentAction(
                                        MoniqueAgentAuthAction::SubmitAuthorizationCode {
                                            session_id: submit_id.clone(),
                                            code,
                                        },
                                    ));
                                }
                            });
                        },
                    ));
            }
        }
        if session.active() {
            actions = actions.child(Self::button(
                ElementId::from(SharedString::from(format!(
                    "monique-login-cancel-{cancel_id}"
                ))),
                t!("monique.accounts.cancel").to_string(),
                cx,
                move |_this, cx| {
                    cx.emit(MoniqueViewEvent::AgentAction(
                        MoniqueAgentAuthAction::CancelLogin {
                            session_id: cancel_id.clone(),
                        },
                    ))
                },
            ));
        }
        div()
            .flex()
            .flex_col()
            .gap(px(6.0))
            .p(px(9.0))
            .rounded(px(8.0))
            .border_1()
            .border_color(ShellDeckColors::warning())
            .child(
                div()
                    .text_size(px(11.0))
                    .text_color(ShellDeckColors::text_primary())
                    .child(format!("{} · {}", session.provider, session.status)),
            )
            .child(actions)
    }

    fn render_agent_accounts(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let account_count = self.agent_accounts.accounts.len();
        let maximum = self.agent_accounts.max_accounts;
        let at_capacity = maximum > 0 && account_count >= maximum;
        let codex_available = self
            .agent_accounts
            .providers
            .iter()
            .any(|provider| provider.id == "codex" && provider.available);
        let claude_available = self
            .agent_accounts
            .providers
            .iter()
            .any(|provider| provider.id == "claude" && provider.available);
        let capacity = if maximum > 0 {
            format!("{account_count} / {maximum}")
        } else {
            account_count.to_string()
        };
        let mut list = div().flex().flex_col().gap(px(7.0));
        for session in &self.agent_accounts.login_sessions {
            list = list.child(self.render_login_session(session, cx));
        }
        for account in &self.agent_accounts.accounts {
            list = list.child(self.render_agent_account(account, cx));
        }
        if self.agent_accounts.accounts.is_empty() && self.agent_accounts.login_sessions.is_empty()
        {
            list = list.child(
                div()
                    .py(px(8.0))
                    .text_size(px(11.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(t!("monique.accounts.empty").to_string()),
            );
        }
        div()
            .id("monique-agent-accounts")
            .mx(px(14.0))
            .mt(px(10.0))
            .p(px(10.0))
            .max_h(px(290.0))
            .overflow_y_scroll()
            .rounded(px(9.0))
            .border_1()
            .border_color(ShellDeckColors::border())
            .bg(ShellDeckColors::bg_primary())
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(7.0))
                    .mb(px(8.0))
                    .child(
                        div()
                            .flex_1()
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .text_size(px(12.0))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(t!("monique.accounts.title").to_string()),
                            )
                            .child(
                                div()
                                    .text_size(px(10.0))
                                    .text_color(ShellDeckColors::text_muted())
                                    .child(t!("monique.accounts.boundary").to_string()),
                            ),
                    )
                    .child(
                        div()
                            .text_size(px(10.0))
                            .text_color(ShellDeckColors::text_muted())
                            .child(if self.account_loading {
                                t!("monique.accounts.updating").to_string()
                            } else {
                                capacity
                            }),
                    )
                    .when(!at_capacity, |header| {
                        header.child(
                            div().w(px(180.0)).child(
                                Input::new(&self.account_label)
                                    .size(InputSize::Sm)
                                    .placeholder(t!("monique.accounts.alias").to_string()),
                            ),
                        )
                    })
                    .when(codex_available && !at_capacity, |header| {
                        header.child(Self::button(
                            "monique-add-codex",
                            t!("monique.accounts.add_codex").to_string(),
                            cx,
                            |this, cx| this.start_agent_login("codex", cx),
                        ))
                    })
                    .when(claude_available && !at_capacity, |header| {
                        header.child(Self::button(
                            "monique-add-claude",
                            t!("monique.accounts.add_claude").to_string(),
                            cx,
                            |this, cx| this.start_agent_login("claude", cx),
                        ))
                    }),
            )
            .child(list)
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
            .child(self.render_agent_accounts(cx))
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
